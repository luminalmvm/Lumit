//! The Cut workspace's edits: an editor's timeline over an ordinary
//! composition, where every row is a Sequence layer. A picture row is a
//! Sequence layer that draws, a sound row is an audio-only one, and a picture
//! clip and the clip that carries its sound are two clips sharing a link.
//!
//! Every call is one `Batch`, so one undo step, and it applies whole or not at
//! all. An edit that cannot be made answers with the reason
//! ([`BridgeCutResult`]) and leaves the document as it was, because an error
//! reaches the panel with no words in it.
//!
//! Runs on the UI thread, like every other document edit.

use flutter_rust_bridge::frb;
use lumit_core::model::{Composition, FootageItem, Layer, LayerKind, ProjectItem};
use lumit_core::sequence::{self, Clip, ClipSource};
use lumit_core::time::{CompTime, Duration, Rational, TimeError};
use lumit_core::Op;
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference,
    effect::BridgeRational,
    footage::FootageReference,
    layer::{placed, trimmed, LayerReference},
    state::LumitBridgeState,
    BridgeError,
};

/// How a Cut edit ended. Anything but `Done` left the document untouched, and
/// says why in a form the panel can put words to.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCutResult {
    /// The edit was made, or there was nothing to change.
    Done,
    /// A clip the edit would carry along would land on one that stays.
    Overlap,
    /// A clip would start before the composition does.
    BeforeStart,
    /// A layer the edit has to change is locked.
    Locked,
    /// A clip has no more to give: an edge would pass its other edge, a
    /// neighbour would be left with no length, or a slip would run off the
    /// top of the source.
    Limit,
    /// A clip's Retime is driven by an expression, which cannot be cut.
    Uncuttable,
    /// Nothing there to act on: no clip under the razor, no gap at that time,
    /// or no edit point at that edge.
    Nothing,
    /// The layer cannot take the clip: a picture on a sound layer, or sound
    /// on a picture layer.
    WrongLayer,
}

/// Why an edit stopped: a refusal for the panel to show, or a fault.
#[frb(ignore)]
enum Stop {
    Refused(BridgeCutResult),
    Failed(BridgeError),
}

impl From<BridgeCutResult> for Stop {
    fn from(why: BridgeCutResult) -> Self {
        Stop::Refused(why)
    }
}

impl From<BridgeError> for Stop {
    fn from(error: BridgeError) -> Self {
        Stop::Failed(error)
    }
}

impl From<TimeError> for Stop {
    fn from(_: TimeError) -> Self {
        Stop::Failed(BridgeError::InvalidTime)
    }
}

type Cut<T = ()> = Result<T, Stop>;

/// One Sequence layer as an edit works on it. Its clips are placed in **comp
/// time**, the layer's own zero already added, so several rows are edited on
/// one clock and a clip changes row without converting anything.
#[frb(ignore)]
struct Row {
    id: Uuid,
    audio_only: bool,
    locked: bool,
    audible: bool,
    /// The layer's own zero as it was read.
    offset: Rational,
    clips: Vec<Clip>,
    /// A layer this edit makes, added with its clips when the edit commits.
    fresh: Option<Layer>,
}

/// An edit in progress: the composition as it was read, and its rows as they
/// will be written. Nothing reaches the document until [`Edit::commit`].
///
/// ponytail: every row is read and compared whole on each edit, and a link is
/// found by scanning. Fine at a few hundred clips; it wants an index when
/// `SetSequenceClips` stops carrying whole lists.
#[frb(ignore)]
struct Edit<'a> {
    of: &'a CompositionReference,
    comp: Composition,
    rows: Vec<Row>,
    /// Layers of other kinds a ripple has carried, moved in `comp` itself.
    carried: Vec<Uuid>,
}

/// Open an edit on `of`, run it, and turn how it ended into the answer.
#[frb(ignore)]
fn cut<'a>(
    of: &'a CompositionReference,
    edit: impl FnOnce(Edit<'a>) -> Cut,
) -> Result<BridgeCutResult, BridgeError> {
    match Edit::open(of).and_then(edit) {
        Ok(()) => Ok(BridgeCutResult::Done),
        Err(Stop::Refused(why)) => Ok(why),
        Err(Stop::Failed(error)) => Err(error),
    }
}

/// Whether this media carries sound. Media that will not probe answers false:
/// a sound clip of a file that cannot be read would be a silent box.
#[frb(ignore)]
fn has_sound(state: &LumitBridgeState, footage: &FootageItem) -> bool {
    #[cfg(feature = "media")]
    {
        FootageReference::resolve_source(state, footage)
            .and_then(|src| crate::probe::ensure_probed(&src))
            .is_some_and(|info| info.audio.is_some())
    }

    #[cfg(not(feature = "media"))]
    {
        let _ = (state, footage);
        false
    }
}

/// Whether `clip` shares any of the row with the span `start..end`.
#[frb(ignore)]
fn covers(clip: &Clip, start: Rational, end: Rational) -> bool {
    clip.place_start < end && start < clip.place_end()
}

/// One clip with its edges moved by `by_start` and `by_end`. With `ripple` a
/// moved head leaves the clip starting where it did and its end takes up the
/// difference, which is what lets the row close up or open behind it.
#[frb(ignore)]
fn retrimmed(clip: &Clip, by_start: Rational, by_end: Rational, ripple: bool) -> Option<Clip> {
    let end = clip.place_end().checked_add(by_end).ok()?;
    if !ripple {
        return trimmed(clip, clip.place_start.checked_add(by_start).ok()?, end);
    }
    let next = trimmed(clip, clip.place_start, end)?;
    let back = Rational::ZERO.checked_sub(by_start).ok()?;
    if by_start > Rational::ZERO {
        next.trim_start(next.place_start.checked_add(by_start).ok()?)?
            .slide(back)
    } else if by_start < Rational::ZERO {
        next.slide(back)?.extend_start(clip.place_start)
    } else {
        Some(next)
    }
}

/// Give the later pieces of a divided link an id of their own.
///
/// A cut or an overwrite leaves both pieces of a clip carrying its link, so a
/// razor through a picture clip and its sound leaves four clips sharing one
/// id. On each row the first clip of a link keeps it and the second, third
/// and so on take a fresh one each, the same fresh one on every row, so the
/// later halves pair up with each other and with nothing else.
#[frb(ignore)]
fn relink(rows: &mut [Row]) {
    // A link, and the ids its later pieces have been given so far.
    let mut fresh: Vec<(Uuid, Vec<Uuid>)> = Vec::new();
    for row in rows {
        let mut order: Vec<usize> = (0..row.clips.len()).collect();
        order.sort_by_key(|i| row.clips[*i].place_start);
        let mut seen: Vec<(Uuid, usize)> = Vec::new();
        for i in order {
            let Some(link) = row.clips[i].link else {
                continue;
            };
            let nth = match seen.iter_mut().find(|(l, _)| *l == link) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    seen.push((link, 0));
                    continue;
                }
            };
            let at = match fresh.iter().position(|(l, _)| *l == link) {
                Some(at) => at,
                None => {
                    fresh.push((link, Vec::new()));
                    fresh.len() - 1
                }
            };
            let ids = &mut fresh[at].1;
            while ids.len() < nth {
                ids.push(Uuid::now_v7());
            }
            row.clips[i].link = Some(ids[nth - 1]);
        }
    }
}

impl<'a> Edit<'a> {
    fn open(of: &'a CompositionReference) -> Cut<Self> {
        let comp = of.composition()?;
        let mut rows = Vec::new();
        for layer in &comp.layers {
            let LayerKind::Sequence { clips } = &layer.kind else {
                continue;
            };
            let offset = layer.start_offset.0;
            let mut clips = clips.clone();
            for c in &mut clips {
                c.place_start = c.place_start.checked_add(offset)?;
            }
            rows.push(Row {
                id: layer.id,
                audio_only: layer.audio_only,
                locked: layer.switches.locked,
                audible: layer.switches.audible,
                offset,
                clips,
                fresh: None,
            });
        }
        Ok(Edit {
            of,
            comp,
            rows,
            carried: Vec::new(),
        })
    }

    /// The comp time of a frame, or the length of that many frames.
    fn time(&self, frame: i64) -> Cut<Rational> {
        Ok(self.comp.frame_rate.time_of_frame(frame)?.0)
    }

    fn row_of(&self, layer: Uuid) -> Option<usize> {
        self.rows.iter().position(|r| r.id == layer)
    }

    /// Where a clip is: its row, and its place in that row's list.
    fn find(&self, clip: Uuid) -> Cut<(usize, usize)> {
        self.rows
            .iter()
            .enumerate()
            .find_map(|(r, row)| Some((r, row.clips.iter().position(|c| c.id == clip)?)))
            .ok_or(Stop::Failed(BridgeError::InvalidLayer))
    }

    /// The named clips, and with `linked` every clip sharing a link with one
    /// of them. Refused when any of them sits on a locked layer.
    fn chosen(&self, clips: &[Uuid], linked: bool) -> Cut<Vec<Uuid>> {
        let mut ids: Vec<Uuid> = Vec::new();
        let mut links: Vec<Uuid> = Vec::new();
        for id in clips {
            let (r, c) = self.find(*id)?;
            if !ids.contains(id) {
                ids.push(*id);
                links.extend(self.rows[r].clips[c].link);
            }
        }
        for row in &self.rows {
            for c in &row.clips {
                let partner = linked && c.link.is_some_and(|l| links.contains(&l));
                if partner && !ids.contains(&c.id) {
                    ids.push(c.id);
                }
                if row.locked && ids.contains(&c.id) {
                    return Err(BridgeCutResult::Locked.into());
                }
            }
        }
        Ok(ids)
    }

    /// A Sequence layer this edit will add, as a row to work on with the rest.
    fn add_row(&mut self, name: String, audio_only: bool, width: f64, height: f64) -> usize {
        let mut layer = crate::edits::base_layer(
            name,
            LayerKind::Sequence { clips: Vec::new() },
            self.comp.duration.0,
            crate::edits::centred_transform(width, height, self.comp.width, self.comp.height),
        );
        layer.audio_only = audio_only;
        // The sound of what a picture row shows lives in the linked clip on a
        // sound row. The mixer would play the picture clip's own sound as
        // well, so a picture row is made silent and nothing is heard twice.
        layer.switches.audible = audio_only;
        crate::edits::solo_on_arrival(&mut layer, self.comp.layers.iter());
        self.rows.push(Row {
            id: layer.id,
            audio_only,
            locked: false,
            audible: audio_only,
            offset: Rational::ZERO,
            clips: Vec::new(),
            fresh: Some(layer),
        });
        self.rows.len() - 1
    }

    /// The ripple rule. On every unlocked Sequence layer the clips starting
    /// at or after `at` move by `delta`, and a clip that only straddles `at`
    /// stays. Layers of every other kind that start at or after `at` are
    /// carried the same way, keys and all. Refused whole if a moved clip
    /// would land on one that stayed.
    fn ripple(&mut self, at: Rational, delta: Rational) -> Cut {
        for row in self.rows.iter_mut().filter(|r| !r.locked) {
            row.clips =
                sequence::shift_from(&row.clips, at, delta).ok_or(BridgeCutResult::Overlap)?;
        }
        for layer in &mut self.comp.layers {
            let sequence = matches!(layer.kind, LayerKind::Sequence { .. });
            if sequence || layer.switches.locked || layer.in_point.0 < at {
                continue;
            }
            // The layer's own zero moves with its span, so its keyframes and
            // its source stay where they were against its bar.
            layer.in_point = CompTime(layer.in_point.0.checked_add(delta)?);
            layer.out_point = CompTime(layer.out_point.0.checked_add(delta)?);
            layer.start_offset = CompTime(layer.start_offset.0.checked_add(delta)?);
            if !self.carried.contains(&layer.id) {
                self.carried.push(layer.id);
            }
        }
        Ok(())
    }

    /// Cut every clip on `rows` that `at` falls inside, and with `linked` the
    /// clips linked to those on any other unlocked row. Answers how many.
    ///
    /// The same cut `LayerReference::cut_clip_at` makes, through
    /// [`Clip::cut`], gathered here so several rows are one undo step.
    fn razor(&mut self, rows: &[usize], at: Rational, linked: bool) -> Cut<usize> {
        let inside = |c: &Clip| c.place_start < at && at < c.place_end();
        let mut links: Vec<Uuid> = Vec::new();
        if linked {
            for r in rows {
                let under = self.rows[*r].clips.iter().filter(|c| inside(c));
                links.extend(under.filter_map(|c| c.link));
            }
        }
        let mut count = 0;
        for (r, row) in self.rows.iter_mut().enumerate() {
            if row.locked {
                continue;
            }
            let named = rows.contains(&r);
            let mut next = Vec::with_capacity(row.clips.len() + 1);
            for c in &row.clips {
                if inside(c) && (named || c.link.is_some_and(|l| links.contains(&l))) {
                    let (left, right) = c.cut(at).ok_or(BridgeCutResult::Uncuttable)?;
                    next.extend([left, right]);
                    count += 1;
                } else {
                    next.push(c.clone());
                }
            }
            row.clips = next;
        }
        Ok(count)
    }

    /// Write the edit: one `Batch` of every row that changed, every layer a
    /// ripple carried and every layer made. Nothing changed writes nothing.
    fn commit(mut self) -> Cut {
        relink(&mut self.rows);
        let (project, comp) = (self.of.project_id(), self.comp.id);
        let ends = self.rows.iter().flat_map(|row| &row.clips);
        let last = ends.map(Clip::place_end).max();
        let mut ops: Vec<Op> = Vec::new();
        let mut added: Vec<Layer> = Vec::new();
        for row in self.rows {
            // A clip's place is layer time and cannot go negative, so a clip
            // put before the layer's own zero takes the zero back with it, as
            // `LayerReference::slide_clip` does.
            let first = row.clips.iter().map(|c| c.place_start).min();
            let offset = first.map_or(row.offset, |first| first.min(row.offset));
            let mut clips = row.clips;
            for c in &mut clips {
                c.place_start = c.place_start.checked_sub(offset)?;
            }
            if let Some(mut layer) = row.fresh {
                if let Some((start, end)) = sequence::clips_span(&clips) {
                    layer.in_point = CompTime(start);
                    layer.out_point = CompTime(end);
                }
                layer.kind = LayerKind::Sequence { clips };
                added.push(layer);
                continue;
            }
            let unchanged = self.comp.layers.iter().any(|l| {
                l.id == row.id
                    && l.start_offset.0 == offset
                    && matches!(&l.kind, LayerKind::Sequence { clips: was } if *was == clips)
            });
            if !unchanged {
                let layer = LayerReference::new(project, comp, row.id);
                ops.extend(layer.clip_ops(clips, CompTime(offset))?);
            }
        }
        for layer in &self.comp.layers {
            if self.carried.contains(&layer.id) {
                ops.push(Op::SetLayerSpan {
                    comp,
                    layer: layer.id,
                    in_point: layer.in_point,
                    out_point: layer.out_point,
                    start_offset: layer.start_offset,
                });
            }
        }
        // A sound row goes in at the bottom before a picture row goes in on
        // top, so neither index is moved by the other.
        added.sort_by_key(|layer| !layer.audio_only);
        for layer in added {
            ops.push(Op::AddLayer {
                comp,
                index: if layer.audio_only {
                    self.comp.layers.len()
                } else {
                    0
                },
                layer: Box::new(layer),
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        // A composition has a fixed length, and a clip past its end would be
        // out of sight. So it grows to the last clip's end, on to the next
        // whole frame, in the same batch and so the same undo step. It is
        // never shortened, and its work area is left as it stands.
        if let Some(last) = last.filter(|last| *last > self.comp.duration.0) {
            let rate = self.comp.frame_rate;
            let frames = rate.frame_at(CompTime(last));
            let whole = rate.time_of_frame(frames)?.0;
            let end = if whole < last {
                rate.time_of_frame(frames + 1)?.0
            } else {
                whole
            };
            ops.push(Op::SetCompSettings {
                comp,
                name: self.comp.name.clone(),
                width: self.comp.width,
                height: self.comp.height,
                frame_rate: rate,
                duration: Duration(end),
                background: self.comp.background,
            });
        }
        let proj = self.of.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn place(
        mut self,
        target: Option<LayerReference>,
        footage: &FootageReference,
        source_in: BridgeRational,
        source_out: BridgeRational,
        at_frame: i64,
        insert: bool,
        linked: bool,
    ) -> Cut {
        let source_in = Rational::new(source_in.num, source_in.den)?;
        let source_out = Rational::new(source_out.num, source_out.den)?;
        let length = source_out.checked_sub(source_in)?;
        if source_in.is_negative() || length <= Rational::ZERO {
            return Err(BridgeError::InvalidTime.into());
        }
        let at = self.time(at_frame.max(0))?;
        let end = at.checked_add(length)?;
        let item = footage.id();
        let (name, picture, sound, (_, width, height)) = {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let Some(ProjectItem::Footage(f)) = doc.item(item) else {
                return Err(BridgeError::InvalidItem.into());
            };
            (
                f.name.clone(),
                CompositionReference::has_picture(&state, f),
                has_sound(&state, f),
                CompositionReference::footage_span_and_size(&state, f, &self.comp),
            )
        };
        let target = match target {
            Some(layer) => Some(self.row_of(layer.id()).ok_or(BridgeError::NotSequence)?),
            None => None,
        };

        // Dropped on a sound row, a file gives its sound and nothing else.
        let onto_sound = target.is_some_and(|r| self.rows[r].audio_only);
        let picture_row = match target {
            _ if !picture || onto_sound => None,
            Some(r) => Some(r),
            None => Some(self.add_row(name.clone(), false, width, height)),
        };
        // The sound goes down as a clip of its own only where the picture row
        // is silent. A picture row that is audible plays its clips' own sound
        // already, and a second copy on a sound row would be heard twice.
        let wanted = sound
            && match picture_row {
                Some(r) => linked && !self.rows[r].audible,
                None => true,
            };
        let sound_row = match (wanted, target) {
            (false, _) => None,
            (true, Some(r)) if onto_sound => Some(r),
            (true, _) => {
                // The sound row this picture row already keeps its sound on,
                // else the first one with room, else a new one at the bottom.
                let links: Vec<Uuid> = picture_row
                    .into_iter()
                    .flat_map(|r| self.rows[r].clips.iter().filter_map(|c| c.link))
                    .collect();
                let open = |row: &&Row| row.audio_only && !row.locked;
                let paired = |row: &&Row| {
                    row.clips
                        .iter()
                        .any(|c| c.link.is_some_and(|l| links.contains(&l)))
                };
                // An insert opens its own room, so only a clip the point
                // falls inside is in the way.
                let free = |row: &&Row| {
                    !row.clips.iter().any(|c| {
                        if insert {
                            c.place_start < at && at < c.place_end()
                        } else {
                            covers(c, at, end)
                        }
                    })
                };
                let found = self.rows.iter().filter(open).find(paired);
                let found = found.or_else(|| self.rows.iter().filter(open).find(free));
                Some(match found.map(|row| row.id) {
                    Some(id) => self.row_of(id).ok_or(BridgeError::InvalidLayer)?,
                    None => {
                        let (w, h) = (f64::from(self.comp.width), f64::from(self.comp.height));
                        self.add_row(name, true, w, h)
                    }
                })
            }
        };

        let rows: Vec<usize> = picture_row.into_iter().chain(sound_row).collect();
        if rows.is_empty() {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        if rows.iter().any(|r| self.rows[*r].locked) {
            return Err(BridgeCutResult::Locked.into());
        }
        if insert {
            // A clip the point falls inside is cut there, so its later half
            // moves along with everything after it.
            self.razor(&rows, at, linked)?;
            self.ripple(at, length)?;
        }
        let link = (picture_row.is_some() && sound_row.is_some()).then(Uuid::now_v7);
        for r in rows {
            let mut clip = Clip::new(ClipSource::Footage(item), source_in, source_out, at, length);
            clip.link = link;
            let dropped = clip.id;
            let row = &mut self.rows[r];
            row.clips.push(clip);
            row.clips = placed(std::mem::take(&mut row.clips), dropped, false);
        }
        self.commit()
    }

    fn delete(mut self, clips: &[Uuid], ripple: bool, linked: bool) -> Cut {
        let gone = self.chosen(clips, linked)?;
        let mut spans: Vec<(Rational, Rational)> = Vec::new();
        for row in &mut self.rows {
            row.clips.retain(|c| {
                let goes = gone.contains(&c.id);
                if goes {
                    spans.push((c.place_start, c.place_end()));
                }
                !goes
            });
        }
        if ripple {
            // Spans that touch close as one, and the latest closes first so
            // an earlier one is still where it was read.
            spans.sort();
            let mut merged: Vec<(Rational, Rational)> = Vec::new();
            for (start, end) in spans {
                match merged.last_mut() {
                    Some(last) if start <= last.1 => last.1 = last.1.max(end),
                    _ => merged.push((start, end)),
                }
            }
            for (start, end) in merged.into_iter().rev() {
                self.ripple(end, start.checked_sub(end)?)?;
            }
        }
        self.commit()
    }

    fn move_by(
        mut self,
        clips: &[Uuid],
        grabbed: Uuid,
        by_frames: i64,
        target: Option<LayerReference>,
        linked: bool,
    ) -> Cut {
        let delta = self.time(by_frames)?;
        let mut named = clips.to_vec();
        named.push(grabbed);
        let moving = self.chosen(&named, linked)?;
        let (from, _) = self.find(grabbed)?;
        let to = match target {
            Some(layer) => self.row_of(layer.id()).ok_or(BridgeError::NotSequence)?,
            None => from,
        };
        // A picture clip moves among picture rows and sound among sound rows.
        // The clips of the grabbed clip's kind all cross as many rows as it
        // does, and the others, its linked sound say, stay on their own.
        let sound = self.rows[from].audio_only;
        if self.rows[to].audio_only != sound {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        let lane: Vec<usize> = (0..self.rows.len())
            .filter(|r| self.rows[*r].audio_only == sound)
            .collect();
        let place = |r: usize| lane.iter().position(|l| *l == r);

        let mut lifted: Vec<(usize, Clip)> = Vec::new();
        for r in 0..self.rows.len() {
            if !self.rows[r].clips.iter().any(|c| moving.contains(&c.id)) {
                continue;
            }
            let landing = match (place(r), place(from), place(to)) {
                (Some(here), Some(from), Some(to)) => (here + to)
                    .checked_sub(from)
                    .and_then(|i| lane.get(i).copied())
                    .ok_or(BridgeCutResult::Limit)?,
                _ => r,
            };
            if self.rows[landing].locked {
                return Err(BridgeCutResult::Locked.into());
            }
            let (go, stay): (Vec<Clip>, Vec<Clip>) = std::mem::take(&mut self.rows[r].clips)
                .into_iter()
                .partition(|c| moving.contains(&c.id));
            self.rows[r].clips = stay;
            for mut clip in go {
                clip.place_start = clip.place_start.checked_add(delta)?;
                if clip.place_start.is_negative() {
                    return Err(BridgeCutResult::BeforeStart.into());
                }
                lifted.push((landing, clip));
            }
        }
        // Each clip overwrites what it lands on, and never a clip that moved
        // with it: two that overlapped before the move still do after it.
        for (landing, clip) in &lifted {
            let row = &mut self.rows[*landing];
            row.clips.push(clip.clone());
            row.clips = placed(std::mem::take(&mut row.clips), clip.id, false);
            row.clips.retain(|c| c.id != clip.id);
        }
        for (landing, clip) in lifted {
            self.rows[landing].clips.push(clip);
        }
        self.commit()
    }

    fn trim(
        mut self,
        clip: Uuid,
        start_frame: i64,
        end_frame: i64,
        ripple: bool,
        linked: bool,
    ) -> Cut {
        let (r, c) = self.find(clip)?;
        let was = self.rows[r].clips[c].clone();
        let by_start = self.time(start_frame)?.checked_sub(was.place_start)?;
        let by_end = self.time(end_frame)?.checked_sub(was.place_end())?;
        for id in self.chosen(&[clip], linked)? {
            let (r, c) = self.find(id)?;
            let row = &mut self.rows[r];
            let next =
                retrimmed(&row.clips[c], by_start, by_end, ripple).ok_or(BridgeCutResult::Limit)?;
            // A picture row shows one clip at a time, so an edge is not
            // pulled over a neighbour. A ripple moves the neighbour instead.
            let (start, end) = (next.place_start, next.place_end());
            let over = |o: &Clip| o.id != id && covers(o, start, end);
            if !ripple && !row.audio_only && row.clips.iter().any(over) {
                return Err(BridgeCutResult::Overlap.into());
            }
            row.clips[c] = next;
        }
        if ripple {
            // Everything after the clip's old end follows its change of
            // length, whichever edge made it.
            self.ripple(was.place_end(), by_end.checked_sub(by_start)?)?;
        }
        self.commit()
    }

    fn roll(mut self, clip: Uuid, end_edge: bool, to_frame: i64, linked: bool) -> Cut {
        let to = self.time(to_frame)?;
        let (r, c) = self.find(clip)?;
        let row = &self.rows[r];
        let point = if end_edge {
            row.clips[c].place_end()
        } else {
            row.clips[c].place_start
        };
        // The clip across the edit point from this one.
        let meets = |o: &&Clip| {
            o.id != clip
                && if end_edge {
                    o.place_start == point
                } else {
                    o.place_end() == point
                }
        };
        let other = row.clips.iter().find(meets);
        let other = other.ok_or(BridgeCutResult::Nothing)?;
        let (left, right) = if end_edge {
            (&row.clips[c], other)
        } else {
            (other, &row.clips[c])
        };
        let (left, left_link, right, right_link) = (left.id, left.link, right.id, right.link);

        let partners = self.chosen(&[left, right], linked)?;
        self.rows[r].clips =
            sequence::roll(&self.rows[r].clips, left, right, to).ok_or(BridgeCutResult::Limit)?;
        // A linked clip whose own edge sits on the same edit point goes with
        // it: the left clip's by its end, the right clip's by its start. One
        // that was cut somewhere else keeps its own edit point.
        for id in partners
            .into_iter()
            .filter(|id| *id != left && *id != right)
        {
            let (r, c) = self.find(id)?;
            let p = &self.rows[r].clips[c];
            let next = if p.link == left_link && p.place_end() == point {
                trimmed(p, p.place_start, to)
            } else if p.link == right_link && p.place_start == point {
                trimmed(p, to, p.place_end())
            } else {
                continue;
            };
            self.rows[r].clips[c] = next.ok_or(BridgeCutResult::Limit)?;
        }
        self.commit()
    }

    fn slip(mut self, clip: Uuid, by_frames: i64, linked: bool) -> Cut {
        let delta = self.time(by_frames)?;
        for id in self.chosen(&[clip], linked)? {
            let (r, c) = self.find(id)?;
            let slot = &mut self.rows[r].clips[c];
            *slot = slot.slip(delta).ok_or(BridgeCutResult::Limit)?;
        }
        self.commit()
    }

    fn slide(mut self, clip: Uuid, by_frames: i64, linked: bool) -> Cut {
        let delta = self.time(by_frames)?;
        for id in self.chosen(&[clip], linked)? {
            let (r, _) = self.find(id)?;
            self.rows[r].clips = sequence::slide_between(&self.rows[r].clips, id, delta)
                .ok_or(BridgeCutResult::Limit)?;
        }
        self.commit()
    }

    fn close_gap(mut self, layer: &LayerReference, at_frame: i64) -> Cut {
        let at = self.time(at_frame)?;
        let r = self.row_of(layer.id()).ok_or(BridgeError::NotSequence)?;
        let row = &self.rows[r];
        if row.locked {
            return Err(BridgeCutResult::Locked.into());
        }
        if row
            .clips
            .iter()
            .any(|c| c.place_start <= at && at < c.place_end())
        {
            return Err(BridgeCutResult::Nothing.into());
        }
        // The gap runs from the last clip to end before `at`, or the start of
        // the composition, to the first clip to start after it.
        let ends = row.clips.iter().map(Clip::place_end);
        let start = ends.filter(|e| *e <= at).max().unwrap_or(Rational::ZERO);
        let starts = row.clips.iter().map(|c| c.place_start);
        let end = starts.filter(|s| *s > at).min();
        let end = end.ok_or(BridgeCutResult::Nothing)?;
        self.ripple(end, start.checked_sub(end)?)?;
        self.commit()
    }

    fn razor_at(mut self, layers: &[LayerReference], at_frame: i64, linked: bool) -> Cut {
        let at = self.time(at_frame)?;
        let rows: Vec<usize> = if layers.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            layers.iter().filter_map(|l| self.row_of(l.id())).collect()
        };
        if self.razor(&rows, at, linked)? == 0 {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }
}

impl CompositionReference {
    /// Put a span of a footage item down at `at_frame`: the Cut workspace's
    /// insert and overwrite, and the drop from its Source viewer.
    ///
    /// `source_in` and `source_out` are the marked span in seconds of source
    /// time. `target` is the Sequence layer to place on, or `None` for a new
    /// picture Sequence layer at the top of the stack. With `insert` the
    /// clips at or after `at_frame` on every unlocked Sequence layer move
    /// later to make room, a clip on the target that the point falls inside
    /// is cut there first, and layers of other kinds that start at or after
    /// the point move too. Without it the clip overwrites what it lands on.
    ///
    /// **Picture and sound are two linked clips.** With `linked`, a file
    /// holding both also puts its sound down as a clip on an audio-only
    /// Sequence layer: the one this picture layer's clips are already linked
    /// to, else the first with room, else a new one at the bottom of the
    /// stack. A picture layer made here has its audible switch off, so the
    /// sound is heard from the linked clip alone. An existing picture layer
    /// whose audible switch is on already plays its clips' own sound, so it
    /// takes the picture clip and no sound clip is made. A file with no
    /// picture, or any file placed on an audio-only target, puts down its
    /// sound alone.
    #[allow(clippy::too_many_arguments)]
    #[frb(sync)]
    pub fn cut_place(
        &self,
        target: Option<LayerReference>,
        footage: &FootageReference,
        source_in: BridgeRational,
        source_out: BridgeRational,
        at_frame: i64,
        insert: bool,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| {
            edit.place(
                target, footage, source_in, source_out, at_frame, insert, linked,
            )
        })
    }

    /// Delete clips, on any Sequence layers of this composition. With
    /// `linked` the clips linked to them go too. With `ripple` what they
    /// leave is closed: everything after each span moves earlier by its
    /// length, on every unlocked Sequence layer. Without it they leave gaps.
    #[frb(sync)]
    pub fn cut_delete(
        &self,
        clips: Vec<Uuid>,
        ripple: bool,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.delete(&clips, ripple, linked))
    }

    /// Move clips by `by_frames`, overwriting what they land on.
    ///
    /// `grabbed` is the clip under the pointer and `target` the Sequence
    /// layer it was dropped on, or `None` to stay on its own. The other
    /// clips of its kind, picture or sound, cross as many layers of that
    /// kind as it does, and clips of the other kind keep their layer. With
    /// `linked` the clips linked to any of them move too.
    #[frb(sync)]
    pub fn cut_move(
        &self,
        clips: Vec<Uuid>,
        grabbed: Uuid,
        by_frames: i64,
        target: Option<LayerReference>,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| {
            edit.move_by(&clips, grabbed, by_frames, target, linked)
        })
    }

    /// Trim a clip so its edges land on `start_frame` and `end_frame`, as
    /// `LayerReference::trim_clip` does: an edge moving inward crops, one
    /// moving outward carries the clip on.
    ///
    /// With `ripple` everything after the clip follows its change of length,
    /// and a trimmed head leaves the clip starting where it did. Without it
    /// nothing else moves, and on a picture layer an edge is refused where it
    /// would cover a neighbour. With `linked` the clips linked to it take the
    /// same change at the same edges.
    #[frb(sync)]
    pub fn cut_trim(
        &self,
        clip: Uuid,
        start_frame: i64,
        end_frame: i64,
        ripple: bool,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| {
            edit.trim(clip, start_frame, end_frame, ripple, linked)
        })
    }

    /// Roll an edit point to `to_frame`: the clip on one side trims as the
    /// clip on the other extends, and nothing else moves.
    ///
    /// The edit point is the end of `clip` when `end_edge` is set, and its
    /// start otherwise. With `linked`, a linked clip whose own edge sits on
    /// the same edit point goes with it.
    #[frb(sync)]
    pub fn cut_roll(
        &self,
        clip: Uuid,
        end_edge: bool,
        to_frame: i64,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.roll(clip, end_edge, to_frame, linked))
    }

    /// Slip a clip: it keeps its place and its length, and shows source
    /// `by_frames` later, or earlier for a negative count. The count is in
    /// this composition's frames. With `linked` its linked clips slip too.
    #[frb(sync)]
    pub fn cut_slip(
        &self,
        clip: Uuid,
        by_frames: i64,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.slip(clip, by_frames, linked))
    }

    /// Slide a clip between its neighbours by `by_frames`: it keeps its
    /// length and its frames, and the clips either side trim and extend to
    /// stay against it. With `linked` its linked clips slide the same way on
    /// their own layers.
    #[frb(sync)]
    pub fn cut_slide(
        &self,
        clip: Uuid,
        by_frames: i64,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.slide(clip, by_frames, linked))
    }

    /// Close the gap on `layer` that `at_frame` falls in: everything after
    /// it moves earlier by its length, on every unlocked Sequence layer.
    #[frb(sync)]
    pub fn cut_close_gap(
        &self,
        layer: &LayerReference,
        at_frame: i64,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.close_gap(layer, at_frame))
    }

    /// Razor: cut every clip under `at_frame` on `layers`, or on every
    /// unlocked Sequence layer when the list is empty. With `linked` a clip
    /// linked to one that was cut is cut too, and the later halves are
    /// linked to each other and no longer to the earlier ones.
    #[frb(sync)]
    pub fn cut_razor(
        &self,
        layers: Vec<LayerReference>,
        at_frame: i64,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.razor_at(&layers, at_frame, linked))
    }
}

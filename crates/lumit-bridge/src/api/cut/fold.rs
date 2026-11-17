//! A clip or a selection of clips made into a composition that one clip then
//! plays.

use super::*;
use lumit_core::model::{Document, Switches};

/// A clip that plays the whole of composition `id`, `length` long, at `at`.
#[frb(ignore)]
fn playing(id: Uuid, length: Rational, at: Rational) -> Clip {
    Clip::new(ClipSource::Comp(id), Rational::ZERO, length, at, length)
}

/// Whether a composition-sized picture placed on this row lands exactly as it
/// does in the composition, with nothing on the row to act on it a second time.
#[frb(ignore)]
fn plain_row(layer: &Layer, comp: &Composition) -> bool {
    let placed = crate::edits::centred_transform(
        f64::from(comp.width),
        f64::from(comp.height),
        comp.width,
        comp.height,
    );
    layer.transform == placed
        && layer.effects.is_empty()
        && layer.masks.is_empty()
        && layer.matte.is_none()
        && layer.parent.is_none()
        && layer.blend == lumit_core::model::BlendMode::Normal
        && !layer.switches.three_d
}

impl Edit<'_> {
    /// The rows of the composition being made, one for each row holding a
    /// chosen clip, top first: a copy of that row with fresh ids, so its own
    /// effects, masks and keys still act on the clips as they did, its clips
    /// the chosen ones and its zero moved so they fall from the composition's
    /// start.
    fn folded_rows(&self, chosen: &[Uuid], from: Rational, doc: &Document) -> Cut<Vec<Layer>> {
        let mut inner = Vec::new();
        for row in &self.rows {
            let clips: Vec<Clip> = row
                .clips
                .iter()
                .filter(|c| chosen.contains(&c.id))
                .map(|c| {
                    let mut clip = c.clone();
                    clip.place_start = c.place_start.checked_sub(row.offset)?;
                    Ok(clip)
                })
                .collect::<Result<_, TimeError>>()?;
            let Some(layer) = self.comp.layers.iter().find(|l| l.id == row.id) else {
                continue;
            };
            let Some((start, end)) = sequence::clips_span(&clips) else {
                continue;
            };
            let zero = row.offset.checked_sub(from)?;
            let mut copy = layer.clone();
            copy.id = Uuid::now_v7();
            for effect in &mut copy.effects {
                effect.id = Uuid::now_v7();
            }
            lumit_core::grade::versions::respawn(&mut copy.grade_versions);
            lumit_render::passes::follow_copy(doc, layer, &mut copy);
            copy.parent = None;
            copy.matte = None;
            copy.start_offset = CompTime(zero);
            copy.in_point = CompTime(zero.checked_add(start)?);
            copy.out_point = CompTime(zero.checked_add(end)?);
            copy.kind = LayerKind::Sequence { clips };
            copy.switches = Switches {
                locked: false,
                ..copy.switches
            };
            inner.push(copy);
        }
        Ok(inner)
    }

    /// Move the clips named, and with `linked` the clips linked to them, into
    /// a new composition called `name`, on the same rows and in the same
    /// places, and put one clip playing it across the span they took. It goes
    /// on the first of their rows when nothing there would act on it twice and
    /// nothing else stands in its way, and on a new row at the top otherwise,
    /// so the picture is what it was. A row left with nothing on it goes.
    pub(super) fn fold(
        mut self,
        clips: &[Uuid],
        linked: bool,
        name: String,
        made: &mut Option<Uuid>,
    ) -> Cut {
        let chosen = self.chosen(clips, linked)?;
        let all = self.rows.iter().flat_map(|row| &row.clips);
        let picked: Vec<&Clip> = all.filter(|c| chosen.contains(&c.id)).collect();
        let (Some(from), Some(to)) = (
            picked.iter().map(|c| c.place_start).min(),
            picked.iter().map(|c| c.place_end()).max(),
        ) else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let length = self.time(self.frame_up(to.checked_sub(from)?)?)?;
        let (id, ops) = {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let inner = self.folded_rows(&chosen, from, &doc)?;
            let rate = self.comp.frame_rate;
            let settings = BridgeCompSettings {
                width: self.comp.width,
                height: self.comp.height,
                fps_num: rate.num(),
                fps_den: rate.den(),
                duration: BridgeRational {
                    num: length.num(),
                    den: length.den(),
                },
                ..BridgeCompSettings::defaults()
            };
            let (id, mut ops) = ProjectReference::new_comp_ops(&doc, name, Some(settings), None)?;
            for op in &mut ops {
                if let Op::AddItem { item, .. } = op {
                    if let ProjectItem::Composition(made) = item.as_mut() {
                        made.layers.clone_from(&inner);
                    }
                }
            }
            (id, ops)
        };
        self.first = ops;
        // The row it goes on: the first with a chosen picture, or sound when
        // no picture was chosen. Picture first, so sound comes along with it.
        let holds = |row: &Row| row.clips.iter().any(|c| chosen.contains(&c.id));
        let sound_only = !self.rows.iter().any(|row| !row.audio_only && holds(row));
        let first = self
            .rows
            .iter()
            .position(|row| holds(row) && row.audio_only == sound_only);
        for row in &mut self.rows {
            row.clips.retain(|c| !chosen.contains(&c.id));
        }
        let target = first.filter(|r| {
            let row = &self.rows[*r];
            let open = !row.clips.iter().any(|c| covers(c, from, to));
            let plain = self
                .comp
                .layers
                .iter()
                .find(|l| l.id == row.id)
                .is_some_and(|l| plain_row(l, &self.comp));
            open && plain
        });
        let r = match target {
            Some(r) => r,
            None => {
                let (w, h) = (f64::from(self.comp.width), f64::from(self.comp.height));
                let row = self.add_row(None, sound_only, w, h);
                self.rows[row].at = first.unwrap_or(0).min(self.comp.layers.len());
                row
            }
        };
        self.hear(r);
        self.rows[r].clips.push(playing(id, length, from));
        // What is left of the rows the clips came off.
        let leaning = |id: Uuid| {
            self.comp
                .layers
                .iter()
                .any(|l| l.parent == Some(id) || l.matte.as_ref().is_some_and(|m| m.layer == id))
        };
        let empty: Vec<Uuid> = self
            .rows
            .iter()
            .filter(|row| row.clips.is_empty() && row.fresh.is_none() && !leaning(row.id))
            .filter(|row| first.is_some_and(|f| self.rows[f].id == row.id) || row.audio_only)
            .map(|row| row.id)
            .collect();
        self.gone.extend(empty);
        *made = Some(id);
        self.commit()
    }

    /// Point a clip at a composition holding its footage, made here, and
    /// answer which. A clip that already plays a composition answers that
    /// one and changes nothing.
    pub(super) fn make_composition(mut self, clip: Uuid, made: &mut Option<Uuid>) -> Cut {
        let (r, c) = self.find(clip)?;
        let this = self.rows[r].clips[c].clone();
        let item = match this.source {
            ClipSource::Comp(id) => {
                *made = Some(id);
                return Ok(());
            }
            ClipSource::Footage(item) => item,
            // A colour has nothing inside it to open.
            ClipSource::Solid(_) => return Err(BridgeCutResult::Nothing.into()),
        };
        if self.rows[r].locked {
            return Err(BridgeCutResult::Locked.into());
        }
        let (id, ops) = {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let Some(ProjectItem::Footage(f)) = doc.item(item) else {
                return Err(BridgeError::InvalidItem.into());
            };
            // The file's whole length and its own size. A still has no
            // length and a sound file no size, and each takes this
            // composition's. Never shorter than what the clip plays, which
            // a still held past the composition's end would be.
            let (length, width, height) =
                CompositionReference::footage_span_and_size(&state, f, &self.comp);
            let length = length.max(self.time(self.frame_up(this.source_out)?)?);
            let rate = self.comp.frame_rate;
            let settings = BridgeCompSettings {
                width: width as u32,
                height: height as u32,
                fps_num: rate.num(),
                fps_den: rate.den(),
                duration: BridgeRational {
                    num: length.num(),
                    den: length.den(),
                },
                ..BridgeCompSettings::defaults()
            };
            let (id, mut ops) =
                ProjectReference::new_comp_ops(&doc, f.name.clone(), Some(settings), None)?;
            for op in &mut ops {
                let Op::AddItem { item: added, .. } = op else {
                    continue;
                };
                let ProjectItem::Composition(inner) = added.as_mut() else {
                    continue;
                };
                let mut layer = crate::edits::base_layer(
                    f.name.clone(),
                    LayerKind::Footage { item },
                    length,
                    crate::edits::centred_transform(width, height, inner.width, inner.height),
                );
                layer.audio_only = !CompositionReference::has_picture(&state, f);
                // The composition sounds the streams the clip did.
                layer.audio_streams = this.audio_streams.clone();
                inner.layers = vec![layer];
                // Composition time is source time in there, so the work
                // area is the stretch the clip plays.
                let start = this.source_in.max(Rational::ZERO);
                if start < this.source_out {
                    inner.work_area = Some((CompTime(start), CompTime(this.source_out)));
                }
            }
            (id, ops)
        };
        self.first = ops;
        self.rows[r].clips[c].source = ClipSource::Comp(id);
        self.rows[r].clips[c].audio_streams = AudioStreams::default();
        *made = Some(id);
        self.commit()
    }
}

/// [`Edit::fold`] as one edit on `of`: the composition made, or `None` when the
/// edit was refused.
#[frb(ignore)]
pub(crate) fn fold_clips(
    of: &CompositionReference,
    clips: &[Uuid],
    linked: bool,
    name: String,
) -> Result<Option<Uuid>, BridgeError> {
    let mut made = None;
    let result = cut(of, |edit| edit.fold(clips, linked, name, &mut made))?;
    Ok(made.filter(|_| result == BridgeCutResult::Done))
}

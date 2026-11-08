//! What an overlap on a picture row is: which Transition effect the incoming
//! clip names, set on purpose or taken by an overlap a drag has just made.

use super::*;
use lumit_core::model::EffectInstance;
use lumit_core::sequence::transition::KINDS;

/// A Sequence layer's clips as the document holds them, for comparing with what
/// an edit is about to write.
#[frb(ignore)]
fn stored<'c>(comp: &'c Composition, row: Uuid) -> &'c [Clip] {
    comp.layers
        .iter()
        .find(|l| l.id == row)
        .and_then(|l| match &l.kind {
            LayerKind::Sequence { clips } => Some(clips.as_slice()),
            _ => None,
        })
        .unwrap_or(&[])
}

/// The clip `clip` overlaps across its start on the row: the one running into it.
#[frb(ignore)]
fn running_into<'c>(clips: &'c [Clip], clip: &Clip) -> Option<&'c Clip> {
    clips
        .iter()
        .find(|o| o.id != clip.id && follows(o, clip) && clip.place_start < o.place_end())
}

/// An overlap an edit has made or kept: its two clips, outgoing first, their
/// links, and the span it covered before the edit and covers now.
#[frb(ignore)]
struct Span {
    ids: (Uuid, Uuid),
    links: (Option<Uuid>, Option<Uuid>),
    was: (Rational, Rational),
    now: (Rational, Rational),
}

#[frb(ignore)]
impl Span {
    /// The overlap of `outgoing` and `incoming` as they were and as `now`
    /// holds them.
    fn of(outgoing: &Clip, incoming: &Clip, now: &[Clip]) -> Option<Self> {
        let out = now.iter().find(|c| c.id == outgoing.id)?;
        let inc = now.iter().find(|c| c.id == incoming.id)?;
        Some(Span {
            ids: (outgoing.id, incoming.id),
            links: (outgoing.link, incoming.link),
            was: (incoming.place_start, outgoing.place_end()),
            now: (inc.place_start, out.place_end()),
        })
    }
}

impl Edit<'_> {
    /// Fade an edge with nothing across it. When its linked clip has an
    /// overlap on another row and the edge sat inside it, the edge goes where
    /// the overlap now ends, as far as its media and the next clip allow, and
    /// the fade spans the overlap. Otherwise it fades on its own for `length`.
    fn fade_alone(&mut self, id: Uuid, end_edge: bool, length: Rational, spans: &[Span]) -> Cut {
        let (r, c) = self.find(id)?;
        let this = self.rows[r].clips[c].clone();
        let (clip, seconds) = self
            .following(&this, end_edge, spans)?
            .unwrap_or((this, length));
        let slot = &mut self.rows[r].clips[c];
        *slot = clip;
        let seconds = seconds.max(Rational::ZERO).min(slot.place_duration);
        if end_edge {
            slot.fade_out.seconds = seconds;
        } else {
            slot.fade_in.seconds = seconds;
        }
        Ok(())
    }

    /// `this` with its edge carried to where its linked clip's overlap now
    /// ends, and how long the fade across that overlap is, or `None` when no
    /// overlap held the edge or the edge cannot go there.
    fn following(
        &self,
        this: &Clip,
        end_edge: bool,
        spans: &[Span],
    ) -> Cut<Option<(Clip, Rational)>> {
        let id = this.id;
        let edge = if end_edge {
            this.place_end()
        } else {
            this.place_start
        };
        let held = spans.iter().find(|s| {
            let (link, other) = if end_edge {
                (s.links.0, s.ids.0)
            } else {
                (s.links.1, s.ids.1)
            };
            this.link.is_some() && link == this.link && other != id
        });
        let Some(span) = held.filter(|s| s.was.0 <= edge && edge <= s.was.1) else {
            return Ok(None);
        };
        let (r, _) = self.find(id)?;
        let clips = &self.rows[r].clips;
        let mut target = if end_edge { span.now.1 } else { span.now.0 };
        if end_edge && target > edge {
            let next = clips.iter().filter(|o| o.id != id && o.place_start >= edge);
            target = next.map(|o| o.place_start).fold(target, Rational::min);
            if let Some(room) = self.room(this, false)? {
                target = target.min(edge.checked_add(room)?);
            }
        } else if !end_edge && target < edge {
            let before = clips.iter().filter(|o| o.id != id && o.place_end() <= edge);
            target = before.map(Clip::place_end).fold(target, Rational::max);
            if let Some(room) = self.room(this, true)? {
                target = target.max(edge.checked_sub(room)?);
            }
        }
        let (start, end) = if end_edge {
            (this.place_start, target)
        } else {
            (target, this.place_end())
        };
        let Some(next) = trimmed(this, start, end) else {
            return Ok(None);
        };
        let across = if end_edge {
            target.checked_sub(span.now.0)?
        } else {
            span.now.1.checked_sub(target)?
        };
        Ok(Some((next, across)))
    }

    /// The clips an edit on one edge of `clip` reaches, each with the edge it
    /// is on: `clip`, and with `linked` the clips linked to it on the same
    /// edge and the clips linked to the clip across the edit point on the
    /// other, so a picture's sound follows wherever the picture goes.
    fn edges(&self, clip: Uuid, end_edge: bool, linked: bool) -> Cut<Vec<(Uuid, bool)>> {
        let mut edges: Vec<(Uuid, bool)> = self
            .chosen(&[clip], linked)?
            .into_iter()
            .map(|id| (id, end_edge))
            .collect();
        if linked {
            let (r, c) = self.find(clip)?;
            let this = &self.rows[r].clips[c];
            let meets = |o: &&Clip| {
                o.id != clip
                    && if end_edge {
                        follows(this, o)
                    } else {
                        follows(o, this)
                    }
            };
            if let Some(other) = self.rows[r].clips.iter().find(meets).map(|o| o.id) {
                for id in self.chosen(&[other], true)? {
                    if id != other && !edges.iter().any(|(e, _)| *e == id) {
                        edges.push((id, !end_edge));
                    }
                }
            }
        }
        Ok(edges)
    }

    /// A transition on one edge of `clip`: a dissolve with the clip across
    /// it, or a fade where nothing is. With `linked` the clips linked to the
    /// clip across take the other edge, so a picture's sound crossfades where
    /// the picture dissolves even when only one side has a sound clip.
    pub(super) fn transition(
        mut self,
        clip: Uuid,
        end_edge: bool,
        frames: i64,
        linked: bool,
    ) -> Cut {
        let frames = frames.max(0);
        let (frame, length) = (self.time(1)?, self.time(frames)?);
        let edges = self.edges(clip, end_edge, linked)?;
        // Both clips of an overlap made already, so its pair isn't made twice.
        let mut done: Vec<Uuid> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        // Edges with nothing across, faded once every overlap is made, so a
        // linked sound can follow its picture's.
        let mut alone: Vec<(Uuid, bool)> = Vec::new();
        for (id, end_edge) in edges {
            if done.contains(&id) {
                continue;
            }
            let (r, c) = self.find(id)?;
            let row = &self.rows[r];
            let this = &row.clips[c];
            // The clip across the edit point: one this edge meets, or
            // already runs into.
            let across = |o: &&Clip| {
                o.id != id
                    && if end_edge {
                        follows(this, o)
                    } else {
                        follows(o, this)
                    }
            };
            let Some(other) = row.clips.iter().find(across) else {
                alone.push((id, end_edge));
                continue;
            };
            let (outgoing, incoming) = if end_edge {
                (this, other)
            } else {
                (other, this)
            };
            let room = (self.room(outgoing, false)?, self.room(incoming, true)?);
            done.extend([outgoing.id, incoming.id]);
            let coming = incoming;
            let incoming = incoming.id;
            let mut next = sequence::transition::dissolve(
                &row.clips,
                outgoing.id,
                incoming,
                frames,
                frame,
                room,
            )
            .ok_or(BridgeCutResult::Limit)?;
            // A picture dissolves in a straight line. A crossfade keeps the
            // shape it has, which holds the level.
            if !row.audio_only && frames > 0 {
                for clip in next.iter_mut().filter(|c| c.id == incoming) {
                    clip.fade_in.shape = FadeShape::Linear;
                }
            }
            spans.extend(Span::of(outgoing, coming, &next));
            self.rows[r].clips = next;
        }
        for (id, end_edge) in alone {
            self.fade_alone(id, end_edge, length, &spans)?;
        }
        self.commit()
    }

    /// A fresh instance of the kind named, sized for `clip`'s row, or `None`
    /// for the dissolve. `Limit` for a name that isn't a kind.
    fn fresh_kind(&self, row: Uuid, clip: Uuid, kind: Option<&str>) -> Cut<Option<EffectInstance>> {
        let Some(kind) = kind else {
            return Ok(None);
        };
        if !KINDS.contains(&kind) {
            return Err(BridgeCutResult::Limit.into());
        }
        let layer = LayerReference::new(self.of.project_id(), self.comp.id, row);
        let (width, height) = layer.clip_picture_size(clip)?;
        Ok(lumit_core::fx::builtins::instantiate_for_raster(
            kind, width, height,
        ))
    }

    /// Give the overlap at this edge of `clip` the kind named, and with
    /// `linked` the clips linked to it the same on their own picture rows. An
    /// edit point that only meets gets an overlap of `frames` first, and a
    /// sound row takes its crossfade and no kind.
    pub(super) fn set_transition(
        mut self,
        clip: Uuid,
        end_edge: bool,
        frames: i64,
        kind: Option<&str>,
        linked: bool,
    ) -> Cut {
        let frames = frames.max(0);
        let mut spans: Vec<Span> = Vec::new();
        let mut alone: Vec<(Uuid, bool)> = Vec::new();
        for (id, end_edge) in self.edges(clip, end_edge, linked)? {
            let (r, c) = self.find(id)?;
            let this = self.rows[r].clips[c].clone();
            let found = if end_edge {
                self.rows[r]
                    .clips
                    .iter()
                    .find(|o| o.id != id && follows(&this, o))
            } else {
                self.rows[r]
                    .clips
                    .iter()
                    .find(|o| o.id != id && follows(o, &this))
            };
            let Some(other) = found.cloned() else {
                if self.rows[r].audio_only && id != clip {
                    alone.push((id, end_edge));
                }
                continue;
            };
            let (outgoing, incoming) = if end_edge {
                (id, other.id)
            } else {
                (other.id, id)
            };
            // Only an edit point that merely meets has any length to find.
            let (start, end) = if end_edge {
                (other.place_start, this.place_end())
            } else {
                (this.place_start, other.place_end())
            };
            if start >= end {
                let frame = self.time(1)?;
                let (going, coming) = if end_edge {
                    (&this, &other)
                } else {
                    (&other, &this)
                };
                let room = (self.room(going, false)?, self.room(coming, true)?);
                self.rows[r].clips = sequence::transition::dissolve(
                    &self.rows[r].clips,
                    outgoing,
                    incoming,
                    frames,
                    frame,
                    room,
                )
                .ok_or(BridgeCutResult::Limit)?;
            }
            let (going, coming) = if end_edge {
                (&this, &other)
            } else {
                (&other, &this)
            };
            spans.extend(Span::of(going, coming, &self.rows[r].clips));
            if self.rows[r].audio_only {
                continue;
            }
            let row = self.rows[r].id;
            let fresh = self.fresh_kind(row, incoming, kind)?;
            for clip in self.rows[r].clips.iter_mut().filter(|c| c.id == incoming) {
                // Asked for what it already is, it keeps the rows it was given.
                let same = clip
                    .transition
                    .as_ref()
                    .map(|t| t.effect.match_name.as_str())
                    == kind;
                if !same {
                    clip.transition = fresh.clone();
                }
                clip.fade_in.shape = FadeShape::Linear;
            }
        }
        // A linked sound with nothing across fades where the picture
        // dissolves.
        let length = self.time(frames)?;
        for (id, end_edge) in alone {
            self.fade_alone(id, end_edge, length, &spans)?;
        }
        self.commit()
    }

    /// Clear the kind from any clip no longer overlapped at its start, and give
    /// [`Edit::arriving`]'s kind to any overlap this edit has just made. Called
    /// as the edit commits.
    pub(super) fn settle_transitions(&mut self) -> Cut {
        let arriving = self.arriving.clone();
        for r in 0..self.rows.len() {
            if self.rows[r].audio_only {
                continue;
            }
            let row = self.rows[r].id;
            let was = stored(&self.comp, row);
            let mut kinds: Vec<(usize, Option<EffectInstance>)> = Vec::new();
            for (at, clip) in self.rows[r].clips.iter().enumerate() {
                let Some(other) = running_into(&self.rows[r].clips, clip) else {
                    if clip.transition.is_some() {
                        kinds.push((at, None));
                    }
                    continue;
                };
                let Some(kind) = &arriving else {
                    continue;
                };
                let held = was.iter().find(|c| c.id == clip.id);
                let before = held.and_then(|c| running_into(was, c));
                if before.is_some_and(|b| b.id == other.id) {
                    continue;
                }
                kinds.push((at, self.fresh_kind(row, clip.id, kind.as_deref())?));
            }
            for (at, kind) in kinds {
                let clip = &mut self.rows[r].clips[at];
                if kind.is_some() {
                    clip.fade_in.shape = FadeShape::Linear;
                }
                clip.transition = kind;
            }
        }
        Ok(())
    }
}

/// [`Edit::set_transition`] as one edit on `of`.
#[frb(ignore)]
pub(crate) fn set_kind(
    of: &CompositionReference,
    clip: Uuid,
    end_edge: bool,
    frames: i64,
    kind: Option<&str>,
    linked: bool,
) -> Result<BridgeCutResult, BridgeError> {
    cut(of, |edit| {
        edit.set_transition(clip, end_edge, frames, kind, linked)
    })
}

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

impl Edit<'_> {
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
        for id in self.chosen(&[clip], linked)? {
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

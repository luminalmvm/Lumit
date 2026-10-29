//! A clip's takes in the Cut workspace (`lumit_core::sequence::takes`):
//! adding one, showing one, stepping through them and taking one away.

use super::*;

/// Whether two clips play the same thing and hold takes of the same things,
/// so one can take the same turn as the other.
#[frb(ignore)]
fn in_step(clip: &Clip, other: &Clip) -> bool {
    let sources = |c: &Clip| c.takes.iter().map(|t| t.source).collect::<Vec<_>>();
    clip.source == other.source && sources(clip) == sources(other)
}

impl Edit<'_> {
    /// Each named clip as `turn` leaves it, and each clip linked to it that
    /// plays the same media, so a picture clip's sound clip goes with it.
    /// `Nothing` when no clip changed.
    pub(super) fn turn_takes(
        mut self,
        clips: &[Uuid],
        turn: impl Fn(&Clip) -> Option<Clip>,
    ) -> Cut {
        let mut done: Vec<Uuid> = Vec::new();
        for id in clips {
            if done.contains(id) {
                continue;
            }
            let (r, c) = self.find(*id)?;
            let clip = self.rows[r].clips[c].clone();
            let mut at = vec![(r, c)];
            for (r, row) in self.rows.iter().enumerate() {
                for (c, other) in row.clips.iter().enumerate() {
                    let linked =
                        other.id != clip.id && other.link.is_some() && other.link == clip.link;
                    if linked && in_step(&clip, other) {
                        at.push((r, c));
                    }
                }
            }
            for (r, c) in at {
                let Some(next) = turn(&self.rows[r].clips[c]) else {
                    continue;
                };
                if self.rows[r].locked {
                    return Err(BridgeCutResult::Locked.into());
                }
                done.push(next.id);
                self.rows[r].clips[c] = next;
            }
        }
        if done.is_empty() {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// Add a footage item to a clip as one more take, from its marked In or
    /// its start, and show it.
    pub(super) fn add_take(self, clip: Uuid, footage: &FootageReference) -> Cut {
        let item = footage.id();
        let source_in = {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            if !matches!(doc.item(item), Some(ProjectItem::Footage(_))) {
                return Err(BridgeError::InvalidItem.into());
            }
            let marked = doc.footage_trims.get(&item).and_then(|t| t.in_point);
            marked.unwrap_or(Rational::ZERO)
        };
        self.turn_takes(&[clip], |c| {
            c.with_take(ClipSource::Footage(item), source_in)
        })
    }
}

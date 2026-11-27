//! A frame the Viewer asked for that needs more video memory than Lumit may
//! use, said once, with whether an undo would put it right.

use super::*;

/// What the worker remembers between frames to say it once.
#[frb(ignore)]
#[derive(Default)]
pub(super) struct Refusals {
    /// Frames the Viewer has put on screen.
    pub(super) shown: u64,
    /// The revision and composition of the last frame asked for, if it drew.
    drawn: Option<(u64, Uuid)>,
    /// The composition whose frames are not drawing, until one draws.
    stuck: Option<Uuid>,
    /// The revision and composition last told about, so a frame asked for
    /// again says nothing new.
    told: Option<(u64, Uuid)>,
}

impl Refusals {
    fn drew(&mut self, revision: u64, comp: Uuid) {
        self.drawn = Some((revision, comp));
        self.stuck = None;
    }

    /// A frame too big for the budget: `None` to say nothing, otherwise
    /// whether an undo would put it right. Only an edit that stopped a frame
    /// that was drawing can be blamed: not a project that opened this way, a
    /// scrub to a heavier frame, or a later edit while it still won't draw.
    fn refused(&mut self, revision: u64, comp: Uuid, can_undo: bool) -> Option<bool> {
        let drawn = self.drawn.take();
        let stuck = self.stuck.replace(comp) == Some(comp);
        if stuck || self.told == Some((revision, comp)) {
            return None;
        }
        self.told = Some((revision, comp));
        Some(can_undo && drawn.is_some_and(|(was, of)| of == comp && was != revision))
    }
}

/// The counts to read before a frame is asked for, for [`tell`].
#[frb(ignore)]
pub(super) fn mark(state: &WorkerState) -> (u64, u64) {
    (state.renderer.frames_over_budget(), state.refusals.shown)
}

/// After a frame the Viewer asked for: if it was too big for the budget and
/// that hasn't been said, say so. `before` is [`mark`] from before the frame.
/// `can_undo` is false where no edit can have caused it, as in playback. A
/// frame that was cancelled or failed some other way changes nothing.
#[frb(ignore)]
pub(super) fn tell(
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
    before: (u64, u64),
    revision: u64,
    comp: Uuid,
    can_undo: bool,
) {
    // A renderer that doesn't skip such frames drew this one.
    let skipped = state.renderer.refuses_frames_over_budget()
        && state.renderer.frames_over_budget() != before.0;
    let over = skipped.then(|| state.renderer.over_budget()).flatten();
    let Some(over) = over else {
        if state.refusals.shown != before.1 {
            state.refusals.drew(revision, comp);
        }
        return;
    };
    let Some(undoable) = state.refusals.refused(revision, comp, can_undo) else {
        return;
    };
    _ = stream.add(WorkerResponse::FrameOverBudget(
        crate::api::cache::BridgeFrameOverBudget {
            wanted: over.wanted,
            budget: over.budget,
            undoable,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::{Refusals, Uuid};

    /// Undo is only offered for the edit that stopped the Viewer drawing: a
    /// wrong offer takes back work that had nothing to do with it.
    #[test]
    fn undo_is_offered_only_for_the_edit_that_stopped_the_frame() {
        let comp = Uuid::from_u128(1);
        let mut said = Refusals::default();
        assert_eq!(said.refused(1, comp, true), Some(false), "opened this way");
        said.drew(1, comp);
        assert_eq!(said.refused(2, comp, true), Some(true), "the edit did it");
        assert_eq!(said.refused(3, comp, true), None, "a later, unrelated edit");
        said.drew(4, comp);
        assert_eq!(said.refused(4, comp, true), Some(false), "a scrub");
        said.drew(4, comp);
        assert_eq!(said.refused(4, comp, true), None, "said already");
    }
}

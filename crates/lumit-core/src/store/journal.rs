//! The journal of edits a document keeps for undo, redo and sharing.

use super::*;

impl Journal {
    /// An edit made here has moved the document. A guest queues it until the
    /// host answers, a guest without its host keeps it for the merge, and the
    /// tap is told either way.
    pub(super) fn moved_locally(&mut self, op: &Op, inverse: &Op) {
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        shared.next_id += 1;
        let id = shared.next_id;
        match &mut shared.link {
            Link::Host => {}
            Link::Guest { pending } => pending.push_back(Pending {
                id,
                op: op.clone(),
                was: inverse.clone(),
                landed: Some((op.clone(), inverse.clone())),
            }),
            Link::Adrift { since, .. } => {
                since.push((op.clone(), inverse.clone()));
                return;
            }
        }
        let was = inverse;
        (shared.tap)(Moved::Local { id, op, was });
    }
    /// Whether moving the document from `before` to `after` would write over
    /// something another person changed after revision `at`. Undo and redo
    /// skip a step that would.
    pub(super) fn overwrites_others(&self, before: &Document, after: &Document, at: u64) -> bool {
        let Some(shared) = self.shared.as_ref() else {
            return false;
        };
        if shared.touched.is_empty() {
            return false;
        }
        let keys = footprint(before, after);
        let later = shared.touched.iter().filter(|(_, rev)| **rev > at);
        any_overlap(&keys, later.map(|(key, _)| key))
    }
}

//! How a committed op reaches the journal and the interface.

use lumit_core::store::DocumentChange;
use uuid::Uuid;

use super::{scope, LumitBridgeState, ScopedChange, SharedJournal, STREAMS};
use crate::api::{
    composition::CompositionReference, layer::LayerReference, project::ProjectReference,
    project_item::ItemReference,
};

impl LumitBridgeState {
    /// Turn a committed op into the narrowest scope Dart can rebuild from.
    ///
    /// This runs inside `DocumentStore`'s observer, which returns nothing, so
    /// there is no caller to hand an error to. It therefore cannot fail: the
    /// scope comes from matching the `Op` enum, so a new variant is a compile
    /// error here rather than a silently unscoped change at runtime.
    pub(super) fn handle_change_callback(
        document_change: DocumentChange,
        project_id: Uuid,
        journal: &SharedJournal,
    ) {
        // Journal first, then tell the interface. A crash between the two loses
        // the redraw, which the next one fixes; a crash the other way round
        // loses the edit, which nothing does.
        if let Ok(journal) = journal.lock() {
            if let Some(journal) = journal.as_ref() {
                // A journal that cannot be written is not worth taking the
                // editor down for — the work is still in the document, and the
                // next save writes it properly.
                let _ = journal.append(&document_change.op);
            }
        }

        let (comp, layer, items) = scope::op_scope(&document_change.op);
        // What this machine has and wants of a shared project's footage is
        // looked at again, on that project's own thread: a clip placed in
        // a composition is one it now wants a stand-in of.
        crate::footage::Carrier::changed(project_id);

        // **Nothing is invalidated here, and that is the point.** This
        // used to drop every held frame of every composition on every committed
        // op, because frames were filed by position: the edit did not change any
        // frame's *name*, so the only safe answer was to throw them all away.
        // The cost was paid on edits that cannot change a pixel — a rename, a
        // work-area nudge, a solo toggle, sound added to a layer — and the cache
        // bar went blank with each one.
        //
        // Frames are now filed under a hash of what is in them (docs/06 §5.2),
        // so an edit renames exactly the frames it changed and every other frame
        // stays addressable. An undo asks for the names it asked for before and
        // finds them still held. There is no invalidation step left to get right.

        // A transcript's change names the footage item it belongs to, so a
        // panel showing that item's words knows to read them again.
        let transcript = match &document_change.op {
            lumit_core::ops::Op::EditTranscript { item, .. } => Some(ItemReference::Footage(
                crate::api::footage::FootageReference::new(project_id, *item),
            )),
            _ => None,
        };
        let change = ScopedChange {
            project: ProjectReference::new(project_id),
            item: transcript.or_else(|| {
                comp.map(|c| ItemReference::Composition(CompositionReference::new(project_id, c)))
            }),
            layer: comp
                .zip(layer)
                .map(|(c, l)| LayerReference::new(project_id, c, l)),
            items,
            local: document_change.local,
        };

        let Ok(streams) = STREAMS.read() else {
            note!("Stream registry poisoned; dropping change for {project_id}");
            return;
        };

        if let Some(stream) = streams.get(&project_id) {
            _ = stream.add(change);
        }
    }
}

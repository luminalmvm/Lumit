//! The document store: immutable snapshots + operation journal
//! (docs/05-ARCHITECTURE.md; docs/impl/playback-scheduler.md §3).
//!
//! The UI thread is the single writer (by convention); readers grab an
//! `Arc<Document>` snapshot at any time, lock-free, and never observe a
//! half-applied edit.

use crate::model::Document;
use crate::ops::{apply, Op, OpError};
use arc_swap::{ArcSwap, ArcSwapOption};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One journal entry: the op as applied, and its exact inverse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub op: Op,
    pub inverse: Op,
}

/// The most undo steps kept in memory (docs/14 §5 compaction story, below).
/// Generous enough that no real editing session reaches it, small enough that
/// the history can never grow without bound. Editing software the owner knows
/// keeps far fewer (After Effects defaults to 32); 500 is a comfortable margin.
pub const MAX_UNDO_DEPTH: usize = 500;

/// The in-memory undo/redo history.
///
/// **Compaction story (docs/14 §5, mandatory for long-lived collections):**
/// `undo` is bounded to [`MAX_UNDO_DEPTH`] entries. Each [`DocumentStore::commit`]
/// that pushes past the cap drops the *oldest* entries — you can no longer undo
/// past that point, but the current document is untouched (dropping history
/// never changes state). `redo` needs no separate bound: it only ever holds
/// entries moved off `undo` by [`DocumentStore::undo`], so it can never exceed
/// the undo depth, and any [`DocumentStore::commit`] clears it outright.
/// Crash recovery does not rely on this history — lumit-project appends every
/// op to an on-disk journal as it is committed, independently of the cap.
#[derive(Default)]
struct Journal {
    undo: Vec<JournalEntry>,
    redo: Vec<JournalEntry>,
    /// The **undo group** in flight, if one is open: the entries committed
    /// since [`DocumentStore::begin_undo_group`], waiting to be folded into a
    /// single step.
    ///
    /// One gesture is one undo step, and some gestures are several ops by
    /// construction — stretching a block of keyframes that spans two layers
    /// writes each layer's curves separately, because a layer is as small as
    /// the ops get. Without this, one drag took two presses of Ctrl-Z to put
    /// back, and how many depended on what happened to be selected.
    ///
    /// **Each op still applies the moment it is committed.** Only the journal
    /// waits: the document, the revision and the change observer all move as
    /// they always did, so a read between two members of a group sees the
    /// world as it actually is. Deferring the *apply* instead would have made
    /// every read-modify-write inside a group read stale.
    group: Option<Vec<JournalEntry>>,
    /// How many [`DocumentStore::begin_undo_group`] calls are outstanding. The
    /// fold happens when this returns to zero, so a grouped gesture that calls
    /// a helper which groups on its own account still ends as one step.
    depth: usize,
}

/// What an observer is told after the store publishes a new snapshot: the op
/// that actually moved the document. The Flutter bridge turns this into a
/// scoped change so only the affected panels rebuild, rather than the whole UI.
pub struct DocumentChange {
    pub op: Op,
}

type ChangeCallback = Arc<dyn Fn(DocumentChange) + Send + Sync>;

pub struct DocumentStore {
    current: ArcSwap<Document>,
    journal: Mutex<Journal>,
    /// The change observer. Behind an `ArcSwapOption` rather than owned
    /// outright so it can be attached to a store that is **already shared**
    /// (K-273): a `&mut self` setter meant the observer had to be registered
    /// before the store went into its `Arc`, which is an ordering rule no type
    /// enforced and one every caller had to remember. Reading and swapping are
    /// lock-free, so the callback — which crosses into the frontend — can
    /// never run under a lock (docs/14 §3: no locks across FFI).
    on_change: ArcSwapOption<ChangeCallback>,
    /// Bumped once per published snapshot (commit, undo, redo, replace).
    /// A reader that remembers the number it last saw can ask "has anything
    /// changed?" for the cost of one atomic load — the frontend's read model
    /// freshens on this (K-184) instead of re-reading the world per rebuild.
    revision: std::sync::atomic::AtomicU64,
}

impl DocumentStore {
    pub fn new(doc: Document) -> Self {
        Self {
            current: ArcSwap::from_pointee(doc),
            journal: Mutex::new(Journal::default()),
            on_change: ArcSwapOption::empty(),
            revision: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The number of snapshots published so far. Equal numbers mean the
    /// document has not changed; unequal mean it has. Never decreases.
    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Acquire)
    }

    /// A snapshot is about to be published: move the number on.
    fn bump_revision(&self) {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    /// Register the change observer. Optional by construction: a frontend that
    /// reads snapshots directly never sets one, so every commit/undo/redo path
    /// must stay a no-op when it is absent.
    ///
    /// Takes `&self`, so it can be called on a store that is already shared —
    /// the observer usually wants to refer back to the thing that owns the
    /// store, which is impossible if it has to be attached first (K-273).
    /// Registering a second observer replaces the first; there is one.
    pub fn set_callback(&self, callback: ChangeCallback) {
        self.on_change.store(Some(Arc::new(callback)));
    }

    /// Tell the observer, if there is one. Callers must drop the journal lock
    /// first: the callback crosses into the frontend (the Flutter bridge pushes
    /// it down a Dart stream over FFI), and docs/14 §3 forbids holding a lock
    /// across FFI. Dropping it also lets the observer re-enter the store —
    /// notifying under the lock would deadlock on its first `commit`.
    fn notify(&self, op: Op) {
        // A lock-free read: the callback re-enters the store (the bridge
        // commits from inside it) and crosses FFI, so it must never run under
        // any lock.
        if let Some(callback) = self.on_change.load_full() {
            callback(DocumentChange { op });
        }
    }

    /// Replace the whole document, keeping the observer and clearing the
    /// history.
    ///
    /// For the one case that is not an edit: crash recovery, which opens a file
    /// and replays a journal over it. Going *through* the store rather than
    /// building a new one is what keeps the change observer attached — a
    /// recovered document installed into a fresh store would leave every panel
    /// listening to a store nothing commits to any more.
    ///
    /// The history is cleared rather than kept, because an undo stack built
    /// against the previous document cannot be applied to this one.
    pub fn replace_document(&self, doc: Document) {
        let mut journal = self.journal.lock();
        journal.undo.clear();
        journal.redo.clear();
        self.current.store(Arc::new(doc));
        self.bump_revision();
    }

    /// Lock-free snapshot for readers (render jobs capture this at schedule time).
    pub fn snapshot(&self) -> Arc<Document> {
        self.current.load_full()
    }

    /// Record how the interface is arranged for this project (K-245), to be
    /// written into the `.lum` on the next save.
    ///
    /// **Not an op, on purpose.** Three things follow from that, and each is the
    /// behaviour we want: dragging a panel is not undoable, so Ctrl-Z never
    /// rearranges the window out from under the user; it is not journalled, so
    /// crash recovery replays edits and not furniture; and it does not bump the
    /// revision, so a project does not read as having unsaved changes because a
    /// panel was resized. Nothing in the engine reads this value, so no reader
    /// can be looking at a stale one.
    ///
    /// The frontend calls it immediately before saving, which is when the
    /// arrangement it describes is the one on screen.
    pub fn set_ui_state(&self, ui_state: Option<serde_json::Value>) {
        // The journal lock is what every writer takes before read-modify-write
        // on the document, so taking it here too is what stops an edit landing
        // between the clone and the store and being dropped. Nothing crosses
        // FFI or awaits while it is held (docs/14 §3).
        let _journal = self.journal.lock();
        let mut doc = Document::clone(&self.snapshot());
        doc.ui_state = ui_state;
        self.current.store(Arc::new(doc));
    }

    /// Push one finished step onto the undo stack, keeping it bounded.
    ///
    /// Compaction (docs/14 §5): the history stays at [`MAX_UNDO_DEPTH`] by
    /// dropping the oldest steps. Dropping history never changes the document
    /// — only how far back an undo can reach.
    fn push_step(journal: &mut Journal, entry: JournalEntry) {
        journal.undo.push(entry);
        if journal.undo.len() > MAX_UNDO_DEPTH {
            let overflow = journal.undo.len() - MAX_UNDO_DEPTH;
            journal.undo.drain(..overflow);
        }
    }

    /// Begin an **undo group**: every [`Self::commit`] until the matching
    /// [`Self::end_undo_group`] becomes one step in the history.
    ///
    /// For a gesture the model cannot express as a single op. Stretching a
    /// selected block of keyframes writes one curve per property and one op
    /// per layer, because that is how coarse the ops are (`Op::SetTransform
    /// Property` and friends replace a whole animation); the user made one
    /// drag and expects one Ctrl-Z. Reversing a selection, staggering it and
    /// pasting a multi-layer clipboard are the same shape of thing.
    ///
    /// **Balanced calls, always.** A group left open records nothing on the
    /// undo stack, so callers pair the two — the Flutter side wraps them in a
    /// `try`/`finally` — and the depth count means a helper that groups on its
    /// own account nests harmlessly inside a caller that already has.
    pub fn begin_undo_group(&self) {
        let mut journal = self.journal.lock();
        journal.depth += 1;
        journal.group.get_or_insert_with(Vec::new);
    }

    /// Close the group [`Self::begin_undo_group`] opened, folding everything
    /// committed inside it into one undo step.
    ///
    /// An empty group leaves the history alone; a group of one is pushed as
    /// itself, because a `Batch` of one op undoes identically and reads worse
    /// in the journal. Two or more become an [`Op::Batch`] whose inverse is
    /// the reversed inverses — exactly what `apply` builds for a batch, so a
    /// folded group and a hand-built one are the same entry.
    ///
    /// Unbalanced calls are a no-op rather than a panic: this is reached from
    /// the frontend across FFI, where docs/14 §2 forbids panicking, and the
    /// worst an extra call can do is close a group that was never open.
    pub fn end_undo_group(&self) {
        let mut journal = self.journal.lock();
        if journal.depth == 0 {
            return;
        }
        journal.depth -= 1;
        if journal.depth > 0 {
            return;
        }
        let Some(held) = journal.group.take() else {
            return;
        };
        let mut held = held;
        let entry = match held.len() {
            0 => return,
            1 => held.remove(0),
            _ => {
                let mut inverses: Vec<Op> = held.iter().map(|e| e.inverse.clone()).collect();
                inverses.reverse();
                JournalEntry {
                    op: Op::Batch {
                        ops: held.into_iter().map(|e| e.op).collect(),
                    },
                    inverse: Op::Batch { ops: inverses },
                }
            }
        };
        Self::push_step(&mut journal, entry);
    }

    /// Apply an operation, journal it, publish the new snapshot.
    pub fn commit(&self, op: Op) -> Result<Arc<Document>, OpError> {
        let mut journal = self.journal.lock();
        let mut doc = Document::clone(&self.snapshot());
        let inverse = apply(&mut doc, &op)?;

        let observed = op.clone();
        let entry = JournalEntry { op, inverse };
        // Inside a group the entry waits to be folded; outside one it is the
        // step. Redo is cleared either way — the document has moved, so the
        // forward history is gone whether or not a gesture is still running.
        match journal.group {
            Some(ref mut held) => held.push(entry),
            None => Self::push_step(&mut journal, entry),
        }
        journal.redo.clear();
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        self.bump_revision();
        drop(journal);
        self.notify(observed);

        Ok(arc)
    }

    /// Undo the most recent operation. Ok(None) when there is nothing to undo.
    pub fn undo(&self) -> Result<Option<Arc<Document>>, OpError> {
        let mut journal = self.journal.lock();
        let Some(entry) = journal.undo.pop() else {
            return Ok(None);
        };
        let mut doc = Document::clone(&self.snapshot());
        // Applying the inverse yields the original op again — symmetry by construction.
        let op = apply(&mut doc, &entry.inverse)?;
        let observed = entry.inverse.clone();
        journal.redo.push(JournalEntry {
            op,
            inverse: entry.inverse.clone(),
        });
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        self.bump_revision();
        drop(journal);
        // The observer sees the *inverse* — the op that actually moved the
        // document — not the op being undone.
        self.notify(observed);

        Ok(Some(arc))
    }

    /// Redo the most recently undone operation. Ok(None) when nothing to redo.
    pub fn redo(&self) -> Result<Option<Arc<Document>>, OpError> {
        let mut journal = self.journal.lock();
        let Some(entry) = journal.redo.pop() else {
            return Ok(None);
        };
        let mut doc = Document::clone(&self.snapshot());
        let observed = entry.op.clone();
        let inverse = apply(&mut doc, &entry.op)?;
        journal.undo.push(JournalEntry {
            op: entry.op,
            inverse,
        });
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        self.bump_revision();
        drop(journal);
        self.notify(observed);

        Ok(Some(arc))
    }

    /// The retained undo ops, oldest first (at most [`MAX_UNDO_DEPTH`] after
    /// compaction). Crash recovery does not read this — lumit-project appends
    /// each op to an on-disk journal as it is committed — so the cap dropping
    /// old entries here never loses a recoverable edit.
    pub fn journal_ops(&self) -> Vec<Op> {
        self.journal
            .lock()
            .undo
            .iter()
            .map(|e| e.op.clone())
            .collect()
    }

    pub fn can_undo(&self) -> bool {
        !self.journal.lock().undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.journal.lock().redo.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::ops::Op;
    use crate::time::{CompTime, Duration, FrameRate, Rational};
    use uuid::Uuid;

    /// **A project's own cache location is an ordinary op**, so it undoes like
    /// anything else and is journalled with the rest — which is what puts it in
    /// the `.lum` and lets it travel with a copy of the project (docs/06 §5.4).
    /// Clearing it is the same op carrying nothing, which is what makes the
    /// round trip work in both directions.
    #[test]
    fn a_projects_cache_location_is_an_undoable_op() {
        let store = DocumentStore::new(Document::new());
        assert!(store.snapshot().cache_location.is_none());

        store
            .commit(Op::SetCacheLocation {
                location: Some(CacheLocation::Custom {
                    folder: "E:/scratch".into(),
                }),
            })
            .unwrap();
        assert_eq!(
            store.snapshot().cache_location,
            Some(CacheLocation::Custom {
                folder: "E:/scratch".into()
            })
        );

        store.undo().unwrap();
        assert!(
            store.snapshot().cache_location.is_none(),
            "undo puts the project back to following the application"
        );
        store.redo().unwrap();
        assert!(store.snapshot().cache_location.is_some());
    }

    /// **The arrangement is carried, not edited** (K-245). Moving a panel is not
    /// work done to the project, so recording it must not put a step on the undo
    /// stack — Ctrl-Z after a save would otherwise rearrange the window — and
    /// must not move the revision, which is what tells the shell the project has
    /// unsaved changes.
    #[test]
    fn recording_the_arrangement_is_neither_undoable_nor_a_change() {
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::SetCacheLocation {
                location: Some(CacheLocation::BesideProject),
            })
            .unwrap();
        let revision = store.revision();

        store.set_ui_state(Some(serde_json::json!({ "dock": "whatever" })));
        assert_eq!(
            store.snapshot().ui_state,
            Some(serde_json::json!({ "dock": "whatever" }))
        );
        assert_eq!(revision, store.revision(), "not a change to the document");

        store.undo().unwrap();
        assert_eq!(
            store.snapshot().ui_state,
            Some(serde_json::json!({ "dock": "whatever" })),
            "undo reaches the edit before it, not the arrangement"
        );
        assert!(store.snapshot().cache_location.is_none());
    }

    fn folder_named(doc: &Document, id: Uuid) -> String {
        doc.folder(id).expect("the folder exists").name.clone()
    }

    /// The Project panel's Folder button (K-451): one op batch, one undo step,
    /// and with no parent named the folder lands at the panel root.
    #[test]
    fn making_a_folder_is_one_undoable_step() {
        let store = DocumentStore::new(Document::new());
        let (id, ops) = crate::ops::new_folder_ops(&store.snapshot(), "Renders", None);
        store.commit(Op::Batch { ops }).unwrap();

        assert_eq!(folder_named(&store.snapshot(), id), "Renders");
        assert!(
            store.snapshot().root_items().contains(&id),
            "with no parent it sits at the panel root"
        );

        store.undo().unwrap();
        assert!(store.snapshot().folder(id).is_none(), "one step, not two");
        store.redo().unwrap();
        assert_eq!(folder_named(&store.snapshot(), id), "Renders");
    }

    /// A parent files the new folder inside it, and the whole thing is still
    /// one undo step — the folder and its filing arrive and leave together.
    #[test]
    fn a_folder_can_be_made_inside_another() {
        let store = DocumentStore::new(Document::new());
        let (outer, ops) = crate::ops::new_folder_ops(&store.snapshot(), "Shoots", None);
        store.commit(Op::Batch { ops }).unwrap();
        let (inner, ops) = crate::ops::new_folder_ops(&store.snapshot(), "Day one", Some(outer));
        store.commit(Op::Batch { ops }).unwrap();

        assert_eq!(
            store.snapshot().folder(outer).unwrap().children,
            vec![inner],
            "the new folder is filed inside its parent"
        );
        assert!(!store.snapshot().root_items().contains(&inner));

        store.undo().unwrap();
        assert!(store.snapshot().folder(inner).is_none());
        assert!(
            store.snapshot().folder(outer).unwrap().children.is_empty(),
            "undoing takes the filing back with the folder"
        );
    }

    /// A parent that has since been deleted is not an error: the folder lands
    /// at the root, which is where the panel would have drawn it anyway.
    #[test]
    fn an_unknown_parent_leaves_the_new_folder_at_the_root() {
        let store = DocumentStore::new(Document::new());
        let (id, ops) =
            crate::ops::new_folder_ops(&store.snapshot(), "Renders", Some(Uuid::now_v7()));
        assert_eq!(ops.len(), 1, "nothing to file it into");
        store.commit(Op::Batch { ops }).unwrap();
        assert!(store.snapshot().root_items().contains(&id));
    }

    /// A solid at the panel root, to have something that is not a folder to
    /// file. Any item kind would do; a solid is the cheapest to build.
    fn loose_item(store: &DocumentStore) -> Uuid {
        let id = Uuid::now_v7();
        store
            .commit(Op::AddItem {
                index: store.snapshot().items.len(),
                item: Box::new(ProjectItem::Solid(SolidDef {
                    id,
                    name: "White solid".into(),
                    colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    width: 1920,
                    height: 1080,
                    extra: serde_json::Map::new(),
                })),
            })
            .unwrap();
        id
    }

    fn make_folder(store: &DocumentStore, name: &str) -> Uuid {
        let (id, ops) = crate::ops::new_folder_ops(&store.snapshot(), name, None);
        store.commit(Op::Batch { ops }).unwrap();
        id
    }

    /// Filing an item into a folder, and refiling it from one folder into
    /// another (K-451): the panel's drag onto a folder row, and its **Move to
    /// folder** menu. Both are one undo step, and a refile takes the item out
    /// of its old folder in the same step it lands in the new one — an item
    /// listed by two folders at once would draw twice in the panel.
    #[test]
    fn filing_an_item_into_a_folder_is_one_undoable_step() {
        let store = DocumentStore::new(Document::new());
        let footage = make_folder(&store, "Footage");
        let audio = make_folder(&store, "Audio");
        let item = loose_item(&store);

        let ops = crate::ops::move_to_folder_ops(&store.snapshot(), item, footage)
            .expect("a real item into a real folder");
        store.commit(Op::Batch { ops }).unwrap();
        assert_eq!(
            store.snapshot().folder(footage).unwrap().children,
            vec![item]
        );
        assert!(!store.snapshot().root_items().contains(&item));

        let ops = crate::ops::move_to_folder_ops(&store.snapshot(), item, audio)
            .expect("refiling is the same call");
        store.commit(Op::Batch { ops }).unwrap();
        assert_eq!(store.snapshot().folder(audio).unwrap().children, vec![item]);
        assert!(
            store
                .snapshot()
                .folder(footage)
                .unwrap()
                .children
                .is_empty(),
            "it leaves the old folder in the same step it joins the new one"
        );

        store.undo().unwrap();
        assert_eq!(
            store.snapshot().folder(footage).unwrap().children,
            vec![item],
            "one undo takes the whole refile back, both folders together"
        );
        store.undo().unwrap();
        assert!(store.snapshot().root_items().contains(&item));
    }

    /// Filing something where it already sits changes nothing, so there is
    /// nothing to commit — a dropped item that never moved must not leave an
    /// undo step behind.
    #[test]
    fn filing_an_item_where_it_already_is_composes_no_ops() {
        let store = DocumentStore::new(Document::new());
        let footage = make_folder(&store, "Footage");
        let item = loose_item(&store);
        let ops = crate::ops::move_to_folder_ops(&store.snapshot(), item, footage).unwrap();
        store.commit(Op::Batch { ops }).unwrap();

        assert_eq!(
            crate::ops::move_to_folder_ops(&store.snapshot(), item, footage),
            Some(Vec::new())
        );
    }

    /// The three refusals: an unknown folder, a folder into itself, and a
    /// folder into its own descendant — that last one would take the whole
    /// branch off the panel root with nothing left to drag it back by.
    #[test]
    fn a_folder_cannot_be_filed_inside_itself_or_its_own_descendant() {
        let store = DocumentStore::new(Document::new());
        let outer = make_folder(&store, "Shoots");
        let (inner, ops) = crate::ops::new_folder_ops(&store.snapshot(), "Day one", Some(outer));
        store.commit(Op::Batch { ops }).unwrap();
        let (deep, ops) = crate::ops::new_folder_ops(&store.snapshot(), "Camera A", Some(inner));
        store.commit(Op::Batch { ops }).unwrap();
        let doc = store.snapshot();

        assert_eq!(crate::ops::move_to_folder_ops(&doc, outer, outer), None);
        assert_eq!(crate::ops::move_to_folder_ops(&doc, outer, inner), None);
        assert_eq!(
            crate::ops::move_to_folder_ops(&doc, outer, deep),
            None,
            "a descendant however far down is still a descendant"
        );
        assert_eq!(
            crate::ops::move_to_folder_ops(&doc, outer, Uuid::now_v7()),
            None,
            "no folder by that id"
        );
        assert_eq!(
            crate::ops::move_to_folder_ops(&doc, Uuid::now_v7(), outer),
            None,
            "no item by that id"
        );
        assert_eq!(
            crate::ops::move_to_folder_ops(&doc, inner, deep),
            None,
            "the branch below the moved folder counts too"
        );
        assert!(
            crate::ops::move_to_folder_ops(&doc, deep, outer).is_some(),
            "the other direction is an ordinary move"
        );
    }

    /// A blank name takes the next unused "Folder N", counting past the names
    /// already taken rather than off the number of folders — so renaming one
    /// away cannot make the next press of the button collide with a name in use.
    #[test]
    fn a_blank_folder_name_takes_the_next_unused_number() {
        let store = DocumentStore::new(Document::new());
        for expected in ["Folder 1", "Folder 2", "Folder 3"] {
            let (id, ops) = crate::ops::new_folder_ops(&store.snapshot(), "   ", None);
            store.commit(Op::Batch { ops }).unwrap();
            assert_eq!(folder_named(&store.snapshot(), id), expected);
        }

        let first = store.snapshot().root_items()[0];
        store
            .commit(Op::RenameItem {
                id: first,
                name: "Renders".into(),
            })
            .unwrap();
        let (id, ops) = crate::ops::new_folder_ops(&store.snapshot(), "", None);
        store.commit(Op::Batch { ops }).unwrap();
        assert_eq!(
            folder_named(&store.snapshot(), id),
            "Folder 1",
            "the default fills the gap a rename left rather than colliding"
        );
    }

    /// **An item's colour tag is an ordinary op** (K-451): undoable, journalled
    /// and saved with the project like a layer's label. Untagged is the absence
    /// of an entry rather than an entry saying zero, so tagging an item and
    /// untagging it again leaves the document exactly as it was found — which
    /// is what keeps a project nobody has tagged free of a line for it.
    #[test]
    fn a_project_items_colour_tag_is_an_undoable_op() {
        let store = DocumentStore::new(Document::new());
        let folder = Folder {
            id: Uuid::now_v7(),
            name: "Renders".into(),
            children: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let id = folder.id;
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Folder(folder)),
            })
            .unwrap();
        assert_eq!(store.snapshot().item_label(id), 0, "untagged to start");

        store.commit(Op::SetItemLabel { id, label: 4 }).unwrap();
        assert_eq!(store.snapshot().item_label(id), 4);

        store.commit(Op::SetItemLabel { id, label: 0 }).unwrap();
        assert_eq!(store.snapshot().item_label(id), 0);
        assert!(
            store.snapshot().item_labels.is_empty(),
            "untagging removes the entry rather than storing a zero"
        );

        store.undo().unwrap();
        assert_eq!(store.snapshot().item_label(id), 4, "undo brings it back");
        store.undo().unwrap();
        assert_eq!(store.snapshot().item_label(id), 0);
        store.redo().unwrap();
        assert_eq!(store.snapshot().item_label(id), 4);
    }

    /// A tag on an item that does not exist would sit in the map for ever, so
    /// the op refuses rather than recording it.
    #[test]
    fn tagging_an_unknown_item_is_refused() {
        let store = DocumentStore::new(Document::new());
        assert_eq!(
            store.commit(Op::SetItemLabel {
                id: Uuid::now_v7(),
                label: 3
            }),
            Err(crate::ops::OpError::UnknownItem)
        );
        assert!(store.snapshot().item_labels.is_empty());
    }

    /// The tag survives the item being deleted and undeleted: `RemoveItem`'s
    /// inverse puts the item back, and it comes back wearing its colour.
    #[test]
    fn a_tag_survives_delete_and_undo() {
        let store = DocumentStore::new(Document::new());
        let folder = Folder {
            id: Uuid::now_v7(),
            name: "Renders".into(),
            children: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let id = folder.id;
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Folder(folder)),
            })
            .unwrap();
        store.commit(Op::SetItemLabel { id, label: 7 }).unwrap();
        store.commit(Op::RemoveItem { id }).unwrap();
        store.undo().unwrap();
        assert_eq!(store.snapshot().item_label(id), 7);
    }

    /// Old projects have no tags at all, and reading one back must not fail on
    /// a field it has never heard of (docs/10 §1.1's serde-default rule). An
    /// untagged document also writes no line for them.
    #[test]
    fn a_project_without_tags_loads_untagged_and_writes_no_line() {
        let store = DocumentStore::new(Document::new());
        assert!(
            !json(&store.snapshot()).contains("item_labels"),
            "an untagged project gains no line for tags"
        );

        let mut older: serde_json::Value = serde_json::from_str(&json(&store.snapshot())).unwrap();
        older.as_object_mut().unwrap().remove("item_labels");
        let doc: Document = serde_json::from_value(older).unwrap();
        assert!(doc.item_labels.is_empty());
        assert_eq!(doc.item_label(Uuid::now_v7()), 0);
    }

    fn t(n: i64, d: i64) -> CompTime {
        CompTime(Rational::new(n, d).unwrap())
    }

    fn test_comp() -> Composition {
        Composition {
            id: Uuid::now_v7(),
            name: "Comp 1".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(30, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn test_layer(item: Uuid) -> Layer {
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "clip.mp4".into(),
            kind: LayerKind::Footage { item },
            in_point: t(0, 1),
            out_point: t(10, 1),
            start_offset: t(0, 1),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: crate::anim::Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn json(doc: &Document) -> String {
        serde_json::to_string(doc).unwrap()
    }

    /// Build a scripted edit sequence against a fresh store.
    fn scripted_ops(doc: &Document) -> (Vec<Op>, Uuid) {
        let comp = test_comp();
        let comp_id = comp.id;
        let footage = FootageItem {
            id: Uuid::now_v7(),
            name: "capture.mp4".into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: "footage/capture.mp4".into(),
                absolute_path: "/tmp/capture.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
        };
        let layer = test_layer(footage.id);
        let layer_id = layer.id;
        let _ = doc;
        (
            vec![
                Op::AddItem {
                    index: 0,
                    item: Box::new(ProjectItem::Footage(footage)),
                },
                Op::AddItem {
                    index: 1,
                    item: Box::new(ProjectItem::Composition(comp)),
                },
                Op::AddLayer {
                    comp: comp_id,
                    index: 0,
                    layer: Box::new(layer),
                },
                Op::SetLayerSpan {
                    comp: comp_id,
                    layer: layer_id,
                    in_point: t(1, 2),
                    out_point: t(19, 2),
                    start_offset: t(1, 2),
                },
                Op::RenameLayer {
                    comp: comp_id,
                    layer: layer_id,
                    name: "hero shot".into(),
                },
                Op::RenameItem {
                    id: comp_id,
                    name: "Main edit".into(),
                },
            ],
            comp_id,
        )
    }

    #[test]
    fn undo_all_restores_initial_redo_all_restores_final() {
        let initial = Document::new();
        let initial_json = json(&initial);
        let store = DocumentStore::new(initial);
        let (ops, _) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        let final_json = json(&store.snapshot());

        while store.undo().unwrap().is_some() {}
        assert_eq!(json(&store.snapshot()), initial_json, "undo-all == initial");

        while store.redo().unwrap().is_some() {}
        assert_eq!(json(&store.snapshot()), final_json, "redo-all == final");
    }

    /// The change observer is optional: the egui shell never sets one and reads
    /// snapshots directly, so `commit`/`undo`/`redo` must all be no-ops on that
    /// front. Fails without the fix — `undo` and `redo` each had a `todo!()`
    /// where the "no observer" arm belongs, so the first undo of the session
    /// panicked (and `todo` is denied workspace-wide, docs/14 §4).
    #[test]
    fn undo_and_redo_do_not_panic_without_a_change_observer() {
        let store = DocumentStore::new(Document::new());
        let (ops, _) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }

        // Nothing registered a callback, so both directions must simply work.
        assert!(store.undo().unwrap().is_some(), "undo with no observer");
        assert!(store.redo().unwrap().is_some(), "redo with no observer");
    }

    /// The observer sees every op that moved the document, in order, and an
    /// undo reports the *inverse* — the op actually applied — not the op being
    /// undone. It is also called with the journal lock released, so a callback
    /// that commits (the Flutter bridge reaches back into the store) cannot
    /// deadlock: this test would hang rather than fail if `notify` ran under it.
    #[test]
    fn the_change_observer_sees_each_op_and_can_re_enter_the_store() {
        let store = Arc::new(Mutex::new(Vec::<Op>::new()));
        let seen = store.clone();

        let doc_store = DocumentStore::new(Document::new());
        doc_store.set_callback(Arc::new(move |change| {
            seen.lock().push(change.op);
        }));

        let (ops, _) = scripted_ops(&doc_store.snapshot());
        let committed = ops.len();
        for op in ops {
            doc_store.commit(op).unwrap();
        }
        assert_eq!(store.lock().len(), committed, "one notify per commit");

        doc_store.undo().unwrap();
        assert_eq!(
            store.lock().len(),
            committed + 1,
            "undo notifies as well as commit"
        );
    }

    /// **An observer can be attached to a store that is already shared**
    /// (K-273).
    ///
    /// The setter used to take `&mut self`, so the observer had to be
    /// registered before the store went into its `Arc` — an ordering rule that
    /// no type enforced and every caller had to remember, and one that the
    /// natural shape of the thing fights: an observer usually wants to talk
    /// about the object that *owns* the store, which does not exist yet at
    /// that point. (`Arc::new_cyclic` is the workaround the test below still
    /// uses for its own reason; it should not be the only way in.)
    #[test]
    fn an_observer_attaches_to_a_store_that_is_already_shared() {
        let store = Arc::new(DocumentStore::new(Document::new()));
        // Shared first, and shared *widely* — a second handle, as a frontend
        // would hold.
        let elsewhere = Arc::clone(&store);

        let seen = Arc::new(Mutex::new(0usize));
        let count = Arc::clone(&seen);
        elsewhere.set_callback(Arc::new(move |_| {
            *count.lock() += 1;
        }));

        let (ops, _) = scripted_ops(&store.snapshot());
        let committed = ops.len();
        for op in ops {
            store.commit(op).unwrap();
        }
        assert_eq!(
            *seen.lock(),
            committed,
            "an observer registered through a shared handle still hears every op"
        );

        // And registering a second one replaces the first: there is one
        // observer, not a list.
        let later = Arc::new(Mutex::new(0usize));
        let count = Arc::clone(&later);
        store.set_callback(Arc::new(move |_| {
            *count.lock() += 1;
        }));
        store.undo().unwrap();
        assert_eq!(*later.lock(), 1, "the new observer hears it");
        assert_eq!(*seen.lock(), committed, "and the old one does not");
    }

    /// An observer that reads back into the store must not deadlock.
    ///
    /// `journal_ops` takes the very mutex `commit` holds, and `parking_lot`'s
    /// `Mutex` is not reentrant, so this hangs forever if `notify` is called
    /// before the guard is dropped. Reaching the assertions at all is the
    /// result. `Arc::new_cyclic` is what lets the callback hold a `Weak` back to
    /// the store it is attached to.
    #[test]
    fn a_re_entrant_observer_does_not_deadlock() {
        let observed = Arc::new(Mutex::new(0usize));
        let count = observed.clone();

        let store = Arc::new_cyclic(|weak: &std::sync::Weak<DocumentStore>| {
            let store = DocumentStore::new(Document::new());
            let weak = weak.clone();
            store.set_callback(Arc::new(move |_| {
                if let Some(store) = weak.upgrade() {
                    // Re-entry: locks the journal that commit just released.
                    *count.lock() = store.journal_ops().len();
                }
            }));
            store
        });

        let (ops, _) = scripted_ops(&store.snapshot());
        let committed = ops.len();
        for op in ops {
            store.commit(op).unwrap();
        }

        assert_eq!(
            *observed.lock(),
            committed,
            "the observer read the journal back from inside the callback"
        );
    }

    /// docs/14 §5: the undo history is compacted to [`MAX_UNDO_DEPTH`], and
    /// compaction never changes the document — it only limits how far back an
    /// undo can reach. Fails without the cap (the history would grow to every
    /// committed op).
    #[test]
    fn undo_history_is_capped_without_changing_the_document() {
        // Store and oracle must share one initial document (Document::new()
        // mints a fresh id each call, so two of them never compare equal).
        let initial = Document::new();
        let mut oracle = initial.clone();
        let store = DocumentStore::new(initial);
        let comp = test_comp();
        let comp_id = comp.id;
        // One AddItem, then well over the cap of cheap renames.
        let ops: Vec<Op> = std::iter::once(Op::AddItem {
            index: 0,
            item: Box::new(ProjectItem::Composition(comp)),
        })
        .chain((0..(MAX_UNDO_DEPTH + 50)).map(|i| Op::RenameItem {
            id: comp_id,
            name: format!("edit {i}"),
        }))
        .collect();

        // Oracle: apply every op straight through, no store, no cap.
        for op in &ops {
            apply(&mut oracle, op).unwrap();
        }
        for op in ops {
            store.commit(op).unwrap();
        }

        // Compaction dropped old history but not state: the store matches the
        // full replay exactly.
        assert_eq!(json(&store.snapshot()), json(&oracle));
        // The history is bounded, not the full run of commits.
        assert_eq!(store.journal_ops().len(), MAX_UNDO_DEPTH);

        // Every retained step undoes cleanly and no more (no underflow/panic).
        let mut undos = 0;
        while store.undo().unwrap().is_some() {
            undos += 1;
        }
        assert_eq!(undos, MAX_UNDO_DEPTH, "exactly the retained steps undo");
        // Redo is transitively bounded — all of it redoes back to the full state.
        let mut redos = 0;
        while store.redo().unwrap().is_some() {
            redos += 1;
        }
        assert_eq!(redos, MAX_UNDO_DEPTH);
        assert_eq!(
            json(&store.snapshot()),
            json(&oracle),
            "redo-all returns to the full state"
        );
    }

    #[test]
    fn journal_replay_reproduces_final_state() {
        let initial = Document::new();
        let mut replayed = initial.clone();
        let store = DocumentStore::new(initial);
        let (ops, _) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        for op in store.journal_ops() {
            crate::ops::apply(&mut replayed, &op).unwrap();
        }
        assert_eq!(json(&replayed), json(&store.snapshot()));
    }

    #[test]
    fn snapshots_are_isolated_from_later_edits() {
        let store = DocumentStore::new(Document::new());
        let before = store.snapshot();
        let (ops, _) = scripted_ops(&before);
        for op in ops {
            store.commit(op).unwrap();
        }
        assert!(before.items.is_empty(), "old snapshot unchanged");
        assert_eq!(store.snapshot().items.len(), 2);
    }

    #[test]
    fn commit_clears_redo() {
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        store.undo().unwrap();
        assert!(store.can_redo());
        store
            .commit(Op::RenameItem {
                id: comp_id,
                name: "diverged".into(),
            })
            .unwrap();
        assert!(!store.can_redo(), "new edit invalidates the redo branch");
    }

    #[test]
    fn transform_property_op_round_trips_through_undo() {
        use crate::anim::{Animation, Keyframe, SideInterp, EASY_EASE};
        use crate::model::TransformProp;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();

        let keys = vec![
            Keyframe {
                time: Rational::new(0, 1).unwrap(),
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: EASY_EASE,
            },
            Keyframe {
                time: Rational::new(2, 1).unwrap(),
                value: 100.0,
                interp_in: EASY_EASE,
                interp_out: SideInterp::Linear,
            },
        ];
        store
            .commit(Op::SetTransformProperty {
                comp: comp_id,
                layer: layer_id,
                prop: TransformProp::Opacity,
                animation: Animation::Keyframed(keys),
            })
            .unwrap();

        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        assert!(layer.transform.opacity.is_animated());
        let mid = layer.transform.opacity.value_at(1.0);
        assert!((mid - 50.0).abs() < 1e-9, "eased midpoint {mid}");
        assert_eq!(layer.transform.opacity.value_at(-1.0), 0.0);
        assert_eq!(layer.transform.opacity.value_at(99.0), 100.0);

        // Undo restores the static default exactly.
        store.undo().unwrap();
        let doc = store.snapshot();
        let layer = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert!(!layer.transform.opacity.is_animated());
        assert_eq!(layer.transform.opacity.value_at(1.0), 100.0);
    }

    #[test]
    fn reorder_layer_moves_and_undoes_exactly() {
        let store = DocumentStore::new(Document::new());
        let comp = test_comp();
        let comp_id = comp.id;
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        // Stack top-to-bottom: A (index 0), B, C.
        let mut ids = Vec::new();
        for _ in 0..3 {
            let layer = test_layer(Uuid::now_v7());
            ids.push(layer.id);
            store
                .commit(Op::AddLayer {
                    comp: comp_id,
                    index: 0,
                    layer: Box::new(layer),
                })
                .unwrap();
        }
        // Added top-first, so the final order is the reverse of insertion.
        let order = |s: &DocumentStore| -> Vec<Uuid> {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .map(|l| l.id)
                .collect()
        };
        let before = order(&store);
        // Move the bottom layer to the top.
        let bottom = *before.last().unwrap();
        store
            .commit(Op::ReorderLayer {
                comp: comp_id,
                layer: bottom,
                new_index: 0,
            })
            .unwrap();
        let after = order(&store);
        assert_eq!(after.first(), Some(&bottom), "moved layer is now on top");
        assert_eq!(after.len(), 3);
        // Undo restores the exact original order.
        store.undo().unwrap();
        assert_eq!(order(&store), before, "reorder undo == original order");
    }

    /// A camera layer's zoom and a layer's 3D switch both round-trip through
    /// undo — the two ops the 2.5D camera work added.
    /// Retime on a Footage layer round-trips through undo; the op refuses a
    /// SetSequenceClips round-trips through undo (a cut is one such op).
    #[test]
    fn sequence_clips_op_round_trips() {
        use crate::model::{Layer, LayerKind, Switches, TransformGroup};
        use crate::sequence::{Clip, ClipSource};
        use crate::time::{CompTime, Rational};
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        let r = |n| Rational::new(n, 1).unwrap();
        let src = Uuid::now_v7();
        let one = Clip::new(ClipSource::Footage(src), r(0), r(4), r(0), r(4));
        let seq_id = Uuid::now_v7();
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(Layer {
                    graph: Default::default(),
                    markers: Vec::new(),
                    id: seq_id,
                    name: "Seq".into(),
                    kind: LayerKind::Sequence {
                        clips: vec![one.clone()],
                    },
                    in_point: CompTime(r(0)),
                    out_point: CompTime(r(4)),
                    start_offset: CompTime(r(0)),
                    transform: TransformGroup::default(),
                    matte: None,
                    parent: None,
                    label: 0,
                    volume_db: crate::anim::Property::zero(),
                    audio_only: false,
                    retime: None,
                    interpolation: Default::default(),
                    parked_flow: None,
                    blend: Default::default(),
                    masks: Vec::new(),
                    paint: Vec::new(),
                    effects: Vec::new(),
                    switches: Switches::default(),
                    extra: serde_json::Map::new(),
                }),
            })
            .unwrap();
        // Cut into two, commit as SetSequenceClips.
        let (l, rc) = one.cut(r(2)).unwrap();
        store
            .commit(Op::SetSequenceClips {
                comp: comp_id,
                layer: seq_id,
                clips: vec![l, rc],
            })
            .unwrap();
        let n = |doc: &Document| match &doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == seq_id)
            .unwrap()
            .kind
        {
            LayerKind::Sequence { clips } => clips.len(),
            _ => 0,
        };
        assert_eq!(n(&store.snapshot()), 2);
        store.undo().unwrap();
        assert_eq!(n(&store.snapshot()), 1);
    }

    /// The frame-interpolation policy round-trips through undo, and it is a
    /// layer's own setting rather than part of its retime (K-249): the layer
    /// here is never retimed at all, and still has a policy — which is the
    /// case that used to be unrepresentable, because the only home for it was
    /// inside a retime the layer did not have.
    #[test]
    fn interpolation_round_trips_and_needs_no_retime() {
        use crate::retime::Interpolation;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();

        let layer_of = |doc: &Document| {
            doc.comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .clone()
        };
        assert_eq!(
            layer_of(&store.snapshot()).interpolation,
            Interpolation::Nearest
        );
        assert!(
            layer_of(&store.snapshot()).retime.is_none(),
            "this layer is deliberately un-retimed"
        );

        store
            .commit(Op::SetLayerInterpolation {
                comp: comp_id,
                layer: layer_id,
                interpolation: Interpolation::Blend,
                parked_flow: None,
            })
            .unwrap();
        assert_eq!(
            layer_of(&store.snapshot()).interpolation,
            Interpolation::Blend
        );

        store.undo().unwrap();
        assert_eq!(
            layer_of(&store.snapshot()).interpolation,
            Interpolation::Nearest
        );
    }

    /// The Retime *property* (K-197) round-trips through undo, and it is what
    /// `source_time_at` answers with — the mapping the render plan and the
    /// cache key both read.
    #[test]
    fn retime_property_round_trips_and_maps_source_time() {
        use crate::model::Layer;
        use crate::time::Rational;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();
        let layer_of = |doc: &Document| {
            doc.comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .clone()
        };

        // No retime: the layer reads its source at its own clock.
        assert!((layer_of(&store.snapshot()).source_time_at(4.0) - 4.0).abs() < 1e-9);

        // Identity over ten seconds, then half of it: local 4 → source 2.
        let ten = Rational::new(10, 1).unwrap();
        let mut retime = Layer::identity_retime(Rational::ZERO, ten);
        if let crate::anim::Animation::Keyframed(keys) = &mut retime.animation {
            keys[1].value = 5.0;
        }
        store
            .commit(Op::SetRetimeProperty {
                comp: comp_id,
                layer: layer_id,
                retime: Some(retime),
            })
            .unwrap();
        assert!((layer_of(&store.snapshot()).source_time_at(4.0) - 2.0).abs() < 1e-9);

        store.undo().unwrap();
        let layer = layer_of(&store.snapshot());
        assert!(layer.retime.is_none());
        assert!((layer.source_time_at(4.0) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn camera_zoom_and_three_d_ops_round_trip_through_undo() {
        use crate::anim::Animation;
        use crate::model::{Layer, LayerKind, Switches, TransformGroup};
        use crate::time::CompTime;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();
        let cam_id = uuid::Uuid::now_v7();
        let duration = store.snapshot().comp(comp_id).unwrap().duration.0;
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(Layer {
                    graph: Default::default(),
                    markers: Vec::new(),
                    id: cam_id,
                    name: "Camera".into(),
                    kind: LayerKind::Camera {
                        zoom: crate::anim::Property::fixed(1000.0),
                        solve_link: None,
                    },
                    in_point: CompTime(Rational::ZERO),
                    out_point: CompTime(duration),
                    start_offset: CompTime(Rational::ZERO),
                    transform: TransformGroup::default(),
                    matte: None,
                    parent: None,
                    label: 0,
                    volume_db: crate::anim::Property::zero(),
                    audio_only: false,
                    retime: None,
                    interpolation: Default::default(),
                    parked_flow: None,
                    blend: Default::default(),
                    masks: Vec::new(),
                    paint: Vec::new(),
                    effects: Vec::new(),
                    switches: Switches::default(),
                    extra: serde_json::Map::new(),
                }),
            })
            .unwrap();

        store
            .commit(Op::SetCameraZoom {
                comp: comp_id,
                layer: cam_id,
                animation: Animation::Static(2500.0),
            })
            .unwrap();
        store
            .commit(Op::SetLayerThreeD {
                comp: comp_id,
                layer: layer_id,
                three_d: true,
            })
            .unwrap();

        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!(comp.camera_pose(1.0).unwrap().zoom, 2500.0);
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        assert!(layer.switches.three_d);

        // Mute round-trips the same way (audible defaults true).
        store
            .commit(Op::SetLayerAudible {
                comp: comp_id,
                layer: layer_id,
                audible: false,
            })
            .unwrap();
        assert!(
            !store
                .snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .audible
        );
        store.undo().unwrap();
        assert!(
            store
                .snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .audible
        );

        // Collapse round-trips the same way (defaults false).
        let clp = |s: &DocumentStore| {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .collapse
        };
        store
            .commit(Op::SetLayerCollapse {
                comp: comp_id,
                layer: layer_id,
                collapse: true,
            })
            .unwrap();
        assert!(clp(&store));
        store.undo().unwrap();
        assert!(!clp(&store));

        // The effect stack + fx switch round-trip the same way.
        let stack = vec![crate::model::EffectInstance {
            id: Uuid::now_v7(),
            effect: crate::model::EffectKey {
                namespace: crate::model::EffectNamespace::Builtin,
                match_name: "glow".into(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: Vec::new(),
            sample_temporally: true,
            custom_name: None,
            linked_pairs: Vec::new(),
            extra: serde_json::Map::new(),
        }];
        store
            .commit(Op::SetLayerEffects {
                comp: comp_id,
                layer: layer_id,
                effects: stack.clone(),
            })
            .unwrap();
        let has_fx = |s: &DocumentStore| {
            !s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .effects
                .is_empty()
        };
        assert!(has_fx(&store));
        store.undo().unwrap();
        assert!(!has_fx(&store));
        store
            .commit(Op::SetLayerFx {
                comp: comp_id,
                layer: layer_id,
                fx: false,
            })
            .unwrap();
        store.undo().unwrap();
        assert!(
            store
                .snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .fx
        );

        // Visibility round-trips the same way (visible defaults true).
        let vis = |s: &DocumentStore| {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .visible
        };
        store
            .commit(Op::SetLayerVisible {
                comp: comp_id,
                layer: layer_id,
                visible: false,
            })
            .unwrap();
        assert!(!vis(&store));
        store.undo().unwrap();
        assert!(vis(&store));

        // Lock and label (K-168) round-trip the same way.
        let lock_label = |s: &DocumentStore| {
            let doc = s.snapshot();
            let l = doc
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .clone();
            (l.switches.locked, l.label)
        };
        store
            .commit(Op::SetLayerLocked {
                comp: comp_id,
                layer: layer_id,
                locked: true,
            })
            .unwrap();
        store
            .commit(Op::SetLayerLabel {
                comp: comp_id,
                layer: layer_id,
                label: 3,
            })
            .unwrap();
        assert_eq!(lock_label(&store), (true, 3));
        store.undo().unwrap();
        assert_eq!(lock_label(&store), (true, 0));
        store.undo().unwrap();
        assert_eq!(lock_label(&store), (false, 0));

        // Relink (docs/07 §3.3): SetMediaRef swaps the whole reference and
        // undoes to exactly the old one, so a relink is one clean step.
        let media_of = |s: &DocumentStore, id: Uuid| {
            s.snapshot().items.iter().find_map(|i| match i {
                ProjectItem::Footage(f) if f.id == id => Some(f.media.clone()),
                _ => None,
            })
        };
        let footage_id = store.snapshot().items.iter().find_map(|i| match i {
            ProjectItem::Footage(f) => Some(f.id),
            _ => None,
        });
        if let Some(fid) = footage_id {
            let before = media_of(&store, fid).unwrap();
            let mut relinked = before.clone();
            relinked.relative_path = "media/moved.mp4".into();
            relinked.absolute_path = "/new/place/moved.mp4".into();
            store
                .commit(Op::SetMediaRef {
                    id: fid,
                    media: Box::new(relinked.clone()),
                })
                .unwrap();
            assert_eq!(media_of(&store, fid).unwrap(), relinked);
            store.undo().unwrap();
            assert_eq!(
                media_of(&store, fid).unwrap(),
                before,
                "relink undoes whole"
            );
        }

        // Volume (docs/09 §6) round-trips like the transform properties.
        let vol = |s: &DocumentStore| {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .volume_db
                .value_at(0.0)
        };
        store
            .commit(Op::SetLayerVolume {
                comp: comp_id,
                layer: layer_id,
                animation: Animation::Static(-12.0),
            })
            .unwrap();
        assert_eq!(vol(&store), -12.0);
        store.undo().unwrap();
        assert_eq!(vol(&store), 0.0, "default volume is unity (0 dB)");

        store.undo().unwrap();
        store.undo().unwrap();
        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!(comp.camera_pose(1.0).unwrap().zoom, 1000.0);
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        assert!(!layer.switches.three_d);

        // Zoom on a non-camera layer is an error, not a silent no-op.
        assert!(store
            .commit(Op::SetCameraZoom {
                comp: comp_id,
                layer: layer_id,
                animation: Animation::Static(1.0),
            })
            .is_err());
    }

    /// The asset-organisation ops behave: a batch is one undo step and
    /// all-or-nothing; folder children, auto-folder slots, comp settings and
    /// solid defs all round-trip exactly.
    #[test]
    fn batch_folder_and_settings_ops_round_trip() {
        use crate::model::{Folder, LinearColour, SolidDef};
        use crate::ops::AutoFolderKind;
        use crate::time::{Duration, FrameRate};
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }

        // One batch: create the Solids folder, remember it, add a solid to it.
        let folder_id = uuid::Uuid::now_v7();
        let solid_id = uuid::Uuid::now_v7();
        let n_items = store.snapshot().items.len();
        store
            .commit(Op::Batch {
                ops: vec![
                    Op::AddItem {
                        index: n_items,
                        item: Box::new(ProjectItem::Folder(Folder {
                            id: folder_id,
                            name: "Solids".into(),
                            children: Vec::new(),
                            extra: serde_json::Map::new(),
                        })),
                    },
                    Op::SetAutoFolder {
                        kind: AutoFolderKind::Solids,
                        folder: Some(folder_id),
                    },
                    Op::AddItem {
                        index: n_items + 1,
                        item: Box::new(ProjectItem::Solid(SolidDef {
                            id: solid_id,
                            name: "White solid".into(),
                            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                            width: 1920,
                            height: 1080,
                            extra: serde_json::Map::new(),
                        })),
                    },
                    Op::SetFolderChildren {
                        folder: folder_id,
                        children: vec![solid_id],
                    },
                ],
            })
            .unwrap();
        let doc = store.snapshot();
        assert_eq!(doc.auto_folders.solids, Some(folder_id));
        assert_eq!(doc.folder(folder_id).unwrap().children, vec![solid_id]);
        assert!(doc.solid(solid_id).is_some());
        assert!(!doc.root_items().contains(&solid_id), "filed, not root");

        // One undo removes the whole batch.
        store.undo().unwrap();
        let doc = store.snapshot();
        assert_eq!(doc.auto_folders.solids, None);
        assert!(doc.solid(solid_id).is_none());
        assert!(doc.folder(folder_id).is_none());
        store.redo().unwrap();

        // A failing member rolls back the whole batch.
        let before = store.snapshot();
        assert!(store
            .commit(Op::Batch {
                ops: vec![
                    Op::RenameItem {
                        id: folder_id,
                        name: "Renamed".into(),
                    },
                    Op::RemoveItem {
                        id: uuid::Uuid::now_v7(), // unknown: fails
                    },
                ],
            })
            .is_err());
        assert_eq!(*store.snapshot(), *before, "all-or-nothing");

        // Comp settings round-trip.
        store
            .commit(Op::SetCompSettings {
                comp: comp_id,
                name: "Retitled".into(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(24, 1).unwrap(),
                duration: Duration(Rational::new(5, 1).unwrap()),
                background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            })
            .unwrap();
        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!((comp.width, comp.height), (1280, 720));
        assert_eq!(comp.name, "Retitled");
        store.undo().unwrap();
        let comp2 = store.snapshot();
        let comp2 = comp2.comp(comp_id).unwrap();
        assert_eq!((comp2.width, comp2.height), (1920, 1080));

        // Solid def edit round-trips and errors on non-solid targets.
        store
            .commit(Op::SetSolidDef {
                def: solid_id,
                name: "Grey solid".into(),
                colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
                width: 640,
                height: 480,
            })
            .unwrap();
        assert_eq!(store.snapshot().solid(solid_id).unwrap().width, 640);
        store.undo().unwrap();
        assert_eq!(store.snapshot().solid(solid_id).unwrap().width, 1920);
        assert!(store
            .commit(Op::SetSolidDef {
                def: comp_id,
                name: "x".into(),
                colour: LinearColour([0.0, 0.0, 0.0, 1.0]),
                width: 1,
                height: 1,
            })
            .is_err());
    }

    #[test]
    fn matte_op_round_trips_and_targets_any_layer() {
        use crate::model::{LayerInputSource, MatteChannel, MatteRef};
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();
        // A second layer to serve as the matte source.
        let matte_layer = test_layer(Uuid::now_v7());
        let matte_id = matte_layer.id;
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(matte_layer),
            })
            .unwrap();

        let matte = MatteRef {
            layer: matte_id,
            channel: MatteChannel::Luma,
            inverted: true,
            source: LayerInputSource::None,
        };
        store
            .commit(Op::SetLayerMatte {
                comp: comp_id,
                layer: layer_id,
                matte: Some(matte),
            })
            .unwrap();
        let doc = store.snapshot();
        let l = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert_eq!(l.matte, Some(matte));

        store.undo().unwrap();
        let doc = store.snapshot();
        let l = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert_eq!(l.matte, None, "undo clears the matte exactly");
    }

    #[test]
    fn layers_saved_before_transforms_existed_still_load() {
        // A pre-transform Layer JSON (as slice-3 Lumit wrote it).
        let old = r#"{
            "id": "018f0e9a-0000-7000-8000-000000000001",
            "name": "clip.mp4",
            "kind": { "Footage": { "item": "018f0e9a-0000-7000-8000-000000000002" } },
            "in_point": [0, 1],
            "out_point": [10, 1],
            "start_offset": [0, 1],
            "switches": { "visible": true, "audible": true, "locked": false }
        }"#;
        let layer: crate::model::Layer = serde_json::from_str(old).unwrap();
        assert_eq!(layer.transform.opacity.value_at(0.0), 100.0);
        assert_eq!(layer.transform.scale_x.value_at(0.0), 100.0);
    }

    #[test]
    fn invalid_ops_leave_document_untouched() {
        let store = DocumentStore::new(Document::new());
        let before = json(&store.snapshot());
        let bogus = Op::RemoveItem { id: Uuid::now_v7() };
        assert!(store.commit(bogus).is_err());
        assert_eq!(json(&store.snapshot()), before);
        assert!(!store.can_undo());
    }

    /// The read model's freshness check (K-184): every published snapshot has
    /// a new revision number, and a refused op leaves it alone. Fails without
    /// the bump on any one of commit, undo or redo — the frontend would then
    /// keep drawing a stale copy after exactly that kind of edit.
    #[test]
    fn every_published_snapshot_has_a_new_revision() {
        let store = DocumentStore::new(Document::new());
        let r0 = store.revision();

        let comp = test_comp();
        let id = comp.id;
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let r1 = store.revision();
        assert_ne!(r0, r1, "a commit publishes a new revision");

        store.undo().unwrap();
        let r2 = store.revision();
        assert_ne!(r1, r2, "an undo publishes a new revision");

        store.redo().unwrap();
        let r3 = store.revision();
        assert_ne!(r2, r3, "a redo publishes a new revision");
        assert!(store.snapshot().comp(id).is_some());

        assert!(store.commit(Op::RemoveItem { id: Uuid::now_v7() }).is_err());
        assert_eq!(store.revision(), r3, "a refused op moves nothing");
    }

    /// A comp holding one layer, and its ids — the setting every lock test
    /// needs before it can lock anything.
    fn doc_with_layer() -> (DocumentStore, Uuid, Uuid) {
        let comp = test_comp();
        let comp_id = comp.id;
        let layer = test_layer(Uuid::now_v7());
        let layer_id = layer.id;
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .expect("the comp goes in");
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(layer),
            })
            .expect("the layer goes in");
        (store, comp_id, layer_id)
    }

    /// **Giving a marker a span is an ordinary undoable edit**
    /// (docs/15-DESIGN.md §12A.1). `SetCompMarkers` replaces the whole list, so
    /// the duration needs no op of its own — but "needs no code" and "works"
    /// are different claims, and only this one is checkable. Undo must put the
    /// marker back as the moment it was, not merely put a marker back.
    #[test]
    fn a_markers_span_is_set_and_undone_like_any_other_edit() {
        use crate::markers::Marker;
        let (store, comp, _) = doc_with_layer();
        let at = crate::time::Rational::new(2, 1).expect("2s");
        let span = crate::time::Rational::new(3, 2).expect("1.5s");

        let moment = Marker::user(Uuid::now_v7(), at);
        store
            .commit(Op::SetCompMarkers {
                comp,
                markers: vec![moment.clone()],
            })
            .expect("a marker goes on the ruler");

        let stretched = Marker {
            duration: Some(span),
            ..moment.clone()
        };
        store
            .commit(Op::SetCompMarkers {
                comp,
                markers: vec![stretched.clone()],
            })
            .expect("and grows a span");
        let markers = |store: &DocumentStore| {
            store
                .snapshot()
                .comp(comp)
                .expect("the comp is there")
                .markers
                .clone()
        };
        assert_eq!(markers(&store), vec![stretched]);

        store.undo().expect("the span is undoable");
        assert_eq!(
            markers(&store),
            vec![moment],
            "undo puts the moment back, not a marker with a stale span"
        );
    }

    fn lock(store: &DocumentStore, comp: Uuid, layer: Uuid, locked: bool) {
        store
            .commit(Op::SetLayerLocked {
                comp,
                layer,
                locked,
            })
            .expect("the lock switch is never itself refused");
    }

    /// **A locked layer refuses the edits the interface used to let through**
    /// (K-291). The Timeline guarded the *gestures* — the bar, the razor,
    /// rename, reorder, delete — while the fold-out's transform, effect and
    /// volume rows went on editing the layer, so the switch did not mean what
    /// it says. The guard is in the op applier, so it covers every caller.
    #[test]
    fn a_locked_layer_refuses_an_edit_to_what_it_is() {
        let (store, comp, layer) = doc_with_layer();
        // Unlocked, the edit lands.
        store
            .commit(Op::RenameLayer {
                comp,
                layer,
                name: "Before".into(),
            })
            .expect("an unlocked layer renames");

        lock(&store, comp, layer, true);
        let revision = store.revision();

        let refused = store.commit(Op::RenameLayer {
            comp,
            layer,
            name: "After".into(),
        });
        assert_eq!(refused, Err(OpError::LayerLocked));
        assert_eq!(
            store.revision(),
            revision,
            "a refused op moves nothing, so nothing to undo is left behind"
        );
        let doc = store.snapshot();
        let l = &doc.comp(comp).expect("comp").layers[0];
        assert_eq!(l.name, "Before", "the layer kept what it had");
    }

    /// The row families the backlog named — transform, effect and volume — plus
    /// the structural edits, all refused through the one guard.
    #[test]
    fn a_locked_layer_refuses_every_family_of_edit() {
        let (store, comp, layer) = doc_with_layer();
        lock(&store, comp, layer, true);

        let refused: Vec<Op> = vec![
            Op::SetLayerVolume {
                comp,
                layer,
                animation: crate::anim::Animation::Static(0.0),
            },
            Op::SetLayerEffects {
                comp,
                layer,
                effects: Vec::new(),
            },
            Op::SetLayerVisible {
                comp,
                layer,
                visible: false,
            },
            Op::SetLayerBlend {
                comp,
                layer,
                blend: BlendMode::Multiply,
            },
            Op::SetLayerMasks {
                comp,
                layer,
                masks: Vec::new(),
            },
            Op::RemoveLayer { comp, layer },
            Op::ReorderLayer {
                comp,
                layer,
                new_index: 0,
            },
        ];
        for op in refused {
            assert_eq!(
                store.commit(op.clone()),
                Err(OpError::LayerLocked),
                "{op:?} must be refused while the layer is locked"
            );
        }
    }

    /// **Lock protects the work, not the housekeeping.** The lock itself has to
    /// be accepted or it could never be undone; shy is a filter on the
    /// Timeline's list and the label is a colour, and neither changes a pixel
    /// or a frame.
    #[test]
    fn a_locked_layer_still_takes_the_lock_the_shy_flag_and_its_label() {
        let (store, comp, layer) = doc_with_layer();
        lock(&store, comp, layer, true);

        store
            .commit(Op::SetLayerShy {
                comp,
                layer,
                shy: true,
            })
            .expect("shy is a view filter, not an edit to the work");
        store
            .commit(Op::SetLayerLabel {
                comp,
                layer,
                label: 3,
            })
            .expect("a label colour is housekeeping");
        // And the way back out.
        lock(&store, comp, layer, false);
        store
            .commit(Op::RenameLayer {
                comp,
                layer,
                name: "Unlocked".into(),
            })
            .expect("an unlocked layer edits again");
    }

    /// A batch is one undo step, so it is all or nothing: a member that names a
    /// locked layer refuses the whole batch and leaves the document as it was.
    #[test]
    fn a_batch_touching_a_locked_layer_is_refused_whole() {
        let (store, comp, layer) = doc_with_layer();
        lock(&store, comp, layer, true);
        let revision = store.revision();

        let refused = store.commit(Op::Batch {
            ops: vec![
                Op::SetWorkArea {
                    comp,
                    work_area: None,
                },
                Op::RenameLayer {
                    comp,
                    layer,
                    name: "Sneaked in".into(),
                },
            ],
        });
        assert_eq!(refused, Err(OpError::LayerLocked));
        assert_eq!(store.revision(), revision, "the batch left nothing behind");
    }

    /// **Undo still works across a lock**, which is the property that makes the
    /// guard safe to put in the applier at all: an edit can only have been made
    /// while the layer was unlocked, so walking backwards always meets the
    /// unlock before it meets the edit.
    #[test]
    fn undo_walks_back_past_a_lock_to_the_edit_beneath_it() {
        let (store, comp, layer) = doc_with_layer();
        store
            .commit(Op::RenameLayer {
                comp,
                layer,
                name: "Edited".into(),
            })
            .expect("edit while unlocked");
        lock(&store, comp, layer, true);

        // Back past the lock…
        store.undo().expect("undo the lock");
        // …and then past the edit, which is only reachable because the layer is
        // unlocked again by the time the inverse is applied.
        store.undo().expect("undo the edit under it");
        let doc = store.snapshot();
        let l = &doc.comp(comp).expect("comp").layers[0];
        assert!(!l.switches.locked, "the lock came off first");
        assert_ne!(l.name, "Edited", "and the edit under it came back out");
    }

    // -----------------------------------------------------------------------
    // Undo groups: one gesture, one step (docs/07 §4.7).
    // -----------------------------------------------------------------------

    /// The claim the block tools rest on: several ops committed inside a group
    /// undo together, and one undo puts every one of them back. Fails without
    /// the group — each `commit` would be its own step, so a stretch that
    /// touched three curves would need three presses of Ctrl-Z.
    #[test]
    fn a_group_of_commits_is_one_undo_step() {
        let initial = Document::new();
        let initial_json = json(&initial);
        let store = DocumentStore::new(initial);
        let (ops, _) = scripted_ops(&store.snapshot());
        let committed = ops.len();

        store.begin_undo_group();
        for op in ops {
            store.commit(op).unwrap();
        }
        store.end_undo_group();

        assert_eq!(
            store.journal_ops().len(),
            1,
            "{committed} ops folded into one step"
        );
        assert!(store.undo().unwrap().is_some(), "the one step undoes");
        assert_eq!(
            json(&store.snapshot()),
            initial_json,
            "and it puts the whole gesture back"
        );
        assert!(!store.can_undo(), "there is nothing under it");
    }

    /// The document does not wait for the group to close: each op applies as it
    /// is committed, so a read taken between two members of a gesture sees the
    /// world as it is. Only the journal waits.
    #[test]
    fn a_group_applies_each_op_as_it_is_committed() {
        let store = DocumentStore::new(Document::new());
        let (ops, comp) = scripted_ops(&store.snapshot());

        store.begin_undo_group();
        for op in ops {
            store.commit(op).unwrap();
            assert!(
                store.revision() > 0,
                "every commit publishes its own snapshot"
            );
        }
        assert!(
            store.snapshot().comp(comp).is_some(),
            "the comp is there before the group closes"
        );
        store.end_undo_group();
    }

    /// A group of one is pushed as itself: a `Batch` of one undoes identically
    /// and reads worse in the journal, and the redo of it must still work.
    #[test]
    fn a_group_of_one_is_not_wrapped_in_a_batch() {
        let store = DocumentStore::new(Document::new());
        let (mut ops, _) = scripted_ops(&store.snapshot());
        ops.truncate(1);

        store.begin_undo_group();
        for op in ops {
            store.commit(op).unwrap();
        }
        store.end_undo_group();

        assert!(
            !matches!(store.journal_ops().as_slice(), [Op::Batch { .. }]),
            "one op stays one op"
        );
        assert!(store.undo().unwrap().is_some());
        assert!(store.redo().unwrap().is_some(), "and redoes");
    }

    /// An empty group leaves the history alone — a gesture that turned out to
    /// change nothing is not an undo step.
    #[test]
    fn an_empty_group_records_nothing() {
        let store = DocumentStore::new(Document::new());
        store.begin_undo_group();
        store.end_undo_group();
        assert!(!store.can_undo(), "nothing was committed, nothing recorded");
    }

    /// Nesting: a helper that groups on its own account inside a caller that
    /// already has must not close the caller's group early. The fold happens
    /// when the outermost one ends.
    #[test]
    fn nested_groups_fold_at_the_outermost_end() {
        let store = DocumentStore::new(Document::new());
        let (ops, _) = scripted_ops(&store.snapshot());

        store.begin_undo_group();
        let mut ops = ops.into_iter();
        store.commit(ops.next().unwrap()).unwrap();
        store.begin_undo_group();
        store.commit(ops.next().unwrap()).unwrap();
        store.end_undo_group();
        assert!(
            !store.can_undo(),
            "the inner end did not close the outer group"
        );
        for op in ops {
            store.commit(op).unwrap();
        }
        store.end_undo_group();

        assert_eq!(store.journal_ops().len(), 1, "one step for the whole nest");
    }

    /// Unbalanced calls are survivable rather than fatal: this is reached from
    /// the frontend across FFI, where docs/14 §2 forbids panicking.
    #[test]
    fn ending_a_group_that_was_never_begun_does_nothing() {
        let store = DocumentStore::new(Document::new());
        store.end_undo_group();
        let (ops, _) = scripted_ops(&store.snapshot());
        let committed = ops.len();
        for op in ops {
            store.commit(op).unwrap();
        }
        assert_eq!(
            store.journal_ops().len(),
            committed,
            "ordinary commits are unaffected"
        );
    }
}

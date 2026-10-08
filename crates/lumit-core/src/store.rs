//! The document store: immutable snapshots + operation journal
//! (docs/05-ARCHITECTURE.md; docs/impl/playback-scheduler.md §3).
//!
//! The UI thread is the single writer (by convention); readers grab an
//! `Arc<Document>` snapshot at any time, lock-free, and never observe a
//! half-applied edit.

use crate::model::Document;
use crate::ops::{apply, Op, OpError};
use crate::shared::{any_overlap, footprint, land, plan_merge, Conflict, Key};
use arc_swap::{ArcSwap, ArcSwapOption};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

/// One journal entry: the op as applied, its exact inverse, and what the step
/// is called in the History list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub op: Op,
    pub inverse: Op,
    /// [`Op::name`] of the op as it was committed, pinned at that moment and
    /// carried through every undo and redo of this step.
    ///
    /// **Why it is stored rather than asked for.** Undoing re-derives the
    /// forward op by applying the inverse, and an op whose inverse is a batch —
    /// [`Op::TrimCompToWorkArea`], a solve link — comes back as that batch. The
    /// edit is the same one, so its row must keep saying the same thing: a step
    /// that renamed itself the moment you undid it would be unreadable exactly
    /// when the list is being used. Not serialised: a recovery journal replays
    /// ops, and a name is for the list on screen.
    #[serde(skip)]
    pub name: &'static str,
    /// The store revision this step was made at. A shared project compares it
    /// with when someone else last changed the same part of the document.
    #[serde(skip)]
    pub at: u64,
}

/// Who a remote edit came from, as whoever delivered it names them.
/// The store carries it through to the tap and reads nothing into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteTag {
    pub peer: u32,
    pub id: u64,
}

/// What a shared store reports each time the document moves.
pub enum Moved<'a> {
    /// An edit made here. `id` is what the host's answer will quote, and
    /// `was` is what the edit replaced, which the host lands it with.
    Local { id: u64, op: &'a Op, was: &'a Op },
    /// An edit from someone else, as it was applied, and what it replaced.
    /// `as_sent` is false when that is not how its author sent it, so they
    /// have to be told as well.
    Remote {
        tag: RemoteTag,
        op: &'a Op,
        was: &'a Op,
        as_sent: bool,
    },
}

/// Where a shared store reports to. Called in the order the document moved, which is
/// the order every peer has to apply the same edits in.
///
/// **Called with the journal lock held**, because that lock is what gives the
/// order. So it must not block and must not call back into the store: push to
/// a bounded queue with `try_send` and return.
pub type Tap = Arc<dyn Fn(Moved<'_>) + Send + Sync>;

/// What a guest without its host has to merge later: the last document both
/// had, and edits made here after it, each with what it replaced.
pub type Apart = (Arc<Document>, Vec<(Op, Op)>);

/// An edit made here that the host has not answered yet.
struct Pending {
    id: u64,
    /// The edit as it was made, and what it replaced then.
    op: Op,
    was: Op,
    /// How it sits on the document now: as applied, and the inverse of that.
    /// Both change each time an edit from the host goes in underneath. `None`
    /// while it does not apply at all.
    landed: Option<(Op, Op)>,
}

/// Which end of a shared project this store is.
enum Link {
    /// The host. Every edit is ordered here.
    Host,
    /// A guest in step with its host, apart from `pending`, oldest first.
    Guest { pending: VecDeque<Pending> },
    /// A guest that has lost its host. `base` is the last document both had,
    /// `since` every edit made here after it with what it replaced.
    Adrift {
        base: Arc<Document>,
        since: Vec<(Op, Op)>,
    },
}

/// Take a guest's unanswered edits off `doc`, newest first, which leaves the
/// host's document as far as this guest has heard it.
fn unwind(pending: &VecDeque<Pending>, doc: &mut Document) -> Result<(), OpError> {
    for p in pending.iter().rev() {
        if let Some((_, inverse)) = &p.landed {
            apply(doc, inverse)?;
        }
    }
    Ok(())
}

/// Put them back on top, oldest first, each cut down to what was changed here
/// ([`land`]). The host lands them the same way on the same document, so both
/// ends arrive at the same one.
fn relay(pending: &mut VecDeque<Pending>, doc: &mut Document) {
    for p in pending.iter_mut() {
        p.landed = land(doc, &p.op, Some(&p.was)).ok();
    }
}

/// The state a store holds while its project is shared.
///
/// `touched` is keyed by part of the document, so it is bounded by the
/// document's own size, and it is dropped with the rest when sharing ends.
/// `since` grows by one op per edit while the host is away, as the crash
/// journal does between saves, and is emptied by [`DocumentStore::rejoin`].
struct Shared {
    link: Link,
    tap: Tap,
    next_id: u64,
    /// The revision at which someone else last changed each part.
    touched: BTreeMap<Key, u64>,
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
    /// Present while the project is shared.
    shared: Option<Shared>,
}

impl Journal {
    /// An edit made here has moved the document. A guest queues it until the
    /// host answers, a guest without its host keeps it for the merge, and the
    /// tap is told either way.
    fn moved_locally(&mut self, op: &Op, inverse: &Op) {
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
    fn overwrites_others(&self, before: &Document, after: &Document, at: u64) -> bool {
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

/// One row of the **History** list: what the step is called, and
/// whether it has been undone — a step that has been undone is still on the
/// road, greyed, until a fresh commit clears the forward history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryEntry {
    pub name: &'static str,
    pub undone: bool,
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
    /// outright so it can be attached to a store that is **already shared**: a
    /// `&mut self` setter meant the observer had to be registered before the
    /// store went into its `Arc`, which is an ordering rule no type enforced
    /// and one every caller had to remember. Reading and swapping are
    /// lock-free, so the callback — which crosses into the frontend — can
    /// never run under a lock (docs/14 §3: no locks across FFI).
    on_change: ArcSwapOption<ChangeCallback>,
    /// Bumped once per published snapshot (commit, undo, redo, replace).
    /// A reader that remembers the number it last saw can ask "has anything
    /// changed?" for the cost of one atomic load — the frontend's read model
    /// freshens on this instead of re-reading the world per rebuild.
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
    /// store, which is impossible if it has to be attached first.
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

    /// Record how the interface is arranged for this project, to be
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

    /// Record what a save packed into the `.lum`, or what an unpack wrote back
    /// out: the new packed map, and the references of the items an unpack
    /// moved to the files it wrote.
    ///
    /// **Not an op**, for [`Self::set_ui_state`]'s reasons and one of its own.
    /// The map says what the file on disk holds, and the save that wrote the
    /// file has already happened. An undo that took an item back out of the
    /// map would have the next save leave its bytes out of the archive, and
    /// for footage whose original is gone those are the only copy.
    pub fn set_packed(
        &self,
        packed: std::collections::BTreeMap<uuid::Uuid, crate::model::PackedMedia>,
        moved: Vec<(uuid::Uuid, crate::model::MediaRef)>,
    ) {
        let _journal = self.journal.lock();
        let mut doc = Document::clone(&self.snapshot());
        doc.packed = packed;
        for (id, media) in moved {
            if let Some(crate::model::ProjectItem::Footage(f)) = doc.item_mut(id) {
                f.media = media;
            }
        }
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
                // The gesture is named after the first thing it did, which is
                // what `Op::name` says of any batch.
                let name = held.first().map_or("Several changes", |e| e.name);
                let at = held.first().map_or(0, |e| e.at);
                JournalEntry {
                    op: Op::Batch {
                        ops: held.into_iter().map(|e| e.op).collect(),
                    },
                    inverse: Op::Batch { ops: inverses },
                    name,
                    at,
                }
            }
        };
        Self::push_step(&mut journal, entry);
    }

    /// Apply an operation, journal it, publish the new snapshot.
    pub fn commit(&self, op: Op) -> Result<Arc<Document>, OpError> {
        self.commit_as(op, None)
    }

    /// [`Self::commit`] for an edit made against an older document, such as
    /// one a merge held back. Only what its author changed from `was` is
    /// written ([`land`]).
    pub fn commit_over(&self, op: Op, was: &Op) -> Result<Arc<Document>, OpError> {
        self.commit_as(op, Some(was))
    }

    fn commit_as(&self, op: Op, was: Option<&Op>) -> Result<Arc<Document>, OpError> {
        let mut journal = self.journal.lock();
        let mut doc = Document::clone(&self.snapshot());
        let (op, inverse) = match was {
            Some(_) => land(&mut doc, &op, was)?,
            None => {
                let inverse = apply(&mut doc, &op)?;
                (op, inverse)
            }
        };

        let observed = op.clone();
        let name = op.name();
        journal.moved_locally(&op, &inverse);
        let at = self.revision() + 1;
        let entry = JournalEntry {
            op,
            inverse,
            name,
            at,
        };
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
        let shared = journal.shared.is_some();
        let before = self.snapshot();
        let (entry, doc, applied, op) = loop {
            let Some(entry) = journal.undo.pop() else {
                return Ok(None);
            };
            let mut doc = Document::clone(&before);
            // Applying the inverse yields the original op again — symmetry by construction.
            // In a shared project the step is landed, so it takes back what
            // this person changed and leaves what anyone else has changed
            // around it since.
            let undone = if shared {
                land(&mut doc, &entry.inverse, Some(&entry.op))
            } else {
                apply(&mut doc, &entry.inverse).map(|op| (entry.inverse.clone(), op))
            };
            match undone {
                // In a shared project a step can stop applying because someone
                // else removed what it changed, or would write over what they
                // changed since. Either is dropped and the step before it tried.
                Err(_) if shared => {}
                Err(e) => return Err(e),
                Ok(_) if journal.overwrites_others(&before, &doc, entry.at) => {}
                Ok((applied, op)) => break (entry, doc, applied, op),
            }
        };
        let observed = applied.clone();
        journal.moved_locally(&applied, &op);
        journal.redo.push(JournalEntry {
            op,
            inverse: applied,
            name: entry.name,
            at: self.revision() + 1,
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
        let shared = journal.shared.is_some();
        let before = self.snapshot();
        // The same landing and skipping as `undo`, for the same reasons.
        let (entry, doc, applied, inverse) = loop {
            let Some(entry) = journal.redo.pop() else {
                return Ok(None);
            };
            let mut doc = Document::clone(&before);
            let redone = if shared {
                land(&mut doc, &entry.op, Some(&entry.inverse))
            } else {
                apply(&mut doc, &entry.op).map(|inverse| (entry.op.clone(), inverse))
            };
            match redone {
                Err(_) if shared => {}
                Err(e) => return Err(e),
                Ok(_) if journal.overwrites_others(&before, &doc, entry.at) => {}
                Ok((applied, inverse)) => break (entry, doc, applied, inverse),
            }
        };
        let observed = applied.clone();
        journal.moved_locally(&applied, &inverse);
        journal.undo.push(JournalEntry {
            op: applied,
            inverse,
            name: entry.name,
            at: self.revision() + 1,
        });
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        self.bump_revision();
        drop(journal);
        self.notify(observed);

        Ok(Some(arc))
    }
    /// Start sharing this project, as the host or as a guest of one. A guest
    /// calls this on a store that holds exactly the host's document.
    pub fn share(&self, guest: bool, tap: Tap) {
        let link = if guest {
            Link::Guest {
                pending: VecDeque::new(),
            }
        } else {
            Link::Host
        };
        self.journal.lock().shared = Some(Shared {
            link,
            tap,
            next_id: 0,
            touched: BTreeMap::new(),
        });
    }

    /// Run `f` on the document with no edit able to land until it returns.
    ///
    /// How a host seats a new guest: the guest is put on the list for every
    /// later edit in the same moment its copy of the document is taken, so it
    /// misses none and gets none twice. `f` runs under the journal lock, so it
    /// has the [`Tap`]'s rules: quick, and no calling back into the store.
    pub fn frozen<R>(&self, f: impl FnOnce(Arc<Document>) -> R) -> R {
        let _journal = self.journal.lock();
        f(self.snapshot())
    }

    /// Carry on as a guest that lost its host before this store was made:
    /// one whose edits since were kept and read back. `base` is the last
    /// document both had, `since` the edits made here after it, and the store
    /// holds `base` with those applied. [`Self::rejoin`] takes it from there.
    pub fn share_apart(&self, tap: Tap, base: Arc<Document>, since: Vec<(Op, Op)>) {
        self.journal.lock().shared = Some(Shared {
            link: Link::Adrift { base, since },
            tap,
            next_id: 0,
            touched: BTreeMap::new(),
        });
    }

    /// What a guest without its host has to keep to merge later, with the
    /// edits from number `from` on. `None` for anyone else.
    #[must_use]
    pub fn apart(&self, from: usize) -> Option<Apart> {
        let journal = self.journal.lock();
        match &journal.shared.as_ref()?.link {
            Link::Adrift { base, since } => {
                Some((base.clone(), since.get(from..).unwrap_or_default().to_vec()))
            }
            _ => None,
        }
    }

    /// How many edits made here the host has not taken: a guest's unanswered
    /// ones, or every one made since it lost its host.
    #[must_use]
    pub fn unanswered(&self) -> usize {
        let journal = self.journal.lock();
        match journal.shared.as_ref().map(|shared| &shared.link) {
            Some(Link::Guest { pending }) => pending.len(),
            Some(Link::Adrift { since, .. }) => since.len(),
            _ => 0,
        }
    }

    /// Stop sharing. The document stays as it is, unanswered edits included.
    pub fn unshare(&self) {
        self.journal.lock().shared = None;
    }

    /// Apply an edit someone else made, without it becoming an undo step here.
    ///
    /// The host is handed a guest's edit with what it replaced there, and
    /// lands it on the document as it stands ([`land`]). A guest is handed
    /// the host's edits as the host applied them, with no `was`. Its document
    /// is the host's plus its own unanswered edits, and the host ordered this
    /// edit before those, so they are taken off, the edit applied, and they
    /// are landed back on top.
    ///
    /// An error on the host means the edit is refused. On a guest it means the
    /// two documents have drifted apart and the guest has to ask for the
    /// host's again.
    pub fn commit_remote(&self, op: &Op, was: Option<&Op>, tag: RemoteTag) -> Result<(), OpError> {
        let mut journal = self.journal.lock();
        let Some(shared) = journal.shared.as_mut() else {
            return Ok(());
        };
        let snapshot = self.snapshot();
        let mut doc = Document::clone(&snapshot);
        let (applied, replaced, keys) = match &mut shared.link {
            Link::Adrift { .. } => return Ok(()),
            Link::Host => {
                let (applied, replaced) = land(&mut doc, op, was)?;
                (applied, replaced, footprint(&snapshot, &doc))
            }
            Link::Guest { pending } => {
                unwind(pending, &mut doc)?;
                let theirs = (!pending.is_empty()).then(|| doc.clone());
                let replaced = apply(&mut doc, op)?;
                let keys = footprint(theirs.as_ref().unwrap_or(&snapshot), &doc);
                relay(pending, &mut doc);
                (op.clone(), replaced, keys)
            }
        };
        let revision = self.revision() + 1;
        shared
            .touched
            .extend(keys.into_iter().map(|k| (k, revision)));
        (shared.tap)(Moved::Remote {
            tag,
            op: &applied,
            was: &replaced,
            as_sent: applied == *op,
        });
        self.current.store(Arc::new(doc));
        self.bump_revision();
        drop(journal);
        self.notify(applied);
        Ok(())
    }

    /// The host applied this guest's oldest unanswered edit just as it was
    /// sent. False when that is not how the edit sits here, which means the
    /// two documents have drifted apart.
    pub fn acknowledge(&self, id: u64) -> bool {
        let mut journal = self.journal.lock();
        let Some(Shared {
            link: Link::Guest { pending },
            ..
        }) = journal.shared.as_mut()
        else {
            return false;
        };
        let as_sent = pending.front().is_some_and(|p| {
            p.id == id
                && p.landed
                    .as_ref()
                    .is_some_and(|(applied, _)| *applied == p.op)
        });
        if as_sent {
            pending.pop_front();
        }
        as_sent
    }

    /// The host applied this guest's edit `id` as `op`, which is not how it
    /// was sent: an insert moved to the end, or a value cut down to what this
    /// guest changed. The host's version goes in underneath the edits made
    /// here since. An error means the documents have drifted apart.
    pub fn commit_answer(&self, id: u64, op: &Op) -> Result<(), OpError> {
        let mut journal = self.journal.lock();
        let Some(Shared {
            link: Link::Guest { pending },
            ..
        }) = journal.shared.as_mut()
        else {
            return Ok(());
        };
        // It usually landed here the way it landed there.
        let same = pending.front().is_some_and(|p| {
            p.id == id && p.landed.as_ref().is_some_and(|(applied, _)| applied == op)
        });
        if same {
            pending.pop_front();
            return Ok(());
        }
        let mut doc = Document::clone(&self.snapshot());
        unwind(pending, &mut doc)?;
        apply(&mut doc, op)?;
        pending.retain(|p| p.id != id);
        relay(pending, &mut doc);
        self.current.store(Arc::new(doc));
        self.bump_revision();
        drop(journal);
        self.notify(op.clone());
        Ok(())
    }

    /// The host refused one of this guest's edits, so it is taken back out
    /// from under the unanswered edits made after it. An error means the
    /// documents have drifted apart.
    pub fn reject(&self, id: u64) -> Result<(), OpError> {
        let mut journal = self.journal.lock();
        let Some(Shared {
            link: Link::Guest { pending },
            ..
        }) = journal.shared.as_mut()
        else {
            return Ok(());
        };
        let Some(index) = pending.iter().position(|p| p.id == id) else {
            return Ok(());
        };
        let mut doc = Document::clone(&self.snapshot());
        unwind(pending, &mut doc)?;
        let refused = pending.remove(index).and_then(|p| p.landed);
        relay(pending, &mut doc);
        self.current.store(Arc::new(doc));
        self.bump_revision();
        drop(journal);
        if let Some((_, inverse)) = refused {
            self.notify(inverse);
        }
        Ok(())
    }

    /// This guest has lost its host. Edits go on being made and are kept, so
    /// [`Self::rejoin`] can bring them onto whatever the host has by then.
    pub fn cast_adrift(&self) {
        let mut journal = self.journal.lock();
        let Some(shared) = journal.shared.as_mut() else {
            return;
        };
        let Link::Guest { pending } = &shared.link else {
            return;
        };
        let mut base = Document::clone(&self.snapshot());
        let _ = unwind(pending, &mut base);
        let since = pending.iter().map(|p| (p.op.clone(), p.was.clone()));
        shared.link = Link::Adrift {
            since: since.collect(),
            base: Arc::new(base),
        };
    }

    /// This guest has its host back, and `theirs` is the host's document now.
    ///
    /// Every edit made here since the host was lost is brought onto it
    /// ([`plan_merge`]). The ones that touch nothing the host's side changed
    /// are applied and sent like any other edit. The rest come back as
    /// conflicts for the person to choose between, with a count of the edits
    /// that no longer apply at all. The undo history is cleared, because its
    /// steps were made against a document that is no longer this one.
    pub fn rejoin(&self, theirs: Document) -> (Vec<Conflict>, usize) {
        let mut journal = self.journal.lock();
        let Some(shared) = journal.shared.as_mut() else {
            return (Vec::new(), 0);
        };
        let Link::Adrift { base, since } = &shared.link else {
            return (Vec::new(), 0);
        };
        let merge = plan_merge(base, theirs, since);
        let mut pending = VecDeque::with_capacity(merge.clean.len());
        for (op, was) in merge.clean {
            shared.next_id += 1;
            let id = shared.next_id;
            (shared.tap)(Moved::Local {
                id,
                op: &op,
                was: &was,
            });
            pending.push_back(Pending {
                id,
                op: op.clone(),
                was: was.clone(),
                landed: Some((op, was)),
            });
        }
        shared.link = Link::Guest { pending };
        shared.touched.clear();
        journal.undo.clear();
        journal.redo.clear();
        self.current.store(Arc::new(merge.merged));
        self.bump_revision();
        (merge.conflicts, merge.refused)
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

    /// The whole road, oldest first: every step that has been applied, then
    /// every step that has been undone and is waiting to be redone.
    ///
    /// # In plain terms
    ///
    /// Undo and redo walk this list one step at a time; this is the list
    /// itself, so the History panel can show it and jump to a point on it. The
    /// steps still applied come first, in the order they happened; the ones
    /// undone follow, in the order redoing would put them back. Where those two
    /// halves meet is where the document currently stands, which is exactly
    /// [`Self::applied_steps`].
    ///
    /// The redo stack is walked backwards because it is a stack: its last entry
    /// is the one an immediate redo would take.
    #[must_use]
    pub fn history(&self) -> Vec<HistoryEntry> {
        let journal = self.journal.lock();
        journal
            .undo
            .iter()
            .map(|e| HistoryEntry {
                name: e.name,
                undone: false,
            })
            .chain(journal.redo.iter().rev().map(|e| HistoryEntry {
                name: e.name,
                undone: true,
            }))
            .collect()
    }

    /// How many of [`Self::history`]'s steps are applied — where the document
    /// stands on the road.
    #[must_use]
    pub fn applied_steps(&self) -> usize {
        self.journal.lock().undo.len()
    }

    /// Move the document to the point on [`Self::history`] where exactly
    /// `applied` steps have been applied: 0 is as far back as the history
    /// reaches, and `history().len()` is everything redone.
    ///
    /// One press of Ctrl-Z at a time, in a loop — a jump *is* undoing or
    /// redoing several times, and going through the same two methods means a
    /// jump cannot reach a state a run of key presses could not. An `applied`
    /// past either end simply stops at that end.
    ///
    /// Each step publishes its own snapshot and tells the observer, so a panel
    /// watching the document sees the same run of changes it would have seen
    /// from the keyboard. That is deliberate; if a leap across hundreds of
    /// steps ever costs too much, the fix is to hold the notification back
    /// until the last one rather than to stop going through undo/redo.
    pub fn jump_to(&self, applied: usize) -> Result<(), OpError> {
        while self.applied_steps() > applied {
            if self.undo()?.is_none() {
                break;
            }
        }
        while self.applied_steps() < applied {
            if self.redo()?.is_none() {
                break;
            }
        }
        Ok(())
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

    /// **The arrangement is carried, not edited**. Moving a panel is not
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
    /// another: the panel's drag onto a folder row, and its **Move to
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

    /// **Attaching, switching and detaching a proxy are ordinary undoable
    /// edits**: one step each, each its own inverse, and detaching
    /// leaves the document exactly as it was found so a project whose proxies
    /// have all gone writes no line for them again.
    #[test]
    fn a_proxy_is_attached_switched_and_detached_in_one_step_each() {
        use crate::model::{FootageItem, MediaRef, ProxyRef};

        let store = DocumentStore::new(Document::new());
        let id = Uuid::now_v7();
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(FootageItem {
                    sequence: None,
                    id,
                    name: "shot".into(),
                    media: MediaRef {
                        relative_path: "shot.mp4".into(),
                        absolute_path: "/media/shot.mp4".into(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    extra: serde_json::Map::new(),
                    colour_space: None,
                    source_layer: None,
                })),
            })
            .unwrap();
        let proxy = |path: &str| {
            Box::new(ProxyRef {
                media: MediaRef {
                    relative_path: path.into(),
                    absolute_path: format!("/media/{path}"),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                enabled: true,
                extra: serde_json::Map::new(),
            })
        };

        assert!(store.snapshot().proxy(id).is_none());
        assert!(
            store.snapshot().use_proxies,
            "a new project is ready to use proxies the moment one is made"
        );

        store
            .commit(Op::SetItemProxy {
                id,
                proxy: Some(proxy("shot_proxy.mov")),
            })
            .unwrap();
        assert_eq!(
            store
                .snapshot()
                .proxy_in_use(id)
                .map(|m| m.relative_path.clone()),
            Some("shot_proxy.mov".to_string())
        );

        // The item's own switch: attached but not read.
        store
            .commit(Op::SetItemUseProxy {
                id,
                use_proxy: false,
            })
            .unwrap();
        assert!(store.snapshot().proxy(id).is_some(), "still attached");
        assert!(store.snapshot().proxy_in_use(id).is_none(), "but not read");

        // The project switch overrules every item.
        store
            .commit(Op::SetItemUseProxy {
                id,
                use_proxy: true,
            })
            .unwrap();
        store
            .commit(Op::SetUseProxies { use_proxies: false })
            .unwrap();
        assert!(store.snapshot().proxy_in_use(id).is_none());
        store.undo().unwrap();
        assert!(store.snapshot().proxy_in_use(id).is_some());

        // Replacing one proxy with another is one step, and undo brings the
        // first one back rather than leaving none.
        store
            .commit(Op::SetItemProxy {
                id,
                proxy: Some(proxy("other_proxy.mov")),
            })
            .unwrap();
        assert_eq!(
            store
                .snapshot()
                .proxy_in_use(id)
                .map(|m| m.relative_path.clone()),
            Some("other_proxy.mov".to_string())
        );
        store.undo().unwrap();
        assert_eq!(
            store
                .snapshot()
                .proxy_in_use(id)
                .map(|m| m.relative_path.clone()),
            Some("shot_proxy.mov".to_string())
        );

        // Detaching removes the entry rather than leaving a disabled one.
        store.commit(Op::SetItemProxy { id, proxy: None }).unwrap();
        assert!(store.snapshot().proxies.is_empty());
        assert!(
            !json(&store.snapshot()).contains("proxies"),
            "a project with none writes no line for them"
        );
        store.undo().unwrap();
        assert!(store.snapshot().proxy(id).is_some(), "undo re-attaches");

        // And the proxy survives the item being deleted and undeleted, exactly
        // as a colour tag does — the entry outlives the item on purpose.
        store.commit(Op::RemoveItem { id }).unwrap();
        store.undo().unwrap();
        assert!(store.snapshot().proxy(id).is_some());
    }

    fn t(n: i64, d: i64) -> CompTime {
        CompTime(Rational::new(n, d).unwrap())
    }

    fn test_comp() -> Composition {
        Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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
            pan: crate::anim::Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
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
            sequence: None,
            id: Uuid::now_v7(),
            name: "capture.mp4".into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: "footage/capture.mp4".into(),
                absolute_path: "/tmp/capture.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            source_layer: None,
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
                    pan: crate::anim::Property::zero(),
                    audio_only: false,
                    adjustment: false,
                    retime: None,
                    interpolation: Default::default(),
                    parked_flow: None,
                    graph_inputs: None,
                    blend: Default::default(),
                    masks: Vec::new(),
                    paint: Vec::new(),
                    puppet: None,
                    effects: Vec::new(),
                    styles: Vec::new(),
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

    /// The Retime *property* round-trips through undo, and it is what
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
                        correction_base: None,
                        options: Default::default(),
                    },
                    in_point: CompTime(Rational::ZERO),
                    out_point: CompTime(duration),
                    start_offset: CompTime(Rational::ZERO),
                    transform: TransformGroup::default(),
                    matte: None,
                    parent: None,
                    label: 0,
                    volume_db: crate::anim::Property::zero(),
                    pan: crate::anim::Property::zero(),
                    audio_only: false,
                    adjustment: false,
                    retime: None,
                    interpolation: Default::default(),
                    parked_flow: None,
                    graph_inputs: None,
                    blend: Default::default(),
                    masks: Vec::new(),
                    paint: Vec::new(),
                    puppet: None,
                    effects: Vec::new(),
                    styles: Vec::new(),
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
            plugin_state: None,
            roto: None,
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

        // Lock and label round-trip the same way.
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

        // The master fader (docs/09 §3.1) is comp state and round-trips
        // the same way — one op, one undo step, back to unity.
        let master = |s: &DocumentStore| s.snapshot().comp(comp_id).unwrap().master_volume_db;
        assert_eq!(master(&store), 0.0, "a comp opens at unity master");
        store
            .commit(Op::SetMasterVolume {
                comp: comp_id,
                db: -4.5,
            })
            .unwrap();
        assert_eq!(master(&store), -4.5);
        store.undo().unwrap();
        assert_eq!(master(&store), 0.0);

        // The confirmed beat grid (docs/09 §5) is comp state on the
        // same pattern: set, read back exactly, undone to nothing — and a
        // grid with no tempo in it is refused rather than stored.
        let grid = |s: &DocumentStore| s.snapshot().comp(comp_id).unwrap().beat_grid;
        assert_eq!(grid(&store), None, "a comp opens with no beat grid");
        let confirmed = crate::model::BeatGrid {
            bpm: 128.0,
            phase: crate::time::Rational::new(1, 100).unwrap(),
        };
        store
            .commit(Op::SetBeatGrid {
                comp: comp_id,
                grid: Some(confirmed),
            })
            .unwrap();
        assert_eq!(grid(&store), Some(confirmed));
        store.undo().unwrap();
        assert_eq!(grid(&store), None);
        assert!(
            store
                .commit(Op::SetBeatGrid {
                    comp: comp_id,
                    grid: Some(crate::model::BeatGrid {
                        bpm: 0.0,
                        phase: crate::time::Rational::ZERO,
                    }),
                })
                .is_err(),
            "a grid with no tempo is no grid"
        );

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

    /// The read model's freshness check: every published snapshot has
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

    /// Taking a **wired** effect out of the stack takes its wires with it, in
    /// the same undo step — and one undo puts both back.
    ///
    /// Without the prune the graph kept an edge naming a box that was gone, and
    /// the next graph write of any kind was refused for it.
    #[test]
    fn removing_a_wired_effect_takes_its_wires_with_it() {
        use crate::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
        let (store, comp, layer) = doc_with_layer();

        let blur = crate::fx::instantiate("blur").expect("the catalogue knows it");
        let blur_id = blur.id;
        store
            .commit(Op::SetLayerEffects {
                comp,
                layer,
                effects: vec![blur],
            })
            .expect("the stack goes in");

        let wiggle = crate::fx::instantiate("wiggle").expect("the catalogue knows it");
        let wiggle_id = wiggle.id;
        let wired = LayerGraph {
            out_unwired: false,
            nodes: vec![wiggle],
            edges: vec![Edge {
                from: OutputRef::Driver {
                    node: wiggle_id,
                    port: "value".into(),
                },
                to: InputRef::Param {
                    node: NodeRef::Effect(blur_id),
                    port: "radius".into(),
                },
            }],
            layout: vec![
                (NodeRef::Driver(wiggle_id), [40.0, 12.0]),
                (NodeRef::Effect(blur_id), [80.0, 12.0]),
            ],
            exposed: vec![NodeRef::Effect(blur_id)],
            groups: Vec::new(),
        };
        store
            .commit(Op::SetLayerGraph {
                comp,
                layer,
                graph: Box::new(wired.clone()),
            })
            .expect("a well-formed graph is accepted");

        let read = |store: &DocumentStore| -> LayerGraph {
            store
                .snapshot()
                .comp(comp)
                .and_then(|c| c.layers.iter().find(|l| l.id == layer))
                .expect("the layer")
                .graph
                .clone()
        };

        // The stack's own op, with the wired effect gone.
        store
            .commit(Op::SetLayerEffects {
                comp,
                layer,
                effects: Vec::new(),
            })
            .expect("the removal applies");

        let pruned = read(&store);
        assert!(pruned.edges.is_empty(), "the wire goes with the box");
        assert_eq!(
            pruned.layout,
            vec![(NodeRef::Driver(wiggle_id), [40.0, 12.0])],
            "so does its place on the canvas"
        );
        assert!(pruned.exposed.is_empty(), "and its E badge");
        assert_eq!(pruned.nodes.len(), 1, "the driver itself stays");
        pruned
            .validate(&[])
            .expect("what is left must be a graph the engine accepts");

        // The proof the prune is *for*: the next graph write is not refused.
        store
            .commit(Op::SetLayerGraph {
                comp,
                layer,
                graph: Box::new(LayerGraph {
                    layout: vec![(NodeRef::Driver(wiggle_id), [4.0, 4.0])],
                    ..pruned
                }),
            })
            .expect("a box may still be dragged after the removal");

        store.undo().expect("undo applies");
        assert!(
            store.undo().expect("undo applies").is_some(),
            "one gesture, one undo step"
        );
        assert_eq!(read(&store), wired, "the undo brings the wires back");
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

    fn lock(store: &DocumentStore, comp: Uuid, layer: Uuid, locked: bool) {
        store
            .commit(Op::SetLayerLocked {
                comp,
                layer,
                locked,
            })
            .expect("the lock switch is never itself refused");
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

    /// The History list is the ops that were committed, named and in order,
    /// and an undone step stays on it greyed rather than vanishing.
    #[test]
    fn the_history_list_names_the_ops_it_was_given() {
        let store = DocumentStore::new(Document::new());
        let (ops, _) = scripted_ops(&store.snapshot());
        let expected: Vec<&'static str> = ops.iter().map(Op::name).collect();
        for op in ops {
            store.commit(op).unwrap();
        }

        let listed: Vec<&'static str> = store.history().iter().map(|e| e.name).collect();
        assert_eq!(listed, expected, "one named row per committed op, in order");
        assert!(store.history().iter().all(|e| !e.undone));
        assert_eq!(store.applied_steps(), expected.len());

        store.undo().unwrap();
        let after = store.history();
        assert_eq!(
            after.len(),
            expected.len(),
            "an undone step is still listed"
        );
        assert_eq!(after.last().map(|e| e.name), expected.last().copied());
        assert!(after.last().is_some_and(|e| e.undone));
        assert_eq!(store.applied_steps(), expected.len() - 1);

        // A fresh commit replaces the forward history, so the row goes.
        store
            .commit(Op::SetAntiAliasing {
                anti_aliasing: AntiAliasing::Off,
            })
            .unwrap();
        let listed = store.history();
        assert_eq!(listed.len(), expected.len());
        assert_eq!(listed.last().map(|e| e.name), Some("Set anti-aliasing"));
    }

    /// Jumping lands on exactly the document that stood at that point on the
    /// road, forwards and backwards, and going back and forth is lossless.
    #[test]
    fn jumping_to_a_history_index_restores_that_state() {
        let store = DocumentStore::new(Document::new());
        let (ops, _) = scripted_ops(&store.snapshot());
        let total = ops.len();
        let mut states = vec![json(&store.snapshot())];
        for op in ops {
            store.commit(op).unwrap();
            states.push(json(&store.snapshot()));
        }

        for target in [0, 3, total, 1] {
            store.jump_to(target).unwrap();
            assert_eq!(store.applied_steps(), target);
            assert_eq!(
                json(&store.snapshot()),
                states[target],
                "jumping to {target} steps applied restores that document"
            );
            assert_eq!(
                store.history().iter().filter(|e| e.undone).count(),
                total - target,
                "the rows past the jump are the undone ones"
            );
        }

        // Past either end simply stops there rather than failing.
        store.jump_to(total + 50).unwrap();
        assert_eq!(store.applied_steps(), total);
    }

    /// **Trim comp to work area**: the comp becomes the work area, and
    /// everything on the timeline slides back with it. Nothing is deleted, and
    /// one undo puts the whole comp back exactly — including the work area,
    /// which is why the batch clears it first.
    #[test]
    fn trimming_a_comp_to_its_work_area_round_trips() {
        let store = DocumentStore::new(Document::new());
        let mut comp = test_comp();
        let comp_id = comp.id;
        let inside = test_layer(Uuid::now_v7());
        let inside_id = inside.id;
        let mut outside = test_layer(Uuid::now_v7());
        outside.in_point = t(20, 1);
        outside.out_point = t(25, 1);
        let outside_id = outside.id;
        comp.layers = vec![inside, outside];
        comp.markers = vec![crate::markers::Marker::user(
            Uuid::now_v7(),
            Rational::new(4, 1).unwrap(),
        )];
        comp.work_area = Some((t(2, 1), t(12, 1)));
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let before = json(&store.snapshot());

        store
            .commit(Op::TrimCompToWorkArea { comp: comp_id })
            .unwrap();

        let doc = store.snapshot();
        let c = doc.comp(comp_id).unwrap();
        assert_eq!(c.duration, Duration(Rational::new(10, 1).unwrap()));
        assert_eq!(c.work_area, None, "the trimmed comp is its own work area");
        let layer =
            |c: &Composition, id: Uuid| c.layers.iter().find(|l| l.id == id).cloned().unwrap();
        let moved = layer(c, inside_id);
        assert_eq!(moved.in_point, t(-2, 1), "the layer slid back by the start");
        assert_eq!(moved.out_point, t(8, 1));
        assert_eq!(moved.start_offset, t(-2, 1), "so its keyframes came too");
        assert_eq!(
            layer(c, outside_id).in_point,
            t(18, 1),
            "a layer outside the work area is kept, not deleted"
        );
        assert_eq!(
            c.markers[0].time,
            t(2, 1),
            "markers travel with the picture"
        );

        store.undo().unwrap();
        assert_eq!(json(&store.snapshot()), before, "one undo puts it all back");
        store.redo().unwrap();
        assert_eq!(
            store.snapshot().comp(comp_id).unwrap().duration,
            Duration(Rational::new(10, 1).unwrap())
        );
    }

    /// **Crop comp to the region of interest**: the frame becomes the
    /// rectangle and every unparented layer moves back by its corner, so the
    /// picture inside it does not budge. A parented layer travels with its
    /// parent and must not be moved twice.
    #[test]
    fn cropping_a_comp_to_a_region_round_trips() {
        let store = DocumentStore::new(Document::new());
        let mut comp = test_comp();
        let comp_id = comp.id;
        let mut free = test_layer(Uuid::now_v7());
        free.transform.position_x = crate::anim::Property::fixed(960.0);
        free.transform.position_y = crate::anim::Property::fixed(540.0);
        let free_id = free.id;
        let mut child = test_layer(Uuid::now_v7());
        child.parent = Some(free_id);
        child.transform.position_x = crate::anim::Property::fixed(100.0);
        let child_id = child.id;
        comp.layers = vec![free, child];
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let before = json(&store.snapshot());

        store
            .commit(Op::CropCompToRegion {
                comp: comp_id,
                x: 200.0,
                y: 100.0,
                width: 640,
                height: 480,
            })
            .unwrap();

        let doc = store.snapshot();
        let c = doc.comp(comp_id).unwrap();
        assert_eq!((c.width, c.height), (640, 480));
        assert_eq!(c.duration, test_comp().duration, "cropping is not a trim");
        let layer =
            |c: &Composition, id: Uuid| c.layers.iter().find(|l| l.id == id).cloned().unwrap();
        let free = layer(c, free_id);
        assert_eq!(free.transform.position_x.value_at(0.0), 760.0);
        assert_eq!(free.transform.position_y.value_at(0.0), 440.0);
        assert_eq!(
            layer(c, child_id).transform.position_x.value_at(0.0),
            100.0,
            "a parented layer moves with its parent, not on its own"
        );

        store.undo().unwrap();
        assert_eq!(json(&store.snapshot()), before, "one undo puts it all back");
    }

    // -- The node graph composition (docs/impl/node-graph-comp.md §1.1, §3) ---

    /// A comp with one Output box in it, filed and ready to be edited.
    fn doc_with_node_graph() -> (DocumentStore, Uuid, crate::comp_graph::CompGraph) {
        let seed = crate::comp_graph::CompGraph::new_with_output();
        let mut comp = test_comp();
        let comp_id = comp.id;
        comp.graph = Some(seed.clone());
        let store = DocumentStore::new(Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .expect("the node graph goes in");
        (store, comp_id, seed)
    }

    fn graph_of(store: &DocumentStore, comp: Uuid) -> crate::comp_graph::CompGraph {
        store
            .snapshot()
            .comp(comp)
            .and_then(|c| c.graph.clone())
            .expect("the comp is a node graph")
    }

    /// **A node graph has no layers, and the engine says so** (§1.1). The guard
    /// sits beside the lock's, so a `Batch` is refused through its members and
    /// the document is left exactly as it was.
    #[test]
    fn a_layer_op_on_a_node_graph_is_refused_and_so_is_a_graph_on_a_layer_comp() {
        use crate::comp_graph::{CompGraph, GraphNode};

        let (store, comp_id, seed) = doc_with_node_graph();
        let before = json(&store.snapshot());

        let add = || Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(test_layer(Uuid::now_v7())),
        };
        assert_eq!(
            store.commit(add()),
            Err(crate::ops::OpError::CompIsNodeGraph)
        );
        assert_eq!(
            store.commit(Op::Batch { ops: vec![add()] }),
            Err(crate::ops::OpError::CompIsNodeGraph),
            "a batch is guarded through its members"
        );
        assert_eq!(json(&store.snapshot()), before, "nothing was swapped");

        // And the other way round: a comp that has layers never becomes one.
        let (layers, layer_comp, _) = doc_with_layer();
        let before = json(&layers.snapshot());
        assert_eq!(
            layers.commit(Op::SetCompGraph {
                comp: layer_comp,
                graph: Box::new(CompGraph::new_with_output()),
            }),
            Err(crate::ops::OpError::CompIsNodeGraph)
        );
        assert_eq!(json(&layers.snapshot()), before);

        // A comp-wide setting is legal on a node graph: it is a composition.
        store
            .commit(Op::SetCompBackground {
                comp: comp_id,
                background: LinearColour([0.5, 0.5, 0.5, 1.0]),
            })
            .expect("a background is not a layer");
        assert_eq!(graph_of(&store, comp_id), seed);
        let _ = GraphNode::Output { id: Uuid::now_v7() };
    }

    /// **Undo symmetry over a scripted run of graph edits** (§7 test 12): every
    /// step walked back leaves the document byte for byte as it started.
    ///
    /// A tiny deterministic sequence rather than a random one, so a failure is
    /// the same failure on every machine.
    #[test]
    fn a_run_of_graph_edits_undoes_back_to_where_it_started() {
        use crate::comp_graph::{CompGraph, GraphEdge, GraphNode};
        use crate::graph::{INPUT_PORT, OUTPUT_PORT};

        let (store, comp_id, seed) = doc_with_node_graph();
        let out_id = seed.output_id().expect("the seeded Output");
        let start = json(&store.snapshot());

        // A small linear congruential generator: the same run every time, on
        // every machine, with no crate for it.
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 16
        };

        let names = ["blur", "invert", "wiggle", "merge", "exposure"];
        let mut graph = seed.clone();
        let mut steps = 0;
        for _ in 0..12 {
            let roll = next() as usize;
            let mut candidate = graph.clone();
            match roll % 3 {
                // Add a box, and wire it into the Output when it makes a
                // picture and the Output is free.
                0 => {
                    let name = names[roll % names.len()];
                    let inst = crate::fx::instantiate(name).expect("the catalogue knows it");
                    let id = inst.id;
                    candidate.nodes.insert(0, GraphNode::Fx(inst));
                    candidate.layout.push((id, [roll as f64 % 500.0, 20.0]));
                    if name != "wiggle" && candidate.wire_into(out_id, INPUT_PORT.id).is_none() {
                        candidate.edges.push(GraphEdge {
                            from: id,
                            from_port: OUTPUT_PORT.id.to_owned(),
                            to: out_id,
                            to_port: INPUT_PORT.id.to_owned(),
                        });
                    }
                }
                // Move a box.
                1 => {
                    if let Some(place) = candidate.layout.first_mut() {
                        place.1[0] += 10.0;
                    }
                }
                // Twirl one open, or shut again.
                _ => {
                    let id = candidate.nodes.first().map(GraphNode::id);
                    if let Some(id) = id {
                        if candidate.exposed.contains(&id) {
                            candidate.exposed.retain(|held| *held != id);
                        } else {
                            candidate.exposed.push(id);
                        }
                    }
                }
            }
            if candidate.validate(Some(&store.snapshot())).is_err() {
                continue;
            }
            store
                .commit(Op::SetCompGraph {
                    comp: comp_id,
                    graph: Box::new(candidate.clone()),
                })
                .expect("a validated graph is accepted");
            graph = candidate;
            steps += 1;
        }
        assert!(steps > 4, "the run has to actually edit something");
        assert_ne!(graph_of(&store, comp_id), seed, "and change the graph");

        for _ in 0..steps {
            store.undo().expect("undo applies");
        }
        assert_eq!(
            json(&store.snapshot()),
            start,
            "every step walked back leaves the document as it began"
        );
        let _: CompGraph = graph;
    }

    /// What a tap was told.
    enum Heard {
        /// An edit made at this end: its number, the edit, and what it replaced.
        Local(u64, Op, Box<Op>),
        /// Someone else's as it was applied, and whether that is how it was sent.
        Remote(RemoteTag, Op, bool),
    }

    /// One end of a shared project: a store, and what its tap was told.
    struct End {
        store: DocumentStore,
        heard: Arc<Mutex<Vec<Heard>>>,
    }

    fn end(doc: &Document, guest: bool) -> End {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let store = DocumentStore::new(doc.clone());
        let sink = heard.clone();
        store.share(
            guest,
            Arc::new(move |moved| {
                sink.lock().push(match moved {
                    Moved::Local { id, op, was } => {
                        Heard::Local(id, op.clone(), Box::new(was.clone()))
                    }
                    Moved::Remote {
                        tag, op, as_sent, ..
                    } => Heard::Remote(tag, op.clone(), as_sent),
                });
            }),
        );
        End { store, heard }
    }

    /// What the host sends one guest.
    enum Wire {
        Applied(RemoteTag, Box<Op>),
        Accepted(u64),
        Rejected(u64),
    }

    /// Carry edits the way sharing does: the host's own first, then each
    /// guest's in the order given, every one answered to every guest.
    fn deliver(host: &End, guests: &[&End], order: &[usize]) {
        let mut wires: Vec<Vec<Wire>> = guests.iter().map(|_| Vec::new()).collect();
        let broadcast = |wires: &mut Vec<Vec<Wire>>| {
            for heard in host.heard.lock().drain(..) {
                let (tag, op, as_sent) = match heard {
                    Heard::Local(id, op, _) => (RemoteTag { peer: 0, id }, op, false),
                    Heard::Remote(tag, op, as_sent) => (tag, op, as_sent),
                };
                for (g, wire) in wires.iter_mut().enumerate() {
                    // Its author is not sent an edit back that went in as sent.
                    wire.push(if as_sent && tag.peer == g as u32 + 1 {
                        Wire::Accepted(tag.id)
                    } else {
                        Wire::Applied(tag, Box::new(op.clone()))
                    });
                }
            }
        };
        broadcast(&mut wires);
        for &g in order {
            let sent: Vec<_> = guests[g].heard.lock().drain(..).collect();
            for heard in sent {
                let Heard::Local(id, op, was) = heard else {
                    continue;
                };
                let tag = RemoteTag {
                    peer: g as u32 + 1,
                    id,
                };
                match host.store.commit_remote(&op, Some(&was), tag) {
                    Ok(()) => broadcast(&mut wires),
                    Err(_) => wires[g].push(Wire::Rejected(id)),
                }
            }
        }
        for (g, wire) in wires.into_iter().enumerate() {
            let guest = &guests[g].store;
            for message in wire {
                match message {
                    Wire::Applied(tag, op) if tag.peer == g as u32 + 1 => {
                        guest.commit_answer(tag.id, &op).unwrap();
                    }
                    Wire::Applied(tag, op) => guest.commit_remote(&op, None, tag).unwrap(),
                    Wire::Accepted(id) => assert!(guest.acknowledge(id), "in step with the host"),
                    Wire::Rejected(id) => guest.reject(id).unwrap(),
                }
            }
            guests[g].heard.lock().clear();
        }
    }

    fn rename(id: Uuid, name: &str) -> Op {
        Op::RenameItem {
            id,
            name: name.into(),
        }
    }

    fn name_of(store: &DocumentStore, id: Uuid) -> Option<String> {
        match store.snapshot().item(id)? {
            ProjectItem::Solid(s) => Some(s.name.clone()),
            _ => None,
        }
    }

    /// Two solids to edit, in a document every end starts from.
    fn two_solids() -> (Document, Uuid, Uuid) {
        let seed = DocumentStore::new(Document::new());
        let (x, y) = (loose_item(&seed), loose_item(&seed));
        (Document::clone(&seed.snapshot()), x, y)
    }

    /// The whole point of the host ordering edits: three people edit the same
    /// things at once, and whichever guest's edits arrive first, everyone ends
    /// on the same document. One run has a rename refused because the item was
    /// deleted, and an insert moved because the list got shorter.
    #[test]
    fn a_shared_project_converges_whichever_order_edits_arrive_in() {
        for order in [[0, 1], [1, 0]] {
            let (doc, x, y) = two_solids();
            let host = end(&doc, false);
            let (a, b) = (end(&doc, true), end(&doc, true));

            a.store.commit(rename(x, "a first")).unwrap();
            let added = loose_item(&a.store);
            a.store.commit(rename(x, "a second")).unwrap();
            b.store.commit(Op::RemoveItem { id: x }).unwrap();
            b.store.commit(rename(y, "b")).unwrap();
            host.store.commit(rename(y, "host")).unwrap();

            deliver(&host, &[&a, &b], &order);

            let settled = host.store.snapshot();
            assert_eq!(*a.store.snapshot(), *settled, "guest a, order {order:?}");
            assert_eq!(*b.store.snapshot(), *settled, "guest b, order {order:?}");
            assert!(settled.item(added).is_some(), "the new item survives");
            assert_eq!(name_of(&host.store, y).as_deref(), Some("b"));
        }
    }

    /// Undo in a shared project is each person's own. A step someone else has
    /// since written over is skipped rather than taking their change away, and
    /// the step before it is undone instead.
    #[test]
    fn undo_leaves_what_someone_else_has_since_changed() {
        let (doc, x, y) = two_solids();
        let host = end(&doc, false);
        host.store.commit(rename(y, "mine")).unwrap();
        host.store.commit(rename(x, "mine")).unwrap();
        let tag = RemoteTag { peer: 1, id: 1 };
        let theirs = rename(x, "theirs");
        host.store.commit_remote(&theirs, None, tag).unwrap();

        host.store.undo().unwrap();

        assert_eq!(name_of(&host.store, x).as_deref(), Some("theirs"));
        assert_eq!(name_of(&host.store, y).as_deref(), Some("White solid"));
        assert!(!host.store.can_undo());
    }

    /// Edits made while the host was away come back onto the host's document.
    /// The ones that touch nothing the host's side changed are applied and
    /// sent. The one that does is held back with the host's version in place.
    #[test]
    fn edits_made_apart_are_merged_and_a_collision_is_held_back() {
        let (doc, x, y) = two_solids();
        let guest = end(&doc, true);
        guest.store.cast_adrift();
        guest.store.commit(rename(x, "mine")).unwrap();
        guest.store.commit(rename(y, "mine")).unwrap();
        let added = loose_item(&guest.store);
        assert!(guest.heard.lock().is_empty(), "nothing is sent while away");

        let mut theirs = doc.clone();
        apply(&mut theirs, &rename(x, "theirs")).unwrap();
        let (conflicts, refused) = guest.store.rejoin(theirs);

        assert_eq!(refused, 0);
        assert_eq!(conflicts.len(), 1);
        let held: Vec<&Op> = conflicts[0].ops.iter().map(|(op, _)| op).collect();
        assert_eq!(held, vec![&rename(x, "mine")]);
        assert_eq!(name_of(&guest.store, x).as_deref(), Some("theirs"));
        assert_eq!(name_of(&guest.store, y).as_deref(), Some("mine"));
        assert!(guest.store.snapshot().item(added).is_some());
        assert_eq!(guest.heard.lock().len(), 2, "the clean edits are sent");
    }

    /// An edit made on top of a held one is held with it. Deleting an item
    /// the host's side renamed and then undoing the delete comes to nothing,
    /// so the undo is not judged on the host's document, where the item was
    /// never gone. Keeping mine puts my item back, and does not take it out.
    #[test]
    fn an_edit_made_on_top_of_a_held_one_is_held_with_it() {
        let (doc, x, _) = two_solids();
        let guest = end(&doc, true);
        guest.store.cast_adrift();
        guest.store.commit(Op::RemoveItem { id: x }).unwrap();
        guest.store.undo().unwrap();

        let mut theirs = doc.clone();
        apply(&mut theirs, &rename(x, "theirs")).unwrap();
        let (conflicts, refused) = guest.store.rejoin(theirs);

        assert_eq!((conflicts.len(), refused), (1, 0));
        assert_eq!(conflicts[0].ops.len(), 2, "the delete and its undo");
        assert_eq!(name_of(&guest.store, x).as_deref(), Some("theirs"));
        for (op, was) in &conflicts[0].ops {
            guest.store.commit_over(op.clone(), was).unwrap();
        }
        assert_eq!(name_of(&guest.store, x).as_deref(), Some("White solid"));
    }

    /// An op writes a layer's whole effect stack, and here two people each
    /// change a different effect on one layer. Both changes stand: at the
    /// same moment, through an undo, and when one was made while the host was
    /// away. Only the same parameter changed by both is a collision.
    #[test]
    fn two_people_can_change_different_effects_on_one_layer() {
        use crate::anim::Property;
        let (seed, comp, layer) = doc_with_layer();
        let blur = || crate::fx::instantiate("blur").expect("the catalogue knows it");
        let stack = vec![blur(), blur()];
        let (first, second) = (stack[0].id, stack[1].id);
        let write = |store: &DocumentStore, effects: Vec<EffectInstance>| {
            let op = Op::SetLayerEffects {
                comp,
                layer,
                effects,
            };
            store.commit(op).unwrap();
        };
        write(&seed, stack.clone());
        let doc = Document::clone(&seed.snapshot());

        let read = |store: &DocumentStore| -> Vec<EffectInstance> {
            let doc = store.snapshot();
            let layers = &doc.comp(comp).unwrap().layers;
            layers
                .iter()
                .find(|l| l.id == layer)
                .unwrap()
                .effects
                .clone()
        };
        let with = |mut effects: Vec<EffectInstance>, effect: Uuid, radius: f64| {
            let on = effects.iter_mut().find(|e| e.id == effect).unwrap();
            let param = on.params.iter_mut().find(|p| p.id == "radius").unwrap();
            param.value = EffectValue::Float(Property::fixed(radius));
            effects
        };
        let set = |store: &DocumentStore, effect: Uuid, radius: f64| {
            write(store, with(read(store), effect, radius));
        };

        let (host, guest) = (end(&doc, false), end(&doc, true));
        set(&host.store, first, 3.0);
        set(&guest.store, second, 7.0);
        deliver(&host, &[&guest], &[0]);
        let both = with(with(stack.clone(), first, 3.0), second, 7.0);
        assert_eq!(read(&host.store), both);
        assert_eq!(read(&guest.store), both);

        guest.store.undo().unwrap();
        deliver(&host, &[&guest], &[0]);
        let hosts = with(stack.clone(), first, 3.0);
        assert_eq!(read(&host.store), hosts, "undo takes back the guest's own");
        assert_eq!(read(&guest.store), hosts);

        let away = end(&doc, true);
        away.store.cast_adrift();
        set(&away.store, second, 7.0);
        set(&away.store, first, 5.0);
        let (conflicts, refused) = away.store.rejoin(Document::clone(&host.store.snapshot()));
        assert_eq!((conflicts.len(), refused), (1, 0));
        assert_eq!(
            read(&away.store),
            both,
            "the host's stands where both changed"
        );

        let (op, was) = conflicts[0].ops[0].clone();
        away.store.commit_over(op, &was).unwrap();
        let mine = with(with(stack, first, 5.0), second, 7.0);
        assert_eq!(read(&away.store), mine, "keeping mine leaves the rest");
    }
}

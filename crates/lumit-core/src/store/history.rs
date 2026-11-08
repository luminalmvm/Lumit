//! The undo history as a list: every step, where the document stands on it,
//! and the jump to another point, which is what the History window reads.

use super::{DocumentStore, HistoryEntry};
use crate::ops::{Op, OpError};
use std::borrow::Cow;

impl DocumentStore {
    /// The retained undo ops, oldest first (at most [`super::MAX_UNDO_DEPTH`] after
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
                name: e.name.clone(),
                undone: false,
            })
            .chain(journal.redo.iter().rev().map(|e| HistoryEntry {
                name: e.name.clone(),
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

    /// Call the step the open undo group folds into `name` in the History
    /// list, as a script's run is called after the script rather than after
    /// the first thing it did. Does nothing when no group is open.
    pub fn name_undo_group(&self, name: impl Into<Cow<'static, str>>) {
        let mut journal = self.journal.lock();
        if journal.group.is_some() {
            journal.group_name = Some(name.into());
        }
    }

    /// What the open undo group will be called in the History list: the name
    /// it was given, or else the first thing done in it. `None` when no group
    /// is open or nothing has been done in it yet.
    pub fn undo_group_name(&self) -> Option<Cow<'static, str>> {
        let journal = self.journal.lock();
        let first = journal.group.as_ref()?.first().map(|e| e.name.clone());
        journal.group_name.clone().or(first)
    }

    pub fn can_undo(&self) -> bool {
        !self.journal.lock().undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.journal.lock().redo.is_empty()
    }

    /// Drop every undo and redo step, leaving the document as it stands. No
    /// snapshot is published and the revision stays put, because nothing in
    /// the document moved. A group still open keeps its own steps and folds
    /// as it would have, onto an empty list.
    pub fn clear_history(&self) {
        let mut journal = self.journal.lock();
        journal.undo.clear();
        journal.redo.clear();
        journal.group_redo = None;
    }
}

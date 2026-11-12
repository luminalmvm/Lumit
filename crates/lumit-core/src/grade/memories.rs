//! A colourist's memories: up to eight grades kept with the project, each in a
//! numbered slot, to lay on any shot with a key.
//!
//! A memory holds a grade as the text Copy grade makes, and a small picture of
//! the shot it was taken from, for the Gallery to show. Neither is read here:
//! both are carried, saved and handed back.

use serde::{Deserialize, Serialize};

use crate::model::Document;
use crate::ops::Op;

/// The slots there are, numbered from 1.
pub const SLOTS: u8 = 8;

/// One kept grade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradeMemory {
    /// Which slot, 1 to [`SLOTS`].
    pub slot: u8,
    /// The grade, as the effects text Copy grade writes.
    pub grade: String,
    /// A small PNG of the shot it came from, base64, or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub picture: String,
}

/// The op that keeps `memory` in `slot`, or empties the slot with `None`.
/// `None` for a slot out of range, or one that already holds just that.
#[must_use]
pub fn set(doc: &Document, slot: u8, memory: Option<GradeMemory>) -> Option<Op> {
    if !(1..=SLOTS).contains(&slot) {
        return None;
    }
    let mut memories = doc.grade_memories.clone();
    memories.retain(|m| m.slot != slot);
    if let Some(mut m) = memory {
        m.slot = slot;
        memories.push(m);
    }
    memories.sort_by_key(|m| m.slot);
    (memories != doc.grade_memories).then_some(Op::SetGradeMemories { memories })
}

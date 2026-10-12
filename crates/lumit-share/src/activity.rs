//! Who changed what in a project that is kept online, and when.
//!
//! Every edit to such a project goes through one order, and each batch in it
//! says, inside its sealed part, who made it and by what clock. From those
//! each machine keeps a short list: one record for each sitting, which is one
//! person's edits with no gap of more than ten minutes. Someone who opens the
//! project in the morning can see what the others did overnight.
//!
//! The list is worked out from the order alone, so every machine comes to
//! the same one. A snapshot carries it as it stood there, or it would forget
//! everything older than the newest snapshot.
//!
//! A batch can also carry notes: some words a person left, with where they
//! were when they wrote them. They are kept in a second list, by the same
//! rules, and are no part of the document.
//!
//! Plain data. What comes out of a batch or a snapshot was written by another
//! machine, so all of it is cut down to size before it is kept. Any thread.

use lumit_core::CompTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// The longest two of a person's edits can be apart, in seconds, and still be
/// one sitting.
const GAP: u64 = 10 * 60;

/// The most sittings kept. The oldest go first.
const SITTINGS: usize = 200;

/// The most compositions one sitting is said to have touched.
const COMPS: usize = 32;

/// The longest name kept, in characters, which is what a person's name is
/// cut to everywhere in a shared project.
const NAME: usize = 64;

/// The most notes kept. The oldest go first.
const NOTES: usize = 200;

/// The longest a note's words are, in characters.
pub const NOTE_LENGTH: usize = 500;

/// The most notes one batch carries.
pub(crate) const NOTES_A_BATCH: usize = 8;

/// Some words a person left in a project kept online, with where they were
/// when they wrote them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Tells one note from another, so that one sent twice is kept once.
    pub id: Uuid,
    /// Whose it is, as [`Sitting::who`] says whose a sitting is.
    pub who: Uuid,
    pub name: String,
    /// When it was written, by the clock of the machine it was written on,
    /// in seconds since 1970.
    pub at: u64,
    pub text: String,
    /// The composition its writer had open, where their playhead was in
    /// it, and the layer they had chosen, for each that there was.
    #[serde(default)]
    pub comp: Option<Uuid>,
    #[serde(default)]
    pub time: Option<CompTime>,
    #[serde(default)]
    pub layer: Option<Uuid>,
    /// Where it stands in the order. 0 while it waits to be sent.
    #[serde(default)]
    pub last: u64,
    /// It came while this machine was catching up, as [`Sitting::new`].
    #[serde(skip)]
    pub new: bool,
    /// The note this one takes down, when that is all it is for. It has no
    /// words of its own. Where it stands in the order the other is gone
    /// from every list, and it is kept, unseen, so that a copy of the other
    /// which some machine still has waiting to be sent is not put back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takes_down: Option<Uuid>,
}

impl Note {
    /// Cut down to size, whoever wrote it, and standing at `last`.
    fn tidied(mut self, last: u64) -> Note {
        self.name = self.name.chars().take(NAME).collect();
        self.text = self.text.chars().take(NOTE_LENGTH).collect();
        self.at = self.at.min(now());
        (self.last, self.new) = (last, false);
        self
    }
}

/// The notes a batch carries, which another machine wrote: the first few,
/// each cut down to size, standing at `last`. One that does not read is
/// passed over, and the edits beside it are not refused for it.
pub(crate) fn notes_of(batch: &Value, last: u64) -> Vec<Note> {
    let notes = batch.get("notes").and_then(Value::as_array);
    let notes = notes.into_iter().flatten().take(NOTES_A_BATCH);
    notes
        .filter_map(|note| serde_json::from_value::<Note>(note.clone()).ok())
        .map(|note| note.tidied(last))
        .collect()
}

/// Add a note to a list, unless it is there already or has been taken down.
/// One that takes another down takes it out of the list and stays itself.
/// The newest [`NOTES`] of each are kept.
pub(crate) fn note(list: &mut Vec<Note>, note: Note) {
    let settled = |had: &Note| had.id == note.id || had.takes_down == Some(note.id);
    if list.iter().any(settled) {
        return;
    }
    if let Some(gone) = note.takes_down {
        list.retain(|had| had.id != gone);
    }
    let takes = note.takes_down.is_some();
    list.push(note);
    let alike = |had: &Note| had.takes_down.is_some() == takes;
    if list.iter().filter(|had| alike(had)).count() > NOTES {
        if let Some(oldest) = list.iter().position(alike) {
            list.remove(oldest);
        }
    }
}

/// The notes as a snapshot at `seq` holds them, as [`stored`] reads the
/// sittings.
pub(crate) fn stored_notes(list: Option<Value>, seq: u64) -> Vec<Note> {
    let list = list.and_then(|list| serde_json::from_value::<Vec<Note>>(list).ok());
    let mut list = list.unwrap_or_default();
    let extra = list.len().saturating_sub(2 * NOTES);
    list.drain(..extra);
    // Each is put in as a batch would have put it, so that the list is one
    // the order could have made whoever stored it.
    let mut stored = Vec::new();
    for one in list {
        let last = one.last.min(seq);
        note(&mut stored, one.tidied(last));
    }
    stored
}

/// One person's edits at one sitting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sitting {
    /// Whose it is: the id of their own copy of the project. A name does not
    /// tell two people apart, or this machine's edits from anyone's.
    pub who: Uuid,
    pub name: String,
    /// When it began and ended, by the clock of the machine the edits were
    /// made on, in seconds since 1970.
    pub began: u64,
    pub ended: u64,
    pub edits: u32,
    /// The compositions the edits were in, in the order they were first
    /// touched.
    pub comps: Vec<Uuid>,
    /// Where the last of its edits stands in the order.
    pub last: u64,
    /// It has edits this machine was not there for: they came while it was
    /// catching up, on joining, opening its copy again or coming back from
    /// away. For this machine alone, so it is not stored.
    #[serde(skip)]
    pub new: bool,
}

/// Who made a batch and when, by its own account, and the compositions its
/// edits name.
#[derive(Default)]
pub(crate) struct By {
    pub(crate) who: Uuid,
    pub(crate) name: String,
    pub(crate) at: u64,
    pub(crate) comps: Vec<Uuid>,
}

impl By {
    /// Read out of a batch, which another machine wrote. Anything that is
    /// not there, or not what it should be, is left empty, and the name is
    /// cut to length: none of it is worth refusing the edits for.
    pub(crate) fn of(batch: &Value, ops: &Value) -> By {
        let text = |field: &str| batch.get(field).and_then(Value::as_str);
        let who = text("who").and_then(|id| Uuid::try_parse(id).ok());
        By {
            who: who.unwrap_or_default(),
            name: text("name")
                .unwrap_or_default()
                .chars()
                .take(NAME)
                .collect(),
            at: batch.get("at").and_then(Value::as_u64).unwrap_or(0),
            comps: comps(ops),
        }
    }
}

/// This machine's clock, in seconds since 1970.
pub(crate) fn now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.map_or(0, |since| since.as_secs())
}

/// The compositions a batch's edits name, at most [`COMPS`] of them. `ops`
/// is the edits as JSON. Every edit made inside a composition says which in
/// a field called `comp`, so this needs no list of them, and an edit that
/// names none adds nothing. A batch of edits is looked inside.
pub(crate) fn comps(ops: &Value) -> Vec<Uuid> {
    fn walk(ops: &Value, found: &mut Vec<Uuid>) {
        for op in ops.as_array().into_iter().flatten() {
            let comp = op.get("comp").and_then(Value::as_str);
            let comp = comp.and_then(|id| Uuid::try_parse(id).ok());
            let fresh = |comp: &Uuid| found.len() < COMPS && !found.contains(comp);
            if let Some(comp) = comp.filter(fresh) {
                found.push(comp);
            }
            if let Some(inside) = op.get("ops") {
                walk(inside, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(ops, &mut found);
    found
}

/// Add a batch to a list: `edits` edits, the last of which stands at `last`
/// in the order. It goes on the end of its person's latest sitting if it
/// follows that closely enough, and starts a new one otherwise.
pub(crate) fn fold(list: &mut Vec<Sitting>, by: By, edits: usize, last: u64) {
    // No edit is from the future. A clock that runs ahead, or lies, would
    // otherwise date a sitting past everything that follows it.
    let at = by.at.min(now());
    let edits = u32::try_from(edits).unwrap_or(u32::MAX);
    let latest = list.iter_mut().rev().find(|sitting| sitting.who == by.who);
    // A clock that has gone back counts as no gap at all.
    match latest.filter(|sitting| at.saturating_sub(sitting.ended) <= GAP) {
        Some(sitting) => {
            sitting.name = by.name;
            sitting.ended = sitting.ended.max(at);
            sitting.edits = sitting.edits.saturating_add(edits);
            sitting.last = last;
            for comp in by.comps {
                if sitting.comps.len() < COMPS && !sitting.comps.contains(&comp) {
                    sitting.comps.push(comp);
                }
            }
        }
        None => {
            list.push(Sitting {
                who: by.who,
                name: by.name,
                began: at,
                ended: at,
                edits,
                comps: by.comps,
                last,
                new: false,
            });
            let extra = list.len().saturating_sub(SITTINGS);
            list.drain(..extra);
        }
    }
}

/// The list as a snapshot at `seq` holds it, which another machine wrote,
/// cut down to what this one would have kept. One that does not read is no
/// list at all: the document beside it is what matters.
pub(crate) fn stored(list: Option<Value>, seq: u64) -> Vec<Sitting> {
    let list = list.and_then(|list| serde_json::from_value::<Vec<Sitting>>(list).ok());
    let mut list = list.unwrap_or_default();
    let (extra, now) = (list.len().saturating_sub(SITTINGS), now());
    list.drain(..extra);
    for sitting in &mut list {
        sitting.name = sitting.name.chars().take(NAME).collect();
        sitting.ended = sitting.ended.min(now);
        sitting.began = sitting.began.min(sitting.ended);
        sitting.comps.truncate(COMPS);
        // Nothing in a snapshot is later than the snapshot.
        sitting.last = sitting.last.min(seq);
    }
    list
}

/// The list on one machine, and how much of it the person there has seen.
#[derive(Default)]
pub(crate) struct Activity {
    /// Oldest first, in the order the sittings began in.
    pub(crate) list: Vec<Sitting>,
    /// How far along the order the person here has seen. Kept with the
    /// copy, so that it opens knowing what is new since.
    pub(crate) seen: u64,
    /// The notes people have left, oldest first, as the order has them.
    pub(crate) notes: Vec<Note>,
    /// Notes written here that the order has not got yet, oldest first.
    /// They wait while this machine is away, and with a copy that closes.
    pub(crate) waiting: Vec<Note>,
}

impl Activity {
    /// Take in a batch as it happens: one watched live, or one of this
    /// machine's own as it is answered. Neither is news to the person here.
    pub(crate) fn add(&mut self, by: By, edits: usize, last: u64) {
        fold(&mut self.list, by, edits, last);
        self.keep_up(last);
    }

    /// Take the lists as the server's snapshot and the entries after it
    /// make them, on the machine `me` that has just caught up to `head`.
    /// What anyone else did past what the person here has seen is new to
    /// them. A note written here that the order turns out to have is
    /// waiting no longer.
    pub(crate) fn replace(
        &mut self,
        mut list: Vec<Sitting>,
        mut notes: Vec<Note>,
        me: Uuid,
        head: u64,
    ) {
        for sitting in &mut list {
            sitting.new = sitting.who != me && sitting.last > self.seen;
        }
        for note in &mut notes {
            let theirs = note.takes_down.is_none() && note.who != me;
            note.new = theirs && note.last > self.seen;
        }
        // In the order, or in it once and taken down since.
        let settled = |mine: &Note| {
            let same = |note: &Note| note.id == mine.id || note.takes_down == Some(mine.id);
            notes.iter().any(same)
        };
        self.waiting.retain(|mine| !settled(mine));
        (self.list, self.notes) = (list, notes);
        self.keep_up(head);
    }

    /// Take in the notes of a batch as it happens, standing at `last`.
    pub(crate) fn noted(&mut self, notes: Vec<Note>, last: u64) {
        for one in notes {
            note(&mut self.notes, one);
        }
        self.keep_up(last);
    }

    /// A note written here, to be sent. False when too many wait already.
    pub(crate) fn write(&mut self, note: Note) -> bool {
        let room = self.waiting.len() < NOTES;
        if room {
            self.waiting.push(note);
        }
        room
    }

    /// The first `count` notes waiting went into the order at `head`.
    pub(crate) fn sent(&mut self, count: usize, head: u64) {
        let sent: Vec<Note> = self
            .waiting
            .drain(..count.min(self.waiting.len()))
            .collect();
        self.noted(
            sent.into_iter().map(|note| note.tidied(head)).collect(),
            head,
        );
    }

    /// The person has been shown the lists, with the machine at `head`.
    pub(crate) fn see(&mut self, head: u64) {
        for sitting in &mut self.list {
            sitting.new = false;
        }
        for note in &mut self.notes {
            note.new = false;
        }
        self.seen = self.seen.max(head);
    }

    /// With nothing new waiting to be seen, the person is as far along as
    /// the machine is.
    fn keep_up(&mut self, head: u64) {
        let new = self.list.iter().any(|sitting| sitting.new);
        if !new && !self.notes.iter().any(|note| note.new) {
            self.seen = self.seen.max(head);
        }
    }
}

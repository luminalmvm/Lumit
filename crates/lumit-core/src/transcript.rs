//! What was said in a footage item's sound, word by word.
//!
//! # In plain terms
//!
//! Transcribing a footage item listens to one of its audio streams and writes
//! down every word, with the moment it starts and ends, how sure the model was
//! and who said it. The words are kept in the item's own source time, so every
//! clip cut from the file reads the same words wherever it sits.
//!
//! A transcript lives in the document beside the items
//! ([`Document::transcripts`]), so it is saved with the project and every
//! correction is an undo step. Each one sits behind an `Arc`, so the copy of
//! the document an edit makes copies a pointer rather than every word.
//!
//! # Thread role and contract
//!
//! Pure data and the edits on it. No IO, no clocks. Edited through
//! [`Op::EditTranscript`] on whichever thread commits, read anywhere.

use std::{ops::Range, sync::Arc};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    model::{Document, ProjectItem},
    ops::{Op, OpError},
    time::SourceTime,
};

/// One footage item's words, in its own source time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// The language the words are in, as the speech model names it: `en`,
    /// `de`, `ja`.
    pub language: String,
    /// Which of the file's audio streams was heard, by its place in the file,
    /// the same number [`crate::model::AudioStreams`] keeps.
    #[serde(default)]
    pub stream: u32,
    /// In order of time, and never overlapping.
    pub words: Vec<Word>,
    /// Everyone a word names. The same person keeps the same id in every item
    /// one job heard, so a rename reaches all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<Speaker>,
    /// The model pack that heard it, as its id and version.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One word as it was heard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    /// As shown, with its punctuation attached and no space in front.
    pub text: String,
    pub start: SourceTime,
    pub end: SourceTime,
    /// How sure the model was, 0 to 1. Exactly 0 is a sound heard between
    /// words that the model never wrote down, which comes in as "um". Nothing
    /// the model wrote is ever that low.
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Uuid>,
    /// This word starts a paragraph, or somebody else's turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub paragraph: bool,
    /// Unknown fields from newer Lumit versions.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Someone who speaks in a transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    pub id: Uuid,
    pub name: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Word {
    /// A word the model heard, with nobody named and no paragraph.
    #[must_use]
    pub fn new(
        text: impl Into<String>,
        start: SourceTime,
        end: SourceTime,
        confidence: f32,
    ) -> Self {
        Self {
            text: text.into(),
            start,
            end,
            confidence,
            speaker: None,
            paragraph: false,
            extra: serde_json::Map::new(),
        }
    }
}

impl Transcript {
    /// The word being said at `t`, if one is.
    #[must_use]
    pub fn word_at(&self, t: SourceTime) -> Option<usize> {
        let after = self.words.partition_point(|w| w.start <= t);
        let index = after.checked_sub(1)?;
        (self.words[index].end > t).then_some(index)
    }

    /// The words any part of which falls between `from` and `to`.
    #[must_use]
    pub fn words_between(&self, from: SourceTime, to: SourceTime) -> Range<usize> {
        let first = self.words.partition_point(|w| w.end <= from);
        let last = self.words.partition_point(|w| w.start < to);
        first..last.max(first)
    }

    /// The name of the speaker `id`, if this transcript has them.
    #[must_use]
    pub fn speaker_name(&self, id: Uuid) -> Option<&str> {
        self.speakers
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.as_str())
    }
}

/// One change to a footage item's transcript, carried by
/// [`Op::EditTranscript`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TranscriptEdit {
    /// Land a transcript, replace one, or take it away with `None`.
    Set(Option<Arc<Transcript>>),
    /// Put `words` where the `remove` words from `at` were. Corrections, a
    /// run's speaker, and splitting or joining words all go through this, and
    /// it carries whole words so its inverse is exact.
    Words {
        at: usize,
        remove: usize,
        words: Vec<Word>,
    },
    /// The whole list of speakers: a rename, a new one, one taken away.
    Speakers(Vec<Speaker>),
}

impl TranscriptEdit {
    /// What the History list calls it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            TranscriptEdit::Set(Some(_)) => "Transcribe",
            TranscriptEdit::Set(None) => "Delete transcript",
            TranscriptEdit::Words { .. } => "Edit transcript",
            TranscriptEdit::Speakers(_) => "Edit speakers",
        }
    }
}

/// Apply `edit` to `item`'s transcript, giving back the op that undoes it.
///
/// # Errors
///
/// [`OpError::UnknownItem`] when `item` is not a footage item, or has no
/// transcript to edit, [`OpError::BadIndex`] for a splice past the end, and
/// [`OpError::InvalidSpan`] when the result would have a word ending before
/// it starts, words out of order or overlapping, or a word naming a speaker
/// the list does not have.
pub(crate) fn apply(doc: &mut Document, item: Uuid, edit: &TranscriptEdit) -> Result<Op, OpError> {
    if !matches!(doc.item(item), Some(ProjectItem::Footage(_))) {
        return Err(OpError::UnknownItem);
    }
    let undo = match edit {
        TranscriptEdit::Set(next) => {
            if let Some(next) = next {
                check(next)?;
            }
            let previous = match next {
                Some(next) => doc.transcripts.insert(item, Arc::clone(next)),
                None => doc.transcripts.remove(&item),
            };
            TranscriptEdit::Set(previous)
        }
        TranscriptEdit::Words { at, remove, words } => {
            let current = doc.transcripts.get(&item).ok_or(OpError::UnknownItem)?;
            let next = splice(current, *at, *remove, words.clone())?;
            let removed = current.words[*at..*at + *remove].to_vec();
            doc.transcripts.insert(item, Arc::new(next));
            TranscriptEdit::Words {
                at: *at,
                remove: words.len(),
                words: removed,
            }
        }
        TranscriptEdit::Speakers(list) => {
            let current = doc.transcripts.get(&item).ok_or(OpError::UnknownItem)?;
            let next = Transcript {
                speakers: list.clone(),
                ..(**current).clone()
            };
            check_speakers(&next)?;
            let previous = std::mem::replace(
                doc.transcripts
                    .entry(item)
                    .or_insert_with(|| Arc::new(next.clone())),
                Arc::new(next),
            );
            TranscriptEdit::Speakers(previous.speakers.clone())
        }
    };
    Ok(Op::EditTranscript {
        item,
        edit: Box::new(undo),
    })
}

/// `transcript` with the `remove` words from `at` swapped for `words`, checked
/// the way [`Op::EditTranscript`] checks it.
///
/// # Errors
///
/// [`OpError::BadIndex`] when the words to remove run past the end, and
/// [`OpError::InvalidSpan`] when the new words break the order or name a
/// speaker the transcript does not have.
pub fn splice(
    transcript: &Transcript,
    at: usize,
    remove: usize,
    words: Vec<Word>,
) -> Result<Transcript, OpError> {
    let end = at.checked_add(remove).ok_or(OpError::BadIndex)?;
    if end > transcript.words.len() {
        return Err(OpError::BadIndex);
    }
    let mut next = Vec::with_capacity(transcript.words.len() - remove + words.len());
    next.extend_from_slice(&transcript.words[..at]);
    let added = words.len();
    next.extend(words);
    next.extend_from_slice(&transcript.words[end..]);
    // Only the seam and the new words can be out of order: everything else was
    // checked when it went in.
    let from = at.saturating_sub(1);
    let to = (at + added + 1).min(next.len());
    check_words(&next[from..to])?;
    let ids: Vec<Uuid> = transcript.speakers.iter().map(|s| s.id).collect();
    if next[at..at + added]
        .iter()
        .any(|w| w.speaker.is_some_and(|id| !ids.contains(&id)))
    {
        return Err(OpError::InvalidSpan);
    }
    Ok(Transcript {
        words: next,
        ..transcript.clone()
    })
}

/// The ops that rename `speaker` to `name` in every transcript holding them,
/// for one [`Op::Batch`]. Empty when nobody has that speaker.
#[must_use]
pub fn rename_speaker(doc: &Document, speaker: Uuid, name: &str) -> Vec<Op> {
    doc.transcripts
        .iter()
        .filter(|(_, t)| t.speakers.iter().any(|s| s.id == speaker))
        .map(|(item, t)| {
            let speakers = t
                .speakers
                .iter()
                .map(|s| Speaker {
                    id: s.id,
                    name: if s.id == speaker {
                        name.to_owned()
                    } else {
                        s.name.clone()
                    },
                })
                .collect();
            Op::EditTranscript {
                item: *item,
                edit: Box::new(TranscriptEdit::Speakers(speakers)),
            }
        })
        .collect()
}

/// A whole transcript, checked before it lands.
fn check(transcript: &Transcript) -> Result<(), OpError> {
    check_words(&transcript.words)?;
    check_speakers(transcript)
}

/// Every word starts before it ends, and each ends before the next starts.
fn check_words(words: &[Word]) -> Result<(), OpError> {
    let ordered = words.iter().all(|w| {
        w.start <= w.end && w.confidence.is_finite() && (0.0..=1.0).contains(&w.confidence)
    }) && words.windows(2).all(|pair| pair[0].end <= pair[1].start);
    if ordered {
        Ok(())
    } else {
        Err(OpError::InvalidSpan)
    }
}

/// Speakers have different ids, and every word names one of them or nobody.
fn check_speakers(transcript: &Transcript) -> Result<(), OpError> {
    let ids: Vec<Uuid> = transcript.speakers.iter().map(|s| s.id).collect();
    let distinct = ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id));
    let named = transcript
        .words
        .iter()
        .all(|w| w.speaker.is_none_or(|id| ids.contains(&id)));
    if distinct && named {
        Ok(())
    } else {
        Err(OpError::InvalidSpan)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::time::Rational;

    fn at(ms: i64) -> SourceTime {
        SourceTime(Rational::new(ms, 1000).unwrap())
    }

    fn word(text: &str, from: i64, to: i64) -> Word {
        Word::new(text, at(from), at(to), 0.9)
    }

    fn item(doc: &mut Document) -> Uuid {
        let id = Uuid::now_v7();
        let footage = crate::model::FootageItem {
            id,
            name: "Talk.mp4".into(),
            media: crate::model::MediaRef {
                relative_path: "Talk.mp4".into(),
                absolute_path: String::new(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            colour_space: None,
            sequence: None,
            source_layer: None,
            interpretation: crate::interpretation::Interpretation::default(),
            extra: serde_json::Map::new(),
        };
        doc.items.push(ProjectItem::Footage(footage));
        id
    }

    /// **Every edit undoes exactly.** A transcript lands, a word is corrected,
    /// a speaker is renamed, and walking the inverses back leaves the document
    /// as it was. Corrections are journalled as these splices, so an inverse
    /// that lost a word would lose it from the project.
    #[test]
    fn every_edit_undoes_exactly() {
        let mut doc = Document::new();
        let id = item(&mut doc);
        let alex = Speaker {
            id: Uuid::now_v7(),
            name: "Speaker 1".into(),
        };
        let mut hello = word("Hello,", 0, 400);
        hello.speaker = Some(alex.id);
        let transcript = Transcript {
            language: "en".into(),
            stream: 0,
            words: vec![hello, word("wrold.", 450, 900)],
            speakers: vec![alex.clone()],
            model: "whisper 1".into(),
            extra: serde_json::Map::new(),
        };
        let empty = doc.clone();

        let mut undo = Vec::new();
        for edit in [
            TranscriptEdit::Set(Some(Arc::new(transcript))),
            TranscriptEdit::Words {
                at: 1,
                remove: 1,
                words: vec![word("world.", 450, 900)],
            },
            TranscriptEdit::Speakers(vec![Speaker {
                name: "Alex".into(),
                ..alex
            }]),
        ] {
            undo.push(apply(&mut doc, id, &edit).unwrap());
        }
        let edited = doc.transcripts[&id].clone();
        assert_eq!(edited.words[1].text, "world.");
        assert_eq!(edited.speaker_name(edited.speakers[0].id), Some("Alex"));
        assert_eq!(edited.word_at(at(500)), Some(1));
        assert_eq!(edited.word_at(at(420)), None, "between two words");
        assert_eq!(edited.words_between(at(300), at(500)), 0..2);
        assert_eq!(rename_speaker(&doc, alex.id, "Sam").len(), 1);
        assert!(rename_speaker(&doc, Uuid::now_v7(), "Nobody").is_empty());

        // A splice past the end, a word over its neighbour, a speaker the
        // list does not have and an item that is not footage change nothing.
        let held = doc.clone();
        let mut stranger = word("who?", 1000, 1200);
        stranger.speaker = Some(Uuid::now_v7());
        for refused in [
            TranscriptEdit::Words {
                at: 5,
                remove: 0,
                words: Vec::new(),
            },
            TranscriptEdit::Words {
                at: 1,
                remove: 0,
                words: vec![word("over", 300, 500)],
            },
            TranscriptEdit::Words {
                at: 2,
                remove: 0,
                words: vec![stranger],
            },
        ] {
            assert!(apply(&mut doc, id, &refused).is_err());
        }
        let nothing = TranscriptEdit::Set(None);
        assert_eq!(
            apply(&mut doc, Uuid::now_v7(), &nothing),
            Err(OpError::UnknownItem)
        );
        assert_eq!(doc, held);
        let names: Vec<&str> = [
            TranscriptEdit::Set(Some(edited)),
            nothing,
            TranscriptEdit::Words {
                at: 0,
                remove: 0,
                words: Vec::new(),
            },
            TranscriptEdit::Speakers(Vec::new()),
        ]
        .iter()
        .map(TranscriptEdit::name)
        .collect();
        assert_eq!(
            names,
            [
                "Transcribe",
                "Delete transcript",
                "Edit transcript",
                "Edit speakers"
            ]
        );

        for op in undo.iter().rev() {
            crate::ops::apply(&mut doc, op).unwrap();
        }
        assert_eq!(doc, empty);
    }
}

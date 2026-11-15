//! Changes to the words themselves: corrections and who said them. The
//! media is never touched.

use lumit_core::time::SourceTime;
use lumit_core::transcript::{splice, Transcript, TranscriptEdit, Word};

use super::*;

/// The grid word times are kept on: a sixteen-thousandth of a second.
const WORD_GRID: i64 = 16_000;

/// `t` on the word grid, the grid line at or before it.
#[frb(ignore)]
fn on_grid(t: Rational) -> Result<Rational, BridgeError> {
    let ticks = (i128::from(t.num()) * i128::from(WORD_GRID)).div_euclid(i128::from(t.den()));
    Rational::from_i128(ticks, i128::from(WORD_GRID)).map_err(|_| BridgeError::InvalidTime)
}

/// The words `texts` become in place of `remove` words from `at`: the same
/// number keep each one's times, speaker and paragraph, any other number is
/// spread evenly over the stretch the old ones took, the first keeping their
/// speaker and paragraph.
#[frb(ignore)]
fn corrected(
    transcript: &Transcript,
    at: usize,
    remove: usize,
    texts: &[String],
) -> Result<Vec<Word>, BridgeError> {
    let old = transcript
        .words
        .get(at..at.checked_add(remove).ok_or(BridgeError::InvalidParam)?)
        .ok_or(BridgeError::InvalidParam)?;
    if texts.len() == old.len() {
        return Ok(old
            .iter()
            .zip(texts)
            .map(|(word, text)| Word {
                text: text.clone(),
                // Somebody typed it, so it is no longer a guess.
                confidence: if *text == word.text {
                    word.confidence
                } else {
                    1.0
                },
                ..word.clone()
            })
            .collect());
    }
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    // An insertion takes the silence between the words either side.
    let (from, to) = match (old.first(), old.last()) {
        (Some(first), Some(last)) => (first.start.0, last.end.0),
        _ => {
            let before = at.checked_sub(1).and_then(|p| transcript.words.get(p));
            let after = transcript.words.get(at);
            let from =
                before.map_or_else(|| after.map_or(Rational::ZERO, |w| w.start.0), |w| w.end.0);
            (from, after.map_or(from, |w| w.start.0.max(from)))
        }
    };
    let span = to.checked_sub(from).map_err(|_| BridgeError::InvalidTime)?;
    let count = i64::try_from(texts.len()).map_err(|_| BridgeError::InvalidParam)?;
    let mut words = Vec::with_capacity(texts.len());
    for (i, text) in (0..).zip(texts) {
        let edge = |k: i64| -> Result<Rational, BridgeError> {
            let part = Rational::new(k, count).map_err(|_| BridgeError::InvalidTime)?;
            let offset = span
                .checked_mul(part)
                .map_err(|_| BridgeError::InvalidTime)?;
            on_grid(
                from.checked_add(offset)
                    .map_err(|_| BridgeError::InvalidTime)?,
            )
        };
        let mut word = Word::new(
            text.clone(),
            SourceTime(edge(i)?),
            SourceTime(edge(i + 1)?),
            1.0,
        );
        if let Some(first) = old.first() {
            word.speaker = first.speaker;
            word.paragraph = i == 0 && first.paragraph;
        }
        words.push(word);
    }
    Ok(words)
}

/// [`ProjectReference::correct_words`].
#[frb(ignore)]
pub(super) fn correct(
    project: &ProjectReference,
    edits: Vec<BridgeWordsEdit>,
) -> Result<(), BridgeError> {
    let state = project.state()?;
    let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
    let doc = state.store.snapshot();
    let mut edits = edits;
    // The latest first in each item, so an earlier one's place holds.
    edits.sort_by(|a, b| (a.item.id(), b.at).cmp(&(b.item.id(), a.at)));
    let mut working: Vec<(Uuid, Transcript)> = Vec::new();
    let mut ops = Vec::with_capacity(edits.len());
    for edit in edits {
        let item = edit.item.id();
        if !working.iter().any(|(id, _)| *id == item) {
            let found = doc.transcripts.get(&item).ok_or(BridgeError::InvalidItem)?;
            working.push((item, (**found).clone()));
        }
        let Some((_, transcript)) = working.iter_mut().find(|(id, _)| *id == item) else {
            continue;
        };
        let (at, remove) = (edit.at as usize, edit.remove as usize);
        let words = corrected(transcript, at, remove, &edit.texts)?;
        *transcript =
            splice(transcript, at, remove, words.clone()).map_err(BridgeError::OpError)?;
        ops.push(Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Words { at, remove, words }),
        });
    }
    if ops.is_empty() {
        return Ok(());
    }
    state
        .store
        .commit(Op::Batch { ops })
        .map_err(BridgeError::OpError)?;
    Ok(())
}

/// [`ProjectReference::set_spoken_speaker`].
#[frb(ignore)]
pub(super) fn set_speaker(
    project: &ProjectReference,
    runs: Vec<BridgeWordRun>,
    speaker: Option<String>,
) -> Result<(), BridgeError> {
    let speaker = speaker
        .map(|s| Uuid::parse_str(&s))
        .transpose()
        .map_err(|_| BridgeError::InvalidParam)?;
    let state = project.state()?;
    let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
    let doc = state.store.snapshot();
    let mut ops = Vec::new();
    let mut joined: Vec<Uuid> = Vec::new();
    for run in runs {
        let item = run.item.id();
        let transcript = doc.transcripts.get(&item).ok_or(BridgeError::InvalidItem)?;
        if let Some(id) = speaker {
            let known = transcript.speakers.iter().any(|s| s.id == id);
            if !known && !joined.contains(&item) {
                let them = doc
                    .transcripts
                    .values()
                    .flat_map(|t| t.speakers.iter())
                    .find(|s| s.id == id)
                    .ok_or(BridgeError::InvalidParam)?;
                let mut list = transcript.speakers.clone();
                list.push(them.clone());
                ops.push(Op::EditTranscript {
                    item,
                    edit: Box::new(TranscriptEdit::Speakers(list)),
                });
                joined.push(item);
            }
        }
        let (from, to) = (run.from as usize, run.to as usize);
        let words = transcript
            .words
            .get(from..=to)
            .ok_or(BridgeError::InvalidParam)?
            .iter()
            .map(|w| Word {
                speaker,
                ..w.clone()
            })
            .collect::<Vec<_>>();
        ops.push(Op::EditTranscript {
            item,
            edit: Box::new(TranscriptEdit::Words {
                at: from,
                remove: words.len(),
                words,
            }),
        });
    }
    if ops.is_empty() {
        return Ok(());
    }
    state
        .store
        .commit(Op::Batch { ops })
        .map_err(BridgeError::OpError)?;
    Ok(())
}

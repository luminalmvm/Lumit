//! Lyric (`.lrc`) files: read.
//!
//! # In plain terms
//!
//! A song's words, one line to a moment: `[01:02.50]and the line`. Each line
//! becomes a caption that stays up until the next one starts. An enhanced file
//! also times each word, `<01:02.80>`, and those times are kept, so the words
//! of a lyric caption can light up as they are sung.
//!
//! # Thread role and contract
//!
//! Pure text in and out. No IO.

use crate::time::Rational;

use super::{
    cues::{Cue, CueWord},
    files::{clean, cue_of_lines},
    CaptionsError,
};

/// How long the last line stays up, with nothing after it to end it.
const LAST_SECONDS: i64 = 4;

/// The captions in an `.lrc` file, in order of time.
///
/// # Errors
///
/// [`CaptionsError::Empty`] for a file with no timed lines.
pub fn read(text: &str) -> Result<Vec<Cue>, CaptionsError> {
    let text = text.trim_start_matches('\u{feff}');
    // Every timed line, with each moment it is sung at.
    let mut sung: Vec<(Rational, String)> = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut moments = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                break;
            };
            match time(&after[..close]) {
                Some(at) => moments.push(at),
                // A tag such as [ar:...] carries no words.
                None if moments.is_empty() => {
                    rest = "";
                    break;
                }
                None => break,
            }
            rest = &after[close + 1..];
        }
        for at in moments {
            sung.push((at, rest.to_owned()));
        }
    }
    sung.sort_by_key(|(at, _)| *at);
    let mut cues = Vec::new();
    for (i, (start, words)) in sung.iter().enumerate() {
        let end = sung.get(i + 1).map_or_else(
            || {
                start
                    .checked_add(Rational::new(LAST_SECONDS, 1).unwrap_or(Rational::ONE))
                    .unwrap_or(*start)
            },
            |(next, _)| *next,
        );
        if let Some(cue) = timed(words, *start, end) {
            cues.push(cue);
        }
    }
    if cues.is_empty() {
        return Err(CaptionsError::Empty);
    }
    Ok(cues)
}

/// One line as a cue, with the word times an enhanced file writes in front of
/// its words.
fn timed(line: &str, start: Rational, end: Rational) -> Option<Cue> {
    let mut plain = String::new();
    let mut moments: Vec<Option<Rational>> = Vec::new();
    let mut pending = None;
    let mut word_open = false;
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<') {
            if let Some(close) = after.find('>') {
                if let Some(at) = time(&after[..close]) {
                    pending = Some(at);
                    rest = &after[close + 1..];
                    continue;
                }
            }
        }
        let mut chars = rest.chars();
        let Some(ch) = chars.next() else {
            break;
        };
        if ch.is_whitespace() {
            word_open = false;
        } else if !word_open {
            moments.push(pending.take());
            word_open = true;
        }
        plain.push(ch);
        rest = chars.as_str();
    }
    let mut cue = cue_of_lines(&[clean(&plain)], start, end)?;
    if moments.len() == cue.words.len() && moments.iter().any(Option::is_some) {
        let mut at = start;
        let starts: Vec<Rational> = moments
            .iter()
            .map(|m| {
                at = m.map_or(at, |m| m.clamp(start, end).max(at));
                at
            })
            .collect();
        let count = cue.words.len();
        for (i, word) in cue.words.iter_mut().enumerate() {
            let next = if i + 1 < count { starts[i + 1] } else { end };
            *word = CueWord {
                start: starts[i],
                end: next.max(starts[i]),
                ..word.clone()
            };
        }
    }
    Some(cue)
}

/// `mm:ss.xx`, or `mm:ss`.
fn time(text: &str) -> Option<Rational> {
    let (minutes, seconds) = text.trim().split_once(':')?;
    let minutes: i64 = minutes.parse().ok()?;
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, "0"));
    let whole: i64 = whole.parse().ok()?;
    let digits = u32::try_from(fraction.len()).ok()?;
    let scale = 10_i64.checked_pow(digits)?;
    let part: i64 = fraction.parse().ok()?;
    let total = minutes.checked_mul(60)?.checked_add(whole)?;
    Rational::new(total.checked_mul(scale)?.checked_add(part)?, scale).ok()
}

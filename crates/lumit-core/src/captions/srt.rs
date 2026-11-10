//! SubRip (`.srt`) captions: read and written.
//!
//! # In plain terms
//!
//! The plainest subtitle file there is: a number, a start and end time, the
//! lines, and a blank line. Players everywhere read it, so it is what most
//! people want beside a video. It has no word timing, so a file read in has its
//! words spread over each caption by length.
//!
//! # Thread role and contract
//!
//! Pure text in and out. No IO.

use crate::time::Rational;

use super::{
    cues::Cue,
    files::{clean, cue_of_lines, stamp, Stamp},
    CaptionsError,
};

/// `cues` as an `.srt` file.
#[must_use]
pub fn write(cues: &[Cue]) -> String {
    let mut out = String::new();
    let mut number = 0;
    for cue in cues {
        let lines = cue.lines();
        if lines.is_empty() {
            continue;
        }
        number += 1;
        out.push_str(&number.to_string());
        out.push('\n');
        out.push_str(&stamp(cue.start, Stamp::Srt));
        out.push_str(" --> ");
        out.push_str(&stamp(cue.end.max(cue.start), Stamp::Srt));
        out.push('\n');
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// The captions in an `.srt` file, in order of time.
///
/// Forgiving the way players are: a missing number, a full stop instead of a
/// comma before the milliseconds, Windows line ends, a byte order mark, and
/// the italic and colour tags people put in are all read; the tags are
/// dropped.
///
/// # Errors
///
/// [`CaptionsError::Unreadable`] with the line that broke, and
/// [`CaptionsError::Empty`] for a file with no captions in it.
pub fn read(text: &str) -> Result<Vec<Cue>, CaptionsError> {
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut cues = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        // The number is optional; the timing line is not.
        let timing = if lines[i].contains("-->") {
            i
        } else if lines.get(i + 1).is_some_and(|l| l.contains("-->")) {
            i + 1
        } else {
            return Err(CaptionsError::Unreadable { line: i + 1 });
        };
        let (start, end) =
            timings(lines[timing]).ok_or(CaptionsError::Unreadable { line: timing + 1 })?;
        let mut body = Vec::new();
        i = timing + 1;
        while i < lines.len() && !lines[i].trim().is_empty() {
            body.push(clean(lines[i]));
            i += 1;
        }
        if let Some(cue) = cue_of_lines(&body, start, end) {
            cues.push(cue);
        }
    }
    if cues.is_empty() {
        return Err(CaptionsError::Empty);
    }
    cues.sort_by_key(|c| c.start);
    Ok(cues)
}

/// The two times on a timing line, ignoring anything written after them.
pub(super) fn timings(line: &str) -> Option<(Rational, Rational)> {
    let (from, rest) = line.split_once("-->")?;
    let to = rest.split_whitespace().next()?;
    let start = super::files::parse_stamp(from.trim())?;
    let end = super::files::parse_stamp(to.trim())?;
    Some((start, end.max(start)))
}

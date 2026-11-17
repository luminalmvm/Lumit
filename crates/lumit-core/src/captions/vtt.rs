//! WebVTT (`.vtt`) captions: read and written.
//!
//! # In plain terms
//!
//! The web's subtitle file. Like SubRip, with one addition that matters here:
//! a caption can carry the moment each word is said, as a time written in
//! front of it. Lumit writes those, so a player that highlights words as they
//! are spoken can, and reads them back, so a file made elsewhere keeps its
//! word timing.
//!
//! # Thread role and contract
//!
//! Pure text in and out. No IO.

use crate::time::Rational;

use super::{
    cues::{Cue, CueWord},
    files::{clean, cue_of_lines, decode_entities, stamp, Stamp},
    srt::timings,
    CaptionsError,
};

/// `cues` as a `.vtt` file in `language`, a code such as `en` (left out when
/// empty). Every word after the first carries the moment it is said.
#[must_use]
pub fn write(cues: &[Cue], language: &str) -> String {
    let mut out = String::from("WEBVTT\n");
    if !language.is_empty() {
        out.push_str("Language: ");
        out.push_str(language);
        out.push('\n');
    }
    out.push('\n');
    for cue in cues {
        let lines = timed_lines(cue);
        if lines.is_empty() {
            continue;
        }
        out.push_str(&stamp(cue.start, Stamp::Vtt));
        out.push_str(" --> ");
        out.push_str(&stamp(cue.end.max(cue.start), Stamp::Vtt));
        out.push('\n');
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// A cue's lines, with a time tag in front of every word but the first, and
/// the three characters the format reserves written as entities.
fn timed_lines(cue: &Cue) -> Vec<String> {
    let mut timed = cue.clone();
    for (i, word) in timed.words.iter_mut().enumerate() {
        let text = escape(&word.text);
        word.text = if i == 0 || word.start <= cue.start {
            text
        } else {
            format!("<{}>{text}", stamp(word.start, Stamp::Vtt))
        };
    }
    timed.label = timed.label.as_deref().map(escape);
    timed.lines()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The captions in a `.vtt` file, in order of time, with the word times it
/// carries.
///
/// Notes, styles and regions are skipped, and so are a cue's settings. A
/// voice tag becomes the caption's speaker label; every other tag is dropped.
///
/// # Errors
///
/// [`CaptionsError::Unreadable`] with the line that broke, and
/// [`CaptionsError::Empty`] for a file with no captions in it.
pub fn read(text: &str) -> Result<Vec<Cue>, CaptionsError> {
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    if !lines.first().is_some_and(|l| l.starts_with("WEBVTT")) {
        return Err(CaptionsError::Unreadable { line: 1 });
    }
    let mut cues = Vec::new();
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.is_empty() {
            i += 1;
            continue;
        }
        if line.starts_with("NOTE") || line.starts_with("STYLE") || line.starts_with("REGION") {
            while i < lines.len() && !lines[i].trim().is_empty() {
                i += 1;
            }
            continue;
        }
        // A header line before the first blank line, or an identifier.
        let timing = if line.contains("-->") {
            i
        } else if lines.get(i + 1).is_some_and(|l| l.contains("-->")) {
            i + 1
        } else {
            i += 1;
            continue;
        };
        let (start, end) =
            timings(lines[timing]).ok_or(CaptionsError::Unreadable { line: timing + 1 })?;
        let mut body = Vec::new();
        i = timing + 1;
        while i < lines.len() && !lines[i].trim().is_empty() {
            body.push(lines[i]);
            i += 1;
        }
        if let Some(cue) = timed_cue(&body, start, end) {
            cues.push(cue);
        }
    }
    if cues.is_empty() {
        return Err(CaptionsError::Empty);
    }
    cues.sort_by_key(|c| c.start);
    Ok(cues)
}

/// One cue from its text lines, its word times taken from the time tags where
/// it has them.
fn timed_cue(body: &[&str], start: Rational, end: Rational) -> Option<Cue> {
    let mut label = None;
    let mut plain = Vec::with_capacity(body.len());
    // The moment written in front of each word, in reading order.
    let mut moments: Vec<Option<Rational>> = Vec::new();
    for line in body {
        let (text, line_moments, voice) = untag(line);
        if label.is_none() {
            label = voice;
        }
        plain.push(text);
        moments.extend(line_moments);
    }
    let mut cue = cue_of_lines(&plain, start, end)?;
    if moments.len() == cue.words.len() && moments.iter().any(Option::is_some) {
        retime(&mut cue.words, &moments, start, end);
    }
    cue.label = label.map(|name| format!("{name}:"));
    Some(cue)
}

/// A line's text without its tags, the moment written in front of each of its
/// words, and the name of a voice tag if it has one.
fn untag(line: &str) -> (String, Vec<Option<Rational>>, Option<String>) {
    let mut text = String::new();
    let mut moments = Vec::new();
    let mut voice = None;
    let mut pending: Option<Rational> = None;
    let mut word_open = false;
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<') {
            let Some(close) = after.find('>') else {
                text.push_str(&decode_entities(rest));
                break;
            };
            let tag = &after[..close];
            if let Some(name) = tag.strip_prefix("v ").or_else(|| tag.strip_prefix("v.")) {
                let name = name.split_once(' ').map_or(name, |(_, n)| n).trim();
                if !name.is_empty() {
                    voice = Some(decode_entities(name));
                }
            } else if let Some(moment) = super::files::parse_stamp(tag) {
                pending = Some(moment);
            }
            rest = &after[close + 1..];
            continue;
        }
        let next = rest.find('<').unwrap_or(rest.len());
        let piece = decode_entities(&rest[..next]);
        for ch in piece.chars() {
            if ch.is_whitespace() {
                word_open = false;
            } else if !word_open {
                moments.push(pending.take());
                word_open = true;
            }
            text.push(ch);
        }
        rest = &rest[next..];
    }
    (clean(&text), moments, voice)
}

/// Words given the moments written in front of them: each starts at its own
/// moment, or where the one before ended, and runs to the next word's start.
fn retime(words: &mut [CueWord], moments: &[Option<Rational>], start: Rational, end: Rational) {
    let mut at = start;
    let starts: Vec<Rational> = moments
        .iter()
        .map(|m| {
            at = m.map_or(at, |m| m.clamp(start, end).max(at));
            at
        })
        .collect();
    for (i, word) in words.iter_mut().enumerate() {
        word.start = starts[i];
        word.end = starts.get(i + 1).copied().unwrap_or(end).max(word.start);
    }
}

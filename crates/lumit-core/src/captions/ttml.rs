//! Timed Text (`.ttml`, `.dfxp`, `.xml`) captions: read and written.
//!
//! # In plain terms
//!
//! The XML subtitle file broadcasters and streaming services ask for. Lumit
//! writes the plain text profile: one paragraph a caption, its lines broken
//! with `<br/>`, its times as clock times. Reading takes the paragraphs of any
//! TTML file, whatever styling or regions it carries, and keeps their words
//! and times.
//!
//! # Thread role and contract
//!
//! Pure text in and out. No IO.

use crate::time::Rational;

use super::{
    cues::Cue,
    files::{clean, cue_of_lines, decode_entities, stamp, Stamp},
    CaptionsError,
};

/// `cues` as a TTML file in `language`, a code such as `en`.
#[must_use]
pub fn write(cues: &[Cue], language: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tt xmlns=\"http://www.w3.org/ns/ttml\"",
    );
    if !language.is_empty() {
        out.push_str(" xml:lang=\"");
        out.push_str(&escape(language));
        out.push('"');
    }
    out.push_str(">\n  <body>\n    <div>\n");
    for cue in cues {
        let lines = cue.lines();
        if lines.is_empty() {
            continue;
        }
        out.push_str("      <p begin=\"");
        out.push_str(&stamp(cue.start, Stamp::Vtt));
        out.push_str("\" end=\"");
        out.push_str(&stamp(cue.end.max(cue.start), Stamp::Vtt));
        out.push_str("\">");
        let lines: Vec<String> = lines.iter().map(|l| escape(l)).collect();
        out.push_str(&lines.join("<br/>"));
        out.push_str("</p>\n");
    }
    out.push_str("    </div>\n  </body>\n</tt>\n");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The captions in a TTML file, in order of time.
///
/// # Errors
///
/// [`CaptionsError::Unreadable`] with the line of a paragraph whose times will
/// not read, and [`CaptionsError::Empty`] for a file with no paragraphs.
pub fn read(text: &str) -> Result<Vec<Cue>, CaptionsError> {
    let rate = frame_rate(text);
    let mut cues = Vec::new();
    let mut rest = text;
    let mut consumed = 0;
    while let Some(at) = find_tag(rest, "p") {
        let tag_end = rest[at..]
            .find('>')
            .map(|e| at + e)
            .ok_or(CaptionsError::Unreadable {
                line: line_of(text, consumed + at),
            })?;
        let tag = &rest[at..=tag_end];
        let close =
            rest[tag_end..]
                .find("</p>")
                .map(|e| tag_end + e)
                .ok_or(CaptionsError::Unreadable {
                    line: line_of(text, consumed + at),
                })?;
        let body = &rest[tag_end + 1..close];
        let line = line_of(text, consumed + at);
        let start = attribute(tag, "begin")
            .and_then(|b| time(&b, rate))
            .ok_or(CaptionsError::Unreadable { line })?;
        let end = match attribute(tag, "end").and_then(|e| time(&e, rate)) {
            Some(end) => end,
            None => attribute(tag, "dur")
                .and_then(|d| time(&d, rate))
                .and_then(|d| start.checked_add(d).ok())
                .ok_or(CaptionsError::Unreadable { line })?,
        };
        let lines: Vec<String> = split_breaks(body).iter().map(|l| clean(l)).collect();
        if let Some(cue) = cue_of_lines(&lines, start, end.max(start)) {
            cues.push(cue);
        }
        consumed += close + 4;
        rest = &rest[close + 4..];
    }
    if cues.is_empty() {
        return Err(CaptionsError::Empty);
    }
    cues.sort_by_key(|c| c.start);
    Ok(cues)
}

/// Where the next `<name` element opens, with or without a namespace prefix.
fn find_tag(text: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = text[from..].find('<') {
        let at = from + at;
        let rest = &text[at + 1..];
        // The element's own name and no further, so a long file is read once.
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == ':'))
            .unwrap_or(rest.len());
        let local = rest[..end].rsplit(':').next().unwrap_or_default();
        if local == name && rest[end..].starts_with(|c: char| c.is_whitespace() || c == '>') {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// The value of `name="..."` in a tag, entities written out.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = tag[from..].find(name) {
        let at = from + at;
        let before = tag[..at].chars().next_back();
        let after = &tag[at + name.len()..];
        if before.is_some_and(char::is_whitespace) {
            if let Some(value) = after.trim_start().strip_prefix('=') {
                let value = value.trim_start();
                let quote = value.chars().next()?;
                if quote == '"' || quote == '\'' {
                    let value = &value[1..];
                    let end = value.find(quote)?;
                    return Some(decode_entities(&value[..end]));
                }
            }
        }
        from = at + name.len();
    }
    None
}

/// The paragraph's lines: split at every `<br/>`, the other tags left for
/// [`clean`] to drop.
fn split_breaks(body: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut rest = body;
    while let Some(at) = rest.find('<') {
        if let Some(line) = lines.last_mut() {
            line.push_str(&rest[..at]);
        }
        let end = rest[at..].find('>').map_or(rest.len(), |e| at + e + 1);
        let tag = &rest[at..end];
        let name = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("");
        if name.rsplit(':').next() == Some("br") {
            lines.push(String::new());
        }
        rest = &rest[end..];
    }
    if let Some(line) = lines.last_mut() {
        line.push_str(rest);
    }
    lines
}

/// The frame rate a file's frame-counted times are in, from its
/// `ttp:frameRate`, or thirty.
fn frame_rate(text: &str) -> i64 {
    find_tag(text, "tt")
        .and_then(|at| {
            let tag = &text[at..at + text[at..].find('>')?];
            attribute(tag, "ttp:frameRate")
        })
        .and_then(|r| r.trim().parse::<i64>().ok())
        .filter(|r| *r > 0)
        .unwrap_or(30)
}

/// A TTML time: a clock time with fractions or frames, or an offset in hours,
/// minutes, seconds, milliseconds or frames.
fn time(text: &str, rate: i64) -> Option<Rational> {
    let text = text.trim();
    if let Some(at) = super::files::parse_stamp(text) {
        return Some(at);
    }
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() == 4 {
        // hh:mm:ss:ff
        let n: Vec<i64> = parts
            .iter()
            .map(|p| p.parse::<i64>().ok())
            .collect::<Option<_>>()?;
        let seconds = n[0]
            .checked_mul(3600)?
            .checked_add(n[1].checked_mul(60)?)?
            .checked_add(n[2])?;
        return Rational::new(seconds.checked_mul(rate)?.checked_add(n[3])?, rate).ok();
    }
    let split = text.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = text.split_at(split);
    let value: f64 = number.trim().parse().ok()?;
    let seconds = match unit {
        "h" => value * 3600.0,
        "m" => value * 60.0,
        "s" => value,
        "ms" => value / 1000.0,
        #[allow(clippy::cast_precision_loss)] // a frame rate
        "f" => value / rate as f64,
        _ => return None,
    };
    Rational::from_f64_on_grid(seconds.max(0.0), 1000).ok()
}

/// The line, counted from one, that byte `at` of `text` is on.
fn line_of(text: &str, at: usize) -> usize {
    text.get(..at).map_or(1, |t| t.matches('\n').count() + 1)
}

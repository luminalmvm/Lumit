//! Advanced SubStation (`.ass`, `.ssa`) captions: read.
//!
//! # In plain terms
//!
//! The subtitle file fansubbers and many editors write, with its own styling
//! language. Lumit reads each dialogue line's times and words and leaves its
//! styling behind, since a caption layer is styled in Lumit.
//!
//! # Thread role and contract
//!
//! Pure text in and out. No IO.

use crate::time::Rational;

use super::{
    cues::Cue,
    files::{clean, cue_of_lines},
    CaptionsError,
};

/// The captions in an `.ass` or `.ssa` file, in order of time.
///
/// # Errors
///
/// [`CaptionsError::Unreadable`] with the line of a dialogue line whose times
/// will not read, and [`CaptionsError::Empty`] for a file with none.
pub fn read(text: &str) -> Result<Vec<Cue>, CaptionsError> {
    let text = text.trim_start_matches('\u{feff}');
    let mut in_events = false;
    // Where the start, end and text fields are, from the section's Format line.
    let mut fields: Option<(usize, usize, usize, usize)> = None;
    let mut cues = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(format) = line.strip_prefix("Format:") {
            let names: Vec<String> = format.split(',').map(|n| n.trim().to_lowercase()).collect();
            let at = |name: &str| names.iter().position(|n| n == name);
            fields = Some((
                at("start").unwrap_or(1),
                at("end").unwrap_or(2),
                at("text").unwrap_or(names.len().saturating_sub(1)),
                names.len(),
            ));
            continue;
        }
        let Some(dialogue) = line.strip_prefix("Dialogue:") else {
            continue;
        };
        let (start_at, end_at, text_at, count) = fields.unwrap_or((1, 2, 9, 10));
        // The text is the last field and may itself hold commas.
        let parts: Vec<&str> = dialogue.splitn(count.max(text_at + 1), ',').collect();
        let unreadable = CaptionsError::Unreadable { line: number + 1 };
        let start = parts
            .get(start_at)
            .and_then(|t| time(t))
            .ok_or(unreadable.clone())?;
        let end = parts.get(end_at).and_then(|t| time(t)).ok_or(unreadable)?;
        let body = parts.get(text_at).copied().unwrap_or("");
        let body = body
            .replace("\\N", "\n")
            .replace("\\n", "\n")
            .replace("\\h", " ");
        let lines: Vec<String> = body.lines().map(clean).collect();
        if let Some(cue) = cue_of_lines(&lines, start, end.max(start)) {
            cues.push(cue);
        }
    }
    if cues.is_empty() {
        return Err(CaptionsError::Empty);
    }
    cues.sort_by_key(|c| c.start);
    Ok(cues)
}

/// `h:mm:ss.cc`, the hundredths the format counts in.
fn time(text: &str) -> Option<Rational> {
    let text = text.trim();
    let (clock, hundredths) = text.split_once('.').unwrap_or((text, "0"));
    let parts: Vec<i64> = clock
        .split(':')
        .map(|p| p.trim().parse::<i64>().ok())
        .collect::<Option<_>>()?;
    let seconds = parts
        .iter()
        .try_fold(0i64, |total, p| total.checked_mul(60)?.checked_add(*p))?;
    let digits = u32::try_from(hundredths.len()).ok()?;
    let scale = 10_i64.checked_pow(digits)?;
    let part: i64 = hundredths.parse().ok()?;
    Rational::new(seconds.checked_mul(scale)?.checked_add(part)?, scale).ok()
}

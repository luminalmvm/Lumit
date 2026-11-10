//! Words into captions: how many to a caption, where the lines break, and how
//! long each one stays up.
//!
//! # In plain terms
//!
//! Speech arrives as a stream of timed words. A caption is a few of them shown
//! together, on one or two lines, for long enough to read. [`build`] cuts the
//! stream into captions that fit the [`Limits`], ending them where the speech
//! pauses or a sentence ends wherever it can, and never across a change of
//! speaker when asked not to.
//!
//! Pure, and on any clock: the words' times can be a footage item's source
//! time, a layer's time or a composition's, and the captions come back on the
//! same one. Cut's transcript export calls this too.
//!
//! # Thread role and contract
//!
//! Pure functions over borrowed words. No IO, no clocks, no threads.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time::Rational;

/// One timed word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueWord {
    /// As shown, punctuation attached, no space in front.
    pub text: String,
    pub start: Rational,
    pub end: Rational,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Uuid>,
    /// Picked out by hand, for a caption style to colour.
    #[serde(default, skip_serializing_if = "is_false")]
    pub emphasis: bool,
}

/// Words shown together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    /// When it appears and goes.
    pub start: Rational,
    pub end: Rational,
    pub words: Vec<CueWord>,
    /// The words that start a new line, by index. Empty is one line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<usize>,
    /// Printed in front of the first line: a speaker's name with its colon,
    /// or nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Lines start with a dash, one per speaker, when two people share it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dashes: bool,
    /// The footage item whose source time these times are in, for captions
    /// that follow the layer they were made from. Unset for every other clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Uuid>,
    /// How far this caption is moved from where the layer puts it, in the
    /// layer's own pixels, right and down. Unset is where the layer puts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f64; 2]>,
    /// Unknown fields from newer Lumit versions.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// How speakers show in the captions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SpeakerLabels {
    /// Not at all.
    #[default]
    Off,
    /// The speaker's name in front of what they say, whenever it changes.
    Names,
    /// A dash in front of each speaker's line, when two share a caption.
    Dashes,
}

/// What a caption may hold, and how long it stays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// The longest a line may be, in characters. A single word longer than
    /// this still gets a line of its own.
    pub chars_per_line: u32,
    /// How many lines a caption may have.
    pub lines: u32,
    /// The most words a caption may hold. Nought is no limit, one is a word at
    /// a time.
    pub words: u32,
    /// The shortest a caption stays up, in seconds, room permitting.
    pub min_duration: Rational,
    /// The longest a caption may run, in seconds. A single word may run over.
    pub max_duration: Rational,
    /// The least time between one caption going and the next appearing.
    pub gap: Rational,
    /// Characters a second a caption should be readable at. It stays up longer
    /// to meet it, room permitting. Nought leaves it out.
    pub reading_speed: u32,
    /// Start a new caption whenever somebody else speaks.
    pub split_on_speaker: bool,
    pub speaker_labels: SpeakerLabels,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            chars_per_line: 42,
            lines: 2,
            words: 0,
            min_duration: Rational::ONE,
            max_duration: Rational::new(7, 1).unwrap_or(Rational::ONE),
            gap: Rational::ZERO,
            reading_speed: 20,
            split_on_speaker: true,
            speaker_labels: SpeakerLabels::Off,
        }
    }
}

impl Limits {
    /// The usual limits for captions in `language`: a shorter line for the
    /// languages written without spaces, where every character carries more.
    #[must_use]
    pub fn for_language(language: &str) -> Self {
        let dense = matches!(language, "zh" | "ja" | "ko" | "yue");
        Self {
            chars_per_line: if dense { 16 } else { 42 },
            reading_speed: if dense { 9 } else { 20 },
            ..Self::default()
        }
    }
}

/// A pause at least this long ends a caption whatever else fits.
const PAUSE: f64 = 1.5;

/// Whether `c` belongs to a script written without spaces between words.
#[must_use]
pub(super) fn unspaced(c: char) -> bool {
    matches!(u32::from(c),
        0x0E00..=0x0EFF      // Thai and Lao
        | 0x1000..=0x109F    // Myanmar
        | 0x1780..=0x17FF    // Khmer
        | 0x3000..=0x30FF    // CJK punctuation, hiragana, katakana
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF
        | 0x20000..=0x3FFFF)
}

/// Whether a space goes between `before` and `after` when they are put side
/// by side: always, unless either side is in a script written without them.
#[must_use]
pub fn spaced(before: &str, after: &str) -> bool {
    let last = before.chars().next_back();
    let first = after.chars().next();
    !(last.is_some_and(unspaced) || first.is_some_and(unspaced))
}

/// Words put side by side, with spaces where their scripts want them.
#[must_use]
pub fn join<'a>(words: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    let mut previous: Option<&str> = None;
    for word in words {
        if let Some(previous) = previous {
            if spaced(previous, word) {
                out.push(' ');
            }
        }
        out.push_str(word);
        previous = Some(word);
    }
    out
}

/// How many characters `words` take on one line.
fn width(words: &[CueWord]) -> usize {
    let mut total = 0;
    for (i, word) in words.iter().enumerate() {
        total += word.text.chars().count();
        if i > 0 && spaced(&words[i - 1].text, &word.text) {
            total += 1;
        }
    }
    total
}

impl Cue {
    /// The caption as it reads: one string per line, the label in front of
    /// the first and a dash in front of each when the cue has dashes.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut from = 0;
        for &at in self.breaks.iter().chain(std::iter::once(&self.words.len())) {
            let at = at.clamp(from, self.words.len());
            if at > from {
                lines.push(join(self.words[from..at].iter().map(|w| w.text.as_str())));
            }
            from = at;
        }
        if self.dashes {
            for line in &mut lines {
                line.insert_str(0, "- ");
            }
        }
        if let (Some(label), Some(first)) = (&self.label, lines.first_mut()) {
            if !label.is_empty() {
                first.insert(0, ' ');
                first.insert_str(0, label);
            }
        }
        lines
    }

    /// The caption as one string, its lines on lines of their own.
    #[must_use]
    pub fn text(&self) -> String {
        self.lines().join("\n")
    }

    /// Characters a second it asks of a reader, the way a subtitler counts
    /// them: every character but the line breaks.
    #[must_use]
    pub fn reading_speed(&self) -> f64 {
        let chars: usize = self.lines().iter().map(|l| l.chars().count()).sum();
        let seconds = self
            .end
            .checked_sub(self.start)
            .map_or(0.0, Rational::to_f64);
        if seconds <= 0.0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)] // a caption is never 2^52 characters
        let chars = chars as f64;
        chars / seconds
    }

    /// The cue with its lines broken again to fit `limits`.
    #[must_use]
    pub fn rebroken(mut self, limits: &Limits) -> Self {
        self.breaks = breaks(&self.words, limits, self.dashes);
        self
    }
}

/// Cut `words` into captions that fit `limits`.
///
/// `names` gives a speaker's name for [`SpeakerLabels::Names`]. The words must
/// be in order of time; they come back in the same order, every one of them in
/// exactly one caption.
#[must_use]
pub fn build(
    words: &[CueWord],
    limits: &Limits,
    names: &dyn Fn(Uuid) -> Option<String>,
) -> Vec<Cue> {
    let mut cues: Vec<Cue> = Vec::new();
    for (from, to) in passages(words, limits) {
        for (start, end) in group(&words[from..to], limits) {
            let piece = &words[from + start..from + end];
            let speakers = speakers_in(piece);
            let dashes = limits.speaker_labels == SpeakerLabels::Dashes && speakers > 1;
            cues.push(Cue {
                start: piece[0].start,
                end: piece[piece.len() - 1].end,
                words: piece.to_vec(),
                breaks: breaks(piece, limits, dashes),
                label: None,
                dashes,
                item: None,
                offset: None,
                extra: serde_json::Map::new(),
            });
        }
    }
    label(&mut cues, limits, names);
    time(&mut cues, limits);
    cues
}

/// How many different speakers a run of words has.
fn speakers_in(words: &[CueWord]) -> usize {
    let mut changes = 0;
    for pair in words.windows(2) {
        if pair[0].speaker != pair[1].speaker {
            changes += 1;
        }
    }
    changes + 1
}

/// The stretches no caption may cross: a long pause, or a new speaker when the
/// limits ask for that. Each is `(from, to)` into `words`.
fn passages(words: &[CueWord], limits: &Limits) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    for i in 1..words.len() {
        let pause = words[i]
            .start
            .checked_sub(words[i - 1].end)
            .map_or(0.0, Rational::to_f64);
        let new_speaker = limits.split_on_speaker && words[i].speaker != words[i - 1].speaker;
        if pause >= PAUSE || new_speaker {
            out.push((from, i));
            from = i;
        }
    }
    if from < words.len() {
        out.push((from, words.len()));
    }
    out
}

/// Whether a word ends a sentence, or at least a clause.
fn ending(word: &str) -> (bool, bool) {
    let last = word
        .trim_end_matches(['"', '\'', ')', ']', '”', '’', '»', '」', '』'])
        .chars()
        .next_back();
    let sentence = matches!(last, Some('.' | '?' | '!' | '…' | '。' | '？' | '！'));
    let clause = matches!(
        last,
        Some(',' | ';' | ':' | '，' | '、' | '；' | '：' | '-' | '—')
    );
    (sentence, clause)
}

/// Whether `words` fit one caption under `limits`.
fn fits(words: &[CueWord], limits: &Limits) -> bool {
    if words.len() <= 1 {
        return true;
    }
    if limits.words > 0 && words.len() > limits.words as usize {
        return false;
    }
    let seconds = words[words.len() - 1]
        .end
        .checked_sub(words[0].start)
        .map_or(0.0, Rational::to_f64);
    if seconds > limits.max_duration.to_f64() {
        return false;
    }
    lay(words, limits).is_some()
}

/// The fewest lines `words` fill at `limits.chars_per_line`, if that is
/// within `limits.lines`, each line as the index of its first word.
fn lay(words: &[CueWord], limits: &Limits) -> Option<Vec<usize>> {
    let per_line = limits.chars_per_line.max(1) as usize;
    let mut starts = vec![0];
    let mut from = 0;
    for i in 1..words.len() {
        if width(&words[from..=i]) > per_line {
            starts.push(i);
            from = i;
        }
    }
    (starts.len() <= limits.lines.max(1) as usize).then_some(starts)
}

/// The best way to cut one passage into captions, each as `(from, to)` into
/// it.
///
/// Every possible ending is weighed: a fuller caption is better, one that ends
/// a sentence or a clause, or at a pause, better still, and one too short to
/// read worse. The cheapest set of endings wins. A caption holds at most a
/// couple of lines of words, so each word looks back only that far.
fn group(words: &[CueWord], limits: &Limits) -> Vec<(usize, usize)> {
    let n = words.len();
    let room = (limits.chars_per_line.max(1) * limits.lines.max(1)) as f64;
    let mut best = vec![f64::INFINITY; n + 1];
    let mut from = vec![0usize; n + 1];
    best[0] = 0.0;
    for end in 1..=n {
        let mut start = end;
        while start > 0 {
            start -= 1;
            let piece = &words[start..end];
            if !fits(piece, limits) {
                break;
            }
            let cost = best[start] + cost(piece, words.get(end), limits, room);
            if cost < best[end] {
                best[end] = cost;
                from[end] = start;
            }
        }
        if best[end].is_infinite() {
            // Nothing fits, which only a single word can cause. It goes alone.
            best[end] = best[end - 1] + 1.0;
            from[end] = end - 1;
        }
    }
    let mut out = Vec::new();
    let mut end = n;
    while end > 0 {
        out.push((from[end], end));
        end = from[end];
    }
    out.reverse();
    out
}

/// What one caption of `piece` costs, `next` being the word after it.
fn cost(piece: &[CueWord], next: Option<&CueWord>, limits: &Limits, room: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)] // a line is never 2^52 characters
    let fill = width(piece) as f64 / room;
    let mut cost = (1.0 - fill.min(1.0)).powi(2);
    let Some(next) = next else {
        // The last caption of a passage ends where the passage does.
        return cost;
    };
    let pause = next
        .start
        .checked_sub(piece[piece.len() - 1].end)
        .map_or(0.0, Rational::to_f64);
    let (sentence, clause) = ending(&piece[piece.len() - 1].text);
    cost += if sentence {
        0.0
    } else if clause {
        0.25
    } else {
        0.6
    };
    cost -= pause.min(0.5) * 0.5;
    let seconds = piece[piece.len() - 1]
        .end
        .checked_sub(piece[0].start)
        .map_or(0.0, Rational::to_f64);
    if seconds < limits.min_duration.to_f64() {
        cost += 0.3;
    }
    if limits.words == 1 {
        // A word at a time has nothing to weigh: every word is its own.
        cost = 0.0;
    }
    cost
}

/// Where a caption's lines start, by word.
///
/// With dashes, a line starts wherever the speaker changes and nowhere else.
/// Otherwise the words are laid on as few lines as fit, and between those the
/// break is moved to where the lines come out most even, the lower line the
/// longer of two, and after a comma or a full stop where one is near.
fn breaks(words: &[CueWord], limits: &Limits, dashes: bool) -> Vec<usize> {
    if dashes {
        return (1..words.len())
            .filter(|&i| words[i].speaker != words[i - 1].speaker)
            .collect();
    }
    let Some(starts) = lay(words, limits) else {
        // Too long for the lines: one line per line's worth.
        return greedy(words, limits);
    };
    let lines = starts.len();
    if lines <= 1 {
        return Vec::new();
    }
    let per_line = limits.chars_per_line.max(1) as usize;
    // Every way to cut the words into exactly `lines` lines that each fit,
    // scored; two or three lines is all a caption ever has, so the search is
    // small.
    let mut best: Option<(f64, Vec<usize>)> = None;
    let mut cuts = vec![0usize; lines - 1];
    search(words, per_line, 1, 0, &mut cuts, &mut best);
    best.map_or_else(|| starts[1..].to_vec(), |(_, cuts)| cuts)
}

/// Try every next cut from `after`, scoring complete sets into `best`.
fn search(
    words: &[CueWord],
    per_line: usize,
    after: usize,
    depth: usize,
    cuts: &mut Vec<usize>,
    best: &mut Option<(f64, Vec<usize>)>,
) {
    if depth == cuts.len() {
        let score = score(words, cuts, per_line);
        if score.is_finite() && best.as_ref().is_none_or(|(b, _)| score < *b) {
            *best = Some((score, cuts.clone()));
        }
        return;
    }
    let left = cuts.len() - depth;
    for at in after..=words.len().saturating_sub(left) {
        cuts[depth] = at;
        search(words, per_line, at + 1, depth + 1, cuts, best);
    }
}

/// How good one set of line starts is: lower is better, infinite does not fit.
fn score(words: &[CueWord], cuts: &[usize], per_line: usize) -> f64 {
    let mut lengths = Vec::with_capacity(cuts.len() + 1);
    let mut from = 0;
    for &at in cuts.iter().chain(std::iter::once(&words.len())) {
        if at <= from {
            return f64::INFINITY;
        }
        let length = width(&words[from..at]);
        if length > per_line && at - from > 1 {
            return f64::INFINITY;
        }
        lengths.push(length);
        from = at;
    }
    #[allow(clippy::cast_precision_loss)] // line lengths
    let mean = lengths.iter().sum::<usize>() as f64 / lengths.len() as f64;
    let mut score = 0.0;
    for (i, &length) in lengths.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let length = length as f64;
        score += (length - mean).powi(2);
        // A top line longer than the one below reads worse than the reverse.
        if let Some(&below) = lengths.get(i + 1) {
            #[allow(clippy::cast_precision_loss)]
            let below = below as f64;
            if length > below {
                score += (length - below) * 2.0;
            }
        }
    }
    for &at in cuts {
        let (sentence, clause) = ending(&words[at - 1].text);
        if sentence {
            score -= 40.0;
        } else if clause {
            score -= 20.0;
        }
    }
    score
}

/// Lines filled one after another, for words too long to fit any other way.
fn greedy(words: &[CueWord], limits: &Limits) -> Vec<usize> {
    let per_line = limits.chars_per_line.max(1) as usize;
    let mut out = Vec::new();
    let mut from = 0;
    for i in 1..words.len() {
        if width(&words[from..=i]) > per_line {
            out.push(i);
            from = i;
        }
    }
    out
}

/// Put the speakers' names on the captions where a new one starts talking.
fn label(cues: &mut [Cue], limits: &Limits, names: &dyn Fn(Uuid) -> Option<String>) {
    if limits.speaker_labels != SpeakerLabels::Names {
        return;
    }
    let mut last: Option<Uuid> = None;
    for cue in cues {
        let speaker = cue.words.first().and_then(|w| w.speaker);
        if speaker.is_some() && speaker != last {
            cue.label = speaker.and_then(names).map(|name| format!("{name}:"));
        }
        last = speaker.or(last);
    }
}

/// How long each caption stays up: from its first word to its last at least,
/// then longer to reach the shortest time and the reading speed, but never so
/// long that it runs into the next caption's gap.
fn time(cues: &mut [Cue], limits: &Limits) {
    let starts: Vec<Rational> = cues.iter().map(|c| c.start).collect();
    for (i, cue) in cues.iter_mut().enumerate() {
        let mut end = cue.end;
        if let Ok(shortest) = cue.start.checked_add(limits.min_duration) {
            end = end.max(shortest);
        }
        if limits.reading_speed > 0 {
            let chars: usize = cue.lines().iter().map(|l| l.chars().count()).sum();
            let chars = i64::try_from(chars).unwrap_or(i64::MAX);
            if let Ok(needed) = Rational::new(chars, i64::from(limits.reading_speed)) {
                if let Ok(readable) = cue.start.checked_add(needed) {
                    end = end.max(readable);
                }
            }
        }
        if let Some(next) = starts.get(i + 1) {
            if let Ok(room) = next.checked_sub(limits.gap) {
                end = end.min(room);
            }
        }
        // Never before the caption starts, whatever the gap asked.
        cue.end = end.max(cue.start);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn words(text: &str, each: i64) -> Vec<CueWord> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| {
                let i = i64::try_from(i).unwrap();
                CueWord {
                    text: w.into(),
                    start: Rational::new(i * each, 1000).unwrap(),
                    end: Rational::new(i * each + each - 20, 1000).unwrap(),
                    speaker: None,
                    emphasis: false,
                }
            })
            .collect()
    }

    /// **Every word lands in exactly one caption, in order, and no line runs
    /// past the limit.** A word dropped here is a word missing from the
    /// burned-in picture and the subtitle file alike.
    #[test]
    fn every_word_lands_once_and_every_line_fits() {
        let text = "So what we found was that the people who came early stayed longer. \
                    Nobody expected that, least of all the organisers, who had planned \
                    for the opposite and booked the hall for the afternoon only. In the \
                    end they kept it open until nine";
        let all = words(text, 300);
        let limits = Limits::default();
        let cues = build(&all, &limits, &|_| None);
        let back: Vec<&str> = cues
            .iter()
            .flat_map(|c| c.words.iter().map(|w| w.text.as_str()))
            .collect();
        let sent: Vec<&str> = all.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(back, sent);
        for cue in &cues {
            let lines = cue.lines();
            assert!(lines.len() <= 2, "{lines:?}");
            for line in &lines {
                assert!(line.chars().count() <= 42, "{line:?}");
            }
            assert!(cue.start <= cue.end);
        }
        for pair in cues.windows(2) {
            assert!(pair[0].end <= pair[1].start, "captions overlap");
        }
        // The first sentence ends a caption rather than running on into the next.
        assert!(
            cues.iter()
                .any(|c| c.words.last().unwrap().text == "longer."),
            "{:?}",
            cues.iter().map(Cue::text).collect::<Vec<_>>()
        );
    }

    /// **Speakers reach the captions.** A new speaker starts a new caption
    /// with their name in front, or shares one with a dash before each line;
    /// one word a caption holds one word; a short caption stays up long
    /// enough to read. Wrong here is a line put in the wrong mouth.
    #[test]
    fn speakers_are_named_and_kept_apart() {
        let (ann, bob) = (Uuid::now_v7(), Uuid::now_v7());
        let mut all = words("Are you coming? Yes, now.", 300);
        for (i, w) in all.iter_mut().enumerate() {
            w.speaker = Some(if i < 3 { ann } else { bob });
        }
        let names = |id: Uuid| Some(if id == ann { "Ann" } else { "Bob" }.to_owned());
        let named = Limits {
            speaker_labels: SpeakerLabels::Names,
            ..Limits::default()
        };
        let cues = build(&all, &named, &names);
        let texts: Vec<String> = cues.iter().map(Cue::text).collect();
        assert_eq!(texts, ["Ann: Are you coming?", "Bob: Yes, now."]);

        let dashed = Limits {
            speaker_labels: SpeakerLabels::Dashes,
            split_on_speaker: false,
            ..Limits::default()
        };
        let cues = build(&all, &dashed, &names);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].lines(), ["- Are you coming?", "- Yes, now."]);

        let one = Limits {
            words: 1,
            ..Limits::default()
        };
        let cues = build(&all, &one, &names);
        assert_eq!(cues.len(), all.len());
        // Up for the shortest time, but never into the next caption.
        assert!(cues[0].end <= cues[1].start);
        let last = cues.last().unwrap();
        assert_eq!(
            last.end.checked_sub(last.start).unwrap(),
            Limits::default().min_duration
        );
        assert!((last.clone().rebroken(&one).reading_speed() - 4.0).abs() < 1e-9);

        let dense = Limits::for_language("ja");
        assert_eq!((dense.chars_per_line, dense.reading_speed), (16, 9));
        assert_eq!(join(["今日は", "晴れ", "です。"]), "今日は晴れです。");
        assert!(spaced("Hello,", "world") && !spaced("晴れ", "です。"));
    }
}

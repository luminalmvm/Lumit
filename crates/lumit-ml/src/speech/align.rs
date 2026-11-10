//! When each word was said, from where the model looked while it wrote it.
//!
//! A few of Whisper's attention heads follow the sound in time as the words
//! are written. Their attention, standardised, smoothed along time with a
//! median filter and averaged, is a cost map from word pieces to 20 ms steps
//! of sound, and the cheapest path through it (dynamic time warping) says
//! where each piece begins. Pieces are then joined into words the way
//! openai's `timing.py` joins them: on spaces for most languages, and on
//! whole characters for Chinese, Japanese, Thai, Lao, Burmese and Cantonese,
//! with leading and trailing punctuation folded into its word. Unlike openai,
//! punctuation written without a space stays inside the word, so
//! "half-finished" and "o'clock" are one word each. Last, each
//! edge moves to the quietest point within 60 ms, because a word begins and
//! ends where the sound dips, and the path only knows 20 ms steps.
//!
//! # Thread role and contract
//!
//! Pure arithmetic on whichever thread runs the transcription. No IO, no
//! clocks, nothing shared.

use super::{mel::HOP, tokens::Tokens};

/// Encoder positions a second.
const PER_SECOND: f64 = 50.0;
/// The median filter's width, in encoder positions.
const MEDIAN: usize = 7;
/// How far an edge may move to find quiet, in 10 ms frames.
const SNAP: isize = 6;
/// Punctuation that belongs to the word after it.
const OPENING: &str = "\"'\u{201c}\u{00bf}([{-";
/// Punctuation that belongs to the word before it.
const CLOSING: &str = "\"'.\u{3002},\u{ff0c}!\u{ff01}?\u{ff1f}:\u{ff1a}\u{201d})]}\u{3001}";
/// Languages written without spaces between words.
const UNSPACED: [&str; 6] = ["zh", "ja", "th", "lo", "my", "yue"];

/// One word, timed within its window.
#[derive(Debug, Clone, PartialEq)]
pub struct Timed {
    /// The word, with any leading space it was written with.
    pub text: String,
    /// Seconds into the window.
    pub start: f64,
    /// Seconds into the window.
    pub end: f64,
    /// The mean probability of its pieces.
    pub probability: f32,
}

/// What [`align`] reads.
#[derive(Debug)]
pub struct Attention<'a> {
    /// One matrix per timing head, `rows` by `columns`.
    pub heads: &'a [Vec<f32>],
    /// Rows: every piece of the sequence the decoder read.
    pub rows: usize,
    /// Columns: encoder positions, normally 1500.
    pub columns: usize,
    /// The row of the start of transcript token.
    pub sot: usize,
    /// The row of the last prefix token, the one before the first text piece.
    pub first: usize,
}

/// Time the words of one window.
///
/// `tokens` are the text pieces, `probabilities` the model's belief in each,
/// and `samples` how long the window's real sound was.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn align(
    attention: &Attention<'_>,
    tokens: &[u32],
    probabilities: &[f32],
    vocabulary: &Tokens,
    eot: u32,
    language: &str,
    samples: usize,
) -> Vec<Timed> {
    let wanted = tokens.len() + 1;
    // A pack that names no timing heads gives nothing to time by.
    if tokens.is_empty() || attention.columns == 0 || attention.first + wanted > attention.rows {
        return Vec::new();
    }
    let used = (samples / HOP / 2).clamp(1, attention.columns);
    let matrix = cost(attention, used, wanted);
    let path = warp(&matrix, wanted, used);

    // The first time each row is reached is when its piece begins.
    let mut starts = vec![0.0f64; wanted];
    let mut last_row = usize::MAX;
    for (row, column) in path {
        if row != last_row {
            starts[row] = column as f64 / PER_SECOND;
            last_row = row;
        }
    }

    let groups = words(tokens, vocabulary, eot, language);
    let mut timed = Vec::with_capacity(groups.len());
    let mut at = 0usize;
    for (text, count) in groups {
        let from = at;
        at += count;
        let mean = probabilities
            .get(from..at)
            .map_or(0.0, |p| p.iter().sum::<f32>() / p.len().max(1) as f32);
        timed.push(Timed {
            text,
            start: starts.get(from).copied().unwrap_or(0.0),
            end: starts.get(at).copied().unwrap_or(0.0),
            probability: mean,
        });
    }
    clip_long(&mut timed);
    fold_punctuation(timed)
}

/// The averaged, standardised, smoothed attention, `wanted` rows from the
/// first text row on, `used` columns.
fn cost(attention: &Attention<'_>, used: usize, wanted: usize) -> Vec<f32> {
    let rows = attention.rows - attention.sot;
    let mut sum = vec![0.0f32; wanted * used];
    let mut column = vec![0.0f32; rows];
    let mut row = vec![0.0f32; used];
    for head in attention.heads {
        let mut plane = vec![0.0f32; rows * used];
        for c in 0..used {
            for (r, slot) in column.iter_mut().enumerate() {
                *slot = head
                    .get((attention.sot + r) * attention.columns + c)
                    .copied()
                    .unwrap_or(0.0);
            }
            let mean = column.iter().sum::<f32>() / rows as f32;
            let spread = (column.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / rows as f32)
                .sqrt()
                .max(1e-10);
            for (r, value) in column.iter().enumerate() {
                plane[r * used + c] = (value - mean) / spread;
            }
        }
        let offset = attention.first - attention.sot;
        for r in 0..wanted {
            row.copy_from_slice(&plane[(offset + r) * used..(offset + r + 1) * used]);
            for (c, slot) in sum[r * used..(r + 1) * used].iter_mut().enumerate() {
                *slot += median(&row, c);
            }
        }
    }
    let heads = attention.heads.len().max(1) as f32;
    sum.iter().map(|v| -v / heads).collect()
}

/// The median of the [`MEDIAN`] values around `at`, the row reflected at
/// its ends.
fn median(row: &[f32], at: usize) -> f32 {
    let len = row.len() as isize;
    let half = (MEDIAN / 2) as isize;
    let mut window = [0.0f32; MEDIAN];
    for (k, slot) in window.iter_mut().enumerate() {
        let mut i = at as isize + k as isize - half;
        if i < 0 {
            i = -i;
        }
        if i >= len {
            i = 2 * (len - 1) - i;
        }
        *slot = row
            .get(i.clamp(0, len - 1) as usize)
            .copied()
            .unwrap_or(0.0);
    }
    window.sort_by(f32::total_cmp);
    window[MEDIAN / 2]
}

/// The cheapest path from the top-left of `cost` to its bottom-right, as
/// (row, column) pairs in order. openai's `dtw_cpu`, step for step.
fn warp(cost: &[f32], rows: usize, columns: usize) -> Vec<(usize, usize)> {
    let width = columns + 1;
    let mut total = vec![f32::INFINITY; (rows + 1) * width];
    let mut trace = vec![0u8; (rows + 1) * width];
    total[0] = 0.0;
    for j in 1..=columns {
        for i in 1..=rows {
            let diagonal = total[(i - 1) * width + j - 1];
            let up = total[(i - 1) * width + j];
            let left = total[i * width + j - 1];
            let (best, step) = if diagonal < up && diagonal < left {
                (diagonal, 0)
            } else if up < diagonal && up < left {
                (up, 1)
            } else {
                (left, 2)
            };
            total[i * width + j] = cost[(i - 1) * columns + j - 1] + best;
            trace[i * width + j] = step;
        }
    }
    let (mut i, mut j) = (rows, columns);
    let mut path = Vec::with_capacity(rows + columns);
    while i > 0 && j > 0 {
        path.push((i - 1, j - 1));
        match trace[i * width + j] {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
    }
    // Off the top or the left edge, the path runs straight to the corner.
    while i > 0 {
        path.push((i - 1, 0));
        i -= 1;
    }
    path.reverse();
    path
}

/// Pieces into words: each word's text and how many pieces it took.
fn words(tokens: &[u32], vocabulary: &Tokens, eot: u32, language: &str) -> Vec<(String, usize)> {
    // First whole characters: pieces are bytes, and a character may span two.
    let mut characters: Vec<(String, usize)> = Vec::new();
    let mut pending: Vec<u32> = Vec::new();
    for token in tokens {
        pending.push(*token);
        let bytes = vocabulary.bytes(&pending, eot);
        let whole = match std::str::from_utf8(&bytes) {
            Ok(_) => true,
            // Bytes that can never become a character are let through.
            Err(e) => e.error_len().is_some(),
        };
        if whole {
            characters.push((String::from_utf8_lossy(&bytes).into_owned(), pending.len()));
            pending.clear();
        }
    }
    if !pending.is_empty() {
        let bytes = vocabulary.bytes(&pending, eot);
        characters.push((String::from_utf8_lossy(&bytes).into_owned(), pending.len()));
    }
    if UNSPACED.contains(&language) {
        return characters;
    }
    let mut out: Vec<(String, usize)> = Vec::new();
    for (text, count) in characters {
        match out.last_mut() {
            Some(last) if !text.starts_with(' ') => {
                last.0.push_str(&text);
                last.1 += count;
            }
            _ => out.push((text, count)),
        }
    }
    out
}

/// Fold opening punctuation into the word after it and closing punctuation
/// into the word before it, as openai's `merge_punctuations` does, then drop
/// what is left empty.
fn fold_punctuation(mut timed: Vec<Timed>) -> Vec<Timed> {
    if timed.len() > 1 {
        let mut following = timed.len() - 1;
        for previous in (0..timed.len() - 1).rev() {
            let word = timed[previous].text.clone();
            if word.starts_with(' ') && is_in(word.trim(), OPENING) {
                timed[following].text = word + timed[following].text.as_str();
                timed[previous].text.clear();
            } else {
                following = previous;
            }
        }
        let mut previous = 0;
        for following in 1..timed.len() {
            let word = timed[following].text.clone();
            if !timed[previous].text.ends_with(' ') && is_in(&word, CLOSING) {
                timed[previous].text.push_str(&word);
                timed[following].text.clear();
            } else {
                previous = following;
            }
        }
    }
    timed.retain(|word| !word.text.trim().is_empty());
    timed
}

/// Whether `word` is one of the characters in `set`.
fn is_in(word: &str, set: &str) -> bool {
    let mut chars = word.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if set.contains(c))
}

/// A word far longer than its neighbours has usually swallowed a pause, as
/// openai's timing notes: the one that ends a sentence swallows the pause
/// after it and has its end pulled in, and the one after it swallows the
/// pause before it and has its start pulled in.
fn clip_long(timed: &mut [Timed]) {
    let mut lengths: Vec<f64> = timed
        .iter()
        .map(|w| w.end - w.start)
        .filter(|d| *d > 0.0)
        .collect();
    if lengths.is_empty() {
        return;
    }
    lengths.sort_by(f64::total_cmp);
    let most = 2.0 * lengths[lengths.len() / 2].min(0.7);
    let ends = |text: &str| text.trim_end().ends_with(['.', '。', '!', '！', '?', '？']);
    for at in 0..timed.len() {
        if timed[at].end - timed[at].start <= most {
            continue;
        }
        if ends(&timed[at].text) {
            timed[at].end = timed[at].start + most;
        } else if at == 0 || ends(&timed[at - 1].text) {
            timed[at].start = timed[at].end - most;
        }
    }
}

/// The loudness of each 10 ms frame of `samples`.
#[must_use]
pub fn loudness(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(HOP)
        .map(|frame| frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32)
        .collect()
}

/// Move a word's edge to the quiet within 60 ms of `seconds`: first to the
/// quietest 10 ms frame boundary, the nearest of equals, then along the quiet
/// to where the sound rises, forward for an `opening` edge and back for a
/// closing one. Measured against a synthesiser's own word times, the second
/// step brings the edge closer than the quietest point alone.
#[must_use]
pub fn snap(seconds: f64, loudness: &[f32], opening: bool) -> f64 {
    let frame = (seconds * 100.0).round() as isize;
    let mut best = (f32::INFINITY, 0isize, frame);
    for shift in -SNAP..=SNAP {
        let at = frame + shift;
        // The boundary at `at` sits between frames `at - 1` and `at`.
        let (Ok(before), Ok(after)) = (usize::try_from(at - 1), usize::try_from(at)) else {
            continue;
        };
        let (Some(a), Some(b)) = (loudness.get(before), loudness.get(after)) else {
            continue;
        };
        let quiet = a.min(*b);
        if quiet < best.0 || (quiet == best.0 && shift.abs() < best.1.abs()) {
            best = (quiet, shift, at);
        }
    }
    if !best.0.is_finite() {
        return seconds;
    }
    // Quiet is anything up to twice the quietest frame.
    let floor = 2.0 * best.0 + 1e-9;
    let quiet = |frame: isize| {
        usize::try_from(frame)
            .ok()
            .and_then(|at| loudness.get(at))
            .is_some_and(|l| *l <= floor)
    };
    let mut at = best.2;
    if opening {
        while at < frame + SNAP && quiet(at) {
            at += 1;
        }
    } else {
        while at > frame - SNAP && quiet(at - 1) {
            at -= 1;
        }
    }
    at as f64 / 100.0
}

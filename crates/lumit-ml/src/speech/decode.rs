//! Writing one window's words: which language it is, then the word pieces
//! one at a time, with openai's own safeguards against a model that has lost
//! its place.
//!
//! Each piece is the most likely one the model allows, with the pieces that
//! are never text taken out first, and a blank or an ending taken out of the
//! first place. If what comes back reads like a loop (it squashes too well),
//! or the model was unsure of it on average, the window is written again a
//! little more loosely, up to five times, as openai's `transcribe.py` does.
//! A window the model thinks holds no speech, and is unsure of, gives no
//! words at all. A guard watches the tail as it is written and stops a run of
//! the same few pieces repeating, which is the commonest way a decoder goes
//! wrong.
//!
//! # Thread role and contract
//!
//! Runs on the transcription's own thread through the [`Whisper`] it is lent.
//! Deterministic: the looser passes draw from a generator seeded by the
//! window, so the same window gives the same words every time on the same
//! provider.

use std::io::Write;

use super::whisper::Whisper;
use crate::{error::MlError, session::Held};

/// How loosely each attempt is written. Zero is always the most likely piece.
const TEMPERATURES: [f32; 6] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
/// More squashable than this and the text is a loop.
const COMPRESSION_LIMIT: f32 = 2.4;
/// Less sure than this on average and the text is doubtful.
const LOGPROB_LIMIT: f32 = -1.0;
/// Surer than this that there is no speech, and doubtful, and the window is
/// silence.
const NO_SPEECH_LIMIT: f32 = 0.6;
/// The most pieces one window may write: half the decoder's reach.
const MOST: usize = 224;

/// One window's words, as pieces.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// The text pieces, without the ending.
    pub tokens: Vec<u32>,
    /// How likely the model thought each piece was.
    pub probabilities: Vec<f32>,
    /// The mean log probability, the ending included.
    pub logprob: f32,
    /// How sure the model was that the window holds no speech.
    pub no_speech: f32,
    /// How well the text squashes.
    pub compression: f32,
    /// Whether the guard had to stop a repeating run.
    pub looped: bool,
}

impl Decoded {
    /// Whether openai's rules would write this window again.
    fn doubtful(&self) -> bool {
        self.looped || self.compression > COMPRESSION_LIMIT || self.logprob < LOGPROB_LIMIT
    }
}

/// The language the window is in, by the model's own guess at its first
/// step.
///
/// # Errors
///
/// Whatever the run refuses, and [`MlError::ShapeMismatch`] for a model with
/// no languages.
pub fn detect(model: &mut Whisper, hidden: &Held) -> Result<String, MlError> {
    let sot = model.settings.sot;
    let (_, rows) = model.begin(hidden, &[sot], &[0])?;
    let scores = rows.first().ok_or(MlError::ShapeMismatch)?;
    model
        .settings
        .languages
        .iter()
        .filter_map(|(code, id)| scores.get(*id as usize).map(|score| (code, *score)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(code, _)| code.clone())
        .ok_or(MlError::ShapeMismatch)
}

/// Write one window's words from `prefix`, the prompt and the start of
/// transcript tokens. `None` when the window is silence.
///
/// # Errors
///
/// Whatever a run refuses.
pub fn decode(
    model: &mut Whisper,
    hidden: &Held,
    prefix: &[u32],
    seed: u64,
) -> Result<Option<Decoded>, MlError> {
    let mut tried: Vec<Decoded> = Vec::new();
    for (attempt, temperature) in TEMPERATURES.iter().enumerate() {
        let written = once(model, hidden, prefix, *temperature, seed ^ attempt as u64)?;
        if written.no_speech > NO_SPEECH_LIMIT && written.logprob < LOGPROB_LIMIT {
            return Ok(None);
        }
        if !written.doubtful() {
            return Ok(Some(written));
        }
        tried.push(written);
    }
    // Every attempt was doubtful. The surest one that is not a loop, else
    // the surest one.
    let best = tried
        .iter()
        .filter(|d| !d.looped && d.compression <= COMPRESSION_LIMIT)
        .max_by(|a, b| a.logprob.total_cmp(&b.logprob))
        .or_else(|| tried.iter().max_by(|a, b| a.logprob.total_cmp(&b.logprob)))
        .cloned();
    Ok(best)
}

/// One attempt at one temperature.
fn once(
    model: &mut Whisper,
    hidden: &Held,
    prefix: &[u32],
    temperature: f32,
    seed: u64,
) -> Result<Decoded, MlError> {
    let settings = model.settings.clone();
    let sot_row = prefix
        .iter()
        .position(|id| *id == settings.sot)
        .ok_or(MlError::ShapeMismatch)?;
    let last_row = prefix.len().saturating_sub(1);
    let (mut cache, rows) = model.begin(hidden, prefix, &[sot_row, last_row])?;
    let mut rows = rows.into_iter();
    let at_sot = rows.next().ok_or(MlError::ShapeMismatch)?;
    let mut scores = rows.next().ok_or(MlError::ShapeMismatch)?;
    let no_speech = probability(&at_sot, settings.no_speech as usize);

    let mut random = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut tokens = Vec::new();
    let mut probabilities = Vec::new();
    let mut total = 0.0f32;
    let mut looped = false;
    let room = MOST.min(448usize.saturating_sub(prefix.len()));
    // Word pieces and the ending, less the ones never written. Every special
    // token after the ending, timestamps and languages included, is out.
    let open: Vec<bool> = (0..scores.len() as u32)
        .map(|id| id <= settings.eot && !settings.suppress.contains(&id))
        .collect();
    for at in 0..room {
        let allowed = |id: usize| {
            open.get(id).copied().unwrap_or(false)
                && !(at == 0 && settings.suppress_first.contains(&(id as u32)))
        };
        let top = scores
            .iter()
            .enumerate()
            .filter(|(id, _)| allowed(*id))
            .map(|(_, s)| *s)
            .fold(f32::NEG_INFINITY, f32::max);
        let spread: f32 = scores
            .iter()
            .enumerate()
            .filter(|(id, _)| allowed(*id))
            .map(|(_, s)| (s - top).exp())
            .sum();
        let chosen = if temperature <= 0.0 {
            scores
                .iter()
                .enumerate()
                .filter(|(id, _)| allowed(*id))
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(settings.eot as usize, |(id, _)| id)
        } else {
            sample(&scores, temperature, top, &mut random, allowed).unwrap_or(settings.eot as usize)
        };
        total += scores
            .get(chosen)
            .map_or(f32::NEG_INFINITY, |s| s - top - spread.ln());
        if chosen as u32 == settings.eot {
            break;
        }
        tokens.push(chosen as u32);
        probabilities.push(probability(&scores, chosen));
        if let Some(keep) = repeating(&tokens) {
            tokens.truncate(keep);
            probabilities.truncate(keep);
            looped = true;
            break;
        }
        scores = model.step(hidden, &mut cache, chosen as u32)?;
    }
    let text = model.tokens.bytes(&tokens, settings.eot);
    Ok(Decoded {
        logprob: total / (tokens.len() + 1) as f32,
        compression: compression(&text),
        tokens,
        probabilities,
        no_speech,
        looped,
    })
}

/// Softmax of `scores` at `id`, over every piece.
fn probability(scores: &[f32], id: usize) -> f32 {
    let top = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let spread: f32 = scores.iter().map(|s| (s - top).exp()).sum();
    scores
        .get(id)
        .map_or(0.0, |s| (s - top).exp() / spread.max(f32::MIN_POSITIVE))
}

/// One draw from the allowed pieces at `temperature`.
fn sample(
    scores: &[f32],
    temperature: f32,
    top: f32,
    state: &mut u64,
    allowed: impl Fn(usize) -> bool,
) -> Option<usize> {
    let weight = |s: f32| ((s - top) / temperature).exp();
    let sum: f32 = scores
        .iter()
        .enumerate()
        .filter(|(id, _)| allowed(*id))
        .map(|(_, s)| weight(*s))
        .sum();
    // xorshift64*, enough for picking one piece in a few thousand.
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    let draw = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32;
    let mut left = draw * sum;
    let mut last = None;
    for (id, s) in scores.iter().enumerate().filter(|(id, _)| allowed(*id)) {
        left -= weight(*s);
        last = Some(id);
        if left <= 0.0 {
            return Some(id);
        }
    }
    last
}

/// How well the text squashes with zlib: its length over its squashed
/// length. A loop squashes very well.
fn compression(text: &[u8]) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let mut squash = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let squashed = squash
        .write_all(text)
        .and_then(|()| squash.finish())
        .map_or(text.len(), |out| out.len());
    text.len() as f32 / squashed.max(1) as f32
}

/// Where to cut a tail that repeats the same few pieces, or `None`.
///
/// One piece ten times over, two or three pieces five times, or four to
/// twenty-four pieces three times. Speech repeats a short phrase now and
/// then; it does not do it this many times in a row. The first time round is
/// kept.
fn repeating(tokens: &[u32]) -> Option<usize> {
    for size in 1..=24usize {
        let times = match size {
            1 => 10,
            2 | 3 => 5,
            _ => 3,
        };
        let span = size * times;
        if tokens.len() < span {
            continue;
        }
        let tail = &tokens[tokens.len() - span..];
        let first = &tail[..size];
        if tail.chunks(size).all(|chunk| chunk == first) {
            return Some(tokens.len() - span + size);
        }
    }
    None
}

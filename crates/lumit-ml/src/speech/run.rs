//! One recording, streamed: listening for speech as samples arrive, cutting
//! the speech into windows, writing each window down as it fills, and at the
//! end putting the fillers, paragraphs and speakers in.
//!
//! A stretch of speech is padded by a fifth of a second either side and kept
//! until the stretches waiting add up to more than thirty seconds; then as
//! many as fit go into one window, the silence between them left out. A
//! stretch that runs past thirty seconds on its own is cut where the listener
//! was least sure it was speech, in its last ten seconds. Each window's word
//! times are worked out inside the window and then mapped back onto the
//! recording through the pieces it was made from. When Whisper skips five
//! seconds or more of a window, that stretch goes back to the front of the
//! queue and is written on its own. Audio that every window has
//! finished with is let go, so an hour costs what one window costs.
//!
//! # Thread role and contract
//!
//! Runs on the thread that owns the [`Speech`]. Each call to [`Run::feed`]
//! listens to what it is handed and writes down only the windows that filled
//! while it did; [`Run::finish`] writes the rest.

use super::{
    align::{self, Attention},
    decode,
    mel::{RATE, WINDOW},
    vad::{Stretch, Tracker, CHUNK},
    voices, Heard, HeardWord, Options, Run, Speakers, Voice,
};
use crate::error::MlError;

/// Padding each side of a stretch of speech: 200 ms.
const PAD: usize = RATE / 5;
/// How far back a stretch too long for one window may be cut: ten seconds.
const CUT_BACK: usize = 10 * RATE;
/// A voice heard for this long with no word over it is a filler.
const FILLER: f64 = 0.25;
/// A chunk at least this likely to be speech is a voice, for fillers.
const VOICED: f32 = 0.5;
/// A stretch of a window this long with no word over it is written again:
/// five seconds.
const SHORT: usize = 5 * RATE;
/// A pause this long starts a paragraph.
const PARAGRAPH: f64 = 1.0;
/// Each fingerprint hears this much: 1.5 s.
const PRINT: usize = 3 * RATE / 2;
/// And the next starts this much later: 0.75 s.
const PRINT_HOP: usize = 3 * RATE / 4;
/// The most prompt pieces Whisper takes: half its reach, less the start of
/// prompt token.
const PROMPT: usize = 223;
/// A piece shorter than this gives no fingerprint: 0.5 s.
const PRINT_SHORTEST: usize = RATE / 2;
/// Fingerprints further apart than this are different voices. Measured with
/// ERes2Net on three synthetic voices, 0.6 split one voice into several and
/// 0.9 joined two women's voices, and 0.7 to 0.8 told all three apart.
const BETWEEN_PRINTS: f32 = 0.75;
/// Stretches of one voice further apart than this are different speakers,
/// on the same measurement.
pub(super) const BETWEEN_STRETCHES: f32 = 0.75;
/// A run of one speaker's words shorter than this, inside another's, is
/// taken as the other's: 0.5 s.
const FLIP: f64 = 0.5;
/// A word's edge further than this from the speech it belongs to has slid
/// into the quiet beside it: 130 ms.
const ADRIFT: usize = RATE * 13 / 100;

/// A stretch of the recording waiting to go into a window, in samples.
#[derive(Debug, Clone, Copy)]
struct Piece {
    start: usize,
    end: usize,
    /// Whether silence comes before it, rather than a cut through speech.
    after_silence: bool,
    /// Whether it was ended by a cut through speech, so nothing is padded on.
    cut: bool,
    /// Whether it is a second try at audio a window skipped, which is never
    /// tried a third time.
    again: bool,
}

/// A word as the run keeps it until the end.
#[derive(Debug, Clone)]
struct Word {
    text: String,
    start: f64,
    end: f64,
    confidence: f32,
    window: usize,
    /// The first word of a window that follows silence.
    opens: bool,
}

/// One fingerprint and the time it heard.
#[derive(Debug, Clone)]
struct Print {
    start: f64,
    end: f64,
    print: Vec<f32>,
}

/// Everything a run keeps between calls.
#[derive(Debug)]
pub(super) struct State {
    options: Options,
    /// The vocabulary as word pieces.
    vocabulary: Vec<u32>,
    language: Option<String>,
    /// Samples from `base` on that something still needs.
    audio: Vec<f32>,
    base: usize,
    received: usize,
    /// Chunks the listener has heard.
    chunks: usize,
    /// Each chunk's probability, from chunk `chance_base` on.
    chances: Vec<f32>,
    chance_base: usize,
    tracker: Tracker,
    /// Where speech was last cut through, so what follows is not padded.
    cut_at: Option<usize>,
    /// The end of the last piece written, which nothing may reach back past.
    written_to: usize,
    /// Every run of chunks heard as speech, in samples, short pauses and
    /// all left out.
    voiced: Vec<(usize, usize)>,
    pending: Vec<Piece>,
    windows: usize,
    words: Vec<Word>,
    prints: Vec<Print>,
}

impl State {
    pub(super) fn new(options: Options, vocabulary: Vec<u32>) -> Self {
        State {
            vocabulary,
            language: options.language.clone(),
            options,
            audio: Vec::new(),
            base: 0,
            received: 0,
            chunks: 0,
            chances: Vec::new(),
            chance_base: 0,
            tracker: Tracker::default(),
            cut_at: None,
            written_to: 0,
            voiced: Vec::new(),
            pending: Vec::new(),
            windows: 0,
            words: Vec::new(),
            prints: Vec::new(),
        }
    }

    /// Keep one chunk's chance, and note it if it was voiced.
    fn heard(&mut self, chance: f32) {
        self.chances.push(chance);
        if chance >= VOICED {
            let (from, to) = (self.chunks * CHUNK, (self.chunks + 1) * CHUNK);
            match self.voiced.last_mut() {
                Some(last) if last.1 == from => last.1 = to,
                _ => self.voiced.push((from, to)),
            }
        }
    }

    /// Samples waiting in pieces.
    fn waiting(&self) -> usize {
        self.pending.iter().map(|p| p.end - p.start).sum()
    }

    /// A stretch has ended: keep it, padded, for a window.
    fn close(&mut self, stretch: Stretch, cut: bool) {
        let (start, end) = (stretch.start * CHUNK, stretch.end * CHUNK);
        let after_cut = self.cut_at == Some(start);
        self.cut_at = cut.then_some(end);
        let mut piece = Piece {
            start: if after_cut {
                start
            } else {
                start.saturating_sub(PAD)
            },
            end: if cut { end } else { end + PAD },
            after_silence: !after_cut,
            cut,
            again: false,
        };
        piece.end = piece.end.min(self.received.max(piece.start));
        match self.pending.last_mut() {
            Some(last) if piece.start <= last.end && !last.cut => {
                last.end = last.end.max(piece.end);
                last.cut = piece.cut;
            }
            _ => {
                let floor = self.pending.last().map_or(self.written_to, |last| last.end);
                piece.start = piece.start.max(floor);
                if piece.end > piece.start {
                    self.pending.push(piece);
                }
            }
        }
    }

    /// The chunk boundary in `[from, to)` samples where speech was least
    /// likely, or `to` when nothing is known.
    fn least_likely(&self, from: usize, to: usize) -> usize {
        let (first, last) = (from.div_ceil(CHUNK), to / CHUNK);
        (first.max(self.chance_base)..last)
            .filter_map(|chunk| {
                self.chances
                    .get(chunk - self.chance_base)
                    .map(|chance| (*chance, chunk))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
            .map_or(to, |(_, chunk)| chunk * CHUNK)
    }

    /// The pieces of the next window: as many as fit in thirty seconds, the
    /// first cut where it must be.
    fn take_window(&mut self) -> Vec<Piece> {
        let mut room = WINDOW;
        let mut out = Vec::new();
        while let Some(first) = self.pending.first().copied() {
            let len = first.end - first.start;
            if len <= room {
                out.push(first);
                self.pending.remove(0);
                room -= len;
                continue;
            }
            if out.is_empty() {
                let limit = first.start + room;
                let at = self
                    .least_likely(limit.saturating_sub(CUT_BACK).max(first.start + 1), limit)
                    .clamp(first.start + 1, limit);
                out.push(Piece {
                    end: at,
                    cut: true,
                    ..first
                });
                if let Some(rest) = self.pending.first_mut() {
                    rest.start = at;
                    rest.after_silence = false;
                }
            }
            break;
        }
        out
    }

    /// Let go of audio and chances nothing will read again.
    fn forget(&mut self) {
        let mut keep = (self.chunks * CHUNK).saturating_sub(PAD);
        if let Some(first) = self.pending.first() {
            keep = keep.min(first.start);
        }
        if let Some(open) = self.tracker.open() {
            keep = keep.min((open * CHUNK).saturating_sub(PAD));
        }
        if keep > self.base {
            let drop = (keep - self.base).min(self.audio.len());
            self.audio.drain(..drop);
            self.base += drop;
        }
        let keep_chunk = keep / CHUNK;
        if keep_chunk > self.chance_base {
            let drop = (keep_chunk - self.chance_base).min(self.chances.len());
            self.chances.drain(..drop);
            self.chance_base += drop;
        }
    }
}

impl Run<'_> {
    /// Hand over more of the recording: 16 kHz mono, any number of samples.
    ///
    /// Listens to all of it, and writes down every window that fills while
    /// it does, which is at most one per thirty seconds of speech handed in.
    ///
    /// # Errors
    ///
    /// Whatever a model refuses.
    pub fn feed(&mut self, samples: &[f32]) -> Result<(), MlError> {
        self.state.audio.extend_from_slice(samples);
        self.state.received += samples.len();
        while (self.state.chunks + 1) * CHUNK <= self.state.received {
            let from = self.state.chunks * CHUNK - self.state.base;
            let chance = self
                .speech
                .ear
                .hear(self.state.audio.get(from..from + CHUNK).unwrap_or(&[]))?;
            self.listened(chance)?;
        }
        Ok(())
    }

    /// One chunk heard.
    fn listened(&mut self, chance: f32) -> Result<(), MlError> {
        let state = &mut self.state;
        state.heard(chance);
        let at = state.chunks;
        state.chunks += 1;
        if let Some(stretch) = state.tracker.push(at, chance) {
            state.close(stretch, false);
        }
        // When the speech going on would not fit beside what is waiting, what
        // is waiting is written first. Speech going on for longer than a
        // window holds on its own is cut where it was least like speech.
        if let Some(open) = state.tracker.open() {
            let open_from = (open * CHUNK).saturating_sub(PAD);
            let now = state.chunks * CHUNK;
            if state.waiting() + now.saturating_sub(open_from) >= WINDOW {
                if state.pending.is_empty() {
                    let from = (open + 1) * CHUNK;
                    let cut =
                        state.least_likely(now.saturating_sub(CUT_BACK).max(from), now) / CHUNK;
                    if let Some(stretch) = state.tracker.cut(cut.max(open + 1), true) {
                        state.close(stretch, true);
                    }
                }
                self.write_window()?;
            }
        }
        while self.state.waiting() > WINDOW {
            self.write_window()?;
        }
        Ok(())
    }

    /// Seconds of the recording fully dealt with.
    #[must_use]
    pub fn heard(&self) -> f64 {
        let state = &self.state;
        let mut done = state.chunks * CHUNK;
        if let Some(first) = state.pending.first() {
            done = done.min(first.start);
        }
        if let Some(open) = state.tracker.open() {
            done = done.min((open * CHUNK).saturating_sub(PAD));
        }
        done as f64 / RATE as f64
    }

    /// Write down what is left and put it all together.
    ///
    /// # Errors
    ///
    /// Whatever a model refuses.
    pub fn finish(mut self) -> Result<Heard, MlError> {
        // The last partial chunk, padded with silence.
        let heard_to = self.state.chunks * CHUNK;
        if self.state.received > heard_to {
            let mut last = vec![0.0f32; CHUNK];
            let from = heard_to - self.state.base;
            let tail = self.state.audio.get(from..).unwrap_or(&[]);
            last[..tail.len().min(CHUNK)].copy_from_slice(&tail[..tail.len().min(CHUNK)]);
            let chance = self.speech.ear.hear(&last)?;
            let state = &mut self.state;
            state.heard(chance);
            let at = state.chunks;
            state.chunks += 1;
            if let Some(stretch) = state.tracker.push(at, chance) {
                state.close(stretch, false);
            }
        }
        let end = self.state.chunks;
        if let Some(stretch) = self.state.tracker.cut(end, false) {
            self.state.close(stretch, false);
        }
        while !self.state.pending.is_empty() {
            self.write_window()?;
        }

        let state = self.state;
        let mut words = state.words;
        words.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut words = fillers(words, &state.voiced);
        for at in 1..words.len() {
            let before = words[at - 1].end;
            if words[at].start < before {
                words[at].start = before;
            }
            if words[at].end < words[at].start {
                words[at].end = words[at].start;
            }
        }
        let mut heard: Vec<HeardWord> = words
            .iter()
            .enumerate()
            .map(|(at, word)| {
                let paragraph = at == 0
                    || word.start - words[at - 1].end >= PARAGRAPH
                    || (word.opens && word.window != words[at - 1].window);
                HeardWord {
                    text: word.text.clone(),
                    start: word.start,
                    end: word.end,
                    confidence: word.confidence,
                    voice: None,
                    paragraph,
                }
            })
            .collect();
        let voices = match state.options.speakers {
            Speakers::Off => Vec::new(),
            Speakers::Detect => speakers(&mut heard, &state.prints, None),
            Speakers::Count(n) => speakers(&mut heard, &state.prints, Some(n.max(1) as usize)),
        };
        Ok(Heard {
            language: state.language.unwrap_or_default(),
            words: heard,
            voices,
        })
    }

    /// Write down the next window.
    fn write_window(&mut self) -> Result<(), MlError> {
        let pieces = self.state.take_window();
        if pieces.is_empty() {
            return Ok(());
        }
        let mut samples = Vec::with_capacity(WINDOW);
        for piece in &pieces {
            let from = piece.start.saturating_sub(self.state.base);
            let to = piece.end.saturating_sub(self.state.base);
            samples.extend_from_slice(self.state.audio.get(from..to).unwrap_or(&[]));
        }
        if let Some(last) = pieces.last() {
            self.state.written_to = last.end;
        }
        let window = self.state.windows;
        self.state.windows += 1;
        let mut pieces = pieces;
        if let Some((from, to)) = self.words_of(&samples, &pieces, window)? {
            // Five seconds of speech with no word over it is Whisper having
            // skipped some of the window, which it does now and then, rather
            // than anybody falling silent: silence never got this far. That
            // stretch goes back to the front of the queue to be written on
            // its own, once.
            let skipped = take(&mut pieces, from, to);
            self.state.pending.splice(0..0, skipped);
        }
        if self.state.options.speakers != Speakers::Off {
            self.prints_of(&pieces)?;
        }
        self.state.forget();
        Ok(())
    }

    /// One window's words, onto the recording's timeline. Hands back the
    /// longest stretch of the window no word covers, in samples, when it is
    /// long enough to have been skipped.
    fn words_of(
        &mut self,
        samples: &[f32],
        pieces: &[Piece],
        window: usize,
    ) -> Result<Option<(usize, usize)>, MlError> {
        let model = &mut self.speech.whisper;
        let hidden = model.encode(samples)?;
        let language = match &self.state.language {
            Some(code) => code.clone(),
            None => {
                let code = decode::detect(model, &hidden)?;
                self.state.language = Some(code.clone());
                code
            }
        };
        let settings = model.settings.clone();
        let task = if self.state.options.translate {
            settings.translate
        } else {
            settings.transcribe
        };
        // The vocabulary goes in as the text before the window, the newest
        // part kept when it is too long.
        let prompt = &self.state.vocabulary;
        let mut prefix = Vec::new();
        if !prompt.is_empty() {
            prefix.push(settings.prev);
            prefix.extend_from_slice(&prompt[prompt.len().saturating_sub(PROMPT)..]);
        }
        let sot = prefix.len();
        prefix.extend([
            settings.sot,
            settings.language(&language).ok_or(MlError::ShapeMismatch)?,
            task,
            settings.no_timestamps,
        ]);
        let Some(decoded) = decode::decode(model, &hidden, &prefix, window as u64)? else {
            return Ok(None);
        };
        if decoded.tokens.is_empty() {
            return Ok(None);
        }
        let mut sequence = prefix.clone();
        sequence.extend_from_slice(&decoded.tokens);
        sequence.push(settings.eot);
        let heads = model.attend(&hidden, &sequence)?;
        let written = if self.state.options.translate {
            "en"
        } else {
            language.as_str()
        };
        let timed = align::align(
            &Attention {
                heads: &heads,
                rows: sequence.len(),
                columns: heads.first().map_or(0, |h| h.len() / sequence.len().max(1)),
                sot,
                first: prefix.len() - 1,
            },
            &decoded.tokens,
            &decoded.probabilities,
            &model.tokens,
            settings.eot,
            written,
            samples.len(),
        );
        let loudness = align::loudness(samples);
        let opens = pieces.first().is_some_and(|p| p.after_silence);
        let mut gap = (0usize, 0usize);
        let mut covered = 0usize;
        for word in timed.iter().chain(std::iter::once(&align::Timed {
            text: String::new(),
            start: samples.len() as f64 / RATE as f64,
            end: samples.len() as f64 / RATE as f64,
            probability: 0.0,
        })) {
            let start = (word.start.max(0.0) * RATE as f64) as usize;
            if start.saturating_sub(covered) > gap.1 - gap.0 {
                gap = (covered, start);
            }
            covered = covered.max((word.end.max(0.0) * RATE as f64) as usize);
        }
        for (at, word) in timed.into_iter().enumerate() {
            let (mut start, mut end) = (
                align::snap(word.start, &loudness, true),
                align::snap(word.end, &loudness, false),
            );
            if end <= start {
                (start, end) = (word.start, word.end.max(word.start));
            }
            let (start, end) = settle(
                on_recording(start, pieces, true),
                on_recording(end, pieces, false),
                &self.state.voiced,
                pieces,
                &loudness,
            );
            self.state.words.push(Word {
                text: word.text.trim().to_owned(),
                start,
                end,
                confidence: word.probability.max(0.01),
                window,
                opens: opens && at == 0,
            });
        }
        let first_try = !on_retry(pieces, gap.0);
        Ok((gap.1 - gap.0 >= SHORT && first_try).then_some(gap))
    }

    /// Fingerprints over one window's pieces.
    fn prints_of(&mut self, pieces: &[Piece]) -> Result<(), MlError> {
        let Some(model) = self.speech.voices.as_mut() else {
            return Ok(());
        };
        let mut spans: Vec<(usize, &[f32])> = Vec::new();
        for piece in pieces {
            let len = piece.end - piece.start;
            if len < PRINT_SHORTEST {
                continue;
            }
            let mut starts: Vec<usize> =
                (0..=len.saturating_sub(PRINT)).step_by(PRINT_HOP).collect();
            if let Some(last) = starts.last() {
                if len > PRINT && last + PRINT < len {
                    starts.push(len - PRINT);
                }
            }
            for offset in starts {
                let from = piece.start + offset - self.state.base;
                if let Some(heard) = self.state.audio.get(from..from + PRINT.min(len)) {
                    spans.push((piece.start + offset, heard));
                }
            }
        }
        let audio: Vec<&[f32]> = spans.iter().map(|(_, heard)| *heard).collect();
        let prints = model.fingerprints(&audio)?;
        for ((start, heard), print) in spans.iter().zip(prints) {
            let start = *start as f64 / RATE as f64;
            self.state.prints.push(Print {
                start,
                end: start + heard.len() as f64 / RATE as f64,
                print,
            });
        }
        Ok(())
    }
}

/// Take the stretch from `from` to `to` samples into a window out of its
/// pieces, leaving the rest, and hand it back marked as a second try.
fn take(pieces: &mut Vec<Piece>, from: usize, to: usize) -> Vec<Piece> {
    let mut kept = Vec::with_capacity(pieces.len() + 1);
    let mut taken = Vec::new();
    let mut at = 0usize;
    for piece in pieces.drain(..) {
        let len = piece.end - piece.start;
        let (a, b) = (from.clamp(at, at + len) - at, to.clamp(at, at + len) - at);
        if a > 0 {
            kept.push(Piece {
                end: piece.start + a,
                cut: b > a || piece.cut,
                ..piece
            });
        }
        if b > a {
            taken.push(Piece {
                start: piece.start + a,
                end: piece.start + b,
                after_silence: piece.after_silence && a == 0,
                cut: b < len || piece.cut,
                again: true,
            });
        }
        if b < len {
            kept.push(Piece {
                start: piece.start + b,
                after_silence: false,
                ..piece
            });
        }
        at += len;
    }
    *pieces = kept;
    taken
}

/// Whether the audio `at` samples into a window is already a second try.
fn on_retry(pieces: &[Piece], at: usize) -> bool {
    let mut left = at;
    for piece in pieces {
        let len = piece.end - piece.start;
        if left < len {
            return piece.again;
        }
        left -= len;
    }
    pieces.last().is_some_and(|piece| piece.again)
}

/// A time inside a window, in seconds, as a time on the recording. An edge
/// on the join of two pieces is in the piece its word is in: the later one
/// for an `opening` edge, the earlier for a closing one.
fn on_recording(seconds: f64, pieces: &[Piece], opening: bool) -> f64 {
    let mut at = (seconds.max(0.0) * RATE as f64) as usize;
    for (index, piece) in pieces.iter().enumerate() {
        let len = piece.end - piece.start;
        if at < len || (at == len && !opening) || index + 1 == pieces.len() {
            return (piece.start + at.min(len)) as f64 / RATE as f64;
        }
        at -= len;
    }
    0.0
}

/// A sample of the recording as a time inside the window made of `pieces`,
/// in seconds, or `None` when the window leaves it out.
fn in_window(sample: usize, pieces: &[Piece]) -> Option<f64> {
    let mut before = 0usize;
    for piece in pieces {
        if (piece.start..=piece.end).contains(&sample) {
            return Some((before + sample - piece.start) as f64 / RATE as f64);
        }
        before += piece.end - piece.start;
    }
    None
}

/// A word's edges on the recording, in seconds, with an edge left in the
/// quiet beside its speech moved up to the speech. A window leaves the pauses
/// out, and Whisper is loose about where a word begins across one, so the
/// first word after a pause can start where the stretch before it ended, a
/// second or more early. `voiced` is where the listener heard speech, in
/// samples.
fn settle(
    start: f64,
    end: f64,
    voiced: &[(usize, usize)],
    pieces: &[Piece],
    loudness: &[f32],
) -> (f64, f64) {
    let sample = |seconds: f64| (seconds.max(0.0) * RATE as f64) as usize;
    let (from, to) = (sample(start), sample(end));
    // The edge again, a chunk outside the speech, moved to where the sound
    // rises or falls as every other edge is.
    let edge = |at: usize, opening: bool| {
        in_window(at, pieces).map_or(at as f64 / RATE as f64, |inside| {
            on_recording(align::snap(inside, loudness, opening), pieces, opening)
        })
    };
    let mut settled = (start, end);
    let next = voiced.get(voiced.partition_point(|v| v.1 <= from));
    if let Some(next) = next.filter(|v| v.0 > from + ADRIFT && v.0 < to) {
        settled.0 = edge(next.0.saturating_sub(CHUNK).max(from), true);
    }
    let last = voiced
        .partition_point(|v| v.0 < to)
        .checked_sub(1)
        .and_then(|at| voiced.get(at));
    if let Some(last) = last.filter(|v| v.1 + ADRIFT < to && v.1 > from) {
        settled.1 = edge((last.1 + CHUNK).min(to), false);
    }
    if settled.1 < settled.0 {
        return (start, end);
    }
    settled
}

/// Put an "um", with a confidence of exactly zero, wherever a voice was
/// heard for a quarter of a second with no word over it.
fn fillers(words: Vec<Word>, voiced: &[(usize, usize)]) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::with_capacity(words.len());
    let mut added = Vec::new();
    let mut next = 0usize;
    for (from, to) in voiced {
        let (from, to) = (*from as f64 / RATE as f64, *to as f64 / RATE as f64);
        let mut cursor = from;
        let mut window = words.get(next.saturating_sub(1)).map_or(0, |w| w.window);
        while let Some(word) = words.get(next) {
            if word.start >= to {
                break;
            }
            if word.start - cursor >= FILLER {
                added.push((cursor, word.start, window));
            }
            cursor = cursor.max(word.end);
            window = word.window;
            next += 1;
        }
        if to - cursor >= FILLER {
            added.push((cursor, to, window));
        }
    }
    out.extend(words);
    out.extend(added.into_iter().map(|(start, end, window)| Word {
        text: "um".to_owned(),
        start,
        end,
        confidence: 0.0,
        window,
        opens: false,
    }));
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Who said each word, and the stretches of one voice.
fn speakers(words: &mut [HeardWord], prints: &[Print], count: Option<usize>) -> Vec<Voice> {
    if prints.is_empty() || words.is_empty() {
        return Vec::new();
    }
    let refs: Vec<&[f32]> = prints.iter().map(|p| p.print.as_slice()).collect();
    let groups = voices::group(&refs, count, BETWEEN_PRINTS);
    let speakers = groups.iter().copied().max().map_or(0, |m| m as usize + 1);

    // Each word takes the voice heard most over it, or the nearest one.
    let mut label: Vec<usize> = words
        .iter()
        .map(|word| {
            let mut votes = vec![0.0f64; speakers];
            for (print, group) in prints.iter().zip(&groups) {
                let overlap = word.end.min(print.end) - word.start.max(print.start);
                if overlap > 0.0 {
                    votes[*group as usize] += overlap;
                }
            }
            let best = votes
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(&a.0)));
            match best {
                Some((group, votes)) if *votes > 0.0 => group,
                _ => {
                    let middle = (word.start + word.end) / 2.0;
                    prints
                        .iter()
                        .zip(&groups)
                        .min_by(|a, b| {
                            let d = |p: &Print| ((p.start + p.end) / 2.0 - middle).abs();
                            d(a.0).total_cmp(&d(b.0))
                        })
                        .map_or(0, |(_, group)| *group as usize)
                }
            }
        })
        .collect();

    // A short run of one voice inside another's is the other's.
    let mut at = 0;
    while at < label.len() {
        let mut end = at;
        while end + 1 < label.len() && label[end + 1] == label[at] {
            end += 1;
        }
        let short = words[end].end - words[at].start < FLIP;
        let neighbour = if at > 0 {
            Some(label[at - 1])
        } else {
            label.get(end + 1).copied()
        };
        if let (true, Some(other)) = (short, neighbour) {
            for slot in &mut label[at..=end] {
                *slot = other;
            }
        }
        at = end + 1;
    }

    // Each run of one voice is one stretch, fingerprinted by the mean of
    // what was heard during it, or of everything that voice said.
    let mean_of = |chosen: &mut dyn Iterator<Item = &Print>| {
        let mut sum: Vec<f32> = Vec::new();
        for print in chosen {
            if sum.is_empty() {
                sum = vec![0.0; print.print.len()];
            }
            for (slot, value) in sum.iter_mut().zip(&print.print) {
                *slot += value;
            }
        }
        voices::unit(sum)
    };
    let mut out: Vec<Voice> = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let mut end = at;
        while end + 1 < words.len() && label[end + 1] == label[at] {
            end += 1;
        }
        let (start, stop, group) = (words[at].start, words[end].end, label[at]);
        let mut during = prints
            .iter()
            .zip(&groups)
            .filter(|(p, g)| **g as usize == group && p.end > start && p.start < stop)
            .map(|(p, _)| p)
            .peekable();
        let embedding = if during.peek().is_some() {
            mean_of(&mut during)
        } else {
            mean_of(
                &mut prints
                    .iter()
                    .zip(&groups)
                    .filter(|(_, g)| **g as usize == group)
                    .map(|(p, _)| p),
            )
        };
        for word in &mut words[at..=end] {
            word.voice = Some(out.len());
        }
        out.push(Voice {
            start,
            end: stop,
            embedding,
        });
        at = end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A word either side of a pause starts and ends with its speech.** A
    /// window leaves the pause out, and Whisper can put a word's edge in the
    /// padding the other side of the join. Left there, the first word after a
    /// pause started where the stretch before it ended, a second early, and
    /// its caption came up with it.
    #[test]
    fn a_word_beside_a_pause_keeps_to_its_speech() {
        let piece = |start, end| Piece {
            start,
            end,
            after_silence: true,
            cut: false,
            again: false,
        };
        // Two seconds kept, a second of silence left out, two more kept, with
        // a voice heard in each but for the padding.
        let pieces = [piece(0, 2 * RATE), piece(3 * RATE, 5 * RATE)];
        let voiced = [
            (RATE / 10, 2 * RATE - PAD),
            (3 * RATE + PAD, 5 * RATE - PAD),
        ];
        let loudness: Vec<f32> = (0..400)
            .map(|frame| {
                let at = if frame < 200 {
                    frame * 160
                } else {
                    3 * RATE + (frame - 200) * 160
                };
                let heard = voiced.iter().any(|v| (v.0..v.1).contains(&at));
                if heard {
                    0.1
                } else {
                    0.0
                }
            })
            .collect();
        let word = |start: f64, end: f64| {
            settle(
                on_recording(start, &pieces, true),
                on_recording(end, &pieces, false),
                &voiced,
                &pieces,
                &loudness,
            )
        };

        // Timed from the padding before the join: it starts when it is said.
        let (start, end) = word(1.9, 2.5);
        assert!((start - 3.2).abs() < 1e-3, "starts at {start}");
        assert!((end - 3.5).abs() < 1e-3, "ends at {end}");
        // Timed to the padding after the join: it ends when it was said.
        let (start, end) = word(1.5, 2.1);
        assert!((start - 1.5).abs() < 1e-3, "starts at {start}");
        assert!((end - 1.8).abs() < 1e-3, "ends at {end}");
        // An edge on the join itself goes with its word.
        assert!((on_recording(2.0, &pieces, true) - 3.0).abs() < 1e-9);
        assert!((on_recording(2.0, &pieces, false) - 2.0).abs() < 1e-9);
    }
}

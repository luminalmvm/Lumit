//! Audio sync for multicam: where each recording of one event starts on a
//! shared clock, found from its sound, and how fast its clock runs.
//!
//! In plain terms: every camera and recorder heard the same room, so their
//! sounds line up once each is slid to the right place. [`analyse`] turns a
//! file's sound into a small onset curve (200 values a second). [`solve`]
//! slides the curves of every pair of devices against each other to find
//! roughly where they meet, then reads the sound itself around that place to
//! pin it to a fraction of a millisecond and to measure clock drift. Last, the
//! pairs are joined into one placement, so two files that never overlap still
//! land right when a third overlaps both.
//!
//! Runs on whichever thread asks. [`analyse`] decodes and is meant to run on
//! the caller's own threads, one file each; [`solve`] is single threaded and
//! gives the same answer every run.

use lumit_media::audio::{SoundReader, SoundSource};
use lumit_media::MediaError;
use realfft::RealFftPlanner;

mod coarse;
mod feature;
mod fine;
mod place;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// The rate sound is analysed at. Speech and most onsets live below 4 kHz.
const RATE: u32 = 8000;
/// Samples between onset frames: 5 ms, so 200 frames a second.
const HOP: usize = 40;
/// Onset frames a second.
const FPS: usize = RATE as usize / HOP;
/// Inputs shorter than this are never matched: there is too little sound to
/// tell one place from another.
const SHORTEST: f64 = 5.0;
/// Frames read from a file at a time while analysing it.
const BLOCK: usize = 65_536;
/// The longest stretch of a file analysed, so a runaway file cannot take
/// unbounded memory in the correlations: twelve hours.
const LONGEST_FRAMES: u64 = 12 * 3600 * RATE as u64;

/// Why sync stopped.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// A file could not be opened or decoded.
    #[error("media: {0}")]
    Media(#[from] MediaError),
    /// An FFT was handed buffers of the wrong size.
    #[error("fft: {0}")]
    Fft(#[from] realfft::FftError),
    /// The caller cancelled.
    #[error("cancelled")]
    Cancelled,
}

/// One file's sound as sync sees it: its onset curve and its length.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Onsets {
    /// The onset curve, 200 values a second, normalised to a unit spread and
    /// nought where the file is silent.
    pub feature: Vec<f32>,
    /// The sound's length in seconds.
    pub seconds: f64,
}

/// One recording handed to [`solve`].
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Its sound, from [`analyse`].
    pub onsets: &'a Onsets,
    /// Inputs sharing a group came from one device: they are never compared
    /// with each other and never overlap.
    pub group: u32,
    /// Where it comes within its group, earliest first.
    pub order: u32,
    /// Keeps its own speed, as a camera's pictures do. A scene's clock is
    /// its longest steady input, else its longest, and the drift of every
    /// other input is measured against it.
    pub steady: bool,
}

/// What [`solve`] may do.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Measure and report clock drift as a speed.
    pub drift: bool,
}

/// How sure [`solve`] is of one input's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Placed, and the sound agrees well.
    Synced,
    /// Placed, but the match is weak or disagrees with another.
    Uncertain,
    /// No shared sound was found.
    Unmatched,
}

/// Where one input landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Where the input's own time 0 sits on the shared clock, in seconds.
    /// The earliest placed input starts at 0. None when it was not placed.
    pub start: Option<f64>,
    /// How much faster than nominal the input's clock runs against the
    /// reference: its own time `t` sits at `start + t / speed` on the shared
    /// clock, so a clip played at this speed stays in sync.
    pub speed: f64,
    /// How sure the placement is.
    pub state: State,
}

/// Mono sound at 8 kHz for the fine stage, read on demand so no whole file
/// is ever held.
pub trait Windows {
    /// `frames` samples of input `input` from `start_seconds` on its own
    /// clock, silence outside the file. None when it cannot be read.
    fn window(&self, input: usize, start_seconds: f64, frames: usize) -> Option<Vec<f32>>;
}

/// [`Windows`] read from the inputs' files, in the order handed to [`solve`].
#[derive(Debug, Clone)]
pub struct FileWindows {
    /// One sound per input.
    pub sounds: Vec<SoundSource>,
}

impl Windows for FileWindows {
    fn window(&self, input: usize, start_seconds: f64, frames: usize) -> Option<Vec<f32>> {
        let sound = self.sounds.get(input)?;
        let start = (start_seconds * f64::from(RATE)).round() as i64;
        let mut out = vec![0f32; frames];
        // A window may start before the file does: that part stays silent.
        let skip = usize::try_from(start.saturating_neg().max(0)).unwrap_or(usize::MAX);
        let Some(rest) = out.get_mut(skip..).filter(|r| !r.is_empty()) else {
            return Some(out);
        };
        let mut reader = SoundReader::open(sound, RATE).ok()?;
        let at = u64::try_from(start.max(0)).unwrap_or(0);
        let (stereo, _) = reader.read(at, rest.len(), false).ok()?;
        mix_into(rest, &stereo);
        Some(out)
    }
}

/// Average interleaved stereo into `out`, as far as both reach.
fn mix_into(out: &mut [f32], stereo: &[f32]) {
    for (o, s) in out.iter_mut().zip(stereo.chunks_exact(2)) {
        if let [l, r] = s {
            *o = 0.5 * (l + r);
        }
    }
}

/// Decode `sound` once and build its [`Onsets`]. Only the onset curve is
/// kept, never the decoded sound.
pub fn analyse(sound: &SoundSource, cancelled: &dyn Fn() -> bool) -> Result<Onsets, SyncError> {
    let mut reader = SoundReader::open(sound, RATE)?;
    let mut builder = feature::Builder::new();
    let mut mono = vec![0f32; BLOCK];
    let mut at = 0u64;
    while at < LONGEST_FRAMES {
        if cancelled() {
            return Err(SyncError::Cancelled);
        }
        let (stereo, _) = reader.read(at, BLOCK, false)?;
        let frames = stereo.len() / 2;
        if frames == 0 {
            break;
        }
        let part = mono.get_mut(..frames).unwrap_or_default();
        mix_into(part, &stereo);
        builder.push(part);
        at += frames as u64;
    }
    Ok(builder.finish())
}

/// One accepted pair. At the shorter input's time `mid` the longer one's
/// time is `mid + offset`, and the longer one's clock runs `1 + b` times
/// as fast; `b` is nought where drift was not measured.
#[derive(Debug, Clone, Copy)]
struct Edge {
    short: usize,
    long: usize,
    mid: f64,
    offset: f64,
    b: f64,
    weight: f64,
    synced: bool,
}

/// Place every input on one clock from its sound.
///
/// Each pair of inputs from different groups is compared, `progress` going
/// from 0 to 1 across the pairs. The result is in the order of `inputs`.
/// Inputs that share no sound with the rest come back [`State::Unmatched`];
/// sets of inputs that overlap each other but none of the rest are separate
/// scenes, laid one after another in input order.
pub fn solve(
    inputs: &[Input],
    windows: &dyn Windows,
    options: Options,
    progress: &mut dyn FnMut(f32),
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Placed>, SyncError> {
    let usable = |i: &Input| i.onsets.seconds >= SHORTEST && !i.onsets.feature.is_empty();
    let mut pairs = Vec::new();
    for (i, x) in inputs.iter().enumerate() {
        for (j, y) in inputs.iter().enumerate().skip(i + 1) {
            if x.group != y.group && usable(x) && usable(y) {
                pairs.push((i, j));
            }
        }
    }
    let mut planner = RealFftPlanner::<f32>::new();
    let mut edges = Vec::new();
    for (done, &(i, j)) in pairs.iter().enumerate() {
        if cancelled() {
            return Err(SyncError::Cancelled);
        }
        let (short, long) = match (inputs.get(i), inputs.get(j)) {
            (Some(x), Some(y)) if y.onsets.feature.len() < x.onsets.feature.len() => (j, i),
            _ => (i, j),
        };
        if let (Some(s), Some(l)) = (inputs.get(short), inputs.get(long)) {
            let found = coarse::search(&mut planner, s.onsets, l.onsets, cancelled)?;
            if let Some(found) = found {
                let pair = fine::Pair {
                    windows,
                    short,
                    long,
                    short_seconds: s.onsets.seconds,
                    long_seconds: l.onsets.seconds,
                };
                let fit = fine::refine(&mut planner, &pair, &found, options.drift, cancelled)?;
                edges.push(Edge {
                    short,
                    long,
                    mid: fit.mid,
                    offset: fit.offset,
                    b: fit.b,
                    weight: found.z * found.r,
                    synced: found.synced && fit.sure,
                });
            }
        }
        progress((done + 1) as f32 / pairs.len() as f32);
    }
    let placed = place::place(inputs, &edges);
    progress(1.0);
    Ok(placed)
}

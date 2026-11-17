//! Where the speech is: Silero VAD over the stream, 32 ms at a time, and the
//! stretches of speech it finds.
//!
//! Silero takes 512 new samples with the 64 before them and its own state
//! from the chunk before, and says how likely the chunk is to be speech. A
//! stretch starts when that reaches one half and ends once it has stayed
//! below about a third for 300 ms, and a stretch shorter than a quarter of a
//! second is dropped as a click. A pack without Silero hears by loudness
//! instead, which knows speech from silence and nothing finer.
//!
//! # Thread role and contract
//!
//! An [`Ear`] owns its session and is never shared: each chunk is an FFI
//! call (14-ENGINEERING-RULES §1.3). It runs on the processor, where a model
//! this small costs less than handing it to the graphics card would. The
//! [`Tracker`] is pure state.

use std::path::Path;

use crate::{
    error::MlError,
    manifest,
    session::{Feed, Held, Place, Prefer, Session},
};

/// New samples in each chunk.
pub const CHUNK: usize = 512;
/// Samples of the chunk before carried into each one.
const CONTEXT: usize = 64;
/// At or above this a chunk starts speech.
const START: f32 = 0.5;
/// Below this a chunk counts towards silence.
const STOP: f32 = 0.35;
/// Chunks of silence that end a stretch: about 300 ms.
const QUIET: usize = 10;
/// Chunks a stretch must last to count: about 250 ms.
const SHORTEST: usize = 8;
/// Loudness that counts as speech when there is no model: about -40 dBFS.
const LOUD: f32 = 1e-4;

/// Something that hears speech.
#[derive(Debug)]
pub enum Ear {
    /// Silero.
    Model(Box<Silero>),
    /// Loudness alone.
    Loudness,
}

/// Silero, open, with the state it carries from chunk to chunk.
#[derive(Debug)]
pub struct Silero {
    session: Session,
    names: manifest::Whisper,
    state: Option<Held>,
    context: [f32; CONTEXT],
}

impl Ear {
    /// Silero from the pack's folder, or loudness when the pack has none.
    ///
    /// # Errors
    ///
    /// Whatever [`Session::open`] answers for a file that is named and will
    /// not open.
    pub fn open(dir: &Path, names: &manifest::Whisper) -> Result<Self, MlError> {
        let Some(file) = &names.vad else {
            return Ok(Ear::Loudness);
        };
        Ok(Ear::Model(Box::new(Silero {
            session: Session::open(dir, file, Prefer::Cpu)?,
            names: names.clone(),
            state: None,
            context: [0.0; CONTEXT],
        })))
    }

    /// Start again, for a new stream.
    pub fn reset(&mut self) {
        if let Ear::Model(silero) = self {
            silero.state = None;
            silero.context = [0.0; CONTEXT];
        }
    }

    /// How likely one chunk of [`CHUNK`] samples is to be speech.
    ///
    /// # Errors
    ///
    /// Whatever the run refuses.
    pub fn hear(&mut self, chunk: &[f32]) -> Result<f32, MlError> {
        match self {
            Ear::Loudness => {
                let power = chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len().max(1) as f32;
                Ok(if power > LOUD { 1.0 } else { 0.0 })
            }
            Ear::Model(silero) => silero.hear(chunk),
        }
    }
}

impl Silero {
    fn hear(&mut self, chunk: &[f32]) -> Result<f32, MlError> {
        let mut input = [0.0f32; CONTEXT + CHUNK];
        input[..CONTEXT].copy_from_slice(&self.context);
        let take = chunk.len().min(CHUNK);
        input[CONTEXT..CONTEXT + take].copy_from_slice(&chunk[..take]);
        self.context.copy_from_slice(&input[CHUNK..]);
        let zeros = [0.0f32; 2 * 128];
        let rate = [16_000i64];
        let state = match &self.state {
            Some(held) => Feed::Held(held),
            None => Feed::F32(&[2, 1, 128], &zeros),
        };
        let mut out = self.session.bound(
            &[
                (
                    self.names.vad_input.as_str(),
                    Feed::F32(&[1, CONTEXT + CHUNK], &input),
                ),
                (self.names.vad_state.as_str(), state),
                (self.names.vad_rate.as_str(), Feed::I64(&[], &rate)),
            ],
            &[
                (self.names.vad_output.as_str(), Place::Host),
                (self.names.vad_state_out.as_str(), Place::Host),
            ],
        )?;
        self.state = out.pop();
        let probability = out
            .first()
            .ok_or(MlError::ShapeMismatch)?
            .read()?
            .1
            .first()
            .copied()
            .ok_or(MlError::ShapeMismatch)?;
        Ok(probability)
    }
}

/// One stretch of speech, in chunks: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stretch {
    /// The first chunk of speech.
    pub start: usize,
    /// The first chunk after it.
    pub end: usize,
}

/// Turns chunk probabilities into stretches as they arrive.
#[derive(Debug, Default)]
pub struct Tracker {
    /// The start of the stretch in progress.
    open: Option<usize>,
    /// The first quiet chunk since speech, while in a stretch.
    quiet_from: Option<usize>,
}

impl Tracker {
    /// Read chunk `at`'s probability. A stretch that has just ended comes
    /// back.
    pub fn push(&mut self, at: usize, probability: f32) -> Option<Stretch> {
        match self.open {
            None => {
                if probability >= START {
                    self.open = Some(at);
                    self.quiet_from = None;
                }
                None
            }
            Some(start) => {
                if probability >= START {
                    self.quiet_from = None;
                    return None;
                }
                if probability < STOP && self.quiet_from.is_none() {
                    self.quiet_from = Some(at);
                }
                match self.quiet_from {
                    Some(quiet) if at + 1 - quiet >= QUIET => {
                        self.open = None;
                        self.quiet_from = None;
                        (quiet - start >= SHORTEST).then_some(Stretch { start, end: quiet })
                    }
                    _ => None,
                }
            }
        }
    }

    /// The start of the stretch in progress, if speech is going on.
    #[must_use]
    pub fn open(&self) -> Option<usize> {
        self.open
    }

    /// End the stretch in progress at chunk `at`, whether or not it has gone
    /// quiet, and carry straight on with a new one from there when `carry`.
    pub fn cut(&mut self, at: usize, carry: bool) -> Option<Stretch> {
        let start = self.open.take()?;
        let end = self.quiet_from.take().unwrap_or(at).min(at);
        if carry {
            self.open = Some(at);
        }
        (end > start).then_some(Stretch { start, end })
    }
}

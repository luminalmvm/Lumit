//! The sound the page decoded, read as the desktop reads a file's.

use std::path::Path;
use std::sync::Arc;

use crate::audio::{AudioBuffer, AudioReader};
use crate::MediaError;

/// `samples`, interleaved stereo at `from`, at `rate`: the same samples when
/// the two agree, and otherwise each drawn between its two neighbours.
// ponytail: linear, so a resampled file loses a little of its top end. A
// windowed sinc is the upgrade if anyone hears it.
fn at_rate(from: u32, samples: Arc<[f32]>, rate: u32) -> Arc<[f32]> {
    if from == rate || from == 0 || rate == 0 {
        return samples;
    }
    let frames = samples.len() / 2;
    let out_frames = (frames as u64 * u64::from(rate) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(rate);
    let sample = |frame: usize, channel: usize| {
        samples
            .get(frame.min(frames.saturating_sub(1)) * 2 + channel)
            .copied()
            .unwrap_or(0.0)
    };
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let x = i as f64 * step;
        let (a, t) = (x as usize, x.fract() as f32);
        for channel in 0..2 {
            let (s0, s1) = (sample(a, channel), sample(a + 1, channel));
            out.push(s0 + (s1 - s0) * t);
        }
    }
    out.into()
}

// ponytail: resampled afresh for each reader. Keep the last rate's beside the
// sound if long files open slowly.
fn sound_at(path: &Path, rate: u32) -> Result<Arc<[f32]>, MediaError> {
    let (from, samples) = super::sound(path).ok_or(MediaError::NoStreams)?;
    Ok(at_rate(from, samples, rate))
}

/// All of `path`'s sound at `target_rate`.
pub(crate) fn decode_all(path: &Path, target_rate: u32) -> Result<AudioBuffer, MediaError> {
    Ok(AudioBuffer {
        rate: target_rate,
        samples: sound_at(path, target_rate)?.to_vec(),
    })
}

/// One reader of a file's sound. It is all in memory, so every frame of it
/// is as near as any other.
pub(crate) struct Sound {
    samples: Arc<[f32]>,
    next: u64,
}

impl Sound {
    fn frames(&self) -> u64 {
        (self.samples.len() / 2) as u64
    }
}

impl AudioReader {
    pub fn open(path: &Path, target_rate: u32) -> Result<Self, MediaError> {
        Self::open_stream(path, 0, target_rate)
    }

    /// The page decodes a file's first sound only.
    pub fn open_stream(path: &Path, stream: u32, target_rate: u32) -> Result<Self, MediaError> {
        if stream != 0 {
            return Err(MediaError::NoStreams);
        }
        Ok(Self(Sound {
            samples: sound_at(path, target_rate)?,
            next: 0,
        }))
    }

    #[must_use]
    pub fn lands(&self) -> bool {
        true
    }

    #[must_use]
    pub fn position(&self) -> u64 {
        self.0.next
    }

    #[must_use]
    pub fn reaches(&self, _start: u64, _exact: bool) -> Option<u64> {
        Some(0)
    }

    #[must_use]
    pub fn total(&self) -> Option<u64> {
        Some(self.0.frames())
    }

    pub fn set_total(&mut self, _total: u64) {}

    pub fn read(
        &mut self,
        start: u64,
        frames: usize,
        _exact: bool,
    ) -> Result<(Vec<f32>, bool), MediaError> {
        let start = start.min(self.0.frames());
        let end = start.saturating_add(frames as u64).min(self.0.frames());
        let out = self
            .0
            .samples
            .get(start as usize * 2..end as usize * 2)
            .unwrap_or_default()
            .to_vec();
        self.0.next = end;
        Ok((out, true))
    }

    pub fn length(&mut self) -> Result<u64, MediaError> {
        Ok(self.0.frames())
    }
}

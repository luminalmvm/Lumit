//! The picture of sound Whisper reads: a log-mel spectrogram of one window,
//! made exactly the way the model was trained on.
//!
//! Thirty seconds of 16 kHz audio, padded with silence when there is less,
//! cut into 25 ms frames every 10 ms with a Hann window, turned into power
//! per frequency, gathered into 80 or 128 mel bands with Slaney's filters
//! (the ones librosa makes by default), then put on a log scale that never
//! reaches more than eight decades below the loudest point and shifted into
//! the small range the model expects.
//!
//! # Thread role and contract
//!
//! Pure arithmetic on whichever thread runs the transcription. A [`Mel`] owns
//! its filters and its FFT plan and is never shared; nothing here does IO or
//! reads a clock.

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

/// Samples a second.
pub const RATE: usize = 16_000;
/// Samples in one window: thirty seconds.
pub const WINDOW: usize = 30 * RATE;
/// Samples between two frames: 10 ms.
pub const HOP: usize = 160;
/// Frames in one window.
pub const FRAMES: usize = WINDOW / HOP;
/// The FFT's length: 25 ms.
const FFT: usize = 400;
/// Frequency bins the FFT hands back.
const BINS: usize = FFT / 2 + 1;

/// A spectrogram maker for one band count.
pub struct Mel {
    bands: usize,
    /// `bands` rows of [`BINS`] weights.
    filters: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
}

impl std::fmt::Debug for Mel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mel").field("bands", &self.bands).finish()
    }
}

impl Mel {
    /// A maker for `bands` mel bands, 80 or 128 for the Whisper there is.
    #[must_use]
    pub fn new(bands: usize) -> Self {
        // A periodic Hann window, as torch makes it.
        let window = (0..FFT)
            .map(|n| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / FFT as f32).cos())
            .collect();
        Mel {
            bands,
            filters: slaney(bands),
            window,
            fft: RealFftPlanner::<f32>::new().plan_fft_forward(FFT),
        }
    }

    /// The spectrogram of up to thirty seconds of samples, `bands` rows of
    /// [`FRAMES`] numbers. More than thirty seconds is cut, less is padded
    /// with silence.
    #[must_use]
    pub fn log_mel(&self, samples: &[f32]) -> Vec<f32> {
        let mut audio = vec![0.0f32; WINDOW];
        let take = samples.len().min(WINDOW);
        audio[..take].copy_from_slice(&samples[..take]);

        let mut frame = self.fft.make_input_vec();
        let mut spectrum = self.fft.make_output_vec();
        let mut scratch = self.fft.make_scratch_vec();
        let mut power = vec![0.0f32; BINS];
        let mut out = vec![0.0f32; self.bands * FRAMES];
        let half = (FFT / 2) as isize;
        for at in 0..FRAMES {
            // Centred frames, the audio reflected at both ends.
            let centre = (at * HOP) as isize;
            for (n, slot) in frame.iter_mut().enumerate() {
                let index = reflect(centre - half + n as isize, WINDOW);
                *slot = audio[index] * self.window[n];
            }
            if self
                .fft
                .process_with_scratch(&mut frame, &mut spectrum, &mut scratch)
                .is_err()
            {
                continue;
            }
            for (slot, bin) in power.iter_mut().zip(&spectrum) {
                *slot = bin.norm_sqr();
            }
            for band in 0..self.bands {
                let weights = &self.filters[band * BINS..(band + 1) * BINS];
                let energy: f32 = weights.iter().zip(&power).map(|(w, p)| w * p).sum();
                out[band * FRAMES + at] = energy.max(1e-10).log10();
            }
        }
        let top = out.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        for value in &mut out {
            *value = (value.max(top - 8.0) + 4.0) / 4.0;
        }
        out
    }
}

/// An index walked off either end of `len` samples, reflected back in
/// without repeating the edge sample, as numpy's reflect padding does.
fn reflect(index: isize, len: usize) -> usize {
    let len = len as isize;
    let mut i = index;
    if i < 0 {
        i = -i;
    }
    if i >= len {
        i = 2 * (len - 1) - i;
    }
    i.clamp(0, len - 1) as usize
}

/// Slaney's mel filters for 16 kHz and a 400-point FFT, area normalised, as
/// `librosa.filters.mel` makes them.
fn slaney(bands: usize) -> Vec<f32> {
    let top = mel_of(RATE as f64 / 2.0);
    let points: Vec<f64> = (0..bands + 2)
        .map(|i| hz_of(top * i as f64 / (bands + 1) as f64))
        .collect();
    let mut filters = vec![0.0f32; bands * BINS];
    for band in 0..bands {
        let (low, centre, high) = (points[band], points[band + 1], points[band + 2]);
        let norm = 2.0 / (high - low);
        for bin in 0..BINS {
            let hz = bin as f64 * RATE as f64 / FFT as f64;
            let rising = (hz - low) / (centre - low);
            let falling = (high - hz) / (high - centre);
            filters[band * BINS + bin] = (rising.min(falling).max(0.0) * norm) as f32;
        }
    }
    filters
}

/// Hertz to Slaney mels: linear below 1 kHz and logarithmic above.
fn mel_of(hz: f64) -> f64 {
    if hz < 1000.0 {
        hz * 3.0 / 200.0
    } else {
        15.0 + (hz / 1000.0).ln() * 27.0 / 6.4f64.ln()
    }
}

/// Slaney mels back to hertz.
fn hz_of(mel: f64) -> f64 {
    if mel < 15.0 {
        mel * 200.0 / 3.0
    } else {
        1000.0 * ((mel - 15.0) * 6.4f64.ln() / 27.0).exp()
    }
}

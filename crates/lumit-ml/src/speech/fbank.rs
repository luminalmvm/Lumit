//! The picture of sound the speaker model reads: Kaldi's log filterbank, the
//! recipe the 3D-Speaker models were trained on and sherpa-onnx feeds them.
//!
//! Samples stay in -1..1, as the model file's own `normalize_samples` says.
//! Frames are 25 ms every 10 ms and centred, with the audio reflected at both
//! ends. Each frame loses its mean, is pre-emphasised by 0.97 and shaped by
//! Povey's window, then goes through a 512-point FFT into 80 mel bands from
//! 20 Hz to 7.6 kHz on Kaldi's mel scale. The log of each band, floored at
//! the smallest f32 step, is the feature. The file's `global-mean` then asks
//! for each band's mean over the stretch to be taken away, which is what
//! [`fbank`] does last.
//!
//! # Thread role and contract
//!
//! Pure arithmetic on whichever thread runs the transcription. No IO, no
//! clocks, nothing shared.

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

/// Bands a frame is gathered into.
pub const BANDS: usize = 80;
/// Samples in one frame: 25 ms.
const LENGTH: usize = 400;
/// Samples between frames: 10 ms.
const SHIFT: usize = 160;
/// The FFT's length, the frame rounded up to a power of two.
const FFT: usize = 512;

/// A filterbank maker.
pub struct Fbank {
    /// [`BANDS`] rows of `FFT / 2` weights.
    filters: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
}

impl std::fmt::Debug for Fbank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Fbank")
    }
}

impl Default for Fbank {
    fn default() -> Self {
        Self::new()
    }
}

impl Fbank {
    /// The one maker the speaker models need.
    #[must_use]
    pub fn new() -> Self {
        let window = (0..LENGTH)
            .map(|n| {
                let hann =
                    0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (LENGTH - 1) as f64).cos();
                hann.powf(0.85) as f32
            })
            .collect();
        Fbank {
            filters: kaldi_mel(),
            window,
            fft: RealFftPlanner::<f32>::new().plan_fft_forward(FFT),
        }
    }

    /// The features of `samples` at 16 kHz, one row of [`BANDS`] per frame,
    /// with each band's mean taken away. Returns the rows and their count.
    #[must_use]
    pub fn fbank(&self, samples: &[f32]) -> (Vec<f32>, usize) {
        let frames = (samples.len() + SHIFT / 2) / SHIFT;
        let mut out = vec![0.0f32; frames * BANDS];
        if samples.is_empty() {
            return (out, 0);
        }
        let mut frame = self.fft.make_input_vec();
        let mut spectrum = self.fft.make_output_vec();
        let mut scratch = self.fft.make_scratch_vec();
        let mut power = vec![0.0f32; FFT / 2];
        let len = samples.len() as isize;
        for at in 0..frames {
            let first = (at * SHIFT + SHIFT / 2) as isize - (LENGTH / 2) as isize;
            let mut piece = [0.0f32; LENGTH];
            for (n, slot) in piece.iter_mut().enumerate() {
                let mut i = first + n as isize;
                if i < 0 {
                    i = -i - 1;
                }
                if i >= len {
                    i = 2 * len - 1 - i;
                }
                *slot = samples[i.clamp(0, len - 1) as usize];
            }
            let mean = piece.iter().sum::<f32>() / LENGTH as f32;
            for value in &mut piece {
                *value -= mean;
            }
            for n in (1..LENGTH).rev() {
                piece[n] -= 0.97 * piece[n - 1];
            }
            piece[0] -= 0.97 * piece[0];
            frame.fill(0.0);
            for ((slot, value), shape) in frame.iter_mut().zip(piece).zip(&self.window) {
                *slot = value * shape;
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
            for band in 0..BANDS {
                let weights = &self.filters[band * FFT / 2..(band + 1) * FFT / 2];
                let energy: f32 = weights.iter().zip(&power).map(|(w, p)| w * p).sum();
                out[at * BANDS + band] = energy.max(f32::EPSILON).ln();
            }
        }
        for band in 0..BANDS {
            let mean = (0..frames).map(|at| out[at * BANDS + band]).sum::<f32>() / frames as f32;
            for at in 0..frames {
                out[at * BANDS + band] -= mean;
            }
        }
        (out, frames)
    }
}

/// Kaldi's triangular mel filters, built and applied on its own mel scale.
fn kaldi_mel() -> Vec<f32> {
    let mel = |hz: f64| 1127.0 * (1.0 + hz / 700.0).ln();
    let (low, high) = (mel(20.0), mel(7600.0));
    let step = (high - low) / (BANDS + 1) as f64;
    let half = FFT / 2;
    let mut filters = vec![0.0f32; BANDS * half];
    for band in 0..BANDS {
        let left = low + band as f64 * step;
        let centre = left + step;
        let right = centre + step;
        for bin in 0..half {
            let at = mel(bin as f64 * 16_000.0 / FFT as f64);
            if at > left && at < right {
                let weight = if at <= centre {
                    (at - left) / (centre - left)
                } else {
                    (right - at) / (right - centre)
                };
                filters[band * half + bin] = weight as f32;
            }
        }
    }
    filters
}

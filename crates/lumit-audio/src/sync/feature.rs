//! The coarse onset curve sync compares: 200 frames a second.
//!
//! Each frame asks how much new energy arrived in any of 16 bands. The log of
//! each band's energy makes a quieter copy of a sound step the same way, and
//! the moving mean and spread taken out at the end cope with gain riding, AGC,
//! and a lav against a camera mic.

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

use super::{Onsets, FPS, HOP, RATE};

/// 32 ms at 8 kHz.
const WINDOW: usize = 256;
const BANDS: usize = 16;
const LOW_HZ: f32 = 100.0;
const HIGH_HZ: f32 = 3600.0;
/// Scales band energy inside the log. Sound more than about 70 dB below full
/// scale stays near the linear part, so hiss does not make onsets of its own,
/// and above it a gain change only shifts the log, which the difference
/// takes out.
const KNEE: f32 = 1e6;
/// Frames more than 50 dB below the file's loud part count as silence.
const SILENCE: f32 = 1e-5;

/// Builds a [`Onsets`] from mono 8 kHz sound handed over in blocks of any
/// size. The frames do not depend on where the blocks split.
pub(crate) struct Builder {
    fft: Arc<dyn RealToComplex<f32>>,
    hann: Vec<f32>,
    /// Which band each FFT bin adds to.
    band: Vec<Option<usize>>,
    /// Sound not yet stepped past.
    pending: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    prev: Option<[f32; BANDS]>,
    flux: Vec<f32>,
    level: Vec<f32>,
    samples: u64,
}

impl Builder {
    pub(crate) fn new() -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        let hann = (0..WINDOW)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos())
            .collect();
        let span = (HIGH_HZ / LOW_HZ).ln();
        let band = (0..=WINDOW / 2)
            .map(|k| {
                let hz = k as f32 * RATE as f32 / WINDOW as f32;
                (LOW_HZ..HIGH_HZ)
                    .contains(&hz)
                    .then(|| (((hz / LOW_HZ).ln() / span * BANDS as f32) as usize).min(BANDS - 1))
            })
            .collect();
        Self {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            hann,
            band,
            pending: Vec::new(),
            prev: None,
            flux: Vec::new(),
            level: Vec::new(),
            samples: 0,
        }
    }

    /// Add the next stretch of sound.
    pub(crate) fn push(&mut self, mono: &[f32]) {
        self.samples += mono.len() as u64;
        let mut pending = std::mem::take(&mut self.pending);
        pending.extend_from_slice(mono);
        let mut at = 0;
        while let Some(frame) = pending.get(at..at + WINDOW) {
            self.step(frame);
            at += HOP;
        }
        pending.drain(..at.min(pending.len()));
        self.pending = pending;
    }

    fn step(&mut self, frame: &[f32]) {
        for ((x, s), w) in self.input.iter_mut().zip(frame).zip(&self.hann) {
            *x = s * w;
        }
        let mut bands = [0f32; BANDS];
        let mut total = 0f32;
        let ok = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_ok();
        if ok {
            for (c, b) in self.spectrum.iter().zip(&self.band) {
                let e = c.norm_sqr() / WINDOW as f32;
                total += e;
                if let Some(slot) = b.and_then(|b| bands.get_mut(b)) {
                    *slot += e;
                }
            }
        }
        let logs = bands.map(|e| (1.0 + KNEE * e).ln());
        // The first frame has nothing to differ from, so it is no onset.
        let flux = self.prev.map_or(0.0, |prev| {
            logs.iter().zip(prev).map(|(l, p)| (l - p).max(0.0)).sum()
        });
        self.prev = Some(logs);
        self.flux.push(flux);
        self.level.push(total);
    }

    /// The finished [`Onsets`].
    pub(crate) fn finish(self) -> Onsets {
        // Take out the slow part, so a sound getting louder overall is not
        // read as one long onset, then scale to a unit spread over 10 s.
        let mean = moving(&self.flux, FPS / 2);
        let high: Vec<f32> = self
            .flux
            .iter()
            .zip(&mean)
            .map(|(f, m)| (f64::from(*f) - m) as f32)
            .collect();
        let squares: Vec<f32> = high.iter().map(|h| h * h).collect();
        let spread_mean = moving(&high, 5 * FPS);
        let spread_square = moving(&squares, 5 * FPS);
        let floor = loud(&self.level) * SILENCE;
        let feature = high
            .iter()
            .zip(spread_mean.iter().zip(&spread_square))
            .zip(&self.level)
            .map(|((h, (m, s)), level)| {
                let sd = (s - m * m).max(0.0).sqrt();
                if *level <= floor || sd < 1e-9 {
                    0.0
                } else {
                    (f64::from(*h) / sd) as f32
                }
            })
            .collect();
        Onsets {
            feature,
            seconds: self.samples as f64 / f64::from(RATE),
        }
    }
}

/// The mean of `x` over `half` frames either side of each frame, fewer at
/// the ends. Prefix sums in f64 keep it exact enough over hours.
fn moving(x: &[f32], half: usize) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(x.len() + 1);
    prefix.push(0f64);
    let mut sum = 0f64;
    for v in x {
        sum += f64::from(*v);
        prefix.push(sum);
    }
    (0..x.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(x.len());
            match (prefix.get(hi), prefix.get(lo)) {
                (Some(h), Some(l)) => (h - l) / (hi - lo) as f64,
                _ => 0.0,
            }
        })
        .collect()
}

/// The file's 95th percentile frame energy: its loud part.
fn loud(level: &[f32]) -> f32 {
    if level.is_empty() {
        return 0.0;
    }
    let mut sorted = level.to_vec();
    let at = (sorted.len() - 1) * 95 / 100;
    let (_, value, _) = sorted.select_nth_unstable_by(at, f32::total_cmp);
    *value
}

//! Weighted prediction error: Reverb reduction's Best quality pass
//! (Nakatani and others, 2010).
//!
//! # In plain terms
//!
//! Late reverb is the sound from a moment ago arriving again, so in each
//! frequency band it can be predicted from the frames a little way back. WPE
//! finds, per band, the 20 weights over the frames from 3 to 22 back that
//! predict the present best, weighting each frame by how quiet the clean
//! estimate is there, and subtracts the prediction. Two rounds: the second
//! reweights by the first's answer. It is a linear filter per band, so it
//! leaves no musical noise behind, and the spectral stage after it has that
//! much less to guess at.
//!
//! The look only finds the weights ([`fit`]); the effect applies them frame
//! by frame in the transform it already runs ([`Filters::predict`]), so
//! nothing the size of the sound is kept. The span is cut into stretches of
//! about 24 seconds, each with its own weights; the frames before a stretch
//! are read for its first taps, as the effect has them to hand. Bands are
//! independent and run across the rayon pool, each writing only its own
//! weights, so the answer does not depend on the scheduling.

use realfft::num_complex::Complex;

use super::profile::Framing;
use crate::fx::audio_chain::Denormals;
use crate::fx::effects::audio::dsp::stft::{frame_at, frame_count, Fft};

/// The prediction's taps and the frames skipped before them.
pub(super) const TAPS: usize = 20;
pub(super) const DELAY: usize = 3;

/// Frames of past spectra the prediction reaches back over.
pub(super) const REACH: usize = DELAY + TAPS - 1;

/// Rounds of reweighting.
const ITERATIONS: usize = 2;

/// About how long one stretch with its own weights is, in seconds.
const STRETCH_S: f64 = 24.0;

/// Every stretch's weights, `[stretch][channel][bin][tap]`.
pub(super) struct Filters {
    /// Frames in a stretch.
    size: usize,
    stretches: usize,
    channels: usize,
    bins: usize,
    taps: Vec<Complex<f32>>,
}

impl Filters {
    /// Frame `j`'s prediction of the late part of `now`, taken away from it:
    /// `past(d)` is the spectrum `d` frames back (one is the frame before).
    pub(super) fn predict<'a>(
        &self,
        j: u64,
        channel: usize,
        now: &mut [Complex<f32>],
        past: impl Fn(usize) -> &'a [Complex<f32>],
    ) {
        let stretch = usize::try_from(j / self.size.max(1) as u64)
            .unwrap_or(usize::MAX)
            .min(self.stretches.saturating_sub(1));
        let base = (stretch * self.channels + channel) * self.bins * TAPS;
        for (k, slot) in now.iter_mut().enumerate().take(self.bins) {
            let g = self
                .taps
                .get(base + k * TAPS..base + (k + 1) * TAPS)
                .unwrap_or(&[]);
            let mut predicted = Complex::new(0.0f32, 0.0);
            for (tap, gk) in g.iter().enumerate() {
                if let Some(y) = past(DELAY + tap).get(k) {
                    predicted += gk.conj() * y;
                }
            }
            *slot -= predicted;
        }
    }
}

/// The weights for the whole of `interleaved`, cut into the frames of
/// `framing`, frame `j` the same frame the effect's framer hands it as `j`.
#[must_use]
pub(super) fn fit(interleaved: &[f32], channels: usize, framing: &Framing) -> Filters {
    use rayon::prelude::*;
    let channels = channels.max(1);
    let (n, hop, bins) = (framing.n, framing.hop, framing.bins);
    let len = interleaved.len() / channels;
    let total = frame_count(len, n, hop).max(1);
    let per = ((STRETCH_S * f64::from(framing.rate.max(1)) / hop as f64) as usize).max(1);
    let stretches = ((total as f64 / per as f64).round() as usize).max(1);
    let size = total.div_ceil(stretches);
    let mut taps = vec![Complex::new(0.0f32, 0.0); stretches * channels * bins * TAPS];
    let mut fft = Fft::new(n);
    let mut frame = vec![0.0f32; n];
    let mut spectrum = fft.spectrum();

    for s in 0..stretches {
        let a = s * size;
        let b = (a + size).min(total);
        if a >= b {
            continue;
        }
        let from = a.saturating_sub(REACH);
        let span = b - from;
        for c in 0..channels {
            // The stretch's spectrogram, one column per bin.
            let mut columns = vec![Complex::new(0.0f32, 0.0); bins * span];
            for (t, j) in (from..b).enumerate() {
                frame_at(
                    interleaved,
                    channels,
                    c,
                    j,
                    hop,
                    &framing.window,
                    &mut frame,
                );
                fft.forward(&mut frame, &mut spectrum);
                for (k, x) in spectrum.iter().enumerate() {
                    if let Some(slot) = columns.get_mut(k * span + t) {
                        *slot = *x;
                    }
                }
            }
            let at = (s * channels + c) * bins * TAPS;
            if let Some(out) = taps.get_mut(at..at + bins * TAPS) {
                out.par_chunks_mut(TAPS)
                    .zip(columns.par_chunks(span.max(1)))
                    .for_each(|(g, column)| {
                        let _flush = Denormals::on();
                        weights(column, a - from, g);
                    });
            }
        }
    }
    Filters {
        size,
        stretches,
        channels,
        bins,
        taps,
    }
}

/// One bin's frames, the first `start` of them context only: the weights
/// that predict the rest best, into `out`. Noughts, which predict nothing,
/// where the bin is silent or the solve fails.
// A dense twenty by twenty system reads plainest by index.
#[allow(clippy::needless_range_loop)]
fn weights(y: &[Complex<f32>], start: usize, out: &mut [Complex<f32>]) {
    let frames = y.len();
    if frames <= start {
        return;
    }
    let past = |t: usize, k: usize| -> Complex<f64> {
        t.checked_sub(DELAY + k)
            .and_then(|i| y.get(i))
            .map_or(Complex::new(0.0, 0.0), |v| {
                Complex::new(f64::from(v.re), f64::from(v.im))
            })
    };
    let mean: f64 = y.iter().map(|v| f64::from(v.norm_sqr())).sum::<f64>() / frames as f64;
    if mean <= 0.0 {
        return;
    }
    let floor = mean * 1e-6 + 1e-20;
    let mut estimate: Vec<Complex<f64>> = y
        .iter()
        .map(|v| Complex::new(f64::from(v.re), f64::from(v.im)))
        .collect();
    let mut r = [[Complex::new(0.0f64, 0.0); TAPS]; TAPS];
    let mut p = [Complex::new(0.0f64, 0.0); TAPS];
    let mut taps = [Complex::new(0.0f64, 0.0); TAPS];
    let mut g = [Complex::new(0.0f64, 0.0); TAPS];
    for _ in 0..ITERATIONS {
        for row in r.iter_mut() {
            row.fill(Complex::new(0.0, 0.0));
        }
        p.fill(Complex::new(0.0, 0.0));
        for t in start..frames {
            let weight = 1.0 / estimate.get(t).map_or(floor, |e| e.norm_sqr().max(floor));
            for (k, slot) in taps.iter_mut().enumerate() {
                *slot = past(t, k);
            }
            let now = past(t + DELAY, 0);
            for (i, row) in r.iter_mut().enumerate() {
                let a = taps.get(i).copied().unwrap_or_default() * weight;
                for (rij, bj) in row.iter_mut().zip(&taps).skip(i) {
                    *rij += a * bj.conj();
                }
                if let Some(pi) = p.get_mut(i) {
                    *pi += a * now.conj();
                }
            }
        }
        // The lower half is the upper's mirror, and a little loading keeps
        // the solve well away from a silent band's singular matrix.
        let trace: f64 = (0..TAPS).map(|i| r[i][i].re).sum();
        let loading = trace / TAPS as f64 * 1e-6 + 1e-30;
        for i in 0..TAPS {
            r[i][i].re += loading;
            r[i][i].im = 0.0;
            for j in 0..i {
                r[i][j] = r[j][i].conj();
            }
        }
        if !solve(&mut r, &p, &mut g) {
            return;
        }
        for t in start..frames {
            let mut predicted = Complex::new(0.0, 0.0);
            for (k, gk) in g.iter().enumerate() {
                predicted += gk.conj() * past(t, k);
            }
            if let Some(e) = estimate.get_mut(t) {
                *e = past(t + DELAY, 0) - predicted;
            }
        }
    }
    for (slot, gk) in out.iter_mut().zip(&g) {
        *slot = Complex::new(gk.re as f32, gk.im as f32);
    }
}

/// Solve the Hermitian positive-definite `r x = p` by Cholesky, in place.
/// `false` when the matrix is not positive definite.
#[allow(clippy::needless_range_loop)]
fn solve(
    r: &mut [[Complex<f64>; TAPS]; TAPS],
    p: &[Complex<f64>; TAPS],
    x: &mut [Complex<f64>; TAPS],
) -> bool {
    // r = L L^H, L kept in the lower triangle.
    for j in 0..TAPS {
        let mut d = r[j][j].re;
        for k in 0..j {
            d -= r[j][k].norm_sqr();
        }
        if d <= 0.0 || !d.is_finite() {
            return false;
        }
        let d = d.sqrt();
        r[j][j] = Complex::new(d, 0.0);
        for i in j + 1..TAPS {
            let mut s = r[i][j];
            for k in 0..j {
                s -= r[i][k] * r[j][k].conj();
            }
            r[i][j] = s / d;
        }
    }
    let mut y = [Complex::new(0.0f64, 0.0); TAPS];
    for i in 0..TAPS {
        let mut s = p[i];
        for k in 0..i {
            s -= r[i][k] * y[k];
        }
        y[i] = s / r[i][i].re;
    }
    for i in (0..TAPS).rev() {
        let mut s = y[i];
        for k in i + 1..TAPS {
            s -= r[k][i].conj() * x[k];
        }
        x[i] = s / r[i][i].re;
    }
    true
}

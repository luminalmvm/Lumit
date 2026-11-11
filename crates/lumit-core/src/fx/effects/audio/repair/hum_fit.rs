//! Hum removal's whole-span fit: where the mains is at every moment, and how
//! loud and in what phase each harmonic is.
//!
//! # In plain terms
//!
//! Everything here runs once, in the look, over the whole input, and leaves
//! behind a small table a block can read: a point every hundredth of a second
//! holding the mains frequency, its phase, and each harmonic's level and
//! phase on each channel.
//!
//! A harmonic is measured by turning the sound down by exactly that
//! harmonic's frequency, so the harmonic itself becomes a slowly moving
//! number near nought hertz, and everything else lands well away from it.
//! A narrow low pass, run forwards and back so it does not lag, then keeps
//! only the harmonic: Width hertz of it. The removal is that number turned
//! back up. A Width of one hertz takes out a hertz around each harmonic and
//! nothing else.
//!
//! That only works if the frequency is right, and twelve times more so at the
//! twelfth harmonic, where a mains that drifts by a quarter of a hertz moves
//! by three. So the rough track the long transforms find is refined first:
//! each of the first eight harmonics, measured as above with a looser filter,
//! turns at a rate that says how far off the track is there, and the
//! harmonics' answers are averaged by how strong each is, smoothed to the
//! slow way the mains really moves, and added on. Twice.
//!
//! The span is read in a fixed order and the points are split between rayon
//! tasks by range, each point summed by exactly one task in sample order, so
//! the answer never depends on the scheduling.

use realfft::num_complex::Complex;

use crate::fx::audio_chain::{Denormals, AUDIO_BLOCK_FRAMES, AUDIO_CHANNELS};
use crate::fx::effects::audio::dsp::biquad::Coeffs;

/// Points per second in the table.
pub const POINTS_PER_S: f64 = 100.0;

/// Harmonics the refinement listens to.
const REFINE_HARMONICS: usize = 8;

/// The refinement's measuring band, in hertz either side of a harmonic.
const REFINE_HALF_HZ: f64 = 1.0;

/// How fast the correction may move, in hertz: the mains wanders over
/// seconds, not tenths of one.
const CORRECTION_HZ: f64 = 0.2;

/// Rounds of refinement.
const ROUNDS: usize = 2;

/// How many points at each end a filter is started from, half a second.
const SETTLE_POINTS: usize = 50;

/// Points handed to one rayon task.
const CHUNK: usize = 512;

/// A sample this quiet or quieter is digital silence, -120 dBFS.
const SILENT: f32 = 1e-6;

/// The table a block reads.
pub struct HumTable {
    /// Samples between points.
    pub step: usize,
    /// The fundamental at each point, in hertz.
    pub f0: Vec<f64>,
    /// The fundamental's phase at each point, in radians.
    pub phase: Vec<f64>,
    /// Harmonics held per point and channel.
    pub harmonics: usize,
    /// Each harmonic's complex level, point by point, `[k][c][h]`, scaled so
    /// the harmonic is `Re(a e^(i h phase))`.
    pub amps: Vec<Complex<f32>>,
    /// The runs of digital silence at least a step long, as first frame and
    /// the frame after the last, in order. Nothing is taken from them.
    pub silence: Vec<(usize, usize)>,
    rate: f64,
}

impl HumTable {
    /// The fundamental's phase at sample `t`, the frequency drawn straight
    /// between points. The look measures with this very function, so a block
    /// takes out exactly what was measured.
    #[must_use]
    pub fn phase_at(&self, t: usize) -> f64 {
        phase_at(&self.f0, &self.phase, self.step, self.rate, t)
    }

    /// Harmonic `h` (from nought) of channel `c` at sample `t`, as a complex
    /// level drawn straight between points.
    #[must_use]
    pub fn amp_at(&self, t: usize, c: usize, h: usize) -> Complex<f64> {
        let step = self.step.max(1);
        let k = t / step;
        let f = (t - k * step) as f64 / step as f64;
        let at = |k: usize| -> Complex<f64> {
            let i = (k * AUDIO_CHANNELS + c) * self.harmonics + h;
            self.amps.get(i).map_or(Complex::new(0.0, 0.0), |a| {
                Complex::new(f64::from(a.re), f64::from(a.im))
            })
        };
        let last = self.f0.len().saturating_sub(1);
        at(k.min(last)) * (1.0 - f) + at((k + 1).min(last)) * f
    }
}

/// The phase at sample `t` from points `step` apart.
fn phase_at(f0: &[f64], phase: &[f64], step: usize, rate: f64, t: usize) -> f64 {
    let step = step.max(1);
    let last = f0.len().saturating_sub(1);
    let k = (t / step).min(last);
    let tau = (t - k * step) as f64;
    let fa = f0.get(k).copied().unwrap_or(50.0);
    let fb = f0.get(k + 1).copied().unwrap_or(fa);
    let p = phase.get(k).copied().unwrap_or(0.0);
    p + std::f64::consts::TAU / rate * (fa * tau + (fb - fa) * tau * tau / (2.0 * step as f64))
}

/// Each point's phase from its frequency, the frequency drawn straight
/// between points.
fn phases(f0: &[f64], step: usize, rate: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(f0.len());
    let mut p = 0.0f64;
    for (k, f) in f0.iter().enumerate() {
        out.push(p);
        let next = f0.get(k + 1).copied().unwrap_or(*f);
        // Wrapped, so a long file does not lose precision in the phase.
        p = (p + std::f64::consts::TAU * step as f64 * (f + next) / 2.0 / rate)
            % std::f64::consts::TAU;
    }
    out
}

/// What [`fit`] is asked to do.
pub struct Ask<'a> {
    /// The rough track, one value every `coarse_hop` frames, block `j`
    /// centred on frame `j coarse_hop`; empty for none.
    pub coarse: &'a [f64],
    pub coarse_hop: usize,
    /// The fundamental where there is no track, one value per block of
    /// [`AUDIO_BLOCK_FRAMES`], the last held past the end.
    pub fallback: &'a [f64],
    /// Refine the track from the harmonics' own turning.
    pub refine: bool,
    /// Harmonics to measure, the fundamental counted.
    pub harmonics: usize,
    /// How wide each is measured, in hertz.
    pub width: f64,
}

/// Fit the hum in `interleaved` (stereo) at `rate`, as `ask` says.
#[must_use]
pub fn fit(interleaved: &[f32], rate: u32, ask: &Ask<'_>) -> HumTable {
    let Ask {
        coarse,
        coarse_hop,
        fallback,
        refine,
        harmonics,
        width,
    } = *ask;
    let rate_f = f64::from(rate.max(1));
    let step = ((rate_f / POINTS_PER_S).round() as usize).max(1);
    let frames = interleaved.len() / AUDIO_CHANNELS;
    let points = frames / step + 2;
    let point_rate = rate_f / step as f64;

    // The rough track at every point, held flat past its ends.
    let mut f0: Vec<f64> = (0..points)
        .map(|k| {
            let block = (k * step) / AUDIO_BLOCK_FRAMES;
            let fallback = fallback
                .get(block)
                .or(fallback.last())
                .copied()
                .unwrap_or(50.0);
            if coarse.is_empty() {
                return fallback;
            }
            let x = (k * step) as f64 / coarse_hop.max(1) as f64;
            let i = x.floor() as usize;
            let f = x - i as f64;
            let a = coarse.get(i).or(coarse.last()).copied().unwrap_or(fallback);
            let b = coarse.get(i + 1).copied().unwrap_or(a);
            a * (1.0 - f) + b * f
        })
        .collect();

    if refine && !coarse.is_empty() {
        let mono: Vec<f32> = interleaved
            .chunks_exact(AUDIO_CHANNELS)
            .map(|f| f.iter().sum::<f32>() / AUDIO_CHANNELS as f32)
            .collect();
        let narrow = butterworth(REFINE_HALF_HZ, point_rate);
        let slow = butterworth(CORRECTION_HZ, point_rate);
        for _ in 0..ROUNDS {
            let phase = phases(&f0, step, rate_f);
            let mut base = demodulate(&mono, 1, &f0, &phase, step, rate_f, REFINE_HARMONICS);
            for h in 0..REFINE_HARMONICS {
                both_ways(&mut base, REFINE_HARMONICS, h, narrow);
            }
            // Each harmonic's turning rate, in hertz of the fundamental,
            // weighted by its strength.
            let mut correction: Vec<Complex<f64>> = (0..points)
                .map(|k| {
                    let (mut num, mut den) = (0.0f64, 0.0f64);
                    for h in 0..REFINE_HARMONICS {
                        let at = |k: usize| {
                            base.get(k * REFINE_HARMONICS + h)
                                .copied()
                                .unwrap_or_default()
                        };
                        let (a, b, span) = match k {
                            0 => (at(0), at(1), 1.0),
                            k if k + 1 >= points => (at(k - 1), at(k), 1.0),
                            k => (at(k - 1), at(k + 1), 2.0),
                        };
                        let turn = (b * a.conj()).arg() / span * point_rate
                            / std::f64::consts::TAU
                            / (h + 1) as f64;
                        let w = at(k).norm_sqr();
                        if turn.is_finite() {
                            num += w * turn;
                            den += w;
                        }
                    }
                    Complex::new(if den > 0.0 { num / den } else { 0.0 }, 0.0)
                })
                .collect();
            both_ways(&mut correction, 1, 0, slow);
            for (f, c) in f0.iter_mut().zip(&correction) {
                // Never further than a hertz from where the transforms put it.
                *f += c.re.clamp(-1.0, 1.0);
            }
        }
    }

    let phase = phases(&f0, step, rate_f);
    let harmonics = harmonics.max(1);
    let lanes = AUDIO_CHANNELS * harmonics;
    let mut base = demodulate(
        interleaved,
        AUDIO_CHANNELS,
        &f0,
        &phase,
        step,
        rate_f,
        harmonics,
    );
    // Critically damped rather than Butterworth: run both ways its kernel
    // never goes below nought, so the weights' sum below can never pass
    // through nought, and the estimate is always a weighted mean of what was
    // measured. Its corner sits where, run both ways, it passes half at
    // Width / 2 from the harmonic, so the notch is as wide as Width says.
    let band = critical(
        width / 2.0 / (std::f64::consts::SQRT_2 - 1.0).sqrt(),
        point_rate,
    );
    // Hum holds still and a voice does not, so the measurement leans on the
    // quiet moments: each point counts in inverse proportion to how loud the
    // sound is there. Through a word the estimate is carried over from the
    // pauses either side rather than taken from the word itself, which is
    // what keeps the voice's own energy near a harmonic out of it.
    let silence = silence(interleaved, step);
    let (scale, mut weight) = trust(interleaved, step, points, &silence);
    for (k, w) in scale.iter().enumerate() {
        for slot in base.get_mut(k * lanes..(k + 1) * lanes).unwrap_or(&mut []) {
            *slot *= w;
        }
    }
    for lane in 0..lanes {
        both_ways(&mut base, lanes, lane, band);
    }
    both_ways(&mut weight, 1, 0, band);
    for (k, w) in weight.iter().enumerate() {
        for slot in base.get_mut(k * lanes..(k + 1) * lanes).unwrap_or(&mut []) {
            *slot /= w.re.max(1e-30);
        }
    }
    HumTable {
        step,
        f0,
        phase,
        harmonics,
        // Twice the level: a real sine is half of it at plus and half at
        // minus its frequency.
        amps: base
            .iter()
            .map(|a| Complex::new((a.re * 2.0) as f32, (a.im * 2.0) as f32))
            .collect(),
        silence,
        rate: rate_f,
    }
}

/// The runs of at least `least` frames where every sample is digital silence.
fn silence(interleaved: &[f32], least: usize) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    let frames = interleaved.len() / AUDIO_CHANNELS;
    for (t, frame) in interleaved.chunks_exact(AUDIO_CHANNELS).enumerate() {
        let quiet = frame.iter().all(|x| x.abs() <= SILENT);
        match (quiet, start) {
            (true, None) => start = Some(t),
            (false, Some(s)) => {
                if t - s >= least {
                    runs.push((s, t));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        if frames - s >= least {
            runs.push((s, frames));
        }
    }
    runs
}

/// How far each point is trusted: one over the sound's mean power around it
/// plus a floor, the floor being the tenth percentile of that power over the
/// span, roughly the hum and the room on their own.
///
/// Digital silence holds no hum, so it is left out: a point's power is taken
/// over its samples with sound, the percentiles over the points with sound,
/// and a point counts in proportion to its share of sound, nothing for one
/// wholly in `silence`. Returned as the scale for each point's measurement
/// and, as a one-lane table, the weight that scaled sum is divided by: a
/// point half in silence measures half its hum, so it is scaled by its trust
/// and weighed by half of it.
fn trust(
    interleaved: &[f32],
    step: usize,
    points: usize,
    silence: &[(usize, usize)],
) -> (Vec<f64>, Vec<Complex<f64>>) {
    let step = step.max(1);
    let mut power = vec![0.0f64; points];
    let mut heard = vec![0.0f64; points];
    let mut all = vec![0.0f64; points];
    let mut run = 0usize;
    for (t, frame) in interleaved.chunks_exact(AUDIO_CHANNELS).enumerate() {
        while silence.get(run).is_some_and(|&(_, end)| end <= t) {
            run += 1;
        }
        let quiet = silence.get(run).is_some_and(|&(start, _)| start <= t);
        let x2 = frame
            .iter()
            .map(|x| f64::from(*x) * f64::from(*x))
            .sum::<f64>()
            / AUDIO_CHANNELS as f64;
        let k = t / step;
        let f = (t - k * step) as f64 / step as f64;
        for (point, w) in [(k, 1.0 - f), (k + 1, f)] {
            if let (Some(p), Some(h), Some(a)) = (
                power.get_mut(point),
                heard.get_mut(point),
                all.get_mut(point),
            ) {
                *a += w;
                if !quiet {
                    *p += x2 * w;
                    *h += w;
                }
            }
        }
    }
    for (p, h) in power.iter_mut().zip(&heard) {
        if *h > 0.0 {
            *p /= *h;
        }
    }
    let mut sorted: Vec<f64> = power
        .iter()
        .zip(&heard)
        .filter(|(_, h)| **h > 0.0)
        .map(|(p, _)| *p)
        .collect();
    sorted.sort_by(f64::total_cmp);
    let at = |share: usize| {
        sorted
            .get(sorted.len() * share / 10)
            .copied()
            .unwrap_or(0.0)
    };
    // Never more than 30 dB between the most and least trusted point, so
    // a quiet moment does not stand for the whole span on its own.
    let floor = at(1).max(at(9) * 1e-3).max(1e-20);
    let scale: Vec<f64> = power
        .iter()
        .zip(&heard)
        .map(|(p, h)| if *h > 0.0 { 1.0 / (p + floor) } else { 0.0 })
        .collect();
    let weight = scale
        .iter()
        .zip(heard.iter().zip(&all))
        .map(|(w, (h, a))| Complex::new(if *a > 0.0 { w * h / a } else { 0.0 }, 0.0))
        .collect();
    (scale, weight)
}

/// Every channel of `interleaved` turned down by each of the first
/// `harmonics` harmonics of the track, and summed onto the points under a
/// triangle two steps wide: `[k][c][h]`, `channels * harmonics` lanes a point.
/// The triangle's response has a deep null at every multiple of the point
/// rate, so neighbouring harmonics a whole number of 100 Hz away fold onto
/// nought hertz only at -90 dB.
fn demodulate(
    interleaved: &[f32],
    channels: usize,
    f0: &[f64],
    phase: &[f64],
    step: usize,
    rate: f64,
    harmonics: usize,
) -> Vec<Complex<f64>> {
    use rayon::prelude::*;
    let channels = channels.max(1);
    let points = f0.len();
    let lanes = channels * harmonics;
    let frames = interleaved.len() / channels;
    let mut out = vec![Complex::new(0.0f64, 0.0); points * lanes];
    out.par_chunks_mut(CHUNK * lanes.max(1))
        .enumerate()
        .for_each(|(chunk, slice)| {
            let _flush = Denormals::on();
            let k0 = chunk * CHUNK;
            let k1 = k0 + slice.len() / lanes.max(1);
            let mut weights = vec![0.0f64; k1 - k0];
            let mut turn = vec![Complex::new(0.0f64, 0.0); harmonics];
            // The samples that reach these points: from a step before the
            // first to the step after the last.
            let first = k0.saturating_sub(1) * step;
            let last = ((k1) * step).min(frames);
            for t in first..last {
                let k = t / step;
                let f = (t - k * step) as f64 / step as f64;
                let (s, c) = phase_at(f0, phase, step, rate, t).sin_cos();
                let one = Complex::new(c, -s);
                let mut r = one;
                for slot in turn.iter_mut() {
                    *slot = r;
                    r *= one;
                }
                for (point, w) in [(k, 1.0 - f), (k + 1, f)] {
                    if point < k0 || point >= k1 {
                        continue;
                    }
                    let local = point - k0;
                    if let Some(sum) = weights.get_mut(local) {
                        *sum += w;
                    }
                    for ch in 0..channels {
                        let x =
                            f64::from(interleaved.get(t * channels + ch).copied().unwrap_or(0.0))
                                * w;
                        for (h, z) in turn.iter().enumerate() {
                            if let Some(slot) = slice.get_mut(local * lanes + ch * harmonics + h) {
                                *slot += z * x;
                            }
                        }
                    }
                }
            }
            for (local, w) in weights.iter().enumerate() {
                if *w > 0.0 {
                    for slot in slice
                        .get_mut(local * lanes..(local + 1) * lanes)
                        .unwrap_or(&mut [])
                    {
                        *slot /= *w;
                    }
                }
            }
        });
    out
}

/// A second-order Butterworth low pass at `hz` for a table at `rate`
/// points a second. The audio filters' own low pass stops at 1 Hz, and the
/// bands here are a fraction of that.
fn butterworth(hz: f64, rate: f64) -> Coeffs {
    let w0 = std::f64::consts::TAU * hz.clamp(1e-3, rate * 0.45) / rate.max(1e-9);
    let (sin, cos) = w0.sin_cos();
    let alpha = sin * std::f64::consts::FRAC_1_SQRT_2;
    let a0 = 1.0 + alpha;
    let b1 = (1.0 - cos) / a0;
    Coeffs {
        b0: b1 * 0.5,
        b1,
        b2: b1 * 0.5,
        a1: -2.0 * cos / a0,
        a2: (1.0 - alpha) / a0,
    }
}

/// A critically damped second-order low pass at `hz` (Q one half), for a
/// table at `rate` points a second. Its two poles are equal and real, so
/// every tap of its kernel is positive, forwards or back.
fn critical(hz: f64, rate: f64) -> Coeffs {
    let w0 = std::f64::consts::TAU * hz.clamp(1e-3, rate * 0.45) / rate.max(1e-9);
    let (sin, cos) = w0.sin_cos();
    let a0 = 1.0 + sin;
    let b1 = (1.0 - cos) / a0;
    Coeffs {
        b0: b1 * 0.5,
        b1,
        b2: b1 * 0.5,
        a1: -2.0 * cos / a0,
        a2: (1.0 - sin) / a0,
    }
}

/// One lane of an interleaved table low-passed forwards and then backwards,
/// so it does not lag. Each pass starts settled on the mean of the first
/// half second it meets, so the ends do not ring.
fn both_ways(table: &mut [Complex<f64>], lanes: usize, lane: usize, c: Coeffs) {
    let lanes = lanes.max(1);
    let len = table.len() / lanes;
    if len == 0 {
        return;
    }
    let pass = |order: &mut dyn Iterator<Item = usize>, table: &mut [Complex<f64>]| {
        let order: Vec<usize> = order.collect();
        let start: Complex<f64> = order
            .iter()
            .take(SETTLE_POINTS)
            .map(|k| table.get(k * lanes + lane).copied().unwrap_or_default())
            .sum::<Complex<f64>>()
            / order.len().clamp(1, SETTLE_POINTS) as f64;
        // Transposed direct form II, settled on a constant `start` (unity
        // gain at nought hertz).
        let mut s2 = start * (c.b2 - c.a2);
        let mut s1 = start * (c.b1 - c.a1) + s2;
        for k in order {
            if let Some(slot) = table.get_mut(k * lanes + lane) {
                let x = *slot;
                let y = x * c.b0 + s1;
                s1 = x * c.b1 - y * c.a1 + s2;
                s2 = x * c.b2 - y * c.a2;
                *slot = y;
            }
        }
    };
    pass(&mut (0..len), table);
    pass(&mut (0..len).rev(), table);
}

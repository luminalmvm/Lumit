//! Hum removal: take mains hum and its harmonics out, following the mains as
//! it drifts.
//!
//! # In plain terms
//!
//! Mains hum is a few sine waves at whole multiples of 50 or 60 Hz, and the
//! mains frequency wanders by a fraction of a hertz as the grid's load
//! changes. Find frequency listens to the whole input first: a 1.4 s
//! transform every 0.7 s, each harmonic's peak placed between bins, the 50
//! and 60 Hz families compared by how far their harmonics stand out, and a
//! frequency track kept for the stronger one, smoothed and drawn straight
//! between blocks ([`HumFinder`]).
//!
//! The track is then refined from the harmonics themselves, and each
//! harmonic on each channel is measured as a slowly moving level and phase
//! Width hertz wide, over the whole input at once ([`super::hum_fit`]). A
//! block takes away what was measured, Reduction of it, less Slope dB for
//! each harmonic up, so the top of a buzz can be left a little in. Measured
//! forwards and back, the estimate neither lags the drift nor needs a moment
//! to settle, so the hum is gone from the first sample to the last, and a
//! hertz either side of each harmonic is all the voice gives up.
//!
//! Digital silence is left alone: it holds no hum to measure, and nothing
//! is taken from it. Hum only plays the estimate that was taken away.
//! Nothing is delayed and nothing rings on.

use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;
use realfft::num_complex::Complex;

use super::hum_fit::{fit, Ask, HumTable};
use super::{first, on, row};
use crate::fx::audio_chain::{AudioProcessor, Denormals, AUDIO_CHANNELS};
use crate::fx::effects::audio::dsp::stft::{frame_count, hann, mono_frame_at, size_at, Fft};
use crate::fx::{
    params::ParamId, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};

const FREQUENCY_HZ: f64 = 50.0;
const HARMONICS: i64 = 12;
const WIDTH_HZ: f64 = 1.0;
const REDUCTION_DB: f64 = 40.0;
const SLOPE_DB: f64 = 0.0;

/// The most harmonics the row offers.
const MAX_HARMONICS: usize = 24;

/// The analysis transform at 48 kHz: 1.37 s, 0.73 Hz a bin.
const FIND_AT_48K: usize = 65_536;

/// Harmonics the finder weighs when it compares the two families.
const FIND_HARMONICS: usize = 8;

/// How far either side of its nominal place a harmonic is looked for, as a
/// fraction of it.
const SEARCH: f64 = 0.03;

/// How far a peak must stand over the bins around it to count as hum, in dB.
const PROMINENT_DB: f32 = 8.0;

/// Frequency is only read while the effect is not finding the hum itself.
pub const AUDIO_DEHUM_ENABLED_WHEN: &[crate::fx::schema::EnabledWhen] =
    &[crate::fx::schema::EnabledWhen {
        param: "frequency",
        on: "auto",
        cond: crate::fx::schema::EnabledCond::BoolIs(false),
    }];

/// Hum removal's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_dehum",
    label = "Hum removal",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
    enabled_when = AUDIO_DEHUM_ENABLED_WHEN,
)]
pub struct AudioDehum {
    /// Find the mains frequency and its drift from the sound itself.
    #[toggle(default = true, label = "Find frequency")]
    pub auto: bool,
    /// The fundamental when Find frequency is off, in hertz.
    #[bounded(min = 40.0, max = 70.0, default = FREQUENCY_HZ, unit = Raw)]
    pub frequency: f32,
    /// How many harmonics are taken out, the fundamental counted.
    #[counter(min = 1, max = 24, hard_min = 1, hard_max = 24, default = HARMONICS, unit = Raw)]
    pub harmonics: i32,
    /// How wide each notch is, in hertz. Read once, at the start of the
    /// span, since the whole span is measured with one filter.
    #[bounded(min = 0.3, max = 5.0, default = WIDTH_HZ, unit = Raw)]
    pub width: f32,
    /// How deep each notch goes, in dB.
    #[bounded(min = 0.0, max = 60.0, default = REDUCTION_DB, unit = Raw)]
    pub reduction: f32,
    /// How much shallower each harmonic up is than the one below, in dB.
    #[bounded(min = 0.0, max = 6.0, default = SLOPE_DB, unit = Raw)]
    pub slope: f32,
    /// Play only the hum that was taken away.
    #[toggle(default = false, label = "Hum only")]
    pub hum_only: bool,
}

/// Hum removal's behaviour.
pub struct AudioDehumDef;

impl EffectDef for AudioDehumDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDehum as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn open_audio(
        &self,
        _state: Option<Vec<u8>>,
        _values: &[(ParamId, f64)],
        rate: u32,
        _offline: bool,
    ) -> Option<Arc<dyn AudioProcessor>> {
        Some(Arc::new(Dehum {
            state: Mutex::new(State { rate, table: None }),
        }))
    }
}

/// What one family of harmonics looks like in one analysis block.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Family {
    /// The fundamental the harmonics agree on, or `None` when none stood out.
    pub f0: Option<f64>,
    /// How far the standing-out harmonics stand out, summed, in dB.
    pub strength: f64,
}

/// The long transform the frequency is found with, planned once.
pub(super) struct HumFinder {
    pub n: usize,
    pub window: Vec<f32>,
    fft: Fft,
    spectrum: Vec<Complex<f32>>,
    db: Vec<f32>,
    bin_hz: f64,
    scratch: Vec<f64>,
}

impl HumFinder {
    pub(super) fn new(rate: u32) -> Self {
        let n = size_at(rate, FIND_AT_48K);
        let fft = Fft::new(n);
        Self {
            window: hann(n),
            spectrum: fft.spectrum(),
            db: vec![0.0; n / 2 + 1],
            bin_hz: f64::from(rate.max(1)) / n as f64,
            scratch: Vec::with_capacity(MAX_HARMONICS),
            fft,
            n,
        }
    }

    /// Take a windowed frame's spectrum as the one the families are read
    /// from. `frame` is used as scratch.
    pub(super) fn analyse(&mut self, frame: &mut [f32]) {
        self.fft.forward(frame, &mut self.spectrum);
        for (d, x) in self.db.iter_mut().zip(&self.spectrum) {
            *d = 10.0 * x.norm_sqr().max(1e-30).log10();
        }
    }

    /// Where harmonic `h` of `f0` peaks within `reach` hertz, placed between
    /// bins, and how far it stands over the bins around it.
    fn peak(&self, hz: f64, reach: f64) -> Option<(f64, f32)> {
        let bins = self.db.len();
        let lo = ((hz - reach) / self.bin_hz).floor().max(1.0) as usize;
        let hi = (((hz + reach) / self.bin_hz).ceil() as usize).min(bins.saturating_sub(2));
        let (k, &top) = self
            .db
            .get(lo..=hi)?
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, d)| (i + lo, d))?;
        let a = f64::from(*self.db.get(k.checked_sub(1)?)?);
        let b = f64::from(top);
        let c = f64::from(*self.db.get(k + 1)?);
        if b < a || b < c {
            return None;
        }
        let denom = a - 2.0 * b + c;
        let p = if denom.abs() > 1e-12 {
            0.5 * (a - c) / denom
        } else {
            0.0
        };
        // The background: the bins a little way out on either side, clear of
        // the window's main lobe.
        let mut sum = 0.0f32;
        let mut count = 0.0f32;
        for d in 6..20usize {
            for at in [k.checked_sub(d), Some(k + d)].into_iter().flatten() {
                if let Some(v) = self.db.get(at) {
                    sum += v;
                    count += 1.0;
                }
            }
        }
        let background = if count > 0.0 { sum / count } else { top };
        Some((
            (k as f64 + p.clamp(-0.5, 0.5)) * self.bin_hz,
            top - background,
        ))
    }

    /// The family of `count` harmonics on `base` in the analysed spectrum.
    pub(super) fn family(&mut self, base: f64, count: usize) -> Family {
        self.scratch.clear();
        let mut strength = 0.0;
        for h in 1..=count {
            let nominal = base * h as f64;
            if let Some((hz, stands)) =
                self.peak(nominal, (nominal * SEARCH).max(2.0 * self.bin_hz))
            {
                if stands > PROMINENT_DB {
                    self.scratch.push(hz / h as f64);
                    strength += f64::from(stands);
                }
            }
        }
        let f0 = median(&mut self.scratch);
        Family { f0, strength }
    }
}

/// How close to the family's median a harmonic's own estimate of the
/// fundamental must lie to count as agreeing with it, in hertz, and how many
/// must agree before one window's reading is called a hum. Measured on the
/// fixtures: a real hum's harmonics agree to within a few hundredths of a
/// hertz, while peaks in speech or a room land a few tenths apart.
const AGREE_HZ: f64 = 0.07;
const AGREE_AT_LEAST: usize = 3;

/// How far from 50 or 60 Hz one window's reading may lie and still be mains.
/// The grid holds within a fraction of a hertz; a voice pitched near 100 Hz
/// lines up with every other harmonic of a family and names something like
/// 51.5, which this turns away.
const NEAR_MAINS_HZ: f64 = 1.0;

impl HumFinder {
    /// The fundamental the harmonics on `base` agree on in the analysed
    /// spectrum, and how many of them agree.
    ///
    /// For a reading taken from one window, where there is no second block to
    /// check it against. A real hum's harmonics all name the same
    /// fundamental; peaks in speech or noise each stand out, and a few of
    /// them can land close together by chance but never most of them. So it
    /// is a hum only when at least [`AGREE_AT_LEAST`] lie within [`AGREE_HZ`]
    /// of the median, they are at least half of those that stand out, and
    /// what they agree on is within [`NEAR_MAINS_HZ`] of `base`.
    pub(super) fn agreed(&mut self, base: f64, count: usize) -> Option<(f64, u32)> {
        let centre = self.family(base, count).f0?;
        let mut near: Vec<f64> = self
            .scratch
            .iter()
            .copied()
            .filter(|f| (f - centre).abs() <= AGREE_HZ)
            .collect();
        if near.len() < AGREE_AT_LEAST || near.len() * 2 < self.scratch.len() {
            return None;
        }
        let agreeing = near.len() as u32;
        median(&mut near)
            .filter(|f0| (f0 - base).abs() <= NEAR_MAINS_HZ)
            .map(|f0| (f0, agreeing))
    }
}

/// The median of `values`, reordering them, or `None` when empty.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mid = values.len() / 2;
    let (_, m, _) = values.select_nth_unstable_by(mid, f64::total_cmp);
    Some(*m)
}

/// The mains frequency through an interleaved span, one value per analysis
/// block of `hop` frames, block `j` centred on frame `j hop`. Empty when no
/// hum stands out anywhere.
pub(super) fn find_track(interleaved: &[f32], rate: u32) -> (Vec<f64>, usize) {
    use rayon::prelude::*;
    let n = size_at(rate, FIND_AT_48K);
    let hop = n / 2;
    let len = interleaved.len() / AUDIO_CHANNELS;
    // Blocks are independent, so they share out over the pool; `collect`
    // keeps them in order.
    let blocks: Vec<[Family; 2]> = (0..frame_count(len, n, hop))
        .into_par_iter()
        .map_init(
            || (HumFinder::new(rate), vec![0.0f32; n]),
            |(finder, frame), j| {
                let _flush = Denormals::on();
                let window = std::mem::take(&mut finder.window);
                mono_frame_at(interleaved, AUDIO_CHANNELS, j, hop, &window, frame);
                finder.window = window;
                finder.analyse(frame);
                [
                    finder.family(50.0, FIND_HARMONICS),
                    finder.family(60.0, FIND_HARMONICS),
                ]
            },
        )
        .collect();
    let strength = |i: usize| {
        blocks
            .iter()
            .map(|b| b.get(i).map_or(0.0, |f| f.strength))
            .sum::<f64>()
    };
    let (s50, s60) = (strength(0), strength(1));
    if s50 <= 0.0 && s60 <= 0.0 {
        return (Vec::new(), hop);
    }
    let pick = usize::from(s60 > s50);
    let found: Vec<Option<f64>> = blocks
        .iter()
        .map(|b| b.get(pick).and_then(|f| f.f0))
        .collect();
    let mut valid: Vec<f64> = found.iter().flatten().copied().collect();
    let Some(global) = median(&mut valid) else {
        return (Vec::new(), hop);
    };
    let raw: Vec<f64> = found.iter().map(|f| f.unwrap_or(global)).collect();
    // A median of three, then a one-pole run forwards and back, so one block
    // fooled by a sung note does not swing the notches and the track does not
    // lag the drift it follows.
    let mut track: Vec<f64> = (0..raw.len())
        .map(|j| {
            let at = |i: usize| raw.get(i).copied().unwrap_or(global);
            let mut three = [
                at(j.saturating_sub(1)),
                at(j),
                at((j + 1).min(raw.len() - 1)),
            ];
            median(&mut three).unwrap_or(global)
        })
        .collect();
    let mut held = track.first().copied().unwrap_or(global);
    for f in track.iter_mut() {
        held = 0.5 * held + 0.5 * *f;
        *f = held;
    }
    let mut held = track.last().copied().unwrap_or(global);
    for f in track.iter_mut().rev() {
        held = 0.5 * held + 0.5 * *f;
        *f = held;
    }
    (track, hop)
}

/// One open Hum removal.
struct Dehum {
    state: Mutex<State>,
}

struct State {
    rate: u32,
    /// What the look measured, or `None` before it has run, when the sound
    /// goes through untouched.
    table: Option<HumTable>,
}

/// The block's settings, read from its values.
struct Rows {
    harmonics: usize,
    depth: [f64; MAX_HARMONICS],
    hum_only: bool,
}

fn rows(values: &[(ParamId, f64)]) -> Rows {
    let reduction = row(values, AudioDehum::REDUCTION, REDUCTION_DB).clamp(0.0, 60.0);
    let slope = row(values, AudioDehum::SLOPE, SLOPE_DB).clamp(0.0, 6.0);
    let mut depth = [0.0; MAX_HARMONICS];
    for (h, d) in depth.iter_mut().enumerate() {
        let db = (reduction - slope * h as f64).max(0.0);
        *d = 1.0 - 10f64.powf(-db / 20.0);
    }
    Rows {
        harmonics: harmonics(values),
        depth,
        hum_only: on(values, AudioDehum::HUM_ONLY, false),
    }
}

/// The Harmonics row as a count.
fn harmonics(values: &[(ParamId, f64)]) -> usize {
    row(values, AudioDehum::HARMONICS, HARMONICS as f64)
        .round()
        .clamp(1.0, MAX_HARMONICS as f64) as usize
}

impl AudioProcessor for Dehum {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        steady: i64,
    ) -> bool {
        let held = self.state.lock();
        let Some(table) = &held.table else {
            output.copy_from_slice(input);
            return true;
        };
        let r = rows(values);
        let count = r.harmonics.min(table.harmonics);
        let first = usize::try_from(steady).unwrap_or(0);
        let mut turn = [Complex::new(0.0f64, 0.0); MAX_HARMONICS];
        let silence = &table.silence;
        let mut run = silence.partition_point(|&(_, end)| end <= first);
        for (i, (came, goes)) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
            .enumerate()
        {
            let t = first + i;
            while silence.get(run).is_some_and(|&(_, end)| end <= t) {
                run += 1;
            }
            // Digital silence has no hum in it, and taking the estimate
            // carried over from either side away would put the hum there.
            if silence.get(run).is_some_and(|&(start, _)| start <= t) {
                for (x, y) in came.iter().zip(goes.iter_mut()) {
                    *y = if r.hum_only { 0.0 } else { *x };
                }
                continue;
            }
            let (s, c) = table.phase_at(t).sin_cos();
            let one = Complex::new(c, s);
            let mut z = one;
            for slot in turn.iter_mut().take(count) {
                *slot = z;
                z *= one;
            }
            for (ch, (x, y)) in came.iter().zip(goes.iter_mut()).enumerate() {
                let removed: f64 = turn
                    .iter()
                    .zip(&r.depth)
                    .take(count)
                    .enumerate()
                    .map(|(h, (z, depth))| (table.amp_at(t, ch, h) * z).re * depth)
                    .sum();
                let d = f64::from(*x);
                *y = if r.hum_only { removed } else { d - removed } as f32;
            }
        }
        true
    }

    fn bakes_whole(&self) -> bool {
        true
    }

    fn look(&self, input: &[f32], values: &[Vec<(ParamId, f64)>]) {
        let mut held = self.state.lock();
        let rate = held.rate;
        let rows = first(values);
        let auto = on(rows, AudioDehum::AUTO, true);
        let (coarse, coarse_hop) = if auto {
            find_track(input, rate)
        } else {
            (Vec::new(), 1)
        };
        // Enough harmonics for the most any block asks for.
        let most = values
            .iter()
            .map(|v| harmonics(v))
            .max()
            .unwrap_or(HARMONICS as usize);
        // Each block's Frequency, so a keyframed one moves the notches.
        let frequency: Vec<f64> = values
            .iter()
            .map(|v| row(v, AudioDehum::FREQUENCY, FREQUENCY_HZ).clamp(40.0, 70.0))
            .collect();
        held.table = Some(fit(
            input,
            rate,
            &Ask {
                coarse: &coarse,
                coarse_hop,
                fallback: &frequency,
                refine: auto,
                harmonics: most,
                width: row(rows, AudioDehum::WIDTH, WIDTH_HZ).clamp(0.3, 5.0),
            },
        ));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;

    /// A 50 Hz series of eight harmonics drifting slowly, over a 440 Hz tone.
    fn hummed(rate: u32, seconds: f64, f0: f64) -> (Vec<f32>, Vec<f32>) {
        let frames = (seconds * f64::from(rate)) as usize / 512 * 512;
        let mut phase = 0.0f64;
        let mut hum = Vec::with_capacity(frames * 2);
        let mut tone = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let t = n as f64 / f64::from(rate);
            let f = f0 + 0.1 * (std::f64::consts::TAU * 0.1 * t).sin();
            phase += std::f64::consts::TAU * f / f64::from(rate);
            let h: f64 = (1..=8)
                .map(|k| (k as f64 * phase + k as f64).sin() * 0.05 / k as f64)
                .sum();
            let s = (std::f64::consts::TAU * 440.0 * t).sin() * 0.3;
            hum.extend([h as f32, h as f32 * 0.8]);
            tone.extend([s as f32, s as f32]);
        }
        (hum, tone)
    }

    #[test]
    fn it_is_deterministic_and_the_same_at_any_rate() {
        let (hum, tone) = hummed(48_000, 3.0, 50.0);
        let input: Vec<f32> = hum.iter().zip(&tone).map(|(a, b)| a + b).collect();
        harness::deterministic(&AudioDehumDef, &[], &input);
        harness::same_seconds_at_any_rate(&AudioDehumDef, &[], 50.0, 3.0, 0.02);
    }

    fn level(x: &[f32], hz: f64, rate: f64) -> f64 {
        let step = std::f64::consts::TAU * hz / rate;
        let (mut re, mut im) = (0.0, 0.0);
        let frames = x.len() / 2;
        for n in 0..frames {
            re += f64::from(x[n * 2]) * (step * n as f64).cos();
            im += f64::from(x[n * 2]) * (step * n as f64).sin();
        }
        2.0 * re.hypot(im) / frames as f64
    }

    /// **Plan 3**: a drifting 50 Hz series comes out at least 30 dB down and a
    /// 440 Hz tone beside it is kept within a dB.
    #[test]
    fn it_takes_the_hum_out_and_keeps_the_tone() {
        let rate = 48_000;
        let (hum, tone) = hummed(rate, 8.0, 50.3);
        let input: Vec<f32> = hum.iter().zip(&tone).map(|(a, b)| a + b).collect();
        let values = harness::values(&AudioDehumDef, &[]);
        let p = harness::looked(&AudioDehumDef, &values, &input);
        let out = harness::run(&*p, &input, &values);
        let residue: Vec<f32> = out.iter().zip(&tone).map(|(o, t)| o - t).collect();
        // The harmonics, each measured on its own, so the tone's own small
        // change is not counted as hum.
        let from = 2 * 48_000 * 2;
        let at = |x: &[f32]| -> f64 {
            (1..=8)
                .map(|h| level(&x[from..], 50.3 * h as f64, 48_000.0).powi(2))
                .sum()
        };
        let down = 10.0 * (at(&hum) / at(&residue)).log10();
        assert!(down > 30.0, "the hum came down only {down} dB");
        let kept = 20.0
            * (level(&out[from..], 440.0, 48_000.0) / level(&tone[from..], 440.0, 48_000.0))
                .log10();
        assert!(kept.abs() < 1.0, "the tone moved {kept} dB");
    }

    /// A steady 50 Hz series of eight harmonics at -55 dBFS, `frames` long.
    fn steady_hum(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let phase = std::f64::consts::TAU * 50.0 * n as f64 / 48_000.0;
                let h: f64 = (1..=8)
                    .map(|k| (k as f64 * phase + k as f64).sin() * 0.002 / k as f64)
                    .sum();
                [h as f32, h as f32 * 0.8]
            })
            .collect()
    }

    /// How far the hum's harmonics come down, in dB, in each tenth of a
    /// second from `from` to `to` (frames): `hum` against what is left of it
    /// in `out`, which is `out` less everything in `input` but the hum. A
    /// tenth of a second holds a whole number of cycles of every harmonic, so
    /// each is read on its own under a Hann window.
    fn hum_down(hum: &[f32], input: &[f32], out: &[f32], from: usize, to: usize) -> Vec<f64> {
        let left: Vec<f32> = out
            .iter()
            .zip(input)
            .zip(hum)
            .map(|((o, i), h)| o - i + h)
            .collect();
        let width = 4_800;
        let energy = |x: &[f32], at: usize| -> f64 {
            (1..=12)
                .map(|h| {
                    let step = std::f64::consts::TAU * 50.0 * h as f64 / 48_000.0;
                    let (mut re, mut im) = (0.0, 0.0);
                    for n in 0..width {
                        let w = 0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / width as f64).cos();
                        let t = at + n;
                        let s = f64::from(x[t * 2]) + f64::from(x[t * 2 + 1]);
                        re += s * w * (step * t as f64).cos();
                        im += s * w * (step * t as f64).sin();
                    }
                    re * re + im * im
                })
                .sum()
        };
        (from..to.saturating_sub(width))
            .step_by(width / 2)
            .map(|at| 10.0 * (energy(hum, at) / energy(&left, at).max(1e-30)).log10())
            .collect()
    }

    /// The hum alone for four seconds, then a steady tone over it for six,
    /// 30 and 45 dB louder than the hum: the hum comes down all the way
    /// through, the loud stretch included, and is never made louder.
    #[test]
    fn a_loud_stretch_after_hum_alone_never_adds_hum() {
        let frames = 48_000 * 10;
        let hum = steady_hum(frames);
        let values = harness::values(&AudioDehumDef, &[("auto", 0.0)]);
        for over_db in [30.0, 45.0] {
            // The hum's power is 3.05e-6; the tone's amplitude for its power
            // to stand `over_db` above that.
            let amp = (2.0 * 3.05e-6 * 10f64.powf(over_db / 10.0)).sqrt();
            let input: Vec<f32> = hum
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    let n = i / 2;
                    let tone = if n >= 48_000 * 4 {
                        (std::f64::consts::TAU * 1_234.5 * n as f64 / 48_000.0).sin() * amp
                    } else {
                        0.0
                    };
                    h + tone as f32
                })
                .collect();
            let p = harness::looked(&AudioDehumDef, &values, &input);
            let out = harness::run(&*p, &input, &values);
            let down = hum_down(&hum, &input, &out, 0, frames);
            let least = down.iter().copied().fold(f64::INFINITY, f64::min);
            assert!(
                least > 20.0,
                "{over_db} dB over: the hum came down only {least} dB somewhere"
            );
        }
    }

    /// Digital silence before the hum and in a gap between two stretches,
    /// the second with a tone 20 dB over the hum: the silence stays silent,
    /// and the hum comes down on both sides of the gap, from its first
    /// sample.
    #[test]
    fn digital_silence_is_not_read_as_a_place_without_hum() {
        let at = |s: f64| (s * 48_000.0) as usize;
        let frames = at(8.1);
        let hum = steady_hum(frames);
        // The tone's amplitude for its power to stand 20 dB over the hum's.
        let amp = (2.0 * 3.05e-6 * 100.0f64).sqrt();
        let silent = |n: usize| n < at(0.1) || (at(3.1)..at(5.1)).contains(&n);
        let mut heard = hum.clone();
        let input: Vec<f32> = hum
            .iter()
            .zip(heard.iter_mut())
            .enumerate()
            .map(|(i, (h, kept))| {
                let n = i / 2;
                if silent(n) {
                    *kept = 0.0;
                    return 0.0;
                }
                let tone = if n >= at(5.1) {
                    (std::f64::consts::TAU * 1_234.5 * n as f64 / 48_000.0).sin() * amp
                } else {
                    0.0
                };
                h + tone as f32
            })
            .collect();
        let values = harness::values(&AudioDehumDef, &[("auto", 0.0)]);
        let p = harness::looked(&AudioDehumDef, &values, &input);
        let out = harness::run(&*p, &input, &values);
        for (from, to) in [(at(0.1), at(3.1)), (at(5.1), frames)] {
            let down = hum_down(&heard, &input, &out, from, to);
            let least = down.iter().copied().fold(f64::INFINITY, f64::min);
            assert!(
                least > 20.0,
                "from {from}: the hum came down only {least} dB somewhere"
            );
        }
        for (n, frame) in out.chunks_exact(2).enumerate().take(frames) {
            if silent(n) {
                assert!(
                    frame.iter().all(|s| s.abs() < 1e-6),
                    "frame {n} of silence came out as {frame:?}"
                );
            }
        }
    }
}

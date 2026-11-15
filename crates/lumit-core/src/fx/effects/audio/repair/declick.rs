//! Click removal: find the short spikes a model of the sound cannot explain,
//! and fill them in from the sound around them.
//!
//! # In plain terms
//!
//! Over a short stretch, sound is predictable: each sample is very nearly a
//! fixed mix of the 53 before it. The mix is fitted to each 55 ms block
//! (autocorrelation and Levinson-Durbin), and a click is where the prediction
//! is suddenly far off: the error passes a multiple of its typical size, the
//! fit's own prediction error. Flagged samples a few apart are joined into
//! one click, and anything longer than Maximum length is let be, because a
//! long surprise is a drum or a consonant rather than a click.
//!
//! The flagged samples are then rebuilt as the values that make the whole
//! block most predictable (least-squares interpolation, Janssen, Veldhuis and
//! Vries 1986), which is a small banded system per block. Flagging generously
//! and repairing exactly is what keeps the voice intact: a single sample
//! flagged by mistake in the middle of a vowel is rebuilt almost to the value
//! it had. That is also why a repair is not widened by default, since a long
//! gap is rebuilt less well than a short one. Blocks overlap by three
//! quarters and are added back under a trapezoid, so a click near one block's
//! edge is repaired by the blocks it sits in the middle of (ffmpeg's adeclick
//! works the same way).
//!
//! Type picks what is hunted: clicks, crackle (a second pass with the model
//! refitted to the repaired block, which finds the small clicks the big ones
//! hid) or mouth clicks (found on the sound above 2 kHz, up to 3 ms long).
//! Clicks only plays what was taken out. The effect runs one block less one
//! sample late.

use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;

use super::{on, row};
use crate::fx::audio_chain::{AudioProcessor, Denormals, AUDIO_CHANNELS};
use crate::fx::effects::audio::dsp::biquad::{Biquad, Coeffs};
use crate::fx::effects::audio::dsp::stft::Framer;
use crate::fx::{
    params::ParamId, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};

const SENSITIVITY: f64 = 5.0;
const MAX_LENGTH_MS: f64 = 2.0;
const WIDEN_MS: f64 = 0.0;

/// The Type row's options, in the order the Choice stores them.
pub const TYPE_CLICKS: u32 = 0;
pub const TYPE_CRACKLE: u32 = 1;
pub const TYPE_MOUTH: u32 = 2;

/// The block at 48 kHz: 55 ms.
const BLOCK_AT_48K: f64 = 2_640.0;

/// The prediction's order at 48 kHz, two per cent of the block.
const ORDER_AT_48K: f64 = 53.0;

/// Flagged samples this close at 48 kHz become one click.
const FUSE_AT_48K: f64 = 5.0;

/// The error, in multiples of the fit's own prediction error, that counts as
/// a click at the default sensitivity.
const THRESHOLD: f64 = 2.0;

/// How much the threshold moves for each step of Sensitivity, as a factor
/// per four steps.
const THRESHOLD_STEP: f64 = 1.6;

/// Crackle's second pass asks for this much more, against the repaired
/// block's smaller error.
const SECOND_PASS: f64 = 1.5;

/// A run whose peak passes this multiple of the threshold is a click worth
/// counting on the display; the rest are small corrections.
const COUNTED: f32 = 2.5;

/// Where mouth clicks are listened for.
const MOUTH_HZ: f64 = 2_000.0;

/// The shortest repair and the least widening for mouth clicks, in ms.
const MOUTH_LONGEST_MS: f64 = 3.0;
const MOUTH_WIDEN_MS: f64 = 0.25;

/// How long the mouth filter is given to settle at a block's start.
const MOUTH_SETTLE_MS: f64 = 2.0;

/// Click removal's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_declick",
    label = "Click removal",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioDeclick {
    /// What is hunted.
    #[choice(label = "Type", options = ["Clicks", "Crackle", "Mouth clicks"], default = 0)]
    pub mode: u32,
    /// How readily a surprise counts as a click.
    #[bounded(min = 1.0, max = 10.0, default = SENSITIVITY, unit = Raw)]
    pub sensitivity: f32,
    /// The longest a click may be, in milliseconds.
    #[bounded(min = 0.1, max = 10.0, default = MAX_LENGTH_MS, unit = Raw, label = "Maximum length")]
    pub max_length: f32,
    /// How far each repair reaches past the click, either side, in
    /// milliseconds.
    #[bounded(min = 0.0, max = 5.0, default = WIDEN_MS, unit = Raw)]
    pub widen: f32,
    /// Play only what was taken out.
    #[toggle(default = false, label = "Clicks only")]
    pub clicks_only: bool,
}

/// Click removal's behaviour.
pub struct AudioDeclickDef;

impl EffectDef for AudioDeclickDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDeclick as EffectMetadata>::SCHEMA
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
        let repairers: Vec<(Repairer, Vec<f32>)> = (0..AUDIO_CHANNELS)
            .map(|_| {
                let r = Repairer::new(rate);
                let before = vec![0.0; r.block];
                (r, before)
            })
            .collect();
        let block = repairers.first().map_or(16, |(r, _)| r.block);
        let framer = Framer::new(
            block,
            block / 4,
            AUDIO_CHANNELS,
            vec![1.0; block],
            trapezoid(block, block / 4),
        );
        let latency = framer.latency() as u32;
        Some(Arc::new(Declick {
            state: Mutex::new(State { framer, repairers }),
            latency,
        }))
    }
}

/// The synthesis window: a box of `n - hop` convolved with a box of `hop`,
/// scaled so copies `hop` apart add up to one. Flat across the middle, where
/// a block's repair is surest, and sloping to nought at its edges.
fn trapezoid(n: usize, hop: usize) -> Vec<f32> {
    let long = n.saturating_sub(hop).max(1);
    let hop = hop.max(1);
    (0..n)
        .map(|i| {
            // How many of the long box's samples the short box covers at i.
            let lo = (i + 1).saturating_sub(hop);
            let hi = i.min(long - 1);
            let overlap = if hi >= lo { hi - lo + 1 } else { 0 };
            (overlap as f64 / long as f64) as f32
        })
        .collect()
}

/// What one block is searched with.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hunt {
    /// The error multiple that counts as a click.
    threshold: f64,
    /// The longest click, in samples.
    longest: usize,
    /// How far a repair reaches either side, in samples.
    widen: usize,
    /// Search the sound above 2 kHz.
    mouth: bool,
    /// Look again with the model refitted to the repaired block.
    second: bool,
}

impl Hunt {
    pub(super) fn of(values: &[(ParamId, f64)], rate: f64) -> Self {
        let mode = row(values, AudioDeclick::MODE, f64::from(TYPE_CLICKS)).round();
        let sensitivity = row(values, AudioDeclick::SENSITIVITY, SENSITIVITY).clamp(1.0, 10.0);
        let mut longest_ms = row(values, AudioDeclick::MAX_LENGTH, MAX_LENGTH_MS).clamp(0.1, 10.0);
        let mut widen_ms = row(values, AudioDeclick::WIDEN, WIDEN_MS).clamp(0.0, 5.0);
        // Sensitivity 5 asks for twice the typical error, 1 for about three
        // times and 10 for a little over once.
        let threshold = THRESHOLD * THRESHOLD_STEP.powf((SENSITIVITY - sensitivity) / 4.0);
        let mouth = mode == f64::from(TYPE_MOUTH);
        if mouth {
            longest_ms = longest_ms.max(MOUTH_LONGEST_MS);
            widen_ms = widen_ms.max(MOUTH_WIDEN_MS);
        }
        Self {
            threshold,
            longest: ((longest_ms * 0.001 * rate).round() as usize).max(1),
            widen: (widen_ms * 0.001 * rate).round() as usize,
            mouth,
            second: mode == f64::from(TYPE_CRACKLE),
        }
    }
}

/// One block's worth of scratch, held so a block allocates nothing.
pub(super) struct Repairer {
    pub block: usize,
    order: usize,
    fuse: usize,
    settle: usize,
    rate: f64,
    coeffs: Vec<f64>,
    scratch: Vec<f64>,
    residual: Vec<f32>,
    detect: Vec<f32>,
    kept: Vec<f32>,
    flags: Vec<bool>,
    unknown: Vec<usize>,
    band: Vec<f64>,
    rhs: Vec<f64>,
    autocorr: Vec<f64>,
    mouth: Coeffs,
}

impl Repairer {
    pub(super) fn new(rate: u32) -> Self {
        let rate_f = f64::from(rate.max(1));
        // Any multiple of four: there is no transform here to keep fast, so
        // the block is as near the same length in seconds as it can be.
        let block = ((BLOCK_AT_48K * rate_f / 48_000.0 / 4.0).round() as usize).max(16) * 4;
        let order = ((ORDER_AT_48K * rate_f / 48_000.0).round() as usize).clamp(8, block / 8);
        Self {
            block,
            order,
            fuse: ((FUSE_AT_48K * rate_f / 48_000.0).round() as usize).max(1),
            settle: (MOUTH_SETTLE_MS * 0.001 * rate_f).round() as usize,
            rate: rate_f,
            coeffs: vec![0.0; order + 1],
            scratch: vec![0.0; order + 1],
            residual: vec![0.0; block],
            detect: vec![0.0; block],
            kept: vec![0.0; block],
            flags: vec![false; block],
            unknown: Vec::with_capacity(block),
            band: Vec::with_capacity(block * (order + 1)),
            rhs: Vec::with_capacity(block),
            autocorr: vec![0.0; order + 1],
            mouth: Coeffs::high_pass(MOUTH_HZ, std::f64::consts::FRAC_1_SQRT_2, rate_f),
        }
    }

    /// Repair one block in place. Its first `before` samples come from
    /// before the sound starts: they are silence, but not silence the sound
    /// had, so nothing is flagged whose prediction reaches back into them. A
    /// clip trimmed in the middle of a word keeps its first samples as they
    /// are rather than having them rebuilt towards a silence that was never
    /// there. Every click worth counting is reported to `found` as its first
    /// sample and the sample after its last, before widening; crackle's
    /// second pass may report one again.
    pub(super) fn repair(
        &mut self,
        x: &mut [f32],
        before: usize,
        hunt: &Hunt,
        mut found: impl FnMut(usize, usize),
    ) {
        if x.len() != self.block {
            return;
        }
        self.flags.fill(false);
        // What is searched: the block itself, or its top for mouth clicks.
        let mut first = self.order + before;
        self.detect.copy_from_slice(x);
        if hunt.mouth {
            let mut hp = Biquad::new(self.mouth);
            for s in self.detect.iter_mut() {
                *s = hp.process(*s);
            }
            first = first.max(self.settle);
        }
        let detect = std::mem::take(&mut self.detect);
        let sigma = self.fit(&detect);
        let flagged = sigma.is_some_and(|sigma| {
            self.predict(&detect);
            self.flag(hunt, first, hunt.threshold * sigma, &mut found)
        });
        self.detect = detect;
        if !flagged {
            return;
        }
        // Mouth clicks are found on the top, but the whole sound is rebuilt
        // from a model of the whole sound.
        if hunt.mouth && self.fit(x).is_none() {
            return;
        }
        self.kept.copy_from_slice(x);
        self.interpolate(x);
        if !hunt.second {
            return;
        }
        // Crackle: the model refitted to the repaired block, which the big
        // clicks no longer pull about, and the original searched again
        // against the repaired block's own error. Both passes' samples are
        // then rebuilt together from the original.
        let Some(sigma) = self.fit(x) else {
            return;
        };
        let kept = std::mem::take(&mut self.kept);
        self.predict(&kept);
        self.flag(
            hunt,
            first,
            hunt.threshold * SECOND_PASS * sigma,
            &mut found,
        );
        x.copy_from_slice(&kept);
        self.kept = kept;
        self.interpolate(x);
    }

    /// Fit the predictor to `x`: autocorrelation of the plain block, then
    /// Levinson-Durbin. The prediction error's typical size, or `None` for a
    /// silent block.
    fn fit(&mut self, x: &[f32]) -> Option<f64> {
        let p = self.order;
        let n = x.len().max(1) as f64;
        for (lag, r) in self.autocorr.iter_mut().enumerate() {
            *r = x
                .iter()
                .zip(x.iter().skip(lag))
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum::<f64>()
                / n;
        }
        let r0 = self.autocorr.first_mut()?;
        if *r0 <= 1e-20 {
            return None;
        }
        // A whisper of white noise keeps the recursion well away from an
        // unstable predictor on a pure tone.
        *r0 *= 1.0 + 1e-9;
        let mut err = *r0;
        self.coeffs.fill(0.0);
        if let Some(c0) = self.coeffs.first_mut() {
            *c0 = 1.0;
        }
        for i in 1..=p {
            let acc: f64 = (0..i)
                .map(|j| {
                    self.coeffs.get(j).copied().unwrap_or(0.0)
                        * self.autocorr.get(i - j).copied().unwrap_or(0.0)
                })
                .sum();
            let k = -acc / err.max(1e-30);
            if !k.is_finite() || k.abs() >= 1.0 {
                break;
            }
            self.scratch.copy_from_slice(&self.coeffs);
            for j in 1..=i {
                let back = self.scratch.get(i - j).copied().unwrap_or(0.0);
                if let Some(c) = self.coeffs.get_mut(j) {
                    *c += k * back;
                }
            }
            err *= 1.0 - k * k;
        }
        Some(err.max(1e-30).sqrt())
    }

    /// The predictor's error over `x`, from the order on, into the residual.
    fn predict(&mut self, x: &[f32]) {
        let p = self.order;
        for (t, slot) in self.residual.iter_mut().enumerate().skip(p) {
            // The newest sample first, as the coefficients run.
            let window = x.get(t - p..=t).unwrap_or(&[]);
            let e: f64 = window
                .iter()
                .rev()
                .zip(&self.coeffs)
                .map(|(x, c)| c * f64::from(*x))
                .sum();
            *slot = e as f32;
        }
    }

    /// Flag what the predictor cannot explain, from `first` to the order
    /// short of the end, where the error passes `limit`; added to the flags
    /// already set. `false` when nothing new was flagged.
    fn flag(
        &mut self,
        hunt: &Hunt,
        first: usize,
        limit: f64,
        found: &mut impl FnMut(usize, usize),
    ) -> bool {
        let p = self.order;
        let last = self.block.saturating_sub(p);
        let limit = limit as f32;
        if first >= last || limit <= 0.0 || !limit.is_finite() {
            return false;
        }
        let over = |residual: &[f32], t: usize| residual.get(t).map_or(0.0, |e| e.abs()) > limit;
        let mut any = false;
        let mut t = first;
        while t < last {
            if !over(&self.residual, t) {
                t += 1;
                continue;
            }
            // A run of flagged samples, gaps up to the fuse joined.
            let start = t;
            let mut end = t + 1;
            let mut gap = 0;
            let mut u = t + 1;
            while u < last && gap <= self.fuse {
                if over(&self.residual, u) {
                    end = u + 1;
                    gap = 0;
                } else {
                    gap += 1;
                }
                u += 1;
            }
            t = end;
            if end - start > hunt.longest {
                continue;
            }
            let peak = self
                .residual
                .get(start..end)
                .unwrap_or(&[])
                .iter()
                .fold(0.0f32, |m, e| m.max(e.abs()));
            if peak > limit * COUNTED {
                found(start, end);
            }
            let lo = start.saturating_sub(hunt.widen).max(p);
            let hi = (end + hunt.widen).min(last);
            for f in self.flags.get_mut(lo..hi).unwrap_or(&mut []) {
                *f = true;
            }
            any = true;
        }
        any
    }

    /// Rebuild the flagged samples of `x` as the values that make the block
    /// most predictable: `M x_u = -b`, where `M` holds the predictor's own
    /// autocorrelation at the gaps between unknowns and `b` its reach into the
    /// known samples.
    fn interpolate(&mut self, x: &mut [f32]) {
        let p = self.order;
        // The predictor's autocorrelation, lags 0..=p.
        for (lag, r) in self.scratch.iter_mut().enumerate() {
            *r = self
                .coeffs
                .iter()
                .zip(self.coeffs.iter().skip(lag))
                .map(|(a, b)| a * b)
                .sum();
        }
        let rc = |d: usize| -> f64 { self.scratch.get(d).copied().unwrap_or(0.0) };
        self.unknown.clear();
        self.unknown.extend(
            self.flags
                .iter()
                .enumerate()
                .filter(|(_, f)| **f)
                .map(|(i, _)| i),
        );
        let m = self.unknown.len();
        if m == 0 {
            return;
        }
        let width = p + 1;
        self.band.clear();
        self.band.resize(m * width, 0.0);
        self.rhs.clear();
        // The right-hand side: each unknown's reach into the known samples.
        for &u in &self.unknown {
            let lo = u.saturating_sub(p);
            let hi = (u + p + 1).min(self.block);
            let mut b = 0.0;
            for k in lo..hi {
                if !self.flags.get(k).copied().unwrap_or(true) {
                    b += rc(u.abs_diff(k)) * f64::from(x.get(k).copied().unwrap_or(0.0));
                }
            }
            self.rhs.push(-b);
        }
        // The banded Cholesky factor, row i holding columns i - p ..= i at
        // offsets p ..= 0. Unknowns are distinct and sorted, so two more than
        // p apart in the list are more than p apart in the block, where the
        // predictor's autocorrelation is nought.
        let loading = rc(0) * 1e-9 + 1e-30;
        for i in 0..m {
            let ui = self.unknown.get(i).copied().unwrap_or(0);
            for j in i.saturating_sub(p)..=i {
                let uj = self.unknown.get(j).copied().unwrap_or(0);
                let lag = ui.abs_diff(uj);
                let mut s = if lag <= p { rc(lag) } else { 0.0 };
                if i == j {
                    s += loading;
                }
                // Columns k from the first both rows hold up to j: row i
                // keeps column k at offset i - k, row j at j - k.
                let terms = j - i.saturating_sub(p).max(j.saturating_sub(p));
                let d = i - j;
                let ri = self
                    .band
                    .get(i * width + d + 1..i * width + d + 1 + terms)
                    .unwrap_or(&[]);
                let rj = self
                    .band
                    .get(j * width + 1..j * width + 1 + terms)
                    .unwrap_or(&[]);
                s -= ri.iter().zip(rj).map(|(a, b)| a * b).sum::<f64>();
                let value = if i == j {
                    s.max(1e-30).sqrt()
                } else {
                    s / self.band.get(j * width).copied().unwrap_or(1.0).max(1e-30)
                };
                if let Some(slot) = self.band.get_mut(i * width + (i - j)) {
                    *slot = value;
                }
            }
        }
        // Forwards then back.
        for i in 0..m {
            let mut s = self.rhs.get(i).copied().unwrap_or(0.0);
            for k in i.saturating_sub(p)..i {
                s -= self.band.get(i * width + (i - k)).copied().unwrap_or(0.0)
                    * self.rhs.get(k).copied().unwrap_or(0.0);
            }
            let d = self.band.get(i * width).copied().unwrap_or(1.0).max(1e-30);
            if let Some(slot) = self.rhs.get_mut(i) {
                *slot = s / d;
            }
        }
        for i in (0..m).rev() {
            let mut s = self.rhs.get(i).copied().unwrap_or(0.0);
            for k in i + 1..(i + p + 1).min(m) {
                s -= self.band.get(k * width + (k - i)).copied().unwrap_or(0.0)
                    * self.rhs.get(k).copied().unwrap_or(0.0);
            }
            let d = self.band.get(i * width).copied().unwrap_or(1.0).max(1e-30);
            if let Some(slot) = self.rhs.get_mut(i) {
                *slot = s / d;
            }
        }
        for (&u, v) in self.unknown.iter().zip(&self.rhs) {
            if let Some(slot) = x.get_mut(u) {
                *slot = *v as f32;
            }
        }
    }

    /// Samples per second this repairer was built for.
    pub(super) fn rate(&self) -> f64 {
        self.rate
    }
}

/// One open Click removal.
struct Declick {
    state: Mutex<State>,
    latency: u32,
}

struct State {
    framer: Framer,
    /// Each channel's repairer, and its frame as it came for Clicks only.
    /// The channels are repaired side by side on the rayon pool; each owns
    /// its own scratch, so the answer does not depend on which runs first.
    repairers: Vec<(Repairer, Vec<f32>)>,
}

impl AudioProcessor for Declick {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let mut held = self.state.lock();
        let State { framer, repairers } = &mut *held;
        let Some(rate) = repairers.first().map(|(r, _)| r.rate()) else {
            output.copy_from_slice(input);
            return true;
        };
        let hunt = Hunt::of(values, rate);
        let only = on(values, AudioDeclick::CLICKS_ONLY, false);
        let block = repairers.first().map_or(16, |(r, _)| r.block);
        let hop = framer.hop();
        framer.process(input, output, |j, frames| {
            use rayon::prelude::*;
            // Frame j starts a block less a hop before sample j hop.
            let lead = usize::try_from(j)
                .map_or(0, |j| (block - hop).saturating_sub(j.saturating_mul(hop)));
            frames
                .par_iter_mut()
                .zip(repairers.par_iter_mut())
                .for_each(|(frame, (repairer, before))| {
                    let _flush = Denormals::on();
                    if only {
                        before.copy_from_slice(frame);
                        repairer.repair(frame, lead, &hunt, |_, _| {});
                        for (f, b) in frame.iter_mut().zip(before.iter()) {
                            *f = b - *f;
                        }
                    } else {
                        repairer.repair(frame, lead, &hunt, |_, _| {});
                    }
                });
        });
        true
    }

    fn latency(&self) -> u32 {
        self.latency
    }
}

/// The clicks found in a mono `window` at `rate`, each as its first sample
/// and the sample after its last, in order, those found by more than one
/// block counted once.
#[must_use]
pub fn find_clicks(window: &[f32], values: &[(ParamId, f64)], rate: u32) -> Vec<(usize, usize)> {
    let mut repairer = Repairer::new(rate);
    let hunt = Hunt::of(values, repairer.rate());
    let block = repairer.block;
    let hop = block / 4;
    let mut found: Vec<(usize, usize)> = Vec::new();
    let mut frame = vec![0.0f32; block];
    // Blocks start a block less a hop before the window, as the stream's do,
    // so its first samples sit in the middle of some block.
    let lead = block - hop;
    let mut start = 0usize;
    while start < window.len() + lead {
        frame.fill(0.0);
        for (i, f) in frame.iter_mut().enumerate() {
            if let Some(x) = (start + i).checked_sub(lead).and_then(|t| window.get(t)) {
                *f = *x;
            }
        }
        repairer.repair(&mut frame, lead.saturating_sub(start), &hunt, |a, b| {
            if let (Some(a), Some(b)) =
                ((start + a).checked_sub(lead), (start + b).checked_sub(lead))
            {
                let b = b.min(window.len());
                if b > a {
                    found.push((a, b));
                }
            }
        });
        start += hop;
    }
    found.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(found.len());
    for (a, b) in found {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;

    /// A sine with a click every 100 ms: a few samples of decaying spike.
    fn clicked(rate: u32, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let clean = harness::tone_at(rate, blocks, 440.0);
        let mut dirty = clean.clone();
        let every = rate as usize / 10;
        let mut at = every / 2;
        while (at + 10) * 2 < dirty.len() {
            for i in 0..6 {
                let v = 0.6 * (if i % 2 == 0 { 1.0 } else { -0.7 }) * (0.6f32).powi(i as i32);
                dirty[(at + i) * 2] += v;
                dirty[(at + i) * 2 + 1] -= v;
            }
            at += every;
        }
        (dirty, clean)
    }

    #[test]
    fn it_is_deterministic_and_the_same_at_any_rate() {
        let (dirty, _) = clicked(48_000, 48);
        harness::deterministic(&AudioDeclickDef, &[], &dirty);
        harness::deterministic(&AudioDeclickDef, &[("mode", 1.0)], &dirty);
        // The tone's abrupt start and stop are clicks in their own right, and
        // their repair differs a little with the rate, so the bucket either
        // side of each is given more room.
        harness::same_seconds_at_any_rate(&AudioDeclickDef, &[], 440.0, 1.0, 0.06);
    }

    /// **Plan 3**: the clicks are repaired and the sine under them is kept,
    /// and each one is found.
    #[test]
    fn it_repairs_clicks_and_keeps_the_sine() {
        let (dirty, clean) = clicked(48_000, 200);
        let values = harness::values(&AudioDeclickDef, &[]);
        let p = harness::looked(&AudioDeclickDef, &values, &dirty);
        let lat = p.latency() as usize * 2;
        let out = harness::run(&*p, &dirty, &values);
        let from = 4_096 * 2;
        let err = |x: &[f32]| -> f64 {
            x.iter()
                .zip(&clean[from..])
                .map(|(a, b)| f64::from(a - b).powi(2))
                .sum::<f64>()
        };
        let before = err(&dirty[from..dirty.len() - lat]);
        let after = err(&out[from + lat..]);
        let better = 10.0 * (before / after).log10();
        assert!(better > 15.0, "the clicks came down only {better} dB");

        let mono: Vec<f32> = dirty.chunks_exact(2).map(|f| f[0]).collect();
        // The planted clicks are all found, and nothing else: not the
        // tone's abrupt start either, which may be a clip's in point.
        let found = find_clicks(&mono, &values, 48_000);
        assert_eq!(found.len(), 21, "found {found:?}");

        // A clean sine goes through as it came.
        let p = harness::looked(&AudioDeclickDef, &values, &clean);
        let out = harness::run(&*p, &clean, &values);
        let moved = out[from + lat..]
            .iter()
            .zip(&clean[from..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(moved < 1e-3, "a clean sine moved by {moved}");
    }
}

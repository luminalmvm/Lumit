//! Reverb reduction: turn down the room's late reflections and leave the
//! voice.
//!
//! # In plain terms
//!
//! A room's reverb decays exponentially, so what is left of the sound from
//! 50 ms ago is predictable: its power then, times the decay over 50 ms. That
//! prediction is the late reverb's power now (Lebart 2001, Habets 2009), and
//! each frequency of each frame gives up half as much again as that
//! prediction ([`Room::gains`]). Reduction is the deepest it goes, and
//! Smoothing how fast a gain closes once the sound has gone.
//!
//! The decay is the room's reverb time. Find reverb time measures it from the
//! whole input first: in each octave band the stretches where the sound is
//! dying away, the longest of them fitted with a straight line in dB, the
//! median taken ([`find_reverb_time`]). Off, the Reverb time row is used.
//!
//! Best quality first finds, over the whole input, the linear prediction
//! that takes much of the tail out without any musical noise
//! ([`super::wpe`]); each frame then has its prediction taken away before
//! the spectral stage, which then has less left to guess at. Reverb only
//! plays what was taken away. The effect runs a frame less one sample
//! late.

use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;
use realfft::num_complex::Complex;

use super::profile::Framing;
use super::wpe::{fit, Filters, REACH};
use super::{on, row};
use crate::fx::audio_chain::{AudioProcessor, AUDIO_CHANNELS};
use crate::fx::effects::audio::dsp::stft::{frame_count, mono_frame_at, Fft, Framer};
use crate::fx::{
    params::ParamId, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};

const REDUCTION_DB: f64 = 12.0;
const REVERB_TIME_S: f64 = 0.8;
const SMOOTHING: f64 = 50.0;

/// Where the late reverb starts, in seconds after the sound.
const LATE_S: f64 = 0.05;

/// How much of each frame's power the running power estimate takes.
const POWER_SMOOTHING: f32 = 0.5;

/// The share of the late estimate fed from the input rather than from its
/// own decay: one is Lebart's estimator, which suits a voice farther from the
/// microphone than the room's critical distance.
const KAPPA: f32 = 1.0;

/// How much more than the predicted late reverb a bin gives up.
const OVER: f32 = 1.5;

/// How fast a gain opens, in milliseconds.
const ATTACK_MS: f64 = 5.0;

/// A one-pole coefficient for a time constant of `ms` at frames `hop_s` apart.
fn coeff(ms: f64, hop_s: f64) -> f32 {
    (-hop_s / (ms.max(1e-3) * 0.001)).exp() as f32
}

/// The octave bands the reverb time is measured in.
const BANDS_HZ: [f64; 5] = [250.0, 500.0, 1_000.0, 2_000.0, 4_000.0];

/// Reverb time is only read while the effect is not finding the room's
/// decay itself.
pub const AUDIO_DEREVERB_ENABLED_WHEN: &[crate::fx::schema::EnabledWhen] =
    &[crate::fx::schema::EnabledWhen {
        param: "reverb_time",
        on: "auto",
        cond: crate::fx::schema::EnabledCond::BoolIs(false),
    }];

/// Reverb reduction's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_dereverb",
    label = "Reverb reduction",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
    enabled_when = AUDIO_DEREVERB_ENABLED_WHEN,
)]
pub struct AudioDereverb {
    /// The deepest the reverb is turned down, in dB.
    #[bounded(min = 0.0, max = 30.0, default = REDUCTION_DB, unit = Raw)]
    pub reduction: f32,
    /// Measure the room's reverb time from the sound itself.
    #[toggle(default = true, label = "Find reverb time")]
    pub auto: bool,
    /// The room's reverb time when Find reverb time is off, in seconds.
    #[bounded(min = 0.1, max = 3.0, default = REVERB_TIME_S, unit = Raw, label = "Reverb time")]
    pub reverb_time: f32,
    /// How slowly the gain closes after a sound.
    #[bounded(min = 0.0, max = 100.0, default = SMOOTHING, unit = Percent)]
    pub smoothing: f32,
    /// A slower, cleaner pass over the whole sound first.
    #[toggle(default = false, label = "Best quality")]
    pub best: bool,
    /// Play only what was taken away.
    #[toggle(default = false, label = "Reverb only")]
    pub reverb_only: bool,
}

/// Reverb reduction's behaviour.
pub struct AudioDereverbDef;

impl EffectDef for AudioDereverbDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDereverb as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn open_audio(
        &self,
        _state: Option<Vec<u8>>,
        values: &[(ParamId, f64)],
        rate: u32,
        _offline: bool,
    ) -> Option<Arc<dyn AudioProcessor>> {
        let framing = Framing::at(rate);
        let framer = Framer::sqrt_hann(framing.n, AUDIO_CHANNELS);
        let latency = framer.latency();
        let fft = Fft::new(framing.n);
        let channels = (0..AUDIO_CHANNELS).map(|_| Room::new(&framing)).collect();
        Some(Arc::new(Dereverb {
            latency: latency as u32,
            auto: on(values, AudioDereverb::AUTO, true),
            best: on(values, AudioDereverb::BEST, false),
            state: Mutex::new(State {
                found: None,
                came: fft.spectrum(),
                filters: None,
                past: vec![vec![fft.spectrum(); REACH]; AUDIO_CHANNELS],
                newest: 0,
                spectrum: fft.spectrum(),
                power: vec![0.0; framing.bins],
                late: vec![0.0; framing.bins],
                gains: vec![1.0; framing.bins],
                fft,
                framer,
                channels,
                framing,
            }),
        }))
    }
}

/// One channel's view of the room.
pub(super) struct Room {
    /// The running power per bin.
    power: Vec<f32>,
    /// The reverberant power per bin over the last few frames, as a ring.
    ring: Vec<Vec<f32>>,
    at: usize,
    /// The time-smoothed gain per bin.
    held: Vec<f32>,
    attack: f32,
    release: f32,
    hop_s: f64,
    smoothing: f64,
}

impl Room {
    /// This frame's late reverberant power per bin into `late`, from the
    /// frame's `power` and the decay per hop.
    pub(super) fn late(&mut self, power: &[f32], decay: f32, late: &mut [f32]) {
        let span = self.ring.len().max(1);
        // The reverberant power now, from its own last value and the running
        // power of the last frame (Habets' recursion; with kappa one,
        // Lebart's), into the ring.
        let previous = (self.at + span - 1) % span;
        let newest = self.at;
        for k in 0..power.len() {
            let was = self
                .ring
                .get(previous)
                .and_then(|r| r.get(k))
                .copied()
                .unwrap_or(0.0);
            let running = self.power.get(k).copied().unwrap_or(0.0);
            let now = (1.0 - KAPPA) * decay * was + KAPPA * decay * running;
            if let Some(slot) = self.ring.get_mut(newest).and_then(|r| r.get_mut(k)) {
                *slot = now;
            }
        }
        self.at = (self.at + 1) % span;
        // The oldest in the ring, carried on to now: the late part.
        let reach = decay.powi(span as i32 - 1);
        for (k, l) in late.iter_mut().enumerate() {
            *l = self
                .ring
                .get(self.at)
                .and_then(|r| r.get(k))
                .copied()
                .unwrap_or(0.0)
                * reach;
        }
        for (r, p) in self.power.iter_mut().zip(power) {
            *r = POWER_SMOOTHING * *r + (1.0 - POWER_SMOOTHING) * p;
        }
    }

    /// A fresh room for the framing.
    pub(super) fn new(framing: &Framing) -> Self {
        let late = ((LATE_S / framing.hop_s()).round() as usize).max(1);
        let mut room = Self {
            power: vec![0.0; framing.bins],
            ring: vec![vec![0.0; framing.bins]; late],
            at: 0,
            held: vec![1.0; framing.bins],
            attack: coeff(ATTACK_MS, framing.hop_s()),
            release: 0.0,
            hop_s: framing.hop_s(),
            smoothing: f64::NAN,
        };
        room.set_smoothing(SMOOTHING);
        room
    }

    /// Set the Smoothing row, 0..100: the release runs from 6 to 100 ms on
    /// a log scale, 25 ms in the middle. Cheap when nothing changed.
    pub(super) fn set_smoothing(&mut self, percent: f64) {
        let s = if percent.is_finite() {
            percent.clamp(0.0, 100.0) / 100.0
        } else {
            0.5
        };
        if s != self.smoothing {
            self.smoothing = s;
            self.release = coeff(6.25 * 16f64.powf(s), self.hop_s);
        }
    }

    /// This frame's gains into `gains`: each bin keeps what its power has
    /// over half as much again as the late reverb predicted for it, never
    /// less than `low`, opening in 5 ms and closing at the release.
    ///
    /// Plain power subtraction rather than Noise reduction's estimator,
    /// because late reverb is not steady: the estimator's memory of the last
    /// frame holds a decaying tail open, where subtraction lets it go the
    /// moment the prediction catches up, and with the release on top it
    /// leaves no more twinkle in the pauses than the input had.
    pub(super) fn gains(&mut self, power: &[f32], late: &[f32], low: f32, gains: &mut [f32]) {
        for (((g, held), &p), &l) in gains
            .iter_mut()
            .zip(self.held.iter_mut())
            .zip(power)
            .zip(late)
        {
            let raw = if p > 0.0 {
                (1.0 - OVER * l / p).max(low)
            } else {
                low
            };
            let k = if raw > *held {
                self.attack
            } else {
                self.release
            };
            *held = k * *held + (1.0 - k) * raw;
            *g = held.max(low);
        }
    }
}

/// The decay of a reverb of `t60` seconds over one hop of `hop_s`, as a power
/// ratio.
pub(super) fn decay_per_hop(t60: f64, hop_s: f64) -> f32 {
    (-2.0 * 3.0 * std::f64::consts::LN_10 / t60.max(0.01) * hop_s).exp() as f32
}

/// One open Reverb reduction.
struct Dereverb {
    state: Mutex<State>,
    latency: u32,
    /// Find reverb time and Best quality, as the instance opened: what the
    /// look has to do, and so whether the rack is baked whole.
    auto: bool,
    best: bool,
}

struct State {
    framing: Framing,
    framer: Framer,
    fft: Fft,
    spectrum: Vec<Complex<f32>>,
    power: Vec<f32>,
    late: Vec<f32>,
    gains: Vec<f32>,
    channels: Vec<Room>,
    /// The reverb time the look found.
    found: Option<f64>,
    /// The spectrum as it came, before anything was taken away.
    came: Vec<Complex<f32>>,
    /// Best quality's prediction weights, once the look has found them.
    filters: Option<Filters>,
    /// Each channel's last few spectra as they came, as a ring, and where
    /// the newest is.
    past: Vec<Vec<Vec<Complex<f32>>>>,
    newest: usize,
}

impl AudioProcessor for Dereverb {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let mut held = self.state.lock();
        let st = &mut *held;
        let reduction = row(values, AudioDereverb::REDUCTION, REDUCTION_DB).clamp(0.0, 30.0);
        let t60 = match (self.auto, st.found) {
            (true, Some(found)) => found,
            _ => row(values, AudioDereverb::REVERB_TIME, REVERB_TIME_S),
        }
        .clamp(0.1, 3.0);
        let smoothing = row(values, AudioDereverb::SMOOTHING, SMOOTHING);
        let only = on(values, AudioDereverb::REVERB_ONLY, false);

        let low = 10f64.powf(-reduction / 20.0) as f32;

        // The decay per hop, and over the late reverb's delay.
        let hop_s = st.framing.hop_s();
        let decay = decay_per_hop(t60, hop_s);
        let State {
            framer,
            fft,
            spectrum,
            came,
            power,
            late,
            gains,
            channels,
            filters,
            past,
            newest,
            ..
        } = st;
        framer.process(input, output, |j, frames| {
            *newest = (*newest + 1) % REACH;
            for (c, ((frame, room), ring)) in frames
                .iter_mut()
                .zip(channels.iter_mut())
                .zip(past.iter_mut())
                .enumerate()
            {
                room.set_smoothing(smoothing);
                fft.forward(frame, spectrum);
                came.copy_from_slice(spectrum);
                // Best quality: the late part the weights predict from the
                // frames before, taken away first.
                if let Some(filters) = filters.as_ref() {
                    let at = *newest;
                    filters.predict(j, c, spectrum, |d| {
                        ring.get((at + REACH - d) % REACH)
                            .map_or(&[][..], Vec::as_slice)
                    });
                }
                if let Some(slot) = ring.get_mut(*newest) {
                    slot.copy_from_slice(came);
                }
                for (p, x) in power.iter_mut().zip(spectrum.iter()) {
                    *p = x.norm_sqr();
                }
                room.late(power, decay, late);
                room.gains(power, late, low, gains);
                for ((x, y), &g) in spectrum.iter_mut().zip(came.iter()).zip(gains.iter()) {
                    // Reverb only: what came in less what goes out.
                    *x = if only { y - *x * g } else { *x * g };
                }
                fft.inverse(spectrum, frame);
            }
        });
        true
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn bakes_whole(&self) -> bool {
        self.auto || self.best
    }

    fn look(&self, input: &[f32], _values: &[Vec<(ParamId, f64)>]) {
        let mut held = self.state.lock();
        let rate = held.framing.rate;
        if self.auto {
            held.found = find_reverb_time(input, AUDIO_CHANNELS, rate);
        }
        if self.best {
            held.filters = Some(fit(input, AUDIO_CHANNELS, &held.framing));
        }
    }
}

/// The reverb time of the room in `interleaved`, in seconds, measured from
/// the stretches where the sound dies away, or `None` when there are too few
/// of them to say.
///
/// In each octave band from 250 Hz to 4 kHz the band's level is followed
/// frame by frame. Every stretch where it falls without a break for at least
/// 85 ms, past its first 20 ms where the sound itself stops, and down to
/// 6 dB over the band's quiet level, is fitted with a straight line; the
/// longest third of the stretches give a median, and the bands' medians give
/// the answer.
#[must_use]
pub fn find_reverb_time(interleaved: &[f32], channels: usize, rate: u32) -> Option<f64> {
    let framing = Framing::at(rate);
    let len = interleaved.len() / channels.max(1);
    let frames = frame_count(len, framing.n, framing.hop);
    let mut fft = Fft::new(framing.n);
    let mut frame = vec![0.0f32; framing.n];
    let mut spectrum = fft.spectrum();
    let ranges: Vec<(usize, usize)> = BANDS_HZ
        .iter()
        .map(|c| {
            let lo = (c / std::f64::consts::SQRT_2 / framing.bin_hz).round() as usize;
            let hi = (c * std::f64::consts::SQRT_2 / framing.bin_hz).round() as usize;
            (lo.max(1), hi.min(framing.bins - 1).max(lo + 1))
        })
        .collect();
    let mut levels = vec![Vec::with_capacity(frames); BANDS_HZ.len()];
    for j in 0..frames {
        mono_frame_at(
            interleaved,
            channels,
            j,
            framing.hop,
            &framing.window,
            &mut frame,
        );
        fft.forward(&mut frame, &mut spectrum);
        for ((lo, hi), level) in ranges.iter().zip(levels.iter_mut()) {
            let e: f64 = spectrum
                .get(*lo..*hi)
                .unwrap_or(&[])
                .iter()
                .map(|x| f64::from(x.norm_sqr()))
                .sum();
            level.push(e);
        }
    }
    let hop_s = framing.hop_s();
    let mut medians: Vec<f64> = levels
        .iter()
        .filter_map(|level| band_reverb_time(level, hop_s))
        .collect();
    let mid = medians.len() / 2;
    if medians.is_empty() {
        return None;
    }
    let (_, m, _) = medians.select_nth_unstable_by(mid, f64::total_cmp);
    Some(m.clamp(0.1, 3.0))
}

/// One band's reverb time from its power per frame.
fn band_reverb_time(power: &[f64], hop_s: f64) -> Option<f64> {
    // Three frames averaged, in dB.
    let db: Vec<f64> = (0..power.len())
        .map(|j| {
            let lo = j.saturating_sub(1);
            let hi = (j + 2).min(power.len());
            let mean =
                power.get(lo..hi).unwrap_or(&[]).iter().sum::<f64>() / (hi - lo).max(1) as f64;
            10.0 * mean.max(1e-30).log10()
        })
        .collect();
    let mut sorted: Vec<f64> = db.iter().copied().filter(|d| *d > -250.0).collect();
    if sorted.len() < 10 {
        return None;
    }
    let tenth = sorted.len() / 10;
    let (_, quiet, _) = sorted.select_nth_unstable_by(tenth, f64::total_cmp);
    let stop = *quiet + 6.0;
    let skip = (0.02 / hop_s).round() as usize;
    let least = (0.085 / hop_s).round() as usize;

    let mut decays: Vec<(usize, f64)> = Vec::new();
    let mut j = 0;
    while j + 1 < db.len() {
        if db.get(j + 1) >= db.get(j) {
            j += 1;
            continue;
        }
        let start = j;
        while j + 1 < db.len() && db.get(j + 1) < db.get(j) {
            j += 1;
        }
        // The stretch start..=j falls throughout. Fit the part after the
        // sound's own stop and above the quiet.
        let from = start + skip;
        let mut to = j;
        while to > from && db.get(to).is_some_and(|d| *d < stop) {
            to -= 1;
        }
        if to < from + least.saturating_sub(skip) || to <= from {
            continue;
        }
        let points = db.get(from..=to).unwrap_or(&[]);
        let count = points.len() as f64;
        let mean_t = (count - 1.0) / 2.0;
        let mean_d = points.iter().sum::<f64>() / count;
        let (mut num, mut den) = (0.0, 0.0);
        for (i, d) in points.iter().enumerate() {
            num += (i as f64 - mean_t) * (d - mean_d);
            den += (i as f64 - mean_t).powi(2);
        }
        let slope = num / den.max(1e-12) / hop_s;
        let fall = points.first().copied().unwrap_or(0.0) - points.last().copied().unwrap_or(0.0);
        if slope < 0.0 && fall >= 8.0 {
            decays.push((points.len(), -60.0 / slope));
        }
    }
    if decays.is_empty() {
        return None;
    }
    decays.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)));
    let keep = (decays.len() / 3).max(5).min(decays.len());
    let mut times: Vec<f64> = decays.iter().take(keep).map(|d| d.1).collect();
    let mid = times.len() / 2;
    let (_, m, _) = times.select_nth_unstable_by(mid, f64::total_cmp);
    Some(*m)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;

    /// Bursts of noise, 150 ms on and 850 ms off, through a room with an
    /// exponential noise tail of `t60` seconds, and the dry bursts.
    fn roomy(rate: u32, seconds: f64, t60: f64) -> (Vec<f32>, Vec<f32>) {
        let frames = (seconds * f64::from(rate)) as usize / 512 * 512;
        let mut seed = 21u64;
        let mut noise = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        let period = rate as usize;
        let on = rate as usize * 15 / 100;
        let dry: Vec<f32> = (0..frames)
            .map(|n| if n % period < on { noise() * 0.3 } else { 0.0 })
            .collect();
        let taps = (t60 * f64::from(rate)) as usize;
        let rir: Vec<f32> = (0..taps)
            .map(|n| {
                let t = n as f64 / f64::from(rate);
                if n == 0 {
                    1.0
                } else {
                    noise() * 0.08 * (-6.9 * t / t60).exp() as f32
                }
            })
            .collect();
        // Convolve sparsely: the bursts are the only sound.
        let mut wet = vec![0.0f32; frames];
        for (n, x) in dry.iter().enumerate() {
            if *x == 0.0 {
                continue;
            }
            for (k, h) in rir.iter().enumerate() {
                if let Some(w) = wet.get_mut(n + k) {
                    *w += x * h;
                }
            }
        }
        let st = |v: &[f32]| v.iter().flat_map(|x| [*x, *x]).collect::<Vec<f32>>();
        (st(&wet), st(&dry))
    }

    #[test]
    fn it_is_deterministic_and_the_same_at_any_rate() {
        let (wet, _) = roomy(48_000, 2.0, 0.5);
        harness::deterministic(&AudioDereverbDef, &[], &wet[..512 * 2 * 160]);
        harness::deterministic(&AudioDereverbDef, &[("best", 1.0)], &wet[..512 * 2 * 160]);
        harness::same_seconds_at_any_rate(&AudioDereverbDef, &[], 440.0, 1.0, 0.03);
    }

    /// **Plan 3**: a room's tail after each burst comes out much quieter and
    /// the bursts themselves keep their level.
    #[test]
    fn it_shortens_the_tail_and_keeps_the_bursts() {
        let rate = 48_000;
        let (wet, _) = roomy(rate, 8.0, 0.8);
        let values = harness::values(&AudioDereverbDef, &[]);
        let p = harness::looked(&AudioDereverbDef, &values, &wet);
        let lat = p.latency() as usize;
        let out = harness::run(&*p, &wet, &values);
        let energy = |x: &[f32], from: usize, to: usize, shift: usize| -> f64 {
            (1..8)
                .map(|s| {
                    let base = s * rate as usize;
                    (base + from..base + to)
                        .map(|n| f64::from(x[(n + shift) * 2]).powi(2))
                        .sum::<f64>()
                })
                .sum()
        };
        // 200 to 600 ms after each burst starts: tail only.
        let tail_in = energy(&wet, 9_600, 28_800, 0);
        let tail_out = energy(&out, 9_600, 28_800, lat);
        let shorter = 10.0 * (tail_in / tail_out).log10();
        eprintln!("found t60 {:?}", find_reverb_time(&wet, 2, rate));
        assert!(shorter > 6.0, "the tail came down only {shorter} dB");
        let burst_in = energy(&wet, 1_000, 6_000, 0);
        let burst_out = energy(&out, 1_000, 6_000, lat);
        let kept = 10.0 * (burst_out / burst_in).log10();
        assert!(kept > -3.0, "the bursts lost {kept} dB");
    }
}

//! De-esser: turn down harsh esses and sh sounds as they happen.
//!
//! # In plain terms
//!
//! The sound is split at Frequency into a low band and a high band by a
//! linear-phase low pass (255 taps at 48 kHz), the high band being the input,
//! delayed to match, less the low one, so the two add back to the input
//! exactly. A sibilant is where the high band suddenly carries much of the
//! energy: its level over the whole sound's, in dB, passes Threshold. That
//! is the relative threshold, which does not need setting again when the
//! voice gets louder.
//!
//! The reduction is the excess times two thirds, no more than Range, and it
//! closes in half a millisecond, holds for 30 ms so one s is one movement,
//! and lets go at Speed. The sound runs 3 ms behind the detector, so the gain
//! is already down when the s arrives. It turns down the high band only, or
//! the whole sound with Whole sound on. Sibilance only plays what was taken
//! away. Below -60 dBFS nothing is touched, so a quiet room tone is not
//! mistaken for an ess.

use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;

use super::{on, row};
use crate::fx::audio_chain::{AudioProcessor, AUDIO_CHANNELS};
use crate::fx::{
    params::ParamId, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};

const THRESHOLD_DB: f64 = -20.0;
pub(super) const FREQUENCY_HZ: f64 = 5_000.0;
const RANGE_DB: f64 = 10.0;
const SPEED_MS: f64 = 60.0;

/// Taps in the crossover at 48 kHz.
const TAPS_AT_48K: f64 = 255.0;

/// How far the sound runs behind the detector, in milliseconds.
const LOOKAHEAD_MS: f64 = 3.0;

/// How fast the reduction closes, in milliseconds.
const ATTACK_MS: f64 = 0.5;

/// How long it holds once the s is over, in milliseconds.
const HOLD_MS: f64 = 30.0;

/// How fast the two levels the detector compares follow the sound, in
/// milliseconds.
const LEVEL_MS: f64 = 1.0;

/// Two thirds: the reduction per dB over the threshold, a three to one
/// ratio.
const SLOPE: f64 = 2.0 / 3.0;

/// Below this whole-sound level nothing is reduced, fading in over the next
/// ten dB.
const QUIET_DB: f64 = -60.0;

/// De-esser's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_deess",
    label = "De-esser",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioDeess {
    /// How much of the energy the high band must carry before it is turned
    /// down, in dB under the whole sound.
    #[bounded(min = -40.0, max = 0.0, default = THRESHOLD_DB, unit = Raw)]
    pub threshold: f32,
    /// Where the high band starts, in hertz.
    #[bounded(min = 2_000.0, max = 12_000.0, default = FREQUENCY_HZ, log = true, unit = Raw)]
    pub frequency: f32,
    /// The most an ess is turned down, in dB.
    #[bounded(min = 0.0, max = 24.0, default = RANGE_DB, unit = Raw)]
    pub range: f32,
    /// How fast the reduction lets go, in milliseconds.
    #[bounded(min = 20.0, max = 200.0, default = SPEED_MS, unit = Raw)]
    pub speed: f32,
    /// Turn the whole sound down on an ess, not just its top.
    #[toggle(default = false, label = "Whole sound")]
    pub wideband: bool,
    /// Play only what was taken away.
    #[toggle(default = false, label = "Sibilance only")]
    pub ess_only: bool,
}

/// De-esser's behaviour.
pub struct AudioDeessDef;

impl EffectDef for AudioDeessDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDeess as EffectMetadata>::SCHEMA
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
        let state = State::new(
            f64::from(rate.max(1)),
            row(values, AudioDeess::FREQUENCY, FREQUENCY_HZ),
        );
        let latency = (state.taps / 2 + state.lookahead) as u32;
        Some(Arc::new(Deess {
            state: Mutex::new(state),
            latency,
        }))
    }
}

/// A linear-phase low pass at `hz`: a Blackman-windowed sinc of `taps` taps
/// (odd), its DC gain one.
pub(super) fn low_pass(hz: f64, rate: f64, taps: usize) -> Vec<f32> {
    let centre = (taps / 2) as f64;
    let fc = (hz / rate).clamp(0.0, 0.5);
    let span = (taps.max(2) - 1) as f64;
    let raw: Vec<f64> = (0..taps)
        .map(|i| {
            let x = i as f64 - centre;
            let sinc = if x == 0.0 {
                2.0 * fc
            } else {
                (std::f64::consts::TAU * fc * x).sin() / (std::f64::consts::PI * x)
            };
            let a = std::f64::consts::TAU * i as f64 / span;
            sinc * (0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos())
        })
        .collect();
    let sum: f64 = raw.iter().sum();
    raw.iter().map(|h| (h / sum.max(1e-12)) as f32).collect()
}

/// A dot product in eight lanes, which the compiler can vectorise and which
/// always adds in the same order.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0.0f32; 8];
    let chunks = a.chunks_exact(8).zip(b.chunks_exact(8));
    for (x, y) in chunks {
        for ((l, p), q) in lanes.iter_mut().zip(x).zip(y) {
            *l += p * q;
        }
    }
    let rest: f32 = a
        .chunks_exact(8)
        .remainder()
        .iter()
        .zip(b.chunks_exact(8).remainder())
        .map(|(p, q)| p * q)
        .sum();
    lanes.iter().sum::<f32>() + rest
}

/// One open De-esser.
struct Deess {
    state: Mutex<State>,
    latency: u32,
}

struct State {
    rate: f64,
    taps: usize,
    lookahead: usize,
    /// The crossover, and the frequency it was built for.
    fir: Vec<f32>,
    fir_hz: f64,
    /// Each channel's last `taps` samples, written twice so a window of them
    /// is always one slice.
    history: Vec<Vec<f32>>,
    write: usize,
    /// Each channel's low and high band, `lookahead` frames of them, as a
    /// ring.
    low: Vec<Vec<f32>>,
    high: Vec<Vec<f32>>,
    ring: usize,
    /// The detector's two levels, as power.
    level_high: f64,
    level_full: f64,
    /// The reduction now, in dB, and how long it has left to hold.
    reduction: f64,
    hold: usize,
}

impl State {
    fn new(rate: f64, hz: f64) -> Self {
        let taps = ((TAPS_AT_48K * rate / 48_000.0).round() as usize) | 1;
        let lookahead = ((LOOKAHEAD_MS * 0.001 * rate).round() as usize).max(1);
        Self {
            rate,
            taps,
            lookahead,
            fir: low_pass(hz, rate, taps),
            fir_hz: hz,
            history: vec![vec![0.0; taps * 2]; AUDIO_CHANNELS],
            write: 0,
            low: vec![vec![0.0; lookahead]; AUDIO_CHANNELS],
            high: vec![vec![0.0; lookahead]; AUDIO_CHANNELS],
            ring: 0,
            level_high: 0.0,
            level_full: 0.0,
            reduction: 0.0,
            hold: 0,
        }
    }
}

/// How far a one-pole moves towards its target in one frame, for a time
/// constant of `ms`.
fn step_of(ms: f64, rate: f64) -> f64 {
    1.0 - (-1.0 / (ms.max(1e-3) * 0.001 * rate)).exp()
}

impl AudioProcessor for Deess {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let mut held = self.state.lock();
        let st = &mut *held;
        let threshold = row(values, AudioDeess::THRESHOLD, THRESHOLD_DB).clamp(-40.0, 0.0);
        let hz = row(values, AudioDeess::FREQUENCY, FREQUENCY_HZ).clamp(2_000.0, 12_000.0);
        let range = row(values, AudioDeess::RANGE, RANGE_DB).clamp(0.0, 24.0);
        let speed = row(values, AudioDeess::SPEED, SPEED_MS).clamp(20.0, 200.0);
        let wideband = on(values, AudioDeess::WIDEBAND, false);
        let only = on(values, AudioDeess::ESS_ONLY, false);
        if (hz - st.fir_hz).abs() > 1.0 {
            st.fir = low_pass(hz, st.rate, st.taps);
            st.fir_hz = hz;
        }
        let level = step_of(LEVEL_MS, st.rate);
        let attack = step_of(ATTACK_MS, st.rate);
        let release = step_of(speed, st.rate);
        let hold = (HOLD_MS * 0.001 * st.rate) as usize;
        let taps = st.taps;
        let centre = taps / 2;

        for (came, goes) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            // Split each channel, and keep its bands for the lookahead.
            let mut e_high = 0.0f64;
            let mut e_full = 0.0f64;
            let mut bands = [(0.0f32, 0.0f32); AUDIO_CHANNELS];
            for ((x, history), band) in came.iter().zip(st.history.iter_mut()).zip(bands.iter_mut())
            {
                if let Some(slot) = history.get_mut(st.write) {
                    *slot = *x;
                }
                if let Some(slot) = history.get_mut(st.write + taps) {
                    *slot = *x;
                }
                // The window ends at the newest sample; the filter is
                // symmetric, so its order within the window does not matter.
                let window = history
                    .get(st.write + 1..st.write + 1 + taps)
                    .unwrap_or(&[]);
                let low = dot(window, &st.fir);
                let delayed = window.get(taps - 1 - centre).copied().unwrap_or(0.0);
                let high = delayed - low;
                e_high += f64::from(high) * f64::from(high);
                e_full += f64::from(delayed) * f64::from(delayed);
                *band = (low, high);
            }
            st.write = (st.write + 1) % taps;

            // The detector, on the bands as they arrive.
            st.level_high += level * (e_high - st.level_high);
            st.level_full += level * (e_full - st.level_full);
            let full_db = 10.0 * (st.level_full / AUDIO_CHANNELS as f64).max(1e-20).log10();
            let rel = 10.0 * (st.level_high.max(1e-20) / st.level_full.max(1e-20)).log10();
            let quiet = ((full_db - QUIET_DB) / 10.0).clamp(0.0, 1.0);
            let want = ((rel - threshold).max(0.0) * SLOPE).min(range) * quiet;
            if want >= st.reduction {
                st.reduction += attack * (want - st.reduction);
                st.hold = hold;
            } else if st.hold > 0 {
                st.hold -= 1;
            } else {
                st.reduction += release * (want - st.reduction);
            }
            let gain = 10f64.powf(-st.reduction / 20.0) as f32;

            // The sound, lookahead frames late.
            for (((band, low), high), slot) in bands
                .iter()
                .zip(st.low.iter_mut())
                .zip(st.high.iter_mut())
                .zip(goes.iter_mut())
            {
                let l = low.get(st.ring).copied().unwrap_or(0.0);
                let h = high.get(st.ring).copied().unwrap_or(0.0);
                if let Some(s) = low.get_mut(st.ring) {
                    *s = band.0;
                }
                if let Some(s) = high.get_mut(st.ring) {
                    *s = band.1;
                }
                let kept = if wideband {
                    (l + h) * gain
                } else {
                    l + h * gain
                };
                *slot = if only { l + h - kept } else { kept };
            }
            st.ring = (st.ring + 1) % st.lookahead;
        }
        true
    }

    fn latency(&self) -> u32 {
        self.latency
    }
}

/// The reduction the detector asks for, in dB, over a mono `window` at the
/// given rows: the mean over the window, for the display's output estimate.
#[must_use]
pub(super) fn mean_reduction(window: &[f32], values: &[(ParamId, f64)], rate: u32) -> f64 {
    let stereo: Vec<f32> = window.iter().flat_map(|x| [*x, *x]).collect();
    let state = State::new(
        f64::from(rate.max(1)),
        row(values, AudioDeess::FREQUENCY, FREQUENCY_HZ),
    );
    let lat = state.taps / 2 + state.lookahead;
    let processor = Deess {
        state: Mutex::new(state),
        latency: lat as u32,
    };
    // The whole sound, so the energy that comes back is the reduction itself.
    let mut values = values.to_vec();
    values.retain(|(id, _)| *id != AudioDeess::ESS_ONLY && *id != AudioDeess::WIDEBAND);
    values.push((AudioDeess::WIDEBAND, 1.0));
    let mut out = vec![0.0f32; stereo.len()];
    let _ = processor.process(&stereo, &mut out, &values, 0);
    let (mut a, mut b) = (0.0f64, 0.0f64);
    for (i, o) in stereo.iter().zip(out.iter().skip(lat * 2)) {
        a += f64::from(*i) * f64::from(*i);
        b += f64::from(*o) * f64::from(*o);
    }
    if a <= 0.0 || b <= 0.0 {
        0.0
    } else {
        10.0 * (a / b).log10()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;

    #[test]
    fn it_is_deterministic_and_the_same_at_any_rate() {
        let input = harness::tone(48, 7_000.0);
        harness::deterministic(&AudioDeessDef, &[], &input);
        harness::same_seconds_at_any_rate(&AudioDeessDef, &[], 7_000.0, 1.0, 0.03);
    }

    /// **Plan 3**: bursts of hiss above 5 kHz over a low tone are ducked by
    /// several dB in the high band, and the tone between them is untouched.
    #[test]
    fn it_ducks_the_esses_and_leaves_the_voice() {
        let rate = 48_000.0;
        let frames = 512 * 200;
        let mut seed = 11u64;
        let mut hiss = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        // Hiss high-passed by a crude difference, in bursts of 120 ms every
        // 400 ms.
        let mut prev = 0.0f32;
        let mut ess = vec![0.0f32; frames];
        for (n, e) in ess.iter_mut().enumerate() {
            let x = hiss();
            let hp = (x - prev) * 0.5;
            prev = x;
            *e = if n % 19_200 < 5_760 { hp * 0.3 } else { 0.0 };
        }
        let voice: Vec<f32> = (0..frames)
            .map(|n| (std::f64::consts::TAU * 200.0 * n as f64 / rate).sin() as f32 * 0.3)
            .collect();
        let input: Vec<f32> = voice
            .iter()
            .zip(&ess)
            .flat_map(|(v, e)| [v + e, v + e])
            .collect();
        let values = harness::values(&AudioDeessDef, &[]);
        let p = harness::looked(&AudioDeessDef, &values, &input);
        let lat = p.latency() as usize;
        let out = harness::run(&*p, &input, &values);

        // Inside a burst, away from its edges: the high part of the output.
        let power = |x: &dyn Fn(usize) -> f32, range: std::ops::Range<usize>| -> f64 {
            range.map(|n| f64::from(x(n)).powi(2)).sum()
        };
        let burst = 19_200 * 3 + 1_000..19_200 * 3 + 5_000;
        let out_l = |n: usize| out[(n + lat) * 2] - voice[n];
        let in_l = |n: usize| ess[n];
        let ducked = 10.0 * (power(&in_l, burst.clone()) / power(&out_l, burst)).log10();
        assert!(ducked > 6.0, "the esses came down only {ducked} dB");
        // Between bursts the voice comes through as it went in.
        let gap = 19_200 * 3 + 9_000..19_200 * 3 + 18_000;
        let moved = gap
            .map(|n| (out[(n + lat) * 2] - voice[n]).abs())
            .fold(0.0f32, f32::max);
        assert!(moved < 1e-3, "the voice moved by {moved}");
    }
}

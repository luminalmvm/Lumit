//! Stereo width: the side signal scaled, the balance, and a mono bass
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A stereo pair is the same sound twice with the differences between the two
//! carrying the image. Take the average of the channels and you have what is
//! common to both, the mid; take half their difference and you have what is
//! only in one, the side. Scale the side and the image narrows or widens, and
//! at nought the two channels are the same sound, which is mono.
//!
//! Mono below is the mastering habit behind that: a bass note spread across
//! the two channels cancels itself on a mono system and rattles a record
//! cutter, so the side signal's bottom end is taken away and everything below
//! the crossover ends up in the middle. The crossover is one pole, because the
//! point is to remove the bass from the side rather than to draw a line.
//!
//! Latency and tail are both nought, so neither is overridden.

use std::f64::consts::{PI, SQRT_2, TAU};
use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::smoother::Smoother;
use super::{frame, put, row};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive.
const SMOOTH_MS: f64 = 10.0;

/// Below this the crossover is off. A one pole at 20 Hz takes nothing
/// audible out of the side, and a row's bottom end has to mean something.
const CROSSOVER_OFF_HZ: f64 = 20.0;

/// The Stereo width effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_stereo_width",
    label = "Stereo width",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioStereoWidth {
    /// How much of the side signal survives. Nought is mono, 100 is the
    /// recording as it stands, 200 is twice the difference.
    #[bounded(min = 0.0, max = 200.0, default = 100.0, unit = Percent)]
    pub width: f32,
    /// Left to right, through the constant-power law: the centre is unity and
    /// the ends are 3 dB up on the side they favour, so a sweep holds its
    /// loudness instead of dipping in the middle.
    #[bounded(min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub balance: f32,
    /// Everything under this frequency is folded into the middle. The row is
    /// linear rather than logarithmic because its bottom end is off, and a
    /// range starting at nought has no ratio to raise.
    #[bounded(min = 0.0, max = 500.0, default = 0.0, unit = Raw)]
    pub mono_below: f32,
}

/// The Stereo width effect's behaviour.
pub struct AudioStereoWidthDef;

impl EffectDef for AudioStereoWidthDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioStereoWidth as EffectMetadata>::SCHEMA
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
        let rate = f64::from(rate.max(1));
        let (left, right) = balance_gains(values);
        Some(Arc::new(WidthAudio {
            rate,
            state: Mutex::new(WidthState {
                width: Smoother::new(SMOOTH_MS, rate, width_of(values)),
                left: Smoother::new(SMOOTH_MS, rate, left),
                right: Smoother::new(SMOOTH_MS, rate, right),
                low: 0.0,
                keep: 0.0,
            }),
        }))
    }
}

/// The width row as a multiplier on the side signal.
fn width_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioStereoWidth::WIDTH, 100.0).max(0.0) / 100.0
}

/// The balance row as one multiplier a side.
///
/// Sine and cosine of the same angle, scaled so the centre is unity: two
/// weights whose squares sum to a constant are what stops the middle of a
/// sweep sounding quieter than its ends.
fn balance_gains(values: &[(ParamId, f64)]) -> (f64, f64) {
    let balance = row(values, AudioStereoWidth::BALANCE, 0.0).clamp(-100.0, 100.0);
    let angle = (balance / 100.0 + 1.0) * PI * 0.25;
    (angle.cos() * SQRT_2, angle.sin() * SQRT_2)
}

/// The smoothed rows and the crossover's one pole.
struct WidthState {
    width: Smoother,
    left: Smoother,
    right: Smoother,
    /// The side signal's bass, which is what gets taken away.
    low: f64,
    /// How much of a new sample the pole takes. Nought is the crossover off.
    keep: f64,
}

/// One open Stereo width.
struct WidthAudio {
    rate: f64,
    state: Mutex<WidthState>,
}

impl AudioProcessor for WidthAudio {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        state.width.set_target(width_of(values));
        let (left, right) = balance_gains(values);
        state.left.set_target(left);
        state.right.set_target(right);

        // The crossover is a coefficient rather than a smoothed value: it is
        // rebuilt at the block's start from this block's frequency and left
        // alone for the block, which is the contract's own control rate.
        let crossover = row(values, AudioStereoWidth::MONO_BELOW, 0.0);
        state.keep = if crossover <= CROSSOVER_OFF_HZ {
            state.low = 0.0;
            0.0
        } else {
            1.0 - (-TAU * crossover / self.rate).exp()
        };

        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let (l, r) = frame(i);
            let mid = f64::from(l + r) * 0.5;
            let mut side = f64::from(l - r) * 0.5 * state.width.step();
            if state.keep > 0.0 {
                state.low += state.keep * (side - state.low);
                side -= state.low;
            }
            put(
                o,
                ((mid + side) * state.left.step()) as f32,
                ((mid - side) * state.right.step()) as f32,
            );
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::harness;
    use super::*;

    /// A run's two channels, told apart.
    fn sides(samples: &[f32]) -> (Vec<f32>, Vec<f32>) {
        samples.chunks_exact(AUDIO_CHANNELS).map(frame).unzip()
    }

    #[test]
    fn a_bake_of_stereo_width_is_the_same_twice_and_survives_a_split() {
        let input = harness::tone(6, 180.0);
        harness::deterministic(&AudioStereoWidthDef, &[], &input);
        harness::deterministic(
            &AudioStereoWidthDef,
            &[("width", 175.0), ("balance", -40.0), ("mono_below", 150.0)],
            &input,
        );
    }

    #[test]
    fn width_at_nought_is_mono() {
        let input = harness::tone(4, 180.0);
        let values = harness::values(&AudioStereoWidthDef, &[("width", 0.0)]);
        let out = harness::run(
            &*harness::open(&AudioStereoWidthDef, &values),
            &input,
            &values,
        );
        let (left, right) = sides(&out);
        for (l, r) in left.iter().zip(&right) {
            assert!((l - r).abs() < 1e-6, "{l} and {r} are not one sound");
        }
        // And the mid is what came in: neither channel invented level.
        let (was_left, was_right) = sides(&input);
        for (n, l) in left.iter().enumerate() {
            let mid = (was_left[n] + was_right[n]) * 0.5;
            assert!((l - mid).abs() < 1e-6, "{l} is not the middle {mid}");
        }
    }

    #[test]
    fn width_at_a_hundred_is_the_recording_and_the_balance_favours_a_side() {
        let input = harness::tone(4, 180.0);
        let plain = harness::values(&AudioStereoWidthDef, &[]);
        let out = harness::run(
            &*harness::open(&AudioStereoWidthDef, &plain),
            &input,
            &plain,
        );
        for (there, here) in out.iter().zip(&input) {
            assert!((there - here).abs() < 1e-6, "{there} is not {here}");
        }

        let hard = harness::values(&AudioStereoWidthDef, &[("balance", 100.0)]);
        let out = harness::run(&*harness::open(&AudioStereoWidthDef, &hard), &input, &hard);
        let (left, right) = sides(&out);
        assert!(
            left.iter().all(|s| s.abs() < 1e-6),
            "hard right should empty the left"
        );
        // The law puts the end of the travel 3 dB up on the middle, so the
        // right comes back at root two of the level it went in at.
        let peak = right.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        let (_, was_right) = sides(&input);
        let was = was_right.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(
            (f64::from(peak / was) - SQRT_2).abs() < 0.01,
            "the right should be the louder for it: {peak} against {was}"
        );
    }

    #[test]
    fn mono_below_takes_the_bass_out_of_the_side() {
        // A low tone in one channel only: all side, no mid worth the name.
        let frames = 96 * crate::fx::AUDIO_BLOCK_FRAMES;
        let step = TAU * 40.0 / f64::from(harness::RATE);
        let input: Vec<f32> = (0..frames)
            .flat_map(|n| [(step * n as f64).sin() as f32 * 0.5, 0.0])
            .collect();

        // How far apart the channels stay, measured past the settling.
        let apart = |over: &[(&str, f64)]| {
            let values = harness::values(&AudioStereoWidthDef, over);
            let out = harness::run(
                &*harness::open(&AudioStereoWidthDef, &values),
                &input,
                &values,
            );
            let (left, right) = sides(&out);
            left[frames / 2..]
                .iter()
                .zip(&right[frames / 2..])
                .map(|(l, r)| (l - r).abs())
                .fold(0.0f32, f32::max)
        };

        let off = apart(&[]);
        let on = apart(&[("mono_below", 300.0)]);
        assert!(off > 0.45, "the tone should start hard in one side: {off}");
        assert!(on < off * 0.25, "40 Hz stayed in the side: {on} of {off}");
    }
}

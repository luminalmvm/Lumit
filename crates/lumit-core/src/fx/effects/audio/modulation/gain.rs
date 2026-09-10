//! Gain: a level in dB, a trim on each side, and a polarity flip
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! Vegas calls it Volume and Audacity calls it Amplify. The Gain row moves
//! both channels together, the two trims move one channel each, and Invert
//! flips the polarity for the one job polarity is for: a pair of microphones
//! wired the other way up, which cancel instead of adding when they are
//! summed.
//!
//! The dB knee is the fader's own. At the bottom of the Gain row's travel the
//! multiplier is exact zero rather than a whisper that costs cycles for ever,
//! which is what [`gain_of_db`] promises and what keeps a layer faded out
//! silent.
//!
//! Latency and tail are both nought, so neither is overridden: this effect
//! answers in the moment and goes quiet with its input.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::db::gain_of_db;
use super::super::dsp::smoother::Smoother;
use super::{frame, put, row};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive. Long enough that the step a baked
/// value makes at a block edge is heard as a ramp rather than a click, short
/// enough that a quick fade still lands where it was drawn.
const SMOOTH_MS: f64 = 5.0;

/// The Gain effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_gain",
    label = "Gain",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, so no matte and no picture: the Audio family's rule
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioGain {
    /// Both channels, in dB. The bottom of the travel is the fader's own
    /// silence floor, so a row swept to the end is silence rather than a very
    /// quiet sound.
    #[bounded(min = -100.0, max = 24.0, default = 0.0, unit = Raw)]
    pub gain: f32,
    /// The left channel alone, in dB, on top of Gain.
    #[bounded(min = -24.0, max = 24.0, default = 0.0, unit = Raw)]
    pub left_trim: f32,
    /// The right channel alone, in dB, on top of Gain.
    #[bounded(min = -24.0, max = 24.0, default = 0.0, unit = Raw)]
    pub right_trim: f32,
    /// 0 leaves the polarity alone, 1 flips both channels.
    #[counter(min = 0, max = 1, default = 0, hard_min = 0, hard_max = 1, unit = Raw)]
    pub invert: i32,
}

/// The Gain effect's behaviour.
pub struct AudioGainDef;

impl EffectDef for AudioGainDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioGain as EffectMetadata>::SCHEMA
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
        let (left, right) = multipliers(values);
        Some(Arc::new(GainAudio {
            state: Mutex::new(GainState {
                left: Smoother::new(SMOOTH_MS, rate, left),
                right: Smoother::new(SMOOTH_MS, rate, right),
            }),
        }))
    }
}

/// What the rows come to: one multiplier a side, the polarity already in it.
///
/// The flip rides in the multiplier rather than in a branch of its own, so
/// switching it lands as a ramp through zero and never as a step.
fn multipliers(values: &[(ParamId, f64)]) -> (f64, f64) {
    let gain = row(values, AudioGain::GAIN, 0.0);
    let polarity = if row(values, AudioGain::INVERT, 0.0) >= 0.5 {
        -1.0
    } else {
        1.0
    };
    (
        gain_of_db(gain + row(values, AudioGain::LEFT_TRIM, 0.0)) * polarity,
        gain_of_db(gain + row(values, AudioGain::RIGHT_TRIM, 0.0)) * polarity,
    )
}

/// The two multipliers on their way to what this block asked for.
struct GainState {
    left: Smoother,
    right: Smoother,
}

/// One open Gain.
struct GainAudio {
    /// Behind a mutex because `process` takes `&self`; uncontended by
    /// construction, since one instance is driven by one bake.
    state: Mutex<GainState>,
}

impl AudioProcessor for GainAudio {
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
        let (left, right) = multipliers(values);
        state.left.set_target(left);
        state.right.set_target(right);
        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let (l, r) = frame(i);
            put(
                o,
                l * state.left.step() as f32,
                r * state.right.step() as f32,
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

    /// The level of a run, as the loudest sample a side.
    fn peaks(samples: &[f32]) -> (f32, f32) {
        samples
            .chunks_exact(AUDIO_CHANNELS)
            .fold((0.0f32, 0.0f32), |(l, r), chunk| {
                let (a, b) = frame(chunk);
                (l.max(a.abs()), r.max(b.abs()))
            })
    }

    #[test]
    fn a_bake_of_gain_is_the_same_bake_twice_and_survives_a_split() {
        let input = harness::tone(6, 220.0);
        harness::deterministic(&AudioGainDef, &[("gain", 6.0)], &input);
        harness::deterministic(
            &AudioGainDef,
            &[("gain", -3.0), ("left_trim", 4.0), ("invert", 1.0)],
            &input,
        );
    }

    #[test]
    fn gain_raises_the_level_by_the_db_asked() {
        let input = harness::tone(8, 220.0);
        let plain = harness::values(&AudioGainDef, &[]);
        let louder = harness::values(&AudioGainDef, &[("gain", 6.0)]);
        let dry = harness::run(&*harness::open(&AudioGainDef, &plain), &input, &plain);
        let wet = harness::run(&*harness::open(&AudioGainDef, &louder), &input, &louder);

        // Unity is the input, sample for sample.
        assert_eq!(dry, input);
        // Six dB is a shade under twice, and the smoothing is long past by the
        // end of eight blocks.
        let (left, right) = peaks(&wet);
        let (was_left, was_right) = peaks(&input);
        assert!(
            (f64::from(left / was_left) - 1.995).abs() < 0.01,
            "left came back at {}",
            left / was_left
        );
        assert!((f64::from(right / was_right) - 1.995).abs() < 0.01);
    }

    #[test]
    fn the_trims_move_one_side_and_invert_flips_both() {
        let input = harness::tone(8, 220.0);
        let values = harness::values(&AudioGainDef, &[("left_trim", -6.0)]);
        let out = harness::run(&*harness::open(&AudioGainDef, &values), &input, &values);
        let (left, right) = peaks(&out);
        let (was_left, was_right) = peaks(&input);
        assert!((f64::from(left / was_left) - 0.5012).abs() < 0.01);
        assert!((f64::from(right / was_right) - 1.0).abs() < 1e-6);

        // Opened inverted, so the multiplier starts at its target and every
        // sample is the input's opposite from the first one.
        let values = harness::values(&AudioGainDef, &[("invert", 1.0)]);
        let out = harness::run(&*harness::open(&AudioGainDef, &values), &input, &values);
        for (there, here) in out.iter().zip(&input) {
            assert!((there + here).abs() < 1e-6, "{there} is not {here} flipped");
        }
    }

    #[test]
    fn the_bottom_of_the_travel_is_silence() {
        let input = harness::tone(4, 220.0);
        let values = harness::values(&AudioGainDef, &[("gain", -100.0)]);
        let out = harness::run(&*harness::open(&AudioGainDef, &values), &input, &values);
        assert!(out.iter().all(|s| *s == 0.0), "the knee is not exact");
    }
}

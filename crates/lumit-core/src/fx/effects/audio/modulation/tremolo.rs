//! Tremolo: the level moved by an oscillator, and auto-pan with it
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! One gain, moved up and down a few times a second. Depth says how far down
//! it goes; the top of the travel is always unity, because a tremolo that
//! boosted on the way up would be a tremolo and a fader at once.
//!
//! Stereo phase is what turns it into an auto-pan. At 180 degrees the two
//! channels are opposite, so as one side ducks the other rises and the sound
//! walks across the picture.
//!
//! The oscillator's phase is carried in the effect's own state and stepped a
//! frame at a time, so a Rate row that moves changes the speed of the cycle
//! rather than jumping it, and a run split at a block edge lands where the
//! whole run would have put it (§3).
//!
//! Latency and tail are both nought, so neither is overridden.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::lfo::{Lfo, Phase};
use super::super::dsp::smoother::Smoother;
use super::{frame, put, row, shape_of};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive.
const SMOOTH_MS: f64 = 10.0;

/// The Tremolo effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_tremolo",
    label = "Tremolo",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioTremolo {
    /// Cycles a second.
    #[bounded(min = 0.01, max = 20.0, default = 5.0, unit = Raw)]
    pub rate: f32,
    /// How far the gain falls at the bottom of the cycle. At 100 the bottom
    /// is silence.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub depth: f32,
    /// 0 sine, 1 triangle, 2 square. The square's edges are slewed, because a
    /// true step is a click.
    #[counter(min = 0, max = 2, default = 0, hard_min = 0, hard_max = 2, unit = Raw)]
    pub shape: i32,
    /// How far the right channel's cycle sits behind the left's. 180 is
    /// auto-pan.
    #[bounded(min = 0.0, max = 180.0, default = 0.0, unit = Degrees)]
    pub stereo_phase: f32,
}

/// The Tremolo effect's behaviour.
pub struct AudioTremoloDef;

impl EffectDef for AudioTremoloDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioTremolo as EffectMetadata>::SCHEMA
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
        Some(Arc::new(TremoloAudio {
            rate,
            state: Mutex::new(TremoloState {
                depth: Smoother::new(SMOOTH_MS, rate, depth_of(values)),
                phase: Phase::default(),
            }),
        }))
    }
}

/// The depth row as the share of the gain the cycle takes away.
fn depth_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioTremolo::DEPTH, 50.0).clamp(0.0, 100.0) / 100.0
}

/// The one row that would zipper if it were not smoothed, and where the
/// cycle stands. Rate and shape do not zipper: they enter through the
/// oscillator, which is built afresh at each block's start.
struct TremoloState {
    depth: Smoother,
    phase: Phase,
}

/// One open Tremolo.
struct TremoloAudio {
    rate: f64,
    state: Mutex<TremoloState>,
}

impl AudioProcessor for TremoloAudio {
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
        state.depth.set_target(depth_of(values));
        let shape = shape_of(row(values, AudioTremolo::SHAPE, 0.0));
        let speed = row(values, AudioTremolo::RATE, 5.0);
        let offset = row(values, AudioTremolo::STEREO_PHASE, 0.0);
        let left = Lfo::new(shape, speed, 0.0);
        let right = Lfo::new(shape, speed, offset);

        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let depth = state.depth.step();
            let phase = state.phase;
            // The cycle's top is unity and its bottom is 1 - depth.
            let gain = |lfo: &Lfo| 1.0 - depth * (1.0 - lfo.value(phase, self.rate)) * 0.5;
            let (l, r) = frame(i);
            put(
                o,
                (f64::from(l) * gain(&left)) as f32,
                (f64::from(r) * gain(&right)) as f32,
            );
            state.phase.advance(speed, self.rate);
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::harness;
    use super::*;

    #[test]
    fn a_bake_of_tremolo_is_the_same_twice_and_survives_a_split() {
        let input = harness::tone(8, 300.0);
        harness::deterministic(&AudioTremoloDef, &[("rate", 3.0)], &input);
        harness::deterministic(
            &AudioTremoloDef,
            &[("rate", 7.0), ("shape", 2.0), ("stereo_phase", 180.0)],
            &input,
        );
    }

    #[test]
    fn the_cycle_runs_at_the_same_speed_at_any_bake_rate() {
        harness::same_seconds_at_any_rate(
            &AudioTremoloDef,
            &[("rate", 4.0), ("depth", 80.0)],
            1_000.0,
            0.6,
            0.02,
        );
    }

    #[test]
    fn the_gain_moves_at_the_rate_asked() {
        harness::modulates_at(&AudioTremoloDef, &[("rate", 2.0), ("depth", 80.0)], 2.0);
    }

    #[test]
    fn the_cycle_tops_out_at_unity_and_bottoms_at_the_depth_asked() {
        // A constant input reads the gain back directly.
        let frames = 96 * crate::fx::AUDIO_BLOCK_FRAMES;
        let input = vec![1.0f32; frames * AUDIO_CHANNELS];
        let values = harness::values(&AudioTremoloDef, &[("rate", 2.0), ("depth", 60.0)]);
        let out = harness::run(&*harness::open(&AudioTremoloDef, &values), &input, &values);

        // Past the smoothing, which is ten milliseconds.
        let settled = &out[frames / 2 * AUDIO_CHANNELS..];
        let top = settled.iter().fold(f32::MIN, |a, s| a.max(*s));
        let bottom = settled.iter().fold(f32::MAX, |a, s| a.min(*s));
        assert!((top - 1.0).abs() < 1e-3, "the top should be unity: {top}");
        assert!(
            (bottom - 0.4).abs() < 1e-3,
            "the bottom should be 0.4: {bottom}"
        );
    }

    #[test]
    fn opposite_phases_duck_one_side_as_they_lift_the_other() {
        let frames = 96 * crate::fx::AUDIO_BLOCK_FRAMES;
        let input = vec![1.0f32; frames * AUDIO_CHANNELS];
        let values = harness::values(
            &AudioTremoloDef,
            &[("rate", 2.0), ("depth", 100.0), ("stereo_phase", 180.0)],
        );
        let out = harness::run(&*harness::open(&AudioTremoloDef, &values), &input, &values);
        // The two gains are a sine and its opposite, so at every frame past
        // the smoothing they sum to the same number.
        for chunk in out[frames / 2 * AUDIO_CHANNELS..].chunks_exact(AUDIO_CHANNELS) {
            let (l, r) = frame(chunk);
            assert!((l + r - 1.0).abs() < 1e-3, "{l} and {r} do not trade");
        }
    }
}

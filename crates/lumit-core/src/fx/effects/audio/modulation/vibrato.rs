//! Vibrato: the pitch moved by an oscillator
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A delay that is getting longer plays back slower, and one getting shorter
//! plays back faster. So a delay line whose read moves smoothly is a pitch
//! that wavers, which is what a singer does on a held note. This is the
//! chorus's delay line with none of the dry sound kept.
//!
//! Depth is in cents, a hundredth of a semitone, because that is what the
//! wobble is heard as. The sweep the line actually needs falls out of it: a
//! delay swept by `A` seconds at `f` cycles a second changes the playback
//! ratio by `A × 2π × f` at the steepest point of a sine, so the sweep is the
//! ratio the cents ask for divided by that. A slow, deep setting would want a
//! sweep longer than the line, so the sweep is held to
//! [`MAX_SWEEP_MS`] and the wobble stops deepening rather than reading
//! somebody else's sound.
//!
//! None of the dry sound is kept, so the six milliseconds the read rests at
//! delays the whole signal. That is reported as latency and the chain places
//! the job earlier by it, which is what puts the wobble around where the dry
//! sat rather than behind it. The tail is nought.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::delay_line::DelayLine;
use super::super::dsp::lfo::{Lfo, Phase};
use super::super::dsp::smoother::Smoother;
use super::{frame, put, row, shape_of};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive.
const SMOOTH_MS: f64 = 20.0;

/// Where the read sits with no sweep on it.
const BASE_MS: f64 = 6.0;

/// The widest sweep either side of that.
const MAX_SWEEP_MS: f64 = 5.0;

/// The Vibrato effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_vibrato",
    label = "Vibrato",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioVibrato {
    /// Cycles a second.
    #[bounded(min = 0.1, max = 20.0, default = 5.0, unit = Raw)]
    pub rate: f32,
    /// How far the pitch moves either way, in cents. A hundred is a semitone.
    #[bounded(min = 0.0, max = 100.0, default = 20.0, unit = Raw)]
    pub depth: f32,
    /// 0 sine, 1 triangle, 2 square.
    #[counter(min = 0, max = 2, default = 0, hard_min = 0, hard_max = 2, unit = Raw)]
    pub shape: i32,
}

/// The Vibrato effect's behaviour.
pub struct AudioVibratoDef;

impl EffectDef for AudioVibratoDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioVibrato as EffectMetadata>::SCHEMA
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
        // One allocation an instance, at construction, sized from the widest
        // read the rows can ask for.
        let frames = ((BASE_MS + MAX_SWEEP_MS + 1.0) * 0.001 * rate) as usize;
        Some(Arc::new(VibratoAudio {
            rate,
            base: BASE_MS * 0.001 * rate,
            state: Mutex::new(VibratoState {
                sweep: Smoother::new(SMOOTH_MS, rate, sweep_frames(values, rate)),
                left: DelayLine::new(frames),
                right: DelayLine::new(frames),
                phase: Phase::default(),
            }),
        }))
    }
}

/// How far the read moves either side of its resting place, in frames.
///
/// The cents are a playback ratio; the sweep that produces it is that ratio
/// over the oscillator's own angular speed.
fn sweep_frames(values: &[(ParamId, f64)], rate: f64) -> f64 {
    let cents = row(values, AudioVibrato::DEPTH, 20.0).max(0.0);
    let speed = row(values, AudioVibrato::RATE, 5.0).max(0.1);
    let ratio = 2f64.powf(cents / 1200.0) - 1.0;
    let seconds = ratio / (std::f64::consts::TAU * speed);
    (seconds * 1000.0).clamp(0.0, MAX_SWEEP_MS) * 0.001 * rate
}

/// The line a side, the sweep on its way to what this block asked for, and
/// where the cycle stands.
struct VibratoState {
    sweep: Smoother,
    left: DelayLine,
    right: DelayLine,
    phase: Phase,
}

/// One open Vibrato.
struct VibratoAudio {
    rate: f64,
    /// Where the read sits with no sweep on it, in frames.
    base: f64,
    state: Mutex<VibratoState>,
}

impl AudioProcessor for VibratoAudio {
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
        state.sweep.set_target(sweep_frames(values, self.rate));
        let speed = row(values, AudioVibrato::RATE, 5.0);
        let lfo = Lfo::new(shape_of(row(values, AudioVibrato::SHAPE, 0.0)), speed, 0.0);

        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let delay = self.base + state.sweep.step() * lfo.value(state.phase, self.rate);
            let (l, r) = frame(i);
            state.left.push(l);
            state.right.push(r);
            put(
                o,
                state.left.read_cubic(delay),
                state.right.read_cubic(delay),
            );
            state.phase.advance(speed, self.rate);
        }
        true
    }

    /// The read rests six milliseconds into the line and no dry sound is
    /// kept, so the whole signal comes out that late. Every other fixed delay
    /// in the suite is reported and made up for, and this one has to be too,
    /// or a copy of the same source on another layer combs against it.
    fn latency(&self) -> u32 {
        (BASE_MS * 0.001 * self.rate).round() as u32
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::harness;
    use super::*;

    #[test]
    fn a_bake_of_vibrato_is_the_same_twice_and_survives_a_split() {
        let input = harness::tone(8, 300.0);
        harness::deterministic(&AudioVibratoDef, &[], &input);
        harness::deterministic(
            &AudioVibratoDef,
            &[("rate", 1.5), ("depth", 80.0), ("shape", 1.0)],
            &input,
        );
    }

    #[test]
    fn the_pitch_moves_at_the_rate_asked() {
        harness::modulates_at(&AudioVibratoDef, &[("rate", 2.0), ("depth", 60.0)], 2.0);
    }

    #[test]
    fn the_read_follows_the_cycle_and_the_sweep_is_the_cents_asked() {
        // A ramp reads the delay back directly: the line hands back the input
        // as it was `delay` frames ago, so the difference between what went in
        // and what came out is the delay itself.
        let frames = 128 * crate::fx::AUDIO_BLOCK_FRAMES;
        let input: Vec<f32> = (0..frames).flat_map(|n| [n as f32, n as f32]).collect();
        let values = harness::values(&AudioVibratoDef, &[("rate", 2.0), ("depth", 60.0)]);
        let out = harness::run(&*harness::open(&AudioVibratoDef, &values), &input, &values);

        // Past the smoothing, the delay swings by the sweep the cents ask for.
        let delays: Vec<f64> = out[frames / 2 * AUDIO_CHANNELS..]
            .chunks_exact(AUDIO_CHANNELS)
            .enumerate()
            .map(|(n, chunk)| f64::from((frames / 2 + n) as f32 - frame(chunk).0))
            .collect();
        let deepest = delays.iter().fold(f64::MIN, |a, d| a.max(*d));
        let shallowest = delays.iter().fold(f64::MAX, |a, d| a.min(*d));
        let sweep = sweep_frames(&values, f64::from(harness::RATE));
        assert!(
            (deepest - shallowest - 2.0 * sweep).abs() < 1.0,
            "the read swung {} frames, not {}",
            deepest - shallowest,
            2.0 * sweep
        );
        // And the middle of the swing is where the read rests.
        let rest = BASE_MS * 0.001 * f64::from(harness::RATE);
        assert!(((deepest + shallowest) * 0.5 - rest).abs() < 1.0);

        // That resting place is the whole signal's delay, and it is what the
        // effect reports, so the chain places the job early enough to put the
        // wobble back where the dry sound sat.
        let processor = harness::open(&AudioVibratoDef, &values);
        assert_eq!(processor.latency(), rest.round() as u32);
        assert_eq!(processor.tail(), 0);
    }
}

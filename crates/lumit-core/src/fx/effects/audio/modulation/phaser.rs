//! Phaser: a chain of all-passes swept by an oscillator
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! An all-pass lets every frequency through at its own level and only turns
//! its phase. On its own that is inaudible. Add the untouched sound back in
//! and the two cancel wherever the chain has turned the phase half a cycle,
//! so a row of notches appears, and sweeping the chain walks the notches up
//! and down the sound. Four all-passes give two notches, twelve give six, and
//! the difference between a phaser and a flanger is that these notches are
//! spread out rather than evenly spaced.
//!
//! Feedback returns the last stage to the first, which narrows the notches
//! into the ringing whistle the effect is known for. It is held short of
//! unity because a chain that passes everything at its own level would
//! otherwise oscillate for ever.
//!
//! The stages are first order rather than the biquad's second, because the
//! note asks for first order and because a first-order section turns a
//! quarter cycle at its own frequency, which is what puts one notch per pair
//! of stages.
//!
//! Latency and tail are both nought, so neither is overridden.

use std::f64::consts::PI;
use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::lfo::{Lfo, Phase, Shape};
use super::super::dsp::smoother::Smoother;
use super::{frame, put, row};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive.
const SMOOTH_MS: f64 = 20.0;

/// The most stages a channel runs. The row's own maximum, so the array is
/// never indexed past what it holds.
const MAX_STAGES: usize = 12;

/// How far the sweep travels at full depth, either side of the centre.
const MAX_OCTAVES: f64 = 2.0;

/// One first-order all-pass and its two remembered samples.
///
/// The coefficient arrives with each sample rather than being stored, because
/// the whole point of the effect is that it moves.
#[derive(Debug, Clone, Copy, Default)]
struct AllPass {
    x1: f64,
    y1: f64,
}

impl AllPass {
    /// One sample through, at the coefficient this frame's frequency asks
    /// for.
    fn process(&mut self, x: f64, coeff: f64) -> f64 {
        let y = coeff * (x - self.y1) + self.x1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The all-pass coefficient for a section that turns a quarter cycle at
/// `freq_hz`. The tangent is the bilinear transform's prewarp, so the corner
/// lands where it was asked for rather than a little flat near the top of the
/// range.
fn coeff_of(freq_hz: f64, rate: f64) -> f64 {
    let freq = freq_hz.clamp(20.0, rate * 0.45);
    let t = (PI * freq / rate).tan();
    (t - 1.0) / (t + 1.0)
}

/// The Phaser effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_phaser",
    label = "Phaser",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, so no matte and no picture: the Audio family's rule
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioPhaser {
    /// How many all-passes the chain holds. Two stages make one notch.
    #[counter(min = 1, max = 12, default = 4, hard_min = 1, hard_max = 12, unit = Raw)]
    pub stages: i32,
    /// Cycles a second.
    #[bounded(min = 0.01, max = 10.0, default = 0.5, unit = Raw)]
    pub rate: f32,
    /// How far the sweep travels. At 100 the chain moves two octaves either
    /// side of the centre.
    #[bounded(min = 0.0, max = 100.0, default = 70.0, unit = Percent)]
    pub depth: f32,
    /// The middle of the sweep, in Hz.
    #[bounded(min = 50.0, max = 8000.0, default = 700.0, log = true, unit = Raw)]
    pub centre: f32,
    /// How much of the last stage goes back into the first. Negative feeds it
    /// back the other way up, which moves the notches.
    #[bounded(min = -95.0, max = 95.0, default = 50.0, unit = Percent)]
    pub feedback: f32,
    /// How far the right channel's cycle sits behind the left's.
    #[bounded(min = 0.0, max = 180.0, default = 0.0, unit = Degrees)]
    pub stereo_phase: f32,
    /// How much of the phased sound is heard. The notches are deepest at 50,
    /// where the two halves cancel outright.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub wet: f32,
}

/// The Phaser effect's behaviour.
pub struct AudioPhaserDef;

impl EffectDef for AudioPhaserDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioPhaser as EffectMetadata>::SCHEMA
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
        Some(Arc::new(PhaserAudio {
            rate,
            state: Mutex::new(PhaserState {
                stages: [[AllPass::default(); MAX_STAGES]; AUDIO_CHANNELS],
                held: [0.0; AUDIO_CHANNELS],
                centre: Smoother::new(SMOOTH_MS, rate, centre_of(values)),
                octaves: Smoother::new(SMOOTH_MS, rate, octaves_of(values)),
                feedback: Smoother::new(SMOOTH_MS, rate, feedback_of(values)),
                wet: Smoother::new(SMOOTH_MS, rate, wet_of(values)),
                phase: Phase::default(),
            }),
        }))
    }
}

/// The middle of the sweep, in Hz.
fn centre_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioPhaser::CENTRE, 700.0).clamp(20.0, 20_000.0)
}

/// How far the sweep travels, in octaves either side of the centre.
fn octaves_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioPhaser::DEPTH, 70.0).clamp(0.0, 100.0) / 100.0 * MAX_OCTAVES
}

/// The feedback row as a multiplier, held short of unity.
fn feedback_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioPhaser::FEEDBACK, 50.0).clamp(-95.0, 95.0) / 100.0
}

/// The wet row as a share of the output.
fn wet_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioPhaser::WET, 50.0).clamp(0.0, 100.0) / 100.0
}

/// The chain a channel, what the feedback holds, the four smoothed rows, and
/// where the sweep's cycle stands.
struct PhaserState {
    stages: [[AllPass; MAX_STAGES]; AUDIO_CHANNELS],
    held: [f64; AUDIO_CHANNELS],
    centre: Smoother,
    octaves: Smoother,
    feedback: Smoother,
    wet: Smoother,
    phase: Phase,
}

/// One open Phaser.
struct PhaserAudio {
    rate: f64,
    /// Behind a mutex because `process` takes `&self`; uncontended by
    /// construction, since one instance is driven by one bake.
    state: Mutex<PhaserState>,
}

impl AudioProcessor for PhaserAudio {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let Ok(mut guard) = self.state.lock() else {
            return false;
        };
        // Reborrowed once, so the chains and the feedback stores can be
        // walked together as two fields rather than two locks of the guard.
        let state = &mut *guard;
        state.centre.set_target(centre_of(values));
        state.octaves.set_target(octaves_of(values));
        state.feedback.set_target(feedback_of(values));
        state.wet.set_target(wet_of(values));
        let stages = row(values, AudioPhaser::STAGES, 4.0).round() as i64;
        let count = stages.clamp(1, MAX_STAGES as i64) as usize;
        let speed = row(values, AudioPhaser::RATE, 0.5);
        let offset = row(values, AudioPhaser::STEREO_PHASE, 0.0);
        let left_lfo = Lfo::new(Shape::Sine, speed, 0.0);
        let right_lfo = Lfo::new(Shape::Sine, speed, offset);

        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let phase = state.phase;
            let centre = state.centre.step();
            let octaves = state.octaves.step();
            let feedback = state.feedback.step();
            let wet = state.wet.step();
            let (l, r) = frame(i);
            let mut mixed = [0.0f32; AUDIO_CHANNELS];

            for (((stages, held), lfo), (dry, slot)) in state
                .stages
                .iter_mut()
                .zip(state.held.iter_mut())
                .zip([left_lfo, right_lfo])
                .zip([l, r].into_iter().zip(mixed.iter_mut()))
            {
                let coeff = coeff_of(
                    centre * (octaves * lfo.value(phase, self.rate)).exp2(),
                    self.rate,
                );
                let mut x = f64::from(dry) + *held * feedback;
                for stage in stages.iter_mut().take(count) {
                    x = stage.process(x, coeff);
                }
                *held = x;
                *slot = (f64::from(dry) + (x - f64::from(dry)) * wet) as f32;
            }

            let (left, right) = frame(&mixed);
            put(o, left, right);
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

    /// The level of the left channel over the settled half of a run.
    fn rms(samples: &[f32]) -> f64 {
        let half = samples.len() / 2 / AUDIO_CHANNELS * AUDIO_CHANNELS;
        let settled = &samples[half..];
        let sum: f64 = settled
            .chunks_exact(AUDIO_CHANNELS)
            .map(|chunk| f64::from(frame(chunk).0).powi(2))
            .sum();
        (sum / (settled.len() / AUDIO_CHANNELS) as f64).sqrt()
    }

    /// What a tone at `hz` comes back at, as a share of what went in.
    fn through(over: &[(&str, f64)], hz: f64) -> f64 {
        let input = harness::tone(40, hz);
        let values = harness::values(&AudioPhaserDef, over);
        let out = harness::run(&*harness::open(&AudioPhaserDef, &values), &input, &values);
        rms(&out) / rms(&input)
    }

    #[test]
    fn a_bake_of_phaser_is_the_same_twice_and_survives_a_split() {
        let input = harness::tone(8, 300.0);
        harness::deterministic(&AudioPhaserDef, &[], &input);
        harness::deterministic(
            &AudioPhaserDef,
            &[
                ("stages", 12.0),
                ("rate", 3.0),
                ("depth", 100.0),
                ("centre", 1_500.0),
                ("feedback", -90.0),
                ("stereo_phase", 180.0),
            ],
            &input,
        );
    }

    #[test]
    fn the_notches_move_at_the_rate_asked() {
        harness::modulates_at(&AudioPhaserDef, &[("rate", 2.0), ("depth", 100.0)], 2.0);
    }

    #[test]
    fn a_still_chain_notches_where_the_stages_put_it() {
        // Depth at nought parks the sweep on the centre, so the notch stands
        // still and can be measured. Four stages turn a quarter cycle each at
        // the centre, so they reach half a cycle together at the frequency
        // whose prewarped tangent is tan(22.5 degrees) of the centre's.
        let centre = 1_000.0;
        let rate = f64::from(harness::RATE);
        let notch = ((PI * centre / rate).tan() * (PI / 8.0).tan()).atan() * rate / PI;
        let still = [
            ("stages", 4.0),
            ("depth", 0.0),
            ("centre", centre),
            ("feedback", 0.0),
            ("wet", 50.0),
        ];

        let at_notch = through(&still, notch);
        assert!(at_notch < 0.15, "the notch only reached {at_notch}");
        // Far below it the chain has barely turned the phase, so the two
        // halves add back up to what went in.
        let below = through(&still, 50.0);
        assert!(below > 0.9, "the bottom end came back at {below}");
        // And far above it they have turned a whole cycle and add up again.
        let above = through(&still, 16_000.0);
        assert!(above > 0.9, "the top end came back at {above}");
    }
}

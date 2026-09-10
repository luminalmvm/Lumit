//! Chorus and flanger: a copy of the sound whose delay will not sit still
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! Take the sound, hold it back a few milliseconds, move that few
//! milliseconds about with an oscillator, and mix it back in. Where the copy
//! meets the original they cancel at whatever frequency the delay is half a
//! cycle of, so the sweep drags a comb of notches up and down the sound. A
//! long delay is heard as a second player slightly out of time, which is a
//! chorus; a short one is heard as the comb itself, which is a flanger.
//!
//! Mode is the difference between the two, and it is one number: how far the
//! sweep travels. Chorus wanders ten milliseconds at full depth, so the Delay
//! row at twenty five gives the note's fifteen to thirty five; flanger
//! wanders two and a quarter, so the row wound down to two and three quarters
//! gives its half to five.
//!
//! Voices are further reads of the same line at spread phases. They cost a
//! read each rather than a line each, which is why four of them are cheap.
//! Feedback returns the wet sound to the line and deepens the notches; it is
//! held short of unity so the comb cannot run away.
//!
//! Latency and tail are both nought, so neither is overridden. Feedback does
//! ring on a little after its input stops, and a ring of a few tens of
//! milliseconds is not worth lengthening every clip for.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::delay_line::DelayLine;
use super::super::dsp::lfo::{Lfo, Phase, Shape};
use super::super::dsp::smoother::Smoother;
use super::{frame, put, row};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive. Long, because the delay is what
/// moves and a delay dragged quickly is a pitch bend.
const SMOOTH_MS: f64 = 40.0;

/// The longest the Delay row goes.
const MAX_DELAY_MS: f64 = 40.0;

/// How far the sweep travels at full depth, one for each mode.
const CHORUS_SWEEP_MS: f64 = 10.0;
const FLANGER_SWEEP_MS: f64 = 2.25;

/// The most reads of the line one channel makes.
const MAX_VOICES: i64 = 4;

/// How far the right channel's cycle sits behind the left's. Fixed rather
/// than a row, because the rows the note lists have no place for it and a
/// chorus that is the same on both sides is not a chorus.
const STEREO_DEG: f64 = 90.0;

/// The Chorus effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_chorus",
    label = "Chorus",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, so no matte and no picture: the Audio family's rule
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioChorus {
    /// 0 chorus, 1 flanger. It sets how far the sweep travels, which is what
    /// the two names are.
    #[counter(min = 0, max = 1, default = 0, hard_min = 0, hard_max = 1, unit = Raw)]
    pub mode: i32,
    /// Cycles a second.
    #[bounded(min = 0.01, max = 10.0, default = 0.5, unit = Raw)]
    pub rate: f32,
    /// How much of the mode's travel the sweep uses.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub depth: f32,
    /// The middle of the sweep, in milliseconds.
    #[bounded(min = 0.1, max = 40.0, default = 25.0, unit = Raw)]
    pub delay: f32,
    /// How much of the wet sound goes back into the line. Negative feeds it
    /// back the other way up, which moves the comb's teeth.
    #[bounded(min = -95.0, max = 95.0, default = 0.0, unit = Percent)]
    pub feedback: f32,
    /// How many reads of the line each channel makes.
    #[counter(min = 1, max = 4, default = 2, hard_min = 1, hard_max = 4, unit = Raw)]
    pub voices: i32,
    /// How far apart in the cycle those reads sit.
    #[bounded(min = 0.0, max = 180.0, default = 90.0, unit = Degrees)]
    pub spread: f32,
    /// How much of the wet sound is heard. At nought the input comes through
    /// untouched.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub wet: f32,
}

/// The Chorus effect's behaviour.
pub struct AudioChorusDef;

impl EffectDef for AudioChorusDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioChorus as EffectMetadata>::SCHEMA
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
        // One allocation a channel, at construction, sized from the longest
        // read the rows can ask for.
        let frames = ((MAX_DELAY_MS + CHORUS_SWEEP_MS + 1.0) * 0.001 * rate) as usize;
        Some(Arc::new(ChorusAudio {
            rate,
            state: Mutex::new(ChorusState {
                lines: [DelayLine::new(frames), DelayLine::new(frames)],
                base: Smoother::new(SMOOTH_MS, rate, base_frames(values, rate)),
                sweep: Smoother::new(SMOOTH_MS, rate, sweep_frames(values, rate)),
                feedback: Smoother::new(SMOOTH_MS, rate, feedback_of(values)),
                wet: Smoother::new(SMOOTH_MS, rate, wet_of(values)),
                phase: Phase::default(),
            }),
        }))
    }
}

/// Whether the rows ask for the flanger's shorter travel.
fn flanging(values: &[(ParamId, f64)]) -> bool {
    row(values, AudioChorus::MODE, 0.0) >= 0.5
}

/// Where the read rests, in frames.
fn base_frames(values: &[(ParamId, f64)], rate: f64) -> f64 {
    row(values, AudioChorus::DELAY, 25.0).clamp(0.0, MAX_DELAY_MS) * 0.001 * rate
}

/// How far the read moves either side of that, in frames.
fn sweep_frames(values: &[(ParamId, f64)], rate: f64) -> f64 {
    let travel = if flanging(values) {
        FLANGER_SWEEP_MS
    } else {
        CHORUS_SWEEP_MS
    };
    row(values, AudioChorus::DEPTH, 50.0).clamp(0.0, 100.0) / 100.0 * travel * 0.001 * rate
}

/// The feedback row as a multiplier, held short of unity.
fn feedback_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioChorus::FEEDBACK, 0.0).clamp(-95.0, 95.0) / 100.0
}

/// The wet row as a share of the output.
fn wet_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioChorus::WET, 50.0).clamp(0.0, 100.0) / 100.0
}

/// A line a channel, the four rows that would zipper, and where the cycle
/// stands. Every voice reads that one phase at its own offset.
struct ChorusState {
    lines: [DelayLine; AUDIO_CHANNELS],
    base: Smoother,
    sweep: Smoother,
    feedback: Smoother,
    wet: Smoother,
    phase: Phase,
}

/// One open Chorus.
struct ChorusAudio {
    rate: f64,
    /// Behind a mutex because `process` takes `&self`; uncontended by
    /// construction, since one instance is driven by one bake.
    state: Mutex<ChorusState>,
}

impl AudioProcessor for ChorusAudio {
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
        state.base.set_target(base_frames(values, self.rate));
        state.sweep.set_target(sweep_frames(values, self.rate));
        state.feedback.set_target(feedback_of(values));
        state.wet.set_target(wet_of(values));
        let voices = (row(values, AudioChorus::VOICES, 2.0).round() as i64).clamp(1, MAX_VOICES);
        let speed = row(values, AudioChorus::RATE, 0.5);
        let spread = row(values, AudioChorus::SPREAD, 90.0);

        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let phase = state.phase;
            let base = state.base.step();
            let sweep = state.sweep.step();
            let feedback = state.feedback.step() as f32;
            let wet = state.wet.step() as f32;
            let (l, r) = frame(i);
            let mut mixed = [0.0f32; AUDIO_CHANNELS];

            for (channel, ((line, dry), slot)) in state
                .lines
                .iter_mut()
                .zip([l, r])
                .zip(mixed.iter_mut())
                .enumerate()
            {
                let mut voiced = 0.0;
                for voice in 0..voices {
                    let offset = spread * voice as f64 + STEREO_DEG * channel as f64;
                    let lfo = Lfo::new(Shape::Sine, speed, offset);
                    // Read before the push, so the feedback that goes in is
                    // the wet sound this frame just made.
                    voiced += line.read_cubic(base + sweep * lfo.value(phase, self.rate));
                }
                let voiced = voiced / voices as f32;
                line.push(dry + voiced * feedback);
                *slot = dry + (voiced - dry) * wet;
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

    /// The delay the wet read came back at, frame by frame, past the
    /// smoothing. A ramp reads it back directly: the line hands back what went
    /// in that many frames ago, so the difference is the delay itself.
    fn delays(over: &[(&str, f64)]) -> Vec<f64> {
        let frames = 128 * crate::fx::AUDIO_BLOCK_FRAMES;
        let input: Vec<f32> = (0..frames).flat_map(|n| [n as f32, n as f32]).collect();
        let mut over = over.to_vec();
        over.extend([("voices", 1.0), ("wet", 100.0)]);
        let values = harness::values(&AudioChorusDef, &over);
        let out = harness::run(&*harness::open(&AudioChorusDef, &values), &input, &values);
        out[frames / 2 * AUDIO_CHANNELS..]
            .chunks_exact(AUDIO_CHANNELS)
            .enumerate()
            .map(|(n, chunk)| f64::from((frames / 2 + n) as f32 - frame(chunk).0))
            .collect()
    }

    #[test]
    fn a_bake_of_chorus_is_the_same_twice_and_survives_a_split() {
        let input = harness::tone(8, 300.0);
        harness::deterministic(&AudioChorusDef, &[], &input);
        harness::deterministic(
            &AudioChorusDef,
            &[
                ("mode", 1.0),
                ("rate", 3.0),
                ("delay", 2.75),
                ("depth", 100.0),
                ("feedback", -60.0),
                ("voices", 4.0),
            ],
            &input,
        );
    }

    #[test]
    fn the_comb_moves_at_the_rate_asked() {
        harness::modulates_at(
            &AudioChorusDef,
            &[("rate", 2.0), ("depth", 100.0), ("wet", 100.0)],
            2.0,
        );
    }

    #[test]
    fn the_mode_sets_how_far_the_sweep_travels() {
        // A frame of travel at 48 kHz, and the two modes' widths from the
        // note: fifteen to thirty five, and half to five.
        let per_ms = f64::from(harness::RATE) * 0.001;
        let width = |over: &[(&str, f64)]| {
            let delays = delays(over);
            let deepest = delays.iter().fold(f64::MIN, |a, d| a.max(*d));
            let shallowest = delays.iter().fold(f64::MAX, |a, d| a.min(*d));
            (
                shallowest / per_ms,
                deepest / per_ms,
                (deepest + shallowest) * 0.5 / per_ms,
            )
        };

        let (from, to, middle) = width(&[("rate", 2.0), ("depth", 100.0)]);
        assert!((from - 15.0).abs() < 0.1, "the chorus started at {from} ms");
        assert!((to - 35.0).abs() < 0.1, "the chorus reached {to} ms");
        assert!((middle - 25.0).abs() < 0.1);

        let (from, to, _) = width(&[
            ("mode", 1.0),
            ("rate", 2.0),
            ("depth", 100.0),
            ("delay", 2.75),
        ]);
        assert!((from - 0.5).abs() < 0.1, "the flanger started at {from} ms");
        assert!((to - 5.0).abs() < 0.1, "the flanger reached {to} ms");
    }

    #[test]
    fn wet_at_nought_leaves_the_input_alone() {
        let input = harness::tone(4, 300.0);
        let values = harness::values(&AudioChorusDef, &[("wet", 0.0), ("feedback", 80.0)]);
        let out = harness::run(&*harness::open(&AudioChorusDef, &values), &input, &values);
        assert_eq!(out, input, "a dry chorus should change nothing at all");
    }
}

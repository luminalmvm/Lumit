//! Wah: a resonant band-pass walked up and down the sound
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A narrow band-pass filter, swept. Everything but a band of the sound is
//! taken away, and moving that band up and down is the vowel a wah pedal
//! makes. What moves it is the Mode row: an oscillator, which is the pedal
//! rocked at a steady rate, or the level of the sound itself, which is the
//! auto-wah and opens the filter as the playing gets harder.
//!
//! The filter is the state-variable one rather than a biquad, because it is
//! swept every sample and a biquad whose coefficients move that fast rings on
//! the answer it gave a moment ago.
//!
//! Resonance narrows the band. The band-pass is normalised so its peak stays
//! at unity as it narrows, which keeps a sharp setting from being a loud one
//! as well as a narrow one.
//!
//! Latency and tail are both nought, so neither is overridden.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::envelope::PeakFollower;
use super::super::dsp::lfo::{Lfo, Phase, Shape};
use super::super::dsp::smoother::Smoother;
use super::super::dsp::svf::Svf;
use super::{frame, put, row};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// How long a moved row takes to arrive.
const SMOOTH_MS: f64 = 20.0;

/// How far the sweep travels at full depth, either side of the frequency.
const MAX_OCTAVES: f64 = 2.0;

/// How the follower chases the playing in envelope mode. Quick up so a note
/// opens the filter as it is struck, slow down so it closes over the note
/// rather than fluttering through it.
const FOLLOW_ATTACK_MS: f64 = 5.0;
const FOLLOW_RELEASE_MS: f64 = 80.0;

/// How hard the level pushes the sweep at full sensitivity.
const MAX_FOLLOW_GAIN: f64 = 16.0;

/// The Wah effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_wah",
    label = "Wah",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, so no matte and no picture: the Audio family's rule
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioWah {
    /// 0 sweeps with an oscillator, 1 sweeps with the level of the sound.
    #[counter(min = 0, max = 1, default = 0, hard_min = 0, hard_max = 1, unit = Raw)]
    pub mode: i32,
    /// Cycles a second, in oscillator mode.
    #[bounded(min = 0.01, max = 10.0, default = 1.5, unit = Raw)]
    pub rate: f32,
    /// How far the sweep travels. At 100 the band moves two octaves either
    /// side of the frequency.
    #[bounded(min = 0.0, max = 100.0, default = 70.0, unit = Percent)]
    pub depth: f32,
    /// Where the band rests, in Hz. The oscillator sweeps either side of it;
    /// the level sweeps up from it.
    #[bounded(min = 100.0, max = 3000.0, default = 500.0, log = true, unit = Raw)]
    pub frequency: f32,
    /// How narrow the band is.
    #[bounded(min = 0.5, max = 20.0, default = 6.0, unit = Raw)]
    pub resonance: f32,
    /// How hard the level pushes the sweep, in envelope mode.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub sensitivity: f32,
    /// How much of the filtered sound is heard.
    #[bounded(min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub wet: f32,
}

/// The Wah effect's behaviour.
pub struct AudioWahDef;

impl EffectDef for AudioWahDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioWah as EffectMetadata>::SCHEMA
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
        let resonance = resonance_of(values);
        Some(Arc::new(WahAudio {
            rate,
            state: Mutex::new(WahState {
                filters: [
                    Svf::new(frequency_of(values), resonance, rate),
                    Svf::new(frequency_of(values), resonance, rate),
                ],
                follower: PeakFollower::new(FOLLOW_ATTACK_MS, FOLLOW_RELEASE_MS, rate),
                frequency: Smoother::new(SMOOTH_MS, rate, frequency_of(values)),
                octaves: Smoother::new(SMOOTH_MS, rate, octaves_of(values)),
                wet: Smoother::new(SMOOTH_MS, rate, wet_of(values)),
                phase: Phase::default(),
            }),
        }))
    }
}

/// Where the band rests, in Hz.
fn frequency_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioWah::FREQUENCY, 500.0).clamp(20.0, 20_000.0)
}

/// How far the sweep travels, in octaves.
fn octaves_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioWah::DEPTH, 70.0).clamp(0.0, 100.0) / 100.0 * MAX_OCTAVES
}

/// How narrow the band is.
fn resonance_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioWah::RESONANCE, 6.0).clamp(0.5, 20.0)
}

/// The wet row as a share of the output.
fn wet_of(values: &[(ParamId, f64)]) -> f64 {
    row(values, AudioWah::WET, 100.0).clamp(0.0, 100.0) / 100.0
}

/// The filter a channel, the one follower both share, and the smoothed rows.
struct WahState {
    filters: [Svf; AUDIO_CHANNELS],
    /// One follower over the louder channel, so the two sides sweep together
    /// and the image does not wander with the playing.
    follower: PeakFollower,
    frequency: Smoother,
    octaves: Smoother,
    wet: Smoother,
    /// Where the sweep's cycle stands, whichever mode is picked.
    phase: Phase,
}

/// One open Wah.
struct WahAudio {
    rate: f64,
    /// Behind a mutex because `process` takes `&self`; uncontended by
    /// construction, since one instance is driven by one bake.
    state: Mutex<WahState>,
}

impl AudioProcessor for WahAudio {
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
        let state = &mut *guard;
        state.frequency.set_target(frequency_of(values));
        state.octaves.set_target(octaves_of(values));
        state.wet.set_target(wet_of(values));
        let following = row(values, AudioWah::MODE, 0.0) >= 0.5;
        let resonance = resonance_of(values);
        // The band-pass peaks at the resonance, so the reciprocal is what
        // holds the peak at unity however narrow the band gets.
        let normalise = (1.0 / resonance) as f32;
        let sensitivity =
            row(values, AudioWah::SENSITIVITY, 50.0).clamp(0.0, 100.0) / 100.0 * MAX_FOLLOW_GAIN;
        let speed = row(values, AudioWah::RATE, 1.5);
        let lfo = Lfo::new(Shape::Sine, speed, 0.0);

        for (i, o) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let phase = state.phase;
            let frequency = state.frequency.step();
            let octaves = state.octaves.step();
            let wet = state.wet.step() as f32;
            let (l, r) = frame(i);

            // The follower runs in both modes so that a mode changed
            // mid-clip does not have to catch up from silence.
            let level = state.follower.process(l.abs().max(r.abs()));
            let travel = if following {
                (level * sensitivity).min(1.0)
            } else {
                lfo.value(phase, self.rate)
            };
            let swept = frequency * (octaves * travel).exp2();

            let mut mixed = [0.0f32; AUDIO_CHANNELS];
            for ((filter, dry), slot) in state.filters.iter_mut().zip([l, r]).zip(mixed.iter_mut())
            {
                filter.set(swept, resonance, self.rate);
                let band = filter.process(dry).band * normalise;
                *slot = dry + (band - dry) * wet;
            }

            let (left, right) = frame(&mixed);
            put(o, left, right);
            // The cycle runs in both modes, so a mode changed mid-clip picks
            // it up where it would have been rather than back at the start.
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

    /// The level of a run, as the loudest sample on the left.
    fn peak(samples: &[f32]) -> f32 {
        samples
            .chunks_exact(AUDIO_CHANNELS)
            .fold(0.0f32, |a, chunk| a.max(frame(chunk).0.abs()))
    }

    #[test]
    fn a_bake_of_wah_is_the_same_twice_and_survives_a_split() {
        let input = harness::tone(8, 700.0);
        harness::deterministic(&AudioWahDef, &[], &input);
        harness::deterministic(
            &AudioWahDef,
            &[
                ("mode", 1.0),
                ("depth", 100.0),
                ("frequency", 300.0),
                ("resonance", 12.0),
                ("sensitivity", 90.0),
            ],
            &input,
        );
    }

    #[test]
    fn the_band_moves_at_the_rate_asked() {
        harness::modulates_at(&AudioWahDef, &[("rate", 2.0), ("depth", 100.0)], 2.0);
    }

    #[test]
    fn the_band_rests_where_the_frequency_row_says() {
        // No sweep, so the band stands still and the tone at its middle comes
        // back where it went in while one two octaves off does not.
        let still = [("depth", 0.0), ("frequency", 800.0), ("resonance", 6.0)];
        let through = |hz: f64| {
            let input = harness::tone(40, hz);
            let values = harness::values(&AudioWahDef, &still);
            let out = harness::run(&*harness::open(&AudioWahDef, &values), &input, &values);
            f64::from(peak(&out) / peak(&input))
        };
        let middle = through(800.0);
        assert!((middle - 1.0).abs() < 0.05, "the band read {middle}");
        assert!(through(200.0) < 0.2, "the bottom end got through");
        assert!(through(3_200.0) < 0.2, "the top end got through");
    }

    #[test]
    fn the_level_opens_the_band_in_envelope_mode() {
        // The band rests low and a tone well above it only gets through as
        // the level pushes the band up to meet it.
        let over = [
            ("mode", 1.0),
            ("depth", 100.0),
            ("frequency", 300.0),
            ("sensitivity", 100.0),
        ];
        let through = |level: f32| {
            let input: Vec<f32> = harness::tone(40, 1_200.0)
                .iter()
                .map(|s| s * level)
                .collect();
            let values = harness::values(&AudioWahDef, &over);
            let out = harness::run(&*harness::open(&AudioWahDef, &values), &input, &values);
            f64::from(peak(&out) / peak(&input))
        };
        let quiet = through(0.02);
        let loud = through(1.0);
        assert!(
            loud > quiet * 4.0,
            "the level did not open the band: {quiet} against {loud}"
        );
    }
}

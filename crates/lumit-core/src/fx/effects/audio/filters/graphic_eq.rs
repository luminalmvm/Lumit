//! Graphic EQ (docs/impl/audio-effects.md §4): ten octave bands, one gain
//! each.
//!
//! # In plain terms
//!
//! This is the parametric with its shapes decided for you: ten bells at fixed
//! octave centres, all at the same width, and a fader for each. It is here
//! because both Vegas and Audacity ship one and a hand will look for it.
//!
//! Every band at nought dB is a passthrough sample for sample, because a bell
//! of no gain is the identity section rather than a near miss. The gains ramp
//! rather than step, and nothing is delayed or left ringing, so latency and
//! tail stay at nought.

use std::f64::consts::SQRT_2;
use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;

use super::{decibels, COEFF_FRAMES, SMOOTH_MS};
use crate::fx::effects::audio::dsp::biquad::{Cascade, Coeffs};
use crate::fx::effects::audio::dsp::db::gain_of_db;
use crate::fx::effects::audio::dsp::smoother::Smoother;
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// Bands: ten octaves, which is the whole of hearing.
const BANDS: usize = 10;

/// Where each fader sits, in hertz. Octaves from 31.25, so the series is exact
/// rather than the rounded numbers printed on the faders.
const CENTRES: [f64; BANDS] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

/// The width every band shares: `sqrt(2^n)/(2^n - 1)` for a bandwidth of one
/// octave. Constant Q is what makes the ten of them add up to the curve the
/// faders draw.
const Q: f64 = SQRT_2;

/// The graphic EQ's controls: ten faders and a trim.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_graphic_eq",
    label = "Graphic EQ",
    version = 1,
    category = Audio,
    // Cost and ROI describe an image operation, and this one has none.
    cost = Trivial,
    roi = Exact,
    // Sound in, sound out: there is no picture for a matte to hold back.
    matte = false,
)]
pub struct AudioGraphicEq {
    /// The 31 Hz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "31 Hz"
    )]
    pub gain_31: f32,
    /// The 63 Hz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "63 Hz"
    )]
    pub gain_63: f32,
    /// The 125 Hz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "125 Hz"
    )]
    pub gain_125: f32,
    /// The 250 Hz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "250 Hz"
    )]
    pub gain_250: f32,
    /// The 500 Hz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "500 Hz"
    )]
    pub gain_500: f32,
    /// The 1 kHz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "1 kHz"
    )]
    pub gain_1k: f32,
    /// The 2 kHz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "2 kHz"
    )]
    pub gain_2k: f32,
    /// The 4 kHz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "4 kHz"
    )]
    pub gain_4k: f32,
    /// The 8 kHz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "8 kHz"
    )]
    pub gain_8k: f32,
    /// The 16 kHz band, in dB.
    #[bounded(
        min = -12.0,
        max = 12.0,
        default = 0.0,
        unit = Raw,
        label = "16 kHz"
    )]
    pub gain_16k: f32,

    /// A trim on the way out, in dB, for the level the curve added or took.
    #[bounded(
        min = -24.0,
        max = 24.0,
        default = 0.0,
        unit = Raw,
        label = "Output"
    )]
    pub output: f32,
}

/// Every fader's row, in band order.
const ROWS: [ParamId; BANDS] = [
    AudioGraphicEq::GAIN_31,
    AudioGraphicEq::GAIN_63,
    AudioGraphicEq::GAIN_125,
    AudioGraphicEq::GAIN_250,
    AudioGraphicEq::GAIN_500,
    AudioGraphicEq::GAIN_1K,
    AudioGraphicEq::GAIN_2K,
    AudioGraphicEq::GAIN_4K,
    AudioGraphicEq::GAIN_8K,
    AudioGraphicEq::GAIN_16K,
];

/// Everything the graphic EQ carries between blocks.
struct GraphicState {
    gains: [Smoother; BANDS],
    /// The output trim as a multiplier, so the dB is raised to a gain once a
    /// block rather than once a frame.
    level: Smoother,
    left: Cascade<BANDS>,
    right: Cascade<BANDS>,
}

/// A live graphic EQ: one per bake, built by [`AudioGraphicEqDef::open_audio`].
struct Graphic {
    rate: f64,
    /// Behind a mutex because [`AudioProcessor::process`] takes `&self`.
    /// Uncontended by construction: one instance is driven by one bake.
    state: Mutex<GraphicState>,
}

impl AudioProcessor for Graphic {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let mut held = self.state.lock();
        let GraphicState {
            gains,
            level,
            left,
            right,
        } = &mut *held;
        for (gain, row) in gains.iter_mut().zip(&ROWS) {
            gain.set_target(decibels(values, *row, 0.0));
        }
        level.set_target(gain_of_db(decibels(values, AudioGraphicEq::OUTPUT, 0.0)));

        for (frame, (source, sink)) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
            .enumerate()
        {
            if frame % COEFF_FRAMES == 0 {
                for (i, gain) in gains.iter().enumerate() {
                    let coeffs = Coeffs::peaking(CENTRES[i], Q, gain.value(), self.rate);
                    left.set(i, coeffs);
                    right.set(i, coeffs);
                }
            }
            for gain in gains.iter_mut() {
                gain.step();
            }
            let trim = level.step() as f32;
            let ([l, r], [lo, ro]) = (source, sink) else {
                continue;
            };
            *lo = left.process(*l) * trim;
            *ro = right.process(*r) * trim;
        }
        true
    }
}

/// The graphic EQ's behaviour.
pub struct AudioGraphicEqDef;

impl EffectDef for AudioGraphicEqDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioGraphicEq as EffectMetadata>::SCHEMA
    }

    /// It touches sound and no pixel, so the resolve step pushes no op for it.
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
        // Opened from the bake's first block, so the first sample is already
        // where the faders say rather than ramping up to it.
        Some(Arc::new(Graphic {
            rate,
            state: Mutex::new(GraphicState {
                gains: std::array::from_fn(|i| {
                    Smoother::new(SMOOTH_MS, rate, decibels(values, ROWS[i], 0.0))
                }),
                level: Smoother::new(
                    SMOOTH_MS,
                    rate,
                    gain_of_db(decibels(values, AudioGraphicEq::OUTPUT, 0.0)),
                ),
                left: Cascade::new(),
                right: Cascade::new(),
            }),
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;
    use crate::fx::AUDIO_BLOCK_SAMPLES;

    /// **Plan 1**: the same input baked twice is bit-identical, and one run is
    /// its own two halves spliced at a block edge with the state carried.
    #[test]
    fn a_bake_of_the_graphic_eq_is_the_same_bake_twice_and_survives_a_split() {
        let input = harness::tone(6, 220.0);
        harness::deterministic(&AudioGraphicEqDef, &[], &input);
        harness::deterministic(
            &AudioGraphicEqDef,
            &[("gain_500", -8.0), ("gain_4k", 6.0), ("output", 2.0)],
            &input,
        );
    }

    /// The same split again, but opened flat and driven with a boost, so the
    /// cut lands in the middle of a ramp rather than on a settled filter.
    #[test]
    fn a_run_split_part_way_through_a_ramp_is_the_same_run() {
        let flat = harness::values(&AudioGraphicEqDef, &[]);
        let driven = harness::values(
            &AudioGraphicEqDef,
            &[("gain_125", 10.0), ("gain_8k", -10.0), ("output", -3.0)],
        );
        let input = harness::tone(4, 300.0);

        let whole = harness::run(&*harness::open(&AudioGraphicEqDef, &flat), &input, &driven);
        let carried = harness::open(&AudioGraphicEqDef, &flat);
        let cut = input.len() / 2;
        let mut halves = harness::run(&*carried, &input[..cut], &driven);
        halves.extend(harness::run_from(
            &*carried,
            &input[cut..],
            &driven,
            cut / AUDIO_BLOCK_SAMPLES,
        ));
        assert_eq!(whole, halves, "the ramp did not carry across the cut");
    }

    /// **Plan 3**: a fader raises its own band and leaves a distant one alone.
    /// 62.5 Hz is four octaves under the 1 kHz band, and four whole cycles of
    /// the measuring window.
    #[test]
    fn a_fader_lifts_its_own_band_and_leaves_a_distant_one_alone() {
        let over = [("gain_1k", 12.0)];
        let lifted = super::super::measured_db(&AudioGraphicEqDef, &over, 1_000.0);
        assert!((lifted - 12.0).abs() < 0.5, "the band measured {lifted}");
        let distant = super::super::measured_db(&AudioGraphicEqDef, &over, 62.5);
        assert!(distant.abs() < 0.5, "62.5 Hz moved by {distant}");
    }

    /// **Flat is a passthrough**, sample for sample: a bell of no gain is the
    /// identity section rather than a near miss, so ten of them in a row
    /// change nothing at all.
    #[test]
    fn every_fader_at_nought_hands_the_sound_straight_back() {
        let values = harness::values(&AudioGraphicEqDef, &[]);
        let input = harness::tone(3, 700.0);
        let out = harness::run(
            &*harness::open(&AudioGraphicEqDef, &values),
            &input,
            &values,
        );
        assert_eq!(out, input);
    }

    /// **A fader driven past its end still makes sound**: the section is built
    /// from whatever number arrives and none of it comes back infinite.
    #[test]
    fn a_fader_driven_past_its_end_still_makes_sound() {
        let values = harness::values(&AudioGraphicEqDef, &[("gain_31", 1e6), ("gain_16k", -1e6)]);
        let input = harness::tone(1, 1_000.0);
        let out = harness::run(
            &*harness::open(&AudioGraphicEqDef, &values),
            &input,
            &values,
        );
        assert!(out.iter().all(|s| s.is_finite()));
    }
}

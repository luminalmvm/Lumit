//! Parametric EQ (docs/impl/audio-effects.md §4): five bands, each of them one
//! of the cookbook's shapes.
//!
//! # In plain terms
//!
//! Five filters in a row, per channel. Band 1 arrives as a high pass and band
//! 5 as a low pass, the three between them are bells, and any band can be set
//! to any of the six shapes. Dropped on and left alone the whole thing is a
//! passthrough, sample for sample, which is what the last test here pins.
//!
//! Frequency, gain and Q ramp rather than step: a baked value only changes
//! once a block, and 512 frames of the same number followed by a different one
//! is the click the ramp exists to stop. Nothing is delayed and nothing rings
//! on, so latency and tail stay at nought.

use std::f64::consts::FRAC_1_SQRT_2;
use std::sync::Arc;

use lumit_fx_macros::Effect;
use parking_lot::Mutex;

use super::{decibels, value, COEFF_FRAMES, SMOOTH_MS};
use crate::fx::effects::audio::dsp::biquad::{Cascade, Coeffs};
use crate::fx::effects::audio::dsp::db::gain_of_db;
use crate::fx::effects::audio::dsp::smoother::Smoother;
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// Bands. Five is what a parametric ships with everywhere else, and the fewest
/// that holds a high pass, three bells and a low pass at once.
const BANDS: usize = 5;

/// The Type row's options, in the order it numbers them. A number outside the
/// list is a bell, which is the shape a band swept past either end should keep.
const BELL: i64 = 0;
const LOW_SHELF: i64 = 1;
const HIGH_SHELF: i64 = 2;
const HIGH_PASS: i64 = 3;
const LOW_PASS: i64 = 4;
const NOTCH: i64 = 5;

/// Where each band sits when the effect is dropped on: a high pass under the
/// voice, three bells across the middle, a low pass above the air.
const CENTRES: [f64; BANDS] = [80.0, 250.0, 1_000.0, 4_000.0, 12_000.0];

/// What shape each band starts as.
const SHAPES: [i64; BANDS] = [HIGH_PASS, BELL, BELL, BELL, LOW_PASS];

/// What Q each band starts at: Butterworth for the two filters, a bell's own
/// width for the three in the middle.
const QS: [f64; BANDS] = [FRAC_1_SQRT_2, 1.0, 1.0, 1.0, FRAC_1_SQRT_2];

/// Which bands start switched on.
///
/// The bells do, because a bell at nought dB is silence-thin: the effect
/// changes nothing until a gain is moved. The high pass and the low pass do
/// not, because a band that cuts the moment the effect lands would take the
/// bottom and the top off a clip nobody asked it to touch.
const ONS: [i64; BANDS] = [0, 1, 1, 1, 0];

/// The parametric EQ's controls: five bands of five rows, then a trim.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_eq",
    label = "Parametric EQ",
    version = 1,
    category = Audio,
    // Cost and ROI describe an image operation, and this one has none.
    cost = Trivial,
    roi = Exact,
    // Sound in, sound out: there is no picture for a matte to hold back.
    matte = false,
)]
pub struct AudioEq {
    /// Whether band 1 is in the path: 0 off, 1 on.
    #[counter(
        min = 0,
        max = 1,
        hard_min = 0,
        hard_max = 1,
        default = ONS[0],
        unit = Raw,
        label = "Band 1 on"
    )]
    pub band1_on: i32,
    /// Band 1's shape: 0 bell, 1 low shelf, 2 high shelf, 3 high pass, 4 low
    /// pass, 5 notch.
    #[counter(
        min = 0,
        max = 5,
        hard_min = 0,
        hard_max = 5,
        default = SHAPES[0],
        unit = Raw,
        label = "Band 1 type"
    )]
    pub band1_type: i32,
    /// Band 1's centre or corner, in hertz.
    #[bounded(
        min = 20.0,
        max = 20_000.0,
        default = CENTRES[0],
        log = true,
        unit = Raw,
        label = "Band 1 frequency"
    )]
    pub band1_freq: f32,
    /// Band 1's gain in dB, which a high pass, a low pass and a notch ignore.
    #[bounded(
        min = -24.0,
        max = 24.0,
        default = 0.0,
        unit = Raw,
        label = "Band 1 gain"
    )]
    pub band1_gain: f32,
    /// Band 1's Q: how narrow the shape is.
    #[bounded(
        min = 0.1,
        max = 10.0,
        default = QS[0],
        unit = Raw,
        label = "Band 1 Q"
    )]
    pub band1_q: f32,

    /// Whether band 2 is in the path: 0 off, 1 on.
    #[counter(
        min = 0,
        max = 1,
        hard_min = 0,
        hard_max = 1,
        default = ONS[1],
        unit = Raw,
        label = "Band 2 on"
    )]
    pub band2_on: i32,
    /// Band 2's shape: 0 bell, 1 low shelf, 2 high shelf, 3 high pass, 4 low
    /// pass, 5 notch.
    #[counter(
        min = 0,
        max = 5,
        hard_min = 0,
        hard_max = 5,
        default = SHAPES[1],
        unit = Raw,
        label = "Band 2 type"
    )]
    pub band2_type: i32,
    /// Band 2's centre or corner, in hertz.
    #[bounded(
        min = 20.0,
        max = 20_000.0,
        default = CENTRES[1],
        log = true,
        unit = Raw,
        label = "Band 2 frequency"
    )]
    pub band2_freq: f32,
    /// Band 2's gain in dB, which a high pass, a low pass and a notch ignore.
    #[bounded(
        min = -24.0,
        max = 24.0,
        default = 0.0,
        unit = Raw,
        label = "Band 2 gain"
    )]
    pub band2_gain: f32,
    /// Band 2's Q: how narrow the shape is.
    #[bounded(
        min = 0.1,
        max = 10.0,
        default = QS[1],
        unit = Raw,
        label = "Band 2 Q"
    )]
    pub band2_q: f32,

    /// Whether band 3 is in the path: 0 off, 1 on.
    #[counter(
        min = 0,
        max = 1,
        hard_min = 0,
        hard_max = 1,
        default = ONS[2],
        unit = Raw,
        label = "Band 3 on"
    )]
    pub band3_on: i32,
    /// Band 3's shape: 0 bell, 1 low shelf, 2 high shelf, 3 high pass, 4 low
    /// pass, 5 notch.
    #[counter(
        min = 0,
        max = 5,
        hard_min = 0,
        hard_max = 5,
        default = SHAPES[2],
        unit = Raw,
        label = "Band 3 type"
    )]
    pub band3_type: i32,
    /// Band 3's centre or corner, in hertz.
    #[bounded(
        min = 20.0,
        max = 20_000.0,
        default = CENTRES[2],
        log = true,
        unit = Raw,
        label = "Band 3 frequency"
    )]
    pub band3_freq: f32,
    /// Band 3's gain in dB, which a high pass, a low pass and a notch ignore.
    #[bounded(
        min = -24.0,
        max = 24.0,
        default = 0.0,
        unit = Raw,
        label = "Band 3 gain"
    )]
    pub band3_gain: f32,
    /// Band 3's Q: how narrow the shape is.
    #[bounded(
        min = 0.1,
        max = 10.0,
        default = QS[2],
        unit = Raw,
        label = "Band 3 Q"
    )]
    pub band3_q: f32,

    /// Whether band 4 is in the path: 0 off, 1 on.
    #[counter(
        min = 0,
        max = 1,
        hard_min = 0,
        hard_max = 1,
        default = ONS[3],
        unit = Raw,
        label = "Band 4 on"
    )]
    pub band4_on: i32,
    /// Band 4's shape: 0 bell, 1 low shelf, 2 high shelf, 3 high pass, 4 low
    /// pass, 5 notch.
    #[counter(
        min = 0,
        max = 5,
        hard_min = 0,
        hard_max = 5,
        default = SHAPES[3],
        unit = Raw,
        label = "Band 4 type"
    )]
    pub band4_type: i32,
    /// Band 4's centre or corner, in hertz.
    #[bounded(
        min = 20.0,
        max = 20_000.0,
        default = CENTRES[3],
        log = true,
        unit = Raw,
        label = "Band 4 frequency"
    )]
    pub band4_freq: f32,
    /// Band 4's gain in dB, which a high pass, a low pass and a notch ignore.
    #[bounded(
        min = -24.0,
        max = 24.0,
        default = 0.0,
        unit = Raw,
        label = "Band 4 gain"
    )]
    pub band4_gain: f32,
    /// Band 4's Q: how narrow the shape is.
    #[bounded(
        min = 0.1,
        max = 10.0,
        default = QS[3],
        unit = Raw,
        label = "Band 4 Q"
    )]
    pub band4_q: f32,

    /// Whether band 5 is in the path: 0 off, 1 on.
    #[counter(
        min = 0,
        max = 1,
        hard_min = 0,
        hard_max = 1,
        default = ONS[4],
        unit = Raw,
        label = "Band 5 on"
    )]
    pub band5_on: i32,
    /// Band 5's shape: 0 bell, 1 low shelf, 2 high shelf, 3 high pass, 4 low
    /// pass, 5 notch.
    #[counter(
        min = 0,
        max = 5,
        hard_min = 0,
        hard_max = 5,
        default = SHAPES[4],
        unit = Raw,
        label = "Band 5 type"
    )]
    pub band5_type: i32,
    /// Band 5's centre or corner, in hertz.
    #[bounded(
        min = 20.0,
        max = 20_000.0,
        default = CENTRES[4],
        log = true,
        unit = Raw,
        label = "Band 5 frequency"
    )]
    pub band5_freq: f32,
    /// Band 5's gain in dB, which a high pass, a low pass and a notch ignore.
    #[bounded(
        min = -24.0,
        max = 24.0,
        default = 0.0,
        unit = Raw,
        label = "Band 5 gain"
    )]
    pub band5_gain: f32,
    /// Band 5's Q: how narrow the shape is.
    #[bounded(
        min = 0.1,
        max = 10.0,
        default = QS[4],
        unit = Raw,
        label = "Band 5 Q"
    )]
    pub band5_q: f32,

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

/// One band's five rows, so a band can read its own values by index.
#[derive(Clone, Copy)]
struct Rows {
    on: ParamId,
    kind: ParamId,
    freq: ParamId,
    gain: ParamId,
    q: ParamId,
}

/// Every band's rows, in band order.
const ROWS: [Rows; BANDS] = [
    Rows {
        on: AudioEq::BAND1_ON,
        kind: AudioEq::BAND1_TYPE,
        freq: AudioEq::BAND1_FREQ,
        gain: AudioEq::BAND1_GAIN,
        q: AudioEq::BAND1_Q,
    },
    Rows {
        on: AudioEq::BAND2_ON,
        kind: AudioEq::BAND2_TYPE,
        freq: AudioEq::BAND2_FREQ,
        gain: AudioEq::BAND2_GAIN,
        q: AudioEq::BAND2_Q,
    },
    Rows {
        on: AudioEq::BAND3_ON,
        kind: AudioEq::BAND3_TYPE,
        freq: AudioEq::BAND3_FREQ,
        gain: AudioEq::BAND3_GAIN,
        q: AudioEq::BAND3_Q,
    },
    Rows {
        on: AudioEq::BAND4_ON,
        kind: AudioEq::BAND4_TYPE,
        freq: AudioEq::BAND4_FREQ,
        gain: AudioEq::BAND4_GAIN,
        q: AudioEq::BAND4_Q,
    },
    Rows {
        on: AudioEq::BAND5_ON,
        kind: AudioEq::BAND5_TYPE,
        freq: AudioEq::BAND5_FREQ,
        gain: AudioEq::BAND5_GAIN,
        q: AudioEq::BAND5_Q,
    },
];

/// One band on its way to what its rows hold.
#[derive(Clone, Copy)]
struct Band {
    freq: Smoother,
    gain: Smoother,
    q: Smoother,
    on: bool,
    kind: i64,
}

impl Band {
    /// Band `i`, sitting where the bake's first block says rather than ramping
    /// up to it.
    fn new(i: usize, values: &[(ParamId, f64)], rate: f64) -> Self {
        let mut band = Self {
            freq: Smoother::new(SMOOTH_MS, rate, CENTRES[i]),
            gain: Smoother::new(SMOOTH_MS, rate, 0.0),
            q: Smoother::new(SMOOTH_MS, rate, QS[i]),
            on: ONS[i] > 0,
            kind: SHAPES[i],
        };
        band.aim(i, values);
        band.freq.snap(band.freq.target());
        band.gain.snap(band.gain.target());
        band.q.snap(band.q.target());
        band
    }

    /// Aim at what this block's values hold. The switch and the shape land at
    /// once, because neither is a number that can be crossfaded.
    fn aim(&mut self, i: usize, values: &[(ParamId, f64)]) {
        let rows = ROWS[i];
        self.freq.set_target(value(values, rows.freq, CENTRES[i]));
        self.gain.set_target(decibels(values, rows.gain, 0.0));
        self.q.set_target(value(values, rows.q, QS[i]));
        self.on = value(values, rows.on, ONS[i] as f64) > 0.5;
        self.kind = value(values, rows.kind, SHAPES[i] as f64).round() as i64;
    }

    /// The section this band is right now.
    fn coeffs(&self, rate: f64) -> Coeffs {
        if !self.on {
            return Coeffs::IDENTITY;
        }
        let (freq, gain, q) = (self.freq.value(), self.gain.value(), self.q.value());
        match self.kind {
            LOW_SHELF => Coeffs::low_shelf(freq, q, gain, rate),
            HIGH_SHELF => Coeffs::high_shelf(freq, q, gain, rate),
            HIGH_PASS => Coeffs::high_pass(freq, q, rate),
            LOW_PASS => Coeffs::low_pass(freq, q, rate),
            NOTCH => Coeffs::notch(freq, q, rate),
            _ => Coeffs::peaking(freq, q, gain, rate),
        }
    }
}

/// Everything the EQ carries between blocks.
struct EqState {
    bands: [Band; BANDS],
    /// The output trim as a multiplier, so the dB is raised to a gain once a
    /// block rather than once a frame.
    level: Smoother,
    left: Cascade<BANDS>,
    right: Cascade<BANDS>,
}

/// A live parametric EQ: one per bake, built by [`AudioEqDef::open_audio`].
struct Parametric {
    rate: f64,
    /// Behind a mutex because [`AudioProcessor::process`] takes `&self`.
    /// Uncontended by construction: one instance is driven by one bake.
    state: Mutex<EqState>,
}

impl AudioProcessor for Parametric {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let mut held = self.state.lock();
        let EqState {
            bands,
            level,
            left,
            right,
        } = &mut *held;
        for (i, band) in bands.iter_mut().enumerate() {
            band.aim(i, values);
        }
        level.set_target(gain_of_db(decibels(values, AudioEq::OUTPUT, 0.0)));

        for (frame, (source, sink)) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
            .enumerate()
        {
            if frame % COEFF_FRAMES == 0 {
                for (i, band) in bands.iter().enumerate() {
                    let coeffs = band.coeffs(self.rate);
                    left.set(i, coeffs);
                    right.set(i, coeffs);
                }
            }
            for band in bands.iter_mut() {
                band.freq.step();
                band.gain.step();
                band.q.step();
            }
            let gain = level.step() as f32;
            let ([l, r], [lo, ro]) = (source, sink) else {
                continue;
            };
            *lo = left.process(*l) * gain;
            *ro = right.process(*r) * gain;
        }
        true
    }
}

/// The parametric EQ's behaviour.
pub struct AudioEqDef;

impl EffectDef for AudioEqDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioEq as EffectMetadata>::SCHEMA
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
        Some(Arc::new(Parametric {
            rate,
            state: Mutex::new(EqState {
                bands: std::array::from_fn(|i| Band::new(i, values, rate)),
                level: Smoother::new(
                    SMOOTH_MS,
                    rate,
                    gain_of_db(decibels(values, AudioEq::OUTPUT, 0.0)),
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
    fn a_bake_of_the_eq_is_the_same_bake_twice_and_survives_a_split() {
        let input = harness::tone(6, 220.0);
        harness::deterministic(&AudioEqDef, &[], &input);
        harness::deterministic(
            &AudioEqDef,
            &[
                ("band1_on", 1.0),
                ("band3_gain", 9.0),
                ("band5_on", 1.0),
                ("output", -2.0),
            ],
            &input,
        );
    }

    /// The same split again, but opened at the defaults and driven with a
    /// boost, so the cut lands in the middle of a ramp rather than on a
    /// settled filter. This is the half of the state a coefficient rebuilt
    /// part way through a block can get wrong.
    #[test]
    fn a_run_split_part_way_through_a_ramp_is_the_same_run() {
        let opened = harness::values(&AudioEqDef, &[]);
        let driven = harness::values(
            &AudioEqDef,
            &[
                ("band2_gain", -12.0),
                ("band4_freq", 6_000.0),
                ("output", 3.0),
            ],
        );
        let input = harness::tone(4, 300.0);

        let whole = harness::run(&*harness::open(&AudioEqDef, &opened), &input, &driven);
        let carried = harness::open(&AudioEqDef, &opened);
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

    /// **Plan 3**: a band boost raises that band's energy and leaves a distant
    /// band alone. Band 3 is the bell at 1 kHz; 62.5 Hz is four octaves under
    /// it, and four whole cycles of the measuring window.
    #[test]
    fn a_band_boost_lifts_that_band_and_leaves_a_distant_one_alone() {
        let over = [("band3_gain", 12.0)];
        let lifted = super::super::measured_db(&AudioEqDef, &over, 1_000.0);
        assert!((lifted - 12.0).abs() < 0.5, "the bell measured {lifted}");
        let distant = super::super::measured_db(&AudioEqDef, &over, 62.5);
        assert!(distant.abs() < 0.5, "62.5 Hz moved by {distant}");
    }

    /// **Plan 1's third clause**: the bell sits at the frequency asked
    /// whatever the bake rate is, so the same tone comes out at the same
    /// level. Every coefficient in the suite is worked out from the rate, and
    /// this is what pins that the rate reaches the filter at all.
    #[test]
    fn the_bell_sits_at_the_same_hertz_at_any_bake_rate() {
        harness::same_seconds_at_any_rate(&AudioEqDef, &[("band3_gain", 6.0)], 1_000.0, 0.6, 0.02);
    }

    /// **Dropped on and left alone it is a passthrough**, sample for sample:
    /// the three bells sit at nought dB, which is the identity section, the
    /// two filters are switched off, and the trim is unity.
    #[test]
    fn the_declared_defaults_hand_the_sound_straight_back() {
        let values = harness::values(&AudioEqDef, &[]);
        let input = harness::tone(3, 700.0);
        let out = harness::run(&*harness::open(&AudioEqDef, &values), &input, &values);
        assert_eq!(out, input);
    }

    /// **A row driven past its end still makes sound**: an unknown shape is a
    /// bell, and the cookbook holds the frequency and the Q off the ends, so
    /// nothing comes back infinite.
    #[test]
    fn a_row_driven_past_its_end_still_makes_sound() {
        let values = harness::values(
            &AudioEqDef,
            &[
                ("band1_on", 1.0),
                ("band1_type", 99.0),
                ("band1_freq", 1e9),
                ("band1_q", 0.0),
            ],
        );
        let input = harness::tone(1, 1_000.0);
        let out = harness::run(&*harness::open(&AudioEqDef, &values), &input, &values);
        assert!(out.iter().all(|s| s.is_finite()));
    }
}

//! Distortion (docs/impl/audio-effects.md §4): five ways to break a waveform,
//! all of them run at four times the rate.
//!
//! # In plain terms
//!
//! Bending a waveform makes harmonics, and a harmonic above half the sample
//! rate has nowhere to go: it folds back down as grit that moves the wrong way
//! when the pitch moves. So the sound is lifted to four times the rate before
//! it is bent and brought back down afterwards, which puts most of those
//! harmonics where the half-band pair can throw them away instead
//! ([`Oversample4x`]).
//!
//! The five shapes are the ones both Vegas and Audacity offer between them.
//! Three of them bend the level: soft clip rounds the peaks over, hard clip
//! takes them straight off, and foldback turns them back down again so the
//! sound gets stranger rather than only louder. The other two throw
//! information away instead: bit crush drops the resolution and rate reduce
//! holds each sample for several, which is the sound of an early sampler.
//!
//! **Drive means one thing per shape, and the row says so.** For the three
//! that bend the level it is a gain in dB into the shaper, which is what a
//! drive knob is. For the two that throw information away a gain would do
//! nothing but move the sound past the shape, so the same travel picks the bit
//! depth and the hold length. One row, one job, whichever shape is chosen.
//!
//! Tone is a tilt: one shelf down and its mirror up, either side of the same
//! corner, so the row leans the sound dark or bright without a hole in the
//! middle. It sits after the shaper, because tone on a distortion is what the
//! harmonics sound like rather than what went into making them.
//!
//! **The latency is real and is reported.** The half-band pair is linear
//! phase, so every frequency comes out exactly
//! [`Oversample4x::LATENCY_FRAMES`] frames late; the chain places the job that
//! much earlier and the sound lands where the dry did (§8). The dry side of
//! the Wet row goes through a line of the same length, or the untreated half
//! of the blend would arrive early and the row would flange rather than fade.
//! There is no tail: the shaper answers in the moment and goes quiet with its
//! input.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::super::dsp::biquad::{Cascade, Coeffs};
use super::super::dsp::db::gain_of_db;
use super::super::dsp::delay_line::DelayLine;
use super::super::dsp::oversample::Oversample4x;
use super::super::dsp::smoother::Smoother;
use super::{aim, held, value_of};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId};

/// The top of the Drive row, in dB.
const MAX_DRIVE_DB: f64 = 48.0;

/// Where the tilt turns over, in Hz. Low enough that Tone leans the whole of
/// the sound rather than only its top.
const TILT_HZ: f64 = 700.0;

/// The tilt's Q. Butterworth, so neither shelf has a bump of its own at the
/// corner and the pair stays a straight lean.
const TILT_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The bit depths the crush runs between: full travel is two bits, none of it
/// is sixteen and is inaudible, which is what makes a Drive of nought a
/// bypass for that shape.
const MAX_BITS: f64 = 16.0;
const MIN_BITS: f64 = 2.0;

/// What Rate reduce comes down to at the top of the Drive row, in Hz. As far
/// down as the shape is worth taking, and counted in hertz rather than in
/// samples so that a bake at 96 kHz sounds like a bake at 48 kHz.
const MIN_REDUCED_HZ: f64 = 3_000.0;

/// How long a moved row takes to arrive.
const SMOOTH_MS: f64 = 20.0;

/// Shape 0: the peaks rounded over.
const SOFT_CLIP: u8 = 0;
/// Shape 1: the peaks taken straight off.
const HARD_CLIP: u8 = 1;
/// Shape 2: the peaks turned back down again.
const FOLDBACK: u8 = 2;
/// Shape 3: the resolution dropped.
const BIT_CRUSH: u8 = 3;
/// Shape 4: each sample held for several.
const RATE_REDUCE: u8 = 4;

/// The Distortion effect's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_distortion",
    label = "Distortion",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound in, sound out: no picture to dissolve, and so no matte row
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioDistortion {
    /// How hard the sound is pushed into the shaper, in dB. On Bit crush and
    /// Rate reduce there is nothing for a gain to do, so the same travel picks
    /// the bit depth and the hold length instead.
    #[bounded(min = 0.0, max = 48.0, default = 12.0, unit = Raw)]
    pub drive: f32,
    /// 0 soft clips, 1 hard clips, 2 folds the peaks back, 3 crushes the bits,
    /// 4 drops the sample rate.
    #[counter(min = 0, max = 4, default = 0, hard_min = 0, hard_max = 4, unit = Raw)]
    pub shape: i32,
    /// A tilt over the shaped sound, in dB. Below nought leans it dark, above
    /// nought leans it bright.
    #[bounded(min = -12.0, max = 12.0, default = 0.0, unit = Raw)]
    pub tone: f32,
    /// The level on the way out, in dB. A shaper that clips at unity is quiet
    /// at a low drive and loud at a high one, and this is where that is put
    /// back.
    #[bounded(min = -24.0, max = 24.0, default = 0.0, unit = Raw)]
    pub output: f32,
    /// The share of the output that is the shaped sound rather than the sound
    /// itself.
    #[bounded(min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub wet: f32,
}

/// The Distortion effect's behaviour: sound, and no picture at all.
pub struct AudioDistortionDef;

impl EffectDef for AudioDistortionDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDistortion as EffectMetadata>::SCHEMA
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
        // A bake at no rate has no sound in it, and every coefficient below
        // would be a division by nothing.
        if rate == 0 {
            return None;
        }
        Some(Arc::new(AudioDistortionProcessor::new(
            values,
            f64::from(rate),
        )))
    }
}

/// One channel's whole path: up, shaped, down, tilted, and the dry held back
/// to meet it.
struct Channel {
    over: Oversample4x,
    tilt: Cascade<2>,
    dry: DelayLine,
    /// What Rate reduce is holding, and how many of the four-times samples it
    /// has held it for.
    held: f32,
    counted: u32,
}

impl Channel {
    fn new() -> Self {
        Self {
            over: Oversample4x::new(),
            tilt: Cascade::new(),
            dry: DelayLine::new(Oversample4x::LATENCY_FRAMES as usize),
            held: 0.0,
            counted: 0,
        }
    }

    /// One sample at four times the rate, bent.
    ///
    /// `levels` is the crush's quantiser and `hold` the reduce's run length;
    /// each is ignored by every shape but its own.
    fn shaped(&mut self, sample: f32, shape: u8, levels: f32, hold: u32) -> f32 {
        match shape {
            // A pair of literals, so `clamp` has no reversed bound to panic
            // on here (docs/14-ENGINEERING-RULES §4).
            HARD_CLIP => sample.clamp(-1.0, 1.0),
            // Reflected about plus and minus one, as many times as it takes,
            // in one go: the triangle that reflection draws, written down
            // rather than looped towards. A loop here would run a hundred
            // times at the top of the Drive row.
            FOLDBACK => ((sample - 1.0).rem_euclid(4.0) - 2.0).abs() - 1.0,
            BIT_CRUSH => (sample * levels).round() / levels,
            RATE_REDUCE => {
                if self.counted == 0 {
                    self.held = sample;
                }
                self.counted += 1;
                if self.counted >= hold {
                    self.counted = 0;
                }
                self.held
            }
            // Soft clip, and the answer for a shape row from a project saved
            // by a later version that knows a sixth one.
            _ => sample.tanh(),
        }
    }
}

/// The live distortion.
struct DistortionState {
    channel: [Channel; 2],
    /// The gain into the shaper. One for the two shapes Drive does not gain.
    drive: Smoother,
    output: Smoother,
    wet: Smoother,
    /// What the tilt was last set for, so the shelves are worked out only when
    /// the row has actually moved.
    tone_db: f64,
    primed: bool,
}

/// One open distortion.
pub struct AudioDistortionProcessor {
    rate: f64,
    state: Mutex<DistortionState>,
}

impl AudioDistortionProcessor {
    /// Open one from the values the chain's first block holds.
    fn new(values: &[(ParamId, f64)], rate: f64) -> Self {
        Self {
            rate,
            state: Mutex::new(DistortionState {
                channel: [Channel::new(), Channel::new()],
                drive: Smoother::new(SMOOTH_MS, rate, drive_gain(values)),
                output: Smoother::new(SMOOTH_MS, rate, 1.0),
                wet: Smoother::new(SMOOTH_MS, rate, 0.0),
                // Not a tone anybody can ask for, so the first block always
                // works the shelves out.
                tone_db: f64::MIN,
                primed: false,
            }),
        }
    }
}

/// The Shape row as one of the five.
fn shape_of(values: &[(ParamId, f64)]) -> u8 {
    held(value_of(values, AudioDistortion::SHAPE, 0.0), 0.0, 4.0).round() as u8
}

/// How far up its own travel the Drive row sits, 0 to 1.
fn drive_amount(values: &[(ParamId, f64)]) -> f64 {
    held(
        value_of(values, AudioDistortion::DRIVE, 12.0),
        0.0,
        MAX_DRIVE_DB,
    ) / MAX_DRIVE_DB
}

/// The gain into the shaper: the Drive row for the three shapes that bend the
/// level, and unity for the two that do not (see the module's own words).
fn drive_gain(values: &[(ParamId, f64)]) -> f64 {
    match shape_of(values) {
        SOFT_CLIP | HARD_CLIP | FOLDBACK => gain_of_db(drive_amount(values) * MAX_DRIVE_DB),
        _ => 1.0,
    }
}

impl AudioProcessor for AudioDistortionProcessor {
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

        // This block's numbers, worked out once at its start.
        let shape = shape_of(values);
        let amount = drive_amount(values);
        let tone = held(value_of(values, AudioDistortion::TONE, 0.0), -12.0, 12.0);
        let out_db = held(value_of(values, AudioDistortion::OUTPUT, 0.0), -24.0, 24.0);
        let wet = held(value_of(values, AudioDistortion::WET, 100.0), 0.0, 100.0) / 100.0;

        aim(&mut state.drive, drive_gain(values), state.primed);
        aim(&mut state.output, gain_of_db(out_db), state.primed);
        aim(&mut state.wet, wet, state.primed);
        state.primed = true;

        // Two shelves about one corner, the second the mirror of the first:
        // together they lean the sound without moving what sits at the corner.
        if tone != state.tone_db {
            let low = Coeffs::low_shelf(TILT_HZ, TILT_Q, -tone, self.rate);
            let high = Coeffs::high_shelf(TILT_HZ, TILT_Q, tone, self.rate);
            for channel in &mut state.channel {
                channel.tilt.set(0, low);
                channel.tilt.set(1, high);
            }
            state.tone_db = tone;
        }

        // Half a step either side of a level, so the quantiser's steps are
        // the bit depth's own.
        let bits = MAX_BITS - amount * (MAX_BITS - MIN_BITS);
        let levels = 2f64.powf(bits - 1.0) as f32;
        // The hold runs at four times the bake rate, so the length that
        // reaches MIN_REDUCED_HZ is worked out from that rate rather than
        // fixed. One is no reduction at all, which is what the bottom of the
        // row should mean.
        let longest = 4.0 * self.rate / MIN_REDUCED_HZ;
        let hold = 1 + (amount * (longest - 1.0)).round() as u32;

        for (frame_in, frame_out) in input.chunks_exact(2).zip(output.chunks_exact_mut(2)) {
            let ([left_in, right_in], [left_out, right_out]) = (frame_in, frame_out) else {
                continue;
            };
            let drive = state.drive.step() as f32;
            let out_gain = state.output.step() as f32;
            let wet = state.wet.step() as f32;

            for (channel, (sample, slot)) in state
                .channel
                .iter_mut()
                .zip([(*left_in, left_out), (*right_in, right_out)])
            {
                let mut up = channel.over.up(sample * drive);
                for slot in &mut up {
                    *slot = channel.shaped(*slot, shape, levels, hold);
                }
                let shaped = channel.tilt.process(channel.over.down(up)) * out_gain;

                // The dry goes through a line as long as the pair's delay, so
                // the two halves of the blend are the same moment of sound.
                channel.dry.push(sample);
                let dry = channel
                    .dry
                    .read_linear(f64::from(Oversample4x::LATENCY_FRAMES));
                *slot = dry * (1.0 - wet) + shaped * wet;
            }
        }
        true
    }

    /// The half-band pair's group delay, which is exact because the filter is
    /// linear phase (docs/impl/audio-effects.md §8).
    fn latency(&self) -> u32 {
        Oversample4x::LATENCY_FRAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::space::{bin, noise, play, play_all, tone};
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES};

    const RATE: u32 = 48_000;
    const RATE_F: f64 = 48_000.0;
    const LATENCY: usize = Oversample4x::LATENCY_FRAMES as usize;

    /// A full set of rows, so a test says only what it cares about.
    fn rows(drive: f64, shape: u8, tilt: f64, out: f64, wet: f64) -> Vec<(ParamId, f64)> {
        vec![
            (AudioDistortion::DRIVE, drive),
            (AudioDistortion::SHAPE, f64::from(shape)),
            (AudioDistortion::TONE, tilt),
            (AudioDistortion::OUTPUT, out),
            (AudioDistortion::WET, wet),
        ]
    }

    fn open(values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        AudioDistortionDef
            .open_audio(None, values, RATE, false)
            .unwrap_or_else(|| Arc::new(AudioDistortionProcessor::new(values, RATE_F)))
    }

    /// Where the loudest sample of a run sits, in frames.
    fn peak_at(signal: &[f32]) -> usize {
        signal
            .chunks_exact(2)
            .enumerate()
            .fold((0usize, 0.0f32), |(top_at, top), (n, frame)| {
                let level = frame.first().copied().unwrap_or(0.0).abs();
                if level > top {
                    (n, level)
                } else {
                    (top_at, top)
                }
            })
            .0
    }

    #[test]
    fn the_same_input_twice_is_bit_identical() {
        for shape in [SOFT_CLIP, HARD_CLIP, FOLDBACK, BIT_CRUSH, RATE_REDUCE] {
            let values = rows(30.0, shape, -6.0, 3.0, 70.0);
            let input = noise(AUDIO_BLOCK_FRAMES * 8);
            let first = play_all(open(&values).as_ref(), &input, &values);
            let second = play_all(open(&values).as_ref(), &input, &values);
            assert_eq!(first, second, "shape {shape} answered differently");
        }
    }

    #[test]
    fn one_run_and_two_runs_split_at_a_block_edge_agree() {
        for shape in [SOFT_CLIP, HARD_CLIP, FOLDBACK, BIT_CRUSH, RATE_REDUCE] {
            let values = rows(30.0, shape, -6.0, 3.0, 70.0);
            let input = noise(AUDIO_BLOCK_FRAMES * 8);
            let whole = play_all(open(&values).as_ref(), &input, &values);

            let carried = open(&values);
            let mut split = vec![0.0; input.len()];
            play(carried.as_ref(), &input, &values, 0..3, &mut split);
            play(carried.as_ref(), &input, &values, 3..8, &mut split);
            assert_eq!(whole, split, "shape {shape} split differently");
        }
    }

    /// Plan 3, the distortion's own: driving the shaper adds harmonics that
    /// were not in the sound before.
    #[test]
    fn driving_the_shaper_adds_harmonics() {
        // A kilohertz at 48 kHz is 48 frames a cycle, so the window measured
        // below holds a whole number of them and there is nothing to leak.
        let input = tone(AUDIO_BLOCK_FRAMES * 12, 1_000.0, RATE_F, 0.5);
        let third = |drive: f64| -> f64 {
            let values = rows(drive, SOFT_CLIP, 0.0, 0.0, 100.0);
            let out = play_all(open(&values).as_ref(), &input, &values);
            // Past the first three blocks, so the smoothed rows have arrived
            // and the half-band pair is full.
            bin(
                out.get(AUDIO_BLOCK_SAMPLES * 3..).unwrap_or_default(),
                3_000.0,
                RATE_F,
            )
        };
        let clean = third(0.0);
        let driven = third(36.0);
        assert!(
            driven > clean * 10.0 && driven > 0.05,
            "the third harmonic went from {clean} to {driven}"
        );
    }

    /// Plan 1's third clause: the same seconds of sound at every rate, the
    /// tilt's corner and the half-band's fill included.
    #[test]
    fn the_shaper_makes_the_same_seconds_at_any_bake_rate() {
        crate::fx::effects::audio::modulation::harness::same_seconds_at_any_rate(
            &AudioDistortionDef,
            &[("drive", MAX_DRIVE_DB), ("shape", f64::from(RATE_REDUCE))],
            1_000.0,
            0.6,
            0.02,
        );
    }

    /// Plan 1's third clause, the shape it bites on: the reduce is counted in
    /// hertz, so the same Drive holds the sound at the same rate whatever the
    /// bake runs at. Counted in frames it came down to 3 kHz out of a 48 kHz
    /// bake and 6 kHz out of a 96 kHz one, and preview and export parted
    /// company on a 96 kHz interface.
    #[test]
    fn rate_reduce_comes_down_to_the_same_hertz_at_any_bake_rate() {
        // A sample and hold at 3 kHz over a 1 kHz tone leaves an image at the
        // difference between the two. Held at 6 kHz instead the image sits at
        // 5 kHz and the 2 kHz bin is empty.
        let values = rows(MAX_DRIVE_DB, RATE_REDUCE, 0.0, 0.0, 100.0);
        for rate in [48_000u32, 96_000] {
            let hz = f64::from(rate);
            let scale = rate as usize / 48_000;
            let input = tone(AUDIO_BLOCK_FRAMES * 32 * scale, 1_000.0, hz, 0.5);
            let processor = AudioDistortionDef
                .open_audio(None, &values, rate, false)
                .unwrap_or_else(|| Arc::new(AudioDistortionProcessor::new(&values, hz)));
            let out = play_all(processor.as_ref(), &input, &values);

            // A fifth of a second, started past the smoothing. Both the tone
            // and the hold divide it, so the measurement has nothing to leak.
            let from = 4_096 * scale * 2;
            let window = 9_600 * scale * 2;
            let settled = out.get(from..from + window).unwrap_or_default();
            let image = bin(settled, 2_000.0, hz);
            assert!(image > 0.05, "at {rate} the 2 kHz image measured {image}");
        }
    }

    /// §8's trap: the pair's delay is exact, is what the effect reports, and
    /// is what the dry side of the blend is held back by.
    #[test]
    fn the_delay_is_the_one_reported_and_the_dry_is_held_to_meet_it() {
        let values = rows(0.0, HARD_CLIP, 0.0, 0.0, 100.0);
        let processor = open(&values);
        assert_eq!(processor.latency(), Oversample4x::LATENCY_FRAMES);
        assert_eq!(processor.tail(), 0);

        // Half of full scale, so a hard clip at unity leaves the pair linear
        // and the impulse arrives whole.
        let mut input = vec![0.0f32; AUDIO_BLOCK_SAMPLES * 2];
        if let Some(frame) = input.get_mut(0..2) {
            frame.fill(0.5);
        }
        let out = play_all(processor.as_ref(), &input, &values);
        assert_eq!(peak_at(&out), LATENCY, "the shaped sound landed elsewhere");

        // With the wet row shut the output is the sound itself, that same
        // delay late and otherwise untouched.
        let dry_values = rows(0.0, HARD_CLIP, 0.0, 0.0, 0.0);
        let sound = noise(AUDIO_BLOCK_FRAMES * 4);
        let out = play_all(open(&dry_values).as_ref(), &sound, &dry_values);
        let held = out.get(LATENCY * 2..).unwrap_or_default();
        let sent = sound.get(..held.len()).unwrap_or_default();
        assert_eq!(held, sent, "the dry side did not line up with the wet");
    }
}

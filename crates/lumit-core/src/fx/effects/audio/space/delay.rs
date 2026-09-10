//! Delay (docs/impl/audio-effects.md §4): the repeat, and how long it takes to
//! give up.
//!
//! # In plain terms
//!
//! Sound goes into a ring and comes back out a set time later. Some of what
//! comes out goes round again, quieter, until it falls under the silence
//! floor, which is where the repeats stop and the effect's tail ends. On the
//! way round it passes a low pass, so each repeat is duller than the one
//! before, the way a real reflection is. Ping-pong is one line's return fed
//! into the other, so the repeats walk across the stereo picture.
//!
//! The tail is worked out once, at the bake, from the time and the feedback
//! the effect opens with: the chain runs that many extra frames of silence so
//! the last repeat actually comes out, and the mix places the whole of what
//! came back, so an echo simply rings on past the clip's out point.
//!
//! # Sync does nothing yet, and this is why
//!
//! The note asks the repeat to lock to the comp's confirmed beat grid. There
//! is no road for it: `chain_bake` hands a processor its state, its baked row
//! values, the rate and the offline flag, and nothing else, and the plugin
//! host says the same thing about transport in its own words
//! (`lumit-aplug`'s `process` passes a null transport, "no transport in v1").
//! So the grid reaches neither a plugin nor a built-in today. The Sync and
//! Note rows are declared because the note's table names them and because the
//! saved value should survive being written now, but Time is what the repeat
//! reads whatever Sync says. When the grid arrives with the mix seam, the
//! whole of the change is here: read the tempo, turn the Note division into
//! milliseconds, and use it instead of Time.

use std::sync::{Arc, Mutex};

use super::super::dsp::biquad::{Biquad, Coeffs};
use super::super::dsp::db::{db_of_gain, SILENCE_FLOOR_DB};
use super::super::dsp::delay_line::DelayLine;
use super::super::dsp::smoother::Smoother;
use super::{aim, held, value_of};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId};
use lumit_fx_macros::Effect;

/// The longest repeat the Time row offers, in milliseconds. The line is taken
/// at this length whatever the row opens at, because a driver may sweep the
/// row up mid-bake and a line cannot grow in `process`.
const MAX_TIME_MS: f64 = 2_000.0;

/// How long the whole of a delay's tail may be, in seconds.
///
/// ponytail: a flat ceiling rather than a measured decay. Two seconds of
/// repeats at 95 per cent feedback take seven minutes to reach the silence
/// floor, and rendering seven minutes of tail past a clip is a worse answer
/// than stopping at half a minute. Measure the real decay if anyone ever
/// wants the whole of one.
const TAIL_CEILING_S: f64 = 30.0;

/// How fast the repeat time follows the row. Slow enough that dragging Time
/// glides rather than jumps, which is what a delay is expected to do.
const TIME_GLIDE_MS: f64 = 60.0;

/// How fast the plain gains follow their rows.
const GAIN_GLIDE_MS: f64 = 20.0;

/// A delay: one line a channel, fed back through a low pass.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_delay",
    label = "Delay",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound in, sound out: no picture to dissolve, and so no matte row
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioDelay {
    /// How far behind the sound the first repeat lands, in milliseconds.
    #[bounded(min = 1.0, max = 2_000.0, default = 250.0, unit = Raw)]
    pub time: f32,
    /// Which division of the beat the repeat would land on, once the grid
    /// reaches the bake. Inert today, as the module's own words say.
    #[counter(
        min = 0,
        max = 6,
        default = 2,
        hard_min = 0,
        hard_max = 6,
        unit = Raw
    )]
    pub note: i32,
    /// Whether the repeat follows the beat grid instead of Time. Inert today.
    #[counter(
        min = 0,
        max = 1,
        default = 0,
        hard_min = 0,
        hard_max = 1,
        unit = Raw
    )]
    pub sync: i32,
    /// How much of each repeat goes round again. Held under a hundred: at one
    /// the repeats would never stop and there would be no tail to report.
    #[bounded(min = 0.0, max = 95.0, default = 35.0, unit = Percent)]
    pub feedback: f32,
    /// Where the low pass in the feedback sits, so each repeat is duller than
    /// the one before.
    #[bounded(
        min = 200.0,
        max = 20_000.0,
        default = 6_000.0,
        log = true,
        unit = Raw
    )]
    pub damping: f32,
    /// How much of one channel's return crosses into the other. At a hundred
    /// the repeats walk left, right, left.
    #[bounded(min = 0.0, max = 100.0, default = 0.0, unit = Percent, label = "Ping-pong")]
    pub ping_pong: f32,
    /// The share of the output that is repeats rather than the sound itself.
    #[bounded(min = 0.0, max = 100.0, default = 35.0, unit = Percent)]
    pub wet: f32,
}

/// The delay's behaviour: sound, and no picture at all.
pub struct AudioDelayDef;

impl EffectDef for AudioDelayDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioDelay as EffectMetadata>::SCHEMA
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
        Some(Arc::new(AudioDelayProcessor::new(values, f64::from(rate))))
    }
}

/// The live delay: two lines, the filters in their feedback, and the ramps
/// that stop a knob clicking.
struct DelayState {
    line: [DelayLine; 2],
    damping: [Biquad; 2],
    /// The repeat, in samples.
    delay: Smoother,
    feedback: Smoother,
    /// How much of the return crosses channels, 0 to 1.
    cross: Smoother,
    wet: Smoother,
    /// What the damping filter was last set for, so the coefficients are
    /// worked out only when the row has actually moved.
    damping_hz: f64,
    /// Whether a block has been through yet.
    primed: bool,
}

/// One open delay.
pub struct AudioDelayProcessor {
    rate: f64,
    tail: u32,
    state: Mutex<DelayState>,
}

impl AudioDelayProcessor {
    /// Open one from the values the chain's first block holds.
    fn new(values: &[(ParamId, f64)], rate: f64) -> Self {
        let time = held(value_of(values, AudioDelay::TIME, 250.0), 1.0, MAX_TIME_MS);
        let feedback = held(value_of(values, AudioDelay::FEEDBACK, 35.0), 0.0, 95.0) / 100.0;
        let delay = frames_of_ms(time, rate);
        // The whole of the Time row's travel, because a driver may sweep the
        // row past what it opened at and the line cannot grow later.
        let longest = frames_of_ms(MAX_TIME_MS, rate).ceil() as usize + 4;
        Self {
            rate,
            tail: tail_frames(delay, feedback, rate),
            state: Mutex::new(DelayState {
                line: [DelayLine::new(longest), DelayLine::new(longest)],
                damping: [Biquad::default(), Biquad::default()],
                delay: Smoother::new(TIME_GLIDE_MS, rate, delay),
                feedback: Smoother::new(GAIN_GLIDE_MS, rate, feedback),
                cross: Smoother::new(GAIN_GLIDE_MS, rate, 0.0),
                wet: Smoother::new(GAIN_GLIDE_MS, rate, 0.0),
                damping_hz: 0.0,
                primed: false,
            }),
        }
    }
}

/// Milliseconds as frames at `rate`, never under the two samples the cubic
/// read needs a neighbour either side of.
fn frames_of_ms(ms: f64, rate: f64) -> f64 {
    (ms * 0.001 * rate).max(2.0)
}

/// Frames between the input stopping and the last repeat falling under the
/// silence floor.
fn tail_frames(delay: f64, feedback: f64, rate: f64) -> u32 {
    // Every pass round the loop is `feedback` of the one before, so the
    // repeats fall by a fixed number of decibels every `delay` frames and the
    // count of passes is one division.
    let per_pass = db_of_gain(feedback);
    let passes = if per_pass < 0.0 {
        SILENCE_FLOOR_DB / per_pass
    } else {
        1.0
    };
    (delay * passes).min(TAIL_CEILING_S * rate) as u32
}

impl AudioProcessor for AudioDelayProcessor {
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
        let time = held(value_of(values, AudioDelay::TIME, 250.0), 1.0, MAX_TIME_MS);
        let feedback = held(value_of(values, AudioDelay::FEEDBACK, 35.0), 0.0, 95.0) / 100.0;
        let damping = held(
            value_of(values, AudioDelay::DAMPING, 6_000.0),
            200.0,
            20_000.0,
        );
        let cross = held(value_of(values, AudioDelay::PING_PONG, 0.0), 0.0, 100.0) / 100.0;
        let wet = held(value_of(values, AudioDelay::WET, 35.0), 0.0, 100.0) / 100.0;

        aim(
            &mut state.delay,
            frames_of_ms(time, self.rate),
            state.primed,
        );
        aim(&mut state.feedback, feedback, state.primed);
        aim(&mut state.cross, cross, state.primed);
        aim(&mut state.wet, wet, state.primed);
        state.primed = true;

        // Coefficients only where the row has moved. The state is kept:
        // throwing a filter's history away mid-sound is a click.
        if damping != state.damping_hz {
            // Butterworth, so the low pass has no peak of its own and the
            // loop gain stays under the feedback the row asked for.
            let coeffs = Coeffs::low_pass(damping, std::f64::consts::FRAC_1_SQRT_2, self.rate);
            for filter in &mut state.damping {
                filter.set(coeffs);
            }
            state.damping_hz = damping;
        }

        for (frame_in, frame_out) in input.chunks_exact(2).zip(output.chunks_exact_mut(2)) {
            let ([left_in, right_in], [left_out, right_out]) = (frame_in, frame_out) else {
                continue;
            };
            let delay = state.delay.step();
            let feedback = state.feedback.step() as f32;
            let cross = state.cross.step() as f32;
            let wet = state.wet.step() as f32;

            // One behind: the line has not been pushed this frame yet, so a
            // repeat of `delay` frames reads `delay - 1` back. That is what
            // makes the loop exactly as long as the row asks for, and what
            // puts the first repeat on the sample the sanity test looks at.
            let read = delay - 1.0;
            let left_tap = state.line[0].read_cubic(read);
            let right_tap = state.line[1].read_cubic(read);

            // The low pass sits in the feedback rather than on the output, so
            // the first repeat is the sound itself and only what goes round
            // again is dulled.
            let left_back = state.damping[0].process(left_tap);
            let right_back = state.damping[1].process(right_tap);
            let left_fed = left_back * (1.0 - cross) + right_back * cross;
            let right_fed = right_back * (1.0 - cross) + left_back * cross;
            state.line[0].push(*left_in + left_fed * feedback);
            state.line[1].push(*right_in + right_fed * feedback);

            *left_out = *left_in * (1.0 - wet) + left_tap * wet;
            *right_out = *right_in * (1.0 - wet) + right_tap * wet;
        }
        true
    }

    /// Nought: the repeat comes out later than the sound, which is the effect
    /// rather than a delay to compensate for.
    fn latency(&self) -> u32 {
        0
    }

    fn tail(&self) -> u32 {
        self.tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::effects::audio::modulation::harness;
    use crate::fx::effects::audio::space::{noise, peak, play, play_all};
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES};

    const RATE: u32 = 48_000;

    /// Plan 1's third clause: the repeat lands the same number of
    /// milliseconds behind the sound whatever the bake rate is. The line is
    /// sized and read in frames, so this is where a rate that never arrived
    /// would show.
    #[test]
    fn the_repeat_lands_at_the_same_time_at_any_bake_rate() {
        harness::same_seconds_at_any_rate(
            &AudioDelayDef,
            &[("time", 100.0), ("feedback", 60.0), ("wet", 100.0)],
            1_000.0,
            0.6,
            0.02,
        );
    }

    /// A full set of rows, so a test says only what it cares about.
    fn rows(time: f64, feedback: f64, cross: f64, wet: f64) -> Vec<(ParamId, f64)> {
        vec![
            (AudioDelay::TIME, time),
            (AudioDelay::NOTE, 2.0),
            (AudioDelay::SYNC, 0.0),
            (AudioDelay::FEEDBACK, feedback),
            (AudioDelay::DAMPING, 20_000.0),
            (AudioDelay::PING_PONG, cross),
            (AudioDelay::WET, wet),
        ]
    }

    fn open(values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        AudioDelayDef
            .open_audio(None, values, RATE, false)
            .unwrap_or_else(|| Arc::new(AudioDelayProcessor::new(values, f64::from(RATE))))
    }

    #[test]
    fn the_same_input_twice_is_bit_identical() {
        let values = rows(37.0, 60.0, 40.0, 50.0);
        let input = noise(AUDIO_BLOCK_FRAMES * 12);
        let first = play_all(open(&values).as_ref(), &input, &values);
        let second = play_all(open(&values).as_ref(), &input, &values);
        assert_eq!(first, second);
    }

    #[test]
    fn one_run_and_two_runs_split_at_a_block_edge_agree() {
        let values = rows(37.0, 60.0, 40.0, 50.0);
        let input = noise(AUDIO_BLOCK_FRAMES * 12);
        let whole = play_all(open(&values).as_ref(), &input, &values);

        let carried = open(&values);
        let mut split = vec![0.0; input.len()];
        play(carried.as_ref(), &input, &values, 0..5, &mut split);
        play(carried.as_ref(), &input, &values, 5..12, &mut split);
        assert_eq!(whole, split);
    }

    /// Plan 3: the first repeat arrives at the time asked.
    #[test]
    fn the_first_repeat_lands_at_the_time_asked() {
        // A hundred milliseconds at 48 kHz is 4,800 frames exactly, and no
        // feedback means one repeat to look at.
        let values = rows(100.0, 0.0, 0.0, 100.0);
        let mut input = vec![0.0f32; AUDIO_BLOCK_SAMPLES * 12];
        if let Some(frame) = input.get_mut(0..2) {
            frame.fill(1.0);
        }
        let out = play_all(open(&values).as_ref(), &input, &values);
        let at =
            out.chunks_exact(2)
                .enumerate()
                .fold((0usize, 0.0f32), |(top_at, top), (n, frame)| {
                    let level = frame.first().copied().unwrap_or(0.0).abs();
                    if level > top {
                        (n, level)
                    } else {
                        (top_at, top)
                    }
                });
        assert_eq!(at.0, 4_800, "the repeat landed at frame {}", at.0);
        assert!((at.1 - 1.0).abs() < 1e-6, "and at full size, not {}", at.1);
    }

    /// Plan 2's half of the same idea: the tail is long enough for the
    /// repeats to reach the silence floor, and the mix is told about it.
    #[test]
    fn the_tail_covers_the_repeats_and_ping_pong_crosses_channels() {
        let values = rows(20.0, 80.0, 100.0, 100.0);
        let processor = open(&values);
        // Twenty milliseconds at 80 per cent: about 960 frames a pass, and
        // the repeats fall 1.94 dB each time.
        let tail = processor.tail();
        assert!(
            (44_000..=52_000).contains(&tail),
            "a tail of {tail} frames is not the repeats' own length"
        );

        let frames = tail as usize + AUDIO_BLOCK_FRAMES;
        let blocks = frames.div_ceil(AUDIO_BLOCK_FRAMES);
        let mut input = vec![0.0f32; blocks * AUDIO_BLOCK_SAMPLES];
        // Only the left channel is struck, so anything heard on the right is
        // the cross-feed and nothing else.
        if let Some(slot) = input.first_mut() {
            *slot = 1.0;
        }
        let out = play_all(processor.as_ref(), &input, &values);
        let right: Vec<f32> = out
            .chunks_exact(2)
            .map(|frame| frame.get(1).copied().unwrap_or(0.0))
            .collect();
        assert!(peak(&right) > 0.5, "ping-pong never crossed");

        // Past the tail there is nothing left to hear.
        let after = peak(out.get(tail as usize * 2..).unwrap_or(&[]));
        assert!(after < 1e-4, "the repeats were still going at {after}");
    }
}

use std::sync::{Arc, Mutex};

use super::super::dsp::biquad::{Biquad, Coeffs};
use super::super::dsp::db::{db_of_gain, SILENCE_FLOOR_DB};
use super::super::dsp::delay_line::DelayLine;
use super::super::dsp::smoother::Smoother;
use super::{aim, held, value_of};
use crate::fx::{
    audio_chain::AudioProcessor, params::ParamId, registry::EffectDef, registry::EffectMetadata,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// The longest repeat the Time row offers, in milliseconds. The line is taken
/// at this length whatever the row opens at, because a driver may sweep the
/// row up mid-bake and a line cannot grow in `process`.
const MAX_TIME_MS: f64 = 2_000.0;

const TAIL_CEILING_S: f64 = 30.0;

/// How fast the repeat time follows the row. Slow enough that dragging Time
/// glides rather than jumps, which is what a delay is expected to do.
const TIME_GLIDE_MS: f64 = 60.0;

const GAIN_GLIDE_MS: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_delay",
    label = "Delay",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AudioDelay {
    /// How far behind the sound the first repeat lands, in milliseconds.
    #[bounded(min = 1.0, max = 2_000.0, default = 250.0, unit = Raw)]
    pub time: f32,
    #[counter(
        min = 0,
        max = 6,
        default = 2,
        hard_min = 0,
        hard_max = 6,
        unit = Raw
    )]
    pub note: i32,
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

struct DelayState {
    line: [DelayLine; 2],
    damping: [Biquad; 2],
    /// The repeat, in samples.
    delay: Smoother,
    feedback: Smoother,
    /// How much of the return crosses channels, 0 to 1.
    cross: Smoother,
    wet: Smoother,
    damping_hz: f64,
    primed: bool,
}

pub struct AudioDelayProcessor {
    rate: f64,
    tail: u32,
    state: Mutex<DelayState>,
}

impl AudioDelayProcessor {
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

    use crate::fx::effects::audio::space::{noise, play, play_all};
    use crate::fx::{audio_chain::AUDIO_BLOCK_FRAMES, audio_chain::AUDIO_BLOCK_SAMPLES};

    const RATE: u32 = 48_000;

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

    #[test]
    fn the_first_repeat_lands_at_the_time_asked() {
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
}

//! The limiter: a ceiling the sound is not allowed through
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A limiter is a compressor that is never allowed to be late. It holds the
//! sound back so that nothing comes out louder than the ceiling, and it does
//! that by hearing the loud part before you do: the sound goes into a delay
//! line while the detector reads it undelayed, so by the time a peak reaches
//! the output the gain is already down for it. Input drives the sound into the
//! ceiling, and Release is how quickly it lets go afterwards.
//!
//! **Why it cannot overshoot.** Whenever the detector wants any reduction at
//! all, the gain is barred from rising for a whole lookahead, and it is
//! allowed to fall by the whole range in that time. So the gain applied to a
//! sample is never above what that sample's own detector reading asked for,
//! whatever the sound did in between. The fall is a straight ramp rather than
//! a step, which is what keeps the reduction from being heard as a click of
//! its own.
//!
//! **True peak** measures between the samples. A run of samples that all sit
//! under the ceiling can still describe a wave that goes over it, and a
//! converter or an encoder downstream will make that wave. The oversampler in
//! [`super::super::dsp::oversample`] is what hears it: four samples where
//! there was one, and the loudest of them is the level. It costs the
//! oversampler's own delay, which is added to the lookahead so the sound and
//! the reading still line up.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::{frames_of_ms, row};
use crate::fx::effects::audio::dsp::{
    db::gain_of_db, delay_line::DelayLine, oversample::Oversample4x, smoother::coeff_of_ms,
    smoother::Smoother,
};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// The frames the true-peak detector hears late: the upward half of the
/// oversampler's pair, which is half of the pair's own delay because both
/// halves are the same linear-phase filter.
const DETECT_FRAMES: u32 = Oversample4x::LATENCY_FRAMES / 2;

/// How fast the input gain follows its row. It multiplies the signal straight
/// away, so a hand on it would step once a block and be heard.
const SMOOTH_MS: f64 = 5.0;

// The declared defaults, repeated as the fallback a missing row takes.
const CEILING_DB: f64 = -0.3;
const INPUT_DB: f64 = 0.0;
const LOOKAHEAD_MS: f64 = 5.0;
const RELEASE_MS: f64 = 100.0;
const TRUE_PEAK_OFF: f64 = 0.0;

/// The Limiter's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_limiter",
    label = "Limiter",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, and no picture at all: no matte row, and nothing to draw
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioLimiter {
    /// The loudest the output may be, in dB.
    #[bounded(min = -24.0, max = 0.0, default = CEILING_DB, unit = Raw)]
    pub ceiling: f32,
    /// Gain into the limiter, in dB: what drives the sound up against the
    /// ceiling.
    #[bounded(min = -12.0, max = 24.0, default = INPUT_DB, unit = Raw)]
    pub input: f32,
    /// How far ahead it hears, in milliseconds. The chain places the sound
    /// this much earlier, so it is free.
    #[bounded(min = 0.0, max = 20.0, default = LOOKAHEAD_MS, unit = Raw)]
    pub lookahead: f32,
    /// How fast it lets go once the loud part has gone, in milliseconds.
    #[bounded(min = 1.0, max = 1000.0, default = RELEASE_MS, unit = Raw)]
    pub release: f32,
    /// 0 off, 1 on: measure between the samples, at four times the rate.
    #[counter(min = 0, max = 1, default = 0, hard_min = 0, hard_max = 1, unit = Raw)]
    pub true_peak: i32,
}

/// The Limiter's behaviour: one live instance per bake.
pub struct AudioLimiterDef;

impl EffectDef for AudioLimiterDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioLimiter as EffectMetadata>::SCHEMA
    }

    /// It processes sound and touches no pixel, so the render path skips it.
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
        Some(Arc::new(Limiter::new(values, f64::from(rate))))
    }
}

/// One open limiter.
struct Limiter {
    /// Frames of lookahead: how long the gain has to arrive, and how long it
    /// is barred from rising after any reduction.
    lookahead: u32,
    /// Frames the sound is held back by, which is the lookahead plus whatever
    /// the true-peak detector hears late. Both rows are read once, at open,
    /// because the chain asks for [`AudioProcessor::latency`] once a run.
    delay: u32,
    /// Behind a mutex because [`AudioProcessor::process`] takes `&self`.
    /// Uncontended by construction: one instance is driven by one bake.
    state: Mutex<State>,
}

/// Everything the block loop carries from one block to the next.
struct State {
    rate: f64,
    left: DelayLine,
    right: DelayLine,
    /// One oversampler a channel, present only when true peak is on.
    up: Option<[Oversample4x; 2]>,
    drive: Smoother,
    /// The gain the release is climbing back from. It never rises while the
    /// hold runs, which is what makes the ceiling a promise.
    floor: f64,
    /// The gain applied to the sample going out now.
    gain: f64,
    /// Frames left of the bar on rising.
    hold: u32,
}

impl Limiter {
    fn new(values: &[(ParamId, f64)], rate: f64) -> Self {
        let lookahead = frames_of_ms(row(values, AudioLimiter::LOOKAHEAD, LOOKAHEAD_MS), rate);
        let true_peak = row(values, AudioLimiter::TRUE_PEAK, TRUE_PEAK_OFF) >= 0.5;
        let delay = lookahead.saturating_add(if true_peak { DETECT_FRAMES } else { 0 });
        Self {
            lookahead,
            delay,
            state: Mutex::new(State {
                rate,
                left: DelayLine::new(delay as usize),
                right: DelayLine::new(delay as usize),
                up: true_peak.then(|| [Oversample4x::new(), Oversample4x::new()]),
                drive: Smoother::new(
                    SMOOTH_MS,
                    rate,
                    gain_of_db(row(values, AudioLimiter::INPUT, INPUT_DB)),
                ),
                floor: 1.0,
                gain: 1.0,
                hold: 0,
            }),
        }
    }
}

impl AudioProcessor for Limiter {
    fn process(
        &self,
        input: &[f32],
        output: &mut [f32],
        values: &[(ParamId, f64)],
        _steady: i64,
    ) -> bool {
        let Ok(mut held) = self.state.lock() else {
            return false;
        };
        let st = &mut *held;

        // The block's numbers, read once at its start
        // (docs/impl/audio-plugins.md §3). A ceiling moved by hand takes a
        // lookahead to bite, which is the same lateness every row here has.
        let ceiling = gain_of_db(row(values, AudioLimiter::CEILING, CEILING_DB));
        st.drive
            .set_target(gain_of_db(row(values, AudioLimiter::INPUT, INPUT_DB)));
        let release = coeff_of_ms(row(values, AudioLimiter::RELEASE, RELEASE_MS), st.rate);
        // The most the gain may fall in one frame, so it always arrives inside
        // the lookahead. With no lookahead there is no time to ramp over.
        let step = if self.lookahead == 0 {
            1.0
        } else {
            1.0 / f64::from(self.lookahead)
        };
        let delay = f64::from(self.delay);

        for (came, goes) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let drive = st.drive.step() as f32;
            let l = came.first().copied().unwrap_or(0.0) * drive;
            let r = came.get(1).copied().unwrap_or(0.0) * drive;
            st.left.push(l);
            st.right.push(r);

            // How loud this frame is: the louder channel, or the loudest of
            // the eight samples the oversampler makes of the pair.
            let peak = match &mut st.up {
                Some([ul, ur]) => {
                    let a = ul.up(l);
                    let b = ur.up(r);
                    a.iter()
                        .chain(b.iter())
                        .fold(0.0f32, |most, s| most.max(s.abs()))
                }
                None => l.abs().max(r.abs()),
            };
            let peak = f64::from(peak);
            let want = if peak > ceiling { ceiling / peak } else { 1.0 };

            if want < 1.0 {
                st.hold = self.lookahead;
            }
            st.floor = st.floor.min(want);
            st.gain = if st.gain > st.floor {
                (st.gain - step).max(st.floor)
            } else {
                st.floor
            };
            let gain = st.gain as f32;
            if let Some(slot) = goes.first_mut() {
                *slot = st.left.read_linear(delay) * gain;
            }
            if let Some(slot) = goes.get_mut(1) {
                *slot = st.right.read_linear(delay) * gain;
            }

            // Let go, for the next frame. Held first, so a peak seen now keeps
            // the gain down for every frame up to the one it lands in.
            if st.hold > 0 {
                st.hold -= 1;
            } else {
                st.floor = 1.0 + (st.floor - 1.0) * release;
            }
        }
        true
    }

    fn latency(&self) -> u32 {
        self.delay
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES};
    use std::f64::consts::TAU;

    const RATE: u32 = 48_000;

    /// A stereo tone, the right channel a little quieter than the left.
    fn tone(frames: usize, freq: f64, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let s = (TAU * freq * n as f64 / f64::from(RATE)).sin() as f32 * amp;
                [s, s * 0.8]
            })
            .collect()
    }

    /// A quarter-rate tone landing half way between its own peaks: every
    /// sample reads 0.707 of `amp`, and the wave they describe reaches `amp`.
    /// This is the sound sample peak cannot see and true peak can.
    fn between_samples(frames: usize, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let s = (TAU * n as f64 / 4.0 + TAU / 8.0).sin() as f32 * amp;
                [s, s]
            })
            .collect()
    }

    fn open(values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        AudioLimiterDef
            .open_audio(None, values, RATE, false)
            .unwrap()
    }

    /// Every whole block of `input`, from block `first`, through `p`.
    fn run(
        p: &dyn AudioProcessor,
        input: &[f32],
        values: &[(ParamId, f64)],
        first: usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; input.len()];
        for (b, (came, goes)) in input
            .chunks_exact(AUDIO_BLOCK_SAMPLES)
            .zip(out.chunks_exact_mut(AUDIO_BLOCK_SAMPLES))
            .enumerate()
        {
            let steady = ((first + b) * AUDIO_BLOCK_FRAMES) as i64;
            assert!(p.process(came, goes, values, steady));
        }
        out
    }

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|s| s.to_bits()).collect()
    }

    fn knobs(ceiling: f64, true_peak: f64) -> Vec<(ParamId, f64)> {
        vec![
            (AudioLimiter::CEILING, ceiling),
            (AudioLimiter::INPUT, 6.0),
            (AudioLimiter::LOOKAHEAD, 2.0),
            (AudioLimiter::RELEASE, 50.0),
            (AudioLimiter::TRUE_PEAK, true_peak),
        ]
    }

    fn loudest(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |most, s| most.max(s.abs()))
    }

    #[test]
    fn the_same_sound_twice_is_bit_identical() {
        let input = tone(AUDIO_BLOCK_FRAMES * 4, 220.0, 0.9);
        let values = knobs(-6.0, 1.0);
        let first = run(&*open(&values), &input, &values, 0);
        let again = run(&*open(&values), &input, &values, 0);
        assert_eq!(bits(&first), bits(&again));
    }

    #[test]
    fn a_run_split_at_a_block_edge_carries_its_state_across() {
        let input = tone(AUDIO_BLOCK_FRAMES * 4, 220.0, 0.9);
        let values = knobs(-6.0, 1.0);
        let whole = run(&*open(&values), &input, &values, 0);

        let split = AUDIO_BLOCK_SAMPLES * 2;
        let carried = open(&values);
        let mut halves = run(&*carried, &input[..split], &values, 0);
        halves.extend(run(&*carried, &input[split..], &values, 2));
        assert_eq!(bits(&whole), bits(&halves));
    }

    /// **Plan 3**: nothing comes out above the ceiling, true peak on or off.
    #[test]
    fn nothing_comes_out_above_the_ceiling() {
        let ceiling = gain_of_db(-6.0);
        let input = tone(AUDIO_BLOCK_FRAMES * 8, 220.0, 1.0);
        for true_peak in [0.0, 1.0] {
            let values = knobs(-6.0, true_peak);
            let out = run(&*open(&values), &input, &values, 0);
            let peak = f64::from(loudest(&out));
            assert!(
                peak <= ceiling + 1e-6,
                "true peak {true_peak}: {peak} went past {ceiling}"
            );
            assert!(
                peak > ceiling * 0.5,
                "true peak {true_peak}: nothing came out"
            );
        }
    }

    /// **Plan 3**, the reason true peak exists: a wave whose peaks fall
    /// between the samples is held down further than the samples alone would
    /// ask for.
    #[test]
    fn true_peak_hears_what_is_between_the_samples() {
        let input = between_samples(AUDIO_BLOCK_FRAMES * 8, 1.0);
        let off = run(&*open(&knobs(-6.0, 0.0)), &input, &knobs(-6.0, 0.0), 0);
        let on = run(&*open(&knobs(-6.0, 1.0)), &input, &knobs(-6.0, 1.0), 0);
        let (off, on) = (loudest(&off), loudest(&on));
        assert!(
            on < off * 0.8,
            "true peak came out at {on} where sample peak came out at {off}"
        );
    }

    /// The delay the sound is held back by is what the chain places it earlier
    /// by, so it has to be told: the lookahead, and the oversampler's own
    /// delay when true peak is on.
    #[test]
    fn the_reported_latency_covers_the_lookahead_and_the_detector() {
        assert_eq!(open(&knobs(-6.0, 0.0)).latency(), 96);
        assert_eq!(open(&knobs(-6.0, 1.0)).latency(), 96 + DETECT_FRAMES);
    }
}

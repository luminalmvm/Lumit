//! The compressor: turn down whatever is louder than the threshold
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A compressor listens to how loud the sound is and turns it down once it
//! goes past the threshold. The ratio is how much of what goes past comes
//! back, the knee is how gently that starts, and the attack and release are
//! how fast the gain moves. Makeup puts back what the compression took, and
//! Wet mixes the untouched sound in beside it, which is parallel compression.
//!
//! The detector hears the louder of the two channels, so a stereo sound is
//! turned down as one thing rather than drifting to the quiet side. Lookahead
//! delays the sound the detector has already heard, so the reduction is in
//! place before the loud part arrives; the chain places the sound that much
//! earlier, which is why it costs nothing.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::{frames_of_ms, row};
use crate::fx::effects::audio::dsp::{
    db::{db_of_gain, gain_of_db},
    delay_line::DelayLine,
    envelope::{GainReduction, RmsFollower},
    smoother::Smoother,
};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

/// The window the RMS detector averages over. Fixed rather than a row: RMS
/// means "how loud over about ten milliseconds", and a second time constant
/// beside the attack and release would be two knobs for one idea.
const RMS_WINDOW_MS: f64 = 10.0;

/// How fast the makeup and the wet mix follow their rows. Both multiply the
/// signal straight away, so a hand on either would step once a block and be
/// heard as a click.
const SMOOTH_MS: f64 = 5.0;

// The declared defaults, repeated as the fallback a missing row takes.
const THRESHOLD_DB: f64 = -18.0;
const RATIO_TO_ONE: f64 = 4.0;
const KNEE_DB: f64 = 6.0;
const ATTACK_MS: f64 = 10.0;
const RELEASE_MS: f64 = 100.0;
const MAKEUP_DB: f64 = 0.0;
const PEAK_DETECTOR: f64 = 0.0;
const LOOKAHEAD_MS: f64 = 0.0;
const WET_PCT: f64 = 100.0;

/// The Compressor's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_compressor",
    label = "Compressor",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, and no picture at all: no matte row, and nothing to draw
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioCompressor {
    /// Where the compression starts, in dB.
    #[bounded(min = -60.0, max = 0.0, default = THRESHOLD_DB, unit = Raw)]
    pub threshold: f32,
    /// How much of what goes past the threshold comes back, as n to 1.
    #[bounded(min = 1.0, max = 20.0, default = RATIO_TO_ONE, unit = Raw)]
    pub ratio: f32,
    /// The dB either side of the threshold the curve bends over, so a sound
    /// arriving at the threshold is not compressed all at once.
    #[bounded(min = 0.0, max = 24.0, default = KNEE_DB, unit = Raw)]
    pub knee: f32,
    /// How fast the reduction deepens, in milliseconds.
    #[bounded(min = 0.1, max = 100.0, default = ATTACK_MS, unit = Raw)]
    pub attack: f32,
    /// How fast it lets go again, in milliseconds.
    #[bounded(min = 5.0, max = 1000.0, default = RELEASE_MS, unit = Raw)]
    pub release: f32,
    /// Gain put back after the compression, in dB.
    #[bounded(min = 0.0, max = 24.0, default = MAKEUP_DB, unit = Raw)]
    pub makeup: f32,
    /// What the detector listens to: 0 peak, 1 RMS.
    #[counter(min = 0, max = 1, default = 0, hard_min = 0, hard_max = 1, unit = Raw)]
    pub detector: i32,
    /// How far ahead the detector hears, in milliseconds.
    #[bounded(min = 0.0, max = 20.0, default = LOOKAHEAD_MS, unit = Raw)]
    pub lookahead: f32,
    /// How much of the compressed sound is in the output, per cent. Below 100
    /// the untouched sound is mixed in beside it.
    #[bounded(min = 0.0, max = 100.0, default = WET_PCT, unit = Percent)]
    pub wet: f32,
}

/// The Compressor's behaviour: one live instance per bake.
pub struct AudioCompressorDef;

impl EffectDef for AudioCompressorDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioCompressor as EffectMetadata>::SCHEMA
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
        Some(Arc::new(Compressor::new(values, f64::from(rate))))
    }
}

/// One open compressor: the lookahead it was built for, and everything that
/// changes as it runs.
struct Compressor {
    /// Frames of lookahead, fixed for the run. The chain reads
    /// [`AudioProcessor::latency`] once, so a row moved by hand mid-run cannot
    /// be allowed to change it.
    lookahead: u32,
    /// Behind a mutex because [`AudioProcessor::process`] takes `&self`.
    /// Uncontended by construction: one instance is driven by one bake.
    state: Mutex<State>,
}

/// Everything the block loop carries from one block to the next.
struct State {
    rate: f64,
    left: DelayLine,
    right: DelayLine,
    rms: RmsFollower,
    reduction: GainReduction,
    makeup: Smoother,
    wet: Smoother,
}

impl Compressor {
    fn new(values: &[(ParamId, f64)], rate: f64) -> Self {
        let lookahead = frames_of_ms(row(values, AudioCompressor::LOOKAHEAD, LOOKAHEAD_MS), rate);
        let held = lookahead as usize;
        Self {
            lookahead,
            state: Mutex::new(State {
                rate,
                left: DelayLine::new(held),
                right: DelayLine::new(held),
                rms: RmsFollower::new(RMS_WINDOW_MS, RMS_WINDOW_MS, rate),
                reduction: GainReduction::new(
                    row(values, AudioCompressor::ATTACK, ATTACK_MS),
                    row(values, AudioCompressor::RELEASE, RELEASE_MS),
                    rate,
                ),
                // Snapped to the first block's values, not ramped up to them:
                // a bake must not open with a fade.
                makeup: Smoother::new(
                    SMOOTH_MS,
                    rate,
                    row(values, AudioCompressor::MAKEUP, MAKEUP_DB),
                ),
                wet: Smoother::new(
                    SMOOTH_MS,
                    rate,
                    row(values, AudioCompressor::WET, WET_PCT) / 100.0,
                ),
            }),
        }
    }
}

/// The reduction, in dB, the soft-knee curve asks for at a level `over` dB
/// past the threshold. Never above nought: a compressor turns down, and one
/// that let go into a boost would be a compressor and a fader at once.
fn curve(over: f64, ratio: f64, knee: f64) -> f64 {
    if knee > 0.0 && 2.0 * over.abs() <= knee {
        // Inside the knee: a quadratic that leaves the flat part and joins the
        // slope with the same tangent, so the turn is not heard as a corner.
        let x = over + knee / 2.0;
        (1.0 / ratio - 1.0) * x * x / (2.0 * knee)
    } else if over > 0.0 {
        over / ratio - over
    } else {
        0.0
    }
}

impl AudioProcessor for Compressor {
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
        // (docs/impl/audio-plugins.md §3).
        let threshold = row(values, AudioCompressor::THRESHOLD, THRESHOLD_DB);
        let ratio = row(values, AudioCompressor::RATIO, RATIO_TO_ONE).max(1.0);
        let knee = row(values, AudioCompressor::KNEE, KNEE_DB).max(0.0);
        let rms = row(values, AudioCompressor::DETECTOR, PEAK_DETECTOR) >= 0.5;
        st.reduction.set_times(
            row(values, AudioCompressor::ATTACK, ATTACK_MS),
            row(values, AudioCompressor::RELEASE, RELEASE_MS),
            st.rate,
        );
        st.makeup
            .set_target(row(values, AudioCompressor::MAKEUP, MAKEUP_DB));
        st.wet
            .set_target(row(values, AudioCompressor::WET, WET_PCT) / 100.0);
        let delay = f64::from(self.lookahead);

        for (came, goes) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let l = came.first().copied().unwrap_or(0.0);
            let r = came.get(1).copied().unwrap_or(l);
            st.left.push(l);
            st.right.push(r);

            // The louder channel, so a stereo sound is turned down as one.
            let louder = l.abs().max(r.abs());
            let level = if rms {
                st.rms.process(louder)
            } else {
                f64::from(louder)
            };
            let want = curve(db_of_gain(level) - threshold, ratio, knee);
            let gain = gain_of_db(st.reduction.process(want) + st.makeup.step());

            // Parallel: the dry sound is the delayed one, so the two halves
            // are the same moment and cannot phase against each other.
            let wet = st.wet.step();
            let mix = (1.0 - wet + wet * gain) as f32;
            if let Some(slot) = goes.first_mut() {
                *slot = st.left.read_linear(delay) * mix;
            }
            if let Some(slot) = goes.get_mut(1) {
                *slot = st.right.read_linear(delay) * mix;
            }
        }
        true
    }

    fn latency(&self) -> u32 {
        self.lookahead
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

    fn open(values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        AudioCompressorDef
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

    fn knobs(lookahead: f64) -> Vec<(ParamId, f64)> {
        vec![
            (AudioCompressor::THRESHOLD, -18.0),
            (AudioCompressor::RATIO, 4.0),
            (AudioCompressor::KNEE, 6.0),
            (AudioCompressor::ATTACK, 1.0),
            (AudioCompressor::RELEASE, 50.0),
            (AudioCompressor::MAKEUP, 0.0),
            (AudioCompressor::DETECTOR, 0.0),
            (AudioCompressor::LOOKAHEAD, lookahead),
            (AudioCompressor::WET, 100.0),
        ]
    }

    /// The loudest sample of the last block, which is the one the ballistics
    /// have settled in.
    fn settled_peak(samples: &[f32]) -> f32 {
        samples
            .chunks_exact(AUDIO_BLOCK_SAMPLES)
            .last()
            .map_or(0.0, |last| last.iter().fold(0.0f32, |m, s| m.max(s.abs())))
    }

    #[test]
    fn the_same_sound_twice_is_bit_identical() {
        let input = tone(AUDIO_BLOCK_FRAMES * 4, 220.0, 0.9);
        let values = knobs(2.0);
        let first = run(&*open(&values), &input, &values, 0);
        let again = run(&*open(&values), &input, &values, 0);
        assert_eq!(bits(&first), bits(&again));
    }

    #[test]
    fn a_run_split_at_a_block_edge_carries_its_state_across() {
        let input = tone(AUDIO_BLOCK_FRAMES * 4, 220.0, 0.9);
        let values = knobs(2.0);
        let whole = run(&*open(&values), &input, &values, 0);

        let split = AUDIO_BLOCK_SAMPLES * 2;
        let carried = open(&values);
        let mut halves = run(&*carried, &input[..split], &values, 0);
        halves.extend(run(&*carried, &input[split..], &values, 2));
        assert_eq!(bits(&whole), bits(&halves));
    }

    /// **Plan 3**: a loud burst comes back quieter, and a quiet one is not
    /// touched at all.
    #[test]
    fn it_holds_a_loud_sound_down_and_leaves_a_quiet_one_alone() {
        let values = knobs(0.0);

        let loud = tone(AUDIO_BLOCK_FRAMES * 8, 220.0, 1.0);
        let out = run(&*open(&values), &loud, &values, 0);
        let peak = settled_peak(&out);
        // Four to one over a threshold of -18 dB puts full scale at about
        // -13.5 dB, which is a fifth of the way up rather than all of it.
        assert!(
            (0.1..0.4).contains(&peak),
            "a full scale tone came back at {peak}"
        );

        // Twenty two dB under the threshold: nothing to do, and the arithmetic
        // says so exactly rather than nearly.
        let quiet = tone(AUDIO_BLOCK_FRAMES * 4, 220.0, 0.01);
        let out = run(&*open(&values), &quiet, &values, 0);
        assert_eq!(bits(&out), bits(&quiet));
    }

    /// Lookahead is what the chain places the sound earlier by, so it has to
    /// be told, and it has to be the row the effect was opened with.
    #[test]
    fn the_lookahead_row_is_the_latency_it_reports() {
        let values = knobs(5.0);
        assert_eq!(open(&values).latency(), 240);
        assert_eq!(open(&knobs(0.0)).latency(), 0);
    }
}

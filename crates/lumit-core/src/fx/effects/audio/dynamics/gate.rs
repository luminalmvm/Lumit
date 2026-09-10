//! The gate: turn down whatever is quieter than the threshold
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A gate is the compressor turned round. It listens to how loud the sound is
//! and turns it down once it falls *under* the threshold, which is how the
//! hiss between the words goes away while the words stay. Range is how far
//! down it goes, and Ratio is how sharply it gets there: at the top of its
//! travel the gate shuts, and lower down it is an expander that only leans on
//! the quiet parts.
//!
//! **Why hysteresis and hold.** A level sitting on the threshold would open
//! and shut on every wobble, and the chatter is far more audible than the
//! noise. So the gate opens at the threshold and shuts a few dB below it, and
//! once open it stays open for the hold before it is allowed to start
//! shutting. The hold is also what carries the gate over a wave's own zero
//! crossings, which is why its default is longer than the slowest note's half
//! period.
//!
//! Nothing is delayed and nothing rings on, so the gate reports no latency and
//! no tail.

use std::sync::{Arc, Mutex};

use lumit_fx_macros::Effect;

use super::{frames_of_ms, row};
use crate::fx::effects::audio::dsp::{
    db::{db_of_gain, gain_of_db},
    envelope::GainReduction,
};
use crate::fx::{AudioProcessor, EffectDef, EffectMetadata, EffectSchema, ParamId, AUDIO_CHANNELS};

// The declared defaults, repeated as the fallback a missing row takes.
const THRESHOLD_DB: f64 = -40.0;
const RATIO_TO_ONE: f64 = 20.0;
const HYSTERESIS_DB: f64 = 3.0;
const ATTACK_MS: f64 = 1.0;
const HOLD_MS: f64 = 10.0;
const RELEASE_MS: f64 = 100.0;
const RANGE_DB: f64 = -60.0;

/// The Gate's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_gate",
    label = "Gate",
    version = 1,
    category = Audio,
    cost = Trivial,
    roi = Exact,
    // Sound, and no picture at all: no matte row, and nothing to draw
    // (docs/impl/audio-effects.md §2).
    matte = false,
)]
pub struct AudioGate {
    /// The level the gate opens at, in dB.
    #[bounded(min = -60.0, max = 0.0, default = THRESHOLD_DB, unit = Raw)]
    pub threshold: f32,
    /// How steeply the quiet part is turned down, as n to 1. One is off, and
    /// the top of the travel is a gate rather than an expander.
    #[bounded(min = 1.0, max = 20.0, default = RATIO_TO_ONE, unit = Raw)]
    pub ratio: f32,
    /// The dB under the threshold the gate has to fall before it may shut, so
    /// a level sitting on the threshold does not chatter.
    #[bounded(min = 0.0, max = 24.0, default = HYSTERESIS_DB, unit = Raw)]
    pub hysteresis: f32,
    /// How fast it opens once the sound arrives, in milliseconds.
    #[bounded(min = 0.1, max = 100.0, default = ATTACK_MS, unit = Raw)]
    pub attack: f32,
    /// How long it stays open after the level drops, in milliseconds.
    #[bounded(min = 0.0, max = 500.0, default = HOLD_MS, unit = Raw)]
    pub hold: f32,
    /// How fast it shuts once the hold has run out, in milliseconds.
    #[bounded(min = 5.0, max = 1000.0, default = RELEASE_MS, unit = Raw)]
    pub release: f32,
    /// The lowest a shut gate goes, in dB. Nought lets everything through and
    /// the bottom of the travel is silence.
    #[bounded(min = -100.0, max = 0.0, default = RANGE_DB, unit = Raw)]
    pub range: f32,
}

/// The Gate's behaviour: one live instance per bake.
pub struct AudioGateDef;

impl EffectDef for AudioGateDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioGate as EffectMetadata>::SCHEMA
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
        Some(Arc::new(Gate::new(values, f64::from(rate))))
    }
}

/// One open gate.
struct Gate {
    /// Behind a mutex because [`AudioProcessor::process`] takes `&self`.
    /// Uncontended by construction: one instance is driven by one bake.
    state: Mutex<State>,
}

/// Everything the block loop carries from one block to the next.
struct State {
    rate: f64,
    reduction: GainReduction,
    /// Whether the gate is letting the sound through now.
    open: bool,
    /// Frames left of the hold. Counted down only while the level is under the
    /// shutting threshold, which is what the hysteresis buys.
    hold: u32,
}

impl Gate {
    fn new(values: &[(ParamId, f64)], rate: f64) -> Self {
        Self {
            state: Mutex::new(State {
                rate,
                // Attack and release go in the other way round: the follower's
                // attack is the reduction deepening, and for a gate deepening
                // the reduction is the gate shutting.
                reduction: GainReduction::new(
                    row(values, AudioGate::RELEASE, RELEASE_MS),
                    row(values, AudioGate::ATTACK, ATTACK_MS),
                    rate,
                ),
                // Shut, so a bake that opens in silence stays silent rather
                // than passing a block of hiss before it notices.
                open: false,
                hold: 0,
            }),
        }
    }
}

impl AudioProcessor for Gate {
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
        let threshold = row(values, AudioGate::THRESHOLD, THRESHOLD_DB);
        let ratio = row(values, AudioGate::RATIO, RATIO_TO_ONE).max(1.0);
        let shut_at = threshold - row(values, AudioGate::HYSTERESIS, HYSTERESIS_DB).max(0.0);
        let range = row(values, AudioGate::RANGE, RANGE_DB).min(0.0);
        let hold = frames_of_ms(row(values, AudioGate::HOLD, HOLD_MS), st.rate);
        st.reduction.set_times(
            row(values, AudioGate::RELEASE, RELEASE_MS),
            row(values, AudioGate::ATTACK, ATTACK_MS),
            st.rate,
        );

        for (came, goes) in input
            .chunks_exact(AUDIO_CHANNELS)
            .zip(output.chunks_exact_mut(AUDIO_CHANNELS))
        {
            let l = came.first().copied().unwrap_or(0.0);
            let r = came.get(1).copied().unwrap_or(l);

            // The louder channel, so a stereo sound is gated as one thing
            // rather than half of it shutting on its own.
            let level = db_of_gain(f64::from(l.abs().max(r.abs())));
            if level >= threshold {
                st.open = true;
                st.hold = hold;
            } else if st.open && level < shut_at {
                if st.hold > 0 {
                    st.hold -= 1;
                } else {
                    st.open = false;
                }
            }

            // Shut, the expander's slope: every dB under the threshold costs
            // another ratio minus one, down to the range and no further.
            let want = if st.open {
                0.0
            } else {
                ((level - threshold) * (ratio - 1.0)).max(range)
            };
            let gain = gain_of_db(st.reduction.process(want)) as f32;
            if let Some(slot) = goes.first_mut() {
                *slot = l * gain;
            }
            if let Some(slot) = goes.get_mut(1) {
                *slot = r * gain;
            }
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES};
    use std::f64::consts::TAU;

    const RATE: u32 = 48_000;

    /// A steady stereo tone at `amp`.
    fn tone(frames: usize, amp: f32) -> Vec<f32> {
        dipped(frames, amp, 0..0, amp)
    }

    /// The same tone with a quieter stretch in the middle of it, the phase
    /// carried straight through so the join is not a click of its own.
    fn dipped(frames: usize, amp: f32, dip: std::ops::Range<usize>, quiet: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let a = if dip.contains(&n) { quiet } else { amp };
                let s = (TAU * 220.0 * n as f64 / f64::from(RATE)).sin() as f32 * a;
                [s, s * 0.8]
            })
            .collect()
    }

    fn open(values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        AudioGateDef.open_audio(None, values, RATE, false).unwrap()
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

    fn knobs(hold: f64) -> Vec<(ParamId, f64)> {
        vec![
            (AudioGate::THRESHOLD, -40.0),
            (AudioGate::RATIO, 20.0),
            (AudioGate::HYSTERESIS, 3.0),
            (AudioGate::ATTACK, 1.0),
            (AudioGate::HOLD, hold),
            (AudioGate::RELEASE, 5.0),
            (AudioGate::RANGE, -60.0),
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

    /// The loudest of the frames in `frames`, both channels.
    fn loudest_over(samples: &[f32], frames: std::ops::Range<usize>) -> f32 {
        samples
            .get(frames.start * AUDIO_CHANNELS..frames.end * AUDIO_CHANNELS)
            .unwrap_or_default()
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn the_same_sound_twice_is_bit_identical() {
        let input = dipped(AUDIO_BLOCK_FRAMES * 4, 0.5, 1_000..1_500, 0.001);
        let values = knobs(10.0);
        let first = run(&*open(&values), &input, &values, 0);
        let again = run(&*open(&values), &input, &values, 0);
        assert_eq!(bits(&first), bits(&again));
    }

    #[test]
    fn a_run_split_at_a_block_edge_carries_its_state_across() {
        let input = dipped(AUDIO_BLOCK_FRAMES * 4, 0.5, 1_000..1_500, 0.001);
        let values = knobs(10.0);
        let whole = run(&*open(&values), &input, &values, 0);

        let split = AUDIO_BLOCK_SAMPLES * 2;
        let carried = open(&values);
        let mut halves = run(&*carried, &input[..split], &values, 0);
        halves.extend(run(&*carried, &input[split..], &values, 2));
        assert_eq!(bits(&whole), bits(&halves));
    }

    /// **Plan 3**: what is under the threshold is silenced and what is over it
    /// comes back as it went in.
    #[test]
    fn it_silences_what_is_under_the_threshold_and_passes_what_is_over() {
        let values = knobs(10.0);

        // Six dB down, well over a threshold of -40: the gate opens and the
        // tone comes back at its own height.
        let loud = tone(AUDIO_BLOCK_FRAMES * 8, 0.5);
        let peak = settled_peak(&run(&*open(&values), &loud, &values, 0));
        assert!(
            (peak - 0.5).abs() < 1e-3,
            "an open gate came back at {peak}"
        );

        // Sixty dB down, twenty under the threshold: the range floor is what
        // comes out, which is a thousandth of it.
        let quiet = tone(AUDIO_BLOCK_FRAMES * 8, 0.001);
        let peak = settled_peak(&run(&*open(&values), &quiet, &values, 0));
        assert!(peak < 1e-5, "a shut gate let {peak} through");
    }

    /// **Plan 3**, why Hold is a row: a dip shorter than the hold does not
    /// shut the gate, and the same dip with no hold at all does.
    #[test]
    fn a_dip_shorter_than_the_hold_does_not_shut_the_gate() {
        let dip = 2_048..2_560;
        let input = dipped(AUDIO_BLOCK_FRAMES * 8, 0.5, dip.clone(), 0.001);
        // The end of the dip, by which time a gate with no hold has had a
        // release and a half to shut.
        let late = 2_432..dip.end;

        let values = knobs(50.0);
        let with_hold = loudest_over(&run(&*open(&values), &input, &values, 0), late.clone());
        assert!(
            with_hold > 0.0009,
            "the hold let the gate shut, at {with_hold}"
        );

        let values = knobs(0.0);
        let without = loudest_over(&run(&*open(&values), &input, &values, 0), late);
        assert!(
            without < with_hold * 0.1,
            "no hold and the gate stayed open, at {without}"
        );
    }
}

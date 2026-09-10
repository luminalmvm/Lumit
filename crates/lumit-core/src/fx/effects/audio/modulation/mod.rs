//! Modulation: the audio effects an LFO drives
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! A tremolo moves a gain, a vibrato and a chorus move a delay, and a phaser
//! and a wah move a filter. Every one of them carries a
//! [`Phase`](super::dsp::lfo::Phase) in its own state and steps it a frame at
//! a time, so a Rate row that moves changes the speed of the cycle instead of
//! jumping it, and a run split at a block edge with the state carried is the
//! run the whole input would have made (§3).
//!
//! [`gain`] and [`stereo_width`] sit here as well. They are the mixer's two
//! utility effects rather than modulation, and a module of their own holding
//! two files would be a heading with nothing under it.
//!
//! Every effect here is the same four pieces: a `#[derive(Effect)]` struct of
//! rows, a definition whose `open_audio` builds the processor from the first
//! block's values, a processor holding its state behind a mutex because
//! [`AudioProcessor::process`](crate::fx::AudioProcessor::process) takes
//! `&self`, and a block loop that recomputes its coefficients at the block's
//! start and allocates nothing.

pub mod chorus;
pub mod gain;
pub mod phaser;
pub mod stereo_width;
pub mod tremolo;
pub mod vibrato;
pub mod wah;

use super::dsp::lfo::Shape;
use crate::fx::ParamId;

/// What `id` holds in this block's baked values, or `fallback` where the bake
/// sent no number for it.
///
/// A row is missing when the instance never stored one, which is what a
/// project saved before the row existed looks like, so the declared default is
/// the answer rather than a refused block.
pub(crate) fn row(values: &[(ParamId, f64)], id: ParamId, fallback: f64) -> f64 {
    values
        .iter()
        .find(|(held, _)| *held == id)
        .map_or(fallback, |(_, value)| *value)
}

/// The Shape row's three options. Anything else is the sine, because a row
/// pushed past its end by a driver still has to sound like something.
pub(crate) fn shape_of(value: f64) -> Shape {
    match value.round() as i64 {
        1 => Shape::Triangle,
        2 => Shape::Square,
        _ => Shape::Sine,
    }
}

/// The two samples of one interleaved frame, or silence where the block ran
/// short. Reading through this rather than by index is what keeps a block of
/// an odd length out of the panic rules (docs/14 §4).
pub(crate) fn frame(chunk: &[f32]) -> (f32, f32) {
    (
        chunk.first().copied().unwrap_or(0.0),
        chunk.get(1).copied().unwrap_or(0.0),
    )
}

/// One frame written back, a chunk too short to hold it left alone.
pub(crate) fn put(chunk: &mut [f32], left: f32, right: f32) {
    if let Some(slot) = chunk.first_mut() {
        *slot = left;
    }
    if let Some(slot) = chunk.get_mut(1) {
        *slot = right;
    }
}

/// What every effect in this module tests with (docs/impl/audio-effects.md §6
/// plans 1 and 3), written once because seven copies of a block loop is seven
/// places for a harness bug to hide.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod harness {
    use std::sync::Arc;

    use crate::fx::{
        AudioProcessor, EffectDef, ParamId, ParamKind, AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES,
        AUDIO_CHANNELS,
    };

    /// The rate every test here bakes at.
    pub const RATE: u32 = 48_000;

    /// The effect's rows at their declared defaults, with `over` applied by id.
    ///
    /// Only the rows the bake actually sends: it hands a processor the Float
    /// side of the document, which is every Slider, Float and Int row and
    /// nothing else (docs/impl/audio-effects.md §2).
    pub fn values(def: &dyn EffectDef, over: &[(&str, f64)]) -> Vec<(ParamId, f64)> {
        def.schema()
            .params
            .iter()
            .filter_map(|param| {
                let declared = match param.kind {
                    ParamKind::Float { default, .. } | ParamKind::Slider { default, .. } => default,
                    ParamKind::Int { default, .. } => default as f64,
                    _ => return None,
                };
                let held = over
                    .iter()
                    .find(|(id, _)| *id == param.id)
                    .map_or(declared, |(_, value)| *value);
                Some((ParamId::new(param.id), held))
            })
            .collect()
    }

    /// A stereo tone at `hz`, whole blocks of it, at the rate the tests bake
    /// at.
    pub fn tone(blocks: usize, hz: f64) -> Vec<f32> {
        tone_at(RATE, blocks, hz)
    }

    /// The same tone sampled at `rate`. The right channel is quieter and
    /// starts a quarter cycle along, so a channel swapped or summed by
    /// mistake shows up rather than hiding behind a copy of itself.
    pub fn tone_at(rate: u32, blocks: usize, hz: f64) -> Vec<f32> {
        let step = std::f64::consts::TAU * hz / f64::from(rate.max(1));
        (0..blocks * AUDIO_BLOCK_FRAMES)
            .flat_map(|n| {
                let phase = step * n as f64;
                [
                    phase.sin() as f32 * 0.5,
                    (phase + std::f64::consts::FRAC_PI_2).sin() as f32 * 0.3,
                ]
            })
            .collect()
    }

    /// One open instance of the effect.
    pub fn open(def: &dyn EffectDef, values: &[(ParamId, f64)]) -> Arc<dyn AudioProcessor> {
        def.open_audio(None, values, RATE, false)
            .expect("an audio effect opens a processor")
    }

    /// Whole blocks through one processor, the run starting at block `first`.
    pub fn run_from(
        processor: &dyn AudioProcessor,
        input: &[f32],
        values: &[(ParamId, f64)],
        first: usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0; input.len()];
        for (block, (i, o)) in input
            .chunks_exact(AUDIO_BLOCK_SAMPLES)
            .zip(out.chunks_exact_mut(AUDIO_BLOCK_SAMPLES))
            .enumerate()
        {
            let steady = ((first + block) * AUDIO_BLOCK_FRAMES) as i64;
            assert!(
                processor.process(i, o, values, steady),
                "a built-in never refuses a block"
            );
        }
        out
    }

    /// The whole input from the chain's first sample.
    pub fn run(
        processor: &dyn AudioProcessor,
        input: &[f32],
        values: &[(ParamId, f64)],
    ) -> Vec<f32> {
        run_from(processor, input, values, 0)
    }

    /// Plan 1: the same input baked twice is bit-identical, and one run is its
    /// own two halves spliced at a block edge with the state carried.
    pub fn deterministic(def: &dyn EffectDef, over: &[(&str, f64)], input: &[f32]) {
        let values = values(def, over);
        let once = run(&*open(def, &values), input, &values);
        let again = run(&*open(def, &values), input, &values);
        assert_eq!(once, again, "two bakes of one input must agree bit for bit");

        let split = input.len() / 2 / AUDIO_BLOCK_SAMPLES * AUDIO_BLOCK_SAMPLES;
        let carried = open(def, &values);
        let mut halves = run(&*carried, &input[..split], &values);
        halves.extend(run_from(
            &*carried,
            &input[split..],
            &values,
            split / AUDIO_BLOCK_SAMPLES,
        ));
        assert_eq!(once, halves, "a run split at a block edge must match it");
    }

    /// The rates a bake can run at, the preview's own in the middle
    /// (docs/impl/audio-effects.md §6 plan 1).
    const RATES: [u32; 3] = [44_100, 48_000, 96_000];

    /// How long one bucket of a coarse envelope is.
    const BUCKET_MS: f64 = 10.0;

    /// The mean level over each bucket of a run.
    ///
    /// Two runs at different rates hold different numbers of samples, so they
    /// cannot be compared one for one. A bucket is a stretch of time rather
    /// than a count of frames, which is exactly the thing under test.
    fn envelope(samples: &[f32], rate: u32) -> Vec<f64> {
        let frames = (BUCKET_MS * 0.001 * f64::from(rate.max(1))) as usize;
        samples
            .chunks_exact(frames.max(1) * AUDIO_CHANNELS)
            .map(|bucket| {
                let sum: f64 = bucket.iter().map(|s| f64::from(s.abs())).sum();
                sum / bucket.len() as f64
            })
            .collect()
    }

    /// Plan 1's third clause: a bake at 44.1, 48 and 96 kHz makes the same
    /// seconds of sound.
    ///
    /// A burst of tone followed by quiet is run at each rate for the same
    /// `seconds` and the three outputs are compared as envelopes. The quiet
    /// stretch is where a delay's repeats and a reverb's decay show
    /// themselves, and the burst is where a filter's gain does. An effect
    /// holding state in frames rather than in seconds runs at the wrong speed
    /// at two of the three rates, and the buckets part company.
    ///
    /// The first bucket is skipped. A smoothed row's first travel and a
    /// half-band's fill are a fixed number of frames, so they are a different
    /// share of a bucket at each rate, and that much is right.
    pub fn same_seconds_at_any_rate(
        def: &dyn EffectDef,
        over: &[(&str, f64)],
        hz: f64,
        seconds: f64,
        tolerance: f64,
    ) {
        let values = values(def, over);
        let buckets = (seconds * 1_000.0 / BUCKET_MS) as usize;
        let envelopes: Vec<Vec<f64>> = RATES
            .into_iter()
            .map(|rate| {
                let bucket = (BUCKET_MS * 0.001 * f64::from(rate)) as usize;
                let blocks =
                    (seconds * f64::from(rate) / AUDIO_BLOCK_FRAMES as f64).round() as usize;
                let mut input = tone_at(rate, blocks, hz);
                // The tone stops a third of the way in: the same moment at
                // every rate, and on a bucket boundary, or the bucket it
                // stopped in would be a different fraction of tone at each.
                let quiet = buckets / 3 * bucket * AUDIO_CHANNELS;
                if let Some(rest) = input.get_mut(quiet..) {
                    rest.fill(0.0);
                }
                let processor = def
                    .open_audio(None, &values, rate, false)
                    .expect("an audio effect opens a processor");
                envelope(&run(&*processor, &input, &values), rate)
            })
            .collect();

        let reference = &envelopes[1];
        for (run, other) in envelopes.iter().enumerate() {
            for at in 1..reference.len().min(other.len()) {
                let (want, got) = (reference[at], other[at]);
                assert!(
                    (want - got).abs() <= tolerance,
                    "{} Hz: bucket {at} measured {got}, and 48 kHz made {want}",
                    RATES[run]
                );
            }
        }
    }

    /// The mean distance between two windows of `frames` frames of one run.
    fn distance(samples: &[f32], a: usize, b: usize, frames: usize) -> f64 {
        let window = |at: usize| &samples[at * AUDIO_CHANNELS..(at + frames) * AUDIO_CHANNELS];
        let (left, right) = (window(a), window(b));
        let sum: f64 = left
            .iter()
            .zip(right)
            .map(|(x, y)| f64::from(x - y).abs())
            .sum();
        sum / left.len() as f64
    }

    /// Plan 3: the effect modulates at the rate asked.
    ///
    /// A settled cycle of the output is compared with the cycle after it and
    /// with the half cycle between them. One LFO cycle later everything is
    /// where it was, so the two agree; half a cycle later the modulation is at
    /// the other end of its travel, so they do not. The tone's own period
    /// divides the cycle, which is what lets the windows be compared sample by
    /// sample rather than through an envelope detector.
    pub fn modulates_at(def: &dyn EffectDef, over: &[(&str, f64)], rate_hz: f64) {
        let cycle = (f64::from(RATE) / rate_hz) as usize;
        let blocks = (cycle * 3).div_ceil(AUDIO_BLOCK_FRAMES);
        let input = tone(blocks, 1_000.0);
        let values = values(def, over);
        let out = run(&*open(def, &values), &input, &values);

        let over_a_cycle = distance(&out, cycle, cycle * 2, cycle);
        let over_a_half = distance(&out, cycle, cycle + cycle / 2, cycle);
        assert!(
            over_a_half > 1e-3,
            "nothing modulated: half a cycle along measured {over_a_half}"
        );
        assert!(
            over_a_cycle < over_a_half * 0.05,
            "a whole cycle along should repeat: {over_a_cycle} against {over_a_half}"
        );
    }
}

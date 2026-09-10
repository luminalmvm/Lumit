//! Space: the audio effects built on a delay line
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! Three effects out of one idea: sound written into a ring and read back
//! later. The delay reads it back once and feeds it round again, the reverb
//! reads a dozen rings of different lengths until the repeats blur into a
//! room, and the distortion keeps a ring only to hold its dry sound in step
//! with a wet one that came back late.
//!
//! Each is a plain struct behind a mutex, built fresh for every bake, and each
//! keeps the block contract (docs/impl/audio-plugins.md §3): coefficients are
//! worked out at the start of a block from that block's own values, anything
//! that would zipper is smoothed a frame at a time, and nothing allocates once
//! the first block has started.

pub mod delay;
pub mod distortion;
pub mod reverb;

use super::dsp::smoother::Smoother;
use crate::fx::ParamId;

/// What `id` holds this block, or `fallback` where the bake sent no such row.
///
/// A linear walk because a block carries a handful of rows in schema order,
/// and a map would cost more to build than the walk saves.
fn value_of(values: &[(ParamId, f64)], id: ParamId, fallback: f64) -> f64 {
    values
        .iter()
        .find(|(held, _)| *held == id)
        .map_or(fallback, |(_, value)| *value)
}

/// Aim a ramp at this block's value, or put it there outright while the
/// processor is still cold.
///
/// The first block snaps because a bake that opened with a ramp up from
/// nothing would fade its own first tenth of a second in, and the export is
/// meant to be the sound the rows describe from its first sample.
fn aim(smoother: &mut Smoother, target: f64, primed: bool) {
    if primed {
        smoother.set_target(target);
    } else {
        smoother.snap(target);
    }
}

/// Hold `value` between `low` and `high`.
///
/// `max`/`min` rather than `clamp`, which panics on a reversed pair
/// (docs/14-ENGINEERING-RULES §4). Every row is clamped again here even though
/// the bake already holds it to the declared range, because a state blob and a
/// hand-written test both reach a processor without passing through the bake.
fn held(value: f64, low: f64, high: f64) -> f64 {
    value.max(low).min(high)
}

/// Play `input` through `processor` a block at a time, as the chain does.
///
/// `blocks` says which of them to play, so a test can stop halfway and carry
/// the same processor into a second run.
#[cfg(test)]
fn play(
    processor: &dyn crate::fx::AudioProcessor,
    input: &[f32],
    values: &[(ParamId, f64)],
    blocks: std::ops::Range<usize>,
    output: &mut [f32],
) {
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_BLOCK_SAMPLES};

    for block in blocks {
        let at = block * AUDIO_BLOCK_SAMPLES;
        let (Some(src), Some(dst)) = (
            input.get(at..at + AUDIO_BLOCK_SAMPLES),
            output.get_mut(at..at + AUDIO_BLOCK_SAMPLES),
        ) else {
            continue;
        };
        let steady = (block * AUDIO_BLOCK_FRAMES) as i64;
        assert!(
            processor.process(src, dst, values, steady),
            "a built-in never refuses a block"
        );
    }
}

/// The whole of `input`, played through a processor that has seen nothing yet.
#[cfg(test)]
fn play_all(
    processor: &dyn crate::fx::AudioProcessor,
    input: &[f32],
    values: &[(ParamId, f64)],
) -> Vec<f32> {
    let blocks = input.len() / crate::fx::AUDIO_BLOCK_SAMPLES;
    let mut output = vec![0.0; input.len()];
    play(processor, input, values, 0..blocks, &mut output);
    output
}

/// A run of interleaved stereo noise, the same every time.
///
/// A tiny multiply-with-carry rather than a crate: the test wants sound that
/// exercises a filter and repeats exactly, and nothing more.
#[cfg(test)]
fn noise(frames: usize) -> Vec<f32> {
    let mut seed: u32 = 0x1234_5678;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / 8_388_608.0 - 1.0
    };
    (0..frames * 2).map(|_| next() * 0.5).collect()
}

/// A sine at `hz`, in both channels, `frames` long. It and [`bin`] are how the
/// distortion's harmonics are measured.
#[cfg(test)]
fn tone(frames: usize, hz: f64, rate: f64, amplitude: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let phase = std::f64::consts::TAU * hz * n as f64 / rate;
        let sample = phase.sin() as f32 * amplitude;
        out.push(sample);
        out.push(sample);
    }
    out
}

/// How much of `signal`'s left channel sits at `hz`: the magnitude of one
/// discrete Fourier bin, worked out on its own rather than through a
/// transform. The window is a whole number of periods in every use here, so
/// there is nothing to leak.
#[cfg(test)]
fn bin(signal: &[f32], hz: f64, rate: f64) -> f64 {
    let mut real = 0.0;
    let mut imaginary = 0.0;
    for (n, frame) in signal.chunks_exact(2).enumerate() {
        let phase = std::f64::consts::TAU * hz * n as f64 / rate;
        let sample = f64::from(frame.first().copied().unwrap_or(0.0));
        real += sample * phase.cos();
        imaginary -= sample * phase.sin();
    }
    let frames = (signal.len() / 2).max(1) as f64;
    2.0 * (real * real + imaginary * imaginary).sqrt() / frames
}

/// The loudest sample in a run, either channel.
#[cfg(test)]
fn peak(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0f32, |top, s| top.max(s.abs()))
}

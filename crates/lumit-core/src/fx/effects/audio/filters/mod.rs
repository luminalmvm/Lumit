//! Filters: the audio effects that shape the spectrum
//! (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! Both equalisers here are the same idea at two sizes: biquads in a row per
//! channel, their coefficients rebuilt from the block's own values. The two
//! rules they share live in this file, because getting either wrong is heard
//! rather than seen.

pub mod eq;
pub mod graphic_eq;

use crate::fx::ParamId;

/// How often a block rebuilds its coefficients, in frames.
///
/// A baked value only changes once every 512 frames, and a swept frequency
/// that moved in one step a block would be heard as a click. Sixteen steps
/// across a block is under a millisecond apiece at any rate, and each costs
/// one `sin_cos` a band.
const COEFF_FRAMES: usize = 32;

/// How long a row takes to reach a new value. Long enough that a hand on a
/// knob does not zipper, short enough that a keyframed sweep still follows.
const SMOOTH_MS: f64 = 20.0;

/// What `id` holds this block, or `default` where the bake sent nothing for
/// it.
///
/// A project saved before a row existed carries no value for it, and the
/// declared default is what such a row means.
fn value(values: &[(ParamId, f64)], id: ParamId, default: f64) -> f64 {
    values
        .iter()
        .find(|(key, _)| *key == id)
        .map_or(default, |(_, held)| *held)
}

/// How far a dB row may be pushed before a section built from it stops being
/// a section.
///
/// The cookbook raises a gain to a power, so a row driven far past its end
/// arrives at the filter as infinity and every sample after it is infinity
/// too. Twice the widest range either equaliser declares is past anything a
/// hand asks for and short of where the arithmetic gives up.
const DB_LIMIT: f64 = 48.0;

/// What `id` holds this block as decibels, held inside [`DB_LIMIT`].
fn decibels(values: &[(ParamId, f64)], id: ParamId, default: f64) -> f64 {
    // The pair is a constant and its own negation, so there is no reversed
    // pair for `clamp` to refuse (docs/14 §4).
    value(values, id, default).clamp(-DB_LIMIT, DB_LIMIT)
}

/// How far the equaliser moves a tone at `hz`, in dB, with `over` laid over
/// its declared rows (docs/impl/audio-effects.md §6 plan 3).
///
/// Both equalisers are measured the same way, so the tone, the window and the
/// arithmetic are written once here rather than twice beside them.
///
/// The window is the run's second half, which leaves the smoothed
/// coefficients four times their glide to arrive before anything is counted,
/// and it is a whole number of cycles at both frequencies the tests ask for:
/// 3072 frames is four cycles of 62.5 Hz and sixty-four of a kilohertz, so
/// there is nothing to leak into the neighbouring bins.
#[cfg(test)]
fn measured_db(def: &dyn crate::fx::EffectDef, over: &[(&str, f64)], hz: f64) -> f64 {
    use super::dsp::db::db_of_gain;
    use super::modulation::harness;
    use crate::fx::{AUDIO_BLOCK_FRAMES, AUDIO_CHANNELS};

    /// How many frames are counted, and as many again ahead of them to settle.
    const WINDOW_FRAMES: usize = 3_072;

    /// The amplitude of the left channel at `hz` over the run's second half:
    /// one discrete Fourier bin worked out on its own, because a whole
    /// transform for a single frequency would be a dependency and a hundred
    /// lines of it.
    fn level(signal: &[f32], hz: f64) -> f64 {
        let frames = signal.len() / AUDIO_CHANNELS;
        let step = std::f64::consts::TAU * hz / f64::from(harness::RATE);
        let (mut real, mut imaginary) = (0.0, 0.0);
        for (n, frame) in signal
            .chunks_exact(AUDIO_CHANNELS)
            .skip(frames / 2)
            .enumerate()
        {
            let phase = step * n as f64;
            let sample = f64::from(frame.first().copied().unwrap_or(0.0));
            real += sample * phase.cos();
            imaginary -= sample * phase.sin();
        }
        2.0 * real.hypot(imaginary) / (frames - frames / 2).max(1) as f64
    }

    let blocks = WINDOW_FRAMES.div_ceil(AUDIO_BLOCK_FRAMES) * 2;
    let input = harness::tone(blocks, hz);
    let values = harness::values(def, over);
    let out = harness::run(&*harness::open(def, &values), &input, &values);
    db_of_gain(level(&out, hz) / level(&input, hz))
}

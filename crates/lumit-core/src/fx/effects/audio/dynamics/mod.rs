//! Dynamics: the audio effects that set their own gain from the level they
//! hear (docs/impl/audio-effects.md §4).
//!
//! # In plain terms
//!
//! Three effects and one idea. Listen to how loud the sound is, work out a
//! gain from it, and apply that gain. The compressor turns down what is above
//! a threshold, the gate turns down what is below one, and the limiter is a
//! compressor that is never allowed to be late. What differs between them is
//! the curve and the ballistics, so each is a short file over the followers in
//! [`super::dsp::envelope`].
//!
//! Two rows are read **once, at open**, and never again: how far an effect
//! looks ahead, and whether the limiter measures between the samples. Both
//! decide the effect's latency, and the chain asks for that once a run.

pub mod compressor;
pub mod gate;
pub mod limiter;

use crate::fx::ParamId;

/// What a row holds at this block's start, or `default` when the bake sent no
/// number for it, which is what a saved instance older than the row looks
/// like.
fn row(values: &[(ParamId, f64)], id: ParamId, default: f64) -> f64 {
    values
        .iter()
        .find(|(held, _)| *held == id)
        .map_or(default, |(_, value)| *value)
}

/// Milliseconds as whole frames at `rate`, which is how a time row becomes a
/// delay line's length.
///
/// Held to two seconds, past every row that asks, so a driven row cannot ask
/// for a buffer the size of a film.
fn frames_of_ms(ms: f64, rate: f64) -> u32 {
    if !ms.is_finite() || ms <= 0.0 || rate <= 0.0 {
        return 0;
    }
    (ms.min(2_000.0) * 0.001 * rate).round() as u32
}

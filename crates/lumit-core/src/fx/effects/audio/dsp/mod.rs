//! The cores every built-in audio effect is built from
//! (docs/impl/audio-effects.md §3).
//!
//! # In plain terms
//!
//! Fifteen effects want the same eight things: dB to a multiplier, a ramp
//! that stops a knob clicking, a biquad, a delay line, an LFO, a level
//! follower, a filter that can be swept, and a way to run a waveshaper at
//! four times the rate. Each is written once here so the effects stay short
//! and one fix reaches all of them.
//!
//! Three rules hold in every file. Samples are `f32` and coefficients are
//! `f64`, so a long recursion does not lose its low end. Nothing allocates
//! per sample; the one buffer a delay line needs is taken at construction.
//! And everything is deterministic and can be reset, because the export is
//! the preview's own arithmetic rather than a second opinion (docs/09 §8).

pub mod biquad;
pub mod db;
pub mod delay_line;
pub mod envelope;
pub mod lfo;
pub mod oversample;
pub mod smoother;
pub mod svf;

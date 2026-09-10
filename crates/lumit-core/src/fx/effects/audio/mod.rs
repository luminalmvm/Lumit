//! The built-in audio effects: sound in, sound out, nothing drawn
//! (docs/impl/audio-effects.md).
//!
//! # In plain terms
//!
//! An audio effect sits on a layer's rack or a clip's rack exactly where a
//! hosted plugin sits, and it is a plain struct with its state. The chain
//! builds one fresh for each bake, hands it 512 interleaved stereo frames at
//! a time, and expects the same answer twice.
//!
//! [`dsp`] holds what they all share: the filters, the delay lines, the
//! followers and the oversampler. The four group modules beside it hold the
//! effects themselves, sorted by what they do to the sound rather than by
//! which product named them first.

pub mod dsp;
pub mod dynamics;
pub mod filters;
pub mod modulation;
pub mod space;

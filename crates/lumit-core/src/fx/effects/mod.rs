//! One file per built-in effect: its declaration, and its behaviour
//! (docs/impl/effect-registry.md §2.1).
//!
//! **In plain terms.** This is where an effect lives now. Everything that used
//! to be spread over a catalogue entry, an enum variant, a resolve arm and a CPU
//! arm sits in one file, and the file is the only place that has to be right.
//!
//! Each module declares a parameter struct with `#[derive(Effect)]` — the fields
//! *are* the controls, the attributes on them *are* the sliders and defaults —
//! and a small companion type carrying the behaviour: the host-side maths, and
//! the CPU reference the GPU kernel is checked against (docs/08 §1.6).
//!
//! The migration is in progress: effects that have not moved yet still live in
//! `builtins.rs` and `resolved.rs` and are listed there. `catalogue.rs` names
//! the ones that have.

pub mod accumulation_mb;
pub mod block_glitch;
pub mod blur;
pub mod chromatic_aberration;
pub mod colour_balance;
pub mod contrast;
pub mod datamosh;
pub mod directional_blur;
pub mod dof;
pub mod echo;
pub mod exposure;
pub mod flash;
pub mod gamma;
pub mod glow;
pub mod hue_shift;
pub mod invert;
pub mod light_wrap;
pub mod lut;
pub mod matte_key;
pub mod motion_blur;
pub mod posterize_time;
pub mod radial_blur;
pub mod rgb_split;
pub mod saturation;
pub mod scanlines;
pub mod sharpen;
pub mod sharpen_simple;
pub mod sprite_flare;
pub mod temperature;
pub mod tint;
pub mod transform;
pub mod vibrancy;
pub mod vignette;

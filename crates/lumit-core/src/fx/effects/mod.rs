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
pub mod add_grain;
pub mod angle_control;
pub mod beam;
pub mod bezier_warp;
pub mod black_and_white;
pub mod block_glitch;
pub mod blur;
pub mod brightness;
pub mod broadcast_safe;
pub mod camera_track;
pub mod card_wipe;
pub mod channel_blur;
pub mod checkbox_control;
pub mod chromatic_aberration;
pub mod clone_to_points;
pub mod colour_balance;
pub mod colour_control;
pub mod contrast;
pub mod corner_pin;
pub mod curves;
pub mod datamosh;
pub mod directional_blur;
pub mod displacement_map;
pub mod dof;
pub mod drop_shadow;
pub mod echo;
pub mod emboss;
pub mod exposure;
pub mod fill;
pub mod find_edges;
pub mod flash;
pub mod fractal_noise;
pub mod gamma;
pub mod glow;
pub mod gradient;
pub mod grid;
pub mod hue_saturation;
pub mod hue_shift;
pub mod invert;
pub mod iris_wipe;
pub mod lens_distort;
pub mod lens_flare;
pub mod levels;
pub mod light_wrap;
pub mod lightning;
pub mod linear_wipe;
pub mod lut;
pub mod matte_key;
pub mod median;
pub mod mirror;
pub mod mosaic;
pub mod motion_blur;
pub mod noise;
pub mod offset;
pub mod particulate;
pub mod photo_filter;
pub mod planar_track;
pub mod point_control;
pub mod polar_coordinates;
pub mod posterize;
pub mod posterize_time;
pub mod radial_blur;
pub mod radial_wipe;
pub mod radio_waves;
pub mod rgb_split;
pub mod ripple;
pub mod roughen_edges;
pub mod saturation;
pub mod scanlines;
pub mod scatter;
pub mod scribble;
pub mod set_matte;
pub mod shadow_highlight;
pub mod shake;
pub mod sharpen;
pub mod sharpen_simple;
pub mod slider_control;
pub mod spherize;
pub mod sprite_flare;
pub mod stroke;
pub mod temperature;
pub mod texturize;
pub mod threshold;
pub mod tile;
pub mod tint;
pub mod transform;
pub mod tritone;
pub mod turbulent_displace;
pub mod twirl;
pub mod vegas;
pub mod venetian_blinds;
pub mod vibrancy;
pub mod vignette;
pub mod warp;
pub mod wave_warp;

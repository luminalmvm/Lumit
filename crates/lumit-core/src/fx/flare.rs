//! The Sprite flare's design: a list of elements (glows, rays, ghosts and so
//! on) kept in the effect, and how they are drawn.
//!
//! The design is static, kept as JSON in the instance's `extra` under
//! [`design::EXTRA_KEY`]. What animates is the effect's own rows: the light,
//! brightness, size, rotation, colour, evolution and flicker.

pub mod controls;
pub mod design;
pub mod instances;
pub mod presets;
pub mod raster;

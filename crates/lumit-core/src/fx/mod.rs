pub mod audio_chain;
pub mod builtins;
pub mod catalogue;
pub mod cie;
pub mod colour_name;
/// A Gradient row's stops between its two colour rows.
pub mod colour_stops;
pub mod denoise;
pub mod drivers;
pub mod effects;
pub mod enabled;
pub mod fft;
pub mod film;
pub mod flare;
pub mod follow;
pub mod labels;
pub mod lens;
pub mod lens_blur;
pub mod lens_flare;
/// The bundled lens prescription library: 1303 .lens files as
/// embedded text, generated from the FlareSim / PhotonsToPhotos collection.
pub mod lens_library;
pub mod lfx;
pub mod markers;
pub mod maths;
pub mod moments;
pub mod names;
pub mod noise;
pub mod params;
pub mod points;
pub mod presets;
pub mod reference;
pub mod registry;
pub mod resolved;
pub mod schema;
pub mod shader;
pub mod shape_points;
/// The layer styles (docs/impl/layer-styles.md): nine named slots a
/// layer wears, in one pinned order — a second, order-locked list beside the
/// effect stack rather than entries in the catalogue.
pub mod styles;
pub mod temporal;

pub mod cpu;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

use builtins::*;
use drivers::ResolvedDrivers;
use markers::*;
use params::*;
use registry::*;
use schema::*;

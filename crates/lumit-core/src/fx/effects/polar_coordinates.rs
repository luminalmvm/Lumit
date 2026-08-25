//! Polar coordinates (docs/08 §3.50): the frame bent into a circle, and back —
//! AE's Polar Coordinates.
//!
//! **In plain terms.** Rectangular to polar wraps the picture round the middle
//! of the frame: the top row becomes a tiny circle at the centre, the bottom row
//! the outermost ring, and the left and right edges meet in a seam pointing
//! straight up. It is how the "tiny planet" shot is made out of a panorama, and
//! how a straight bar of light becomes a halo. Polar to rectangular is the exact
//! opposite and unrolls a circle into a strip.
//!
//! **Interpolation is a morph, not a fade.** At 50 every pixel is drawn from
//! half-way along its own path into polar space, so the frame is caught bending;
//! the Mix every effect ends with would instead lay the finished bend over the
//! untouched frame and show both at once.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Polar coordinates' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "polar_coordinates",
    label = "Polar coordinates",
    version = 1,
    category = Distortion,
    // Three transcendentals and one bilinear tap a pixel.
    cost = Cheap,
    // A ring at the frame's edge is drawn from a row at the picture's bottom:
    // any output pixel can come from anywhere.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct PolarCoordinates {
    /// Which way the picture is bent. Rectangular to polar wraps rows into
    /// rings; Polar to rectangular unrolls rings into rows, and is the exact
    /// inverse map (§3.50 decision 4).
    #[choice(
        label = "Conversion",
        options = ["Rectangular to polar", "Polar to rectangular"],
        default = 0
    )]
    pub conversion: u32,

    /// How far along the bend each pixel is drawn from, per cent. 0 is the
    /// bit-exact identity and 100 the finished conversion; between the two the
    /// frame is genuinely part-bent rather than part-faded.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub interpolation: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl PolarCoordinates {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The centre and the radius scale are deliberately absent: both are
    /// functions of the raster, which the kernel knows and the host does not —
    /// §3.39's precedent, and the same reason Linear wipe leaves its extent to
    /// the kernel.
    #[must_use]
    pub fn packed(self) -> cpu::PolarParams {
        cpu::PolarParams {
            to_polar: self.conversion == 0,
            interp: (self.interpolation / 100.0).clamp(0.0, 1.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Polar coordinates' behaviour.
pub struct PolarCoordinatesDef;

impl EffectDef for PolarCoordinatesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PolarCoordinates as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::polar_coordinates(rgba, w, h, &PolarCoordinates::read(p).packed());
    }
}

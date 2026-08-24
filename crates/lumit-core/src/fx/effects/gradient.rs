//! Gradient (docs/08 §3.35): a linear or radial two-colour ramp with scatter.
//!
//! **In plain terms.** Two points and two colours. Everything on the line
//! through the first point is the first colour, everything at the second point
//! is the second, and in between the two are mixed by how far along you are —
//! or, in Radial, by how far *out*. Scatter jiggles that distance by a hair per
//! pixel, which is the standard cure for the contour rings a long, shallow ramp
//! shows on an 8-bit display.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Gradient's controls.
///
/// The two points are px@comp (§2.3), declared `Px` so the resolve step converts
/// them to the raster in play and
/// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::rescale_spatial)
/// moves them **together** if the stack is reused at another size (K-266) — a
/// ramp that slid when the preview resolution changed would be a ramp nobody
/// could grade against.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "gradient",
    label = "Gradient",
    version = 1,
    category = Generate,
    cost = Cheap,
    roi = Exact,
    // The ramp replaces the frame outright, so nothing of the input's colour is
    // read and there is nothing to unpremultiply (§2.2).
    premultiplied = true,
    seeded = true,
)]
pub struct Gradient {
    /// Linear projects onto the axis; Radial measures distance from Start, with
    /// End sitting on the outer edge.
    #[choice(options = ["Linear", "Radial"], default = 0)]
    pub shape: u32,

    /// px@comp. The default pair runs top-to-bottom down a 1080p frame.
    #[slider(label = "Start x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub start_x: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "Start y", min = 0.0, max = 2160.0, default = 0.0, unit = Px)]
    pub start_y: f32,

    /// The colour at Start. Scene-linear and open above 1 (§2.1).
    #[colour(label = "Start colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub start_colour: [f32; 4],

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "End x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub end_x: f32,

    /// px@comp; see [`start_x`](Self::start_x).
    #[slider(label = "End y", min = 0.0, max = 2160.0, default = 1080.0, unit = Px)]
    pub end_y: f32,

    /// The colour at End.
    #[colour(label = "End colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub end_colour: [f32; 4],

    /// Per cent: how far a pixel's position along the ramp may be dithered. A
    /// few per cent breaks banding without softening the ramp's ends.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub scatter: f32,

    /// Which draw the scatter follows (§2.4).
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent —
    /// where an imported AE Ramp's "Blend with original" lands.
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

impl Gradient {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// Both reciprocals are taken here, once, and floored against the same
    /// epsilon: Start and End at the same place give a zero-length axis, and the
    /// floor collapses the ramp to one flat colour rather than dividing by zero
    /// (docs/14 §4). The two points
    /// arrive already scaled to the raster by the declared `Px` unit. Both render
    /// paths read this one method, so the CPU reference and the WGSL kernel
    /// cannot drift apart.
    #[must_use]
    pub fn packed(self) -> cpu::GradientParams {
        let axis = [self.end_x - self.start_x, self.end_y - self.start_y];
        let len2 = axis[0] * axis[0] + axis[1] * axis[1];
        // One epsilon, applied to the squared length, so the linear and radial
        // reciprocals degenerate at exactly the same point.
        let len2 = len2.max(1e-6);
        cpu::GradientParams {
            radial: self.shape == 1,
            start: [self.start_x, self.start_y],
            axis,
            inv_len2: 1.0 / len2,
            inv_len: 1.0 / len2.sqrt(),
            c0: [
                self.start_colour[0],
                self.start_colour[1],
                self.start_colour[2],
            ],
            c1: [self.end_colour[0], self.end_colour[1], self.end_colour[2]],
            scatter: (self.scatter / 100.0).clamp(0.0, 1.0),
            seed: self.seed,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Gradient's behaviour.
pub struct GradientDef;

impl EffectDef for GradientDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Gradient as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::gradient(rgba, w, h, &Gradient::read(p).packed());
    }
}

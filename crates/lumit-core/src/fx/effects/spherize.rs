//! Spherize (docs/08 §3.52): a glass ball held over the picture — AE's
//! Spherize.
//!
//! **In plain terms.** Inside a circle the picture is magnified as if seen
//! through a marble: the middle swells and everything it pushed aside is
//! squeezed into the last few pixels before the rim. Bulge turned negative does
//! the exact opposite and pinches instead.
//!
//! The two directions are mutually inverse maps rather than one map with a sign,
//! so a bulge and a pinch of the same strength cancel — which is what makes the
//! control honest and is proved by test.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Spherize's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "spherize",
    label = "Spherize",
    version = 1,
    category = Distortion,
    // One arc sine or sine and one bilinear tap a pixel.
    cost = Cheap,
    // The ball can span the whole frame.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct Spherize {
    /// How wide the ball is, px@comp (§2.3). Declared `Px`, so the resolve
    /// step scales it to the raster in play — **AE's is a signed length in
    /// raster pixels**, and §3.52's fourth note records why Lumit splits the
    /// sign off into [`bulge`](Self::bulge).
    #[slider(
        min = 0.0,
        max = 2000.0,
        default = 550.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// Which way the glass bends, and how much of it there is, per cent.
    /// Positive magnifies the middle; negative pinches it by the exactly
    /// inverse map; 0 is the bit-exact identity.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 100.0,
        hard_min = -100.0,
        hard_max = 100.0
    )]
    pub bulge: f32,

    /// px@comp: where the ball's middle sits (K-260 — point parameters are
    /// pixels). The schema default is a nominal 1080p centre;
    /// [`instantiate_for_raster`](crate::fx::instantiate_for_raster) centres a
    /// fresh instance on the actual comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl Spherize {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// radius arrives as a reciprocal so the kernel runs no division, and Bulge
    /// arrives as a signed fraction — its sign chooses the map and its magnitude
    /// blends toward it.
    #[must_use]
    pub fn packed(self) -> cpu::SpherizeParams {
        let radius = self.radius.max(0.0);
        cpu::SpherizeParams {
            centre: [self.centre_x, self.centre_y],
            radius,
            // Floored so a zero radius does not divide; the kernel's `r >=
            // radius` test short-circuits before the reciprocal is used anyway.
            inv_radius: 1.0 / radius.max(1e-3),
            bulge: (self.bulge / 100.0).clamp(-1.0, 1.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Spherize's behaviour.
pub struct SpherizeDef;

impl EffectDef for SpherizeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Spherize as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::spherize(rgba, w, h, &Spherize::read(p).packed());
    }
}

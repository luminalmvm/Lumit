//! Linear wipe (docs/08 §3.46): a straight edge swept across the frame — AE's
//! Linear Wipe.
//!
//! **In plain terms.** A line is drawn across the picture and everything behind
//! it is taken away. Completion says how far the line has travelled, Wipe angle
//! which way it runs, Feather how soft its edge is, and Wipe centre where it
//! pivots. Keyframe Completion from 0 to 100 and you have a cut made out of a
//! move.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Linear wipe's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "linear_wipe",
    label = "Linear wipe",
    version = 1,
    category = Transition,
    cost = Trivial,
    roi = Exact,
    // The picture is scaled by a coverage, which is the premultiplied form of
    // "less of this pixel" — all four channels, no round trip (§3.34's
    // reasoning).
    premultiplied = true,
)]
pub struct LinearWipe {
    /// Where the wipe line pivots, px@comp (K-260: point parameters are PIXELS).
    /// The schema default is nominal 1080p centre; `instantiate_for_raster`
    /// centres a fresh instance on the actual comp.
    #[slider(label = "Wipe centre x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Wipe centre y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// How far the edge has travelled, per cent. **50 by default, where AE's is
    /// 0**, for docs/08 §3.39's reason: an effect whose default state has
    /// removed nothing is an effect that has not been applied (§1.2).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub completion: f32,

    /// Which way the edge runs, degrees, **measured from straight up and
    /// turning clockwise** (AE's convention, and §3.43's). At 90° the edge is
    /// vertical and the left of the frame goes first; at 0° the top goes first.
    #[dial(label = "Wipe angle", default = 90.0, step = 15.0)]
    pub angle: f32,

    /// How soft the edge is, px@comp. Keeps AE's 0: Completion above already
    /// makes the effect visible, and a second divergence would be taste imposed
    /// on a control that has a right answer of its own.
    #[slider(min = 0.0, max = 500.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub feather: f32,

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

impl LinearWipe {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// one trigonometric pair is taken here, host-side, for §1.6's reason;
    /// `(sin θ, −cos θ)` is "from straight up, clockwise" on a raster whose y
    /// grows downward. The frame's extent along that direction is *not* here —
    /// it is a function of the raster, which the kernel knows and the host does
    /// not (§3.39's precedent).
    #[must_use]
    pub fn packed(self) -> cpu::LinearWipeParams {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        cpu::LinearWipeParams {
            centre: [self.centre_x, self.centre_y],
            normal: [sin, -cos],
            completion: (self.completion / 100.0).clamp(0.0, 1.0),
            // Floored so the hard-edged case is a step rather than a divide by
            // zero (docs/14 §4); neither path divides per pixel.
            band: self.feather.max(0.0).max(1e-3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Linear wipe's behaviour.
pub struct LinearWipeDef;

impl EffectDef for LinearWipeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LinearWipe as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::linear_wipe(rgba, w, h, &LinearWipe::read(p).packed());
    }
}

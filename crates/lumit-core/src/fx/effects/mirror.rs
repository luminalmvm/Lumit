//! Mirror (docs/08 §3.41): one half of the frame reflected onto the other.
//!
//! **In plain terms.** Draw a line through the picture. Everything on one side
//! of it is thrown away and replaced by the reflection of what is on the other
//! side. Centre says where the line passes through, Angle which way it runs and
//! which side is replaced.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Mirror's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "mirror",
    label = "Mirror",
    version = 1,
    category = Distortion,
    cost = Cheap,
    // A reflection about a movable axis reaches anywhere in the frame.
    roi = FullFrame,
    premultiplied = true,
)]
pub struct Mirror {
    /// px@comp: where the reflection axis passes through (K-260 — point
    /// parameters are pixels, never per cent of frame). The schema default is
    /// nominal 1080p centre; `instantiate_for_raster` centres a fresh instance
    /// on the actual comp.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// Degrees: the direction the reflection points. At 0 the right half of the
    /// frame becomes a mirror of the left. A positive angle turns the axis
    /// clockwise on screen, because the raster's y grows downward — the same
    /// reading every other angle in the catalogue has.
    #[dial(default = 0.0, step = 15.0)]
    pub angle: f32,

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

impl Mirror {
    /// The centre in raster pixels, the axis normal as a host-computed
    /// cosine/sine pair (§1.6: WGSL's trigonometry is not correctly rounded and
    /// carries no guarantee of agreeing with Rust's), and the mix.
    #[must_use]
    pub fn packed(self) -> ([f32; 2], [f32; 2], f32) {
        let rad = self.angle.to_radians();
        (
            [self.centre_x, self.centre_y],
            [rad.cos(), rad.sin()],
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Mirror's behaviour.
pub struct MirrorDef;

impl EffectDef for MirrorDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Mirror as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (centre, normal, mix) = Mirror::read(p).packed();
        cpu::mirror(rgba, w, h, centre, normal, mix);
    }
}

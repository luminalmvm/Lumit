//! Directional blur (docs/08 §3.8): a line-integral streak along an
//! angle.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Directional blur's controls.
///
/// Length, Angle and the edge switch the Gaussian has (the three-way Edges
/// control is Radial's alone). Length may exceed the frame (slider to 2000
/// px@comp, hard-unbounded above); the kernel's tap count still clamps
/// ([`cpu::dir_blur_taps`]), so a long streak stays bounded in cost. ROI is
/// full-frame: an unbounded Length cannot be padded statically.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "directional_blur",
    label = "Directional blur",
    version = 1,
    category = BlurSharpen,
    cost = Moderate,
    roi = FullFrame,
    // The matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Length per pixel: white streaks the full Length, grey a \
         shorter one, black not at all",
    ),
)]
pub struct DirectionalBlur {
    /// The full streak length, px@comp (§2.3). Unbounded above; the
    /// slider reaches 2000 and typing goes further.
    #[slider(
        min = 0.0,
        max = 2000.0,
        default = 200.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub length: f32,

    /// Streak direction, degrees (0° = +x, y-down raster).
    #[slider(
        min = -180.0,
        max = 180.0,
        default = 0.0,
        hard_min = -3600.0,
        hard_max = 3600.0,
        unit = Degrees
    )]
    pub angle: f32,

    /// On holds the border pixel outward, so the edge of the frame stays solid.
    /// Off streaks the edge away into transparency and lets the streak run
    /// past it. See Gaussian blur's own.
    #[toggle(label = "Repeat edge pixels", default = true)]
    pub repeat_edge_pixels: bool,

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

impl DirectionalBlur {
    /// The streak length in raster pixels, its angle, the edge policy and the
    /// mix (docs/impl/effect-registry.md §2.4). `length` arrives already
    /// converted from % diagonal by the resolve step, so this only floors it —
    /// the same `.max(0.0)` the old arm applied to the same product. Edge is
    /// 1 Repeat or 0 Transparent. The unit direction and the tap count are
    /// derived from these by whichever path runs, exactly as they were before.
    pub fn packed(self) -> (f32, f32, u32, f32) {
        (
            self.length.max(0.0),
            self.angle,
            u32::from(self.repeat_edge_pixels),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Directional blur's behaviour.
pub struct DirectionalBlurDef;

impl EffectDef for DirectionalBlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<DirectionalBlur as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (length_px, angle_deg, edge, mix) = DirectionalBlur::read(p).packed();
        cpu::blur_directional(rgba, w, h, length_px, angle_deg, edge, mix);
    }

    fn spread_px(&self, p: Params<'_>) -> [f32; 2] {
        let (length_px, angle_deg, edge, mix) = DirectionalBlur::read(p).packed();
        if edge != 0 || mix <= 0.0 {
            return [0.0, 0.0];
        }
        // The streak is centred on the pixel, so half of it leaves each way,
        // and only along its own direction.
        let (dx, dy) = crate::fx::maths::rgb_split_offset(length_px * 0.5, angle_deg);
        [dx.abs(), dy.abs()]
    }
}

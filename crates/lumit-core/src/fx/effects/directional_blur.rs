//! Directional blur (docs/08 §3.8, K-137): a line-integral streak along an
//! angle.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Directional blur's controls.
///
/// Length/Angle only, fixed Repeat edge (the Edges control is Radial's alone
/// now). Length may exceed the frame (slider to 2000 px@comp,
/// hard-unbounded above per K-090); the kernel's tap count still clamps
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
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Length per pixel: white streaks the full Length, grey a \
         shorter one, black not at all",
    ),
)]
pub struct DirectionalBlur {
    /// The full streak length, px@comp (§2.3). Unbounded above (K-090); the
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
        hard_max = 3600.0
    )]
    pub angle: f32,

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

impl DirectionalBlur {
    /// The streak length in raster pixels, its angle, the edge policy and the
    /// mix (docs/impl/effect-registry.md §2.4). `length` arrives already
    /// converted from % diagonal by the resolve step, so this only floors it —
    /// the same `.max(0.0)` the old arm applied to the same product. Edge is
    /// the fixed Repeat. The unit direction and the tap count are derived from
    /// these by whichever path runs, exactly as they were before.
    pub fn packed(self) -> (f32, f32, u32, f32) {
        (
            self.length.max(0.0),
            self.angle,
            1,
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
}

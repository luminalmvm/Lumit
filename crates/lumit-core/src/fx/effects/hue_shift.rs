//! Hue shift (docs/08 §3.10): a rotation about the colour wheel.

use crate::fx::{cpu, hue_matrix, hue_matrix_rgb, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Hue shift's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "hue_shift",
    label = "Hue shift",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Angle toward 0 per pixel: white turns the hue by the full \
         Angle, grey part of the way, black not at all",
    ),
)]
pub struct HueShift {
    /// Degrees on a dial (docs/07 §6): a hue shift is a rotation about the
    /// colour wheel, so the control is that wheel. Wraps every 360, and
    /// unbounded so an animated hue winds through whole turns rather than
    /// stopping.
    #[dial(default = 0.0, step = 15.0)]
    pub angle: f32,

    /// On (default): the constant-luminance rotation (Rec. 709 luma held). Off:
    /// a plain-RGB spin about the grey axis, brightness free to change with the
    /// hue. Absent on projects saved before this bool existed → true, the
    /// historical behaviour.
    #[toggle(default = true)]
    pub preserve_luminance: bool,

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

impl HueShift {
    /// The rotation matrix this instance carries (K-136).
    ///
    /// The bool only picks which host-computed matrix is used, so the CPU
    /// reference and the WGSL kernel stay in parity: neither of them rotates.
    pub fn matrix(self) -> [f32; 9] {
        if self.preserve_luminance {
            hue_matrix(f64::from(self.angle))
        } else {
            hue_matrix_rgb(f64::from(self.angle))
        }
    }

    /// The matrix the kernel multiplies by, and the mix
    /// (docs/impl/effect-registry.md §2.4). Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> ([f32; 9], f32) {
        (self.matrix(), (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// Hue shift's behaviour.
pub struct HueShiftDef;

impl EffectDef for HueShiftDef {
    fn schema(&self) -> &'static EffectSchema {
        &<HueShift as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (m, mix) = HueShift::read(p).packed();
        cpu::hue_shift(rgba, m, mix);
    }
}

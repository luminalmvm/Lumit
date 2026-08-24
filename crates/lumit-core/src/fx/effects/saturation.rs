//! Saturation (docs/08 §3.11): a mix between the picture and its Rec. 709 luma.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Saturation's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "saturation",
    label = "Saturation",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: grading premultiplied shifts matte edges.
    premultiplied = false,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "pulls Saturation toward 100 per pixel: white applies the full \
         Saturation, black leaves the colour as it was",
    ),
)]
pub struct Saturation {
    /// Per cent about Rec. 709 luma: 0 = greyscale, 100 = neutral, 200 =
    /// doubled. The maths (a mix of luma and colour by saturation ÷ 100) simply
    /// keeps extrapolating above 200, so the hard ceiling is open (K-135): the
    /// slider reaches a heavy 400, and typing higher pushes further.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub saturation: f32,

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

impl Saturation {
    /// The numbers the kernel multiplies by: the per-cent controls as plain
    /// factors (docs/impl/effect-registry.md §2.4).
    ///
    /// Both render paths read this one method, so the CPU reference and the
    /// WGSL kernel cannot drift apart — the §1.6 oracle only checks the kernel
    /// against the reference, never the two conversions against each other.
    ///
    /// Floored at 0 (greyscale), open above (K-135): the luma/colour mix
    /// extrapolates past 200 % cleanly, so no upper clamp.
    pub fn packed(self) -> (f32, f32) {
        (
            (self.saturation / 100.0).max(0.0),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Saturation's behaviour.
pub struct SaturationDef;

impl EffectDef for SaturationDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Saturation as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (saturation, mix) = Saturation::read(p).packed();
        cpu::saturate(rgba, saturation, mix);
    }
}

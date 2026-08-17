//! Vibrancy (docs/08 §3.12): saturation that lifts the flat pixels hardest.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Vibrancy's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "vibrancy",
    label = "Vibrancy",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: grading premultiplied shifts matte edges.
    premultiplied = false,
)]
pub struct Vibrancy {
    /// Per cent: 0 = neutral (bit-exact identity), higher lifts the
    /// less-saturated pixels more. The slider reaches a heavy 200; typing
    /// higher pushes further (K-135 open ceiling), floored at 0.
    #[slider(min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0)]
    pub amount: f32,

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

/// Vibrancy's behaviour.
pub struct VibrancyDef;

impl EffectDef for VibrancyDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Vibrancy as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let v = Vibrancy::read(p);
        // Floored at 0 (neutral), open above (K-135): the per-pixel factor
        // extrapolates cleanly, so no upper clamp.
        cpu::vibrance(
            rgba,
            (v.amount / 100.0).max(0.0),
            (v.mix / 100.0).clamp(0.0, 1.0),
        );
    }
}

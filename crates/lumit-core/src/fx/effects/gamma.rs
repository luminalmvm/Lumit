//! Gamma (docs/08 §3.15): the power curve, raising to 1/gamma.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Gamma's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "gamma",
    label = "Gamma",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: a non-linear curve shifts matte edges.
    premultiplied = false,
)]
pub struct Gamma {
    /// The power curve raises to 1/gamma. 1 is neutral; hard floor 0.01 keeps
    /// 1/gamma finite, no hard ceiling above.
    #[slider(min = 0.1, max = 4.0, default = 1.0, hard_min = 0.01)]
    pub gamma: f32,

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

/// Gamma's behaviour.
pub struct GammaDef;

impl EffectDef for GammaDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Gamma as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let v = Gamma::read(p);
        // Hard floor 0.01 keeps 1/gamma finite; no ceiling.
        cpu::gamma(rgba, v.gamma.max(0.01), (v.mix / 100.0).clamp(0.0, 1.0));
    }
}

//! Exposure (docs/08 §3.13): photographic stops, each one a doubling of light.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Exposure's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "exposure",
    label = "Exposure",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
)]
pub struct Exposure {
    /// Photographic stops; each +1 doubles the light. 0 is neutral.
    #[slider(min = -5.0, max = 5.0, default = 0.0)]
    pub stops: f32,

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

/// Exposure's behaviour.
pub struct ExposureDef;

impl EffectDef for ExposureDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Exposure as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let v = Exposure::read(p);
        cpu::exposure(
            rgba,
            f64::from(v.stops).exp2() as f32,
            (v.mix / 100.0).clamp(0.0, 1.0),
        );
    }
}

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

impl Exposure {
    /// The linear gain the kernel multiplies by, and the mix
    /// (docs/impl/effect-registry.md §2.4). Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    ///
    /// `2f64.powf` rather than `f64::exp2`, because that is the call the resolve
    /// arm made before the effect moved to the registry: the two agree to well
    /// within the §1.6 tolerance, but they are not obliged to agree in the last
    /// bit, and a migration must not change a single one.
    pub fn packed(self) -> (f32, f32) {
        (
            2f64.powf(f64::from(self.stops)) as f32,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Exposure's behaviour.
pub struct ExposureDef;

impl EffectDef for ExposureDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Exposure as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (factor, mix) = Exposure::read(p).packed();
        cpu::exposure(rgba, factor, mix);
    }
}

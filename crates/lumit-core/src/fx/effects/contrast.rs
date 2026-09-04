//! Contrast (docs/08 §3.14): a scale about mid-grey.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Contrast's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "contrast",
    label = "Contrast",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: an affine grade shifts matte edges.
    premultiplied = false,
)]
pub struct Contrast {
    /// Per cent about mid-grey: 0 = flat grey, 100 = neutral, 200 = doubled.
    /// Hard min 0 (no inversion); unbounded above. The response between those
    /// points is quadratic in the distance from 100, so the first few
    /// per cent are a nudge rather than a jump.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub contrast: f32,

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

impl Contrast {
    /// The factor the kernel multiplies by, and the mix
    /// (docs/impl/effect-registry.md §2.4). Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    ///
    /// `k = 1 + t|t|`, where `t` is the distance from neutral in hundredths
    /// (`contrast / 100 − 1`): the response is **quadratic** in that distance,
    /// not linear. A per cent either side of 100 moves a hundredth of
    /// what it used to, which is what the slider needed to be usable near
    /// neutral, and the two ends are exactly where they were — 0 flattens to
    /// grey, 200 doubles. Neutral is `t == 0`, so 100 % is still the bit-exact
    /// identity both paths short-circuit on.
    pub fn packed(self) -> (f32, f32) {
        let t = self.contrast / 100.0 - 1.0;
        (
            (1.0 + t * t.abs()).max(0.0),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Contrast's behaviour.
pub struct ContrastDef;

impl EffectDef for ContrastDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Contrast as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (k, mix) = Contrast::read(p).packed();
        cpu::contrast(rgba, k, mix);
    }
}

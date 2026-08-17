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
    /// Hard min 0 (no inversion); unbounded above.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0)]
    pub contrast: f32,

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

impl Contrast {
    /// The factor the kernel multiplies by, and the mix
    /// (docs/impl/effect-registry.md §2.4). Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    ///
    /// k = contrast per cent / 100; hard min 0 (no inversion), unbounded above
    /// — the schema's own honest shape.
    pub fn packed(self) -> (f32, f32) {
        (
            (self.contrast / 100.0).max(0.0),
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

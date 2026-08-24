//! Invert (docs/08 §3.22): 1 − c, on unpremultiplied colour.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Invert's controls — the Mix alone.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "invert",
    label = "Invert",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: 1 − c is affine, so it shifts matte edges.
    premultiplied = false,
)]
pub struct Invert {
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

impl Invert {
    /// The mix the kernel blends by (docs/impl/effect-registry.md §2.4) — the
    /// whole of Invert's host maths, since 1 − c takes no parameter of its own.
    /// Both render paths read this one method.
    pub fn packed(self) -> f32 {
        (self.mix / 100.0).clamp(0.0, 1.0)
    }
}

/// Invert's behaviour.
pub struct InvertDef;

impl EffectDef for InvertDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Invert as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::invert(rgba, Invert::read(p).packed());
    }
}

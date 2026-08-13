//! Tint (docs/08 §3.23): black and white remapped to two chosen colours.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Tint's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "tint",
    label = "Tint",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: a colour remap shifts matte edges.
    premultiplied = false,
)]
pub struct Tint {
    /// Scene-linear RGBA (alpha ignored): the colour dark input maps to.
    #[colour(label = "Map black to", default = [0.0, 0.0, 0.0, 1.0], min = 0.0, max = 4.0)]
    pub black: [f32; 4],

    /// Scene-linear RGBA (alpha ignored): the colour bright input maps to.
    #[colour(label = "Map white to", default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub white: [f32; 4],

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

/// Tint's behaviour.
pub struct TintDef;

impl EffectDef for TintDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Tint as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let v = Tint::read(p);
        // The two mapped colours are scene-linear RGB; alpha is ignored, and the
        // CPU reference and the WGSL kernel read the identical numbers.
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        cpu::tint(
            rgba,
            rgb(v.black),
            rgb(v.white),
            (v.mix / 100.0).clamp(0.0, 1.0),
        );
    }
}

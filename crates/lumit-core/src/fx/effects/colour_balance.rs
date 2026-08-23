//! Colour balance (docs/08 §3.10 as amended by K-090): lift / gamma / gain.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Colour balance's controls.
///
/// Defaults are neutral — a grade's "tasteful default" is a preset choice,
/// which is what the §3.10 preset browser is for.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "colour_balance",
    label = "Colour balance",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: grading premult shifts matte edges.
    premultiplied = false,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "pulls Lift toward 0 and Gamma and Gain toward 1 per pixel: white \
         applies the full grade, black none",
    ),
)]
pub struct ColourBalance {
    /// Added after gain: raises (or crushes, negative) the blacks.
    #[colour(default = [0.0, 0.0, 0.0, 1.0], min = -1.0, max = 1.0)]
    pub lift: [f32; 4],

    /// Mid-tone curve per channel; 1 is neutral.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.1, max = 4.0)]
    pub gamma: [f32; 4],

    /// Linear multiplier per channel; 1 is neutral.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], min = 0.0, max = 4.0)]
    pub gain: [f32; 4],

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

impl ColourBalance {
    /// The three RGB triples the kernel grades with, and the mix
    /// (docs/impl/effect-registry.md §2.4). Alpha is ignored: a grade never
    /// touches the matte. Gamma floors at 0.01 so the reciprocal exponent
    /// stays finite. Both render paths read this one method, so the CPU
    /// reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> ([f32; 3], [f32; 3], [f32; 3], f32) {
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        (
            rgb(self.lift),
            rgb(self.gamma).map(|g| g.max(0.01)),
            rgb(self.gain),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Colour balance's behaviour.
pub struct ColourBalanceDef;

impl EffectDef for ColourBalanceDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ColourBalance as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (lift, gamma, gain, mix) = ColourBalance::read(p).packed();
        cpu::colour_balance(rgba, lift, gamma, gain, mix);
    }
}

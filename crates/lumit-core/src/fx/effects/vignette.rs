//! Vignette (docs/08 §3.14): darkening toward black away from the frame centre.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Vignette's controls.
///
/// Every distance here is read against the Roundness-blended metric
/// [`cpu::vignette`] derives from the raster's own `w`/`h`, so **nothing is
/// spatial**: the metric is already resolution-relative by construction, and a
/// value in it does not move when the raster does. That is why the effect
/// declares no `PctDiag`/`Px` parameter and why the old `rescale_px` listed it
/// among the arms that did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "vignette",
    label = "Vignette",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
)]
pub struct Vignette {
    /// 0..1: the darkening strength; 0 is the neutral point (bit-exact
    /// passthrough, pinned by test).
    #[slider(min = 0.0, max = 1.0, default = 0.5, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub amount: f32,

    /// 0..1: how far from centre the clear area reaches, in the
    /// Roundness-blended distance metric below (1.0 = that metric's own
    /// reference edge).
    #[slider(min = 0.0, max = 1.0, default = 0.75, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub radius: f32,

    /// Feather width beyond Radius, in the same normalised metric. The metric is
    /// not capped at 1 (a distance reaches ~√2 at a corner under circular
    /// roundness), so Softness may exceed 1 for a wider feather (K-135): the
    /// hard ceiling is open, the slider reaches 2.
    #[slider(min = 0.0, max = 2.0, default = 0.5, hard_min = 0.0, unit = Raw)]
    pub softness: f32,

    /// 1 = circular (both axes read equal pixel distances as equal); 0 = follows
    /// the frame's own aspect ratio (an ellipse exactly reaching every edge at
    /// Radius 1).
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub roundness: f32,

    /// Gamma on the black↔clear falloff (T16): 1 = the plain smoothstep, > 1
    /// rolls the dark in later then faster, < 1 earlier and gentler — a
    /// curve/levels on the darkening amount.
    #[slider(min = 0.2, max = 4.0, default = 1.0, hard_min = 0.05, unit = Raw)]
    pub ramp: f32,

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

impl Vignette {
    /// Amount, radius, softness, roundness, ramp and mix, clamped exactly as the
    /// old resolve arm clamped them (docs/impl/effect-registry.md §2.4): the
    /// three 0..1 dials cap at both ends, Softness floors at 0 and is open above
    /// (K-135), and Ramp floors at 0.05 so the falloff's exponent stays finite.
    /// Both render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> (f32, f32, f32, f32, f32, f32) {
        (
            self.amount.clamp(0.0, 1.0),
            self.radius.clamp(0.0, 1.0),
            self.softness.max(0.0),
            self.roundness.clamp(0.0, 1.0),
            self.ramp.max(0.05),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Vignette's behaviour.
pub struct VignetteDef;

impl EffectDef for VignetteDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Vignette as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (amount, radius, softness, roundness, ramp, mix) = Vignette::read(p).packed();
        cpu::vignette(rgba, w, h, amount, radius, softness, roundness, ramp, mix);
    }
}

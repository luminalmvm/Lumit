//! Tritone (docs/08 §3.60): three colours mapped onto the tone range — AE's
//! Tritone.
//!
//! **In plain terms.** The picture's own colours are thrown away and replaced by
//! a ramp of three the user chooses: the darkest pixels become the Shadows
//! colour, the brightest the Highlights colour, and everything between is mixed
//! through the Midtones colour. It is how a photograph becomes a cyanotype, a
//! sepia print, or the two-colour title card of a trailer.
//!
//! §3.24 Tint is the same idea with two colours and stays. What this adds is the
//! middle stop, which is what makes the look a *duotone print* rather than a
//! straight fade between two ends.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Tritone's controls.
///
/// The three defaults are a real duotone rather than a no-op (§3.10): a deep
/// blue in the shadows, a warm brown through the middle and a warm white at the
/// top, which is a split-toned print and the reason to reach for the effect.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "tritone",
    label = "Tritone",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: a luma-driven colour remap does not commute with premultiplied
    // alpha — §3.24's reason, and a soft edge would fringe.
    premultiplied = false,
)]
pub struct Tritone {
    /// The colour the brightest pixels take. Scene-linear and open above 1
    /// (§2.1); the alpha is ignored.
    #[colour(default = [1.0, 0.98, 0.90, 1.0], max = 4.0)]
    pub highlights: [f32; 4],

    /// The colour mid-grey takes.
    #[colour(default = [0.45, 0.30, 0.22, 1.0], max = 4.0)]
    pub midtones: [f32; 4],

    /// The colour the darkest pixels take.
    #[colour(default = [0.02, 0.03, 0.10, 1.0], max = 4.0)]
    pub shadows: [f32; 4],

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent —
    /// where an imported AE Tritone's "Blend with original" lands.
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

impl Tritone {
    /// The three stops and the mix (docs/impl/effect-registry.md §2.4). The
    /// alphas are dropped here rather than in either kernel.
    #[must_use]
    pub fn packed(self) -> cpu::TritoneParams {
        cpu::TritoneParams {
            shadows: [self.shadows[0], self.shadows[1], self.shadows[2]],
            midtones: [self.midtones[0], self.midtones[1], self.midtones[2]],
            highlights: [self.highlights[0], self.highlights[1], self.highlights[2]],
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Tritone's behaviour.
pub struct TritoneDef;

impl EffectDef for TritoneDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Tritone as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::tritone(rgba, &Tritone::read(p).packed());
    }
}

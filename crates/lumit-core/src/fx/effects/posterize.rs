//! Posterize (docs/08 §3.58): the tone ladder cut into steps — AE's Posterize.
//!
//! **In plain terms.** A photograph has millions of shades. This throws almost
//! all of them away and keeps a handful of evenly-spaced rungs, so a smooth
//! gradient becomes a set of flat bands — the poster-print look, and the first
//! half of a cel-shaded cartoon.
//!
//! The one thing worth knowing is *where* the rungs go. Lumit works in
//! scene-linear light (docs/08 §2.1), which is a measurement of photons and not
//! of what a person sees, so evenly-spaced-in-light rungs would pile up in the
//! highlights and leave the shadows nearly smooth. The rungs are therefore
//! spaced evenly in a square root of the light, which is close enough to what
//! the eye does — and, being one exactly-rounded instruction on both paths,
//! close enough that the CPU and the GPU always agree on which rung a value
//! lands on (§3.58 decision 2).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Posterize's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "posterize",
    label = "Posterize",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: quantising premultiplied colour would band a soft edge by its
    // coverage and fringe it.
    premultiplied = false,
)]
pub struct Posterize {
    /// How many rungs each channel keeps. 2 is the two-tone print, 8 is the
    /// poster, and the hard maximum is AE's 255.
    #[counter(min = 2, max = 64, default = 8, hard_min = 2, hard_max = 255)]
    pub levels: i32,

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

impl Posterize {
    /// The number of *gaps* between rungs and the mix
    /// (docs/impl/effect-registry.md §2.4). The subtraction and the clamp
    /// happen here, once, so the CPU reference and the WGSL kernel divide by
    /// identical numbers.
    #[must_use]
    pub fn packed(self) -> (f32, f32) {
        (
            (self.levels.clamp(2, 255) - 1) as f32,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Posterize's behaviour.
pub struct PosterizeDef;

impl EffectDef for PosterizeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Posterize as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (n, mix) = Posterize::read(p).packed();
        cpu::posterize(rgba, n, mix);
    }
}

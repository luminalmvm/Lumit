//! Brightness (docs/08 §3.32, K-397): AE's Brightness & Contrast as one
//! effect — a sibling of the one-knob Contrast (§3.18), not a mode of it.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Brightness' controls: AE's two sliders, under AE's names and at AE's
/// neutral point (0 for both), so an imported `ADBE Brightness & Contrast 2`
/// lands on one effect with one keyframed pair rather than being split.
///
/// Neutral by default — the grade family's sanctioned exception to the
/// "no no-op default" rule (docs/08 §3.10).
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "brightness",
    label = "Brightness",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: an affine grade does not commute with premultiplied alpha,
    // exactly as Contrast's `− pivot` does not.
    premultiplied = false,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "pulls Brightness toward 0 and Contrast toward 0 per pixel: white \
         applies both in full, black neither",
    ),
)]
pub struct Brightness {
    /// Per cent, added to every channel: ±100 is ±1.0 of scene-linear light.
    /// Unbounded either way — the offset stays meaningful past the slider.
    #[slider(min = -100.0, max = 100.0, default = 0.0)]
    pub brightness: f32,

    /// Per cent about mid-grey, AE's signed spelling: 0 is neutral, −100
    /// flattens the picture to grey, +100 doubles the spread. Floored at −100
    /// (below it the picture would invert about the pivot, which is Invert's
    /// job) and open above.
    #[slider(min = -100.0, max = 100.0, default = 0.0, hard_min = -100.0)]
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

impl Brightness {
    /// The offset, the contrast factor and the mix
    /// (docs/impl/effect-registry.md §2.4). Both scalars are worked out here,
    /// once, so the CPU reference and the WGSL kernel multiply by identical
    /// numbers.
    #[must_use]
    pub fn packed(self) -> (f32, f32, f32) {
        (
            self.brightness / 100.0,
            1.0 + self.contrast.max(-100.0) / 100.0,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Brightness' behaviour.
pub struct BrightnessDef;

impl EffectDef for BrightnessDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Brightness as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (b, k, mix) = Brightness::read(p).packed();
        cpu::brightness(rgba, b, k, mix);
    }
}

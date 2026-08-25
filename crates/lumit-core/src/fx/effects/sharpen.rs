//! Unsharp mask (docs/08 §3.9): sharpening in linear light, on unpremultiplied
//! colour (§2.2 — sharpening premultiplied values haloes matte edges). The
//! unpremultiply → sharpen → re-premultiply wrap is fused into the kernel.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// The Unsharp mask's controls.
///
/// Labelled "Unsharp mask" since K-138 split the plain 3×3 Sharpen out beside
/// it; the `match_name` stays "sharpen" so saved projects are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "sharpen",
    label = "Unsharp mask",
    version = 1,
    category = BlurSharpen,
    cost = Cheap,
    // Radius' own hard maximum: 100 px@comp.
    roi = PaddedPx(100.0),
    // §2.2: operates on unpremultiplied colour.
    premultiplied = false,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Amount per pixel: white adds the full Amount of detail \
         back, black none",
    ),
)]
pub struct Sharpen {
    /// Per cent of the detail signal added back (§3.9: 0–300 %).
    #[slider(
        min = 0.0,
        max = 300.0,
        default = 60.0,
        hard_min = 0.0,
        hard_max = 300.0,
        unit = Percent
    )]
    pub amount: f32,

    /// px@comp (§2.3) — the width of the detail the mask lifts; small values
    /// crispen, larger add clarity.
    #[slider(
        min = 1.0,
        max = 50.0,
        default = 8.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Px
    )]
    pub radius: f32,

    /// Linear-light contrast below which detail is left alone, so compression
    /// noise is not amplified (§3.9).
    #[slider(min = 0.0, max = 1.0, default = 0.05, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub threshold: f32,

    /// Sharpen the luma signal only — avoids chroma fringing on compressed game
    /// capture (§3.9).
    #[toggle(default = true)]
    pub luminance_only: bool,

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

impl Sharpen {
    /// The detail fraction, the internal gaussian's half-width in raster
    /// pixels, the threshold, the luma switch and the mix (docs/impl/
    /// effect-registry.md §2.4). `radius` arrives already converted from %
    /// diagonal by the resolve step, so this only floors it. Both render paths
    /// read this one method, so the CPU reference and the WGSL kernel cannot
    /// drift apart.
    pub fn packed(self) -> (f32, f32, f32, bool, f32) {
        (
            (self.amount / 100.0).clamp(0.0, 3.0),
            self.radius.max(0.0),
            self.threshold.clamp(0.0, 1.0),
            self.luminance_only,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// The Unsharp mask's behaviour.
pub struct SharpenDef;

impl EffectDef for SharpenDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Sharpen as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (amount, radius_px, threshold, luma_only, mix) = Sharpen::read(p).packed();
        cpu::sharpen(rgba, w, h, amount, radius_px, threshold, luma_only, mix);
    }
}

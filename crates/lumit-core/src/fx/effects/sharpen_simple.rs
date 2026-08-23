//! Sharpen (docs/08 §3.9, K-138): the plain, radius-free sibling of the Unsharp
//! mask — a fixed 3×3 high-pass convolution scaled by Amount, `out = u +
//! amount·(4·u − up − down − left − right)` per RGB channel with clamp-addressed
//! neighbours. On unpremultiplied colour (§2.2, the wrap fused into the kernel),
//! alpha untouched; the neighbours read the edge pixel (clamp/Repeat) so a
//! border never invents dark detail. Amount 0 is the bit-exact passthrough.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Sharpen's controls. One job, cheap, one pixel of reach — the honest "just
/// sharpen it" control next to the Unsharp mask's radius/threshold/luma knobs.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "sharpen_simple",
    label = "Sharpen",
    version = 1,
    category = BlurSharpen,
    cost = Cheap,
    // A fixed 3×3 kernel reads one pixel out; % diag of one raster pixel is
    // tiny, so 1 % over-covers at any sensible resolution.
    roi = PaddedPctDiag(1.0),
    // §2.2: sharpening premultiplied haloes matte edges.
    premultiplied = false,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Amount per pixel: white sharpens by the full Amount, black \
         not at all",
    ),
)]
pub struct SharpenSimple {
    /// High-pass strength: 1 is the classic 5/−1 sharpen kernel, 0 a no-op.
    /// Clamped at zero below (a negative amount would blur, out of scope),
    /// unbounded above (K-090).
    #[slider(min = 0.0, max = 5.0, default = 1.0, hard_min = 0.0)]
    pub amount: f32,

    /// Neighbour distance in raster pixels (T15): 1 = the classic 3×3 kernel,
    /// larger sharpens over a coarser neighbourhood.
    ///
    /// Deliberately **not** spatial. It is a kernel *stride* in raster pixels
    /// rather than a length at comp size, so it does not follow the preview
    /// raster — which is exactly what the old `rescale_px` said when it skipped
    /// this field, and a migration changes no behaviour.
    #[slider(min = 1.0, max = 8.0, default = 1.0, hard_min = 1.0)]
    pub radius: f32,

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

impl SharpenSimple {
    /// The high-pass strength, the neighbour distance and the mix
    /// (docs/impl/effect-registry.md §2.4) — the same floors the old arm
    /// applied. Both render paths read this one method, so the CPU reference and
    /// the WGSL kernel cannot drift apart.
    pub fn packed(self) -> (f32, f32, f32) {
        (
            self.amount.max(0.0),
            self.radius.max(1.0),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Sharpen's behaviour.
pub struct SharpenSimpleDef;

impl EffectDef for SharpenSimpleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SharpenSimple as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (amount, radius, mix) = SharpenSimple::read(p).packed();
        cpu::sharpen_simple(rgba, w, h, amount, radius, mix);
    }
}

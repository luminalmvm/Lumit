//! Gaussian blur (docs/08 §3.8, K-137): the separable two-pass blur, one job
//! per effect since the mode-driven Blur was split.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Gaussian blur's controls.
///
/// `match_name` stays "blur", so a project saved with the old combined effect
/// loads here as Gaussian at its Radius, byte-identically (its now-unread
/// mode/length/centre params are simply ignored). The Edges control is Radial's
/// alone now; this one resolves at the old default, Repeat.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "blur",
    label = "Gaussian blur",
    version = 1,
    category = BlurSharpen,
    cost = Moderate,
    // The Radius slider's own hard maximum, in px@comp (K-433): 2 000 px is
    // the farthest a typed radius can reach, so the tile is never short of it.
    roi = PaddedPx(2000.0),
    // K-395: the blur claims the injected Matte row inside its own maths — see
    // the `matte` doc below for what it means. The generic strength dissolve
    // does not also run.
    matte = (
        "matte",
        "scales the blur radius per pixel: white blurs at the full Radius, grey \
         blurs narrowly, black not at all — a blur whose width varies across \
         the frame, which a strength dissolve cannot produce",
    ),
)]
pub struct Blur {
    /// Kernel half-width, px@comp (§2.3, K-419). Default per §1.2's "drop it
    /// on and it already looks right".
    ///
    /// Declared `Px`, so the resolve step scales it to the raster in play (a
    /// Half preview blurs the same part of the picture as the export) and
    /// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size.
    #[slider(
        min = 0.0,
        max = 500.0,
        default = 30.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
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

impl Blur {
    /// The kernel half-width in raster pixels, the edge policy and the mix
    /// (docs/impl/effect-registry.md §2.4). `radius` arrives already converted
    /// from % diagonal by the resolve step, so this only floors it — the same
    /// `.max(0.0)` the old arm applied to the same product. Edge is the fixed
    /// Repeat (K-137 dropped the Gaussian Edges control; 1 was its default).
    /// Both render paths read this one method, so the CPU reference and the
    /// WGSL kernel cannot drift apart.
    pub fn packed(self) -> (f32, u32, f32) {
        (self.radius.max(0.0), 1, (self.mix / 100.0).clamp(0.0, 1.0))
    }
}

/// Gaussian blur's behaviour.
pub struct BlurDef;

impl EffectDef for BlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Blur as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (radius_px, edge, mix) = Blur::read(p).packed();
        cpu::blur_gaussian(rgba, w, h, radius_px, edge, mix);
    }
}

//! Radial blur (docs/08 §3.8, K-137): arcs (Spin) or rays (Zoom) about a
//! centre.

use crate::fx::{cpu, EdgesMode, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Radial blur's controls.
///
/// Type is Spin / Zoom; both reduce to one linear scale of the pixel's own
/// (position − centre) vector — Zoom along it (an exact ray sample), Spin along
/// its perpendicular (the tangent approximation to the true arc) — so neither
/// needs a division or a runtime trig call, and every tap collapses to exactly
/// the pixel at Centre with no epsilon guard. This is the one blur to keep the
/// shared Edges control (P3, K-145); its taps run through the same
/// `bilinear_edge` sampler the others use.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "radial_blur",
    label = "Radial blur",
    version = 1,
    category = BlurSharpen,
    cost = Moderate,
    roi = FullFrame,
)]
pub struct RadialBlur {
    /// Peak tap spread, px@comp (§2.3), reached at the farthest corner from
    /// Centre. Unbounded above (K-090); the tap count clamps in
    /// [`cpu::radial_blur_taps`], so cost stays bounded.
    #[slider(
        min = 0.0,
        max = 2000.0,
        default = 150.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub amount: f32,

    /// % of comp width. `resolve_stack` carries only `diag_px` (no separate
    /// width/height), so this stays a plain number here and [`packed`](Self::
    /// packed) turns it into a *fraction* the CPU/GPU function scales by its own
    /// w — exactly how chromatic aberration derives the frame centre. Not
    /// spatial, therefore: a fraction of the raster does not move when the
    /// raster does.
    #[slider(label = "Centre X", min = 0.0, max = 100.0, default = 50.0)]
    pub centre_x: f32,

    /// % of comp height (see [`centre_x`](Self::centre_x)).
    #[slider(label = "Centre Y", min = 0.0, max = 100.0, default = 50.0)]
    pub centre_y: f32,

    /// Spin (arcs about Centre) or Zoom (rays through it).
    #[choice(label = "Type", options = ["Spin", "Zoom"], default = 0)]
    pub radial_type: u32,

    /// The reusable Edges control (P3, K-145).
    #[choice(
        label = "Edges",
        options = *crate::fx::EDGE_OPTIONS,
        // Repeat: full-frame game footage never darkens along the border.
        default = 1
    )]
    pub edge: u32,

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

impl RadialBlur {
    /// The centre as a raster fraction, the peak spread in raster pixels,
    /// whether it spins, the edge policy and the mix (docs/impl/
    /// effect-registry.md §2.4).
    ///
    /// `amount` arrives already converted from % diagonal by the resolve step.
    /// The stored Choices map exactly as the old arm mapped them: anything but
    /// index 1 is Spin, and the edge index goes through [`EdgesMode`] clamped to
    /// the known set, falling back to Repeat. Both render paths read this one
    /// method, so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> ([f32; 2], f32, bool, u32, f32) {
        (
            [self.centre_x / 100.0, self.centre_y / 100.0],
            self.amount.max(0.0),
            self.radial_type != 1,
            EdgesMode::from_code(self.edge.min(2))
                .unwrap_or(EdgesMode::Repeat)
                .code(),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Radial blur's behaviour.
pub struct RadialBlurDef;

impl EffectDef for RadialBlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RadialBlur as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (centre_frac, amount_px, spin, edge, mix) = RadialBlur::read(p).packed();
        cpu::blur_radial(rgba, w, h, centre_frac, amount_px, spin, edge, mix);
    }
}

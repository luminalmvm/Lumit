//! Radial blur (docs/08 §3.8): arcs (Spin), rays (Zoom) or one-way fading
//! streaks (Rays) about a centre.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EdgesMode,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Radial blur's controls.
///
/// Type is Spin / Zoom; both reduce to one linear scale of the pixel's own
/// (position − centre) vector — Zoom along it (an exact ray sample), Spin along
/// its perpendicular (the tangent approximation to the true arc) — so neither
/// needs a division or a runtime trig call, and every tap collapses to exactly
/// the pixel at Centre with no epsilon guard. This is the one blur to keep the
/// shared Edges control (P3); its taps run through the same
/// `bilinear_edge` sampler the others use.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "radial_blur",
    label = "Radial blur",
    // 2: the centre pair crossed from a per cent of the frame to px@comp.
    // `migrate_percent_to_px` converts a v1 instance on load.
    version = 2,
    category = BlurSharpen,
    cost = Moderate,
    roi = FullFrame,
    // The matte scales the amount, inside the kernel (the owner's rule for
    // mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Amount per pixel: white sweeps the full Amount, grey a \
         shorter one, black not at all",
    ),
)]
pub struct RadialBlur {
    /// Peak tap spread, px@comp (§2.3), reached at the farthest corner from
    /// Centre. Unbounded above; the tap count clamps in
    /// [`cpu::radial_blur_taps`], so cost stays bounded.
    #[slider(
        min = 0.0,
        max = 2000.0,
        default = 150.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub amount: f32,

    /// px@comp (no point is a per cent of the frame). The schema default
    /// is the nominal 1080p centre; `instantiate_for_raster` centres a fresh
    /// instance on the actual comp, and the resolve step scales the stored
    /// pixels to the raster being rendered, so a Half preview spins about the
    /// same place as the export.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// px@comp; see [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// Spin (arcs about Centre), Zoom (rays through it) or Rays (streaks away
    /// from it that fade as they go, as light through an edge).
    #[choice(label = "Type", options = ["Spin", "Zoom", "Rays"], default = 0)]
    pub radial_type: u32,

    /// The reusable Edges control (P3).
    #[choice(
        label = "Edges",
        options = *crate::fx::builtins::EDGE_OPTIONS,
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
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl RadialBlur {
    /// The centre in raster pixels, the peak spread in raster pixels, its
    /// kind (0 Zoom, 1 Spin, 2 Rays), the edge policy and the mix
    /// (docs/impl/effect-registry.md §2.4).
    ///
    /// Both the centre and the amount arrive already scaled to the raster by
    /// the resolve step, which is what a `Unit::Px` parameter gets.
    /// The stored Choices map exactly as the old arm mapped them: anything but
    /// Zoom or Rays is Spin, and the edge index goes through [`EdgesMode`]
    /// clamped to the known set, falling back to Repeat. Both render paths read
    /// this one method, so the CPU reference and the WGSL kernel cannot drift
    /// apart.
    pub fn packed(self) -> ([f32; 2], f32, u32, u32, f32) {
        (
            [self.centre_x, self.centre_y],
            self.amount.max(0.0),
            match self.radial_type {
                1 => 0,
                2 => 2,
                _ => 1,
            },
            EdgesMode::from_code(self.edge.min(2))
                .unwrap_or(EdgesMode::Repeat)
                .code(),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// The Rays kind: each pixel reads back along the line to Centre, the nearer
/// the more, so the picture streaks one way, away from Centre, and fades as it
/// goes. Amount is the reach at the farthest corner, and it stops at Centre.
pub fn rays(rgba: &mut [f32], w: u32, h: u32, centre: [f32; 2], amount: f32, edge: u32, mix: f32) {
    let original = rgba.to_vec();
    let diagonal = f64::from(w * w + h * h).sqrt() as f32;
    let reach = if diagonal > 0.0 {
        (amount / (0.5 * diagonal)).min(1.0)
    } else {
        0.0
    };
    let taps = cpu::radial_blur_taps(amount);
    let weights: Vec<(f32, f32)> = (0..taps)
        .map(|t| {
            let u = (t as f32 + 0.5) / taps as f32;
            (u, (1.0 - u) * (1.0 - u))
        })
        .collect();
    let total: f32 = weights.iter().map(|(_, weight)| weight).sum();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let pos = (x as f32 + 0.5, y as f32 + 0.5);
            let d = (pos.0 - centre[0], pos.1 - centre[1]);
            let mut sum = [0.0f32; 4];
            for (u, weight) in &weights {
                let at = cpu::bilinear_edge(
                    &original,
                    w,
                    h,
                    pos.0 - u * reach * d.0,
                    pos.1 - u * reach * d.1,
                    edge,
                );
                for (s, c) in sum.iter_mut().zip(at) {
                    *s += c * weight;
                }
            }
            for c in 0..4 {
                rgba[i + c] = original[i + c] * (1.0 - mix) + sum[c] / total * mix;
            }
        }
    }
}

/// Radial blur's behaviour.
pub struct RadialBlurDef;

impl EffectDef for RadialBlurDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RadialBlur as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (centre_px, amount_px, kind, edge, mix) = RadialBlur::read(p).packed();
        match kind {
            2 => rays(rgba, w, h, centre_px, amount_px, edge, mix),
            _ => cpu::blur_radial(rgba, w, h, centre_px, amount_px, kind == 1, edge, mix),
        }
    }
}

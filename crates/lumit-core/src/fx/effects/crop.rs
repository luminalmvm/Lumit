//! Crop: takes the picture away past four edges, each set in per cent of the
//! frame from its own side, with a soft edge if asked for. The crop half of
//! Vegas's Pan/Crop. Nothing is cropped at the defaults, so it can go on a
//! clip beside a Transform without changing what the clip shows.

use crate::fx::{
    params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Crop's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "crop",
    label = "Crop",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    // The picture is scaled by a coverage, the premultiplied form of "less of
    // this pixel", as Linear wipe's is.
    premultiplied = true,
)]
pub struct Crop {
    /// How much of the frame goes from the left, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub left: f32,

    /// How much goes from the top, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub top: f32,

    /// How much goes from the right, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub right: f32,

    /// How much goes from the bottom, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 0.0, unit = Percent)]
    pub bottom: f32,

    /// How soft the edges are, px@comp.
    #[slider(min = 0.0, max = 500.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub feather: f32,

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

/// What both render paths draw from: the edges as fractions of the frame, the
/// feather in raster pixels and the mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropParams {
    /// Left, top, right and bottom, each 0..1 from its own side.
    pub edges: [f32; 4],
    /// The feather's width in raster pixels, floored above zero.
    pub band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl Crop {
    /// The bundle both kernels read.
    #[must_use]
    pub fn packed(self) -> CropParams {
        let edge = |v: f32| (v / 100.0).clamp(0.0, 1.0);
        CropParams {
            edges: [
                edge(self.left),
                edge(self.top),
                edge(self.right),
                edge(self.bottom),
            ],
            // Floored so a hard edge is a step rather than a divide by zero.
            band: self.feather.max(0.0).max(1e-3),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// How much of the pixel centred at (`x`, `y`) the crop keeps, 0 to 1, on a
/// `w` by `h` raster. The WGSL twin is the same expression.
#[must_use]
pub fn crop_keep(x: f32, y: f32, w: f32, h: f32, p: &CropParams) -> f32 {
    let [left, top, right, bottom] = p.edges;
    let inside = (x - left * w)
        .min(w - right * w - x)
        .min(y - top * h)
        .min(h - bottom * h - y);
    (inside / p.band + 0.5).clamp(0.0, 1.0)
}

/// The CPU oracle: every channel scaled by what the crop keeps.
pub fn crop(rgba: &mut [f32], w: u32, h: u32, p: &CropParams) {
    let (fw, fh) = (w as f32, h as f32);
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let x = (i as u32 % w.max(1)) as f32 + 0.5;
        let y = (i as u32 / w.max(1)) as f32 + 0.5;
        let keep = crop_keep(x, y, fw, fh, p);
        // `1 − mix·(1 − keep)`, so a kept pixel scales by exactly 1.
        let k = 1.0 - p.mix * (1.0 - keep);
        for c in px.iter_mut() {
            *c *= k;
        }
    }
}

/// Crop's behaviour.
pub struct CropDef;

impl EffectDef for CropDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Crop as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        crop(rgba, w, h, &Crop::read(p).packed());
    }
}

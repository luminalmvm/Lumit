//! Mood lighting (docs/08 §3.98): slow pools of coloured light laid over the
//! picture, and a firmer contrast under them — AE's Mood Lighting presets.
//!
//! **In plain terms.** Somebody has put lamps just out of shot. Where a pool of
//! light lands the picture is warmer and brighter; between the pools it falls
//! away cooler and darker; and the whole frame comes back with more contrast
//! than it went in with. Drift moves through the pools rather than redrawing
//! them, so a keyframe on it gives light that breathes.
//!
//! The pools are [`crate::fx::noise`], the same field Fractal noise draws and
//! Turbulent displace steers with — not a copy of it
//! (docs/impl/ae-effect-parity.md). Contrast is §3.14's own curve for the same
//! reason: one grade, spelled once.

use crate::fx::{cpu, noise, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Mood lighting's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "mood_lighting",
    label = "Mood lighting",
    version = 1,
    category = Stylise,
    // Three octaves of 3-D noise a pixel, then a pointwise grade.
    cost = Moderate,
    // The light is read at the pixel's own place; nothing is tapped from a
    // neighbour.
    roi = Exact,
    // §2.2: a multiply and a grade of premultiplied colour would shift matte
    // edges.
    premultiplied = false,
    seeded = true,
    matte = (
        "matte",
        "scales Intensity and Contrast per pixel: white lights and grades in full, grey more gently, black leaves the picture alone",
    ),
)]
pub struct MoodLighting {
    /// Per cent: how far the light pulls the picture either side of neutral. 0
    /// is no light at all, which leaves Contrast working on its own. 100 is the
    /// lamp at full, and the effect is meant to be seen on the way in; turn it
    /// down for a grade that only hints.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub intensity: f32,

    /// Scene-linear RGBA (alpha ignored): the colour of the light inside a pool.
    /// A mid grey is the neutral — it multiplies by one — so the two colours
    /// say which way each end of the field pulls, not how hard.
    #[colour(label = "Light colour", default = [0.98, 0.72, 0.42, 1.0], max = 4.0)]
    pub light: [f32; 4],

    /// Scene-linear RGBA (alpha ignored): the colour between the pools. Cool and
    /// dark by default, which is the other half of what makes a mood.
    #[colour(label = "Shade colour", default = [0.10, 0.16, 0.34, 1.0], max = 4.0)]
    pub shade: [f32; 4],

    /// px@comp: how big one pool of light is. Large by default — these are lamps
    /// off the edge of the frame, not texture.
    #[slider(min = 1.0, max = 4000.0, default = 900.0, hard_min = 1.0, unit = Px)]
    pub scale: f32,

    /// Degrees: the field's depth coordinate. One full turn moves the pools on
    /// by one of their own widths, matching Fractal noise (§3.37 decision 3), so
    /// keyframing it drifts the light rather than reseeding it.
    #[dial(default = 0.0, step = 45.0)]
    pub drift: f32,

    /// Per cent about mid-grey: 100 is neutral, below flattens, above firms up.
    /// §3.14's control and §3.14's curve — the response is quadratic in the
    /// distance from 100, so the first few per cent are a nudge.
    #[slider(min = 0.0, max = 200.0, default = 115.0, hard_min = 0.0, unit = Percent)]
    pub contrast: f32,

    /// Which arrangement of pools this instance lights with (§2.4).
    #[seed]
    pub seed: u32,

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

impl MoodLighting {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The field's shape is settled here and nowhere else: three octaves of
    /// signed Perlin, halving in amplitude and doubling in frequency. That is
    /// what "amorphous" is — soft blobs with a little structure on them — and
    /// leaving it to the kernels would be two places to change it and one of
    /// them forgotten. Scale arrives already scaled to the raster by its
    /// declared `Px` unit and leaves as a reciprocal, so the kernel divides by
    /// nothing.
    #[must_use]
    pub fn packed(self) -> cpu::MoodLightingParams {
        cpu::MoodLightingParams {
            field: noise::FractalField {
                seed: self.seed,
                octaves: 3,
                gain: 0.5,
                lacunarity: 2.0,
                perlin: true,
                turbulent: false,
                cycle: 0,
            },
            inv_scale: 1.0 / self.scale.max(1.0),
            z: self.drift / 360.0,
            intensity: (self.intensity / 100.0).max(0.0),
            light: [self.light[0], self.light[1], self.light[2]],
            shade: [self.shade[0], self.shade[1], self.shade[2]],
            // §3.14's factor, read from §3.14's own effect so the two cannot
            // answer the same slider differently.
            contrast: super::contrast::Contrast {
                contrast: self.contrast,
                mix: 100.0,
            }
            .packed()
            .0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Mood lighting's behaviour.
pub struct MoodLightingDef;

impl EffectDef for MoodLightingDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MoodLighting as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::mood_lighting(rgba, w, h, &MoodLighting::read(p).packed());
    }
}

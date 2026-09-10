//! Pixel sort (docs/08 §3.99): runs of pixels picked out by a threshold, each
//! one sorted along its own line.
//!
//! **In plain terms.** Walk a row of the picture and note every run of pixels
//! whose brightness (or red, or hue) falls inside a band. Sort each run on its
//! own and put it back where it was. The rest of the row is left exactly as it
//! arrived. What comes out is the smeared, glitched look that has been made by
//! hand in generative art for years, and the whole of the difference between a
//! good one and a mess is which pixels get into a run.
//!
//! **Why there is a Maximum span length.** Every pixel finds its own place by
//! reading its whole run, so a run of a thousand costs a thousand reads for
//! each of its thousand pixels. The cap is what keeps that bounded, and it is
//! a look as much as a saving, which is why it is a slider rather than a
//! preference.
//!
//! **And why there is a seed beside it.** The cap chops the line into equal
//! pieces, and equal pieces on every line would draw the chop as a column down
//! the frame. The seed offsets each line's pieces by its own amount, so the
//! breaks scatter and the cap stays a cap instead of becoming a pattern.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Pixel sort's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "pixel_sort",
    label = "Pixel sort",
    version = 1,
    category = Stylise,
    // Every pixel reads its own span to find its place in it, so the cost is
    // Maximum span length taps a pixel rather than one.
    cost = Heavy,
    // A pixel can be drawn from anywhere in its own span, and a span can start
    // a thousand pixels away.
    roi = FullFrame,
    // §2.2: the whole texel travels, alpha with it. Nothing here grades a
    // colour, so there is nothing to unpremultiply for — the key is worked out
    // on straight colour inside the kernel and thrown away again.
    premultiplied = true,
    // Its pixels are a function of its parameters and its input, never of the
    // clock: the seed picks the offsets, the playhead does not.
    seeded = false,
    // **The matte-rule override.** The matte is not a strength here — it says
    // *where the spans are*, read inside the effect's own maths, so the
    // generic dissolve must not run as well.
    matte = (
        "matte",
        "says where the spans may form, alongside Min and Max: a pixel sorts \
         only where the matte is at least half lit",
    ),
)]
pub struct PixelSort {
    /// Which property the sort orders by. Luminance is the default because it
    /// is the one every reference implementation starts from and the one that
    /// reads as sorting rather than as a colour separation.
    #[choice(
        label = "Sort by",
        options = ["Red", "Green", "Blue", "Luminance", "Hue", "Saturation"],
        default = 3
    )]
    pub sort_by: u32,

    /// Which way a span runs. Horizontal sorts along rows, Vertical down
    /// columns; there is no angle between them, because a span at an angle is a
    /// resampled line rather than a rearrangement of pixels that already exist.
    #[choice(label = "Direction", options = ["Horizontal", "Vertical"], default = 0)]
    pub direction: u32,

    /// What a sorted span is written back as. Sort is the plain thing; Stretch
    /// gives the whole span the pixel the sort put at its far end, which reads
    /// as one long streak; Mirror lays the sorted run out from both ends inward,
    /// so the span climbs to its middle and falls back.
    #[choice(label = "Span mode", options = ["Sort", "Stretch", "Mirror"], default = 0)]
    pub span_mode: u32,

    /// Orders every span the other way round. It flips Stretch's far end too,
    /// so Stretch with Reverse on smears the other extreme.
    #[toggle(default = false)]
    pub reverse: bool,

    /// The bottom of the band that sorts. A pixel joins a span when its Sort by
    /// value is at least this and at most Max.
    #[bounded(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub min: f32,

    /// The top of it. Below Min nothing sorts at all, which is the honest
    /// answer to an empty band rather than a silent swap of the two.
    #[bounded(min = 0.0, max = 1.0, default = 1.0, unit = Raw)]
    pub max: f32,

    /// px@comp (§2.3): the most pixels one span may hold, and so how far a
    /// pixel reads to find its place. 300 is the default because it is long
    /// enough to streak and short enough to leave the picture readable.
    #[slider(
        label = "Maximum span length",
        min = 0.0,
        max = 1000.0,
        default = 300.0,
        hard_min = 0.0,
        hard_max = 1024.0,
        unit = Px
    )]
    pub max_span: f32,

    /// Which offsets the spans take on each line (§2.4). Sits second-last,
    /// immediately before Mix, as every seeded effect's does.
    #[seed(label = "Random span offsets")]
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

impl PixelSort {
    /// The numbers both kernels want (docs/impl/effect-registry.md §2.4).
    ///
    /// Maximum span length arrives already scaled to the raster by its declared
    /// `Px` unit, and leaves as a whole number of pixels clamped to the shared
    /// ceiling — so neither path has to know what that ceiling is, and neither
    /// can pick a different one. Min and Max are **not**
    /// swapped when they cross: an empty band sorts nothing, which is what the
    /// panel shows the user they asked for.
    #[must_use]
    pub fn packed(self) -> cpu::PixelSortParams {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let stride = self
            .max_span
            .clamp(1.0, cpu::PIXEL_SORT_MAX_SPAN as f32)
            .round() as u32;
        cpu::PixelSortParams {
            sort_by: self.sort_by,
            vertical: self.direction == 1,
            span_mode: self.span_mode,
            reverse: self.reverse,
            min: self.min.clamp(0.0, 1.0),
            max: self.max.clamp(0.0, 1.0),
            stride,
            seed: self.seed,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Pixel sort's behaviour.
pub struct PixelSortDef;

impl EffectDef for PixelSortDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PixelSort as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::pixel_sort(rgba, w, h, &PixelSort::read(p).packed());
    }
}

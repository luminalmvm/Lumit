//! Shadow highlight (docs/08 §3.63): the local rescue of a backlit shot — AE's
//! Shadow/Highlight.
//!
//! **In plain terms.** Someone stood in front of a window comes out as a
//! silhouette: the camera exposed for the window, and the face went to nothing.
//! Lifting the whole picture would blow the window out. This lifts only the dark
//! *regions* and pulls down only the bright ones, and the word doing the work is
//! **regions**.
//!
//! That is what makes it local-adaptive, and it is one idea: whether a pixel is
//! treated as a shadow is decided by how bright its *neighbourhood* is, not how
//! bright it is. A white shirt button inside a dark jacket is part of a shadow
//! and is lifted with it, instead of being singled out and left behind. The
//! neighbourhood's brightness comes from the shipped §3.8 gaussian at Radius —
//! the third effect to reuse it, after Drop shadow's softening and Roughen
//! edges' distance field — and it steers the *mask* only. Nothing is softened;
//! the blur is a question, not an answer.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params};
use lumit_fx_macros::Effect;

/// The panel's one disclosure — AE's "More Options", trimmed to the two controls
/// that survived (§3.63).
pub const SHADOW_HIGHLIGHT_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "More options",
    params: &["colour_correction", "midtone_contrast"],
    collapsed: true,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// Shadow highlight's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "shadow_highlight",
    label = "Shadow highlight",
    version = 1,
    category = Colour,
    // One gaussian, then one pointwise pass.
    cost = Moderate,
    // Radius' own hard maximum in px@comp, exactly as the Gaussian blur
    // declares its own.
    roi = PaddedPx(2000.0),
    // §2.2: a gain about a luma is a grade, and a grade does not commute with
    // premultiplied alpha.
    premultiplied = false,
    groups = SHADOW_HIGHLIGHT_GROUPS,
    // K-395: the matte scales the amount, inside the kernel (the owner's
    // rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Shadow amount and Highlight amount per pixel: white applies \
         both in full, black neither",
    ),
)]
pub struct ShadowHighlight {
    /// Per cent: how hard the dark regions are lifted. 100 trebles them.
    #[slider(
        label = "Shadow amount",
        min = 0.0,
        max = 100.0,
        default = 25.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub shadow_amount: f32,

    /// Per cent: how far up the tone range counts as shadow. Low keeps the lift
    /// in the deepest darks; high reaches into the midtones.
    #[slider(
        label = "Shadow tonal width",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub shadow_tonal_width: f32,

    /// Per cent: how hard the bright regions are pulled down. 100 takes them to
    /// a third.
    #[slider(
        label = "Highlight amount",
        min = 0.0,
        max = 100.0,
        default = 25.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub highlight_amount: f32,

    /// Per cent: how far down the tone range counts as highlight.
    #[slider(
        label = "Highlight tonal width",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub highlight_tonal_width: f32,

    /// How large a neighbourhood decides whether a pixel is in shadow, px@comp
    /// (§2.3) — the same unit and the same default the Gaussian blur's Radius
    /// has. Small reaches for local contrast; large behaves like a
    /// whole-picture tone curve.
    #[slider(
        min = 0.0,
        max = 500.0,
        default = 30.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub radius: f32,

    /// Per cent: how much saturation is put back where the picture moved.
    /// Lifting a shadow reads as desaturated; this is the cure, and 0 is the
    /// exact identity in colour.
    #[slider(
        label = "Colour correction",
        min = -100.0,
        max = 100.0,
        default = 20.0,
        hard_min = -100.0
    )]
    pub colour_correction: f32,

    /// Per cent about the perceptual middle: the contrast the two lifts flatten,
    /// put back by hand. 0 is neutral.
    #[slider(
        label = "Midtone contrast",
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0
    )]
    pub midtone_contrast: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent —
    /// where an imported AE Shadow/Highlight's "Blend with original" lands.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl ShadowHighlight {
    /// The narrowest a tonal width is allowed to be. Below this the smoothstep
    /// would divide by zero, and a mask a thousandth of the range wide is a step
    /// nobody asked for (§3.59's floor, same reasoning).
    pub const MIN_WIDTH: f32 = 1e-3;

    /// The two gains, the two mask widths, the blur radius and the two more
    /// options (docs/impl/effect-registry.md §2.4). Every division and every
    /// floor happens here, once, so the CPU reference and the WGSL kernel
    /// multiply by identical numbers.
    #[must_use]
    pub fn packed(self) -> cpu::ShadowHighlightParams {
        let shadow = (self.shadow_amount / 100.0).clamp(0.0, 1.0) * 2.0;
        let highlight = (self.highlight_amount / 100.0).clamp(0.0, 1.0) * 2.0;
        let contrast = self.midtone_contrast.max(-100.0) / 100.0;
        cpu::ShadowHighlightParams {
            shadow,
            highlight,
            shadow_width: (self.shadow_tonal_width / 100.0).max(Self::MIN_WIDTH),
            highlight_width: (self.highlight_tonal_width / 100.0).max(Self::MIN_WIDTH),
            // Already raster pixels: the declared Px unit scaled it.
            radius_px: self.radius.max(0.0),
            contrast: 1.0 + contrast,
            colour_correction: self.colour_correction.max(-100.0) / 100.0,
            // Nothing to lift, nothing to pull, nothing to steepen: the exact
            // identity, and the gaussian is not even run.
            active: shadow > 0.0 || highlight > 0.0 || contrast != 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Shadow highlight's behaviour.
pub struct ShadowHighlightDef;

impl EffectDef for ShadowHighlightDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ShadowHighlight as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::shadow_highlight(rgba, w, h, &ShadowHighlight::read(p).packed());
    }
}

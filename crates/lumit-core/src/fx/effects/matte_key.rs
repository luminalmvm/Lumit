//! Matte key (docs/08 §3.21, K-121/K-154): a Keylight-style colour-difference
//! keyer — a proper greenscreen key, not a chroma-distance approximation.
//!
//! **In plain terms.** Thirteen controls and no side table: the largest effect
//! to migrate that is still nothing but numbers. The screen colour's largest
//! channel picks the axis the key measures along, the per-cent dials become
//! plain 0..1 fractions, and the two Choice rows are normalised through the
//! shared [`MatteKeyView`] / [`ReplaceMethod`] wire codes so the CPU reference
//! and the WGSL kernel branch on the same integers. All of that used to sit in
//! a resolve arm; it sits in [`MatteKey::packed`] now, called once by whichever
//! render path is running.
//!
//! Migration (unchanged by this move): a project saved before K-154 keeps its
//! stored `key` (screen colour) and `spill` (now the despill amount); its old
//! `tolerance` / `softness` are superseded and simply go unread.

use crate::fx::{
    cpu, EffectDef, EffectMetadata, EffectSchema, MatteKeyParams, MatteKeyView, ParamGroup, Params,
    ReplaceMethod,
};
use lumit_fx_macros::Effect;

/// The panel's one disclosure group — the matte-tidying rows, collapsed until
/// the key itself is set up. Named here because the derive takes the groups as
/// one expression.
pub const MATTE_KEY_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Screen matte",
    params: &[
        "clip_black",
        "clip_white",
        "clip_rollback",
        "replace_method",
        "replace_colour",
    ],
    collapsed: true,
    visible_when: None,
    visible_when_lens_elements: None,
}];

/// Matte key's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "matte_key",
    label = "Matte key",
    version = 2,
    category = Utility,
    // Every step is clamp/min/max/lerp — continuous, so the §1.6 ULP oracle
    // holds, and pointwise, so the region of interest is exact.
    cost = Cheap,
    roi = Exact,
    // §2.2: keying and despill work on straight colour, so the host wraps the
    // kernel in unpremultiply → key → re-premultiply.
    premultiplied = false,
    groups = MATTE_KEY_GROUPS,
)]
pub struct MatteKey {
    /// Final result (the keyed picture), Screen matte (the alpha as greyscale),
    /// or Status (a continuous heat of the matte). Default Final so the effect
    /// keys the moment it is dropped on.
    #[choice(
        options = ["Final result", "Screen matte", "Status"],
        default = 0
    )]
    pub view: u32,

    /// Scene-linear RGBA; alpha ignored. Default a saturated green, the
    /// greenscreen the effect exists to remove. Its largest channel picks the
    /// primary screen axis (so a blue screen keys too).
    #[colour(label = "Screen colour", default = [0.0, 0.6, 0.0, 1.0], max = 4.0)]
    pub key: [f32; 4],

    /// Per cent → a 0.. multiplier on the matte fall-off. 100 % keys the exact
    /// screen colour to zero; higher keys more aggressively.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0)]
    pub screen_gain: f32,

    /// Per cent → 0..1: how the two non-screen channels are weighted into the
    /// reference (0 = their min, 100 = their max, 50 = mean).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub screen_balance: f32,

    /// Scene-linear RGBA; shifts the reference the despill clamps the primary
    /// down to. A neutral grey is a no-op.
    #[colour(default = [0.5, 0.5, 0.5, 1.0], max = 4.0)]
    pub despill_bias: [f32; 4],

    /// Scene-linear RGBA; shifts what colour counts as neutral for the screen
    /// matte. A neutral grey is a no-op.
    #[colour(default = [0.5, 0.5, 0.5, 1.0], max = 4.0)]
    pub alpha_bias: [f32; 4],

    /// Per cent of the primary's screen excess drained from kept pixels
    /// (defaults full-on, Keylight-like; an older instance keeps its stored
    /// value, an even older one without the param reads 0).
    #[slider(
        label = "Despill amount",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub spill: f32,

    /// Per cent → 0..1: matte at/below this maps to 0 (fully keyed), cleaning
    /// residual grey out of the background.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub clip_black: f32,

    /// Per cent → 0..1: matte at/above this maps to 1 (fully kept), filling
    /// holes in the foreground.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub clip_white: f32,

    /// Per cent → 0..1: eases the clips back toward the un-clipped matte,
    /// recovering fine edge detail (0 = full clip, the default).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub clip_rollback: f32,

    /// How despilled areas are recoloured. Default Soft colour, as Keylight (it
    /// settles into shading rather than a flat patch).
    #[choice(
        options = ["Source", "Hard colour", "Soft colour", "None"],
        default = 2
    )]
    pub replace_method: u32,

    /// Scene-linear RGBA used by the Hard/Soft replace methods; a neutral grey
    /// desaturates spill edges without a colour cast.
    #[colour(default = [0.5, 0.5, 0.5, 1.0], max = 4.0)]
    pub replace_colour: [f32; 4],

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

impl MatteKey {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4),
    /// normalised exactly as the old resolve arm normalised it: every per-cent
    /// dial to a fraction — Screen gain floored at zero and open above, the rest
    /// clamped to 0..1 — and both Choice rows through their shared wire-code
    /// enums, so an unknown stored index falls back to Final and Soft colour
    /// rather than reaching the kernel. Both render paths read this one method,
    /// so the CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self) -> MatteKeyParams {
        MatteKeyParams {
            view: MatteKeyView::from_code(self.view).code(),
            key: self.key,
            gain: (self.screen_gain / 100.0).max(0.0),
            balance: (self.screen_balance / 100.0).clamp(0.0, 1.0),
            despill_bias: self.despill_bias,
            alpha_bias: self.alpha_bias,
            spill: (self.spill / 100.0).clamp(0.0, 1.0),
            clip_black: (self.clip_black / 100.0).clamp(0.0, 1.0),
            clip_white: (self.clip_white / 100.0).clamp(0.0, 1.0),
            clip_rollback: (self.clip_rollback / 100.0).clamp(0.0, 1.0),
            replace_method: ReplaceMethod::from_code(self.replace_method).code(),
            replace_colour: self.replace_colour,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Matte key's behaviour.
pub struct MatteKeyDef;

impl EffectDef for MatteKeyDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MatteKey as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        cpu::matte_key(rgba, &MatteKey::read(p).packed());
    }
}

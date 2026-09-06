//! Matte key (docs/08 §3.21): a Keylight-style
//! colour-difference keyer — a proper greenscreen key, not a chroma-distance
//! approximation.
//!
//! **In plain terms.** The screen colour's largest channel picks the axis the
//! key measures along, the per-cent dials become plain 0..1 fractions, and the
//! two Choice rows are normalised through the shared [`MatteKeyView`] /
//! [`ReplaceMethod`] wire codes so the CPU reference and the WGSL kernel branch
//! on the same integers. All of that used to sit in a resolve arm; it sits in
//! [`MatteKey::packed`] now, called once by whichever render path is running.
//!
//! **The spatial controls** are the half of Keylight that cannot be
//! done a pixel at a time: Screen pre-blur softens the picture the key is
//! *judged from*, and shrink/grow, Softness and the two Despots tidy the matte
//! as a picture of its own before it is spent on the colour. Two mask-path rows
//! hold parts of the frame open or shut whatever the key made of them.
//! None of that is decided here — this file still only *normalises* numbers;
//! the pipeline is [`cpu::matte_key_spatial`], and it is one branch away from
//! the pointwise keyer, so **the defaults render the bytes they always did**.
//!
//! Migration (unchanged by this move): a project saved by an earlier version
//! keeps its stored `key` (screen colour) and `spill` (now the despill
//! amount); its old `tolerance` / `softness` are superseded and simply go
//! unread. The spatial rows are all neutral by default, so an older project is
//! not re-keyed by them either.

use crate::fx::{
    cpu, EffectDef, EffectMetadata, EffectSchema, MatteKeyParams, MatteKeyView, ParamGroup,
    ParamId, Params, ReplaceMethod, ResolveCx, Value,
};
use lumit_fx_macros::Effect;

/// The panel's one disclosure group — the matte-tidying rows, collapsed until
/// the key itself is set up. Named here because the derive takes the groups as
/// one expression.
pub const MATTE_KEY_GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "Screen matte",
    params: &[
        "pre_blur",
        "clip_black",
        "clip_white",
        "clip_rollback",
        "shrink_grow",
        "softness",
        "despot_black",
        "despot_white",
        "inside_mask",
        "outside_mask",
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
    // holds. It is no longer pointwise: the spatial stages judge a
    // pixel's matte by its neighbours, so the cost is the multi-pass one and
    // the region of interest is dilated by the sum of the three spatial
    // controls' own hard maxima (100 px pre-blur + 50 px shrink/grow + 100 px
    // softness + the despot's one pixel), exactly as the Gaussian blur sizes
    // its padding from its Radius' hard maximum. At the defaults not
    // one of those passes runs and the old single pointwise pass is what
    // executes.
    cost = Moderate,
    roi = PaddedPx(251.0),
    // The owner's rule for mattes: a keyer carries no Matte row. Its
    // subject is the picture it keys, and a strength matte over a key is a
    // garbage matte — which is a mask's job, not this row's.
    matte = false,
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
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub screen_gain: f32,

    /// Per cent → 0..1: how the two non-screen channels are weighted into the
    /// reference (0 = their min, 100 = their max, 50 = mean).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
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
        hard_max = 100.0,
        unit = Percent
    )]
    pub spill: f32,

    /// How far the picture the key is **judged from** is softened, px@comp
    /// (§2.3), before the matte is measured — Keylight's Screen pre-blur. Grain
    /// and compression noise stop reading as detail; the colour that comes out
    /// is still the sharp original. 0, the default, skips the stage and the
    /// effect keys exactly as it did before.
    #[slider(
        label = "Screen pre-blur",
        min = 0.0,
        max = 20.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Px
    )]
    pub pre_blur: f32,

    /// Per cent → 0..1: matte at/below this maps to 0 (fully keyed), cleaning
    /// residual grey out of the background.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub clip_black: f32,

    /// Per cent → 0..1: matte at/above this maps to 1 (fully kept), filling
    /// holes in the foreground.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub clip_white: f32,

    /// Per cent → 0..1: eases the clips back toward the un-clipped matte,
    /// recovering fine edge detail (0 = full clip, the default).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub clip_rollback: f32,

    /// Marches the matte's edge outward (+) or inward (−), px@comp (§2.3) —
    /// Keylight's Screen shrink/grow. Morphological, not a blur: the
    /// edge moves and stays as crisp as it was. 0, the default, is the neutral.
    #[slider(
        label = "Screen shrink/grow",
        min = -10.0,
        max = 10.0,
        default = 0.0,
        hard_min = -50.0,
        hard_max = 50.0,
        unit = Px
    )]
    pub shrink_grow: f32,

    /// How far the matte itself is blurred, px@comp (§2.3) — and only the
    /// matte, so the picture keeps its own sharpness. 0, the default,
    /// is the neutral.
    #[slider(
        label = "Screen softness",
        min = 0.0,
        max = 20.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Px
    )]
    pub softness: f32,

    /// Per cent → 0..1: how far an isolated **dark** speck — a pinhole in the
    /// foreground — is lifted to the darkest of its neighbours. A pixel
    /// on a real edge has a neighbour on its own side, so edges are left alone.
    #[slider(
        label = "Despot black",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub despot_black: f32,

    /// Per cent → 0..1: how far an isolated **bright** speck — a fleck left in
    /// the keyed background — is dropped to the brightest of its neighbours.
    #[slider(
        label = "Despot white",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub despot_white: f32,

    /// A mask of this layer's whose inside is forced **opaque**, whatever the
    /// key made of it — the hold-out for a patch of foreground the
    /// screen colour eats. Unset is nothing held in, never the first mask: a
    /// garbage matte nobody asked for would be a keyer that stopped keying.
    #[mask_path(label = "Inside mask", self_default = false)]
    pub inside_mask: bool,

    /// A mask of this layer's whose inside is forced **transparent** —
    /// the shape drawn round a light stand, a rig, the edge of the screen.
    /// Unset is nothing cut out, for the same reason.
    #[mask_path(label = "Outside mask", self_default = false)]
    pub outside_mask: bool,

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
        hard_max = 100.0,
        unit = Percent
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
            // The three spatial widths arrive already taken to the raster by
            // the resolve step (they are declared `Px`), so this only floors
            // them; shrink/grow keeps its sign, which is what says shrink from
            // grow.
            pre_blur: self.pre_blur.max(0.0),
            shrink_grow: self.shrink_grow,
            softness: self.softness.max(0.0),
            despot_black: (self.despot_black / 100.0).clamp(0.0, 1.0),
            despot_white: (self.despot_white / 100.0).clamp(0.0, 1.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

impl MatteKey {
    /// Raster pixels per comp pixel (§2.3), pushed at resolve because the mask
    /// carriage hands its vertices over in px@comp and the garbage mattes are
    /// filled in the raster. Never a panel row — the same
    /// derived value Scribble, Stroke and Vegas already carry, under the same
    /// id.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag so no
    /// caller has to know the id.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }
}

/// Matte key's behaviour.
pub struct MatteKeyDef;

impl EffectDef for MatteKeyDef {
    fn schema(&self) -> &'static EffectSchema {
        &<MatteKey as EffectMetadata>::SCHEMA
    }

    /// The spatial pipeline, with **no** garbage masks: the geometry arrives
    /// beside the op rather than in the bag, the shape Scribble and Set matte
    /// have, so the trait's own entry point cannot see it. Everything else —
    /// pre-blur, shrink/grow, softness, despot — is numbers, so it runs here,
    /// and with the defaults `matte_key_spatial` hands straight over to the
    /// pointwise keyer. The §1.6 oracle for the mask rows themselves is
    /// [`cpu::matte_key_spatial`] called directly from the lumit-gpu test,
    /// which can build a polyline.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let blank = cpu::MaskFillParams::blank();
        cpu::matte_key_spatial(rgba, w, h, &MatteKey::read(p).packed(), &blank, &blank);
    }

    /// The raster factor, so the garbage mattes are filled where their masks
    /// are drawn.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(MatteKey::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

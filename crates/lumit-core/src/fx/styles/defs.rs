//! The nine style declarations (docs/impl/layer-styles.md §1's parameter
//! table), in §2's painting order.
//!
//! **In plain terms.** Nine little control panels, written the way every effect
//! in the catalogue is written, so that keyframes, expressions, drivers, undo
//! and the panel widgets all arrive with them and none of it had to be built
//! twice. **Seven of the nine render** (§8): Drop shadow, Outer glow, Gradient
//! overlay, Colour overlay, Inner glow, Inner shadow and Stroke. Satin and Bevel
//! and emboss are declared but not rendered in v1, so that an imported project
//! keeps its data and no file ever has to migrate when their kernels land.
//!
//! Four of the seven are **one kernel** — the drop-shadow core, generalised.
//! Outer glow is that kernel at zero offset; Inner shadow is it reading the
//! inverted alpha and compositing inside the shape; Inner glow is the inner
//! shadow at zero offset, with the Centre source declining the inversion. That
//! is why each `packed` below is short: the arithmetic lives in one place and
//! these are the four uniforms that tell it apart.
//!
//! Three conventions run through the lot:
//!
//! - **Angles are measured from straight up, clockwise**, the convention
//!   `effects/drop_shadow.rs` pinned — so 135° is the familiar shadow down and
//!   to the right, and an AE import converts once at the seam (§7).
//! - **Distances and sizes are px@comp** (docs/08 §2.3), never a per cent of the
//!   frame: a shadow eight pixels from a title is eight pixels from it in a 4K
//!   comp too.
//! - **No Matte row** (`matte = false`). Every *effect* gets a Matte, but
//!   a style dresses the layer's own alpha; gating a shadow by another layer is
//!   an effect's job, and the injected row would put a slot on the render's
//!   parallel matte list that nothing fills.
//!
//! A word on **Opacity**. The two overlay styles have no separate Opacity row:
//! their Mix row is labelled "Opacity" and *is* it. That is not a saving, it is
//! the only place the number can go and still mean what Photoshop means — the
//! seam applies Mix **after** the style's Blend mode, which is exactly
//! "blend the overlay in, then take this much of the result", whereas a second
//! opacity inside the kernel would fade the overlay *before* it was blended and
//! give a different picture on every mode but Normal. The styles that draw new
//! pixels rather than recolouring existing ones — the shadows, the glows, the
//! stroke — keep their own Opacity, because there it means how dark the shadow
//! is and not how much of the style you take.

use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_drop_shadow",
    label = "Drop shadow",
    version = 1,
    category = Stylise,
    cost = Moderate,
    // The shadow reaches Distance + Softness past every edge, and there is no
    // honest smaller bound without reading both sliders.
    roi = FullFrame,
    // Built and composited premultiplied throughout, exactly as the effect is:
    // `colour · opacity · k` IS "this colour at this coverage".
    premultiplied = true,
    matte = false,
)]
pub struct DropShadowStyle {
    /// The shadow's colour, scene-linear. Open above 1 so a coloured light's
    /// shadow can be typed brighter than white; the alpha lane is ignored.
    #[colour(label = "Shadow colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub shadow_colour: [f32; 4],

    /// How dark the shadow is, per cent. Photoshop's default, and a shadow at
    /// full opacity reads as a hole rather than as a shadow.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// Where the light is coming from, degrees, measured from straight up and
    /// turning clockwise.
    #[dial(default = 135.0, step = 15.0)]
    pub direction: f32,

    /// How far the shadow slides, px@comp.
    #[slider(min = 0.0, max = 500.0, default = 12.0, hard_min = 0.0, unit = Px)]
    pub distance: f32,

    /// The gaussian half-width the shape is softened by, px@comp. Independent of
    /// Distance on purpose: a shadow can be moved without changing how sharp it
    /// is, which is what animating one usually wants.
    #[slider(min = 0.0, max = 250.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    /// How far the softened edge is pushed back out, per cent — Photoshop's
    /// Spread. 0 leaves the gaussian exactly as it fell (and takes no branch at
    /// all, so a shadow at 0 is the picture the effect has always drawn, to the
    /// bit); 100 remaps the whole ramp to a hard edge, which is how a soft
    /// shadow becomes a solid drop.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub spread: f32,

    /// Whether the layer's own shape removes the shadow before the layer is
    /// composited over it — Photoshop's "Layer knocks out drop shadow", and on
    /// by default as it is there.
    ///
    /// On an opaque layer the two settings are the same picture: the shadow is
    /// hidden behind the shape either way. The difference is a **semi-
    /// transparent** layer, where off lets the shadow show through in proportion
    /// to the transparency, and on takes it away.
    #[toggle(label = "Layer knocks out shadow", default = true)]
    pub knockout: bool,

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

impl DropShadowStyle {
    /// The bundle both render paths consume — the Drop shadow effect's own
    /// bundle, generalised (docs/impl/layer-styles.md §4).
    ///
    /// The one trigonometric pair is spent here, host-side, so neither render
    /// path evaluates a sine; `sin θ, −cos θ` is "from straight up, clockwise"
    /// on a raster whose y grows downward.
    #[must_use]
    pub fn packed(self) -> cpu::DropShadowParams {
        let theta = self.direction.to_radians();
        let (sin, cos) = theta.sin_cos();
        cpu::DropShadowParams {
            colour: [
                self.shadow_colour[0],
                self.shadow_colour[1],
                self.shadow_colour[2],
            ],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            offset: [self.distance * sin, self.distance * -cos],
            softness_px: self.softness.max(0.0),
            shadow_only: false,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            spread_scale: cpu::spread_scale(self.spread),
            knockout: self.knockout,
            invert: false,
            inner: false,
        }
    }
}

pub struct DropShadowStyleDef;

impl EffectDef for DropShadowStyleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<DropShadowStyle as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::drop_shadow(rgba, w, h, &DropShadowStyle::read(p).packed());
    }

    fn spread_px(&self, p: Params<'_>) -> [f32; 2] {
        DropShadowStyle::read(p).packed().spread()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_colour_overlay",
    label = "Colour overlay",
    version = 1,
    category = Stylise,
    cost = Trivial,
    roi = Exact,
    // `colour · a` IS the premultiplied form of "this colour at this coverage",
    // and the source colour is never read, so there is no round trip to lose
    // precision in (docs/08 §2.2).
    premultiplied = true,
    matte = false,
)]
pub struct ColourOverlay {
    /// The colour the layer's coverage is flooded with. Scene-linear and open
    /// above 1; the alpha lane is ignored.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// How much of the overlay is taken, per cent — this style's **Opacity**, and
    /// the host-uniform Mix row under the label the panel shows (see the module
    /// note on why the two are one number here).
    #[slider(
        label = "Opacity",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl ColourOverlay {
    /// The colour's three channels and the mix — the Fill effect's own bundle,
    /// so the CPU reference and the WGSL kernel read one expression.
    #[must_use]
    pub fn packed(self) -> ([f32; 3], f32) {
        (
            [self.colour[0], self.colour[1], self.colour[2]],
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

pub struct ColourOverlayDef;

impl EffectDef for ColourOverlayDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ColourOverlay as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (colour, mix) = ColourOverlay::read(p).packed();
        cpu::fill(rgba, colour, mix);
    }
}

/// Outer glow — §2's entry 2, behind the layer and in front of its shadow.
///
/// The drop-shadow kernel with zero offset: blur the alpha, tint it, composite
/// it under. Not the Glow *effect*, which is a bright-pass bloom on colour and a
/// different machine entirely.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_outer_glow",
    label = "Outer glow",
    version = 1,
    category = Stylise,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    matte = false,
)]
pub struct OuterGlow {
    /// The glow's colour, scene-linear. A warm near-white is the one that reads
    /// as light rather than as a coloured halo.
    #[colour(label = "Glow colour", default = [1.0, 0.94, 0.72, 1.0], max = 4.0)]
    pub glow_colour: [f32; 4],

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// The gaussian half-width the shape is softened by, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 12.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub spread: f32,

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

impl OuterGlow {
    /// The drop-shadow bundle at **zero offset** (docs/impl/layer-styles.md §4).
    ///
    /// There is no direction to spend a sine on: a glow that leaned would be a
    /// shadow. Everything else is the shadow's own arithmetic, which is the
    /// point — the two styles are one kernel and the pixel test says so.
    #[must_use]
    pub fn packed(self) -> cpu::DropShadowParams {
        cpu::DropShadowParams {
            colour: [
                self.glow_colour[0],
                self.glow_colour[1],
                self.glow_colour[2],
            ],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            offset: [0.0, 0.0],
            softness_px: self.softness.max(0.0),
            shadow_only: false,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            spread_scale: cpu::spread_scale(self.spread),
            // A glow is not knocked out by its own layer: it is meant to be seen
            // through a semi-transparent one, which is half of what makes it
            // read as light rather than as a shadow with a colour.
            knockout: false,
            invert: false,
            inner: false,
        }
    }
}

pub struct OuterGlowDef;

impl EffectDef for OuterGlowDef {
    fn schema(&self) -> &'static EffectSchema {
        &<OuterGlow as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::drop_shadow(rgba, w, h, &OuterGlow::read(p).packed());
    }

    fn spread_px(&self, p: Params<'_>) -> [f32; 2] {
        OuterGlow::read(p).packed().spread()
    }
}

/// Gradient overlay — §2's entry 4, interior, under the colour overlay.
///
/// Two colour stops, matching the Gradient effect's own two-stop model: a ramp
/// editor is that effect's upgrade to inherit, not this one's.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_gradient_overlay",
    label = "Gradient overlay",
    version = 1,
    category = Stylise,
    cost = Cheap,
    roi = Exact,
    premultiplied = true,
    matte = false,
)]
pub struct GradientOverlay {
    #[colour(label = "Colour A", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour_a: [f32; 4],

    #[colour(label = "Colour B", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub colour_b: [f32; 4],

    /// Linear runs the ramp along the angle; Radial measures it out from the
    /// middle of the layer's own bounds.
    #[choice(options = ["Linear", "Radial"], default = 0)]
    pub gradient_type: u32,

    /// Which way the ramp runs, degrees, from straight up and clockwise.
    #[dial(default = 180.0, step = 15.0)]
    pub angle: f32,

    /// How far the ramp is stretched across the layer, per cent. 100 spans the
    /// layer exactly; more spreads the interesting part over a smaller band.
    #[slider(
        min = 1.0,
        max = 400.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub scale: f32,

    #[toggle(default = false)]
    pub reverse: bool,

    /// How much of the overlay is taken, per cent — this style's Opacity (see
    /// the module note).
    #[slider(
        label = "Opacity",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl GradientOverlay {
    #[must_use]
    pub fn packed(self, w: u32, h: u32) -> cpu::gradient::GradientParams {
        let theta = self.angle.to_radians();
        let (sin, cos) = theta.sin_cos();
        // "From straight up, clockwise" on a raster whose y grows downward —
        // the convention every angle in this file follows.
        let dir = [sin, -cos];
        let (wf, hf) = (w as f32, h as f32);
        let s = (self.scale / 100.0).max(0.01);
        let centre = [wf * 0.5, hf * 0.5];
        let (start, axis) = if self.gradient_type == 1 {
            // Radial reads only the axis's LENGTH, so the direction is spent on
            // nothing and the radius is the half-diagonal — the distance at
            // which a ramp centred on the raster has reached every corner.
            ([centre[0], centre[1]], [wf.hypot(hf) * 0.5 * s, 0.0])
        } else {
            let half = (dir[0].abs() * wf + dir[1].abs() * hf) * 0.5 * s;
            (
                [centre[0] - dir[0] * half, centre[1] - dir[1] * half],
                [dir[0] * half * 2.0, dir[1] * half * 2.0],
            )
        };
        let (a, b) = if self.reverse {
            (self.colour_b, self.colour_a)
        } else {
            (self.colour_a, self.colour_b)
        };
        cpu::gradient::GradientParams {
            shape: u32::from(self.gradient_type == 1),
            c0: [a[0], a[1], a[2], 1.0],
            c1: [b[0], b[1], b[2], 1.0],
            // The style has no Scatter: dither is the generator's cure for
            // banding across a whole frame, and an overlay rides whatever the
            // layer under it already does.
            scatter: 0.0,
            seed: 0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            clip_to_alpha: true,
            ..cpu::gradient::GradientParams::aimed(start, axis)
        }
    }
}

pub struct GradientOverlayDef;

impl EffectDef for GradientOverlayDef {
    fn schema(&self) -> &'static EffectSchema {
        &<GradientOverlay as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let display = crate::fx::effects::gradient::in_display(p);
        let g = GradientOverlay::read(p).packed(w, h).in_display(display);
        cpu::gradient::gradient(rgba, w, h, &g);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_satin",
    label = "Satin",
    version = 1,
    category = Stylise,
    cost = Moderate,
    roi = Exact,
    premultiplied = true,
    matte = false,
)]
pub struct Satin {
    #[colour(label = "Satin colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub satin_colour: [f32; 4],

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    #[dial(default = 135.0, step = 15.0)]
    pub direction: f32,

    /// How far they are thrown, px@comp.
    #[slider(min = 0.0, max = 500.0, default = 11.0, hard_min = 0.0, unit = Px)]
    pub distance: f32,

    /// The gaussian half-width the sheen is softened by, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 14.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    #[toggle(default = false)]
    pub invert: bool,

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

pub struct SatinDef;

impl EffectDef for SatinDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Satin as EffectMetadata>::SCHEMA
    }
}

/// Inner glow — §2's entry 7, interior.
///
/// Inner shadow with zero offset; the Centre source inverts the sense of the
/// distance, so the light comes from the middle of the shape instead of its rim.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_inner_glow",
    label = "Inner glow",
    version = 1,
    category = Stylise,
    cost = Moderate,
    roi = Exact,
    premultiplied = true,
    matte = false,
)]
pub struct InnerGlow {
    #[colour(label = "Glow colour", default = [1.0, 0.94, 0.72, 1.0], max = 4.0)]
    pub glow_colour: [f32; 4],

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// The gaussian half-width the glow is softened by, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 12.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub choke: f32,

    #[choice(options = ["Edge", "Centre"], default = 0)]
    pub source: u32,

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

impl InnerGlow {
    /// The inner shadow's bundle at **zero offset**
    /// (docs/impl/layer-styles.md §4).
    ///
    /// **The Source choice is the inversion, and nothing else.** Edge reads the
    /// softened picture of what the shape is *not*, which is brightest against
    /// the rim and dies away inward; Centre declines that inversion and reads
    /// the shape itself, which is brightest in the middle and falls off toward
    /// the rim. Those two are exact complements inside the shape, which is what
    /// "the Centre source inverts the distance sense" means once the arithmetic
    /// is written down.
    #[must_use]
    pub fn packed(self) -> cpu::DropShadowParams {
        cpu::DropShadowParams {
            colour: [
                self.glow_colour[0],
                self.glow_colour[1],
                self.glow_colour[2],
            ],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            offset: [0.0, 0.0],
            softness_px: self.softness.max(0.0),
            shadow_only: false,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            spread_scale: cpu::spread_scale(self.choke),
            knockout: false,
            invert: self.source == 0,
            inner: true,
        }
    }
}

pub struct InnerGlowDef;

impl EffectDef for InnerGlowDef {
    fn schema(&self) -> &'static EffectSchema {
        &<InnerGlow as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::drop_shadow(rgba, w, h, &InnerGlow::read(p).packed());
    }
}

/// Inner shadow — §2's entry 8, interior.
///
/// The drop-shadow kernel on the **inverted** alpha, clipped back inside the
/// shape and composited over it.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_inner_shadow",
    label = "Inner shadow",
    version = 1,
    category = Stylise,
    cost = Moderate,
    roi = Exact,
    premultiplied = true,
    matte = false,
)]
pub struct InnerShadow {
    #[colour(label = "Shadow colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub shadow_colour: [f32; 4],

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// Where the light is coming from, degrees, from straight up and clockwise.
    #[dial(default = 135.0, step = 15.0)]
    pub direction: f32,

    /// How far the shadow slides inward, px@comp.
    #[slider(min = 0.0, max = 500.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub distance: f32,

    /// The gaussian half-width the shape is softened by, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub choke: f32,

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

impl InnerShadow {
    /// The drop-shadow bundle with both halves of the inner wrapper on
    /// (docs/impl/layer-styles.md §4).
    ///
    /// `invert` reads the coverage from what the shape is *not*, so sliding it
    /// by the offset walks the outside world **into** the shape; `inner` keeps
    /// the result there, carrying the layer's own colour toward the shadow's and
    /// leaving its alpha alone. The offset is the drop shadow's own, which is
    /// why the dark band lands on the side the light comes from.
    #[must_use]
    pub fn packed(self) -> cpu::DropShadowParams {
        let theta = self.direction.to_radians();
        let (sin, cos) = theta.sin_cos();
        cpu::DropShadowParams {
            colour: [
                self.shadow_colour[0],
                self.shadow_colour[1],
                self.shadow_colour[2],
            ],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            offset: [self.distance * sin, self.distance * -cos],
            softness_px: self.softness.max(0.0),
            shadow_only: false,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            // Choke is Spread's twin on an interior style: the same remap of the
            // same ramp, pulling the softened edge back in rather than out.
            spread_scale: cpu::spread_scale(self.choke),
            knockout: false,
            invert: true,
            inner: true,
        }
    }
}

pub struct InnerShadowDef;

impl EffectDef for InnerShadowDef {
    fn schema(&self) -> &'static EffectSchema {
        &<InnerShadow as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::drop_shadow(rgba, w, h, &InnerShadow::read(p).packed());
    }
}

/// Stroke — §2's entry 9, straddling the edge and over the interiors.
///
/// An **alpha-contour** stroke, and deliberately not the Stroke *effect*, which
/// paints a mask's own path. Its kernel is a separable dilate/erode of the
/// alpha, the edge band being the difference of the two.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_stroke",
    label = "Stroke",
    version = 1,
    category = Stylise,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    matte = false,
)]
pub struct StrokeStyle {
    #[colour(label = "Stroke colour", default = [1.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub stroke_colour: [f32; 4],

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// How thick the stroke is, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 3.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    #[choice(options = ["Outside", "Inside", "Centre"], default = 0)]
    pub position: u32,

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

impl StrokeStyle {
    /// The contour bundle both paths consume
    /// (docs/impl/layer-styles.md §4).
    ///
    /// **Position is spent here into two half-widths**, so no kernel ever
    /// branches on the choice: Outside fattens the shape by the whole Size and
    /// leaves the thin copy where it was, Inside thins it by the whole Size and
    /// leaves the fat copy, and Centre does half of each. The band is the
    /// difference of the two copies, so those three lines are the whole of what
    /// "which side of the edge" means.
    #[must_use]
    pub fn packed(self) -> cpu::StrokeContourParams {
        let size = self.size.max(0.0);
        let (grow_px, shrink_px) = match self.position {
            1 => (0.0, size),
            2 => (size * 0.5, size * 0.5),
            _ => (size, 0.0),
        };
        cpu::StrokeContourParams {
            colour: [
                self.stroke_colour[0],
                self.stroke_colour[1],
                self.stroke_colour[2],
            ],
            opacity: (self.opacity / 100.0).clamp(0.0, 1.0),
            grow_px,
            shrink_px,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

pub struct StrokeStyleDef;

impl EffectDef for StrokeStyleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<StrokeStyle as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::stroke_contour(rgba, w, h, &StrokeStyle::read(p).packed());
    }

    fn spread_px(&self, p: Params<'_>) -> [f32; 2] {
        let stroke = StrokeStyle::read(p).packed();
        let reach = if stroke.opacity > 0.0 && stroke.mix > 0.0 {
            stroke.grow_px
        } else {
            0.0
        };
        [reach, reach]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "style_bevel_emboss",
    label = "Bevel and emboss",
    version = 1,
    category = Stylise,
    cost = Heavy,
    roi = FullFrame,
    premultiplied = true,
    matte = false,
)]
pub struct BevelEmboss {
    #[choice(
        options = ["Outer bevel", "Inner bevel", "Emboss", "Pillow emboss", "Stroke emboss"],
        default = 1
    )]
    pub bevel_style: u32,

    #[choice(options = ["Smooth", "Chisel hard", "Chisel soft"], default = 0)]
    pub technique: u32,

    #[slider(
        min = 1.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Percent
    )]
    pub depth: f32,

    #[choice(options = ["Up", "Down"], default = 0)]
    pub direction: u32,

    /// How wide the relief band is, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 5.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// How far the relief is blurred after it is built, px@comp.
    #[slider(min = 0.0, max = 250.0, default = 0.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    /// Where the light is coming from, degrees, from straight up and clockwise.
    #[dial(default = 135.0, step = 15.0)]
    pub angle: f32,

    /// How high above the surface the light sits, degrees. 90 is straight
    /// overhead and flattens the relief; 0 is a raking light.
    #[slider(min = 0.0, max = 90.0, default = 30.0, hard_min = 0.0, hard_max = 90.0)]
    pub altitude: f32,

    /// The colour of the lit face, scene-linear.
    #[colour(label = "Highlight colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub highlight_colour: [f32; 4],

    #[slider(
        label = "Highlight opacity",
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub highlight_opacity: f32,

    /// The colour of the shaded face, scene-linear.
    #[colour(label = "Shadow colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub shadow_colour: [f32; 4],

    #[slider(
        label = "Shadow opacity",
        min = 0.0,
        max = 100.0,
        default = 75.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub shadow_opacity: f32,

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

pub struct BevelEmbossDef;

impl EffectDef for BevelEmbossDef {
    fn schema(&self) -> &'static EffectSchema {
        &<BevelEmboss as EffectMetadata>::SCHEMA
    }
}

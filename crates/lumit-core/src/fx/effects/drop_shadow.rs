//! Drop shadow (docs/08 §3.43): the layer's own shape, softened, tinted, moved
//! and drawn underneath it — AE's Drop Shadow.
//!
//! **In plain terms.** The layer already carries its shape in its alpha. This
//! blurs that shape, slides it in the direction of the light, paints it in one
//! colour and puts it *behind* the layer. Direction is measured from straight up
//! and turns clockwise, so 135° is the familiar shadow down and to the right;
//! Distance is how far it slides, Softness how blurred it is, Opacity how dark.
//! Shadow only throws the layer away and keeps the shadow, which is how a
//! shadow is put on a different layer from the thing casting it.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Drop shadow's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "drop_shadow",
    label = "Drop shadow",
    version = 1,
    category = Stylise,
    // It carries a gaussian (docs/08 §3.8's kernel, reused) plus one composite.
    cost = Moderate,
    // The shadow reaches Distance + Softness outside every edge of the shape,
    // and there is no honest smaller bound without reading both sliders.
    roi = FullFrame,
    // The shadow is built and composited in premultiplied form throughout:
    // `colour · opacity · k` IS "this colour at this coverage" (§3.34's
    // reasoning), and "source over shadow" is the premultiplied composite.
    premultiplied = true,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the shadow's Opacity per pixel, read where the shadow falls:          white casts it in full, black casts none",
    ),
)]
pub struct DropShadow {
    /// The shadow's colour, scene-linear. Open above 1 so a coloured light's
    /// shadow can be typed brighter than white if a comp wants it; the alpha
    /// lane is ignored, as on every colour parameter in the catalogue — Opacity
    /// below is how much of it there is.
    #[colour(label = "Shadow colour", default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub shadow_colour: [f32; 4],

    /// How dark the shadow is, per cent. AE's default, and a shadow at full
    /// opacity reads as a hole rather than as a shadow.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// Where the light is coming from, degrees. **Measured from straight up,
    /// turning clockwise** — AE's convention and the one that reads as a light
    /// direction. It is the only angle in the catalogue whose zero is not the
    /// +x axis; the *turn* is still clockwise on screen like every other
    /// (docs/08 §3.41), which is what the shared rule actually says.
    #[dial(default = 135.0, step = 15.0)]
    pub direction: f32,

    /// How far the shadow slides, px@comp (§2.3).
    #[slider(min = 0.0, max = 500.0, default = 12.0, hard_min = 0.0, unit = Px)]
    pub distance: f32,

    /// The gaussian half-width the shape is softened by, px@comp. Independent
    /// of Distance on purpose: a shadow can then be moved without changing how
    /// sharp it is, which is what animating one usually wants.
    #[slider(min = 0.0, max = 250.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub softness: f32,

    /// Keep the shadow and throw the layer away — how a shadow ends up on a
    /// different layer from the thing casting it.
    #[toggle(label = "Shadow only", default = false)]
    pub shadow_only: bool,

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

impl DropShadow {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    ///
    /// The one trigonometric pair is taken here, host-side, for §1.6's reason —
    /// neither render path evaluates its own sine. `sin θ, −cos θ` is "measured
    /// from straight up, clockwise" on a raster whose y grows downward.
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
            shadow_only: self.shadow_only,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Drop shadow's behaviour.
pub struct DropShadowDef;

impl EffectDef for DropShadowDef {
    fn schema(&self) -> &'static EffectSchema {
        &<DropShadow as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::drop_shadow(rgba, w, h, &DropShadow::read(p).packed());
    }
}

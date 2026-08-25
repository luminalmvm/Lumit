//! Glow (docs/08 §3.3): exposure-aware bloom in scene-linear light — a
//! bright-pass with a soft knee, a wide gaussian on the leftover light, and an
//! additive recombine.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Glow's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "glow",
    label = "Glow",
    version = 1,
    category = Stylise,
    cost = Moderate,
    // Radius is raw px@comp (K-135), unbounded above, so a tight %-diag padding
    // cannot be declared statically across every comp resolution — full-frame is
    // the safe static bound (mirroring Chromatic aberration's own px@comp
    // parameter).
    roi = FullFrame,
    // K-395: the glow claims the injected Matte row inside its own maths — the
    // matte gates the bright pass, so it decides which pixels are *allowed to
    // glow*, not how much of a finished glow survives. The generic strength
    // dissolve does not also run.
    matte = (
        "matte",
        "gates which pixels may seed the halo, before the bright pass: light \
         only escapes from where the matte is bright, but it still spills \
         outward across dark matte — which fading the finished glow cannot do",
    ),
)]
pub struct Glow {
    /// Linear-light value above which pixels bloom. The K-090 one-sided hard
    /// range made concrete: clamped at zero below, unbounded above — HDR values
    /// beyond the slider are legal and glow harder (§2.1). Default 0.8 so
    /// highlights just shy of white already bloom on a fresh instance.
    #[slider(min = 0.0, max = 4.0, default = 0.8, hard_min = 0.0, unit = Raw)]
    pub threshold: f32,

    /// Soft-knee width: the threshold's onset is eased by a smoothstep over
    /// ±knee around it (§3.3 step 1), so the bloom fades in rather than snapping
    /// on. The id stays `knee` — a stable identifier, addressed by expressions
    /// and saved projects — while the panel reads "Softness".
    #[slider(
        label = "Softness",
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub knee: f32,

    /// px@comp (§2.3, K-135): the halo gaussian's half-width in real pixels,
    /// clamped at zero below and unbounded above, so a wide bloom is a matter of
    /// typing a larger number rather than hitting a cap. Declared `Px`, so the
    /// resolve step scales it by the preview factor and the generic rescale moves
    /// it again — what the old arm and `rescale_px` did between them.
    #[slider(
        min = 0.0,
        max = 200.0,
        default = 24.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub radius: f32,

    /// Gain on the added halo; 0 is the effect's neutral point (bit-exact
    /// passthrough, pinned by test).
    #[slider(min = 0.0, max = 10.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub intensity: f32,

    /// The halo's colour, scene-linear — HDR tints are legal.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub tint: [f32; 4],

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

impl Glow {
    /// Radius, threshold, knee, intensity, tint and mix, clamped exactly as the
    /// old resolve arm clamped them (docs/impl/effect-registry.md §2.4). The
    /// radius arrives already scaled by the §2.3 preview factor, so this only
    /// floors it — the same `.max(0.0)` the arm applied to the same product.
    /// Both render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> (f32, f32, f32, f32, [f32; 4], f32) {
        (
            self.radius.max(0.0),
            self.threshold.max(0.0),
            self.knee.clamp(0.0, 1.0),
            self.intensity.max(0.0),
            self.tint,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Glow's behaviour.
pub struct GlowDef;

impl EffectDef for GlowDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Glow as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (radius_px, threshold, knee, intensity, tint, mix) = Glow::read(p).packed();
        // No matte through the single-buffer dispatcher: it carries one
        // picture, and this effect's matte is a second one (the K-387 rule the
        // depth pass and the LUT already follow). The §1.6 oracle for the matted
        // path is `cpu::glow` called directly from the lumit-gpu test, which can
        // upload it.
        cpu::glow(
            rgba,
            w,
            h,
            radius_px,
            threshold,
            knee,
            intensity,
            tint,
            mix,
            &[],
        );
    }
}

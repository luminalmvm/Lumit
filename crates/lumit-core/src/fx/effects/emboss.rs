//! Emboss (docs/08 §3.67): the picture as grey relief — AE's Emboss.
//!
//! **In plain terms.** The picture is treated as a surface with height, lit from
//! one direction, and what comes back is the stamped-metal look: a flat grey
//! sheet with the frame's edges raised out of it, bright on the side facing the
//! light and dark on the other. It is grey on purpose — AE's Emboss suppresses
//! colour and so does this — so for a coloured relief, put a §3.24 Tint or a
//! §3.60 Tritone after it.
//!
//! Relief 0 is **not** the identity. With no separation between the two taps
//! there is no relief to see, and the honest answer is the surface with no light
//! on it: flat mid-grey. Mix is what turns the effect down (§3.57's Fractal
//! influence makes the same point from the other side).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Emboss's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "emboss",
    label = "Emboss",
    version = 1,
    category = Stylise,
    // Two bilinear taps a pixel.
    cost = Cheap,
    // Relief's own reach. Its hard maximum is open, so the padding is the
    // slider's 20 px@comp doubled.
    roi = PaddedPx(40.0),
    // §2.2: a difference of premultiplied colour is a difference of the coverage
    // wherever the coverage moves.
    premultiplied = false,
    // The matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Relief per pixel: white raises the full relief, grey a shallower          one, black leaves the sheet flat",
    ),
)]
pub struct Emboss {
    /// Degrees, from straight up and clockwise — §3.43's convention, which is
    /// AE's. This is where the light is, and the slope facing it is the bright
    /// one.
    #[dial(default = 45.0, step = 15.0)]
    pub direction: f32,

    /// px@comp: how far apart the two taps are, which is how thick the relief
    /// reads. 0 is flat mid-grey, not the identity (see the module note).
    #[slider(min = 0.0, max = 20.0, default = 2.0, hard_min = 0.0, unit = Px)]
    pub relief: f32,

    /// Per cent: the gain on the difference between the two taps. 0 is flat
    /// mid-grey; above 100 the relief goes to black and white quickly.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub contrast: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    /// This is AE's "Blend With Original".
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

impl Emboss {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4). The
    /// trigonometry happens here, once — the kernel never sees an angle, which
    /// is §3.5's rule and the reason the two paths cannot disagree about a
    /// sine.
    #[must_use]
    pub fn packed(self) -> cpu::EmbossParams {
        let t = self.direction.to_radians();
        let r = self.relief.max(0.0);
        cpu::EmbossParams {
            // Straight up is −y in the raster, and clockwise from there is +x.
            offset: [r * t.sin(), -r * t.cos()],
            contrast: (self.contrast / 100.0).max(0.0),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Emboss's behaviour.
pub struct EmbossDef;

impl EffectDef for EmbossDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Emboss as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::emboss(rgba, w, h, &Emboss::read(p).packed());
    }
}

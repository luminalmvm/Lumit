//! Threshold (docs/08 §3.59): every pixel to black or to white — AE's
//! Threshold.
//!
//! **In plain terms.** Each pixel is asked one question — is it brighter than
//! the Level? — and comes back white if it is and black if it is not. It is how
//! a photograph becomes a stencil, a logo becomes a matte, and a face becomes
//! the two-tone poster.
//!
//! Two small departures from the obvious. The Level is measured on the
//! *perceptual* position of the pixel's brightness rather than on the light
//! itself, so 50 lands on the grey a person points at (§3.58's square root
//! again). And the crossing is never a bare step: it has a floor a thousandth of
//! the range wide, which is far under a pixel on any real edge but is enough to
//! antialias the cut — and enough that the CPU and the GPU cannot disagree about
//! a pixel sitting exactly on the line.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Threshold's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "threshold",
    label = "Threshold",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // §2.2: the decision is about the pixel's own colour, not about how much of
    // it there is.
    premultiplied = false,
)]
pub struct Threshold {
    /// Per cent: where the cut sits on the perceptual tone range. 50 is
    /// mid-grey.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub level: f32,

    /// Per cent of the tone range the crossing is spread over. 0 is AE's hard
    /// cut (floored at a thousandth, so it is still antialiased); raise it for a
    /// gradient between the two tones. Not an AE control (K-401), and neutral at
    /// its default.
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0)]
    pub softness: f32,

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

impl Threshold {
    /// The narrowest the crossing is allowed to be, in tone. A thousandth of
    /// the range is well under a pixel across any real edge — so Softness 0
    /// reads as the hard cut AE gives and still arrives antialiased — and it is
    /// what stops the smoothstep dividing by zero (§3.57's rule, second outing).
    pub const MIN_HALF_WIDTH: f32 = 1e-3;

    /// The cut's position, its half-width and the mix
    /// (docs/impl/effect-registry.md §2.4).
    #[must_use]
    pub fn packed(self) -> (f32, f32, f32) {
        (
            (self.level / 100.0).clamp(0.0, 1.0),
            (self.softness / 200.0).max(Self::MIN_HALF_WIDTH),
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Threshold's behaviour.
pub struct ThresholdDef;

impl EffectDef for ThresholdDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Threshold as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (level, hw, mix) = Threshold::read(p).packed();
        cpu::threshold(rgba, level, hw, mix);
    }
}

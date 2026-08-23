//! Warp (docs/08 §3.56): the thirteen bend presets — AE's Warp, which is
//! Photoshop's.
//!
//! **In plain terms.** Thirteen named ways of bending a picture, each one a
//! single slider deep. Pick the shape you want by its name — Arc, Flag, Fisheye,
//! Twist — turn Bend up, and the two Distortion sliders taper the result as if
//! it were seen at an angle. It is the effect to reach for when the bend is a
//! *look* rather than something to be dragged into place; §3.55 is the one for
//! dragging.
//!
//! One kernel runs all thirteen: the frame is normalised to −1..1 on each axis,
//! the chosen style moves the sample there, and the **difference** is carried
//! back to pixels — which is what makes Bend 0 the bit-exact identity rather
//! than a rebuilt coordinate a rounding away from it.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Warp's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "warp",
    label = "Warp",
    version = 1,
    category = Distortion,
    // A handful of arithmetic and one bilinear tap a pixel.
    cost = Cheap,
    // Every style can pull from anywhere in the frame.
    roi = FullFrame,
    premultiplied = true,
    // K-427: the matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Bend and both distortions per pixel: white bends the full \
         amount, grey less, black not at all",
    ),
)]
pub struct Warp {
    /// Which bend. Photoshop's thirteen, by their own names — people reach for
    /// these by look, not by formula (§3.56's third note).
    #[choice(
        label = "Style",
        options = [
            "Arc",
            "Arc upper",
            "Arc lower",
            "Arch",
            "Bulge",
            "Flag",
            "Wave",
            "Fish",
            "Rise",
            "Fisheye",
            "Inflate",
            "Squeeze",
            "Twist"
        ],
        default = 0
    )]
    pub style: u32,

    /// How much of the style there is, per cent. 0 is the exact identity for
    /// every style; the sign turns the bend the other way.
    #[slider(
        min = -100.0,
        max = 100.0,
        default = 50.0,
        hard_min = -100.0,
        hard_max = 100.0
    )]
    pub bend: f32,

    /// Per cent: tapers the picture between its left and right edges, as if the
    /// bent shape were turned away from the camera about a vertical axis.
    #[slider(
        label = "Horizontal distortion",
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0
    )]
    pub horizontal_distortion: f32,

    /// Per cent: the same taper between top and bottom.
    #[slider(
        label = "Vertical distortion",
        min = -100.0,
        max = 100.0,
        default = 0.0,
        hard_min = -100.0,
        hard_max = 100.0
    )]
    pub vertical_distortion: f32,

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

impl Warp {
    /// How far a taper is allowed to go. At 1 the divisor reaches zero at one
    /// edge and the map runs to infinity there; 0.9 leaves a tenth of the frame
    /// still on the near side, which is as extreme as a perspective taper reads
    /// before it stops being a picture.
    pub const MAX_TAPER: f32 = 0.9;

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4).
    /// Three per cents become fractions, the style becomes a code, and nothing
    /// else can be lifted out of the kernel — every style is a function of the
    /// pixel.
    #[must_use]
    pub fn packed(self) -> cpu::WarpParams {
        cpu::WarpParams {
            style: self.style.min(12),
            bend: (self.bend / 100.0).clamp(-1.0, 1.0),
            h_distort: (self.horizontal_distortion / 100.0)
                .clamp(-Self::MAX_TAPER, Self::MAX_TAPER),
            v_distort: (self.vertical_distortion / 100.0).clamp(-Self::MAX_TAPER, Self::MAX_TAPER),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Warp's behaviour.
pub struct WarpDef;

impl EffectDef for WarpDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Warp as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::warp(rgba, w, h, &Warp::read(p).packed());
    }
}

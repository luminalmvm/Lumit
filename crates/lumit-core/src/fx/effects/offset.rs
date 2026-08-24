//! Offset (docs/08 §3.40): the frame slid, wrapping round.
//!
//! **In plain terms.** Push the picture sideways or up and down; whatever leaves
//! one side arrives at the other. Nothing is ever revealed, so there is no edge
//! policy to choose. It is how a seamless texture is repositioned without a
//! seam, and how a scrolling background is made out of one still.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Offset's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "offset",
    label = "Offset",
    version = 1,
    category = Distortion,
    cost = Cheap,
    // The wrap means any output pixel can come from any input pixel.
    roi = FullFrame,
    premultiplied = true,
    // K-427: the matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the shift per pixel, read where the pixel lands: white slides \
         the full distance, grey less, black not at all",
    ),
)]
pub struct Offset {
    /// px@comp: how far the picture moves to the right. **A shift, not AE's
    /// destination point** (§3.40): a shift is what animates sensibly, since a
    /// linear keyframe pair then scrolls at a constant speed.
    #[slider(label = "Shift x", min = -3840.0, max = 3840.0, default = 0.0, unit = Px)]
    pub shift_x: f32,

    /// px@comp: how far the picture moves down; see
    /// [`shift_x`](Self::shift_x).
    #[slider(label = "Shift y", min = -2160.0, max = 2160.0, default = 0.0, unit = Px)]
    pub shift_y: f32,

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

impl Offset {
    /// The shift in raster pixels (already converted by the resolve step) and
    /// the mix. Both render paths read this one method, so the CPU reference and
    /// the WGSL kernel cannot drift apart.
    #[must_use]
    pub fn packed(self) -> ([f32; 2], f32) {
        (
            [self.shift_x, self.shift_y],
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Offset's behaviour.
pub struct OffsetDef;

impl EffectDef for OffsetDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Offset as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (shift, mix) = Offset::read(p).packed();
        cpu::offset(rgba, w, h, shift, mix);
    }
}

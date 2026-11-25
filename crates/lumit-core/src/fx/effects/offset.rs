use crate::fx::{
    cpu, params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "offset",
    label = "Offset",
    version = 1,
    category = Distortion,
    cost = Cheap,
    roi = FullFrame,
    premultiplied = true,
    // The matte scales the displacement, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
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

    #[slider(label = "Shift y", min = -2160.0, max = 2160.0, default = 0.0, unit = Px)]
    pub shift_y: f32,

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

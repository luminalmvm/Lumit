//! Light leak's pass: one dispatch of `fx_light_leak.wgsl`.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One frame of Light leak. Mirrors
/// `lumit_core::fx::effects::light_leak::LightLeakParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightLeakOp {
    /// The band's normal over its width.
    pub normal: [f32; 2],
    /// The picture's centre, raster pixels.
    pub centre: [f32; 2],
    /// One over the picture's height.
    pub inv_h: f32,
    /// The depth of the coarse and the fine noise.
    pub z: [f32; 2],
    /// Colour at the heart, times Amount.
    pub colour: [f32; 3],
    /// Edge colour, times Amount.
    pub colour_2: [f32; 3],
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightLeakParams {
    normal: [f32; 2],
    centre: [f32; 2],
    z: [f32; 2],
    inv_h: f32,
    seed: u32,
    colour: [f32; 4],
    colour_2: [f32; 4],
}

impl FxEngine {
    /// Apply one frame of Light leak to a linear working texture, returning a
    /// new texture of the same size.
    pub fn light_leak(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LightLeakOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-light-leak-out");
        let [r, g, b] = op.colour;
        let [r2, g2, b2] = op.colour_2;
        let p = LightLeakParams {
            normal: op.normal,
            centre: op.centre,
            z: op.z,
            inv_h: op.inv_h,
            seed: op.seed,
            colour: [r, g, b, op.mix],
            colour_2: [r2, g2, b2, 0.0],
        };
        self.dispatch(
            ctx,
            &self.light_leak,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&p),
        );
        out
    }
}

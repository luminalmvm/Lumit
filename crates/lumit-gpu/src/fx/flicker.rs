//! Flicker's pass: one dispatch of `fx_flicker.wgsl`.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One frame of Flicker. Mirrors
/// `lumit_core::fx::effects::flicker::FlickerParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlickerOp {
    /// Light times Colour, per channel.
    pub gain: [f32; 3],
    /// The contrast about a mid grey. 1 leaves it.
    pub contrast: f32,
    /// 0..1, times the whole pixel.
    pub opacity: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlickerParams {
    gain: [f32; 4],
    rest: [f32; 4],
}

impl FxEngine {
    /// Apply one frame of Flicker to a linear working texture, returning a new
    /// texture of the same size.
    pub fn flicker(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FlickerOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-flicker-out");
        let [r, g, b] = op.gain;
        let p = FlickerParams {
            gain: [r, g, b, op.contrast],
            rest: [op.opacity, op.mix, 0.0, 0.0],
        };
        self.dispatch(
            ctx,
            &self.flicker,
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

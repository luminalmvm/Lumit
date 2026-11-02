//! Jolt's passes: the blur through the shared gaussian, across and down at
//! their own radii, then `fx_jolt.wgsl` for the rest.

use crate::GpuContext;

use super::{blur::BlurParams, work_texture, FxEngine};

/// One frame of Jolt. Mirrors `lumit_core::fx::effects::jolt::JoltParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoltOp {
    /// The blur's radius across and down, raster pixels.
    pub blur: [f32; 2],
    /// Each pixel reads the picture at `pixel × zoom + offset`.
    pub zoom: f32,
    /// See [`zoom`](Self::zoom), raster pixels.
    pub offset: [f32; 2],
    /// Red is read this far back and blue this far on, raster pixels.
    pub split: [f32; 2],
    /// The colour mixed in, scene-linear, and how far.
    pub tint: [f32; 4],
    /// The light, multiplied in.
    pub gain: f32,
    /// The whole pixel, multiplied in.
    pub opacity: f32,
    /// 0 Transparent, 1 Repeat, 2 Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct JoltParams {
    offset: [f32; 2],
    split: [f32; 2],
    tint: [f32; 4],
    zoom: f32,
    gain: f32,
    opacity: f32,
    mix_amt: f32,
    edge: u32,
    _pad: [u32; 3],
}

impl FxEngine {
    /// Apply one frame of Jolt to a linear working texture, returning a new
    /// texture of the same size. A pass for each way the blur goes, then one
    /// for everything else. Mix blends back towards `orig`, which is `src`
    /// unless a time hit moved the picture.
    pub fn jolt(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &JoltOp,
    ) -> wgpu::Texture {
        let mut blurred: Option<wgpu::Texture> = None;
        for (dir, radius) in [([1.0, 0.0], op.blur[0]), ([0.0, 1.0], op.blur[1])] {
            if radius <= 0.0 {
                continue;
            }
            let from = blurred.as_ref().unwrap_or(src);
            let to = work_texture(ctx, w, h, "fx-jolt-blur");
            let p = BlurParams {
                dir,
                radius,
                sigma: (radius * 0.5).max(1e-3),
                edge: op.edge,
                mix_amt: 1.0,
                matte_on: 0.0,
                _pad0: 0.0,
                field: [[0.0; 4]; 3],
            };
            self.dispatch(
                ctx,
                &self.blur,
                from,
                from,
                &to,
                w,
                h,
                bytemuck::bytes_of(&p),
            );
            if let Some(spent) = blurred.replace(to) {
                ctx.recycle(spent);
            }
        }
        let out = work_texture(ctx, w, h, "fx-jolt-out");
        let p = JoltParams {
            offset: op.offset,
            split: op.split,
            tint: op.tint,
            zoom: op.zoom,
            gain: op.gain,
            opacity: op.opacity,
            mix_amt: op.mix,
            edge: op.edge,
            _pad: [0; 3],
        };
        let from = blurred.as_ref().unwrap_or(src);
        self.dispatch(
            ctx,
            &self.jolt,
            from,
            orig,
            &out,
            w,
            h,
            bytemuck::bytes_of(&p),
        );
        if let Some(spent) = blurred {
            ctx.recycle(spent);
        }
        out
    }
}

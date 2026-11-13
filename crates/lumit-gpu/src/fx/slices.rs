//! Slices' pass: one dispatch of `fx_slices.wgsl`.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One frame of Slices. Mirrors
/// `lumit_core::fx::effects::slices::SlicesParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlicesOp {
    /// How far through the cut, 0..1.
    pub t: f32,
    /// Strips, 2..=64.
    pub count: u32,
    /// The strips are columns that travel up and down.
    pub vertical: bool,
    pub alternate: bool,
    pub random: bool,
    /// How far apart the strips set off, 0..0.95.
    pub stagger: f32,
    /// 0 Linear, 1 Smooth, 2 Sharp.
    pub ease: u32,
    /// This is the incoming picture.
    pub incoming: bool,
    /// Half the widest smear window, in a strip's own time.
    pub smear: f32,
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SlicesParams {
    t: f32,
    stagger: f32,
    smear: f32,
    mix: f32,
    count: u32,
    vertical: u32,
    alternate: u32,
    random: u32,
    ease: u32,
    incoming: u32,
    seed: u32,
    _pad: u32,
}

impl FxEngine {
    /// Apply one frame of Slices to a linear working texture, returning a new
    /// texture of the same size.
    pub fn slices(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SlicesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-slices-out");
        let p = SlicesParams {
            t: op.t,
            stagger: op.stagger,
            smear: op.smear,
            mix: op.mix,
            count: op.count.clamp(2, 64),
            vertical: u32::from(op.vertical),
            alternate: u32::from(op.alternate),
            random: u32::from(op.random),
            ease: op.ease,
            incoming: u32::from(op.incoming),
            seed: op.seed,
            _pad: 0,
        };
        self.dispatch(
            ctx,
            &self.slices,
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

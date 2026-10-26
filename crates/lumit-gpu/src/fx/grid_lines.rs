//! Grid's Lines type (`fx_gridlines.wgsl`). The setup is worked out on the
//! host by `lumit_core::fx::effects::grid_lines::GridLines`, and this carries
//! it across field for field.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One family of lines, as `GridLines` worked it out.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LineAxisOp {
    pub cell: f32,
    pub origin: f32,
    pub half: f32,
    /// Negative runs the lines without end.
    pub reach: f32,
    /// -1 for lines without end.
    pub last: i32,
    pub middle: i32,
}

/// One frame of Lines, in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLinesOp {
    pub centre: [f32; 2],
    pub axes: [[f32; 3]; 3],
    pub focal: f32,
    pub distance: f32,
    pub u: LineAxisOp,
    pub v: LineAxisOp,
    pub major_every: i32,
    pub major_width: f32,
    /// Premultiplied.
    pub colour: [f32; 4],
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    centre_focal_dist: [f32; 4],
    axis_u: [f32; 4],
    axis_v: [f32; 4],
    axis_n: [f32; 4],
    // cell, origin, half, reach
    u: [f32; 4],
    v: [f32; 4],
    // u last, u middle, v last, v middle
    ends: [i32; 4],
    colour: [f32; 4],
    major_width: f32,
    mix_amt: f32,
    major_every: i32,
    _pad: i32,
}

impl FxEngine {
    /// Draws Grid's Lines over a linear working texture, returning a new one.
    pub fn grid_lines(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GridLinesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-grid-lines-out");
        let a = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let line = |l: &LineAxisOp| [l.cell, l.origin, l.half, l.reach];
        self.dispatch(
            ctx,
            &self.grid_lines,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&Params {
                centre_focal_dist: [op.centre[0], op.centre[1], op.focal, op.distance],
                axis_u: a(op.axes[0]),
                axis_v: a(op.axes[1]),
                axis_n: a(op.axes[2]),
                u: line(&op.u),
                v: line(&op.v),
                ends: [op.u.last, op.u.middle, op.v.last, op.v.middle],
                colour: op.colour,
                major_width: op.major_width,
                mix_amt: op.mix,
                major_every: op.major_every,
                _pad: 0,
            }),
        );
        out
    }
}

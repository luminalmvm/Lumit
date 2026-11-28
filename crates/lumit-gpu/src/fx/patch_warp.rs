use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved Bezier warp (docs/08 §3.55). Mirrors
/// `lumit_core::fx::cpu::BezierWarpParams`; the twelve points travel two to a
/// `vec4`, because a `vec2` in a WGSL uniform array has a 16-byte stride and
/// half the uniform would be padding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BezierWarpOp {
    /// The twelve points in AE's clockwise walk from the upper left, raster
    /// pixels.
    pub pts: [[f32; 2]; 12],
    /// Newton steps a pixel, 1..=12.
    pub steps: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Where the frame sits on the raster, `[x, y, width, height]`. A zero
    /// width is the whole raster.
    pub frame: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BezierWarpParams {
    q: [[f32; 4]; 6],
    mix_amt: f32,
    steps: u32,
    /// 1 = scale the bend from the straight frame by the matte.
    matte_on: f32,
    _pad1: u32,
    frame: [f32; 4],
}

/// One resolved Mesh warp. Mirrors
/// `lumit_core::fx::effects::mesh_warp::MeshWarpParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshWarpOp {
    pub rows: u32,
    pub columns: u32,
    /// The crossings row by row, as fractions of the frame.
    pub pts: Vec<[f32; 2]>,
    /// How far each crossing's row handle reaches, fractions of the frame.
    pub hs: Vec<[f32; 2]>,
    /// How far each crossing's column handle reaches, fractions of the frame.
    pub vs: Vec<[f32; 2]>,
    /// Newton steps a pixel, 1..=12.
    pub steps: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Where the frame sits on the raster, `[x, y, width, height]`. A zero
    /// width is the whole raster.
    pub frame: [f32; 4],
}

/// The most crossings the kernel's uniform holds, six floats each.
const MESH_WARP_MAX_VERTICES: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeshWarpParams {
    rows: u32,
    cols: u32,
    steps: u32,
    count: u32,
    mix_amt: f32,
    matte_on: f32,
    _pad0: u32,
    _pad1: u32,
    frame: [f32; 4],
    pts: [[f32; 4]; MESH_WARP_MAX_VERTICES * 6 / 4],
}

impl FxEngine {
    pub fn bezier_warp(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &BezierWarpOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-bezier-warp-out");
        let mut q = [[0.0f32; 4]; 6];
        for (i, pair) in q.iter_mut().enumerate() {
            *pair = [
                op.pts[i * 2][0],
                op.pts[i * 2][1],
                op.pts[i * 2 + 1][0],
                op.pts[i * 2 + 1][1],
            ];
        }
        self.dispatch_matted(
            ctx,
            &self.bezier_warp,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BezierWarpParams {
                q,
                mix_amt: op.mix,
                steps: op.steps,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
                frame: if op.frame[2] > 0.0 {
                    op.frame
                } else {
                    [0.0, 0.0, w as f32, h as f32]
                },
            }),
        );
        out
    }

    /// Apply one Mesh warp to a linear working texture, returning a new
    /// texture of the same size. A grid that does not fit the kernel, or does
    /// not match its own size, is drawn flat.
    pub fn mesh_warp(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &MeshWarpOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-mesh-warp-out");
        let count = ((op.rows + 1) * (op.columns + 1)) as usize;
        let mut params = MeshWarpParams {
            rows: op.rows.max(1),
            cols: op.columns.max(1),
            steps: op.steps,
            count: count as u32,
            mix_amt: op.mix,
            matte_on: f32::from(matte.is_some()),
            _pad0: 0,
            _pad1: 0,
            frame: if op.frame[2] > 0.0 {
                op.frame
            } else {
                [0.0, 0.0, w as f32, h as f32]
            },
            pts: [[0.0; 4]; MESH_WARP_MAX_VERTICES * 6 / 4],
        };
        if count > MESH_WARP_MAX_VERTICES
            || op.pts.len() != count
            || op.hs.len() != count
            || op.vs.len() != count
        {
            params.mix_amt = 0.0;
        } else {
            for i in 0..count {
                let six = [
                    op.pts[i][0],
                    op.pts[i][1],
                    op.hs[i][0],
                    op.hs[i][1],
                    op.vs[i][0],
                    op.vs[i][1],
                ];
                for (k, value) in six.into_iter().enumerate() {
                    let at = i * 6 + k;
                    params.pts[at / 4][at % 4] = value;
                }
            }
        }
        self.dispatch_matted(
            ctx,
            &self.mesh_warp,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }
}

//! The ground a 3D viewport draws into its own frame. Not an effect: the
//! realiser lays it over a viewport's finished composite, tested against the
//! scene's depth so a layer standing in front hides it pixel by pixel. An
//! export never has one.

use super::{work_texture, FxEngine};
use crate::GpuContext;

/// Where the ground is seen from and how it is drawn, all in comp pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundOp {
    pub eye: [f32; 3],
    /// The camera's right, down-the-picture and forward axes.
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub zoom: f32,
    pub comp: (f32, f32),
    /// The floor's height, and the middle its lines are counted from.
    pub floor: f32,
    pub middle: (f32, f32),
    /// How far apart the lines are; every tenth one is drawn stronger.
    pub step: f32,
    /// Raster pixels per comp pixel.
    pub scale: f32,
    /// The clip w the depth texture is measured from.
    pub near: f32,
    /// Linear, straight alpha.
    pub colour: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GroundParams {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    colour: [f32; 4],
    grid: [f32; 4],
    depth: [f32; 4],
}

impl FxEngine {
    /// `frame` with the ground drawn in where nothing in `depth` (clip w less
    /// [`GroundOp::near`], as [`crate::Compositor::composite_depth`] makes it)
    /// is nearer. Both textures are `w × h`.
    pub fn ground(
        &self,
        ctx: &GpuContext,
        frame: &wgpu::Texture,
        depth: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GroundOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-ground-out");
        let v4 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
        self.dispatch(
            ctx,
            &self.ground,
            frame,
            depth,
            &out,
            w,
            h,
            bytemuck::bytes_of(&GroundParams {
                eye: v4(op.eye, op.floor),
                right: v4(op.right, op.comp.0),
                up: v4(op.up, op.comp.1),
                forward: v4(op.forward, op.zoom),
                colour: op.colour,
                grid: [op.middle.0, op.middle.1, op.step, op.scale],
                depth: [op.near, 0.0, 0.0, 0.0],
            }),
        );
        out
    }
}

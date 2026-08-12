//! The lighting pass (docs/06, K-361) — shading a layer with the comp's Light
//! layers. Not an effect: it has no entry in docs/08 and no `Resolved`
//! variant, and the realiser calls it directly between a layer's effect stack
//! and its composite.
//!
//! The maths lives in `lumit_core::lighting`, which is the oracle this kernel
//! is compared against. The types are restated here because an engine GPU
//! crate does not depend on the model crate (docs/05).

use super::{work_texture, FxEngine};
use crate::GpuContext;

/// How many lights shade one layer in a single pass — the same budget the
/// oracle keeps (`lumit_core::lighting::MAX_LIT_LIGHTS`).
pub const MAX_LIT_LIGHTS: usize = 8;

/// One light, flattened for the kernel: the four comp-pixel corners of its
/// emitting rectangle (all four equal for a point or a spot), its colour and
/// falloff, its spot axis, and whether it is an area light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingLight {
    pub corners: [[f32; 3]; 4],
    pub colour: [f32; 3],
    pub falloff_px: f32,
    pub is_area: bool,
    /// Cosine of a spot's half-angle; below -1 means "not a spot".
    pub cone_cos: f32,
    pub axis: [f32; 3],
}

/// One layer's plane in comp pixels, plus the lights that reach it. An empty
/// `lights` is the no-op the realiser relies on.
#[derive(Debug, Clone, PartialEq)]
pub struct LightingOp {
    pub origin: [f32; 3],
    pub du: [f32; 3],
    pub dv: [f32; 3],
    pub normal: [f32; 3],
    pub lights: Vec<LightingLight>,
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLight {
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    c3: [f32; 4],
    /// rgb = colour, w = falloff px.
    colour: [f32; 4],
    /// xyz = spot axis, w = cos(half-angle).
    axis: [f32; 4],
    /// x = area flag; the rest keeps the struct 16-byte aligned.
    flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightingParams {
    origin: [f32; 4],
    du: [f32; 4],
    dv: [f32; 4],
    /// xyz = plane normal, w = live light count.
    normal: [f32; 4],
    lights: [GpuLight; MAX_LIT_LIGHTS],
}

fn v4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

impl FxEngine {
    /// Shade one layer's linear working texture with the comp's lights,
    /// returning a new texture of the same size. Multiplies the picture by
    /// `1 + light`, so an unlit pixel is untouched; `op.lights` empty
    /// short-circuits inside the kernel to a straight copy.
    pub fn lighting(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LightingOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-lighting-out");
        let mut lights = [GpuLight::default(); MAX_LIT_LIGHTS];
        let live = op.lights.len().min(MAX_LIT_LIGHTS);
        for (slot, l) in lights.iter_mut().zip(&op.lights) {
            *slot = GpuLight {
                c0: v4(l.corners[0], 0.0),
                c1: v4(l.corners[1], 0.0),
                c2: v4(l.corners[2], 0.0),
                c3: v4(l.corners[3], 0.0),
                colour: v4(l.colour, l.falloff_px),
                axis: v4(l.axis, l.cone_cos),
                flags: [f32::from(u8::from(l.is_area)), 0.0, 0.0, 0.0],
            };
        }
        self.dispatch(
            ctx,
            &self.lighting,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&LightingParams {
                origin: v4(op.origin, 0.0),
                du: v4(op.du, 0.0),
                dv: v4(op.dv, 0.0),
                normal: v4(op.normal, live as f32),
                lights,
            }),
        );
        out
    }
}

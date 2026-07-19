//! Blur family kernels (docs/08 §3.3, §3.8, §3.9): box/gaussian blur,
//! directional and radial blur, unsharp-mask sharpen, and the glow bloom that
//! reuses the shared gaussian.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved blur, in raster pixels (the caller converts from the
/// spec's %-of-diagonal units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurOp {
    pub radius_px: f32,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One resolved directional blur (docs/08 §3.8): a line integral along a
/// host-computed unit direction. `taps` must equal
/// `lumit_core::fx::cpu::dir_blur_taps(length_px)` so the GPU dispatches
/// the oracle's exact kernel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirBlurOp {
    /// Unit streak direction (host-computed cos/sin).
    pub dx: f32,
    pub dy: f32,
    /// Full streak length, raster pixels.
    pub length_px: f32,
    /// Evenly spaced bilinear taps across the streak.
    pub taps: i32,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DirBlurParams {
    dx: f32,
    dy: f32,
    length: f32,
    taps: i32,
    edge: u32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved radial blur — Blur's Radial mode (docs/08 §3.8, schema
/// status note). `taps` must equal
/// `lumit_core::fx::cpu::radial_blur_taps(amount_px)` so the GPU dispatches
/// the oracle's exact kernel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialBlurOp {
    /// Centre as a *fraction* of the raster (not raster pixels) — the
    /// kernel scales it by its own `textureDimensions`, exactly like the
    /// CPU reference scales it by the `w`/`h` it is handed.
    pub centre_frac: [f32; 2],
    /// Peak tap spread in raster pixels, reached at the frame's farthest
    /// corner from Centre.
    pub amount_px: f32,
    /// Evenly spaced taps along the ray (Zoom) or its perpendicular (Spin).
    pub taps: i32,
    /// True = Spin (tangent direction), false = Zoom (radial direction).
    pub spin: bool,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RadialBlurParams {
    centre: [f32; 2],
    amount: f32,
    taps: i32,
    spin: u32,
    edge: u32,
    mix_amt: f32,
    _pad: f32,
}

/// One resolved sharpen (docs/08 §3.9), amounts already fractional and the
/// gaussian radius already in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenOp {
    /// Fraction of the detail signal added back (0..3 = 0–300%).
    pub amount: f32,
    pub radius_px: f32,
    /// Linear-light soft gate under which detail is left alone.
    pub threshold: f32,
    /// True: sharpen the Rec. 709 luma only.
    pub luma_only: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurParams {
    dir: [f32; 2],
    radius: f32,
    sigma: f32,
    edge: u32,
    mix_amt: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SharpenParams {
    amount: f32,
    threshold: f32,
    luma_only: u32,
    mix_amt: f32,
}

/// One resolved simple 3×3 sharpen (docs/08 §3.9, K-138): a high-pass
/// convolution scaled by `amount`, the radius-free sibling of the Unsharp
/// mask above. Amount 0 is the bit-exact passthrough.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenSimpleOp {
    /// High-pass strength (1 = the classic 5/−1 kernel); 0 is a passthrough.
    pub amount: f32,
    /// Neighbour distance in raster pixels (T15): 1 = a 3×3 kernel.
    pub radius: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SharpenSimpleParams {
    amount: f32,
    radius: f32,
    mix_amt: f32,
    _pad: [f32; 1],
}

/// One resolved glow (docs/08 §3.3, v1 core): bright-pass with a soft knee,
/// the shared gaussian on the leftover light, additive recombine. The
/// radius is already in raster pixels; intensity 0 is the neutral point
/// (bit-exact passthrough, matching the CPU reference's short-circuit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlowOp {
    /// The halo gaussian's half-width, raster pixels.
    pub radius_px: f32,
    /// Linear-light bright threshold, ≥ 0 (unbounded above, K-090).
    pub threshold: f32,
    /// Soft-knee width around the threshold, 0..1.
    pub knee: f32,
    /// Gain on the added halo.
    pub intensity: f32,
    /// Scene-linear RGBA halo tint (alpha unused).
    pub tint: [f32; 4],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlowParams {
    tint: [f32; 4],
    threshold: f32,
    knee: f32,
    intensity: f32,
    mix_amt: f32,
}

impl FxEngine {
    /// Apply one gaussian blur to a linear working texture, returning a new
    /// texture of the same size (two separable passes; the host Mix blends
    /// the final pass against the untouched input, docs/08 §1.5).
    pub fn blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BlurOp,
    ) -> wgpu::Texture {
        let tmp = work_texture(ctx, w, h, "fx-blur-tmp");
        let out = work_texture(ctx, w, h, "fx-blur-out");
        let sigma = (op.radius_px * 0.5).max(1e-3);
        // Horizontal into tmp (mix 1: the blend happens once, at the end).
        self.dispatch(
            ctx,
            &self.blur,
            src,
            src,
            &tmp,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [1.0, 0.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: 1.0,
                _pad: [0.0; 2],
            }),
        );
        // Vertical into out, blending against the original input.
        self.dispatch(
            ctx,
            &self.blur,
            &tmp,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [0.0, 1.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one directional blur (docs/08 §3.8) to a linear working
    /// texture, returning a new texture of the same size. One pass: a
    /// box-weighted line integral of bilinear taps along the unit direction.
    pub fn dir_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &DirBlurOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-dir-blur-out");
        self.dispatch(
            ctx,
            &self.dir_blur,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&DirBlurParams {
                dx: op.dx,
                dy: op.dy,
                length: op.length_px,
                taps: op.taps,
                edge: op.edge,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one radial blur — Blur's Radial mode (docs/08 §3.8) — to a
    /// linear working texture, returning a new texture of the same size.
    /// One pass: box-weighted taps along a ray (Zoom) or its perpendicular
    /// (Spin), the shared schema-status-note maths.
    pub fn radial_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &RadialBlurOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-radial-blur-out");
        self.dispatch(
            ctx,
            &self.radial_blur,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RadialBlurParams {
                centre: op.centre_frac,
                amount: op.amount_px,
                taps: op.taps,
                spin: u32::from(op.spin),
                edge: op.edge,
                mix_amt: op.mix,
                _pad: 0.0,
            }),
        );
        out
    }

    /// Apply one unsharp mask (docs/08 §3.9) to a linear working texture,
    /// returning a new texture of the same size. Four passes: unpremultiply
    /// (§2.2, fused into the kernel chain), a separable gaussian on the
    /// unpremultiplied colour (reusing the blur kernel, Repeat edges — the
    /// CPU reference blurs with the same fixed policy), then the combine
    /// pass that gates, re-premultiplies and applies the host Mix.
    pub fn sharpen(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SharpenOp,
    ) -> wgpu::Texture {
        let un = work_texture(ctx, w, h, "fx-sharpen-un");
        let tmp = work_texture(ctx, w, h, "fx-sharpen-tmp");
        let blurred = work_texture(ctx, w, h, "fx-sharpen-blur");
        let out = work_texture(ctx, w, h, "fx-sharpen-out");
        let params = SharpenParams {
            amount: op.amount,
            threshold: op.threshold,
            luma_only: u32::from(op.luma_only),
            mix_amt: op.mix,
        };
        self.dispatch(
            ctx,
            &self.sharpen_unpremultiply,
            src,
            src,
            &un,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        let sigma = (op.radius_px * 0.5).max(1e-3);
        for (pass_src, pass_dst, dir) in [(&un, &tmp, [1.0, 0.0]), (&tmp, &blurred, [0.0, 1.0])] {
            self.dispatch(
                ctx,
                &self.blur,
                pass_src,
                pass_src,
                pass_dst,
                w,
                h,
                bytemuck::bytes_of(&BlurParams {
                    dir,
                    radius: op.radius_px,
                    sigma,
                    edge: 1, // Repeat, always (see the schema comment)
                    mix_amt: 1.0,
                    _pad: [0.0; 2],
                }),
            );
        }
        self.dispatch(
            ctx,
            &self.sharpen_combine,
            &blurred,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }

    /// Apply one simple 3×3 sharpen (docs/08 §3.9, K-138) to a linear working
    /// texture, returning a new texture of the same size. One pass: the
    /// high-pass convolution over the pixel and its four clamp-addressed axis
    /// neighbours, the §2.2 unpremultiply wrap fused into the kernel. Amount 0
    /// is the bit-exact passthrough (the kernel short-circuits, matching the
    /// CPU reference); Mix 0 is the identity.
    pub fn sharpen_simple(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SharpenSimpleOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-sharpen-simple-out");
        self.dispatch(
            ctx,
            &self.sharpen_simple,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SharpenSimpleParams {
                amount: op.amount,
                radius: op.radius,
                mix_amt: op.mix,
                _pad: [0.0; 1],
            }),
        );
        out
    }

    /// Apply one glow (docs/08 §3.3, v1 core) to a linear working texture,
    /// returning a new texture of the same size. Four passes: the bright
    /// pass keeps only the light above the threshold (soft knee, all four
    /// premultiplied channels — the halo carries alpha), the shared
    /// separable gaussian widens it (Repeat edges, fixed: the halo holds
    /// its strength along frame borders), and the combine pass adds
    /// `intensity · tint · halo` back onto the untouched input in linear,
    /// alpha saturating at 1. Intensity 0 short-circuits inside the combine
    /// kernel to the bit-exact identity.
    pub fn glow(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GlowOp,
    ) -> wgpu::Texture {
        let bright = work_texture(ctx, w, h, "fx-glow-bright");
        let tmp = work_texture(ctx, w, h, "fx-glow-tmp");
        let blurred = work_texture(ctx, w, h, "fx-glow-blur");
        let out = work_texture(ctx, w, h, "fx-glow-out");
        let params = GlowParams {
            tint: op.tint,
            threshold: op.threshold,
            knee: op.knee,
            intensity: op.intensity,
            mix_amt: op.mix,
        };
        self.dispatch(
            ctx,
            &self.glow_bright,
            src,
            src,
            &bright,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        let sigma = (op.radius_px * 0.5).max(1e-3);
        for (pass_src, pass_dst, dir) in [(&bright, &tmp, [1.0, 0.0]), (&tmp, &blurred, [0.0, 1.0])]
        {
            self.dispatch(
                ctx,
                &self.blur,
                pass_src,
                pass_src,
                pass_dst,
                w,
                h,
                bytemuck::bytes_of(&BlurParams {
                    dir,
                    radius: op.radius_px,
                    sigma,
                    edge: 1, // Repeat, always (see the CPU reference)
                    mix_amt: 1.0,
                    _pad: [0.0; 2],
                }),
            );
        }
        self.dispatch(
            ctx,
            &self.glow_combine,
            &blurred,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }
}

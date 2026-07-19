//! Stylise and geometry kernels (docs/08 §3.5, §3.12, §3.14, §3.21): the matte
//! key, vignette, affine transform, block glitch and scanlines.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved matte key (docs/08 §3.21): a soft chroma key on straight
/// (unpremultiplied) colour. `key` is the scene-linear RGBA key colour (alpha
/// ignored); `tol`/`soft`/`spill` are 0..1 fractions. The kernel derives the
/// key's chroma and hue direction from `key`, exactly as the CPU reference
/// does, so both paths use the same numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatteKeyOp {
    /// Scene-linear RGBA key colour (alpha ignored).
    pub key: [f32; 4],
    /// Chroma-distance threshold, 0..1: at/below it a pixel is fully keyed.
    pub tol: f32,
    /// Soft-edge width above `tol`, 0..1: the smoothstep transition span.
    pub soft: f32,
    /// Key-hue spill removal, 0..1.
    pub spill: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatteKeyParams {
    key: [f32; 4],
    tol: f32,
    soft: f32,
    spill: f32,
    mix_amt: f32,
}

/// One resolved vignette (docs/08 §3.14): darkens toward black away from
/// the frame centre. Radius/Softness/Roundness are already-clamped
/// fractions; the kernel derives the distance metric from its own
/// `textureDimensions`, exactly like the CPU reference derives it from
/// `w`/`h` — no raster conversion happens host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignetteOp {
    /// 0..1: darkening strength; 0 is the neutral point.
    pub amount: f32,
    /// 0..1: the clear centre's reach.
    pub radius: f32,
    /// 0..1: feather width beyond radius.
    pub softness: f32,
    /// 0..1: 1 = circular, 0 = follows the frame's aspect.
    pub roundness: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VignetteParams {
    amount: f32,
    radius: f32,
    softness: f32,
    roundness: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved transform (docs/08 §3.5, K-090): the inverse affine arrives
/// host-computed (`lumit_core::fx::transform_op`) so the kernel never runs
/// its own trigonometry and the CPU reference consumes bit-identical
/// numbers. A degenerate (zero-scale) transform arrives as opacity 0 with
/// an identity matrix — fully transparent, exactly like the reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOp {
    /// Row-major inverse linear 2×2: (m00, m01, m10, m11).
    pub m: [f32; 4],
    /// Inverse translation: sample q = m·p + off.
    pub off: [f32; 2],
    /// 0..1, multiplied into premultiplied RGBA.
    pub opacity: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// The revealed border's edge policy (P3, K-145): 0 Transparent, 1 Repeat,
    /// 2 Mirror. The Transform effect passes 0; Shake threads its Edges control.
    pub edge: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TransformParams {
    m: [f32; 4],
    off: [f32; 2],
    opacity: f32,
    mix_amt: f32,
    edge: u32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Block glitch (docs/08 §3.12, split out of the old combined
/// Glitch effect by K-107). `tick` arrives already computed from local time
/// (`lumit_core::fx::GLITCH_TICK_HZ`), so the kernel never sees raw time or
/// does its own time maths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockGlitchOp {
    /// The master 0..1 dial; scales every hashed quantity.
    pub intensity: f32,
    pub seed: u32,
    pub tick: i32,
    /// Raster pixels (px@comp × the §2.3 preview factor).
    pub block_size_px: f32,
    /// 0..1, fraction of block_size_px.
    pub jitter_frac: f32,
    /// Peak per-block displacement, raster pixels.
    pub amount_px: f32,
    /// Peak per-block R/B split, raster pixels.
    pub chan_px: f32,
    /// 0..1: odds (before the Intensity scale) a block slice-repeats.
    pub slice_frac: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlockGlitchParams {
    intensity: f32,
    seed: u32,
    tick: i32,
    block_size: f32,
    jitter_frac: f32,
    amount: f32,
    chan: f32,
    slice_frac: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Scanlines (docs/08 §3.12, split out of the old combined
/// Glitch effect by K-107). `roll_px` arrives already computed from local
/// time (roll speed × time × period), so the kernel never sees raw time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanlinesOp {
    /// The master 0..1 dial; scales the darken strength.
    pub intensity: f32,
    /// Raster pixels (px@comp × the §2.3 preview factor).
    pub period_px: f32,
    /// 0..1.
    pub darkness: f32,
    /// The scanline pattern's pixel offset at this frame, host-computed.
    pub roll_px: f32,
    pub interlace: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScanlinesParams {
    intensity: f32,
    period: f32,
    darkness: f32,
    roll_px: f32,
    interlace: u32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

impl FxEngine {
    /// Apply one matte key (docs/08 §3.21) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the §2.2
    /// unpremultiply wrap is fused into the kernel, which derives the key's
    /// chroma/hue direction from `key` exactly as the CPU reference does. There
    /// is no neutral short-circuit (the default keys); Mix 0 is the identity.
    pub fn matte_key(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &MatteKeyOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-matte-key-out");
        self.dispatch(
            ctx,
            &self.matte_key,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&MatteKeyParams {
                key: op.key,
                tol: op.tol,
                soft: op.soft,
                spill: op.spill,
                mix_amt: op.mix,
            }),
        );
        out
    }

    /// Apply one vignette (docs/08 §3.14) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the
    /// kernel derives the distance metric from its own texture size, and
    /// Amount 0 short-circuits inside it.
    pub fn vignette(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &VignetteOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-vignette-out");
        self.dispatch(
            ctx,
            &self.vignette,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&VignetteParams {
                amount: op.amount,
                radius: op.radius,
                softness: op.softness,
                roundness: op.roundness,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one transform (docs/08 §3.5, K-090) to a linear working
    /// texture, returning a new texture of the same size. One pass: each
    /// output pixel takes a single bilinear tap through the host-computed
    /// inverse affine, transparent outside the frame, opacity folded in.
    /// Identity parameters reproduce the input bit-exactly.
    pub fn transform(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TransformOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-transform-out");
        self.dispatch(
            ctx,
            &self.transform,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TransformParams {
                m: op.m,
                off: op.off,
                opacity: op.opacity,
                mix_amt: op.mix,
                edge: op.edge,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Block glitch (docs/08 §3.12, split out by K-107) to a
    /// linear working texture, returning a new texture of the same size.
    /// One pointwise-with-taps pass: block UV displacement and channel
    /// offset.
    pub fn block_glitch(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BlockGlitchOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-block-glitch-out");
        self.dispatch(
            ctx,
            &self.block_glitch,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlockGlitchParams {
                intensity: op.intensity,
                seed: op.seed,
                tick: op.tick,
                block_size: op.block_size_px,
                jitter_frac: op.jitter_frac,
                amount: op.amount_px,
                chan: op.chan_px,
                slice_frac: op.slice_frac,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Scanlines (docs/08 §3.12, split out by K-107) to a linear
    /// working texture, returning a new texture of the same size. One
    /// pointwise pass: periodic darkening in raster Y, no neighbour taps.
    pub fn scanlines(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ScanlinesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-scanlines-out");
        self.dispatch(
            ctx,
            &self.scanlines,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ScanlinesParams {
                intensity: op.intensity,
                period: op.period_px,
                darkness: op.darkness,
                roll_px: op.roll_px,
                interlace: u32::from(op.interlace),
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }
}

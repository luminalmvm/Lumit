//! Stylise and geometry kernels (docs/08 §3.5, §3.12, §3.14, §3.21): the matte
//! key, vignette, affine transform, block glitch and scanlines.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved matte key (docs/08 §3.21, K-121/K-154): a Keylight-style
/// colour-difference keyer on straight (unpremultiplied) colour. Mirrors
/// `lumit_core::fx::MatteKeyParams` field-for-field so the kernel and the CPU
/// oracle consume the identical numbers (K-031). The kernel derives the screen's
/// primary channel and reference from `key`, exactly as the CPU reference does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatteKeyOp {
    /// Output view wire code: 0 Final, 1 Screen matte, 2 Status.
    pub view: u32,
    /// Scene-linear RGBA screen (key) colour; alpha ignored.
    pub key: [f32; 4],
    /// Screen gain (matte fall-off strength), `≥ 0`.
    pub gain: f32,
    /// Screen balance, 0..1 (secondary-channel weighting).
    pub balance: f32,
    /// Despill bias (scene-linear RGBA, alpha ignored).
    pub despill_bias: [f32; 4],
    /// Alpha bias (scene-linear RGBA, alpha ignored).
    pub alpha_bias: [f32; 4],
    /// Despill amount, 0..1.
    pub spill: f32,
    /// Clip black, 0..1.
    pub clip_black: f32,
    /// Clip white, 0..1.
    pub clip_white: f32,
    /// Clip rollback, 0..1.
    pub clip_rollback: f32,
    /// Replace method wire code: 0 Source, 1 Hard, 2 Soft, 3 None.
    pub replace_method: u32,
    /// Scene-linear RGBA replace colour.
    pub replace_colour: [f32; 4],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatteKeyParams {
    // Four vec4 colours first (each 16-byte aligned for the WGSL uniform).
    key: [f32; 4],
    despill_bias: [f32; 4],
    alpha_bias: [f32; 4],
    replace_colour: [f32; 4],
    // Then the scalars, packed to a 16-byte multiple with three pad floats.
    gain: f32,
    balance: f32,
    spill: f32,
    clip_black: f32,
    clip_white: f32,
    clip_rollback: f32,
    view: u32,
    replace_method: u32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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
    /// Gamma on the falloff (T16): 1 = plain smoothstep.
    pub ramp: f32,
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
    ramp: f32,
    mix_amt: f32,
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

/// The number of Shake motion-blur sub-frame taps (T18/K-165): the fixed-size
/// end of the uniform array and the WGSL kernel's `array<Tap, 9>` / `MAX_TAPS`.
/// Must equal `lumit_core::fx::SHAKE_MB_SAMPLES` — the GPU crate can't name that
/// const (lumit-core is a dev-dependency only), so the oracle tests assert the
/// two agree, and the WGSL literal is kept in step by the same tests.
pub const SHAKE_MB_SAMPLES: usize = 9;

/// One resolved Shake motion blur (docs/08 §3.4, T18/K-165): the shake's own
/// inter-frame smear. Each tap is a host-computed inverse affine (the same
/// `shake_affine` → `transform_op` construction the plain Shake uses, one per
/// motion-blur sub-frame); the kernel resamples the input through the first
/// `count` taps and averages them in premultiplied linear space. `count` is
/// always ≥ 1 (the host only builds this when motion blur is on). Mirrors
/// `lumit_core::fx::cpu::transform_average`.
#[derive(Debug, Clone, Copy)]
pub struct ShakeMbOp {
    /// Up to [`SHAKE_MB_SAMPLES`] inverse affines `(m, off)`.
    pub taps: [ShakeMbTap; SHAKE_MB_SAMPLES],
    /// Active taps, `1..=SHAKE_MB_SAMPLES`.
    pub count: u32,
    /// The revealed border's edge policy (P3, K-145): 0 Transparent, 1 Repeat,
    /// 2 Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One motion-blur sub-frame's inverse affine `(m, off)` (T18): row-major
/// inverse linear 2×2 and the inverse translation, exactly as [`TransformOp`].
#[derive(Debug, Clone, Copy)]
pub struct ShakeMbTap {
    pub m: [f32; 4],
    pub off: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShakeMbTapUniform {
    m: [f32; 4],
    off: [f32; 4], // .xy used; .zw pad to the uniform's 16-byte stride
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShakeMbParams {
    taps: [ShakeMbTapUniform; SHAKE_MB_SAMPLES],
    count: u32,
    edge: u32,
    mix_amt: f32,
    _pad: f32,
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
/// Glitch effect by K-107; single Intensity since FX-13/K-147). `roll_px`
/// arrives already computed from local time (roll speed × time × period), so
/// the kernel never sees raw time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanlinesOp {
    /// The single 0..1 dial: how dark the dark lines get (1 = black).
    pub intensity: f32,
    /// Raster pixels (px@comp × the §2.3 preview factor).
    pub period_px: f32,
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
    roll_px: f32,
    interlace: u32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

impl FxEngine {
    /// Apply one matte key (docs/08 §3.21, K-121/K-154) to a linear working
    /// texture, returning a new texture of the same size. One pointwise pass; the
    /// §2.2 unpremultiply wrap is fused into the kernel, which derives the screen's
    /// primary channel and reference from `key` exactly as the CPU reference does.
    /// There is no neutral short-circuit (the default keys); Mix 0 is the identity.
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
                despill_bias: op.despill_bias,
                alpha_bias: op.alpha_bias,
                replace_colour: op.replace_colour,
                gain: op.gain,
                balance: op.balance,
                spill: op.spill,
                clip_black: op.clip_black,
                clip_white: op.clip_white,
                clip_rollback: op.clip_rollback,
                view: op.view,
                replace_method: op.replace_method,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
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
                ramp: op.ramp,
                mix_amt: op.mix,
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

    /// Apply one Shake motion blur (docs/08 §3.4, T18/K-165): resample the input
    /// through the op's sub-frame inverse affines and average them, then blend
    /// by mix — the shake's own inter-frame smear, on this effect alone. One
    /// pass with up to [`SHAKE_MB_SAMPLES`] bilinear taps.
    pub fn shake_mb(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ShakeMbOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-shake-mb-out");
        let mut taps = [ShakeMbTapUniform {
            m: [1.0, 0.0, 0.0, 1.0],
            off: [0.0; 4],
        }; SHAKE_MB_SAMPLES];
        for (dst, s) in taps.iter_mut().zip(op.taps.iter()) {
            dst.m = s.m;
            dst.off = [s.off[0], s.off[1], 0.0, 0.0];
        }
        self.dispatch(
            ctx,
            &self.shake_mb,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ShakeMbParams {
                taps,
                count: op.count.clamp(1, SHAKE_MB_SAMPLES as u32),
                edge: op.edge,
                mix_amt: op.mix,
                _pad: 0.0,
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
                roll_px: op.roll_px,
                interlace: u32::from(op.interlace),
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }
}

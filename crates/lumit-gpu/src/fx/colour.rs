//! Colour and tone adjustment kernels (docs/08 §3.7, §3.10, §3.16–§3.20,
//! §3.23–§3.24): flash, colour balance, saturation, exposure, temperature,
//! invert, tint, contrast, gamma and hue shift.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved flash (docs/08 §3.7, manual form): the trigger envelope is
/// already evaluated host-side into a plain strength.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlashOp {
    /// 0..1 — envelope × intensity, clamped.
    pub strength: f32,
    /// Scene-linear RGBA flash colour (alpha unused).
    pub colour: [f32; 4],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlashParams {
    colour: [f32; 4],
    strength: f32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved colour balance (docs/08 §3.10 as amended by K-090): gain →
/// lift → gamma per channel, in linear on unpremultiplied colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourBalanceOp {
    pub lift: [f32; 3],
    /// Per-channel, > 0 (the resolver clamps).
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ColourBalanceParams {
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved saturation (docs/08 §3.10 as amended by K-090): scale about
/// Rec. 709 luma, in linear on unpremultiplied colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SaturationOp {
    /// 0 = greyscale, 1 = neutral, 2 = doubled, open above (K-135).
    pub saturation: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SaturationParams {
    saturation: f32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved vibrancy (docs/08 §3.10, K-152): a saturation boost weighted
/// by each pixel's current colourfulness, in linear on unpremultiplied colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VibrancyOp {
    /// 0 = neutral; higher lifts less-saturated pixels more, open above (K-135).
    pub amount: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VibrancyParams {
    amount: f32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved exposure (docs/08 §3.16): a single scene-linear gain on the
/// RGB channels. `factor` is `2^stops`, computed host-side so the CPU
/// reference and the kernel multiply by the identical number; alpha is
/// untouched. `factor == 1.0` (0 stops) is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureOp {
    /// The linear gain, `2^stops`. 1.0 is the neutral point.
    pub factor: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExposureParams {
    factor: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved temperature (docs/08 §3.20): a warm/cool white-balance shift as
/// a per-channel gain in scene-linear light. `gain_r`/`gain_b` are computed
/// host-side (`gain_r = max(0, 1 + 0.75·k)`, `gain_b = max(0, 1 − 0.75·k)` for
/// `k = temperature / 100`, K-135), so the CPU reference and the kernel multiply
/// by byte-identical numbers; green and alpha are untouched. Gains `(1.0, 1.0)`
/// (temperature 0)
/// are the bit-exact neutral point. Premultiplied, exactly like [`ExposureOp`]:
/// a per-channel scalar scales premultiplied colour consistently, so no
/// unpremultiply round trip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureOp {
    /// The scene-linear red gain. 1.0 (with `gain_b` 1.0) is the neutral point.
    pub gain_r: f32,
    /// The scene-linear blue gain. 1.0 (with `gain_r` 1.0) is the neutral point.
    pub gain_b: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TemperatureParams {
    gain_r: f32,
    gain_b: f32,
    mix_amt: f32,
    _pad0: f32,
}

/// One resolved invert (docs/08 §3.23): the colour inverse `out.rgb = 1 − u`
/// per RGB channel, on unpremultiplied colour (`1 − c` is affine, so it does
/// not commute with premultiplied alpha), alpha untouched. There is no neutral
/// value — invert always inverts — so only Mix 0 is the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvertOp {
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InvertParams {
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved tint (docs/08 §3.24): a luminance duotone
/// `out.rgb = black + (white − black)·luma(u)` with Rec.709 luma on
/// unpremultiplied colour (a colour remap does not commute with premultiplied
/// alpha), alpha untouched. `black`/`white` are the scene-linear RGB the darkest
/// and brightest input map to; Mix 0 is the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TintOp {
    /// Scene-linear RGB the darkest input maps to.
    pub black: [f32; 3],
    /// Scene-linear RGB the brightest input maps to.
    pub white: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TintParams {
    black: [f32; 4],
    white: [f32; 4],
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved contrast (docs/08 §3.18): the affine grade
/// `(u − 0.5) × k + 0.5` per RGB channel about a fixed mid-grey pivot, on
/// unpremultiplied colour (an affine grade does not commute with premultiplied
/// alpha), alpha untouched. `k == 1.0` (Contrast 100 %) is the bit-exact
/// neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastOp {
    /// The contrast factor, `contrast_percent / 100`. 1.0 is the neutral point.
    pub k: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ContrastParams {
    k: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved gamma (docs/08 §3.19): the per-channel power curve
/// `out = pow(max(u, 0), 1/gamma)` on unpremultiplied colour (a non-linear
/// curve does not commute with premultiplied alpha), alpha untouched.
/// `gamma == 1.0` is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GammaOp {
    /// The gamma value; the kernel raises to `1/gamma`. 1.0 is the neutral
    /// point (clamped ≥ 0.01 host-side so the reciprocal stays finite).
    pub gamma: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GammaParams {
    gamma: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved hue shift (docs/08 §3.17): a row-major linear 3×3 colour
/// matrix, computed host-side (`lumit_core::fx::hue_matrix`) so the CPU
/// reference and the kernel multiply by identical coefficients. The identity
/// matrix is the neutral point; alpha is untouched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueShiftOp {
    /// Row-major 3×3: `[m00,m01,m02, m10,m11,m12, m20,m21,m22]`.
    pub m: [f32; 9],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HueParams {
    m: [f32; 9],
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

impl FxEngine {
    /// Apply one flash (docs/08 §3.7, manual form) to a linear working
    /// texture, returning a new texture of the same size. One pointwise
    /// pass; the trigger envelope arrives pre-evaluated in the op.
    pub fn flash(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FlashOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-flash-out");
        self.dispatch(
            ctx,
            &self.flash,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&FlashParams {
                colour: op.colour,
                strength: op.strength,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one colour balance (docs/08 §3.10 as amended by K-090) to a
    /// linear working texture, returning a new texture of the same size.
    /// One pointwise pass; the §2.2 unpremultiply wrap is fused into the
    /// kernel, and fully neutral parameters short-circuit inside it.
    pub fn colour_balance(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ColourBalanceOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-colour-balance-out");
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        self.dispatch(
            ctx,
            &self.colour_balance,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ColourBalanceParams {
                lift: v4(op.lift),
                gamma: v4(op.gamma),
                gain: v4(op.gain),
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one saturation (docs/08 §3.10 as amended by K-090) to a linear
    /// working texture, returning a new texture of the same size. One
    /// pointwise pass; the §2.2 unpremultiply wrap is fused into the
    /// kernel, and saturation 1 short-circuits inside it.
    pub fn saturation(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SaturationOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-saturation-out");
        self.dispatch(
            ctx,
            &self.saturation,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SaturationParams {
                saturation: op.saturation,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one vibrancy (docs/08 §3.10, K-152) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the §2.2
    /// unpremultiply wrap is fused into the kernel, and amount 0 short-circuits
    /// inside it to the bit-exact identity.
    pub fn vibrancy(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &VibrancyOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-vibrancy-out");
        self.dispatch(
            ctx,
            &self.vibrancy,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&VibrancyParams {
                amount: op.amount,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one exposure (docs/08 §3.16) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: RGB × the
    /// host-computed `factor`, alpha untouched; `factor == 1.0` short-circuits
    /// to the input inside the kernel.
    pub fn exposure(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ExposureOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-exposure-out");
        self.dispatch(
            ctx,
            &self.exposure,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ExposureParams {
                factor: op.factor,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one temperature (docs/08 §3.20) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: R × the
    /// host-computed `gain_r` and B × `gain_b`, green and alpha untouched;
    /// `gain_r == 1.0 && gain_b == 1.0` (temperature 0) short-circuits to the
    /// input inside the kernel. Premultiplied, exactly like [`Self::exposure`].
    pub fn temperature(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TemperatureOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-temperature-out");
        self.dispatch(
            ctx,
            &self.temperature,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TemperatureParams {
                gain_r: op.gain_r,
                gain_b: op.gain_b,
                mix_amt: op.mix,
                _pad0: 0.0,
            }),
        );
        out
    }

    /// Apply one invert (docs/08 §3.23) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass: `1 − u` per channel, the
    /// §2.2 unpremultiply wrap fused into the kernel. There is no neutral
    /// short-circuit (invert always inverts); Mix 0 is the identity.
    pub fn invert(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &InvertOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-invert-out");
        self.dispatch(
            ctx,
            &self.invert,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&InvertParams {
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one tint (docs/08 §3.24) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass: the luma-driven lerp
    /// between the two mapped colours, the §2.2 unpremultiply wrap fused into the
    /// kernel; Mix 0 is the identity.
    pub fn tint(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TintOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-tint-out");
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        self.dispatch(
            ctx,
            &self.tint,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TintParams {
                black: v4(op.black),
                white: v4(op.white),
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one contrast (docs/08 §3.18) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: the
    /// affine grade about mid-grey, the §2.2 unpremultiply wrap fused into the
    /// kernel; `k == 1.0` short-circuits to the input inside the kernel.
    pub fn contrast(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ContrastOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-contrast-out");
        self.dispatch(
            ctx,
            &self.contrast,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ContrastParams {
                k: op.k,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one gamma (docs/08 §3.19) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass: the per-channel power
    /// curve `pow(max(u, 0), 1/gamma)`, the §2.2 unpremultiply wrap fused into
    /// the kernel; `gamma == 1.0` short-circuits to the input inside the kernel.
    pub fn gamma(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GammaOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-gamma-out");
        self.dispatch(
            ctx,
            &self.gamma,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&GammaParams {
                gamma: op.gamma,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one hue shift (docs/08 §3.17) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: RGB × the
    /// host-computed colour matrix, alpha untouched.
    pub fn hue_shift(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &HueShiftOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-hue-out");
        self.dispatch(
            ctx,
            &self.hue_shift,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&HueParams {
                m: op.m,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }
}

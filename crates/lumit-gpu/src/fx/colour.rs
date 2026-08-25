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
    /// 1 = pull Lift toward 0 and Gamma and Gain toward 1 by the matte (K-395).
    matte_on: f32,
    _pad: [f32; 2],
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
    /// 1 = pull Saturation toward 1 by the matte (K-395).
    matte_on: f32,
    _pad0: f32,
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
    /// 1 = scale Amount by the matte (K-395).
    matte_on: f32,
    _pad0: f32,
}

/// One resolved exposure (docs/08 §3.16): a single scene-linear gain on the
/// RGB channels. `factor` is `2^stops`, computed host-side so the CPU
/// reference and the kernel multiply by the identical number; alpha is
/// untouched. `factor == 1.0` (0 stops) is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureOp {
    /// The Stops the factor was made from, for the matted branch (K-395).
    pub stops: f32,
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
    /// The stops behind `factor`, read only under a matte: the gain there is
    /// `exp2(stops * k)` (K-395).
    stops: f32,
    /// 1 = scale Stops toward 0 by the matte.
    matte_on: f32,
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
    /// Temperature / 100 clamped to +-2, for the matted branch (K-395).
    pub t: f32,
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
    /// Temperature / 100, read only under a matte: the gains there are
    /// rebuilt from `t * k` (K-395).
    t: f32,
    /// 1 = scale Temperature toward 0 by the matte.
    matte_on: f32,
    _pad: [f32; 3],
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
    /// 1 = pull Gamma toward 1 by the matte (K-395).
    matte_on: f32,
    _pad1: f32,
}

/// Entries in one channel's baked tone curve — `lumit_core::fx::cpu::
/// CURVE_TABLE`, repeated here because `lumit-gpu` does not depend on the
/// core crate (docs/05).
pub const CURVE_TABLE: usize = 257;

/// `vec4`s one channel's table occupies in the uniform: 257 rounded up to a
/// multiple of four.
const CURVE_VEC4S: usize = CURVE_TABLE.div_ceil(4);

/// One resolved Curves (docs/08 §3.30, K-412): five baked tone-curve tables
/// and the mix. The spline is fitted host-side by `Curves::packed`, so this
/// kernel looks up and interpolates and does nothing else — which is what
/// leaves the §1.6 oracle checking the lookup rather than two spline fits.
/// Channel 0 is Master, 1..3 R/G/B, 4 Alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvesOp {
    /// `[channel][entry]`: the curve sampled at `entry / 256`.
    pub t: [[f32; CURVE_TABLE]; 5],
    /// Every channel is the identity diagonal — the bit-exact passthrough,
    /// decided host-side because the kernel cannot compare 1285 numbers a
    /// pixel.
    pub neutral: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CurvesParams {
    /// The five tables, each padded up to `CURVE_VEC4S` vec4s — a uniform
    /// array of scalars would take a 16-byte stride and be four times the
    /// size.
    t: [[f32; 4]; 5 * CURVE_VEC4S],
    mix_amt: f32,
    neutral: u32,
    _pad: [f32; 2],
}

/// One resolved Levels (docs/08 §3.31): five rows indexed `[row][channel]` —
/// input black, the reciprocal input span, the reciprocal gamma, output black
/// and the output span — both reciprocals taken host-side by `Levels::packed`
/// so nothing divides per pixel. Channel 0 is Master, 1..3 R/G/B.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelsOp {
    pub r: [[f32; 4]; 5],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LevelsParams {
    r: [[f32; 4]; 5],
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved Brightness (docs/08 §3.32): AE's Brightness & Contrast pair as
/// the affine grade `(u + b − 0.5)·k + 0.5` on unpremultiplied colour. `b` and
/// `k` are computed host-side; `(0.0, 1.0)` is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrightnessOp {
    /// The scene-linear offset, `Brightness ÷ 100`. 0.0 is neutral.
    pub b: f32,
    /// The contrast factor, `1 + Contrast ÷ 100`. 1.0 is neutral.
    pub k: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BrightnessParams {
    b: f32,
    k: f32,
    mix_amt: f32,
    /// 1 = pull Brightness toward 0 and Contrast toward 1 by the matte (K-395).
    matte_on: f32,
}

/// One resolved Hue and saturation (docs/08 §3.33): seven bands of
/// `(hue degrees, saturation %, lightness %, unused)` — Master first, then the
/// six ranges centred on red, yellow, green, cyan, blue and magenta. All
/// twenty-one adjustments at zero is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueSaturationOp {
    pub bands: [[f32; 4]; 7],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HueSaturationParams {
    bands: [[f32; 4]; 7],
    mix_amt: f32,
    /// 1 = scale every adjustment toward 0 by the matte (K-395).
    matte_on: f32,
    _pad: [f32; 2],
}

/// One resolved hue shift (docs/08 §3.17): a row-major linear 3×3 colour
/// matrix, computed host-side (`lumit_core::fx::hue_matrix`) so the CPU
/// reference and the kernel multiply by identical coefficients. The identity
/// matrix is the neutral point; alpha is untouched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueShiftOp {
    /// The Angle in radians and which matrix it makes (K-136), for the matted
    /// branch (K-395); `m` is the host matrix for the unmatted one.
    pub angle_rad: f32,
    pub preserve: bool,
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
    /// 1 = scale Angle toward 0 by the matte (K-395): the kernel then builds
    /// the matrix for `angle_rad * k` itself, from the same coefficients.
    matte_on: f32,
    angle_rad: f32,
    /// 1 = the constant-luminance matrix, 0 = the plain-RGB spin (K-136).
    preserve: f32,
    _pad: [f32; 3],
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
        matte: Option<&wgpu::Texture>,
        op: &ColourBalanceOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-colour-balance-out");
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        self.dispatch_matted(
            ctx,
            &self.colour_balance,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ColourBalanceParams {
                lift: v4(op.lift),
                gamma: v4(op.gamma),
                gain: v4(op.gain),
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad: [0.0; 2],
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
        matte: Option<&wgpu::Texture>,
        op: &SaturationOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-saturation-out");
        self.dispatch_matted(
            ctx,
            &self.saturation,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SaturationParams {
                saturation: op.saturation,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad0: 0.0,
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
        matte: Option<&wgpu::Texture>,
        op: &VibrancyOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-vibrancy-out");
        self.dispatch_matted(
            ctx,
            &self.vibrancy,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&VibrancyParams {
                amount: op.amount,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad0: 0.0,
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
        matte: Option<&wgpu::Texture>,
        op: &ExposureOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-exposure-out");
        self.dispatch_matted(
            ctx,
            &self.exposure,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ExposureParams {
                factor: op.factor,
                mix_amt: op.mix,
                stops: op.stops,
                matte_on: f32::from(matte.is_some()),
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
        matte: Option<&wgpu::Texture>,
        op: &TemperatureOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-temperature-out");
        self.dispatch_matted(
            ctx,
            &self.temperature,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TemperatureParams {
                gain_r: op.gain_r,
                gain_b: op.gain_b,
                mix_amt: op.mix,
                t: op.t,
                matte_on: f32::from(matte.is_some()),
                _pad: [0.0; 3],
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
        matte: Option<&wgpu::Texture>,
        op: &GammaOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-gamma-out");
        self.dispatch_matted(
            ctx,
            &self.gamma,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&GammaParams {
                gamma: op.gamma,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Curves (docs/08 §3.30) to a linear working texture, returning
    /// a new texture of the same size. One pointwise pass: the per-channel
    /// curve then Master, alpha on its own curve, the §2.2 unpremultiply wrap
    /// fused into the kernel; the identity curve set short-circuits inside it.
    pub fn curves(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &CurvesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-curves-out");
        let mut t = [[0.0f32; 4]; 5 * CURVE_VEC4S];
        for (c, table) in op.t.iter().enumerate() {
            for (i, v) in table.iter().enumerate() {
                t[c * CURVE_VEC4S + i / 4][i % 4] = *v;
            }
        }
        self.dispatch(
            ctx,
            &self.curves,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&CurvesParams {
                t,
                mix_amt: op.mix,
                neutral: u32::from(op.neutral),
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one Levels (docs/08 §3.31) to a linear working texture, returning
    /// a new texture of the same size. One pointwise pass: the per-channel map
    /// then Master, the §2.2 unpremultiply wrap fused into the kernel; neutral
    /// rows short-circuit inside it.
    pub fn levels(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &LevelsOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-levels-out");
        self.dispatch(
            ctx,
            &self.levels,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&LevelsParams {
                r: op.r,
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one Brightness (docs/08 §3.32) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: the
    /// affine grade about mid-grey, the §2.2 unpremultiply wrap fused into the
    /// kernel; the neutral pair short-circuits inside it.
    pub fn brightness(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &BrightnessOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-brightness-out");
        self.dispatch_matted(
            ctx,
            &self.brightness,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BrightnessParams {
                b: op.b,
                k: op.k,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Hue and saturation (docs/08 §3.33) to a linear working
    /// texture, returning a new texture of the same size. One pointwise pass:
    /// the HSV round trip with the master and six weighted ranges, the §2.2
    /// unpremultiply wrap fused into the kernel; all-zero adjustments
    /// short-circuit inside it.
    pub fn hue_saturation(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &HueSaturationOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-hue-saturation-out");
        self.dispatch_matted(
            ctx,
            &self.hue_saturation,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&HueSaturationParams {
                bands: op.bands,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad: [0.0; 2],
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
        matte: Option<&wgpu::Texture>,
        op: &HueShiftOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-hue-out");
        self.dispatch_matted(
            ctx,
            &self.hue_shift,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&HueParams {
                m: op.m,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                angle_rad: op.angle_rad,
                preserve: f32::from(op.preserve),
                _pad: [0.0; 3],
            }),
        );
        out
    }
}

/// One resolved Posterize (docs/08 §3.58): the tone ladder cut into `n + 1`
/// rungs, spaced evenly in a square root of the light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PosterizeOp {
    /// Levels − 1, computed host-side.
    pub n: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PosterizeParams {
    n: f32,
    mix_amt: f32,
    /// 1 = pull the step count toward 255 by the matte (K-395).
    matte_on: f32,
    _pad1: f32,
}

/// One resolved Threshold (docs/08 §3.59): the cut's perceptual position and
/// half-width, both computed host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdOp {
    /// Level ÷ 100.
    pub level: f32,
    /// Half the crossing's width, floored at a thousandth of the range.
    pub half_width: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ThresholdParams {
    level: f32,
    hw: f32,
    mix_amt: f32,
    /// 1 = scale the level by the matte (K-559).
    matte_on: f32,
}

/// One resolved Tritone (docs/08 §3.60): the three stops of the ramp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TritoneOp {
    pub shadows: [f32; 3],
    pub midtones: [f32; 3],
    pub highlights: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TritoneParams {
    shadows: [f32; 4],
    midtones: [f32; 4],
    highlights: [f32; 4],
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved Photo filter (docs/08 §3.61): the glass's scene-linear colour,
/// already decoded host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoFilterOp {
    pub filter: [f32; 3],
    /// Density ÷ 100. 0.0 is the bit-exact identity.
    pub density: f32,
    /// 1.0 to restore the pixel's own luma afterwards, 0.0 to let the filter
    /// cost light as a real one does.
    pub preserve: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PhotoFilterParams {
    filter: [f32; 4],
    density: f32,
    preserve: f32,
    mix_amt: f32,
    /// 1 = scale Density by the matte (K-395).
    matte_on: f32,
}

/// One resolved Black and white (docs/08 §3.62): the six weights as fractions
/// in red, yellow, green, cyan, blue, magenta order, and the tint already
/// divided through by its own luma.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackAndWhiteOp {
    pub weights: [f32; 6],
    pub tint: [f32; 3],
    /// 1.0 to tint, 0.0 to leave the grey grey.
    pub tint_on: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlackAndWhiteParams {
    w0: [f32; 4],
    w1: [f32; 4],
    tint: [f32; 4],
    tint_on: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved Shadow highlight (docs/08 §3.63). `radius_px` drives the
/// shipped gaussian the kernel reads its neighbourhood from; `active` false
/// never reaches the GPU at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowHighlightOp {
    /// Shadow amount ÷ 100 × 2.
    pub shadow: f32,
    /// Highlight amount ÷ 100 × 2.
    pub highlight: f32,
    /// Shadow tonal width ÷ 100, floored host-side.
    pub shadow_width: f32,
    /// Highlight tonal width ÷ 100, floored host-side.
    pub highlight_width: f32,
    /// The neighbourhood's radius, in raster pixels.
    pub radius_px: f32,
    /// 1 + Midtone contrast ÷ 100.
    pub contrast: f32,
    /// Colour correction ÷ 100.
    pub colour_correction: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowHighlightParams {
    shadow: f32,
    highlight: f32,
    shadow_width: f32,
    highlight_width: f32,
    contrast: f32,
    colour_correction: f32,
    mix_amt: f32,
    /// 1 = scale Shadow amount and Highlight amount by the matte (K-395).
    matte_on: f32,
}

/// Wave 2's Stylise I batch (docs/08 §3.58–§3.63, K-404): six tone and colour
/// effects, five of them one pointwise pass and the sixth one gaussian plus a
/// pointwise pass.
impl FxEngine {
    /// Apply one Posterize (docs/08 §3.58) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the §2.2
    /// unpremultiply wrap is fused into the kernel.
    pub fn posterize(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &PosterizeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-posterize-out");
        self.dispatch_matted(
            ctx,
            &self.posterize,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&PosterizeParams {
                n: op.n,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Threshold (docs/08 §3.59) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; alpha is
    /// untouched, so a thresholded picture keeps its shape. A bound `matte`
    /// scales the level per pixel (K-559).
    pub fn threshold(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &ThresholdOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-threshold-out");
        self.dispatch_matted(
            ctx,
            &self.threshold,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ThresholdParams {
                level: op.level,
                hw: op.half_width,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Tritone (docs/08 §3.60) to a linear working texture, returning
    /// a new texture of the same size. One pointwise pass.
    pub fn tritone(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TritoneOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-tritone-out");
        let rgb = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
        self.dispatch(
            ctx,
            &self.tritone,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TritoneParams {
                shadows: rgb(op.shadows),
                midtones: rgb(op.midtones),
                highlights: rgb(op.highlights),
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one Photo filter (docs/08 §3.61) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; Density 0
    /// short-circuits inside the kernel.
    pub fn photo_filter(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &PhotoFilterOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-photo-filter-out");
        self.dispatch_matted(
            ctx,
            &self.photo_filter,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&PhotoFilterParams {
                filter: [op.filter[0], op.filter[1], op.filter[2], 1.0],
                density: op.density,
                preserve: op.preserve,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Black and white (docs/08 §3.62) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass.
    pub fn black_and_white(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BlackAndWhiteOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-black-and-white-out");
        self.dispatch(
            ctx,
            &self.black_and_white,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlackAndWhiteParams {
                w0: [op.weights[0], op.weights[1], op.weights[2], op.weights[3]],
                w1: [op.weights[4], op.weights[5], 0.0, 0.0],
                tint: [op.tint[0], op.tint[1], op.tint[2], 1.0],
                tint_on: op.tint_on,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Shadow highlight (docs/08 §3.63) to a linear working texture,
    /// returning a new texture of the same size. Two passes: the shipped §3.8
    /// gaussian at Radius, whose luma answers "how bright is this pixel's
    /// neighbourhood?", and then one pointwise pass that never reads the blur's
    /// colour.
    pub fn shadow_highlight(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &ShadowHighlightOp,
    ) -> wgpu::Texture {
        let soft = self.blur(
            ctx,
            src,
            w,
            h,
            None,
            &super::BlurOp {
                radius_px: op.radius_px,
                // Repeat: the frame's own border must not read as a dark
                // neighbourhood and lift the picture's edges.
                edge: 1,
                mix: 1.0,
            },
        );
        let out = work_texture(ctx, w, h, "fx-shadow-highlight-out");
        self.dispatch_matted(
            ctx,
            &self.shadow_highlight,
            src,
            &soft,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ShadowHighlightParams {
                shadow: op.shadow,
                highlight: op.highlight,
                shadow_width: op.shadow_width,
                highlight_width: op.highlight_width,
                contrast: op.contrast,
                colour_correction: op.colour_correction,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }
}

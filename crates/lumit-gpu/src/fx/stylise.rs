//! Stylise and geometry kernels (docs/08 §3.5, §3.12, §3.14, §3.21): the matte
//! key, vignette, affine transform, block glitch and scanlines.

use crate::GpuContext;

use super::{work_texture, BlurOp, FxEngine};

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
    /// Screen pre-blur radius, raster pixels (K-546). 0 skips the stage.
    pub pre_blur: f32,
    /// Screen matte shrink (−) / grow (+), raster pixels (K-546). 0 skips it.
    pub shrink_grow: f32,
    /// Screen matte softness (a blur of the matte), raster pixels (K-546).
    pub softness: f32,
    /// Despot black, 0..1 (K-546).
    pub despot_black: f32,
    /// Despot white, 0..1 (K-546).
    pub despot_white: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl MatteKeyOp {
    /// Whether any spatial stage is asked for — mirrors
    /// `lumit_core::fx::MatteKeyParams::spatial`, so both render paths choose
    /// the fast pointwise kernel or the staged pipeline on the same predicate.
    #[must_use]
    pub fn spatial(&self) -> bool {
        self.pre_blur > 0.0
            || self.shrink_grow != 0.0
            || self.softness > 0.0
            || self.despot_black > 0.0
            || self.despot_white > 0.0
    }
}

/// The most straight pieces one garbage mask's outline is walked as — mirrors
/// `lumit_core::fx::cpu::MASK_FILL_SEGMENTS` (K-546).
pub const MASK_FILL_SEGMENTS: usize = 512;

/// The farthest a shrink or grow may reach, raster pixels — mirrors
/// `lumit_core::fx::cpu::MATTE_MORPH_MAX_PX` (K-546). Both paths clamp, so both
/// paths reach the same distance when a nonsense number arrives.
pub const MATTE_MORPH_MAX_PX: f32 = 256.0;

/// One resolved garbage mask (docs/08 §3.21, K-546): a closed outline in raster
/// pixels and how soft its edge is. Mirrors `lumit_core::fx::cpu::MaskFillParams`
/// field-for-field, exactly as [`MatteKeyOp`] mirrors its own params (K-031).
///
/// A `count` of zero is the row's no-op: the pass is not dispatched at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskFillOp {
    /// The outline, as `(ax, ay, bx, by)` in raster pixels, closed.
    pub segments: [[f32; 4]; MASK_FILL_SEGMENTS],
    /// How many of the above are real.
    pub count: u32,
    /// The feather ramp's width, raster pixels, never below one.
    pub ramp: f32,
    /// The mask's own grow (+) / shrink (−), raster pixels.
    pub expansion: f32,
}

impl MaskFillOp {
    /// A mask that holds nothing out.
    #[must_use]
    pub fn blank() -> Self {
        Self {
            segments: [[0.0; 4]; MASK_FILL_SEGMENTS],
            count: 0,
            ramp: 1.0,
            expansion: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaskFillParams {
    count: u32,
    ramp: f32,
    expansion: f32,
    /// 0 = inside (force opaque), 1 = outside (force transparent).
    mode: u32,
    segments: [[f32; 4]; MASK_FILL_SEGMENTS],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatteTidyParams {
    dx: i32,
    dy: i32,
    ri: i32,
    frac: f32,
    grow: u32,
    black: f32,
    white: f32,
    _pad0: f32,
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

/// The uniform the pointwise kernel, the screen pass and the combine pass all
/// read — the same bytes for all three, since the spatial stages are told what
/// to do by their own small uniforms and never by this one.
fn matte_key_params(op: &MatteKeyOp) -> MatteKeyParams {
    MatteKeyParams {
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
    }
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
    /// 1 = scale the displacement by the matte (K-427) — the Shake's claim;
    /// the Transform effect never binds one.
    matte_on: f32,
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
    /// 1 = scale every tap's displacement by the matte (K-427).
    matte_on: f32,
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
    /// 1 = scale Intensity by the matte (K-427).
    matte_on: f32,
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
    /// 1 = widen Line period by the matte (K-427).
    matte_on: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Roughen edges (docs/08 §3.57). Mirrors
/// `lumit_core::fx::cpu::RoughenEdgesParams` with its `FractalField` flattened,
/// which is the shape the uniform wants anyway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoughenEdgesOp {
    pub seed: u32,
    /// 1..=10.
    pub octaves: u32,
    pub gain: f32,
    pub lacunarity: f32,
    /// Depth loop length in cells; 0 for a field that never repeats.
    pub cycle: i32,
    /// Bit 0 Perlin, bit 1 Turbulent (the Spiky edge type).
    pub flags: u32,
    /// The field's origin, raster pixels.
    pub offset: [f32; 2],
    /// `1 ÷ Scale`, raster pixels.
    pub inv_scale: f32,
    /// The field's depth coordinate.
    pub z: f32,
    /// Border, raster pixels: the first pass's gaussian radius.
    pub border_px: f32,
    /// Fractal influence ÷ 100.
    pub influence: f32,
    /// Half the cut's width, in alpha.
    pub half_width: f32,
    /// Scene-linear RGB the chewed band is painted in.
    pub colour: [f32; 3],
    /// 1 to paint the band, 0 to leave it.
    pub colour_on: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RoughenEdgesParams {
    colour: [f32; 4],
    offset: [f32; 2],
    inv_scale: f32,
    z: f32,
    gain: f32,
    lacunarity: f32,
    influence: f32,
    half_width: f32,
    colour_on: f32,
    mix_amt: f32,
    seed: u32,
    octaves: u32,
    cycle: i32,
    flags: u32,
    _pad0: u32,
    _pad1: u32,
}

impl FxEngine {
    /// Apply one Roughen edges (docs/08 §3.57) to a linear working texture,
    /// returning a new texture of the same size.
    ///
    /// **Two passes, and the first is the shipped §3.8 gaussian.** Blurring the
    /// picture by Border turns its alpha into a ramp whose half-way contour sits
    /// exactly where the original edge was and whose slope is Border wide — the
    /// distance field the roughening needs, without a distance transform. §3.43
    /// reuses the same blur for its own reasons; this is the second time it has
    /// paid.
    pub fn roughen_edges(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &RoughenEdgesOp,
    ) -> wgpu::Texture {
        // The whole claim: Border IS this blur's radius, and the matte scales a
        // radius per pixel already (K-428, K-395). The second pass needs no
        // matte of its own — a narrower ramp is a narrower band to chew.
        let soft = self.blur(
            ctx,
            src,
            w,
            h,
            matte,
            &super::BlurOp {
                radius_px: op.border_px,
                // Transparent: the shape's own edge is what is being measured,
                // and repeating the border pixel outward would put a phantom
                // edge along the frame's own sides.
                edge: 0,
                mix: 1.0,
            },
        );
        let out = work_texture(ctx, w, h, "fx-roughen-edges-out");
        self.dispatch(
            ctx,
            &self.roughen_edges,
            src,
            &soft,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RoughenEdgesParams {
                colour: [op.colour[0], op.colour[1], op.colour[2], 1.0],
                offset: op.offset,
                inv_scale: op.inv_scale,
                z: op.z,
                gain: op.gain,
                lacunarity: op.lacunarity,
                influence: op.influence,
                half_width: op.half_width,
                colour_on: op.colour_on,
                mix_amt: op.mix,
                seed: op.seed,
                octaves: op.octaves,
                cycle: op.cycle,
                flags: op.flags,
                _pad0: 0,
                _pad1: 0,
            }),
        );
        out
    }

    /// Apply one matte key (docs/08 §3.21, K-121/K-154, K-546) to a linear
    /// working texture, returning a new texture of the same size.
    ///
    /// **The default is one pointwise pass** — the §2.2 unpremultiply wrap fused
    /// into the kernel, the screen's primary channel and reference derived from
    /// `key` exactly as the CPU reference derives them — and that is the whole
    /// of it whenever no spatial control is asked for and neither garbage mask
    /// is set, so an old project renders the bytes it always did.
    ///
    /// Ask for a spatial control and the matte becomes a picture of its own for
    /// the length of the effect: pre-blur the picture the key is judged from,
    /// write the matte, march its edge, blur it, de-speck it, cut the garbage
    /// masks into it, and only then spend it on the original colour. Seven
    /// stages, in the order `lumit_core::fx::cpu::matte_key_spatial` runs them
    /// (§1.6: the CPU is the oracle). Every stage that is not asked for is
    /// skipped rather than run at a neutral setting.
    ///
    /// There is no neutral short-circuit (the default keys); Mix 0 is the
    /// identity.
    #[allow(clippy::too_many_arguments)]
    pub fn matte_key(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &MatteKeyOp,
        inside: &MaskFillOp,
        outside: &MaskFillOp,
    ) -> wgpu::Texture {
        let params = matte_key_params(op);
        let bytes = bytemuck::bytes_of(&params);
        if !op.spatial() && inside.count == 0 && outside.count == 0 {
            let out = work_texture(ctx, w, h, "fx-matte-key-out");
            self.dispatch(ctx, &self.matte_key, src, src, &out, w, h, bytes);
            return out;
        }

        // 1. The picture the key is judged from. The colour that comes out is
        // still `src`, sharp — only the judgement is softened.
        let blurred = (op.pre_blur > 0.0).then(|| {
            self.blur(
                ctx,
                src,
                w,
                h,
                None,
                &BlurOp {
                    radius_px: op.pre_blur,
                    edge: 1,
                    mix: 1.0,
                },
            )
        });
        let key_src = blurred.as_ref().unwrap_or(src);

        // 2. The matte, as an ordinary working texture with the same number in
        // every channel — which is what lets stage 4 be the shared blur.
        let mut matte = work_texture(ctx, w, h, "fx-matte-key-matte");
        self.dispatch(
            ctx,
            &self.matte_key_screen,
            key_src,
            key_src,
            &matte,
            w,
            h,
            bytes,
        );

        // 3. Shrink/grow: two separable morphological passes.
        if op.shrink_grow != 0.0 {
            let r = op.shrink_grow.abs().min(MATTE_MORPH_MAX_PX);
            let ri = r.floor() as i32;
            let frac = r - ri as f32;
            let grow = u32::from(op.shrink_grow > 0.0);
            let tidy = |dx: i32, dy: i32| MatteTidyParams {
                dx,
                dy,
                ri,
                frac,
                grow,
                black: 0.0,
                white: 0.0,
                _pad0: 0.0,
            };
            let tmp = work_texture(ctx, w, h, "fx-matte-key-morph-h");
            self.dispatch(
                ctx,
                &self.matte_morph,
                &matte,
                &matte,
                &tmp,
                w,
                h,
                bytemuck::bytes_of(&tidy(1, 0)),
            );
            let out = work_texture(ctx, w, h, "fx-matte-key-morph-v");
            self.dispatch(
                ctx,
                &self.matte_morph,
                &tmp,
                &tmp,
                &out,
                w,
                h,
                bytemuck::bytes_of(&tidy(0, 1)),
            );
            matte = out;
        }

        // 4. Softness: the shared Gaussian blur, on the matte alone.
        if op.softness > 0.0 {
            matte = self.blur(
                ctx,
                &matte,
                w,
                h,
                None,
                &BlurOp {
                    radius_px: op.softness,
                    edge: 1,
                    mix: 1.0,
                },
            );
        }

        // 5. Despot black and white, in one pass.
        if op.despot_black > 0.0 || op.despot_white > 0.0 {
            let out = work_texture(ctx, w, h, "fx-matte-key-despot");
            self.dispatch(
                ctx,
                &self.matte_despot,
                &matte,
                &matte,
                &out,
                w,
                h,
                bytemuck::bytes_of(&MatteTidyParams {
                    dx: 0,
                    dy: 0,
                    ri: 0,
                    frac: 0.0,
                    grow: 0,
                    black: op.despot_black,
                    white: op.despot_white,
                    _pad0: 0.0,
                }),
            );
            matte = out;
        }

        // 6. The garbage masks: inside first, then outside — the order the CPU
        // reference applies them in.
        for (m, mode) in [(inside, 0u32), (outside, 1u32)] {
            if m.count == 0 {
                continue;
            }
            let out = work_texture(ctx, w, h, "fx-matte-key-mask");
            self.dispatch(
                ctx,
                &self.matte_mask,
                &matte,
                &matte,
                &out,
                w,
                h,
                bytemuck::bytes_of(&MaskFillParams {
                    count: m.count,
                    ramp: m.ramp,
                    expansion: m.expansion,
                    mode,
                    segments: m.segments,
                }),
            );
            matte = out;
        }

        // 7. Spend the finished matte on the original colour.
        let out = work_texture(ctx, w, h, "fx-matte-key-out");
        self.dispatch(ctx, &self.matte_key_combine, src, &matte, &out, w, h, bytes);
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
    ///
    /// `matte` is the Shake's claim (K-427): it scales the displacement the
    /// affine gives each pixel toward none, read at the destination pixel. The
    /// Transform effect passes `None` and keeps the strength dissolve.
    pub fn transform(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &TransformOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-transform-out");
        self.dispatch_matted(
            ctx,
            &self.transform,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TransformParams {
                m: op.m,
                off: op.off,
                opacity: op.opacity,
                mix_amt: op.mix,
                edge: op.edge,
                matte_on: f32::from(matte.is_some()),
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
        matte: Option<&wgpu::Texture>,
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
        self.dispatch_matted(
            ctx,
            &self.shake_mb,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ShakeMbParams {
                taps,
                count: op.count.clamp(1, SHAKE_MB_SAMPLES as u32),
                edge: op.edge,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
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
        matte: Option<&wgpu::Texture>,
        op: &BlockGlitchOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-block-glitch-out");
        self.dispatch_matted(
            ctx,
            &self.block_glitch,
            src,
            src,
            matte,
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
                matte_on: f32::from(matte.is_some()),
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
        matte: Option<&wgpu::Texture>,
        op: &ScanlinesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-scanlines-out");
        self.dispatch_matted(
            ctx,
            &self.scanlines,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ScanlinesParams {
                intensity: op.intensity,
                period: op.period_px,
                roll_px: op.roll_px,
                interlace: u32::from(op.interlace),
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }
}

/// One resolved Median (docs/08 §3.64). Mirrors
/// `lumit_core::fx::cpu::MedianParams`, with the network's run length worked out
/// beside it so the kernel never derives a count of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MedianOp {
    /// Half the window's width, whole raster pixels, `0..=3`.
    pub radius: i32,
    /// `⌈(2r+1)² ÷ 2⌉`: how many of the smallest the selection network carries,
    /// and therefore the 1-based rank of the median.
    pub keep: i32,
    /// 1 to median the coverage with the colour, 0 to leave it.
    pub alpha_on: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MedianParams {
    radius: i32,
    keep: i32,
    alpha_on: f32,
    mix_amt: f32,
    /// 1 = the matte scales Radius per pixel (K-428).
    matte_on: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Mosaic (docs/08 §3.65). Mirrors
/// `lumit_core::fx::cpu::MosaicParams`; the kernel derives the block bounds from
/// its own `textureDimensions`, in integers, exactly as the CPU reference
/// derives them from `w`/`h`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MosaicOp {
    /// Blocks across and blocks down, each `1..=2000`.
    pub blocks: [i32; 2],
    /// 1 for the block's centre pixel, 0 for the sampled mean.
    pub sharp: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MosaicParams {
    blocks_x: i32,
    blocks_y: i32,
    sharp: f32,
    mix_amt: f32,
}

/// One resolved Find edges (docs/08 §3.66).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FindEdgesOp {
    /// 1 for bright edges on black, 0 for AE's dark edges on white.
    pub invert: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FindEdgesParams {
    invert: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved Emboss (docs/08 §3.67). Mirrors
/// `lumit_core::fx::cpu::EmbossParams`: Direction and Relief arrive folded into
/// one vector, so the kernel never runs its own trigonometry (§3.5's rule).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbossOp {
    /// Toward the light, raster pixels.
    pub offset: [f32; 2],
    /// Contrast ÷ 100.
    pub contrast: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EmbossParams {
    offset: [f32; 2],
    contrast: f32,
    mix_amt: f32,
    /// 1 = the matte scales Relief per pixel (K-428).
    matte_on: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Texturize (docs/08 §3.68). Mirrors
/// `lumit_core::fx::cpu::TexturizeParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexturizeOp {
    /// Toward the light, raster pixels.
    pub offset: [f32; 2],
    /// Texture contrast ÷ 100.
    pub contrast: f32,
    /// `100 ÷ Scale`.
    pub inv_scale: f32,
    /// 0 Stretch, 1 Tile, 2 Centre.
    pub placement: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TexturizeParams {
    offset: [f32; 2],
    contrast: f32,
    inv_scale: f32,
    placement: u32,
    mix_amt: f32,
    /// 1 = the matte scales Relief per pixel (K-428).
    matte_on: f32,
    _pad1: f32,
}

impl FxEngine {
    /// Apply one Median (docs/08 §3.64) to a linear working texture, returning a
    /// new texture of the same size.
    ///
    /// One pass, and the catalogue's only `heavy` one: up to 1 225
    /// compare-exchanges a pixel. Radius 0 short-circuits inside the kernel to
    /// the bit-exact identity.
    pub fn median(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &MedianOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-median-out");
        self.dispatch_matted(
            ctx,
            &self.median,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&MedianParams {
                radius: op.radius,
                keep: op.keep.max(1),
                alpha_on: op.alpha_on,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Mosaic (docs/08 §3.65) to a linear working texture, returning a
    /// new texture of the same size. One pass: one tap in the sharp mode, at
    /// most 64 in the averaged one.
    pub fn mosaic(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &MosaicOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-mosaic-out");
        self.dispatch(
            ctx,
            &self.mosaic,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&MosaicParams {
                blocks_x: op.blocks[0],
                blocks_y: op.blocks[1],
                sharp: op.sharp,
                mix_amt: op.mix,
            }),
        );
        out
    }

    /// Apply one Find edges (docs/08 §3.66) to a linear working texture,
    /// returning a new texture of the same size. One pass, eight taps a pixel.
    pub fn find_edges(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FindEdgesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-find-edges-out");
        self.dispatch(
            ctx,
            &self.find_edges,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&FindEdgesParams {
                invert: op.invert,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Emboss (docs/08 §3.67) to a linear working texture, returning a
    /// new texture of the same size. One pass, two bilinear taps a pixel.
    pub fn emboss(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &EmbossOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-emboss-out");
        self.dispatch_matted(
            ctx,
            &self.emboss,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&EmbossParams {
                offset: op.offset,
                contrast: op.contrast,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Texturize (docs/08 §3.68) to a linear working texture,
    /// returning a new texture of the same size.
    ///
    /// `texture` is the Texture row's layer, already rendered at this raster
    /// (docs/impl/layer-input.md) and bound in the `orig` slot — this being a
    /// single pass, `src` is already its own unprocessed original. **An unset
    /// row is the identity**, returned here rather than in the kernel, because a
    /// texture that does not exist is not a texture of zero relief.
    #[allow(clippy::too_many_arguments)]
    pub fn texturize(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        texture: Option<&wgpu::Texture>,
        matte: Option<&wgpu::Texture>,
        op: &TexturizeOp,
    ) -> wgpu::Texture {
        let Some(texture) = texture else {
            return src.clone();
        };
        let out = work_texture(ctx, w, h, "fx-texturize-out");
        self.dispatch_matted(
            ctx,
            &self.texturize,
            src,
            texture,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TexturizeParams {
                offset: op.offset,
                contrast: op.contrast,
                inv_scale: op.inv_scale,
                placement: op.placement,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Stroke **style** (K-706, docs/impl/layer-styles.md §4) to a
    /// linear working texture, returning a new texture of the same size.
    ///
    /// Three passes: the two separable morphological passes that carry the
    /// fattened and thinned copies of the alpha together, then one combine that
    /// cuts the band between them, paints it and lays it over the layer.
    pub fn stroke_contour(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &StrokeOp,
    ) -> wgpu::Texture {
        // The rings and the fractional easing are taken once here rather than
        // per pixel (K-137's host-side arithmetic rule), by exactly the
        // expressions `cpu::matte_morph` takes them by — same clamp, same floor,
        // same remainder — so the two paths cannot disagree about how wide the
        // element is, and a nonsense number cannot turn a pass into a hang.
        let rings = |px: f32| {
            let r = px.abs().min(MATTE_MORPH_MAX_PX);
            let ri = r.floor() as i32;
            (ri, r - ri as f32)
        };
        let (grow_ri, grow_frac) = rings(op.grow_px);
        let (shrink_ri, shrink_frac) = rings(op.shrink_px);
        let params = |dx: i32, dy: i32, stage: u32| StrokeParams {
            colour: [op.colour[0], op.colour[1], op.colour[2], 1.0],
            dx,
            dy,
            grow_ri,
            grow_frac,
            shrink_ri,
            shrink_frac,
            opacity: op.opacity,
            mix_amt: op.mix,
            stage,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        let tmp = work_texture(ctx, w, h, "fx-stroke-morph-h");
        self.dispatch(
            ctx,
            &self.stroke_morph,
            src,
            src,
            &tmp,
            w,
            h,
            bytemuck::bytes_of(&params(1, 0, 0)),
        );
        let pair = work_texture(ctx, w, h, "fx-stroke-morph-v");
        self.dispatch(
            ctx,
            &self.stroke_morph,
            &tmp,
            &tmp,
            &pair,
            w,
            h,
            bytemuck::bytes_of(&params(0, 1, 1)),
        );
        let out = work_texture(ctx, w, h, "fx-stroke-out");
        self.dispatch(
            ctx,
            &self.stroke_combine,
            src,
            &pair,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params(0, 0, 1)),
        );
        out
    }
}

/// One resolved Stroke **style** (K-706). Mirrors
/// `lumit_core::fx::cpu::StrokeContourParams` field-for-field so the kernel and
/// the CPU oracle consume the identical numbers (K-031); the Position choice was
/// already spent into the two half-widths by `StrokeStyle::packed`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeOp {
    /// Scene-linear RGB; the band's coverage supplies the rest.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// How far the alpha is grown outward before the band is cut, raster pixels.
    pub grow_px: f32,
    /// How far it is shrunk inward, raster pixels.
    pub shrink_px: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StrokeParams {
    colour: [f32; 4],
    dx: i32,
    dy: i32,
    grow_ri: i32,
    grow_frac: f32,
    shrink_ri: i32,
    shrink_frac: f32,
    opacity: f32,
    mix_amt: f32,
    /// 0 = the first separable pass (read the layer's alpha), 1 = the second.
    stage: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

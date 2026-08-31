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
    /// 1 = scale Length by the matte (K-395).
    matte_on: f32,
    _pad0: f32,
}

/// One resolved radial blur — Blur's Radial mode (docs/08 §3.8, schema
/// status note). `taps` must equal
/// `lumit_core::fx::cpu::radial_blur_taps(amount_px)` so the GPU dispatches
/// the oracle's exact kernel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialBlurOp {
    /// Centre in raster pixels (K-558: px@comp, already scaled to this
    /// raster by the resolve step) — the kernel reads it as it stands,
    /// exactly like the CPU reference does.
    pub centre_px: [f32; 2],
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
    /// 1 = scale Amount by the matte (K-395).
    matte_on: f32,
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
pub(super) struct BlurParams {
    pub(super) dir: [f32; 2],
    pub(super) radius: f32,
    pub(super) sigma: f32,
    pub(super) edge: u32,
    pub(super) mix_amt: f32,
    /// 1 = scale the radius by the bound matte's luma (K-395). 0 on every
    /// internal blur — the glow's halo, the sharpen's unsharp pass and Light
    /// wrap's spill are not the user's Gaussian blur, and their matte (if any)
    /// has already been spent elsewhere.
    pub(super) matte_on: f32,
    /// Was the Matte's Invert; since K-425 the seam applies it once, before
    /// the kernel (`FxEngine::matte_prepare`), and this pad is always 0.
    pub(super) _pad0: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SharpenParams {
    amount: f32,
    threshold: f32,
    luma_only: u32,
    mix_amt: f32,
    /// 1 = scale Amount by the matte (K-395); the combine pass alone reads it.
    matte_on: f32,
    _pad: [f32; 3],
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
    /// 1 = scale Amount by the matte (K-395).
    matte_on: f32,
}

/// One resolved light wrap (docs/08 §3.28, K-358): the background's light
/// spilled around the foreground's edge. A zero width, intensity or mix is the
/// bit-exact passthrough — there is no band to fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightWrapOp {
    /// How far the wrap reaches inside the edge, raster pixels — and the
    /// radius the background is softened by, which are the same distance.
    pub width_px: f32,
    /// Gain on the spill before it is screened on.
    pub intensity: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightWrapParams {
    w: u32,
    h: u32,
    intensity: f32,
    mix_amt: f32,
}

/// One resolved sprite flare (docs/08 §3.29, K-359) — the art-directed flare,
/// placed from a light position rather than from the picture's bright pixels.
/// Mirrors `lumit_core::fx::cpu::SpriteFlareParams`; this crate never depends
/// on `lumit-core` (docs/05 §architecture), so the shape is restated rather
/// than shared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteFlareOp {
    /// Where the light is, in raster pixels.
    pub light: [f32; 2],
    /// Master gain; 0 is the bit-exact passthrough.
    pub intensity: f32,
    /// Scene-linear RGB every element is multiplied by.
    pub tint: [f32; 3],
    pub glow_size: f32,
    pub glow_intensity: f32,
    pub ghosts: u32,
    pub ghost_spacing: f32,
    pub ghost_size: f32,
    pub ghost_intensity: f32,
    pub streak_length: f32,
    pub streak_intensity: f32,
    pub streak_angle_deg: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// The sprite flare's uniform block (docs/08 §3.29, K-359) — field for field
/// what `fx_sprite_flare.wgsl` declares.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteFlareParams {
    w: u32,
    h: u32,
    light_x: f32,
    light_y: f32,
    intensity: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    glow_size: f32,
    glow_intensity: f32,
    ghosts: u32,
    ghost_spacing: f32,
    ghost_size: f32,
    ghost_intensity: f32,
    streak_length: f32,
    streak_intensity: f32,
    streak_angle_deg: f32,
    mix_amt: f32,
    _pad: [f32; 2],
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
pub(super) struct GlowParams {
    pub(super) tint: [f32; 4],
    pub(super) threshold: f32,
    pub(super) knee: f32,
    pub(super) intensity: f32,
    pub(super) mix_amt: f32,
    /// 1 = gate the bright pass by the matte's luma (K-395). The combine pass
    /// reads the same uniform and ignores both fields.
    pub(super) matte_on: f32,
    /// Was Invert; the seam applies it once since K-425. Always 0.
    pub(super) _pad0: f32,
    pub(super) _pad: [f32; 2],
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
        matte: Option<&wgpu::Texture>,
        op: &BlurOp,
    ) -> wgpu::Texture {
        let tmp = work_texture(ctx, w, h, "fx-blur-tmp");
        let out = work_texture(ctx, w, h, "fx-blur-out");
        let sigma = (op.radius_px * 0.5).max(1e-3);
        // The Matte scales the radius per pixel (K-395) — see fx_blur.wgsl.
        // Both passes carry it, because each reads its DESTINATION pixel's
        // matte and the two halves must agree on this pixel's kernel width.
        let matte_on = f32::from(matte.is_some());
        // Horizontal into tmp (mix 1: the blend happens once, at the end).
        self.dispatch_matted(
            ctx,
            &self.blur,
            src,
            src,
            matte,
            &tmp,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [1.0, 0.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: 1.0,
                matte_on,
                _pad0: 0.0,
            }),
        );
        // Vertical into out, blending against the original input.
        self.dispatch_matted(
            ctx,
            &self.blur,
            &tmp,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [0.0, 1.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: op.mix,
                matte_on,
                _pad0: 0.0,
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
        matte: Option<&wgpu::Texture>,
        op: &DirBlurOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-dir-blur-out");
        self.dispatch_matted(
            ctx,
            &self.dir_blur,
            src,
            src,
            matte,
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
                matte_on: f32::from(matte.is_some()),
                _pad0: 0.0,
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
        matte: Option<&wgpu::Texture>,
        op: &RadialBlurOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-radial-blur-out");
        self.dispatch_matted(
            ctx,
            &self.radial_blur,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RadialBlurParams {
                centre: op.centre_px,
                amount: op.amount_px,
                taps: op.taps,
                spin: u32::from(op.spin),
                edge: op.edge,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
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
        matte: Option<&wgpu::Texture>,
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
            matte_on: f32::from(matte.is_some()),
            _pad: [0.0; 3],
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
                    matte_on: 0.0,
                    _pad0: 0.0,
                }),
            );
        }
        self.dispatch_matted(
            ctx,
            &self.sharpen_combine,
            &blurred,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }

    /// **Sprite flare** (docs/08 §3.29, K-359): the art-directed flare, drawn
    /// from a light POSITION rather than from the picture's bright pixels — so
    /// it cannot flicker on footage, because there is no threshold to cross.
    ///
    /// One procedural pass, no inputs but the layer itself. Intensity 0 and
    /// Mix 0 are the bit-exact passthrough, matching the CPU reference's
    /// short-circuit.
    pub fn sprite_flare(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        p: &SpriteFlareOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-sprite-flare-out");
        self.dispatch(
            ctx,
            &self.sprite_flare,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SpriteFlareParams {
                w,
                h,
                light_x: p.light[0],
                light_y: p.light[1],
                intensity: p.intensity,
                tint_r: p.tint[0],
                tint_g: p.tint[1],
                tint_b: p.tint[2],
                glow_size: p.glow_size,
                glow_intensity: p.glow_intensity,
                ghosts: p.ghosts,
                ghost_spacing: p.ghost_spacing,
                ghost_size: p.ghost_size,
                ghost_intensity: p.ghost_intensity,
                streak_length: p.streak_length,
                streak_intensity: p.streak_intensity,
                streak_angle_deg: p.streak_angle_deg,
                mix_amt: p.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// **Light wrap** (docs/08 §3.28, K-358): spill the background's light
    /// around the foreground's edge, so a keyed subject sits *in* the plate
    /// rather than on it.
    ///
    /// Four passes, of which two are the ordinary gaussian: blur the
    /// background over the wrap's width to get the spill, blur the foreground
    /// over the same width to get its softened matte (only the alpha is
    /// wanted, and blurring the whole thing gets it for nothing), fold the two
    /// into one texture, then screen the spill onto the edge band. The
    /// `lumit_core::fx::cpu::light_wrap` reference does the same four steps in
    /// the same order.
    ///
    /// A zero width, intensity or mix is the bit-exact passthrough — there is
    /// no band to fill — and the caller short-circuits before reaching here.
    pub fn light_wrap(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        background: &wgpu::Texture,
        op: &LightWrapOp,
    ) -> wgpu::Texture {
        let blur = BlurOp {
            radius_px: op.width_px,
            // Repeat the edge pixel, so a subject touching the frame border
            // wraps with the plate rather than with black.
            edge: 1,
            mix: 1.0,
        };
        let spill = self.blur(ctx, background, w, h, None, &blur);
        let soft = self.blur(ctx, src, w, h, None, &blur);
        let params = LightWrapParams {
            w,
            h,
            intensity: op.intensity,
            mix_amt: op.mix,
        };
        let packed = work_texture(ctx, w, h, "fx-light-wrap-packed");
        self.dispatch(
            ctx,
            &self.light_wrap_pack,
            &spill,
            &soft,
            &packed,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        let out = work_texture(ctx, w, h, "fx-light-wrap-out");
        self.dispatch(
            ctx,
            &self.light_wrap_combine,
            src,
            &packed,
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
        matte: Option<&wgpu::Texture>,
        op: &SharpenSimpleOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-sharpen-simple-out");
        self.dispatch_matted(
            ctx,
            &self.sharpen_simple,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SharpenSimpleParams {
                amount: op.amount,
                radius: op.radius,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
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
        matte: Option<&wgpu::Texture>,
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
            // The Matte gates the SEED (K-395) — see fx_glow.wgsl. It touches
            // the bright pass only: the halo then spreads from the pixels that
            // survived, which is the whole difference from dissolving the
            // finished glow.
            matte_on: f32::from(matte.is_some()),
            _pad0: 0.0,
            _pad: [0.0; 2],
        };
        // The bright pass wants ONE picture, so its `orig` slot is free and the
        // matte rides in it (the kernel's own comment says so). Passing it as
        // the matte binding instead would work equally well; this keeps the
        // shared blur kernel below the only user of binding 4.
        self.dispatch(
            ctx,
            &self.glow_bright,
            src,
            matte.unwrap_or(src),
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
                    matte_on: 0.0,
                    _pad0: 0.0,
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

/// One resolved Channel blur (docs/08 §3.45): the separable gaussian with a
/// radius per channel, already in raster pixels. Mirrors the arguments of
/// `lumit_core::fx::cpu::channel_blur`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelBlurOp {
    /// Red, green, blue and alpha kernel half-widths, raster pixels. A zero
    /// copies that channel through untouched.
    pub radii: [f32; 4],
    /// 0 = Transparent, 1 = Repeat (AE's "Repeat edge pixels" switch).
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChannelBlurParams {
    radius: [f32; 4],
    sigma: [f32; 4],
    dir: [f32; 2],
    mix_amt: f32,
    edge: u32,
    /// 1 = scale all four radii by the matte (K-395); both passes read it.
    matte_on: f32,
    _pad: [f32; 3],
}

/// One resolved Drop shadow (docs/08 §3.43). Mirrors
/// `lumit_core::fx::cpu::DropShadowParams` field-for-field; the direction's
/// sine and cosine are already spent into `offset` host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropShadowOp {
    /// Scene-linear RGB; the shadow's coverage supplies the rest.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// Where the shadow sits relative to the shape, raster pixels.
    pub offset: [f32; 2],
    /// The gaussian half-width the shape is softened by, raster pixels.
    pub softness_px: f32,
    /// Draw the shadow alone, without the layer that cast it.
    pub shadow_only: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// Spread's threshold-remap slope (K-706); 1.0 is no spread and no branch.
    pub spread_scale: f32,
    /// The layer's own shape knocks the shadow out before the composite (K-706).
    pub knockout: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DropShadowParams {
    colour: [f32; 4],
    offset: [f32; 2],
    opacity: f32,
    mix_amt: f32,
    shadow_only: u32,
    /// 1 = the matte scales the shadow's Opacity per pixel (K-428).
    matte_on: f32,
    /// Spread's threshold-remap slope (K-706); 1.0 takes no branch.
    spread_scale: f32,
    /// 1 = the layer's shape knocks the shadow out before the composite (K-706).
    knockout: u32,
}

impl FxEngine {
    /// Apply one Channel blur (docs/08 §3.45) to a linear working texture,
    /// returning a new texture of the same size. Two passes, exactly as
    /// [`Self::blur`] runs them — the difference is entirely inside the kernel,
    /// which carries four radii instead of one.
    pub fn channel_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &ChannelBlurOp,
    ) -> wgpu::Texture {
        let tmp = work_texture(ctx, w, h, "fx-chanblur-tmp");
        let out = work_texture(ctx, w, h, "fx-chanblur-out");
        // The four σ are taken once here, not per pixel (K-137's host-side
        // arithmetic rule), and floored exactly as the CPU reference floors
        // them so a zero radius cannot divide by zero on either path.
        let sigma: [f32; 4] = std::array::from_fn(|c| (op.radii[c] * 0.5).max(1e-3));
        for (pass_src, pass_orig, pass_dst, dir, mix) in [
            (src, src, &tmp, [1.0, 0.0], 1.0),
            (&tmp, src, &out, [0.0, 1.0], op.mix),
        ] {
            self.dispatch_matted(
                ctx,
                &self.channel_blur,
                pass_src,
                pass_orig,
                matte,
                pass_dst,
                w,
                h,
                bytemuck::bytes_of(&ChannelBlurParams {
                    radius: op.radii,
                    sigma,
                    dir,
                    mix_amt: mix,
                    edge: op.edge,
                    matte_on: f32::from(matte.is_some()),
                    _pad: [0.0; 3],
                }),
            );
        }
        out
    }

    /// Apply one Drop shadow (docs/08 §3.43) to a linear working texture,
    /// returning a new texture of the same size.
    ///
    /// Three passes: the shared §3.8 gaussian twice over the source, then one
    /// combine that reads the softened alpha at the offset and composites the
    /// shadow *underneath*. The blur is taken where the shape stands because a
    /// translation and a convolution commute — one gaussian instead of a
    /// gaussian plus a resample.
    pub fn drop_shadow(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &DropShadowOp,
    ) -> wgpu::Texture {
        // The blur takes no matte: it is where the SHAPE stands, and the claim
        // is on the shadow's Opacity where the shadow FALLS (K-428).
        let soft = self.blur(
            ctx,
            src,
            w,
            h,
            None,
            &BlurOp {
                radius_px: op.softness_px,
                // Transparent: a shape touching the frame border casts a shadow
                // that leaves the frame, and repeating the border pixel outward
                // would smear it into a fan.
                edge: 0,
                mix: 1.0,
            },
        );
        let out = work_texture(ctx, w, h, "fx-drop-shadow-out");
        self.dispatch_matted(
            ctx,
            &self.drop_shadow,
            src,
            &soft,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&DropShadowParams {
                colour: [op.colour[0], op.colour[1], op.colour[2], 1.0],
                offset: op.offset,
                opacity: op.opacity,
                mix_amt: op.mix,
                shadow_only: u32::from(op.shadow_only),
                matte_on: f32::from(matte.is_some()),
                spread_scale: op.spread_scale,
                knockout: u32::from(op.knockout),
            }),
        );
        out
    }
}

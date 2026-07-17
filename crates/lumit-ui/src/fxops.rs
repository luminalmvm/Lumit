//! The one place a resolved effect stack becomes GPU passes.
//!
//! In plain terms: `lumit_core::fx::resolve_stack` turns a layer's effect
//! list into plain numbers, and this module walks that list calling the
//! matching `FxEngine` kernel for each entry. The preview, the export
//! renderer, and adjustment-layer staging all call through here, so an
//! effect wired up once runs identically in all three — a new `Resolved`
//! variant only ever needs one new arm.

use lumit_core::fx::Resolved;
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

type Tex = egui_wgpu::wgpu::Texture;

/// Run `ops` over `tex` in order, returning the final texture (the input
/// unchanged when `ops` is empty). `w`/`h` are the texture's raster size.
pub fn run_ops(fx: &FxEngine, ctx: &GpuContext, tex: Tex, w: u32, h: u32, ops: &[Resolved]) -> Tex {
    let mut tex = tex;
    for op in ops {
        match op {
            Resolved::Blur {
                radius_px,
                edge,
                mix,
            } => {
                tex = fx.blur(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::BlurOp {
                        radius_px: *radius_px,
                        edge: *edge,
                        mix: *mix,
                    },
                );
            }
            Resolved::DirBlur {
                length_px,
                angle_deg,
                edge,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(1.0, *angle_deg);
                tex = fx.dir_blur(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::DirBlurOp {
                        dx,
                        dy,
                        length_px: *length_px,
                        taps: lumit_core::fx::cpu::dir_blur_taps(*length_px),
                        edge: *edge,
                        mix: *mix,
                    },
                );
            }
            Resolved::RadialBlur {
                centre_frac,
                amount_px,
                spin,
                edge,
                mix,
            } => {
                tex = fx.radial_blur(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::RadialBlurOp {
                        centre_frac: *centre_frac,
                        amount_px: *amount_px,
                        taps: lumit_core::fx::cpu::radial_blur_taps(*amount_px),
                        spin: *spin,
                        edge: *edge,
                        mix: *mix,
                    },
                );
            }
            Resolved::Sharpen {
                amount,
                radius_px,
                threshold,
                luma_only,
                mix,
            } => {
                tex = fx.sharpen(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::SharpenOp {
                        amount: *amount,
                        radius_px: *radius_px,
                        threshold: *threshold,
                        luma_only: *luma_only,
                        mix: *mix,
                    },
                );
            }
            Resolved::RgbSplit {
                amount_px,
                angle_deg,
                radial,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(*amount_px, *angle_deg);
                tex = fx.rgb_split(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::RgbSplitOp {
                        dx,
                        dy,
                        amount_px: *amount_px,
                        radial: *radial,
                        mix: *mix,
                    },
                );
            }
            Resolved::SpectralSplit {
                amount_px,
                angle_deg,
                radial,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(*amount_px, *angle_deg);
                tex = fx.spectral_split(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::SpectralSplitOp {
                        dx,
                        dy,
                        amount_px: *amount_px,
                        radial: *radial,
                        basis: lumit_core::fx::spectral_basis_vec4(),
                        mix: *mix,
                    },
                );
            }
            Resolved::Flash {
                strength,
                colour,
                mix,
            } => {
                tex = fx.flash(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::FlashOp {
                        strength: *strength,
                        colour: *colour,
                        mix: *mix,
                    },
                );
            }
            Resolved::ColourBalance {
                lift,
                gamma,
                gain,
                mix,
            } => {
                tex = fx.colour_balance(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::ColourBalanceOp {
                        lift: *lift,
                        gamma: *gamma,
                        gain: *gain,
                        mix: *mix,
                    },
                );
            }
            Resolved::Saturation { saturation, mix } => {
                tex = fx.saturation(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::SaturationOp {
                        saturation: *saturation,
                        mix: *mix,
                    },
                );
            }
            Resolved::Transform {
                anchor,
                position,
                scale,
                rotation_deg,
                opacity,
                mix,
            } => {
                let (m, off, opacity) = lumit_core::fx::transform_op(
                    *anchor,
                    *position,
                    *scale,
                    *rotation_deg,
                    *opacity,
                );
                tex = fx.transform(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::TransformOp {
                        m,
                        off,
                        opacity,
                        mix: *mix,
                    },
                );
            }
            Resolved::Glow {
                radius_px,
                threshold,
                knee,
                intensity,
                tint,
                mix,
            } => {
                tex = fx.glow(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::GlowOp {
                        radius_px: *radius_px,
                        threshold: *threshold,
                        knee: *knee,
                        intensity: *intensity,
                        tint: *tint,
                        mix: *mix,
                    },
                );
            }
            // Shake dispatches the Transform kernel (docs/08 §3.4: a
            // transform-domain effect): the shared affine turns the
            // resolved wobble into the same op the CPU reference builds,
            // so both paths consume bit-identical numbers.
            Resolved::Shake {
                offset_px,
                rotation_deg,
                zoom,
                amp_px,
                rotation_max_deg,
                zoom_min,
                auto_scale,
                mix,
            } => {
                let (anchor, position, scale, rot) = lumit_core::fx::shake_affine(
                    w,
                    h,
                    *offset_px,
                    *rotation_deg,
                    *zoom,
                    *amp_px,
                    *rotation_max_deg,
                    *zoom_min,
                    *auto_scale,
                );
                let (m, off, opacity) =
                    lumit_core::fx::transform_op(anchor, position, scale, rot, 1.0);
                tex = fx.transform(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::TransformOp {
                        m,
                        off,
                        opacity,
                        mix: *mix,
                    },
                );
            }
        }
    }
    tex
}

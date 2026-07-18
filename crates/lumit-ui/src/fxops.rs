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
/// `neighbours` are the layer's decoded neighbour frames keyed by offset
/// (empty unless the stack has a temporal effect); a temporal op like Echo
/// reads them, single-frame ops ignore them. `flow_field` is the layer's
/// dense motion field (per-pixel `(u, v)` at this raster size), present only
/// when the stack has a flow-consuming effect (Flow motion blur, or Datamosh
/// within Glitch — §3.12, K-104); a missing field makes that effect a
/// passthrough (degrade, never fault).
#[allow(clippy::too_many_arguments)]
pub fn run_ops(
    fx: &FxEngine,
    ctx: &GpuContext,
    tex: Tex,
    w: u32,
    h: u32,
    ops: &[Resolved],
    neighbours: &[(i32, Tex)],
    flow_field: Option<&Tex>,
) -> Tex {
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
            Resolved::ChromaticAberration { amount_px, mix } => {
                tex = fx.chromatic_aberration(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::ChromaticAberrationOp {
                        amount_px: *amount_px,
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
            Resolved::Vignette {
                amount,
                radius,
                softness,
                roundness,
                mix,
            } => {
                tex = fx.vignette(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::VignetteOp {
                        amount: *amount,
                        radius: *radius,
                        softness: *softness,
                        roundness: *roundness,
                        mix: *mix,
                    },
                );
            }
            Resolved::Exposure { factor, mix } => {
                tex = fx.exposure(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::ExposureOp {
                        factor: *factor,
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
            Resolved::Glitch {
                intensity,
                seed,
                tick,
                block_enabled,
                block_size_px,
                jitter_frac,
                amount_px,
                chan_px,
                slice_frac,
                scanline_enabled,
                period_px,
                darkness,
                roll_px,
                interlace,
                mix,
                datamosh_enabled,
            } => {
                tex = fx.glitch(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::GlitchOp {
                        intensity: *intensity,
                        seed: *seed,
                        tick: *tick,
                        block_enabled: *block_enabled,
                        block_size_px: *block_size_px,
                        jitter_frac: *jitter_frac,
                        amount_px: *amount_px,
                        chan_px: *chan_px,
                        slice_frac: *slice_frac,
                        scanline_enabled: *scanline_enabled,
                        period_px: *period_px,
                        darkness: *darkness,
                        roll_px: *roll_px,
                        interlace: *interlace,
                        mix: *mix,
                    },
                );
                // Datamosh (§3.12, K-104) reads the layer's -1 neighbour and
                // its current→previous flow field, exactly as Motion blur
                // reads its own +1-neighbour flow field. Either missing (a
                // non-footage layer, or a dropped decode) skips the section
                // — a passthrough, never a fault.
                if *datamosh_enabled {
                    if let (Some(flow), Some((_, prev))) =
                        (flow_field, neighbours.iter().find(|(o, _)| *o == -1))
                    {
                        tex = fx.datamosh(
                            ctx,
                            &tex,
                            prev,
                            flow,
                            w,
                            h,
                            &lumit_gpu::fx::DatamoshOp {
                                intensity: *intensity,
                            },
                        );
                    }
                }
            }
            Resolved::Echo { weights, mode, mix } => {
                // Echo reads the layer's neighbour frames (offsets -1..-8);
                // the render decoded exactly the ones the window needs.
                let by_offset: Vec<(i32, &Tex)> = neighbours.iter().map(|(o, t)| (*o, t)).collect();
                tex = fx.echo(
                    ctx,
                    &tex,
                    &by_offset,
                    w,
                    h,
                    &lumit_gpu::fx::EchoOp {
                        weights: *weights,
                        mode: *mode,
                        mix: *mix,
                    },
                );
            }
            Resolved::MotionBlur {
                shutter_frac,
                samples,
                mix,
            } => {
                // Flow motion blur reads the layer's dense motion field, which
                // the decode worker computed from the current + next source
                // frames. With no field (a plain layer, or a decode that
                // dropped the neighbour) it is a passthrough — never a fault.
                if let Some(flow) = flow_field {
                    tex = fx.motion_blur(
                        ctx,
                        &tex,
                        flow,
                        w,
                        h,
                        &lumit_gpu::fx::MotionBlurOp {
                            shutter_frac: *shutter_frac,
                            samples: *samples,
                            mix: *mix,
                        },
                    );
                }
            }
        }
    }
    tex
}

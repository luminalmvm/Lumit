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

/// A parsed-and-uploaded `.cube` LUT ready to bind (docs/08 §3.11,
/// docs/impl/lut.md): the 3D cube texture plus its per-axis size `N`. Held by
/// the caller's path-keyed cache and cloned into a `run_ops` `luts` slot;
/// `wgpu::Texture` is an `Arc` handle, so the clone is cheap and shares the one
/// upload.
#[derive(Clone)]
pub struct LoadedLut {
    pub texture: Tex,
    pub size: u32,
}

/// Render one referenced layer alone into the depth input a depth-of-field
/// effect samples (docs/impl/layer-input.md §2). The **one** helper the preview
/// (`GpuViewer`) and export (`Renderer`) paths both call, so the depth pass is
/// byte-identical in the viewport and the file (K-031) — exactly as
/// `Compositor::motion_blur_average` and the matte "render alone" composite are
/// shared.
///
/// The effect stack runs on the consuming layer's own working raster `(w, h)`
/// (the layer's decoded size, which shrinks under reduced-resolution preview),
/// and the DoF kernel reads the depth at that same pixel grid — so the depth
/// input must be exactly `(w, h)` and aligned with the layer texture. v1 model
/// (documented in docs/08 §3.22): the referenced layer's **source** is
/// resampled to fill `(w, h)` — the depth pass is expected to share the
/// footage's framing (the standard "footage + matching depth pass" workflow),
/// so it is stretched to the working raster and its own transform is not
/// applied. A placement-aware depth is a recorded follow-up. `linear` is the
/// referenced layer's source in the working linear format, sized
/// `(src_w, src_h)`; each caller uploads/linearises it its own way, as the
/// matte path does, and this helper owns only the resample so it never drifts.
pub fn render_layer_input(
    compositor: &lumit_gpu::Compositor,
    ctx: &GpuContext,
    w: u32,
    h: u32,
    linear: &Tex,
    src_w: f32,
    src_h: f32,
) -> Tex {
    compositor.composite_with_camera(
        ctx,
        w,
        h,
        [0.0, 0.0, 0.0, 0.0],
        &[lumit_gpu::CompositeLayer {
            texture: linear,
            size: (src_w.max(1.0), src_h.max(1.0)),
            position: (0.0, 0.0),
            anchor: (0.0, 0.0),
            // Stretch the source to fill the whole working raster.
            scale: (
                w as f32 / src_w.max(1.0) * 100.0,
                h as f32 / src_h.max(1.0) * 100.0,
            ),
            rotation_deg: 0.0,
            // Full opacity: a depth pass is read as a scalar, never dimmed.
            opacity: 100.0,
            matte: None,
            blend: lumit_gpu::Blend::Normal,
            z: 0.0,
            rotation_x_deg: 0.0,
            rotation_y_deg: 0.0,
            three_d: false,
            layer_mask: None,
            pre: None,
        }],
        None,
    )
}

/// Run `ops` over `tex` in order, returning the final texture (the input
/// unchanged when `ops` is empty). `w`/`h` are the texture's raster size.
/// `neighbours` are the layer's decoded neighbour frames keyed by offset
/// (empty unless the stack has a temporal effect); a temporal op like Echo
/// reads them, single-frame ops ignore them. `flow_field` is the layer's
/// dense motion field (per-pixel `(u, v)` at this raster size), present only
/// when the stack has a flow-consuming effect (Flow motion blur, or Datamosh
/// — §3.12, K-107); a missing field makes that effect a passthrough
/// (degrade, never fault). `luts` is the parallel LUT list (docs/08 §3.11): the
/// k-th `Resolved::Lut` op binds `luts[k]` — a `None` slot (unset, missing, 1D
/// or unreadable file) is a passthrough, exactly like a missing flow field.
/// `layer_inputs` is the parallel depth-input list (docs/08 §3.22, docs/impl/
/// layer-input.md): the k-th `Resolved::Dof` op binds `layer_inputs[k]` — the
/// referenced layer rendered alone at comp size, or `None` (unset, missing or
/// cyclic) for a passthrough, exactly like a missing LUT.
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
    luts: &[Option<LoadedLut>],
    layer_inputs: &[Option<Tex>],
) -> Tex {
    let mut tex = tex;
    // The k-th Resolved::Lut op consumes the k-th `luts` slot (the whole
    // threading contract — see resolve_stack's `lut` arm and CompLayerDraw's
    // lut_files); a slot is present only when its `.cube` file loaded. The
    // k-th Resolved::Dof op consumes the k-th `layer_inputs` slot the same way
    // (its depth-layer render).
    let mut lut_i = 0usize;
    let mut dof_i = 0usize;
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
            Resolved::MatteKey {
                key,
                tol,
                soft,
                spill,
                mix,
            } => {
                tex = fx.matte_key(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::MatteKeyOp {
                        key: *key,
                        tol: *tol,
                        soft: *soft,
                        spill: *spill,
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
            Resolved::HueShift { m, mix } => {
                tex = fx.hue_shift(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::HueShiftOp { m: *m, mix: *mix },
                );
            }
            Resolved::Contrast { k, mix } => {
                tex = fx.contrast(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::ContrastOp { k: *k, mix: *mix },
                );
            }
            Resolved::Gamma { gamma, mix } => {
                tex = fx.gamma(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::GammaOp {
                        gamma: *gamma,
                        mix: *mix,
                    },
                );
            }
            Resolved::Temperature {
                gain_r,
                gain_b,
                mix,
            } => {
                tex = fx.temperature(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::TemperatureOp {
                        gain_r: *gain_r,
                        gain_b: *gain_b,
                        mix: *mix,
                    },
                );
            }
            Resolved::Invert { mix } => {
                tex = fx.invert(ctx, &tex, w, h, &lumit_gpu::fx::InvertOp { mix: *mix });
            }
            Resolved::Tint { black, white, mix } => {
                tex = fx.tint(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::TintOp {
                        black: *black,
                        white: *white,
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
            Resolved::BlockGlitch {
                intensity,
                seed,
                tick,
                block_size_px,
                jitter_frac,
                amount_px,
                chan_px,
                slice_frac,
                mix,
            } => {
                tex = fx.block_glitch(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::BlockGlitchOp {
                        intensity: *intensity,
                        seed: *seed,
                        tick: *tick,
                        block_size_px: *block_size_px,
                        jitter_frac: *jitter_frac,
                        amount_px: *amount_px,
                        chan_px: *chan_px,
                        slice_frac: *slice_frac,
                        mix: *mix,
                    },
                );
            }
            Resolved::Scanlines {
                intensity,
                period_px,
                darkness,
                roll_px,
                interlace,
                mix,
            } => {
                tex = fx.scanlines(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::ScanlinesOp {
                        intensity: *intensity,
                        period_px: *period_px,
                        darkness: *darkness,
                        roll_px: *roll_px,
                        interlace: *interlace,
                        mix: *mix,
                    },
                );
            }
            Resolved::Datamosh { intensity, mix } => {
                // Datamosh (§3.12, K-107) reads the layer's -1 neighbour and
                // its current→previous flow field, exactly as Motion blur
                // reads its own +1-neighbour flow field. Either missing (a
                // non-footage layer, or a dropped decode) is a passthrough,
                // never a fault. The existing GPU/CPU maths take a single
                // blend fraction; Mix folds into Intensity here rather than
                // adding a second uniform, since mixing the same two inputs
                // twice collapses to one mix by the product.
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
                            intensity: *intensity * *mix,
                        },
                    );
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
            Resolved::Lut { mix } => {
                // The k-th Lut op binds the k-th `luts` slot (§3.11). A None
                // slot — an unset, missing, 1D or unreadable file — is a
                // passthrough (the labelled no-op rule; never a fault). The
                // parsed cube travels beside the op, exactly as Motion blur's
                // flow field does, since a path is not Copy in `Resolved`.
                let loaded = luts.get(lut_i).and_then(|o| o.as_ref());
                lut_i += 1;
                if let Some(l) = loaded {
                    tex = fx.lut(ctx, &tex, w, h, &l.texture, l.size, *mix);
                }
            }
            Resolved::Dof {
                focus,
                range,
                near_aperture,
                far_aperture,
                depth_invert,
                display,
                mix,
            } => {
                // The k-th Dof op binds the k-th `layer_inputs` slot (docs/08
                // §3.22, docs/impl/layer-input.md): the referenced layer
                // rendered alone at comp size, its red channel read as depth.
                // A None slot — unset, missing or cyclic — is a passthrough
                // (the labelled no-op rule; never a fault). The depth is a
                // whole texture, so it travels beside the op, exactly as the
                // LUT cube does, since it is not Copy in `Resolved`.
                let depth = layer_inputs.get(dof_i).and_then(|o| o.as_ref());
                dof_i += 1;
                if let Some(depth) = depth {
                    tex = fx.dof(
                        ctx,
                        &tex,
                        w,
                        h,
                        depth,
                        *focus,
                        *range,
                        *near_aperture,
                        *far_aperture,
                        *depth_invert,
                        *display,
                        *mix,
                    );
                }
            }
        }
    }
    tex
}

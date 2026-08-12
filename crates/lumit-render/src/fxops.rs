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

type Tex = wgpu::Texture;

/// A parsed-and-uploaded `.cube` LUT ready to bind (docs/08 §3.11,
/// docs/impl/lut.md): the 3D cube texture, its per-axis size `N`, and the input
/// domain the file declared. Held by [`LutCache`] and cloned into a `run_ops`
/// `luts` slot; `wgpu::Texture` is an `Arc` handle, so the clone is cheap and
/// shares the one upload.
#[derive(Clone)]
pub struct LoadedLut {
    pub texture: Tex,
    pub size: u32,
    /// `DOMAIN_MIN` / `DOMAIN_MAX` from the file (default `0..1`), carried to
    /// the kernel so the GPU remaps exactly as the CPU oracle does (K-271).
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

/// How many distinct `.cube` files stay uploaded at once. A comp references a
/// handful at most (docs/impl/lut.md §4); past that the least recently used one
/// is dropped, which releases its GPU texture. Small on purpose: an unbounded
/// cache turned every path a session ever touched into a permanent upload.
const LUT_CACHE_MAX: usize = 8;

/// The parsed-and-uploaded `.cube` files a render can bind, keyed by **path and
/// last-modified time** and bounded to [`LUT_CACHE_MAX`] entries, most recently
/// used first (docs/impl/lut.md §4).
///
/// **Why the mtime is part of the key.** Grading is iterative: export a cube
/// from the grading tool, look at it in Lumit, adjust, export again over the
/// same path. Keyed by path alone, the second export never appeared — Lumit
/// kept showing the first grade until the application was restarted, with
/// nothing on screen to say so (K-271). A file whose mtime has moved is a
/// different entry, so it is parsed and uploaded again.
///
/// A path the filesystem will not stat (it vanished, or the platform has no
/// mtime for it) keys as `None`, which still matches itself: such a file is
/// cached by path exactly as before rather than being re-read every frame.
#[derive(Default)]
pub struct LutCache {
    /// Most recently used first. A `Vec` rather than a map because the bound is
    /// eight: a linear scan of eight strings is faster than hashing one, and
    /// the ordering the LRU needs is the `Vec`'s own.
    entries: Vec<(String, Option<std::time::SystemTime>, LoadedLut)>,
}

impl LutCache {
    /// The cube at `path`, parsed and uploaded on a miss. `None` for anything
    /// that is not a usable 3D cube — an unreadable file, a parse error, or a
    /// 1D LUT — which leaves the effect a labelled passthrough rather than a
    /// render failure (docs/08 §3.11, the never-crash rule).
    pub fn get_or_load(&mut self, ctx: &GpuContext, path: &str) -> Option<LoadedLut> {
        let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        if let Some(i) = self
            .entries
            .iter()
            .position(|(p, t, _)| p == path && *t == mtime)
        {
            // Touch: this entry is now the most recently used.
            let hit = self.entries.remove(i);
            let lut = hit.2.clone();
            self.entries.insert(0, hit);
            return Some(lut);
        }
        // A stale entry for this path (the file was edited) is replaced, not
        // kept alongside: the old grade is exactly what nobody wants back.
        self.entries.retain(|(p, _, _)| p != path);
        let text = std::fs::read_to_string(path).ok()?;
        let lumit_core::lut::Lut::Cube3d(l) = lumit_core::lut::parse_cube(&text).ok()? else {
            return None;
        };
        let loaded = LoadedLut {
            texture: lumit_gpu::fx::upload_lut_3d(ctx, l.size as u32, &l.data),
            size: l.size as u32,
            domain_min: l.domain_min,
            domain_max: l.domain_max,
        };
        self.entries
            .insert(0, (path.to_owned(), mtime, loaded.clone()));
        self.entries.truncate(LUT_CACHE_MAX);
        Some(loaded)
    }

    /// How many cubes are held — the bound's test hook, and a number worth
    /// having when a profile asks where the video memory went.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One layer-input slot as [`run_ops`] receives it — the realised twin of
/// [`crate::draw::LayerInputDraw`] (docs/impl/layer-input.md, K-288).
pub enum LayerInput {
    /// Nothing to read: the effect degrades to its labelled no-op.
    Absent,
    /// The effect's **own input** at its point in the stack. There is no
    /// texture to carry here because only `run_ops` knows it — it is the
    /// picture the chain is holding when the op comes round, which on an
    /// adjustment layer is the composite of everything below.
    ThisLayer,
    /// Another layer, already rendered alone at this raster.
    Texture(Tex),
}

impl LayerInput {
    /// The texture this slot names, given the texture the chain currently
    /// holds. `None` for [`LayerInput::Absent`] — the passthrough.
    #[must_use]
    pub fn texture<'a>(&'a self, current: &'a Tex) -> Option<&'a Tex> {
        match self {
            LayerInput::Absent => None,
            LayerInput::ThisLayer => Some(current),
            LayerInput::Texture(t) => Some(t),
        }
    }
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
/// referenced layer rendered alone at comp size, [`LayerInput::ThisLayer`]
/// for the effect's own input (K-288), or [`LayerInput::Absent`] (unset,
/// missing or cyclic) for a passthrough, exactly like a missing LUT.
/// `flare_mattes` is the parallel Lens flare Matte-source list (docs/08
/// §3.27, K-257), and `flare_lens` the parallel custom-prescription list
/// (K-264, `lens_file` as content hash + text; None = use the picked
/// library lens): the k-th `Resolved::LensFlare` op binds `flare_mattes[k]`
/// — the referenced matte layer rendered alone at this raster, this effect's
/// own input, or absent (unset, dangling, or not in Matte mode) which
/// detects no sources, the LUT/DoF passthrough convention.
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
    layer_inputs: &[LayerInput],
    flare_mattes: &[LayerInput],
    flare_lens: &[Option<(u64, String)>],
    mut timings: Option<&mut Vec<f32>>,
) -> Tex {
    let mut tex = tex;
    // The k-th Resolved::Lut op consumes the k-th `luts` slot (the whole
    // threading contract — see resolve_stack's `lut` arm and CompLayerDraw's
    // lut_files); a slot is present only when its `.cube` file loaded. The k-th
    // layer-input-consuming op consumes the k-th
    // `layer_inputs` slot the same way. Both share one counter because
    // `build.rs`'s `layer_inputs_for` enumerates them with one predicate, in one
    // order; two counters would let the two sides drift apart silently.
    let mut lut_i = 0usize;
    let mut dof_i = 0usize;
    let mut flare_i = 0usize;
    for op in ops {
        // Only a *profiled* render reads a clock here, and it reads it either
        // side of a fence — see crate::profile on why an unfenced span would
        // time the paperwork rather than the work.
        let started = timings.as_ref().map(|_| std::time::Instant::now());
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
            Resolved::SharpenSimple {
                amount,
                radius,
                mix,
            } => {
                tex = fx.sharpen_simple(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::SharpenSimpleOp {
                        amount: *amount,
                        radius: *radius,
                        mix: *mix,
                    },
                );
            }
            Resolved::RgbSplit {
                amount_px,
                angle_deg,
                scale,
                tints,
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
                        scale: *scale,
                        tints: *tints,
                        mix: *mix,
                    },
                );
            }
            Resolved::SpectralSplit {
                amount_px,
                angle_deg,
                radial,
                samples,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(*amount_px, *angle_deg);
                let (basis, count) = lumit_core::fx::spectral_basis_uniform(*samples, *tints);
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
                        basis,
                        count,
                        mix: *mix,
                    },
                );
            }
            Resolved::ChromaticAberration {
                amount_px,
                tints,
                mix,
            } => {
                tex = fx.chromatic_aberration(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::ChromaticAberrationOp {
                        amount_px: *amount_px,
                        tints: *tints,
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
            Resolved::Vibrancy { amount, mix } => {
                tex = fx.vibrancy(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::VibrancyOp {
                        amount: *amount,
                        mix: *mix,
                    },
                );
            }
            Resolved::MatteKey(p) => {
                tex = fx.matte_key(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::MatteKeyOp {
                        view: p.view,
                        key: p.key,
                        gain: p.gain,
                        balance: p.balance,
                        despill_bias: p.despill_bias,
                        alpha_bias: p.alpha_bias,
                        spill: p.spill,
                        clip_black: p.clip_black,
                        clip_white: p.clip_white,
                        clip_rollback: p.clip_rollback,
                        replace_method: p.replace_method,
                        replace_colour: p.replace_colour,
                        mix: p.mix,
                    },
                );
            }
            Resolved::Vignette {
                amount,
                radius,
                softness,
                roundness,
                ramp,
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
                        ramp: *ramp,
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
                        // The Transform effect has no Edges control: a
                        // transparent border, its long-standing behaviour.
                        edge: 0,
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
            // so both paths consume bit-identical numbers. With its own
            // motion blur on (T18/K-165) it instead builds one affine per
            // sub-frame and dispatches the averaging kernel, the same
            // sub-frames `cpu::transform_average` averages.
            Resolved::Shake {
                offset_px,
                rotation_deg,
                zoom,
                edge,
                mix,
                mb,
            } => match mb {
                Some(samples) => {
                    let mut taps = [lumit_gpu::fx::ShakeMbTap {
                        m: [1.0, 0.0, 0.0, 1.0],
                        off: [0.0, 0.0],
                    }; lumit_gpu::fx::SHAKE_MB_SAMPLES];
                    for (t, s) in taps.iter_mut().zip(samples.iter()) {
                        let (anchor, position, scale, rot) =
                            lumit_core::fx::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                        let (m, off, _opacity) =
                            lumit_core::fx::transform_op(anchor, position, scale, rot, 1.0);
                        *t = lumit_gpu::fx::ShakeMbTap { m, off };
                    }
                    tex = fx.shake_mb(
                        ctx,
                        &tex,
                        w,
                        h,
                        &lumit_gpu::fx::ShakeMbOp {
                            taps,
                            count: samples.len() as u32,
                            // Shake's own Edges control governs the revealed border.
                            edge: *edge,
                            mix: *mix,
                        },
                    );
                }
                None => {
                    let (anchor, position, scale, rot) =
                        lumit_core::fx::shake_affine(w, h, *offset_px, *rotation_deg, *zoom);
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
                            // Shake's own Edges control governs the revealed border.
                            edge: *edge,
                        },
                    );
                }
            },
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
                        roll_px: *roll_px,
                        interlace: *interlace,
                        mix: *mix,
                    },
                );
            }
            Resolved::Datamosh {
                intensity,
                displacement,
                bloom,
                steps,
                mix,
            } => {
                // Datamosh (§3.12, K-107; flow-driven melt K-164) reads the
                // layer's -1 neighbour and its current→previous flow field,
                // exactly as Motion blur reads its own +1-neighbour flow field.
                // Either missing (a non-footage layer, or a dropped decode) is a
                // passthrough, never a fault. The blend maths take a single
                // fraction; Mix folds into Intensity here rather than adding a
                // second uniform, since mixing the same two inputs twice
                // collapses to one mix by the product.
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
                            displacement: *displacement,
                            bloom: *bloom,
                            steps: *steps,
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
                view,
            } => {
                // Fast motion blur reads the layer's dense motion field (with a
                // confidence channel, FX-19), which the decode worker computed
                // from the current + next source frames. With no field (a plain
                // layer, or a decode that dropped the neighbour) it is a
                // passthrough — never a fault.
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
                            view: view.code(),
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
                    tex = fx.lut(
                        ctx,
                        &tex,
                        w,
                        h,
                        &l.texture,
                        l.size,
                        *mix,
                        l.domain_min,
                        l.domain_max,
                    );
                }
            }
            Resolved::SpriteFlare(p) => {
                tex = fx.sprite_flare(
                    ctx,
                    &tex,
                    w,
                    h,
                    &lumit_gpu::fx::SpriteFlareOp {
                        light: p.light,
                        intensity: p.intensity,
                        tint: p.tint,
                        glow_size: p.glow_size,
                        glow_intensity: p.glow_intensity,
                        ghosts: p.ghosts,
                        ghost_spacing: p.ghost_spacing,
                        ghost_size: p.ghost_size,
                        ghost_intensity: p.ghost_intensity,
                        streak_length: p.streak_length,
                        streak_intensity: p.streak_intensity,
                        streak_angle_deg: p.streak_angle_deg,
                        mix: p.mix,
                    },
                );
            }
            Resolved::LightWrap {
                width_px,
                intensity,
                mix,
            } => {
                // Shares the layer-input counter with Dof (K-358): both are
                // enumerated by `build.rs`'s one `layer_input_param`
                // predicate, so one counter keeps the slots and the ops in
                // step. An absent Background — unset, missing or cyclic — is
                // the passthrough, the labelled no-op every layer-input
                // effect follows.
                let background = layer_inputs.get(dof_i).and_then(|o| o.texture(&tex));
                dof_i += 1;
                if let Some(background) = background {
                    if *width_px > 0.0 && *intensity > 0.0 && *mix > 0.0 {
                        tex = fx.light_wrap(
                            ctx,
                            &tex,
                            w,
                            h,
                            background,
                            &lumit_gpu::fx::LightWrapOp {
                                width_px: *width_px,
                                intensity: *intensity,
                                mix: *mix,
                            },
                        );
                    }
                }
            }
            Resolved::Dof {
                focus,
                range,
                near_aperture,
                far_aperture,
                depth_invert,
                blade_normals,
                blade_count,
                apothem2,
                roundness,
                rim,
                aspect_scale,
                threshold,
                bokeh_power,
                repeat_edge,
                depth_bound,
                depth_channel,
                use_focus_point,
                focus_point,
                gamma,
                remove_edge_leak,
                detect_edge_threshold,
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
                let depth = layer_inputs.get(dof_i).and_then(|o| o.texture(&tex));
                dof_i += 1;
                if let Some(depth) = depth {
                    tex = fx.dof(
                        ctx,
                        &tex,
                        w,
                        h,
                        depth,
                        &lumit_gpu::fx::DofOp {
                            focus: *focus,
                            range: *range,
                            near_aperture: *near_aperture,
                            far_aperture: *far_aperture,
                            blade_normals: *blade_normals,
                            blade_count: *blade_count,
                            apothem2: *apothem2,
                            roundness: *roundness,
                            rim: *rim,
                            aspect_scale: *aspect_scale,
                            threshold: *threshold,
                            bokeh_power: *bokeh_power,
                            repeat_edge: *repeat_edge,
                            depth_bound: *depth_bound,
                            depth_channel: *depth_channel,
                            depth_invert: *depth_invert,
                            use_focus_point: *use_focus_point,
                            focus_point: *focus_point,
                            gamma: *gamma,
                            remove_edge_leak: *remove_edge_leak,
                            detect_edge_threshold: *detect_edge_threshold,
                            display: *display,
                            mix: *mix,
                        },
                    );
                }
            }
            Resolved::LensFlare(p) => {
                // Lens flare (docs/08 §3.27, K-256/K-257). Every frame-time
                // number the GPU needs is derived here through the one
                // lumit-core module that owns the formulas (K-031: the CPU
                // reference and the kernels read identical values); the heavy
                // bake is a lazy closure the GPU side calls only when its
                // parameter-hash cache misses. The k-th LensFlare op binds
                // the k-th `flare_mattes` slot (its Matte source).
                use lumit_core::fx::lens_flare as lf;
                let matte = flare_mattes.get(flare_i).and_then(|o| o.texture(&tex));
                let custom = flare_lens.get(flare_i).and_then(|o| o.as_ref());
                flare_i += 1;
                let (tier_base, tier_lambda, flare_div) = lf::quality_ladder(p.quality);
                // The Detail dial scales the tier's base and wavelength
                // count (K-265) — through the shared helpers, so this
                // equals the CPU reference.
                let grid = lf::detail_base(tier_base, p.detail);
                let lambda_count = lf::detail_lambda(tier_lambda, p.detail);
                let energy = p.ghost_intensity;
                // The traced bands with their eight radiometric sub-samples
                // (K-364), Ghost intensity folded into every sub-weight —
                // the bake's auto-exposure gain joins it GPU-side.
                let bands: Vec<lumit_gpu::fx::FlareBand> =
                    lf::spectral_bands(lambda_count, p.dispersion)
                        .into_iter()
                        .map(|b| lumit_gpu::fx::FlareBand {
                            traced_nm: b.traced_nm,
                            sub_idx: b.sub_idx,
                            sub_rgb: b
                                .sub_rgb
                                .map(|c| [c[0] * energy, c[1] * energy, c[2] * energy]),
                        })
                        .collect();
                let op = lumit_gpu::fx::LensFlareOp {
                    // Raster pixels → fraction here, where the raster is
                    // known (K-260: the parameter is px@comp).
                    light_frac: [p.light[0] / w.max(1) as f32, p.light[1] / h.max(1) as f32],
                    // Manual mode's lights, area-sampled (K-355). One entry
                    // for a point source — which is what a zero Source size
                    // gives — and a grid across the emitting area otherwise,
                    // each carrying its share of the flux.
                    manual_lights: lf::expand_area_lights(
                        &lf::manual_light(p, w, h),
                        lf::AREA_SAMPLES_MAX,
                    )
                    .iter()
                    .map(|l| [l.pos[0], l.pos[1], l.rgb[0], l.rgb[1], l.rgb[2]])
                    .collect(),
                    intensity: p.intensity,
                    bands,
                    max_ghosts: p.max_ghosts,
                    coating: p.coating,
                    focus_m: p.focus_m,
                    fstop: p.fstop,
                    blades: p.blades,
                    aperture_rotation_deg: p.aperture_rotation_deg,
                    roundness: p.roundness,
                    aperture_softness: p.aperture_softness,
                    ghost_softness: p.ghost_softness,
                    grid,
                    flare_div,
                    screen_transform: lf::screen_transform(w),
                    starburst_intensity: p.starburst_intensity,
                    scale: p.scale,
                    anamorphic: p.anamorphic,
                    source: p.source,
                    threshold: p.threshold,
                    threshold_softness: p.threshold_softness,
                    light_tint: p.light_tint,
                    use_source_colour: p.use_source_colour,
                    blend: p.blend,
                    mix: p.mix,
                    bake_key: lf::bake_key_with(p, custom.map(|(h, _)| *h)),
                };
                let params = *p;
                let custom_text = custom.map(|(_, text)| text.clone());
                // Manual mode's frame-time grid probe (K-267): the GPU
                // hands back its cached bake's tables and this closure runs
                // the one lumit-core probe both twins share, at the frame's
                // actual light direction.
                let light_frac = op.light_frac;
                let aspect = h as f32 / w.max(1) as f32;
                let probe = move |pb: &lumit_gpu::fx::FlareProbeBake| {
                    let needs = lf::frame_grid_needs_from_rows(
                        pb.surfaces,
                        pb.ghosts,
                        pb.sensor_z_mm,
                        pb.focal_mm,
                        pb.pupil_mm,
                        pb.start_z_mm,
                        pb.pair_count,
                        lf::light_direction(light_frac, aspect, pb.focal_mm),
                        params.coating,
                        lf::fstop_scale(pb.native_fstop, params.fstop),
                        lf::focus_shift_mm(params.focus_m, pb.focal_mm),
                    );
                    lf::plan_frame_grids(grid, pb.spreads, &needs)
                };
                tex = fx.lens_flare(
                    ctx,
                    &tex,
                    w,
                    h,
                    &op,
                    matte,
                    // The bake as something the bake thread can own and run
                    // (K-350): one small `Arc` a flare a frame, beside a pass
                    // that traces hundreds of thousands of rays. Whether it is
                    // actually run beside the frame or inside it is the
                    // engine's policy, not this call's — see
                    // `FxEngine::set_deferred_flare_bakes`.
                    &(std::sync::Arc::new(move || {
                        let b = lf::bake_with(&params, custom_text.as_deref());
                        lumit_gpu::fx::FlareBakeData {
                            surfaces: b
                                .surfaces
                                .iter()
                                .map(|s| {
                                    [
                                        s.radius_mm,
                                        s.z_mm,
                                        s.semi_ap_mm,
                                        s.cauchy_a,
                                        s.cauchy_b,
                                        s.coating_layers,
                                        s.is_stop,
                                        0.0,
                                    ]
                                })
                                .collect(),
                            ghosts: b.pairs.clone(),
                            spreads: b.spreads.clone(),
                            sensor_z_mm: b.sensor_z_mm,
                            focal_mm: b.focal_mm,
                            native_fstop: b.native_fstop,
                            pupil_mm: b.pupil_mm,
                            start_z_mm: b.start_z_mm,
                            energy_gain: b.energy_gain,
                            reflectance: b.reflectance.clone(),
                            starburst: b.starburst,
                            sb_res: lf::STARBURST_RES,
                            sb_fields: lf::STARBURST_FIELDS as u32,
                        }
                    }) as lumit_gpu::fx::FlareBake),
                    &probe,
                );
            }
        }

        if let (Some(started), Some(into)) = (started, timings.as_mut()) {
            // Same reason as the per-layer fence in `realise`: a frame's
            // commands are batched, and timing a queue that has not been
            // handed over would measure nothing. A measured frame gives the
            // batching up, effect by effect.
            ctx.flush();
            ctx.device.poll(wgpu::Maintain::Wait);
            into.push(started.elapsed().as_secs_f32() * 1000.0);
        }
    }
    tex
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A tiny valid `.cube` of `size`, with an optional declared domain.
    fn cube_text(size: usize, domain: Option<([f32; 3], [f32; 3])>) -> String {
        let mut s = format!("LUT_3D_SIZE {size}\n");
        if let Some((lo, hi)) = domain {
            s += &format!("DOMAIN_MIN {} {} {}\n", lo[0], lo[1], lo[2]);
            s += &format!("DOMAIN_MAX {} {} {}\n", hi[0], hi[1], hi[2]);
        }
        let maxf = (size - 1) as f32;
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    s += &format!(
                        "{} {} {}\n",
                        r as f32 / maxf,
                        g as f32 / maxf,
                        b as f32 / maxf
                    );
                }
            }
        }
        s
    }

    /// **A `.cube` edited on disk must be re-read** (K-271, docs/impl/lut.md §4).
    ///
    /// The regression: the cache keyed by path alone, so exporting a new grade
    /// over the same filename — the whole loop of grading — kept showing the
    /// first one until the application was restarted, with nothing on screen to
    /// say the file on disk and the picture had parted company.
    #[test]
    fn an_edited_cube_is_read_again() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("grade.cube");
        std::fs::write(&path, cube_text(2, None)).expect("written");
        let name = path.to_string_lossy().to_string();

        let mut cache = LutCache::default();
        let first = cache.get_or_load(&ctx, &name).expect("loaded");
        assert_eq!(first.size, 2);
        assert_eq!(cache.len(), 1);

        // Re-asking without touching the file is a cache hit: still one entry,
        // still the cube that was parsed the first time.
        let again = cache.get_or_load(&ctx, &name).expect("loaded");
        assert_eq!(cache.len(), 1, "an untouched file is not read twice");
        assert_eq!(again.size, first.size);

        // Now edit it. The sleep is the point of the test, not laziness: two
        // writes inside one filesystem timestamp tick are indistinguishable,
        // and the cache is keyed on that tick.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, cube_text(3, None)).expect("re-written");
        let edited = cache.get_or_load(&ctx, &name).expect("loaded");
        assert_eq!(edited.size, 3, "the new grade is the one that renders");
        assert_eq!(
            cache.len(),
            1,
            "and the stale entry is replaced, not kept beside it"
        );
    }

    /// The declared domain travels with the cube, so the kernel can remap
    /// through it exactly as the CPU oracle does (K-271). A default-domain file
    /// still reads 0..1.
    #[test]
    fn the_declared_domain_travels_with_the_cube() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut cache = LutCache::default();

        let plain = dir.path().join("plain.cube");
        std::fs::write(&plain, cube_text(2, None)).expect("written");
        let loaded = cache
            .get_or_load(&ctx, &plain.to_string_lossy())
            .expect("loaded");
        assert_eq!(loaded.domain_min, [0.0; 3]);
        assert_eq!(loaded.domain_max, [1.0; 3]);

        let log = dir.path().join("log.cube");
        std::fs::write(
            &log,
            cube_text(2, Some(([-0.25, 0.0, 0.1], [1.5, 0.75, 1.0]))),
        )
        .expect("written");
        let loaded = cache
            .get_or_load(&ctx, &log.to_string_lossy())
            .expect("loaded");
        assert_eq!(loaded.domain_min, [-0.25, 0.0, 0.1]);
        assert_eq!(loaded.domain_max, [1.5, 0.75, 1.0]);
    }

    /// The cache is bounded and evicts the least recently used, so a long
    /// session that touches many `.cube` files does not accumulate uploads
    /// (docs/impl/lut.md §4). Re-asking for an old file *keeps it alive*, which
    /// is what "least recently used" has to mean.
    #[test]
    fn the_cache_is_bounded_and_keeps_what_is_used() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut cache = LutCache::default();
        let path = |i: usize| dir.path().join(format!("cube{i}.cube"));
        for i in 0..=LUT_CACHE_MAX {
            std::fs::write(path(i), cube_text(2, None)).expect("written");
        }

        // Fill it exactly, then keep the first one fresh by asking for it again.
        for i in 0..LUT_CACHE_MAX {
            cache
                .get_or_load(&ctx, &path(i).to_string_lossy())
                .expect("loaded");
        }
        assert_eq!(cache.len(), LUT_CACHE_MAX);
        cache
            .get_or_load(&ctx, &path(0).to_string_lossy())
            .expect("still there");

        // One more file evicts the least recently used — which is now cube1,
        // not cube0.
        cache
            .get_or_load(&ctx, &path(LUT_CACHE_MAX).to_string_lossy())
            .expect("loaded");
        assert_eq!(cache.len(), LUT_CACHE_MAX, "the bound holds");
        assert!(
            cache
                .entries
                .iter()
                .any(|(p, _, _)| p == &*path(0).to_string_lossy()),
            "the one that was used again survived"
        );
        assert!(
            !cache
                .entries
                .iter()
                .any(|(p, _, _)| p == &*path(1).to_string_lossy()),
            "and the least recently used went"
        );
    }
}

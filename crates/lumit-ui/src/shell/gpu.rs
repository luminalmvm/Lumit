//! `shell::gpu` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Self-contained (the GPU viewer and the draw-list
//! types it consumes); references crate deps by full path, so it needs no
//! `use super::*` prelude.

/// One decoded layer ready to composite (evaluator v0).
#[cfg(feature = "media")]
pub struct MatteDraw {
    pub rgba: Vec<u8>,
    pub tex_w: u32,
    pub tex_h: u32,
    pub natural_size: (f32, f32),
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub opacity: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    pub luma: bool,
    pub inverted: bool,
    /// The matte source's own effect stack, resolved at the matte's layer time
    /// (docs/impl/layer-input.md; K-decision). Non-empty only when the consumer's
    /// `MatteRef::after_effects` is set — the effects then run on the matte
    /// texture (upload → linearise → `run_ops`) before it is composited alone, so
    /// a keyed or blurred matte gates by its processed pixels. Empty is the
    /// source-only default. Temporal inputs (neighbours/flow/depth) are not fed
    /// through an after-effects matte in v1, so an echo or flow effect on the
    /// matte source degrades to a still (documented boundary).
    pub fx: Vec<lumit_core::fx::Resolved>,
    /// The matte source's `lut` file paths, 1:1 and in order with the `Resolved::
    /// Lut` ops in `fx` (as for a layer's own `lut_files`). Empty unless
    /// `after_effects` and the matte source has a LUT.
    pub lut_files: Vec<Option<String>>,
}

/// A depth-of-field depth input packaged for the preview (docs/impl/
/// layer-input.md §2): the referenced layer's **source** pixels, ready for
/// [`crate::fxops::render_layer_input`] to resample into the consuming layer's
/// working raster. The referenced layer is rendered source-only (its own
/// effect stack is not applied), exactly as a matte source is — so a depth
/// reference can never recurse into another effect, and the preview and export
/// threads produce the same depth pass (K-031).
#[cfg(feature = "media")]
pub struct DofInputDraw {
    pub rgba: Vec<u8>,
    pub tex_w: u32,
    pub tex_h: u32,
    /// The depth layer's own effect stack, resolved at its layer time — run on
    /// the depth texture before it is resampled, when the consuming effect's
    /// `depth_after_effects` flag is set (K-125, mirroring the after-effects
    /// matte). Empty is the source-only default. Temporal inputs are not fed
    /// through an after-effects depth input in v1 (same boundary as the matte).
    pub fx: Vec<lumit_core::fx::Resolved>,
    /// The depth layer's `lut` file paths, 1:1 with the `Resolved::Lut` ops in
    /// `fx`. Empty unless `depth_after_effects` and the depth layer has a LUT.
    pub lut_files: Vec<Option<String>>,
}

/// Where a draw's pixels come from: decoded/synthesised bytes, or a nested
/// comp realised recursively on the GPU (Precomp layers).
#[cfg(feature = "media")]
pub enum DrawSource {
    Pixels {
        rgba: Vec<u8>,
        tex_w: u32,
        tex_h: u32,
    },
    Nested {
        width: u32,
        height: u32,
        background: [f64; 4],
        draws: Vec<CompLayerDraw>,
        /// The nested comp's own active camera at this time.
        camera: Option<lumit_core::model::CameraPose>,
    },
    /// An adjustment layer's staging point (docs/06 §1.5): no pixels of its
    /// own — the draw's `fx` runs on the composite of every draw before it,
    /// and its placement/opacity/`mask_cov` shape the coverage that
    /// attenuates the result. Only emitted with a live, non-empty stack.
    Adjust,
}

/// The below-stack re-rendered at a held/sample time for a temporal adjustment
/// (Posterize Time, docs/08 §3.25; docs/impl/temporal-rerender.md): the draws
/// beneath the effect's layer, built by `build_comp_draws` at the held comp
/// time `tau`, plus the comp's camera at `tau`. Carried on the adjustment's
/// [`DrawSource::Adjust`] draw so `Realiser::realise` composites the held
/// version in place of the plain below-composite — the same `build_comp_draws`
/// + `realise` export drives, so preview equals export (K-031).
#[cfg(feature = "media")]
pub struct TemporalBelow {
    pub draws: Vec<CompLayerDraw>,
    pub camera: Option<lumit_core::model::CameraPose>,
}

/// The below-stack re-rendered at N sub-frame times for an accumulation motion
/// blur adjustment (docs/08 §3.26; docs/impl/temporal-rerender.md §3): one draw
/// list + camera per shutter sample. `Realiser::realise` renders each, averages
/// the N finished composites with the hardware additive-at-1/N pass
/// ([`lumit_gpu::Compositor::accumulate`]), then blends that average against the
/// plain frame-time below-composite by `mix`, and the result stands in for the
/// below-composite the adjustment's own effects and coverage blend see — the
/// same `render_below_at` (via `below_draws_at`) export drives, so preview equals
/// export (K-031). Carried on the adjustment's [`DrawSource::Adjust`] draw; None
/// on every ordinary draw and every non-accumulation adjustment.
#[cfg(feature = "media")]
pub struct AccumulationBelow {
    /// One below-stack draw list + camera per sub-frame sample time `τ_k`.
    pub samples: Vec<(Vec<CompLayerDraw>, Option<lumit_core::model::CameraPose>)>,
    /// Averaged-over-original blend, 0..1 (1 = full accumulation blur).
    pub mix: f32,
}

#[cfg(feature = "media")]
pub struct CompLayerDraw {
    pub source: DrawSource,
    /// The layer's natural pixel size — transforms act in comp pixels even
    /// when the texture was decoded at a reduced preview resolution.
    pub natural_size: (f32, f32),
    pub position: (f32, f32),
    pub anchor: (f32, f32),
    pub scale: (f32, f32),
    pub rotation_deg: f32,
    pub opacity: f32,
    pub z: f32,
    pub rotation_x_deg: f32,
    pub rotation_y_deg: f32,
    pub three_d: bool,
    pub matte: Option<MatteDraw>,
    pub blend: lumit_gpu::Blend,
    /// Layer-space mask coverage (white RGBA, alpha = coverage) for
    /// GPU-sourced layers — Precomps, whose pixels never exist CPU-side.
    pub mask_cov: Option<(Vec<u8>, u32, u32)>,
    /// Parent placement for layers spliced out of a collapsed Precomp
    /// (docs/06 §1.4): multiplied in front of this draw's own placement so
    /// content is resampled once, never twice. From lumit_gpu::place_matrix.
    pub pre: Option<[[f32; 4]; 4]>,
    /// The layer's live effect stack, resolved to plain numbers at this
    /// frame (docs/08; radius already in texture pixels). Applied to the
    /// linear source texture after masks, before the transform.
    pub fx: Vec<lumit_core::fx::Resolved>,
    /// Decoded neighbour source frames for a temporal effect (echo etc.),
    /// keyed by frame offset — same sRGB8 form and decoded size as a Pixels
    /// source. Empty unless the stack is temporal.
    pub neighbours: Vec<(i32, Vec<u8>, u32, u32)>,
    /// The layer's dense forward flow field `(u, v, conf, w, h)` for Fast
    /// motion blur (docs/08 §3.2), carried from its decode job — `w × h` matches
    /// the decoded source. `conf` is the per-pixel confidence in 0..1 (FX-19)
    /// that tapers the streak; Datamosh reads only `(u, v)`. None unless the
    /// stack wants one.
    #[allow(clippy::type_complexity)]
    pub flow_field: Option<(Vec<f32>, Vec<f32>, Vec<f32>, u32, u32)>,
    /// The ordered file paths of the layer's enabled built-in `lut` effects
    /// (docs/08 §3.11; None = unset). Because `resolve_stack` keeps the same
    /// filter and order and a `lut` effect always resolves to exactly one
    /// `Resolved::Lut`, this list is 1:1 and in order with the stack's `Lut`
    /// ops — the caller loads each path and passes the parallel `luts` to
    /// `run_ops`. No GPU work happens here; these are just the strings.
    pub lut_files: Vec<Option<String>>,
    /// The depth inputs of the layer's enabled built-in `dof` effects (docs/08
    /// §3.22, docs/impl/layer-input.md; None = unset/dangling). Because
    /// `resolve_stack` keeps the same filter and order and a `dof` effect
    /// always resolves to exactly one `Resolved::Dof`, this list is 1:1 and in
    /// order with the stack's `Dof` ops — the caller renders each one alone at
    /// comp size and passes the parallel `layer_inputs` to `run_ops`. Each
    /// carries the referenced layer's source pixels; the GPU render happens in
    /// `realise_segment`.
    pub dof_inputs: Vec<Option<DofInputDraw>>,
    /// Per-layer motion-blur sub-frame placements (docs/06 §4, K-120): the
    /// layer's own transform re-evaluated across the open shutter. Empty unless
    /// the comp master and the layer switch are both on (and samples ≥ 2), in
    /// which case the compositor draws the layer's SAME texture at each of
    /// these and averages them into one smeared layer; the single-placement
    /// fields above stay the frame-time (k=0-ish) representative placement.
    pub mb: Vec<lumit_gpu::MbSample>,
    /// The below-stack re-rendered at a held time for a temporal adjustment
    /// (Posterize Time, docs/08 §3.25). Some only on an adjustment
    /// [`DrawSource::Adjust`] draw whose stack holds a Posterize Time effect
    /// scoped to *everything below*: `realise` then composites this held
    /// version (with the adjustment's own remaining effects) in place of the
    /// plain below-composite, blended by the adjustment's coverage. None on
    /// every ordinary draw, so nothing changes when no temporal effect is live.
    pub temporal_below: Option<TemporalBelow>,
    /// The below-stack re-rendered at N sub-frame times for accumulation motion
    /// blur (docs/08 §3.26). Some only on an adjustment [`DrawSource::Adjust`]
    /// draw whose stack holds a live accumulation MB effect: `realise` averages
    /// the N finished composites and blends by `mix`, standing in for the plain
    /// below-composite. Takes precedence over `temporal_below` when both are set
    /// (one temporal re-render per adjustment in v1). None on every ordinary draw.
    pub accumulation_below: Option<AccumulationBelow>,
}

/// GPU display path (slice 5 completion): decoded sRGB bytes → linear fp16
/// working texture → display texture registered with egui. Falls back to the
/// CPU/egui-texture path when no wgpu render state exists.
#[cfg(feature = "media")]
pub struct GpuViewer {
    ctx: lumit_gpu::GpuContext,
    engine: lumit_gpu::ColourEngine,
    compositor: lumit_gpu::Compositor,
    fx: lumit_gpu::fx::FxEngine,
    render_state: egui_wgpu::RenderState,
    /// Keep the display texture alive while egui samples it.
    current: Option<(egui_wgpu::wgpu::Texture, egui::TextureId)>,
    /// The VRAM tier (docs/06 §5): displayed textures per frame key, LRU by
    /// position (back = most recent), so a warm scrub re-presents with zero
    /// upload or colour work. Budgeted by texture bytes.
    vram: Vec<VramFrame>,
    vram_bytes: u64,
    /// The live VRAM-tier budget (Settings → Performance, K-100). Starts at
    /// [`VRAM_TIER_CAP`] and moves when the owner drags the slider.
    vram_cap: u64,
    /// Parsed-and-uploaded `.cube` LUTs keyed by path (docs/08 §3.11,
    /// docs/impl/lut.md §4): parse + upload happen once per distinct file, not
    /// per frame. `RefCell` because `realise`/`realise_segment` take `&self`.
    /// Path-only key for now — mtime invalidation and bounding are documented
    /// follow-ups (an edited-on-disk LUT needs the app reopened).
    lut_cache: std::cell::RefCell<std::collections::HashMap<String, crate::fxops::LoadedLut>>,
}

/// One VRAM-tier entry: the display texture, its egui registration, and size.
#[cfg(feature = "media")]
pub(crate) struct VramFrame {
    key: u128,
    /// Never read — held so the GPU texture outlives its egui registration.
    _texture: egui_wgpu::wgpu::Texture,
    id: egui::TextureId,
    size: egui::Vec2,
    bytes: u64,
}

/// VRAM-tier budget (docs/13-PERFORMANCE-RULES.md: budgets gate merges; the
/// governor makes this adaptive later). 512 MB of display textures ≈ 60
/// frames of 4K, several hundred of 1080p.
#[cfg(feature = "media")]
pub(crate) const VRAM_TIER_CAP: u64 = 512 * 1024 * 1024;

/// How many oldest entries must go so `total` fits under `cap` after adding
/// `incoming` bytes. Pure, so the eviction policy is testable off-GPU.
#[cfg(feature = "media")]
pub(crate) fn vram_evict_count(entry_bytes: &[u64], total: u64, incoming: u64, cap: u64) -> usize {
    let mut running = total.saturating_add(incoming);
    let mut n = 0;
    for b in entry_bytes {
        if running <= cap {
            break;
        }
        running = running.saturating_sub(*b);
        n += 1;
    }
    n
}

/// The GPU primitives that turn a comp draw list into a linear texture,
/// borrowed from whichever owner is compositing — the preview's [`GpuViewer`]
/// or the export renderer (`crate::export`). Factoring the realise logic behind
/// one borrowed handle is what lets the preview and export share a single
/// re-render path (`render_below_at`, docs/impl/temporal-rerender.md): both
/// drive the identical compositor, so a comp realises the same in the viewport
/// and the file (K-031).
#[cfg(feature = "media")]
pub(crate) struct Realiser<'a> {
    /// Owned handle (a cheap Arc-backed clone via [`lumit_gpu::GpuContext::
    /// from_parts`]) so the moved realise code keeps passing `&self.ctx`
    /// unchanged; the engines below cannot be cloned, so they stay borrowed.
    pub ctx: lumit_gpu::GpuContext,
    pub engine: &'a lumit_gpu::ColourEngine,
    pub compositor: &'a lumit_gpu::Compositor,
    pub fx: &'a lumit_gpu::fx::FxEngine,
    pub lut_cache:
        &'a std::cell::RefCell<std::collections::HashMap<String, crate::fxops::LoadedLut>>,
}

#[cfg(feature = "media")]
impl GpuViewer {
    pub fn new(render_state: egui_wgpu::RenderState) -> Self {
        let ctx = lumit_gpu::GpuContext::from_parts(
            render_state.device.clone(),
            render_state.queue.clone(),
        );
        let engine = lumit_gpu::ColourEngine::new(&ctx);
        let compositor = lumit_gpu::Compositor::new(&ctx);
        let fx = lumit_gpu::fx::FxEngine::new(&ctx);
        Self {
            ctx,
            engine,
            compositor,
            fx,
            render_state,
            current: None,
            vram: Vec::new(),
            vram_bytes: 0,
            vram_cap: VRAM_TIER_CAP,
            lut_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    /// A second handle to the shared device for the export thread.
    pub fn export_context(&self) -> lumit_gpu::GpuContext {
        lumit_gpu::GpuContext::from_parts(self.ctx.device.clone(), self.ctx.queue.clone())
    }

    /// Borrow this viewer's GPU primitives as a [`Realiser`] — the shared
    /// draw-list compositor both the preview (here) and export
    /// (`render_comp_linear`) drive, so a comp realises identically in the
    /// viewport and the file (K-031).
    fn realiser(&self) -> Realiser<'_> {
        Realiser {
            ctx: lumit_gpu::GpuContext::from_parts(self.ctx.device.clone(), self.ctx.queue.clone()),
            engine: &self.engine,
            compositor: &self.compositor,
            fx: &self.fx,
            lut_cache: &self.lut_cache,
        }
    }

    /// Realise a draw list into a linear comp texture — delegates to the shared
    /// [`Realiser`].
    fn realise(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> egui_wgpu::wgpu::Texture {
        self.realiser()
            .realise(camera, width, height, background, layers)
    }
}

#[cfg(feature = "media")]
impl Realiser<'_> {
    /// Turn a layer's ordered `lut_files` into the parallel `luts` list
    /// `run_ops` binds (docs/08 §3.11): each `Some(path)` is parsed and
    /// uploaded once (cached by path), a 1D or unreadable/absent file yields a
    /// `None` slot (a labelled no-op, never a fault — docs/impl/lut.md §8). The
    /// output is 1:1 and in order with `files`, so the k-th slot lines up with
    /// the k-th `Resolved::Lut` op.
    fn load_luts(&self, files: &[Option<String>]) -> Vec<Option<crate::fxops::LoadedLut>> {
        let mut cache = self.lut_cache.borrow_mut();
        files
            .iter()
            .map(|slot| {
                let path = slot.as_ref()?;
                if !cache.contains_key(path) {
                    // Any IO/parse error, or a 1D LUT, leaves the slot empty:
                    // the effect is a passthrough, never a panic (§3.11).
                    if let Some(loaded) = std::fs::read_to_string(path)
                        .ok()
                        .and_then(|text| lumit_core::lut::parse_cube(&text).ok())
                        .and_then(|lut| match lut {
                            lumit_core::lut::Lut::Cube3d(l) => Some(crate::fxops::LoadedLut {
                                texture: lumit_gpu::fx::upload_lut_3d(
                                    &self.ctx,
                                    l.size as u32,
                                    &l.data,
                                ),
                                size: l.size as u32,
                            }),
                            lumit_core::lut::Lut::Cube1d(_) => None,
                        })
                    {
                        cache.insert(path.clone(), loaded);
                    }
                }
                cache.get(path).cloned()
            })
            .collect()
    }

    /// Realise a draw list into a linear comp texture (recursive for
    /// Nested), staging at each Adjust draw (docs/06 §1.5): everything
    /// before it composites into an intermediate, the adjustment's stack
    /// runs on that, and the two blend by coverage; the draws after
    /// composite straight onto the blended result (seeded, no resample).
    ///
    /// Render a layer's depth-of-field depth inputs (docs/impl/layer-input.md
    /// §2): each `DofInputDraw` (the referenced layer's source pixels) is
    /// uploaded, linearised and resampled into the effect's working raster
    /// `(w, h)` through the shared [`crate::fxops::render_layer_input`], so the
    /// parallel `layer_inputs` handed to `run_ops` is 1:1 with the stack's
    /// `Dof` ops and aligned with the layer texture the kernel blurs. Export
    /// renders these identically (K-031).
    fn render_dof_inputs(
        &self,
        inputs: &[Option<DofInputDraw>],
        w: u32,
        h: u32,
    ) -> Vec<Option<egui_wgpu::wgpu::Texture>> {
        inputs
            .iter()
            .map(|slot| {
                let d = slot.as_ref()?;
                let src = self
                    .engine
                    .upload_srgb8(&self.ctx, &d.rgba, d.tex_w, d.tex_h);
                let linear = self.engine.linearise(&self.ctx, &src);
                // After-effects depth (K-125): run the depth layer's own stack
                // on its texture before it is resampled, when the consumer set
                // depth_after_effects. Temporal inputs stay empty in v1 (same
                // boundary as the after-effects matte). Export does the same, so
                // the two depth passes match (K-031).
                let linear = if d.fx.is_empty() {
                    linear
                } else {
                    let luts = self.load_luts(&d.lut_files);
                    crate::fxops::run_ops(
                        self.fx,
                        &self.ctx,
                        linear,
                        d.tex_w,
                        d.tex_h,
                        &d.fx,
                        &[],
                        None,
                        &luts,
                        &[],
                    )
                };
                Some(crate::fxops::render_layer_input(
                    self.compositor,
                    &self.ctx,
                    w,
                    h,
                    &linear,
                    d.tex_w as f32,
                    d.tex_h as f32,
                ))
            })
            .collect()
    }

    pub(crate) fn realise(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> egui_wgpu::wgpu::Texture {
        let mut acc: Option<egui_wgpu::wgpu::Texture> = None;
        let mut start = 0usize;
        for (i, l) in layers.iter().enumerate() {
            if !matches!(l.source, DrawSource::Adjust) {
                continue;
            }
            let below =
                self.realise_segment(camera, width, height, background, &layers[start..i], &acc);
            // An adjustment layer processes the composite below, which has no
            // footage neighbour frames — temporal effects on an adjustment
            // layer are a later refinement, so no neighbours here. Its LUT and
            // depth-of-field effects still apply (§3.11, §3.22): load/render
            // them the same way the per-layer path does, so preview stays
            // identical to export (K-031). The adjustment stack runs on the
            // comp-sized composite, so its depth inputs resample to comp size.
            let luts = self.load_luts(&l.lut_files);
            let layer_inputs = self.render_dof_inputs(&l.dof_inputs, width, height);
            // Posterize Time everything-below (docs/08 §3.25): the input this
            // adjustment's own effects run on is the below-stack held at the
            // posterised time, not the plain below-composite. The held draws and
            // camera were built by the shared `below_draws_at` (identical to the
            // texture export's `render_below_at` produces, K-031); the coverage
            // blend below still lays the result over the live below-at-t, so a
            // mask reveals the held region. None on an ordinary adjustment.
            // Accumulation motion blur (docs/08 §3.26) takes precedence: it
            // renders N sub-frame below-stacks and averages them; else Posterize
            // holds one below-stack; else the plain below-composite.
            let fx_input = if let Some(ab) = &l.accumulation_below {
                self.accumulate_below(width, height, background, ab, &below)
            } else if let Some(tb) = &l.temporal_below {
                self.realise(tb.camera, width, height, background, &tb.draws)
            } else {
                below.clone()
            };
            let processed = crate::fxops::run_ops(
                self.fx,
                &self.ctx,
                fx_input,
                width,
                height,
                &l.fx,
                &[],
                None,
                &luts,
                &layer_inputs,
            );
            let coverage = self.coverage_texture(camera, width, height, l);
            acc = Some(self.fx.adjust_blend(
                &self.ctx,
                &below,
                &processed,
                &coverage,
                width,
                height,
                (l.opacity / 100.0).clamp(0.0, 1.0),
            ));
            start = i + 1;
        }
        self.realise_segment(camera, width, height, background, &layers[start..], &acc)
    }

    /// Accumulation motion blur (docs/08 §3.26, docs/impl/temporal-rerender.md
    /// §3): render each sub-frame below-stack through the same realise path,
    /// average the N finished composites with the hardware additive-at-`1/N` pass
    /// ([`lumit_gpu::Compositor::accumulate`]), then blend that average against
    /// the frame-time below-composite `below` by `mix` (a linear interpolation
    /// the additive blend gives exactly). The result stands in for the
    /// below-composite the adjustment's own effects and coverage blend see. A
    /// still scene averages back to `below` bit-for-bit (the K-031 identity); a
    /// moving one smears. Export runs the identical combine, so the two agree.
    fn accumulate_below(
        &self,
        width: u32,
        height: u32,
        background: [f64; 4],
        ab: &AccumulationBelow,
        below: &egui_wgpu::wgpu::Texture,
    ) -> egui_wgpu::wgpu::Texture {
        let frames: Vec<egui_wgpu::wgpu::Texture> = ab
            .samples
            .iter()
            .map(|(draws, camera)| self.realise(*camera, width, height, background, draws))
            .collect();
        if frames.is_empty() {
            // No samples (N < 2) degrades to the plain below — never a panic.
            return below.clone();
        }
        // Equal weights 1/N sum to 1: the premultiplied arithmetic mean.
        let weight = 1.0 / frames.len() as f32;
        let avg_layers: Vec<(&egui_wgpu::wgpu::Texture, f32)> =
            frames.iter().map(|f| (f, weight)).collect();
        let average = self
            .compositor
            .accumulate(&self.ctx, width, height, &avg_layers);
        if ab.mix >= 1.0 {
            average
        } else {
            // Mix blends the blurred average against the live below-composite.
            self.compositor.accumulate(
                &self.ctx,
                width,
                height,
                &[(below, 1.0 - ab.mix), (&average, ab.mix)],
            )
        }
    }

    /// The adjustment layer's comp-space coverage (docs/06 §1.5): its mask
    /// raster — white where the effects apply — placed by its transform,
    /// so the transform moves the coverage map, never the picture. No
    /// masks means full coverage (a white quad over the whole comp).
    fn coverage_texture(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        l: &CompLayerDraw,
    ) -> egui_wgpu::wgpu::Texture {
        let white = [255u8, 255, 255, 255];
        let (rgba, w, h): (&[u8], u32, u32) = match &l.mask_cov {
            Some((rgba, w, h)) => (rgba, *w, *h),
            None => (&white, 1, 1),
        };
        let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        self.compositor.composite_with_camera(
            &self.ctx,
            width,
            height,
            [0.0, 0.0, 0.0, 0.0],
            &[lumit_gpu::CompositeLayer {
                texture: &linear,
                size: l.natural_size,
                position: l.position,
                anchor: l.anchor,
                scale: l.scale,
                rotation_deg: l.rotation_deg,
                // Layer opacity is applied once, in the blend itself.
                opacity: 100.0,
                matte: None,
                blend: lumit_gpu::Blend::Normal,
                z: l.z,
                rotation_x_deg: l.rotation_x_deg,
                rotation_y_deg: l.rotation_y_deg,
                three_d: l.three_d,
                layer_mask: None,
                pre: None,
            }],
            cam_mat,
        )
    }

    /// Composite one adjustment-free run of draws; `seed` (a previous
    /// stage's output) replaces the cleared background when present.
    fn realise_segment(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        seed: &Option<egui_wgpu::wgpu::Texture>,
    ) -> egui_wgpu::wgpu::Texture {
        let mut linear_textures: Vec<egui_wgpu::wgpu::Texture> = Vec::with_capacity(layers.len());
        for l in layers {
            let tex = match &l.source {
                DrawSource::Pixels { rgba, tex_w, tex_h } => {
                    let src = self.engine.upload_srgb8(&self.ctx, rgba, *tex_w, *tex_h);
                    self.engine.linearise(&self.ctx, &src)
                }
                DrawSource::Nested {
                    width,
                    height,
                    background,
                    draws,
                    camera,
                } => self.realise(*camera, *width, *height, *background, draws),
                DrawSource::Adjust => {
                    // realise splits segments at every Adjust draw, so none
                    // reaches here; a transparent texel keeps the no-panic
                    // rule (and draws nothing) if that ever regresses.
                    let src = self.engine.upload_srgb8(&self.ctx, &[0, 0, 0, 0], 1, 1);
                    self.engine.linearise(&self.ctx, &src)
                }
            };
            // The effect stack runs on the linear source, after masks and
            // before the transform (docs/08 §1.5; docs/06 render order).
            let tex = if l.fx.is_empty() {
                tex
            } else {
                let (w, h) = (tex.width(), tex.height());
                // Neighbour source frames a temporal effect (echo) reads;
                // empty for a plain stack, so this uploads nothing then.
                let neighbours: Vec<(i32, egui_wgpu::wgpu::Texture)> = l
                    .neighbours
                    .iter()
                    .map(|(offset, rgba, nw, nh)| {
                        let src = self.engine.upload_srgb8(&self.ctx, rgba, *nw, *nh);
                        (*offset, self.engine.linearise(&self.ctx, &src))
                    })
                    .collect();
                // The dense motion field for Fast motion blur, uploaded as its
                // own texture (only when it matches the layer's raster). The
                // confidence rides in the .z channel (FX-19).
                let flow = l.flow_field.as_ref().and_then(|(u, v, conf, fw, fh)| {
                    (*fw == w && *fh == h)
                        .then(|| lumit_gpu::fx::upload_flow_field(&self.ctx, u, v, conf, w, h))
                });
                // The parsed-and-uploaded `.cube` LUTs, 1:1 with the stack's
                // `Resolved::Lut` ops (§3.11); the same load export uses (K-031).
                let luts = self.load_luts(&l.lut_files);
                // The depth-of-field depth inputs, resampled to this layer's
                // working raster (w, h), 1:1 with the stack's Resolved::Dof ops
                // (§3.22); the same render export runs (K-031).
                let layer_inputs = self.render_dof_inputs(&l.dof_inputs, w, h);
                crate::fxops::run_ops(
                    self.fx,
                    &self.ctx,
                    tex,
                    w,
                    h,
                    &l.fx,
                    &neighbours,
                    flow.as_ref(),
                    &luts,
                    &layer_inputs,
                )
            };
            linear_textures.push(tex);
        }
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // Per-layer motion blur (docs/06 §4, K-120): a blurring layer's
        // fx-processed texture is drawn at each sub-frame placement and
        // averaged into one comp-sized smear by the shared helper both preview
        // and export call (K-031). The layer's real blend/opacity/matte/mask
        // then apply once to the averaged image, at the 1:1 composite below.
        let mb_textures: Vec<Option<egui_wgpu::wgpu::Texture>> = linear_textures
            .iter()
            .zip(layers)
            .map(|(tex, l)| {
                (!l.mb.is_empty()).then(|| {
                    self.compositor.motion_blur_average(
                        &self.ctx,
                        width,
                        height,
                        tex,
                        l.natural_size,
                        &l.mb,
                        l.three_d,
                        l.pre,
                        cam_mat,
                    )
                })
            })
            .collect();
        // Layer-space mask textures (Precomp masks — GPU mask pass).
        let mask_textures: Vec<Option<egui_wgpu::wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.mask_cov
                    .as_ref()
                    .map(|(rgba, w, h)| self.engine.upload_srgb8(&self.ctx, rgba, *w, *h))
            })
            .collect();
        // Matte layers render alone into comp space (one texture per consumer;
        // the shared-matte cache optimisation arrives with the evaluator).
        let matte_textures: Vec<Option<egui_wgpu::wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.matte.as_ref().map(|m| {
                    let src = self
                        .engine
                        .upload_srgb8(&self.ctx, &m.rgba, m.tex_w, m.tex_h);
                    let linear = self.engine.linearise(&self.ctx, &src);
                    // After-effects matte (K-decision): run the matte source's own
                    // stack on its texture before it gates the consumer, so a keyed
                    // or blurred matte works. Temporal inputs stay empty in v1 — the
                    // matte source's echo/flow degrades to a still (documented). The
                    // same run export performs, so the two agree (K-031).
                    let linear = if m.fx.is_empty() {
                        linear
                    } else {
                        let luts = self.load_luts(&m.lut_files);
                        crate::fxops::run_ops(
                            self.fx,
                            &self.ctx,
                            linear,
                            m.tex_w,
                            m.tex_h,
                            &m.fx,
                            &[],
                            None,
                            &luts,
                            &[],
                        )
                    };
                    self.compositor.composite_with_camera(
                        &self.ctx,
                        width,
                        height,
                        [0.0, 0.0, 0.0, 0.0],
                        &[lumit_gpu::CompositeLayer {
                            texture: &linear,
                            size: m.natural_size,
                            position: m.position,
                            anchor: m.anchor,
                            scale: m.scale,
                            rotation_deg: m.rotation_deg,
                            opacity: m.opacity,
                            matte: None,
                            blend: lumit_gpu::Blend::Normal,
                            z: m.z,
                            rotation_x_deg: m.rotation_x_deg,
                            rotation_y_deg: m.rotation_y_deg,
                            three_d: m.three_d,
                            layer_mask: None,
                            pre: None,
                        }],
                        cam_mat,
                    )
                })
            })
            .collect();
        let comp_layers: Vec<lumit_gpu::CompositeLayer> = linear_textures
            .iter()
            .zip(layers)
            .zip(&matte_textures)
            .zip(&mask_textures)
            .zip(&mb_textures)
            .map(|((((texture, l), matte_tex), mask_tex), mb_tex)| {
                let matte = matte_tex.as_ref().map(|mt| lumit_gpu::MatteInput {
                    texture: mt,
                    luma: l.matte.as_ref().is_some_and(|m| m.luma),
                    inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
                });
                match mb_tex {
                    // Motion-blurred: composite the averaged comp-sized smear
                    // 1:1 (identity placement), the layer's real blend, opacity,
                    // matte and mask applied once to the averaged image.
                    Some(avg) => lumit_gpu::CompositeLayer {
                        texture: avg,
                        size: (width as f32, height as f32),
                        position: (0.0, 0.0),
                        anchor: (0.0, 0.0),
                        scale: (100.0, 100.0),
                        rotation_deg: 0.0,
                        opacity: l.opacity,
                        z: 0.0,
                        rotation_x_deg: 0.0,
                        rotation_y_deg: 0.0,
                        three_d: false,
                        matte,
                        blend: l.blend,
                        layer_mask: mask_tex.as_ref(),
                        pre: None,
                    },
                    None => lumit_gpu::CompositeLayer {
                        texture,
                        size: l.natural_size,
                        position: l.position,
                        anchor: l.anchor,
                        scale: l.scale,
                        rotation_deg: l.rotation_deg,
                        opacity: l.opacity,
                        z: l.z,
                        rotation_x_deg: l.rotation_x_deg,
                        rotation_y_deg: l.rotation_y_deg,
                        three_d: l.three_d,
                        matte,
                        blend: l.blend,
                        layer_mask: mask_tex.as_ref(),
                        pre: l.pre,
                    },
                }
            })
            .collect();
        self.compositor.composite_seeded(
            &self.ctx,
            width,
            height,
            background,
            &comp_layers,
            cam_mat,
            seed.as_ref(),
        )
    }
}

#[cfg(feature = "media")]
impl GpuViewer {
    /// Realise a comp frame straight to display-ready sRGB bytes (Kura's
    /// cache-fill path — nothing is registered for painting).
    pub(crate) fn realise_to_bytes(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> Option<Vec<u8>> {
        let linear = self.realise(camera, width, height, background, layers);
        let shown = self.engine.display(&self.ctx, &linear);
        self.engine.readback8(&self.ctx, &shown).ok()
    }

    /// Realise a comp's draws and register the frame for painting.
    pub(crate) fn present_comp(
        &mut self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> (egui::TextureId, egui::Vec2) {
        let linear = self.realise(camera, width, height, background, layers);
        let shown = self.engine.display(&self.ctx, &linear);
        let view = shown.create_view(&Default::default());
        let id = self.render_state.renderer.write().register_native_texture(
            &self.ctx.device,
            &view,
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        if let Some((_, old)) = self.current.replace((shown, id)) {
            self.render_state.renderer.write().free_texture(&old);
        }
        (id, egui::vec2(width as f32, height as f32))
    }

    /// A warm VRAM hit: re-present a frame whose display texture is still on
    /// the GPU — no upload, no colour passes (docs/06 §5: VRAM reads first).
    pub(crate) fn present_vram(&mut self, key: u128) -> Option<(egui::TextureId, egui::Vec2)> {
        let idx = self.vram.iter().position(|e| e.key == key)?;
        let entry = self.vram.remove(idx);
        let out = (entry.id, entry.size);
        self.vram.push(entry); // back = most recently used
        Some(out)
    }

    /// Present a RAM-tier frame and keep its display texture in the VRAM tier
    /// under `key`, evicting oldest entries past the byte budget.
    pub(crate) fn present_keyed(
        &mut self,
        key: u128,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> (egui::TextureId, egui::Vec2) {
        let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        let shown = self.engine.display(&self.ctx, &linear);
        let view = shown.create_view(&Default::default());
        let id = self.render_state.renderer.write().register_native_texture(
            &self.ctx.device,
            &view,
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        let bytes = u64::from(w) * u64::from(h) * 4;
        let sizes: Vec<u64> = self.vram.iter().map(|e| e.bytes).collect();
        let drop_n = vram_evict_count(&sizes, self.vram_bytes, bytes, self.vram_cap);
        for old in self.vram.drain(..drop_n) {
            self.vram_bytes = self.vram_bytes.saturating_sub(old.bytes);
            self.render_state.renderer.write().free_texture(&old.id);
        }
        let size = egui::vec2(w as f32, h as f32);
        self.vram.push(VramFrame {
            key,
            _texture: shown,
            id,
            size,
            bytes,
        });
        self.vram_bytes = self.vram_bytes.saturating_add(bytes);
        (id, size)
    }

    /// Move the VRAM-tier budget (Settings → Performance, K-100), evicting
    /// oldest entries immediately if the new cap is below what is currently
    /// held — the same oldest-first policy `present_keyed` uses on insert,
    /// just with nothing incoming.
    pub(crate) fn set_vram_cap(&mut self, bytes: u64) {
        self.vram_cap = bytes;
        let sizes: Vec<u64> = self.vram.iter().map(|e| e.bytes).collect();
        let drop_n = vram_evict_count(&sizes, self.vram_bytes, 0, self.vram_cap);
        for old in self.vram.drain(..drop_n) {
            self.vram_bytes = self.vram_bytes.saturating_sub(old.bytes);
            self.render_state.renderer.write().free_texture(&old.id);
        }
    }

    /// Drop every VRAM-tier entry (Settings → Performance "Clear cache",
    /// K-100), releasing each texture's egui registration so nothing leaks.
    pub(crate) fn clear_vram(&mut self) {
        for old in self.vram.drain(..) {
            self.render_state.renderer.write().free_texture(&old.id);
        }
        self.vram_bytes = 0;
    }

    /// Upload a decoded frame through the colour pipeline; returns the egui
    /// texture id + size to paint.
    pub(crate) fn present(&mut self, rgba: &[u8], w: u32, h: u32) -> (egui::TextureId, egui::Vec2) {
        let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        let shown = self.engine.display(&self.ctx, &linear);
        let view = shown.create_view(&Default::default());
        let id = self.render_state.renderer.write().register_native_texture(
            &self.ctx.device,
            &view,
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        if let Some((_, old)) = self.current.replace((shown, id)) {
            self.render_state.renderer.write().free_texture(&old);
        }
        (id, egui::vec2(w as f32, h as f32))
    }
}

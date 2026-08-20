//! Turning a draw list into pixels on the graphics card.
//!
//! # In plain terms
//!
//! [`crate::draw`] describes a frame; this module *makes* it. [`Realiser`]
//! borrows the GPU primitives from whoever owns them — the egui Viewer, the
//! headless renderer the Flutter frontend drives, or the exporter — and walks a
//! draw list: upload each layer's pixels, convert them to the linear working
//! space, run its effect stack, apply its matte and masks, place it with its
//! transform, and blend it onto the accumulating frame. Nested comps recurse;
//! adjustment layers split the walk in two so the stack below can be
//! composited, processed, and blended back by coverage.
//!
//! Because every caller drives this one walk, a comp looks the same in the
//! viewport, in Flutter, and in the exported file (K-031).

use crate::draw::{AccumulationBelow, CompLayerDraw, DrawSource, LayerInputDraw};
use crate::fxops::LoadedLut;

/// The GPU primitives that turn a comp draw list into a linear texture,
/// borrowed from whichever owner is compositing — a frontend's viewer, the
/// headless renderer, or the export renderer. Factoring the realise logic behind
/// one borrowed handle is what lets preview and export share a single re-render
/// path (`render_below_at`, docs/impl/temporal-rerender.md): all of them drive
/// the identical compositor, so a comp realises the same in the viewport and
/// the file (K-031).
pub struct Realiser<'a> {
    /// Owned handle (a cheap Arc-backed clone via [`lumit_gpu::GpuContext::
    /// from_parts`]) so the realise code can keep passing `&self.ctx`; the
    /// engines below cannot be cloned, so they stay borrowed.
    pub ctx: lumit_gpu::GpuContext,
    pub engine: &'a lumit_gpu::ColourEngine,
    pub compositor: &'a lumit_gpu::Compositor,
    pub fx: &'a lumit_gpu::fx::FxEngine,
    pub lut_cache: &'a std::cell::RefCell<crate::fxops::LutCache>,
    /// The preview render scale (the K-185 follow-up): every composite this
    /// walk performs allocates its target at [`lumit_gpu::scaled_size`] of the
    /// comp dims while all geometry stays in logical comp pixels. A field
    /// rather than a parameter so the nested/below/adjustment recursions
    /// inherit it with no signature ripple. Export always builds with 1.0, so
    /// the K-031 preview == export identity is untouched at full scale.
    pub render_scale: f32,
    /// The project's anti-aliasing sample count (K-274,
    /// docs/impl/anti-aliasing.md): how many coverage samples per pixel the
    /// composite is drawn with. 1 is the picture Lumit made before the setting
    /// existed. A field beside [`Self::render_scale`], for the same reason —
    /// the nested, below and adjustment recursions inherit it with no signature
    /// ripple — but **not** like it in one way that matters: `render_scale`
    /// differs between preview and export by design, and this must not. Both
    /// paths read the same project field, which is what keeps the K-031
    /// preview-equals-export identity true with anti-aliasing on.
    ///
    /// Already run through [`lumit_gpu::supported_sample_count`] by whoever
    /// built the realiser: by the time it is here it is a count this adapter
    /// really offers.
    pub samples: u32,
    /// The frame's recorder, when this render is being watched (docs/13 §7.1):
    /// it counts finished layers for the Viewer's progress bar and measures
    /// each layer and effect for the render-time indicators. `None` — every
    /// frame of playback, and every unwatched render — walks exactly as it did
    /// before this existed: no clocks, no fences, no extra allocations.
    pub profiler: Option<&'a crate::profile::FrameProfiler>,
}

impl Realiser<'_> {
    /// Start one layer's measurement: the clock, and the list its effect stack
    /// writes a millisecond into per op. Both `None` unless this render is
    /// being measured, which is what keeps an ordinary frame free of them.
    #[allow(clippy::type_complexity)]
    fn layer_clock(&self) -> (Option<std::time::Instant>, Option<Vec<f32>>) {
        match self.profiler {
            Some(p) if p.timing() => (Some(std::time::Instant::now()), Some(Vec::new())),
            _ => (None, None),
        }
    }

    /// Close one layer's measurement and hand it to the profiler: the fence
    /// (see `crate::profile` on why the clock cannot be read without one), the
    /// per-effect list paired back up with the ids the draw carries, and the
    /// layer counted towards the progress bar.
    fn layer_done(
        &self,
        l: &CompLayerDraw,
        started: Option<std::time::Instant>,
        fx_ms: Option<Vec<f32>>,
    ) {
        let Some(p) = self.profiler else {
            return;
        };
        let ms = match started {
            Some(started) => {
                // Hand this layer's work over before waiting on it. Batching
                // holds a frame's commands back to the end, and a fence over an
                // empty queue would time nothing — so a *measured* frame gives
                // up the batching, layer by layer, which is the cost the
                // stopwatch already declares (docs/13 §7.1, K-276: measuring
                // waits for the card at each layer, which is why it is opt-in
                // and never runs during playback).
                self.ctx.flush();
                self.ctx.device.poll(wgpu::Maintain::Wait);
                started.elapsed().as_secs_f32() * 1000.0
            }
            // Progress is being reported but nothing is being measured: the
            // layer still counts towards the bar, it just has no number.
            None => 0.0,
        };
        let effects = fx_ms
            .map(|ms| {
                l.fx_ids
                    .iter()
                    .copied()
                    .zip(ms)
                    .map(|(effect, ms)| crate::profile::EffectTiming { effect, ms })
                    .collect()
            })
            .unwrap_or_default();
        p.layer_done(l.layer, ms, effects);
    }

    /// Read a layer's `lens_file` paths into (content hash, text) slots,
    /// 1:1 with the stack's `lens_flare` ops (K-264). A `None`
    /// slot (unset, missing on disk, unreadable) degrades to the picked
    /// library lens inside the bake — a labelled fallback, never a fault.
    /// No cache, deliberately: a .lens file is about a kilobyte, the read
    /// is microseconds beside the frame it feeds, and the GPU bake cache
    /// keys on the CONTENT hash — so an edited file takes effect on the
    /// next frame instead of whenever a path-keyed cache is purged.
    fn load_flare_lens(&self, files: &[Option<String>]) -> Vec<Option<(u64, String)>> {
        files
            .iter()
            .map(|slot| {
                slot.as_ref().and_then(|path| {
                    std::fs::read_to_string(path)
                        .ok()
                        .map(|text| (lumit_core::fx::lens_flare::lens_text_hash(&text), text))
                })
            })
            .collect()
    }

    /// Turn a layer's ordered `lut_files` into the parallel `luts` list
    /// `run_ops` binds (docs/08 §3.11): each `Some(path)` is parsed and
    /// uploaded once — cached by path *and* last-modified time, bounded and
    /// LRU-evicted (K-271, docs/impl/lut.md §4) — and a 1D or unreadable/absent
    /// file yields a `None` slot (a labelled no-op, never a fault). The output
    /// is 1:1 and in order with `files`, so the k-th slot lines up with the
    /// k-th `lut` op.
    fn load_luts(&self, files: &[Option<String>]) -> Vec<Option<LoadedLut>> {
        let mut cache = self.lut_cache.borrow_mut();
        files
            .iter()
            .map(|slot| cache.get_or_load(&self.ctx, slot.as_ref()?))
            .collect()
    }

    /// Render a layer's layer-input slots (docs/impl/layer-input.md §2) — the
    /// depth passes of its `dof` effects, the matte sources of its Lens
    /// flares. Each [`LayerInputDraw::Layer`] (the referenced layer's source
    /// pixels) is uploaded, linearised and resampled into the effect's
    /// working raster `(w, h)` through the shared
    /// [`crate::fxops::render_layer_input`], so the parallel list handed to
    /// `run_ops` is 1:1 with the stack's ops and aligned with the layer
    /// texture the kernel reads. Export renders these identically (K-031).
    ///
    /// [`LayerInputDraw::ThisLayer`] (K-288) renders nothing here: it names
    /// the texture `run_ops` is already carrying, which only `run_ops` can
    /// hand over, so it passes through as [`LayerInput::ThisLayer`].
    fn render_layer_inputs(
        &self,
        inputs: &[LayerInputDraw],
        w: u32,
        h: u32,
    ) -> Vec<crate::fxops::LayerInput> {
        use crate::fxops::LayerInput;
        inputs
            .iter()
            .map(|slot| {
                let d = match slot {
                    LayerInputDraw::Absent => return LayerInput::Absent,
                    LayerInputDraw::ThisLayer => return LayerInput::ThisLayer,
                    LayerInputDraw::Layer(d) => d,
                };
                // A Precomp input realises its nested comp exactly as a
                // Precomp layer's picture does (K-266); anything else is
                // the uploaded source pixels.
                let linear = if let Some(n) = &d.nested {
                    self.realise(n.camera, n.width, n.height, n.background, &n.draws)
                } else {
                    let src = self
                        .engine
                        .upload_srgb8(&self.ctx, &d.rgba, d.tex_w, d.tex_h);
                    self.engine.linearise(&self.ctx, &src)
                };
                // Effects-and-masks depth (K-142): run the depth layer's own
                // stack on its texture before it is resampled, when the consumer's
                // depth source is Effects and masks (`d.fx` non-empty). Temporal
                // inputs stay empty in v1 (same boundary as the matte). Export
                // does the same, so the two depth passes match (K-031).
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
                        &[],
                        &[],
                        // No mask paths through a referenced layer's own stack
                        // (K-408) — the same v1 boundary its mattes take.
                        &[],
                        // A matte's own stack is part of the effect that reads
                        // it, not a row of its own: its cost is inside that
                        // layer's span already.
                        None,
                    )
                };
                LayerInput::Texture(crate::fxops::render_layer_input(
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

    /// Realise a draw list into a linear comp texture (recursive for
    /// Nested), staging at each Adjust draw (docs/06 §1.5): everything
    /// before it composites into an intermediate, the adjustment's stack
    /// runs on that, and the two blend by coverage; the draws after
    /// composite straight onto the blended result (seeded, no resample).
    pub fn realise(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> wgpu::Texture {
        self.realise_region(camera, width, height, background, layers, None)
    }

    /// As [`Self::realise`], but compositing only `region` of the composition
    /// — the Viewer's region of interest (K-362). The returned texture is the
    /// region's size, not the comp's, and every layer lands where it would
    /// have, simply windowed.
    ///
    /// **A region is refused, and the whole frame composited, when the draw
    /// list holds an adjustment layer or a motion-blurring layer.** Both stage
    /// through a comp-sized intermediate whose maths is written against the
    /// comp raster, and a half-applied window there would be a wrong picture
    /// rather than a slow one. The caller crops the result instead, so what it
    /// gets back is the same picture either way — a region never changes the
    /// image, only how much of it is computed. `realise` itself is the
    /// no-region entry, which is also what the nested-comp recursion below
    /// calls: a Precomp renders itself entire, and this comp's window applies
    /// once, to the finished thing.
    pub fn realise_region(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        region: Option<lumit_gpu::Region>,
    ) -> wgpu::Texture {
        let region = region.filter(|_| region_is_safe(layers));
        // Depth is what tells the profiler which layers are rows of the
        // composition being rendered and which are inside a Precomp (see
        // crate::profile). Paired around the whole walk, recursion included, so
        // a nested comp's layers can never be mistaken for this comp's.
        if let Some(p) = self.profiler {
            p.enter_comp();
        }
        // One command buffer for the whole walk, recursion included. Every pass
        // below records into it and nothing reaches the driver until the
        // outermost `end_frame`, so a frame costs one round trip rather than one
        // per layer and per effect. `begin_frame` nests, which is what lets this
        // sit on the recursive entry point rather than being threaded by hand.
        self.ctx.begin_frame();
        let out = self.realise_at_depth(camera, width, height, background, layers, region);
        self.ctx.end_frame();
        if let Some(p) = self.profiler {
            p.leave_comp();
        }
        out
    }

    fn realise_at_depth(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        region: Option<lumit_gpu::Region>,
    ) -> wgpu::Texture {
        // The actual raster this comp's composites land on; all geometry
        // below stays in the logical `width`×`height` comp pixels.
        let (tw, th) = lumit_gpu::scaled_size(width, height, self.render_scale);
        let mut acc: Option<wgpu::Texture> = None;
        let mut start = 0usize;
        for (i, l) in layers.iter().enumerate() {
            if !matches!(l.source, DrawSource::Adjust) {
                continue;
            }
            let below = self.realise_segment(
                camera,
                width,
                height,
                background,
                &layers[start..i],
                &acc,
                region,
            );
            // An adjustment layer processes the composite below, which has no
            // footage neighbour frames — temporal effects on an adjustment
            // layer are a later refinement, so no neighbours here. Its LUT and
            // depth-of-field effects still apply (§3.11, §3.22): load/render
            // them the same way the per-layer path does, so preview stays
            // identical to export (K-031). The adjustment stack runs on the
            // comp-sized composite, so its depth inputs resample to comp size.
            let luts = self.load_luts(&l.lut_files);
            let layer_inputs = self.render_layer_inputs(&l.dof_inputs, tw, th);
            let mattes = self.render_layer_inputs(&l.mattes, tw, th);
            let flare_lens = self.load_flare_lens(&l.flare_lens_files);
            // The stack was resolved against the comp raster; this render
            // target may be smaller (reduced-resolution preview), and every
            // px-dimensioned parameter must shrink with it or the flare's
            // light (and every aperture and radius) lands past where the
            // user put it (K-266).
            let fx_ops = match l.fx_ref_width {
                Some(ref_w) if ref_w > 0.0 => {
                    let mut ops = l.fx.clone();
                    ops.rescale_spatial(tw as f32 / ref_w);
                    ops
                }
                _ => l.fx.clone(),
            };
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
            // An adjustment layer's own cost starts here: everything below it
            // has been composited (and timed as its own layers), and what
            // follows — the held or accumulated below-stack, this stack, the
            // coverage and the blend — is what this row is spending.
            let (started, mut fx_ms) = self.layer_clock();
            let fx_input = if let Some(ab) = &l.accumulation_below {
                self.accumulate_below(width, height, background, ab, &below)
            } else if let Some(tb) = &l.temporal_below {
                self.realise(tb.camera, width, height, background, &tb.draws)
            } else {
                below.clone()
            };
            // The adjustment's own stack, coverage and blend all run on the
            // ACTUAL raster: `adjust_blend` reads its three inputs texel by
            // texel, so they must agree on their size.
            let processed = crate::fxops::run_ops(
                self.fx,
                &self.ctx,
                fx_input,
                tw,
                th,
                &fx_ops,
                &[],
                None,
                &luts,
                &layer_inputs,
                &flare_lens,
                &mattes,
                &l.mask_paths,
                fx_ms.as_mut(),
            );
            let coverage = self.coverage_texture(camera, width, height, l);
            acc = Some(self.fx.adjust_blend(
                &self.ctx,
                &below,
                &processed,
                &coverage,
                tw,
                th,
                (l.opacity / 100.0).clamp(0.0, 1.0),
            ));
            self.layer_done(l, started, fx_ms);
            start = i + 1;
        }
        self.realise_segment(
            camera,
            width,
            height,
            background,
            &layers[start..],
            &acc,
            region,
        )
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
        below: &wgpu::Texture,
    ) -> wgpu::Texture {
        let frames: Vec<wgpu::Texture> = ab
            .samples
            .iter()
            .map(|(draws, camera)| self.realise(*camera, width, height, background, draws))
            .collect();
        if frames.is_empty() {
            // No samples (N < 2) degrades to the plain below — never a panic.
            return below.clone();
        }
        // The sub-frames and `below` are all at the ACTUAL raster size; the
        // combine is a full-frame identity pass, so it runs at that size too.
        let (tw, th) = lumit_gpu::scaled_size(width, height, self.render_scale);
        // Equal weights 1/N sum to 1: the premultiplied arithmetic mean.
        let weight = 1.0 / frames.len() as f32;
        let avg_layers: Vec<(&wgpu::Texture, f32)> = frames.iter().map(|f| (f, weight)).collect();
        let average = self.compositor.accumulate(&self.ctx, tw, th, &avg_layers);
        if ab.mix >= 1.0 {
            average
        } else {
            // Mix blends the blurred average against the live below-composite.
            self.compositor.accumulate(
                &self.ctx,
                tw,
                th,
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
    ) -> wgpu::Texture {
        let white = [255u8, 255, 255, 255];
        let (rgba, w, h): (&[u8], u32, u32) = match &l.mask_cov {
            Some((rgba, w, h)) => (rgba, *w, *h),
            None => (&white, 1, 1),
        };
        let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
        let linear = self.engine.linearise(&self.ctx, &src);
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // Rendered at the render scale: `adjust_blend` reads coverage texel by
        // texel against the below/processed rasters, so they must match.
        self.compositor.composite_seeded(
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
            None,
            self.render_scale,
            self.samples,
            // Coverage is only asked for by an adjustment layer, and a region
            // is refused whenever the comp has one — see `realise_region`.
            None,
        )
    }

    // One more argument than clippy likes. The alternative is a struct that
    // exists only to be unpacked at the single call site.
    #[allow(clippy::too_many_arguments)]
    /// Composite one adjustment-free run of draws; `seed` (a previous
    /// stage's output) replaces the cleared background when present.
    fn realise_segment(
        &self,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
        seed: &Option<wgpu::Texture>,
        region: Option<lumit_gpu::Region>,
    ) -> wgpu::Texture {
        let mut linear_textures: Vec<wgpu::Texture> = Vec::with_capacity(layers.len());
        for l in layers {
            // One row's own cost: its source uploaded and linearised (or its
            // Precomp realised entire) and then its effect stack. The composite
            // that follows is one pass over the whole segment rather than a
            // per-layer act, so it lands in the frame total instead
            // (crate::profile).
            let (started, mut fx_ms) = self.layer_clock();
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
            let mut tex = if l.fx.is_empty() {
                tex
            } else {
                let (w, h) = (tex.width(), tex.height());
                // Neighbour source frames a temporal effect (echo) reads;
                // empty for a plain stack, so this uploads nothing then.
                let neighbours: Vec<(i32, wgpu::Texture)> = l
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
                // `lut` ops (§3.11); the same load export uses (K-031).
                let luts = self.load_luts(&l.lut_files);
                // The layer inputs — a depth pass, a Light wrap background —
                // resampled to this layer's working raster (w, h), 1:1 with the
                // stack's consuming ops (§3.22, §3.28); the same render export
                // runs (K-031).
                let layer_inputs = self.render_layer_inputs(&l.dof_inputs, w, h);
                let mattes = self.render_layer_inputs(&l.mattes, w, h);
                let flare_lens = self.load_flare_lens(&l.flare_lens_files);
                // A stack resolved against a raster wider than the one it is
                // about to run on (a Precomp layer's, under reduced-resolution
                // preview) has its px@comp parameters rescaled to this raster,
                // the same correction the adjustment path applies (K-266,
                // K-268). `None` — every other kind — resolved at its own
                // decode scale already, so nothing moves.
                let fx_ops = match l.fx_ref_width {
                    Some(ref_w) if ref_w > 0.0 => {
                        let mut ops = l.fx.clone();
                        ops.rescale_spatial(w as f32 / ref_w);
                        ops
                    }
                    _ => l.fx.clone(),
                };
                crate::fxops::run_ops(
                    self.fx,
                    &self.ctx,
                    tex,
                    w,
                    h,
                    &fx_ops,
                    &neighbours,
                    flow.as_ref(),
                    &luts,
                    &layer_inputs,
                    &flare_lens,
                    &mattes,
                    &l.mask_paths,
                    fx_ms.as_mut(),
                )
            };
            // The lighting pass (docs/06, K-361): shade the finished layer
            // with the comp's Light layers, after its effects and before it is
            // placed. `l.lights` is empty unless the comp holds lights and
            // this layer accepts them, and empty means the pass never runs —
            // so a comp without lights renders byte-for-byte as before.
            if let Some(op) = lighting_op(l, tex.width(), tex.height()) {
                tex = self
                    .fx
                    .lighting(&self.ctx, &tex, tex.width(), tex.height(), &op);
            }
            self.layer_done(l, started, fx_ms);
            linear_textures.push(tex);
        }
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // Per-layer motion blur (docs/06 §4, K-120): a blurring layer's
        // fx-processed texture is drawn at each sub-frame placement and
        // averaged into one comp-sized smear by the shared helper both preview
        // and export call (K-031). The layer's real blend/opacity/matte/mask
        // then apply once to the averaged image, at the 1:1 composite below.
        let mb_textures: Vec<Option<wgpu::Texture>> = linear_textures
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
                        self.render_scale,
                        self.samples,
                    )
                })
            })
            .collect();
        // Layer-space mask textures (Precomp masks — GPU mask pass).
        let mask_textures: Vec<Option<wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.mask_cov
                    .as_ref()
                    .map(|(rgba, w, h)| self.engine.upload_srgb8(&self.ctx, rgba, *w, *h))
            })
            .collect();
        // Matte layers render alone into comp space (one texture per consumer;
        // the shared-matte cache optimisation arrives with the evaluator).
        // Deliberately at FULL comp resolution whatever the render scale: the
        // fragment samples the matte by normalised comp UV, so any size is
        // correct — shrink it later if it ever shows in a profile.
        let matte_textures: Vec<Option<wgpu::Texture>> = layers
            .iter()
            .map(|l| {
                l.matte.as_ref().map(|m| {
                    // A Precomp matte realises its nested comp exactly as a
                    // Precomp layer's picture does (K-268, the K-266 layer-input
                    // shape); anything else is the uploaded source pixels.
                    let linear = if let Some(n) = &m.nested {
                        self.realise(n.camera, n.width, n.height, n.background, &n.draws)
                    } else {
                        let src = self
                            .engine
                            .upload_srgb8(&self.ctx, &m.rgba, m.tex_w, m.tex_h);
                        self.engine.linearise(&self.ctx, &src)
                    };
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
                            &[],
                            &[],
                            // As above: a referenced layer's own stack walks no
                            // mask path in v1 (K-408).
                            &[],
                            // A matte's own stack is part of the layer it
                            // gates, not a row of its own.
                            None,
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
            self.render_scale,
            self.samples,
            region,
        )
    }
}

/// The lighting pass's parameters for one draw (docs/06, K-361), or `None`
/// when nothing lights it — which is the common case and the reason a comp
/// without Light layers pays nothing.
///
/// The layer's plane is described to the kernel as an affine frame rather than
/// a matrix: where texel (0, 0) sits in comp pixels, and how far one texel of
/// movement carries you in each direction. That is all a placement is (the
/// camera's perspective comes later, when the lit layer is projected), and it
/// works out the same whether the layer is being rendered at full resolution
/// or a quarter of it, because the steps are measured against the raster this
/// pass is actually running on.
fn lighting_op(
    l: &crate::draw::CompLayerDraw,
    w: u32,
    h: u32,
) -> Option<lumit_gpu::fx::LightingOp> {
    use crate::build::{place_dir, place_point, unit};

    if l.lights.is_empty() || w == 0 || h == 0 {
        return None;
    }
    // A 2D layer is drawn flat at z = 0 whatever its transform says, because
    // no camera reads those fields. Shading it at its stored depth would light
    // a layer that is not where the light was told it is.
    let (z, rx, ry) = if l.three_d {
        (l.z, l.rotation_x_deg, l.rotation_y_deg)
    } else {
        (0.0, 0.0, 0.0)
    };
    let mut place =
        lumit_gpu::place_matrix(l.position, l.anchor, l.scale, l.rotation_deg, z, rx, ry);
    if let Some(pre) = &l.pre {
        place = lumit_gpu::concat_place(*pre, place);
    }
    let (nw, nh) = l.natural_size;
    let origin = place_point(&place, 0.0, 0.0, 0.0);
    let du = place_dir(&place, nw / w as f32, 0.0, 0.0);
    let dv = place_dir(&place, 0.0, nh / h as f32, 0.0);
    Some(lumit_gpu::fx::LightingOp {
        origin,
        du,
        dv,
        // Wound so an untransformed layer faces the viewer: this model puts
        // increasing z away from the camera, as After Effects does, so the
        // side a light has to be on is the negative one.
        normal: unit(cross(dv, du)),
        lights: l.lights.clone(),
    })
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Whether a region of interest can be applied to this draw list (K-362), or
/// whether the whole frame has to be composited and cropped instead.
///
/// Two stagings are written against a comp-sized intermediate: an **adjustment
/// layer**, whose effect stack runs on the composite of everything below it,
/// and a **motion-blurring layer**, whose sub-frame copies are averaged into a
/// comp-sized texture first. Windowing either of them halfway would produce a
/// wrong picture rather than a fast one, so the region steps aside and the
/// caller crops. Nothing about the image changes — only how much of it was
/// computed to get there.
fn region_is_safe(layers: &[CompLayerDraw]) -> bool {
    !layers
        .iter()
        .any(|l| matches!(l.source, DrawSource::Adjust) || !l.mb.is_empty())
}

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
//! viewport, in Flutter, and in the exported file.

use crate::draw::{AccumulationBelow, CompLayerDraw, DrawSource, LayerInputDraw};

/// The GPU primitives that turn a comp draw list into a linear texture,
/// borrowed from whichever owner is compositing — a frontend's viewer, the
/// headless renderer, or the export renderer. Factoring the realise logic behind
/// one borrowed handle is what lets preview and export share a single re-render
/// path (`render_below_at`, docs/impl/temporal-rerender.md): all of them drive
/// the identical compositor, so a comp realises the same in the viewport and
/// the file.
pub struct Realiser<'a> {
    /// Owned handle (a cheap Arc-backed clone via [`lumit_gpu::GpuContext::
    /// from_parts`]) so the realise code can keep passing `&self.ctx`; the
    /// engines below cannot be cloned, so they stay borrowed.
    pub ctx: lumit_gpu::GpuContext,
    pub engine: &'a lumit_gpu::ColourEngine,
    pub compositor: &'a lumit_gpu::Compositor,
    pub fx: &'a lumit_gpu::fx::FxEngine,
    pub lut_cache: &'a std::cell::RefCell<crate::fxops::LutCache>,
    /// The per-effect intermediate cache, held by the same owner as
    /// the LUT cache and for the same reason: it outlives a frame. Every
    /// layer's stack reads from it; only committed renders add to it (see
    /// [`crate::fxops::FxCache::keep_outputs`]).
    pub fx_cache: &'a std::cell::RefCell<crate::fxops::FxCache>,
    /// The preview render scale: every composite this
    /// walk performs allocates its target at [`lumit_gpu::scaled_size`] of the
    /// comp dims while all geometry stays in logical comp pixels. A field
    /// rather than a parameter so the nested/below/adjustment recursions
    /// inherit it with no signature ripple. Export always builds with 1.0, so
    /// the preview == export identity is untouched at full scale.
    pub render_scale: f32,
    /// The project's anti-aliasing sample count (docs/impl/anti-aliasing.md):
    /// how many coverage samples per pixel the composite is drawn with. 1 is
    /// the picture Lumit made before the setting existed. A field beside
    /// [`Self::render_scale`], for the same reason — the nested, below and
    /// adjustment recursions inherit it with no signature ripple — but **not**
    /// like it in one way that matters: `render_scale` differs between preview
    /// and export by design, and this must not. Both paths read the same
    /// project field, which is what keeps the preview-equals-export identity
    /// true with anti-aliasing on.
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
    /// The footage input transforms this frame may need, already baked and
    /// uploaded (docs/impl/ocio.md §5.2). `None` — no config, an unusable
    /// one, or any of the test builders — means every source linearises
    /// through the built-in interpretation, which is what this always did.
    ///
    /// Every source of **image content** reads through it: a layer's own pixels
    /// ([`DrawSource::Pixels`]), a matte source's ([`crate::draw::MatteDraw`]),
    /// and a layer input's — a Light wrap background plate, a Texturize texture
    /// ([`crate::draw::DofInputDraw`]). A mask's coverage raster is not image
    /// content: it is a shape's alpha, drawn by Lumit rather than decoded from
    /// anybody's camera, so it has no colour space to be interpreted through and
    /// takes the built-in path deliberately.
    pub colour_inputs: Option<&'a crate::colour::InputTransforms>,
    /// The project's OCIO config, loaded and usable, for the OCIO effects'
    /// tables (docs/08 §3.97). `None` - no config, an unusable one, or any of
    /// the test builders - leaves the three config-bound OCIO effects as
    /// passthroughs; the file transform reads its file either way.
    pub colour_config: Option<&'a std::sync::Arc<crate::colour::Loaded>>,
    /// The flow backend a composite measurement runs on (docs/08 §3.2),
    /// held by the render's owner beside the caches above and for the same
    /// reason: the solver's plan is worth keeping between frames. `None` — a
    /// caller that never asks for one, and every builder in the tests that has
    /// no use for it — means a Motion blur or Datamosh on an adjustment or
    /// Precomp layer degrades to the passthrough it was before this existed,
    /// which is exactly what an unavailable GPU flow does too.
    pub flow: Option<&'a std::cell::RefCell<CompositeFlow>>,
}

/// The flow backend for measuring a **composite's** motion (docs/08 §3.2),
/// built on the render's own device the first time a layer actually asks for
/// one.
///
/// # In plain terms
///
/// Measuring motion means comparing two pictures, and building the flow solver
/// compiles a dozen shaders. A project with no Motion blur and no Datamosh
/// on an adjustment or Precomp layer never asks, so it never pays: this starts
/// empty and fills itself in on the first question.
#[derive(Default)]
pub struct CompositeFlow {
    engine: Option<lumit_flow::FlowEngine>,
}

impl Realiser<'_> {
    /// The baked input transform for one colour space name, or `None` for the
    /// built-in interpretation — which is the answer for an unassigned item, a
    /// project with no config, a config that is not usable, and a name the
    /// loaded config does not have.
    fn input_transform(&self, space: &Option<String>) -> Option<&lumit_gpu::OcioArtefact> {
        self.colour_inputs?.get(space.as_deref())
    }

    /// Source pixels to a linear working texture, whichever width they arrived
    /// at. Every picture that comes from a file goes through here — a layer's
    /// own, a matte source's, a depth pass fed to an effect — so there is one
    /// answer to "what happens to a float plate" rather than three.
    ///
    /// **Eight-bit** is uploaded sRGB-encoded and decoded by the linearise
    /// pass, as it always was.
    ///
    /// **Float** is already scene-linear, so with no colour space assigned the
    /// upload *is* the answer and no pass runs at all. With one assigned — an
    /// ACEScg render, say — the same pass runs on the float values as they
    /// stand: an input transform is a colour conversion, not a decode, and
    /// `lumit_gpu::unencoded` leaves a float format alone, which is what makes
    /// the one pass right for both widths.
    fn upload_source(
        &self,
        rgba: &[u8],
        w: u32,
        h: u32,
        format: lumit_media::PixelFormat,
        colour_space: &Option<String>,
    ) -> wgpu::Texture {
        let ocio = self.input_transform(colour_space);
        match (format, ocio) {
            (lumit_media::PixelFormat::LinearF32, None) => {
                self.engine.upload_float_frame(&self.ctx, rgba, w, h)
            }
            (lumit_media::PixelFormat::LinearF32, ocio) => {
                let src = self.engine.upload_float_frame(&self.ctx, rgba, w, h);
                // Into the source's own format, not the narrower working one:
                // the transform must not be where the precision goes.
                let format = src.format();
                self.engine.linearise_into(&self.ctx, &src, ocio, format)
            }
            (lumit_media::PixelFormat::Srgb8, ocio) => {
                let src = self.engine.upload_srgb8(&self.ctx, rgba, w, h);
                self.engine.linearise_through(&self.ctx, &src, ocio)
            }
        }
    }

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
                // stopwatch already declares (docs/13 §7.1: measuring
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
    /// 1:1 with the stack's `lens_flare` ops. A `None`
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

    /// Turn a layer's ordered `colour_tables` into the parallel `tables` list
    /// `run_ops` binds (docs/08 §3.11): each `Some(path)` is parsed and
    /// uploaded once — cached by path *and* last-modified time, bounded and
    /// LRU-evicted (docs/impl/lut.md §4) — and a 1D or unreadable/absent
    /// file yields a `None` slot (a labelled no-op, never a fault). The output
    /// is 1:1 and in order with `files`, so the k-th slot lines up with the
    /// k-th `lut` op.
    fn load_tables(
        &self,
        files: &[Option<crate::colour::TableRequest>],
    ) -> Vec<Option<crate::fxops::ColourTable>> {
        use crate::colour::TableRequest;
        use crate::fxops::ColourTable;
        let mut cache = self.lut_cache.borrow_mut();
        files
            .iter()
            .map(|slot| match slot.as_ref()? {
                TableRequest::Cube(path) => {
                    cache.get_or_load(&self.ctx, path).map(ColourTable::Cube)
                }
                TableRequest::Ocio(request) => cache
                    .get_or_bake(&self.ctx, self.engine, request, self.colour_config)
                    .map(ColourTable::Ocio),
            })
            .collect()
    }

    /// Render a layer's layer-input slots (docs/impl/layer-input.md §2) — the
    /// depth passes of its `dof` effects, the matte sources of its Lens
    /// flares. Each [`LayerInputDraw::Layer`] (the referenced layer's source
    /// pixels) is uploaded, linearised and resampled into the effect's
    /// working raster `(w, h)` through the shared
    /// [`crate::fxops::render_layer_input`], so the parallel list handed to
    /// `run_ops` is 1:1 with the stack's ops and aligned with the layer
    /// texture the kernel reads. Export renders these identically.
    ///
    /// [`LayerInputDraw::ThisLayer`] renders nothing here: it names
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
                // Precomp layer's picture does; anything else is
                // the uploaded source pixels.
                let linear = if let Some(n) = &d.nested {
                    self.realise_nested(n.key, n.camera, n.width, n.height, n.background, &n.draws)
                } else {
                    // Through the referenced layer's own colour space:
                    // a background plate is a picture like any other, and it is
                    // read as what it is rather than as what the layer reading
                    // it happens to be.
                    self.upload_source(&d.rgba, d.tex_w, d.tex_h, d.format, &d.colour_space)
                };
                // Effects-and-masks depth: run the depth layer's own
                // stack on its texture before it is resampled, when the consumer's
                // depth source is Effects and masks (`d.fx` non-empty). Temporal
                // inputs stay empty in v1 (same boundary as the matte). Export
                // does the same, so the two depth passes match.
                let linear = if d.fx.is_empty() {
                    linear
                } else {
                    let tables = self.load_tables(&d.colour_tables);
                    crate::fxops::run_ops(
                        self.fx,
                        &self.ctx,
                        linear,
                        d.tex_w,
                        d.tex_h,
                        &d.fx,
                        &[],
                        &[],
                        &tables,
                        &[],
                        &[],
                        &[],
                        // No mask paths through a referenced layer's own stack
                        // — the same v1 boundary its mattes take.
                        &[],
                        // And no birth schedules: a Particulate inside a
                        // referenced layer's own stack takes the same v1
                        // boundary, and an unscheduled op passes its picture
                        // through.
                        &[],
                        // A matte's own stack is part of the effect that reads
                        // it, not a row of its own: its cost is inside that
                        // layer's span already.
                        None,
                        // Nothing names a referenced layer's picture in v1,
                        // so its stack runs uncached.
                        None,
                    )
                };
                // Same boundary as the matte's: the referenced layer
                // is placed by `d.tex_w × d.tex_h` just below, so its own stack
                // is cropped back to that raster rather than growing it.
                let linear = lumit_gpu::fx::fit_centred(&self.ctx, linear, d.tex_w, d.tex_h);
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

    /// A nested comp's picture: the texture held under its frame key
    /// when there is one, else [`Self::realise`] and — on a committed render —
    /// file it. The three places a Precomp is realised (a layer, a matte, a
    /// layer input) all come through here, so "the same nested comp used in
    /// five places renders once" (docs/06 §5.2) holds for every one of them.
    /// The texture is read-only downstream — the stack and the composite both
    /// make new textures — so handing the same one out twice is safe.
    pub fn realise_nested(
        &self,
        key: Option<u128>,
        camera: Option<lumit_core::model::CameraPose>,
        width: u32,
        height: u32,
        background: [f64; 4],
        layers: &[CompLayerDraw],
    ) -> wgpu::Texture {
        let Some(key) = key else {
            return self.realise(camera, width, height, background, layers);
        };
        let key = crate::fxops::nested_texture_key(key, self.render_scale, self.samples);
        // A measured walk (docs/13 §7.1) wants a number on every row inside
        // the Precomp too, so it realises the nested comp whether or not the
        // picture is held — and still files what it made.
        if self.profiler.is_none() {
            if let Some(held) = self.fx_cache.borrow_mut().nested(key) {
                return held;
            }
        }
        let made = self.realise(camera, width, height, background, layers);
        self.fx_cache.borrow_mut().put_nested(key, made.clone());
        made
    }

    /// Stamp a Precomp layer's paint strokes into its realised picture.
    ///
    /// Every other kind of layer is painted on the CPU, in the layer's own
    /// 8-bit sRGB raster, before it is uploaded — a Precomp has no such
    /// raster, because its picture is made on the graphics card. So the
    /// picture comes back the way an export reads it (the neutral display
    /// encode, never the Viewer's exposure), goes through the *same*
    /// `apply_strokes` every other layer uses, and is uploaded again. One
    /// rasteriser, one set of rules, and a stroke that lands identically
    /// wherever it is painted.
    ///
    /// ponytail: an 8-bit round trip per painted Precomp per frame — the
    /// nested picture is quantised to the depth every footage layer is
    /// painted at anyway, and read back synchronously. A GPU stamping pass
    /// (docs/impl/paint.md, "Not built") is the upgrade if a painted
    /// Precomp ever shows in a profile; nothing stored changes when it
    /// arrives. Costs exactly nothing on the Precomp layers — almost all of
    /// them — that carry no paint.
    fn paint_over(
        &self,
        tex: wgpu::Texture,
        natural_w: f64,
        natural_h: f64,
        strokes: &[lumit_core::paint::PaintStroke],
        t: f64,
    ) -> wgpu::Texture {
        if strokes.is_empty() {
            return tex;
        }
        let (w, h) = (tex.width(), tex.height());
        let display = self
            .engine
            .display(&self.ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL);
        let Ok(mut rgba) = self.engine.readback8(&self.ctx, &display) else {
            // The read failed (a lost device, a surface gone). The unpainted
            // picture is the calm answer; engine crates do not panic.
            return tex;
        };
        lumit_core::paint::apply_strokes(&mut rgba, w, h, natural_w, natural_h, strokes, t);
        let src = self.engine.upload_srgb8(&self.ctx, &rgba, w, h);
        self.engine.linearise(&self.ctx, &src)
    }

    /// As [`Self::realise`], but compositing only `region` of the composition
    /// — the Viewer's region of interest. The returned texture is the
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
            // identical to export. The adjustment stack runs on the
            // comp-sized composite, so its depth inputs resample to comp size.
            let tables = self.load_tables(&l.colour_tables);
            let layer_inputs = self.render_layer_inputs(&l.dof_inputs, tw, th);
            let mattes = self.render_layer_inputs(&l.mattes, tw, th);
            let flare_lens = self.load_flare_lens(&l.flare_lens_files);
            // The stack was resolved against the comp raster; this render
            // target may be smaller (reduced-resolution preview), and every
            // px-dimensioned parameter must shrink with it or the flare's
            // light (and every aperture and radius) lands past where the
            // user put it.
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
            // texture export's `render_below_at` produces); the coverage
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
            // **The motion of the composite below** (docs/08 §3.2). An
            // adjustment layer has no decoded frames, so its Motion blur
            // and Datamosh used to bind nothing and pass through — on exactly
            // the layer §3.2 calls the commonest place to put the effect. The
            // below-stack was built again at each neighbour time; realise it
            // and measure between the two here, where both are textures.
            // Empty unless one of those effects is live.
            let (flow_neighbours, flow) =
                self.measure_below_flow(l, &fx_input, width, height, background);
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
                &flow_neighbours,
                &flow,
                &tables,
                &layer_inputs,
                &flare_lens,
                &mattes,
                &l.mask_paths,
                &l.points_schedules,
                fx_ms.as_mut(),
                // The composite below carries no name in v1.
                None,
            );
            // An adjustment layer's stack cannot grow the raster: what
            // follows blends it against the composite beneath it, texel for
            // texel, and there is no "beneath" outside the comp to grow into.
            // A Tile above 100 % output on an adjustment layer therefore reads
            // as the plain clipped tiling it always did.
            let processed = lumit_gpu::fx::fit_centred(&self.ctx, processed, tw, th);
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
    /// still scene averages back to `below` bit-for-bit; a moving one smears.
    /// Export runs the identical combine, so the two agree.
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
            .map(|(draws, camera)| {
                let frame = self.realise(*camera, width, height, background, draws);
                // Hand this sample's work to the card and wait for it before
                // starting the next. A frame is one batch, and nothing a
                // batch allocates comes back until it has run: with the N
                // samples inside it, every sample's intermediates stayed
                // alive together. Eight samples over one 1080p layer held
                // 2.1 GB against 200 MB for the frame alone, which is enough
                // to take a card down. Waiting per sample keeps one sample's
                // scratch alive at a time; the finished sample textures are
                // all that accumulate. The wait costs the overlap between
                // encoding a sample and drawing the last, small beside N
                // full renders.
                self.ctx.flush();
                self.ctx.settle();
                frame
            })
            .collect();
        if frames.is_empty() {
            // No samples (N < 2) degrades to the plain below — never a panic.
            return below.clone();
        }
        // The sub-frames and `below` are all at the ACTUAL raster size; the
        // combine is a full-frame identity pass, so it runs at that size too.
        let (tw, th) = lumit_gpu::scaled_size(width, height, self.render_scale);
        // **The Matte scales Shutter angle per pixel**. With none bound
        // — the default, and every project saved before this — the equal-weight
        // hardware pass below runs exactly as it always did, byte for byte.
        // With one, each pixel's weights come from how far open its
        // own shutter is, and the average is taken over a shorter slice of the
        // same N moments, shrunk toward the frame's own instant.
        let matte = self
            .render_layer_inputs(std::slice::from_ref(&ab.matte), tw, th)
            .into_iter()
            .next()
            .and_then(|slot| slot.texture(below).cloned());
        let average = if let Some(matte) = matte {
            // Channel and Invert, once, before anything reads it. The
            // dispatch seam does this for every other effect; this one has no
            // dispatch, so it does it here.
            let matte =
                if lumit_core::fx::cpu::matte_needs_prepare(ab.matte_channel, ab.matte_invert) {
                    self.fx.matte_prepare(
                        &self.ctx,
                        &matte,
                        tw,
                        th,
                        ab.matte_channel,
                        ab.matte_invert,
                    )
                } else {
                    matte
                };
            self.fx
                .accumulate_with_shutter(&self.ctx, &frames, &matte, tw, th, ab.anchor)
        } else {
            // Equal weights 1/N sum to 1: the premultiplied arithmetic mean.
            let weight = 1.0 / frames.len() as f32;
            let avg_layers: Vec<(&wgpu::Texture, f32)> =
                frames.iter().map(|f| (f, weight)).collect();
            self.compositor.accumulate(&self.ctx, tw, th, &avg_layers)
        };
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

    /// **Motion measured between two composites** (docs/08 §3.2): the
    /// picture this layer's effects run on, and the same picture built again at
    /// each neighbour time ([`CompLayerDraw::flow_below`]).
    ///
    /// Returns the neighbour pictures and the fields measured against them, in
    /// the same shape a footage layer's decoded neighbours and fields arrive in
    /// — so nothing downstream knows where they came from. Both halves matter:
    /// Motion blur reads only the field, Datamosh also drags the `-1`
    /// picture along it, and the contract that each consumer gets the
    /// measurement it asked for holds here because the builder emitted one
    /// entry per offset the stack wanted.
    ///
    /// Empty in, empty out — and empty is the common case, so an ordinary
    /// frame does not reach the flow engine at all.
    #[allow(clippy::type_complexity)]
    fn measure_below_flow(
        &self,
        l: &CompLayerDraw,
        now: &wgpu::Texture,
        width: u32,
        height: u32,
        background: [f64; 4],
    ) -> (Vec<(i32, wgpu::Texture)>, Vec<(i32, wgpu::Texture)>) {
        let mut neighbours = Vec::with_capacity(l.flow_below.len());
        let mut fields = Vec::with_capacity(l.flow_below.len());
        for (offset, draws, camera) in &l.flow_below {
            let then = self.realise(*camera, width, height, background, draws);
            if let Some(field) = self.measure_flow(now, &then) {
                fields.push((*offset, field));
            }
            neighbours.push((*offset, then));
        }
        (neighbours, fields)
    }

    /// One measurement, entirely on the card: two pictures in, the `rgba32float`
    /// field the kernels read out (docs/08 §3.2).
    ///
    /// The flow engine's texture entry point converts each picture to the luma
    /// its pyramid starts from with one compute pass, so neither composite is
    /// ever read back — a readback of two 1080p frames costs several times what
    /// the measurement does.
    ///
    /// `None` is always a degrade and never a fault: no backend was given, the
    /// device has no GPU flow, or the solver faulted (in which case the engine
    /// has already given up on the GPU for good). The caller then binds no
    /// field, and an effect with no field is its documented passthrough.
    ///
    /// ponytail: measured per realise, not cached across them. The decode
    /// worker's flow cache keys by *content* — the source item and the two
    /// frames — and a composite has no such name (`fx_input_key` is `None` for
    /// exactly this reason), so filing one under the layer and the time
    /// would go stale the moment somebody edited a layer below. Caching would
    /// also save the smaller half: the neighbour composite is a whole extra comp
    /// render and the flow is ~8 ms on top of it. So the ceiling is a doubled
    /// comp render for every frame that reaches a flow effect above an
    /// adjustment, and the trigger is that doubling measured: docs/13 B3's
    /// scrub p95 on a comp whose flow effect sits on an adjustment layer
    /// against the same effect over footage, where the decode worker's cache
    /// answers instead. If the composite path is the one missing B3's 50 ms
    /// while the footage path holds it, this is the whole of the difference.
    /// The upgrade is to name the below-composite the way a nested comp's
    /// frame is named, and then both halves can be cached under the one name.
    fn measure_flow(&self, a: &wgpu::Texture, b: &wgpu::Texture) -> Option<wgpu::Texture> {
        let cell = self.flow?;
        // The solver reads these two pictures on an encoder of its own, and a
        // command that has not been submitted has not run.
        self.ctx.flush();
        let mut held = cell.borrow_mut();
        let engine = held
            .engine
            .get_or_insert_with(|| lumit_flow::FlowEngine::with_context(&self.ctx));
        // Half resolution, the same working size the decode worker measures an
        // effect's motion at and for its reasons (docs/impl/optical-flow.md §1):
        // a wider patch relative to repeating detail, at a quarter of the cost.
        // Sharing that setting is also what keeps a composite measured the way
        // footage is, rather than by a second rule nobody wrote down.
        let set = lumit_flow::FlowSettings {
            divisor: 2,
            ..lumit_flow::FlowSettings::default()
        };
        let (fwd, bwd) = engine.flow_pair_textures(a, b, &set)?;
        // The forward–backward confidence the streak is steered by (FX-19), and
        // the field brought up to the raster the effect runs on — the same two
        // deterministic functions the decode path applies to a measured pair, so
        // a composite's field and a footage layer's are the same kind of thing.
        let conf = lumit_flow::confidence(&fwd, &bwd);
        let (w, h) = (a.width(), a.height());
        let (u, v, c) = lumit_flow::field_to_size(&fwd, &conf, w as usize, h as usize);
        Some(lumit_gpu::fx::upload_flow_field(
            &self.ctx, &u, &v, &c, w, h,
        ))
    }

    /// The adjustment layer's comp-space coverage (docs/06 §1.5): its mask
    /// raster — white where the effects apply — placed by its transform,
    /// so the transform moves the coverage map, never the picture. No
    /// masks means full coverage (a white quad over the whole comp).
    /// One matte source rendered alone into comp space.
    ///
    /// Deliberately at FULL comp resolution whatever the render scale: the
    /// fragment samples the matte by normalised comp UV, so any size is
    /// correct — shrink it later if it ever shows in a profile.
    ///
    /// Both places a matte gates something come through here — an ordinary
    /// layer's picture, and an adjustment layer's coverage — so the two can
    /// never disagree about what a matte source looks like.
    fn matte_texture(
        &self,
        width: u32,
        height: u32,
        cam_mat: Option<lumit_gpu::Mat4>,
        m: &crate::draw::MatteDraw,
    ) -> wgpu::Texture {
        // A Precomp matte realises its nested comp exactly as a Precomp
        // layer's picture does (the layer-input shape); anything else is the
        // uploaded source pixels.
        let linear = if let Some(n) = &m.nested {
            self.realise_nested(n.key, n.camera, n.width, n.height, n.background, &n.draws)
        } else {
            // Through the matte source's own colour space,
            // as its own picture would be: log footage read as
            // sRGB gates by the wrong shape, and a matte is
            // nothing but a shape.
            self.upload_source(&m.rgba, m.tex_w, m.tex_h, m.format, &m.colour_space)
        };
        // After-effects matte: run the matte source's own
        // stack on its texture before it gates the consumer, so a keyed
        // or blurred matte works. Temporal inputs stay empty in v1 — the
        // matte source's echo/flow degrades to a still (documented). The
        // same run export performs, so the two agree.
        let linear = if m.fx.is_empty() {
            linear
        } else {
            let tables = self.load_tables(&m.colour_tables);
            crate::fxops::run_ops(
                self.fx,
                &self.ctx,
                linear,
                m.tex_w,
                m.tex_h,
                &m.fx,
                &[],
                &[],
                &tables,
                &[],
                &[],
                &[],
                // As above: a referenced layer's own stack walks no
                // mask path, and carries no birth schedule, in v1.
                &[],
                &[],
                // A matte's own stack is part of the layer it
                // gates, not a row of its own.
                None,
                // As above: unnamed in v1, so uncached.
                None,
            )
        };
        // A matte source's own stack cannot grow the raster
        // either: it is placed by `m.natural_size` below,
        // and a matte is a shape, not a picture that reaches
        // further. Cropped back to the size it was placed at.
        let linear = lumit_gpu::fx::fit_centred(&self.ctx, linear, m.tex_w, m.tex_h);
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
    }

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
        // **The adjustment layer's own Matte**. An adjustment has no
        // picture for a matte to gate — what it has is coverage, which is
        // exactly where the effect lands, so the matte multiplies into that.
        // The result reads the way it does on any other layer: the stack
        // applies where the matte says and nowhere else. `None` on every
        // adjustment without one, which composites as it always did.
        let matte_tex = l
            .matte
            .as_ref()
            .map(|m| self.matte_texture(width, height, cam_mat, m));
        let matte = matte_tex.as_ref().map(|mt| lumit_gpu::MatteInput {
            texture: mt,
            luma: l.matte.as_ref().is_some_and(|m| m.luma),
            inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
        });
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
                matte,
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
        // How much wider and taller each layer's finished picture is than the
        // source it started from. `(1.0, 1.0)` for every layer whose
        // stack holds no Tile above 100 % output, which is every layer until one
        // does — and multiplying a placement by 1.0 moves nothing, so the
        // composite below is byte for byte what it was.
        let mut grown: Vec<(f32, f32)> = Vec::with_capacity(layers.len());
        for l in layers {
            // One row's own cost: its source uploaded and linearised (or its
            // Precomp realised entire) and then its effect stack. The composite
            // that follows is one pass over the whole segment rather than a
            // per-layer act, so it lands in the frame total instead
            // (crate::profile).
            let (started, mut fx_ms) = self.layer_clock();
            let tex = match &l.source {
                DrawSource::Pixels {
                    rgba,
                    tex_w,
                    tex_h,
                    format,
                    colour_space,
                } => self.upload_source(rgba, *tex_w, *tex_h, *format, colour_space),
                DrawSource::Nested {
                    width,
                    height,
                    background,
                    draws,
                    camera,
                    key,
                    paint,
                    paint_time,
                } => {
                    let nested =
                        self.realise_nested(*key, *camera, *width, *height, *background, draws);
                    self.paint_over(
                        nested,
                        f64::from(*width),
                        f64::from(*height),
                        paint,
                        *paint_time,
                    )
                }
                DrawSource::Adjust => {
                    // realise splits segments at every Adjust draw, so none
                    // reaches here; a transparent texel keeps the no-panic
                    // rule (and draws nothing) if that ever regresses.
                    let src = self.engine.upload_srgb8(&self.ctx, &[0, 0, 0, 0], 1, 1);
                    self.engine.linearise(&self.ctx, &src)
                }
            };
            // The raster the stack starts from, kept because one effect can
            // grow it and the composite must then place a picture wider
            // than the layer it came from.
            let (source_w, source_h) = (tex.width(), tex.height());
            // The effect stack runs on the linear source, after masks and
            // before the transform (docs/08 §1.5; docs/06 render order).
            let mut tex = if l.fx.is_empty() {
                tex
            } else {
                let (w, h) = (tex.width(), tex.height());
                // Neighbour source frames a temporal effect (echo) reads;
                // empty for a plain stack, so this uploads nothing then.
                // **A Precomp measures its own motion** (docs/08 §3.2).
                // A comp has no decoded frames either, so a Motion blur or
                // Datamosh on a Precomp layer bound nothing and passed through;
                // the nested picture was built again at each neighbour time, and
                // the pair is compared here. Every other kind takes the decoded
                // route below, unchanged.
                let (neighbours, flow) = match &l.source {
                    DrawSource::Nested {
                        width,
                        height,
                        background,
                        ..
                    } if !l.flow_below.is_empty() => {
                        self.measure_below_flow(l, &tex, *width, *height, *background)
                    }
                    _ => {
                        let neighbours: Vec<(i32, wgpu::Texture)> = l
                            .neighbours
                            .iter()
                            .map(|(offset, rgba, nw, nh)| {
                                let src = self.engine.upload_srgb8(&self.ctx, rgba, *nw, *nh);
                                (*offset, self.engine.linearise(&self.ctx, &src))
                            })
                            .collect();
                        // The dense motion fields, one per offset a
                        // flow-consuming effect asked for, each uploaded
                        // as its own texture (only when it matches the layer's
                        // raster). The confidence rides in the .z channel
                        // (FX-19).
                        let flow: Vec<(i32, wgpu::Texture)> = l
                            .flow_fields
                            .iter()
                            .filter(|(_, _, _, _, fw, fh)| *fw == w && *fh == h)
                            .map(|(offset, u, v, conf, _, _)| {
                                (
                                    *offset,
                                    lumit_gpu::fx::upload_flow_field(&self.ctx, u, v, conf, w, h),
                                )
                            })
                            .collect();
                        (neighbours, flow)
                    }
                };
                // The parsed-and-uploaded `.cube` LUTs, 1:1 with the stack's
                // `lut` ops (§3.11); the same load export uses.
                let tables = self.load_tables(&l.colour_tables);
                // The layer inputs — a depth pass, a Light wrap background —
                // resampled to this layer's working raster (w, h), 1:1 with the
                // stack's consuming ops (§3.22, §3.28); the same render export
                // runs.
                let layer_inputs = self.render_layer_inputs(&l.dof_inputs, w, h);
                let mattes = self.render_layer_inputs(&l.mattes, w, h);
                let flare_lens = self.load_flare_lens(&l.flare_lens_files);
                // A stack resolved against a raster wider than the one it is
                // about to run on (a Precomp layer's, under reduced-resolution
                // preview) has its px@comp parameters rescaled to this raster,
                // the same correction the adjustment path applies. `None` —
                // every other kind — resolved at its own decode scale already,
                // so nothing moves.
                let fx_ops = match l.fx_ref_width {
                    Some(ref_w) if ref_w > 0.0 => {
                        let mut ops = l.fx.clone();
                        ops.rescale_spatial(w as f32 / ref_w);
                        ops
                    }
                    _ => l.fx.clone(),
                };
                // The propagated mattes, uploaded once each at the source's
                // own raster. `fit_centred` inside the walk takes them
                // to the working raster, which is where every differently-sized
                // input is fitted. Empty on every layer with no Roto brush,
                // which costs nothing at all.
                let roto_mattes: Vec<Option<wgpu::Texture>> = l
                    .roto_mattes
                    .iter()
                    .map(|slot| slot.as_ref().map(|m| upload_roto_matte(&self.ctx, m)))
                    .collect();
                crate::fxops::run_ops_with_roto(
                    self.fx,
                    &self.ctx,
                    tex,
                    w,
                    h,
                    &fx_ops,
                    &neighbours,
                    &flow,
                    &tables,
                    &layer_inputs,
                    &flare_lens,
                    &mattes,
                    &l.mask_paths,
                    &l.points_schedules,
                    &roto_mattes,
                    fx_ms.as_mut(),
                    // The per-effect cache, for a source the builder
                    // could name; a nested comp, text or shape runs as before.
                    l.fx_input_key.map(|key| (self.fx_cache, key)),
                )
            };
            // The lighting pass (docs/06): shade the finished layer
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
            grown.push((
                tex.width() as f32 / source_w as f32,
                tex.height() as f32 / source_h as f32,
            ));
            linear_textures.push(tex);
        }
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
        // Per-layer motion blur (docs/06 §4): a blurring layer's
        // fx-processed texture is drawn at each sub-frame placement and
        // averaged into one comp-sized smear by the shared helper both preview
        // and export call. The layer's real blend/opacity/matte/mask
        // then apply once to the averaged image, at the 1:1 composite below.
        let mb_textures: Vec<Option<wgpu::Texture>> = linear_textures
            .iter()
            .zip(layers)
            .zip(&grown)
            .map(|((tex, l), g)| {
                (!l.mb.is_empty()).then(|| {
                    // A grown picture is placed by the grown transform at every
                    // sub-frame too, or the smear would be of a layer
                    // in a different place from the one the composite draws.
                    let mb: Vec<lumit_gpu::MbSample> = if *g == (1.0, 1.0) {
                        Vec::new()
                    } else {
                        let (dx, dy) = grow_offset(l.natural_size, *g);
                        l.mb.iter()
                            .map(|smp| lumit_gpu::MbSample {
                                anchor: (smp.anchor.0 + dx, smp.anchor.1 + dy),
                                ..*smp
                            })
                            .collect()
                    };
                    self.compositor.motion_blur_average(
                        &self.ctx,
                        width,
                        height,
                        tex,
                        grow_size(l.natural_size, *g),
                        if mb.is_empty() { &l.mb } else { &mb },
                        l.three_d,
                        l.pre,
                        cam_mat,
                        self.render_scale,
                        self.samples,
                    )
                })
            })
            .collect();
        // Layer-space mask textures (Precomp masks — GPU mask pass). No colour
        // space is applied to one and none ever will be: this raster is a
        // shape's coverage, drawn here from the mask's own geometry, not image
        // content that arrived in somebody's colour space.
        let mask_textures: Vec<Option<wgpu::Texture>> = layers
            .iter()
            .zip(&grown)
            .map(|(l, g)| {
                l.mask_cov.as_ref().map(|(rgba, w, h)| {
                    // A grown picture is placed over a wider quad, and
                    // the mask is sampled across that quad — so a mask left at
                    // the layer's own size would stretch, and its edge would
                    // move. Grown into the same margin with nothing in it: the
                    // copies Tile put outside the layer are outside the mask,
                    // which is what a mask means.
                    let (mw, mh) = (
                        ((*w as f32 * g.0).round() as u32).max(*w),
                        ((*h as f32 * g.1).round() as u32).max(*h),
                    );
                    let src = self.engine.upload_srgb8(&self.ctx, rgba, *w, *h);
                    if (mw, mh) == (*w, *h) {
                        src
                    } else {
                        let linear = self.engine.linearise(&self.ctx, &src);
                        lumit_gpu::fx::fit_centred(&self.ctx, linear, mw, mh)
                    }
                })
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
                l.matte
                    .as_ref()
                    .map(|m| self.matte_texture(width, height, cam_mat, m))
            })
            .collect();
        let comp_layers: Vec<lumit_gpu::CompositeLayer> = linear_textures
            .iter()
            .zip(layers)
            .zip(&matte_textures)
            .zip(&mask_textures)
            .zip(&mb_textures)
            .zip(&grown)
            .map(|(((((texture, l), matte_tex), mask_tex), mb_tex), g)| {
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
                        // A picture its stack grew covers more of layer
                        // space than the layer's own rectangle, evenly on all
                        // four sides — so the quad is that much bigger and the
                        // anchor moves with it, which leaves every pixel of the
                        // original exactly where it was.
                        size: grow_size(l.natural_size, *g),
                        position: l.position,
                        anchor: {
                            let (dx, dy) = grow_offset(l.natural_size, *g);
                            (l.anchor.0 + dx, l.anchor.1 + dy)
                        },
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

/// The quad a grown picture needs (docs/08 §3.39).
///
/// **In plain terms.** Tile with Output width or height above 100 % hands back a
/// picture wider than the layer it came from, grown evenly on all four sides.
/// The layer's placement is expressed as a rectangle (`size`) with a pin in it
/// (`anchor`); to draw the wider picture in the same place, the rectangle grows
/// by the same factor and the pin slides to stay over the same pixel. Every
/// layer whose stack grew nothing has a factor of exactly 1, and both functions
/// then return what they were given, to the bit.
fn grow_size(natural: (f32, f32), grow: (f32, f32)) -> (f32, f32) {
    (natural.0 * grow.0, natural.1 * grow.1)
}

/// How far the pin slides: half the width the growth added, on each axis.
/// See [`grow_size`].
fn grow_offset(natural: (f32, f32), grow: (f32, f32)) -> (f32, f32) {
    (
        natural.0 * (grow.0 - 1.0) * 0.5,
        natural.1 * (grow.1 - 1.0) * 0.5,
    )
}

/// The lighting pass's parameters for one draw (docs/06), or `None`
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

/// Whether a region of interest can be applied to this draw list, or
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

/// One propagated matte on the card: a gray8 plane at the source's own
/// raster, uploaded as the working linear format so it goes through the same
/// sampler every other auxiliary picture does.
///
/// Grey into all four channels. `set_matte` reads the luminance of whichever
/// channel it is told to, so the alpha carries the same number as the colour and
/// the plane reads correctly whichever way it is asked. The values are the
/// matte's own 0..1 and are **not** colour, so no transfer function is applied
/// to them — a coverage is a weight, not a picture.
/// A gray8 byte goes straight to the fp16 bits the texture holds, through a
/// 256-entry table — the same numbers the f32 route produced, without the
/// full-plane f32 temporary in between.
///
// ponytail: uploaded fresh every rendered frame. An unchanged matte — the
// common case, since a propagated run does not move while it is being watched
// — re-uploads the whole plane per frame per Roto layer. Memoising the texture
// per (effect instance, source frame) is the upgrade, and it belongs beside
// `LutCache`/`FxCache` on the `Realiser`'s owner rather than in the roto store,
// which is process-wide and knows nothing of a device. Observable trigger: a
// Roto layer's upload showing up in the docs/13 budget traces, or scrubbing a
// 4K propagated shot dropping frames the matte solve is not paying for.
fn upload_roto_matte(ctx: &lumit_gpu::GpuContext, m: &crate::draw::RotoMatteDraw) -> wgpu::Texture {
    let mut table = [0u16; 256];
    for (v, bits) in table.iter_mut().enumerate() {
        *bits = lumit_gpu::fx::f16_bits(v as f32 / 255.0);
    }
    let n = (m.width as usize) * (m.height as usize);
    let mut halfs = Vec::with_capacity(n * 4);
    for i in 0..n {
        let a = table[usize::from(m.gray.get(i).copied().unwrap_or(0))];
        halfs.extend_from_slice(&[a, a, a, a]);
    }
    lumit_gpu::fx::upload_linear_f16(ctx, &halfs, m.width, m.height)
}

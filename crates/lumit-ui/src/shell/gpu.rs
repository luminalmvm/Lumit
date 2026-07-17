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
    /// The layer's dense forward flow field `(u, v, w, h)` for Flow motion
    /// blur (docs/08 §3.2), carried from its decode job — `w × h` matches the
    /// decoded source. None unless the stack wants one.
    pub flow_field: Option<(Vec<f32>, Vec<f32>, u32, u32)>,
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
        }
    }

    /// A second handle to the shared device for the export thread.
    pub fn export_context(&self) -> lumit_gpu::GpuContext {
        lumit_gpu::GpuContext::from_parts(self.ctx.device.clone(), self.ctx.queue.clone())
    }

    /// Realise a draw list into a linear comp texture (recursive for
    /// Nested), staging at each Adjust draw (docs/06 §1.5): everything
    /// before it composites into an intermediate, the adjustment's stack
    /// runs on that, and the two blend by coverage; the draws after
    /// composite straight onto the blended result (seeded, no resample).
    fn realise(
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
            // layer are a later refinement, so no neighbours here.
            let processed = crate::fxops::run_ops(
                &self.fx,
                &self.ctx,
                below.clone(),
                width,
                height,
                &l.fx,
                &[],
                None,
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
                // The dense motion field for Flow motion blur, uploaded as its
                // own texture (only when it matches the layer's raster).
                let flow = l.flow_field.as_ref().and_then(|(u, v, fw, fh)| {
                    (*fw == w && *fh == h)
                        .then(|| lumit_gpu::fx::upload_flow_field(&self.ctx, u, v, w, h))
                });
                crate::fxops::run_ops(
                    &self.fx,
                    &self.ctx,
                    tex,
                    w,
                    h,
                    &l.fx,
                    &neighbours,
                    flow.as_ref(),
                )
            };
            linear_textures.push(tex);
        }
        let cam_mat = camera.map(|pose| crate::export::camera_mat(width, height, pose));
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
            .map(
                |(((texture, l), matte_tex), mask_tex)| lumit_gpu::CompositeLayer {
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
                    matte: matte_tex.as_ref().map(|mt| lumit_gpu::MatteInput {
                        texture: mt,
                        luma: l.matte.as_ref().is_some_and(|m| m.luma),
                        inverted: l.matte.as_ref().is_some_and(|m| m.inverted),
                    }),
                    blend: l.blend,
                    layer_mask: mask_tex.as_ref(),
                    pre: l.pre,
                },
            )
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
        let drop_n = vram_evict_count(&sizes, self.vram_bytes, bytes, VRAM_TIER_CAP);
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

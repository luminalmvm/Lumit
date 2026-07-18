//! `AppState` preview engine: the frame cache and fill scheduler, the disk
//! cache tier, comp-preview compositing jobs, and audio preparation.

use super::*;

impl AppState {
    /// Work-area frame span of a comp (start, end-exclusive); full when unset.
    pub fn work_area_frames(&self, comp: &Composition) -> (usize, usize) {
        let total = self.comp_frame_count(comp);
        let fps = comp.frame_rate.fps().max(1.0);
        match comp.work_area {
            Some((a, b)) => {
                let s = ((a.0.to_f64() * fps).round() as usize).min(total.saturating_sub(1));
                let e = ((b.0.to_f64() * fps).round() as usize).clamp(s + 1, total);
                (s, e)
            }
            None => (0, total),
        }
    }

    /// AE's B/N: set the work-area start or end at the playhead.
    pub fn set_work_area_edge(&mut self, end_edge: bool) {
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let fps = comp.frame_rate.fps().max(1.0);
        let t = self.preview_frame as f64 / fps;
        let dur = comp.duration.0.to_f64();
        let (mut a, mut b) = comp
            .work_area
            .map(|(a, b)| (a.0.to_f64(), b.0.to_f64()))
            .unwrap_or((0.0, dur));
        if end_edge {
            b = (t + 1.0 / fps).min(dur);
            if a >= b {
                a = 0.0;
            }
        } else {
            a = t.min(dur - 1.0 / fps);
            if b <= a {
                b = dur;
            }
        }
        let wa = if a <= 0.0 && (b - dur).abs() < 1e-9 {
            None // full span = no work area
        } else {
            Some((
                CompTime(
                    Rational::from_f64_on_grid(a, Rational::FLICK_DEN).unwrap_or(Rational::ZERO),
                ),
                CompTime(
                    Rational::from_f64_on_grid(b, Rational::FLICK_DEN).unwrap_or(comp.duration.0),
                ),
            ))
        };
        self.commit(Op::SetWorkArea {
            comp: comp_id,
            work_area: wa,
        });
    }

    /// Frame count of the comp preview (comp duration × comp rate).
    pub fn comp_frame_count(&self, comp: &Composition) -> usize {
        let dur = comp.duration.0.to_f64();
        (dur * comp.frame_rate.fps()).round().max(1.0) as usize
    }

    /// Build and send the batch request rendering `preview_comp` at the
    /// current frame (evaluator v0: footage layers, no retime yet).
    #[cfg(feature = "media")]
    /// The evaluator's window onto probed media: which file and which source
    /// frame a Footage layer shows, with the decode width folded into the
    /// identity so each preview-resolution tier keys separately (docs/06
    /// §5.2 quality axis; Auto folds the live zoom in the same way).
    #[cfg(feature = "media")]
    fn stamper<'a>(&'a self, doc: &'a Document) -> PreviewStamper<'a> {
        PreviewStamper {
            doc,
            media: &self.media,
            auto_res: self.preview_auto_res,
            display_scale: self.last_display_scale,
            divisor: self.preview_divisor,
        }
    }

    /// Content-hash key for one frame of a comp, or None while some footage
    /// is unprobed (rendered live, not cached).
    #[cfg(feature = "media")]
    pub fn frame_key_for(&self, comp_id: Uuid, frame: usize) -> Option<u128> {
        let doc = self.store.snapshot();
        let comp = doc.comp(comp_id)?;
        let t = frame as f64 / comp.frame_rate.fps().max(1.0);
        lumit_eval::comp_frame_key(
            &doc,
            comp,
            t,
            lumit_eval::Quality { divisor: 1 },
            &self.stamper(&doc),
        )
        .map(|k| k.0)
    }

    /// Ask the preview engine to render an arbitrary frame (the background
    /// cache fill) without moving the playhead.
    #[cfg(feature = "media")]
    pub fn request_fill_frame(&mut self, comp_id: Uuid, frame: usize) {
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let t = frame as f64 / comp.frame_rate.fps().max(1.0);
        let mut jobs = Vec::new();
        let mut visited = vec![comp_id];
        self.collect_comp_jobs(&doc, comp, t, &mut jobs, &mut visited);
        self.fill_in_flight = Some((comp_id, frame));
        self.preview_engine.request_comp(comp_id, frame, jobs);
    }

    /// The next work-area frame worth filling (forward-biased from the
    /// playhead), or None when the work area is fully cached or unkeyable.
    #[cfg(feature = "media")]
    pub fn next_fill_frame(&self, comp_id: Uuid) -> Option<usize> {
        let doc = self.store.snapshot();
        let comp = doc.comp(comp_id)?;
        let (start, end) = self.work_area_frames(comp);
        let playhead = self.preview_frame.clamp(start, end.saturating_sub(1));
        fill_walk_order(playhead, start, end)
            .into_iter()
            .find_map(|frame| {
                let key = self.frame_key_for(comp_id, frame)?;
                (!self.comp_frame_cache.contains_key(&key)).then_some(frame)
            })
    }

    /// The next frame worth warming ahead of the playhead during playback: the
    /// first uncached frame in the bounded forward lookahead window, or None
    /// when that window is fully cached or unkeyable. Unlike the idle fill this
    /// is strictly forward — it chases the audio clock, never behind it
    /// (docs/impl/playback-scheduler.md §5).
    #[cfg(feature = "media")]
    pub(crate) fn next_playback_prefetch(
        &self,
        comp_id: Uuid,
        playhead: usize,
        end: usize,
    ) -> Option<usize> {
        // Fixed lookahead for now; the impl note adapts it to render cost in a
        // later slice. Eight to sixteen frames per the note; twelve is a middle.
        const LOOKAHEAD: usize = 12;
        playback_lookahead(playhead, end, LOOKAHEAD)
            .into_iter()
            .find_map(|frame| {
                let key = self.frame_key_for(comp_id, frame)?;
                (!self.comp_frame_cache.contains_key(&key)).then_some(frame)
            })
    }

    /// One number capturing the preview-quality state (memo key component).
    #[cfg(feature = "media")]
    fn quality_tag(&self) -> u32 {
        if self.preview_auto_res {
            1000 + (self.last_display_scale.clamp(0.05, 1.0) * 100.0) as u32
        } else {
            self.preview_divisor
        }
    }

    /// Which of a comp's frames are in Kura's RAM tier — the timeline cache
    /// bar (docs/07-UI-SPEC.md: cache bars). Memoised per (document, cache
    /// state, quality); comps beyond 2 400 frames skip the bar for now (the
    /// evaluator's incremental bar replaces this scan — S-budget debt).
    #[cfg(feature = "media")]
    pub fn cache_bar(&mut self, comp: &Composition) -> Option<std::sync::Arc<Vec<CacheTier>>> {
        let total = self.comp_frame_count(comp);
        if total == 0 || total > 2400 {
            return None;
        }
        // A snapshot of the on-disk set (one lock, then no contention in the
        // per-frame loop); its size joins the memo key so disk stores/evicts
        // refresh the bar.
        let disk: std::collections::HashSet<u128> = self
            .disk_io
            .as_ref()
            .and_then(|io| io.known.lock().ok().map(|k| k.clone()))
            .unwrap_or_default();
        let key = (
            std::sync::Arc::as_ptr(&self.store.snapshot()) as usize,
            self.cache_epoch,
            self.quality_tag(),
            comp.id,
            disk.len(),
        );
        if let Some((k, bars)) = &self.cache_bar_memo {
            if *k == key {
                return Some(bars.clone());
            }
        }
        let bars: Vec<CacheTier> = (0..total)
            .map(|f| match self.frame_key_for(comp.id, f) {
                Some(k) if self.comp_frame_cache.contains_key(&k) => CacheTier::Ram,
                Some(k) if disk.contains(&k) => CacheTier::Disk,
                _ => CacheTier::None,
            })
            .collect();
        let bars = std::sync::Arc::new(bars);
        self.cache_bar_memo = Some((key, bars.clone()));
        Some(bars)
    }

    /// Keep the disk tier pointed at the saved project's sidecar (or the
    /// user's Settings → Performance → Cache root override, when set),
    /// starting the IO worker on first use. Cheap to call every frame.
    #[cfg(feature = "media")]
    pub fn disk_sync_root(&mut self, cache_root_override: Option<&std::path::Path>) {
        let root = self
            .path
            .as_deref()
            .and_then(|p| lumit_cache::disk::cache_root_for(p, cache_root_override));
        if root == self.disk_root {
            return;
        }
        if self.disk_io.is_none() {
            self.disk_io = Some(diskio::spawn());
        }
        if let Some(io) = &self.disk_io {
            let _ = io.tx.send(diskio::Cmd::SetRoot(root.clone()));
        }
        self.disk_load_pending.clear();
        self.disk_root = root;
    }

    /// Park a rendered frame on disk (write-behind; no-op while unsaved).
    #[cfg(feature = "media")]
    pub fn disk_store_behind(&mut self, key: u128, width: u32, height: u32, rgba: Vec<u8>) {
        if let Some(io) = &self.disk_io {
            let _ = io.tx.send(diskio::Cmd::Store(key, width, height, rgba));
        }
    }

    /// Whether the disk tier holds this frame (per the worker's mirror).
    #[cfg(feature = "media")]
    pub fn disk_has(&self, key: u128) -> bool {
        self.disk_io
            .as_ref()
            .and_then(|io| io.known.lock().ok().map(|k| k.contains(&key)))
            .unwrap_or(false)
    }

    /// Ask the worker to promote a frame disk → RAM (idempotent per key
    /// until the load lands or misses).
    #[cfg(feature = "media")]
    pub fn disk_request_load(&mut self, key: u128) {
        if self.disk_load_pending.contains(&key) {
            return;
        }
        if let Some(io) = &self.disk_io {
            if io.tx.send(diskio::Cmd::Load(key)).is_ok() {
                self.disk_load_pending.insert(key);
            }
        }
    }

    /// Fold completed disk loads into the RAM tier. Returns true when any
    /// frame landed (the caller repaints / re-presents).
    #[cfg(feature = "media")]
    pub fn drain_disk_loads(&mut self) -> bool {
        let mut any = false;
        // Collect first: inserting borrows self mutably.
        let mut landed = Vec::new();
        if let Some(io) = &self.disk_io {
            while let Ok((key, f)) = io.loaded.try_recv() {
                landed.push((key, f));
            }
        }
        for (key, f) in landed {
            self.disk_load_pending.remove(&key);
            self.comp_frame_cache.insert(
                key,
                CachedCompFrame {
                    width: f.width,
                    height: f.height,
                    rgba: f.rgba,
                },
            );
            self.cache_epoch += 1;
            any = true;
        }
        any
    }

    #[cfg(feature = "media")]
    pub fn refresh_comp_preview(&mut self) {
        use lumit_core::model::LayerKind;
        let Some(comp_id) = self.preview_comp else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let frames = self.comp_frame_count(comp);
        self.preview_frame = self.preview_frame.min(frames.saturating_sub(1));
        let t = self.preview_frame as f64 / comp.frame_rate.fps();

        // A real request supersedes any background fill in flight.
        self.fill_in_flight = None;

        // Kura warm path: a cached frame presents without decoding anything.
        if let Some(key) = self.frame_key_for(comp_id, self.preview_frame) {
            if self.comp_frame_cache.contains_key(&key) {
                self.cached_present = Some(key);
                return;
            }
        }

        // Recursive job collection: visible layers, their matte sources, and
        // everything nested comps need at their mapped times (cycle-guarded).
        let mut jobs = Vec::new();
        let mut visited = vec![comp_id];
        self.collect_comp_jobs(&doc, comp, t, &mut jobs, &mut visited);
        let _ = LayerKind::Footage {
            item: Uuid::now_v7(),
            retime: None,
        }; // keep the import used in both cfg modes
        self.preview_engine
            .request_comp(comp_id, self.preview_frame, jobs);
    }

    /// Decode target width for a source of `natural_w` px under the current
    /// resolution mode. Auto: displayed size, capped at 100% (never above
    /// native, however far the view is zoomed in).
    /// Any live pointer interaction (scrubbing or a drag) — background cache
    /// fills pause while it is true so they don't fight the interaction.
    pub fn is_interacting(&self) -> bool {
        self.preview_draft
            || self.prop_edit.is_some()
            || self.trim_edit.is_some()
            || self.move_edit.is_some()
            || self.graph_edit.is_some()
            || self.graph_marquee.is_some()
            || self.graph_speed_edit.is_some()
            || self.graph_tangent_edit.is_some()
            || self.graph_retime_edit.is_some()
            || self.mask_drag.is_some()
            || self.origin_drag.is_some()
            || self.shape_drag.is_some()
    }

    pub fn target_width_for(&self, natural_w: u32) -> Option<u32> {
        decode_target_width(
            natural_w,
            self.preview_draft,
            self.preview_auto_res,
            self.last_display_scale,
            self.preview_divisor,
        )
    }

    /// Recursively collect decode jobs for a comp at time `t`
    /// (docs/06-RENDER-PIPELINE.md: Precomp evaluation).
    #[cfg(feature = "media")]
    fn collect_comp_jobs(
        &self,
        doc: &Document,
        comp: &Composition,
        t: f64,
        jobs: &mut Vec<preview::CompJob>,
        visited: &mut Vec<Uuid>,
    ) {
        use lumit_core::model::LayerKind;
        let in_span =
            |l: &lumit_core::model::Layer| t >= l.in_point.0.to_f64() && t < l.out_point.0.to_f64();
        let mut wanted: Vec<Uuid> = Vec::new();
        for l in &comp.layers {
            if l.switches.visible && in_span(l) {
                wanted.push(l.id);
                if let Some(m) = &l.matte {
                    if !wanted.contains(&m.layer) {
                        wanted.push(m.layer);
                    }
                }
                // Layer-input references (K-123, e.g. a DoF depth pass) decode
                // exactly like matte sources: the referenced layer is usually
                // hidden (you don't want the depth map rendering), but its
                // pixels still feed the effect.
                for e in l.effects.iter().filter(|e| e.enabled) {
                    for p in &e.params {
                        if let lumit_core::model::EffectValue::Layer(Some(id)) = p.value {
                            if !wanted.contains(&id) {
                                wanted.push(id);
                            }
                        }
                    }
                }
            }
        }
        for layer in &comp.layers {
            if !wanted.contains(&layer.id) || !in_span(layer) {
                continue;
            }
            let lt = t - layer.start_offset.0.to_f64();
            match &layer.kind {
                // No footage source to decode (an adjustment layer processes
                // the composite below; solids/text/cameras rasterise elsewhere).
                LayerKind::Solid { .. }
                | LayerKind::Text { .. }
                | LayerKind::Camera { .. }
                | LayerKind::Adjustment => {}
                LayerKind::Sequence { clips } => {
                    // Resolve the clip under the playhead to a footage frame
                    // (comp-source clips + gaps are handled elsewhere/skip).
                    if let Some((_id, lumit_core::sequence::ClipSource::Footage(item), st)) =
                        lumit_core::sequence::resolve(clips, lt)
                    {
                        if let (
                            Some(ProjectItem::Footage(f)),
                            Some(media::MediaStatus::Ready {
                                probe,
                                frames: src_frames,
                                ..
                            }),
                        ) = (doc.item(item), self.media.map.get(&item))
                        {
                            if let Some(video) = probe.video.as_ref() {
                                use lumit_core::retime::Interpolation;
                                let interp = lumit_core::sequence::active_clip(clips, lt)
                                    .map(|c| c.interpolation.clone());
                                let blend_on = matches!(
                                    interp,
                                    Some(Interpolation::Blend | Interpolation::Flow(_))
                                );
                                let flow = matches!(interp, Some(Interpolation::Flow(_)));
                                let flow_full = matches!(
                                    &interp,
                                    Some(Interpolation::Flow(p)) if !p.half_resolution
                                );
                                let sample_fps = match &interp {
                                    Some(Interpolation::Flow(p)) => p.input_fps,
                                    _ => None,
                                };
                                let (source_frame, blend) = crate::pixels::frame_pick(
                                    st,
                                    video.fps(),
                                    *src_frames,
                                    blend_on,
                                    sample_fps,
                                );
                                jobs.push(preview::CompJob {
                                    layer: layer.id,
                                    item,
                                    path: PathBuf::from(&f.media.absolute_path),
                                    source_frame,
                                    target_width: self.target_width_for(video.width),
                                    natural_w: video.width,
                                    natural_h: video.height,
                                    blend,
                                    flow,
                                    flow_full,
                                    // Temporal effects on Sequence clips are a
                                    // later refinement (clip-relative neighbour
                                    // resolution); footage layers first.
                                    temporal: Vec::new(),
                                    flow_neighbour: None,
                                });
                            }
                        }
                    }
                }
                LayerKind::Precomp { comp: nested_id } => {
                    if visited.contains(nested_id) {
                        continue; // cycle guard
                    }
                    if let Some(nested) = doc.comp(*nested_id) {
                        visited.push(*nested_id);
                        self.collect_comp_jobs(doc, nested, lt, jobs, visited);
                        visited.pop();
                    }
                }
                LayerKind::Footage { item, retime } => {
                    let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
                        continue;
                    };
                    let Some(media::MediaStatus::Ready {
                        probe,
                        frames: src_frames,
                        ..
                    }) = self.media.map.get(item)
                    else {
                        continue; // not probed yet; retried once Ready
                    };
                    let Some(video) = probe.video.as_ref() else {
                        continue;
                    };
                    // Retime maps local time → source time before frame pick;
                    // its interpolation policy decides nearest vs blend.
                    let source_time = retime.as_ref().map(|r| r.evaluate(lt)).unwrap_or(lt);
                    use lumit_core::retime::Interpolation;
                    let interp = retime.as_ref().map(|r| &r.interpolation);
                    let blend_on =
                        matches!(interp, Some(Interpolation::Blend | Interpolation::Flow(_)));
                    let flow = matches!(interp, Some(Interpolation::Flow(_)));
                    let flow_full =
                        matches!(interp, Some(Interpolation::Flow(p)) if !p.half_resolution);
                    let sample_fps = match interp {
                        Some(Interpolation::Flow(p)) => p.input_fps,
                        _ => None,
                    };
                    let (source_frame, blend) = crate::pixels::frame_pick(
                        source_time,
                        video.fps(),
                        *src_frames,
                        blend_on,
                        sample_fps,
                    );
                    // Neighbour source frames for a temporal effect stack
                    // (echo/trails, flow motion blur, datamosh): the layer's
                    // source at each non-zero offset in the stack's window,
                    // mapped through the retime like the primary frame. Empty
                    // unless the stack actually reads other frames, so a plain
                    // footage layer decodes exactly one frame.
                    let temporal =
                        if lumit_core::fx::stack_is_temporal(&layer.effects, layer.switches.fx) {
                            let comp_dt = 1.0 / comp.frame_rate.fps().max(1.0);
                            lumit_core::fx::stack_temporal_window(&layer.effects, layer.switches.fx)
                                .into_iter()
                                .filter(|&o| o != 0)
                                .map(|o| {
                                    let nlt = lt + f64::from(o) * comp_dt;
                                    let nst =
                                        retime.as_ref().map(|r| r.evaluate(nlt)).unwrap_or(nlt);
                                    let (nf, _) = crate::pixels::frame_pick(
                                        nst,
                                        video.fps(),
                                        *src_frames,
                                        false,
                                        None,
                                    );
                                    (o, nf)
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                    jobs.push(preview::CompJob {
                        layer: layer.id,
                        item: *item,
                        path: PathBuf::from(&f.media.absolute_path),
                        source_frame,
                        target_width: self.target_width_for(video.width),
                        natural_w: video.width,
                        natural_h: video.height,
                        blend,
                        flow,
                        flow_full,
                        temporal,
                        // Flow motion blur / Datamosh measure motion between
                        // this frame and their requested neighbour (already
                        // in `temporal`).
                        flow_neighbour: lumit_core::fx::stack_flow_neighbour(
                            &layer.effects,
                            layer.switches.fx,
                        ),
                    });
                }
            }
        }
    }

    /// Re-request the current preview frame (selection/scrub/resolution change).
    #[cfg(feature = "media")]
    pub fn refresh_preview(&mut self) {
        if self.preview_comp.is_some() {
            self.refresh_comp_preview();
            return;
        }
        let Some(id) = self.preview_item else { return };
        let doc = self.store.snapshot();
        let Some(ProjectItem::Footage(f)) = doc.item(id) else {
            return;
        };
        let (width, frames) = match self.media.map.get(&id) {
            Some(media::MediaStatus::Ready { probe, frames, .. }) => {
                (probe.video.as_ref().map(|v| v.width).unwrap_or(0), *frames)
            }
            _ => return, // not probed yet; selection will refresh on Ready
        };
        if frames == 0 || width == 0 {
            return;
        }
        self.preview_frame = self.preview_frame.min(frames - 1);
        let target = self.target_width_for(width);
        self.preview_engine.request(
            id,
            PathBuf::from(&f.media.absolute_path),
            self.preview_frame,
            target,
        );
    }

    /// Frames-per-second of the current preview item, once probed.
    #[cfg(feature = "media")]
    pub fn preview_fps(&self) -> Option<f64> {
        let id = self.preview_item?;
        match self.media.map.get(&id) {
            Some(media::MediaStatus::Ready { probe, .. }) => {
                probe.video.as_ref().map(|v| v.fps()).filter(|f| *f > 0.0)
            }
            _ => None,
        }
    }

    /// Kick off background audio decode for the preview item (idempotent).
    #[cfg(feature = "media")]
    pub fn request_preview_audio(&mut self) {
        let Some(id) = self.preview_item else { return };
        if self.audio_cache.contains_key(&id) {
            return;
        }
        let has_audio = matches!(
            self.media.map.get(&id),
            Some(media::MediaStatus::Ready { probe, .. }) if probe.audio.is_some()
        );
        if !has_audio {
            return;
        }
        let doc = self.store.snapshot();
        let Some(ProjectItem::Footage(f)) = doc.item(id) else {
            return;
        };
        let path = PathBuf::from(&f.media.absolute_path);
        let rate = self
            .ensure_audio_engine()
            .map(|e| e.device_rate())
            .unwrap_or(48_000);
        let tx = self.audio_tx.clone();
        std::thread::spawn(move || {
            let result = lumit_media::audio::decode_all(&path, rate).map_err(|e| e.to_string());
            let _ = tx.send((id, result));
        });
    }

    #[cfg(feature = "media")]
    pub(crate) fn ensure_audio_engine(&mut self) -> Option<&lumit_audio::AudioEngine> {
        if self.audio_engine.is_none() {
            match lumit_audio::AudioEngine::new() {
                Ok(engine) => self.audio_engine = Some(engine),
                Err(e) => {
                    self.error = Some(format!("audio: {e}"));
                    return None;
                }
            }
        }
        self.audio_engine.as_ref()
    }

    /// Drain finished audio decodes; auto-load the current preview item's.
    #[cfg(feature = "media")]
    pub fn poll_audio(&mut self) {
        while let Ok((id, result)) = self.audio_rx.try_recv() {
            match result {
                Ok(buffer) => {
                    self.audio_cache.insert(id, std::sync::Arc::new(buffer));
                }
                Err(e) => self.error = Some(format!("audio decode: {e}")),
            }
        }
    }

    /// Every audible footage layer with an audio stream, as mixdown jobs:
    /// the one list playback, beat detection, and export all mix from, so
    /// they always hear the same comp.
    #[cfg(feature = "media")]
    pub fn comp_audio_jobs(
        &self,
        doc: &lumit_core::model::Document,
        comp: &lumit_core::model::Composition,
    ) -> Vec<crate::export::AudioJob> {
        use lumit_core::model::LayerKind;
        let mut jobs = Vec::new();
        for layer in &comp.layers {
            if !layer.switches.audible {
                continue;
            }
            let LayerKind::Footage { item, .. } = &layer.kind else {
                continue;
            };
            let has_audio = matches!(
                self.media.map.get(item),
                Some(media::MediaStatus::Ready { probe, .. }) if probe.audio.is_some()
            );
            if !has_audio {
                continue;
            }
            let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
                continue;
            };
            jobs.push(crate::export::AudioJob {
                path: PathBuf::from(&f.media.absolute_path),
                in_s: layer.in_point.0.to_f64(),
                out_s: layer.out_point.0.to_f64(),
                offset_s: layer.start_offset.0.to_f64(),
            });
        }
        jobs
    }

    /// Kick off background decode + mix of a comp's audio layers into one
    /// buffer (Hibiki mix): the layers' sound laid on the comp timeline at
    /// their offsets and trims. The result arrives via [`Self::poll_comp_audio`].
    /// A comp with no audio layers prepares nothing (it plays on the fallback
    /// wall clock).
    #[cfg(feature = "media")]
    pub fn prepare_comp_audio(&mut self, comp_id: Uuid) {
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let rate = self
            .ensure_audio_engine()
            .map(|e| e.device_rate())
            .unwrap_or(48_000);
        let jobs = self.comp_audio_jobs(&doc, comp);
        if jobs.is_empty() {
            return; // silent comp: wall-clock fallback drives playback
        }
        let duration_s = comp.duration.0.to_f64();
        let tx = self.comp_audio_tx.clone();
        std::thread::spawn(move || {
            let samples = crate::export::mixdown(&jobs, rate, duration_s);
            let _ = tx.send((comp_id, lumit_media::AudioBuffer { rate, samples }));
        });
    }

    /// Drain any finished comp-audio mix; load it into the engine and, if the
    /// comp is playing, start its clock at the current playhead.
    #[cfg(feature = "media")]
    pub fn poll_comp_audio(&mut self) {
        let mut newest = None;
        while let Ok(msg) = self.comp_audio_rx.try_recv() {
            newest = Some(msg);
        }
        let Some((comp_id, buffer)) = newest else {
            return;
        };
        if self.preview_comp != Some(comp_id) {
            return; // the user moved on before the mix finished
        }
        let doc = self.store.snapshot();
        let Some(fps) = doc.comp(comp_id).map(|c| c.frame_rate.fps().max(1.0)) else {
            return;
        };
        if self.ensure_audio_engine().is_none() {
            return;
        }
        let playing = self.comp_playback.is_some();
        let start_s = self.preview_frame as f64 / fps;
        // Waveform peaks for the timeline (computed before the buffer moves
        // into the engine); ~2 buckets per horizontal pixel is plenty.
        self.comp_waveform = Some((
            comp_id,
            lumit_audio::mix::waveform_peaks(&buffer.samples, 2048),
        ));
        if let Some(engine) = &self.audio_engine {
            engine.load(std::sync::Arc::new(buffer));
            engine.seek_seconds(start_s);
            if playing {
                engine.play();
            }
        }
        self.audio_loaded_comp = Some(comp_id);
        self.audio_loaded = None; // the footage buffer is no longer loaded
    }
}

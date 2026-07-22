//! Headless comp rendering — the seam the Flutter Viewer shares with export.
//!
//! # In plain terms
//!
//! The Viewer needs the *real* picture: every layer composited, transformed,
//! blended, with its effects — the same pixels the egui Viewer shows and the
//! same pixels the exported file carries. That work lives in [`crate::export`]'s
//! `Renderer`, which needs no window and no egui: only a GPU device, the media
//! decoders, and the document. This module wraps that renderer in a small,
//! reusable object a *second* frontend (Flutter, over `lumit-bridge`) can hold
//! and drive frame by frame.
//!
//! A [`HeadlessRenderer`] owns the expensive, must-persist state — the GPU
//! context (whose adapter is created once) and the compiled shader engines —
//! plus a decoder pool and a probe cache, and lends them to a fresh
//! `export::Renderer` for each call. Because it drives the identical compositor
//! export drives, preview == export == Flutter (K-031). The bridge borrowing
//! this seam is the deliberate, temporary architecture recorded as K-175: the
//! bridge reaches into `lumit-ui`'s renderer until the pixel pass moves into an
//! engine crate.
//!
//! Everything here is gated behind the `media` feature (it needs `lumit-gpu`,
//! `lumit-flow` and `lumit-media`); a `--no-default-features` build has no
//! headless renderer at all, exactly as it has no export.

#![cfg(feature = "media")]

use crate::export::{AudioJob, ItemInfo, Renderer};
use lumit_core::model::{Composition, Document, FootageItem, LayerKind, ProjectItem};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The persistent GPU engines + decoder pool a render needs, held between calls
/// so shaders compile once and warm decoders survive a scrub. Lent to a
/// [`Renderer`] for the duration of one render and taken back afterwards.
struct Parts {
    colour: lumit_gpu::ColourEngine,
    compositor: lumit_gpu::Compositor,
    fx: lumit_gpu::fx::FxEngine,
    flow: lumit_flow::FlowEngine,
    lut_cache: std::cell::RefCell<HashMap<String, crate::fxops::LoadedLut>>,
    decoders: HashMap<Uuid, lumit_media::VideoDecoder>,
}

/// One footage item's probe result, cached so a scrub does not re-probe. Slate
/// sizing is deliberately *not* stored here — the missing/failed slate is sized
/// to the comp being rendered at call time, since the same item can appear in
/// comps of different dimensions.
enum Probe {
    /// Decodable video: its exact rate and frame count (the `frame_pick` inputs).
    Ok { fps: f64, frames: usize },
    /// Not on disk, or present-but-unreadable: render the colour-bars slate,
    /// exactly as export's `item_infos` carries a `Missing` item (docs/07 §3.3).
    Slate,
}

/// A reusable, window-free renderer that turns `(Document, comp, frame)` into an
/// RGBA8 buffer through the export compositor. Hold one per frontend session.
///
/// The GPU adapter is acquired in [`HeadlessRenderer::new`]; a machine with no
/// adapter fails there, and the caller (the bridge) never constructs a second —
/// it returns its calm "no frame" state instead of retrying every call.
pub struct HeadlessRenderer {
    gpu: lumit_gpu::GpuContext,
    /// `Some` except for the instant a render borrows the engines. A render that
    /// unwinds (never expected — engine crates forbid panics) leaves this `None`,
    /// and further calls answer a calm error rather than crashing.
    parts: Option<Parts>,
    /// The GPU scope pass (K-096 v1). Held directly rather than in [`Parts`]
    /// because a scope trace runs *from a finished frame*, not during a
    /// composite, so it is never lent to the `Renderer` — it borrows `&self.gpu`
    /// on its own. Compiled once with the other engines.
    scope: lumit_gpu::scope::ScopeEngine,
    /// The `ItemInfo` map the renderer reads, rebuilt each call (cheap — it only
    /// reads `probe_cache`) so a missing item's slate matches the current comp.
    items: HashMap<Uuid, ItemInfo>,
    /// Probe results by footage id, so each file is probed at most once.
    probe_cache: HashMap<Uuid, Probe>,
    /// Whether each footage item carries an audio stream, cached so building the
    /// export audio jobs probes each file at most once (export path only).
    audio_cache: HashMap<Uuid, bool>,
    /// The Windows zero-copy Viewer target (K-177), held for the session and
    /// re-created only when the comp's dimensions change. `None` until the first
    /// `render_to_shared` call. Present only in the opt-in shared-texture build.
    #[cfg(all(windows, feature = "shared-texture"))]
    shared: Option<lumit_gpu::shared::SharedTexture>,
}

/// A rendered frame that stayed on the GPU: the NT handle of the shared texture
/// it lives in, plus its dimensions and format (K-177). Handed across the bridge
/// so the Windows runner can register the texture with Flutter without any pixel
/// copy. The handle stays valid across frames (the same texture is re-used) and
/// only changes when the comp is resized.
#[cfg(all(windows, feature = "shared-texture"))]
pub struct SharedFrameInfo {
    /// The NT `HANDLE` value of the shared texture (a
    /// `kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle` surface).
    pub handle: u64,
    pub width: u32,
    pub height: u32,
    /// Always RGBA8888 (`DXGI_FORMAT_R8G8B8A8_UNORM` holding sRGB-encoded bytes),
    /// the identical pixels the read-back path produces.
    pub format: &'static str,
}

/// The inputs one export needs, built through the headless seam (K-175) so the
/// bridge can drive the exact egui exporter (`crate::export::start`): the footage
/// [`ItemInfo`] map, the comp's audio jobs, and a GPU context sharing the
/// renderer's device. Handed to the exporter, which spawns its own encode thread
/// (K-017).
pub struct ExportInputs {
    pub items: HashMap<Uuid, ItemInfo>,
    pub audio: Vec<AudioJob>,
    pub gpu: lumit_gpu::GpuContext,
}

impl HeadlessRenderer {
    /// Build a headless renderer, acquiring a GPU adapter and compiling the
    /// shader engines. `Err` when no adapter exists (the bridge turns this into
    /// its "no adapter" state) or the device request fails.
    pub fn new() -> Result<Self, String> {
        let gpu = lumit_gpu::GpuContext::headless().map_err(|e| e.to_string())?;
        let parts = Parts {
            colour: lumit_gpu::ColourEngine::new(&gpu),
            compositor: lumit_gpu::Compositor::new(&gpu),
            fx: lumit_gpu::fx::FxEngine::new(&gpu),
            flow: lumit_flow::FlowEngine::with_context(&gpu),
            lut_cache: std::cell::RefCell::new(HashMap::new()),
            decoders: HashMap::new(),
        };
        let scope = lumit_gpu::scope::ScopeEngine::new(&gpu);
        Ok(Self {
            gpu,
            parts: Some(parts),
            scope,
            items: HashMap::new(),
            probe_cache: HashMap::new(),
            audio_cache: HashMap::new(),
            #[cfg(all(windows, feature = "shared-texture"))]
            shared: None,
        })
    }

    /// Build the inputs one export of `comp_id` needs (the bridge's v0.4 export
    /// path, K-175): the footage [`ItemInfo`] map (probed exactly as a render
    /// probes, sharing this renderer's cache), the comp's audio jobs, and a GPU
    /// context sharing this renderer's device. `None` when `comp_id` is unknown.
    /// The exporter (`crate::export::start`) takes these and spawns its own
    /// encode thread (K-017), so this call is cheap and holds no GPU work.
    pub fn export_inputs(&mut self, doc: &Document, comp_id: Uuid) -> Option<ExportInputs> {
        let comp = doc.comp(comp_id)?;
        let (cw, ch) = (comp.width, comp.height);
        self.sync_items(doc, (cw, ch));
        let items = self.items.clone();
        let audio = self.collect_audio(doc, comp);
        // Share the device/queue (wgpu handles are reference-counted); the
        // exporter builds its own engines on top, exactly as the egui path's
        // `export_context` lends the display device.
        let gpu =
            lumit_gpu::GpuContext::from_parts(self.gpu.device.clone(), self.gpu.queue.clone());
        Some(ExportInputs { items, audio, gpu })
    }

    /// Collect `comp`'s audio jobs for export — the headless twin of
    /// `AppState::comp_audio_jobs` (docs/09 §6): every audible footage layer with
    /// an audio stream, its span mapped to the comp timeline, plus nested Precomp
    /// layers' contents scaled by their carrier Volumes. Solo silences non-soloed
    /// audio per comp, exactly as the video gate does.
    fn collect_audio(&mut self, doc: &Document, comp: &Composition) -> Vec<AudioJob> {
        let mut jobs = Vec::new();
        let mut visited = vec![comp.id];
        self.collect_audio_jobs(
            doc,
            comp,
            0.0,
            (f64::NEG_INFINITY, f64::INFINITY),
            &[],
            &mut visited,
            &mut jobs,
        );
        jobs
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_audio_jobs(
        &mut self,
        doc: &Document,
        comp: &Composition,
        base_s: f64,
        window: (f64, f64),
        carriers: &[(lumit_core::anim::Property, f64)],
        visited: &mut Vec<Uuid>,
        jobs: &mut Vec<AudioJob>,
    ) {
        let any_solo = lumit_core::model::any_solo(comp);
        for layer in &comp.layers {
            if !layer.switches.audible || (any_solo && !layer.switches.solo) {
                continue;
            }
            let in_s = (layer.in_point.0.to_f64() + base_s).max(window.0);
            let out_s = (layer.out_point.0.to_f64() + base_s).min(window.1);
            if out_s <= in_s {
                continue;
            }
            let offset_s = layer.start_offset.0.to_f64() + base_s;
            match &layer.kind {
                LayerKind::Footage { item, .. } => {
                    let Some(ProjectItem::Footage(f)) = doc.item(*item) else {
                        continue;
                    };
                    if !self.has_audio(*item, &footage_path(f)) {
                        continue;
                    }
                    jobs.push(AudioJob {
                        item: *item,
                        path: footage_path(f),
                        in_s,
                        out_s,
                        offset_s,
                        volume: layer.volume_db.clone(),
                        carriers: carriers.to_vec(),
                    });
                }
                LayerKind::Precomp { comp: nested_id } => {
                    if visited.contains(nested_id) {
                        continue;
                    }
                    let Some(nested) = doc.comp(*nested_id) else {
                        continue;
                    };
                    let mut inner = carriers.to_vec();
                    inner.push((layer.volume_db.clone(), offset_s));
                    visited.push(*nested_id);
                    self.collect_audio_jobs(
                        doc,
                        nested,
                        offset_s,
                        (in_s, out_s),
                        &inner,
                        visited,
                        jobs,
                    );
                    visited.pop();
                }
                _ => {}
            }
        }
    }

    /// Whether footage `item` at `path` carries an audio stream, cached so each
    /// file is probed for audio at most once across an export session.
    fn has_audio(&mut self, item: Uuid, path: &Path) -> bool {
        if let Some(&has) = self.audio_cache.get(&item) {
            return has;
        }
        let has = path.is_file()
            && lumit_media::probe::probe(path)
                .map(|p| p.audio.is_some())
                .unwrap_or(false);
        self.audio_cache.insert(item, has);
        has
    }

    /// Render composition `comp_id` at integer `frame` to tightly-packed RGBA8,
    /// returning `(pixels, width, height)`. `scale` of 1.0 is the comp's own
    /// resolution; a smaller positive `scale` downsamples the *output* (the
    /// internal composite is always full resolution — see the note below).
    ///
    /// The frame is `frame / fps` seconds of comp time, `fps` the comp's exact
    /// rational rate, exactly as export computes it. A missing layer inside the
    /// comp is drawn as colour bars by the compositor itself (the slate is baked
    /// into the composited frame, not painted around it), so the returned buffer
    /// already carries it — the Flutter Viewer needs no separate slate on the
    /// comp path.
    ///
    /// `scale` note: the export compositor renders at the comp's dimensions;
    /// there is no cheap reduced-resolution target on this path, so `scale`
    /// only resizes the finished buffer (a cheaper blit for the Viewer), it does
    /// not reduce the GPU cost. A future reduced-resolution preview render would
    /// change that.
    pub fn render_rgba(
        &mut self,
        doc: &Document,
        comp_id: Uuid,
        frame: u64,
        scale: f32,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let comp = doc
            .comp(comp_id)
            .ok_or_else(|| "headless render: unknown composition".to_string())?;
        let (cw, ch) = (comp.width, comp.height);
        self.sync_items(doc, (cw, ch));
        let fps = comp.frame_rate.fps().max(1.0);
        let t = frame as f64 / fps;

        let Some(parts) = self.parts.take() else {
            return Err("headless render: renderer is unavailable after an earlier fault".into());
        };
        let mut renderer = Renderer {
            doc,
            items: &self.items,
            gpu: &self.gpu,
            colour: parts.colour,
            compositor: parts.compositor,
            decoders: parts.decoders,
            flow: parts.flow,
            fx: parts.fx,
            lut_cache: parts.lut_cache,
        };
        // Drive the exact export path: composite to a linear texture, encode to
        // the display transfer function, read the bytes back (K-031).
        let mut visited = vec![comp_id];
        let out = render_to_rgba(
            &mut renderer,
            comp,
            t,
            &mut visited,
            &self.gpu,
            cw,
            ch,
            scale,
        );
        // Return the engines and warm decoders to the pool, even on error, so a
        // single failed frame does not discard the compiled shaders.
        self.parts = Some(Parts {
            colour: renderer.colour,
            compositor: renderer.compositor,
            fx: renderer.fx,
            flow: renderer.flow,
            lut_cache: renderer.lut_cache,
            decoders: renderer.decoders,
        });
        out
    }

    /// Compute a scope trace (waveform/vectorscope/histogram, K-096 v1) from an
    /// already-rendered comp frame's display bytes, returning the `GRID × GRID`
    /// RGBA8 trace. `rgba` is the exact frame the Viewer shows (served from the
    /// bridge's rendered-frame cache, so the scope traces the same frame at no
    /// re-render cost); the binning runs on the GPU and only the tiny trace is
    /// read back.
    ///
    /// `kind` is `0` luma / `1` RGB waveform / `2` vectorscope / `3` histogram
    /// (an unknown value is a calm `Err`); `colours` carries the frontend's fixed
    /// `ScopeColours` as `[bg, trace, red, green, blue]` RGB byte triples, so no
    /// colour literal lives in the engine (docs/15-DESIGN.md) and the bridge need
    /// not name `lumit-gpu`. `Err` on an unknown kind or if the tiny readback
    /// fails.
    pub fn render_scope(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        kind: u32,
        colours: [[u8; 3]; 5],
    ) -> Result<Vec<u8>, String> {
        let kind = match kind {
            0 => lumit_gpu::scope::ScopeKind::WaveformLuma,
            1 => lumit_gpu::scope::ScopeKind::WaveformRgb,
            2 => lumit_gpu::scope::ScopeKind::Vectorscope,
            3 => lumit_gpu::scope::ScopeKind::Histogram,
            other => return Err(format!("headless scope: unknown kind {other}")),
        };
        let colours = lumit_gpu::scope::ScopeColours {
            bg: colours[0],
            trace: colours[1],
            red: colours[2],
            green: colours[3],
            blue: colours[4],
        };
        self.scope
            .trace_rgba8(&self.gpu, kind, colours, rgba, width, height)
            .map_err(|e| e.to_string())
    }

    /// Render composition `comp_id` at integer `frame` into the Windows shared
    /// GPU texture, returning its NT handle and dimensions ([`SharedFrameInfo`],
    /// K-177) — the zero-copy sibling of [`Self::render_rgba`]. The frame never
    /// leaves the graphics card: it is composited and display-encoded exactly as
    /// `render_rgba` does (preview == export == Flutter, K-031), then copied into
    /// the shared texture instead of being read back to the CPU.
    ///
    /// The shared texture is created on the first call and re-used across frames
    /// (a stable handle); a comp of different dimensions re-creates it and reports
    /// the new handle. `Err` on an unknown comp, when wgpu is not on the D3D12
    /// backend, or any D3D interop failure — the bridge turns that into "no
    /// shared frame" and Dart falls back to the read-back path.
    #[cfg(all(windows, feature = "shared-texture"))]
    pub fn render_to_shared(
        &mut self,
        doc: &Document,
        comp_id: Uuid,
        frame: u64,
    ) -> Result<SharedFrameInfo, String> {
        let comp = doc
            .comp(comp_id)
            .ok_or_else(|| "headless render: unknown composition".to_string())?;
        let (cw, ch) = (comp.width, comp.height);
        self.sync_items(doc, (cw, ch));
        let fps = comp.frame_rate.fps().max(1.0);
        let t = frame as f64 / fps;

        let Some(parts) = self.parts.take() else {
            return Err("headless render: renderer is unavailable after an earlier fault".into());
        };
        let mut renderer = Renderer {
            doc,
            items: &self.items,
            gpu: &self.gpu,
            colour: parts.colour,
            compositor: parts.compositor,
            decoders: parts.decoders,
            flow: parts.flow,
            fx: parts.fx,
            lut_cache: parts.lut_cache,
        };
        let mut visited = vec![comp_id];
        // Ensure the shared texture matches the comp size (create/recreate), then
        // composite and copy into it — disjoint field borrows (`&self.gpu`
        // immutable, `&mut self.shared` mutable) so the borrow checker is happy.
        let out = render_display_into_shared(
            &mut renderer,
            comp,
            t,
            &mut visited,
            &self.gpu,
            &mut self.shared,
            cw,
            ch,
        );
        // Return the engines and warm decoders to the pool, even on error.
        self.parts = Some(Parts {
            colour: renderer.colour,
            compositor: renderer.compositor,
            fx: renderer.fx,
            flow: renderer.flow,
            lut_cache: renderer.lut_cache,
            decoders: renderer.decoders,
        });
        out
    }

    /// Rebuild the `ItemInfo` map from the document's footage, probing any item
    /// not already in `probe_cache`. Slate items are sized to `slate` (the
    /// comp's dimensions this call), matching export's `item_infos`.
    fn sync_items(&mut self, doc: &Document, slate: (u32, u32)) {
        self.items.clear();
        for item in &doc.items {
            let ProjectItem::Footage(f) = item else {
                continue;
            };
            let probe = self
                .probe_cache
                .entry(f.id)
                .or_insert_with(|| probe_item(&footage_path(f)));
            match probe {
                Probe::Ok { fps, frames } => {
                    self.items.insert(
                        f.id,
                        ItemInfo {
                            path: footage_path(f),
                            fps: *fps,
                            frames: *frames,
                            missing: None,
                        },
                    );
                }
                // A slate item carries the comp's size so its geometry matches a
                // real layer's (the same reasoning export's `ItemInfo::missing`
                // documents). A `Failed` file in export is simply absent from the
                // map; here it slates instead, so an unreadable source is visibly
                // flagged in the Viewer rather than silently dropped.
                Probe::Slate => {
                    self.items.insert(
                        f.id,
                        ItemInfo {
                            path: footage_path(f),
                            fps: 1.0,
                            frames: 1,
                            missing: Some(slate),
                        },
                    );
                }
            }
        }
    }
}

/// Composite once and read the pixels back, then apply the output `scale`.
/// Split out so `render_rgba` can restore the engine pool on either arm.
#[allow(clippy::too_many_arguments)]
fn render_to_rgba(
    renderer: &mut Renderer,
    comp: &lumit_core::model::Composition,
    t: f64,
    visited: &mut Vec<Uuid>,
    gpu: &lumit_gpu::GpuContext,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let linear = renderer.render_comp_linear(comp, t, visited)?;
    let shown = renderer.colour.display(gpu, &linear);
    let rgba = renderer
        .colour
        .readback8(gpu, &shown)
        .map_err(|e| e.to_string())?;
    // Full resolution unless a valid, shrinking scale is asked for; the resize
    // preserves aspect (same-aspect target, so no letterbox bars appear) and
    // reuses the export path's bilinear resampler.
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 1e-4 {
        return Ok((rgba, width, height));
    }
    let sw = ((width as f32 * scale).round() as u32).max(1);
    let sh = ((height as f32 * scale).round() as u32).max(1);
    let scaled = crate::pixels::letterbox_resize(&rgba, width, height, sw, sh);
    Ok((scaled, sw, sh))
}

/// Composite once, display-encode, and copy the result into the shared texture
/// (creating/resizing it as needed), returning its handle. Split out so
/// `render_to_shared` can restore the engine pool on either arm. The composite +
/// display passes are byte-for-byte what `render_to_rgba` runs; only the final
/// step differs (a GPU-to-GPU copy instead of a read-back).
#[cfg(all(windows, feature = "shared-texture"))]
#[allow(clippy::too_many_arguments)]
fn render_display_into_shared(
    renderer: &mut Renderer,
    comp: &lumit_core::model::Composition,
    t: f64,
    visited: &mut Vec<Uuid>,
    gpu: &lumit_gpu::GpuContext,
    shared: &mut Option<lumit_gpu::shared::SharedTexture>,
    width: u32,
    height: u32,
) -> Result<SharedFrameInfo, String> {
    let linear = renderer.render_comp_linear(comp, t, visited)?;
    let shown = renderer.colour.display(gpu, &linear);

    // Re-create the shared texture when it is missing or the comp changed size —
    // a new handle is reported then, which the bridge relays so Dart re-registers.
    let needs_new = match shared.as_ref() {
        Some(s) => s.width != width || s.height != height,
        None => true,
    };
    if needs_new {
        *shared = Some(lumit_gpu::shared::SharedTexture::new(gpu, width, height)?);
    }
    let target = shared
        .as_ref()
        .ok_or_else(|| "headless render: shared texture missing after create".to_string())?;
    target.present(gpu, &shown);
    Ok(SharedFrameInfo {
        handle: target.handle(),
        width,
        height,
        format: "rgba8888",
    })
}

/// The on-disk path a footage item points at (absolute when known, else the
/// stored relative path) — the same resolution the bridge's decode path uses.
fn footage_path(f: &FootageItem) -> PathBuf {
    if f.media.absolute_path.is_empty() {
        PathBuf::from(&f.media.relative_path)
    } else {
        PathBuf::from(&f.media.absolute_path)
    }
}

/// Probe one footage path into a [`Probe`]. A path that is not a file, an
/// unreadable file, a file with no video stream, or one whose frame index will
/// not build all fall to [`Probe::Slate`] — none of them is an error, they are
/// the states the slate exists for. A clean video caches its exact rate and
/// frame count, warming the on-disk frame index so the decoder open reuses it.
fn probe_item(path: &Path) -> Probe {
    if !path.is_file() {
        return Probe::Slate;
    }
    let Ok(probe) = lumit_media::probe::probe(path) else {
        return Probe::Slate;
    };
    let Some(video) = probe.video.as_ref() else {
        return Probe::Slate;
    };
    let Some(index) = load_or_build_index(path) else {
        return Probe::Slate;
    };
    Probe::Ok {
        fps: video.fps(),
        frames: index.frame_count(),
    }
}

/// Load the cached frame index for `path` if one matches, else build it and try
/// to cache it — the same warm-the-cache dance the bridge's decode path runs, so
/// the count here and the decoder the renderer opens share one index. `None`
/// when the index cannot be built.
fn load_or_build_index(path: &Path) -> Option<lumit_media::FrameIndex> {
    let cache_dir = lumit_project::media_index_dir();
    if let (Some(dir), Ok(fp)) = (&cache_dir, lumit_media::Fingerprint::of(path)) {
        if let Some(index) = lumit_media::FrameIndex::load_cached(dir, &fp) {
            return Some(index);
        }
    }
    let index = lumit_media::index::build_frame_index(path).ok()?;
    if let Some(dir) = &cache_dir {
        let _ = index.save_to(dir);
    }
    Some(index)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::{
        Composition, LayerKind, LinearColour, ProjectItem, SolidDef, Switches, TransformGroup,
    };
    use lumit_core::store::DocumentStore;
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

    /// A transform that centres a `w`×`h` object over a `w`×`h` comp (anchor at
    /// the object's middle, position at the comp's middle) — a copy of the
    /// engine's `centred_transform`, so the solid fills the frame.
    fn centred(w: u32, h: u32) -> TransformGroup {
        TransformGroup {
            anchor_x: Property::fixed(f64::from(w) * 0.5),
            anchor_y: Property::fixed(f64::from(h) * 0.5),
            position_x: Property::fixed(f64::from(w) * 0.5),
            position_y: Property::fixed(f64::from(h) * 0.5),
            ..Default::default()
        }
    }

    /// Build a document with one comp holding a single full-frame solid layer of
    /// `colour`, returning the store and the comp id. Drives the real model, so
    /// the render walks the same path a user-built comp would.
    fn doc_with_solid(colour: LinearColour, w: u32, h: u32) -> (DocumentStore, Uuid) {
        let mut doc = Document::new();
        let solid_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: solid_id,
            name: "Solid".into(),
            colour,
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        let comp_id = Uuid::now_v7();
        let layer = lumit_core::model::Layer {
            id: Uuid::now_v7(),
            name: "Solid".into(),
            kind: LayerKind::Solid { def: solid_id },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: centred(w, h),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        doc.items.push(ProjectItem::Composition(Composition {
            id: comp_id,
            name: "Scene".into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (DocumentStore::new(doc), comp_id)
    }

    /// A full-frame red solid composites to red in the centre pixel — the GPU
    /// oracle that proves the headless seam drives the real compositor. Skips
    /// when the machine has no adapter (the lavapipe/hardware convention the
    /// lumit-gpu tests use).
    #[test]
    fn solid_comp_renders_its_colour_in_the_centre() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        // Pure-red scene-linear solid, 8×8.
        let (store, comp_id) = doc_with_solid(LinearColour([1.0, 0.0, 0.0, 1.0]), 8, 8);
        let doc = store.snapshot();
        let (rgba, w, h) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("render");
        assert_eq!((w, h), (8, 8));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Centre pixel: strongly red, weak green/blue, opaque. sRGB-encoded, so
        // the exact byte depends on the transfer function; assert the channel
        // ordering and that red dominates rather than an exact value.
        let idx = (((h / 2) * w + w / 2) * 4) as usize;
        let (red, green, blue, alpha) = (rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]);
        assert!(red > 200, "red channel should dominate, got {red}");
        assert!(green < 60, "green should be low, got {green}");
        assert!(blue < 60, "blue should be low, got {blue}");
        assert_eq!(alpha, 255, "the solid is opaque");
    }

    /// `scale` below 1 downsamples the output buffer; the centre stays the solid
    /// colour, proving the resize path is wired and does not corrupt the frame.
    #[test]
    fn scale_downsamples_the_output() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 1.0, 0.0, 1.0]), 16, 16);
        let doc = store.snapshot();
        let (rgba, w, h) = r.render_rgba(&doc, comp_id, 0, 0.5).expect("render");
        assert_eq!((w, h), (8, 8), "half scale halves each dimension");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let idx = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(rgba[idx + 1] > 200, "green solid stays green after resize");
    }

    /// The zero-copy path (K-177) renders a real comp into a shared GPU texture
    /// and reports a non-zero NT handle whose dimensions are stable across two
    /// frames (the texture is re-used, not re-created). Skips when there is no
    /// GPU adapter; also skips calmly if this machine's wgpu is not on the D3D12
    /// backend (the shared path needs D3D12 — the read-back path still works).
    #[cfg(all(windows, feature = "shared-texture"))]
    #[test]
    fn solid_comp_renders_to_a_stable_shared_handle() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let (store, comp_id) = doc_with_solid(LinearColour([0.0, 0.0, 1.0, 1.0]), 32, 16);
        let doc = store.snapshot();
        let first = match r.render_to_shared(&doc, comp_id, 0) {
            Ok(info) => info,
            Err(e) => {
                // e.g. wgpu chose Vulkan over D3D12, or no shared-heap support.
                eprintln!("skipping: shared texture unavailable here: {e}");
                return;
            }
        };
        assert_ne!(first.handle, 0, "a shared render yields a non-zero handle");
        assert_eq!((first.width, first.height), (32, 16));
        assert_eq!(first.format, "rgba8888");

        // A second frame re-uses the same texture: same dimensions, same handle.
        let second = r
            .render_to_shared(&doc, comp_id, 1)
            .expect("second shared render");
        assert_eq!((second.width, second.height), (32, 16));
        assert_eq!(
            second.handle, first.handle,
            "the handle is stable while the comp size is unchanged"
        );
    }

    /// An unknown comp id on the shared path is a calm error, never a panic.
    #[cfg(all(windows, feature = "shared-texture"))]
    #[test]
    fn unknown_comp_is_an_error_on_the_shared_path() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let (store, _comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 4, 4);
        let doc = store.snapshot();
        assert!(r.render_to_shared(&doc, Uuid::now_v7(), 0).is_err());
    }

    /// An unknown comp id is a calm error, never a panic.
    #[test]
    fn unknown_comp_is_an_error() {
        let mut r = match HeadlessRenderer::new() {
            Ok(r) => r,
            Err(_) => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        let (store, _comp_id) = doc_with_solid(LinearColour([1.0, 1.0, 1.0, 1.0]), 4, 4);
        let doc = store.snapshot();
        let err = r.render_rgba(&doc, Uuid::now_v7(), 0, 1.0);
        assert!(err.is_err(), "an unknown comp id yields an error");
    }
}

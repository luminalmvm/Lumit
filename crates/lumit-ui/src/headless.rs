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

use crate::export::{ItemInfo, Renderer};
use lumit_core::model::{Document, FootageItem, ProjectItem};
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
    /// The `ItemInfo` map the renderer reads, rebuilt each call (cheap — it only
    /// reads `probe_cache`) so a missing item's slate matches the current comp.
    items: HashMap<Uuid, ItemInfo>,
    /// Probe results by footage id, so each file is probed at most once.
    probe_cache: HashMap<Uuid, Probe>,
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
        Ok(Self {
            gpu,
            parts: Some(parts),
            items: HashMap::new(),
            probe_cache: HashMap::new(),
        })
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

//! Application state behind the shell: the document store, project path,
//! journal, dirty tracking, autosave. Slice 3 of docs/impl/phase-0-kickoff.md.

use lumit_core::model::{Composition, Document, FootageItem, LinearColour, MediaRef, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_core::DocumentStore;
use lumit_project::JournalFile;
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

pub const AUTOSAVE_INTERVAL_SECS: u64 = 300;
pub const AUTOSAVE_KEEP: usize = 5;

/// Latest-wins background frame decoding for the Viewer (slice 5).
/// In plain terms: the UI sends "show frame N of item X" requests down a
/// channel; a worker thread owns the decoders and answers with pixels; stale
/// requests are simply skipped (the epoch/latest-wins idea from
/// docs/impl/playback-scheduler.md, in miniature).
#[cfg(feature = "media")]
pub mod preview {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
    use std::sync::Arc;
    use uuid::Uuid;

    pub struct FramePixels {
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
        pub frame: usize,
        pub item: Uuid,
    }

    struct Request {
        generation: u64,
        item: Uuid,
        path: PathBuf,
        frame: usize,
        target_width: Option<u32>,
    }

    /// One layer's decode job inside a comp render request.
    pub struct CompJob {
        pub layer: Uuid,
        pub item: Uuid,
        pub path: PathBuf,
        pub source_frame: usize,
        pub target_width: Option<u32>,
        /// The source's native pixel size, independent of the decode width.
        /// Transforms act in comp pixels, so this — not the decoded size —
        /// sizes the layer (auto res must not scale geometry with zoom).
        pub natural_w: u32,
        pub natural_h: u32,
        /// Frame interpolation: `Some((ceil_frame, weight))` pairs
        /// `source_frame` with `ceil_frame` at `weight` (K-021 Blend/Flow).
        pub blend: Option<(usize, f32)>,
        /// When true, `blend`'s pair is combined with optical-flow synthesis
        /// rather than a plain crossfade (K-021 Flow policy).
        pub flow: bool,
        /// Full-resolution flow fields (FlowParams.half_resolution = false).
        pub flow_full: bool,
        /// Neighbour source frames a temporal effect stack needs (echo, flow
        /// motion blur, datamosh): `(offset, source_frame)`, one per non-zero
        /// offset in the stack's temporal window. Empty for a plain layer, so
        /// a single-frame stack decodes exactly one frame.
        pub temporal: Vec<(i32, usize)>,
        /// True when the stack has a flow-consuming effect (Flow motion blur,
        /// docs/08 §3.2): the decode worker measures the dense motion between
        /// this frame and the +1 neighbour (already fetched via `temporal`)
        /// and stamps it onto [`CompLayerPixels::flow_field`].
        pub wants_flow: bool,
    }

    pub struct CompLayerPixels {
        pub layer: Uuid,
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
        /// Native source size (see [`CompJob::natural_w`]); drives geometry.
        pub natural_w: u32,
        pub natural_h: u32,
        /// Decoded neighbour frames for a temporal effect (see
        /// [`CompJob::temporal`]): `(offset, rgba)`, same size as `rgba`.
        pub temporal: Vec<(i32, Vec<u8>)>,
        /// Dense forward flow (per-pixel `(u, v)` motion in pixels, row-major,
        /// same `width × height` as `rgba`) between this frame and the next,
        /// present only when [`CompJob::wants_flow`] and the +1 neighbour
        /// decoded. Flow motion blur (docs/08 §3.2) smears along it.
        pub flow_field: Option<(Vec<f32>, Vec<f32>)>,
    }

    pub struct CompFrame {
        pub comp: Uuid,
        pub frame: usize,
        /// Top-of-stack first (document order); the renderer draws bottom-up.
        pub layers: Vec<CompLayerPixels>,
    }

    pub enum PreviewResult {
        Footage(FramePixels),
        Comp(CompFrame),
    }

    pub struct PreviewEngine {
        tx: Sender<Message>,
        pub results: Receiver<Result<PreviewResult, String>>,
        generation: Arc<AtomicU64>,
    }

    enum Message {
        Footage(Request),
        Comp {
            generation: u64,
            comp: Uuid,
            frame: usize,
            jobs: Vec<CompJob>,
        },
    }

    impl Default for PreviewEngine {
        fn default() -> Self {
            let (tx, rx) = channel::<Message>();
            let (result_tx, results) = channel();
            let generation = Arc::new(AtomicU64::new(0));
            let live = generation.clone();
            std::thread::spawn(move || {
                let mut decoders: HashMap<Uuid, lumit_media::VideoDecoder> = HashMap::new();
                // Decoded-frame RAM cache (K-016 tier seed): recently shown
                // frames re-display instantly instead of re-decoding.
                let mut frame_cache: lumit_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame> =
                    lumit_cache::ByteLru::new(512 * 1024 * 1024);
                // Flow backend, created on the first Flow-policy frame: its
                // own headless GPU when one exists, the CPU oracle otherwise
                // (lumit-flow degrades by itself — never a fault).
                let mut flow_engine: Option<lumit_flow::FlowEngine> = None;
                loop {
                    // Block for one request, then drain to the newest (latest wins).
                    let mut req = match rx.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    };
                    loop {
                        match rx.try_recv() {
                            Ok(newer) => req = newer,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let generation = match &req {
                        Message::Footage(r) => r.generation,
                        Message::Comp { generation, .. } => *generation,
                    };
                    if generation != live.load(Ordering::Relaxed) {
                        continue; // superseded while queued
                    }
                    let result = match req {
                        Message::Footage(r) => {
                            decode(&mut decoders, &mut frame_cache, &r).map(PreviewResult::Footage)
                        }
                        Message::Comp {
                            comp, frame, jobs, ..
                        } => decode_comp(
                            &mut decoders,
                            &mut frame_cache,
                            &mut flow_engine,
                            comp,
                            frame,
                            &jobs,
                        )
                        .map(PreviewResult::Comp),
                    };
                    let _ = result_tx.send(result);
                }
            });
            Self {
                tx,
                results,
                generation,
            }
        }
    }

    struct CachedFrame {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    impl lumit_cache::ByteSized for CachedFrame {
        fn byte_size(&self) -> usize {
            self.rgba.len() + 16
        }
    }

    fn decode(
        decoders: &mut HashMap<Uuid, lumit_media::VideoDecoder>,
        cache: &mut lumit_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
        req: &Request,
    ) -> Result<FramePixels, String> {
        let cache_key = (req.item, req.frame, req.target_width);
        if let Some(hit) = cache.get(&cache_key) {
            return Ok(FramePixels {
                width: hit.width,
                height: hit.height,
                rgba: hit.rgba.clone(),
                frame: req.frame,
                item: req.item,
            });
        }
        let dec = match decoders.entry(req.item) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let index =
                    lumit_media::index::build_frame_index(&req.path).map_err(|e| e.to_string())?;
                let dec =
                    lumit_media::VideoDecoder::open(&req.path, index).map_err(|e| e.to_string())?;
                e.insert(dec)
            }
        };
        let frame = req.frame.min(dec.frame_count().saturating_sub(1));
        let out = dec
            .frame_rgba(frame, req.target_width)
            .map_err(|e| e.to_string())?;
        cache.insert(
            cache_key,
            CachedFrame {
                width: out.width,
                height: out.height,
                rgba: out.rgba.clone(),
            },
        );
        Ok(FramePixels {
            width: out.width,
            height: out.height,
            rgba: out.rgba,
            frame,
            item: req.item,
        })
    }

    impl PreviewEngine {
        /// Ask for a frame; any not-yet-decoded older request is abandoned.
        pub fn request(&self, item: Uuid, path: PathBuf, frame: usize, target_width: Option<u32>) {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = self.tx.send(Message::Footage(Request {
                generation,
                item,
                path,
                frame,
                target_width,
            }));
        }

        /// Ask for every layer frame of a comp at one comp frame (latest wins).
        pub fn request_comp(&self, comp: Uuid, frame: usize, jobs: Vec<CompJob>) {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = self.tx.send(Message::Comp {
                generation,
                comp,
                frame,
                jobs,
            });
        }
    }

    fn decode_comp(
        decoders: &mut HashMap<Uuid, lumit_media::VideoDecoder>,
        cache: &mut lumit_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
        flow_engine: &mut Option<lumit_flow::FlowEngine>,
        comp: Uuid,
        frame: usize,
        jobs: &[CompJob],
    ) -> Result<CompFrame, String> {
        let mut layers = Vec::with_capacity(jobs.len());
        for job in jobs {
            let req = Request {
                generation: 0,
                item: job.item,
                path: job.path.clone(),
                frame: job.source_frame,
                target_width: job.target_width,
            };
            let px = decode(decoders, cache, &req)?;
            // Neighbour frames for a temporal effect (job.temporal is empty
            // for a plain layer, so this loop does nothing then). A neighbour
            // that fails to decode is simply dropped — a missing echo tap
            // degrades the effect, never the frame.
            let temporal: Vec<(i32, Vec<u8>)> = job
                .temporal
                .iter()
                .filter_map(|&(offset, frame)| {
                    let nreq = Request {
                        generation: 0,
                        item: job.item,
                        path: job.path.clone(),
                        frame,
                        target_width: job.target_width,
                    };
                    decode(decoders, cache, &nreq)
                        .ok()
                        .map(|p| (offset, p.rgba))
                })
                .collect();
            // Flow motion blur (docs/08 §3.2) needs a dense motion field: the
            // forward flow from this frame to the next (offset +1, decoded
            // above). Computed from the raw current frame before it is consumed
            // into `rgba` below, where both frames live as RGBA — exactly as
            // the Flow retiming policy computes its flow, on the shared engine
            // that reuses the GPU when one is present. A dropped +1 neighbour
            // just leaves it None, and motion blur degrades to a passthrough.
            let flow_field = if job.wants_flow {
                temporal.iter().find(|(o, _)| *o == 1).map(|(_, next)| {
                    let (w, h) = (px.width as usize, px.height as usize);
                    let ga = lumit_flow::to_gray(&px.rgba, w, h);
                    let gb = lumit_flow::to_gray(next, w, h);
                    let (fwd, _bwd) = flow_engine
                        .get_or_insert_with(lumit_flow::FlowEngine::new_auto)
                        .flow_pair(&ga, &gb);
                    (fwd.u, fwd.v)
                })
            } else {
                None
            };
            // Blend / Flow policy: combine with the next source frame.
            let rgba = if let Some((ceil, w)) = job.blend {
                let req2 = Request {
                    generation: req.generation,
                    item: req.item,
                    path: job.path.clone(),
                    frame: ceil,
                    target_width: req.target_width,
                };
                let px2 = decode(decoders, cache, &req2)?;
                if job.flow {
                    let quality = if job.flow_full {
                        lumit_flow::FlowQuality::Full
                    } else {
                        lumit_flow::FlowQuality::Half
                    };
                    flow_engine
                        .get_or_insert_with(lumit_flow::FlowEngine::new_auto)
                        .interpolate_at(
                            &px.rgba,
                            &px2.rgba,
                            px.width as usize,
                            px.height as usize,
                            w,
                            quality,
                        )
                } else {
                    crate::pixels::blend_rgba(&px.rgba, &px2.rgba, w)
                }
            } else {
                px.rgba
            };
            layers.push(CompLayerPixels {
                layer: job.layer,
                width: px.width,
                height: px.height,
                rgba,
                natural_w: job.natural_w,
                natural_h: job.natural_h,
                temporal,
                flow_field,
            });
        }
        Ok(CompFrame {
            comp,
            frame,
            layers,
        })
    }
}

/// Probe/index results for footage items, filled by background threads.
#[cfg(feature = "media")]
pub mod media {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use uuid::Uuid;

    pub enum MediaStatus {
        Probing,
        Ready {
            probe: lumit_media::MediaProbe,
            frames: usize,
            vfr: bool,
        },
        Failed(String),
    }

    pub struct MediaRegistry {
        pub map: HashMap<Uuid, MediaStatus>,
        tx: Sender<(Uuid, MediaStatus)>,
        rx: Receiver<(Uuid, MediaStatus)>,
    }

    impl Default for MediaRegistry {
        fn default() -> Self {
            let (tx, rx) = channel();
            Self {
                map: HashMap::new(),
                tx,
                rx,
            }
        }
    }

    impl MediaRegistry {
        /// Drain background results into the map. Called once per UI frame.
        pub fn poll(&mut self) {
            while let Ok((id, status)) = self.rx.try_recv() {
                self.map.insert(id, status);
            }
        }

        pub fn any_probing(&self) -> bool {
            self.map.values().any(|s| matches!(s, MediaStatus::Probing))
        }

        /// Probe + build/load the frame index on a background thread
        /// (docs/impl/media-io.md §2 — never on the UI thread, K-017).
        pub fn spawn_probe(&mut self, id: Uuid, path: PathBuf) {
            self.map.insert(id, MediaStatus::Probing);
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let status = probe_and_index(&path);
                let _ = tx.send((id, status));
            });
        }
    }

    fn probe_and_index(path: &std::path::Path) -> MediaStatus {
        let probe = match lumit_media::probe::probe(path) {
            Ok(p) => p,
            Err(e) => return MediaStatus::Failed(e.to_string()),
        };
        // Audio-only items need no frame index.
        if probe.video.is_none() {
            return MediaStatus::Ready {
                probe,
                frames: 0,
                vfr: false,
            };
        }
        let cache_dir = lumit_project::media_index_dir();
        let cached = match (&cache_dir, lumit_media::Fingerprint::of(path)) {
            (Some(dir), Ok(fp)) => lumit_media::FrameIndex::load_cached(dir, &fp),
            _ => None,
        };
        let index = match cached {
            Some(index) => index,
            None => match lumit_media::index::build_frame_index(path) {
                Ok(index) => {
                    if let Some(dir) = &cache_dir {
                        let _ = index.save_to(dir);
                    }
                    index
                }
                Err(e) => return MediaStatus::Failed(e.to_string()),
            },
        };
        MediaStatus::Ready {
            probe,
            frames: index.frame_count(),
            vfr: index.vfr,
        }
    }
}

/// While the user is actively scrubbing or dragging, footage decodes at most
/// this wide so a frame comes back fast (the specified resolution reloads the
/// moment they stop). Chosen to keep even 4K sources instant to draft.
const DRAFT_MAX_WIDTH: u32 = 640;

/// Infallible constructor for small literal rationals.
/// One decode-width policy for requests AND cache keys — if these ever
/// disagreed, a cached frame could present at the wrong resolution. `draft`
/// caps the width for instant feedback and never exceeds the specified tier.
fn decode_target_width(
    natural_w: u32,
    draft: bool,
    auto_res: bool,
    display_scale: f32,
    divisor: u32,
) -> Option<u32> {
    let specified = if auto_res {
        let scale = display_scale.clamp(0.05, 1.0);
        let w = (natural_w as f32 * scale).round() as u32;
        (w < natural_w).then_some(w.max(16))
    } else {
        (divisor > 1).then(|| natural_w / divisor)
    };
    if draft {
        // Never coarser than needed: cap the specified width, never raise it.
        let w = specified.unwrap_or(natural_w).min(DRAFT_MAX_WIDTH);
        return (w < natural_w).then_some(w.max(16));
    }
    specified
}

/// Frame visit order for the idle background cache fill: the playhead first,
/// then a forward-biased walk — roughly three frames ahead of the playhead for
/// every one behind — because playback and scrubbing usually head forwards, so
/// the frames most likely to be viewed next should cache first (Mack). Every
/// work-area frame appears exactly once.
fn fill_walk_order(playhead: usize, start: usize, end: usize) -> Vec<usize> {
    let mut order = Vec::new();
    if end <= start || playhead < start || playhead >= end {
        return order;
    }
    let span = end - start;
    order.push(playhead);
    let (mut ahead, mut behind) = (1usize, 1usize);
    let mut k = 0usize;
    while order.len() < span && k < span * 2 + 8 {
        // One behind for every three ahead; when a side is exhausted the other
        // takes over so every frame is still visited.
        let want_behind = k % 4 == 3;
        let forward = playhead + ahead;
        if !want_behind && forward < end {
            order.push(forward);
            ahead += 1;
        } else if let Some(f) = playhead.checked_sub(behind).filter(|f| *f >= start) {
            order.push(f);
            behind += 1;
        } else if forward < end {
            order.push(forward);
            ahead += 1;
        }
        k += 1;
    }
    order
}

/// Frames to warm ahead of the playhead during playback: the bounded forward
/// window `[playhead + 1, playhead + lookahead]`, clamped to the work-area end
/// (`end` exclusive). Playback presentation chases the audio clock, so warming
/// a little ahead of it keeps the work-area loop smooth once frames are cached
/// (docs/impl/playback-scheduler.md §5). Empty once the playhead reaches the end.
fn playback_lookahead(playhead: usize, end: usize, lookahead: usize) -> Vec<usize> {
    let first = playhead.saturating_add(1);
    let stop = first.saturating_add(lookahead).min(end);
    (first..stop).collect()
}

/// Pan-behind: the position that keeps a layer visually fixed when its origin
/// (anchor) moves from `anchor` to `new_anchor`. Position places the anchor in
/// comp space, so shifting the anchor by Δ in layer space must shift position
/// by the layer's scale·rotation applied to Δ (docs/01-GLOSSARY.md anchor).
pub fn pan_behind_position(
    anchor: (f64, f64),
    new_anchor: (f64, f64),
    position: (f64, f64),
    scale_pct: (f64, f64),
    rotation_deg: f64,
) -> (f64, f64) {
    let vx = (new_anchor.0 - anchor.0) * scale_pct.0 / 100.0;
    let vy = (new_anchor.1 - anchor.1) * scale_pct.1 / 100.0;
    let (sin, cos) = rotation_deg.to_radians().sin_cos();
    (
        position.0 + vx * cos - vy * sin,
        position.1 + vx * sin + vy * cos,
    )
}

/// A transform whose origin (anchor) is the centre of a `nat_w`×`nat_h`
/// object, placed at the centre of a `comp_w`×`comp_h` composition — the AE
/// default so a new layer appears centred and pivots about its middle.
fn centred_transform(
    nat_w: f64,
    nat_h: f64,
    comp_w: u32,
    comp_h: u32,
) -> lumit_core::model::TransformGroup {
    use lumit_core::anim::Property;
    lumit_core::model::TransformGroup {
        anchor_x: Property::fixed(nat_w * 0.5),
        anchor_y: Property::fixed(nat_h * 0.5),
        position_x: Property::fixed(f64::from(comp_w) * 0.5),
        position_y: Property::fixed(f64::from(comp_h) * 0.5),
        ..Default::default()
    }
}

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap_or(Rational::ZERO)
}

/// The composition settings dialogue (AE: Composition Settings): used both
/// for creating a comp (editing = None) and editing one later. Opened with
/// footage-matched defaults when a drop starts the project's first comp.
/// The parametric shape the shape tool draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ShapeKind {
    #[default]
    Rectangle,
    Ellipse,
    Star,
}

impl ShapeKind {
    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Star => "Star",
        }
    }
}

/// What a pointer drag/click does in the Viewer (the toolbar's mouse mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ToolMode {
    /// Click selects; drag pans (object selection arrives with the object
    /// tools — for now Select pans like Hand so the view stays navigable).
    #[default]
    Select,
    /// Drag pans the view (the hand).
    Hand,
    /// Drag rubber-bands a new mask of the current [`ShapeKind`].
    Shape,
    /// Click places mask vertices (pen).
    Pen,
}

pub struct CompDialog {
    /// Some = editing an existing comp; None = creating a new one.
    pub editing: Option<Uuid>,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_s: f64,
    /// Keep width:height fixed: editing one dimension rescales the other.
    pub lock_ratio: bool,
    /// The locked aspect (width / height), captured when the lock engages.
    pub aspect: f64,
    /// Item to add as the first layer once the comp exists (drag-drop with
    /// no comp yet).
    pub pending_item: Option<Uuid>,
}

/// The lane guide-line mode — what the faint vertical lines mark.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimelineGrid {
    /// Detected beats and markers (the montage default).
    Beats,
    /// The time grid: seconds, subdividing to frames as the zoom allows.
    Time,
    /// No guide lines.
    Off,
}

/// Cache-bar memo key: (document snapshot ptr, cache epoch, quality tag,
/// comp id, disk-set size) — the bar is stale iff any of these moved.
#[cfg(feature = "media")]
type CacheBarKey = (usize, u64, u32, Uuid, usize);

/// A frame's cache-bar tier (docs/06 §5.6): green plays now, blue promotes.
#[cfg(feature = "media")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    None,
    /// In RAM at current quality — plays in real time now (green).
    Ram,
    /// On disk only — promotable, not yet playable (blue).
    Disk,
}

/// The disk tier's IO side (docs/06 §5.4): one background thread owns the
/// [`lumit_cache::disk::DiskCache`] so the UI thread never touches the
/// filesystem. Writes are fire-and-forget (write-behind); loads come back
/// through a channel and are folded into the RAM tier each frame. The shared
/// `known` set mirrors which hashes exist on disk, for the cache bar's blue
/// tier and the fill scheduler's promote-before-render choice.
#[cfg(feature = "media")]
pub mod diskio {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Arc, Mutex};

    /// Default disk budget (docs/06 §5.4; user-set cap arrives with settings).
    pub const DEFAULT_CAP_BYTES: u64 = 50 * 1024 * 1024 * 1024;

    pub enum Cmd {
        /// Point the cache at a project's sidecar (None = unsaved: disabled).
        SetRoot(Option<PathBuf>),
        /// Park a rendered frame (write-behind).
        Store(u128, u32, u32, Vec<u8>),
        /// Bring a frame back for the RAM tier.
        Load(u128),
    }

    pub struct DiskIo {
        pub tx: Sender<Cmd>,
        pub loaded: Receiver<(u128, lumit_cache::disk::DiskFrame)>,
        /// Hashes present on disk, mirrored by the worker.
        pub known: Arc<Mutex<HashSet<u128>>>,
    }

    /// Spawn the worker. It exits when the sender side drops.
    pub fn spawn() -> DiskIo {
        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        let (loaded_tx, loaded) = std::sync::mpsc::channel();
        let known: Arc<Mutex<HashSet<u128>>> = Arc::default();
        let known_worker = known.clone();
        std::thread::Builder::new()
            .name("nebula-disk".into())
            .spawn(move || {
                let mut cache: Option<lumit_cache::disk::DiskCache> = None;
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::SetRoot(root) => {
                            cache = root
                                .map(|r| lumit_cache::disk::DiskCache::open(r, DEFAULT_CAP_BYTES));
                            let hashes =
                                cache.as_ref().map(|c| c.known_hashes()).unwrap_or_default();
                            if let Ok(mut k) = known_worker.lock() {
                                k.clear();
                                k.extend(hashes);
                            }
                        }
                        Cmd::Store(hash, w, h, rgba) => {
                            if let Some(c) = &mut cache {
                                c.store(hash, w, h, &rgba);
                                if c.contains(hash) {
                                    if let Ok(mut k) = known_worker.lock() {
                                        k.insert(hash);
                                    }
                                }
                            }
                        }
                        Cmd::Load(hash) => {
                            let frame = cache.as_mut().and_then(|c| c.load(hash));
                            match frame {
                                Some(f) => {
                                    let _ = loaded_tx.send((hash, f));
                                }
                                None => {
                                    // Missing or corrupt-discarded: unmirror it
                                    // so the fill falls back to rendering.
                                    if let Ok(mut k) = known_worker.lock() {
                                        k.remove(&hash);
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .ok();
        DiskIo { tx, loaded, known }
    }
}

/// One display-ready comp frame in Kura's RAM tier (sRGB bytes as shown and
/// as exported — the same pixels, K-031).
#[cfg(feature = "media")]
pub struct CachedCompFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(feature = "media")]
impl lumit_cache::ByteSized for CachedCompFrame {
    fn byte_size(&self) -> usize {
        self.rgba.len() + 16
    }
}

/// See [`AppState::stamper`].
#[cfg(feature = "media")]
pub struct PreviewStamper<'a> {
    doc: &'a Document,
    media: &'a media::MediaRegistry,
    auto_res: bool,
    display_scale: f32,
    divisor: u32,
}

#[cfg(feature = "media")]
impl lumit_eval::SourceStamper for PreviewStamper<'_> {
    fn stamp(&self, item: Uuid, lt: f64) -> Option<(String, u64)> {
        let Some(ProjectItem::Footage(f)) = self.doc.item(item) else {
            return None;
        };
        let media::MediaStatus::Ready { probe, frames, .. } = self.media.map.get(&item)? else {
            return None;
        };
        let video = probe.video.as_ref()?;
        let source_frame =
            ((lt * video.fps()).round().max(0.0) as usize).min(frames.saturating_sub(1));
        // Key at the specified resolution: draft frames are never cached, so
        // the content-hash key always represents the settled resolution.
        let target = decode_target_width(
            video.width,
            false,
            self.auto_res,
            self.display_scale,
            self.divisor,
        );
        Some((
            format!("{}#w{}", f.media.absolute_path, target.unwrap_or(0)),
            source_frame as u64,
        ))
    }
}

/// A recovery offer: the saved document plus the journal ops beyond it.
pub struct PendingRecovery {
    pub doc: Document,
    pub path: PathBuf,
    pub ops: Vec<Op>,
}

/// Beat-analysis result handed back from the worker: (comp, bpm, onsets).
#[cfg(feature = "media")]
type BeatMsg = (Uuid, f64, Vec<(f64, f32)>);

/// A marquee selection in the graph editor: which keyframes of which channel
/// are selected. In plain terms: the box you drag over a curve remembers its
/// keys here, and every entry is pinned to both its index *and* the time the
/// key had when it was selected — if any other edit inserts, removes or
/// re-orders keys, the pins no longer line up and the whole selection reads
/// as stale (and clears) instead of ever editing the wrong keyframes.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphSelection {
    /// The layer whose curve the selection was made on.
    pub layer: Uuid,
    /// The transform property (channel) the indices refer to. Ignored when
    /// `retime` is set — the Retime Time channel isn't a transform property.
    pub prop: lumit_core::model::TransformProp,
    /// True when the selection was made on the footage layer's Retime Time
    /// channel (K-078) rather than a transform property, so a selection on one
    /// never leaks onto the other.
    pub retime: bool,
    /// (keyframe index, its time when selected), ascending by index.
    pub keys: Vec<(usize, Rational)>,
}

impl GraphSelection {
    /// The selected indices, if every pin still lines up with `keys`; `None`
    /// means the selection is stale (the keyframe list changed underneath).
    pub fn indices_for(&self, keys: &[lumit_core::anim::Keyframe]) -> Option<Vec<usize>> {
        self.keys
            .iter()
            .map(|&(i, t)| keys.get(i).filter(|k| k.time == t).map(|_| i))
            .collect()
    }
}

pub struct AppState {
    pub store: DocumentStore,
    pub path: Option<PathBuf>,
    journal: Option<JournalFile>,
    pub dirty: bool,
    pub selected_comp: Option<Uuid>,
    /// Item highlighted in the Project panel (any kind, not just comps).
    pub selected_item: Option<Uuid>,
    /// Open composition-settings dialogue, if any.
    pub comp_dialog: Option<CompDialog>,
    pub pending_recovery: Option<PendingRecovery>,
    pub error: Option<String>,
    #[cfg(feature = "media")]
    pub media: media::MediaRegistry,
    #[cfg(feature = "media")]
    pub preview_engine: preview::PreviewEngine,
    #[cfg(feature = "media")]
    audio_engine: Option<lumit_audio::AudioEngine>,
    #[cfg(feature = "media")]
    audio_cache: std::collections::HashMap<Uuid, std::sync::Arc<lumit_media::AudioBuffer>>,
    #[cfg(feature = "media")]
    audio_loaded: Option<Uuid>,
    #[cfg(feature = "media")]
    audio_rx: std::sync::mpsc::Receiver<(Uuid, Result<lumit_media::AudioBuffer, String>)>,
    #[cfg(feature = "media")]
    audio_tx: std::sync::mpsc::Sender<(Uuid, Result<lumit_media::AudioBuffer, String>)>,
    /// The comp whose mixed audio is loaded in the engine (drives its clock).
    #[cfg(feature = "media")]
    audio_loaded_comp: Option<Uuid>,
    /// Background-mixed comp audio arriving from the prepare thread.
    #[cfg(feature = "media")]
    comp_audio_rx: std::sync::mpsc::Receiver<(Uuid, lumit_media::AudioBuffer)>,
    #[cfg(feature = "media")]
    comp_audio_tx: std::sync::mpsc::Sender<(Uuid, lumit_media::AudioBuffer)>,
    /// Detected beats (comp id, bpm, (time_s, confidence)…) from the analysis
    /// thread.
    #[cfg(feature = "media")]
    beats_rx: std::sync::mpsc::Receiver<BeatMsg>,
    #[cfg(feature = "media")]
    beats_tx: std::sync::mpsc::Sender<BeatMsg>,
    /// (comp id, estimated BPM) from the last beat detection, shown by the ruler.
    #[cfg(feature = "media")]
    pub detected_bpm: Option<(Uuid, f64)>,
    /// (comp id, (min,max) peaks) for the timeline waveform, computed when the
    /// comp's audio is mixed. Drawn under the ruler.
    #[cfg(feature = "media")]
    pub comp_waveform: Option<(Uuid, Vec<(f32, f32)>)>,
    /// In-flight property drag (layer, property, provisional value): commits
    /// once on release so a drag is ONE undo step, not hundreds.
    pub prop_edit: Option<(Uuid, lumit_core::model::TransformProp, f64)>,
    /// In-flight *linked* scale drag (layer, x%, y%): the live preview needs
    /// both axes, since one drag moves both (else only x scales until release).
    pub scale_preview: Option<(Uuid, f64, f64)>,
    /// In-flight bar-edge trim: (layer, trimming_out_edge, provisional seconds).
    pub trim_edit: Option<(Uuid, bool, f64)>,
    /// In-flight whole-layer move (drag the bar body): (layer, provisional new
    /// in-point in comp seconds, unsnapped). On release the whole span shifts —
    /// in/out/start_offset together — so the content moves with the bar.
    pub move_edit: Option<(Uuid, f64)>,
    /// Layer whose properties the graph editor shows (clicked in the Timeline).
    pub selected_layer: Option<Uuid>,
    /// Selected clip within a Sequence layer (clicked sub-bar), for per-clip
    /// speed editing.
    pub selected_clip: Option<Uuid>,
    /// Mask vertex mid-drag in the Viewer: (mask index, vertex index,
    /// layer-space position). Committed as one SetLayerMasks op on release.
    pub mask_drag: Option<(usize, usize, (f64, f64))>,
    /// The active pointer tool (toolbar): what a Viewer drag/click does.
    pub tool: ToolMode,
    /// The shape the shape tool draws (its last-picked kind).
    pub shape_kind: ShapeKind,
    /// Shape-tool rubber-band start in layer space; Some while dragging.
    pub shape_drag: Option<(f64, f64)>,
    /// Origin (anchor) mid-drag in the Viewer: the new anchor in layer space.
    /// Committed as one Batch (anchor + pan-behind position) on release.
    pub origin_drag: Option<(f64, f64)>,
    /// The pen's in-progress path (layer space); closes into a mask when the
    /// first vertex is clicked again.
    pub pen_path: Vec<lumit_core::mask::Vertex>,
    /// Property shown in the graph editor.
    pub graph_prop: Option<lumit_core::model::TransformProp>,
    /// In-flight keyframe drag: (key index, provisional layer-time, value).
    pub graph_edit: Option<(usize, f64, f64)>,
    /// In-flight marquee (rubber-band) drag on the graph's background:
    /// (press anchor, current corner) in screen points. `Some` only while the
    /// mouse button is down; on release it becomes a `graph_selection`.
    pub graph_marquee: Option<(egui::Pos2, egui::Pos2)>,
    /// Keyframes selected in the graph editor — by the marquee, or the last
    /// dragged key. Pinned to one channel; see `GraphSelection`.
    pub graph_selection: Option<GraphSelection>,
    /// In-flight speed-graph drag: (key index, provisional speed in
    /// value-units/second). Separate from `graph_edit` because the speed lens
    /// edits a keyframe's tangent (K-070), not its value or time.
    pub graph_speed_edit: Option<(usize, f64)>,
    /// In-flight value-lens tangent-handle drag: (key index, out side?,
    /// provisional slope in value-units/second, provisional influence in (0, 1]).
    /// `out` chooses the forward or backward handle; the curve previews live and
    /// the release writes the bezier side(s) back (unified unless Alt-dragged).
    pub graph_tangent_edit: Option<(usize, bool, f64, f64)>,
    /// The in-flight tangent drag's mirroring mode: (was the key unified when
    /// the drag started, has Alt been held at any point since). Mirroring =
    /// XOR of the two (see `tangent_mirrors`): Alt toggles it once and latches,
    /// so a break survives releasing Alt, and Alt on a broken key re-unifies.
    pub graph_tangent_mode: Option<(bool, bool)>,
    /// A pending interpolation change for the graphed transform channel's keys
    /// (selection, or all keys when nothing is selected): set by F9 and the
    /// bottom-bar Linear/Bezier buttons, consumed by `graph_plot`.
    pub graph_set_interp: Option<lumit_core::anim::SideInterp>,
    /// Graph editor lens: false = value graph, true = speed graph
    /// (docs/01-GLOSSARY.md §3: two views of the same data, never separate).
    pub graph_speed_view: bool,
    /// Manual value-lens y-range `(min, max)` when the user has scrolled or
    /// zoomed the graph vertically (K-079). `None` = auto-fit to the curve (the
    /// default); the bottom-bar Fit toggle clears it back to `None`.
    pub graph_view_y: Option<(f64, f64)>,
    /// Whether the value graph keeps re-fitting its y-range to the curve every
    /// frame (the bottom-bar Fit toggle, on by default). A vertical wheel,
    /// Ctrl-wheel zoom or scrollbar drag switches it off and takes over via
    /// `graph_view_y`; switching it back on clears the manual range.
    pub graph_auto_fit: bool,
    /// The plot height (px) the current manual `graph_view_y` was framed at.
    /// When the timeline panel is resized the manual range grows or shrinks
    /// about its centre by the height ratio, so the value scale (units per
    /// pixel) holds — more height shows more curve, never a stretch. `None`
    /// while auto-fitting, or until `graph_plot` stamps the live height.
    pub graph_view_h: Option<f32>,
    /// The auto-fit y-range `graph_plot` computed last frame, so a first
    /// vertical scroll can seed a manual range from what's on screen (K-079).
    pub graph_last_fit: Option<(f64, f64)>,
    /// Graph the selected footage layer's Retime channel (K-075) rather than a
    /// transform property: value lens = source position as frame timecode,
    /// derivative lens = speed %.
    pub graph_retime: bool,
    /// Vegas-editor preference (K-075): the Speed/Retime channel opens to the
    /// speed-% (derivative) lens by default; off, to the frame-timecode lens.
    /// Session state for now — a persisted Settings home is a later refinement.
    pub vegas_default_lens: bool,
    /// What the faint vertical guide lines through the lanes mark: detected
    /// beats (default), the time grid (seconds, subdividing with zoom), or
    /// nothing. Session state, like the other timeline preferences.
    pub timeline_grid: TimelineGrid,
    /// In-flight speed-keyframe drag on the Retime channel's % lens (K-075, 2b):
    /// (keyframe index, provisional speed per cent). The retime rebuilds from the
    /// edited keyframe on release; downstream boundaries recompute (K-070).
    pub graph_retime_edit: Option<(usize, f64)>,
    /// Comp shown in the Viewer (takes precedence over preview_item).
    pub preview_comp: Option<Uuid>,
    /// Wall-clock comp playback v0 (the frame scheduler replaces this):
    /// (started, frame at start).
    pub comp_playback: Option<(Instant, usize)>,
    /// Footage item currently shown in the Viewer, and the scrub position.
    pub preview_item: Option<Uuid>,
    pub preview_frame: usize,
    /// Preview resolution divisor: 1 = Full, 2 = Half, 3 = Third, 4 = Quarter.
    /// Ignored while `preview_auto_res` is on.
    pub preview_divisor: u32,
    /// Auto resolution (K-030 family): decode at the size actually displayed,
    /// capped at 100% — zooming past 1:1 never upsamples the decode.
    pub preview_auto_res: bool,
    /// True while the user is actively scrubbing the playhead: the preview
    /// decodes a coarse draft for instant feedback, then reloads at the
    /// specified resolution once scrubbing stops (Mack's "force realtime").
    pub preview_draft: bool,
    /// View zoom (1.0 = fit) and pan, in screen pixels. View controls only —
    /// never part of any render (07-UI-SPEC: Viewer).
    pub view_zoom: f32,
    pub view_pan: egui::Vec2,
    /// Screen pixels per native image pixel at last paint (Auto res input).
    pub last_display_scale: f32,
    /// Draggable width of the timeline's left (layer-controls) column, px.
    pub timeline_name_w: f32,
    /// Lane-area horizontal view (07-UI-SPEC §4): zoom (1.0 = the whole comp fits
    /// the track width; larger zooms in) and the comp time at the left edge.
    /// Alt-wheel zooms, Shift-wheel scrolls; vertical scroll is the ScrollArea's.
    pub timeline_zoom: f64,
    pub timeline_view_start: f64,
    /// Timeline right area shows the graph editor (curves) instead of the
    /// layer bars — a mode of the Timeline, not a separate panel (K-070).
    pub timeline_graph_mode: bool,
    /// Kura's RAM tier for final comp frames (K-016): display-ready sRGB
    /// bytes keyed by content hash. Hash mismatch is the only invalidation.
    #[cfg(feature = "media")]
    pub comp_frame_cache: lumit_cache::ByteLru<u128, CachedCompFrame>,
    /// Bumped on every cache insert (cache-bar memo + repaint driver).
    #[cfg(feature = "media")]
    pub cache_epoch: u64,
    /// A warm frame the shell should present instead of waiting on a render.
    #[cfg(feature = "media")]
    pub cached_present: Option<u128>,
    /// The (comp, frame) currently rendering for the background cache fill.
    #[cfg(feature = "media")]
    pub fill_in_flight: Option<(Uuid, usize)>,
    /// The disk tier's IO worker (docs/06 §5.4), started lazily once the
    /// project has a path (unsaved projects have no sidecar to cache into).
    pub disk_io: Option<diskio::DiskIo>,
    /// The sidecar root the worker currently points at (memo, so the root is
    /// re-sent only when the project path actually changes).
    disk_root: Option<std::path::PathBuf>,
    /// Keys with a disk load in flight — suppresses duplicate load requests
    /// until the frame lands in RAM (drained each frame).
    disk_load_pending: std::collections::HashSet<u128>,
    /// Cache-bar memo: recomputed only when the memo key changes.
    #[cfg(feature = "media")]
    cache_bar_memo: Option<(CacheBarKey, std::sync::Arc<Vec<CacheTier>>)>,
    last_autosave: Instant,
    comp_counter: usize,
    /// Comps open as Timeline tabs, in tab order (07-UI-SPEC §4: one Timeline
    /// panel, one tab per open comp). `selected_comp` names the active tab and
    /// is always one of these when it is set. Session state, not saved in the
    /// document.
    pub open_comps: Vec<Uuid>,
    /// Set by the Timeline comp strip's context menu ("Pop out timeline"): a
    /// solo Timeline has no dock tab to host the pop-out button (K-086), and
    /// the strip renders deep inside the panel, so the request travels to the
    /// shell through here. Consumed each frame after the dock draws.
    pub pop_out_timeline: bool,
}

impl Default for AppState {
    fn default() -> Self {
        let doc = Document::new();
        let journal = JournalFile::for_document(doc.id);
        #[cfg(feature = "media")]
        let (audio_tx, audio_rx) = std::sync::mpsc::channel();
        #[cfg(feature = "media")]
        let (comp_audio_tx, comp_audio_rx) = std::sync::mpsc::channel();
        #[cfg(feature = "media")]
        let (beats_tx, beats_rx) = std::sync::mpsc::channel();
        Self {
            store: DocumentStore::new(doc),
            path: None,
            journal,
            dirty: false,
            selected_comp: None,
            pending_recovery: None,
            error: None,
            #[cfg(feature = "media")]
            media: media::MediaRegistry::default(),
            #[cfg(feature = "media")]
            preview_engine: preview::PreviewEngine::default(),
            #[cfg(feature = "media")]
            audio_engine: None,
            #[cfg(feature = "media")]
            audio_cache: std::collections::HashMap::new(),
            #[cfg(feature = "media")]
            audio_loaded: None,
            #[cfg(feature = "media")]
            audio_loaded_comp: None,
            #[cfg(feature = "media")]
            comp_audio_rx,
            #[cfg(feature = "media")]
            comp_audio_tx,
            #[cfg(feature = "media")]
            beats_rx,
            #[cfg(feature = "media")]
            beats_tx,
            #[cfg(feature = "media")]
            comp_waveform: None,
            #[cfg(feature = "media")]
            detected_bpm: None,
            #[cfg(feature = "media")]
            audio_rx,
            #[cfg(feature = "media")]
            audio_tx,
            prop_edit: None,
            scale_preview: None,
            trim_edit: None,
            move_edit: None,
            selected_layer: None,
            selected_clip: None,
            graph_prop: None,
            graph_edit: None,
            graph_marquee: None,
            graph_selection: None,
            graph_speed_edit: None,
            graph_tangent_edit: None,
            graph_tangent_mode: None,
            graph_set_interp: None,
            graph_speed_view: false,
            graph_view_y: None,
            graph_auto_fit: true,
            graph_view_h: None,
            graph_last_fit: None,
            graph_retime: false,
            vegas_default_lens: false,
            timeline_grid: TimelineGrid::Beats,
            graph_retime_edit: None,
            preview_comp: None,
            comp_playback: None,
            preview_item: None,
            preview_frame: 0,
            preview_divisor: 1,
            preview_auto_res: false,
            preview_draft: false,
            view_zoom: 1.0,
            view_pan: egui::Vec2::ZERO,
            last_display_scale: 1.0,
            timeline_name_w: 300.0,
            timeline_zoom: 1.0,
            timeline_view_start: 0.0,
            timeline_graph_mode: false,
            #[cfg(feature = "media")]
            comp_frame_cache: lumit_cache::ByteLru::new(512 * 1024 * 1024),
            #[cfg(feature = "media")]
            cache_epoch: 0,
            #[cfg(feature = "media")]
            cached_present: None,
            #[cfg(feature = "media")]
            fill_in_flight: None,
            disk_io: None,
            disk_root: None,
            disk_load_pending: std::collections::HashSet::new(),
            #[cfg(feature = "media")]
            cache_bar_memo: None,
            last_autosave: Instant::now(),
            comp_counter: 0,
            open_comps: Vec::new(),
            pop_out_timeline: false,
            selected_item: None,
            mask_drag: None,
            tool: ToolMode::default(),
            shape_kind: ShapeKind::default(),
            shape_drag: None,
            origin_drag: None,
            pen_path: Vec::new(),
            comp_dialog: None,
        }
    }
}

impl AppState {
    fn report<T>(&mut self, r: Result<T, impl std::fmt::Display>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// Back to auto-fit for the value graph (K-079): drop any manual y-range
    /// (and the plot height it was framed at) so the graph re-fits the curve
    /// continuously. Called when the graphed channel or lens changes — a fresh
    /// channel always starts fitted — and by the Fit toggle switching back on.
    pub fn graph_reset_fit(&mut self) {
        self.graph_auto_fit = true;
        self.graph_view_y = None;
        self.graph_view_h = None;
    }

    /// All document mutation funnels through here: commit, journal, dirty.
    pub fn commit(&mut self, op: Op) {
        match self.store.commit(op.clone()) {
            Ok(_) => {
                self.dirty = true;
                if let Some(journal) = &self.journal {
                    if let Err(e) = journal.append(&op) {
                        self.error = Some(format!("journal: {e}"));
                    }
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn undo(&mut self) {
        match self.store.undo() {
            Ok(Some(_)) => self.dirty = true,
            Ok(None) => {}
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn redo(&mut self) {
        match self.store.redo() {
            Ok(Some(_)) => self.dirty = true,
            Ok(None) => {}
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn install(&mut self, doc: Document, path: Option<PathBuf>, dirty: bool) {
        #[cfg(feature = "media")]
        for item in &doc.items {
            if let ProjectItem::Footage(f) = item {
                self.media
                    .spawn_probe(f.id, PathBuf::from(&f.media.absolute_path));
            }
        }
        self.journal = JournalFile::for_document(doc.id);
        self.selected_comp = doc.items.iter().find_map(|i| match i {
            ProjectItem::Composition(c) => Some(c.id),
            _ => None,
        });
        // Open the first comp as the sole Timeline tab; the rest open on demand.
        self.open_comps = self.selected_comp.into_iter().collect();
        self.preview_comp = None;
        self.preview_item = None;
        self.store = DocumentStore::new(doc);
        self.path = path;
        self.dirty = dirty;
        self.comp_counter = 0;
    }

    pub fn new_project(&mut self) {
        if let Some(journal) = &self.journal {
            let _ = journal.clear();
        }
        self.install(Document::new(), None, false);
    }

    pub fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Lumit project", &["kir"])
            .pick_file();
        if let Some(path) = picked {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        let Some((doc, _manifest)) = self.report(lumit_project::open(path)) else {
            return;
        };
        // Crash recovery: a non-empty journal for this document means the last
        // session ended without a save (docs/10-FILE-FORMAT.md §4).
        let ops = JournalFile::for_document(doc.id)
            .and_then(|j| j.read().ok())
            .unwrap_or_default();
        if ops.is_empty() {
            self.install(doc, Some(path.to_owned()), false);
        } else {
            self.pending_recovery = Some(PendingRecovery {
                doc,
                path: path.to_owned(),
                ops,
            });
        }
    }

    pub fn resolve_recovery(&mut self, recover: bool) {
        let Some(pending) = self.pending_recovery.take() else {
            return;
        };
        let mut doc = pending.doc;
        if recover {
            let mut replayed = 0usize;
            for op in &pending.ops {
                if lumit_core::ops::apply(&mut doc, op).is_err() {
                    break;
                }
                replayed += 1;
            }
            self.install(doc, Some(pending.path), true);
            if replayed < pending.ops.len() {
                self.error = Some(format!(
                    "recovered {replayed} of {} changes; the rest could not be replayed",
                    pending.ops.len()
                ));
            }
            // Journal stays until the user saves.
        } else {
            if let Some(journal) = JournalFile::for_document(doc.id) {
                let _ = journal.clear();
            }
            self.install(doc, Some(pending.path), false);
        }
    }

    pub fn save(&mut self) {
        let path = match &self.path {
            Some(p) => Some(p.clone()),
            None => rfd::FileDialog::new()
                .add_filter("Lumit project", &["kir"])
                .set_file_name("untitled.lum")
                .save_file(),
        };
        let Some(path) = path else { return };
        let doc = self.store.snapshot();
        if self.report(lumit_project::save(&doc, &path)).is_some() {
            if let Some(journal) = &self.journal {
                let _ = journal.clear();
            }
            self.path = Some(path);
            self.dirty = false;
        }
    }

    pub fn autosave_tick(&mut self) {
        if self.dirty
            && self.path.is_some()
            && self.last_autosave.elapsed().as_secs() >= AUTOSAVE_INTERVAL_SECS
        {
            self.last_autosave = Instant::now();
            if let Some(path) = self.path.clone() {
                let doc = self.store.snapshot();
                let _ = self.report(lumit_project::autosave(&doc, &path, AUTOSAVE_KEEP));
            }
        }
    }

    pub fn import_footage_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Media",
                &[
                    "mp4", "mov", "mkv", "avi", "webm", "png", "jpg", "jpeg", "wav", "mp3", "flac",
                ],
            )
            .pick_files();
        let Some(files) = picked else { return };
        self.import_paths(files);
    }

    /// Import media files (dialogue or drag-and-drop onto the window).
    pub fn import_paths(&mut self, files: Vec<PathBuf>) {
        let base = self.store.snapshot().items.len();
        for (i, file) in files.into_iter().enumerate() {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "footage".into());
            let item = FootageItem {
                id: Uuid::now_v7(),
                name: name.clone(),
                extra: serde_json::Map::new(),
                media: MediaRef {
                    relative_path: name,
                    absolute_path: file.to_string_lossy().into_owned(),
                    extra: serde_json::Map::new(),
                },
            };
            #[cfg(feature = "media")]
            let probe_target = (item.id, file.clone());
            self.commit(Op::AddItem {
                index: base + i,
                item: Box::new(ProjectItem::Footage(item)),
            });
            #[cfg(feature = "media")]
            self.media.spawn_probe(probe_target.0, probe_target.1);
        }
    }

    /// Add a footage item as a new top layer of the target comp
    /// (docs/16-ROADMAP.md phase 1: comps become buildable by hand).
    pub fn add_footage_to_comp(&mut self, item_id: Uuid) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(ProjectItem::Footage(f)) = doc.item(item_id) else {
            return;
        };

        // Span: media duration when known (frame-exact via the comp grid),
        // else the full comp.
        let comp_dur = comp.duration.0;
        #[cfg(feature = "media")]
        let out = match self.media.map.get(&item_id) {
            Some(media::MediaStatus::Ready { probe, .. }) => {
                let frames = (probe.duration_seconds * comp.frame_rate.fps()).round() as i64;
                comp.frame_rate
                    .time_of_frame(frames.max(1))
                    .map(|t| if t.0 < comp_dur { t.0 } else { comp_dur })
                    .unwrap_or(comp_dur)
            }
            _ => comp_dur,
        };
        #[cfg(not(feature = "media"))]
        let out = comp_dur;

        // Origin (anchor) at the footage's centre, placed at the comp centre —
        // so it appears centred and scales/rotates about its middle (AE model).
        #[cfg(feature = "media")]
        let (nat_w, nat_h) = match self.media.map.get(&item_id) {
            Some(media::MediaStatus::Ready { probe, .. }) => probe
                .video
                .as_ref()
                .map(|v| (f64::from(v.width), f64::from(v.height)))
                .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
            _ => (f64::from(comp.width), f64::from(comp.height)),
        };
        #[cfg(not(feature = "media"))]
        let (nat_w, nat_h) = (f64::from(comp.width), f64::from(comp.height));
        let transform = centred_transform(nat_w, nat_h, comp.width, comp.height);

        let layer = Layer {
            id: Uuid::now_v7(),
            name: f.name.clone(),
            kind: LayerKind::Footage {
                item: item_id,
                retime: None,
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(out),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Nest one comp inside another as a Precomp layer (cycle-guarded).
    pub fn add_precomp_to_comp(&mut self, nested_id: Uuid) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::time::CompTime;
        let Some(target_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        if target_id == nested_id || self.would_cycle(nested_id, target_id) {
            self.error = Some("that nesting would loop compositions".into());
            return;
        }
        let doc = self.store.snapshot();
        let (Some(target), Some(nested)) = (doc.comp(target_id), doc.comp(nested_id)) else {
            return;
        };
        let out = if nested.duration.0 < target.duration.0 {
            nested.duration.0
        } else {
            target.duration.0
        };
        let transform = centred_transform(
            f64::from(nested.width),
            f64::from(nested.height),
            target.width,
            target.height,
        );
        let layer = Layer {
            id: Uuid::now_v7(),
            name: nested.name.clone(),
            kind: LayerKind::Precomp { comp: nested_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(out),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: target_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(target_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Would nesting `nested` inside `target` create a cycle? True if target
    /// is reachable from nested through Precomp layers.
    fn would_cycle(&self, nested: Uuid, target: Uuid) -> bool {
        use lumit_core::model::LayerKind;
        let doc = self.store.snapshot();
        let mut stack = vec![nested];
        let mut seen = vec![];
        while let Some(id) = stack.pop() {
            if id == target {
                return true;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(c) = doc.comp(id) {
                for l in &c.layers {
                    if let LayerKind::Precomp { comp } = &l.kind {
                        stack.push(*comp);
                    }
                }
            }
        }
        false
    }

    /// Add a text layer with a starter document.
    pub fn add_text_layer(&mut self) {
        use lumit_core::model::{
            Layer, LayerKind, LinearColour, Switches, TextDocument, TransformGroup,
        };
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let transform = TransformGroup {
            position_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
            position_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Text".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: "Text".into(),
                    size: 72.0,
                    fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add a Camera layer at the comp centre. Default zoom follows the AE
    /// 50 mm model: comp width x 50/36 (full-frame film width in mm), so a
    /// fresh camera shows the comp exactly as it looked flat.
    pub fn add_camera_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, Switches, TransformGroup};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let transform = TransformGroup {
            position_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
            position_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Camera".into(),
            kind: LayerKind::Camera {
                zoom: lumit_core::anim::Property::fixed(f64::from(comp.width) * 50.0 / 36.0),
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform,
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add an adjustment layer at the top of the stack — a comp-sized effect
    /// container whose stack applies to everything beneath it within its span
    /// (docs/01-GLOSSARY.md), staged and blended by coverage as of K-091.
    pub fn add_adjustment_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, Switches, TransformGroup};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "Adjustment".into(),
            kind: LayerKind::Adjustment,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Add a Sequence layer (Vegas-style clip row). If a footage item is
    /// selected in the Project panel it becomes the first clip spanning the
    /// footage; otherwise the layer starts empty. This is a first, simple
    /// build path — richer clip editing (drag, cut, trim) follows.
    pub fn add_sequence_layer(&mut self) {
        use lumit_core::model::{Layer, LayerKind, Switches};
        use lumit_core::sequence::{Clip, ClipSource};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        // One clip from the selected footage item, if there is one.
        let mut clips = Vec::new();
        let mut name = "Sequence".to_string();
        if let Some(sel) = self.selected_item {
            if let Some(ProjectItem::Footage(f)) = doc.item(sel) {
                #[cfg(feature = "media")]
                let dur = match self.media.map.get(&sel) {
                    Some(media::MediaStatus::Ready { probe, .. }) => probe.duration_seconds,
                    _ => comp.duration.0.to_f64(),
                };
                #[cfg(not(feature = "media"))]
                let dur = comp.duration.0.to_f64();
                let dur = Rational::from_f64_on_grid(
                    dur.max(1.0 / comp.frame_rate.fps().max(1.0)),
                    Rational::FLICK_DEN,
                )
                .unwrap_or(comp.duration.0);
                clips.push(Clip::new(
                    ClipSource::Footage(sel),
                    Rational::ZERO,
                    dur,
                    Rational::ZERO,
                    dur,
                ));
                name = f.name.clone();
            }
        }
        let out = if let Some(c) = clips.first() {
            CompTime(c.place_end())
        } else {
            CompTime(comp.duration.0)
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name,
            kind: LayerKind::Sequence { clips },
            in_point: CompTime(Rational::ZERO),
            out_point: out,
            start_offset: CompTime(Rational::ZERO),
            transform: centred_transform(
                f64::from(comp.width),
                f64::from(comp.height),
                comp.width,
                comp.height,
            ),
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Convert the selected imported-footage layer into a sequenced layer
    /// (K-071): its one footage becomes a single clip you can then cut and
    /// retime. Only footage layers qualify. One undo step; the layer keeps its
    /// id, transform, masks and span, carrying any existing retime into the
    /// clip.
    pub fn convert_to_sequenced_layer(&mut self) {
        use lumit_core::model::LayerKind;
        use lumit_core::sequence::{Clip, ClipSource};
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a footage layer to convert".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(index) = comp.layers.iter().position(|l| l.id == layer_id) else {
            return;
        };
        let layer = &comp.layers[index];
        let LayerKind::Footage { item, retime } = &layer.kind else {
            self.error = Some("only footage layers convert to sequenced".into());
            return;
        };
        // Footage duration → the clip's source/place length.
        #[cfg(feature = "media")]
        let dur_s = match self.media.map.get(item) {
            Some(media::MediaStatus::Ready { probe, .. }) => probe.duration_seconds,
            _ => (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04),
        };
        #[cfg(not(feature = "media"))]
        let dur_s = (layer.out_point.0.to_f64() - layer.in_point.0.to_f64()).max(0.04);
        let dur = Rational::from_f64_on_grid(dur_s.max(0.04), Rational::FLICK_DEN)
            .unwrap_or(layer.out_point.0);
        let clip = Clip {
            id: Uuid::now_v7(),
            source: ClipSource::Footage(*item),
            source_in: Rational::ZERO,
            source_out: dur,
            place_start: Rational::ZERO,
            place_duration: dur,
            retime: retime
                .clone()
                .unwrap_or_else(|| lumit_core::retime::Retime::identity(dur, Rational::ZERO)),
            interpolation: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut new_layer = layer.clone();
        new_layer.kind = LayerKind::Sequence { clips: vec![clip] };
        // One undo step: drop the footage layer, add the sequenced one in its
        // place (same id and index, so it's a true in-place conversion).
        self.commit(Op::Batch {
            ops: vec![
                Op::RemoveLayer {
                    comp: comp_id,
                    layer: layer_id,
                },
                Op::AddLayer {
                    comp: comp_id,
                    index,
                    layer: Box::new(new_layer),
                },
            ],
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Razor: cut the selected Sequence layer's clip at the playhead into two
    /// (one undo step). The beat-sync covenant holds — clip places don't move.
    pub fn cut_sequence_at_playhead(&mut self) {
        use lumit_core::model::LayerKind;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a sequence layer to cut".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            self.error = Some("the razor needs a sequence layer".into());
            return;
        };
        // Exact layer-local cut time at the playhead.
        let Ok(comp_t) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let Ok(tau) = comp_t.0.checked_sub(layer.start_offset.0) else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.contains(tau.to_f64())) else {
            self.error = Some("no clip under the playhead".into());
            return;
        };
        let Some((left, right)) = clips[idx].cut(tau) else {
            self.error = Some("can't cut an eased ramp here yet".into());
            return;
        };
        let mut new_clips = clips.clone();
        new_clips.splice(idx..=idx, [left, right]);
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Set the selected clip's speed ramp (start/end percent; 100 = source
    /// rate) with an ease, keeping its place on the layer (beat-sync covenant).
    /// Equal start/end with a Linear ease is a plain constant speed.
    pub fn set_selected_clip_ramp(
        &mut self,
        v0_pct: f64,
        v1_pct: f64,
        ease: lumit_core::retime::Ease,
    ) {
        use lumit_core::model::LayerKind;
        let (Some(comp_id), Some(layer_id), Some(clip_id)) =
            (self.selected_comp, self.selected_layer, self.selected_clip)
        else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.id == clip_id) else {
            return;
        };
        let pct = |p: f64| {
            lumit_core::Rational::from_f64_on_grid(p / 100.0, 1000)
                .unwrap_or(lumit_core::Rational::ONE)
        };
        let mut new_clips = clips.clone();
        new_clips[idx] = new_clips[idx].with_ramp(pct(v0_pct), pct(v1_pct), ease);
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Set the selected clip's frame interpolation (Nearest / Blend / Flow).
    pub fn set_selected_clip_interp(&mut self, interp: lumit_core::retime::Interpolation) {
        use lumit_core::model::LayerKind;
        let (Some(comp_id), Some(layer_id), Some(clip_id)) =
            (self.selected_comp, self.selected_layer, self.selected_clip)
        else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.id == clip_id) else {
            return;
        };
        let mut new_clips = clips.clone();
        new_clips[idx].interpolation = interp;
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Delete the clip under the playhead in the selected sequence layer,
    /// leaving a gap (the Vegas surface allows gaps — K-071).
    pub fn delete_clip_at_playhead(&mut self) {
        use lumit_core::model::LayerKind;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            self.error = Some("select a sequence layer".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Sequence { clips } = &layer.kind else {
            self.error = Some("not a sequence layer".into());
            return;
        };
        let Ok(comp_t) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let Ok(tau) = comp_t.0.checked_sub(layer.start_offset.0) else {
            return;
        };
        let Some(idx) = clips.iter().position(|c| c.contains(tau.to_f64())) else {
            self.error = Some("no clip under the playhead".into());
            return;
        };
        let mut new_clips = clips.clone();
        new_clips.remove(idx);
        self.commit(Op::SetSequenceClips {
            comp: comp_id,
            layer: layer_id,
            clips: new_clips,
        });
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Ops that guarantee the auto-filing folder for `kind` exists, plus its
    /// id. Tracks the folder by id (AE habit: renaming or nesting the Solids
    /// folder keeps it the Solids folder); a deleted one is recreated.
    fn ensure_auto_folder_ops(&self, kind: lumit_core::ops::AutoFolderKind) -> (Uuid, Vec<Op>) {
        use lumit_core::model::Folder;
        use lumit_core::ops::AutoFolderKind;
        let doc = self.store.snapshot();
        let slot = match kind {
            AutoFolderKind::Solids => doc.auto_folders.solids,
            AutoFolderKind::Compositions => doc.auto_folders.compositions,
        };
        if let Some(id) = slot {
            if doc.folder(id).is_some() {
                return (id, Vec::new());
            }
        }
        let id = Uuid::now_v7();
        let name = match kind {
            AutoFolderKind::Solids => "Solids",
            AutoFolderKind::Compositions => "Compositions",
        };
        (
            id,
            vec![
                Op::AddItem {
                    index: doc.items.len(),
                    item: Box::new(ProjectItem::Folder(Folder {
                        id,
                        name: name.into(),
                        children: Vec::new(),
                        extra: serde_json::Map::new(),
                    })),
                },
                Op::SetAutoFolder {
                    kind,
                    folder: Some(id),
                },
            ],
        )
    }

    /// The op that files `item` into `folder` (appended), given the ops in
    /// `prior` may have just created the folder.
    fn file_into_folder_op(&self, folder: Uuid, item: Uuid, prior: &[Op]) -> Op {
        let doc = self.store.snapshot();
        let mut children = doc
            .folder(folder)
            .map(|f| f.children.clone())
            .unwrap_or_default();
        // The folder may not exist yet (created earlier in this batch).
        let _ = prior;
        children.push(item);
        Op::SetFolderChildren { folder, children }
    }

    /// Add a Solid layer backed by a SolidDef asset filed in the Solids
    /// auto-folder (docs/03-DATA-MODEL.md §2: solids are assets so they
    /// dedupe). One batch, one undo step.
    pub fn add_solid_layer(&mut self) {
        use lumit_core::model::{
            Layer, LayerKind, LinearColour, SolidDef, Switches, TransformGroup,
        };
        use lumit_core::ops::AutoFolderKind;
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let (folder_id, mut ops) = self.ensure_auto_folder_ops(AutoFolderKind::Solids);
        let def_id = Uuid::now_v7();
        let n_solids = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Solid(_)))
            .count();
        let added = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        ops.push(Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Solid(SolidDef {
                id: def_id,
                name: format!("White solid {}", n_solids + 1),
                colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                width: comp.width,
                height: comp.height,
                extra: serde_json::Map::new(),
            })),
        });
        ops.push(self.file_into_folder_op(folder_id, def_id, &ops));
        let layer = Layer {
            id: Uuid::now_v7(),
            name: format!("White solid {}", n_solids + 1),
            kind: LayerKind::Solid { def: def_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup {
                position_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
                position_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
                anchor_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
                anchor_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
                ..TransformGroup::default()
            },
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        ops.push(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.commit(Op::Batch { ops });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Open the settings dialogue for a new comp. Defaults match the pending
    /// footage when a drop starts the comp; otherwise the house defaults.
    pub fn open_new_comp_dialog(&mut self, pending_item: Option<Uuid>) {
        let mut dialog = CompDialog {
            editing: None,
            name: format!("Comp {}", self.comp_counter + 1),
            width: 1920,
            height: 1080,
            fps: 60.0,
            duration_s: 30.0,
            lock_ratio: true,
            aspect: 1920.0 / 1080.0,
            pending_item,
        };
        #[cfg(feature = "media")]
        if let Some(item) = pending_item {
            if let Some(media::MediaStatus::Ready { probe, frames, .. }) = self.media.map.get(&item)
            {
                if let Some(v) = &probe.video {
                    dialog.width = v.width;
                    dialog.height = v.height;
                    dialog.aspect = f64::from(v.width) / f64::from(v.height).max(1.0);
                    dialog.fps = v.fps();
                    dialog.duration_s = *frames as f64 / v.fps().max(1.0);
                }
            }
        }
        self.comp_dialog = Some(dialog);
    }

    /// Open the settings dialogue pre-filled from an existing comp.
    pub fn open_comp_settings(&mut self, comp_id: Uuid) {
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        self.comp_dialog = Some(CompDialog {
            editing: Some(comp_id),
            name: comp.name.clone(),
            width: comp.width,
            height: comp.height,
            fps: comp.frame_rate.fps(),
            duration_s: comp.duration.0.to_f64(),
            lock_ratio: true,
            aspect: f64::from(comp.width) / f64::from(comp.height).max(1.0),
            pending_item: None,
        });
    }

    /// fps as a rational: exact when whole, NTSC-snapped near x/1.001,
    /// millifps otherwise.
    fn frame_rate_of(fps: f64) -> Option<FrameRate> {
        let fps = fps.clamp(1.0, 1000.0);
        let whole = fps.round();
        if (fps - whole).abs() < 0.001 {
            return FrameRate::new(whole as u32, 1).ok();
        }
        let ntsc_base = (fps * 1.001).round();
        if (fps - ntsc_base * 1000.0 / 1001.0).abs() < 0.001 {
            return FrameRate::new(ntsc_base as u32 * 1000, 1001).ok();
        }
        FrameRate::new((fps * 1000.0).round() as u32, 1000).ok()
    }

    /// Apply the open dialogue: create the comp (filed in the Compositions
    /// auto-folder, one undo step) or update the existing one.
    pub fn confirm_comp_dialog(&mut self) {
        use lumit_core::ops::AutoFolderKind;
        let Some(dialog) = self.comp_dialog.take() else {
            return;
        };
        let Some(frame_rate) = Self::frame_rate_of(dialog.fps) else {
            self.error = Some("invalid frame rate".into());
            return;
        };
        let duration = Duration(
            Rational::from_f64_on_grid(dialog.duration_s.max(0.04), Rational::FLICK_DEN)
                .unwrap_or(rat(30, 1)),
        );
        let width = dialog.width.clamp(16, 16384);
        let height = dialog.height.clamp(16, 16384);
        if let Some(comp_id) = dialog.editing {
            let doc = self.store.snapshot();
            let Some(comp) = doc.comp(comp_id) else {
                return;
            };
            self.commit(Op::SetCompSettings {
                comp: comp_id,
                name: dialog.name,
                width,
                height,
                frame_rate,
                duration,
                background: comp.background,
            });
            #[cfg(feature = "media")]
            self.refresh_preview();
            return;
        }
        self.comp_counter += 1;
        let comp = Composition {
            id: Uuid::now_v7(),
            name: dialog.name,
            width,
            height,
            frame_rate,
            duration,
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let id = comp.id;
        let doc = self.store.snapshot();
        let (folder_id, mut ops) = self.ensure_auto_folder_ops(AutoFolderKind::Compositions);
        let added = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        ops.push(Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Composition(comp)),
        });
        ops.push(self.file_into_folder_op(folder_id, id, &ops));
        self.commit(Op::Batch { ops });
        // A brand-new comp opens as the active Timeline tab and the viewed
        // comp, so it is the target for the next add. Without this, items kept
        // landing in a comp opened earlier: the old `preview_comp` lagged
        // behind `selected_comp`, and there was no tab to switch back with.
        self.open_comp(id);
        if let Some(item) = dialog.pending_item {
            self.add_item_to_comp(item);
        }
    }

    /// Make `id` the active comp: shown in the Timeline and the Viewer, and
    /// listed as an open Timeline tab. The Timeline shows one tab per open comp
    /// (07-UI-SPEC §4), so opening a comp adds its tab rather than replacing
    /// whichever comp was open before. No-op for a non-comp id.
    pub fn open_comp(&mut self, id: Uuid) {
        if self.store.snapshot().comp(id).is_none() {
            return;
        }
        if !self.open_comps.contains(&id) {
            self.open_comps.push(id);
        }
        self.selected_comp = Some(id);
        self.selected_item = Some(id);
        self.preview_comp = Some(id);
        self.preview_item = None;
        self.preview_frame = 0;
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Close an open comp's Timeline tab. The comp itself stays in the project;
    /// only its tab closes. If it was the active tab, its neighbour takes over
    /// (or the Timeline empties when the last tab closes).
    pub fn close_comp_tab(&mut self, id: Uuid) {
        let Some(pos) = self.open_comps.iter().position(|c| *c == id) else {
            return;
        };
        self.open_comps.remove(pos);
        if self.selected_comp != Some(id) {
            return;
        }
        // Prefer the tab that shifted into this slot (the one to the right),
        // else the new last tab, else nothing left to show.
        match self
            .open_comps
            .get(pos)
            .or_else(|| self.open_comps.last())
            .copied()
        {
            Some(next) => {
                self.selected_comp = Some(next);
                self.selected_item = Some(next);
                self.preview_comp = Some(next);
                self.preview_item = None;
                self.preview_frame = 0;
                #[cfg(feature = "media")]
                self.refresh_preview();
            }
            None => {
                self.selected_comp = None;
                self.preview_comp = None;
            }
        }
    }

    /// Add any project item to the active comp as a new top layer (footage,
    /// solid, or another comp as a Precomp — the drag-and-drop entry point).
    pub fn add_item_to_comp(&mut self, item_id: Uuid) {
        let doc = self.store.snapshot();
        match doc.item(item_id) {
            Some(ProjectItem::Footage(_)) => self.add_footage_to_comp(item_id),
            Some(ProjectItem::Composition(_)) => self.add_precomp_to_comp(item_id),
            Some(ProjectItem::Solid(_)) => self.add_solid_def_layer(item_id),
            _ => {}
        }
    }

    /// Add a layer referencing an existing SolidDef (dragging a solid asset
    /// back into a comp — the def dedupes, no new asset).
    pub fn add_solid_def_layer(&mut self, def_id: Uuid) {
        use lumit_core::model::{Layer, LayerKind, Switches, TransformGroup};
        use lumit_core::time::CompTime;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            self.error = Some("select a composition first".into());
            return;
        };
        let doc = self.store.snapshot();
        let (Some(comp), Some(def)) = (doc.comp(comp_id), doc.solid(def_id)) else {
            return;
        };
        let layer = Layer {
            id: Uuid::now_v7(),
            name: def.name.clone(),
            kind: LayerKind::Solid { def: def_id },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(comp.duration.0),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup {
                position_x: lumit_core::anim::Property::fixed(f64::from(comp.width) * 0.5),
                position_y: lumit_core::anim::Property::fixed(f64::from(comp.height) * 0.5),
                anchor_x: lumit_core::anim::Property::fixed(f64::from(def.width) * 0.5),
                anchor_y: lumit_core::anim::Property::fixed(f64::from(def.height) * 0.5),
                ..TransformGroup::default()
            },
            matte: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        self.commit(Op::AddLayer {
            comp: comp_id,
            index: 0,
            layer: Box::new(layer),
        });
        self.preview_comp = Some(comp_id);
        #[cfg(feature = "media")]
        self.refresh_preview();
    }

    /// Manual New composition: always the dialogue (K-068 flow).
    pub fn new_composition(&mut self) {
        self.open_new_comp_dialog(None);
    }

    /// True when `candidate` sits inside `ancestor`'s folder subtree.
    fn folder_contains(doc: &Document, ancestor: Uuid, candidate: Uuid) -> bool {
        let mut stack = vec![ancestor];
        let mut seen = Vec::new();
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue; // defensive: malformed cycles never hang the UI
            }
            seen.push(id);
            if let Some(f) = doc.folder(id) {
                for c in &f.children {
                    if *c == candidate {
                        return true;
                    }
                    stack.push(*c);
                }
            }
        }
        false
    }

    /// Move an item into a folder (None = the panel root): one undo step
    /// removing it from every folder that lists it, then filing it. Dropping
    /// a folder into itself or its own subtree is refused quietly.
    pub fn move_item_to_folder(&mut self, item: Uuid, target: Option<Uuid>) {
        let doc = self.store.snapshot();
        if Some(item) == target {
            return;
        }
        if let Some(t) = target {
            if doc.folder(t).is_none() || Self::folder_contains(&doc, item, t) {
                return;
            }
        }
        let mut ops = Vec::new();
        for pi in &doc.items {
            if let ProjectItem::Folder(f) = pi {
                if f.children.contains(&item) && Some(f.id) != target {
                    ops.push(Op::SetFolderChildren {
                        folder: f.id,
                        children: f.children.iter().copied().filter(|c| *c != item).collect(),
                    });
                }
            }
        }
        if let Some(t) = target {
            if let Some(f) = doc.folder(t) {
                if !f.children.contains(&item) {
                    let mut children = f.children.clone();
                    children.push(item);
                    ops.push(Op::SetFolderChildren {
                        folder: t,
                        children,
                    });
                }
            }
        }
        match ops.len() {
            0 => {}
            1 => {
                if let Some(op) = ops.pop() {
                    self.commit(op);
                }
            }
            _ => self.commit(Op::Batch { ops }),
        }
    }

    /// Create an empty folder at the panel root.
    pub fn new_folder(&mut self) {
        use lumit_core::model::Folder;
        let doc = self.store.snapshot();
        let n = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Folder(_)))
            .count();
        self.commit(Op::AddItem {
            index: doc.items.len(),
            item: Box::new(ProjectItem::Folder(Folder {
                id: Uuid::now_v7(),
                name: format!("Folder {}", n + 1),
                children: Vec::new(),
                extra: serde_json::Map::new(),
            })),
        });
    }

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
    fn next_playback_prefetch(&self, comp_id: Uuid, playhead: usize, end: usize) -> Option<usize> {
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

    /// Keep the disk tier pointed at the saved project's sidecar, starting
    /// the IO worker on first use. Cheap to call every frame.
    #[cfg(feature = "media")]
    pub fn disk_sync_root(&mut self) {
        let root = self
            .path
            .as_deref()
            .and_then(lumit_cache::disk::sidecar_root);
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
                                    wants_flow: false,
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
                        // Flow motion blur measures motion between this frame
                        // and the +1 neighbour (already in `temporal`).
                        wants_flow: lumit_core::fx::stack_wants_flow_field(
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
    fn ensure_audio_engine(&mut self) -> Option<&lumit_audio::AudioEngine> {
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

    /// Trim the selected retimed footage layer so it ends exactly where the
    /// retime runs out of source (K-022): no auto-ripple — an explicit command
    /// the overrun indicator invites. No-op with a note if it doesn't overrun.
    #[cfg(feature = "media")]
    pub fn trim_selected_to_source_end(&mut self) {
        use lumit_core::model::LayerKind;
        use lumit_core::time::CompTime;
        use lumit_core::Rational;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Footage {
            item,
            retime: Some(rt),
        } = &layer.kind
        else {
            self.error = Some("select a retimed footage layer".into());
            return;
        };
        let src_dur = match self.media.map.get(item) {
            Some(media::MediaStatus::Ready { probe, frames, .. }) => match probe.video.as_ref() {
                Some(v) => *frames as f64 / v.fps().max(1.0),
                None => return,
            },
            _ => return,
        };
        let src_r = Rational::from_f64_on_grid(src_dur, 1000).unwrap_or(Rational::ONE);
        let Some(ot) = rt.overrun_local_time(src_r) else {
            self.error = Some("this clip doesn't run out of source".into());
            return;
        };
        let in_point = layer.in_point;
        let start_offset = layer.start_offset;
        let new_out = start_offset.0.to_f64() + ot;
        let out_point =
            CompTime(Rational::from_f64_on_grid(new_out, 1000).unwrap_or(layer.out_point.0));
        self.commit(Op::SetLayerSpan {
            comp: comp_id,
            layer: layer_id,
            in_point,
            out_point,
            start_offset,
        });
        self.refresh_preview();
    }

    /// Drop a user marker at the playhead on the current composition.
    pub fn add_marker_at_playhead(&mut self) {
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Ok(t) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let mut markers = comp.markers.clone();
        markers.push(lumit_core::markers::Marker::user(uuid::Uuid::now_v7(), t.0));
        markers.sort_by_key(|m| m.time.0);
        self.commit(lumit_core::Op::SetCompMarkers {
            comp: comp_id,
            markers,
        });
    }

    /// Remove all detected Beat markers from the current composition, keeping
    /// user and chapter markers.
    pub fn clear_beat_markers(&mut self) {
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        if !comp.markers.iter().any(|m| m.is_beat()) {
            return;
        }
        let markers: Vec<_> = comp
            .markers
            .iter()
            .filter(|m| !m.is_beat())
            .cloned()
            .collect();
        self.commit(lumit_core::Op::SetCompMarkers {
            comp: comp_id,
            markers,
        });
    }

    /// Detect beat markers for `comp_id` off the UI thread: mix the comp's
    /// audio (same path as playback), run onset + tempo detection, and hand the
    /// beat times back ([`Self::poll_beats`] turns them into markers). No-op —
    /// with an error note — for a silent comp.
    #[cfg(feature = "media")]
    pub fn detect_beats(&mut self, comp_id: Uuid, sensitivity: f32) {
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
            self.error = Some("no audio in this composition to detect beats from".into());
            return;
        }
        let duration_s = comp.duration.0.to_f64();
        let tx = self.beats_tx.clone();
        std::thread::spawn(move || {
            let samples = crate::export::mixdown(&jobs, rate, duration_s);
            let analysis = lumit_audio::beat::analyse_stereo(&samples, rate, sensitivity);
            // Grid-assist: nudge near-grid onsets onto the tempo grid (≤45ms),
            // which removes the small analysis latency without moving outliers.
            let times: Vec<f64> = analysis.onsets.iter().map(|o| o.time).collect();
            let snapped = lumit_audio::beat::snap_to_grid(&times, analysis.bpm, 0.045);
            let beats: Vec<(f64, f32)> = snapped
                .iter()
                .zip(&analysis.onsets)
                .map(|(t, o)| (*t, o.confidence))
                .collect();
            let _ = tx.send((comp_id, analysis.bpm, beats));
        });
    }

    /// Drain finished beat analysis into Beat markers (replacing only the prior
    /// Beat markers) as one undo step.
    #[cfg(feature = "media")]
    pub fn poll_beats(&mut self) {
        let mut newest = None;
        while let Ok(msg) = self.beats_rx.try_recv() {
            newest = Some(msg);
        }
        let Some((comp_id, bpm, beats)) = newest else {
            return;
        };
        self.detected_bpm = Some((comp_id, bpm));
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let new_beats: Vec<lumit_core::markers::Marker> = beats
            .iter()
            .filter_map(|(t, c)| {
                let time = lumit_core::Rational::from_f64_on_grid(t.max(0.0), 1000).ok()?;
                Some(lumit_core::markers::Marker::beat(
                    uuid::Uuid::now_v7(),
                    time,
                    *c,
                ))
            })
            .collect();
        let markers = lumit_core::markers::with_regenerated_beats(&comp.markers, new_beats);
        self.commit(lumit_core::Op::SetCompMarkers {
            comp: comp_id,
            markers,
        });
    }

    #[cfg(feature = "media")]
    pub fn is_playing(&self) -> bool {
        self.comp_playback.is_some() || self.audio_engine.as_ref().is_some_and(|e| e.is_playing())
    }

    /// Advance comp playback; returns true while playing (UI keeps repainting).
    /// Audio-clock-driven when this comp's mixed audio is loaded and running
    /// (the audio card's sample count is the one clock — docs/impl §4), else a
    /// wall-clock fallback so silent comps and the pre-mix moment still play.
    #[cfg(feature = "media")]
    pub fn comp_playback_tick(&mut self) -> bool {
        if self.comp_playback.is_none() {
            return false;
        }
        let Some(comp_id) = self.preview_comp else {
            self.comp_playback = None;
            return false;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            self.comp_playback = None;
            return false;
        };
        let (wa_start, wa_end) = self.work_area_frames(comp);
        let fps = comp.frame_rate.fps().max(1.0);

        let clock_driven = self.audio_loaded_comp == Some(comp_id)
            && self.audio_engine.as_ref().is_some_and(|e| e.is_playing());
        // The audio clock IS comp time (mix sample 0 = comp time 0).
        let frame = if clock_driven {
            let t = self
                .audio_engine
                .as_ref()
                .map(|e| e.clock_seconds())
                .unwrap_or(0.0);
            (t * fps).round().max(0.0) as usize
        } else if let Some((started, start_frame)) = self.comp_playback {
            start_frame + (started.elapsed().as_secs_f64() * fps) as usize
        } else {
            return false;
        };

        if frame >= wa_end {
            // Loop the work area (07-UI-SPEC transport: loop work area default).
            // Re-seek AND re-play the mix so a loop that reached the buffer's
            // end (which pauses the stream) restarts cleanly.
            if self.audio_loaded_comp == Some(comp_id) {
                if let Some(engine) = &self.audio_engine {
                    engine.seek_seconds(wa_start as f64 / fps);
                    engine.play();
                }
            } else {
                self.comp_playback = Some((Instant::now(), wa_start));
            }
            self.preview_frame = wa_start;
            self.refresh_preview();
            return true;
        }
        if frame != self.preview_frame {
            self.preview_frame = frame;
            self.refresh_preview();
        }
        // Warm a little ahead of the clock so the work-area loop stays smooth
        // once frames are cached (docs/impl/playback-scheduler.md §5). Only when
        // the frame under the playhead is already cached — so this never
        // pre-empts the present request above — and one prefetch at a time.
        if self.fill_in_flight.is_none() {
            let present_cached = self
                .frame_key_for(comp_id, frame)
                .is_some_and(|k| self.comp_frame_cache.contains_key(&k));
            if present_cached {
                if let Some(prefetch) = self.next_playback_prefetch(comp_id, frame, wa_end) {
                    self.request_fill_frame(comp_id, prefetch);
                }
            }
        }
        true
    }

    #[cfg(feature = "media")]
    pub fn playback_clock(&self) -> Option<f64> {
        self.audio_engine.as_ref().map(|e| e.clock_seconds())
    }

    /// Space: play/pause. Plays the open composition (audio-synced, mixing its
    /// audio layers) or the previewed footage, from the current frame.
    #[cfg(feature = "media")]
    pub fn toggle_play(&mut self) {
        // Any second press pauses.
        if self.is_playing() {
            if let Some(engine) = &self.audio_engine {
                engine.pause();
            }
            self.comp_playback = None;
            return;
        }

        // Composition playback.
        if let Some(comp_id) = self.preview_comp {
            let doc = self.store.snapshot();
            let Some(fps) = doc.comp(comp_id).map(|c| c.frame_rate.fps().max(1.0)) else {
                return;
            };
            // Start on the wall clock immediately; audio joins when mixed.
            self.comp_playback = Some((Instant::now(), self.preview_frame));
            if self.audio_loaded_comp == Some(comp_id) {
                if let Some(engine) = &self.audio_engine {
                    engine.seek_seconds(self.preview_frame as f64 / fps);
                    engine.play();
                }
            } else {
                self.prepare_comp_audio(comp_id);
            }
            return;
        }

        // Footage playback.
        let Some(id) = self.preview_item else { return };
        let Some(fps) = self.preview_fps() else {
            return;
        };
        let Some(buffer) = self.audio_cache.get(&id).cloned() else {
            self.request_preview_audio(); // will be ready on a later press
            return;
        };
        let start = self.preview_frame as f64 / fps;
        if self.ensure_audio_engine().is_none() {
            return;
        }
        let needs_load = self.audio_loaded != Some(id);
        if needs_load {
            self.audio_loaded = Some(id);
            self.audio_loaded_comp = None; // a footage buffer is loaded now
        }
        if let Some(engine) = &self.audio_engine {
            if needs_load {
                engine.load(buffer);
            }
            engine.seek_seconds(start);
            engine.play();
        }
    }

    pub fn project_title(&self) -> String {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        if self.dirty {
            format!("{name} • Lumit")
        } else {
            format!("{name} — Lumit")
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn draft_width_caps_for_instant_scrub_but_never_exceeds_specified() {
        // Full res, dragging: capped at the draft width for a fast decode.
        assert_eq!(decode_target_width(1920, true, false, 1.0, 1), Some(640));
        // Draft never coarser than needed: half res (960) already below no cap,
        // still above 640 -> draft caps to 640.
        assert_eq!(decode_target_width(1920, true, false, 1.0, 2), Some(640));
        // Quarter res (480) is finer than the draft cap: keep 480, don't raise.
        assert_eq!(decode_target_width(1920, true, false, 1.0, 4), Some(480));
        // Auto res zoomed right out (192) stays 192 under draft.
        assert_eq!(decode_target_width(1920, true, true, 0.1, 1), Some(192));
        // A source already smaller than the cap needs no draft decode.
        assert_eq!(decode_target_width(320, true, false, 1.0, 1), None);
    }

    #[test]
    fn fill_walk_is_forward_biased_and_complete() {
        let order = fill_walk_order(5, 0, 10);
        assert_eq!(order[0], 5); // the playhead caches first
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..10).collect::<Vec<_>>()); // every frame once
                                                         // Of the four frames right after the playhead, at least three are ahead.
        let ahead = order[1..5].iter().filter(|&&f| f > 5).count();
        assert!(ahead >= 3, "expected a forward bias: {order:?}");
        // Playhead at the work-area start: everything is ahead, no panic.
        assert_eq!(fill_walk_order(0, 0, 4), vec![0, 1, 2, 3]);
        // Degenerate spans return cleanly.
        assert_eq!(fill_walk_order(0, 0, 1), vec![0]);
        assert!(fill_walk_order(0, 0, 0).is_empty());
    }

    #[test]
    fn playback_lookahead_is_a_bounded_forward_window() {
        // Strictly forward, starting just past the playhead.
        assert_eq!(playback_lookahead(5, 100, 4), vec![6, 7, 8, 9]);
        // Clamps to the (exclusive) work-area end.
        assert_eq!(playback_lookahead(8, 10, 4), vec![9]);
        // Empty at or past the end, and with a zero lookahead.
        assert!(playback_lookahead(9, 10, 4).is_empty());
        assert!(playback_lookahead(10, 10, 4).is_empty());
        assert!(playback_lookahead(5, 100, 0).is_empty());
    }

    #[test]
    fn specified_width_is_unchanged_when_not_drafting() {
        assert_eq!(decode_target_width(1920, false, false, 1.0, 1), None);
        assert_eq!(decode_target_width(1920, false, false, 1.0, 2), Some(960));
        assert_eq!(decode_target_width(1000, false, true, 0.5, 1), Some(500));
    }

    /// K-068: solids are assets auto-filed into a "Solids" folder that is
    /// followed by id (rename it, it still collects); comps auto-file into
    /// "Compositions"; each creation is one undo step.
    #[test]
    fn pan_behind_keeps_the_layer_fixed() {
        // No rotation, 100% scale: position tracks the anchor 1:1.
        let p = pan_behind_position(
            (50.0, 50.0),
            (60.0, 50.0),
            (100.0, 100.0),
            (100.0, 100.0),
            0.0,
        );
        assert!((p.0 - 110.0).abs() < 1e-9 && (p.1 - 100.0).abs() < 1e-9);
        // 200% scale doubles the position shift for the same anchor move.
        let p = pan_behind_position((0.0, 0.0), (10.0, 0.0), (0.0, 0.0), (200.0, 200.0), 0.0);
        assert!((p.0 - 20.0).abs() < 1e-9 && p.1.abs() < 1e-9);
        // 90° rotation sends an x-move of the anchor into +y of position.
        let p = pan_behind_position((0.0, 0.0), (10.0, 0.0), (0.0, 0.0), (100.0, 100.0), 90.0);
        assert!(p.0.abs() < 1e-9 && (p.1 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn centred_transform_puts_origin_at_object_centre() {
        // A 1920×1080 object in a 1280×720 comp: anchor at the object's
        // centre, position at the comp's centre (AE default).
        let tr = centred_transform(1920.0, 1080.0, 1280, 720);
        assert_eq!(tr.anchor_x.value_at(0.0), 960.0);
        assert_eq!(tr.anchor_y.value_at(0.0), 540.0);
        assert_eq!(tr.position_x.value_at(0.0), 640.0);
        assert_eq!(tr.position_y.value_at(0.0), 360.0);
        // Scale/rotation stay neutral so only the origin/position changed.
        assert_eq!(tr.scale_x.value_at(0.0), 100.0);
        assert_eq!(tr.rotation.value_at(0.0), 0.0);
    }

    #[test]
    fn auto_folders_collect_solids_and_comps() {
        let mut app = AppState::default();
        app.new_composition();
        app.confirm_comp_dialog();
        let doc = app.store.snapshot();
        let comps_folder = doc.auto_folders.compositions.expect("comps folder");
        assert_eq!(doc.folder(comps_folder).unwrap().children.len(), 1);

        app.add_solid_layer();
        let doc = app.store.snapshot();
        let solids_folder = doc.auto_folders.solids.expect("solids folder");
        let first_children = doc.folder(solids_folder).unwrap().children.clone();
        assert_eq!(first_children.len(), 1);
        assert!(doc.solid(first_children[0]).is_some());

        // Rename the folder: the habit follows the id, not the name.
        app.commit(Op::RenameItem {
            id: solids_folder,
            name: "My colours".into(),
        });
        app.add_solid_layer();
        let doc = app.store.snapshot();
        assert_eq!(doc.folder(solids_folder).unwrap().children.len(), 2);
        assert_eq!(doc.folder(solids_folder).unwrap().name, "My colours");

        // One undo removes the whole second solid creation (batch), and the
        // layer count in the comp drops with it.
        let comp_id = app.selected_comp.unwrap();
        assert_eq!(doc.comp(comp_id).unwrap().layers.len(), 2);
        app.undo();
        let doc = app.store.snapshot();
        assert_eq!(doc.folder(solids_folder).unwrap().children.len(), 1);
        assert_eq!(doc.comp(comp_id).unwrap().layers.len(), 1);

        // Deleting the folder recreates it on next use (fresh id).
        app.commit(Op::RemoveItem { id: solids_folder });
        app.add_solid_layer();
        let doc = app.store.snapshot();
        let new_folder = doc.auto_folders.solids.unwrap();
        assert_ne!(new_folder, solids_folder);
        assert_eq!(doc.folder(new_folder).unwrap().children.len(), 1);

        // Move-to-folder: filing a solid under Compositions then back to root.
        let solid_id = doc.folder(new_folder).unwrap().children[0];
        app.move_item_to_folder(solid_id, Some(comps_folder));
        let doc = app.store.snapshot();
        assert!(doc
            .folder(comps_folder)
            .unwrap()
            .children
            .contains(&solid_id));
        assert!(!doc.folder(new_folder).unwrap().children.contains(&solid_id));
        app.move_item_to_folder(solid_id, None);
        let doc = app.store.snapshot();
        assert!(doc.root_items().contains(&solid_id));

        // A folder cannot be filed into its own subtree.
        app.move_item_to_folder(comps_folder, Some(comps_folder));
        let doc = app.store.snapshot();
        assert!(!doc
            .folder(comps_folder)
            .unwrap()
            .children
            .contains(&comps_folder));
    }

    /// K-068: the dialogue edits an existing comp's settings invertibly.
    #[test]
    fn comp_settings_dialog_edits_and_undoes() {
        let mut app = AppState::default();
        app.new_composition();
        app.confirm_comp_dialog();
        let comp_id = app.selected_comp.unwrap();

        app.open_comp_settings(comp_id);
        {
            let d = app.comp_dialog.as_mut().unwrap();
            assert_eq!(d.editing, Some(comp_id));
            assert_eq!((d.width, d.height), (1920, 1080));
            d.width = 1280;
            d.height = 720;
            d.fps = 23.976;
            d.name = "Retitled".into();
        }
        app.confirm_comp_dialog();
        let doc = app.store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!((comp.width, comp.height), (1280, 720));
        assert_eq!(comp.name, "Retitled");
        // NTSC snap: 23.976 becomes exactly 24000/1001.
        assert!((comp.frame_rate.fps() - 24000.0 / 1001.0).abs() < 1e-9);
        app.undo();
        let doc = app.store.snapshot();
        assert_eq!(doc.comp(comp_id).unwrap().width, 1920);
    }

    /// Regression: a freshly created composition is the active one, so the
    /// next item dropped in lands in it — not in a comp opened earlier. The
    /// bug was `preview_comp` (the add target) lagging behind `selected_comp`
    /// after a second comp was created.
    #[test]
    fn a_new_composition_becomes_the_active_add_target() {
        let mut app = AppState::default();

        // First comp, with one footage layer.
        app.new_composition();
        app.confirm_comp_dialog();
        let comp1 = app.selected_comp.unwrap();
        app.import_paths(vec![std::path::PathBuf::from("clip.mp4")]);
        let footage = app
            .store
            .snapshot()
            .items
            .iter()
            .find_map(|i| match i {
                ProjectItem::Footage(f) => Some(f.id),
                _ => None,
            })
            .unwrap();
        app.add_item_to_comp(footage);
        assert_eq!(app.store.snapshot().comp(comp1).unwrap().layers.len(), 1);

        // Second comp: creating it makes it the active comp everywhere.
        app.new_composition();
        app.confirm_comp_dialog();
        let comp2 = app.selected_comp.unwrap();
        assert_ne!(comp1, comp2);
        assert_eq!(
            app.preview_comp,
            Some(comp2),
            "a new comp should also become the viewed/edited comp"
        );

        // The next add must land in comp2, and must not touch comp1.
        app.add_item_to_comp(footage);
        let doc = app.store.snapshot();
        assert_eq!(
            doc.comp(comp2).unwrap().layers.len(),
            1,
            "layer must land in the newly created composition"
        );
        assert_eq!(
            doc.comp(comp1).unwrap().layers.len(),
            1,
            "the earlier composition must not receive the new layer"
        );
    }

    /// 07-UI-SPEC §4: the Timeline keeps one tab per open comp. Creating comps
    /// opens their tabs; the active tab follows the newest; closing a tab hands
    /// the active comp to a neighbour and never deletes the comp itself.
    #[test]
    fn comps_open_as_timeline_tabs_and_close_cleanly() {
        let mut app = AppState::default();
        app.new_composition();
        app.confirm_comp_dialog();
        let comp1 = app.selected_comp.unwrap();
        app.new_composition();
        app.confirm_comp_dialog();
        let comp2 = app.selected_comp.unwrap();

        // Both comps are open; the newest is active.
        assert_eq!(app.open_comps, vec![comp1, comp2]);
        assert_eq!(app.selected_comp, Some(comp2));
        assert_eq!(app.preview_comp, Some(comp2));

        // Switching back to the first comp's tab re-activates it without
        // re-opening (no duplicate tab).
        app.open_comp(comp1);
        assert_eq!(app.open_comps, vec![comp1, comp2]);
        assert_eq!(app.selected_comp, Some(comp1));

        // Closing the active tab hands off to its neighbour; the comp survives.
        app.close_comp_tab(comp1);
        assert_eq!(app.open_comps, vec![comp2]);
        assert_eq!(app.selected_comp, Some(comp2));
        assert!(app.store.snapshot().comp(comp1).is_some());

        // Closing the last tab empties the Timeline.
        app.close_comp_tab(comp2);
        assert!(app.open_comps.is_empty());
        assert_eq!(app.selected_comp, None);
        assert_eq!(app.preview_comp, None);

        // Deleting a comp also drops its tab if it happened to be open.
        app.open_comp(comp2);
        app.commit(Op::RemoveItem { id: comp2 });
        app.close_comp_tab(comp2);
        assert!(app.open_comps.is_empty());
    }

    /// The slice 3 drill: save, edit past the save, crash (drop without
    /// saving), reopen — the journal restores every post-save change.
    #[test]
    fn kill_and_recover_drill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drill.lum");

        let doc_id;
        let final_json;
        {
            let mut app = AppState::default();
            doc_id = app.store.snapshot().id;
            app.new_composition();
            app.confirm_comp_dialog();
            app.path = Some(path.clone());
            app.save();
            assert!(!app.dirty);

            // Edits after the save — journalled, never saved.
            app.new_composition();
            app.confirm_comp_dialog();
            app.new_composition();
            app.confirm_comp_dialog();
            assert!(app.dirty);
            final_json = serde_json::to_string(&*app.store.snapshot()).unwrap();
            // "kill -9": app dropped here with dirty state.
        }

        let mut app2 = AppState::default();
        app2.open_path(&path);
        let pending = app2.pending_recovery.as_ref().expect("recovery offered");
        assert_eq!(pending.ops.len(), 2);
        app2.resolve_recovery(true);
        assert_eq!(
            serde_json::to_string(&*app2.store.snapshot()).unwrap(),
            final_json,
            "recovered document equals the pre-crash document"
        );
        assert!(app2.dirty, "recovered state needs a save");

        // Saving clears the journal: a fresh open offers no recovery.
        app2.save();
        let mut app3 = AppState::default();
        app3.open_path(&path);
        assert!(app3.pending_recovery.is_none());

        let _ = JournalFile::for_document(doc_id).map(|j| j.clear());
    }

    #[test]
    fn discarding_recovery_opens_last_save_and_clears_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drill2.lum");
        let saved_json;
        {
            let mut app = AppState::default();
            app.new_composition();
            app.confirm_comp_dialog();
            app.path = Some(path.clone());
            app.save();
            saved_json = serde_json::to_string(&*app.store.snapshot()).unwrap();
            app.new_composition(); // journalled, then "crash"
            app.confirm_comp_dialog();
        }
        let mut app2 = AppState::default();
        app2.open_path(&path);
        assert!(app2.pending_recovery.is_some());
        app2.resolve_recovery(false);
        assert_eq!(
            serde_json::to_string(&*app2.store.snapshot()).unwrap(),
            saved_json
        );
        let mut app3 = AppState::default();
        app3.open_path(&path);
        assert!(
            app3.pending_recovery.is_none(),
            "journal cleared on discard"
        );
    }
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod preview_tests {
    use super::preview::PreviewEngine;
    use lumit_media::index::tests_support::fixture;
    use std::time::Duration;

    /// End-to-end: request a frame the way the Viewer does; receive pixels.
    #[test]
    fn preview_engine_decodes_requested_frame_at_requested_size() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let engine = PreviewEngine::default();
        let id = uuid::Uuid::now_v7();
        engine.request(id, file, 45, Some(160));
        let result = engine
            .results
            .recv_timeout(Duration::from_secs(20))
            .expect("engine replied")
            .expect("decode succeeded");
        let super::preview::PreviewResult::Footage(px) = result else {
            panic!("expected a footage frame");
        };
        assert_eq!(px.item, id);
        assert_eq!(px.frame, 45);
        assert_eq!((px.width, px.height), (160, 120));
        assert_eq!(px.rgba.len(), 160 * 120 * 4);
    }

    /// Latest-wins: flood requests; the engine may skip stale ones and the
    /// final delivered frame is the newest request.
    #[test]
    fn preview_engine_latest_request_wins() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available");
            return;
        };
        let engine = PreviewEngine::default();
        let id = uuid::Uuid::now_v7();
        for n in 0..60 {
            engine.request(id, file.clone(), n, None);
        }
        let mut last = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match engine.results.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(super::preview::PreviewResult::Footage(px))) => {
                    last = Some(px.frame);
                    if px.frame == 59 {
                        break;
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => panic!("decode failed: {e}"),
                Err(_) => {
                    if last == Some(59) {
                        break;
                    }
                }
            }
        }
        assert_eq!(last, Some(59), "newest request must be the one served last");
    }
}

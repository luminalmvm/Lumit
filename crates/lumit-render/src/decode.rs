//! Latest-wins background frame decoding for the Viewer (slice 5), moved
//! verbatim from app_state.rs.

use std::collections::HashMap;
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

pub struct Request {
    pub generation: u64,
    pub item: Uuid,
    pub source: lumit_media::MediaSource,
    pub frame: usize,
    pub target_width: Option<u32>,
    /// The file is missing (docs/07 §3.3): answer with the test-bar slate at
    /// this size instead of decoding. Viewing a lost clip on its own must
    /// show the same bars a comp shows for it — the Viewer previously drew
    /// nothing at all here, which looks identical to a broken application.
    pub slate: Option<(u32, u32)>,
}

/// One layer's decode job inside a comp render request.
pub struct CompJob {
    pub layer: Uuid,
    pub item: Uuid,
    /// One media file, or the numbered run of stills this item is (K-439) —
    /// whichever it is, the decode worker opens it the same way.
    pub source: lumit_media::MediaSource,
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
    /// Set when `blend`'s pair is combined by optical-flow synthesis rather
    /// than a plain crossfade (K-021 Flow policy), carrying the parameters the
    /// synthesis runs with (K-331).
    ///
    /// `None` covers both "the policy is not Flow" and "it is, but the
    /// engagement gate declined" — flow that cannot help renders as Nearest,
    /// and the plan is where that is decided so the decode, the render and the
    /// cache key all see one answer.
    pub flow: Option<lumit_core::retime::FlowParams>,
    /// Neighbour source frames a temporal effect stack needs (echo, flow
    /// motion blur, datamosh): `(offset, source_frame)`, one per non-zero
    /// offset in the stack's temporal window. Empty for a plain layer, so
    /// a single-frame stack decodes exactly one frame.
    pub temporal: Vec<(i32, usize)>,
    /// One entry per flow-consuming effect in the stack (Flow motion blur,
    /// docs/08 §3.2, wants `1`; Datamosh, §3.12, K-104, wants `-1`), sorted and
    /// deduplicated: the decode worker measures the dense motion from this frame
    /// to the neighbour at each offset (already fetched via `temporal`) and
    /// stamps them onto [`CompLayerPixels::flow_fields`]. Empty for a stack that
    /// consumes no flow. See [`lumit_core::fx::stack_flow_neighbours`] — and
    /// K-444 for why this is a list rather than the one slot it used to be.
    pub flow_neighbours: Vec<i32>,
    /// The file could not be found (docs/07 §3.3): the worker synthesises the
    /// test-bar slate at the layer's size instead of decoding, so a comp with
    /// missing footage shows unmistakably-absent bars rather than silent
    /// black — and never fails the whole frame.
    pub slate: bool,
}

/// One measured motion field at a layer's decoded size: per-pixel `u` and `v` in
/// pixels, then the per-pixel confidence in 0..1 (FX-19), each `width × height`.
pub type FlowFieldData = (Vec<f32>, Vec<f32>, Vec<f32>);

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
    /// Dense forward flow (per-pixel `(u, v)` motion in pixels, plus a per-pixel
    /// `conf`idence in 0..1, row-major, same `width × height` as `rgba`) from
    /// this frame to each neighbour [`CompJob::flow_neighbours`] asked for,
    /// keyed by that offset. An offset whose neighbour did not decode is simply
    /// absent, which is its consumer's passthrough. Fast motion blur (docs/08
    /// §3.2, offset `1`) smears along its field, scaling the streak by `conf`
    /// (FX-19); Datamosh (§3.12, K-104, offset `-1`) warps the previous frame
    /// along the `(u, v)` and ignores `conf`. **Both at once is one field each,
    /// not one field shared** (K-444): they are opposite measurements.
    pub flow_fields: Vec<(i32, FlowFieldData)>,
    /// The content name of this decode (K-421): a hash of the [`CompJob`]
    /// identity [`crate::plan::same_decode`] compares — item, path, source
    /// frame, decode width, slate, blend partner, flow settings, temporal
    /// window. Two jobs that decode the same pixels get the same name, which is
    /// what lets the per-effect cache recognise a layer's source across
    /// renders without hashing its bytes.
    pub source_key: u128,
}

pub struct CompFrame {
    pub comp: Uuid,
    pub frame: usize,
    /// The media epoch this frame was rendered under — see
    /// `AppState::media_epoch`. A render started before a probe landed drew a
    /// layer whose state was still unknown (so it drew nothing); if its
    /// result arrives afterwards it is banked under a key derived from the
    /// *new* state, filing black pixels under the slate's name. Clearing the
    /// cache when the probe lands cannot help — these frames are still in
    /// flight at that moment — so the receiver drops them by epoch instead.
    ///
    /// Deliberately not the request generation: every request bumps that,
    /// background fills included, so gating on it would let a fill supersede
    /// a display render and the Viewer would stop updating.
    pub media_epoch: u64,
    /// Top-of-stack first (document order); the renderer draws bottom-up.
    pub layers: Vec<CompLayerPixels>,
    /// Wall time this frame's layers took to decode on the worker thread — the
    /// dominant, measurable part of the true render cost. Realtime mode feeds it
    /// to the adaptive controller. Measured here (not as dispatch→display on the
    /// UI thread) so it reflects real work, not the UI's repaint-poll interval —
    /// otherwise every frame would appear to cost one repaint (~16 ms) and the
    /// resolution would walk down even on comps that play fine at Full.
    pub render_cost: std::time::Duration,
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
        media_epoch: u64,
    },
    /// Resize the decoded-frame cache (its slice of the one RAM budget,
    /// Settings → Performance). Applied immediately, never latest-wins-dropped.
    SetCacheBudget(usize),
}

impl Default for PreviewEngine {
    fn default() -> Self {
        let (tx, rx) = channel::<Message>();
        let (result_tx, results) = channel();
        let generation = Arc::new(AtomicU64::new(0));
        let live = generation.clone();
        std::thread::spawn(move || {
            let mut pool = DecodePool::new();
            loop {
                // Block for one request, then drain to the newest (latest
                // wins). Budget messages apply on the spot — they must never
                // be dropped by the latest-wins replacement.
                let mut req = loop {
                    match rx.recv() {
                        Ok(Message::SetCacheBudget(bytes)) => pool.set_budget(bytes),
                        Ok(r) => break r,
                        Err(_) => return,
                    }
                };
                loop {
                    match rx.try_recv() {
                        Ok(Message::SetCacheBudget(bytes)) => pool.set_budget(bytes),
                        Ok(newer) => req = newer,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
                let generation = match &req {
                    Message::Footage(r) => r.generation,
                    Message::Comp { generation, .. } => *generation,
                    Message::SetCacheBudget(_) => continue, // handled above
                };
                if generation != live.load(Ordering::Relaxed) {
                    continue; // superseded while queued
                }
                let result = match req {
                    Message::Footage(r) => pool.decode_footage(&r).map(PreviewResult::Footage),
                    Message::Comp {
                        comp,
                        frame,
                        jobs,
                        media_epoch,
                        ..
                    } => pool
                        // Nobody watches a background decode's progress: the
                        // bar belongs to the frame the Viewer is waiting for.
                        .decode_comp(comp, frame, &jobs, media_epoch, &|_| {})
                        .map(PreviewResult::Comp),
                    Message::SetCacheBudget(_) => continue, // handled above
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

/// The decoders, the decoded-frame cache and the flow backend one decoding
/// context owns.
///
/// # In plain terms
///
/// Opening a video file is expensive and seeking around it is worse, so the
/// pipeline keeps one open decoder per footage item and a byte-budgeted cache of
/// the frames it has already read. This is that state, bundled so it can be
/// owned by whoever is doing the decoding: the background worker thread that
/// serves the egui Viewer, or — for the Flutter frontend, whose render calls
/// already arrive on a worker — the headless renderer itself.
///
/// The decoded-frame cache is what makes a scrub cheap: revisiting a frame is a
/// map lookup rather than a seek and a decode. Note that it holds *decoded
/// source frames*, not finished comp frames — those are named and cached a level
/// up, in [`crate::cache`].
pub struct DecodePool {
    decoders: HashMap<Uuid, lumit_media::VideoDecoder>,
    frame_cache: lumit_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
    /// Flow backend, created on the first Flow-policy frame. Uses [`Self::gpu`]
    /// — the renderer's own device — when the pool was given one, so flow runs
    /// where the frames are already going rather than on a second device of its
    /// own (K-331). Falls back to a headless device, then to the CPU oracle;
    /// lumit-flow degrades by itself and never faults.
    flow_engine: Option<lumit_flow::FlowEngine>,
    /// The renderer's GPU, when the owner shared it.
    gpu: Option<lumit_gpu::GpuContext>,
    /// Measured flow pairs (K-331), so a scrub does not remeasure and the two
    /// consumers of a layer's motion share one measurement.
    flow_cache: lumit_cache::ByteLru<FlowKey, CachedFlow>,
    /// How many comp frames this pool has actually decoded. Diagnostic, and the
    /// thing the drag fast path is *measured* by: a value drag must not move it
    /// (see the headless preview tests).
    comp_decodes: u64,
}

/// The decoded-frame cache's default share of RAM (K-016 tier seed); Settings →
/// Performance moves it.
pub const DEFAULT_DECODE_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// The measured-flow cache's share of RAM (K-331).
///
/// A 1080p field pair is about 37 MB at native flow resolution, so this holds
/// roughly seven of them — a scrub window, which is what it is for. Smaller than
/// the frame cache on purpose: a missed flow entry now costs ~8 ms of GPU work,
/// where a missed frame costs a seek and a decode.
pub const DEFAULT_FLOW_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// One measured flow pair, keyed by the two source frames and the settings that
/// produced it.
///
/// **Deliberately RAM only.** docs/06 §5.4 reserved a `flow/` folder beside
/// `frames/`, and it should stay empty: measuring a 1080p pair on the GPU costs
/// about 8 ms, while reading 37 MB of stored field back off an SSD costs more
/// than that. A disk tier for flow would be a cache slower than the thing it
/// caches. RAM still pays, because it is what lets the retime policy and a
/// flow-consuming effect on the same layer share one measurement instead of
/// making it twice.
struct CachedFlow {
    fwd: lumit_flow::FlowField,
    bwd: lumit_flow::FlowField,
}

impl lumit_cache::ByteSized for CachedFlow {
    fn byte_size(&self) -> usize {
        let one = |f: &lumit_flow::FlowField| f.u.len() * 8 + f.valid.len() + 48;
        one(&self.fwd) + one(&self.bwd)
    }
}

/// What a cached flow pair is filed under: which source, which two frames of
/// it, and the settings it was measured with — every one of which changes the
/// field (K-331).
type FlowKey = (Uuid, usize, usize, u64);

/// A stable hash of the settings that shape a measurement.
fn flow_settings_key(s: &lumit_flow::FlowSettings) -> u64 {
    // Only the fields that change the *field* — the synthesis-side knobs
    // (occlusion, fallback, the guard) consume it and do not alter it, so
    // folding them in would split the cache for no reason.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    s.divisor.hash(&mut h);
    s.iterations.hash(&mut h);
    s.min_level_dim.hash(&mut h);
    s.refine_iters.hash(&mut h);
    s.flow_sigma2().to_bits().hash(&mut h);
    h.finish()
}

impl Default for DecodePool {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodePool {
    #[must_use]
    pub fn new() -> Self {
        Self {
            decoders: HashMap::new(),
            frame_cache: lumit_cache::ByteLru::new(DEFAULT_DECODE_CACHE_BYTES),
            flow_engine: None,
            gpu: None,
            flow_cache: lumit_cache::ByteLru::new(DEFAULT_FLOW_CACHE_BYTES),
            comp_decodes: 0,
        }
    }

    /// A pool that runs flow on the renderer's device rather than one of its
    /// own (K-331). The handles are reference-counted clones, so this shares
    /// the device rather than duplicating it — flow work then queues behind the
    /// same driver as everything else instead of competing with it from a
    /// second context.
    #[must_use]
    pub fn with_gpu(ctx: &lumit_gpu::GpuContext) -> Self {
        Self {
            gpu: Some(lumit_gpu::GpuContext::from_parts(
                ctx.device.clone(),
                ctx.queue.clone(),
            )),
            ..Self::new()
        }
    }

    /// How many comp frames this pool has decoded since it was made.
    #[must_use]
    pub fn comp_decodes(&self) -> u64 {
        self.comp_decodes
    }

    /// What the decoded-frame cache is holding, and how many decoders are
    /// open — the pool's share of the memory report (K-294).
    ///
    /// The decoders are counted rather than measured: what a `VideoDecoder`
    /// holds is FFmpeg's business (and, with hardware decode, the driver's), so
    /// a number of them is honest where a number of bytes would be invented.
    #[must_use]
    pub fn memory(&self) -> (usize, usize) {
        (self.frame_cache.used_bytes(), self.decoders.len())
    }

    /// Resize the decoded-frame cache (its slice of the one RAM budget).
    pub fn set_budget(&mut self, bytes: usize) {
        self.frame_cache.set_budget(bytes);
    }

    /// Drop every cached decoded frame, keeping the open decoders (Settings →
    /// Clear cache). The decoders are cheap to keep and expensive to re-open.
    pub fn clear(&mut self) {
        self.flow_cache.clear();
        self.frame_cache.clear();
    }

    /// Decode one source frame (or synthesise the missing-footage slate).
    pub fn decode_footage(&mut self, req: &Request) -> Result<FramePixels, String> {
        decode(&mut self.decoders, &mut self.frame_cache, req)
    }

    /// File a frame decoded elsewhere (the decode-ahead thread) into the
    /// decoded-frame cache, under the same key a decode here would use — the
    /// hand-off that makes a prefetched render decode nothing.
    pub fn preload(
        &mut self,
        item: Uuid,
        frame: usize,
        target_width: Option<u32>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) {
        self.frame_cache.insert(
            (item, frame, target_width),
            CachedFrame {
                width,
                height,
                rgba,
            },
        );
    }

    /// Decode every layer of one comp frame from its plan — the pixels
    /// [`crate::build`] then turns into a draw list. `progress` is called with
    /// the number of jobs finished as each one lands, which is what lets a
    /// Viewer draw an honest bar through the slowest stage of a frame; pass
    /// `&|_| {}` where nobody is watching.
    pub fn decode_comp(
        &mut self,
        comp: Uuid,
        frame: usize,
        jobs: &[CompJob],
        media_epoch: u64,
        progress: &dyn Fn(usize),
    ) -> Result<CompFrame, String> {
        self.comp_decodes += 1;
        decode_comp(
            &mut self.decoders,
            &mut self.frame_cache,
            &mut self.flow_engine,
            &mut self.flow_cache,
            self.gpu.as_ref(),
            comp,
            frame,
            jobs,
            media_epoch,
            progress,
        )
    }
}

fn decode(
    decoders: &mut HashMap<Uuid, lumit_media::VideoDecoder>,
    cache: &mut lumit_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
    req: &Request,
) -> Result<FramePixels, String> {
    if let Some((w, h)) = req.slate {
        let (w, h) = (w.max(1), h.max(1));
        return Ok(FramePixels {
            width: w,
            height: h,
            rgba: lumit_media::slate::colour_bars(w, h),
            frame: req.frame,
            item: req.item,
        });
    }
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
            // The sidecar cache first (crate::media_index): the index the
            // probe already wrote when the project opened is the index this
            // decoder opens with, so the first preview frame of a session
            // costs a read rather than a fresh packet scan of the file.
            let index =
                crate::media_index::load_or_build_index(&req.source).map_err(|e| e.to_string())?;
            let dec =
                lumit_media::VideoDecoder::open(&req.source, index).map_err(|e| e.to_string())?;
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
    pub fn request(
        &self,
        item: Uuid,
        source: lumit_media::MediaSource,
        frame: usize,
        target_width: Option<u32>,
    ) {
        self.request_inner(item, source, frame, target_width, None);
    }

    /// As [`Self::request`], but answers with the missing-footage slate at
    /// `size` rather than decoding (docs/07 §3.3).
    pub fn request_slate(&self, item: Uuid, size: (u32, u32)) {
        self.request_inner(
            item,
            lumit_media::MediaSource::default(),
            0,
            None,
            Some(size),
        );
    }

    fn request_inner(
        &self,
        item: Uuid,
        source: lumit_media::MediaSource,
        frame: usize,
        target_width: Option<u32>,
        slate: Option<(u32, u32)>,
    ) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(Message::Footage(Request {
            generation,
            item,
            source,
            frame,
            target_width,
            slate,
        }));
    }

    /// Ask for every layer frame of a comp at one comp frame (latest wins).
    /// Resize the decoded-frame cache (its slice of the RAM budget).
    pub fn set_cache_budget(&self, bytes: usize) {
        let _ = self.tx.send(Message::SetCacheBudget(bytes));
    }

    pub fn request_comp(&self, comp: Uuid, frame: usize, jobs: Vec<CompJob>, media_epoch: u64) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(Message::Comp {
            generation,
            comp,
            frame,
            media_epoch,
            jobs,
        });
    }
}

/// Measure the flow between two source frames, or return the pair already
/// measured for them (K-331).
///
/// **The one door both consumers go through.** A layer can want motion for two
/// reasons at once — the retime policy inventing an in-between frame, and Fast
/// motion blur or Datamosh streaking the current one — and before this they
/// each ran DIS separately over the same footage. Whichever asks first pays;
/// the other gets the answer. It is also what makes a scrub cheap, since going
/// back over a span re-asks for pairs already measured.
///
/// Keyed by content, never by timeline position: the source, the two frames,
/// and the settings that shape the measurement.
#[allow(clippy::too_many_arguments)]
fn flow_for(
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>,
    gpu: Option<&lumit_gpu::GpuContext>,
    item: Uuid,
    frame_a: usize,
    frame_b: usize,
    a: &lumit_flow::Gray,
    b: &lumit_flow::Gray,
    set: &lumit_flow::FlowSettings,
) -> (lumit_flow::FlowField, lumit_flow::FlowField) {
    let key = (item, frame_a, frame_b, flow_settings_key(set));
    if let Some(hit) = flow_cache.get(&key) {
        return (hit.fwd.clone(), hit.bwd.clone());
    }
    let (fwd, bwd) = flow_engine
        .get_or_insert_with(|| flow_engine_for(gpu))
        .flow_pair_with(a, b, set);
    flow_cache.insert(
        key,
        CachedFlow {
            fwd: fwd.clone(),
            bwd: bwd.clone(),
        },
    );
    (fwd, bwd)
}

/// The flow engine for this pool: on the renderer's device when one was shared
/// (K-331), otherwise a headless device of its own, otherwise the CPU oracle.
fn flow_engine_for(gpu: Option<&lumit_gpu::GpuContext>) -> lumit_flow::FlowEngine {
    match gpu {
        Some(ctx) => lumit_flow::FlowEngine::with_context(ctx),
        None => lumit_flow::FlowEngine::new_auto(),
    }
}

/// Translate a layer's stored [`lumit_core::retime::FlowParams`] into the
/// plain-numbers [`lumit_flow::FlowSettings`] the engine takes (K-331).
///
/// `lumit-flow` is an engine crate that knows nothing of the document, so the
/// mapping has to live somewhere that sees both — here, once, so preview,
/// export and the flow cache can never translate the same parameters into two
/// different measurements.
pub fn flow_settings(p: &lumit_core::retime::FlowParams) -> lumit_flow::FlowSettings {
    use lumit_core::retime::{FlowFallback, OcclusionMode};
    lumit_flow::FlowSettings {
        divisor: p.resolution.divisor(),
        iterations: p.detail.iterations(),
        min_level_dim: p.detail.min_level_dim(),
        smoothness: p.smoothness as f32,
        occlusion: match p.occlusion {
            OcclusionMode::VisibleOnly => lumit_flow::OcclusionMode::VisibleOnly,
            OcclusionMode::Blend => lumit_flow::OcclusionMode::Blend,
        },
        fallback: match p.fallback {
            FlowFallback::Blend => lumit_flow::Fallback::Blend,
            FlowFallback::Nearest => lumit_flow::Fallback::Nearest,
        },
        hud_guard: p.hud_guard,
        refine_iters: p.detail.refine_iters(),
    }
}

#[allow(clippy::too_many_arguments)] // one worker call; bundling would hide it
fn decode_comp(
    decoders: &mut HashMap<Uuid, lumit_media::VideoDecoder>,
    cache: &mut lumit_cache::ByteLru<(Uuid, usize, Option<u32>), CachedFrame>,
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>,
    gpu: Option<&lumit_gpu::GpuContext>,
    comp: Uuid,
    frame: usize,
    jobs: &[CompJob],
    media_epoch: u64,
    progress: &dyn Fn(usize),
) -> Result<CompFrame, String> {
    let decode_started = std::time::Instant::now();
    let mut layers = Vec::with_capacity(jobs.len());
    for job in jobs {
        let req = Request {
            generation: 0,
            item: job.item,
            source: job.source.clone(),
            frame: job.source_frame,
            target_width: job.target_width,
            slate: None, // the comp path builds its slate below, from natural size
        };
        // Missing media renders the slate; nothing else about the layer
        // changes, so transforms, effects and blending all still apply.
        let px = if job.slate {
            let (w, h) = (job.natural_w.max(1), job.natural_h.max(1));
            FramePixels {
                width: w,
                height: h,
                rgba: lumit_media::slate::colour_bars(w, h),
                frame: job.source_frame,
                item: job.item,
            }
        } else {
            decode(decoders, cache, &req)?
        };
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
                    source: job.source.clone(),
                    frame,
                    target_width: job.target_width,
                    slate: None,
                };
                decode(decoders, cache, &nreq)
                    .ok()
                    .map(|p| (offset, p.rgba))
            })
            .collect();
        // Flow motion blur (docs/08 §3.2, offset +1) and Datamosh (§3.12,
        // K-104, offset -1) both need a dense motion field: the forward
        // flow from this frame to the requested neighbour (already
        // decoded above). Computed from the raw current frame before it
        // is consumed into `rgba` below, where both frames live as RGBA —
        // exactly as the Flow retiming policy computes its flow, on the
        // shared engine that reuses the GPU when one is present. A
        // dropped neighbour just leaves that offset absent, degrading its
        // flow-consuming effect to a passthrough.
        //
        // **One measurement per offset asked for** (K-444). The two effects
        // want opposite directions — forward to the next frame, back to the
        // previous — so a single shared field was never something both could
        // read; the layer used to carry one and the first effect in stack order
        // silently took it. They are separate entries in `flow_cache` below,
        // keyed by the frame pair, so a stack with only one of them measures
        // exactly once, as it always did.
        let flow_fields: Vec<(i32, FlowFieldData)> = job
            .flow_neighbours
            .iter()
            .filter_map(|&offset| {
                let (_, other) = temporal.iter().find(|(o, _)| *o == offset)?;
                Some((offset, {
                    let (w, h) = (px.width as usize, px.height as usize);
                    // An effect asking for motion has no parameters of its own,
                    // and sharing the retime policy's settings would make one
                    // layer's blur depend on the other's retime.
                    //
                    // Half resolution, though, not the retime default of
                    // native. Retime measures natively so that preview and
                    // export agree about the picture (K-331); an effect has no
                    // such argument, and a smaller working size is *better*
                    // here rather than merely cheaper. Between consecutive
                    // frames of a fast camera move the displacement is large,
                    // and an 8×8 patch on a 1080p frame is a tiny window on
                    // content that is often periodic — container ribs, railings,
                    // brickwork — where the patch matches many positions
                    // equally well and picks one. Halving doubles what each
                    // patch spans relative to that repeat, which is the
                    // difference between disambiguating it and guessing. It is
                    // also the working resolution docs/impl/optical-flow.md §1
                    // names as the default, and a quarter of the cost.
                    let set = lumit_flow::FlowSettings {
                        divisor: 2,
                        ..lumit_flow::FlowSettings::default()
                    };
                    let (ga, gb, _) = lumit_flow::flow_grays(&px.rgba, other, w, h, &set);
                    let nb = job
                        .temporal
                        .iter()
                        .find(|(o, _)| *o == offset)
                        .map_or(job.source_frame, |(_, f)| *f);
                    let (fwd, bwd) = flow_for(
                        flow_engine,
                        flow_cache,
                        gpu,
                        job.item,
                        job.source_frame,
                        nb,
                        &ga,
                        &gb,
                        &set,
                    );
                    // The per-pixel confidence Fast motion blur tapers the streak
                    // by (FX-19); the same deterministic function export runs, so
                    // the two match (K-031). Datamosh ignores it.
                    let conf = lumit_flow::confidence(&fwd, &bwd);
                    // The consumers want a field at the frame's own size. The
                    // vectors scale with the image — a 3 px move at half res is
                    // 6 px at full — while the confidence is a 0..1 weight and
                    // must not be touched by that scaling.
                    lumit_flow::field_to_size(&fwd, &conf, w, h)
                }))
            })
            .collect();
        // Blend / Flow policy: combine with the next source frame.
        let rgba = if let Some((ceil, w)) = job.blend {
            let req2 = Request {
                generation: req.generation,
                item: req.item,
                source: job.source.clone(),
                frame: ceil,
                target_width: req.target_width,
                slate: None,
            };
            let px2 = decode(decoders, cache, &req2)?;
            if let Some(params) = &job.flow {
                let set = flow_settings(params);
                let (fw, fh) = (px.width as usize, px.height as usize);
                // Measure through the shared cache, then paint. Splitting the
                // two — rather than calling `interpolate_at`, which does both —
                // is what lets the measurement be reused: synthesis differs per
                // frame because φ does, but the field between two source frames
                // is the same field however many phases are drawn from it, and
                // a slow ramp draws many.
                let (ga, gb, _) = lumit_flow::flow_grays(&px.rgba, &px2.rgba, fw, fh, &set);
                let (fwd, bwd) = flow_for(
                    flow_engine,
                    flow_cache,
                    gpu,
                    job.item,
                    job.source_frame,
                    ceil,
                    &ga,
                    &gb,
                    &set,
                );
                flow_engine
                    .get_or_insert_with(|| flow_engine_for(gpu))
                    .synthesize_at(&px.rgba, &px2.rgba, fw, fh, &fwd, &bwd, w, &set)
            } else {
                lumit_core::pixels::blend_rgba(&px.rgba, &px2.rgba, w)
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
            flow_fields,
            source_key: job.source_key(),
        });
        // One more source frame in hand. Reported after the layer is filed, so
        // "n of m done" is true of what has actually been decoded.
        progress(layers.len());
    }
    Ok(CompFrame {
        comp,
        frame,
        media_epoch,
        layers,
        render_cost: decode_started.elapsed(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **The decode-ahead hand-off.** A frame filed by [`DecodePool::preload`]
    /// must be served by the render's own decode as a cache hit — proven by
    /// requesting it against a path that does not exist, which would error if
    /// anything tried to open a decoder. And the key is the whole contract:
    /// a different decode width is a genuine miss, never a wrong-sized hit.
    #[test]
    fn a_preloaded_frame_is_served_without_touching_the_file() {
        let mut pool = DecodePool::new();
        let item = Uuid::now_v7();
        pool.preload(item, 3, Some(64), 2, 2, vec![200u8; 16]);

        let hit = pool.decode_footage(&Request {
            generation: 0,
            item,
            source: lumit_media::MediaSource::file("Z:/definitely/not/here/gone.mp4"),
            frame: 3,
            target_width: Some(64),
            slate: None,
        });
        let px = hit.expect("preloaded pixels are a cache hit; the file does not exist");
        assert_eq!((px.width, px.height, px.rgba[0]), (2, 2, 200));

        // Same frame, different decode width: not this entry.
        assert!(pool
            .decode_footage(&Request {
                generation: 0,
                item,
                source: lumit_media::MediaSource::file("Z:/definitely/not/here/gone.mp4"),
                frame: 3,
                target_width: None,
                slate: None,
            })
            .is_err());
    }

    /// **The decoder opens from the cached frame index.** Opening a decoder
    /// used to scan every packet of the file, ignoring the sidecar the probe
    /// had just written — seconds of work repeated in every session, paid at
    /// the first preview frame after a project opened.
    ///
    /// Proven by seeding a sidecar that is genuinely this file's (it carries
    /// the file's fingerprint, so the cache accepts it) but cut short to ten
    /// frames. A decoder that reads it clamps a request for frame 40 to frame
    /// 9; one that re-scans the file sees all 120 and answers frame 40.
    #[test]
    fn the_decoder_opens_from_the_cached_frame_index() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = lumit_media::index::tests_support::fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        let cache = dir.path().join("media-index");
        let mut index = lumit_media::index::build_frame_index(&file).unwrap();
        assert_eq!(index.frame_count(), 120, "the fixture is 120 frames");
        index.entries.truncate(10);
        index.save_to(&cache).unwrap();

        let mut pool = DecodePool::new();
        let px = crate::media_index::with_cache_dir(&cache, || {
            pool.decode_footage(&Request {
                generation: 0,
                item: Uuid::now_v7(),
                source: lumit_media::MediaSource::file(file.clone()),
                frame: 40,
                target_width: None,
                slate: None,
            })
        })
        .expect("the fixture decodes");

        assert_eq!(
            px.frame, 9,
            "the decoder must open with the cached index, not a fresh scan"
        );
    }

    /// The other half of the bargain: a sidecar written for different content
    /// is never replayed. The file is re-encoded in place — same path, same
    /// name, different bytes and a different length — and the decoder must
    /// rebuild rather than trust the index it finds.
    #[test]
    fn a_changed_file_is_not_decoded_from_its_old_index() {
        let dir = tempfile::tempdir().unwrap();
        let Some(file) = lumit_media::index::tests_support::fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        let cache = dir.path().join("media-index");
        let mut index = lumit_media::index::build_frame_index(&file).unwrap();
        index.entries.truncate(10);
        index.save_to(&cache).unwrap();

        // The clip is replaced by a longer one at the same path: the stale
        // ten-frame index would clamp frame 40 to 9 if it were reused.
        let Some(replacement) = lumit_media::index::tests_support::vfr_fixture(dir.path()) else {
            eprintln!("skipping: no ffmpeg CLI available for fixture generation");
            return;
        };
        std::fs::copy(&replacement, &file).unwrap();
        let frames = lumit_media::index::build_frame_index(&file)
            .unwrap()
            .frame_count();
        assert!(
            frames > 40,
            "the replacement must be longer than the stale index, got {frames} frames"
        );

        let mut pool = DecodePool::new();
        let px = crate::media_index::with_cache_dir(&cache, || {
            pool.decode_footage(&Request {
                generation: 0,
                item: Uuid::now_v7(),
                source: lumit_media::MediaSource::file(file.clone()),
                frame: 40,
                target_width: None,
                slate: None,
            })
        })
        .expect("the replacement decodes");

        assert_eq!(
            px.frame, 40,
            "a fingerprint mismatch must rebuild the index, never reuse it"
        );
    }

    /// K-331: the flow cache is keyed by content — the source, the two frames,
    /// and the settings that shape the measurement — so a second ask for the
    /// same pair is answered rather than remeasured, and a changed setting is
    /// a different entry rather than a stale one.
    #[test]
    fn flow_entries_are_named_by_what_produced_them() {
        let base = lumit_flow::FlowSettings::default();
        let same = lumit_flow::FlowSettings::default();
        assert_eq!(flow_settings_key(&base), flow_settings_key(&same));

        // Everything that changes the *field* splits the entry.
        for changed in [
            lumit_flow::FlowSettings { divisor: 2, ..base },
            lumit_flow::FlowSettings {
                iterations: 32,
                ..base
            },
            lumit_flow::FlowSettings {
                min_level_dim: 48,
                ..base
            },
            lumit_flow::FlowSettings {
                refine_iters: 3,
                ..base
            },
            lumit_flow::FlowSettings {
                smoothness: 90.0,
                ..base
            },
        ] {
            assert_ne!(
                flow_settings_key(&base),
                flow_settings_key(&changed),
                "a setting that changes the measurement must change its name"
            );
        }
        // The synthesis-side knobs consume the field without altering it, so
        // splitting the cache for them would measure twice for one answer.
        for shared in [
            lumit_flow::FlowSettings {
                occlusion: lumit_flow::OcclusionMode::Blend,
                ..base
            },
            lumit_flow::FlowSettings {
                fallback: lumit_flow::Fallback::Nearest,
                ..base
            },
            lumit_flow::FlowSettings {
                hud_guard: false,
                ..base
            },
        ] {
            assert_eq!(
                flow_settings_key(&base),
                flow_settings_key(&shared),
                "a synthesis knob must not split the measurement cache"
            );
        }
    }

    /// A measured pair comes back from the cache instead of being measured
    /// again — the thing that makes a scrub cheap, and what lets the retime
    /// policy and a flow effect on one layer share a single measurement.
    #[test]
    fn a_measured_flow_pair_is_reused() {
        let mut engine: Option<lumit_flow::FlowEngine> = Some(lumit_flow::FlowEngine::cpu());
        let mut cache = lumit_cache::ByteLru::new(DEFAULT_FLOW_CACHE_BYTES);
        let item = Uuid::now_v7();
        let (w, h) = (32usize, 32usize);
        let px = |shift: usize| -> Vec<u8> {
            let mut v = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let c = (((x + shift) * 7 + y * 13) % 256) as u8;
                    let i = (y * w + x) * 4;
                    v[i] = c;
                    v[i + 1] = c;
                    v[i + 2] = c;
                    v[i + 3] = 255;
                }
            }
            v
        };
        let (a, b) = (px(0), px(2));
        let ga = lumit_flow::to_gray(&a, w, h);
        let gb = lumit_flow::to_gray(&b, w, h);
        let set = lumit_flow::FlowSettings::default();
        let call = |e: &mut Option<lumit_flow::FlowEngine>,
                    c: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>| {
            flow_for(e, c, None, item, 0, 1, &ga, &gb, &set)
        };
        let (f1, _) = call(&mut engine, &mut cache);
        assert_eq!(cache.len(), 1, "the first ask files an entry");
        // Drop the engine entirely: a second ask that had to measure would now
        // have to build a new one, and could not return the identical field.
        let mut none: Option<lumit_flow::FlowEngine> = None;
        let (f2, _) = call(&mut none, &mut cache);
        assert!(none.is_none(), "the second ask never touched an engine");
        assert_eq!(f1.u, f2.u);
        assert_eq!(f1.v, f2.v);
        assert_eq!(f1.valid, f2.valid);
    }
}

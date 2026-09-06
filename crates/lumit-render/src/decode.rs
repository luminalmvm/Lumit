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
    /// How wide `rgba`'s samples are: eight-bit sRGB for nearly everything,
    /// scene-linear float for a float source such as OpenEXR.
    pub format: lumit_media::PixelFormat,
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
    /// Read these four named channels of the file instead of the picture it
    /// opens as (docs/08 §3.97). See [`CompJob::channels`]; `None` on every
    /// ordinary request, which is nearly all of them.
    pub channels: Option<[Option<String>; 4]>,
}

/// One layer's decode job inside a comp render request.
pub struct CompJob {
    pub layer: Uuid,
    pub item: Uuid,
    /// One media file, or the numbered run of stills this item is —
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
    /// `source_frame` with `ceil_frame` at `weight` (the Blend and Flow
    /// retiming policies).
    pub blend: Option<(usize, f32)>,
    /// Set when `blend`'s pair is combined by optical-flow synthesis rather
    /// than a plain crossfade (the Flow policy), carrying the parameters the
    /// synthesis runs with.
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
    /// docs/08 §3.2, wants `1`; Datamosh, §3.12, wants `-1`), sorted and
    /// deduplicated: the decode worker measures the dense motion from this frame
    /// to the neighbour at each offset (already fetched via `temporal`) and
    /// stamps them onto [`CompLayerPixels::flow_fields`]. Empty for a stack that
    /// consumes no flow. See [`lumit_core::fx::stack_flow_neighbours`] - a list
    /// rather than the one slot it used to be.
    pub flow_neighbours: Vec<i32>,
    /// The file could not be found (docs/07 §3.3): the worker synthesises the
    /// test-bar slate at the layer's size instead of decoding, so a comp with
    /// missing footage shows unmistakably-absent bars rather than silent
    /// black — and never fails the whole frame.
    pub slate: bool,
    /// Which of the file's own channels become red, green, blue and alpha, when
    /// this layer carries an enabled Extract channels effect (docs/08 §3.97).
    ///
    /// Set here rather than read in the effect stack because it changes *which
    /// numbers are decoded*, not what happens to them afterwards — the same
    /// shape [`Self::flow`] has, and for the same reason: by the time a stack
    /// runs, the decode has already happened. `None` on every ordinary layer,
    /// which decodes the picture the file opens as.
    pub channels: Option<[Option<String>; 4]>,
    /// The sub-frame moments an accumulation motion blur above this layer
    /// wants its footage at (docs/08 §3.26), one per shutter sample. Empty for
    /// a layer no such adjustment covers, so a plain layer decodes exactly one
    /// frame. Each is the frame pair the moment falls between, picked the way
    /// the Blend policy picks: a moment that lands on a real frame is that
    /// frame alone, and one between two is synthesised from both with
    /// [`Self::shutter_flow`].
    pub shutter: Vec<ShutterSample>,
    /// How the in-between moments in [`Self::shutter`] are made: the layer's
    /// own Flow settings when its Retime uses Flow, otherwise the defaults.
    /// `None` only when `shutter` is empty.
    pub shutter_flow: Option<lumit_core::retime::FlowParams>,
}

/// One sub-frame moment a covered clip is decoded at for accumulation motion
/// blur: which moment (the offset, in comp frames, the adjustment's shutter
/// maths produced, and looks it up by again) and which source frames show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShutterSample {
    pub offset: f64,
    pub source_frame: usize,
    /// `Some((ceil, weight))` when the moment falls between two frames.
    pub blend: Option<(usize, f32)>,
}

/// One measured motion field at a layer's decoded size: per-pixel `u` and `v` in
/// pixels, then the per-pixel confidence in 0..1 (FX-19), each `width × height`.
pub type FlowFieldData = (Vec<f32>, Vec<f32>, Vec<f32>);

pub struct CompLayerPixels {
    pub layer: Uuid,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// How wide `rgba`'s samples are, and those of every neighbour in
    /// `temporal` beside it — they come from the one decoder, so they agree.
    /// Float here is a source (OpenEXR) that kept its range and precision.
    pub format: lumit_media::PixelFormat,
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
    /// absent, which is its consumer's passthrough. Motion blur (docs/08
    /// §3.2, offset `1`) smears along its field, scaling the streak by `conf`
    /// (FX-19); Datamosh (§3.12, offset `-1`) warps the previous frame
    /// along the `(u, v)` and ignores `conf`. **Both at once is one field each,
    /// not one field shared**: they are opposite measurements.
    pub flow_fields: Vec<(i32, FlowFieldData)>,
    /// This clip at each sub-frame moment an accumulation motion blur above
    /// it asked for (see [`CompJob::shutter`]), keyed by offset: a whole
    /// layer-pixels of its own, the same size as `rgba` and named apart from
    /// it, so the builder can stand it in for this entry when it builds that
    /// sample's below-stack without copying a frame. A moment that failed to
    /// decode is simply absent, and that sample falls back to the frame-time
    /// pixels. Boxed so a layer with no shutter costs a pointer, not a struct.
    pub shutter: Vec<(f64, Box<CompLayerPixels>)>,
    /// The content name of this decode: a hash of the [`CompJob`]
    /// identity [`crate::plan::same_decode`] compares — item, path, source
    /// frame, decode width, slate, blend partner, flow settings, temporal
    /// window. Two jobs that decode the same pixels get the same name, which is
    /// what lets the per-effect cache recognise a layer's source across
    /// renders without hashing its bytes.
    pub source_key: u128,
    /// The **source frame** these pixels are, which is what a Roto brush's
    /// matte is indexed by. The stamper works it out again for the
    /// frame key; the draw builder cannot, because the document holds no frame
    /// rate for a media item and this is the one place the plan's answer is
    /// already in hand.
    pub source_frame: i64,
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

/// What names one decoded source frame: the item, the source frame, the decode
/// width, and which of the file's channels were read (docs/08 §3.97). The last
/// is `None` for every ordinary decode.
type FrameCacheKey = (Uuid, usize, Option<u32>, Option<[Option<String>; 4]>);

struct CachedFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    format: lumit_media::PixelFormat,
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
    frame_cache: lumit_cache::ByteLru<FrameCacheKey, CachedFrame>,
    /// Flow backend, created on the first Flow-policy frame. Uses [`Self::gpu`]
    /// — the renderer's own device — when the pool was given one, so flow runs
    /// where the frames are already going rather than on a second device of its
    /// own. Falls back to a headless device, then to the CPU oracle;
    /// lumit-flow degrades by itself and never faults.
    flow_engine: Option<lumit_flow::FlowEngine>,
    /// The renderer's GPU, when the owner shared it.
    gpu: Option<lumit_gpu::GpuContext>,
    /// Measured flow pairs, so a scrub does not remeasure and the two
    /// consumers of a layer's motion share one measurement.
    flow_cache: lumit_cache::ByteLru<FlowKey, CachedFlow>,
    /// How many comp frames this pool has actually decoded. Diagnostic, and the
    /// thing the drag fast path is *measured* by: a value drag must not move it
    /// (see the headless preview tests).
    comp_decodes: u64,
}

/// The decoded-frame cache's default share of RAM; Settings → Performance
/// moves it.
pub const DEFAULT_DECODE_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// The measured-flow cache's share of RAM.
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
/// field.
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
    /// own. The handles are reference-counted clones, so this shares
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
    /// open — the pool's share of the memory report.
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
        decoded: lumit_media::DecodedFrame,
    ) {
        self.frame_cache.insert(
            (item, frame, target_width, None),
            CachedFrame {
                width: decoded.width,
                height: decoded.height,
                rgba: decoded.rgba,
                format: decoded.format,
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
    cache: &mut lumit_cache::ByteLru<FrameCacheKey, CachedFrame>,
    req: &Request,
) -> Result<FramePixels, String> {
    if let Some((w, h)) = req.slate {
        let (w, h) = (w.max(1), h.max(1));
        return Ok(FramePixels {
            width: w,
            height: h,
            rgba: lumit_media::slate::colour_bars(w, h),
            // The slate is drawn here, in bytes, whatever the missing file
            // would have been.
            format: lumit_media::PixelFormat::Srgb8,
            frame: req.frame,
            item: req.item,
        });
    }
    // The extracted channels name the pixels as surely as the frame number
    // does — the same file read as `Z` is not the picture it opens as — so they
    // are part of the key rather than something the cache could get wrong.
    let cache_key = (req.item, req.frame, req.target_width, req.channels.clone());
    if let Some(hit) = cache.get(&cache_key) {
        return Ok(FramePixels {
            width: hit.width,
            height: hit.height,
            rgba: hit.rgba.clone(),
            format: hit.format,
            frame: req.frame,
            item: req.item,
        });
    }
    // Extract channels: the file's own reader, because the named channels are
    // the thing ffmpeg cannot reach (docs/impl/media-io.md §5b). Only an EXR
    // takes this path; anything else falls through and decodes as itself, which
    // is what the effect's dropdowns offering nothing already told the user.
    if let Some(slots) = &req.channels {
        if let Some(path) = lumit_media::exr::file_for(&req.source, req.frame) {
            let out = lumit_media::exr::downsample(
                lumit_media::exr::read_channels(&path, slots).map_err(|e| e.to_string())?,
                req.target_width,
            );
            cache.insert(
                cache_key,
                CachedFrame {
                    width: out.width,
                    height: out.height,
                    rgba: out.rgba.clone(),
                    format: out.format,
                },
            );
            return Ok(FramePixels {
                width: out.width,
                height: out.height,
                rgba: out.rgba,
                format: out.format,
                frame: req.frame,
                item: req.item,
            });
        }
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
            format: out.format,
        },
    );
    Ok(FramePixels {
        width: out.width,
        height: out.height,
        rgba: out.rgba,
        format: out.format,
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
            // The Viewer shows a footage item as itself; the effect that
            // extracts channels lives on a layer, which is the comp path.
            channels: None,
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
/// measured for them.
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

/// The flow engine for this pool: on the renderer's device when one was
/// shared, otherwise a headless device of its own, otherwise the CPU oracle.
fn flow_engine_for(gpu: Option<&lumit_gpu::GpuContext>) -> lumit_flow::FlowEngine {
    match gpu {
        Some(ctx) => lumit_flow::FlowEngine::with_context(ctx),
        None => lumit_flow::FlowEngine::new_auto(),
    }
}

/// Translate a layer's stored [`lumit_core::retime::FlowParams`] into the
/// plain-numbers [`lumit_flow::FlowSettings`] the engine takes.
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

/// The content name of a clip's picture at one shutter moment: its frame-time
/// name and the moment, so the per-effect cache files the two apart.
fn moment_key(source_key: u128, offset: f64) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(b"shutter-moment/1/");
    h.update(&source_key.to_le_bytes());
    h.update(&offset.to_le_bytes());
    let mut k = [0u8; 16];
    k.copy_from_slice(&h.finalize().as_bytes()[..16]);
    u128::from_le_bytes(k)
}

/// The picture at a moment between two source frames: `a` alone when `blend`
/// is `None`, else `a` and the `ceil` frame combined at `weight`, by flow
/// synthesis when `flow` says so and by a plain crossfade otherwise. The one
/// place a Blend or Flow retime and an accumulation shutter sample make their
/// in-between frame, so the two cannot disagree about what a moment looks like.
///
/// Flow measures through the shared cache and then paints. Splitting the two,
/// rather than calling `interpolate_at` which does both, is what lets the
/// measurement be reused: synthesis differs per moment because the weight
/// does, but the field between two source frames is the same field however
/// many moments are drawn from it, and a slow ramp or an eight-sample shutter
/// draws many.
#[allow(clippy::too_many_arguments)]
fn combine_pair(
    decoders: &mut HashMap<Uuid, lumit_media::VideoDecoder>,
    cache: &mut lumit_cache::ByteLru<FrameCacheKey, CachedFrame>,
    flow_engine: &mut Option<lumit_flow::FlowEngine>,
    flow_cache: &mut lumit_cache::ByteLru<FlowKey, CachedFlow>,
    gpu: Option<&lumit_gpu::GpuContext>,
    job: &CompJob,
    a: FramePixels,
    blend: Option<(usize, f32)>,
    flow: Option<&lumit_core::retime::FlowParams>,
) -> Result<Vec<u8>, String> {
    let Some((ceil, w)) = blend else {
        return Ok(a.rgba);
    };
    let req2 = Request {
        generation: 0,
        item: job.item,
        source: job.source.clone(),
        frame: ceil,
        target_width: job.target_width,
        slate: None,
        // The retime's other end is read through the same channels, or the
        // crossfade would be between two different pictures.
        channels: job.channels.clone(),
    };
    let b = decode(decoders, cache, &req2)?;
    // Both policies keep the plate's own width: a retimed OpenEXR is still an
    // OpenEXR (docs/impl/media-io.md §5a).
    let float = a.format == lumit_media::PixelFormat::LinearF32;
    let Some(params) = flow else {
        return Ok(if float {
            lumit_core::pixels::blend_f32(&a.rgba, &b.rgba, w)
        } else {
            lumit_core::pixels::blend_rgba(&a.rgba, &b.rgba, w)
        });
    };
    let set = flow_settings(params);
    let (fw, fh) = (a.width as usize, a.height as usize);
    let (ga, gb, _) = if float {
        lumit_flow::flow_grays_f32(&a.rgba, &b.rgba, fw, fh, &set)
    } else {
        lumit_flow::flow_grays(&a.rgba, &b.rgba, fw, fh, &set)
    };
    let (fwd, bwd) = flow_for(
        flow_engine,
        flow_cache,
        gpu,
        job.item,
        a.frame,
        ceil,
        &ga,
        &gb,
        &set,
    );
    let engine = flow_engine.get_or_insert_with(|| flow_engine_for(gpu));
    // The float synthesis runs on the processor rather than the card — see
    // `synthesize_at_f32` for why — and gives the same picture the eight-bit
    // one would, without rounding it.
    Ok(if float {
        engine.synthesize_at_f32(&a.rgba, &b.rgba, fw, fh, &fwd, &bwd, w, &set)
    } else {
        engine.synthesize_at(&a.rgba, &b.rgba, fw, fh, &fwd, &bwd, w, &set)
    })
}

#[allow(clippy::too_many_arguments)] // one worker call; bundling would hide it
fn decode_comp(
    decoders: &mut HashMap<Uuid, lumit_media::VideoDecoder>,
    cache: &mut lumit_cache::ByteLru<FrameCacheKey, CachedFrame>,
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
            channels: job.channels.clone(),
        };
        // Missing media renders the slate; nothing else about the layer
        // changes, so transforms, effects and blending all still apply.
        let px = if job.slate {
            let (w, h) = (job.natural_w.max(1), job.natural_h.max(1));
            FramePixels {
                width: w,
                height: h,
                rgba: lumit_media::slate::colour_bars(w, h),
                format: lumit_media::PixelFormat::Srgb8,
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
                    // A neighbour frame is the same picture at another moment,
                    // so it is read through the same channels — an echo of an
                    // extracted depth pass is an echo of the depth pass.
                    channels: job.channels.clone(),
                };
                decode(decoders, cache, &nreq)
                    .ok()
                    .map(|p| (offset, p.rgba))
            })
            .collect();
        // Flow motion blur (docs/08 §3.2, offset +1) and Datamosh
        // (§3.12, offset -1) both need a dense motion field: the forward
        // flow from this frame to the requested neighbour (already
        // decoded above). Computed from the raw current frame before it
        // is consumed into `rgba` below, where both frames live as RGBA —
        // exactly as the Flow retiming policy computes its flow, on the
        // shared engine that reuses the GPU when one is present. A
        // dropped neighbour just leaves that offset absent, degrading its
        // flow-consuming effect to a passthrough.
        //
        // **One measurement per offset asked for**. The two effects
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
                    // export agree about the picture; an effect has no
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
                    // The per-pixel confidence Motion blur tapers the streak
                    // by (FX-19); the same deterministic function export runs, so
                    // the two match. Datamosh ignores it.
                    let conf = lumit_flow::confidence(&fwd, &bwd);
                    // The consumers want a field at the frame's own size. The
                    // vectors scale with the image — a 3 px move at half res is
                    // 6 px at full — while the confidence is a 0..1 weight and
                    // must not be touched by that scaling.
                    lumit_flow::field_to_size(&fwd, &conf, w, h)
                }))
            })
            .collect();
        // The moments an accumulation motion blur above wants this clip at
        // (empty for a plain layer, so this loop does nothing then). Each is
        // made the way the primary frame is made under a Blend or Flow retime,
        // and a moment that fails to decode is dropped: that sample then shows
        // the frame-time pixels, which degrades the blur, never the frame.
        let source_key = job.source_key();
        // Read off the primary frame before it is combined away: every moment
        // of this clip is the same file at the same width, so they all carry
        // the plate's own sample width (docs/impl/media-io.md §5a).
        let format = px.format;
        let shutter: Vec<(f64, Box<CompLayerPixels>)> = job
            .shutter
            .iter()
            .filter_map(|s| {
                let sreq = Request {
                    generation: 0,
                    item: job.item,
                    source: job.source.clone(),
                    frame: s.source_frame,
                    target_width: job.target_width,
                    slate: None,
                    // A shutter moment is the same picture at another instant,
                    // so it is read through the same channels.
                    channels: job.channels.clone(),
                };
                let spx = decode(decoders, cache, &sreq).ok()?;
                let (width, height) = (spx.width, spx.height);
                let rgba = combine_pair(
                    decoders,
                    cache,
                    flow_engine,
                    flow_cache,
                    gpu,
                    job,
                    spx,
                    s.blend,
                    job.shutter_flow.as_ref(),
                )
                .ok()?;
                Some((
                    s.offset,
                    Box::new(CompLayerPixels {
                        layer: job.layer,
                        width,
                        height,
                        rgba,
                        format,
                        natural_w: job.natural_w,
                        natural_h: job.natural_h,
                        // A sample render strips temporal inputs anyway, so
                        // none are decoded for a moment.
                        temporal: Vec::new(),
                        flow_fields: Vec::new(),
                        shutter: Vec::new(),
                        // A different picture, so a different name: the
                        // per-effect cache must not hand this moment the
                        // frame-time output.
                        source_key: moment_key(source_key, s.offset),
                        source_frame: i64::try_from(s.source_frame).unwrap_or(0),
                    }),
                ))
            })
            .collect();
        // Blend / Flow policy: combine with the next source frame.
        let (width, height) = (px.width, px.height);
        let rgba = combine_pair(
            decoders,
            cache,
            flow_engine,
            flow_cache,
            gpu,
            job,
            px,
            job.blend,
            job.flow.as_ref(),
        )?;
        layers.push(CompLayerPixels {
            layer: job.layer,
            width,
            height,
            rgba,
            format,
            natural_w: job.natural_w,
            natural_h: job.natural_h,
            temporal,
            flow_fields,
            shutter,
            source_key,
            source_frame: i64::try_from(job.source_frame).unwrap_or(0),
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
        pool.preload(
            item,
            3,
            Some(64),
            lumit_media::DecodedFrame {
                width: 2,
                height: 2,
                rgba: vec![200u8; 16],
                format: lumit_media::PixelFormat::Srgb8,
            },
        );

        let hit = pool.decode_footage(&Request {
            generation: 0,
            item,
            source: lumit_media::MediaSource::file("Z:/definitely/not/here/gone.mp4"),
            frame: 3,
            target_width: Some(64),
            slate: None,
            channels: None,
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
                channels: None,
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
                channels: None,
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
                channels: None,
            })
        })
        .expect("the replacement decodes");

        assert_eq!(
            px.frame, 40,
            "a fingerprint mismatch must rebuild the index, never reuse it"
        );
    }

    /// The flow cache is keyed by content — the source, the two frames,
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

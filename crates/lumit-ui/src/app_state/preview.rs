//! Latest-wins background frame decoding for the Viewer (slice 5), moved
//! verbatim from app_state.rs.

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
    /// Set when the stack has a flow-consuming effect (Flow motion blur,
    /// docs/08 §3.2, wants `1`; Datamosh, §3.12, K-104, wants `-1`): the
    /// decode worker measures the dense motion from this frame to the
    /// neighbour at that offset (already fetched via `temporal`) and
    /// stamps it onto [`CompLayerPixels::flow_field`]. See
    /// [`lumit_core::fx::stack_flow_neighbour`].
    pub flow_neighbour: Option<i32>,
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
    /// Dense forward flow (per-pixel `(u, v)` motion in pixels, plus a per-pixel
    /// `conf`idence in 0..1, row-major, same `width × height` as `rgba`) from
    /// this frame to the neighbour at [`CompJob::flow_neighbour`]'s offset,
    /// present only when that neighbour decoded. Fast motion blur (docs/08 §3.2,
    /// offset `1`) smears along it, scaling the streak by `conf` (FX-19);
    /// Datamosh (§3.12, K-104, offset `-1`) warps the previous frame along the
    /// `(u, v)` and ignores `conf`.
    pub flow_field: Option<(Vec<f32>, Vec<f32>, Vec<f32>)>,
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
        // Flow motion blur (docs/08 §3.2, offset +1) and Datamosh (§3.12,
        // K-104, offset -1) both need a dense motion field: the forward
        // flow from this frame to the requested neighbour (already
        // decoded above). Computed from the raw current frame before it
        // is consumed into `rgba` below, where both frames live as RGBA —
        // exactly as the Flow retiming policy computes its flow, on the
        // shared engine that reuses the GPU when one is present. A
        // dropped neighbour just leaves it None, degrading the
        // flow-consuming effect to a passthrough.
        let flow_field = job.flow_neighbour.and_then(|offset| {
            temporal
                .iter()
                .find(|(o, _)| *o == offset)
                .map(|(_, other)| {
                    let (w, h) = (px.width as usize, px.height as usize);
                    let ga = lumit_flow::to_gray(&px.rgba, w, h);
                    let gb = lumit_flow::to_gray(other, w, h);
                    let (fwd, bwd) = flow_engine
                        .get_or_insert_with(lumit_flow::FlowEngine::new_auto)
                        .flow_pair(&ga, &gb);
                    // The per-pixel confidence Fast motion blur tapers the streak
                    // by (FX-19); the same deterministic function export runs, so
                    // the two match (K-031). Datamosh ignores it.
                    let conf = lumit_flow::confidence(&fwd, &bwd);
                    (fwd.u, fwd.v, conf)
                })
        });
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

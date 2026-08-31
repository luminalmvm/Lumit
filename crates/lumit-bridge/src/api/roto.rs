//! The Roto brush's surface across the seam (K-713, docs/impl/roto.md §5–§8):
//! strokes down, the two buttons down, the span and the progress up.
//!
//! # In plain terms
//!
//! The cutting-out itself happens elsewhere — on its own thread, in
//! `lumit-render`, over the media file rather than over the layer. This module
//! is the doorway. It carries a scribble the user drew on the picture into the
//! document as a stroke, turns a press of **Propagate** into a job, and answers
//! "how far along is it, and how much of the shot does the matte cover" whenever
//! the panel repaints.
//!
//! **Nothing here does arithmetic the interface could not check.** A stroke
//! arrives already in **source raster pixels** — the viewer converts, because
//! only it knows the chain of transforms the pointer came through — and is
//! stored exactly as it arrives (K-248). A refusal comes back as a *reason*,
//! never as a sentence: the words are Dart's, from the arb, the way the camera
//! track's are (K-303, tracking.md §5c deviation 3).
//!
//! **The strokes ride the ordinary effect-stack commit.** Adding a stroke stages
//! it on the handle and `LayerReference::set_effects` commits, so a scribble is
//! one `SetLayerEffects`, one journal entry and one undo step, exactly as a
//! parameter edit or a shader edit is. There is no roto-shaped op, because there
//! is no roto-shaped question the stack commit cannot answer.

use flutter_rust_bridge::frb;
use lumit_core::model::LayerKind;
use lumit_core::roto::{RotoBlock, RotoStroke, RotoStrokeKind, ROTO_BRUSH};
use uuid::Uuid;

use crate::api::{effect::BridgeEffectInstance, layer::LayerReference, BridgeError};

/// The Roto brush's two Action parameters, by the ids its schema declares
/// (`crates/lumit-core/src/fx/effects/roto_brush.rs`). Spelled once, here,
/// because a typo would be silent: the press would simply do nothing.
pub(crate) const PROPAGATE: &str = "propagate";
pub(crate) const CANCEL: &str = "cancel";

// ---------------------------------------------------------------------------
// Strokes, down
// ---------------------------------------------------------------------------

/// What a stroke claims — the bridge's copy of
/// [`lumit_core::roto::RotoStrokeKind`], so Dart switches over a generated enum
/// rather than passing a number nobody checks.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRotoStrokeKind {
    /// This is the subject.
    Foreground,
    /// This is not.
    Background,
    /// Give this the refine band, whatever the segmentation decided.
    Refine,
}

impl BridgeRotoStrokeKind {
    fn read(self) -> RotoStrokeKind {
        match self {
            BridgeRotoStrokeKind::Foreground => RotoStrokeKind::Foreground,
            BridgeRotoStrokeKind::Background => RotoStrokeKind::Background,
            BridgeRotoStrokeKind::Refine => RotoStrokeKind::Refine,
        }
    }
}

/// One validated stroke from the wire's flat form. An odd-length `points`, a
/// stroke with none, a non-finite coordinate or a non-positive radius is
/// refused rather than stored: a stroke nobody can stamp is a stroke that
/// would silently do nothing.
fn stroke_of(
    points: &[f32],
    radius: f32,
    kind: BridgeRotoStrokeKind,
    frame: i64,
) -> Result<RotoStroke, BridgeError> {
    if points.is_empty() || !points.len().is_multiple_of(2) || !points.iter().all(|v| v.is_finite())
    {
        return Err(BridgeError::InvalidParam);
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(BridgeError::InvalidParam);
    }
    Ok(RotoStroke {
        id: Uuid::now_v7(),
        points: points.chunks_exact(2).map(|p| (p[0], p[1])).collect(),
        radius,
        kind: kind.read(),
        frame,
    })
}

impl LayerReference {
    /// The first scribble on a layer that carries no Roto brush: add the brush
    /// **and** file the stroke, in one commit (K-723, superseding K-717's
    /// refusal).
    ///
    /// The stroke rides *inside* the new instance, so the whole gesture is one
    /// `SetLayerEffects` — one op, one journal entry, one undo step, exactly
    /// what a scribble on a layer that already had the brush costs. K-717
    /// refused this on the grounds that `add_effect` plus a stroke would be two
    /// ops; landing the stroke in the instance before it is pushed is what
    /// dissolves that. The stroke sets the base frame, as a first stroke always
    /// does.
    ///
    /// Answers the new instance's id — what the release-time solve
    /// ([`roto_solve_frame`]) and the overlay's next read are addressed to.
    #[frb(sync)]
    pub fn roto_first_stroke(
        &self,
        points: Vec<f32>,
        radius: f32,
        kind: BridgeRotoStrokeKind,
        frame: i64,
    ) -> Result<Uuid, BridgeError> {
        let stroke = stroke_of(&points, radius, kind, frame)?;
        let comp = self.composition()?;
        // Exactly what `add_effect` builds, minus its driver fork — the Roto
        // brush is an image op — so the instance a scribble adds and the one
        // the menu adds cannot drift apart.
        let mut instance = lumit_core::fx::instantiate_for_raster(
            ROTO_BRUSH,
            f64::from(comp.width),
            f64::from(comp.height),
        )
        .ok_or(BridgeError::UnknownEffectName)?;
        lumit_core::fx::point_self_layer_params_at(&mut instance, self.layer_id);
        instance.roto = Some(RotoBlock {
            base_frame: Some(frame),
            strokes: vec![stroke],
        });
        let id = instance.id;
        self.with_effects(move |effects| {
            effects.push(instance);
            Ok(())
        })?;
        Ok(id)
    }
}

impl BridgeEffectInstance {
    /// Add one stroke to this Roto brush, on the **staged** copy (K-713).
    ///
    /// `points` are `[x0, y0, x1, y1, …]` in **source raster pixels** on the
    /// unaltered footage — the viewer converts, because only it knows the chain
    /// of transforms the pointer came through, and the matte has to describe the
    /// file's frames rather than this comp's (K-248). `frame` is the source
    /// frame the stroke was drawn on.
    ///
    /// **The first stroke sets the base frame**, which is what makes Propagate
    /// answerable at all; a later stroke on another frame is a *correction* and
    /// leaves the base where it is. Moving the base is
    /// [`Self::roto_set_base_frame`], deliberately a separate gesture.
    ///
    /// An odd-length `points`, a stroke with none, or a non-finite coordinate is
    /// refused rather than stored: a stroke nobody can stamp is a stroke that
    /// would silently do nothing.
    #[frb(sync)]
    pub fn roto_add_stroke(
        &mut self,
        points: Vec<f32>,
        radius: f32,
        kind: BridgeRotoStrokeKind,
        frame: i64,
    ) -> Result<(), BridgeError> {
        if self.effect.effect.match_name != ROTO_BRUSH {
            return Err(BridgeError::InvalidEffect);
        }
        let stroke = stroke_of(&points, radius, kind, frame)?;
        let block = self.roto_block_mut();
        block.base_frame.get_or_insert(frame);
        block.strokes.push(stroke);
        Ok(())
    }

    /// Move the base frame — the frame propagation runs outward from — on the
    /// staged copy.
    ///
    /// A real edit and not a preference: every cached matte depends on it, so
    /// moving it retires the whole run, which is exactly what a user asking for
    /// the shot to be re-decided from somewhere else means.
    #[frb(sync)]
    pub fn roto_set_base_frame(&mut self, frame: Option<i64>) -> Result<(), BridgeError> {
        if self.effect.effect.match_name != ROTO_BRUSH {
            return Err(BridgeError::InvalidEffect);
        }
        self.roto_block_mut().base_frame = frame;
        Ok(())
    }

    /// Throw away every stroke and the base frame, on the staged copy — the
    /// panel's "start again". The cached mattes are not touched: they are keyed
    /// by the strokes that made them, so they are simply never asked for again,
    /// and an undo brings the strokes and their mattes both back.
    #[frb(sync)]
    pub fn roto_clear(&mut self) -> Result<(), BridgeError> {
        if self.effect.effect.match_name != ROTO_BRUSH {
            return Err(BridgeError::InvalidEffect);
        }
        self.effect.roto = None;
        Ok(())
    }

    /// Every stroke this instance holds, for the overlay to draw (K-713).
    ///
    /// Read on the gesture that needs it and on a document revision moving —
    /// never per rebuild, which is the contract every other panel read has
    /// (K-681).
    #[frb(sync)]
    pub fn roto_strokes(&self) -> Vec<BridgeRotoStroke> {
        self.roto_block()
            .map(|block| {
                block
                    .strokes
                    .iter()
                    .map(|s| BridgeRotoStroke {
                        id: s.id,
                        points: s.points.iter().flat_map(|p| [p.0, p.1]).collect(),
                        radius: s.radius,
                        kind: match s.kind {
                            RotoStrokeKind::Foreground => BridgeRotoStrokeKind::Foreground,
                            RotoStrokeKind::Background => BridgeRotoStrokeKind::Background,
                            RotoStrokeKind::Refine => BridgeRotoStrokeKind::Refine,
                        },
                        frame: s.frame,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[frb(ignore)]
    fn roto_block(&self) -> Option<&RotoBlock> {
        self.effect.roto.as_ref()
    }

    #[frb(ignore)]
    fn roto_block_mut(&mut self) -> &mut RotoBlock {
        self.effect.roto.get_or_insert_with(RotoBlock::default)
    }
}

/// One stored stroke, as the overlay draws it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRotoStroke {
    pub id: Uuid,
    /// `[x0, y0, x1, y1, …]` in source raster pixels (K-248).
    pub points: Vec<f32>,
    pub radius: f32,
    pub kind: BridgeRotoStrokeKind,
    pub frame: i64,
}

// ---------------------------------------------------------------------------
// The status row, up
// ---------------------------------------------------------------------------

/// How far a propagation has got — the bridge form of
/// [`lumit_render::roto::Progress`], flattened so the panel reads fields rather
/// than unwrapping a shape.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRotoStage {
    /// Nothing has ever been asked for this brush in this session.
    Idle,
    /// Accepted, not started.
    Queued,
    /// Working outward from the base frame.
    Solving,
    /// There is a run in the store.
    Done,
    /// Stopped between frames. **The finished prefix was kept** (K-540), so the
    /// span below is real and a later Propagate resumes from it.
    Cancelled,
    /// Refused — see [`BridgeRotoStatus::failure`].
    Failed,
}

/// Why a propagation produced no mattes.
///
/// A **reason, not a sentence** (K-303): the engine's own `RotoFailure` carries
/// English, and English crossing here would ship untranslated inside a
/// translated window. Dart switches over this and picks the arb key, which is
/// the shape `BridgeTrackFailure` already uses.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRotoFailure {
    /// No resolved media fingerprint — nothing to key a cache with.
    Offline,
    /// No optical flow on this device. Refused rather than degraded to the CPU
    /// oracle, which at seconds a frame pair would look like a hang.
    FlowUnavailable,
    /// One propagation at a time.
    Busy,
    /// Propagate pressed before any stroke.
    NoBaseFrame,
    /// The media could not be opened, or carries no video.
    Unreadable,
    /// Opened, but with no frames to propagate through.
    NoFrames,
    /// The base frame's strokes do not describe a subject.
    NoSeeds,
}

impl BridgeRotoFailure {
    fn read(e: lumit_render::roto::RotoFailure) -> BridgeRotoFailure {
        use lumit_render::roto::RotoFailure as F;
        match e {
            F::Offline => BridgeRotoFailure::Offline,
            F::FlowUnavailable => BridgeRotoFailure::FlowUnavailable,
            F::Busy => BridgeRotoFailure::Busy,
            F::NoBaseFrame => BridgeRotoFailure::NoBaseFrame,
            F::Unreadable => BridgeRotoFailure::Unreadable,
            F::NoFrames => BridgeRotoFailure::NoFrames,
            F::NoSeeds => BridgeRotoFailure::NoSeeds,
        }
    }
}

/// Everything the Roto brush's status row draws, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeRotoStatus {
    pub stage: BridgeRotoStage,
    /// Frames finished so far, and how many the clip has. Both zero outside
    /// [`BridgeRotoStage::Solving`].
    pub done: u32,
    pub total: u32,
    /// How many of `done` were **copied** from an earlier run rather than
    /// solved — the number that makes prefix reuse visible to the person
    /// waiting (docs/impl/roto.md §5).
    pub reused: u32,
    /// Set only at [`BridgeRotoStage::Failed`].
    pub failure: Option<BridgeRotoFailure>,
    /// The span the matte actually covers, in **source** frames, or `None`
    /// before anything has been propagated. Outside it the effect is a
    /// passthrough and the panel says so.
    pub first_frame: Option<i64>,
    pub last_frame: Option<i64>,
    /// How many frames the clip has, against which the span is whole or
    /// partial. Zero when nothing has been propagated.
    pub clip_frames: u32,
    /// Whether there is a base frame at all — what makes Propagate offerable
    /// rather than a button that refuses.
    pub base_frame: Option<i64>,
    /// How many strokes the instance holds.
    pub strokes: u32,
}

/// The Roto brush `effect` on `layer`, as its status row draws it.
///
/// **Polled while it is moving and never otherwise** (the camera track's §5c
/// second deviation): a press moves no document revision, so there is nothing to
/// refresh against, and the engine keeps progress as a value in a map precisely
/// so nobody has to hold a subscription.
#[frb(sync)]
pub fn roto_status(layer: LayerReference, effect: Uuid) -> Result<BridgeRotoStatus, BridgeError> {
    let item = layer.item()?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    if fx.effect.match_name != ROTO_BRUSH {
        return Err(BridgeError::InvalidEffect);
    }
    let block = fx.roto.clone().unwrap_or_default();
    let run = lumit_render::roto::propagated(effect);
    let (stage, done, total, reused, failure) = match lumit_render::roto::progress(effect) {
        None => (BridgeRotoStage::Idle, 0, 0, 0, None),
        Some(lumit_render::roto::Progress::Queued) => (BridgeRotoStage::Queued, 0, 0, 0, None),
        Some(lumit_render::roto::Progress::Solving {
            done,
            total,
            reused,
        }) => (
            BridgeRotoStage::Solving,
            done as u32,
            total as u32,
            reused as u32,
            None,
        ),
        Some(lumit_render::roto::Progress::Done) => (BridgeRotoStage::Done, 0, 0, 0, None),
        Some(lumit_render::roto::Progress::Cancelled) => {
            (BridgeRotoStage::Cancelled, 0, 0, 0, None)
        }
        Some(lumit_render::roto::Progress::Failed(e)) => (
            BridgeRotoStage::Failed,
            0,
            0,
            0,
            Some(BridgeRotoFailure::read(e)),
        ),
    };
    Ok(BridgeRotoStatus {
        stage,
        done,
        total,
        reused,
        failure,
        first_frame: run.as_ref().map(|r| r.first_frame),
        last_frame: run.as_ref().map(|r| r.last_frame),
        clip_frames: run.as_ref().map_or(0, |r| r.clip_frames as u32),
        base_frame: block.base_frame,
        strokes: block.strokes.len() as u32,
    })
}

// ---------------------------------------------------------------------------
// Which frame of the file is on screen, and where its matte's edge runs
// ---------------------------------------------------------------------------

/// Which frame of the **file** this layer is showing at composition frame
/// `frame` (K-248, K-717).
///
/// A stroke's `frame` is a source frame index, and the viewer only knows the
/// composition's ruler. Between the two sit the layer's start offset and its
/// Retime map, both of which live in the document and neither of which Dart can
/// evaluate — a Retime is a property curve. So the one number the gesture is
/// missing is asked for here, through exactly the arithmetic the decode planner
/// does (`layer_time` → `source_time_at` → `frame_pick`), because a stroke filed
/// against the wrong frame of a retimed layer would seed a frame the user never
/// looked at and be silently, invisibly wrong.
///
/// Read on a frame change and held, never per rebuild (K-681).
///
/// `NotFootage` for anything but a footage layer, and for media that will not
/// probe: a Roto brush on a layer with no file behind it has no source frame to
/// name, which is a refusal rather than a guess at zero.
#[frb(sync)]
pub fn roto_source_frame(layer: LayerReference, frame: i64) -> Result<i64, BridgeError> {
    let comp = layer.composition()?;
    let item = layer.item()?;
    let LayerKind::Footage { item: media } = item.kind else {
        return Err(BridgeError::NotFootage);
    };
    let t_comp = comp
        .frame_rate
        .time_of_frame(frame)
        .map_err(|_| BridgeError::InvalidParam)?;
    let lt = lumit_core::time::layer_time(t_comp.0.to_f64(), item.start_offset.0);
    let source_time = item.source_time_at(lt);
    let (fps, frames) = media_rate(&layer, media)?;
    // `blend` false and no sample rate: the frame a stroke is filed against is
    // the frame the user is *looking at*, which is the nearest native one — a
    // blend pair names two, and a matte is filed under one.
    let (picked, _) = lumit_core::pixels::frame_pick(source_time, fps, frames, false, None);
    i64::try_from(picked).map_err(|_| BridgeError::InvalidParam)
}

/// The media's own rate and how many frames it runs, from the probe cache.
///
/// The frame count is `duration × rate` rounded, which is the same sum
/// `add_footage_layer` sizes a clip's span with — the two must not disagree
/// about where a shot ends. It is a frame coarser than the media index's exact
/// count, and it is used for one thing: clamping the last frame.
#[frb(ignore)]
fn media_rate(layer: &LayerReference, media: Uuid) -> Result<(f64, usize), BridgeError> {
    let project = layer.project()?;
    let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
    let doc = state.store.snapshot();
    let footage = doc
        .items
        .iter()
        .find_map(|i| match i {
            lumit_core::model::ProjectItem::Footage(f) if f.id == media => Some(f),
            _ => None,
        })
        .ok_or(BridgeError::InvalidItem)?;

    #[cfg(feature = "media")]
    {
        let src = crate::api::footage::FootageReference::resolve_source(&state, footage)
            .ok_or(BridgeError::NotFootage)?;
        let info = crate::probe::ensure_probed(&src).ok_or(BridgeError::NotFootage)?;
        let video = info.video.as_ref().ok_or(BridgeError::NotFootage)?;
        let fps = video.fps();
        if fps <= 0.0 {
            return Err(BridgeError::NotFootage);
        }
        let frames = (info.duration_seconds * fps).round().max(1.0) as usize;
        Ok((fps, frames))
    }

    // Without a decoder nothing probes, and a rate invented here would file
    // strokes against frames no build with a decoder agrees about.
    #[cfg(not(feature = "media"))]
    {
        let _ = footage;
        Err(BridgeError::NotFootage)
    }
}

/// How many boundary points cross at most. Twelve thousand outlines a subject
/// at 1080p several times over; past that the outline is thinned evenly rather
/// than cut short, so a busy matte draws a sparser edge and never half an edge.
const MAX_BOUNDARY_POINTS: usize = 12_000;

/// Where the propagated matte's **edge** runs at `frame`, as
/// `[x0, y0, x1, y1, …]` in source raster pixels (K-717).
///
/// The Roto brush's **Boundary** view keeps the picture and asks the viewer to
/// draw the edge over it (`lumit_core::fx::effects::roto_brush::VIEW_OPTIONS`),
/// which is the overlay's business rather than the stack's. This is the one
/// thing the overlay cannot work out for itself: the matte is in the store, and
/// the matte itself deliberately never crosses (docs/17) — it is two megabytes
/// a frame of pixels the render path already reads on its own way to the card.
/// Its outline is a few thousand numbers.
///
/// Empty — no run, outside the propagated span, the cache folder deleted — is
/// the passthrough's honest answer and never a fault.
///
/// ponytail: a full-plane scan per frame change (~2 ms at 1080p) emitting
/// stride-thinned edge *pixels* rather than a traced contour, so a hairline edge
/// draws as dots rather than as a line. The upgrade is a contour traced once and
/// filed with the run. Observable trigger: the boundary reading as dotted at
/// ordinary magnification, or a scrub with the overlay up dropping frames.
#[frb(sync)]
pub fn roto_boundary(effect: Uuid, frame: i64) -> Vec<f32> {
    match lumit_render::roto::matte(effect, frame) {
        Some((width, height, gray)) => boundary_of(width, height, &gray),
        None => Vec::new(),
    }
}

/// [`roto_boundary`]'s scan, over a plane rather than over the store — so the
/// geometry can be asserted without a propagation, which is a minute of decoding
/// a real file away.
#[frb(ignore)]
fn boundary_of(width: u32, height: u32, gray: &[u8]) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    if w < 2 || h < 2 || gray.len() < w * h {
        return Vec::new();
    }
    // A pixel is on the edge when it disagrees with the neighbour to its right
    // or the one below — the same two comparisons the matte's own bounding box
    // is derived from, and enough to enclose the subject once.
    let edge = |i: usize| {
        (gray[i] >= 128) != (gray[i + 1] >= 128) || (gray[i] >= 128) != (gray[i + w] >= 128)
    };
    let mut count = 0usize;
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            if edge(y * w + x) {
                count += 1;
            }
        }
    }
    if count == 0 {
        return Vec::new();
    }
    // Counted before anything is allocated, so the answer is exactly as long as
    // it needs to be and never grows with the picture (docs/14 §5).
    let stride = count.div_ceil(MAX_BOUNDARY_POINTS).max(1);
    let mut out = Vec::with_capacity(count.div_ceil(stride) * 2);
    let mut seen = 0usize;
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            if !edge(y * w + x) {
                continue;
            }
            if seen.is_multiple_of(stride) {
                out.push(x as f32);
                out.push(y as f32);
            }
            seen += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The buttons, down
// ---------------------------------------------------------------------------

/// The propagation job one Roto brush on one footage layer describes — the
/// shared front half of a Propagate press and of the release-time solve.
fn job_of(
    layer: &LayerReference,
    fx: &lumit_core::model::EffectInstance,
) -> Result<lumit_render::roto::RotoJob, BridgeError> {
    let media = match layer.item()?.kind {
        LayerKind::Footage { item } => item,
        _ => return Err(BridgeError::NotFootage),
    };
    let (path, fingerprint) = crate::api::track::media_source(layer, media)?;
    lumit_render::roto::job_for(fx, path, &fingerprint, true).ok_or(BridgeError::NotFootage)
}

/// Press **Propagate** or **Cancel** on a Roto brush.
///
/// Reached through [`crate::api::track::fire_effect_action`], which is the one
/// doorway every Action press goes through — an Action carries no value, so a
/// press is an *event*: nothing is staged, nothing is committed, and no undo
/// entry appears.
pub(crate) fn press(
    layer: &LayerReference,
    fx: &lumit_core::model::EffectInstance,
    param: &str,
) -> Result<(), BridgeError> {
    match param {
        CANCEL => {
            lumit_render::roto::cancel(fx.id);
            Ok(())
        }
        PROPAGATE => match lumit_render::roto::request(job_of(layer, fx)?) {
            lumit_render::roto::Requested::Started => Ok(()),
            // Every refusal has a name and the status row reads it back;
            // what the *press* owes the caller is only that it did not
            // start, which is one error rather than seven.
            lumit_render::roto::Requested::Refused(e) => {
                lumit_render::roto::note_refusal(fx.id, e);
                Err(BridgeError::AnalysisBusy)
            }
        },
        _ => Err(BridgeError::InvalidParam),
    }
}

/// Solve the scribbled frame's own matte, now — the release-time feedback a
/// committed stroke asks for (K-723, docs/impl/roto.md §6 step 1).
///
/// The same job a Propagate press builds, stopped after `frame`: the walk runs
/// from the base toward it, lends back every cached frame whose strokes did not
/// change, and files what it solves in the ordinary sidecar — so a later
/// Propagate resumes from it rather than repeating it. Progress lands in the
/// same map the status row polls, which is where "Solving…" comes from while
/// the second or so passes.
///
/// `true` when the job started. `false` is a **quiet** refusal — another job
/// holding the one slot, offline media — because this is best-effort feedback
/// on the way out of a gesture that already succeeded: the stroke is filed and
/// visible either way, and Propagate remains the press that reports its
/// refusals.
#[frb(sync)]
pub fn roto_solve_frame(
    layer: LayerReference,
    effect: Uuid,
    frame: i64,
) -> Result<bool, BridgeError> {
    let item = layer.item()?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    if fx.effect.match_name != ROTO_BRUSH {
        return Err(BridgeError::InvalidEffect);
    }
    let mut job = job_of(&layer, fx)?;
    job.stop_after = Some(frame);
    Ok(matches!(
        lumit_render::roto::request(job),
        lumit_render::roto::Requested::Started
    ))
}

#[cfg(test)]
mod tests {
    use super::{boundary_of, MAX_BOUNDARY_POINTS};

    /// A filled square in the middle of a small plane: the edge is the ring of
    /// pixels where the answer changes, and nothing inside or outside it.
    #[test]
    fn the_boundary_is_the_ring_where_the_matte_changes() {
        let (w, h) = (16u32, 16u32);
        let mut gray = vec![0u8; (w * h) as usize];
        for y in 4..12 {
            for x in 4..12 {
                gray[y * w as usize + x] = 255;
            }
        }
        let out = boundary_of(w, h, &gray);
        assert!(!out.is_empty(), "a square in a field has an edge");
        assert_eq!(out.len() % 2, 0, "points cross as x, y pairs");
        for p in out.chunks_exact(2) {
            let (x, y) = (p[0], p[1]);
            // Every emitted point sits on the square's rim — one pixel outside
            // it on the low side, on it on the high side, never in its middle
            // and never out in the empty field.
            let near = (3.0..=11.0).contains(&x) && (3.0..=11.0).contains(&y);
            assert!(near, "({x}, {y}) is not on the square's rim");
            let inside = (4.0..=10.0).contains(&x) && (4.0..=10.0).contains(&y);
            assert!(!inside, "({x}, {y}) is in the square's middle");
        }
    }

    /// An empty matte, and a plane too small to have neighbours, are both the
    /// passthrough's honest answer rather than a fault.
    #[test]
    fn nothing_to_outline_is_no_points() {
        assert!(boundary_of(16, 16, &vec![0u8; 256]).is_empty());
        assert!(boundary_of(16, 16, &vec![255u8; 256]).is_empty());
        assert!(boundary_of(1, 1, &[255]).is_empty());
        assert!(boundary_of(16, 16, &[0, 1, 2]).is_empty(), "a short plane");
    }

    /// A matte noisy enough to put an edge under every pixel is thinned evenly
    /// rather than cut short: the cap holds, and the last point is still near
    /// the bottom of the picture.
    #[test]
    fn a_busy_matte_is_thinned_rather_than_truncated() {
        let (w, h) = (400u32, 400u32);
        // A checkerboard: every pixel disagrees with both its neighbours.
        let gray: Vec<u8> = (0..(w * h) as usize)
            .map(|i| {
                let (x, y) = (i % w as usize, i / w as usize);
                if (x + y) % 2 == 0 {
                    255
                } else {
                    0
                }
            })
            .collect();
        let out = boundary_of(w, h, &gray);
        assert!(out.len() / 2 <= MAX_BOUNDARY_POINTS, "the cap holds");
        assert!(out.len() / 2 > MAX_BOUNDARY_POINTS / 2, "and is nearly met");
        let last_y = out[out.len() - 1];
        assert!(
            last_y > f32::from(h as u16) * 0.9,
            "the outline reaches the bottom of the picture rather than stopping \
             where the cap ran out"
        );
    }
}

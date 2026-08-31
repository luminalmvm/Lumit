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
        if points.is_empty() || points.len() % 2 != 0 || !points.iter().all(|v| v.is_finite()) {
            return Err(BridgeError::InvalidParam);
        }
        if !radius.is_finite() || radius <= 0.0 {
            return Err(BridgeError::InvalidParam);
        }
        let block = self.roto_block_mut();
        block.base_frame.get_or_insert(frame);
        block.strokes.push(RotoStroke {
            id: Uuid::now_v7(),
            points: points.chunks_exact(2).map(|p| (p[0], p[1])).collect(),
            radius,
            kind: kind.read(),
            frame,
        });
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
// The buttons, down
// ---------------------------------------------------------------------------

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
        PROPAGATE => {
            let media = match layer.item()?.kind {
                LayerKind::Footage { item } => item,
                _ => return Err(BridgeError::NotFootage),
            };
            let (path, fingerprint) = crate::api::track::media_source(layer, media)?;
            let job = lumit_render::roto::job_for(fx, path, &fingerprint, true);
            let job = job.ok_or(BridgeError::NotFootage)?;
            match lumit_render::roto::request(job) {
                lumit_render::roto::Requested::Started => Ok(()),
                // Every refusal has a name and the status row reads it back;
                // what the *press* owes the caller is only that it did not
                // start, which is one error rather than seven.
                lumit_render::roto::Requested::Refused(e) => {
                    lumit_render::roto::note_refusal(fx.id, e);
                    Err(BridgeError::AnalysisBusy)
                }
            }
        }
        _ => Err(BridgeError::InvalidParam),
    }
}

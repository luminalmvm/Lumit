//! The planes tier's surface across the seam (docs/impl/addons.md §6.1,
//! §10): two buttons down, a span and a progress up.
//!
//! # In plain terms
//!
//! Reading a shot with a model happens elsewhere - on its own thread, in
//! `lumit-render`, over the media file rather than over the layer. This module
//! is the doorway. It turns a press of **Analyse** into a job and answers "how
//! far along is it, which provider is running, and how much of the shot does
//! the answer cover" whenever the card repaints.
//!
//! **A refusal comes back as a reason, never a sentence.** The words are
//! Dart's, from the arb, the way the camera track's and the Roto brush's are.
//! The one exception in the whole mechanism is the badge's detail slot, which
//! names the addon to install and is handled in [`crate::api::effect`].
//!
//! **A press that cannot work says so before it starts.** With no runtime or no
//! pack there is nothing to spawn a thread for, so the refusal is answered on
//! the caller's own thread and left where the next status read finds it - a
//! press is an event and has nothing else to poll against.

use flutter_rust_bridge::frb;
use lumit_core::model::LayerKind;
use uuid::Uuid;

use crate::api::{layer::LayerReference, BridgeError};

/// The planes tier's two Action parameters, by the ids the effects' schemas
/// declare. Spelled once, here, because a typo would be silent: the press would
/// simply do nothing.
pub(crate) const ANALYSE: &str = "analyse";
pub(crate) const CANCEL: &str = "cancel";

// ---------------------------------------------------------------------------
// What the status card reads
// ---------------------------------------------------------------------------

/// How far an analysis has got - the bridge form of
/// [`lumit_render::planes::Progress`], flattened so the panel reads fields
/// rather than unwrapping a shape.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePlaneStage {
    /// Nothing has ever been asked for this effect since Lumit started.
    Idle,
    /// Accepted, not started.
    Queued,
    /// Reading the clip.
    Solving,
    /// There is an answer in the store.
    Done,
    /// Stopped between frames. **The finished prefix was kept**, so the span
    /// below is real and a later Analyse carries on from it.
    Cancelled,
    /// Refused - see [`BridgePlaneStatus::failure`].
    Failed,
}

/// Why an analysis produced no planes.
///
/// A **reason, not a sentence**: the engine's own `PlaneFailure` carries
/// English, and English crossing here would ship untranslated inside a
/// translated window. Dart switches over this and picks the arb key, which is
/// the shape `BridgeRotoFailure` already uses.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePlaneFailure {
    /// No model runtime is installed.
    RuntimeMissing,
    /// Nothing installed does this effect's task.
    PackMissing,
    /// A pack that would not open, or a run the model turned down.
    ModelFailed,
    /// One model job at a time.
    Busy,
    /// The media could not be opened, or carries no video.
    Unreadable,
    /// Opened, but with no frames to read.
    NoFrames,
    /// Stopped between frames.
    Cancelled,
}

/// Every arm spelled out rather than a wildcard, so a reason added in the
/// engine is a compile error here rather than a blank line on screen.
fn failure_of(e: lumit_render::planes::PlaneFailure) -> BridgePlaneFailure {
    use lumit_render::planes::PlaneFailure as F;
    match e {
        F::RuntimeMissing => BridgePlaneFailure::RuntimeMissing,
        F::PackMissing => BridgePlaneFailure::PackMissing,
        F::ModelFailed => BridgePlaneFailure::ModelFailed,
        F::Busy => BridgePlaneFailure::Busy,
        F::Unreadable => BridgePlaneFailure::Unreadable,
        F::NoFrames => BridgePlaneFailure::NoFrames,
        F::Cancelled => BridgePlaneFailure::Cancelled,
    }
}

/// Everything a planes-tier effect's status row draws, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgePlaneStatus {
    pub stage: BridgePlaneStage,
    /// Frames read so far, and how many the clip has. Both zero outside
    /// [`BridgePlaneStage::Solving`].
    pub done: u32,
    pub total: u32,
    /// Set only at [`BridgePlaneStage::Failed`].
    pub failure: Option<BridgePlaneFailure>,
    /// Which provider read the frames, as the run recorded it: `DirectML`,
    /// `CoreML` or `CPU` (§8 - the fallback is allowed and reported, never
    /// silent). Empty before anything has been read.
    pub provider: String,
    /// The span the answer actually covers, in **source** frames, or `None`
    /// before anything has been read. Outside it the effect is a passthrough
    /// and the card says so.
    pub first_frame: Option<i64>,
    pub last_frame: Option<i64>,
    /// How many frames the clip has, against which the span is whole or
    /// partial. Zero when nothing has been read.
    pub clip_frames: u32,
}

/// The planes-tier `effect` on `layer`, as its status row draws it.
///
/// **Polled while it is moving and never otherwise** (the camera track's §5c
/// second deviation): a press moves no document revision, so there is nothing
/// to refresh against, and the engine keeps progress as a value in a map
/// precisely so nobody has to hold a subscription.
#[frb(sync)]
pub fn plane_status(layer: LayerReference, effect: Uuid) -> Result<BridgePlaneStatus, BridgeError> {
    let item = layer.item()?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    if lumit_core::planes::task_of(fx).is_none() {
        return Err(BridgeError::InvalidEffect);
    }
    let run = lumit_render::planes::analysed(effect);
    let (stage, done, total, failure) = match lumit_render::planes::progress(effect) {
        None => (BridgePlaneStage::Idle, 0, 0, None),
        Some(lumit_render::planes::Progress::Queued) => (BridgePlaneStage::Queued, 0, 0, None),
        Some(lumit_render::planes::Progress::Solving { done, total }) => {
            (BridgePlaneStage::Solving, done as u32, total as u32, None)
        }
        Some(lumit_render::planes::Progress::Done) => (BridgePlaneStage::Done, 0, 0, None),
        Some(lumit_render::planes::Progress::Cancelled) => {
            (BridgePlaneStage::Cancelled, 0, 0, None)
        }
        Some(lumit_render::planes::Progress::Failed(e)) => {
            (BridgePlaneStage::Failed, 0, 0, Some(failure_of(e)))
        }
    };
    Ok(BridgePlaneStatus {
        stage,
        done,
        total,
        failure,
        provider: run
            .as_ref()
            .map_or_else(String::new, |r| r.provider().to_owned()),
        first_frame: run.as_ref().map(|r| r.first_frame),
        last_frame: run.as_ref().map(|r| r.last_frame),
        clip_frames: run.as_ref().map_or(0, |r| r.clip_frames as u32),
    })
}

// ---------------------------------------------------------------------------
// The buttons, down
// ---------------------------------------------------------------------------

/// The analysis job one planes-tier effect on one footage layer describes.
fn job_of(
    layer: &LayerReference,
    fx: &lumit_core::model::EffectInstance,
) -> Result<lumit_render::planes::PlaneJob, BridgeError> {
    let media = match layer.item()?.kind {
        LayerKind::Footage { item } => item,
        _ => return Err(BridgeError::NotFootage),
    };
    let (path, fingerprint) = crate::api::track::media_source(layer, media)?;
    lumit_render::planes::job_for(fx, path, &fingerprint, true).ok_or(BridgeError::NotFootage)
}

/// Press **Analyse** or **Cancel** on a planes-tier effect.
///
/// Reached through [`crate::api::track::fire_effect_action`], which is the one
/// doorway every Action press goes through - an Action carries no value, so a
/// press is an *event*: nothing is staged, nothing is committed, and no undo
/// entry appears.
pub(crate) fn press(
    layer: &LayerReference,
    fx: &lumit_core::model::EffectInstance,
    param: &str,
) -> Result<(), BridgeError> {
    match param {
        CANCEL => {
            lumit_render::planes::cancel(fx.id);
            Ok(())
        }
        ANALYSE => {
            // A machine with no runtime and no pack has nothing to spawn a
            // thread for, so the refusal is answered here and left where the
            // status read will find it.
            if let Some(why) = lumit_render::planes::refusal_for(fx) {
                lumit_render::planes::note_refusal(fx.id, why);
                return Err(BridgeError::AddonMissing);
            }
            match lumit_render::planes::request(job_of(layer, fx)?) {
                lumit_render::planes::Requested::Started => Ok(()),
                // Every refusal has a name and the status row reads it back;
                // what the *press* owes the caller is only that it did not
                // start, which is one error rather than seven.
                lumit_render::planes::Requested::Refused(e) => {
                    lumit_render::planes::note_refusal(fx.id, e);
                    Err(BridgeError::AnalysisBusy)
                }
            }
        }
        _ => Err(BridgeError::InvalidParam),
    }
}

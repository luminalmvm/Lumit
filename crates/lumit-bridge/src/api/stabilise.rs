//! Stabilise's surface across the seam: its status line, the points the
//! Viewer draws, dropping and restoring them, and the stabilised precomp.
//!
//! # In plain terms
//!
//! The analysis happens elsewhere, on its own thread in `lumit-render`, and
//! the frames are worked out in `lumit-core`. This is the doorway: it answers
//! "how far along is it" whenever the panel repaints, hands the Viewer the
//! points already placed where the steadied picture shows them, and carries
//! the gestures a stabilised shot offers. A press of Analyse, Cancel or Use
//! this frame arrives through the trackers' one doorway,
//! [`crate::api::track::fire_effect_action`], and is answered by [`press`].
//!
//! Failures come back as reasons, never sentences: the words are Dart's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Property};
use lumit_core::fx::effects::stabilise::{self as fx, DROPPED_KEY, MATCH_NAME, MODE_REVERSE};
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectParam, EffectValue, Layer, LayerKind,
};
use lumit_render::track::{AnalysisError, Progress};
use uuid::Uuid;

use crate::api::layer::LayerReference;
use crate::api::track::{BridgeTrackFailure, BridgeTrackStage};
use crate::api::BridgeError;

/// Everything Stabilise's status line draws, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeStabiliseStatus {
    /// For a Reverse whose stabilised layer's analysis is not in hand, that
    /// layer's own stage, with Tracking and Solving read as Queued.
    pub stage: BridgeTrackStage,
    /// Frames followed so far and how many there are, while following.
    pub done: u32,
    pub total: u32,
    /// Set only when the stage is failed.
    pub failure: Option<BridgeTrackFailure>,
    /// The filed analysis's generation, nought with none: it changes whenever
    /// another analysis lands, which is when picked points stop meaning what
    /// they meant.
    pub generation: u64,
    /// Frames analysed and how many were asked for. Fewer analysed than asked
    /// for is a run that could not be followed to the end.
    pub frames: u32,
    pub clip_frames: u32,
    /// The scale the edges need on the frames the layer shows, 1 for none.
    pub scale: f64,
    /// Whether that is more than the Maximum scale allows.
    pub short: bool,
    /// Whether `scale` and `short` are known. With an analysis in hand they
    /// are not while the plan for the rows now is still being made: read
    /// again shortly.
    pub scale_ready: bool,
    /// A row that changes what an analysis finds has moved since this one was
    /// made. It is still drawn with until the next one lands.
    pub stale: bool,
    /// Points followed, and how many of them were dropped.
    pub points: u32,
    pub dropped: u32,
    /// Reverse: the layer whose shake is put back, or `None` when the row names
    /// nothing stabilised.
    pub reversing: Option<String>,
    /// Whether the layer shows something an analysis can read: footage or a
    /// Precomp layer.
    pub analysable: bool,
    /// Whether **Make stabilised precomp** and **Convert to keyframes** can be
    /// done now: a switched-on Stabilise taking the shake out, with its
    /// analysis in hand. Neither on a guide layer, which is already the copy
    /// inside a stabilised precomp.
    pub can_precomp: bool,
    pub can_convert: bool,
}

impl BridgeStabiliseStatus {
    fn idle() -> Self {
        BridgeStabiliseStatus {
            stage: BridgeTrackStage::Idle,
            done: 0,
            total: 0,
            failure: None,
            generation: 0,
            frames: 0,
            clip_frames: 0,
            scale: 1.0,
            short: false,
            scale_ready: true,
            stale: false,
            points: 0,
            dropped: 0,
            reversing: None,
            analysable: true,
            can_precomp: false,
            can_convert: false,
        }
    }
}

/// The points at one frame, and the analysis they belong to.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeStabilisePoints {
    /// False while the plan the points are placed with is being made, with
    /// no points: read again shortly, and keep what is drawn meanwhile.
    pub ready: bool,
    /// The analysis's generation, nought with none. A pick made under another
    /// one names other points.
    pub generation: u64,
    pub points: Vec<BridgeStabilisePoint>,
}

/// One followed point, ready to draw.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeStabilisePoint {
    /// The tracker's id: what a pick names and a drop takes.
    pub track: u32,
    /// Composition pixels, where the steadied picture shows the point.
    pub x: f64,
    pub y: f64,
    /// Whether it moved with the picture. One that did not was set aside.
    pub kept: bool,
}

fn failure_of(why: &AnalysisError) -> BridgeTrackFailure {
    match why {
        AnalysisError::Unreadable => BridgeTrackFailure::Unreadable,
        AnalysisError::NoFrames => BridgeTrackFailure::NoFrames,
        AnalysisError::TooBig => BridgeTrackFailure::TooBig,
        AnalysisError::Tracking(_) => BridgeTrackFailure::Tracking,
        _ => BridgeTrackFailure::NoFeatures,
    }
}

/// The instance `effect` on `layer`, if it is a Stabilise.
fn instance(layer: &Layer, effect: Uuid) -> Option<&EffectInstance> {
    layer
        .effects
        .iter()
        .find(|e| e.id == effect && e.effect.match_name == MATCH_NAME)
}

/// The analysis job `fx` on `item` of `comp` would be asked for now. `fresh`
/// for a press of Analyse, which never answers from the sidecar.
fn job_of(
    layer: &LayerReference,
    doc: &Arc<Document>,
    comp: &Composition,
    item: &Layer,
    fx: &EffectInstance,
    fresh: bool,
) -> Result<lumit_render::stabilise::Job, BridgeError> {
    let span = lumit_core::planes::source_span(fx, Some(item), comp);
    match item.kind {
        LayerKind::Footage { item: media } => {
            let (path, fingerprint) = crate::api::track::media_source(layer, media)?;
            lumit_render::stabilise::job_for(item, fx, path, &fingerprint, fresh, span)
        }
        LayerKind::Precomp { .. } => {
            lumit_render::stabilise::job_for_precomp(doc, item, fx, fresh, span)
        }
        _ => None,
    }
    .ok_or(BridgeError::NotFootage)
}

/// What the rows of one Stabilise asked for at one document revision.
#[derive(Clone, Copy, PartialEq)]
enum Asked {
    Key(lumit_render::stabilise::Key),
    /// Footage whose file cannot be found.
    Offline,
    /// Nothing an analysis can read.
    Nothing,
}

/// What one Stabilise asked for, with the project and revision it was worked
/// out at.
type AskedAt = (Uuid, u64, Asked);

/// The key each Stabilise's rows asked for, by instance. One entry per
/// Stabilise polled, replaced when it is next polled at another revision.
fn asked() -> &'static Mutex<HashMap<Uuid, AskedAt>> {
    static ASKED: OnceLock<Mutex<HashMap<Uuid, AskedAt>>> = OnceLock::new();
    ASKED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What `fx`'s rows ask for at `revision`, worked out once per revision. The
/// first read at a revision builds the job, and asks for it when what is
/// filed is not it: a changed row starts its analysis once.
fn asked_for(
    layer: &LayerReference,
    doc: &Arc<Document>,
    revision: u64,
    comp: &Composition,
    item: &Layer,
    fx: &EffectInstance,
    filed: Option<&lumit_core::stabilise::analysis::Filed>,
) -> Asked {
    let held = asked()
        .lock()
        .ok()
        .and_then(|m| m.get(&fx.id).copied())
        .filter(|(p, r, _)| *p == layer.project_id && *r == revision);
    if let Some((_, _, a)) = held {
        return a;
    }
    let a = match job_of(layer, doc, comp, item, fx, false) {
        Ok(job) => {
            let key = job.key;
            if filed.is_none_or(|f| f.key != key) {
                lumit_render::stabilise::ask(job);
            }
            Asked::Key(key)
        }
        Err(BridgeError::MediaPathUnresolved) => Asked::Offline,
        Err(_) => Asked::Nothing,
    };
    if let Ok(mut m) = asked().lock() {
        m.insert(fx.id, (layer.project_id, revision, a));
    }
    a
}

/// The stage and failure an analysis reading says.
fn stage_of(p: Option<Progress>) -> (BridgeTrackStage, Option<BridgeTrackFailure>) {
    match p {
        Some(Progress::Queued) => (BridgeTrackStage::Queued, None),
        Some(Progress::Tracking { .. }) => (BridgeTrackStage::Tracking, None),
        Some(Progress::Solving) => (BridgeTrackStage::Solving, None),
        Some(Progress::Cancelled) => (BridgeTrackStage::Cancelled, None),
        Some(Progress::Failed(why)) => (BridgeTrackStage::Failed, Some(failure_of(&why))),
        Some(Progress::Done) | None => (BridgeTrackStage::Idle, None),
    }
}

/// How `effect` on `layer` is getting on.
///
/// Polled, never subscribed to, so it does no work twice: the key the rows
/// ask for is worked out once per document revision, and the scale is read
/// off a plan already in hand, never made here. With none, the plan is made
/// on a worker and `scale_ready` is false until it is. The first read at a
/// revision is also where a Stabilise whose rows were changed in a way that
/// changes what is followed gets its new analysis asked for: it is asked for
/// once, and the frames keep the old one until it lands.
///
/// For a Reverse, `stage` and `failure` are the stabilised layer's own while
/// its analysis is not in hand: queued while it runs, and failed, cancelled
/// or idle when it will not land by itself.
#[frb(sync)]
#[must_use]
pub fn stabilise_status(layer: LayerReference, effect: Uuid) -> BridgeStabiliseStatus {
    let mut status = BridgeStabiliseStatus::idle();
    let Ok(project) = layer.project() else {
        return status;
    };
    let Ok((doc, revision)) = project.read().map(|p| p.store.snapshot_and_revision()) else {
        return status;
    };
    let Some(comp) = doc.comp(layer.comp_id) else {
        return status;
    };
    let Some(item) = comp.layers.iter().find(|l| l.id == layer.layer_id) else {
        return status;
    };
    let Some(fx) = instance(item, effect) else {
        return status;
    };
    if matches!(fx.param("mode"), Some(EffectValue::Choice(MODE_REVERSE))) {
        if let Some((target, tfx, ready)) = fx::reversing(&doc, comp, item, fx) {
            status.reversing = Some(target.name.clone());
            (status.stage, status.failure) = if ready {
                (BridgeTrackStage::Done, None)
            } else {
                match stage_of(lumit_render::stabilise::progress(tfx.id)) {
                    (BridgeTrackStage::Tracking | BridgeTrackStage::Solving, _) => {
                        (BridgeTrackStage::Queued, None)
                    }
                    other => other,
                }
            };
        }
        return status;
    }
    status.analysable = fx::source_of(item).is_some();
    let filed = lumit_core::stabilise::analysis::filed(effect)
        .filter(|f| Some(f.source) == fx::source_of(item));
    if let Some(f) = &filed {
        status.stage = BridgeTrackStage::Done;
        status.generation = f.generation;
        status.frames = f.analysis.frames;
        status.clip_frames = f.analysis.asked;
        status.points = u32::try_from(f.analysis.runs.len()).unwrap_or(u32::MAX);
        status.dropped = u32::try_from(fx::dropped(fx, &f.points).len()).unwrap_or(u32::MAX);
        match fx::needed_scale(&doc, comp, item, fx) {
            fx::Held::Ready((scale, short)) => {
                status.scale = scale;
                status.short = short;
            }
            fx::Held::Making => status.scale_ready = false,
            fx::Held::Nothing => {}
        }
        status.can_convert = fx.enabled && !item.switches.guide;
        status.can_precomp = fx.enabled && !item.switches.guide;
    }
    // Whether what is filed is still what the rows ask for. Asked before the
    // progress is read, so the read that asks already says it is waiting.
    match asked_for(&layer, &doc, revision, comp, item, fx, filed.as_ref()) {
        Asked::Key(key) => status.stale = filed.as_ref().is_some_and(|f| f.key != key),
        // Footage whose file cannot be found has nothing to analyse, and says
        // so rather than waiting for an analysis that never starts.
        Asked::Offline if filed.is_none() => {
            status.stage = BridgeTrackStage::Failed;
            status.failure = Some(BridgeTrackFailure::Unreadable);
        }
        Asked::Offline | Asked::Nothing => {}
    }
    match lumit_render::stabilise::progress(effect) {
        Some(Progress::Queued) => status.stage = BridgeTrackStage::Queued,
        Some(Progress::Tracking { done, total }) => {
            status.stage = BridgeTrackStage::Tracking;
            status.done = u32::try_from(done).unwrap_or(u32::MAX);
            status.total = u32::try_from(total).unwrap_or(u32::MAX);
            // A first analysis fills the span bar as it goes. A second keeps
            // the span it is drawn with until it lands.
            if filed.is_none() {
                status.frames = status.done;
                status.clip_frames = status.total;
            }
        }
        Some(Progress::Solving) => status.stage = BridgeTrackStage::Solving,
        Some(Progress::Cancelled) if filed.is_none() => status.stage = BridgeTrackStage::Cancelled,
        Some(Progress::Failed(why)) if filed.is_none() => {
            status.stage = BridgeTrackStage::Failed;
            status.failure = Some(failure_of(&why));
        }
        _ => {}
    }
    status
}

/// The points `effect` followed on `layer`, showing at composition frame
/// `frame`, placed where the steadied picture draws them. Asked for once per
/// frame, never per rebuild, and never makes a plan: with none in hand it is
/// made on a worker and the answer is not ready.
#[frb(sync)]
#[must_use]
pub fn stabilise_points(layer: LayerReference, effect: Uuid, frame: i64) -> BridgeStabilisePoints {
    let found = || {
        let doc = layer.document().ok()?;
        let comp = doc.comp(layer.comp_id)?;
        let item = comp.layers.iter().find(|l| l.id == layer.layer_id)?;
        let fx = instance(item, effect)?;
        let t = comp.frame_rate.time_of_frame(frame).ok()?.0.to_f64();
        Some(fx::points_at(&doc, comp, item, fx, t))
    };
    let (ready, generation, points) = match found() {
        Some(fx::Held::Ready((generation, points))) => (true, generation, points),
        Some(fx::Held::Making) => (false, 0, Vec::new()),
        Some(fx::Held::Nothing) | None => (true, 0, Vec::new()),
    };
    BridgeStabilisePoints {
        ready,
        generation,
        points: points
            .into_iter()
            .map(|p| BridgeStabilisePoint {
                track: p.track,
                x: p.at[0],
                y: p.at[1],
                kept: p.kept,
            })
            .collect(),
    }
}

/// Set row `id` of `inst` to `value`, adding the row if an older project left
/// it out.
fn set(inst: &mut EffectInstance, id: &str, value: EffectValue) {
    match inst.params.iter_mut().find(|p| p.id == id) {
        Some(p) => p.value = value,
        None => inst.params.push(EffectParam {
            id: id.to_string(),
            value,
            extra: serde_json::Map::new(),
        }),
    }
}

/// Write `inst` back into `layer`'s stack, as one undo step.
fn commit_instance(
    layer: &LayerReference,
    item: &Layer,
    inst: EffectInstance,
) -> Result<(), BridgeError> {
    let effects = item
        .effects
        .iter()
        .map(|e| {
            if e.id == inst.id {
                inst.clone()
            } else {
                e.clone()
            }
        })
        .collect();
    layer.commit(lumit_core::ops::Op::SetLayerEffects {
        comp: layer.comp_id,
        layer: layer.layer_id,
        effects,
    })
}

/// Drop `tracks` from `effect`'s analysis, and every point that moves with
/// them, as one undo step. The list is kept against that analysis, so a list
/// left from another one is replaced, never added to. Answers how many are
/// dropped now in all: nought, and nothing written, with no analysis in hand.
#[frb(sync)]
pub fn stabilise_drop_points(
    layer: LayerReference,
    effect: Uuid,
    tracks: Vec<u32>,
) -> Result<u32, BridgeError> {
    let doc = layer.document()?;
    let comp = doc.comp(layer.comp_id).ok_or(BridgeError::InvalidItem)?;
    let item = comp
        .layers
        .iter()
        .find(|l| l.id == layer.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;
    let fx = instance(item, effect).ok_or(BridgeError::InvalidEffect)?;
    let Some((ids, points)) = fx::dropping(&doc, comp, item, fx, &tracks) else {
        return Ok(0);
    };
    let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    commit_instance(&layer, item, fx::with_dropped(fx, &ids, &points))?;
    Ok(count)
}

/// Bring every dropped point back, as one undo step.
#[frb(sync)]
pub fn stabilise_restore_points(layer: LayerReference, effect: Uuid) -> Result<(), BridgeError> {
    let item = layer.item()?;
    let fx = instance(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    if !fx.extra.contains_key(DROPPED_KEY) {
        return Ok(());
    }
    commit_instance(&layer, &item, fx::with_dropped(fx, &[], &[0; 32]))
}

/// **Create stabilised precomp**: a comp named `name` holding a copy of the
/// stabilised layer as a guide, steadied with its edges shown, and in its
/// place above the original a Precomp layer that puts the shake back. Work
/// done inside the precomp sits on a still picture and moves with the shot
/// outside it; the guide is left out of every export, so the shot itself goes
/// out as filmed. One undo step.
///
/// The copy keeps the Stabilise's own identity, which the original gives up,
/// so the analysis already made is the copy's from the start.
#[frb(sync)]
pub fn make_stabilised_precomp(
    layer: LayerReference,
    effect: Uuid,
    name: String,
) -> Result<LayerReference, BridgeError> {
    let doc = layer.document()?;
    let comp = layer.composition()?;
    let item = layer.item()?;
    instance(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    let index = comp
        .layers
        .iter()
        .position(|l| l.id == item.id)
        .ok_or(BridgeError::InvalidLayer)?;

    // The copy: a guide, every other effect a fresh identity, the Stabilise
    // its own, showing its edges so nothing of the plate is cropped away.
    let mut copy = item.clone();
    copy.id = Uuid::now_v7();
    copy.switches.guide = true;
    for e in &mut copy.effects {
        if e.id == effect {
            set(e, "edges", EffectValue::Choice(0));
            set(e, "mode", EffectValue::Choice(0));
        } else {
            e.id = Uuid::now_v7();
        }
    }
    let mut with_copy = comp.clone();
    with_copy.layers.insert(index, copy.clone());
    let (mut ops, precomp) = lumit_core::edits::precompose_ops(
        &doc,
        &with_copy,
        lumit_core::edits::Precompose {
            layers: &[copy.id],
            name,
            leave_attributes: false,
            adjust_duration: false,
            group: None,
            source_length: &|_| None,
        },
    )
    .map_err(BridgeError::OpError)?;

    // The original plays as filmed, so it gives its Stabilise up.
    let mut original = item.effects.clone();
    original.retain(|e| e.id != effect);
    // The Precomp layer puts the shake back: a Reverse naming itself, which
    // reads the stabilised layer inside it.
    let mut reverse =
        lumit_core::fx::builtins::instantiate(MATCH_NAME).ok_or(BridgeError::InvalidEffect)?;
    set(&mut reverse, "mode", EffectValue::Choice(MODE_REVERSE));
    set(
        &mut reverse,
        "stabilised",
        EffectValue::Layer(Some(precomp)),
    );
    let mut all = vec![lumit_core::ops::Op::AddLayer {
        comp: comp.id,
        index,
        layer: Box::new(copy),
    }];
    all.append(&mut ops);
    all.push(lumit_core::ops::Op::SetLayerEffects {
        comp: comp.id,
        layer: precomp,
        effects: vec![reverse],
    });
    all.push(lumit_core::ops::Op::SetLayerEffects {
        comp: comp.id,
        layer: item.id,
        effects: original,
    });
    layer.commit(lumit_core::ops::Op::Batch { ops: all })?;
    Ok(LayerReference::new(
        layer.project_id,
        layer.comp_id,
        precomp,
    ))
}

/// **Convert to keyframes**: `effect`'s stabilisation written onto the layer's
/// own Anchor point, Scale and Rotation, one key a frame, and the effect
/// switched off. The motion is then ordinary keyframes, to edit, copy or
/// parent to; switching the effect back on and taking the keys away is where
/// it started. One undo step. Answers how many keys each property has.
///
/// Refused for a Reverse, a switched-off effect, and one with no analysis in
/// hand.
#[frb(sync)]
pub fn stabilise_bake_keyframes(layer: LayerReference, effect: Uuid) -> Result<u32, BridgeError> {
    let doc = layer.document()?;
    let item = layer.item()?;
    instance(&item, effect).ok_or(BridgeError::InvalidEffect)?;
    fx::source_of(&item).ok_or(BridgeError::NotFootage)?;
    let (op, keys) = lumit_core::stabilise::keys::bake(&doc, layer.comp_id, layer.layer_id, effect)
        .ok_or(BridgeError::InvalidEffect)?;
    layer.commit(op)?;
    Ok(keys)
}

/// A press of one of Stabilise's buttons, arriving through the trackers' one
/// doorway. `frame` is the playhead, which **Use this frame** reads.
pub(crate) fn press(
    layer: &LayerReference,
    item: &Layer,
    fx: &EffectInstance,
    param: &str,
    frame: Option<u64>,
) -> Result<(), BridgeError> {
    match param {
        "analyse" => {
            let doc = layer.document()?;
            let comp = doc.comp(layer.comp_id).ok_or(BridgeError::InvalidItem)?;
            lumit_render::stabilise::request(job_of(layer, &doc, comp, item, fx, true)?);
            Ok(())
        }
        "cancel" => {
            lumit_render::stabilise::cancel(fx.id);
            Ok(())
        }
        "use_frame" => {
            let comp = layer.composition()?;
            let frame = i64::try_from(frame.unwrap_or(0)).map_err(|_| BridgeError::InvalidTime)?;
            let t = comp
                .frame_rate
                .time_of_frame(frame)
                .map_err(|_| BridgeError::InvalidTime)?
                .0
                .to_f64();
            let lt = lumit_core::time::layer_time(t, item.start_offset.0).max(0.0);
            let mut inst = fx.clone();
            set(
                &mut inst,
                "reference",
                EffectValue::Float(Property {
                    animation: Animation::Static(lt),
                    extra: serde_json::Map::new(),
                }),
            );
            commit_instance(layer, item, inst)
        }
        _ => Err(BridgeError::InvalidParam),
    }
}

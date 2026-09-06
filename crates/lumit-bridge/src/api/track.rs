//! The Camera track effect's whole surface across the seam: pressing its
//! buttons, reading how far the analysis has got, drawing its point cloud,
//! and everything a solve-linked Camera layer needs.
//!
//! # In plain terms
//!
//! The tracking itself happens elsewhere — on its own thread, in
//! `lumit-render`, over the media file rather than over the layer. This module
//! is the doorway: it turns a button press in the panel into a job, answers
//! "how far along is it" whenever the panel repaints, hands the interface the
//! solved points already worked out onto the picture for one frame, and carries
//! the three gestures a solved shot offers — make a camera that follows it,
//! bake that camera into keyframes, and drop a Null or a Solid where the
//! selected points are.
//!
//! **Nothing here does arithmetic the interface could not check.** The points
//! come back in composition pixels, depth already normalised, because a depth
//! cue is a decision about the whole cloud and the interface draws what it is
//! given (docs/17 "the engine owns the decisions"). The failure of an analysis
//! comes back as a *reason*, never as a sentence: the words are Dart's, from the
//! arb, the way an import report's reasons are.

use std::path::PathBuf;

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Property};
use lumit_core::fx::{PressFrame, Value};
use lumit_core::model::{
    Composition, EffectNamespace, EffectValue, Fingerprint, LayerKind, ProjectItem,
};
use lumit_core::time::Rational;
use lumit_core::track::LinkState;
use uuid::Uuid;

use crate::api::layer::InstanceHome;
use crate::api::{layer::LayerReference, state::PROJECTS, BridgeError};

/// The Camera track effect's two Action parameters, by the ids its schema
/// declares (`crates/lumit-core/src/fx/effects/camera_track.rs`). Spelled once,
/// here, because a typo would be silent: the press would simply do nothing.
const ANALYSE: &str = "analyse";
const CANCEL: &str = "cancel";
/// The Planar track's third Action: write the Corner pin.
const PIN: &str = "pin";
/// The Planar track's fourth: write the movement onto the target layer's own
/// transform instead.
const TRANSFORM_KEYS: &str = "transform_keys";

// ---------------------------------------------------------------------------
// What the status row reads
// ---------------------------------------------------------------------------

/// How far the analysis of a tracked layer has got — the bridge form of
/// [`lumit_render::track::Progress`], flattened so the panel reads fields
/// rather than unwrapping a shape.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTrackStage {
    /// Nothing has ever been asked for this media in this session.
    Idle,
    /// Accepted, not started.
    Queued,
    /// Decoding and following features.
    Tracking,
    /// The frames are in; the geometry and the solve are running.
    Solving,
    /// There is a solve in the store.
    Done,
    /// Stopped. Nothing was written.
    Cancelled,
    /// Refused — see [`BridgeTrackStatus::failure`].
    Failed,
}

/// Why an analysis produced no camera path.
///
/// A **reason, not a sentence**: the engine's own `AnalysisError`
/// carries English, and English crossing here would ship untranslated inside a
/// translated window. Dart switches over this and picks the arb key, which is
/// the shape `lumit_import::Reason` already uses for the import report.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTrackFailure {
    /// The media could not be opened, or carries no video.
    Unreadable,
    /// Opened, but with no frames to track.
    NoFrames,
    /// The frames themselves could not be followed (a size change mid-clip).
    Tracking,
    /// Too little in the picture held still enough to follow.
    NoFeatures,
    /// The camera only turned: there is no position to solve and no depth to
    /// recover (docs/impl/tracking.md §4's sixth deviation).
    RotationOnly,
    /// The shot carries a camera move the solver could not stand behind.
    NoSolve,
    /// The quad's contents are not one flat surface, or move against
    /// themselves.
    NotPlanar,
}

/// Everything the Camera track's status row draws, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeTrackStatus {
    pub stage: BridgeTrackStage,
    /// Frames tracked so far, and how many there are. Both zero outside
    /// [`BridgeTrackStage::Tracking`].
    pub done: u32,
    pub total: u32,
    /// Set only at [`BridgeTrackStage::Failed`].
    pub failure: Option<BridgeTrackFailure>,
    /// The solve's mean reprojection error in source pixels, and how many
    /// points and frames it holds. Zero until there is a solve to describe.
    pub mean_error: f64,
    pub points: u32,
    /// How many frames of the clip carry a solved camera. The span always
    /// starts at the clip's first frame — the analysis tracks the source from
    /// its beginning and can only stop early, never start late — so this and
    /// `clip_frames` are the whole of the bar the panel draws.
    pub frames: u32,
    /// How many frames the clip has. `frames < clip_frames` is a **partial**
    /// track: the shot could not be followed all the way through, and what was
    /// solved is the part before it stopped carrying
    /// ([`lumit_render::track::Solved::is_partial`]).
    pub clip_frames: u32,
}

impl BridgeTrackStatus {
    fn idle() -> Self {
        BridgeTrackStatus {
            stage: BridgeTrackStage::Idle,
            done: 0,
            total: 0,
            failure: None,
            mean_error: 0.0,
            points: 0,
            frames: 0,
            clip_frames: 0,
        }
    }
}

/// One solved point, ready to draw: where it lands on the picture at the frame
/// asked for, and how near the camera it was.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeTrackPoint {
    /// The track this point came from — what a selection names, and what
    /// [`add_layer_at_points`] takes back.
    pub track: u32,
    /// **Composition pixels**, origin top-left, the coordinates every other
    /// overlay in the Viewer works in.
    pub x: f64,
    pub y: f64,
    /// Nearness over the cloud on this frame, 0..1, 1 being the nearest —
    /// already normalised, because a depth cue is arithmetic over the whole
    /// cloud and the interface draws what it is given.
    pub depth: f64,
}

/// How a Camera layer's placement is being arrived at — the bridge form of
/// [`lumit_core::track::LinkState`], which is the badge.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLinkState {
    Unlinked,
    Derived,
    Held,
    Unresolved,
}

/// A Camera layer's solve link, as the badge and the Convert command read it.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeCameraLink {
    pub state: BridgeLinkState,
    /// The tracked layer the link names, or `None` for an ordinary camera.
    pub tracked: Option<Uuid>,
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// How `layer`'s analysis is getting on.
///
/// Polled, never subscribed to: the reading is a value in a map that whoever
/// repaints samples, exactly as the cache bar is sampled
/// (docs/impl/tracking.md §5b). A layer that is not tracked footage reads
/// [`BridgeTrackStage::Idle`], which is what it is.
#[frb(sync)]
#[must_use]
pub fn track_status(layer: LayerReference) -> BridgeTrackStatus {
    let Some(media) = tracked_media(&layer) else {
        return BridgeTrackStatus::idle();
    };
    let mut status = BridgeTrackStatus::idle();
    match lumit_render::track::progress(media) {
        Some(lumit_render::track::Progress::Queued) => status.stage = BridgeTrackStage::Queued,
        Some(lumit_render::track::Progress::Tracking { done, total }) => {
            status.stage = BridgeTrackStage::Tracking;
            status.done = u32::try_from(done).unwrap_or(u32::MAX);
            status.total = u32::try_from(total).unwrap_or(u32::MAX);
        }
        Some(lumit_render::track::Progress::Solving) => status.stage = BridgeTrackStage::Solving,
        Some(lumit_render::track::Progress::Done) => status.stage = BridgeTrackStage::Done,
        Some(lumit_render::track::Progress::Cancelled) => {
            status.stage = BridgeTrackStage::Cancelled
        }
        Some(lumit_render::track::Progress::Failed(why)) => {
            status.stage = BridgeTrackStage::Failed;
            status.failure = Some(failure_of(&why));
        }
        // Nothing asked for in this session — but a warm pass may have filed a
        // solve from the sidecar, and a solve in hand is `Done` whoever put it
        // there.
        None => {}
    }
    if let Some(solved) = lumit_render::track::solved(media) {
        if status.stage == BridgeTrackStage::Idle {
            status.stage = BridgeTrackStage::Done;
        }
        status.mean_error = solved.solve.mean_reprojection_px;
        status.points = u32::try_from(solved.solve.points.len()).unwrap_or(u32::MAX);
        status.frames = u32::try_from(solved.solve.poses.len()).unwrap_or(u32::MAX);
        status.clip_frames = u32::try_from(solved.clip_frames).unwrap_or(u32::MAX);
    }
    status
}

fn failure_of(why: &lumit_render::track::AnalysisError) -> BridgeTrackFailure {
    use lumit_render::track::AnalysisError;
    use lumit_track::SolveError;
    match why {
        AnalysisError::Unreadable => BridgeTrackFailure::Unreadable,
        // A cancelled run is reported as `Cancelled`, never as a failure, so
        // this arm is unreachable through `progress`. It is spelled out rather
        // than caught by a wildcard so that adding a variant to the engine's
        // enum is a compile error here instead of a silent "could not solve".
        AnalysisError::Cancelled => BridgeTrackFailure::NoSolve,
        AnalysisError::NoFrames => BridgeTrackFailure::NoFrames,
        AnalysisError::Tracking(_) => BridgeTrackFailure::Tracking,
        AnalysisError::Solve(SolveError::NoTracks | SolveError::NoKeyframes) => {
            BridgeTrackFailure::NoFeatures
        }
        AnalysisError::Solve(SolveError::RotationOnly) => BridgeTrackFailure::RotationOnly,
        AnalysisError::Solve(_) => BridgeTrackFailure::NoSolve,
        // A planar refusal is about the quad, not the shot: too little inside
        // it to follow reads as the same "nothing to work with" the camera
        // half calls `NoFeatures`, and a quad that is not one flat surface has
        // its own reason.
        AnalysisError::Planar(lumit_track::PlanarError::TooFewFeatures) => {
            BridgeTrackFailure::NoFeatures
        }
        AnalysisError::Planar(lumit_track::PlanarError::NotPlanar) => BridgeTrackFailure::NotPlanar,
        AnalysisError::Planar(lumit_track::PlanarError::Cancelled) => BridgeTrackFailure::NoSolve,
    }
}

/// `layer`'s solved point cloud as it lands on composition frame `frame`.
///
/// **Once per frame change, never per rebuild** — the rule the Levels histogram
/// follows and the bridge-call budget pins. The frame the cloud is drawn
/// from is found by the one walk the camera link uses
/// ([`lumit_core::track::tracked_solved_frame`]), so the dots and the camera
/// they were solved with can never disagree about which moment this is.
///
/// **The reference frame** is §5b's second deviation, unchanged: the solve
/// describes a media item, not a composition, so its pixels are read at the
/// footage's own raster centred on the comp. Exact for the ordinary case — a
/// comp made from the shot — and off by the size ratio otherwise.
#[frb(sync)]
#[must_use]
pub fn tracked_points(layer: LayerReference, frame: i64) -> Vec<BridgeTrackPoint> {
    let Ok(proj) = layer.project() else {
        return Vec::new();
    };
    let Ok(state) = proj.read() else {
        return Vec::new();
    };
    let doc = state.store.snapshot();
    drop(state);
    let Some(comp) = doc.comp(layer.comp_id) else {
        return Vec::new();
    };
    let Ok(t) = comp.frame_rate.time_of_frame(frame) else {
        return Vec::new();
    };
    let store = lumit_render::track::Store;
    let Some((media, solved_frame, _held)) =
        lumit_core::track::tracked_solved_frame(&doc, comp, layer.layer_id, t.0.to_f64(), &store)
    else {
        return Vec::new();
    };
    let (cx, cy) = (f64::from(comp.width) * 0.5, f64::from(comp.height) * 0.5);
    lumit_render::track::projected_points(media, solved_frame)
        .into_iter()
        .map(|p| BridgeTrackPoint {
            track: p.track,
            x: cx + p.x,
            y: cy + p.y,
            depth: p.depth,
        })
        .collect()
}

/// `camera`'s solve link and how it is reading at composition frame `frame` —
/// the badge, in one crossing.
#[frb(sync)]
#[must_use]
pub fn camera_link(camera: LayerReference, frame: i64) -> BridgeCameraLink {
    let unlinked = BridgeCameraLink {
        state: BridgeLinkState::Unlinked,
        tracked: None,
    };
    let Ok(proj) = camera.project() else {
        return unlinked;
    };
    let Ok(state) = proj.read() else {
        return unlinked;
    };
    let doc = state.store.snapshot();
    drop(state);
    let Some(comp) = doc.comp(camera.comp_id) else {
        return unlinked;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == camera.layer_id) else {
        return unlinked;
    };
    let LayerKind::Camera { solve_link, .. } = layer.kind else {
        return unlinked;
    };
    let Ok(t) = comp.frame_rate.time_of_frame(frame) else {
        return unlinked;
    };
    let linked = lumit_core::track::camera_pose_of(
        &doc,
        comp,
        layer,
        t.0.to_f64(),
        &lumit_render::track::Store,
    );
    BridgeCameraLink {
        state: match linked.map(|l| l.state) {
            Some(LinkState::Derived) => BridgeLinkState::Derived,
            Some(LinkState::Held) => BridgeLinkState::Held,
            Some(LinkState::Unresolved) => BridgeLinkState::Unresolved,
            _ => BridgeLinkState::Unlinked,
        },
        tracked: solve_link,
    }
}

// ---------------------------------------------------------------------------
// Pressing
// ---------------------------------------------------------------------------

/// Press one of an effect's Action parameters.
///
/// An Action carries no value, so this is an **event** and not a write: nothing
/// is staged, nothing is committed, and no undo entry appears. The built-in
/// buttons are the trackers' and the Roto brush's; an unknown effect or
/// parameter is refused rather than ignored, because a button that silently
/// does nothing is the hardest kind of fault to see.
///
/// A **plugin's** button is the one exception to "no write": it goes to the
/// plugin, which may open its own window and stay there, and what the plugin
/// wrote comes back into the document as one undo step when it returns. `frame`
/// is the playhead, so the plugin sees the frame the user is looking at.
#[frb(sync)]
pub fn fire_effect_action(
    layer: LayerReference,
    effect: Uuid,
    param: String,
    frame: Option<u64>,
) -> Result<(), BridgeError> {
    let item = layer.item()?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    // The Roto brush's two go the same way, and so do the Planar
    // track's four: one doorway, so a press is one crossing whichever effect
    // made it.
    if fx.effect.match_name == lumit_core::roto::ROTO_BRUSH {
        return crate::api::roto::press(&layer, fx, &param);
    }
    // The Planar track's four buttons go the same way as the Camera track's
    // two: one doorway, so a press is one crossing whichever effect made it.
    if fx.effect.match_name == lumit_core::track::PLANAR_TRACK {
        return match param.as_str() {
            CANCEL => {
                lumit_render::track::cancel(effect);
                Ok(())
            }
            PIN => create_corner_pin(layer, effect),
            TRANSFORM_KEYS => create_transform_keys(layer, effect),
            ANALYSE => {
                let media = match item.kind {
                    LayerKind::Footage { item } => item,
                    _ => return Err(BridgeError::NotFootage),
                };
                let (path, fingerprint) = media_source(&layer, media)?;
                let job = lumit_render::track::planar_job_for(&item, path, &fingerprint, true)
                    .ok_or(BridgeError::NotFootage)?;
                match lumit_render::track::request(job) {
                    lumit_render::track::Requested::Started => Ok(()),
                    _ => Err(BridgeError::AnalysisBusy),
                }
            }
            _ => Err(BridgeError::InvalidParam),
        };
    }
    if matches!(fx.effect.namespace, EffectNamespace::Ofx) {
        return press_plugin(layer, effect, param, frame.unwrap_or(0));
    }
    if fx.effect.match_name != lumit_core::track::CAMERA_TRACK {
        return Err(BridgeError::InvalidParam);
    }
    // A Camera track names its source: a footage item, or — on a Precomp
    // layer — the nested composition, whose frames are rendered rather than
    // decoded.
    let media = lumit_core::track::tracked_source_id(&item).ok_or(BridgeError::NotFootage)?;
    match param.as_str() {
        CANCEL => {
            lumit_render::track::cancel(media);
            Ok(())
        }
        ANALYSE => {
            let job = match item.kind {
                LayerKind::Footage { .. } => {
                    let (path, fingerprint) = media_source(&layer, media)?;
                    lumit_render::track::job_for(&item, path, &fingerprint, true)
                }
                _ => {
                    let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
                    let project = projects
                        .get(&layer.project_id)
                        .ok_or(BridgeError::InvalidProject)?
                        .clone();
                    drop(projects);
                    let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
                    let doc = state.store.snapshot();
                    drop(state);
                    lumit_render::track::job_for_precomp(&doc, &item, true)
                }
            }
            .ok_or(BridgeError::NotFootage)?;
            match lumit_render::track::request(job) {
                lumit_render::track::Requested::Started => Ok(()),
                _ => Err(BridgeError::AnalysisBusy),
            }
        }
        _ => Err(BridgeError::InvalidParam),
    }
}

/// Where a footage item's file is, and what names its content.
///
/// The fingerprint the project already holds is used where there is one; only a
/// project that has never fingerprinted this item pays the read, and that read
/// is a head-and-tail hash rather than the whole file. It happens on the
/// caller's thread because the *key* has to exist before the job can be handed
/// over, and the job is what carries the disk work away.
pub(crate) fn media_source(
    layer: &LayerReference,
    media: Uuid,
) -> Result<(PathBuf, Fingerprint), BridgeError> {
    let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
    let project = projects
        .get(&layer.project_id)
        .ok_or(BridgeError::InvalidProject)?
        .clone();
    drop(projects);
    let state = project.read().map_err(|_| BridgeError::ReadFailed)?;
    let doc = state.store.snapshot();
    let footage = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Footage(f) if f.id == media => Some(f),
            _ => None,
        })
        .ok_or(BridgeError::InvalidItem)?;
    let path = crate::api::footage::FootageReference::resolve_path(&state, footage)
        .ok_or(BridgeError::MediaPathUnresolved)?;
    let fingerprint = match footage.media.fingerprint.clone() {
        Some(fingerprint) => fingerprint,
        None => {
            lumit_project::fingerprint_path(&path).map_err(|_| BridgeError::MediaPathUnresolved)?
        }
    };
    Ok((path, fingerprint))
}

// ---------------------------------------------------------------------------
// A plugin's own button
// ---------------------------------------------------------------------------

/// Press a plugin's button on a thread of its own.
///
/// The plugin may open a window and stay in it until the user closes it, so
/// the press is kept off the interface thread. What the plugin wrote goes into
/// the document when it comes back, and the panel hears of that through the
/// ordinary change stream, so the caller has nothing to wait for.
fn press_plugin(
    layer: LayerReference,
    effect: Uuid,
    param: String,
    frame: u64,
) -> Result<(), BridgeError> {
    std::thread::Builder::new()
        .name("plugin press".to_owned())
        .spawn(move || {
            if let Err(why) = press_plugin_now(&layer, effect, &param, frame) {
                lumit_render::gpufx::ofx::note(effect, Some(format!("{why:?}")));
            }
        })
        .map_err(|_| BridgeError::AnalysisBusy)?;
    Ok(())
}

/// The press itself, on whatever thread called it. Split from the spawn so a
/// test can run it and read the document afterwards.
pub(crate) fn press_plugin_now(
    layer: &LayerReference,
    effect: Uuid,
    param: &str,
    frame: u64,
) -> Result<(), BridgeError> {
    let comp = layer.composition()?;
    let item = comp
        .layers
        .iter()
        .find(|l| l.id == layer.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    let def = lumit_core::fx::BUILTIN_DEFS
        .get(&fx.effect.match_name)
        .ok_or(BridgeError::InvalidEffect)?;
    // The layer's own clock at the playhead, which is what its keys are on.
    let at = comp
        .frame_rate
        .time_of_frame(i64::try_from(frame).map_err(|_| BridgeError::InvalidTime)?)
        .map_err(|_| BridgeError::InvalidTime)?
        .0
        .checked_sub(item.start_offset.0)
        .map_err(|_| BridgeError::InvalidTime)?;
    let (rgba, width, height) = press_frame(layer, &comp, effect, frame);
    let source = PressFrame {
        rgba: &rgba,
        width,
        height,
    };
    let pressed = def.press(fx, at.to_f64(), param, &source).map_err(|why| {
        lumit_render::gpufx::ofx::note(effect, Some(why));
        BridgeError::InvalidParam
    })?;
    layer.with_instances(InstanceHome::Effects, move |list| {
        let fx = list
            .iter_mut()
            .find(|e| e.id == effect)
            .ok_or(BridgeError::InvalidEffect)?;
        for (row, value) in pressed.rows {
            if let Some(param) = fx.params.iter_mut().find(|p| p.id == row) {
                write_row(&mut param.value, value, at);
            }
        }
        fx.set_plugin_state(pressed.memory.as_deref().unwrap_or(&[]));
        Ok(())
    })
}

/// The picture the plugin's window is shown: the comp at `frame` with the
/// pressed effect and everything after it on its layer switched off, so the
/// plugin sees what it would be handed to render. Black when there is no
/// graphics adapter, which still lets the window open.
fn press_frame(
    layer: &LayerReference,
    comp: &Composition,
    effect: Uuid,
    frame: u64,
) -> (Vec<u8>, u32, u32) {
    let black = || {
        (
            vec![0; (comp.width * comp.height * 4) as usize],
            comp.width,
            comp.height,
        )
    };
    let Ok(project) = layer.project() else {
        return black();
    };
    let Ok(state) = project.read() else {
        return black();
    };
    let mut doc = lumit_core::model::Document::clone(&state.store.snapshot());
    drop(state);
    let staged = doc.items.iter_mut().find_map(|item| match item {
        ProjectItem::Composition(c) if c.id == layer.comp_id => Some(c),
        _ => None,
    });
    if let Some(item) = staged.and_then(|c| c.layers.iter_mut().find(|l| l.id == layer.layer_id)) {
        let mut reached = false;
        for fx in &mut item.effects {
            reached |= fx.id == effect;
            if reached {
                fx.enabled = false;
            }
        }
    }
    crate::render::thumbnail(&std::sync::Arc::new(doc), layer.comp_id, frame, 1.0)
        .unwrap_or_else(black)
}

/// Put a value the plugin wrote onto the document's row at `at`, keeping the
/// row's animation: a static row takes the value, a keyed row gets a key
/// there, and a row an expression drives stays the expression's.
fn write_row(value: &mut EffectValue, written: Value, at: Rational) {
    fn set(property: &mut Property, v: f64, at: Rational) {
        if let Animation::Static(held) = &mut property.animation {
            *held = v;
            return;
        }
        if matches!(property.animation, Animation::Expression(_)) {
            return;
        }
        property.insert_key_preserving_shape(at);
        if let Animation::Keyframed(keys) = &mut property.animation {
            let there = keys
                .iter_mut()
                .find(|key| (key.time.to_f64() - at.to_f64()).abs() < 1e-9);
            if let Some(key) = there {
                key.value = v;
            }
        }
    }
    match (value, written) {
        (EffectValue::Float(property), Value::Float(v)) => set(property, f64::from(v), at),
        (EffectValue::Float(property), Value::Int(v)) => set(property, f64::from(v), at),
        (EffectValue::Bool(held), Value::Bool(v)) => *held = v,
        (EffectValue::Choice(held), Value::Choice(v)) => *held = v,
        (EffectValue::Colour(channels), Value::Colour(colour)) => {
            for (property, v) in channels.iter_mut().zip(colour) {
                set(property, f64::from(v), at);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The gestures a solve offers
// ---------------------------------------------------------------------------

/// Add a Camera layer whose motion is derived from `tracked`'s solve.
///
/// The link is the whole point: nothing is copied, so re-analysing the shot
/// moves this camera with it, and every clip of the same footage reads the same
/// solve through its own time mapping. Its transform rows are the **correction
/// lane** — dragging one nudges the solved motion rather than replacing
/// it — and they start at the pose captured here, which is that lane's nought.
#[frb(sync)]
pub fn add_solved_camera(tracked: LayerReference) -> Result<LayerReference, BridgeError> {
    use lumit_core::anim::Property;
    use lumit_core::model::TransformGroup;

    let comp = tracked.composition()?;
    let mut layer = crate::edits::base_layer(
        "Camera".into(),
        LayerKind::Camera {
            zoom: Property::fixed(f64::from(comp.width) * 50.0 / 36.0),
            solve_link: Some(tracked.layer_id),
            correction_base: None,
        },
        comp.duration.0,
        TransformGroup {
            position_x: Property::fixed(f64::from(comp.width) * 0.5),
            position_y: Property::fixed(f64::from(comp.height) * 0.5),
            ..TransformGroup::default()
        },
    );
    // The layer arrives already linked, so `Op::SetCameraSolveLink` — which is
    // where the base is normally captured — is never applied to it. Captured
    // here instead, off the layer that is about to be added, which is the same
    // arithmetic on the same properties.
    let base = lumit_core::model::stored_camera_pose_lt(&layer, 0.0);
    if let LayerKind::Camera {
        correction_base, ..
    } = &mut layer.kind
    {
        *correction_base = base.map(Box::new);
    }
    let id = layer.id;
    tracked.commit(lumit_core::Op::AddLayer {
        comp: tracked.comp_id,
        index: 0,
        layer: Box::new(layer),
    })?;
    Ok(LayerReference::new(tracked.project_id, tracked.comp_id, id))
}

/// Bake a solve-linked camera into keyframes and sever the link.
///
/// One key per composition frame, at the composition's rate, exactly the motion
/// that was being derived — and from then on an ordinary camera the user edits.
/// The bake is honest about being many keyframes: they are real, editable, and
/// the graph editor shows them like any others.
///
/// One undo step, and undo restores the link last, which is what lets the
/// read-only guard stay unconditional in both directions.
#[frb(sync)]
pub fn convert_camera_to_keyframes(camera: LayerReference) -> Result<(), BridgeError> {
    let proj = camera.project()?;
    let doc = {
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.snapshot()
    };
    let op = lumit_core::track::bake_solve_link(
        &doc,
        camera.comp_id,
        camera.layer_id,
        &lumit_render::track::Store,
    )
    .ok_or(BridgeError::NotLinked)?;
    camera.commit(op)
}

/// **Clear corrections**: put a linked camera's own properties back to
/// the pose the link was made at, leaving the link itself alone.
///
/// One undo step. Refused when there is no link, or nothing in the lane — a
/// command that committed an empty batch would put a step on the undo stack
/// that changed nothing.
#[frb(sync)]
pub fn clear_camera_corrections(camera: LayerReference) -> Result<(), BridgeError> {
    let proj = camera.project()?;
    let doc = {
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.snapshot()
    };
    let op = lumit_core::track::clear_corrections(&doc, camera.comp_id, camera.layer_id)
        .ok_or(BridgeError::NotLinked)?;
    camera.commit(op)
}

/// Point `camera` at a tracked layer, or clear the link.
///
/// The link is a property of the Camera layer, so this is an ordinary
/// undoable edit — and `None` is how a camera stops being derived without
/// baking anything.
#[frb(sync)]
pub fn set_camera_solve_link(
    camera: LayerReference,
    tracked: Option<Uuid>,
) -> Result<(), BridgeError> {
    let LayerKind::Camera { .. } = camera.item()?.kind else {
        return Err(BridgeError::NotCamera);
    };
    camera.commit(lumit_core::Op::SetCameraSolveLink {
        comp: camera.comp_id,
        layer: camera.layer_id,
        solve_link: tracked,
    })
}

/// Put a Null (or a Solid) at the mean solved position of `tracks` — the
/// Camera track's creation gesture, After Effects' own.
///
/// The layer is 3D and sits where the points are, **oriented to face the
/// camera**: a layer carrying the camera's own rotation is parallel to its image
/// plane, which is what facing it means. `frame` is the composition frame the
/// selection was made on, which is the moment whose camera the layer is turned
/// to.
///
/// Refused when none of the ids names a solved point — there is no position to
/// put anything at, and a layer at the origin would be a silent lie.
#[frb(sync)]
pub fn add_layer_at_points(
    tracked: LayerReference,
    tracks: Vec<u32>,
    frame: i64,
    solid: bool,
) -> Result<LayerReference, BridgeError> {
    use lumit_core::anim::Property;
    use lumit_core::model::{LinearColour, SolidDef, TransformGroup};
    use lumit_core::ops::AutoFolderKind;

    let comp = tracked.composition()?;
    let media = tracked_media(&tracked).ok_or(BridgeError::NotFootage)?;
    let at = lumit_render::track::point_centroid(media, &tracks).ok_or(BridgeError::NoSolve)?;

    // The camera the layer is turned to face: the active one at this frame,
    // with its link followed, so a null made under a derived camera faces the
    // motion that is actually being drawn.
    let doc = {
        let proj = tracked.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.snapshot()
    };
    let t = comp
        .frame_rate
        .time_of_frame(frame)
        .map_err(|_| BridgeError::InvalidTime)?;
    let facing = doc
        .comp(tracked.comp_id)
        .and_then(|c| lumit_render::track::camera_pose(&doc, c, t.0.to_f64()))
        .map_or((0.0, 0.0, 0.0), |pose| pose.rotation_deg);

    let transform = TransformGroup {
        position_x: Property::fixed(at[0]),
        position_y: Property::fixed(at[1]),
        position_z: Property::fixed(at[2]),
        rotation_x: Property::fixed(facing.0),
        rotation_y: Property::fixed(facing.1),
        rotation: Property::fixed(facing.2),
        ..TransformGroup::default()
    };

    let mut ops = Vec::new();
    let (name, kind, transform) = if solid {
        // A solid is an asset, so it is filed in the Solids auto-folder the
        // same way `add_solid_layer` files one — one batch, one undo step.
        let (folder, folder_ops) =
            crate::edits::ensure_auto_folder_ops(&doc, AutoFolderKind::Solids);
        ops.extend(folder_ops);
        let def = Uuid::now_v7();
        let solids = doc
            .items
            .iter()
            .filter(|i| matches!(i, ProjectItem::Solid(_)))
            .count();
        let name = format!("White solid {}", solids + 1);
        let added = ops
            .iter()
            .filter(|o| matches!(o, lumit_core::Op::AddItem { .. }))
            .count();
        // A hundred pixels square: big enough to see where the point is,
        // small enough not to cover the shot it was found in.
        let edge = 100;
        ops.push(lumit_core::Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Solid(SolidDef {
                id: def,
                name: name.clone(),
                colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                width: edge,
                height: edge,
                extra: serde_json::Map::new(),
            })),
        });
        ops.push(crate::edits::file_into_folder_op(&doc, folder, def));
        (
            name,
            LayerKind::Solid { def },
            TransformGroup {
                // The anchor at the solid's middle, so it pivots about the
                // point rather than hanging off its corner.
                anchor_x: Property::fixed(f64::from(edge) * 0.5),
                anchor_y: Property::fixed(f64::from(edge) * 0.5),
                ..transform
            },
        )
    } else {
        ("Track null".to_owned(), LayerKind::Null, transform)
    };

    let mut layer = crate::edits::base_layer(name, kind, comp.duration.0, transform);
    // 2.5D, or the position in z means nothing and the layer sits flat over the
    // picture wherever the camera goes.
    layer.switches.three_d = true;
    crate::edits::solo_on_arrival(&mut layer, comp.layers.iter());
    let id = layer.id;
    ops.push(lumit_core::Op::AddLayer {
        comp: tracked.comp_id,
        index: 0,
        layer: Box::new(layer),
    });
    tracked.commit(lumit_core::Op::Batch { ops })?;
    Ok(LayerReference::new(tracked.project_id, tracked.comp_id, id))
}

/// The media item a layer carrying an enabled Camera track reads, or `None` for
/// any layer that is not one.
fn tracked_media(layer: &LayerReference) -> Option<Uuid> {
    lumit_core::track::tracked_source_id(&layer.item().ok()?)
}
// Which layer of a composition is the tracked one is deliberately **not** a
// call: the read model already carries every layer's every effect, so
// the interface finds the layer whose stack holds an enabled Camera track from
// data it is already holding. A call here would be one crossing per repaint for
// an answer that never changes between document revisions.

// ---------------------------------------------------------------------------
// The planar track
// ---------------------------------------------------------------------------

/// Everything the Planar track's status row draws, in one crossing.
///
/// Deliberately not [`BridgeTrackStatus`] with two fields ignored. A planar
/// track has no point cloud and no reprojection error, and a camera solve has
/// no re-anchor count; one struct carrying both would have four rows that mean
/// nothing in half the places it is read, and the panel would have to know
/// which half it was holding.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgePlanarStatus {
    pub stage: BridgeTrackStage,
    /// Frames followed so far, and how many there are. Both zero outside
    /// [`BridgeTrackStage::Tracking`].
    pub done: u32,
    pub total: u32,
    /// Set only at [`BridgeTrackStage::Failed`].
    pub failure: Option<BridgeTrackFailure>,
    /// How many frames of the clip carry the surface, and how many the clip
    /// has. `frames < clip_frames` is a **partial** track: the surface was lost
    /// part-way and what was followed is the part before it.
    pub frames: u32,
    pub clip_frames: u32,
    /// How many times the measurement was re-anchored (docs/impl/tracking.md
    /// §6). Zero is a track measured entirely against its reference frame, and
    /// so one carrying no accumulated drift at all — which is the one number
    /// about a planar track that says how much to trust its far end.
    pub reanchors: u32,
}

impl BridgePlanarStatus {
    fn idle() -> Self {
        BridgePlanarStatus {
            stage: BridgeTrackStage::Idle,
            done: 0,
            total: 0,
            failure: None,
            frames: 0,
            clip_frames: 0,
            reanchors: 0,
        }
    }
}

/// How the Planar track instance `effect` on `layer` is getting on.
///
/// Polled while it is moving and never otherwise, exactly as
/// [`track_status`] is — the reading is a value in a map, not a subscription.
/// The answer is filed under the **effect instance**, because what was tracked
/// is the quad this instance holds.
#[frb(sync)]
#[must_use]
pub fn planar_status(layer: LayerReference, effect: Uuid) -> BridgePlanarStatus {
    let mut status = BridgePlanarStatus::idle();
    let Ok(item) = layer.item() else {
        return status;
    };
    if !item
        .effects
        .iter()
        .any(|e| e.id == effect && e.effect.match_name == lumit_core::track::PLANAR_TRACK)
    {
        return status;
    }
    match lumit_render::track::progress(effect) {
        Some(lumit_render::track::Progress::Queued) => status.stage = BridgeTrackStage::Queued,
        Some(lumit_render::track::Progress::Tracking { done, total }) => {
            status.stage = BridgeTrackStage::Tracking;
            status.done = u32::try_from(done).unwrap_or(u32::MAX);
            status.total = u32::try_from(total).unwrap_or(u32::MAX);
        }
        Some(lumit_render::track::Progress::Solving) => status.stage = BridgeTrackStage::Solving,
        Some(lumit_render::track::Progress::Done) => status.stage = BridgeTrackStage::Done,
        Some(lumit_render::track::Progress::Cancelled) => {
            status.stage = BridgeTrackStage::Cancelled
        }
        Some(lumit_render::track::Progress::Failed(why)) => {
            status.stage = BridgeTrackStage::Failed;
            status.failure = Some(failure_of(&why));
        }
        None => {}
    }
    if let Some(tracked) = lumit_render::track::planar(effect) {
        if status.stage == BridgeTrackStage::Idle {
            status.stage = BridgeTrackStage::Done;
        }
        status.frames = u32::try_from(tracked.track.frames.len()).unwrap_or(u32::MAX);
        status.clip_frames = u32::try_from(tracked.clip_frames).unwrap_or(u32::MAX);
        status.reanchors = tracked.track.reanchors;
    }
    status
}

/// **Create corner pin**: put a Corner pin on the layer the Planar
/// track's *Pin layer* row names, its four points keyframed to the tracked
/// surface.
///
/// One undoable edit, and one the user can throw away like any other: the pin is
/// an ordinary effect with ordinary keyframes on it from the moment it lands.
///
/// Refused when nothing has been tracked under this instance, or when no pin
/// layer has been chosen — a button that quietly did nothing would be the
/// hardest kind of fault to see.
#[frb(sync)]
pub fn create_corner_pin(tracked: LayerReference, effect: Uuid) -> Result<(), BridgeError> {
    let item = tracked.item()?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    if fx.effect.match_name != lumit_core::track::PLANAR_TRACK {
        return Err(BridgeError::InvalidParam);
    }
    let target = match fx.param(lumit_core::track::PIN_LAYER_PARAM) {
        Some(lumit_core::model::EffectValue::Layer(Some(id))) => *id,
        _ => return Err(BridgeError::InvalidLayer),
    };
    let doc = {
        let proj = tracked.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.snapshot()
    };
    let op = lumit_core::track::corner_pin_from_track(
        &doc,
        tracked.comp_id,
        tracked.layer_id,
        effect,
        target,
        &lumit_render::track::Store,
    )
    .ok_or(BridgeError::NoSolve)?;
    tracked.commit(op)
}

/// **Create transform keys**: the movement the Planar track measured,
/// written onto the *Pin layer*'s own Position — and, unless the **Transform**
/// row says position alone, its Rotation and Scale as well.
///
/// The corner pin's sibling, sharing its analysis, its target row and its
/// refusals. One undoable edit, and ordinary keyframes from the moment they
/// land.
///
/// Not on the frb surface: nothing in Dart calls it, the press arrives through
/// [`fire_effect_action`] like every other Action, and a generated binding
/// nobody uses is a codegen run nobody needed.
pub(crate) fn create_transform_keys(
    tracked: LayerReference,
    effect: Uuid,
) -> Result<(), BridgeError> {
    let item = tracked.item()?;
    let fx = item
        .effects
        .iter()
        .find(|e| e.id == effect)
        .ok_or(BridgeError::InvalidEffect)?;
    if fx.effect.match_name != lumit_core::track::PLANAR_TRACK {
        return Err(BridgeError::InvalidParam);
    }
    let target = match fx.param(lumit_core::track::PIN_LAYER_PARAM) {
        Some(lumit_core::model::EffectValue::Layer(Some(id))) => *id,
        _ => return Err(BridgeError::InvalidLayer),
    };
    // One point can only say where it went, so only that is written. An index
    // this build does not know reads as the whole movement, which is the row's
    // own default — never a fault (docs/14 §4).
    let scale_and_rotation = !matches!(
        fx.param(lumit_core::track::FOLLOW_PARAM),
        Some(lumit_core::model::EffectValue::Choice(
            lumit_core::fx::effects::planar_track::FOLLOW_ONE_POINT
        ))
    );
    let doc = {
        let proj = tracked.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.snapshot()
    };
    let op = lumit_core::track::transform_from_track(
        &doc,
        tracked.comp_id,
        tracked.layer_id,
        effect,
        target,
        scale_and_rotation,
        &lumit_render::track::Store,
    )
    .ok_or(BridgeError::NoSolve)?;
    tracked.commit(op)
}

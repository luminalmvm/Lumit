//! The solve link: a Camera layer driven by a tracked layer's camera solve
//! (docs/03-DATA-MODEL.md §5.6, docs/impl/tracking.md phase 4).
//!
//! # In plain terms
//!
//! When a shot has been camera-tracked, the engine knows where the real camera
//! was on every frame of that *file*. The obvious thing to do with that is to
//! stamp it onto a Camera layer as several thousand keyframes. Lumit does not,
//! at least not at first: the Camera layer **points at** the tracked layer and
//! works out its placement afresh each frame. Re-solve the shot, trim it,
//! reorder its cuts, ramp its speed — the camera follows, because nothing was
//! ever copied.
//!
//! Working out "which moment of the file is on screen right now" is the whole
//! of the mechanism, and it is the same walk the renderer already does to
//! decide which frame of the file to decode: composition time, less the
//! layer's offset, through the layer's or the clip's Retime, to a moment of the
//! source. That is why a reordered Sequence layer and a speed ramp both come
//! out right for free — the tracker ran on the file, once.
//!
//! **Track once, then nudge**. A solve is a measurement, and a
//! measurement can be a little wrong — the ground plane tilts, the shot drifts,
//! the whole move wants to sit half a metre to the left. So a linked camera's
//! own transform rows are not read-only: they are a **correction lane**. What
//! they hold over and above the pose they held when the link was made is added,
//! channel by channel, to the solved pose. Drag the camera and you have keyed a
//! correction; re-analyse the shot and the correction rides on top of the new
//! solve, because it was never part of it.
//!
//! Two honest failures, and neither is silent:
//!
//! - The link asks for a moment **outside** what was solved (the layer runs on
//!   past the solved range, or a retime reaches before its start). The camera
//!   **holds** the nearest solved frame — the last derived motion — and the
//!   reading says [`LinkState::Held`].
//! - The link cannot be followed at all: the layer was deleted, its media is
//!   offline, or nothing has been solved for it. The camera falls back to the
//!   properties the document itself holds, read as a **pose** rather than as a
//!   correction — the numbers it had when the link was made, plus whatever has
//!   been nudged since — and the reading says [`LinkState::Unresolved`]. Never a
//!   freeze nobody mentioned, never a crash.
//!
//! **The store is a trait, not a thing.** What actually holds the solves is the
//! project's `track/` sidecar, and it arrives in stage 2. Everything here talks
//! to [`CameraSolveStore`], so the model half is complete and testable now with
//! a written-down solve standing in for a real one.

use std::sync::Arc;

use uuid::Uuid;

use crate::anim::{Animation, Keyframe, SideInterp};
use crate::model::{stored_camera_pose, CameraPose, Composition, Document, Layer, LayerKind};
use crate::ops::Op;
use crate::sequence::ClipSource;
use crate::time::Rational;

/// The match name of the Camera track effect — what marks a layer as *the*
/// tracked layer inside a precomp (the parent-comp rule).
///
/// One spelling, here, because two places compare against it: the walk below
/// and the effect's own declaration
/// ([`crate::fx::effects::camera_track`](crate::fx::effects::camera_track)).
pub const CAMERA_TRACK: &str = "camera_track";

/// What was solved for one piece of media: the frame rate its solved frames are
/// numbered at, and the range that actually has poses.
///
/// Frames rather than seconds because a solve is per *frame* — one pose per
/// decoded picture — and rounding a source time to a frame is the one place
/// that conversion should happen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolvedRange {
    /// The rate the solved frames are numbered at (the media's own rate).
    pub fps: f64,
    /// First solved frame, inclusive.
    pub first_frame: i64,
    /// Last solved frame, inclusive.
    pub last_frame: i64,
}

/// Where the solved camera paths live, as far as the model is concerned.
///
/// The real implementation is the project's `track/` sidecar and arrives in
/// stage 2; tests inject a written-down one. Deliberately two small methods
/// rather than one that hands back a whole solve: the model needs the range to
/// decide whether it is holding, and one pose to place the camera, and nothing
/// else — a trait that handed over the point cloud as well would be a trait
/// the model could accidentally start depending on.
///
/// **The poses are already in Lumit's camera terms**: comp pixels, AE-style
/// position and rotation, exactly what [`CameraPose`] means everywhere else.
/// Turning `lumit_track::solve::CameraSolve`'s world-to-camera rotations, its solve
/// units and its focal in source pixels into these is the *store's* job, and it
/// is stage 2's — putting it here would make the model depend on the tracker,
/// which the crate graph forbids in that direction (docs/05).
pub trait CameraSolveStore {
    /// What has been solved for the footage item `media`, or `None` when
    /// nothing has (never analysed, media offline, sidecar rebuilt away).
    fn solved_range(&self, media: Uuid) -> Option<SolvedRange>;

    /// The solved camera at source frame `frame` of `media`. `None` for a frame
    /// outside the solved range, which callers here never ask for — they clamp
    /// first, because clamping *is* the hold.
    fn solved_pose(&self, media: Uuid, frame: i64) -> Option<CameraPose>;
}

/// How a Camera layer's placement was arrived at — the flag the interface must
/// be able to read, so a link that stopped working says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// No solve link: an ordinary camera, driven by its own properties.
    Unlinked,
    /// Derived from the linked layer's solve, at the frame it asked for.
    Derived,
    /// Derived, but from the nearest solved frame rather than the one asked
    /// for — the layer has run past the solved range, or a retime reached
    /// before its start. The last derived motion, held.
    Held,
    /// The link could not be followed at all (layer gone, media offline,
    /// nothing solved). The camera's own stored properties are in play, and the
    /// interface says so rather than showing a freeze nobody explained.
    Unresolved,
}

/// A camera's placement plus how it was arrived at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkedPose {
    pub pose: CameraPose,
    pub state: LinkState,
}

/// How many precomp hops the walk will make before giving up. A cycle is
/// already an invalid document (docs/03 §5.2) and is guarded at insertion; this
/// is the belt to that brace, so a malformed file degrades to `Unresolved`
/// rather than recursing until the stack ends (14-ENGINEERING-RULES §4).
const MAX_NESTING: u32 = 16;

/// The **active** camera's placement at comp time `t`, following its solve link
/// where it has one. `None` with no camera layer, exactly as
/// [`Composition::camera_pose`] means it.
///
/// This is the reading the render path wants once solves exist; today's callers
/// use `camera_pose`, which is this with no store and therefore
/// [`LinkState::Unresolved`] on any linked camera — the correct degrade, and
/// what stage 2 replaces by threading the real store.
#[must_use]
pub fn camera_pose_at(
    doc: &Arc<Document>,
    comp: &Composition,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let layer = comp.active_camera(t)?;
    camera_pose_of(doc, comp, layer, t, store)
}

/// The solves the renderer holds, once it holds any. Process-wide because the
/// renderer's own store is.
static SOLVES: std::sync::OnceLock<&'static (dyn CameraSolveStore + Sync)> =
    std::sync::OnceLock::new();

/// Hand the model the renderer's solves, so a camera that follows one is posed
/// the same way everywhere. The first store given is the one kept.
pub fn use_solve_store(store: &'static (dyn CameraSolveStore + Sync)) {
    let _ = SOLVES.set(store);
}

/// The renderer's solves, once it has handed them over.
pub(crate) fn solve_store() -> Option<&'static dyn CameraSolveStore> {
    SOLVES.get().map(|store| *store as &dyn CameraSolveStore)
}

/// Nothing solved yet: a link with nothing to follow reads the stored pose.
pub(crate) struct Unsolved;

impl CameraSolveStore for Unsolved {
    fn solved_range(&self, _media: Uuid) -> Option<SolvedRange> {
        None
    }
    fn solved_pose(&self, _media: Uuid, _frame: i64) -> Option<CameraPose> {
        None
    }
}

/// The active camera's placement at comp time `t` as the picture is drawn
/// with it, its solve link and its parents followed. The one reading the
/// renderer and expressions both take.
#[must_use]
pub fn active_camera_pose(doc: &Arc<Document>, comp: &Composition, t: f64) -> Option<CameraPose> {
    // The renderer hands its store over the first time it reads a solve, so
    // until then nothing has been solved.
    let store = solve_store().unwrap_or(&Unsolved);
    camera_pose_at(doc, comp, t, store).map(|linked| linked.pose)
}

/// One named Camera layer's placement in the comp at comp time `t`, whether
/// or not it is the active one, carried out of its parent's space.
#[must_use]
pub fn camera_pose_of(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let mut linked = own_camera_pose_of(doc, comp, layer, t, store)?;
    let context = crate::model::comp_context(doc, comp, t);
    let read = crate::model::context_reader(&context, t);
    if let Some(space) = crate::model::parent_space(comp, layer, t, &read, Some((doc, store))) {
        linked.pose = space.pose(linked.pose);
    }
    Some(linked)
}

/// [`camera_pose_of`] in the camera's own values, before its parent: what the
/// bake walks, frame by frame, and writes back.
#[must_use]
pub fn own_camera_pose_of(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let context = crate::model::comp_context(doc, comp, t);
    let stored = stored_camera_pose(layer, t, &crate::model::context_reader(&context, t))?;
    let LayerKind::Camera {
        solve_link,
        correction_base,
        ..
    } = &layer.kind
    else {
        return None;
    };
    let Some(tracked) = *solve_link else {
        return Some(LinkedPose {
            pose: stored,
            state: LinkState::Unlinked,
        });
    };
    match derived_pose(doc, comp, tracked, t, store, 0) {
        Some((solved, held)) => Some(LinkedPose {
            pose: correct(solved, stored, correction_base.as_deref()),
            state: if held {
                LinkState::Held
            } else {
                LinkState::Derived
            },
        }),
        None => Some(LinkedPose {
            pose: stored,
            state: LinkState::Unresolved,
        }),
    }
}

/// The solved pose with the layer's own correction on it.
///
/// **Channel-wise addition**, and that is the whole composition: each of the
/// seven numbers is `solved + (stored − base)`. Order does not arise, because
/// addition on a channel commutes with itself — two corrections in either order
/// give the same camera, and undo is exact rather than nearly exact.
///
/// The alternative was to compose the correction as a transform *in the solved
/// camera's own space*, which is what a parent-child rig would do. It was
/// rejected for two reasons. A row would stop meaning what it says — "position
/// x" would move the camera along the shot's axis rather than the comp's, so a
/// pan would swing the correction round with it and a fixed nudge would drift
/// across the shot, which is the opposite of correcting a drift. And the rows
/// would no longer be the curve the graph editor draws: a correction keyed in
/// camera space and read back in comp space is not the shape that was dragged.
///
/// No base — an unlinked camera, or a project written before the lane existed —
/// means no correction, so the solve is followed exactly.
#[must_use]
fn correct(solved: CameraPose, stored: CameraPose, base: Option<&CameraPose>) -> CameraPose {
    let Some(base) = base else {
        // No lane, so the solve is followed exactly - but a solve carries no
        // depth of field, and the camera's own is not a correction.
        return CameraPose {
            dof: stored.dof,
            ..solved
        };
    };
    CameraPose {
        zoom: solved.zoom + (stored.zoom - base.zoom),
        position: (
            solved.position.0 + (stored.position.0 - base.position.0),
            solved.position.1 + (stored.position.1 - base.position.1),
            solved.position.2 + (stored.position.2 - base.position.2),
        ),
        rotation_deg: (
            solved.rotation_deg.0 + (stored.rotation_deg.0 - base.rotation_deg.0),
            solved.rotation_deg.1 + (stored.rotation_deg.1 - base.rotation_deg.1),
            solved.rotation_deg.2 + (stored.rotation_deg.2 - base.rotation_deg.2),
        ),
        // A solve has no depth of field; the camera's own is what applies.
        dof: stored.dof,
    }
}

/// Whether `layer`'s correction lane holds anything — **edited since track**,
/// which is what the dot beside the link says.
///
/// A property answers yes the moment it stops being exactly the static value it
/// was linked at: a different number, a keyframe, or an expression. A lane keyed
/// with every key back on the base reads as corrected, which is honest — the
/// user put keys there, and Clear corrections is what takes them away.
#[must_use]
pub fn has_correction(layer: &Layer) -> bool {
    let LayerKind::Camera {
        zoom,
        solve_link: Some(_),
        correction_base: Some(base),
        ..
    } = &layer.kind
    else {
        return false;
    };
    let tr = &layer.transform;
    [
        (&tr.position_x.animation, base.position.0),
        (&tr.position_y.animation, base.position.1),
        (&tr.position_z.animation, base.position.2),
        (&tr.rotation_x.animation, base.rotation_deg.0),
        (&tr.rotation_y.animation, base.rotation_deg.1),
        (&tr.rotation.animation, base.rotation_deg.2),
        (&zoom.animation, base.zoom),
    ]
    .into_iter()
    .any(|(animation, nought)| *animation != Animation::Static(nought))
}

/// **Clear corrections**: put every property of the correction lane back
/// to the pose the link was made at, as one undoable step.
///
/// The link is untouched — this is the *nudge* being taken back, not the track.
/// `None` when `camera` is not a linked Camera layer in `comp`, or when there is
/// nothing in the lane to clear.
#[must_use]
pub fn clear_corrections(doc: &Document, comp: Uuid, camera: Uuid) -> Option<Op> {
    use crate::model::TransformProp;

    let c = doc.comp(comp)?;
    let layer = c.layers.iter().find(|l| l.id == camera)?;
    if !has_correction(layer) {
        return None;
    }
    let LayerKind::Camera {
        correction_base: Some(base),
        ..
    } = &layer.kind
    else {
        return None;
    };
    let props = [
        (TransformProp::PositionX, base.position.0),
        (TransformProp::PositionY, base.position.1),
        (TransformProp::PositionZ, base.position.2),
        (TransformProp::RotationX, base.rotation_deg.0),
        (TransformProp::RotationY, base.rotation_deg.1),
        (TransformProp::Rotation, base.rotation_deg.2),
    ];
    let mut ops: Vec<Op> = props
        .into_iter()
        .map(|(prop, value)| Op::SetTransformProperty {
            comp,
            layer: camera,
            prop,
            animation: Animation::Static(value),
        })
        .collect();
    ops.push(Op::SetCameraZoom {
        comp,
        layer: camera,
        animation: Animation::Static(base.zoom),
    });
    Some(Op::Batch { ops })
}

/// Which solved frame of which media the tracked layer `tracked` is showing at
/// comp time `t`, clamped into what was actually solved — and whether the clamp
/// bit, which is what [`LinkState::Held`] means.
///
/// The one walk everything downstream shares: the camera link asks it for a
/// pose, and the interface asks it which frame of the point cloud to draw. Two
/// readings of the same moment cannot disagree if there is only one walk.
#[must_use]
pub fn tracked_solved_frame(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<(Uuid, i64, bool)> {
    solved_frame(doc, comp, tracked, t, store, 0)
}

/// [`tracked_solved_frame`] with the precomp depth the walk is already at.
fn solved_frame(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
    store: &dyn CameraSolveStore,
    depth: u32,
) -> Option<(Uuid, i64, bool)> {
    let (media, st) = tracked_source_at(doc, comp, tracked, t, depth)?;
    if !st.is_finite() {
        return None;
    }
    let range = store.solved_range(media)?;
    if !range.fps.is_finite() || range.fps <= 0.0 || range.last_frame < range.first_frame {
        return None;
    }
    let asked = (st * range.fps).round();
    // Past `i64`'s reach is past any solve's reach, and saturating is the same
    // answer as clamping would have been.
    #[allow(clippy::cast_possible_truncation)]
    let asked = asked.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
    let frame = asked.clamp(range.first_frame, range.last_frame);
    Some((media, frame, frame != asked))
}

/// The solved pose the link comes to at comp time `t`, and whether it was
/// **held** (clamped into the solved range) to get there.
fn derived_pose(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
    store: &dyn CameraSolveStore,
    depth: u32,
) -> Option<(CameraPose, bool)> {
    let (media, frame, held) = solved_frame(doc, comp, tracked, t, store, depth)?;
    let pose = store.solved_pose(media, frame)?;
    Some((pose, held))
}

/// Walk comp time `t` down `layer`'s time chain to `(footage item, source
/// time)` — the one question the link asks, and the same mapping the renderer
/// uses to choose which frame to decode.
///
/// Through a **Precomp** layer (or a comp-sourced clip) the walk continues
/// inside: the nested comp's own tracked layer is found by the Camera track
/// effect on it, which is what makes the parent-comp workflow resolve — a
/// linked camera in the parent comp names the precomp layer, and the chain
/// finds the footage inside.
///
/// **Unless the Precomp layer wears the effect itself**, in which case the walk
/// stops there and the nested comp *is* the tracked source. A Camera track is
/// allowed on a Precomp layer, and what that asks to track is the picture the
/// nested comp makes — a comp of stills panned by a camera move has no footage
/// inside it to follow, and there is nothing further down to descend to.
/// The solve is filed under the comp's own id, the store being asked about a
/// source rather than about a file.
///
/// `None` for a layer that has no source to track (a solid, a null, a camera),
/// a gap in a Sequence layer, a precomp naming a comp that is gone, or a
/// precomp with no tracked layer in it.
fn tracked_source_at(
    doc: &Document,
    comp: &Composition,
    layer: Uuid,
    t: f64,
    depth: u32,
) -> Option<(Uuid, f64)> {
    if depth > MAX_NESTING {
        return None;
    }
    let l = comp.layers.iter().find(|l| l.id == layer)?;
    let lt = crate::time::layer_time(t, l.start_offset.0);
    match &l.kind {
        LayerKind::Footage { item } => Some((*item, l.source_time_at(lt))),
        LayerKind::Precomp { comp: nested } => {
            let st = l.source_time_at(lt);
            if wears_camera_track(l) || wears_planar_track(l) {
                Some((*nested, st))
            } else {
                descend(doc, *nested, st, depth)
            }
        }
        LayerKind::Sequence { clips } => {
            // The clip under the playhead and the moment of its source it
            // shows — trimmed extent and retime included, which is exactly why
            // reordering the cuts or ramping one of them changes nothing about
            // which solved frame comes back.
            let (_, source, st) = crate::sequence::resolve(clips, lt)?;
            match source {
                ClipSource::Footage(item) => Some((item, st)),
                ClipSource::Comp(nested) => descend(doc, nested, st, depth),
                ClipSource::Solid(_) => None,
            }
        }
        _ => None,
    }
}

/// One hop into a nested composition at *its* comp time `t`, continuing from
/// the tracked layer inside it.
fn descend(doc: &Document, nested: Uuid, t: f64, depth: u32) -> Option<(Uuid, f64)> {
    let nc = doc.comp(nested)?;
    let inner = tracked_layer(nc)?;
    tracked_source_at(doc, nc, inner, t, depth + 1)
}

/// Whether `layer` carries an enabled Camera track — the effect *is* the
/// handle, spelled once because three questions ask it.
#[must_use]
pub fn wears_camera_track(layer: &Layer) -> bool {
    layer
        .effects
        .iter()
        .any(|e| e.enabled && e.effect.match_name == CAMERA_TRACK)
}

/// What a tracked layer's solve is filed under: its footage item, or — for a
/// Precomp layer wearing the effect — the nested composition it shows.
///
/// The store is asked about a *source*, not about a file (docs/impl/tracking.md
/// §5e), and this is the one place that says which uuid a given layer's source
/// is. `None` for a layer with no Camera track on it, or one whose kind has no
/// source an analysis could read.
#[must_use]
pub fn tracked_source_id(layer: &Layer) -> Option<Uuid> {
    if !wears_camera_track(layer) {
        return None;
    }
    match layer.kind {
        LayerKind::Footage { item } => Some(item),
        LayerKind::Precomp { comp } => Some(comp),
        _ => None,
    }
}

/// The layer in `comp` carrying an enabled Camera track effect — the tracked
/// layer, the effect being the handle. The first one, in stack order, so the
/// answer never depends on the playhead; `None` when the comp has none.
#[must_use]
pub fn tracked_layer(comp: &Composition) -> Option<Uuid> {
    comp.layers
        .iter()
        .find(|l| wears_camera_track(l))
        .map(|l| l.id)
}

/// **Convert to keyframes**: bake the derived motion into ordinary
/// keyframes and sever the link, as one undoable step.
///
/// One key per composition frame across the camera layer's own span, linear on
/// both sides, on the six transform properties and on the camera's zoom — real,
/// editable keyframes the graph editor draws like any others, which is the
/// bargain the ruling makes for being honest that there are a lot of them.
///
/// **The link is cleared first**, inside the batch, and it still is now that a
/// linked camera's transform takes edits: the numbers written after it
/// are the *corrected* pose, absolute, and they would be read as a correction if
/// the link were still on. Clearing first also drops the correction base, so
/// nothing is left over to be added twice. Undo reverses the members — the
/// transforms go back, then the link, which re-derives the base it had.
///
/// Deterministic: the frames come from the comp's own rate, the poses from the
/// same [`own_camera_pose_of`] the render reads, and a frame the link cannot
/// resolve bakes the same fallback the render would have shown. `None` when
/// `camera` is not a Camera layer in `comp`, when it carries no link, or when
/// its span holds no frames.
#[must_use]
pub fn bake_solve_link(
    doc: &Arc<Document>,
    comp: Uuid,
    camera: Uuid,
    store: &dyn CameraSolveStore,
) -> Option<Op> {
    use crate::model::TransformProp;

    let c = doc.comp(comp)?;
    let layer = c.layers.iter().find(|l| l.id == camera)?;
    let LayerKind::Camera { solve_link, .. } = &layer.kind else {
        return None;
    };
    solve_link.as_ref()?;

    let rate = c.frame_rate;
    let first = rate.frame_at(layer.in_point);
    let last = rate.frame_at(layer.out_point);
    if last <= first {
        return None;
    }

    // Six transform tracks plus the zoom, filled in one walk of the frames so
    // the derived pose is asked for once per frame rather than seven times.
    let mut keys: [Vec<Keyframe>; 7] = Default::default();
    let mut near = (0.0, 0.0, 0.0);
    for n in first..last {
        let ct = rate.time_of_frame(n).ok()?;
        let mut pose = own_camera_pose_of(doc, c, layer, ct.0.to_f64(), store)?.pose;
        // The rows in the camera's own order, with no Orientation left on top.
        pose.rotation_deg = crate::camera::unoriented(
            (0.0, 0.0, 0.0),
            pose.rotation_deg,
            layer.transform.after_effects_order,
            near,
        );
        near = pose.rotation_deg;
        // Keyframe times are LAYER time, which is what every property is
        // evaluated at — the same subtraction `stored_camera_pose` makes.
        let time = ct.0.checked_sub(layer.start_offset.0).ok()?;
        for (slot, value) in keys.iter_mut().zip([
            pose.position.0,
            pose.position.1,
            pose.position.2,
            pose.rotation_deg.0,
            pose.rotation_deg.1,
            pose.rotation_deg.2,
            pose.zoom,
        ]) {
            slot.push(key(time, value));
        }
    }

    let props = [
        TransformProp::PositionX,
        TransformProp::PositionY,
        TransformProp::PositionZ,
        TransformProp::RotationX,
        TransformProp::RotationY,
        TransformProp::Rotation,
    ];
    let mut ops = Vec::with_capacity(props.len() + 2);
    // First, so the read-only refusal below does not stop the very edit that
    // ends the link.
    ops.push(Op::SetCameraSolveLink {
        comp,
        layer: camera,
        solve_link: None,
    });
    let mut track = keys.into_iter();
    for prop in props {
        ops.push(Op::SetTransformProperty {
            comp,
            layer: camera,
            prop,
            animation: Animation::Keyframed(track.next()?),
        });
    }
    ops.push(Op::SetCameraZoom {
        comp,
        layer: camera,
        animation: Animation::Keyframed(track.next()?),
    });
    // The baked rows hold the whole turn, so an Orientation would turn it twice.
    let orientation = [
        TransformProp::OrientationX,
        TransformProp::OrientationY,
        TransformProp::OrientationZ,
    ];
    for (prop, o) in orientation.into_iter().zip(&layer.transform.orientation) {
        if o.animation != Animation::Static(0.0) {
            ops.push(Op::SetTransformProperty {
                comp,
                layer: camera,
                prop,
                animation: Animation::Static(0.0),
            });
        }
    }
    Some(Op::Batch { ops })
}

/// One baked key: linear on both sides, because the samples are one per frame
/// and there is nothing between two of them to shape.
fn key(time: Rational, value: f64) -> Keyframe {
    Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

// ---------------------------------------------------------------------------
// The planar track, and the Corner pin it writes
// ---------------------------------------------------------------------------

/// The match name of the Planar track effect (docs/08 §3.87). One
/// spelling, here, for [`CAMERA_TRACK`]'s reason.
pub const PLANAR_TRACK: &str = "planar_track";

/// The parameter id of the Planar track's **Pin layer** row — which layer both
/// of its export gestures write to. Spelled once because the effect declares it
/// and the gestures read it, and a typo would be a button that quietly refused.
pub const PIN_LAYER_PARAM: &str = "pin_layer";

/// The eight parameter ids of a Corner pin's four points, in [`Quad`] order:
/// upper left, upper right, lower left, lower right, each `x` then `y`
/// (docs/08 §3.48).
const CORNER_PIN_POINTS: [(&str, &str); 4] = [
    ("upper_left_x", "upper_left_y"),
    ("upper_right_x", "upper_right_y"),
    ("lower_left_x", "lower_left_y"),
    ("lower_right_x", "lower_right_y"),
];

/// The four corners of a tracked surface, in the order a Corner pin declares
/// them: upper left, upper right, lower left, lower right.
///
/// The same shape `lumit_track::planar::Quad` is, spelled again here because
/// `lumit-core` may not depend on the tracker (docs/05: the crate graph runs the
/// other way) and four pairs of numbers are not worth a shared crate.
pub type Quad = [[f64; 2]; 4];

/// Where the planar tracks live, as far as the model is concerned — the shape
/// [`CameraSolveStore`] is, for the same reason and with the same two methods.
///
/// **Filed by the effect instance, not by the media.** A camera solve describes
/// a *file*: two layers cutting the same shot share one, and that is right. A
/// planar track describes the quad somebody drew, so two Planar tracks on one
/// clip are two different answers and neither is the other's. The effect
/// instance is the only thing in the document that names one quad.
///
/// **The corners are in composition pixels**, which for the ordinary case — a
/// comp made from the shot — are the source raster's own, exactly as
/// docs/impl/tracking.md §5b's second deviation reads a camera solve's world.
pub trait PlanarTrackStore {
    /// What has been tracked under `track`, or `None` when nothing has.
    fn planar_range(&self, track: Uuid) -> Option<SolvedRange>;

    /// The tracked quad at source frame `frame`. `None` outside the range,
    /// which callers here never ask for: they clamp first, because clamping
    /// *is* the hold.
    fn planar_corners(&self, track: Uuid, frame: i64) -> Option<Quad>;
}

/// Whether `layer` carries an enabled Planar track.
#[must_use]
pub fn wears_planar_track(layer: &Layer) -> bool {
    layer
        .effects
        .iter()
        .any(|e| e.enabled && e.effect.match_name == PLANAR_TRACK)
}

/// The enabled Planar track instance on `layer`, if it has one — the first, in
/// stack order, so the answer never depends on the playhead.
#[must_use]
pub fn planar_track_effect(layer: &Layer) -> Option<&crate::model::EffectInstance> {
    layer
        .effects
        .iter()
        .find(|e| e.enabled && e.effect.match_name == PLANAR_TRACK)
}

/// The source moment a tracked layer shows at comp time `t`, and which source
/// it is — the camera link's own walk, without a store to ask what was solved.
///
/// Public because the planar track needs the same mapping for a different
/// question, and two walks over the same time chain would be two chances to
/// disagree about which moment is on screen.
#[must_use]
pub fn tracked_source_time(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
) -> Option<(Uuid, f64)> {
    tracked_source_at(doc, comp, tracked, t, 0)
}

/// **Create corner pin**: a Corner pin on `target`, its four points
/// keyframed to the surface `tracked` followed.
///
/// One key per composition frame of the target layer's own extent, at the
/// composition's rate, in px@comp — the same shape [`bake_solve_link`] writes,
/// and for the same reason: what the tracker measured is one number per frame,
/// and pretending otherwise would mean inventing an interpolation nobody asked
/// for. The keys are real, editable, and the graph editor shows them like any
/// others.
///
/// The moment each key reads is found by [`tracked_source_time`], so a trimmed
/// clip, a speed ramp, a reordered Sequence layer and a precomp all come out
/// right for free — the track was of the *source*, once. A comp frame
/// the track does not reach clamps into its range, which is the same hold a
/// linked camera performs.
///
/// The pin is **appended** to `target`'s stack rather than replacing anything:
/// a corner pin is a warp and warps belong last, and a layer that already has
/// one keeps it — the user asked for a pin, not for a tidy-up.
///
/// `None` when the target is gone, when nothing has been tracked under
/// `effect`, or when the target layer has no frames to key.
#[must_use]
pub fn corner_pin_from_track(
    doc: &Document,
    comp: Uuid,
    tracked: Uuid,
    effect: Uuid,
    target: Uuid,
    store: &dyn PlanarTrackStore,
) -> Option<Op> {
    let c = doc.comp(comp)?;
    let layer = c.layers.iter().find(|l| l.id == target)?;
    let samples = tracked_quads(doc, c, tracked, effect, layer, store)?;

    // Eight tracks filled in one walk of the samples, so the quad is unpacked
    // once per frame rather than eight times.
    let mut keys: [Vec<Keyframe>; 8] = Default::default();
    for (time, quad) in samples {
        for (slot, value) in keys
            .iter_mut()
            .zip(quad.into_iter().flat_map(|corner| [corner[0], corner[1]]))
        {
            slot.push(key(time, value));
        }
    }

    let mut pin = crate::fx::builtins::instantiate("corner_pin")?;
    let mut track = keys.into_iter();
    for (x, y) in CORNER_PIN_POINTS {
        for id in [x, y] {
            let animation = Animation::Keyframed(track.next()?);
            let param = pin.params.iter_mut().find(|p| p.id == id)?;
            param.value = crate::model::EffectValue::Float(crate::anim::Property {
                animation,
                extra: serde_json::Map::new(),
            });
        }
    }

    let mut effects = layer.effects.clone();
    effects.push(pin);
    Some(Op::SetLayerEffects {
        comp,
        layer: target,
        effects,
    })
}

/// The tracked quad at every composition frame of `layer`'s own extent, each
/// paired with the **layer** time a key on `layer` would carry it at.
///
/// One walk, shared by the two gestures below, because they differ only in what
/// they do with the corners: which source moment a frame reads, how a frame
/// outside the track clamps into it, and which time a key lands at are one
/// answer, and two copies of it would be two chances to drift apart.
///
/// `None` when nothing has been tracked under `effect`, when the layer holds no
/// frames, or when no frame of it could be read.
fn tracked_quads(
    doc: &Document,
    c: &Composition,
    tracked: Uuid,
    effect: Uuid,
    layer: &Layer,
    store: &dyn PlanarTrackStore,
) -> Option<Vec<(Rational, Quad)>> {
    let range = store.planar_range(effect)?;
    if !range.fps.is_finite() || range.fps <= 0.0 || range.last_frame < range.first_frame {
        return None;
    }

    let rate = c.frame_rate;
    let first = rate.frame_at(layer.in_point);
    let last = rate.frame_at(layer.out_point);
    if last <= first {
        return None;
    }

    let mut out = Vec::with_capacity(usize::try_from(last - first).unwrap_or(0));
    for n in first..last {
        let Ok(ct) = rate.time_of_frame(n) else {
            continue;
        };
        let t = ct.0.to_f64();
        let Some((_, st)) = tracked_source_time(doc, c, tracked, t) else {
            continue;
        };
        if !st.is_finite() {
            continue;
        }
        let asked = (st * range.fps).round();
        #[allow(clippy::cast_possible_truncation)]
        let asked = asked.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
        let Some(quad) =
            store.planar_corners(effect, asked.clamp(range.first_frame, range.last_frame))
        else {
            continue;
        };
        // Keyframe times are **layer** time — the target layer's, since these
        // keys live on the target layer — which is what every property is
        // evaluated at.
        let Ok(time) = ct.0.checked_sub(layer.start_offset.0) else {
            continue;
        };
        out.push((time, quad));
    }
    (!out.is_empty()).then_some(out)
}

/// The parameter id of the Planar track's **Follow** row — what the analysis
/// follows, and therefore what [`transform_from_track`] can write. Spelled once
/// because the effect declares it and both the job and the gesture read it, and
/// a typo would be a button that quietly wrote the wrong half.
pub const FOLLOW_PARAM: &str = "follow";

/// One quad read as a *pose*: where its centre is, which way it lies, and how
/// big it is.
///
/// The angle and the size come from the two horizontal edges — upper left to
/// upper right, lower left to lower right — because a projective warp treats
/// opposite edges differently and the pair together is the honest middle. The
/// two edge vectors are **summed and one `atan2` taken of the sum**, rather than
/// two angles averaged: adding vectors cannot wrap round ±180° and averaging
/// angles can, which would be one wrong frame in the middle of an otherwise
/// clean spin.
///
/// The angle is measured in composition space, where y runs down, so a positive
/// angle turns clockwise on screen — which is exactly what a layer's Rotation
/// means (`Mat4::from_rotation_z`, the same y-down space). No sign flip, and so
/// no place for one to be forgotten.
fn quad_pose(q: Quad) -> ([f64; 2], f64, f64) {
    let centre = [
        (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
        (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
    ];
    let top = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
    let bottom = [q[3][0] - q[2][0], q[3][1] - q[2][1]];
    let angle = (top[1] + bottom[1]).atan2(top[0] + bottom[0]);
    let span = (top[0].hypot(top[1]) + bottom[0].hypot(bottom[1])) / 2.0;
    (centre, angle, span)
}

/// **Create transform keys**: the movement `tracked` measured, written
/// onto `target`'s own Position — and, when `scale_and_rotation`, its Rotation
/// and Scale as well.
///
/// **The same four corners the corner pin uses.** Where their centre went is a
/// position, how far the quad turned is a rotation, how much it grew is a scale;
/// one analysis carries all three, so there is no second kind of track to run and
/// no second answer that could disagree with the first. Asking for *position
/// alone* is not a cheaper measurement, it is a narrower use of the one that was
/// made — which is the honest reading of "one point, or two": a small, plain
/// patch gives a position worth having long before it gives a rotation worth
/// having.
///
/// **Added, not stamped.** Each key is the property's own value at that moment
/// plus what the track moved by since its reference frame — times it, for scale —
/// so the layer keeps where the user put it and gains the movement, and a layer
/// that was already animated keeps that animation underneath. Stamping the
/// tracked centre absolutely would teleport the layer onto the tracked feature,
/// which is never what the gesture is for.
///
/// One key per composition frame of `target`'s own extent, linear both sides —
/// [`corner_pin_from_track`]'s shape exactly, through the same walk, so a trimmed
/// clip, a speed ramp, a reordered Sequence layer and a precomp all come out
/// right for free.
///
/// `None` when the target is gone, when nothing has been tracked under `effect`,
/// when the target holds no frames, or when the reference quad has no size for a
/// scale to be measured against.
#[must_use]
pub fn transform_from_track(
    doc: &Document,
    comp: Uuid,
    tracked: Uuid,
    effect: Uuid,
    target: Uuid,
    scale_and_rotation: bool,
    store: &dyn PlanarTrackStore,
) -> Option<Op> {
    use crate::model::TransformProp;

    let c = doc.comp(comp)?;
    let layer = c.layers.iter().find(|l| l.id == target)?;
    let samples = tracked_quads(doc, c, tracked, effect, layer, store)?;

    // The reference pose is the *track's* first frame, not the first frame this
    // layer happens to show: it is the shape the quad was drawn as, and every
    // delta below is measured from it.
    let range = store.planar_range(effect)?;
    let reference = store.planar_corners(effect, range.first_frame)?;
    let (origin, origin_angle, origin_span) = quad_pose(reference);
    if !origin_span.is_finite() || origin_span <= 0.0 {
        return None;
    }

    // Position x, position y, rotation, scale x, scale y — filled in one walk so
    // the quad is read once per frame rather than five times.
    let mut keys: [Vec<Keyframe>; 5] = Default::default();
    // The previous frame's *unwrapped* turn, so a spin past ±180° carries on
    // counting instead of snapping back the way `atan2` would.
    let mut turned = 0.0f64;
    for (time, quad) in samples {
        let (centre, angle, span) = quad_pose(quad);
        let t = time.to_f64();
        let raw = angle - origin_angle;
        let tau = std::f64::consts::TAU;
        turned = raw + ((turned - raw) / tau).round() * tau;
        let base = |prop| layer.transform.get(prop).map_or(0.0, |p| p.value_at(t));
        let scale = span / origin_span;
        for (slot, value) in keys.iter_mut().zip([
            base(TransformProp::PositionX) + (centre[0] - origin[0]),
            base(TransformProp::PositionY) + (centre[1] - origin[1]),
            base(TransformProp::Rotation) + turned.to_degrees(),
            base(TransformProp::ScaleX) * scale,
            base(TransformProp::ScaleY) * scale,
        ]) {
            slot.push(key(time, value));
        }
    }

    // Position always; the other three only when asked for, because writing a
    // rotation of nought over one the user animated would be the button quietly
    // deleting their work.
    let props = [
        TransformProp::PositionX,
        TransformProp::PositionY,
        TransformProp::Rotation,
        TransformProp::ScaleX,
        TransformProp::ScaleY,
    ];
    let wanted = if scale_and_rotation { props.len() } else { 2 };
    let ops = props
        .into_iter()
        .zip(keys)
        .take(wanted)
        .map(|(prop, channel)| Op::SetTransformProperty {
            comp,
            layer: target,
            prop,
            animation: Animation::Keyframed(channel),
        })
        .collect();
    Some(Op::Batch { ops })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

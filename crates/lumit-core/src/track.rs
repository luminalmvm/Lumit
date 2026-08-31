//! The solve link: a Camera layer driven by a tracked layer's camera solve
//! (K-417, docs/03-DATA-MODEL.md §5.6, docs/impl/tracking.md phase 4).
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
//! out right for free — the tracker ran on the file, once, exactly as K-248
//! says it must.
//!
//! **Track once, then nudge** (K-578). A solve is a measurement, and a
//! measurement can be a little wrong — the ground plane tilts, the shot drifts,
//! the whole move wants to sit half a metre to the left. So a linked camera's
//! own transform rows are not read-only: they are a **correction lane**. What
//! they hold over and above the pose they held when the link was made is added,
//! channel by channel, to the solved pose. Drag the camera and you have keyed a
//! correction; re-analyse the shot and the correction rides on top of the new
//! solve, because it was never part of it.
//!
//! Two honest failures, and neither is silent (K-417):
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

use uuid::Uuid;

use crate::anim::{Animation, Keyframe, SideInterp};
use crate::model::{stored_camera_pose, CameraPose, Composition, Document, Layer, LayerKind};
use crate::ops::Op;
use crate::sequence::ClipSource;
use crate::time::Rational;

/// The match name of the Camera track effect — what marks a layer as *the*
/// tracked layer inside a precomp (K-417's parent-comp ruling).
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
/// Turning `lumit_track::CameraSolve`'s world-to-camera rotations, its solve
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

/// How a Camera layer's placement was arrived at — the flag K-417 requires the
/// interface to be able to read, so a link that stopped working says so.
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
/// where it has one. `None` → the comp renders flat, exactly as
/// [`Composition::camera_pose`] means it.
///
/// This is the reading the render path wants once solves exist; today's callers
/// use `camera_pose`, which is this with no store and therefore
/// [`LinkState::Unresolved`] on any linked camera — the correct degrade, and
/// what stage 2 replaces by threading the real store.
#[must_use]
pub fn camera_pose_at(
    doc: &Document,
    comp: &Composition,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let layer = comp.active_camera(t)?;
    camera_pose_of(doc, comp, layer, t, store)
}

/// One named Camera layer's placement at comp time `t`, whether or not it is
/// the active one — what the bake walks, frame by frame.
#[must_use]
pub fn camera_pose_of(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let stored = stored_camera_pose(layer, t)?;
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

/// The solved pose with the layer's own correction on it (K-578).
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
        return solved;
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
    }
}

/// Whether `layer`'s correction lane holds anything — **edited since track**
/// (K-578), which is what the dot beside the link says.
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

/// **Clear corrections** (K-578): put every property of the correction lane back
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
/// pose, and the interface asks it which frame of the point cloud to draw
/// (K-417's overlay). Two readings of the same moment cannot disagree if there
/// is only one walk.
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
/// effect on it, which is what makes K-417's parent-comp workflow resolve — a
/// linked camera in the parent comp names the precomp layer, and the chain
/// finds the footage inside.
///
/// **Unless the Precomp layer wears the effect itself**, in which case the walk
/// stops there and the nested comp *is* the tracked source. K-417 allows a
/// Camera track on a Precomp layer, and what that asks to track is the picture
/// the nested comp makes — a comp of stills panned by a camera move has no
/// footage inside it to follow, and there is nothing further down to descend to.
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

/// Whether `layer` carries an enabled Camera track — K-417's definition that
/// the effect *is* the handle, spelled once because three questions ask it.
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
/// layer, by K-417's definition that the effect *is* the handle. The first one,
/// in stack order, so the answer never depends on the playhead; `None` when the
/// comp has none.
#[must_use]
pub fn tracked_layer(comp: &Composition) -> Option<Uuid> {
    comp.layers
        .iter()
        .find(|l| wears_camera_track(l))
        .map(|l| l.id)
}

/// **Convert to keyframes** (K-417): bake the derived motion into ordinary
/// keyframes and sever the link, as one undoable step.
///
/// One key per composition frame across the camera layer's own span, linear on
/// both sides, on the six transform properties and on the camera's zoom — real,
/// editable keyframes the graph editor draws like any others, which is the
/// bargain the ruling makes for being honest that there are a lot of them.
///
/// **The link is cleared first**, inside the batch, and it still is now that a
/// linked camera's transform takes edits (K-578): the numbers written after it
/// are the *corrected* pose, absolute, and they would be read as a correction if
/// the link were still on. Clearing first also drops the correction base, so
/// nothing is left over to be added twice. Undo reverses the members — the
/// transforms go back, then the link, which re-derives the base it had.
///
/// Deterministic: the frames come from the comp's own rate, the poses from the
/// same [`camera_pose_of`] the render reads, and a frame the link cannot
/// resolve bakes the same fallback the render would have shown. `None` when
/// `camera` is not a Camera layer in `comp`, when it carries no link, or when
/// its span holds no frames.
#[must_use]
pub fn bake_solve_link(
    doc: &Document,
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
    for n in first..last {
        let ct = rate.time_of_frame(n).ok()?;
        let pose = camera_pose_of(doc, c, layer, ct.0.to_f64(), store)?.pose;
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
// The planar track, and the Corner pin it writes (K-579)
// ---------------------------------------------------------------------------

/// The match name of the Planar track effect (docs/08 §3.87, K-579). One
/// spelling, here, for [`CAMERA_TRACK`]'s reason.
pub const PLANAR_TRACK: &str = "planar_track";

/// The parameter id of the Planar track's **Pin layer** row — which layer the
/// corner-pin gesture writes to. Spelled once because the effect declares it and
/// the gesture reads it, and a typo would be a button that quietly refused.
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
/// The same shape `lumit_track::Quad` is, spelled again here because
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

/// **Create corner pin** (K-579): a Corner pin on `target`, its four points
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
/// right for free — the track was of the *source*, once (K-248). A comp frame
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

    // Eight tracks filled in one walk of the frames, so the quad is asked for
    // once per frame rather than eight times.
    let mut keys: [Vec<Keyframe>; 8] = Default::default();
    let mut wrote = 0usize;
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
        // keys live on the target layer's effect — which is what every property
        // is evaluated at.
        let Ok(time) = ct.0.checked_sub(layer.start_offset.0) else {
            continue;
        };
        for (slot, value) in keys
            .iter_mut()
            .zip(quad.into_iter().flat_map(|corner| [corner[0], corner[1]]))
        {
            slot.push(key(time, value));
        }
        wrote += 1;
    }
    if wrote == 0 {
        return None;
    }

    let mut pin = crate::fx::instantiate("corner_pin")?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::Property;
    use crate::model::{
        BlendMode, CameraPose, Composition, LinearColour, ProjectItem, Switches, TransformGroup,
        TransformProp,
    };
    use crate::ops::apply;
    use crate::sequence::Clip;
    use crate::time::{CompTime, Duration, FrameRate};

    /// A written-down solve standing in for a real one: sixty frames at 30 fps,
    /// with the pose a plain function of the frame so a test can say what it
    /// expects without carrying a table.
    ///
    /// The point of stage 1 is that this is *all* the model needs, and it
    /// arrives through the same trait the sidecar will.
    struct Synthetic {
        media: Uuid,
    }

    impl Synthetic {
        const FPS: f64 = 30.0;
        const FIRST: i64 = 0;
        const LAST: i64 = 59;

        fn new(media: Uuid) -> Self {
            Self { media }
        }

        /// The pose at solved frame `n` — distinct in every field per frame, so
        /// a test that lands on the wrong frame cannot accidentally pass.
        fn pose(n: i64) -> CameraPose {
            let f = n as f64;
            CameraPose {
                zoom: 1000.0 + f,
                position: (f, f * 2.0, f * 3.0),
                rotation_deg: (f * 0.5, f * 0.25, f * 0.125),
            }
        }
    }

    impl CameraSolveStore for Synthetic {
        fn solved_range(&self, media: Uuid) -> Option<SolvedRange> {
            (media == self.media).then_some(SolvedRange {
                fps: Self::FPS,
                first_frame: Self::FIRST,
                last_frame: Self::LAST,
            })
        }

        fn solved_pose(&self, media: Uuid, frame: i64) -> Option<CameraPose> {
            (media == self.media && (Self::FIRST..=Self::LAST).contains(&frame))
                .then(|| Synthetic::pose(frame))
        }
    }

    /// A store with nothing in it — never analysed, or the media offline.
    struct Empty;

    impl CameraSolveStore for Empty {
        fn solved_range(&self, _: Uuid) -> Option<SolvedRange> {
            None
        }
        fn solved_pose(&self, _: Uuid, _: i64) -> Option<CameraPose> {
            None
        }
    }

    fn secs(n: i64) -> CompTime {
        CompTime(Rational::new(n, 1).unwrap())
    }

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    /// A Retime map from `(layer time, source time)` pairs, linear between them.
    fn retime(points: &[(i64, f64)]) -> Property {
        Property {
            animation: Animation::Keyframed(
                points
                    .iter()
                    .map(|(t, v)| super::key(rat(*t, 30), *v))
                    .collect(),
            ),
            extra: serde_json::Map::new(),
        }
    }

    fn layer(name: &str, kind: LayerKind, out_s: i64) -> Layer {
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: name.into(),
            kind,
            in_point: secs(0),
            out_point: secs(out_s),
            start_offset: secs(0),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            pan: Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// A tracked layer is one carrying the Camera track effect — that is the
    /// whole of what marks it (K-417).
    fn tracked(mut l: Layer) -> Layer {
        l.effects
            .push(crate::fx::instantiate(CAMERA_TRACK).expect("the effect is registered"));
        l
    }

    fn camera(link: Option<Uuid>) -> Layer {
        layer(
            "Camera 1",
            LayerKind::Camera {
                zoom: Property::fixed(777.0),
                solve_link: link,
                correction_base: None,
            },
            4,
        )
    }

    fn comp(name: &str, layers: Vec<Layer>) -> Composition {
        Composition {
            master_volume_db: 0.0,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: name.into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(rat(4, 1)),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn document(comps: Vec<Composition>) -> Document {
        let mut doc = Document::new();
        for c in comps {
            doc.items.push(ProjectItem::Composition(c));
        }
        doc
    }

    /// The plain case: a footage layer at source rate, so comp frame `n` is
    /// solved frame `n` and the camera wears the solve exactly.
    #[test]
    fn a_link_through_a_plain_clip_reads_the_frame_under_the_playhead() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let doc = document(vec![c.clone()]);
        let store = Synthetic::new(media);

        for n in 0..60 {
            let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
            assert_eq!(got.state, LinkState::Derived, "frame {n}");
            assert_eq!(got.pose, Synthetic::pose(i64::from(n)), "frame {n}");
        }
    }

    /// A retimed clip, freeze included. The mapping is the layer's own Retime —
    /// the same one the renderer decodes through — so the camera sees exactly
    /// the frame the picture shows.
    #[test]
    fn a_link_through_a_retimed_clip_follows_the_retime() {
        let media = Uuid::now_v7();
        let mut footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        // Half speed for the first second, then frozen on source 0.5 s.
        footage.retime = Some(retime(&[(0, 0.0), (30, 0.5), (60, 0.5)]));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let doc = document(vec![c.clone()]);
        let store = Synthetic::new(media);

        // Half speed: comp frame 10 is 1/3 s, source 1/6 s, solved frame 5.
        let got = camera_pose_at(&doc, &c, 10.0 / 30.0, &store).unwrap();
        assert_eq!(got.state, LinkState::Derived);
        assert_eq!(got.pose, Synthetic::pose(5));

        // The freeze: every moment of the second half reads source 0.5 s,
        // solved frame 15 — and it is a *derived* read, not a held one. The
        // solve has that frame; the retime simply keeps asking for it.
        for n in [31, 40, 59] {
            let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
            assert_eq!(got.state, LinkState::Derived, "frame {n}");
            assert_eq!(got.pose, Synthetic::pose(15), "frame {n}");
        }
    }

    /// K-248 made testable: the tracker ran on the file, so reordering the cuts
    /// changes which solved frame is on screen and nothing else. The same source
    /// moment gives the same pose whichever order the clips sit in.
    #[test]
    fn reordering_a_sequence_layer_moves_the_pose_with_the_source_frame() {
        let media = Uuid::now_v7();
        let source = ClipSource::Footage(media);
        // Two half-second clips: one showing source 0.0–0.5, one showing
        // 1.0–1.5. `ab` plays them in that order, `ba` the other way round.
        let clip = |source_in: i64, place: i64| {
            Clip::new(
                source,
                rat(source_in, 30),
                rat(source_in + 15, 30),
                rat(place, 30),
                rat(15, 30),
            )
        };
        let ab = vec![clip(0, 0), clip(30, 15)];
        let ba = vec![clip(30, 0), clip(0, 15)];

        for (clips, first_half, second_half) in [(ab, 0, 30), (ba, 30, 0)] {
            let seq = tracked(layer("cut", LayerKind::Sequence { clips }, 1));
            let c = comp("main", vec![camera(Some(seq.id)), seq]);
            let doc = document(vec![c.clone()]);
            let store = Synthetic::new(media);

            for n in 0..15 {
                let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
                assert_eq!(got.pose, Synthetic::pose(first_half + i64::from(n)));
            }
            for n in 15..30 {
                let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
                assert_eq!(got.pose, Synthetic::pose(second_half + i64::from(n) - 15));
            }
        }
    }

    /// K-417's parent-comp ruling: a linked camera in the parent comp names the
    /// **precomp layer**, and the chain resolves through it to the tracked layer
    /// inside — the precomp layer's own Retime included.
    #[test]
    fn a_link_resolves_through_a_precomp_to_the_tracked_layer_inside() {
        let media = Uuid::now_v7();
        let inner = comp(
            "inner",
            vec![tracked(layer(
                "shot",
                LayerKind::Footage { item: media },
                2,
            ))],
        );

        let mut nested = layer("inner", LayerKind::Precomp { comp: inner.id }, 2);
        // The precomp layer plays at half speed, so comp frame 20 shows the
        // nested comp's 1/3 s, which is the tracked layer's solved frame 10.
        nested.retime = Some(retime(&[(0, 0.0), (60, 1.0)]));
        let outer = comp("outer", vec![camera(Some(nested.id)), nested]);
        let doc = document(vec![inner, outer.clone()]);
        let store = Synthetic::new(media);

        let got = camera_pose_at(&doc, &outer, 20.0 / 30.0, &store).unwrap();
        assert_eq!(got.state, LinkState::Derived);
        assert_eq!(got.pose, Synthetic::pose(10));

        // A precomp with no tracked layer in it has nothing to resolve to, and
        // says so rather than guessing at the first footage it finds.
        let bare = comp(
            "bare",
            vec![layer("plain", LayerKind::Footage { item: media }, 2)],
        );
        let lone = layer("bare", LayerKind::Precomp { comp: bare.id }, 2);
        let outer = comp("outer", vec![camera(Some(lone.id)), lone]);
        let doc = document(vec![bare, outer.clone()]);
        assert_eq!(
            camera_pose_at(&doc, &outer, 0.0, &store).unwrap().state,
            LinkState::Unresolved
        );
    }

    /// A Camera track **on the precomp layer itself** stops the walk there: the
    /// nested comp is the tracked source, and its solve is filed under the
    /// comp's own id. That is the case a comp of stills panned by a camera move
    /// needs — there is no footage inside it to descend to — and it must not be
    /// confused with the parent-comp workflow above, which is the same document
    /// shape without the effect on the precomp layer.
    #[test]
    fn a_camera_track_on_the_precomp_layer_tracks_the_nested_comp_itself() {
        let inner_media = Uuid::now_v7();
        let inner = comp(
            "inner",
            vec![tracked(layer(
                "shot",
                LayerKind::Footage { item: inner_media },
                2,
            ))],
        );
        // The store knows the *comp*, and knows nothing of the footage inside
        // it: a walk that descended would resolve to nothing at all.
        let store = Synthetic::new(inner.id);

        let nested = tracked(layer("inner", LayerKind::Precomp { comp: inner.id }, 2));
        assert_eq!(tracked_source_id(&nested), Some(inner.id));
        let outer = comp("outer", vec![camera(Some(nested.id)), nested]);
        let doc = document(vec![inner, outer.clone()]);

        let got = camera_pose_at(&doc, &outer, 10.0 / 30.0, &store).unwrap();
        assert_eq!(got.state, LinkState::Derived);
        assert_eq!(got.pose, Synthetic::pose(10));

        // And without the effect on it the same document descends, as it always
        // did — the two workflows are told apart by the effect and nothing else.
        let inner2 = comp(
            "inner",
            vec![tracked(layer(
                "shot",
                LayerKind::Footage { item: inner_media },
                2,
            ))],
        );
        let plain = layer("inner", LayerKind::Precomp { comp: inner2.id }, 2);
        let outer2 = comp("outer", vec![camera(Some(plain.id)), plain]);
        let doc2 = document(vec![inner2, outer2.clone()]);
        assert_eq!(
            camera_pose_at(&doc2, &outer2, 10.0 / 30.0, &store)
                .unwrap()
                .state,
            LinkState::Unresolved,
            "the walk stopped at the precomp although nothing asked it to"
        );
        assert_eq!(
            camera_pose_at(&doc2, &outer2, 10.0 / 30.0, &Synthetic::new(inner_media))
                .unwrap()
                .pose,
            Synthetic::pose(10),
            "without the effect the walk must still reach the footage inside"
        );
    }

    /// The two honest failures, and neither is silent: past the end of the solve
    /// the last derived motion **holds**, and a link that cannot be followed at
    /// all falls back to the camera's own properties and says which it is.
    #[test]
    fn an_unresolvable_link_holds_and_says_which_kind_of_unresolvable() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
        let footage_id = footage.id;
        let c = comp("main", vec![camera(Some(footage_id)), footage]);
        let doc = document(vec![c.clone()]);

        // Three seconds in is past the sixty solved frames: the last one holds.
        let got = camera_pose_at(&doc, &c, 3.0, &Synthetic::new(media)).unwrap();
        assert_eq!(got.state, LinkState::Held);
        assert_eq!(got.pose, Synthetic::pose(59));

        // Nothing solved at all: the document's own numbers, flagged.
        let got = camera_pose_at(&doc, &c, 0.0, &Empty).unwrap();
        assert_eq!(got.state, LinkState::Unresolved);
        assert_eq!(got.pose.zoom, 777.0);

        // The linked layer deleted: the same answer, for the same reason.
        let mut orphan = c.clone();
        orphan.layers.retain(|l| l.id != footage_id);
        let doc = document(vec![orphan.clone()]);
        assert_eq!(
            camera_pose_at(&doc, &orphan, 0.0, &Synthetic::new(media))
                .unwrap()
                .state,
            LinkState::Unresolved
        );

        // A camera with no link is none of those.
        let plain = comp("plain", vec![camera(None)]);
        assert_eq!(
            camera_pose_at(&document(vec![plain.clone()]), &plain, 0.0, &Empty)
                .unwrap()
                .state,
            LinkState::Unlinked
        );
    }

    /// **Track once, then nudge** (K-578): a linked camera takes a transform
    /// edit, and what it takes is a *correction* — added to the solved pose,
    /// channel by channel, wherever the link resolves.
    #[test]
    fn a_correction_rides_on_top_of_the_solve() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);
        let store = Synthetic::new(media);

        // The camera was built already linked, so the base it would have been
        // given by `SetCameraSolveLink` is written here the same way the bridge
        // writes it for a camera that arrives linked.
        set_base(&mut doc, comp_id, cam_id);
        let base = base_of(&doc, comp_id, cam_id);
        assert!(
            !has_correction(layer_of(&doc, comp_id, cam_id)),
            "a camera nobody has touched carries no correction"
        );

        // Uncorrected, the derived pose is the solve exactly.
        let at = |doc: &Document, n: i64| {
            let c = doc.comp(comp_id).unwrap();
            camera_pose_at(doc, c, n as f64 / 30.0, &store).unwrap()
        };
        assert_eq!(at(&doc, 10).pose, Synthetic::pose(10));

        // Two nudges, in the two shapes an edit arrives in: a plain drag, and a
        // keyed one. The zoom is corrected too, because a solved focal is as
        // capable of being a little wrong as a solved position.
        for (prop, value) in [
            (TransformProp::PositionX, base.position.0 + 40.0),
            (TransformProp::RotationY, base.rotation_deg.1 - 2.5),
        ] {
            apply(
                &mut doc,
                &Op::SetTransformProperty {
                    comp: comp_id,
                    layer: cam_id,
                    prop,
                    animation: Animation::Static(value),
                },
            )
            .expect("a linked camera takes a transform edit");
        }
        apply(
            &mut doc,
            &Op::SetCameraZoom {
                comp: comp_id,
                layer: cam_id,
                animation: Animation::Static(base.zoom + 7.0),
            },
        )
        .expect("and a zoom edit");
        assert!(has_correction(layer_of(&doc, comp_id, cam_id)));

        // Derived: the solve, plus the difference, and nothing else moved.
        for n in [0, 10, 59] {
            let got = at(&doc, n);
            assert_eq!(got.state, LinkState::Derived, "frame {n}");
            let want = Synthetic::pose(n);
            assert_eq!(
                got.pose,
                CameraPose {
                    zoom: want.zoom + 7.0,
                    position: (want.position.0 + 40.0, want.position.1, want.position.2),
                    rotation_deg: (
                        want.rotation_deg.0,
                        want.rotation_deg.1 - 2.5,
                        want.rotation_deg.2
                    ),
                },
                "frame {n}"
            );
        }

        // Held: past the solved range the correction rides on the held pose,
        // not on nothing — the last derived motion, corrected.
        let held = at(&doc, 90);
        assert_eq!(held.state, LinkState::Held);
        assert_eq!(held.pose.position.0, Synthetic::pose(59).position.0 + 40.0);

        // Unresolved: nothing is being corrected, so the numbers are read as a
        // pose. The camera keeps drawing from where it was put.
        let c = doc.comp(comp_id).unwrap();
        let lost = camera_pose_at(&doc, c, 0.0, &Empty).unwrap();
        assert_eq!(lost.state, LinkState::Unresolved);
        assert_eq!(lost.pose.position.0, base.position.0 + 40.0);
        assert_eq!(lost.pose.zoom, base.zoom + 7.0);
    }

    /// A keyed correction is an ordinary keyframed property, and it is added at
    /// the value it has on each frame — so a correction can ramp in.
    #[test]
    fn a_keyed_correction_is_added_frame_by_frame() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);
        set_base(&mut doc, comp_id, cam_id);
        let base = base_of(&doc, comp_id, cam_id);
        let store = Synthetic::new(media);

        // Nought at comp frame 0, thirty at comp frame 30, linear between.
        apply(
            &mut doc,
            &Op::SetTransformProperty {
                comp: comp_id,
                layer: cam_id,
                prop: TransformProp::PositionY,
                animation: Animation::Keyframed(vec![
                    key(rat(0, 30), base.position.1),
                    key(rat(30, 30), base.position.1 + 30.0),
                ]),
            },
        )
        .expect("a keyed correction is an ordinary edit");

        for n in [0, 10, 30] {
            let c = doc.comp(comp_id).unwrap();
            let got = camera_pose_at(&doc, c, f64::from(n) / 30.0, &store).unwrap();
            assert!(
                (got.pose.position.1 - (Synthetic::pose(i64::from(n)).position.1 + f64::from(n)))
                    .abs()
                    < 1e-9,
                "frame {n}: {:?}",
                got.pose.position.1
            );
        }
    }

    /// **Clear corrections** takes the nudge back and leaves the track alone,
    /// in one undo step — and the dot goes out with it.
    #[test]
    fn clearing_corrections_keeps_the_link_and_undoes_in_one_step() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let footage_id = footage.id;
        let c = comp("main", vec![camera(Some(footage_id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);
        set_base(&mut doc, comp_id, cam_id);

        assert!(
            clear_corrections(&doc, comp_id, cam_id).is_none(),
            "there is nothing to clear on a camera nobody has nudged"
        );

        apply(
            &mut doc,
            &Op::SetTransformProperty {
                comp: comp_id,
                layer: cam_id,
                prop: TransformProp::PositionZ,
                animation: Animation::Static(120.0),
            },
        )
        .expect("nudged");
        let corrected = layer_of(&doc, comp_id, cam_id).clone();
        assert!(has_correction(&corrected));

        let clear = clear_corrections(&doc, comp_id, cam_id).expect("there is a lane to clear");
        let undo = apply(&mut doc, &clear).expect("clearing applies");
        let after = layer_of(&doc, comp_id, cam_id);
        assert!(!has_correction(after), "the lane is empty again");
        assert!(
            matches!(after.kind, LayerKind::Camera { solve_link: Some(l), .. } if l == footage_id),
            "clearing a correction must not clear the track"
        );

        apply(&mut doc, &undo).expect("the undo applies");
        assert_eq!(
            layer_of(&doc, comp_id, cam_id),
            &corrected,
            "one step puts the nudge back exactly"
        );
    }

    /// The base is captured when the link is made, kept when it is re-pointed,
    /// dropped when it is cleared — and undo puts back the one that was there.
    #[test]
    fn the_correction_base_follows_the_link() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let other = tracked(layer("shot 2", LayerKind::Footage { item: media }, 2));
        let (footage_id, other_id) = (footage.id, other.id);
        let mut cam = camera(None);
        cam.transform.position_x = Property::fixed(960.0);
        let c = comp("main", vec![cam, footage, other]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);

        assert!(
            base_opt(&doc, comp_id, cam_id).is_none(),
            "no link, no lane"
        );

        let link = |to: Option<Uuid>| Op::SetCameraSolveLink {
            comp: comp_id,
            layer: cam_id,
            solve_link: to,
        };
        let undo = apply(&mut doc, &link(Some(footage_id))).expect("linked");
        let base = base_of(&doc, comp_id, cam_id);
        assert_eq!(base.position.0, 960.0, "the pose it was linked at");
        assert!(!has_correction(layer_of(&doc, comp_id, cam_id)));

        // Nudge, then re-point: the nudge is the user's and rides on.
        apply(
            &mut doc,
            &Op::SetTransformProperty {
                comp: comp_id,
                layer: cam_id,
                prop: TransformProp::PositionX,
                animation: Animation::Static(1000.0),
            },
        )
        .expect("nudged");
        apply(&mut doc, &link(Some(other_id))).expect("re-pointed");
        assert_eq!(
            base_of(&doc, comp_id, cam_id).position.0,
            960.0,
            "re-pointing a link must not swallow the correction into the base"
        );
        assert!(has_correction(layer_of(&doc, comp_id, cam_id)));

        // Clearing the link ends the lane.
        apply(&mut doc, &link(None)).expect("unlinked");
        assert!(base_opt(&doc, comp_id, cam_id).is_none());
        assert!(!has_correction(layer_of(&doc, comp_id, cam_id)));

        // And undoing the very first link, from the document it was made in,
        // leaves no lane behind either.
        let mut fresh = doc.clone();
        apply(&mut fresh, &link(Some(footage_id))).expect("relinked");
        apply(&mut fresh, &undo).expect("undone");
        assert!(base_opt(&fresh, comp_id, cam_id).is_none());
    }

    fn layer_of(doc: &Document, comp: Uuid, layer: Uuid) -> &Layer {
        doc.comp(comp)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer)
            .unwrap()
    }

    fn base_opt(doc: &Document, comp: Uuid, layer: Uuid) -> Option<CameraPose> {
        match &layer_of(doc, comp, layer).kind {
            LayerKind::Camera {
                correction_base, ..
            } => correction_base.as_deref().copied(),
            _ => None,
        }
    }

    fn base_of(doc: &Document, comp: Uuid, layer: Uuid) -> CameraPose {
        base_opt(doc, comp, layer).expect("a linked camera has a base")
    }

    /// What `Op::SetCameraSolveLink` does for a camera that is linked by an
    /// edit, done by hand for the fixtures that are built linked.
    fn set_base(doc: &mut Document, comp: Uuid, layer: Uuid) {
        let l = doc
            .comp_mut(comp)
            .unwrap()
            .layers
            .iter_mut()
            .find(|l| l.id == layer)
            .unwrap();
        let pose = crate::model::stored_camera_pose_lt(l, 0.0);
        if let LayerKind::Camera {
            correction_base, ..
        } = &mut l.kind
        {
            *correction_base = pose.map(Box::new);
        }
    }

    /// **Convert to keyframes**: the baked keys reproduce the derived path
    /// exactly, at every comp frame of the layer's span, and the link is gone.
    /// One undo step puts both back.
    #[test]
    fn the_bake_reproduces_the_derived_path_and_undo_restores_the_link() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);
        set_base(&mut doc, comp_id, cam_id);
        let store = Synthetic::new(media);
        let before = doc.comp(comp_id).unwrap().layers[0].clone();

        // What the link says, frame by frame, before anything is baked. The
        // camera's span is 0..4 s at 30 fps, so 120 frames — sixty derived and
        // sixty held, which means the bake is checked on both readings.
        let want: Vec<CameraPose> = (0..120)
            .map(|n| {
                let c = doc.comp(comp_id).unwrap();
                camera_pose_at(&doc, c, f64::from(n) / 30.0, &store)
                    .unwrap()
                    .pose
            })
            .collect();
        // The comparison below is against the derived path itself, so it would
        // pass just as happily on a path that never resolved and never moved.
        // These two say the path under test is the real one: sixty distinct
        // solved poses, then sixty held on the last of them.
        assert_eq!(want[..60], (0..60).map(Synthetic::pose).collect::<Vec<_>>());
        assert!(want[60..].iter().all(|p| *p == Synthetic::pose(59)));

        let bake = bake_solve_link(&doc, comp_id, cam_id, &store).expect("there is a link to bake");
        assert!(
            matches!(&bake, Op::Batch { ops } if ops.len() == 8),
            "one batch: the link, six transform tracks and the zoom"
        );
        // One undo step, not a hundred and twenty.
        let undo = apply(&mut doc, &bake).expect("the bake applies");

        let baked = doc.comp(comp_id).unwrap();
        assert!(
            matches!(
                baked.layers[0].kind,
                LayerKind::Camera {
                    solve_link: None,
                    ..
                }
            ),
            "the bake severs the link"
        );
        for (n, expected) in want.iter().enumerate() {
            assert_eq!(
                baked.camera_pose(n as f64 / 30.0).as_ref(),
                Some(expected),
                "baked frame {n} does not match the derived path"
            );
        }
        // Undo restores the link — and restores it *last*, after the transforms
        // are home, so the read-only refusal does not trip on the way out
        // either. One step, and the layer is byte-for-byte what it was.
        apply(&mut doc, &undo).expect("the undo applies");
        assert_eq!(
            doc.comp(comp_id).unwrap().layers[0],
            before,
            "undo puts the link and the properties back exactly"
        );
    }

    /// The bake is deterministic, and refuses what it cannot do.
    #[test]
    fn the_bake_is_deterministic_and_refuses_an_unlinked_camera() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let doc = document(vec![c]);
        let store = Synthetic::new(media);

        let a = bake_solve_link(&doc, comp_id, cam_id, &store).unwrap();
        let b = bake_solve_link(&doc, comp_id, cam_id, &store).unwrap();
        assert_eq!(a, b, "two bakes of one document differ");

        let plain = comp("plain", vec![camera(None)]);
        let (plain_id, plain_cam) = (plain.id, plain.layers[0].id);
        assert!(
            bake_solve_link(&document(vec![plain]), plain_id, plain_cam, &store).is_none(),
            "there is nothing to bake without a link"
        );
    }

    // -----------------------------------------------------------------------
    // The planar track's corner pin (K-579)
    // -----------------------------------------------------------------------

    /// A written-down planar track: sixty frames at 30 fps, the quad a plain
    /// function of the frame so a test can say what it expects without carrying
    /// a table, and every corner distinct so landing on the wrong frame or the
    /// wrong corner cannot accidentally pass.
    struct SyntheticPlane {
        track: Uuid,
    }

    impl SyntheticPlane {
        const FPS: f64 = 30.0;
        const FIRST: i64 = 0;
        const LAST: i64 = 59;

        fn quad(n: i64) -> Quad {
            let f = n as f64;
            [
                [100.0 + f, 200.0 + f * 2.0],
                [300.0 + f * 3.0, 210.0 + f * 4.0],
                [110.0 + f * 5.0, 400.0 + f * 6.0],
                [320.0 + f * 7.0, 410.0 + f * 8.0],
            ]
        }
    }

    impl PlanarTrackStore for SyntheticPlane {
        fn planar_range(&self, track: Uuid) -> Option<SolvedRange> {
            (track == self.track).then_some(SolvedRange {
                fps: Self::FPS,
                first_frame: Self::FIRST,
                last_frame: Self::LAST,
            })
        }

        fn planar_corners(&self, track: Uuid, frame: i64) -> Option<Quad> {
            (track == self.track && (Self::FIRST..=Self::LAST).contains(&frame))
                .then(|| SyntheticPlane::quad(frame))
        }
    }

    /// A layer wearing the Planar track effect — the handle, exactly as the
    /// Camera track is one.
    fn planar(mut l: Layer) -> Layer {
        l.effects
            .push(crate::fx::instantiate(PLANAR_TRACK).expect("the effect is registered"));
        l
    }

    /// The keyframed value of one Corner pin parameter, at a layer time.
    fn pin_value(effects: &[crate::model::EffectInstance], id: &str, t: f64) -> f64 {
        let pin = effects
            .iter()
            .rev()
            .find(|e| e.effect.match_name == "corner_pin")
            .expect("a corner pin was added");
        let param = pin
            .params
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("corner pin has no {id}"));
        match &param.value {
            crate::model::EffectValue::Float(p) => p.value_at(t),
            other => panic!("{id} is {other:?}, not a float"),
        }
    }

    /// The plain case: comp frame `n` is source frame `n`, so the pin's eight
    /// numbers are the tracked quad's eight, frame for frame.
    #[test]
    fn a_corner_pin_from_a_track_lands_the_quad_frame_for_frame() {
        let media = Uuid::now_v7();
        let shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
        let effect = shot.effects[0].id;
        let target = layer("screen", LayerKind::Null, 2);
        let (tracked_id, target_id) = (shot.id, target.id);
        let c = comp("main", vec![target, shot]);
        let comp_id = c.id;
        let mut doc = document(vec![c]);
        let store = SyntheticPlane { track: effect };

        let op = corner_pin_from_track(&doc, comp_id, tracked_id, effect, target_id, &store)
            .expect("a track with an answer in it writes a pin");
        let inverse = apply(&mut doc, &op).unwrap();

        let effects = &doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == target_id)
            .unwrap()
            .effects;
        for n in [0i64, 7, 29, 59] {
            let t = n as f64 / 30.0;
            let want = SyntheticPlane::quad(n);
            for (i, (x, y)) in CORNER_PIN_POINTS.into_iter().enumerate() {
                assert!(
                    (pin_value(effects, x, t) - want[i][0]).abs() < 1e-6,
                    "corner {i} x at frame {n}"
                );
                assert!(
                    (pin_value(effects, y, t) - want[i][1]).abs() < 1e-6,
                    "corner {i} y at frame {n}"
                );
            }
        }

        // One undo step, and it takes the whole pin back — the stack is
        // committed whole, as every effect edit is.
        apply(&mut doc, &inverse).unwrap();
        assert!(doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == target_id)
            .unwrap()
            .effects
            .is_empty());
    }

    /// K-248 again, from the planar side: the track is of the *source*, so a
    /// retimed clip's pin follows the retime rather than the comp's clock. A
    /// half-speed first second puts source frame 5 under comp frame 10, and the
    /// freeze after it holds source frame 15 for the rest of the shot.
    #[test]
    fn a_corner_pin_follows_the_tracked_layers_retime() {
        let media = Uuid::now_v7();
        let mut shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
        let effect = shot.effects[0].id;
        shot.retime = Some(retime(&[(0, 0.0), (30, 0.5), (60, 0.5)]));
        let target = layer("screen", LayerKind::Null, 2);
        let (tracked_id, target_id) = (shot.id, target.id);
        let c = comp("main", vec![target, shot]);
        let comp_id = c.id;
        let mut doc = document(vec![c]);
        let store = SyntheticPlane { track: effect };

        let op = corner_pin_from_track(&doc, comp_id, tracked_id, effect, target_id, &store)
            .expect("a retimed clip still writes a pin");
        apply(&mut doc, &op).unwrap();
        let effects = &doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == target_id)
            .unwrap()
            .effects;

        let at_ten = pin_value(effects, "upper_left_x", 10.0 / 30.0);
        assert!(
            (at_ten - SyntheticPlane::quad(5)[0][0]).abs() < 1e-6,
            "comp frame 10 should read source frame 5, got {at_ten}"
        );
        for n in [31i64, 40, 59] {
            let held = pin_value(effects, "upper_left_x", n as f64 / 30.0);
            assert!(
                (held - SyntheticPlane::quad(15)[0][0]).abs() < 1e-6,
                "the freeze should hold source frame 15 at comp frame {n}, got {held}"
            );
        }
    }

    /// Both refusals, and the one that is easiest to get wrong: a store with
    /// nothing in it under this effect's id must refuse rather than write a pin
    /// full of the schema's own defaults.
    #[test]
    fn a_corner_pin_is_refused_when_there_is_nothing_to_read() {
        let media = Uuid::now_v7();
        let shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
        let effect = shot.effects[0].id;
        let target = layer("screen", LayerKind::Null, 2);
        let (tracked_id, target_id) = (shot.id, target.id);
        let c = comp("main", vec![target, shot]);
        let comp_id = c.id;
        let doc = document(vec![c]);

        // A track filed under a *different* effect: the right shape of answer
        // about the wrong quad, which is the failure a media-keyed store would
        // have made silently.
        let elsewhere = SyntheticPlane {
            track: Uuid::now_v7(),
        };
        assert!(
            corner_pin_from_track(&doc, comp_id, tracked_id, effect, target_id, &elsewhere)
                .is_none()
        );
        // And a target that is not in the comp at all.
        let store = SyntheticPlane { track: effect };
        assert!(
            corner_pin_from_track(&doc, comp_id, tracked_id, effect, Uuid::now_v7(), &store)
                .is_none()
        );
    }
}

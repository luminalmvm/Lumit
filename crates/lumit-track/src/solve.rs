//! Phase 3 — the global solve: relative rotations, rotation averaging, global
//! positions, triangulation, one bundle adjustment, and a pose for every frame
//! (docs/impl/tracking.md §4).
//!
//! # In plain terms
//!
//! Phase 1 followed specks. Phase 2 worked out, for chosen pairs of frames, how
//! the two views relate and which specks belong to the still world. Neither
//! knows where the camera *was*. This file answers that, for the whole shot at
//! once, and it does it in the order the field settled on after twenty years of
//! the other order drifting:
//!
//! 1. **Turn each pair's relationship into a rotation and a direction.** The
//!    fundamental matrix from phase 2 describes the pair without knowing the
//!    lens; told the focal length, it becomes the *essential* matrix, which
//!    does. That one factorises into "the camera turned by this much and
//!    travelled off in that direction" — four ways, of which exactly one puts
//!    the scene in front of both cameras rather than behind one. That test
//!    picks it.
//! 2. **Average the rotations.** Every pair has an opinion about how much the
//!    camera turned between its two frames, and the opinions disagree slightly
//!    and, for a bad pair, wildly. Rotation averaging finds the one set of
//!    orientations that fits all the opinions best, letting the loudest
//!    disagreements be outvoted rather than split the difference with them.
//!    The first keyframe is declared to be the world's orientation, because
//!    nothing in the pictures can say otherwise.
//! 3. **Place the cameras.** Each pair also knows the *direction* it travelled,
//!    but not how far — that is what a single pair can never tell you. Given
//!    all the directions at once, though, there is only one arrangement of
//!    camera positions that agrees with all of them, and finding it is a
//!    least-squares problem. One case genuinely has no answer: a camera that
//!    moves in a dead straight line gives every pair the same direction, and
//!    nothing then says whether the second frame is a third of the way along or
//!    two thirds. That is reported ([`SolveNote::ColinearBaselines`]), not
//!    quietly guessed at.
//! 4. **Put the specks in space.** With cameras placed, each track is two or
//!    more lines through space that should meet; where they come closest is the
//!    3D point. Points that come out behind a camera, or whose lines are so
//!    nearly parallel that "where they meet" is meaningless, are thrown away.
//! 5. **Adjust everything together.** See [`crate::bundle`].
//! 6. **Fill in the frames between the keyframes** by asking the opposite
//!    question — given these known points, where must the camera have been to
//!    see them there?
//!
//! What comes out is a [`CameraSolve`]: a pose per frame, a focal per segment,
//! the point cloud, and the error the answer actually achieves. Two runs over
//! the same tracks produce the identical bits.
//!
//! # Thread role and contract
//!
//! Pure computation, no IO, no clocks, no threads. [`solve_camera`] is one
//! bounded call — every loop in it has a fixed iteration cap — and is meant to
//! run on a worker, not the UI thread.

use crate::bundle::{self, BundleCamera, BundleObs};
use crate::geom::{self, eigen_ascending, invert3, mat_vec, mul3, transpose3, Mat3};
use crate::pairs::{PairGeometry, PairVerdict};
use crate::segment::{ZoomBoundary, ZoomKind};
use crate::{Correspondence, TrackSet, TrackState};

/// Every knob the global solve takes. The defaults are docs/impl/tracking.md
/// §4's choices made numeric.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolveSettings {
    /// Focal to assume, as a multiple of the raster's long edge, when nothing
    /// in the shot yields a self-calibration. 1.2 is about a 45° horizontal
    /// field of view on a 16:9 frame — an unremarkable lens.
    pub default_focal_factor: f64,
    /// Focal clamp, as multiples of the long edge: roughly a 120° and a 10°
    /// horizontal field of view. A self-calibration outside this is not a wide
    /// lens, it is a failed estimate.
    pub min_focal_factor: f64,
    pub max_focal_factor: f64,
    /// IRLS sweeps in rotation averaging. Fixed, not convergence-driven, so the
    /// work is the same every run.
    pub rotation_iterations: usize,
    /// Alternations in translation averaging.
    pub position_iterations: usize,
    /// Ray angle, in degrees, a triangulated point must subtend at some pair of
    /// its observing cameras. Below this the depth is guesswork.
    pub min_parallax_deg: f64,
    /// Reprojection error, in source pixels, above which a triangulated point
    /// or a resection correspondence is dropped.
    pub max_reprojection_px: f64,
    /// Huber knee for the bundle, in source pixels.
    pub huber_px: f64,
    /// Levenberg–Marquardt iteration cap.
    pub bundle_iterations: usize,
    /// Points a frame needs before it can be resectioned at all.
    pub min_resection_points: usize,
    /// Ratio of the smallest to the largest eigenvalue of the baseline
    /// directions' scatter below which the motion is called colinear and the
    /// positions are reported as resting on the bundle rather than on the
    /// directions.
    pub colinear_ratio: f64,
    /// Passes over the geometry. The second re-derives every relative pose from
    /// the focal the first pass's bundle settled on, which is the difference
    /// between a solve that stands on a guessed lens and one that does not.
    pub passes: usize,
    /// Frames between focal knots inside a detected zoom ramp — about one knot
    /// per second at common rates. The knots are deliberately sparse: each one
    /// is a column of the reduced camera system, and a lens ramp is a smooth
    /// curve that a handful of knots describes.
    pub knot_spacing_frames: i64,
    /// The most focal knots one segment may carry, however long its ramps run.
    /// Keeps the reduced system's growth bounded (the dense factorisation in
    /// [`crate::bundle`] is cubic in its width).
    pub max_knots_per_segment: usize,
}

impl Default for SolveSettings {
    fn default() -> Self {
        SolveSettings {
            default_focal_factor: 1.2,
            min_focal_factor: 0.3,
            max_focal_factor: 6.0,
            rotation_iterations: 12,
            position_iterations: 24,
            min_parallax_deg: 0.5,
            max_reprojection_px: 4.0,
            huber_px: 2.0,
            bundle_iterations: 40,
            min_resection_points: 8,
            colinear_ratio: 1e-3,
            passes: 2,
            knot_spacing_frames: 25,
            max_knots_per_segment: 8,
        }
    }
}

/// Why a shot could not be solved. Each variant is a refusal, not a fault: the
/// pictures did not carry the answer, and inventing one is the failure mode
/// this crate exists to avoid (K-415).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SolveError {
    /// The track set is empty or has no raster.
    #[error("there are no tracks to solve")]
    NoTracks,
    /// No keyframe pair survived phase 2, or none of them yielded a usable
    /// two-view geometry.
    #[error("no keyframe pair carries a usable geometry")]
    NoKeyframes,
    /// Every usable pair was rotation-only: the camera turned or zoomed but
    /// never travelled, so there is no translation to solve and no depth to
    /// recover. The rotations are recoverable and a nodal solve is a separate
    /// product; this refuses the one that was asked for.
    #[error(
        "every pair is rotation-only; the camera never travelled, so there is no position to solve"
    )]
    RotationOnly,
    /// Nothing triangulated: the cameras were placed but no track produced a
    /// 3D point that survived cheirality and parallax.
    #[error("no track triangulated to a point in front of its cameras")]
    NoPoints,
    /// The caller asked for the solve to stop. Nothing partial is handed back:
    /// a half-adjusted bundle is not a camera path, and the whole point of the
    /// flag is that the answer never depends on when it was raised
    /// (14-ENGINEERING-RULES §1.4).
    #[error("the solve was cancelled")]
    Cancelled,
}

/// Something the solve wants the caller to know without failing over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SolveNote {
    /// The baseline directions all point the same way, so the *ratios* between
    /// the baselines are not determined by the directions at all — a dead
    /// straight dolly. The spacing that came out rests on the bundle adjustment
    /// and the point cloud, which do constrain it, rather than on the
    /// translation averaging, which cannot.
    ColinearBaselines,
    /// No pair in the shot self-calibrated, so the focal started from
    /// [`SolveSettings::default_focal_factor`] and is whatever the bundle made
    /// of it.
    ///
    /// `segment` is the segment the guess was anchored on, and today that is
    /// always `0`: the zoom cuts tie every segment's focal to the first one's by
    /// a measured ratio (docs/impl/tracking.md §4's deviation 2), so the shot
    /// has one focal unknown and one place for it to have been guessed. The
    /// field is here because per-segment self-calibration is what a focal hint
    /// or a solved ramp would reintroduce, and callers should not have to change
    /// shape when it does.
    FocalGuessed { segment: usize },
    /// A smooth zoom was detected inside this segment, and its focal was
    /// solved as a curve — sparse knots in the bundle, linearly interpolated
    /// between, read per frame through [`SolvedPose::focal_px`]. The note
    /// keeps reporting where the lens moved, because a ramp is still the
    /// weaker case: the pose–focal trade is better conditioned across a cut
    /// than inside a smooth ramp.
    ZoomRamp { first_frame: i64, last_frame: i64 },
    /// This many frames could not be resectioned against the cloud and were
    /// interpolated from their neighbours ([`PoseSource::Interpolated`]).
    InterpolatedFrames(usize),
    /// This many keyframes were not connected to the first one through any
    /// chain of pairs and were dropped from the global solve; they are
    /// resectioned or interpolated like any other frame.
    DisconnectedKeyframes(usize),
}

/// One stretch of the shot over which the lens does not cut.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SolveSegment {
    pub first_frame: i64,
    pub last_frame: i64,
    /// The solved focal length in source raster pixels — the knot's value for
    /// a constant segment, the mean over the segment's frames where a ramp
    /// bent the focal into a curve (the per-frame values are on
    /// [`SolvedPose::focal_px`]).
    pub focal_px: f64,
    /// Whether a smooth zoom was detected inside this segment. See
    /// [`SolveNote::ZoomRamp`].
    pub ramp: bool,
}

/// Where one frame's pose came from — an honest label, because an interpolated
/// pose is not a measured one and phase 4 should be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PoseSource {
    /// A keyframe: solved globally and refined by the bundle.
    Keyframe,
    /// Resectioned against the solved point cloud.
    Resection,
    /// Neither was possible; the pose is interpolated between the nearest
    /// solved frames (or copied from the nearest one, at the ends).
    Interpolated,
}

/// One frame's camera.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SolvedPose {
    pub frame: i64,
    /// World → camera rotation, row-major.
    pub rotation: Mat3,
    /// Camera centre in world coordinates.
    pub position: [f64; 3],
    /// Index into [`CameraSolve::segments`].
    pub segment: usize,
    /// This frame's own focal in source pixels — the segment's constant where
    /// the lens held still, a point on the solved knot curve where it ramped —
    /// so a per-frame export never has to walk the segment table.
    pub focal_px: f64,
    /// Mean reprojection error over the solved points visible on this frame, in
    /// source pixels. `0.0` where no solved point is visible.
    pub mean_reprojection_px: f64,
    pub source: PoseSource,
}

/// One triangulated point: the track it came from and where it is. Colourless —
/// the tracker works on luma and has no colour to give (docs/impl/tracking.md
/// §4's output shape).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScenePoint {
    pub track: u32,
    pub position: [f64; 3],
}

/// The whole answer: what phase 4's export reads.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CameraSolve {
    /// One per frame of the track set's range, in frame order.
    pub poses: Vec<SolvedPose>,
    /// The segment table, in frame order, covering the range without gaps.
    pub segments: Vec<SolveSegment>,
    /// The point cloud, in track-id order.
    pub points: Vec<ScenePoint>,
    /// The frames the global solve stood on, ascending.
    pub keyframes: Vec<i64>,
    /// Mean reprojection error over every observation the bundle saw.
    pub mean_reprojection_px: f64,
    /// What the caller should know. Empty is the happy case.
    pub notes: Vec<SolveNote>,
}

/// Solve the camera for a whole shot (docs/impl/tracking.md §4).
///
/// `pairs` are phase 2's keyframe pairs ([`crate::select_keyframes`]) and
/// `zooms` its focal-change boundaries ([`crate::detect_zoom`]); a zoom
/// [`ZoomKind::Cut`] splits the focal unknowns into segments while pose stays
/// continuous across it. Tracks marked [`TrackState::Moving`] never enter the
/// solve.
///
/// # Errors
///
/// [`SolveError`] where the shot does not carry an answer — no tracks, no
/// usable pair, a rotation-only shot with no translation in it, or nothing that
/// triangulates. None of these is a fault; each is a refusal to invent.
pub fn solve_camera(
    set: &TrackSet,
    pairs: &[PairGeometry],
    zooms: &[ZoomBoundary],
    settings: &SolveSettings,
) -> Result<CameraSolve, SolveError> {
    solve_camera_cancellable(set, pairs, zooms, settings, &|| false)
}

/// [`solve_camera`] with a stop button: `cancel` is asked between passes and at
/// every Levenberg–Marquardt iteration, and a `true` abandons the solve with
/// [`SolveError::Cancelled`].
///
/// The seam docs/impl/tracking.md §4 recorded as owed. It cannot change an
/// answer, only refuse to finish one: nothing partial is returned, and a run
/// that is never cancelled takes exactly the path [`solve_camera`] takes, which
/// is why determinism is unaffected.
///
/// # Errors
///
/// As [`solve_camera`], plus [`SolveError::Cancelled`].
pub fn solve_camera_cancellable(
    set: &TrackSet,
    pairs: &[PairGeometry],
    zooms: &[ZoomBoundary],
    settings: &SolveSettings,
    cancel: &dyn Fn() -> bool,
) -> Result<CameraSolve, SolveError> {
    let (first, last) = set.frame_range().ok_or(SolveError::NoTracks)?;
    let (w, h) = set.source_size();
    if w == 0 || h == 0 {
        return Err(SolveError::NoTracks);
    }
    let centre = [w as f64 / 2.0, h as f64 / 2.0];
    let long_edge = w.max(h) as f64;
    let mut notes: Vec<SolveNote> = Vec::new();

    // --- 1. Segments and their focal unknowns -------------------------------
    let mut segments = segment_table(first, last, zooms);
    for seg in &segments {
        if seg.ramp {
            notes.push(SolveNote::ZoomRamp {
                first_frame: seg.first_frame,
                last_frame: seg.last_frame,
            });
        }
    }
    let usable: Vec<&PairGeometry> = pairs
        .iter()
        .filter(|g| g.verdict != PairVerdict::Degenerate)
        .collect();
    if usable.is_empty() {
        return Err(SolveError::NoKeyframes);
    }
    let range = (
        settings.min_focal_factor * long_edge,
        settings.max_focal_factor * long_edge,
    );
    // The lens ratio across a cut is not a thing to re-estimate: the zoom
    // detector already measured it from every track in the frame, and a
    // scope-in from 300 px to 420 px *is* a median excess log scale of ln 1.4.
    // So the focal knots are tied to one another by those measured ratios —
    // across cuts and along ramps alike — and the whole shot has one base
    // focal unknown to search for, which every pair in it votes on.
    let mut curve = FocalCurve::build(&segments, zooms, settings);
    let voters: Vec<(&PairGeometry, f64, f64)> = usable
        .iter()
        .filter(|g| g.verdict == PairVerdict::Translating)
        .map(|g| {
            (
                *g,
                curve.value_at(&segments, g.from),
                curve.value_at(&segments, g.to),
            )
        })
        .collect();
    let base = match focal_from_pairs(&voters, centre, range) {
        Some(f) => f,
        None => {
            notes.push(SolveNote::FocalGuessed { segment: 0 });
            settings.default_focal_factor * long_edge
        }
    };
    for v in &mut curve.values {
        *v = (*v * base).clamp(range.0, range.1);
    }
    sync_segment_focals(&mut segments, &curve);

    // --- 2..6. Two passes over the geometry ---------------------------------
    //
    // The first pass stands on a focal guessed from the pairs, which is the
    // weakest number in this file: two-view self-calibration is fragile, and a
    // focal 30% out bends every relative rotation with it. The second pass
    // throws that away and re-derives every relative pose from the focal the
    // bundle worked out, which is the strongest number available — it was
    // fitted to every observation at once. Bounded at two, because a third
    // moves nothing measurable and an unbounded loop is not a solve, it is a
    // hope.
    let mut outcome: Option<Pass> = None;
    for _ in 0..settings.passes.max(1) {
        if cancel() {
            return Err(SolveError::Cancelled);
        }
        let pass = one_pass(set, &usable, &segments, &curve, centre, settings, cancel)?;
        curve.values.clone_from(&pass.focals);
        sync_segment_focals(&mut segments, &curve);
        outcome = Some(pass);
    }
    let Some(pass) = outcome else {
        return Err(SolveError::NoKeyframes);
    };
    let Pass {
        keyframes,
        cams,
        points,
        tracks,
        mean_px,
        ..
    } = pass;
    notes.extend(pass.notes.iter().copied());

    // --- 7. A pose for every frame -------------------------------------------
    let mut lookup: Vec<(u32, usize)> = tracks.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    lookup.sort_unstable();
    let mut poses = fill_frames(
        set, first, last, &keyframes, &cams, &segments, &curve, &points, &lookup, centre, settings,
    );
    let interpolated = poses
        .iter()
        .filter(|p| p.source == PoseSource::Interpolated)
        .count();
    if interpolated > 0 {
        notes.push(SolveNote::InterpolatedFrames(interpolated));
    }
    poses.sort_by_key(|p| p.frame);

    let mut cloud: Vec<ScenePoint> = tracks
        .iter()
        .zip(points.iter())
        .map(|(id, p)| ScenePoint {
            track: *id,
            position: *p,
        })
        .collect();
    cloud.sort_by_key(|p| p.track);

    Ok(CameraSolve {
        poses,
        segments,
        points: cloud,
        keyframes,
        mean_reprojection_px: mean_px,
        notes,
    })
}

/// What one pass over the geometry produced.
struct Pass {
    keyframes: Vec<i64>,
    cams: Vec<BundleCamera>,
    /// The adjusted focal knot values, in [`FocalCurve::values`]'s order.
    focals: Vec<f64>,
    points: Vec<[f64; 3]>,
    tracks: Vec<u32>,
    mean_px: f64,
    notes: Vec<SolveNote>,
}

/// Relative poses → rotation averaging → global positions → triangulation →
/// bundle adjustment, from the focal knots `curve` currently carries.
fn one_pass(
    set: &TrackSet,
    usable: &[&PairGeometry],
    segments: &[SolveSegment],
    curve: &FocalCurve,
    centre: [f64; 2],
    settings: &SolveSettings,
    cancel: &dyn Fn() -> bool,
) -> Result<Pass, SolveError> {
    let mut notes: Vec<SolveNote> = Vec::new();

    // --- The keyframe view graph --------------------------------------------
    let mut frames: Vec<i64> = Vec::with_capacity(usable.len() * 2);
    for g in usable {
        frames.push(g.from);
        frames.push(g.to);
    }
    frames.sort_unstable();
    frames.dedup();

    let mut rels: Vec<Relative> = Vec::with_capacity(usable.len());
    for g in usable {
        let (Ok(i), Ok(j)) = (frames.binary_search(&g.from), frames.binary_search(&g.to)) else {
            continue;
        };
        let fi = curve.value_at(segments, g.from);
        let fj = curve.value_at(segments, g.to);
        if g.verdict == PairVerdict::Translating {
            let pts: Vec<Correspondence> = set
                .correspondences(g.from, g.to)
                .into_iter()
                .filter(|c| g.is_inlier(c.id))
                .collect();
            if let Some((rot, dir)) = relative_pose(&g.fundamental, fi, fj, centre, &pts) {
                rels.push(Relative {
                    i,
                    j,
                    rot,
                    dir: Some(dir),
                });
                continue;
            }
        }
        if let Some(rot) = rotation_from_homography(&g.homography, fi, fj, centre) {
            rels.push(Relative {
                i,
                j,
                rot,
                dir: None,
            });
        }
    }
    if rels.is_empty() {
        return Err(SolveError::NoKeyframes);
    }
    if rels.iter().all(|r| r.dir.is_none()) {
        return Err(SolveError::RotationOnly);
    }

    // Only the component reachable from the first keyframe can be placed in one
    // world; the rest are resectioned later like any in-between frame.
    let keep = reachable(frames.len(), &rels);
    let dropped = keep.iter().filter(|k| !**k).count();
    if dropped > 0 {
        notes.push(SolveNote::DisconnectedKeyframes(dropped));
    }
    let mut remap = vec![usize::MAX; frames.len()];
    let mut keyframes: Vec<i64> = Vec::new();
    for (old, &alive) in keep.iter().enumerate() {
        if alive {
            if let Some(slot) = remap.get_mut(old) {
                *slot = keyframes.len();
            }
            if let Some(f) = frames.get(old) {
                keyframes.push(*f);
            }
        }
    }
    let rels: Vec<Relative> = rels
        .iter()
        .filter_map(|r| {
            let (Some(&i), Some(&j)) = (remap.get(r.i), remap.get(r.j)) else {
                return None;
            };
            if i == usize::MAX || j == usize::MAX {
                return None;
            }
            Some(Relative { i, j, ..*r })
        })
        .collect();
    let views = keyframes.len();
    if views < 2 {
        return Err(SolveError::NoKeyframes);
    }

    // --- Rotation averaging ---------------------------------------------------
    let rotations = average_rotations(views, &rels, settings.rotation_iterations);

    // --- Global positions ------------------------------------------------------
    let mut edges: Vec<(usize, usize, [f64; 3])> = Vec::new();
    for r in &rels {
        let (Some(d), Some(ri)) = (r.dir, rotations.get(r.i)) else {
            continue;
        };
        // The direction came out in camera i's frame; the world is camera i's
        // frame rotated back.
        if let Some(u) = normalise3(mat_vec(&transpose3(ri), d)) {
            edges.push((r.i, r.j, u));
        }
    }
    if edges.is_empty() {
        return Err(SolveError::RotationOnly);
    }
    if colinear(&edges, settings.colinear_ratio) {
        notes.push(SolveNote::ColinearBaselines);
    }
    let positions = average_positions(views, &edges, settings.position_iterations);

    // --- Triangulation ----------------------------------------------------------
    let mut cams: Vec<BundleCamera> = Vec::with_capacity(views);
    for (k, frame) in keyframes.iter().enumerate() {
        cams.push(BundleCamera {
            rot: rotations.get(k).copied().unwrap_or(IDENTITY),
            pos: positions.get(k).copied().unwrap_or([0.0; 3]),
            focal: curve.ref_at(segments, *frame),
        });
    }
    let mut focals: Vec<f64> = curve.values.clone();
    let (mut points, tracks, obs) =
        triangulate_tracks(set, &keyframes, &cams, &focals, centre, settings);
    if points.is_empty() {
        return Err(SolveError::NoPoints);
    }
    let obs_before = obs.len();

    // --- Bundle adjustment --------------------------------------------------------
    let mut report = bundle::bundle_adjust(
        &mut cams,
        &mut focals,
        &mut points,
        &obs,
        centre,
        settings.huber_px,
        settings.bundle_iterations,
        cancel,
    );
    // Now — and only now — reprojection is the right judge. Anything the
    // adjusted model still cannot explain was never a still-world point, and
    // the model is refitted without it.
    let (kept, tracks, obs) = keep_explained(
        &points,
        &tracks,
        &obs,
        &cams,
        &focals,
        centre,
        settings.max_reprojection_px,
    );
    let mut points = kept;
    if points.is_empty() {
        return Err(SolveError::NoPoints);
    }
    if obs.len() < obs_before {
        report = bundle::bundle_adjust(
            &mut cams,
            &mut focals,
            &mut points,
            &obs,
            centre,
            settings.huber_px,
            settings.bundle_iterations,
            cancel,
        );
    }
    // A cancelled bundle stopped early, so `cams`, `focals` and `points` are a
    // half-adjusted model rather than a solve. It is thrown away here rather
    // than filled out into frames and handed back looking finished.
    if cancel() {
        return Err(SolveError::Cancelled);
    }
    Ok(Pass {
        keyframes,
        cams,
        focals,
        points,
        tracks,
        mean_px: report.mean_px,
        notes,
    })
}

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// One edge of the keyframe view graph.
#[derive(Clone, Copy)]
struct Relative {
    i: usize,
    j: usize,
    /// `R_j · R_iᵀ` — how much the camera turned between the two views.
    rot: Mat3,
    /// Unit direction from camera `i` to camera `j`, **in camera i's frame**.
    /// `None` for a rotation-only pair, which has no baseline at all.
    dir: Option<[f64; 3]>,
}

// --- Segments ----------------------------------------------------------------

fn segment_table(first: i64, last: i64, zooms: &[ZoomBoundary]) -> Vec<SolveSegment> {
    let mut cuts: Vec<i64> = zooms
        .iter()
        .filter(|z| z.kind == ZoomKind::Cut)
        .map(|z| z.frame)
        .filter(|f| *f >= first && *f < last)
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<SolveSegment> = Vec::with_capacity(cuts.len() + 1);
    let mut start = first;
    for c in cuts {
        if c < start {
            continue;
        }
        out.push(SolveSegment {
            first_frame: start,
            last_frame: c,
            focal_px: 0.0,
            ramp: false,
        });
        start = c + 1;
    }
    out.push(SolveSegment {
        first_frame: start,
        last_frame: last,
        focal_px: 0.0,
        ramp: false,
    });
    for seg in &mut out {
        seg.ramp = zooms.iter().any(|z| {
            z.kind == ZoomKind::Ramp && z.end_frame >= seg.first_frame && z.frame < seg.last_frame
        });
    }
    out
}

fn segment_index(segments: &[SolveSegment], frame: i64) -> usize {
    segments
        .iter()
        .position(|s| frame >= s.first_frame && frame <= s.last_frame)
        .unwrap_or(0)
}

// --- The focal curve -----------------------------------------------------------

/// The focal curve the solve works on: one knot for a constant-focal segment,
/// a sparse ascending row of knots for a segment with a detected zoom ramp,
/// every knot an independent column of the bundle (docs/impl/tracking.md §4).
/// A frame before its segment's first knot reads that knot, one past the last
/// reads the last, and one between two knots reads a linear blend — a
/// piecewise-linear spline, which over the handful of frames a lens rack spans
/// is within a per-cent of any smoother one.
struct FocalCurve {
    /// Per segment: `(frame, knot index)`, ascending by frame, never empty.
    knots: Vec<Vec<(i64, usize)>>,
    /// One value per knot — relative scales until the base focal is known,
    /// source pixels afterwards.
    values: Vec<f64>,
}

impl FocalCurve {
    /// Lay the knots out and give each its initial *relative* scale.
    ///
    /// A cut's `log_scale` is the median of how much every patch in the frame
    /// grew between the two frames it sits between, in excess of the shot's
    /// own travel. With pose continuous across a cut — which is what a cut is
    /// — that growth is the focal ratio and nothing else, and it was measured
    /// over hundreds of tracks rather than estimated from two views. A ramp's
    /// is the same number per pair, so its knots compound the measured rate
    /// along the run. The whole shot therefore hangs off **one** base focal
    /// for the self-calibration to find, exactly as before, and every knot
    /// starts on the detector's own measurement of where the lens went.
    fn build(
        segments: &[SolveSegment],
        zooms: &[ZoomBoundary],
        settings: &SolveSettings,
    ) -> FocalCurve {
        let spacing = settings.knot_spacing_frames.max(1);
        let cap = settings.max_knots_per_segment.max(2);
        let mut knots: Vec<Vec<(i64, usize)>> = Vec::with_capacity(segments.len());
        let mut values: Vec<f64> = Vec::new();
        // Relative scale at the segment's first frame, compounded across the
        // cuts between segments and the ramps inside them.
        let mut entry = 1.0f64;
        for seg in segments {
            let ramps: Vec<&ZoomBoundary> = zooms
                .iter()
                .filter(|z| {
                    z.kind == ZoomKind::Ramp
                        && z.frame.max(seg.first_frame) < (z.end_frame + 1).min(seg.last_frame)
                })
                .collect();
            let mut frames: Vec<i64> = vec![seg.first_frame];
            if seg.ramp {
                for z in &ramps {
                    let start = z.frame.max(seg.first_frame);
                    let end = (z.end_frame + 1).min(seg.last_frame);
                    let mut f = start;
                    while f < end {
                        frames.push(f);
                        f += spacing;
                    }
                    frames.push(end);
                }
            }
            frames.sort_unstable();
            frames.dedup();
            if frames.len() > cap {
                // Thin evenly, keeping the first and last knot.
                let n = frames.len();
                let thinned: Vec<i64> = (0..cap)
                    .filter_map(|k| frames.get((k * (n - 1)) / (cap - 1)).copied())
                    .collect();
                frames = thinned;
                frames.dedup();
            }
            // The measured lens motion up to `frame`, in log scale, summed
            // over the ramps' pairs before it.
            let cumulative = |frame: i64| -> f64 {
                ramps
                    .iter()
                    .map(|z| {
                        let start = z.frame.max(seg.first_frame);
                        let end = (z.end_frame + 1).min(seg.last_frame);
                        z.log_scale * (frame.min(end) - start).max(0) as f64
                    })
                    .sum()
            };
            let total = cumulative(seg.last_frame);
            let mut list = Vec::with_capacity(frames.len());
            for f in frames {
                list.push((f, values.len()));
                let scale = entry * cumulative(f).exp();
                values.push(if scale.is_finite() { scale } else { entry });
            }
            knots.push(list);
            if total.is_finite() {
                entry *= total.exp();
            }
            let step = zooms
                .iter()
                .find(|z| z.kind == ZoomKind::Cut && z.frame == seg.last_frame)
                .map_or(0.0, |z| z.log_scale);
            if step.is_finite() {
                entry *= step.exp();
            }
        }
        FocalCurve { knots, values }
    }

    /// Which knot or pair of knots the frame reads, and the blend between them.
    fn ref_at(&self, segments: &[SolveSegment], frame: i64) -> bundle::FocalRef {
        let Some(list) = self.knots.get(segment_index(segments, frame)) else {
            return bundle::FocalRef::fixed(0);
        };
        let Some(&(first_frame, first_index)) = list.first() else {
            return bundle::FocalRef::fixed(0);
        };
        if frame <= first_frame {
            return bundle::FocalRef::fixed(first_index);
        }
        for pair in list.windows(2) {
            let [(fa, a), (fb, b)] = pair else { continue };
            if frame <= *fb {
                let span = (fb - fa).max(1) as f64;
                return bundle::FocalRef {
                    a: *a,
                    b: *b,
                    t: ((frame - fa) as f64 / span).clamp(0.0, 1.0),
                };
            }
        }
        list.last()
            .map_or(bundle::FocalRef::fixed(first_index), |&(_, i)| {
                bundle::FocalRef::fixed(i)
            })
    }

    /// The focal the frame reads, in whatever units [`FocalCurve::values`] is
    /// currently in.
    fn value_at(&self, segments: &[SolveSegment], frame: i64) -> f64 {
        self.ref_at(segments, frame)
            .value(&self.values)
            .unwrap_or(1.0)
    }
}

/// Write the curve back onto the segment table's reporting field: the knot's
/// value where the segment is constant, the mean over its frames where a ramp
/// bent the focal (the per-frame values are on the poses).
fn sync_segment_focals(segments: &mut [SolveSegment], curve: &FocalCurve) {
    for i in 0..segments.len() {
        let Some(seg) = segments.get(i).copied() else {
            continue;
        };
        let focal = match curve.knots.get(i) {
            Some(list) if list.len() == 1 => list
                .first()
                .and_then(|&(_, k)| curve.values.get(k))
                .copied()
                .unwrap_or(seg.focal_px),
            _ => {
                let frames = (seg.last_frame - seg.first_frame + 1).max(1);
                let mut sum = 0.0f64;
                for f in seg.first_frame..=seg.last_frame {
                    sum += curve.value_at(segments, f);
                }
                sum / frames as f64
            }
        };
        if let Some(seg) = segments.get_mut(i) {
            seg.focal_px = focal;
        }
    }
}

/// How far a candidate pair of focals is from making `f` an essential matrix.
///
/// An essential matrix — and only an essential matrix — has two equal non-zero
/// singular values and one zero. `E = K_toᵀ·F·K_from` for the true focals
/// therefore scores zero here, and the asymmetry rises either side of them. The
/// singular values come out of the same symmetric eigensolver everything else
/// in this crate uses: they are the square roots of the eigenvalues of `EᵀE`.
fn essential_asymmetry(f: &Mat3, focal_from: f64, focal_to: f64, centre: [f64; 2]) -> Option<f64> {
    let e = mul3(
        &transpose3(&intrinsics(focal_to, centre)),
        &mul3(f, &intrinsics(focal_from, centre)),
    );
    let (vals, _) = eigen_ascending(&mul3(&transpose3(&e), &e));
    let (s1, s2) = (vals[2].max(0.0).sqrt(), vals[1].max(0.0).sqrt());
    if !s1.is_finite() || s1 + s2 < 1e-300 {
        return None;
    }
    Some((s1 - s2) / (s1 + s2))
}

/// The first segment's focal length, in source pixels, that best explains every
/// keyframe pair — docs/impl/tracking.md §4's F→K self-calibration.
///
/// The note names Bougnoux's closed form. It was written, measured, and
/// replaced: on the synthetic orbit its per-pair answers ranged from 57 px to
/// 578 px for a true 300, because the formula divides by a quantity that goes
/// to zero at the critical configurations and every real shot spends time near
/// one. What survives is the *constraint* the formula solves — that `KᵀFK` must
/// have two equal singular values — minimised numerically over a bounded range
/// instead of solved in closed form, and minimised over **all** the segment's
/// pairs at once rather than pair by pair. A coarse sweep in log-focal finds the
/// basin, a fixed number of ternary steps polishes it; both are bounded, so the
/// work and the answer are the same every run.
fn focal_from_pairs(
    pairs: &[(&PairGeometry, f64, f64)],
    centre: [f64; 2],
    range: (f64, f64),
) -> Option<f64> {
    if pairs.is_empty() || !(range.0 > 0.0 && range.1 > range.0) {
        return None;
    }
    // The median over pairs, so one pair sitting on a critical configuration
    // cannot choose the lens for the whole shot.
    let cost = |focal: f64| -> f64 {
        let mut v: Vec<f64> = pairs
            .iter()
            .filter_map(|(g, sf, st)| {
                essential_asymmetry(&g.fundamental, focal * sf, focal * st, centre)
            })
            .collect();
        geom::median(&mut v).unwrap_or(f64::INFINITY)
    };
    let (lo, hi) = (range.0.ln(), range.1.ln());
    const SWEEP: usize = 96;
    let mut best = (f64::INFINITY, lo);
    for i in 0..=SWEEP {
        let x = lo + (hi - lo) * i as f64 / SWEEP as f64;
        let c = cost(x.exp());
        if c < best.0 {
            best = (c, x);
        }
    }
    if !best.0.is_finite() {
        return None;
    }
    let step = (hi - lo) / SWEEP as f64;
    let (mut a, mut b) = ((best.1 - step).max(lo), (best.1 + step).min(hi));
    for _ in 0..40 {
        let m1 = a + (b - a) / 3.0;
        let m2 = b - (b - a) / 3.0;
        if cost(m1.exp()) <= cost(m2.exp()) {
            b = m2;
        } else {
            a = m1;
        }
    }
    let focal = (0.5 * (a + b)).exp();
    if focal.is_finite() && focal > 0.0 {
        Some(focal)
    } else {
        None
    }
}

// --- Two-view pose -----------------------------------------------------------

fn intrinsics(f: f64, centre: [f64; 2]) -> Mat3 {
    [[f, 0.0, centre[0]], [0.0, f, centre[1]], [0.0, 0.0, 1.0]]
}

/// The relative rotation and baseline direction of a translating pair.
///
/// `E = K_toᵀ · F · K_from` turns the uncalibrated geometry into the calibrated
/// one; the factorisation gives four candidates and cheirality — the scene has
/// to be in front of both cameras — picks the one that is physically possible.
fn relative_pose(
    f: &Mat3,
    focal_from: f64,
    focal_to: f64,
    centre: [f64; 2],
    pts: &[Correspondence],
) -> Option<(Mat3, [f64; 3])> {
    if pts.len() < 5 || focal_from <= 0.0 || focal_to <= 0.0 {
        return None;
    }
    let e = mul3(
        &transpose3(&intrinsics(focal_to, centre)),
        &mul3(f, &intrinsics(focal_from, centre)),
    );
    let candidates = essential_candidates(&e)?;
    let normalised: Vec<([f64; 2], [f64; 2])> = pts
        .iter()
        .map(|c| {
            (
                [
                    (c.from[0] - centre[0]) / focal_from,
                    (c.from[1] - centre[1]) / focal_from,
                ],
                [
                    (c.to[0] - centre[0]) / focal_to,
                    (c.to[1] - centre[1]) / focal_to,
                ],
            )
        })
        .collect();
    let p1: Proj = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let mut best: Option<(usize, Mat3, [f64; 3])> = None;
    for (rot, t) in candidates {
        let p2: Proj = [
            [rot[0][0], rot[0][1], rot[0][2], t[0]],
            [rot[1][0], rot[1][1], rot[1][2], t[1]],
            [rot[2][0], rot[2][1], rot[2][2], t[2]],
        ];
        let mut good = 0usize;
        for (a, b) in &normalised {
            let Some(x) = triangulate(&[(p1, *a), (p2, *b)]) else {
                continue;
            };
            if x[2] <= 0.0 {
                continue;
            }
            let z = rot[2][0] * x[0] + rot[2][1] * x[1] + rot[2][2] * x[2] + t[2];
            if z > 0.0 {
                good += 1;
            }
        }
        // Strictly greater keeps the first candidate on a tie, which is what
        // makes the walk over the four deterministic.
        if good > best.as_ref().map_or(0, |b| b.0) {
            best = Some((good, rot, t));
        }
    }
    let (good, rot, t) = best?;
    if good * 2 < normalised.len() {
        return None;
    }
    // The second camera's centre in the first camera's frame.
    normalise3(mat_vec(&transpose3(&rot), [-t[0], -t[1], -t[2]])).map(|d| (rot, d))
}

/// The four `(R, t)` decompositions of an essential matrix, in a fixed order.
fn essential_candidates(e: &Mat3) -> Option<Vec<(Mat3, [f64; 3])>> {
    let (vals, vecs) = eigen_ascending(&mul3(&transpose3(e), e));
    let s1 = vals[2].max(0.0).sqrt();
    let s2 = vals[1].max(0.0).sqrt();
    if !s1.is_finite() || s1 < 1e-300 || s2 < 1e-9 * s1 {
        return None;
    }
    let v1 = [vecs[0][2], vecs[1][2], vecs[2][2]];
    let v2 = [vecs[0][1], vecs[1][1], vecs[2][1]];
    let v3 = cross3(v1, v2);
    let u1 = normalise3(mat_vec(e, v1))?;
    let u2 = normalise3(mat_vec(e, v2))?;
    let u3 = cross3(u1, u2);
    let vm = from_columns(v1, v2, v3);
    let um = from_columns(u1, u2, u3);
    let wm: Mat3 = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    let ra = mul3(&um, &mul3(&wm, &transpose3(&vm)));
    let rb = mul3(&um, &mul3(&transpose3(&wm), &transpose3(&vm)));
    let neg = [-u3[0], -u3[1], -u3[2]];
    Some(vec![(ra, u3), (ra, neg), (rb, u3), (rb, neg)])
}

/// The rotation a homography implies for a pair with no translation:
/// `H = K_to · R · K_from⁻¹`, orthogonalised because noise makes it not quite a
/// rotation.
fn rotation_from_homography(
    h: &Mat3,
    focal_from: f64,
    focal_to: f64,
    centre: [f64; 2],
) -> Option<Mat3> {
    if focal_from <= 0.0 || focal_to <= 0.0 {
        return None;
    }
    let ki = invert3(&intrinsics(focal_to, centre))?;
    closest_rotation(&mul3(&ki, &mul3(h, &intrinsics(focal_from, centre))))
}

/// The rotation closest to `m` in the least-squares sense: `M·(MᵀM)^(−½)`, with
/// the reflection case corrected by flipping the smallest eigendirection.
pub(crate) fn closest_rotation(m: &Mat3) -> Option<Mat3> {
    let (vals, vecs) = eigen_ascending(&mul3(&transpose3(m), m));
    if !vals[0].is_finite() || vals[0] <= 1e-24 {
        return None;
    }
    let build = |flip: bool| -> Mat3 {
        let mut a = [[0.0f64; 3]; 3];
        for i in 0..3 {
            let sign = if flip && i == 0 { -1.0 } else { 1.0 };
            let s = sign / vals[i].sqrt();
            let v = [vecs[0][i], vecs[1][i], vecs[2][i]];
            for (r, row) in a.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    *cell += s * v[r] * v[c];
                }
            }
        }
        mul3(m, &a)
    };
    let r = build(false);
    Some(if geom::det3(&r) < 0.0 { build(true) } else { r })
}

// --- Rotation averaging --------------------------------------------------------

/// Robust rotation averaging: a spanning-tree initialisation, then IRLS sweeps
/// that reweight each pair by how far it sits from the consensus.
///
/// The linearisation is the standard one: with `R_k ← exp(δ_k)·R_k`, the
/// residual `log(R̃_ij·R_i·R_jᵀ)` moves to first order as `a_ij + R̃_ij·δ_i − δ_j`,
/// which is a plain linear least squares over the `δ`. Camera 0 is the gauge and
/// takes no parameters, so the world's orientation is the first keyframe's.
fn average_rotations(views: usize, rels: &[Relative], iterations: usize) -> Vec<Mat3> {
    let mut rot = vec![IDENTITY; views];
    let mut done = vec![false; views];
    if let Some(d) = done.first_mut() {
        *d = true;
    }
    let mut queue: Vec<usize> = vec![0];
    let mut head = 0usize;
    while let Some(&k) = queue.get(head) {
        head += 1;
        for r in rels {
            if r.i == k && done.get(r.j) == Some(&false) {
                if let (Some(base), Some(slot)) = (rot.get(k).copied(), rot.get_mut(r.j)) {
                    *slot = mul3(&r.rot, &base);
                }
                if let Some(d) = done.get_mut(r.j) {
                    *d = true;
                }
                queue.push(r.j);
            } else if r.j == k && done.get(r.i) == Some(&false) {
                if let (Some(base), Some(slot)) = (rot.get(k).copied(), rot.get_mut(r.i)) {
                    *slot = mul3(&transpose3(&r.rot), &base);
                }
                if let Some(d) = done.get_mut(r.i) {
                    *d = true;
                }
                queue.push(r.i);
            }
        }
    }

    let width = views.saturating_sub(1) * 3;
    if width == 0 {
        return rot;
    }
    for iteration in 0..iterations {
        let residual: Vec<[f64; 3]> = rels
            .iter()
            .map(|r| {
                let (Some(ri), Some(rj)) = (rot.get(r.i), rot.get(r.j)) else {
                    return [0.0; 3];
                };
                so3_log(&mul3(&r.rot, &mul3(ri, &transpose3(rj))))
            })
            .collect();
        let mut a = bundle::Dense::zero(width);
        let mut b = vec![0.0f64; width];
        for (r, e) in rels.iter().zip(residual.iter()) {
            // L2 for the first two sweeps to get near the answer, then L1-style
            // reweighting, which is what makes a wrong pair lose rather than
            // drag (docs/impl/tracking.md §4's "robust L1→IRLS L2 ladder").
            let n = norm3(*e);
            let w = if iteration < 2 {
                1.0
            } else {
                1.0 / n.max(1e-3)
            };
            let (pi, pj) = (block_base(r.i), block_base(r.j));
            let m = r.rot;
            let mt = transpose3(&m);
            let mtm = mul3(&mt, &m);
            let mte = mat_vec(&mt, *e);
            if let Some(pi) = pi {
                for (k, row) in mtm.iter().enumerate() {
                    for (l, cell) in row.iter().enumerate() {
                        a.add(pi + k, pi + l, w * cell);
                    }
                    if let Some(slot) = b.get_mut(pi + k) {
                        *slot -= w * mte[k];
                    }
                }
            }
            if let Some(pj) = pj {
                for (k, ek) in e.iter().enumerate() {
                    a.add(pj + k, pj + k, w);
                    if let Some(slot) = b.get_mut(pj + k) {
                        *slot += w * ek;
                    }
                }
            }
            if let (Some(pi), Some(pj)) = (pi, pj) {
                // The cross block is −w·Mᵀ, and the normal matrix is symmetric.
                for (l, row) in m.iter().enumerate() {
                    for (k, cell) in row.iter().enumerate() {
                        a.add(pi + k, pj + l, -w * cell);
                        a.add(pj + l, pi + k, -w * cell);
                    }
                }
            }
        }
        for i in 0..width {
            a.add(i, i, 1e-9 * a.at(i, i) + 1e-12);
        }
        let Some(l) = bundle::cholesky(&a) else { break };
        let delta = bundle::cholesky_solve(&l, &b);
        if delta.iter().any(|v| !v.is_finite()) {
            break;
        }
        let mut biggest = 0.0f64;
        for k in 1..views {
            let Some(base) = block_base(k) else { continue };
            let mut d = [0.0f64; 3];
            for (q, cell) in d.iter_mut().enumerate() {
                *cell = delta.get(base + q).copied().unwrap_or(0.0);
            }
            biggest = biggest.max(norm3(d));
            if let Some(slot) = rot.get_mut(k) {
                *slot = mul3(&bundle::so3_exp(d), slot);
            }
        }
        if biggest < 1e-12 {
            break;
        }
    }
    rot
}

fn block_base(index: usize) -> Option<usize> {
    index.checked_sub(1).map(|i| i * 3)
}

/// The rotation vector of `r`: axis times angle.
fn so3_log(r: &Mat3) -> [f64; 3] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    let cos = ((trace - 1.0) * 0.5).clamp(-1.0, 1.0);
    let angle = cos.acos();
    let axis = [r[2][1] - r[1][2], r[0][2] - r[2][0], r[1][0] - r[0][1]];
    if angle < 1e-8 {
        return [0.5 * axis[0], 0.5 * axis[1], 0.5 * axis[2]];
    }
    if angle < std::f64::consts::PI - 1e-4 {
        let k = angle / (2.0 * angle.sin());
        return [k * axis[0], k * axis[1], k * axis[2]];
    }
    // Near half a turn the antisymmetric part vanishes; the magnitudes come out
    // of the diagonal and the signs out of whatever is left of the off-diagonal.
    let mut v = [0.0f64; 3];
    for (i, cell) in v.iter_mut().enumerate() {
        *cell = ((r[i][i] + 1.0) * 0.5).max(0.0).sqrt();
    }
    let mut biggest = 0usize;
    for i in 1..3 {
        if v[i] > v[biggest] {
            biggest = i;
        }
    }
    if axis[biggest] < 0.0 {
        for cell in &mut v {
            *cell = -*cell;
        }
    }
    let pivot = v[biggest];
    if pivot.abs() > 1e-12 {
        for (i, cell) in v.iter_mut().enumerate() {
            if i != biggest && axis[i] * axis[biggest] < 0.0 {
                *cell = -*cell;
            }
        }
    }
    [angle * v[0], angle * v[1], angle * v[2]]
}

// --- Translation averaging -----------------------------------------------------

/// Whether the baseline directions all point along one line, which is the case
/// direction constraints alone cannot resolve (docs/impl/tracking.md §4).
fn colinear(edges: &[(usize, usize, [f64; 3])], ratio: f64) -> bool {
    let mut scatter = [[0.0f64; 3]; 3];
    for (_, _, d) in edges {
        for (r, row) in scatter.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell += d[r] * d[c];
            }
        }
    }
    let (vals, _) = eigen_ascending(&scatter);
    let big = vals[2];
    big > 0.0 && vals[1] <= ratio * big
}

/// BATA-style translation averaging: alternate between the baseline lengths a
/// set of positions implies and the positions those lengths imply, with a Huber
/// reweighting scaled to the residuals so one wrong direction cannot lead.
///
/// The lengths are renormalised to mean 1 each round, which is what stops the
/// least squares from taking its free ride to the all-cameras-in-one-place
/// solution.
fn average_positions(
    views: usize,
    edges: &[(usize, usize, [f64; 3])],
    iterations: usize,
) -> Vec<[f64; 3]> {
    let mut pos = vec![[0.0f64; 3]; views];
    // A spanning-tree initialisation with unit baselines: not the answer, but
    // the right shape to start the alternation from.
    let mut done = vec![false; views];
    if let Some(d) = done.first_mut() {
        *d = true;
    }
    let mut queue: Vec<usize> = vec![0];
    let mut head = 0usize;
    while let Some(&k) = queue.get(head) {
        head += 1;
        for (i, j, d) in edges {
            if *i == k && done.get(*j) == Some(&false) {
                if let (Some(base), Some(slot)) = (pos.get(k).copied(), pos.get_mut(*j)) {
                    *slot = add3(base, *d);
                }
                if let Some(f) = done.get_mut(*j) {
                    *f = true;
                }
                queue.push(*j);
            } else if *j == k && done.get(*i) == Some(&false) {
                if let (Some(base), Some(slot)) = (pos.get(k).copied(), pos.get_mut(*i)) {
                    *slot = sub3(base, *d);
                }
                if let Some(f) = done.get_mut(*i) {
                    *f = true;
                }
                queue.push(*i);
            }
        }
    }

    let width = views.saturating_sub(1);
    if width == 0 || edges.is_empty() {
        return pos;
    }
    let mut lengths = vec![1.0f64; edges.len()];
    for _ in 0..iterations {
        // The lengths each edge's direction would need, given where the cameras
        // currently are; kept positive, because a negative one is the solve
        // trying to reverse a direction it was told.
        let mut total = 0.0f64;
        for ((i, j, d), s) in edges.iter().zip(lengths.iter_mut()) {
            let (Some(a), Some(b)) = (pos.get(*i), pos.get(*j)) else {
                continue;
            };
            *s = dot3(*d, sub3(*b, *a)).max(1e-6);
            total += *s;
        }
        if total <= 0.0 {
            break;
        }
        let scale = edges.len() as f64 / total;
        for s in &mut lengths {
            *s *= scale;
        }

        let mut residuals: Vec<f64> = edges
            .iter()
            .zip(lengths.iter())
            .map(|((i, j, d), s)| {
                let (Some(a), Some(b)) = (pos.get(*i), pos.get(*j)) else {
                    return 0.0;
                };
                norm3(sub3(sub3(*b, *a), scale3(*d, *s)))
            })
            .collect();
        let knee = (1.5 * geom::median(&mut residuals).unwrap_or(0.0)).max(1e-6);

        let mut a = bundle::Dense::zero(width);
        let mut rhs = [
            vec![0.0f64; width],
            vec![0.0f64; width],
            vec![0.0f64; width],
        ];
        for ((i, j, d), s) in edges.iter().zip(lengths.iter()) {
            let (Some(pa), Some(pb)) = (pos.get(*i), pos.get(*j)) else {
                continue;
            };
            let r = norm3(sub3(sub3(*pb, *pa), scale3(*d, *s)));
            let w = if r <= knee { 1.0 } else { knee / r };
            let (bi, bj) = (block1(*i), block1(*j));
            if let Some(bi) = bi {
                a.add(bi, bi, w);
            }
            if let Some(bj) = bj {
                a.add(bj, bj, w);
            }
            if let (Some(bi), Some(bj)) = (bi, bj) {
                a.add(bi, bj, -w);
                a.add(bj, bi, -w);
            }
            for (k, row) in rhs.iter_mut().enumerate() {
                let v = w * s * d[k];
                if let Some(bi) = bi {
                    if let Some(slot) = row.get_mut(bi) {
                        *slot -= v;
                    }
                }
                if let Some(bj) = bj {
                    if let Some(slot) = row.get_mut(bj) {
                        *slot += v;
                    }
                }
            }
        }
        for i in 0..width {
            a.add(i, i, 1e-9 * a.at(i, i) + 1e-12);
        }
        let Some(l) = bundle::cholesky(&a) else { break };
        let mut moved = false;
        for (k, row) in rhs.iter().enumerate() {
            let x = bundle::cholesky_solve(&l, row);
            if x.iter().any(|v| !v.is_finite()) {
                return pos;
            }
            for v in 1..views {
                let Some(b) = block1(v) else { continue };
                if let Some(slot) = pos.get_mut(v).and_then(|p| p.get_mut(k)) {
                    let next = x.get(b).copied().unwrap_or(0.0);
                    if (next - *slot).abs() > 1e-12 {
                        moved = true;
                    }
                    *slot = next;
                }
            }
        }
        // A canonical scale, so the same shot always lands at the same size and
        // the bundle's damping always sees the same magnitudes.
        let mut rms = 0.0f64;
        for p in &pos {
            rms += dot3(*p, *p);
        }
        rms = (rms / views as f64).sqrt();
        if rms > 1e-12 {
            for p in &mut pos {
                *p = scale3(*p, 1.0 / rms);
            }
        }
        if !moved {
            break;
        }
    }
    pos
}

fn block1(index: usize) -> Option<usize> {
    index.checked_sub(1)
}

/// The keyframes reachable from the first one through any chain of pairs.
fn reachable(views: usize, rels: &[Relative]) -> Vec<bool> {
    let mut seen = vec![false; views];
    if let Some(s) = seen.first_mut() {
        *s = true;
    }
    let mut queue: Vec<usize> = vec![0];
    let mut head = 0usize;
    while let Some(&k) = queue.get(head) {
        head += 1;
        for r in rels {
            let other = if r.i == k {
                r.j
            } else if r.j == k {
                r.i
            } else {
                continue;
            };
            if seen.get(other) == Some(&false) {
                if let Some(s) = seen.get_mut(other) {
                    *s = true;
                }
                queue.push(other);
            }
        }
    }
    seen
}

// --- Triangulation --------------------------------------------------------------

/// A 3×4 projection matrix.
type Proj = [[f64; 4]; 3];

fn projection(rot: &Mat3, pos: [f64; 3], focal: f64, centre: [f64; 2]) -> Proj {
    let kr = mul3(&intrinsics(focal, centre), rot);
    let t = mat_vec(&kr, [-pos[0], -pos[1], -pos[2]]);
    [
        [kr[0][0], kr[0][1], kr[0][2], t[0]],
        [kr[1][0], kr[1][1], kr[1][2], t[1]],
        [kr[2][0], kr[2][1], kr[2][2], t[2]],
    ]
}

/// Direct linear triangulation: two rows per view saying "the projected point
/// and the observed point are the same direction", solved for the null space.
fn triangulate(views: &[(Proj, [f64; 2])]) -> Option<[f64; 3]> {
    if views.len() < 2 {
        return None;
    }
    let mut ata = [[0.0f64; 4]; 4];
    for (p, obs) in views {
        for (axis, o) in obs.iter().enumerate() {
            let mut row = [0.0f64; 4];
            for (k, cell) in row.iter_mut().enumerate() {
                *cell = o * p[2][k] - p[axis][k];
            }
            for (dst, ri) in ata.iter_mut().zip(row.iter()) {
                for (cell, rj) in dst.iter_mut().zip(row.iter()) {
                    *cell += ri * rj;
                }
            }
        }
    }
    let (_, vecs) = eigen_ascending(&ata);
    let v = [vecs[0][0], vecs[1][0], vecs[2][0], vecs[3][0]];
    if !v[3].is_finite() || v[3].abs() < 1e-12 {
        return None;
    }
    let x = [v[0] / v[3], v[1] / v[3], v[2] / v[3]];
    if x.iter().any(|c| !c.is_finite()) {
        return None;
    }
    Some(x)
}

/// Triangulate every still track against the placed keyframes.
///
/// Returns the cloud, the track id each point came from, and the observations
/// the bundle will read — all in a fixed order.
fn triangulate_tracks(
    set: &TrackSet,
    keyframes: &[i64],
    cams: &[BundleCamera],
    focals: &[f64],
    centre: [f64; 2],
    settings: &SolveSettings,
) -> (Vec<[f64; 3]>, Vec<u32>, Vec<BundleObs>) {
    let projections: Vec<Proj> = cams
        .iter()
        .map(|c| projection(&c.rot, c.pos, c.focal.value(focals).unwrap_or(1.0), centre))
        .collect();
    let min_cos = (settings.min_parallax_deg.to_radians()).cos();
    let mut points = Vec::new();
    let mut tracks = Vec::new();
    let mut obs = Vec::new();
    let mut views: Vec<(usize, [f64; 2])> = Vec::new();
    for track in set.tracks() {
        if track.state == TrackState::Moving {
            continue;
        }
        views.clear();
        for (k, frame) in keyframes.iter().enumerate() {
            if let Some(p) = track.point_at(*frame) {
                views.push((k, [p.x, p.y]));
            }
        }
        if views.len() < 2 {
            continue;
        }
        let sample: Vec<(Proj, [f64; 2])> = views
            .iter()
            .filter_map(|(k, p)| projections.get(*k).map(|proj| (*proj, *p)))
            .collect();
        let Some(x) = triangulate(&sample) else {
            continue;
        };
        // Cheirality on every observing camera, and a real angle between at
        // least one pair of rays. Deliberately *not* a reprojection test:
        // before the bundle this is an initialisation, and judging it by the
        // number the bundle exists to minimise would throw away the points that
        // are about to be fixed. Reprojection is the gate after the bundle,
        // where it is the right question.
        let mut rays: Vec<[f64; 3]> = Vec::with_capacity(views.len());
        let mut ok = true;
        for (k, _) in &views {
            let Some(cam) = cams.get(*k) else {
                ok = false;
                break;
            };
            let focal = cam.focal.value(focals).unwrap_or(1.0);
            if bundle::project_point(cam, focal, centre, &x).is_none() {
                ok = false;
                break;
            }
            match normalise3(sub3(x, cam.pos)) {
                Some(r) => rays.push(r),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let mut parallax = false;
        for (a, ra) in rays.iter().enumerate() {
            for rb in rays.iter().skip(a + 1) {
                if dot3(*ra, *rb).abs() < min_cos {
                    parallax = true;
                }
            }
        }
        if !parallax {
            continue;
        }
        let index = points.len();
        points.push(x);
        tracks.push(track.id);
        for (k, p) in &views {
            obs.push(BundleObs {
                cam: *k,
                point: index,
                image: *p,
            });
        }
    }
    (points, tracks, obs)
}

/// Drop the points the adjusted model cannot explain, and renumber what is
/// left so the observation list stays consistent.
fn keep_explained(
    points: &[[f64; 3]],
    tracks: &[u32],
    obs: &[BundleObs],
    cams: &[BundleCamera],
    focals: &[f64],
    centre: [f64; 2],
    limit: f64,
) -> (Vec<[f64; 3]>, Vec<u32>, Vec<BundleObs>) {
    let mut sum = vec![0.0f64; points.len()];
    let mut count = vec![0usize; points.len()];
    for o in obs {
        let (Some(cam), Some(x)) = (cams.get(o.cam), points.get(o.point)) else {
            continue;
        };
        let focal = cam.focal.value(focals).unwrap_or(1.0);
        let d = match bundle::project_point(cam, focal, centre, x) {
            Some((q, _)) => (q[0] - o.image[0]).hypot(q[1] - o.image[1]),
            None => f64::INFINITY,
        };
        if let (Some(s), Some(c)) = (sum.get_mut(o.point), count.get_mut(o.point)) {
            *s += d;
            *c += 1;
        }
    }
    let mut remap = vec![usize::MAX; points.len()];
    let mut kept_points = Vec::with_capacity(points.len());
    let mut kept_tracks = Vec::with_capacity(points.len());
    for (i, (s, c)) in sum.iter().zip(count.iter()).enumerate() {
        // A NaN mean is a point nothing could project, and it must fall on the
        // reject side; `> limit` would keep it.
        let mean = s / *c as f64;
        if *c == 0 || !mean.is_finite() || mean > limit {
            continue;
        }
        let (Some(x), Some(id)) = (points.get(i), tracks.get(i)) else {
            continue;
        };
        if let Some(slot) = remap.get_mut(i) {
            *slot = kept_points.len();
        }
        kept_points.push(*x);
        kept_tracks.push(*id);
    }
    let kept_obs = obs
        .iter()
        .filter_map(|o| {
            let i = remap.get(o.point).copied().unwrap_or(usize::MAX);
            if i == usize::MAX {
                return None;
            }
            Some(BundleObs { point: i, ..*o })
        })
        .collect();
    (kept_points, kept_tracks, kept_obs)
}

// --- Every frame ------------------------------------------------------------

/// Give every frame in the range a pose: the keyframes from the bundle, the
/// rest resectioned against the cloud, and whatever neither reaches
/// interpolated between its neighbours and labelled as such.
#[allow(clippy::too_many_arguments)]
fn fill_frames(
    set: &TrackSet,
    first: i64,
    last: i64,
    keyframes: &[i64],
    cams: &[BundleCamera],
    segments: &[SolveSegment],
    curve: &FocalCurve,
    points: &[[f64; 3]],
    lookup: &[(u32, usize)],
    centre: [f64; 2],
    settings: &SolveSettings,
) -> Vec<SolvedPose> {
    let mut out: Vec<SolvedPose> = Vec::new();
    let mut world: Vec<[f64; 3]> = Vec::new();
    let mut image: Vec<[f64; 2]> = Vec::new();
    for frame in first..=last {
        let seg = segment_index(segments, frame);
        let focal = curve.value_at(segments, frame);
        let cam = match keyframes.binary_search(&frame) {
            Ok(k) => cams.get(k).copied(),
            Err(_) => {
                world.clear();
                image.clear();
                for track in set.tracks() {
                    if track.state == TrackState::Moving {
                        continue;
                    }
                    let (Ok(i), Some(p)) = (
                        lookup.binary_search_by_key(&track.id, |e| e.0),
                        track.point_at(frame),
                    ) else {
                        continue;
                    };
                    let Some(x) = lookup.get(i).and_then(|e| points.get(e.1)) else {
                        continue;
                    };
                    world.push(*x);
                    image.push([p.x, p.y]);
                }
                resect(
                    &world,
                    &image,
                    focal,
                    centre,
                    curve.ref_at(segments, frame),
                    settings,
                )
            }
        };
        let source = match keyframes.binary_search(&frame) {
            Ok(_) => PoseSource::Keyframe,
            Err(_) => PoseSource::Resection,
        };
        match cam {
            Some(cam) => {
                let error = mean_reprojection(set, frame, &cam, focal, centre, points, lookup);
                out.push(SolvedPose {
                    frame,
                    rotation: cam.rot,
                    position: cam.pos,
                    segment: seg,
                    focal_px: focal,
                    mean_reprojection_px: error,
                    source,
                });
            }
            None => out.push(SolvedPose {
                frame,
                rotation: IDENTITY,
                position: [0.0; 3],
                segment: seg,
                focal_px: focal,
                mean_reprojection_px: 0.0,
                source: PoseSource::Interpolated,
            }),
        }
    }
    interpolate_gaps(&mut out);
    out
}

/// Fill every [`PoseSource::Interpolated`] hole from the solved frames either
/// side — a slerp on the rotation and a straight line on the position. At the
/// ends there is only one neighbour, so the pose is held.
fn interpolate_gaps(poses: &mut [SolvedPose]) {
    let solved: Vec<usize> = poses
        .iter()
        .enumerate()
        .filter(|(_, p)| p.source != PoseSource::Interpolated)
        .map(|(i, _)| i)
        .collect();
    if solved.is_empty() {
        return;
    }
    for i in 0..poses.len() {
        if poses.get(i).map(|p| p.source) != Some(PoseSource::Interpolated) {
            continue;
        }
        let before = solved.iter().rev().find(|&&k| k < i).copied();
        let after = solved.iter().find(|&&k| k > i).copied();
        let (rot, pos) = match (before, after) {
            (Some(a), Some(b)) => {
                let (Some(pa), Some(pb)) = (poses.get(a), poses.get(b)) else {
                    continue;
                };
                let t = (i - a) as f64 / (b - a) as f64;
                let d = so3_log(&mul3(&pb.rotation, &transpose3(&pa.rotation)));
                (
                    mul3(&bundle::so3_exp(scale3(d, t)), &pa.rotation),
                    add3(pa.position, scale3(sub3(pb.position, pa.position), t)),
                )
            }
            (Some(a), None) => match poses.get(a) {
                Some(p) => (p.rotation, p.position),
                None => continue,
            },
            (None, Some(b)) => match poses.get(b) {
                Some(p) => (p.rotation, p.position),
                None => continue,
            },
            (None, None) => continue,
        };
        if let Some(p) = poses.get_mut(i) {
            p.rotation = rot;
            p.position = pos;
        }
    }
}

fn mean_reprojection(
    set: &TrackSet,
    frame: i64,
    cam: &BundleCamera,
    focal: f64,
    centre: [f64; 2],
    points: &[[f64; 3]],
    lookup: &[(u32, usize)],
) -> f64 {
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for track in set.tracks() {
        let (Ok(i), Some(p)) = (
            lookup.binary_search_by_key(&track.id, |e| e.0),
            track.point_at(frame),
        ) else {
            continue;
        };
        let Some(x) = lookup.get(i).and_then(|e| points.get(e.1)) else {
            continue;
        };
        if let Some((q, _)) = bundle::project_point(cam, focal, centre, x) {
            sum += (q[0] - p.x).hypot(q[1] - p.y);
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// Resect one frame against the solved cloud: a normalised DLT for the initial
/// pose, then two rounds of dropping whatever disagrees and refitting, then a
/// Gauss–Newton refinement on the survivors.
///
/// The trimming is docs/impl/tracking.md §4's "RANSAC-lite": phase 2 has already
/// taken the moving tracks out and triangulation has already refused anything
/// that would not sit still, so what is left is a low outlier rate that a trim
/// handles and random sampling would only slow down.
fn resect(
    world: &[[f64; 3]],
    image: &[[f64; 2]],
    focal: f64,
    centre: [f64; 2],
    focal_ref: bundle::FocalRef,
    settings: &SolveSettings,
) -> Option<BundleCamera> {
    if world.len() < settings.min_resection_points || focal <= 0.0 {
        return None;
    }
    let mut keep: Vec<usize> = (0..world.len()).collect();
    let mut cam: Option<BundleCamera> = None;
    for round in 0..3 {
        let sample_world: Vec<[f64; 3]> =
            keep.iter().filter_map(|&i| world.get(i)).copied().collect();
        let sample_image: Vec<[f64; 2]> =
            keep.iter().filter_map(|&i| image.get(i)).copied().collect();
        if sample_world.len() < settings.min_resection_points {
            break;
        }
        let Some(c) = resect_dlt(&sample_world, &sample_image, focal, centre, focal_ref) else {
            break;
        };
        cam = Some(c);
        if round == 2 {
            break;
        }
        let next: Vec<usize> = keep
            .iter()
            .copied()
            .filter(|&i| {
                let (Some(x), Some(p)) = (world.get(i), image.get(i)) else {
                    return false;
                };
                bundle::project_point(&c, focal, centre, x).is_some_and(|(q, _)| {
                    (q[0] - p[0]).hypot(q[1] - p[1]) <= settings.max_reprojection_px
                })
            })
            .collect();
        if next.len() < settings.min_resection_points || next.len() == keep.len() {
            keep = next;
            break;
        }
        keep = next;
    }
    let mut cam = cam?;
    let sample_world: Vec<[f64; 3]> = keep.iter().filter_map(|&i| world.get(i)).copied().collect();
    let sample_image: Vec<[f64; 2]> = keep.iter().filter_map(|&i| image.get(i)).copied().collect();
    if sample_world.len() < settings.min_resection_points {
        return None;
    }
    bundle::refine_pose(
        &mut cam,
        focal,
        &sample_world,
        &sample_image,
        centre,
        settings.huber_px,
        8,
    );
    // A pose is only handed back if it actually explains the frame.
    let mut sum = 0.0f64;
    for (x, p) in sample_world.iter().zip(sample_image.iter()) {
        let (q, _) = bundle::project_point(&cam, focal, centre, x)?;
        sum += (q[0] - p[0]).hypot(q[1] - p[1]);
    }
    if sum / sample_world.len() as f64 > settings.max_reprojection_px {
        return None;
    }
    Some(cam)
}

/// The linear resection: solve for the 3×4 camera outright, then pull the
/// nearest rotation out of its left 3×3.
fn resect_dlt(
    world: &[[f64; 3]],
    image: &[[f64; 2]],
    focal: f64,
    centre: [f64; 2],
    focal_ref: bundle::FocalRef,
) -> Option<BundleCamera> {
    let n = world.len();
    if n < 6 || image.len() < n {
        return None;
    }
    // Condition the world points, exactly as Hartley conditions image points:
    // raw scene coordinates differ in magnitude across the equation and the
    // answer would otherwise be arithmetic noise.
    let mut mean = [0.0f64; 3];
    for x in world {
        mean = add3(mean, *x);
    }
    mean = scale3(mean, 1.0 / n as f64);
    let mut spread = 0.0f64;
    for x in world {
        spread += norm3(sub3(*x, mean));
    }
    spread /= n as f64;
    if !spread.is_finite() || spread < 1e-12 {
        return None;
    }
    let s = 3.0f64.sqrt() / spread;

    let mut ata = [[0.0f64; 12]; 12];
    for (x, p) in world.iter().zip(image.iter()) {
        let xn = scale3(sub3(*x, mean), s);
        let u = (p[0] - centre[0]) / focal;
        let v = (p[1] - centre[1]) / focal;
        let rows = [
            [
                xn[0],
                xn[1],
                xn[2],
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                -u * xn[0],
                -u * xn[1],
                -u * xn[2],
                -u,
            ],
            [
                0.0,
                0.0,
                0.0,
                0.0,
                xn[0],
                xn[1],
                xn[2],
                1.0,
                -v * xn[0],
                -v * xn[1],
                -v * xn[2],
                -v,
            ],
        ];
        for row in &rows {
            for (dst, ri) in ata.iter_mut().zip(row.iter()) {
                for (cell, rj) in dst.iter_mut().zip(row.iter()) {
                    *cell += ri * rj;
                }
            }
        }
    }
    let (_, vecs) = eigen_ascending(&ata);
    let mut p = [[0.0f64; 4]; 3];
    for (i, cell) in p.iter_mut().flatten().enumerate() {
        *cell = vecs[i][0];
    }
    // Undo the conditioning: P = P_normalised · T.
    let mut pw = [[0.0f64; 4]; 3];
    for (r, row) in pw.iter_mut().enumerate() {
        for k in 0..3 {
            row[k] = p[r][k] * s;
        }
        row[3] = p[r][3] - s * (p[r][0] * mean[0] + p[r][1] * mean[1] + p[r][2] * mean[2]);
    }
    let m: Mat3 = [
        [pw[0][0], pw[0][1], pw[0][2]],
        [pw[1][0], pw[1][1], pw[1][2]],
        [pw[2][0], pw[2][1], pw[2][2]],
    ];
    let (m, t) = if geom::det3(&m) < 0.0 {
        (
            [
                [-m[0][0], -m[0][1], -m[0][2]],
                [-m[1][0], -m[1][1], -m[1][2]],
                [-m[2][0], -m[2][1], -m[2][2]],
            ],
            [-pw[0][3], -pw[1][3], -pw[2][3]],
        )
    } else {
        (m, [pw[0][3], pw[1][3], pw[2][3]])
    };
    let mut frobenius = 0.0f64;
    for row in &m {
        for v in row {
            frobenius += v * v;
        }
    }
    let scale = (frobenius / 3.0).sqrt();
    if !scale.is_finite() || scale < 1e-12 {
        return None;
    }
    let rot = closest_rotation(&m)?;
    let t = scale3(t, 1.0 / scale);
    let pos = mat_vec(&transpose3(&rot), [-t[0], -t[1], -t[2]]);
    if pos.iter().any(|c| !c.is_finite()) {
        return None;
    }
    Some(BundleCamera {
        rot,
        pos,
        focal: focal_ref,
    })
}

// --- Small vector arithmetic ---------------------------------------------------

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn norm3(a: [f64; 3]) -> f64 {
    dot3(a, a).sqrt()
}

fn normalise3(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = norm3(a);
    if !n.is_finite() || n < 1e-12 {
        return None;
    }
    Some(scale3(a, 1.0 / n))
}

fn from_columns(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Mat3 {
    [[a[0], b[0], c[0]], [a[1], b[1], c[1]], [a[2], b[2], c[2]]]
}

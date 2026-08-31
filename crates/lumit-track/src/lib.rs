//! `lumit-track` — the 2D track substrate for camera and object tracking
//! (K-415; the algorithms are pinned in docs/impl/tracking.md, and §2 is what
//! this crate builds: phase 1).
//!
//! # In plain terms
//!
//! Before anything can work out where the camera was, something has to follow
//! hundreds of small, distinctive specks of the picture from frame to frame.
//! That is all this crate does, and everything later stands on how well it does
//! it.
//!
//! Three ideas carry the whole thing.
//!
//! *Pick specks worth following.* A patch of sky matches every other patch of
//! sky; a corner matches only itself. The detector scores every pixel by how
//! distinctive its surroundings are and keeps the best few in each square of a
//! grid laid over the frame, so the specks end up spread across the picture
//! instead of piled on the one bright object.
//!
//! *Follow each speck by matching its little square of pixels in the next
//! frame* — allowing the square to stretch and rotate, not merely slide. That
//! matters more than it sounds: a zoom makes every patch grow, and a tracker
//! that can only slide its square loses every feature the moment the lens moves.
//! The stretch it measures is kept, because the amount everything grew between
//! two frames is precisely how a zoom is later detected.
//!
//! *Check the answer, and stop rather than lie.* Every step is run backwards as
//! well as forwards; if following the speck back does not return to where it
//! started, the match was wrong. The patch is also compared against how the
//! speck looked when it was first seen, so a track cannot quietly slide off its
//! feature over a hundred frames. A track that fails either test **ends** — it
//! is never teleported somewhere plausible, because a wrong point is far worse
//! than a missing one for the solve that reads these later.
//!
//! # Thread role and contract
//!
//! Pure computation: no IO, no clocks, no threads of its own, no interior
//! mutability. Frames arrive as borrowed luma planes ([`FramePlane`]), one at a
//! time through [`Tracker::push`], so cancellation is the caller's frame loop
//! (14-ENGINEERING-RULES §1.4) and the crate never owns a long uninterruptible
//! run. Everything is deterministic: fixed iteration orders throughout, no
//! `HashMap` in any path that reaches a result, and all accumulation in `f64`
//! over `f32` pixels, so two runs over the same frames produce the identical
//! [`TrackSet`].
//!
//! Coordinates are **source raster pixels**, per K-248: the tracker runs on the
//! full, unaltered footage, and mapping through comp scale and retimes happens
//! at export.
//!
//! ```
//! use lumit_track::{FramePlane, TrackSettings, Tracker};
//!
//! // Two 64×64 frames of nothing much, one pixel apart.
//! let a: Vec<f32> = (0..64 * 64).map(|i| ((i % 64) as f32 / 64.0)).collect();
//! let mut tracker = Tracker::new(TrackSettings::default());
//! tracker.push(0, FramePlane::new(&a, 64, 64)?, None)?;
//! tracker.push(1, FramePlane::new(&a, 64, 64)?, None)?;
//! let set = tracker.finish();
//! assert_eq!(set.source_size(), (64, 64));
//! # Ok::<(), lumit_track::TrackError>(())
//! ```

mod bundle;
mod detect;
mod exclude;
mod geom;
mod klt;
mod pairs;
mod planar;
mod pyramid;
mod segment;
mod solve;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use exclude::ExclusionMask;
pub use geom::{
    fundamental_eight_point, fundamental_seven_point, homography_dlt, project, sampson_distance,
    transfer_distance, Mat3,
};
pub use pairs::{
    estimate_pair, homography_ransac, select_keyframes, GeometrySettings, PairGeometry, PairVerdict,
};
pub use planar::{
    point_outlines, points_quad, quad_outline, solve_planar, solve_planar_cancellable,
    solve_points, solve_points_cancellable, PlanarError, PlanarFrame, PlanarSettings, PlanarTrack,
    PointSettings, Quad,
};
pub use segment::{
    detect_zoom, segment_dynamic_tracks, SegmentSettings, Segmentation, TrackSplit, ZoomBoundary,
    ZoomKind, ZoomSettings,
};
pub use solve::{
    solve_camera, solve_camera_cancellable, CameraSolve, PoseSource, ScenePoint, SolveError,
    SolveNote, SolveSegment, SolveSettings, SolvedPose,
};

use detect::{BucketGrid, Scratch};
use pyramid::Pyramid;

/// Everything that can go wrong at this crate's boundary. All of it is a caller
/// mistake about sizes or ordering; a *tracking* failure is never an error, it
/// is a track that ends (docs/impl/tracking.md §2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrackError {
    /// The luma slice is too short for the dimensions given.
    #[error("frame plane is {got} samples, but {w}×{h} needs {want}")]
    PlaneSize {
        w: usize,
        h: usize,
        want: usize,
        got: usize,
    },
    /// A frame arrived at a different size from the one the set was started at.
    /// The tracker holds one raster geometry for its whole life.
    #[error("frame {frame} is {w}×{h}, but this tracker was started at {sw}×{sh}")]
    SizeChanged {
        frame: i64,
        w: usize,
        h: usize,
        sw: usize,
        sh: usize,
    },
    /// A flow seed's grid does not match the frame's.
    #[error("flow seed is {w}×{h}, but the frame is {fw}×{fh}")]
    SeedSize {
        w: usize,
        h: usize,
        fw: usize,
        fh: usize,
    },
    /// Frames must be pushed in increasing frame order.
    #[error("frame {got} was pushed after frame {last}; frames must increase")]
    FrameOrder { got: i64, last: i64 },
}

/// A borrowed single-channel luma plane, row-major, in 0..1 encoded luma.
///
/// The tracker takes a borrow and never a buffer: media IO belongs to
/// `lumit-media`, and this crate has no business owning a frame
/// (docs/impl/tracking.md §1).
#[derive(Clone, Copy)]
pub struct FramePlane<'a> {
    luma: &'a [f32],
    w: usize,
    h: usize,
}

impl<'a> FramePlane<'a> {
    /// Borrow `luma` as a `w × h` plane. The slice may be longer than `w · h`
    /// (a padded decode buffer); it may not be shorter.
    ///
    /// # Errors
    ///
    /// [`TrackError::PlaneSize`] when the slice is too short, or either
    /// dimension is zero.
    pub fn new(luma: &'a [f32], w: usize, h: usize) -> Result<Self, TrackError> {
        let want = w.saturating_mul(h);
        if w == 0 || h == 0 || luma.len() < want {
            return Err(TrackError::PlaneSize {
                w,
                h,
                want,
                got: luma.len(),
            });
        }
        Ok(FramePlane { luma, w, h })
    }

    /// Plane width in source raster pixels.
    #[must_use]
    pub fn width(&self) -> usize {
        self.w
    }

    /// Plane height in source raster pixels.
    #[must_use]
    pub fn height(&self) -> usize {
        self.h
    }
}

/// An optional per-pair dense flow field used as the KLT's starting guess
/// (docs/impl/tracking.md §2: "Flow is a *seed*, never a verdict").
///
/// `vectors[y · w + x]` is the displacement in source raster pixels of the pixel
/// at `(x, y)` from the previous frame to this one — the same convention
/// `lumit-flow`'s `FlowField` uses, so a caller hands over `u`/`v` interleaved
/// with no reinterpretation.
#[derive(Clone, Copy)]
pub struct FlowSeed<'a> {
    vectors: &'a [[f32; 2]],
    w: usize,
    h: usize,
}

impl<'a> FlowSeed<'a> {
    /// Borrow `vectors` as a `w × h` flow field.
    ///
    /// # Errors
    ///
    /// [`TrackError::PlaneSize`] when the slice is too short for the
    /// dimensions.
    pub fn new(vectors: &'a [[f32; 2]], w: usize, h: usize) -> Result<Self, TrackError> {
        let want = w.saturating_mul(h);
        if w == 0 || h == 0 || vectors.len() < want {
            return Err(TrackError::PlaneSize {
                w,
                h,
                want,
                got: vectors.len(),
            });
        }
        Ok(FlowSeed { vectors, w, h })
    }

    /// The flow at the nearest sample to `(x, y)`. Nearest rather than bilinear
    /// on purpose: this is a starting guess that a Gauss–Newton solve refines
    /// from, and a quarter-pixel of interpolation buys nothing it does not
    /// already recover.
    fn at(&self, x: f64, y: f64) -> [f64; 2] {
        let xi = (x.round().max(0.0) as usize).min(self.w - 1);
        let yi = (y.round().max(0.0) as usize).min(self.h - 1);
        let v = self.vectors[yi * self.w + xi];
        [f64::from(v[0]), f64::from(v[1])]
    }
}

/// Every knob phase 1 takes. The defaults are docs/impl/tracking.md §2's
/// numbers; a caller with a reason changes one, and the whole run stays
/// deterministic either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSettings {
    /// Detection grid, buckets across × down (§2: 16 × 16).
    pub grid: (usize, usize),
    /// Best-N features per bucket.
    pub per_bucket: usize,
    /// Response floor as a fraction of the frame's best response.
    pub quality: f32,
    /// Minimum spacing in pixels between two features, so "best N in a bucket"
    /// cannot land all N on one corner.
    pub min_separation: f64,
    /// Half-width of the window the Shi–Tomasi normal matrix is summed over.
    pub detect_radius: usize,
    /// KLT window half-width: the window is `2·half_window + 1` on a side
    /// (§2: ~15 px at level 0).
    pub half_window: usize,
    /// Pyramid levels, including level 0 (§2: 3–4). Clamped down where the
    /// frame is too small to support them.
    pub levels: usize,
    /// Gauss–Newton iteration cap per level.
    pub max_iters: usize,
    /// Convergence: stop when the window's corners move less than this, in
    /// pixels at the level being solved.
    pub epsilon: f64,
    /// Forward–backward error a step may not reach, in level-0 pixels
    /// (§2: 0.5 px).
    pub fb_max: f64,
    /// NCC against the track's reference patch below which the track ends.
    pub ncc_floor: f64,
    /// NCC below which the reference patch is refreshed from the current frame:
    /// the track has drifted but is still on its feature, and keeping a stale
    /// reference would end it a few frames later for having changed slowly.
    /// Above this the reference is kept, which is what stops a track walking off
    /// its feature one refresh at a time.
    pub ncc_refresh: f64,
    /// Live tracks in a bucket below which the bucket is detected into again
    /// (§2's re-detection).
    pub redetect_below: usize,
}

impl Default for TrackSettings {
    fn default() -> Self {
        TrackSettings {
            grid: (16, 16),
            per_bucket: 2,
            quality: 0.01,
            min_separation: 6.0,
            detect_radius: 2,
            half_window: 7,
            levels: 4,
            max_iters: 15,
            epsilon: 0.01,
            fb_max: 0.5,
            ncc_floor: 0.8,
            ncc_refresh: 0.95,
            redetect_below: 1,
        }
    }
}

/// What a track is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackState {
    /// Still being followed at the last frame pushed.
    Live,
    /// Verification failed, the feature left the frame, or it entered an
    /// exclusion mask. The points already recorded stand; there will be no more.
    Ended,
    /// Segmented out as belonging to something moving by itself. **Reserved for
    /// phase 2** (docs/impl/tracking.md §3's epipolar segmentation); phase 1
    /// never sets it.
    Moving,
}

/// One observation: where a track was on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackPoint {
    /// Source frame index, as the caller numbered it.
    pub frame: i64,
    /// Source raster pixels (K-248).
    pub x: f64,
    pub y: f64,
}

/// What one frame-to-frame step measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackStep {
    /// The affine warp's 2×2 linear part for this step — how the patch had to
    /// stretch, shear and rotate to match. Kept because the phase-2 zoom-burst
    /// detector reads `log(scale)` out of it; see
    /// [`TrackSet::median_log_scale`].
    pub a: [[f64; 2]; 2],
    /// Normalised cross-correlation against the reference patch after the step.
    pub ncc: f64,
    /// Forward–backward error in level-0 pixels.
    pub fb: f64,
}

impl TrackStep {
    /// `log(scale)` of this step's affine: `½·ln|det A|`, so a patch that grew
    /// by `s` in both directions reads `ln s`. `None` for a degenerate or
    /// mirrored `A`, which has no scale to speak of.
    #[must_use]
    pub fn log_scale(&self) -> Option<f64> {
        let det = self.a[0][0] * self.a[1][1] - self.a[0][1] * self.a[1][0];
        if det.is_finite() && det > 0.0 {
            Some(0.5 * det.ln())
        } else {
            None
        }
    }
}

/// One followed feature: contiguous points from where it was born to where it
/// ended, and one step record between each neighbouring pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Stable within a [`TrackSet`], assigned in detection order and never
    /// reused.
    pub id: u32,
    /// One per frame from [`Self::first_frame`] onward, contiguous and in
    /// order. Never empty.
    pub points: Vec<TrackPoint>,
    /// `steps[i]` is the step from `points[i]` to `points[i + 1]`, so this is
    /// always one shorter than `points`.
    pub steps: Vec<TrackStep>,
    pub state: TrackState,
    /// The track this one was cut from, where phase 2's dynamic segmentation
    /// found a feature that agreed with the camera and then stopped
    /// ([`TrackSet::split_track`]). `None` on every detected track.
    pub parent: Option<u32>,
}

impl Track {
    /// The frame this track was born on.
    #[must_use]
    pub fn first_frame(&self) -> i64 {
        self.points.first().map_or(0, |p| p.frame)
    }

    /// The last frame this track was seen on.
    #[must_use]
    pub fn last_frame(&self) -> i64 {
        self.points.last().map_or(-1, |p| p.frame)
    }

    /// Whether the track has a point on `frame`.
    #[must_use]
    pub fn covers(&self, frame: i64) -> bool {
        !self.points.is_empty() && frame >= self.first_frame() && frame <= self.last_frame()
    }

    /// This track's point on `frame`, if it has one. Points are contiguous, so
    /// this is an index, not a search.
    #[must_use]
    pub fn point_at(&self, frame: i64) -> Option<TrackPoint> {
        if !self.covers(frame) {
            return None;
        }
        let i = usize::try_from(frame - self.first_frame()).ok()?;
        self.points.get(i).copied()
    }

    /// The step leaving `frame` — the one that took this track from `frame` to
    /// `frame + 1`.
    #[must_use]
    pub fn step_from(&self, frame: i64) -> Option<TrackStep> {
        if !self.covers(frame) {
            return None;
        }
        let i = usize::try_from(frame - self.first_frame()).ok()?;
        self.steps.get(i).copied()
    }
}

/// One track's position on two frames — what the phase-2 two-view geometry
/// consumes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Correspondence {
    pub id: u32,
    pub from: [f64; 2],
    pub to: [f64; 2],
}

/// The whole result of a run: every track, live and ended, in id order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackSet {
    tracks: Vec<Track>,
    width: usize,
    height: usize,
}

impl TrackSet {
    /// Every track, in id order — which is detection order, which is
    /// deterministic (§2: bucket row-major, response descending, ties by
    /// `(y, x)`).
    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The track with `id`.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<&Track> {
        // Ids are assigned in ascending order and never reused, so the vector is
        // sorted by id and this is a binary search, not a scan.
        let i = self.tracks.binary_search_by_key(&id, |t| t.id).ok()?;
        self.tracks.get(i)
    }

    /// The raster the coordinates are in, in source pixels.
    #[must_use]
    pub fn source_size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The first and last frame any track has a point on, or `None` for an empty
    /// set.
    #[must_use]
    pub fn frame_range(&self) -> Option<(i64, i64)> {
        let mut range: Option<(i64, i64)> = None;
        for t in &self.tracks {
            if t.points.is_empty() {
                continue;
            }
            let (a, b) = (t.first_frame(), t.last_frame());
            range = Some(match range {
                None => (a, b),
                Some((lo, hi)) => (lo.min(a), hi.max(b)),
            });
        }
        range
    }

    /// Every track alive across the whole of `from..=to`, in id order.
    pub fn tracks_over(&self, from: i64, to: i64) -> impl Iterator<Item = &Track> {
        self.tracks
            .iter()
            .filter(move |t| t.covers(from) && t.covers(to))
    }

    /// The correspondences between two frames: every track present on both, in
    /// id order.
    #[must_use]
    pub fn correspondences(&self, from: i64, to: i64) -> Vec<Correspondence> {
        self.tracks_over(from, to)
            .filter_map(|t| {
                let a = t.point_at(from)?;
                let b = t.point_at(to)?;
                Some(Correspondence {
                    id: t.id,
                    from: [a.x, a.y],
                    to: [b.x, b.y],
                })
            })
            .collect()
    }

    /// The median `log(scale)` over every track stepping from `frame` to
    /// `frame + 1`, read out of the steps' affine matrices.
    ///
    /// This is the zoom-burst detector's input (docs/impl/tracking.md §3): a
    /// lens that jumped between two frames shows as a burst in this number,
    /// while a steady non-zero value is a zoom ramp. Median rather than mean
    /// because a handful of tracks on a moving object must not move the answer.
    /// `None` where no track has a usable step there.
    #[must_use]
    pub fn median_log_scale(&self, frame: i64) -> Option<f64> {
        let mut v: Vec<f64> = self
            .tracks
            .iter()
            .filter_map(|t| t.step_from(frame))
            .filter_map(|s| s.log_scale())
            .collect();
        geom::median(&mut v)
    }

    /// How many tracks were still live at the last frame pushed.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.tracks
            .iter()
            .filter(|t| t.state == TrackState::Live)
            .count()
    }

    /// This pair of frames' two-view geometry — the convenience over
    /// [`estimate_pair`] that pulls the correspondences itself.
    #[must_use]
    pub fn pair_geometry(
        &self,
        from: i64,
        to: i64,
        settings: &GeometrySettings,
    ) -> Option<PairGeometry> {
        estimate_pair(
            &self.correspondences(from, to),
            self.source_size(),
            from,
            to,
            settings,
        )
    }

    /// Drop everything after `last_frame`, leaving a set that describes only
    /// the span up to and including it.
    ///
    /// What a run that stopped part-way hands on. The frames after a tracking
    /// failure do not carry a poorer answer, they carry no answer: leaving them
    /// in would have keyframe selection reach for a pair nothing spans and the
    /// solve place cameras on frames nothing measured. A track that ends up
    /// with no points at all is dropped; one that is cut short is
    /// [`TrackState::Ended`], because that is now where it stops. Ids and their
    /// order are otherwise untouched.
    pub fn truncate(&mut self, last_frame: i64) {
        for t in &mut self.tracks {
            // Points are in ascending frame order, so the ones to keep are a
            // prefix and the count of them is the whole of the arithmetic.
            let keep = t
                .points
                .iter()
                .take_while(|p| p.frame <= last_frame)
                .count();
            if keep == t.points.len() {
                continue;
            }
            t.points.truncate(keep);
            // `steps[i]` measures the motion out of `points[i]`, so a track
            // with `n` points has `n - 1` steps — the invariant `split_track`
            // keeps for the same reason.
            t.steps.truncate(keep.saturating_sub(1));
            // Ended, because that is now where it stops — but never over
            // `Moving`, which is a verdict about the track rather than about
            // its extent, and losing it would put a mover back in the solve.
            if t.state == TrackState::Live {
                t.state = TrackState::Ended;
            }
        }
        self.tracks.retain(|t| !t.points.is_empty());
    }

    /// Cut a track in two after `after_frame`, handing the suffix a fresh id.
    ///
    /// The original keeps its id and every point up to and including
    /// `after_frame`, and is left [`TrackState::Ended`] because it now stops
    /// there. The new track carries the points from `after_frame + 1` onwards,
    /// inherits the original's state, and records the original as its
    /// [`Track::parent`]. The one step that straddled the cut belongs to
    /// neither half and is dropped — it measured a frame-to-frame motion that
    /// the split has just declared was two different things.
    ///
    /// Returns the new id, or `None` when there is no such track or the cut
    /// would leave a half empty.
    pub fn split_track(&mut self, id: u32, after_frame: i64) -> Option<u32> {
        let index = self.tracks.binary_search_by_key(&id, |t| t.id).ok()?;
        // Ids ascend and are never reused, so the last track holds the largest.
        let new_id = self.tracks.last()?.id.checked_add(1)?;
        let track = self.tracks.get_mut(index)?;
        if !track.covers(after_frame) || after_frame >= track.last_frame() {
            return None;
        }
        let cut = usize::try_from(after_frame - track.first_frame()).ok()?;
        // Validated before anything is moved: a refused split must leave the
        // store exactly as it found it.
        if cut + 1 >= track.points.len() {
            return None;
        }
        let points = track.points.split_off(cut + 1);
        let mut steps = track.steps.split_off(cut.min(track.steps.len()));
        if !steps.is_empty() {
            steps.remove(0);
        }
        let state = track.state;
        track.state = TrackState::Ended;
        self.tracks.push(Track {
            id: new_id,
            points,
            steps,
            state,
            parent: Some(id),
        });
        Some(new_id)
    }
}

/// End the track at `index` and drop it from the live list (the `false` this
/// hands back to `retain_mut`). Every phase-1 failure — a singular solve, an
/// exclusion mask, forward–backward, NCC — lands here: a track that fails is
/// never teleported, it stops (docs/impl/tracking.md §2).
fn end_track(tracks: &mut [Track], index: usize) -> bool {
    if let Some(t) = tracks.get_mut(index) {
        t.state = TrackState::Ended;
    }
    false
}

/// A live track's working state, alongside its entry in [`Tracker::tracks`].
struct LiveTrack {
    index: usize,
    pos: [f64; 2],
    /// Last step's displacement — the constant-velocity prior (§2's seeding).
    vel: [f64; 2],
    /// The patch NCC is measured against, from when the track was born or last
    /// refreshed.
    reference: Vec<f64>,
}

/// The phase-1 tracker: push frames in order, take the [`TrackSet`] at the end.
///
/// One frame per call is the cancellation seam (14-ENGINEERING-RULES §1.4) —
/// the caller's loop checks its epoch token between frames, and no single call
/// runs long enough to need one inside.
pub struct Tracker {
    settings: TrackSettings,
    masks: Vec<ExclusionMask>,
    prev: Pyramid,
    cur: Pyramid,
    /// The response pass's frame-sized buffers, allocated once (§5).
    scratch: Scratch,
    tracks: Vec<Track>,
    live: Vec<LiveTrack>,
    /// Survivors of the last [`Tracker::advance`], counted before re-detection
    /// refills the buckets — see [`Tracker::carried_count`].
    carried: usize,
    next_id: u32,
    last_frame: Option<i64>,
    size: Option<(usize, usize)>,
}

impl Tracker {
    /// A tracker with no exclusion masks.
    #[must_use]
    pub fn new(settings: TrackSettings) -> Self {
        Tracker {
            settings,
            masks: Vec::new(),
            prev: Pyramid::new(),
            cur: Pyramid::new(),
            scratch: Scratch::default(),
            tracks: Vec::new(),
            live: Vec::new(),
            carried: 0,
            next_id: 0,
            last_frame: None,
            size: None,
        }
    }

    /// Exclusion regions: no feature is born inside one, and a track that
    /// wanders into one ends (§2's mask rule).
    #[must_use]
    pub fn with_masks(mut self, masks: Vec<ExclusionMask>) -> Self {
        self.set_masks(masks);
        self
    }

    /// Replace the exclusion regions between frames.
    ///
    /// A mask drawn round a moving object is keyframed to follow it, so the
    /// regions are not one fixed set for a whole run: the caller re-flattens
    /// the shapes at each frame's own moment and hands them over here before
    /// pushing that frame. Everything the regions decide — where a feature may
    /// be born, and whether a carried track has wandered somewhere it must not
    /// be — is read out of this field at push time, so a swap between frames
    /// takes effect from the next push and disturbs nothing already tracked.
    pub fn set_masks(&mut self, masks: Vec<ExclusionMask>) {
        self.masks = masks;
    }

    /// How many tracks are live right now — a caller's progress read.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// How many tracks **carried across** the frame just pushed: the survivors
    /// of the step out of the frame before it, counted before re-detection
    /// refilled the emptied buckets. Zero before the second frame, which
    /// nothing carried into.
    ///
    /// This, and not [`Tracker::live_count`], is the measure of whether the
    /// chain of correspondence is intact. Live count recovers within one frame
    /// however badly a shot fails — detection seeds fresh features into
    /// whatever buckets emptied, and a dim or blurred frame is not refused
    /// features because the quality floor is relative to that frame's own best
    /// (§2). So live count says how many specks are being followed; only this
    /// says how many of them tie this frame to the last one, which is the only
    /// thing any later phase can use.
    #[must_use]
    pub fn carried_count(&self) -> usize {
        self.carried
    }

    /// Take the next frame. `frame` is the source frame index; frames must be
    /// pushed in increasing order and every frame must be the same size.
    ///
    /// `seed` is an optional dense flow field for *this* pair, which wins over
    /// the constant-velocity prior as the KLT's starting guess.
    ///
    /// # Errors
    ///
    /// [`TrackError::FrameOrder`] out of order, [`TrackError::SizeChanged`] on a
    /// different raster, [`TrackError::SeedSize`] on a mismatched seed grid.
    pub fn push(
        &mut self,
        frame: i64,
        plane: FramePlane<'_>,
        seed: Option<FlowSeed<'_>>,
    ) -> Result<(), TrackError> {
        if let Some(last) = self.last_frame {
            if frame <= last {
                return Err(TrackError::FrameOrder { got: frame, last });
            }
        }
        match self.size {
            None => self.size = Some((plane.w, plane.h)),
            Some((sw, sh)) if sw == plane.w && sh == plane.h => {}
            Some((sw, sh)) => {
                return Err(TrackError::SizeChanged {
                    frame,
                    w: plane.w,
                    h: plane.h,
                    sw,
                    sh,
                })
            }
        }
        if let Some(s) = seed {
            if s.w != plane.w || s.h != plane.h {
                return Err(TrackError::SeedSize {
                    w: s.w,
                    h: s.h,
                    fw: plane.w,
                    fh: plane.h,
                });
            }
        }

        let levels = Pyramid::usable_levels(
            plane.w,
            plane.h,
            self.settings.levels,
            2 * self.settings.half_window + 4,
        );

        if self.last_frame.is_none() {
            self.prev.fill(plane.luma, plane.w, plane.h, levels);
            self.last_frame = Some(frame);
            self.seed_features(frame, true);
            return Ok(());
        }

        self.cur.fill(plane.luma, plane.w, plane.h, levels);
        self.advance(frame, seed);
        // Read here, between the carry and the re-detection: after
        // `seed_features` the emptied buckets are full again and the collapse
        // is invisible.
        self.carried = self.live.len();
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.last_frame = Some(frame);
        self.seed_features(frame, false);
        Ok(())
    }

    /// Finish and hand over the tracks.
    #[must_use]
    pub fn finish(self) -> TrackSet {
        let (width, height) = self.size.unwrap_or((0, 0));
        TrackSet {
            tracks: self.tracks,
            width,
            height,
        }
    }

    fn grid(&self) -> BucketGrid {
        let (w, h) = self.size.unwrap_or((1, 1));
        BucketGrid {
            gx: self.settings.grid.0.max(1).min(w.max(1)),
            gy: self.settings.grid.1.max(1).min(h.max(1)),
            w: w.max(1),
            h: h.max(1),
        }
    }

    /// Detect features into empty (or, on the first frame, all) buckets of
    /// `self.prev`, which by the time this runs is the frame just pushed.
    fn seed_features(&mut self, frame: i64, first: bool) {
        let grid = self.grid();
        let per = self.settings.per_bucket;
        let mut need: Vec<(usize, usize)> = Vec::new();
        if first {
            need.extend((0..grid.count()).map(|b| (b, per)));
        } else {
            let mut census = vec![0usize; grid.count()];
            for l in &self.live {
                if let Some(b) = grid.index_of(l.pos[0], l.pos[1]) {
                    census[b] += 1;
                }
            }
            let floor = self.settings.redetect_below.min(per);
            for (b, &c) in census.iter().enumerate() {
                if c < floor && per > c {
                    need.push((b, per - c));
                }
            }
            if need.is_empty() {
                return;
            }
        }
        let Some(base) = self.prev.levels.first() else {
            return;
        };
        detect::response_map_into(&mut self.scratch, base, self.settings.detect_radius);
        let resp = &self.scratch.resp;
        // The floor is relative to this frame's best response (§2), so a dim
        // frame is not simply refused features.
        let best = resp.iter().copied().fold(0.0f32, f32::max);
        let floor = best * self.settings.quality;
        let occupied: Vec<[f64; 2]> = self.live.iter().map(|l| l.pos).collect();
        let half = self.settings.half_window as i64;
        let found = detect::detect(
            resp,
            &grid,
            &need,
            floor,
            self.settings.half_window + 2,
            self.settings.min_separation,
            &occupied,
            &self.masks,
        );
        for p in found {
            let Some(reference) = klt::patch(base, p, half) else {
                continue;
            };
            let id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            self.tracks.push(Track {
                id,
                points: vec![TrackPoint {
                    frame,
                    x: p[0],
                    y: p[1],
                }],
                steps: Vec::new(),
                state: TrackState::Live,
                parent: None,
            });
            self.live.push(LiveTrack {
                index: self.tracks.len() - 1,
                pos: p,
                vel: [0.0, 0.0],
                reference,
            });
        }
    }

    /// Carry every live track from `prev` to `cur`, verifying each
    /// (§2: forward–backward, then NCC; a failure ends the track).
    fn advance(&mut self, frame: i64, seed: Option<FlowSeed<'_>>) {
        let Tracker {
            settings,
            masks,
            prev,
            cur,
            tracks,
            live,
            ..
        } = self;
        let half = settings.half_window as i64;
        // `retain_mut` keeps the survivors in order, so ids stay ascending and
        // the next frame walks them in the same sequence (§1's determinism).
        live.retain_mut(|l| {
            let start = seed.map_or(l.vel, |f| f.at(l.pos[0], l.pos[1]));
            let Some(fwd) = klt::solve(
                prev,
                cur,
                l.pos,
                start,
                half,
                settings.max_iters,
                settings.epsilon,
            ) else {
                return end_track(tracks, l.index);
            };
            if exclude::excluded(masks, fwd.pos[0], fwd.pos[1]) {
                return end_track(tracks, l.index);
            }
            // Forward–backward: follow the settled patch back and see whether it
            // returns to where it started. Seeded with the negated forward
            // displacement, so the backward solve starts from the right answer
            // and the test measures drift rather than the search's luck.
            let back_seed = [l.pos[0] - fwd.pos[0], l.pos[1] - fwd.pos[1]];
            let Some(back) = klt::solve(
                cur,
                prev,
                fwd.pos,
                back_seed,
                half,
                settings.max_iters,
                settings.epsilon,
            ) else {
                return end_track(tracks, l.index);
            };
            let fb = (back.pos[0] - l.pos[0]).hypot(back.pos[1] - l.pos[1]);
            if fb >= settings.fb_max {
                return end_track(tracks, l.index);
            }
            let Some(now) = cur
                .levels
                .first()
                .and_then(|p| klt::patch(p, fwd.pos, half))
            else {
                return end_track(tracks, l.index);
            };
            let ncc = klt::ncc(&l.reference, &now);
            if ncc < settings.ncc_floor {
                return end_track(tracks, l.index);
            }
            if ncc < settings.ncc_refresh {
                l.reference = now;
            }
            let Some(t) = tracks.get_mut(l.index) else {
                return false;
            };
            t.points.push(TrackPoint {
                frame,
                x: fwd.pos[0],
                y: fwd.pos[1],
            });
            t.steps.push(TrackStep { a: fwd.a, ncc, fb });
            l.vel = [fwd.pos[0] - l.pos[0], fwd.pos[1] - l.pos[1]];
            l.pos = fwd.pos;
            true
        });
    }
}

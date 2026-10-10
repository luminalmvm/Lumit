//! The camera-track analysis job, its sidecar cache, and the store the render
//! path reads (docs/impl/tracking.md §5b).
//!
//! # In plain terms
//!
//! Pressing **Analyse** on a Camera track effect starts one long piece of work:
//! decode every frame of the clip's *source file*, follow hundreds of specks
//! through it, and solve for where the camera was. That takes minutes on a real
//! shot, so three things have to be true and all three are arranged here.
//!
//! **It happens somewhere else.** The analysis runs on its own thread — never a
//! pool worker, for the same reason decoding never does (docs/05 §2): it holds a
//! decoder open and stalls unpredictably on seeks, and a pool worker doing that
//! starves every frame behind it. You keep editing while it runs.
//!
//! **It can be stopped, and it says how far it has got.** The frame loop is the
//! cancellation seam — one flag, checked between frames — and the solve has its
//! own ([`lumit_track::solve_camera_cancellable`]). Progress is a value anyone
//! can read, not a callback anyone has to hold.
//!
//! **It stops where the pictures stop carrying the answer.** Not every shot
//! can be followed all the way to its end — the lens racks, the frame whites
//! out, the camera whips — and when the specks stop crossing from one frame to
//! the next there is nothing after that point to solve *against* anything
//! before it. The run ends there, the span that worked is solved and kept, and
//! the result says how far it got ([`Solved::is_partial`]). Half a shot
//! honestly measured is worth having; a whole one with an invented tail is not.
//!
//! **It is not done twice.** The answer depends on the file's bytes and the
//! analysis settings and on nothing else, so it is written to the `track/`
//! sidecar under a name made of exactly those two things. The next session, the
//! next project, and the copy of the project on another drive all read it back
//! instead of tracking again. The solve is deterministic, so a rebuild and a
//! cache hit are the same bits — asserted, not assumed. Deleting the folder at
//! any moment costs a re-analysis and nothing else.
//!
//! # What the store hands back
//!
//! `lumit-core` describes solved cameras through [`CameraSolveStore`] and knows
//! nothing about the tracker. The conversion from the tracker's terms —
//! world-to-camera rotation, a camera centre in solve units, a focal length in
//! source pixels — into Lumit's [`CameraPose`] is therefore *here*, next to the
//! solve it converts, and it is the interesting part of this file. It is derived
//! rather than guessed: the tracker projects a world point as
//! `centre + focal · p.xy / p.z` with `p = R(P − C)`, the compositor projects it
//! through [`lumit_gpu::composite::camera_matrix`], and setting those two equal
//! gives exactly one answer for zoom, position and rotation (see
//! [`to_camera_pose`]). The test compares against that real matrix rather than
//! against a re-derivation of it.
//!
//! **One assumption, stated:** the solve's world is read as comp pixels with the
//! footage at its own raster size, which is exact for the ordinary case of a comp
//! made from the shot and off by the size ratio otherwise. The store is asked
//! about a *media item*, not about a comp — one clip can be in many comps — so
//! there is nowhere else for the reference frame to come from.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use lumit_core::fx::effects::bake_points;
use lumit_core::fx::effects::track_points::{self, Analysis, Baked, Followed};
use lumit_core::model::{
    CameraPose, Composition, Document, EffectInstance, EffectValue, Fingerprint, Layer, LayerKind,
};
use lumit_core::track::{
    CameraSolveStore, LinkedPose, PlanarTrackStore, Quad, SolvedRange, CAMERA_TRACK, PLANAR_TRACK,
};
use lumit_track::{
    detect_zoom, point_outlines, points_quad, quad_outline, segment_dynamic_tracks,
    select_keyframes, solve_camera_cancellable, solve_planar_cancellable, solve_points_cancellable,
    BlobSettings, BlobTracker, CameraSolve, ExclusionMask, FramePlane, GeometrySettings, Mat3,
    PlanarError, PlanarSettings, PlanarTrack, PointSettings, SegmentSettings, SolveError,
    SolveSettings, SolvedPose, TrackError, TrackSettings, Tracker, ZoomSettings,
};
use uuid::Uuid;

use crate::sidecar;

// ---------------------------------------------------------------------------
// What an analysis is asked for
// ---------------------------------------------------------------------------

/// The settings that change what an analysis finds, read off the Camera track
/// effect. Part of the cache key: change one and it is a different solve, not a
/// stale one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisSettings {
    /// Feature density, as the effect's Choice index (docs/08 §3.85).
    pub density: u32,
    /// Whether the layer's masks exclude regions from tracking.
    pub use_masks: bool,
    /// A Planar track's **Follow** index; zero — the surface — for a
    /// Camera track, which has no such row. It is a setting rather than a field
    /// of [`JobKind`] because it changes what the analysis *finds*, so it
    /// belongs in the cache key: two runs over the same boxes asking different
    /// questions are two answers.
    pub follow: u32,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        AnalysisSettings {
            density: lumit_core::fx::effects::camera_track::DENSITY_DEFAULT,
            use_masks: true,
            follow: lumit_core::fx::effects::planar_track::FOLLOW_DEFAULT,
        }
    }
}

impl AnalysisSettings {
    /// Read the settings off one Camera track instance. A parameter the
    /// instance does not carry — an older project, a hand-edited file — reads
    /// as the effect's own default rather than failing (docs/14 §4).
    #[must_use]
    pub fn of(fx: &EffectInstance) -> Self {
        let d = Self::default();
        AnalysisSettings {
            density: match fx.param("density") {
                Some(EffectValue::Choice(v)) => *v,
                _ => d.density,
            },
            use_masks: match fx.param("use_masks") {
                Some(EffectValue::Bool(v)) => *v,
                _ => d.use_masks,
            },
            follow: match fx.param(lumit_core::track::FOLLOW_PARAM) {
                Some(EffectValue::Choice(v)) => *v,
                _ => d.follow,
            },
        }
    }

    /// Whether this is one of the point options rather than the surface.
    fn points(self) -> bool {
        self.follow != lumit_core::fx::effects::planar_track::FOLLOW_DEFAULT
    }

    fn tracker(self) -> TrackSettings {
        let (across, down, per_bucket) =
            lumit_core::fx::effects::camera_track::density(self.density);
        TrackSettings {
            grid: (across, down),
            // A point track is confined to one or two small boxes, and the
            // detection grid is over the whole frame: a box the size of a badge
            // lands inside a bucket or two, so the ordinary two-per-bucket would
            // leave a point standing on two features. Everything outside the
            // boxes is excluded, so raising the count costs nothing anywhere
            // else — the buckets the boxes do not touch have nothing to detect.
            //
            // ponytail: one number rather than a grid sized to the region; a
            // per-region detector is the upgrade if a very small box starves.
            per_bucket: if self.points() {
                per_bucket.max(32)
            } else {
                per_bucket
            },
            ..TrackSettings::default()
        }
    }
}

/// The exclusion regions of one run, **as they move**.
///
/// The layer's own masks, kept as masks rather than as one flattened set, so the
/// analysis can ask what shape each of them has at each frame's own moment. A
/// mask drawn round a mover is keyframed to follow it, and a tracker that took
/// the shape it starts on would exclude the wrong part of every frame after the
/// first.
///
/// **The factor is usually one** (§5b's seventh deviation): a mask's vertices are
/// in the layer's own pixel coordinates, and for a footage layer those *are* the
/// source raster, which is what the tracker works in, so nothing
/// converts. A Precomp layer analysed at a reduced raster is the one exception,
/// and it hands its render scale in as `to_analysis`.
///
/// **The clock is the source's own.** Source frame `n` is read at layer time
/// `n / fps`, which is exact for the ordinary case and is what the old
/// flatten-at-zero was the `n = 0` instance of. A retime between layer time and
/// source time is not inverted — the analysis is of the *file*, from its first
/// frame at its own rate, and one clip is in many layers with many retimes,
/// only one of which could be honoured.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskTrack {
    masks: Vec<lumit_core::mask::Mask>,
    /// Layer pixels → analysis pixels. One for footage; the render scale for a
    /// Precomp layer analysed at a reduced raster.
    to_analysis: f64,
    /// A **Planar track's** quad, or a point track's one or two search boxes,
    /// as the outlines the tracker must stay inside. They belong
    /// here and not in [`AnalysisSettings`] because they are exactly what an
    /// exclusion region is — a shape deciding where features may live — and
    /// putting them here means they are hashed into the analysis key with the
    /// rest of the geometry rather than needing their own arrangement.
    ///
    /// Several outlines are **one** region with several contours, never several
    /// regions: regions are unioned as exclusions, so two inverted boxes would
    /// forbid everything outside either of them.
    bounds: Vec<Vec<[f64; 2]>>,
}

impl Default for MaskTrack {
    fn default() -> Self {
        MaskTrack {
            masks: Vec::new(),
            to_analysis: 1.0,
            bounds: Vec::new(),
        }
    }
}

impl MaskTrack {
    /// The exclusion regions `layer` contributes, or none when the effect's
    /// *Use masks* is off. `to_analysis` scales the layer's own pixels into the
    /// ones the analysis reads — one for footage.
    #[must_use]
    pub fn of(layer: &Layer, settings: AnalysisSettings, to_analysis: f64) -> Self {
        MaskTrack {
            masks: if settings.use_masks {
                layer.masks.clone()
            } else {
                Vec::new()
            },
            to_analysis,
            bounds: Vec::new(),
        }
    }

    /// The same regions, plus a **boundary** the tracker may not leave — a
    /// Planar track's quad. An *inverted* region is what "work only
    /// inside this shape" already means, so the quad needs no mechanism
    /// of its own.
    #[must_use]
    pub fn within(self, outline: Vec<[f64; 2]>) -> Self {
        self.within_all(vec![outline])
    }

    /// The same, for a boundary made of several disjoint pieces — a two-point
    /// track's search boxes. They become one region of several contours,
    /// tested even-odd together, which for disjoint boxes reads as "inside any
    /// of them".
    #[must_use]
    pub fn within_all(mut self, outlines: Vec<Vec<[f64; 2]>>) -> Self {
        self.bounds = outlines;
        self
    }

    /// The regions as they stand at layer time `t`.
    #[must_use]
    pub fn at(&self, t: f64) -> Vec<ExclusionMask> {
        // The boundary first, so a run with no masks at all still has it.
        let bounds = (!self.bounds.is_empty()).then(|| {
            ExclusionMask::from_contours(
                self.bounds
                    .iter()
                    .map(|outline| {
                        outline
                            .iter()
                            .map(|p| [p[0] * self.to_analysis, p[1] * self.to_analysis])
                            .collect()
                    })
                    .collect(),
                true,
            )
        });
        bounds
            .into_iter()
            .chain(
                self.masks
                    .iter()
                    .map(|m| ExclusionMask::from_mask(m, t, self.to_analysis)),
            )
            .collect()
    }

    /// Whether any of the masks changes shape over the run. False — the
    /// ordinary case — lets the analysis flatten once for the whole clip
    /// instead of once per frame.
    #[must_use]
    pub fn animated(&self) -> bool {
        self.masks.iter().any(|m| m.path_keys.len() > 1)
    }

    /// Feed the geometry into the analysis key.
    ///
    /// The still shapes go in exactly as they always did, so every solve already
    /// in the sidecar for an unanimated mask keeps the name it was filed under.
    /// A **keyed** path then adds its own keys — each one's moment, its shape,
    /// and the eases either side of it, which are what decide every shape in
    /// between. Without that the key would name only the shape at zero, and an
    /// animation edited after an analysis would read its own stale solve back
    /// as if nothing had changed.
    fn feed(&self, h: &mut blake3::Hasher) {
        // The boundary is in `at`, so a quad moving changes the key and a
        // re-drawn quad cannot read the old quad's answer back.
        for region in self.at(0.0) {
            feed_region(h, &region);
        }
        for mask in &self.masks {
            if mask.path_keys.len() < 2 {
                continue;
            }
            h.update(b"mask-anim/");
            for k in &mask.path_keys {
                h.update(&k.time.to_f64().to_le_bytes());
                feed_side(h, k.interp_in);
                feed_side(h, k.interp_out);
                feed_region(
                    h,
                    &ExclusionMask::from_mask(mask, k.time.to_f64(), self.to_analysis),
                );
            }
        }
    }
}

/// One flattened region's contribution to the key.
fn feed_region(h: &mut blake3::Hasher, region: &ExclusionMask) {
    let (contours, inverted) = region.outlines();
    h.update(b"mask/");
    h.update(&[u8::from(inverted)]);
    for points in contours {
        for point in points {
            h.update(&point[0].to_le_bytes());
            h.update(&point[1].to_le_bytes());
        }
    }
}

/// One keyframe side's contribution: which of the four it is, and the two
/// numbers that shape it. `Auto` carries a remembered ease that does not
/// evaluate, and it goes in anyway — it is cheap, and a key that ignored a
/// stored field would have to be revisited the day the field starts mattering.
fn feed_side(h: &mut blake3::Hasher, side: lumit_core::anim::SideInterp) {
    use lumit_core::anim::SideInterp;
    let (tag, speed, influence) = match side {
        SideInterp::Hold => (0u8, 0.0, 0.0),
        SideInterp::Linear => (1, 0.0, 0.0),
        SideInterp::Bezier { speed, influence } => (2, speed, influence),
        SideInterp::Auto {
            clamped,
            speed,
            influence,
        } => (if clamped { 3 } else { 4 }, speed, influence),
    };
    h.update(&[tag]);
    h.update(&speed.to_le_bytes());
    h.update(&influence.to_le_bytes());
}

/// What names one analysis: the file's content, the settings it was analysed
/// under, and this module's own format version.
///
/// A blake3 of all three, so the sidecar is one flat folder of fixed-length
/// names — the media index's shape, for the media index's reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalysisKey([u8; 32]);

impl AnalysisKey {
    /// The key for `fingerprint` analysed at `settings` with `masks`.
    ///
    /// The masks are hashed as geometry, not as ids: two different masks that
    /// flatten to the same outline exclude the same pixels and deserve the same
    /// cached answer, and renaming one deserves not to throw it away. What
    /// exactly goes in — including the animation, since the analysis honours it
    /// — is [`MaskTrack::feed`].
    #[must_use]
    pub fn new(fingerprint: &Fingerprint, settings: AnalysisSettings, masks: &MaskTrack) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"lumit-track/");
        h.update(&FORMAT_VERSION.to_le_bytes());
        h.update(&fingerprint.size.to_le_bytes());
        h.update(fingerprint.head_tail_hash.as_bytes());
        h.update(&settings.density.to_le_bytes());
        h.update(&[u8::from(settings.use_masks)]);
        // Fed only when it is not the surface, so every answer already in a
        // sidecar keeps the name it was filed under. What it guards against is
        // narrow but real: a one-point box and a quad drawn to the same outline
        // are the same geometry asked two different questions.
        if settings.follow != 0 {
            h.update(b"follow/");
            h.update(&settings.follow.to_le_bytes());
        }
        masks.feed(&mut h);
        AnalysisKey(*h.finalize().as_bytes())
    }

    /// The key for a Bake points bake: the instance it was baked for and
    /// nothing else, so one instance has one file and a new bake replaces the
    /// old. What it was baked from is inside the file, where Auto checks it,
    /// and Read hands the file on whatever it says.
    #[must_use]
    pub fn bake(effect: Uuid) -> Self {
        let mut h = blake3::Hasher::new();
        // The 2 is the layout of the file. A file in an older layout has
        // another name, so it is never asked for.
        h.update(b"lumit-track/bake/2/");
        h.update(&FORMAT_VERSION.to_le_bytes());
        h.update(effect.as_bytes());
        AnalysisKey(*h.finalize().as_bytes())
    }

    /// The key for a Track points analysis: the file's content and every row
    /// the analysis reads. Which footage item shows the file is left out, so
    /// two projects on the same clip share one analysis.
    #[must_use]
    pub fn points(fingerprint: &Fingerprint, analysis: &Analysis) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"lumit-track/points/");
        h.update(&FORMAT_VERSION.to_le_bytes());
        h.update(&fingerprint.size.to_le_bytes());
        h.update(fingerprint.head_tail_hash.as_bytes());
        h.update(&[u8::from(analysis.blobs), u8::from(analysis.invert)]);
        h.update(&analysis.density.to_le_bytes());
        for v in [
            analysis.quality,
            analysis.spacing,
            analysis.window,
            analysis.threshold,
            analysis.min_area,
            analysis.max_area,
            analysis.max_distance,
        ] {
            h.update(&v.to_le_bytes());
        }
        AnalysisKey(*h.finalize().as_bytes())
    }

    fn file_name(&self) -> String {
        let mut name = String::with_capacity(68);
        for byte in self.0 {
            name.push_str(&format!("{byte:02x}"));
        }
        name.push_str(".ltrk");
        name
    }
}

/// One frame of a clip, as brightness — everything the tracker reads.
///
/// A trait because a tracked source is not always a file. [`MediaLuma`] decodes
/// one; [`CompLuma`] *renders* a nested composition, which is what a Camera
/// track on a Precomp layer needs; and the engine tests feed a
/// synthetic scene with a camera path they wrote down, since asking them to
/// encode a video first would be measuring ffmpeg. Whichever it is, it is opened
/// on the analysis thread and never on the caller's.
pub trait LumaFrames {
    /// `(frames, width, height, frames per second)`.
    fn info(&self) -> (usize, u32, u32, f64);
    /// Frame `n` as row-major 0..1 luma, `width · height` long. `None` ends the
    /// run early — a clip that stops decoding part-way is tracked as far as it
    /// went, which is more useful than nothing.
    fn luma(&mut self, n: usize) -> Option<Vec<f32>>;
    /// Analysis pixels per source pixel. One for everything that hands over the
    /// source at its own size; less than one for a source rendered smaller than
    /// it really is, whose solve is scaled back up before it is published
    /// ([`rescale`]). Defaulted, so only [`CompLuma`] says anything.
    fn analysis_scale(&self) -> f64 {
        1.0
    }
}

/// What an analysis is being asked for — the one branch in the whole file, and
/// it is taken only after the frames have been followed.
///
/// Everything before it is identical: the same decode, the same detector, the
/// same KLT, the same masks, the same cancellation seam, the same progress
/// readings. Only the question asked of the finished [`lumit_track::TrackSet`]
/// differs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JobKind {
    /// Where the camera was: a whole-scene solve.
    Camera,
    /// Where one flat surface is, as four corners per frame. The quad is
    /// the reference shape, in the analysis's own pixels; the tracker is already
    /// confined to it by the job's [`MaskTrack`].
    Planar { quad: Quad },
    /// Where one or two small patches are, each followed on its own and
    /// assumed to be on nothing in particular. The answer is the same
    /// [`PlanarTrack`] shape: the region box under whatever warp the points can
    /// honestly support — a slide from one, a similarity from two.
    Points { regions: PointRegions },
    /// Many points followed for their own sake, for a Track points effect:
    /// corners or blobs, as its rows say. It shares the frame source, the
    /// progress readings and the sidecar with the others, and has a frame
    /// loop of its own ([`follow_points`]).
    TrackPoints(Analysis),
    /// A Bake points bake being read back from the sidecar when a project
    /// opens. Only ever a warm job: the bake itself needs the document, which
    /// a [`Job`] does not carry, so it is started by [`request_bake`].
    Bake,
}

/// A point track's search regions, in the analysis's own pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointRegions {
    /// The two points, of which only the first is read when `two` is false.
    pub points: [[f64; 2]; 2],
    /// Whether the second point is in use.
    pub two: bool,
    /// Half a search box's side.
    pub radius: f64,
}

impl PointRegions {
    /// The points actually in use — one or two.
    #[must_use]
    pub fn used(&self) -> &[[f64; 2]] {
        &self.points[..if self.two { 2 } else { 1 }]
    }

    /// The shape the answer is reported as: the box round the points in use.
    #[must_use]
    pub fn quad(&self) -> Quad {
        points_quad(self.used(), self.radius)
    }

    /// One search box per point, as the contours the tracker is confined to.
    #[must_use]
    pub fn outlines(&self) -> Vec<Vec<[f64; 2]>> {
        point_outlines(self.used(), self.radius)
    }
}

/// One analysis, as handed to the worker.
pub struct Job {
    /// What the answer is filed under, and what [`progress`] is read by.
    ///
    /// For a camera solve that is the **source**: a footage item, or the nested
    /// composition a Precomp layer's Camera track names — one clip, one answer,
    /// shared by every layer cutting it. For a planar track it is the **effect
    /// instance**, because what was tracked is the quad somebody drew and two
    /// Planar tracks on one clip are two different answers.
    pub media: Uuid,
    /// What the sidecar calls it, or `None` for a source with no content name
    /// cheap enough to compute — a nested composition, whose picture is the
    /// whole document beneath it at every frame (docs/impl/tracking.md §5e).
    /// Such an analysis is neither read from nor written to the sidecar; it
    /// lives in the store for the session.
    pub key: Option<AnalysisKey>,
    pub settings: AnalysisSettings,
    /// Which question the followed tracks are asked.
    pub kind: JobKind,
    /// Regions no track may be born in or wander into, in source raster pixels
    /// and re-flattened at each frame's own moment.
    pub masks: MaskTrack,
    /// Opens the frames, **on the worker thread**. `None` means the media could
    /// not be read, which is a refusal and not a fault.
    pub open: Box<dyn FnOnce() -> Option<Box<dyn LumaFrames>> + Send>,
    /// `false` asks only for a cache hit: the warm pass a project open makes,
    /// which must never start tracking a clip nobody asked about.
    pub analyse: bool,
}

/// How far an analysis has got. Read, never subscribed to — the interface
/// samples it as it repaints, exactly as it samples the cache bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Accepted, not started.
    Queued,
    /// Decoding and following features: `done` of `total` frames.
    Tracking {
        done: usize,
        total: usize,
    },
    /// Frames are in; the geometry and the solve are running.
    Solving,
    /// There is a solve in the store for this media.
    Done,
    /// Stopped between frames, or in the solve. Nothing was written.
    Cancelled,
    Failed(AnalysisError),
}

/// Why an analysis did not produce a camera path. Every variant is a refusal
/// rather than a fault: the pictures did not carry the answer, or the file
/// could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisError {
    /// The media could not be opened, or carries no video.
    #[error("the media could not be read")]
    Unreadable,
    /// Opened, but with no frames or no raster.
    #[error("the media has no frames to track")]
    NoFrames,
    /// The tracker refused the frames it was given (a size change mid-clip).
    #[error("the frames could not be tracked: {0}")]
    Tracking(TrackError),
    /// The shot does not carry a camera solve.
    #[error("the shot could not be solved: {0}")]
    Solve(SolveError),
    /// The quad does not carry a planar track.
    #[error("the surface could not be followed: {0}")]
    Planar(PlanarError),
    /// The caller stopped it.
    #[error("the analysis was cancelled")]
    Cancelled,
}

// ---------------------------------------------------------------------------
// What comes back
// ---------------------------------------------------------------------------

/// One media's solve, as the render path wants it: the poses already converted,
/// indexed by frame, plus the solve itself for the point cloud.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    /// The media's own rate, which the solved frame numbers count at.
    pub fps: f64,
    /// How many frames the **clip** has, against which the solved span is a
    /// whole answer or a partial one. See [`Solved::is_partial`].
    pub clip_frames: usize,
    pub first_frame: i64,
    pub last_frame: i64,
    /// One per frame from `first_frame`, in Lumit's camera terms.
    poses: Vec<CameraPose>,
    /// The tracker's own answer, kept for the point-cloud overlay and the 2D
    /// exports that read the same store.
    pub solve: CameraSolve,
}

impl Solved {
    fn new(fps: f64, clip_frames: usize, solve: CameraSolve) -> Option<Self> {
        let first = solve.poses.first()?.frame;
        let last = solve.poses.last()?.frame;
        Some(Solved {
            fps,
            clip_frames,
            first_frame: first,
            last_frame: last,
            poses: solve.poses.iter().map(to_camera_pose).collect(),
            solve,
        })
    }

    /// Whether the clip runs on past what was solved — a run that stopped at a
    /// tracking failure, or frames that stopped decoding.
    ///
    /// The span is a prefix by construction: the job tracks the source from its
    /// first frame and can only ever stop early, never start late. So the one
    /// comparison below is the whole test, and there is no second range to keep
    /// in step with this one.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.last_frame + 1 < i64::try_from(self.clip_frames).unwrap_or(i64::MAX)
    }

    /// The converted pose at solved frame `frame`.
    #[must_use]
    pub fn pose(&self, frame: i64) -> Option<CameraPose> {
        let i = usize::try_from(frame.checked_sub(self.first_frame)?).ok()?;
        self.poses.get(i).copied()
    }
}

/// Turn one solved pose into Lumit's camera terms.
///
/// **Derived, not chosen.** The tracker puts a world point `P` at
/// `centre + f · p.xy / p.z`, where `p = R(P − C)`, `R` is the world-to-camera
/// rotation and `C` the camera centre. The compositor's own matrix
/// ([`lumit_gpu::composite::camera_matrix`]) puts it at
/// `centre + zoom · a.xy / a.z`, where `a = Rot⁻¹(P − position)`, the eye is
/// `position`, and `Rot = Ry·Rx·Rz` is built from the pose's Euler angles
/// (docs/impl/camera.md §3). Setting those equal for every `P` leaves no
/// freedom at all:
///
/// - `zoom = f`, because the two scale factors must agree;
/// - `Rot⁻¹ = R`, so the Euler angles are those of `Rᵀ`;
/// - `position = C`, the camera centre itself.
///
/// The rotation stays a rotation through the conversion: transposing an
/// orthonormal matrix inverts it, so nothing is fitted or normalised here.
fn to_camera_pose(p: &SolvedPose) -> CameraPose {
    let (rx, ry, rz) = euler_of_transpose(&p.rotation);
    CameraPose {
        zoom: p.focal_px,
        position: (p.position[0], p.position[1], p.position[2]),
        rotation_deg: (rx.to_degrees(), ry.to_degrees(), rz.to_degrees()),
        dof: None,
    }
}

/// The `(x, y, z)` Euler angles, in radians, of `Rᵀ` under the compositor's
/// `Ry·Rx·Rz` order.
///
/// Writing `M = Rᵀ` (so `M[i][j] = R[j][i]`) and multiplying the three
/// elementary rotations out gives `M[1][2] = −sin x`, `M[1][0] = cos x · sin z`,
/// `M[1][1] = cos x · cos z`, `M[0][2] = sin y · cos x` and
/// `M[2][2] = cos y · cos x`, which is where every line below comes from. At
/// `cos x = 0` the camera is looking straight along its own y axis, y and z name
/// the same turn, and the convention is to spend it all on y.
fn euler_of_transpose(r: &Mat3) -> (f64, f64, f64) {
    let sin_x = (-r[2][1]).clamp(-1.0, 1.0);
    let rx = sin_x.asin();
    let cos_x = (1.0 - sin_x * sin_x).max(0.0).sqrt();
    if cos_x < 1e-9 {
        return (rx, (-r[0][2]).atan2(r[0][0]), 0.0);
    }
    (rx, r[2][0].atan2(r[2][2]), r[0][1].atan2(r[1][1]))
}

// ---------------------------------------------------------------------------
// The sidecar
// ---------------------------------------------------------------------------

/// Bump when the record's meaning changes. Old files then hash to a different
/// name and are simply never asked for, which is the same disposal the frame
/// cache's algorithm version performs.
///
/// **2** added the clip's own frame count, which is what makes a cached solve
/// still able to say it is a partial one (a version 1 record could not, and a
/// solve that read back as complete would be a lie the sidecar told).
///
/// **3** made the body an [`Answer`] rather than a bare `CameraSolve`, so a
/// planar track is cached in the same folder under the same rules.
/// Every version 2 record is orphaned by it — a re-analysis each, once — which
/// is the disposal this constant exists to perform.
const FORMAT_VERSION: u16 = 3;

/// `LUMTRK\0` — read before anything is deserialised, so a file that is not one
/// of ours is refused rather than fed to a decoder.
const MAGIC: &[u8; 7] = b"LUMTRK\0";

/// What an analysis came to — the sidecar's body, and what [`analyse`] returns.
///
/// One enum rather than two record types and two magics: the two answers are
/// filed in the same folder under the same key rule, differ only in their
/// payload, and a reader that had to guess which of two formats a file was
/// would be a second thing to keep honest for no gain.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Answer {
    Camera(Box<CameraSolve>),
    Planar(Box<PlanarTrack>),
    /// A Track points analysis: every followed point's path, in id order.
    Points(Vec<Followed>),
    /// A Bake points bake: a points stream for every frame of a layer's span,
    /// as [`pack_bake`] writes it.
    Bake(Vec<u8>),
}

/// What one sidecar file holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    /// Repeated inside the file as well as in its name, so a collision or a
    /// renamed file is caught rather than believed.
    key: [u8; 32],
    fps: f64,
    /// The clip's own length, so a cache hit knows a partial answer is partial.
    clip_frames: u64,
    answer: Answer,
}

/// Serialise a record, in the crate's shared framing ([`crate::sidecar`]).
fn encode(key: AnalysisKey, fps: f64, clip_frames: usize, answer: &Answer) -> Option<Vec<u8>> {
    let body = bincode::serialize(&Record {
        key: key.0,
        fps,
        clip_frames: clip_frames as u64,
        answer: answer.clone(),
    })
    .ok()?;
    Some(sidecar::frame(MAGIC, FORMAT_VERSION, &body))
}

/// The inverse, refusing anything it cannot vouch for: wrong magic, a version
/// from the future, a body that will not parse, or a key that is not the one
/// asked for. Every refusal costs a re-analysis and nothing else.
fn decode(bytes: &[u8], key: AnalysisKey) -> Option<(f64, usize, Answer)> {
    let body = sidecar::unframe(bytes, MAGIC, FORMAT_VERSION)?;
    let record: Record = bincode::deserialize(body).ok()?;
    let clip_frames = usize::try_from(record.clip_frames).unwrap_or(usize::MAX);
    (record.key == key.0).then_some((record.fps, clip_frames, record.answer))
}

/// Read a solve out of the sidecar, or `None` for every way that can fail to
/// happen — no folder, no file, an unreadable one, one written by a newer build.
fn read_sidecar(dir: &Path, key: AnalysisKey) -> Option<(f64, usize, Answer)> {
    let bytes = std::fs::read(dir.join(key.file_name())).ok()?;
    decode(&bytes, key)
}

fn write_sidecar(dir: &Path, key: AnalysisKey, fps: f64, clip_frames: usize, answer: &Answer) {
    if let Some(bytes) = encode(key, fps, clip_frames, answer) {
        sidecar::write(dir, &key.file_name(), &bytes);
    }
}

/// Where the sidecar lives. Overridable in tests, which must never write into
/// the user's own cache folder — the shape [`crate::media_index`] uses.
fn cache_dir() -> Option<PathBuf> {
    test_cache_dir().or_else(lumit_project::track_cache_dir)
}

#[cfg(not(test))]
fn test_cache_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
static TEST_CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

// A process-wide override rather than a thread-local one, because the analysis
// runs on a thread this module spawns and a thread-local would not follow it
// there — the sidecar would land in the user's own cache folder, which no test
// may ever write to. Tests take `serially()` for the same reason.
#[cfg(test)]
fn test_cache_dir() -> Option<PathBuf> {
    TEST_CACHE_DIR.lock().ok().and_then(|dir| dir.clone())
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// Every solve this session knows about, by footage item.
///
/// An `RwLock` rather than a channel because the readers are the render path —
/// one lookup per frame, from whichever thread is building it — and the writer
/// is one analysis finishing. The guard is dropped inside the accessor in every
/// case: nothing here is ever held across a decode, a GPU submit or an FFI call
/// (docs/14 §1.3).
///
/// **Eviction:** one entry per analysed media, replaced when that media is
/// re-analysed and dropped wholesale by [`clear`] when a project closes. A solve
/// is a few hundred kilobytes and a project has tens of tracked shots, so there
/// is nothing here to evict piecemeal.
fn solves() -> &'static RwLock<HashMap<Uuid, Arc<Solved>>> {
    static SOLVES: OnceLock<RwLock<HashMap<Uuid, Arc<Solved>>>> = OnceLock::new();
    SOLVES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// One planar track, as the store keeps it: the answer, the media's rate, and
/// the clip's own length so a partial track can say it is one.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanarSolved {
    pub fps: f64,
    pub clip_frames: usize,
    pub track: PlanarTrack,
}

impl PlanarSolved {
    /// Whether the clip runs on past what was followed — the surface was lost,
    /// or the frames stopped decoding. [`Solved::is_partial`]'s reasoning, and
    /// its arithmetic.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.track
            .frame_range()
            .is_some_and(|(_, last)| last + 1 < i64::try_from(self.clip_frames).unwrap_or(i64::MAX))
    }
}

/// Every planar track this session knows about, **by Planar track instance**.
///
/// A separate table from [`solves`] rather than a union in one, because the two
/// are keyed by different things for a reason: a camera solve describes
/// a file and is shared by every layer cutting it; a planar track describes the
/// quad somebody drew and is not shared with anything. One table would have to
/// hold both keys and every reader would have to know which kind it had found.
fn planars() -> &'static RwLock<HashMap<Uuid, Arc<PlanarSolved>>> {
    static PLANARS: OnceLock<RwLock<HashMap<Uuid, Arc<PlanarSolved>>>> = OnceLock::new();
    PLANARS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The one analysis in flight, and what every media's last analysis did.
struct Jobs {
    /// `(media, the flag its frame loop and its solve consult)`.
    running: Option<(Uuid, Arc<AtomicBool>)>,
    /// Bounded by the media a session analyses; cleared by [`clear`].
    progress: HashMap<Uuid, Progress>,
}

fn jobs() -> &'static Mutex<Jobs> {
    static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
    JOBS.get_or_init(|| {
        Mutex::new(Jobs {
            running: None,
            progress: HashMap::new(),
        })
    })
}

/// The solved camera paths, as `lumit-core` sees them.
///
/// A unit struct because the store *is* the process's one table: a solve belongs
/// to a media file, not to a project, and two projects open on the same footage
/// would otherwise each pay for it.
pub struct Store;

impl CameraSolveStore for Store {
    fn solved_range(&self, media: Uuid) -> Option<SolvedRange> {
        let held = solves().read().ok()?;
        let solved = held.get(&media)?;
        Some(SolvedRange {
            fps: solved.fps,
            first_frame: solved.first_frame,
            last_frame: solved.last_frame,
        })
    }

    fn solved_pose(&self, media: Uuid, frame: i64) -> Option<CameraPose> {
        let held = solves().read().ok()?;
        held.get(&media)?.pose(frame)
    }
}

impl PlanarTrackStore for Store {
    fn planar_range(&self, track: Uuid) -> Option<SolvedRange> {
        let held = planars().read().ok()?;
        let solved = held.get(&track)?;
        let (first_frame, last_frame) = solved.track.frame_range()?;
        Some(SolvedRange {
            fps: solved.fps,
            first_frame,
            last_frame,
        })
    }

    fn planar_corners(&self, track: Uuid, frame: i64) -> Option<Quad> {
        let held = planars().read().ok()?;
        held.get(&track)?.track.corners_at(frame)
    }
}

/// What has been tracked under the Planar track instance `track` — the status
/// row's reading and the corner-pin gesture's input.
#[must_use]
pub fn planar(track: Uuid) -> Option<Arc<PlanarSolved>> {
    planars().read().ok()?.get(&track).cloned()
}

/// Put a planar track in the store. Public for [`publish`]'s reason: this is how
/// one gets in, and the bridge's own tests need one without an encoder to make
/// it with.
pub fn publish_planar(track: Uuid, fps: f64, clip_frames: usize, answer: PlanarTrack) {
    if let Ok(mut held) = planars().write() {
        held.insert(
            track,
            Arc::new(PlanarSolved {
                fps,
                clip_frames,
                track: answer,
            }),
        );
    }
}

/// What has been solved for `media`, for the overlay and the exports that read
/// the point cloud. Cloned out of the table so no lock is held while it is used.
#[must_use]
pub fn solved(media: Uuid) -> Option<Arc<Solved>> {
    solves().read().ok()?.get(&media).cloned()
}

/// How far `media`'s analysis has got, or `None` if none was ever asked for.
#[must_use]
pub fn progress(media: Uuid) -> Option<Progress> {
    jobs().lock().ok()?.progress.get(&media).cloned()
}

/// One solved point as the overlay draws it: which track it is, where it lands
/// on the picture, and how near the camera it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectedPoint {
    /// The track's own id — what a selection names, and what
    /// [`point_centroid`] takes back.
    pub track: u32,
    /// Pixels **from the frame's centre**, which is the only origin the tracker
    /// has. The caller adds its own centre; the store cannot, because it is
    /// asked about a media item and one clip lives in many comps (§5b's second
    /// deviation).
    pub x: f64,
    pub y: f64,
    /// Nearness, 0..1 over the points visible on this frame: 1 is the nearest.
    /// Normalised **here** rather than in the interface, because a depth cue is
    /// arithmetic over the whole cloud and the interface draws what it is given
    /// (docs/17's "the engine owns the decisions").
    pub depth: f64,
}

/// `media`'s point cloud as it lands on solved frame `frame`.
///
/// The tracker's own projection, `f · p.xy / p.z` with `p = R(P − C)` — the
/// same equation [`to_camera_pose`] is derived from, so a dot drawn here and
/// the picture drawn through the compositor's matrix agree by construction
/// rather than by a second implementation being kept in step. A point behind
/// the camera has no place on the picture and is simply not returned.
#[must_use]
pub fn projected_points(media: Uuid, frame: i64) -> Vec<ProjectedPoint> {
    let Some(solved) = solved(media) else {
        return Vec::new();
    };
    let Some(pose) = solved
        .solve
        .poses
        .iter()
        .find(|p| p.frame == frame)
        .or_else(|| solved.solve.poses.first())
    else {
        return Vec::new();
    };
    let r = pose.rotation;
    let c = pose.position;
    let mut out: Vec<ProjectedPoint> = Vec::with_capacity(solved.solve.points.len());
    for point in &solved.solve.points {
        let d = [
            point.position[0] - c[0],
            point.position[1] - c[1],
            point.position[2] - c[2],
        ];
        let p = [
            r[0][0] * d[0] + r[0][1] * d[1] + r[0][2] * d[2],
            r[1][0] * d[0] + r[1][1] * d[1] + r[1][2] * d[2],
            r[2][0] * d[0] + r[2][1] * d[1] + r[2][2] * d[2],
        ];
        if p[2] <= 1e-6 || !p[2].is_finite() {
            continue;
        }
        out.push(ProjectedPoint {
            track: point.track,
            x: pose.focal_px * p[0] / p[2],
            y: pose.focal_px * p[1] / p[2],
            depth: p[2],
        });
    }
    // `depth` carries the camera-space distance until here; turn it into the
    // cue. A cloud all at one distance reads as one size rather than as an
    // arbitrary split — dividing by a zero spread would be inventing a
    // foreground.
    let near = out.iter().map(|p| p.depth).fold(f64::INFINITY, f64::min);
    let far = out
        .iter()
        .map(|p| p.depth)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = far - near;
    for p in &mut out {
        p.depth = if spread > 1e-9 {
            ((far - p.depth) / spread).clamp(0.0, 1.0)
        } else {
            1.0
        };
    }
    out
}

/// Where the named tracks sit in the world, averaged — the position the
/// creation gesture puts a Null or a Solid at.
///
/// The tracker's world **is** Lumit's comp-pixel world: `to_camera_pose` hands
/// a camera centre straight over as a `position`, so a scene point needs no
/// conversion either. `None` when none of the ids names a solved point.
#[must_use]
pub fn point_centroid(media: Uuid, tracks: &[u32]) -> Option<[f64; 3]> {
    let solved = solved(media)?;
    let mut sum = [0.0f64; 3];
    let mut n = 0u32;
    for point in &solved.solve.points {
        if !tracks.contains(&point.track) {
            continue;
        }
        for (s, v) in sum.iter_mut().zip(point.position) {
            *s += v;
        }
        n += 1;
    }
    (n > 0).then(|| sum.map(|s| s / f64::from(n)))
}

/// The **active** camera's placement at comp time `t`, with its solve link
/// followed. The reading the render path takes, replacing
/// [`lumit_core::model::Composition::camera_pose`], which answers only what the
/// document holds.
#[must_use]
pub fn camera_pose(doc: &Document, comp: &Composition, t: f64) -> Option<CameraPose> {
    linked_pose(doc, comp, t).map(|l| l.pose)
}

/// As [`camera_pose`], keeping the [`lumit_core::track::LinkState`] the
/// interface draws as a badge.
#[must_use]
pub fn linked_pose(doc: &Document, comp: &Composition, t: f64) -> Option<LinkedPose> {
    lumit_core::track::camera_pose_at(doc, comp, t, &Store)
}

/// Forget every solve and every progress reading — what closing a project does.
/// The sidecar is untouched, so reopening reads them straight back.
pub fn clear() {
    if let Ok(mut held) = solves().write() {
        held.clear();
    }
    if let Ok(mut held) = planars().write() {
        held.clear();
    }
    track_points::retain(|_| false);
    bake_points::retain(|_| false);
    if let Ok(mut held) = jobs().lock() {
        held.progress.clear();
    }
}

/// Forget the solves and progress readings stored under `ids`, and no others.
/// What closing a project does with its own media and Planar track instances
/// (see [`owned_ids`]), since the store is shared by every project in the
/// process and another one's solves are not this one's to drop. The sidecar is
/// untouched, so reopening reads them straight back.
pub fn forget(ids: &[Uuid]) {
    if let Ok(mut held) = solves().write() {
        held.retain(|id, _| !ids.contains(id));
    }
    if let Ok(mut held) = planars().write() {
        held.retain(|id, _| !ids.contains(id));
    }
    track_points::retain(|id| !ids.contains(id));
    bake_points::retain(|id| !ids.contains(id));
    if let Ok(mut held) = jobs().lock() {
        held.progress.retain(|id, _| !ids.contains(id));
    }
}

/// Every id a document's solves are stored under: its footage items, which key
/// the camera solves and the progress readings, and its Planar track instances,
/// enabled or not, which key the planar ones.
#[must_use]
pub fn owned_ids(doc: &Document) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for item in &doc.items {
        match item {
            lumit_core::model::ProjectItem::Footage(f) => ids.push(f.id),
            lumit_core::model::ProjectItem::Composition(comp) => ids.extend(
                comp.layers
                    .iter()
                    .flat_map(|layer| &layer.effects)
                    .filter(|e| {
                        e.effect.match_name == PLANAR_TRACK
                            || e.effect.match_name == track_points::MATCH_NAME
                            || e.effect.match_name == bake_points::MATCH_NAME
                    })
                    .map(|e| e.id),
            ),
            _ => {}
        }
    }
    ids
}

// ---------------------------------------------------------------------------
// Asking for one
// ---------------------------------------------------------------------------

/// What happened when an analysis was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    /// Accepted; watch [`progress`].
    Started,
    /// Another analysis is running. One at a time, deliberately: this is a
    /// minutes-long decode-bound job, and two of them share one disk and halve
    /// each other. Ask again when it is done.
    Busy,
    /// The thread could not be started at all.
    Refused,
}

/// Start `job` on its own thread.
///
/// Returns as soon as the thread is spawned; the cache probe happens *there*, so
/// no caller — least of all the interface thread — ever waits on the disk
/// (docs/14 §1.1).
pub fn request(job: Job) -> Requested {
    start(job.media, move |cancel| run(job, cancel))
}

/// Claim the one analysis slot for `media` and run `work` on a thread of its
/// own, handed the flag Cancel raises.
fn start(media: Uuid, work: impl FnOnce(&AtomicBool) + Send + 'static) -> Requested {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut held) = jobs().lock() else {
            return Requested::Refused;
        };
        if held.running.is_some() {
            return Requested::Busy;
        }
        held.running = Some((media, Arc::clone(&cancel)));
        held.progress.insert(media, Progress::Queued);
    }
    let spawned = std::thread::Builder::new()
        .name("lumit-track".into())
        .spawn(move || work(&cancel));
    if spawned.is_err() {
        // The slot was claimed above and nothing will ever release it, so it is
        // released here — otherwise one failed spawn would answer `Busy` for the
        // rest of the session.
        finish(media, None);
        return Requested::Refused;
    }
    Requested::Started
}

/// Every cached solve a document could be holding, as warm-pass jobs: one for
/// each footage item a layer wears an enabled Camera track on.
///
/// Read off the document straight after its media is relinked, which is when
/// `absolute_path` and the fingerprint are both filled in — no file is opened
/// here and none is opened by the jobs, which carry `analyse: false` and
/// therefore never get past the sidecar probe.
///
/// One job per **media**, not per layer: two layers cutting the same shot at the
/// same settings are one analysis, and at different settings they are two
/// answers only one of which the store can hold. First in document order wins,
/// which is stable.
///
/// A footage item with no fingerprint or no resolved path is skipped: it is
/// offline, and there is nothing to name a solve with.
#[must_use]
pub fn warm_jobs(doc: &Document) -> Vec<Job> {
    let mut seen: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in &doc.items {
        let lumit_core::model::ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &comp.layers {
            // A bake is filed under its instance and may sit on any kind of
            // layer, so it is read back before the footage test below.
            let bakes = layer.effects.iter().filter(is_bake_points);
            out.extend(bakes.map(|fx| Job {
                media: fx.id,
                key: Some(AnalysisKey::bake(fx.id)),
                settings: AnalysisSettings::default(),
                kind: JobKind::Bake,
                masks: MaskTrack::default(),
                open: Box::new(|| None),
                analyse: false,
            }));
            let LayerKind::Footage { item: media } = layer.kind else {
                continue;
            };
            if camera_track_effect(layer).is_none()
                && planar_track_effect(layer).is_none()
                && track_points_effects(layer).next().is_none()
            {
                continue;
            }
            let Some(footage) = doc.items.iter().find_map(|i| match i {
                lumit_core::model::ProjectItem::Footage(f) if f.id == media => Some(f),
                _ => None,
            }) else {
                continue;
            };
            let (Some(fingerprint), false) = (
                footage.media.fingerprint.as_ref(),
                footage.media.absolute_path.is_empty(),
            ) else {
                continue;
            };
            let path = PathBuf::from(&footage.media.absolute_path);
            // One camera job per **media** — two layers cutting the same shot
            // are one analysis. One planar job per **effect instance**, because
            // two quads on one shot are two answers, so there is
            // nothing to deduplicate.
            if seen.insert(media) {
                if let Some(job) = job_for(layer, path.clone(), fingerprint, false) {
                    out.push(job);
                }
            }
            // One per Track points instance, for the Planar track's reason.
            for fx in track_points_effects(layer) {
                out.extend(points_job_for(layer, fx, path.clone(), fingerprint, false));
            }
            if let Some(job) = planar_job_for(layer, path, fingerprint, false) {
                out.push(job);
            }
        }
    }
    out
}

/// Read every one of `jobs` back out of the sidecar, on one thread, filling the
/// store with whatever is already there. What opening a project does:
/// until this runs, a solve-linked camera resolves only after somebody presses
/// Analyse in the session, though the answer was on the disk all along.
///
/// **Not [`request`], deliberately.** `request` owns the one-analysis-at-a-time
/// slot, so warming the second tracked clip of a project would answer
/// [`Requested::Busy`] and simply not happen. A warm pass is a small file read
/// per clip and nothing else — no decoder, no minutes — so the whole batch runs
/// on one thread of its own, claims no slot, and cannot collide with an analysis
/// the user starts while it is going.
///
/// `analyse` is forced off on the way past: nothing this function is handed may
/// start tracking a clip nobody asked about.
pub fn warm(jobs: Vec<Job>) {
    if jobs.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("lumit-track-warm".into())
        .spawn(move || {
            let never = AtomicBool::new(false);
            for mut job in jobs {
                job.analyse = false;
                run(job, &never);
            }
        });
}

/// Stop `media`'s analysis. Running: the flag is raised and the run ends between
/// frames or inside the solve. Not running: the reading is set to cancelled, so
/// a job still waiting to start does not.
pub fn cancel(media: Uuid) {
    let Ok(mut held) = jobs().lock() else {
        return;
    };
    if let Some((running, flag)) = &held.running {
        if *running == media {
            flag.store(true, Ordering::Relaxed);
        }
    }
    held.progress.insert(media, Progress::Cancelled);
}

/// Publish one step of a running analysis.
fn report(media: Uuid, step: Progress) {
    if let Ok(mut held) = jobs().lock() {
        held.progress.insert(media, step);
    }
}

/// Take the job off the running slot and publish its outcome.
///
/// `None` **forgets** the media instead: a warm pass that found nothing, or a
/// spawn that never happened, has nothing to say about it, and leaving `Queued`
/// standing would have the interface waiting on a job that will never run.
fn finish(media: Uuid, outcome: Option<Progress>) {
    if let Ok(mut held) = jobs().lock() {
        if held.running.as_ref().is_some_and(|(m, _)| *m == media) {
            held.running = None;
        }
        match outcome {
            Some(step) => held.progress.insert(media, step),
            None => held.progress.remove(&media),
        };
    }
}

/// The whole of one job, on the analysis thread: read the sidecar, and only if
/// there is nothing there, track and solve.
fn run(job: Job, cancel: &AtomicBool) {
    let media = job.media;
    let key = job.key;
    let kind = job.kind;
    let dir = cache_dir();

    if let Some((fps, clip_frames, answer)) = key
        .zip(dir.as_deref())
        .and_then(|(key, d)| read_sidecar(d, key))
    {
        // A file that held nothing readable is treated as no file.
        if file(media, kind, fps, clip_frames, answer) {
            finish(media, Some(Progress::Done));
            return;
        }
    }
    if !job.analyse {
        // A warm pass found nothing. That is not a failure and must not look
        // like one: nobody asked for this clip to be tracked.
        finish(media, None);
        return;
    }

    match analyse(job, cancel, &|step| report(media, step)) {
        Ok((fps, clip_frames, answer)) => {
            // Written before the store is filled, so a solve the interface can
            // see is a solve the next session will find (and never the other
            // way round). Cancellation writes nothing at all, which is what
            // makes a stopped run leave no trace. A **partial** solve is
            // cached like any other: it is the honest answer for that file at
            // those settings, and re-deriving it would only take the same
            // minutes to stop in the same place. A source with no key — a
            // nested comp — is filed nowhere and lives for the session.
            if let (Some(key), Some(dir)) = (key, dir.as_deref()) {
                write_sidecar(dir, key, fps, clip_frames, &answer);
            }
            file(media, kind, fps, clip_frames, answer);
            finish(media, Some(Progress::Done));
        }
        Err(AnalysisError::Cancelled) => finish(media, Some(Progress::Cancelled)),
        Err(e) => finish(media, Some(Progress::Failed(e))),
    }
}

/// Put whichever kind of answer this is into whichever table holds it — the one
/// place a finished analysis and a cache hit both come through. `false` for an
/// answer that could not be read, which went nowhere.
fn file(id: Uuid, kind: JobKind, fps: f64, clip_frames: usize, answer: Answer) -> bool {
    match answer {
        Answer::Camera(solve) => publish(id, fps, clip_frames, *solve),
        Answer::Planar(track) => publish_planar(id, fps, clip_frames, *track),
        // The rows it was analysed under come from the job and not from the
        // sidecar, so a file another project wrote is filed under this
        // project's own footage item.
        Answer::Points(tracks) => {
            if let JobKind::TrackPoints(analysis) = kind {
                track_points::publish(
                    id,
                    Baked {
                        analysis,
                        fps,
                        frames: u32::try_from(clip_frames).unwrap_or(u32::MAX),
                        tracks,
                    },
                );
            }
        }
        // A file that will not unpack is no bake.
        Answer::Bake(bytes) => match unpack_bake(&bytes) {
            Some(baked) => bake_points::publish(id, baked),
            None => return false,
        },
    }
    true
}

/// Put a solve in the store, converted.
///
/// `clip_frames` is the **clip's** own length, which is what makes the solve
/// able to say whether it covers all of it ([`Solved::is_partial`]); it is not
/// derivable from the poses, which describe only the span that was solved.
///
/// Public because this is how a solve gets in and there is exactly one way: an
/// analysis finishing, a sidecar being read back — and the bridge's own tests,
/// which need a solve in the store without an encoder to make one out of.
pub fn publish(media: Uuid, fps: f64, clip_frames: usize, solve: CameraSolve) {
    let Some(solved) = Solved::new(fps, clip_frames, solve) else {
        return;
    };
    if let Ok(mut held) = solves().write() {
        held.insert(media, Arc::new(solved));
    }
}

/// Decode, track, solve. The whole cost of an analysis, and — given the same
/// frames — a pure function of its inputs.
///
/// Separated from [`run`] so the engine tests can drive it directly with a
/// deterministic progress log, rather than racing a thread to observe one.
fn analyse(
    job: Job,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(f64, usize, Answer), AnalysisError> {
    let kind = job.kind;
    if let JobKind::TrackPoints(analysis) = kind {
        return follow_points(job, analysis, cancel, report);
    }
    let (fps, clip_frames, mut set, scale) = track_frames(job, cancel, report)?;

    report(Progress::Solving);
    let stop = || cancel.load(Ordering::Relaxed);

    // The one branch by job kind, and it is here rather than anywhere earlier:
    // every line above this point is the same work whichever question is being
    // asked.
    if let JobKind::Points { regions } = kind {
        let first = set.frame_range().map_or(0, |(f, _)| f);
        let mut track = solve_points_cancellable(
            &set,
            first,
            regions.used(),
            regions.quad(),
            &PointSettings {
                radius: regions.radius,
                ..PointSettings::default()
            },
            &stop,
        )
        .map_err(|e| match e {
            PlanarError::Cancelled => AnalysisError::Cancelled,
            other => AnalysisError::Planar(other),
        })?;
        rescale_planar(&mut track, scale);
        return Ok((fps, clip_frames, Answer::Planar(Box::new(track))));
    }
    if let JobKind::Planar { quad } = kind {
        let first = set.frame_range().map_or(0, |(f, _)| f);
        let mut track = solve_planar_cancellable(
            &set,
            first,
            quad,
            set.source_size(),
            &PlanarSettings::default(),
            &stop,
        )
        .map_err(|e| match e {
            PlanarError::Cancelled => AnalysisError::Cancelled,
            other => AnalysisError::Planar(other),
        })?;
        rescale_planar(&mut track, scale);
        return Ok((fps, clip_frames, Answer::Planar(Box::new(track))));
    }

    let pairs = select_keyframes(&set, &GeometrySettings::default());
    segment_dynamic_tracks(&mut set, &pairs, &SegmentSettings::default());
    let zooms = detect_zoom(&set, &ZoomSettings::default());
    let mut solve =
        solve_camera_cancellable(&set, &pairs, &zooms, &SolveSettings::default(), &stop).map_err(
            |e| match e {
                SolveError::Cancelled => AnalysisError::Cancelled,
                other => AnalysisError::Solve(other),
            },
        )?;
    rescale(&mut solve, scale);
    Ok((fps, clip_frames, Answer::Camera(Box::new(solve))))
}

/// The most a bake's file may say it unpacks to: twice what a bake is
/// allowed to hold. A bake's frames are counted against
/// [`bake_points::MAX_BYTES`] at the size they are written, and the rest of
/// the file is under a megabyte, so every file this program wrote fits.
const BAKE_BYTES: usize = 2 * bake_points::MAX_BYTES;

/// A bake as its sidecar holds it: laid out for a compressor by
/// [`bake_points::Baked::stored`], then LZ4.
fn pack_bake(baked: &bake_points::Baked) -> Option<Vec<u8>> {
    let body = bincode::serialize(&baked.stored()).ok()?;
    Some(lz4_flex::compress_prepend_size(&body))
}

/// The inverse. The size the file announces is checked before anything is
/// allocated for it, since the file's number is not this program's.
fn unpack_bake(bytes: &[u8]) -> Option<bake_points::Baked> {
    let announced = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
    if announced as usize > BAKE_BYTES {
        return None;
    }
    let body = lz4_flex::decompress_size_prepended(bytes).ok()?;
    bake_points::Baked::restored(&bincode::deserialize(&body).ok()?)
}

fn is_bake_points(fx: &&EffectInstance) -> bool {
    fx.effect.match_name == bake_points::MATCH_NAME
}

/// Start baking the stream wired into the Bake points instance `effect`, on
/// `layer` of `comp`, over the layer's whole span.
///
/// It shares the one slot, the progress readings and Cancel with the
/// analyses, filed under the instance as a Planar track is. A bake is not a
/// [`Job`] because it reads the document and not a file's frames. Pressing
/// Bake always bakes: the file from last time is replaced, never read back,
/// since what it was made from may have changed in a way the fingerprint
/// cannot see.
pub fn request_bake(doc: Arc<Document>, comp: Uuid, layer: Uuid, effect: Uuid) -> Requested {
    start(effect, move |cancel| {
        let step = |step| report(effect, step);
        let outcome = match bake(&doc, comp, layer, effect, cancel, &step) {
            Ok(baked) => {
                // Written before the table is filled, as a solve is.
                if let (Some(dir), Some(bytes)) = (cache_dir(), pack_bake(&baked)) {
                    let (key, span) = (AnalysisKey::bake(effect), baked.span as usize);
                    write_sidecar(&dir, key, baked.fps, span, &Answer::Bake(bytes));
                }
                bake_points::publish(effect, baked);
                Progress::Done
            }
            Err(AnalysisError::Cancelled) => Progress::Cancelled,
            Err(e) => Progress::Failed(e),
        };
        finish(effect, Some(outcome));
    })
}

/// The whole of one bake, on the analysis thread: every frame of the layer's
/// span inside the composition, and the stream wired into the effect at each.
///
/// Each frame is asked of the draw builder first, which answers without
/// drawing anything for every stream it can work out. A stream whose points
/// depend on a picture is not one of those. For that the frame is drawn, on a
/// renderer this thread owns so the Viewer's is left alone, and the stream is
/// read as the walk hands it to the effect.
fn bake(
    doc: &Arc<Document>,
    comp_id: Uuid,
    layer_id: Uuid,
    effect: Uuid,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<bake_points::Baked, AnalysisError> {
    let find = |doc: &Arc<Document>| {
        let comp = doc.comp(comp_id)?;
        let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
        let at = layer.effects.iter().position(|e| e.id == effect)?;
        Some((comp.frame_rate, comp.duration, layer.clone(), at))
    };
    let (rate, duration, layer, at) = find(doc).ok_or(AnalysisError::Unreadable)?;
    let fingerprint = doc
        .comp(comp_id)
        .and_then(|comp| bake_points::fingerprint(doc, comp, &layer, effect))
        .ok_or(AnalysisError::Unreadable)?;
    let first = rate.frame_at(layer.in_point).max(0);
    let end = rate
        .frame_at(layer.out_point)
        .min(rate.frame_at(lumit_core::time::CompTime(duration.0)));
    let span = usize::try_from(end.saturating_sub(first)).unwrap_or(0);
    if span == 0 {
        return Err(AnalysisError::NoFrames);
    }

    // What the frames are made from: the document with this effect on Bypass,
    // so it is handed the live stream whatever bake is in hand, and with
    // nothing after it on the layer, which no stream above could depend on.
    // The layer is shown and nothing is soloed, so a walk reaches it, and
    // there is no motion blur, so the one walk is at the frame's own moment.
    let mut staged = Document::clone(doc);
    if let Some(comp) = staged.comp_mut(comp_id) {
        comp.motion_blur.enabled = false;
        for l in &mut comp.layers {
            l.switches.solo = false;
            if l.id != layer_id {
                continue;
            }
            l.switches.visible = true;
            l.effects.truncate(at + 1);
            if let Some(mode) = l
                .effects
                .last_mut()
                .and_then(|e| e.params.iter_mut().find(|p| p.id == "mode"))
            {
                mode.value = EffectValue::Choice(bake_points::MODE_BYPASS);
            }
        }
    }
    let staged = Arc::new(staged);
    let (_, _, layer, _) = find(&staged).ok_or(AnalysisError::Unreadable)?;
    let comp = staged.comp(comp_id).ok_or(AnalysisError::Unreadable)?;
    let inst = layer.effects.last().ok_or(AnalysisError::Unreadable)?;

    let total = span.min(bake_points::MAX_FRAMES);
    let mut baked = bake_points::Baked::new(
        fingerprint,
        rate.fps(),
        first,
        u32::try_from(span).unwrap_or(u32::MAX),
    );
    // ponytail: a drawn frame is the whole composition at full size, since
    // the stream may read any of it. Drawing this layer's stack alone is the
    // upgrade if baking a heavy composition is too slow.
    let mut renderer: Option<crate::headless::HeadlessRenderer> = None;
    let mut held = 0usize;
    for k in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        report(Progress::Tracking { done: k, total });
        let frame = first.saturating_add(k as i64);
        let t = rate
            .time_of_frame(frame)
            .map_err(|_| AnalysisError::NoFrames)?
            .0
            .to_f64();
        let mut stream = match crate::build::points_input_at(&staged, comp, &layer, inst, t) {
            Some(stream) => stream,
            None => {
                let renderer = match &mut renderer {
                    Some(renderer) => renderer,
                    slot => {
                        let mut made = crate::headless::HeadlessRenderer::new()
                            .map_err(|_| AnalysisError::Unreadable)?;
                        made.sync_colour(&staged);
                        slot.insert(made)
                    }
                };
                crate::fxops::watch_points(Some(effect));
                let drawn =
                    renderer.render_rgba(&staged, comp_id, u64::try_from(frame).unwrap_or(0), 1.0);
                let seen = crate::fxops::take_watched_points();
                crate::fxops::watch_points(None);
                drawn.map_err(|_| AnalysisError::Unreadable)?;
                seen.unwrap_or_default()
            }
        };
        stream.keep_newest(bake_points::MAX_POINTS);
        match baked.push(&stream, bake_points::MAX_BYTES.saturating_sub(held)) {
            Some(added) => held += added,
            None => break,
        }
    }
    report(Progress::Tracking {
        done: baked.frames.len(),
        total,
    });
    baked.seal();
    Ok(baked)
}

/// The most blobs followed on one frame.
///
/// ponytail: one number rather than a row. It bounds what a noisy clip can
/// cost: this many points a frame in the sidecar at twelve bytes each. A Max
/// points row is the upgrade if somebody needs more.
const MAX_BLOBS: usize = 1000;

/// A Track points analysis: every frame of the clip through the feature
/// tracker or the blob tracker, and each followed point's whole path back.
///
/// [`track_frames`] is not reused because it stops where a camera solve would
/// have nothing to stand on, and there is no solve here: a frame with three
/// points on it is three points.
fn follow_points(
    job: Job,
    analysis: Analysis,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(f64, usize, Answer), AnalysisError> {
    let mut frames = (job.open)().ok_or(AnalysisError::Unreadable)?;
    let (total, width, height, fps) = frames.info();
    if total == 0 || width == 0 || height == 0 || fps <= 0.0 || !fps.is_finite() {
        return Err(AnalysisError::NoFrames);
    }
    let (w, h) = (width as usize, height as usize);

    // The Camera track's own mapping of Feature density, and the tracker's
    // defaults for everything the effect has no row for.
    let (across, down, per_bucket) =
        lumit_core::fx::effects::camera_track::density(analysis.density);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut features = Tracker::new(TrackSettings {
        grid: (across, down),
        per_bucket,
        quality: analysis.quality,
        min_separation: f64::from(analysis.spacing),
        // The row is the whole window, which is twice this and one more.
        half_window: (((analysis.window - 1.0) / 2.0).round().clamp(2.0, 31.0)) as usize,
        ..TrackSettings::default()
    });
    let mut blobs = BlobTracker::new(BlobSettings {
        threshold: analysis.threshold,
        invert: analysis.invert,
        min_area: f64::from(analysis.min_area),
        max_area: f64::from(analysis.max_area),
        max_move: f64::from(analysis.max_distance),
        max_blobs: MAX_BLOBS,
    });

    let mut pushed = 0usize;
    for n in 0..total {
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        report(Progress::Tracking { done: n, total });
        let Some(luma) = frames.luma(n) else {
            break;
        };
        let plane = FramePlane::new(&luma, w, h).map_err(AnalysisError::Tracking)?;
        if analysis.blobs {
            blobs.push(n as i64, plane)
        } else {
            features.push(n as i64, plane, None)
        }
        .map_err(AnalysisError::Tracking)?;
        pushed += 1;
    }
    report(Progress::Tracking {
        done: pushed,
        total,
    });

    // The frames are numbered from nought, so a first frame always fits.
    let first = |frame: i64| u32::try_from(frame).unwrap_or(0);
    #[allow(clippy::cast_possible_truncation)]
    let tracks: Vec<Followed> = if analysis.blobs {
        blobs
            .finish()
            .iter()
            .map(|t| Followed {
                id: t.id,
                first: first(t.points.first().map_or(0, |p| p.frame)),
                at: t
                    .points
                    .iter()
                    .map(|p| [p.x as f32, p.y as f32, p.area as f32])
                    .collect(),
            })
            .collect()
    } else {
        features
            .finish()
            .tracks()
            .iter()
            // A feature seen on one frame and lost on the next was never
            // followed anywhere. Empty buckets throw up a few of those every
            // frame, and they would only flicker.
            .filter(|t| t.points.len() > 1)
            .map(|t| Followed {
                id: t.id,
                first: first(t.first_frame()),
                // How well the patch still matched the one the track began
                // with. The first frame is that patch, so it matches fully.
                at: std::iter::once(1.0)
                    .chain(t.steps.iter().map(|s| s.ncc.clamp(0.0, 1.0)))
                    .zip(&t.points)
                    .map(|(ncc, p)| [p.x as f32, p.y as f32, ncc as f32])
                    .collect(),
            })
            .collect()
    };
    Ok((fps, total, Answer::Points(tracks)))
}

/// [`rescale`], for a planar track: the corners are the whole of its geometry,
/// and they are pixels, so one multiply each is the whole conversion.
fn rescale_planar(track: &mut PlanarTrack, scale: f64) {
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 1e-9 {
        return;
    }
    let f = 1.0 / scale;
    for corner in &mut track.reference_quad {
        corner[0] *= f;
        corner[1] *= f;
    }
    for frame in &mut track.frames {
        for corner in &mut frame.corners {
            corner[0] *= f;
            corner[1] *= f;
        }
    }
}

/// Put a solve measured at `scale` back into the source's own pixels.
///
/// A source rendered smaller than it is — a nested comp at the analysis raster —
/// gives a solve in *those* pixels: a focal in them, a world in them, an error
/// in them. Everything downstream reads the store as the source's own pixels
/// (§5b's second deviation), so the whole answer is multiplied by `1 / scale`
/// here, at the one place a solve is finished.
///
/// **Uniform, so the geometry is untouched.** The projection is
/// `focal · p.xy / p.z` with `p = R(P − C)`: multiplying the focal, the camera
/// centres and the world points all by the same number multiplies the projected
/// pixels by it too and changes nothing else — not a rotation, not a depth
/// ordering, not a reprojection *ratio*. It is a change of unit, not a fit.
///
/// The scaling is done after the solve rather than to the tracks before it,
/// because the solver's thresholds are in pixels: feeding it inflated
/// coordinates would silently tighten every one of them.
fn rescale(solve: &mut CameraSolve, scale: f64) {
    if !scale.is_finite() || scale <= 0.0 || (scale - 1.0).abs() < 1e-9 {
        return;
    }
    let f = 1.0 / scale;
    for pose in &mut solve.poses {
        pose.focal_px *= f;
        pose.mean_reprojection_px *= f;
        for v in &mut pose.position {
            *v *= f;
        }
    }
    for segment in &mut solve.segments {
        segment.focal_px *= f;
    }
    for point in &mut solve.points {
        for v in &mut point.position {
            *v *= f;
        }
    }
    solve.mean_reprojection_px *= f;
}

/// How many tracks must carry across a frame boundary for anything past it to
/// be solvable at all.
///
/// **The solver's own minimum, not a taste.** Every later phase stands on
/// two-view geometry between frames, and its minimal sample is seven
/// correspondences (the 7-point fundamental); eight is the smallest set that
/// can be *verified* rather than merely fitted, since a fit through exactly its
/// minimal sample has nothing left over to disagree with it. Below that the
/// chain of correspondence is **severed** at that frame: no geometry through it
/// can be estimated, so no frame after it can be tied to any frame before it,
/// however well the frames after it track among themselves. That is a
/// statement about the arithmetic rather than a threshold anyone tuned, which
/// is why it is the signal the job stops on.
///
/// ponytail: a hard geometric floor and nothing else. A shot that degrades
/// badly without ever crossing it still solves badly, and says so through the
/// mean reprojection error the status row already reports; a relative-collapse
/// detector (carriage falling to a fraction of its own recent level) is the
/// upgrade if real footage turns out to need one, and it would need a threshold
/// somebody has to defend.
const MIN_CARRIED: usize = 8;

/// The decode-and-follow half: every frame of the clip through the tracker, one
/// at a time, with the frame loop as the cancellation seam.
///
/// Separate from the solve because it is the half that reads pixels, and so the
/// half whose claims — the mask exclusion, the progress readings — are about
/// tracks rather than about cameras.
///
/// **It can stop before the end of the clip**, and the set it hands back then
/// covers only the span that worked (`(fps, the clip's own frame count, the
/// tracks, the scale they were measured at)`). Two things end a run early and they are reported the same way,
/// because they mean the same thing to everything downstream: the frames stop
/// arriving ([`LumaFrames::luma`] answering `None`), or the tracking itself
/// fails — fewer than [`MIN_CARRIED`] tracks carrying across a frame boundary.
/// The frames after such a boundary are dropped rather than solved: they are
/// not a poorer answer, they are no answer, and half a shot honestly measured
/// is worth more than a whole one with an invented tail.
fn track_frames(
    job: Job,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(f64, usize, lumit_track::TrackSet, f64), AnalysisError> {
    let mut frames = (job.open)().ok_or(AnalysisError::Unreadable)?;
    let (total, width, height, fps) = frames.info();
    if total == 0 || width == 0 || height == 0 || fps <= 0.0 || !fps.is_finite() {
        return Err(AnalysisError::NoFrames);
    }
    let (w, h) = (width as usize, height as usize);

    let mut tracker = Tracker::new(job.settings.tracker());
    // A still mask is flattened once for the whole run; a keyed one is
    // re-flattened per frame, below. Per frame rather than per span because a
    // path flatten is a few hundred line segments off a handful of cubics —
    // microseconds — against a pyramid build and several hundred KLT solves for
    // the same frame, which are milliseconds. A span table would be a second
    // thing to keep honest about where the shape actually is, to save a cost
    // that does not show.
    let animated = job.masks.animated();
    if !animated {
        tracker.set_masks(job.masks.at(0.0));
    }
    let mut pushed = 0usize;
    let mut severed: Option<i64> = None;
    for n in 0..total {
        // The frame loop is the cancellation seam (docs/14 §1.4): one check per
        // frame, and the crate being driven owns no long uninterruptible run of
        // its own.
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        report(Progress::Tracking { done: n, total });
        let Some(luma) = frames.luma(n) else {
            break;
        };
        if animated {
            // Source frame `n` shows at layer time `n / fps`, which is the
            // clock the mask's keys are on (see [`MaskTrack`]).
            tracker.set_masks(job.masks.at(n as f64 / fps));
        }
        let plane = FramePlane::new(&luma, w, h).map_err(AnalysisError::Tracking)?;
        tracker
            .push(n as i64, plane, None)
            .map_err(AnalysisError::Tracking)?;
        // Nothing carries into the first frame, so there is nothing to judge
        // until the second one.
        if n > 0 && tracker.carried_count() < MIN_CARRIED {
            severed = Some(n as i64);
            break;
        }
        pushed += 1;
    }
    report(Progress::Tracking {
        done: pushed,
        total,
    });
    let mut set = tracker.finish();
    if let Some(n) = severed {
        // The failing frame itself is dropped with everything after it: the
        // last frame anything is known about is the one before the boundary
        // that nothing crossed.
        set.truncate(n - 1);
    }
    Ok((fps, total, set, frames.analysis_scale()))
}

// ---------------------------------------------------------------------------
// Building a job from the document
// ---------------------------------------------------------------------------

/// The Camera track instance on `layer`, if it carries an enabled one.
#[must_use]
pub fn camera_track_effect(layer: &Layer) -> Option<&EffectInstance> {
    layer
        .effects
        .iter()
        .find(|e| e.enabled && e.effect.match_name == CAMERA_TRACK)
}

/// Build the job for the tracked layer `layer` of `comp`, ready to hand to
/// [`request`]. `path` is where the frontend resolved the media to (the
/// document keeps no absolute paths, so only the frontend can say).
///
/// `None` when the layer is not footage, or carries no enabled Camera track.
#[must_use]
pub fn job_for(
    layer: &Layer,
    path: PathBuf,
    fingerprint: &Fingerprint,
    analyse: bool,
) -> Option<Job> {
    let LayerKind::Footage { item } = layer.kind else {
        return None;
    };
    let settings = AnalysisSettings::of(camera_track_effect(layer)?);
    let masks = MaskTrack::of(layer, settings, 1.0);
    Some(Job {
        media: item,
        key: Some(AnalysisKey::new(fingerprint, settings, &masks)),
        settings,
        kind: JobKind::Camera,
        masks,
        open: Box::new(move || MediaLuma::open(&path).map(|s| Box::new(s) as Box<dyn LumaFrames>)),
        analyse,
    })
}

/// The Planar track instance on `layer`, if it carries an enabled one — the
/// first in stack order, so the answer never depends on the playhead.
#[must_use]
pub fn planar_track_effect(layer: &Layer) -> Option<&EffectInstance> {
    layer
        .effects
        .iter()
        .find(|e| e.enabled && e.effect.match_name == PLANAR_TRACK)
}

/// The reference quad an instance declares, in [`Quad`] order.
///
/// The eight rows are px@comp, and for a footage layer those *are* the
/// source raster the tracker works in (§5b's seventh deviation), so nothing
/// converts. A row the instance does not carry reads as the effect's own
/// default rather than failing (docs/14 §4) — but a *static* read is the only
/// honest one here: the quad is the shape the surface has on the reference
/// frame, and animating it would be asking the tracker to follow a moving
/// target from a moving start.
#[must_use]
pub fn planar_quad(fx: &EffectInstance) -> Quad {
    let at = |id: &str| match fx.param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => 0.0,
    };
    [
        [at("upper_left_x"), at("upper_left_y")],
        [at("upper_right_x"), at("upper_right_y")],
        [at("lower_left_x"), at("lower_left_y")],
        [at("lower_right_x"), at("lower_right_y")],
    ]
}

/// The point rows off a Planar track instance, read statically at layer time
/// zero exactly as the quad is and for the same reason: they are where the
/// things being followed are on the reference frame.
#[must_use]
pub fn planar_points(fx: &EffectInstance, follow: u32) -> PointRegions {
    let at = |id: &str, fallback: f64| match fx.param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => fallback,
    };
    PointRegions {
        points: [
            [at("point1_x", 0.0), at("point1_y", 0.0)],
            [at("point2_x", 0.0), at("point2_y", 0.0)],
        ],
        two: follow == lumit_core::fx::effects::planar_track::FOLLOW_TWO_POINTS,
        // A region of nothing would exclude the whole frame and refuse; the
        // schema's own default is the answer to a row that is not there.
        radius: (at("region", 80.0) * 0.5).max(1.0),
    }
}

/// Build the job for a layer wearing a Planar track.
///
/// Filed under the **effect instance**, not the media: what was tracked is the
/// quad, and two Planar tracks on one clip are two answers. The sidecar key
/// still carries the file's fingerprint — the answer depends on the pixels as
/// much as on the quad — so a second project opening the same rushes with the
/// same quad finds the same file.
///
/// `None` when the layer is not footage, or carries no enabled Planar track.
#[must_use]
pub fn planar_job_for(
    layer: &Layer,
    path: PathBuf,
    fingerprint: &Fingerprint,
    analyse: bool,
) -> Option<Job> {
    let LayerKind::Footage { .. } = layer.kind else {
        return None;
    };
    let fx = planar_track_effect(layer)?;
    let settings = AnalysisSettings::of(fx);
    let (kind, masks) = if settings.points() {
        let regions = planar_points(fx, settings.follow);
        (
            JobKind::Points { regions },
            MaskTrack::of(layer, settings, 1.0).within_all(regions.outlines()),
        )
    } else {
        let quad = planar_quad(fx);
        (
            JobKind::Planar { quad },
            MaskTrack::of(layer, settings, 1.0).within(quad_outline(quad)),
        )
    };
    Some(Job {
        media: fx.id,
        key: Some(AnalysisKey::new(fingerprint, settings, &masks)),
        settings,
        kind,
        masks,
        open: Box::new(move || MediaLuma::open(&path).map(|s| Box::new(s) as Box<dyn LumaFrames>)),
        analyse,
    })
}

/// The enabled Track points instances on `layer`, in stack order.
pub fn track_points_effects(layer: &Layer) -> impl Iterator<Item = &EffectInstance> {
    layer
        .effects
        .iter()
        .filter(|e| e.enabled && e.effect.match_name == track_points::MATCH_NAME)
}

/// Build the job for the Track points instance `fx` on `layer`.
///
/// Filed under the effect instance, as a Planar track is and for its reason:
/// two of them on one clip, one following features and one blobs, are two
/// answers. The layer's masks are not read.
///
/// `None` when the layer is not footage.
#[must_use]
pub fn points_job_for(
    layer: &Layer,
    fx: &EffectInstance,
    path: PathBuf,
    fingerprint: &Fingerprint,
    analyse: bool,
) -> Option<Job> {
    let analysis = Analysis::of(fx, layer)?;
    Some(Job {
        media: fx.id,
        key: Some(AnalysisKey::points(fingerprint, &analysis)),
        settings: AnalysisSettings::default(),
        kind: JobKind::TrackPoints(analysis),
        masks: MaskTrack::default(),
        open: Box::new(move || MediaLuma::open(&path).map(|s| Box::new(s) as Box<dyn LumaFrames>)),
        analyse,
    })
}

/// Build the job for a **Precomp** layer wearing a Camera track: the
/// nested composition is the tracked source, and its frames are rendered rather
/// than decoded.
///
/// `None` when the layer is not a precomp, names a comp that is gone, or carries
/// no enabled Camera track. `analyse` is ignored to the extent that a nested
/// comp has no sidecar entry to warm — see [`Job::key`] — so a job built here
/// with `analyse: false` does nothing at all, honestly.
#[must_use]
pub fn job_for_precomp(doc: &Arc<Document>, layer: &Layer, analyse: bool) -> Option<Job> {
    let LayerKind::Precomp { comp: nested } = layer.kind else {
        return None;
    };
    let settings = AnalysisSettings::of(camera_track_effect(layer)?);
    let scale = analysis_scale(doc.comp(nested)?);
    let doc = Arc::clone(doc);
    Some(Job {
        media: nested,
        key: None,
        settings,
        kind: JobKind::Camera,
        // The masks are in the precomp layer's own pixels, which are the nested
        // comp's raster — so they need the render scale, the one case where the
        // factor is not one.
        masks: MaskTrack::of(layer, settings, scale),
        open: Box::new(move || {
            CompLuma::open(doc, nested).map(|s| Box::new(s) as Box<dyn LumaFrames>)
        }),
        analyse,
    })
}

// ---------------------------------------------------------------------------
// The real frames
// ---------------------------------------------------------------------------

/// [`LumaFrames`] over a media file, through the same frame index and decoder
/// every other consumer opens (docs/impl/media-io.md §2).
pub struct MediaLuma {
    decoder: lumit_media::VideoDecoder,
    frames: usize,
    width: u32,
    height: u32,
    fps: f64,
}

impl MediaLuma {
    /// Open `path` for tracking. Costs a frame-index build the first time the
    /// file is seen, which is why this is called on the analysis thread.
    #[must_use]
    pub fn open(path: &Path) -> Option<Self> {
        let video = lumit_media::probe::probe(path).ok()?.video?;
        let index = crate::media_index::load_or_build_index(path).ok()?;
        let frames = index.frame_count();
        let decoder = lumit_media::VideoDecoder::open(path, index).ok()?;
        Some(MediaLuma {
            decoder,
            frames,
            width: video.width,
            height: video.height,
            fps: video.fps(),
        })
    }
}

impl LumaFrames for MediaLuma {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.width, self.height, self.fps)
    }

    fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
        let frame = self.decoder.frame_luma(n).ok()?;
        // A frame that decodes at a different size from the one probed would
        // end the tracker's run with a size error; ending here instead says the
        // same thing without turning a readable clip into a failure.
        (frame.width == self.width && frame.height == self.height).then_some(frame.luma)
    }
}

// ---------------------------------------------------------------------------
// The rendered frames
// ---------------------------------------------------------------------------

/// The longest edge an analysis render is allowed.
///
/// A nested comp has no "source raster" to be unscaled at (§5b's fifth
/// deviation, which is about a *file*): its size is whatever the comp is, and a
/// UHD comp analysed at its own size would render eight million pixels a frame
/// for hundreds of frames to follow a few hundred specks. 960 keeps a
/// half-decent feature scale — the tracker's windows are a dozen pixels across,
/// and a speck a lens can resolve survives a halving — while capping the render
/// bill at roughly a 720p frame. The solve comes back in analysis pixels and is
/// scaled to comp pixels before it is published ([`rescale`]), so nothing
/// downstream can tell what raster it was measured at.
const ANALYSIS_MAX_EDGE: u32 = 960;

/// The render scale one comp is analysed at: 1 for anything already small
/// enough, and otherwise whatever brings its long edge to
/// [`ANALYSIS_MAX_EDGE`].
fn analysis_scale(comp: &Composition) -> f64 {
    let long = comp.width.max(comp.height).max(1);
    f64::from(ANALYSIS_MAX_EDGE) / f64::from(long)
}

/// [`LumaFrames`] over a **nested composition**, rendered frame by frame through
/// the same headless walk an export uses (preview and export are one
/// walk, so an analysis sees exactly the picture the comp makes).
///
/// Its own renderer on its own device, like an export's, so an analysis never
/// contends with the Viewer's GPU work; and built inside [`Job::open`], so the
/// device is created on the analysis thread rather than the caller's.
pub struct CompLuma {
    renderer: crate::headless::HeadlessRenderer,
    doc: Arc<Document>,
    comp: Uuid,
    frames: usize,
    /// The analysis raster — the comp's own size reduced by [`Self::scale`].
    width: u32,
    height: u32,
    fps: f64,
    scale: f32,
}

impl CompLuma {
    /// Open `comp` of `doc` for tracking, or `None` when the comp is gone, has
    /// no frames, or this machine has no graphics adapter to render with.
    #[must_use]
    pub fn open(doc: Arc<Document>, comp_id: Uuid) -> Option<Self> {
        let comp = doc.comp(comp_id)?;
        let fps = comp.frame_rate.fps();
        let frames = comp
            .frame_rate
            .frame_at(lumit_core::time::CompTime(comp.duration.0));
        let frames = usize::try_from(frames).ok()?;
        let scale = analysis_scale(comp);
        #[allow(clippy::cast_possible_truncation)]
        let scale = scale.min(1.0) as f32;
        // The size the renderer will actually hand back, from the renderer's own
        // rounding rather than a second copy of it.
        let (width, height) = lumit_gpu::composite::scaled_size(comp.width, comp.height, scale);
        let renderer = crate::headless::HeadlessRenderer::new().ok()?;
        Some(CompLuma {
            renderer,
            doc,
            comp: comp_id,
            frames,
            width,
            height,
            fps,
            scale,
        })
    }
}

impl LumaFrames for CompLuma {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.width, self.height, self.fps)
    }

    fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
        let frame = u64::try_from(n).ok()?;
        let (rgba, w, h) = self
            .renderer
            .render_rgba(&self.doc, self.comp, frame, self.scale)
            .ok()?;
        if (w, h) != (self.width, self.height) {
            return None;
        }
        // Rec.709 luma off the display-encoded bytes. Which weighting, and
        // whether the bytes are encoded, does not matter to the tracker: it
        // verifies patches by normalised correlation, which is blind to gain
        // and lift, and every frame is converted the same way.
        Some(
            rgba.chunks_exact(4)
                .map(|px| {
                    (0.2126 * f32::from(px[0])
                        + 0.7152 * f32::from(px[1])
                        + 0.0722 * f32::from(px[2]))
                        / 255.0
                })
                .collect(),
        )
    }

    fn analysis_scale(&self) -> f64 {
        f64::from(self.scale)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::{
        BlendMode, Composition, LinearColour, ProjectItem, Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};

    // --- The synthetic shot -------------------------------------------------
    //
    // A camera solve needs pixels, and it needs a scene that is not flat: a
    // single textured plane, however it is moved, is explained by a homography
    // and the solve refuses it as rotation-only, correctly. So the test renders
    // a real one — two textured planes at different depths, the near one
    // present in a coarse checker of patches so both are visible at once and
    // the occlusion is exact — seen by a camera whose path is written down. It
    // is ray-cast per pixel, which is a dozen lines and needs no assets, no
    // encoder and no graphics card.

    const W: usize = 400;
    const H: usize = 300;
    const FRAMES: usize = 24;
    const FPS: f64 = 24.0;
    const FOCAL: f64 = 320.0;
    const NEAR_Z: f64 = 700.0;
    const FAR_Z: f64 = 1300.0;
    /// One checker patch of the near plane, in world units.
    const PATCH: f64 = 260.0;

    /// A deterministic integer hash in 0..1 — the phase-1 tests' splitmix
    /// finaliser, so both halves of the tracking work see the same kind of
    /// picture (docs/impl/tracking.md §5).
    fn hash2(ix: i64, iy: i64) -> f64 {
        let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        ((h >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn noise(x: f64, y: f64) -> f64 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let sx = fx * fx * (3.0 - 2.0 * fx);
        let sy = fy * fy * (3.0 - 2.0 * fy);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let a = hash2(ix, iy);
        let b = hash2(ix + 1, iy);
        let c = hash2(ix, iy + 1);
        let d = hash2(ix + 1, iy + 1);
        let top = a + (b - a) * sx;
        let bot = c + (d - c) * sx;
        top + (bot - top) * sy
    }

    /// Three octaves of value noise: corner-rich for Shi-Tomasi, smooth enough
    /// for a gradient solve. `x` and `y` arrive already scaled to pixels, so
    /// both planes carry features of the same size on screen however far away
    /// they are.
    fn texture(x: f64, y: f64) -> f32 {
        let v = 0.20
            + 0.34 * noise(x / 17.0, y / 17.0)
            + 0.22 * noise(x / 7.0, y / 7.0)
            + 0.14 * noise(x / 3.0, y / 3.0);
        v.clamp(0.0, 1.0) as f32
    }

    fn mul3(a: Mat3, b: Mat3) -> Mat3 {
        let mut m = [[0.0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        m
    }

    /// Rotation about x, then y, then z, in radians, as a world-to-camera
    /// matrix. Any orthonormal matrix would do — this one only has to be a
    /// rotation, and far enough from the identity that the Euler extraction
    /// under test cannot pass by accident.
    fn rotation(rx: f64, ry: f64, rz: f64) -> Mat3 {
        let (sx, cx) = (rx.sin(), rx.cos());
        let (sy, cy) = (ry.sin(), ry.cos());
        let (sz, cz) = (rz.sin(), rz.cos());
        let x = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
        let y = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let z = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
        mul3(mul3(z, y), x)
    }

    /// The truth: where the camera was on frame `n`, and which way it pointed.
    fn truth(n: usize) -> (Mat3, [f64; 3]) {
        let a = n as f64;
        (
            rotation(
                (0.20 * a).to_radians(),
                (0.28 * a).to_radians(),
                (0.32 * a).to_radians(),
            ),
            [-70.0 + 7.0 * a, 25.0 - 2.2 * a, -2.5 * a],
        )
    }

    /// The scene, rendered on demand.
    struct Shot {
        frames: usize,
        /// Frames from here on carry no picture to follow — a whiteout, a lens
        /// cap, a clip running into blank. See [`Shot::degrading`].
        good: usize,
    }

    impl Shot {
        fn new() -> Self {
            Shot {
                frames: FRAMES,
                good: FRAMES,
            }
        }

        fn render(n: usize) -> Vec<f32> {
            let (r, c) = truth(n);
            let (cx, cy) = (W as f64 / 2.0, H as f64 / 2.0);
            let mut out = vec![0.0f32; W * H];
            for py in 0..H {
                for px in 0..W {
                    let d = [px as f64 + 0.5 - cx, py as f64 + 0.5 - cy, FOCAL];
                    // R-transpose times d — the ray direction, in the world.
                    let dir = [
                        r[0][0] * d[0] + r[1][0] * d[1] + r[2][0] * d[2],
                        r[0][1] * d[0] + r[1][1] * d[1] + r[2][1] * d[2],
                        r[0][2] * d[0] + r[1][2] * d[1] + r[2][2] * d[2],
                    ];
                    let hit = |z: f64| -> Option<[f64; 2]> {
                        if dir[2].abs() < 1e-9 {
                            return None;
                        }
                        let s = (z - c[2]) / dir[2];
                        (s > 0.0).then(|| [c[0] + s * dir[0], c[1] + s * dir[1]])
                    };
                    // The near plane where its checker is solid, the far plane
                    // through the holes. Nearest-hit-that-exists is exact
                    // occlusion here, because the far plane is everywhere.
                    let value = match hit(NEAR_Z) {
                        Some(p)
                            if ((p[0] / PATCH).floor() as i64 + (p[1] / PATCH).floor() as i64)
                                .rem_euclid(2)
                                == 0 =>
                        {
                            let k = FOCAL / NEAR_Z;
                            texture(p[0] * k, p[1] * k)
                        }
                        _ => match hit(FAR_Z) {
                            Some(p) => {
                                let k = FOCAL / FAR_Z;
                                texture(p[0] * k + 613.0, p[1] * k - 271.0)
                            }
                            None => 0.5,
                        },
                    };
                    out[py * W + px] = value;
                }
            }
            out
        }
    }

    impl LumaFrames for Shot {
        fn info(&self) -> (usize, u32, u32, f64) {
            (self.frames, W as u32, H as u32, FPS)
        }

        fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
            if n >= self.frames {
                return None;
            }
            Some(if n < self.good {
                Shot::render(n)
            } else {
                vec![0.5f32; W * H]
            })
        }
    }

    /// The whole module's state — the store, the running slot, the sidecar
    /// override — is process-wide, so two of these overlapping would read each
    /// other's solves.
    fn serially() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// Point the sidecar at `dir` and start from an empty store.
    fn with_cache(dir: &Path) {
        *TEST_CACHE_DIR.lock().unwrap() = Some(dir.to_path_buf());
        clear();
    }

    fn fingerprint(tag: &str) -> Fingerprint {
        Fingerprint {
            size: 4096,
            mtime_secs: 0,
            head_tail_hash: tag.into(),
        }
    }

    fn job(media: Uuid, tag: &str, masks: MaskTrack) -> Job {
        let settings = AnalysisSettings::default();
        Job {
            media,
            key: Some(AnalysisKey::new(&fingerprint(tag), settings, &masks)),
            settings,
            kind: JobKind::Camera,
            masks,
            open: Box::new(|| Some(Box::new(Shot::new()) as Box<dyn LumaFrames>)),
            analyse: true,
        }
    }

    /// Run one analysis here and now, keeping every progress reading it
    /// published — the deterministic half, so nothing has to race a thread to
    /// see what it did.
    /// What one analysis answers: the media's rate, the clip's length, and the
    /// solve. Named only so the pair it is half of is readable.
    type Analysed = Result<(f64, usize, Answer), AnalysisError>;

    /// The camera half of [`Answer`], for the tests that asked for one — which
    /// is every one written before the planar track existed.
    type AnalysedCamera = Result<(f64, usize, CameraSolve), AnalysisError>;

    fn run_here_answer(job: Job, cancel: &AtomicBool) -> (Analysed, Vec<Progress>) {
        let log = Mutex::new(Vec::new());
        let out = analyse(job, cancel, &|step| {
            if let Ok(mut held) = log.lock() {
                held.push(step);
            }
        });
        (out, log.into_inner().unwrap())
    }

    fn run_here(job: Job, cancel: &AtomicBool) -> (AnalysedCamera, Vec<Progress>) {
        let (out, log) = run_here_answer(job, cancel);
        let camera = out.map(|(fps, frames, answer)| match answer {
            Answer::Camera(solve) => (fps, frames, *solve),
            _ => panic!("this job asked for a camera solve"),
        });
        (camera, log)
    }

    /// A camera solve as the sidecar holds it.
    fn filed(solve: &CameraSolve) -> Answer {
        Answer::Camera(Box::new(solve.clone()))
    }

    /// Where the compositor's own camera matrix puts a world point, in comp
    /// pixels. The real matrix, not a re-derivation of it — the conversion under
    /// test is only correct if it agrees with what actually draws the frame.
    fn project_through_compositor(pose: &CameraPose, p: [f64; 3]) -> [f64; 2] {
        let m = lumit_gpu::composite::camera_matrix(
            W as f32,
            H as f32,
            pose.zoom as f32,
            (
                pose.position.0 as f32,
                pose.position.1 as f32,
                pose.position.2 as f32,
            ),
            (
                pose.rotation_deg.0 as f32,
                pose.rotation_deg.1 as f32,
                pose.rotation_deg.2 as f32,
            ),
        )
        .to_cols_array();
        // Column-major, so element (row, col) is `m[col * 4 + row]`.
        let v = |row: usize| -> f64 {
            (0..3)
                .map(|k| f64::from(m[k * 4 + row]) * p[k])
                .sum::<f64>()
                + f64::from(m[12 + row])
        };
        let w = v(3);
        [v(0) / w, v(1) / w]
    }

    /// Where the *tracker* puts the same point, through the same solved pose.
    fn project_through_solve(pose: &SolvedPose, p: [f64; 3]) -> Option<[f64; 2]> {
        let d = [
            p[0] - pose.position[0],
            p[1] - pose.position[1],
            p[2] - pose.position[2],
        ];
        let v: Vec<f64> = pose
            .rotation
            .iter()
            .map(|row| row[0] * d[0] + row[1] * d[1] + row[2] * d[2])
            .collect();
        (v[2] > 1e-6).then(|| {
            [
                W as f64 / 2.0 + pose.focal_px * v[0] / v[2],
                H as f64 / 2.0 + pose.focal_px * v[1] / v[2],
            ]
        })
    }

    // --- A comp with a camera linked to the tracked layer -------------------

    fn secs(n: i64, d: i64) -> CompTime {
        CompTime(Rational::new(n, d).unwrap())
    }

    fn layer(name: &str, kind: LayerKind, out: CompTime) -> Layer {
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: name.into(),
            kind,
            in_point: secs(0, 1),
            out_point: out,
            start_offset: secs(0, 1),
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
            graph_inputs: None,
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

    /// A footage layer wearing the Camera track effect, and a Camera layer
    /// linked to it — the shape the camera track describes, built for real
    /// rather than mocked.
    fn linked_document(media: Uuid) -> (Document, Composition) {
        let mut footage = layer(
            "shot",
            LayerKind::Footage { item: media },
            secs(FRAMES as i64, FPS as i64),
        );
        footage
            .effects
            .push(lumit_core::fx::instantiate(CAMERA_TRACK).expect("the effect is registered"));
        // The camera outlives the shot on purpose, so the walk past the end of
        // the solve — the hold — has somewhere to happen.
        let mut camera = layer(
            "Camera 1",
            LayerKind::Camera {
                zoom: Property::fixed(999.0),
                solve_link: Some(footage.id),
                correction_base: None,
                options: Default::default(),
            },
            secs(20, 1),
        );
        // Built already linked, so the base `Op::SetCameraSolveLink` would have
        // captured is written here — the same pose, off the same properties.
        let base = lumit_core::model::stored_camera_pose_lt(&camera, 0.0);
        if let LayerKind::Camera {
            correction_base, ..
        } = &mut camera.kind
        {
            *correction_base = base.map(Box::new);
        }
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "main".into(),
            width: W as u32,
            height: H as u32,
            frame_rate: FrameRate::new(FPS as u32, 1).unwrap(),
            duration: Duration(Rational::new(FRAMES as i64, FPS as i64).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![camera, footage],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        doc.items
            .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                sequence: None,
                id: media,
                name: "shot.mov".into(),
                media: lumit_core::model::MediaRef {
                    relative_path: "shot.mov".into(),
                    absolute_path: String::new(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
                colour_space: None,
                source_layer: None,
            }));
        doc.items.push(ProjectItem::Composition(comp.clone()));
        (doc, comp)
    }

    /// Put the footage item online: a resolved path and a stamped fingerprint,
    /// which is what a relinked project's items carry and the only shape
    /// [`warm_jobs`] will look at.
    fn online(doc: &mut Document, media: Uuid, tag: &str) {
        for item in &mut doc.items {
            if let ProjectItem::Footage(f) = item {
                if f.id == media {
                    f.media.absolute_path = "shot.mov".into();
                    f.media.fingerprint = Some(fingerprint(tag));
                }
            }
        }
    }

    /// A written-down solve: a camera sliding along x, looking straight down
    /// its own z. Every frame distinct, so a warm pass landing on the wrong one
    /// cannot pass, and no tracking is needed to make it.
    fn written_solve() -> CameraSolve {
        let poses = (0..FRAMES as i64)
            .map(|n| SolvedPose {
                frame: n,
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                position: [n as f64 * 3.0, 0.0, -FAR_Z],
                segment: 0,
                focal_px: FOCAL,
                mean_reprojection_px: 0.0,
                source: lumit_track::PoseSource::Keyframe,
            })
            .collect();
        CameraSolve {
            poses,
            segments: vec![lumit_track::SolveSegment {
                first_frame: 0,
                last_frame: FRAMES as i64 - 1,
                focal_px: FOCAL,
                ramp: false,
            }],
            points: vec![lumit_track::ScenePoint {
                track: 0,
                position: [0.0, 0.0, 0.0],
            }],
            keyframes: vec![0, FRAMES as i64 - 1],
            mean_reprojection_px: 0.0,
            notes: Vec::new(),
        }
    }

    /// What the frame key needs to know about the shot: it is a readable video
    /// of this size at this rate. Without a probe the layer is unkeyable by
    /// design, and there would be no key to compare.
    fn probes(media: Uuid) -> HashMap<Uuid, crate::source::SourceProbe> {
        let mut map = HashMap::new();
        map.insert(
            media,
            crate::source::SourceProbe::Video {
                fps: FPS,
                width: W as u32,
                height: H as u32,
                frames: FRAMES,
                audio: false,
            },
        );
        map
    }

    // --- The tests ----------------------------------------------------------

    /// The whole of stage 2 in one run: a synthetic shot is tracked and solved,
    /// progress is published as it goes, the solve lands in the store, and a
    /// Camera layer linked to the tracked layer reads it frame for frame.
    ///
    /// The conversion — the interesting half of this file — is checked against
    /// `lumit_gpu::composite::camera_matrix` itself: for every solved frame and
    /// every point in the cloud, the compositor's own matrix must put the point
    /// exactly where the tracker put it. That is an algebraic identity, so it
    /// neither flakes on solve quality nor passes on a solve that failed — the
    /// assertions above it hold the solve to its own standard separately.
    #[test]
    fn an_analysis_solves_a_shot_and_a_linked_camera_reads_it() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();

        let cancel = AtomicBool::new(false);
        let (out, steps) = run_here(job(media, "solve", MaskTrack::default()), &cancel);
        let (fps, clip_frames, solve) = out.expect("the synthetic shot solves");
        assert_eq!(clip_frames, FRAMES, "the whole clip was followed");

        // Progress is observable, and it is honest: it starts at nothing done,
        // reaches every frame, and only then says it is solving.
        assert_eq!(
            steps.first(),
            Some(&Progress::Tracking {
                done: 0,
                total: FRAMES
            })
        );
        assert!(steps.contains(&Progress::Tracking {
            done: FRAMES,
            total: FRAMES
        }));
        assert_eq!(steps.last(), Some(&Progress::Solving));

        // The solve stands up on its own terms before anything is asked of the
        // conversion: every frame posed, the lens recovered, and the rotations
        // far enough from the identity that an Euler mistake cannot hide.
        assert_eq!(solve.poses.len(), FRAMES, "a pose per frame");
        let focal = solve.segments.first().unwrap().focal_px;
        assert!(
            (focal - FOCAL).abs() / FOCAL < 0.08,
            "focal recovered as {focal} against a true {FOCAL}"
        );
        let turn = solve
            .poses
            .iter()
            .map(|p| {
                (0..3)
                    .flat_map(|i| (0..3).map(move |j| (i, j)))
                    .map(|(i, j)| (p.rotation[i][j] - if i == j { 1.0 } else { 0.0 }).abs())
                    .fold(0.0f64, f64::max)
            })
            .fold(0.0f64, f64::max);
        assert!(
            turn > 0.05,
            "the camera barely turned ({turn}), so the Euler extraction is not being tested"
        );

        // The conversion, against the matrix that actually draws the frame.
        let solved = Solved::new(fps, FRAMES, solve).expect("the solve has frames");
        assert!(
            !solved.is_partial(),
            "the whole clip was followed, so nothing is partial about it"
        );
        let mut worst = 0.0f64;
        let mut compared = 0usize;
        for pose in &solved.solve.poses {
            let lumit = solved.pose(pose.frame).expect("every frame converts");
            for point in &solved.solve.points {
                let Some(want) = project_through_solve(pose, point.position) else {
                    continue;
                };
                let got = project_through_compositor(&lumit, point.position);
                worst = worst.max((got[0] - want[0]).abs().max((got[1] - want[1]).abs()));
                compared += 1;
            }
        }
        assert!(compared > 500, "only {compared} projections were compared");
        assert!(
            worst < 0.05,
            "the compositor and the tracker disagree by {worst} px about where a solved point is"
        );

        // Into the store, and out through the link.
        publish(media, solved.fps, solved.clip_frames, solved.solve.clone());
        let (doc, comp) = linked_document(media);
        let mut seen: Vec<CameraPose> = Vec::new();
        for n in 0..FRAMES {
            let got = linked_pose(&doc, &comp, n as f64 / FPS).expect("the comp has a camera");
            assert_eq!(
                got.state,
                lumit_core::track::LinkState::Derived,
                "frame {n} did not resolve through the link"
            );
            assert_eq!(
                Some(got.pose),
                solved.pose(n as i64),
                "frame {n} reads a different pose from the one the store holds"
            );
            seen.push(got.pose);
        }
        assert!(
            seen.windows(2).filter(|w| w[0] != w[1]).count() > FRAMES / 2,
            "the linked camera did not actually move"
        );
        // Past the solved range the last derived motion holds, and says so.
        let past = linked_pose(&doc, &comp, 10.0).unwrap();
        assert_eq!(past.state, lumit_core::track::LinkState::Held);
        assert_eq!(Some(past.pose), solved.pose(solved.last_frame));
        clear();
    }

    /// **Track once, then nudge**, from the render path's side: a
    /// correction moves the frame's *name* as well as its picture, and a second
    /// analysis lands under it rather than over it.
    ///
    /// The second claim is the one that matters. A correction that was folded
    /// into the camera's own numbers would be silently replaced by the next
    /// solve; this asserts that the same nudge is still on top of a solve whose
    /// every pose is different.
    #[test]
    fn a_correction_moves_the_frame_key_and_survives_a_second_solve() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        let (doc, comp) = linked_document(media);

        // Nudged: fifty pixels along x and three degrees about y, written the
        // way a drag writes them — straight onto the camera's own properties.
        let mut nudged = comp.clone();
        {
            let cam = nudged
                .layers
                .iter_mut()
                .find(|l| matches!(l.kind, LayerKind::Camera { .. }))
                .expect("the camera");
            cam.transform.position_x = Property::fixed(50.0);
            cam.transform.rotation_y = Property::fixed(3.0);
        }

        let probed = probes(media);
        let doc = Arc::new(doc);
        let key = |c: &Composition| {
            let stamper =
                crate::cache::Stamper::new(&doc, &probed, crate::plan::Quality::default());
            lumit_eval::comp_frame_key(&doc, c, 0.0, lumit_eval::Quality::default(), &stamper)
        };
        let pose = |c: &Composition| {
            linked_pose(&doc, c, 0.5)
                .expect("the comp has a camera")
                .pose
        };

        publish(media, FPS, FRAMES, written_solve());
        let plain = pose(&comp);
        let corrected = pose(&nudged);
        assert!((corrected.position.0 - (plain.position.0 + 50.0)).abs() < 1e-9);
        assert!((corrected.rotation_deg.1 - (plain.rotation_deg.1 + 3.0)).abs() < 1e-9);

        assert_ne!(
            key(&comp).expect("a probed comp is keyable"),
            key(&nudged).expect("still keyable"),
            "a correction changes the picture, so it must change the frame's name"
        );

        // Analyse again, to a different answer. The correction is not part of
        // the solve, so it is still exactly on top of it.
        let mut again = written_solve();
        for solved in &mut again.poses {
            solved.position[1] += 40.0;
        }
        publish(media, FPS, FRAMES, again);
        let fresh = pose(&comp);
        assert_ne!(fresh, plain, "the second solve is a different answer");
        let resolved = pose(&nudged);
        assert!((resolved.position.0 - (fresh.position.0 + 50.0)).abs() < 1e-9);
        assert!((resolved.rotation_deg.1 - (fresh.rotation_deg.1 + 3.0)).abs() < 1e-9);
        clear();
    }

    /// Cancelling stops between frames, writes nothing at all, and leaves the
    /// next run to do the whole job cleanly.
    #[test]
    fn cancelling_leaves_no_cache_entry_and_a_clean_rerun() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        let key = AnalysisKey::new(
            &fingerprint("cancel"),
            AnalysisSettings::default(),
            &MaskTrack::default(),
        );

        // Raised after a handful of frames, which is where a real Cancel lands.
        let cancel = AtomicBool::new(false);
        let mut first = job(media, "cancel", MaskTrack::default());
        first.key = Some(key);
        let log = Mutex::new(Vec::new());
        let out = analyse(first, &cancel, &|step| {
            if let Progress::Tracking { done, .. } = step {
                if done >= 5 {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            if let Ok(mut held) = log.lock() {
                held.push(step);
            }
        });
        assert_eq!(out, Err(AnalysisError::Cancelled));
        let steps = log.into_inner().unwrap();
        assert!(
            !steps.contains(&Progress::Solving),
            "a cancelled run got as far as solving"
        );

        // Nothing written, because `run` is what writes and it never had an
        // answer to write.
        assert!(read_sidecar(dir.path(), key).is_none());
        assert!(solved(media).is_none(), "a cancelled run published a solve");

        // And the same job, uncancelled, is a whole clean analysis.
        let clean = AtomicBool::new(false);
        let mut again = job(media, "cancel", MaskTrack::default());
        again.key = Some(key);
        let (out, _) = run_here(again, &clean);
        assert!(out.is_ok(), "the rerun after a cancel found nothing broken");
        clear();
    }

    /// The sidecar's whole contract: a rebuild and a cache hit agree bit for
    /// bit, a file written by a newer Lumit is refused rather than believed, and
    /// deleting the folder costs an analysis and nothing else.
    #[test]
    fn a_cache_hit_and_a_rebuild_are_the_same_bytes() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        let key = AnalysisKey::new(
            &fingerprint("cache"),
            AnalysisSettings::default(),
            &MaskTrack::default(),
        );
        let cancel = AtomicBool::new(false);

        let mut first = job(media, "cache", MaskTrack::default());
        first.key = Some(key);
        let (fps, clip_frames, solve) = run_here(first, &cancel).0.unwrap();
        write_sidecar(dir.path(), key, fps, clip_frames, &filed(&solve));
        let written = std::fs::read(dir.path().join(key.file_name())).unwrap();

        // Read back: the same solve, and the same file when written again.
        let (read_fps, read_clip, read_solve) =
            read_sidecar(dir.path(), key).expect("the sidecar is there");
        assert_eq!(read_fps, fps);
        assert_eq!(
            read_clip, clip_frames,
            "the clip's own length did not survive the round trip, so a cached              partial solve would read back as a whole one"
        );
        assert_eq!(
            read_solve,
            filed(&solve),
            "the round trip changed the solve"
        );
        assert_eq!(
            encode(key, read_fps, read_clip, &read_solve).unwrap(),
            written,
            "re-encoding what was read gives different bytes"
        );

        // A second analysis of the same input is the same solve — which is what
        // makes the cache safe to trust at all.
        let mut again = job(media, "cache", MaskTrack::default());
        again.key = Some(key);
        let (fps_again, clip_again, solve_again) = run_here(again, &cancel).0.unwrap();
        assert_eq!(fps_again, fps);
        assert_eq!(
            encode(key, fps_again, clip_again, &filed(&solve_again)).unwrap(),
            written,
            "a rebuild and a cache hit are not the same bits"
        );

        // A different key does not read this file, however tempting.
        let other = AnalysisKey::new(
            &fingerprint("cache"),
            AnalysisSettings {
                density: 0,
                ..AnalysisSettings::default()
            },
            &MaskTrack::default(),
        );
        assert!(read_sidecar(dir.path(), other).is_none());
        // Neither does a file from a version this build does not know.
        let mut future = written.clone();
        future[7] = 0xff;
        future[8] = 0xff;
        assert!(decode(&future, key).is_none(), "a newer file was believed");
        assert!(decode(b"not ours at all", key).is_none());

        // Deleting the sidecar is always safe: the next read finds nothing and
        // the next analysis rebuilds the identical answer.
        std::fs::remove_file(dir.path().join(key.file_name())).unwrap();
        assert!(read_sidecar(dir.path(), key).is_none());
        let mut rebuilt = job(media, "cache", MaskTrack::default());
        rebuilt.key = Some(key);
        let (fps_rebuilt, clip_rebuilt, solve_rebuilt) = run_here(rebuilt, &cancel).0.unwrap();
        assert_eq!(
            encode(key, fps_rebuilt, clip_rebuilt, &filed(&solve_rebuilt)).unwrap(),
            written,
            "a rebuild after a delete is not what was deleted"
        );
        clear();
    }

    /// A mask on the tracked layer is a region the analysis does not enter
    /// (the mask geometry, read by the tracker): nothing solved comes back from
    /// inside it. The same shot without the mask *does* put points there, which
    /// is what stops this passing on a region that was empty anyway.
    #[test]
    fn a_masked_region_births_no_tracks() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let cancel = AtomicBool::new(false);

        // A rectangle over most of the top-left quarter, in the layer's own
        // pixels — which for a footage layer are the source raster's, so the
        // tracker needs no conversion.
        let mut tracked = layer(
            "shot",
            LayerKind::Footage {
                item: Uuid::now_v7(),
            },
            secs(1, 1),
        );
        tracked.masks.push(lumit_core::mask::Mask::rectangle(
            20.0,
            20.0,
            (W / 2) as f64 - 40.0,
            (H / 2) as f64 - 40.0,
        ));
        let track = MaskTrack::of(&tracked, AnalysisSettings::default(), 1.0);
        let masks = track.at(0.0);
        assert_eq!(masks.len(), 1);
        assert!(
            !track.animated(),
            "a drawn rectangle does not move on its own"
        );

        // Asked of the tracks, not of the solved cloud: a solved point is a
        // *place in space*, and projecting it through frames the track was not
        // alive on would count positions the tracker never visited. The claim is
        // about where features were followed, so it is asked where features
        // were followed.
        let inside = |set: &lumit_track::TrackSet, mask: &ExclusionMask| -> usize {
            set.tracks()
                .iter()
                .flat_map(|t| t.points.iter())
                .filter(|p| mask.excludes(p.x, p.y))
                .count()
        };

        let media = Uuid::now_v7();
        let unmasked = track_frames(
            job(media, "unmasked", MaskTrack::default()),
            &cancel,
            &|_| {},
        )
        .unwrap()
        .2;
        let masked = track_frames(job(media, "masked", track.clone()), &cancel, &|_| {})
            .unwrap()
            .2;

        assert!(
            inside(&unmasked, &masks[0]) > 0,
            "the region is empty even unmasked, so masking it proves nothing"
        );
        assert_eq!(
            inside(&masked, &masks[0]),
            0,
            "a track lived inside a masked region"
        );
        // And the mask took only what it covers: the rest of the frame is still
        // tracked, so this is an exclusion and not a failure to track at all.
        assert!(
            masked.tracks().len() > unmasked.tracks().len() / 2,
            "masking a quarter of the frame cost most of the tracks"
        );
        clear();
    }

    /// Opening a project reads its solves back, and closing it lets them go.
    ///
    /// The whole point of the warm pass: the analysis ran in some earlier
    /// session, the answer has been sitting in the sidecar ever since, and the
    /// linked camera must resolve on the first frame drawn — with nobody
    /// pressing Analyse and nothing decoded. The job the pass builds is checked
    /// against the key the file was written under, because a warm pass asking
    /// for a *different* analysis would find nothing and look identical to
    /// having no cache at all.
    #[test]
    fn opening_a_project_reads_its_cached_solves_back() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        let (mut doc, comp) = linked_document(media);
        online(&mut doc, media, "warm-open");

        let key = AnalysisKey::new(
            &fingerprint("warm-open"),
            AnalysisSettings::default(),
            &MaskTrack::default(),
        );
        let solve = written_solve();
        write_sidecar(dir.path(), key, FPS, FRAMES, &filed(&solve));
        assert_eq!(
            linked_pose(&doc, &comp, 0.0).map(|l| l.state),
            Some(lumit_core::track::LinkState::Unresolved),
            "the store starts empty, so the link starts unresolved"
        );

        let jobs = warm_jobs(&doc);
        assert_eq!(jobs.len(), 1, "one tracked clip, one warm job");
        assert_eq!(
            jobs[0].key,
            Some(key),
            "the warm pass asked for a different analysis from the one on disk"
        );
        assert!(!jobs[0].analyse, "a warm job may never start tracking");
        warm(jobs);

        let mut waited = 0;
        while !matches!(progress(media), Some(Progress::Done)) {
            assert!(waited < 500, "the warm pass never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
        }

        for n in [0i64, 5, FRAMES as i64 - 1] {
            let got = linked_pose(&doc, &comp, n as f64 / FPS).expect("the comp has a camera");
            assert_eq!(
                got.state,
                lumit_core::track::LinkState::Derived,
                "frame {n} did not resolve without Analyse being pressed"
            );
            assert_eq!(Some(got.pose), solved(media).and_then(|s| s.pose(n)));
        }

        // And closing the project lets them go, without touching the file that
        // would warm them again.
        clear();
        assert!(
            solved(media).is_none(),
            "closing kept the solve in the store"
        );
        assert!(
            read_sidecar(dir.path(), key).is_some(),
            "closing deleted the sidecar it must never touch"
        );
    }

    // --- The precomp path ---------------------------------------------------

    /// A nested comp that pans a field of solids past the camera, at a raster
    /// big enough for the analysis cap to bite, plus a Precomp layer wearing a
    /// Camera track. `(doc, the nested comp's id, the precomp layer)`.
    fn nested_pan(frames: usize) -> (Arc<Document>, Uuid, Layer) {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        use lumit_core::model::SolidDef;

        let mut doc = Document::new();
        let nested_id = Uuid::now_v7();
        let (cw, ch) = (1280u32, 720u32);

        // One oversized solid carrying Fractal noise, panned across the comp.
        // Noise rather than a field of squares: the tracker's step is a
        // linearisation of the picture around each patch, so it wants texture
        // with a gradient of some width, and a hard-edged rectangle gives it a
        // one-pixel cliff and nothing else — sixty corners are detected and not
        // one of them survives a four-pixel step. This is the same reason the
        // synthetic shot above is procedurally textured.
        let def = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: "Field".into(),
            colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
            width: cw + 400,
            height: ch + 400,
            extra: serde_json::Map::new(),
        }));
        let mut field = layer(
            "field",
            LayerKind::Solid { def },
            secs(frames as i64, FPS as i64),
        );
        field.transform.anchor_x = Property::fixed(f64::from(cw + 400) / 2.0);
        field.transform.anchor_y = Property::fixed(f64::from(ch + 400) / 2.0);
        field.transform.position_y = Property::fixed(f64::from(ch) / 2.0);
        // Moving the layer moves the pattern with it: a generator draws into the
        // layer's own raster and the transform places that raster, so this is
        // one rigid pan and nothing else changes between frames.
        field.transform.position_x = Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: Rational::new(0, 1).unwrap(),
                    value: f64::from(cw) / 2.0,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
                Keyframe {
                    time: Rational::new(frames as i64, FPS as i64).unwrap(),
                    value: f64::from(cw) / 2.0 + 60.0,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        };
        field
            .effects
            .push(lumit_core::fx::instantiate("fractal_noise").expect("the effect is registered"));

        doc.items.push(ProjectItem::Composition(Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: nested_id,
            name: "nested".into(),
            width: cw,
            height: ch,
            frame_rate: FrameRate::new(FPS as u32, 1).unwrap(),
            duration: Duration(Rational::new(frames as i64, FPS as i64).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![field],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }));

        let mut precomp = layer(
            "nested",
            LayerKind::Precomp { comp: nested_id },
            secs(frames as i64, FPS as i64),
        );
        precomp
            .effects
            .push(lumit_core::fx::instantiate(CAMERA_TRACK).expect("the effect is registered"));
        (Arc::new(doc), nested_id, precomp)
    }

    /// The analysis reads **rendered** frames of a nested comp, at the analysis
    /// raster rather than the comp's own, and follows features through them.
    ///
    /// The claim is the frame source, not the solve: whether a given shot solves
    /// is the solver's business and is tested on the synthetic one above. What
    /// has to be true here is that frames arrive from the compositor at the
    /// capped size, that they are different frames, and that the tracker carries
    /// features from one to the next through the same loop a decoded clip goes
    /// through — cancellation seam included, since it is that same loop.
    #[test]
    fn a_precomp_analysis_follows_rendered_frames_at_the_analysis_raster() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        const N: usize = 10;
        let (doc, nested_id, precomp) = nested_pan(N);

        let mut source = match CompLuma::open(Arc::clone(&doc), nested_id) {
            Some(s) => s,
            None => {
                eprintln!("skipping: no GPU adapter");
                return;
            }
        };
        // 1280 × 720 capped to a 960 long edge: three quarters.
        assert_eq!(source.info(), (N, 960, 540, FPS));
        assert!((source.analysis_scale() - 0.75).abs() < 1e-6);
        let first = source.luma(0).expect("the compositor rendered a frame");
        assert_eq!(first.len(), 960 * 540);
        let later = source.luma(N - 1).expect("and the last one");
        assert_ne!(
            first, later,
            "the source handed back the same picture twice"
        );
        drop(source);

        // And through the whole job, which is the seam that matters.
        let cancel = AtomicBool::new(false);
        let (fps, total, set, scale) = track_frames(
            job_for_precomp(&doc, &precomp, true).unwrap(),
            &cancel,
            &|_| {},
        )
        .expect("the rendered frames were tracked");
        assert_eq!((fps, total), (FPS, N));
        assert!((scale - 0.75).abs() < 1e-6);
        let carried = set
            .tracks()
            .iter()
            .filter(|t| t.points.len() > N / 2)
            .count();
        assert!(
            carried >= MIN_CARRIED,
            "only {carried} of {} features survived the rendered comp",
            set.tracks().len()
        );

        // The cancellation seam is the frame loop, the same one a decoded clip
        // uses: a raised flag refuses before a frame is rendered.
        let stopped = AtomicBool::new(true);
        assert_eq!(
            track_frames(
                job_for_precomp(&doc, &precomp, true).unwrap(),
                &stopped,
                &|_| {}
            )
            .err(),
            Some(AnalysisError::Cancelled)
        );
        clear();
    }

    // -----------------------------------------------------------------------
    // The planar track
    // -----------------------------------------------------------------------

    /// A flat, textured surface sliding across the frame — everything a planar
    /// track needs, and nothing it does not.
    ///
    /// Deliberately *not* [`Shot`]: that fixture is two planes at different
    /// depths, arranged so the solve has parallax to find, and parallax is
    /// exactly what a planar track has no use for. One plane under a written-down
    /// projective motion is the ground truth here, and the motion is written as a
    /// homography so the test's expectation is the same kind of object the
    /// tracker produces.
    struct PlaneShot;

    impl PlaneShot {
        const FRAMES: usize = 14;

        /// Where a point of frame 0 sits on frame `n`: a slide with a mild
        /// perspective tilt, so an affine tracker could not fake this.
        fn warp(n: usize) -> Mat3 {
            let d = n as f64;
            let (cx, cy) = (W as f64 / 2.0, H as f64 / 2.0);
            let to = [[1.0, 0.0, -cx], [0.0, 1.0, -cy], [0.0, 0.0, 1.0]];
            let fro = [[1.0, 0.0, cx], [0.0, 1.0, cy], [0.0, 0.0, 1.0]];
            let m = [
                [1.0, 0.0, 2.1 * d],
                [0.0, 1.0, -1.3 * d],
                [0.00012 * d, 0.00006 * d, 1.0],
            ];
            mul3(fro, mul3(m, to))
        }

        fn corner_at(n: usize, p: [f64; 2]) -> [f64; 2] {
            lumit_track::project(&Self::warp(n), p).expect("the fixture's warps stay finite")
        }
    }

    impl LumaFrames for PlaneShot {
        fn info(&self) -> (usize, u32, u32, f64) {
            (Self::FRAMES, W as u32, H as u32, FPS)
        }

        fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
            if n >= Self::FRAMES {
                return None;
            }
            // The inverse warp, because rendering asks where an output pixel
            // came from.
            let back = invert(Self::warp(n))?;
            let mut out = vec![0.35f32; W * H];
            for y in 0..H {
                for x in 0..W {
                    let p = lumit_track::project(&back, [x as f64, y as f64])?;
                    if (30.0..370.0).contains(&p[0]) && (20.0..280.0).contains(&p[1]) {
                        out[y * W + x] = texture(p[0], p[1]);
                    }
                }
            }
            Some(out)
        }
    }

    /// The 3×3 inverse, by the adjugate — the only piece of linear algebra this
    /// fixture needs, written out rather than looped because nine cofactors in a
    /// loop is harder to check than nine cofactors on the page.
    fn invert(m: Mat3) -> Option<Mat3> {
        let adj = [
            [
                m[1][1] * m[2][2] - m[1][2] * m[2][1],
                m[0][2] * m[2][1] - m[0][1] * m[2][2],
                m[0][1] * m[1][2] - m[0][2] * m[1][1],
            ],
            [
                m[1][2] * m[2][0] - m[1][0] * m[2][2],
                m[0][0] * m[2][2] - m[0][2] * m[2][0],
                m[0][2] * m[1][0] - m[0][0] * m[1][2],
            ],
            [
                m[1][0] * m[2][1] - m[1][1] * m[2][0],
                m[0][1] * m[2][0] - m[0][0] * m[2][1],
                m[0][0] * m[1][1] - m[0][1] * m[1][0],
            ],
        ];
        let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
        (det.abs() > 1e-12).then(|| adj.map(|row| row.map(|v| v / det)))
    }

    /// The quad the planar tests follow, well inside the textured region.
    const PIN_QUAD: Quad = [[90.0, 60.0], [300.0, 60.0], [90.0, 230.0], [300.0, 230.0]];

    fn planar_job(track: Uuid, tag: &str) -> Job {
        let settings = AnalysisSettings::default();
        let masks = MaskTrack::default().within(quad_outline(PIN_QUAD));
        Job {
            media: track,
            key: Some(AnalysisKey::new(&fingerprint(tag), settings, &masks)),
            settings,
            kind: JobKind::Planar { quad: PIN_QUAD },
            masks,
            open: Box::new(|| Some(Box::new(PlaneShot) as Box<dyn LumaFrames>)),
            analyse: true,
        }
    }

    /// The whole planar path, end to end: a real analysis of a rendered shot,
    /// the corners against the warp each frame was drawn under, the answer in
    /// the store, and a Corner pin written from it that lands on the surface.
    #[test]
    fn a_planar_analysis_follows_a_surface_and_writes_a_corner_pin() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());

        let effect = Uuid::now_v7();
        let cancel = AtomicBool::new(false);
        let (out, log) = run_here_answer(planar_job(effect, "planar"), &cancel);
        let (fps, clip_frames, answer) = out.expect("a textured plane is followable");
        assert_eq!(fps, FPS);
        assert_eq!(clip_frames, PlaneShot::FRAMES);
        assert!(
            log.contains(&Progress::Solving),
            "the planar half publishes the same readings the camera half does"
        );
        let Answer::Planar(track) = answer else {
            panic!("a planar job answered with a camera solve");
        };
        assert_eq!(
            track.frames.len(),
            PlaneShot::FRAMES,
            "every frame of a clean slide should be followed"
        );

        // The corners, against the ground truth each frame was rendered under.
        let mut worst = 0.0f64;
        for f in &track.frames {
            let n = usize::try_from(f.frame).unwrap();
            for (got, corner) in f.corners.iter().zip(PIN_QUAD) {
                let want = PlaneShot::corner_at(n, corner);
                worst = worst.max((got[0] - want[0]).hypot(got[1] - want[1]));
            }
        }
        // Measured 0.94 px at the worst corner of the most warped frame, with
        // no re-anchor anywhere — every frame measured against frame 0 directly,
        // which is the claim that separates this from a chained tracker.
        assert!(worst < 1.5, "worst corner error {worst} px");
        assert_eq!(track.reanchors, 0);
        assert!(
            track.frames.iter().skip(1).all(|f| f.inliers > 100),
            "the surface should carry a crowd of correspondences, not a handful"
        );

        // Into the store, and out again through the trait `lumit-core` reads.
        publish_planar(effect, fps, clip_frames, *track);
        let store = Store;
        let range = store
            .planar_range(effect)
            .expect("the track is in the store");
        assert_eq!(range.fps, FPS);
        assert_eq!(
            (range.first_frame, range.last_frame),
            (0, PlaneShot::FRAMES as i64 - 1)
        );
        assert!(store.planar_corners(effect, 5).is_some());
        assert!(
            store.planar_corners(Uuid::now_v7(), 5).is_none(),
            "a different instance is a different answer, not this one"
        );

        // And the gesture the whole thing exists for: a Corner pin on another
        // layer, keyed to the surface. The pin's numbers are the store's,
        // through the comp's own clock.
        let media = Uuid::now_v7();
        let span = secs(PlaneShot::FRAMES as i64, FPS as i64);
        let mut shot = layer("shot", LayerKind::Footage { item: media }, span);
        let mut fx = lumit_core::fx::instantiate(PLANAR_TRACK).unwrap();
        fx.id = effect;
        shot.effects.push(fx);
        let target = layer("screen", LayerKind::Null, span);
        let (tracked_id, target_id) = (shot.id, target.id);
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "main".into(),
            width: W as u32,
            height: H as u32,
            frame_rate: FrameRate::new(FPS as u32, 1).unwrap(),
            duration: Duration(Rational::new(PlaneShot::FRAMES as i64, FPS as i64).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![target, shot],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));

        let op = lumit_core::track::corner_pin_from_track(
            &doc, comp_id, tracked_id, effect, target_id, &store,
        )
        .expect("a track in the store writes a pin");
        lumit_core::ops::apply(&mut doc, &op).unwrap();
        let pinned = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == target_id)
            .unwrap()
            .effects
            .last()
            .expect("the pin was appended");
        assert_eq!(pinned.effect.match_name, "corner_pin");
        let read = |id: &str, t: f64| match pinned.param(id) {
            Some(EffectValue::Float(p)) => p.value_at(t),
            _ => panic!("the pin has no {id}"),
        };
        for n in [0usize, 6, 13] {
            let t = n as f64 / FPS;
            let want = PlaneShot::corner_at(n, PIN_QUAD[0]);
            let got = (read("upper_left_x", t), read("upper_left_y", t));
            assert!(
                (got.0 - want[0]).hypot(got.1 - want[1]) < 1.5,
                "the pin's upper left is {got:?} at frame {n}, wanted {want:?}"
            );
        }
        clear();
    }

    // -----------------------------------------------------------------------
    // The point track
    // -----------------------------------------------------------------------

    /// Two textured squares on a flat background, each sliding its **own** way.
    /// Deliberately not [`PlaneShot`]: the whole claim of a point track is that
    /// the two things it follows need not be on one surface, and a fixture where
    /// they were would prove nothing the planar test does not already.
    struct PatchShot;

    impl PatchShot {
        const FRAMES: usize = 12;
        /// Half a patch's side.
        const HALF: f64 = 26.0;
        const P1: [f64; 2] = [110.0, 90.0];
        const P2: [f64; 2] = [290.0, 200.0];

        /// Where each patch's centre is on frame `n` — two unrelated slides.
        fn centres(n: usize) -> [[f64; 2]; 2] {
            let d = n as f64;
            [
                [Self::P1[0] + 1.8 * d, Self::P1[1] + 0.4 * d],
                [Self::P2[0] - 1.1 * d, Self::P2[1] + 1.6 * d],
            ]
        }
    }

    impl LumaFrames for PatchShot {
        fn info(&self) -> (usize, u32, u32, f64) {
            (Self::FRAMES, W as u32, H as u32, FPS)
        }

        fn luma(&mut self, n: usize) -> Option<Vec<f32>> {
            if n >= Self::FRAMES {
                return None;
            }
            let now = Self::centres(n);
            let home = Self::centres(0);
            let mut out = vec![0.35f32; W * H];
            for y in 0..H {
                for x in 0..W {
                    let (fx, fy) = (x as f64, y as f64);
                    for (c, r) in now.iter().zip(home) {
                        if (fx - c[0]).abs() <= Self::HALF && (fy - c[1]).abs() <= Self::HALF {
                            // Sampled in the patch's own frame-0 coordinates, so
                            // each carries its texture along with it.
                            out[y * W + x] = texture(fx - c[0] + r[0], fy - c[1] + r[1]);
                        }
                    }
                }
            }
            Some(out)
        }
    }

    fn point_job(track: Uuid, tag: &str, two: bool) -> Job {
        let home = PatchShot::centres(0);
        let regions = PointRegions {
            points: home,
            two,
            radius: 40.0,
        };
        let settings = AnalysisSettings {
            follow: if two {
                lumit_core::fx::effects::planar_track::FOLLOW_TWO_POINTS
            } else {
                lumit_core::fx::effects::planar_track::FOLLOW_ONE_POINT
            },
            ..AnalysisSettings::default()
        };
        let masks = MaskTrack::default().within_all(regions.outlines());
        Job {
            media: track,
            key: Some(AnalysisKey::new(&fingerprint(tag), settings, &masks)),
            settings,
            kind: JobKind::Points { regions },
            masks,
            open: Box::new(|| Some(Box::new(PatchShot) as Box<dyn LumaFrames>)),
            analyse: true,
        }
    }

    /// The whole point path, end to end: a real analysis of two independently
    /// moving patches, the reported box against the similarity the render
    /// actually applied, and the answer reaching the store.
    #[test]
    fn a_two_point_analysis_follows_both_patches_and_reaches_the_store() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());

        let effect = Uuid::now_v7();
        let cancel = AtomicBool::new(false);
        let (out, _) = run_here_answer(point_job(effect, "points", true), &cancel);
        let (fps, clip_frames, answer) = out.expect("two textured patches are followable");
        assert_eq!(fps, FPS);
        assert_eq!(clip_frames, PatchShot::FRAMES);
        let Answer::Planar(track) = answer else {
            panic!("a point job answers with a track");
        };
        assert_eq!(
            track.frames.len(),
            PatchShot::FRAMES,
            "every frame was followed"
        );
        assert_eq!(track.reanchors, 0, "nothing thinned over twelve frames");

        // The truth is the similarity taking the two reference centres to where
        // the fixture actually put them — an exact formula, not a measurement.
        let home = PatchShot::centres(0);
        let mut worst = 0.0f64;
        for f in &track.frames {
            let now = PatchShot::centres(f.frame as usize);
            let (ax, ay) = (home[1][0] - home[0][0], home[1][1] - home[0][1]);
            let (bx, by) = (now[1][0] - now[0][0], now[1][1] - now[0][1]);
            let d = ax * ax + ay * ay;
            let (wr, wi) = ((bx * ax + by * ay) / d, (by * ax - bx * ay) / d);
            for (got, corner) in f.corners.iter().zip(track.reference_quad) {
                let (ex, ey) = (corner[0] - home[0][0], corner[1] - home[0][1]);
                let want = [now[0][0] + wr * ex - wi * ey, now[0][1] + wi * ex + wr * ey];
                worst = worst.max((got[0] - want[0]).hypot(got[1] - want[1]));
            }
        }
        assert!(worst < 2.0, "worst corner error {worst} px");

        // And into the store, under the effect instance, readable the way the
        // Corner pin and the transform keys read it.
        publish_planar(effect, fps, clip_frames, *track);
        let range = Store.planar_range(effect).expect("filed under the effect");
        assert_eq!(range.first_frame, 0);
        assert_eq!(
            range.last_frame,
            PatchShot::FRAMES as i64 - 1,
            "the whole clip was followed"
        );
        assert!(Store.planar_corners(effect, 5).is_some());
        clear();
    }

    /// A Scatter's points are made while a frame is drawn and exist nowhere
    /// else, so a driver wired to them counts none. Baked, they are plain
    /// data: a driver wired to the Bake points counts them, and the frame
    /// drawn from the bake is the frame drawn from the live stream.
    #[test]
    fn a_baked_scatter_is_read_by_a_driver_and_draws_as_it_did_live() {
        use lumit_core::fx::drivers::points_sample::{COUNT_PORT, POINTS_PORT};
        use lumit_core::graph::{Edge, InputRef, NodeRef, OutputRef};
        use lumit_core::model::SolidDef;

        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let Ok(mut renderer) = crate::headless::HeadlessRenderer::new() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };

        let set = |inst: &mut EffectInstance, id: &str, value: EffectValue| {
            inst.params.iter_mut().find(|p| p.id == id).unwrap().value = value;
        };
        let number = |v: f64| EffectValue::Float(Property::fixed(v));
        let wire = |from: OutputRef, node: NodeRef, port: &str| Edge {
            from,
            to: InputRef::Param {
                node,
                port: port.into(),
            },
        };
        let stream_of = |effect: Uuid| OutputRef::EffectData {
            effect,
            port: "points".into(),
        };

        // A board with a Scatter on it, baked, and a web drawn from the bake.
        // Neither of the first two draws, so the web is the whole picture.
        let mut scatter = lumit_core::fx::instantiate("scatter").unwrap();
        set(&mut scatter, "density", number(3000.0));
        set(&mut scatter, "mix", number(0.0));
        // A Pick points between them marks half the points under a name, so
        // the stream that is baked carries a named column.
        let mut pick = lumit_core::fx::instantiate("pick_points").unwrap();
        set(&mut pick, "result", EffectValue::Choice(1));
        set(&mut pick, "group_name", EffectValue::Text("kept".into()));
        set(&mut pick, "to", number(50.0));
        set(&mut pick, "mix", number(0.0));
        let mut bake = lumit_core::fx::instantiate(bake_points::MATCH_NAME).unwrap();
        set(&mut bake, "mix", number(0.0));
        let mut connect = lumit_core::fx::instantiate("connect_points").unwrap();
        set(&mut connect, "max_distance", number(8.0));
        let (scatter_id, bake_id, connect_id) = (scatter.id, bake.id, connect.id);
        let pick_id = pick.id;

        let (cw, ch) = (64u32, 36u32);
        let def = Uuid::now_v7();
        let mut board = layer("board", LayerKind::Solid { def }, secs(4, 24));
        board.effects = vec![scatter, pick, bake, connect];
        board.graph.edges = vec![
            wire(stream_of(scatter_id), NodeRef::Effect(pick_id), "points"),
            wire(stream_of(pick_id), NodeRef::Effect(bake_id), "points"),
            wire(stream_of(bake_id), NodeRef::Effect(connect_id), "points"),
        ];
        let board_id = board.id;
        let comp_id = Uuid::now_v7();
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: "Board".into(),
            colour: LinearColour([0.1, 0.2, 0.9, 1.0]),
            width: cw,
            height: ch,
            extra: serde_json::Map::new(),
        }));
        doc.items.push(ProjectItem::Composition(Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: comp_id,
            name: "bake".into(),
            width: cw,
            height: ch,
            frame_rate: FrameRate::new(24, 1).unwrap(),
            duration: Duration(Rational::new(4, 24).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![board],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }));

        // The same document with the Bake points on another Mode.
        let with_mode = |mode: u32| {
            let mut doc = doc.clone();
            let board = &mut doc.comp_mut(comp_id).unwrap().layers[0];
            set(&mut board.effects[2], "mode", EffectValue::Choice(mode));
            Arc::new(doc)
        };
        let (auto, bypass) = (with_mode(0), with_mode(bake_points::MODE_BYPASS));
        let mut frame_of =
            |doc: &Arc<Document>| renderer.render_rgba(doc, comp_id, 2, 1.0).unwrap().0;
        let live = frame_of(&bypass);
        assert_eq!(frame_of(&auto), live, "nothing is baked, so Auto is live");

        // And with two Points sample drivers: one counts the bake's stream
        // and one Scatter's own, each onto a Slider control for the reading.
        let counts = |doc: &Arc<Document>| {
            let mut doc = Document::clone(doc);
            let board = &mut doc.comp_mut(comp_id).unwrap().layers[0];
            let mut read = Vec::new();
            for source in [bake_id, scatter_id] {
                let sample = lumit_core::fx::instantiate("points_sample").unwrap();
                let slider = lumit_core::fx::instantiate("slider_control").unwrap();
                board.graph.edges.extend([
                    wire(stream_of(source), NodeRef::Driver(sample.id), POINTS_PORT),
                    wire(
                        OutputRef::Driver {
                            node: sample.id,
                            port: COUNT_PORT.into(),
                        },
                        NodeRef::Effect(slider.id),
                        "slider",
                    ),
                ]);
                read.push(NodeRef::Effect(slider.id));
                board.graph.nodes.push(sample);
                board.effects.push(slider);
            }
            let doc = Arc::new(doc);
            let board = &doc.comp(comp_id).unwrap().layers[0];
            let driven = lumit_core::fx::resolve_drivers(
                &board.graph,
                2.0 / 24.0,
                Arc::new(lumit_core::expression::ExpressionContext {
                    document: doc.clone(),
                    comp: Some(comp_id),
                    layer: Some(board_id),
                    comp_time: 2.0 / 24.0,
                    current_depth: 0,
                    inputs: None,
                }),
                None,
            );
            read.iter()
                .map(
                    |node| match driven.param(*node, lumit_core::fx::ParamId::new("slider")) {
                        Some(lumit_core::fx::Value::Float(v)) => v,
                        _ => 0.0,
                    },
                )
                .collect::<Vec<f32>>()
        };
        assert_eq!(counts(&auto), [0.0, 0.0], "nothing to count before a bake");

        assert_eq!(
            request_bake(Arc::clone(&auto), comp_id, board_id, bake_id),
            Requested::Started
        );
        loop {
            match progress(bake_id) {
                Some(Progress::Done) => break,
                Some(Progress::Failed(why)) => panic!("the bake failed: {why}"),
                Some(Progress::Cancelled) => panic!("the bake was stopped"),
                _ => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        let baked = bake_points::baked(bake_id).expect("the bake is in the table");
        assert_eq!(
            baked.frames.len(),
            4,
            "one stream for every frame of the span"
        );

        let read = counts(&auto);
        assert!(read[0] > 0.0, "the driver counts the baked points");
        assert_eq!(read[1], 0.0, "and still none of Scatter's own");
        assert_eq!(
            frame_of(&auto),
            live,
            "the bake draws what the live stream drew"
        );
        assert_eq!(counts(&bypass)[0], 0.0, "Bypass reads no bake");

        // And the baked stream is the live one, named column and all: the
        // stream a live walk hands the effect, against the bake read back.
        crate::fxops::watch_points(Some(bake_id));
        frame_of(&bypass);
        let handed = crate::fxops::take_watched_points().expect("the walk reached the effect");
        crate::fxops::watch_points(None);
        let kept = handed.named.iter().find(|(name, _)| name == "kept");
        let kept = &kept.expect("the live stream carries the named column").1;
        assert!(
            kept.contains(&1.0) && kept.contains(&0.0),
            "some kept, some not"
        );
        let board = &auto.comp(comp_id).unwrap().layers[0];
        let read = bake_points::reading(
            &auto,
            auto.comp(comp_id).unwrap(),
            board,
            &board.effects[2],
            2.0 / 24.0,
        )
        .stream(handed.projection)
        .expect("Auto reads the bake");
        assert!(read == handed, "the bake is the stream that was live");

        // The file holds the same: forgotten and read back the way opening
        // the project reads it, the stream is still that one.
        let read_back = || {
            bake_points::retain(|_| false);
            for job in warm_jobs(&auto) {
                run(job, &AtomicBool::new(false));
            }
            let board = &auto.comp(comp_id).unwrap().layers[0];
            let inst = &board.effects[2];
            bake_points::reading(&auto, auto.comp(comp_id).unwrap(), board, inst, 2.0 / 24.0)
                .stream(handed.projection)
        };
        assert!(read_back() == Some(handed.clone()), "the file round-trips");

        // And a file that is not a bake reads as no bake at all.
        let junk = Answer::Bake(vec![7; 64]);
        write_sidecar(dir.path(), AnalysisKey::bake(bake_id), 24.0, 4, &junk);
        assert_eq!(read_back(), None, "Auto is live again");
        assert_eq!(progress(bake_id), None, "and nothing claims to be done");
        clear();
    }
}

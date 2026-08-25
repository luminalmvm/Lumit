//! The camera-track analysis job, its sidecar cache, and the store the render
//! path reads (K-417 stage 2, docs/impl/tracking.md §5b).
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

use lumit_core::model::{
    CameraPose, Composition, Document, EffectInstance, EffectValue, Fingerprint, Layer, LayerKind,
};
use lumit_core::track::{CameraSolveStore, LinkedPose, SolvedRange, CAMERA_TRACK};
use lumit_track::{
    detect_zoom, segment_dynamic_tracks, select_keyframes, solve_camera_cancellable, CameraSolve,
    ExclusionMask, FramePlane, GeometrySettings, Mat3, SegmentSettings, SolveError, SolveSettings,
    SolvedPose, TrackError, TrackSettings, Tracker, ZoomSettings,
};
use uuid::Uuid;

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
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        AnalysisSettings {
            density: lumit_core::fx::effects::camera_track::DENSITY_DEFAULT,
            use_masks: true,
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
        }
    }

    fn tracker(self) -> TrackSettings {
        let (across, down, per_bucket) =
            lumit_core::fx::effects::camera_track::density(self.density);
        TrackSettings {
            grid: (across, down),
            per_bucket,
            ..TrackSettings::default()
        }
    }
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
    /// cached answer, and renaming one deserves not to throw it away.
    #[must_use]
    pub fn new(
        fingerprint: &Fingerprint,
        settings: AnalysisSettings,
        masks: &[ExclusionMask],
    ) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"lumit-track/");
        h.update(&FORMAT_VERSION.to_le_bytes());
        h.update(&fingerprint.size.to_le_bytes());
        h.update(fingerprint.head_tail_hash.as_bytes());
        h.update(&settings.density.to_le_bytes());
        h.update(&[u8::from(settings.use_masks)]);
        for mask in masks {
            let (points, inverted) = mask.outline();
            h.update(b"mask/");
            h.update(&[u8::from(inverted)]);
            for point in points {
                h.update(&point[0].to_le_bytes());
                h.update(&point[1].to_le_bytes());
            }
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
/// A trait because the analysis must be drivable without a media file: the
/// engine tests feed it a rendered scene with a camera path they wrote down, and
/// asking them to encode a video first would be measuring ffmpeg. [`MediaLuma`]
/// is the real one; it is opened on the analysis thread, never on the caller's.
pub trait LumaFrames {
    /// `(frames, width, height, frames per second)`.
    fn info(&self) -> (usize, u32, u32, f64);
    /// Frame `n` as row-major 0..1 luma, `width · height` long. `None` ends the
    /// run early — a clip that stops decoding part-way is tracked as far as it
    /// went, which is more useful than nothing.
    fn luma(&mut self, n: usize) -> Option<Vec<f32>>;
}

/// One analysis, as handed to the worker.
pub struct Job {
    /// The footage item the solve is filed under.
    pub media: Uuid,
    /// What the sidecar calls it.
    pub key: AnalysisKey,
    pub settings: AnalysisSettings,
    /// Regions no track may be born in or wander into, already in source raster
    /// pixels.
    pub masks: Vec<ExclusionMask>,
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
/// rather than a fault (K-415's rule, inherited): the pictures did not carry the
/// answer, or the file could not be read.
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
/// `centre + zoom · a.xy / (a.z + zoom)`, where `a = Rot⁻¹(P − position)` and
/// `Rot = Ry·Rx·Rz` is built from the pose's Euler angles. Setting those equal
/// for every `P` leaves no freedom at all:
///
/// - `zoom = f`, because the two scale factors must agree;
/// - `Rot⁻¹ = R`, so the Euler angles are those of `Rᵀ`;
/// - `a.z = p.z − f`, so `position = C + Rᵀ·(0, 0, f)` — the camera centre
///   pushed forward along its own optical axis by the focal length, which is the
///   film-plane centre. That is precisely what Lumit's `position` names, since
///   its perspective matrix has already put the eye `zoom` behind it.
///
/// The rotation stays a rotation through the conversion: transposing an
/// orthonormal matrix inverts it, so nothing is fitted or normalised here.
fn to_camera_pose(p: &SolvedPose) -> CameraPose {
    let f = p.focal_px;
    let r = p.rotation;
    // Rᵀ·(0, 0, f) is `f` times R's third **row**, transposition being what
    // turns a row into a column.
    let axis = [f * r[2][0], f * r[2][1], f * r[2][2]];
    let (rx, ry, rz) = euler_of_transpose(&r);
    CameraPose {
        zoom: f,
        position: (
            p.position[0] + axis[0],
            p.position[1] + axis[1],
            p.position[2] + axis[2],
        ),
        rotation_deg: (rx.to_degrees(), ry.to_degrees(), rz.to_degrees()),
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
const FORMAT_VERSION: u16 = 2;

/// `LUMTRK\0` — read before anything is deserialised, so a file that is not one
/// of ours is refused rather than fed to a decoder.
const MAGIC: &[u8; 7] = b"LUMTRK\0";

/// What one sidecar file holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    /// Repeated inside the file as well as in its name, so a collision or a
    /// renamed file is caught rather than believed.
    key: [u8; 32],
    fps: f64,
    /// The clip's own length, so a cache hit knows a partial solve is partial.
    clip_frames: u64,
    solve: CameraSolve,
}

/// Serialise a record: magic, version, then the body.
///
/// The version sits **outside** the body deliberately: a reader has to be able
/// to say "this was written by a newer Lumit" without first parsing a shape it
/// does not know, which is the same refuse-newer rule `manifest.json` follows
/// (docs/10 §1).
fn encode(key: AnalysisKey, fps: f64, clip_frames: usize, solve: &CameraSolve) -> Option<Vec<u8>> {
    let body = bincode::serialize(&Record {
        key: key.0,
        fps,
        clip_frames: clip_frames as u64,
        solve: solve.clone(),
    })
    .ok()?;
    let mut out = Vec::with_capacity(body.len() + 9);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

/// The inverse, refusing anything it cannot vouch for: wrong magic, a version
/// from the future, a body that will not parse, or a key that is not the one
/// asked for. Every refusal costs a re-analysis and nothing else.
fn decode(bytes: &[u8], key: AnalysisKey) -> Option<(f64, usize, CameraSolve)> {
    let (head, body) = bytes.split_at_checked(9)?;
    if head.get(..7)? != MAGIC {
        return None;
    }
    let version = u16::from_le_bytes([*head.get(7)?, *head.get(8)?]);
    if version > FORMAT_VERSION {
        return None;
    }
    let record: Record = bincode::deserialize(body).ok()?;
    let clip_frames = usize::try_from(record.clip_frames).unwrap_or(usize::MAX);
    (record.key == key.0).then_some((record.fps, clip_frames, record.solve))
}

/// Read a solve out of the sidecar, or `None` for every way that can fail to
/// happen — no folder, no file, an unreadable one, one written by a newer build.
fn read_sidecar(dir: &Path, key: AnalysisKey) -> Option<(f64, usize, CameraSolve)> {
    let bytes = std::fs::read(dir.join(key.file_name())).ok()?;
    decode(&bytes, key)
}

/// Write one, best-effort. A cache that cannot be written costs the next session
/// a re-analysis; it is never worth failing an answer that is already in hand.
fn write_sidecar(dir: &Path, key: AnalysisKey, fps: f64, clip_frames: usize, solve: &CameraSolve) {
    let Some(bytes) = encode(key, fps, clip_frames, solve) else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join(key.file_name()), bytes);
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

/// Where the named tracks sit in the world, averaged — the position K-417's
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
/// followed (K-417). The reading the render path takes, replacing
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
    if let Ok(mut held) = jobs().lock() {
        held.progress.clear();
    }
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
    let media = job.media;
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
        .spawn(move || run(job, &cancel));
    if spawned.is_err() {
        // The slot was claimed above and nothing will ever release it, so it is
        // released here — otherwise one failed spawn would answer `Busy` for the
        // rest of the session.
        finish(media, None);
        return Requested::Refused;
    }
    Requested::Started
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
    let dir = cache_dir();

    if let Some((fps, clip_frames, solve)) = dir.as_deref().and_then(|d| read_sidecar(d, key)) {
        publish(media, fps, clip_frames, solve);
        finish(media, Some(Progress::Done));
        return;
    }
    if !job.analyse {
        // A warm pass found nothing. That is not a failure and must not look
        // like one: nobody asked for this clip to be tracked.
        finish(media, None);
        return;
    }

    match analyse(job, cancel, &|step| report(media, step)) {
        Ok((fps, clip_frames, solve)) => {
            // Written before the store is filled, so a solve the interface can
            // see is a solve the next session will find (and never the other
            // way round). Cancellation writes nothing at all, which is what
            // makes a stopped run leave no trace. A **partial** solve is
            // cached like any other: it is the honest answer for that file at
            // those settings, and re-deriving it would only take the same
            // minutes to stop in the same place.
            if let Some(dir) = dir.as_deref() {
                write_sidecar(dir, key, fps, clip_frames, &solve);
            }
            publish(media, fps, clip_frames, solve);
            finish(media, Some(Progress::Done));
        }
        Err(AnalysisError::Cancelled) => finish(media, Some(Progress::Cancelled)),
        Err(e) => finish(media, Some(Progress::Failed(e))),
    }
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
) -> Result<(f64, usize, CameraSolve), AnalysisError> {
    let (fps, clip_frames, mut set) = track_frames(job, cancel, report)?;

    report(Progress::Solving);
    let stop = || cancel.load(Ordering::Relaxed);
    let pairs = select_keyframes(&set, &GeometrySettings::default());
    segment_dynamic_tracks(&mut set, &pairs, &SegmentSettings::default());
    let zooms = detect_zoom(&set, &ZoomSettings::default());
    let solve = solve_camera_cancellable(&set, &pairs, &zooms, &SolveSettings::default(), &stop)
        .map_err(|e| match e {
            SolveError::Cancelled => AnalysisError::Cancelled,
            other => AnalysisError::Solve(other),
        })?;
    Ok((fps, clip_frames, solve))
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
/// tracks)`). Two things end a run early and they are reported the same way,
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
) -> Result<(f64, usize, lumit_track::TrackSet), AnalysisError> {
    let mut frames = (job.open)().ok_or(AnalysisError::Unreadable)?;
    let (total, width, height, fps) = frames.info();
    if total == 0 || width == 0 || height == 0 || fps <= 0.0 || !fps.is_finite() {
        return Err(AnalysisError::NoFrames);
    }
    let (w, h) = (width as usize, height as usize);

    let mut tracker = Tracker::new(job.settings.tracker()).with_masks(job.masks);
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
    Ok((fps, total, set))
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

/// The exclusion masks a tracked layer contributes.
///
/// **The factor is one**, and that is a statement rather than an omission: a
/// mask's vertices are in the layer's own pixel coordinates, and for a footage
/// layer those are the source raster — `build.rs` rasterises them at the layer's
/// *natural* size, which is the decoded file's own size regardless of the
/// preview tier. The tracker works in the same pixels (K-248), so nothing has to
/// be converted.
///
/// **Flattened at layer time zero.** A tracker takes one fixed set of regions
/// for a whole run, so a mask keyframed to follow a moving object cannot be
/// honoured; the shape it starts on is used. Owed, and listed in docs/TODO.md.
#[must_use]
pub fn exclusion_masks(layer: &Layer, settings: AnalysisSettings) -> Vec<ExclusionMask> {
    if !settings.use_masks {
        return Vec::new();
    }
    layer
        .masks
        .iter()
        .map(|m| ExclusionMask::from_mask(m, 0.0, 1.0))
        .collect()
}

/// Build the job for the tracked layer `layer` of `comp`, ready to hand to
/// [`request`]. `path` is where the frontend resolved the media to (K-173 keeps
/// absolute paths out of the document, so only it can say).
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
    let masks = exclusion_masks(layer, settings);
    Some(Job {
        media: item,
        key: AnalysisKey::new(fingerprint, settings, &masks),
        settings,
        masks,
        open: Box::new(move || MediaLuma::open(&path).map(|s| Box::new(s) as Box<dyn LumaFrames>)),
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

        /// The same shot, `FRAMES` of it followable and `tail` frames of
        /// nothing after that.
        ///
        /// **Featureless rather than merely poor**, and that is the point: the
        /// verification the tracker ends a track on is normalised correlation,
        /// which is blind to gain and lift by construction, so a picture that
        /// merely fades down in contrast is followed happily and *should* be. A
        /// frame with no structure at all is what actually severs the chain —
        /// the gradient normal matrix is singular, every KLT solve refuses, and
        /// nothing carries across. It is also a real thing footage does.
        fn degrading(tail: usize) -> Self {
            Shot {
                frames: FRAMES + tail,
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

    fn job(media: Uuid, tag: &str, masks: Vec<ExclusionMask>) -> Job {
        let settings = AnalysisSettings::default();
        Job {
            media,
            key: AnalysisKey::new(&fingerprint(tag), settings, &masks),
            settings,
            masks,
            open: Box::new(|| Some(Box::new(Shot::new()) as Box<dyn LumaFrames>)),
            analyse: true,
        }
    }

    /// [`job`], over a shot that stops carrying a picture after `FRAMES`.
    fn degrading_job(media: Uuid, tag: &str, tail: usize) -> Job {
        Job {
            open: Box::new(move || Some(Box::new(Shot::degrading(tail)) as Box<dyn LumaFrames>)),
            ..job(media, tag, Vec::new())
        }
    }

    /// Run one analysis here and now, keeping every progress reading it
    /// published — the deterministic half, so nothing has to race a thread to
    /// see what it did.
    /// What one analysis answers: the media's rate, the clip's length, and the
    /// solve. Named only so the pair it is half of is readable.
    type Analysed = Result<(f64, usize, CameraSolve), AnalysisError>;

    fn run_here(job: Job, cancel: &AtomicBool) -> (Analysed, Vec<Progress>) {
        let log = Mutex::new(Vec::new());
        let out = analyse(job, cancel, &|step| {
            if let Ok(mut held) = log.lock() {
                held.push(step);
            }
        });
        (out, log.into_inner().unwrap())
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
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// A footage layer wearing the Camera track effect, and a Camera layer
    /// linked to it — the shape K-417 describes, built for real rather than
    /// mocked.
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
        let camera = layer(
            "Camera 1",
            LayerKind::Camera {
                zoom: Property::fixed(999.0),
                solve_link: Some(footage.id),
            },
            secs(20, 1),
        );
        let comp = Composition {
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
            }));
        doc.items.push(ProjectItem::Composition(comp.clone()));
        (doc, comp)
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
        let (out, steps) = run_here(job(media, "solve", Vec::new()), &cancel);
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

    /// A shot that stops being followable is solved as far as it went, and the
    /// job stops there (K-540).
    ///
    /// Three claims, and they are one claim: the analysis does not decode the
    /// frames it cannot use, the solve covers exactly the span that carried,
    /// and the result says so — so a camera linked to it derives inside that
    /// span and holds outside it, which is K-417's rule meeting a range that
    /// now ends early.
    #[test]
    fn a_shot_that_stops_carrying_is_solved_as_far_as_it_went() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        const TAIL: usize = 6;

        let cancel = AtomicBool::new(false);
        let (out, steps) = run_here(degrading_job(media, "partial", TAIL), &cancel);
        let (fps, clip_frames, solve) = out.expect("the followable part of the shot solves");

        // It stopped: the analysis never reached the frames it could not use,
        // and the last thing it said about the tracking says which ones it did.
        assert_eq!(clip_frames, FRAMES + TAIL, "the clip's own length");
        let tracked: Vec<&Progress> = steps
            .iter()
            .filter(|s| matches!(s, Progress::Tracking { .. }))
            .collect();
        assert_eq!(
            tracked.last(),
            Some(&&Progress::Tracking {
                done: FRAMES,
                total: FRAMES + TAIL
            }),
            "the run did not stop where the shot stopped carrying"
        );
        assert!(
            !steps.contains(&Progress::Tracking {
                done: FRAMES + TAIL - 1,
                total: FRAMES + TAIL
            }),
            "the job carried on decoding frames nothing could be followed through"
        );

        // And it finalised rather than discarded: a pose for every frame of the
        // span that worked, and none for any frame after it.
        assert_eq!(
            solve.poses.len(),
            FRAMES,
            "the solve does not cover the span that carried"
        );
        assert_eq!(solve.poses.first().map(|p| p.frame), Some(0));
        assert_eq!(
            solve.poses.last().map(|p| p.frame),
            Some(FRAMES as i64 - 1),
            "a frame past the failure was given a camera"
        );
        let focal = solve.segments.first().unwrap().focal_px;
        assert!(
            (focal - FOCAL).abs() / FOCAL < 0.08,
            "the partial solve is still a solve: focal {focal} against a true {FOCAL}"
        );

        // The store says it is partial, and the range it hands the model is the
        // span rather than the clip.
        publish(media, fps, clip_frames, solve);
        let solved = solved(media).expect("published");
        assert!(solved.is_partial(), "a solve short of its clip is partial");
        assert_eq!(solved.first_frame, 0);
        assert_eq!(solved.last_frame, FRAMES as i64 - 1);

        // The link derives inside the span and holds outside it — the same
        // clamp K-417 already required, now against a range that ends early.
        let (doc, comp) = linked_document(media);
        let last = linked_pose(&doc, &comp, (FRAMES - 1) as f64 / FPS).expect("a camera");
        assert_eq!(last.state, lumit_core::track::LinkState::Derived);
        for n in FRAMES..FRAMES + TAIL {
            let held = linked_pose(&doc, &comp, n as f64 / FPS).expect("a camera");
            assert_eq!(
                held.state,
                lumit_core::track::LinkState::Held,
                "frame {n} is past the solve and should be holding"
            );
            assert_eq!(
                Some(held.pose),
                solved.pose(solved.last_frame),
                "the hold is the last derived motion, not some other frame"
            );
        }
        clear();
    }

    /// A solve landing renames the frames drawn with it. Without this the frames
    /// banked under the camera's *stored* transform would be served back after
    /// the link started deriving a different one, and the picture would silently
    /// disagree with the camera.
    #[test]
    fn a_solve_landing_renames_the_frames_it_changes() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        let (doc, comp) = linked_document(media);
        let doc = Arc::new(doc);

        let probed = probes(media);
        let key = || {
            let stamper =
                crate::cache::Stamper::new(&doc, &probed, crate::plan::Quality::default());
            lumit_eval::comp_frame_key(&doc, &comp, 0.0, lumit_eval::Quality::default(), &stamper)
        };
        let before = key().expect("a probed comp is keyable");

        let cancel = AtomicBool::new(false);
        let (out, _) = run_here(job(media, "key", Vec::new()), &cancel);
        let (fps, clip_frames, solve) = out.expect("the synthetic shot solves");
        publish(media, fps, clip_frames, solve);

        assert_ne!(
            before,
            key().expect("still keyable"),
            "the derived camera is not in the frame's name"
        );
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
        let key = AnalysisKey::new(&fingerprint("cancel"), AnalysisSettings::default(), &[]);

        // Raised after a handful of frames, which is where a real Cancel lands.
        let cancel = AtomicBool::new(false);
        let mut first = job(media, "cancel", Vec::new());
        first.key = key;
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
        let mut again = job(media, "cancel", Vec::new());
        again.key = key;
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
        let key = AnalysisKey::new(&fingerprint("cache"), AnalysisSettings::default(), &[]);
        let cancel = AtomicBool::new(false);

        let mut first = job(media, "cache", Vec::new());
        first.key = key;
        let (fps, clip_frames, solve) = run_here(first, &cancel).0.unwrap();
        write_sidecar(dir.path(), key, fps, clip_frames, &solve);
        let written = std::fs::read(dir.path().join(key.file_name())).unwrap();

        // Read back: the same solve, and the same file when written again.
        let (read_fps, read_clip, read_solve) =
            read_sidecar(dir.path(), key).expect("the sidecar is there");
        assert_eq!(read_fps, fps);
        assert_eq!(
            read_clip, clip_frames,
            "the clip's own length did not survive the round trip, so a cached              partial solve would read back as a whole one"
        );
        assert_eq!(read_solve, solve, "the round trip changed the solve");
        assert_eq!(
            encode(key, read_fps, read_clip, &read_solve).unwrap(),
            written,
            "re-encoding what was read gives different bytes"
        );

        // A second analysis of the same input is the same solve — which is what
        // makes the cache safe to trust at all.
        let mut again = job(media, "cache", Vec::new());
        again.key = key;
        let (fps_again, clip_again, solve_again) = run_here(again, &cancel).0.unwrap();
        assert_eq!(fps_again, fps);
        assert_eq!(
            encode(key, fps_again, clip_again, &solve_again).unwrap(),
            written,
            "a rebuild and a cache hit are not the same bits"
        );

        // A different key does not read this file, however tempting.
        let other = AnalysisKey::new(
            &fingerprint("cache"),
            AnalysisSettings {
                density: 0,
                use_masks: true,
            },
            &[],
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
        let mut rebuilt = job(media, "cache", Vec::new());
        rebuilt.key = key;
        let (fps_rebuilt, clip_rebuilt, solve_rebuilt) = run_here(rebuilt, &cancel).0.unwrap();
        assert_eq!(
            encode(key, fps_rebuilt, clip_rebuilt, &solve_rebuilt).unwrap(),
            written,
            "a rebuild after a delete is not what was deleted"
        );
        clear();
    }

    /// A mask on the tracked layer is a region the analysis does not enter
    /// (K-408's geometry, read by the tracker): nothing solved comes back from
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
        let masks = exclusion_masks(&tracked, AnalysisSettings::default());
        assert_eq!(masks.len(), 1);

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
        let unmasked = track_frames(job(media, "unmasked", Vec::new()), &cancel, &|_| {})
            .unwrap()
            .2;
        let masked = track_frames(job(media, "masked", masks.clone()), &cancel, &|_| {})
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

    /// The thread path: `request` accepts one analysis, refuses a second while
    /// it runs, and the solve arrives in the store and the sidecar without any
    /// caller waiting on the disk.
    #[test]
    fn the_worker_thread_runs_one_analysis_and_files_it() {
        let _serial = serially();
        let dir = tempfile::tempdir().unwrap();
        with_cache(dir.path());
        let media = Uuid::now_v7();
        let key = AnalysisKey::new(&fingerprint("thread"), AnalysisSettings::default(), &[]);
        let mut first = job(media, "thread", Vec::new());
        first.key = key;

        assert_eq!(request(first), Requested::Started);
        // One at a time: a second request while the first is in flight is
        // refused rather than queued behind it.
        assert_eq!(
            request(job(Uuid::now_v7(), "second", Vec::new())),
            Requested::Busy
        );

        let mut waited = 0;
        while !matches!(progress(media), Some(Progress::Done)) {
            assert!(
                waited < 1200,
                "the analysis never finished: {:?}",
                progress(media)
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 1;
        }
        assert!(solved(media).is_some(), "the solve is not in the store");
        assert!(read_sidecar(dir.path(), key).is_some(), "nothing was filed");

        // And a warm pass — the one a project open makes — finds it without
        // decoding anything, which is what its refusal to open the media proves.
        clear();
        let warm = Job {
            media,
            key,
            settings: AnalysisSettings::default(),
            masks: Vec::new(),
            open: Box::new(|| panic!("a warm pass must never open the media")),
            analyse: false,
        };
        assert_eq!(request(warm), Requested::Started);
        let mut waited = 0;
        while !matches!(progress(media), Some(Progress::Done)) {
            assert!(waited < 500, "the warm pass never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
        }
        assert!(
            solved(media).is_some(),
            "the warm pass did not fill the store"
        );

        // A warm pass for a clip nobody has analysed reports *nothing* rather
        // than sitting at `Queued` for ever, which is the difference between
        // "not analysed" and "about to be".
        let cold = Uuid::now_v7();
        let miss = Job {
            media: cold,
            key: AnalysisKey::new(&fingerprint("cold"), AnalysisSettings::default(), &[]),
            settings: AnalysisSettings::default(),
            masks: Vec::new(),
            open: Box::new(|| panic!("a warm pass must never open the media")),
            analyse: false,
        };
        assert_eq!(request(miss), Requested::Started);
        let mut waited = 0;
        while progress(cold).is_some() {
            assert!(
                waited < 500,
                "the warm miss never cleared: {:?}",
                progress(cold)
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
        }
        assert!(solved(cold).is_none());
        clear();
    }
}

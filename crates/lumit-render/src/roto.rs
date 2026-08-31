//! The roto propagation job, the `roto/` sidecar, and the store the render path
//! reads (docs/impl/roto.md §5–§8, K-710 stage 2).
//!
//! # In plain terms
//!
//! The user scribbles on one frame and the engine cuts the subject out of it.
//! **Propagate** is what turns that one cut-out into a cut-out on every frame of
//! the shot, and this file is the whole of that: the work, where it happens,
//! where its answers are kept, and what happens when the user changes their mind
//! half way through.
//!
//! **It happens somewhere else.** One propagation at a time, on a thread spawned
//! for it and named `lumit-roto` — never a pool worker, for the reason decoding
//! never is (docs/05 §2): it holds a decoder open, it stalls on seeks, and it
//! runs for a minute. You keep editing while it runs. A second request while one
//! is in flight is answered `Busy` rather than queued: two of them share one
//! disk and one graphics card and halve each other.
//!
//! **It can be stopped, and stopping keeps what it had.** This is the one place
//! this job differs from the camera tracker's, and it is deliberate (the K-540
//! pattern). A cancelled *track* throws its half-solve away, because half a
//! camera path adjusted toward an answer it never reached is not an answer. A
//! cancelled *propagation* has fifty finished mattes, each correct, each
//! correctly named — so they are written, the span says how far it got, and a
//! later Propagate carries on from them instead of starting again.
//!
//! **It is not done twice.** A matte depends on the file's bytes, the settings,
//! the base frame and the strokes between the base and that frame
//! ([`lumit_core::roto`]), and on nothing else. Each frame is filed beside the
//! fingerprint of exactly those strokes — its **chain hash** — so a
//! re-propagation after a correction *copies* every frame whose fingerprint
//! still matches and re-solves only the ones that changed. Correcting frame 200
//! of 300 re-solves a hundred frames, not three hundred, and the tests assert
//! that by counting solves rather than by timing them.
//!
//! **It refuses rather than pretends.** No fingerprint, no cache to key
//! (`Offline`); no GPU flow on this device, and the CPU oracle at seconds a pair
//! would misrepresent a minutes-long job as hung (`FlowUnavailable`); one at a
//! time (`Busy`); Propagate before any stroke (`NoBaseFrame`). Every one is a
//! refusal and none is a fault.
//!
//! # What the store hands back
//!
//! A [`RotoRun`] per Roto brush **instance** — not per media, unlike a camera
//! solve: two brushes on one clip are two subjects cut two ways, and the strokes
//! that made them are the effect's own. The render path asks it for one frame's
//! matte, gets a full-raster gray8 plane back through a small
//! decompressed-frame cache, and holds no lock while it uses it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use lumit_core::model::{Document, EffectInstance, Fingerprint, LayerKind};
use lumit_core::roto::{chain_hash, RotoBlock, RotoSettings, RotoStrokeKind};
use lumit_roto::{base_seeds, FlowField, FrameRgb, RotoSolver, RotoStroke, Seeds, StrokeKind};
use uuid::Uuid;

use crate::sidecar;

// ---------------------------------------------------------------------------
// What a propagation is asked for
// ---------------------------------------------------------------------------

/// One frame of a clip as **encoded RGBA8** at the source's own raster —
/// everything a propagation reads.
///
/// A trait for [`LumaFrames`](crate::track::LumaFrames)'s reason: the engine
/// tests feed a synthetic shot with a matte they wrote down, since asking them
/// to encode a video first would be measuring ffmpeg. Whichever it is, it is
/// opened on the propagation thread and never on the caller's.
pub trait RotoFrames {
    /// `(frames, width, height, frames per second)`.
    fn info(&self) -> (usize, u32, u32, f64);
    /// Frame `n` as row-major RGBA8, `width · height · 4` long. `None` ends the
    /// run early — a clip that stops decoding part-way is propagated as far as
    /// it went, which is the same honesty a partial track has.
    fn rgba(&mut self, n: usize) -> Option<Vec<u8>>;
}

/// What names one propagation's file: the media's own content, and everything
/// [`lumit_core::roto::key_hash`] covers (the tier version, the settings, the
/// base frame and the whole stroke table).
///
/// Two halves rather than one hash, because the note asks for a name whose
/// **media prefix** can be enumerated: a re-propagation after a correction has a
/// different key and must still be able to find the previous run's file to copy
/// frames out of. Sixteen bytes each — 2^128 either side is not a collision
/// anybody will meet, and a 129-character file name is a nuisance on every
/// platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RotoKey {
    media: [u8; 16],
    run: [u8; 16],
}

impl RotoKey {
    /// The key for `fingerprint` propagated from `block` under `settings`.
    #[must_use]
    pub fn new(fingerprint: &Fingerprint, block: &RotoBlock, settings: RotoSettings) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"lumit-roto/media/");
        h.update(&fingerprint.size.to_le_bytes());
        h.update(fingerprint.head_tail_hash.as_bytes());
        let mut media = [0u8; 16];
        media.copy_from_slice(&h.finalize().as_bytes()[..16]);
        let mut run = [0u8; 16];
        run.copy_from_slice(&lumit_core::roto::key_hash(block, settings)[..16]);
        RotoKey { media, run }
    }

    fn prefix(&self) -> String {
        self.media.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn file_name(&self) -> String {
        let run: String = self.run.iter().map(|b| format!("{b:02x}")).collect();
        format!("{}-{run}.lrot", self.prefix())
    }
}

/// One propagation, as handed to the worker.
pub struct RotoJob {
    /// The **Roto brush instance** the answer is filed under, and what
    /// [`progress`] is read by.
    pub instance: Uuid,
    /// What the sidecar calls it, or `None` for a source with no fingerprint —
    /// which is [`RotoFailure::Offline`], refused before a thread is spawned.
    pub key: Option<RotoKey>,
    pub settings: RotoSettings,
    /// The strokes and the base frame, copied off the document at the moment
    /// the button was pressed. A run is of a stroke table, not of a live
    /// document that may move under it.
    pub block: RotoBlock,
    /// Opens the frames, **on the worker thread**.
    pub open: Box<dyn FnOnce() -> Option<Box<dyn RotoFrames>> + Send>,
    /// `false` asks only for a cache hit: the warm pass a project open makes,
    /// which must never start propagating a shot nobody asked about.
    pub propagate: bool,
}

/// How far a propagation has got. Read, never subscribed to — the interface
/// samples it as it repaints, exactly as it samples the cache bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Accepted, not started.
    Queued,
    /// Solving: `done` of `total` frames, `reused` of them copied from a
    /// previous run rather than solved. The second number is what makes prefix
    /// reuse visible to the person waiting.
    Solving {
        done: usize,
        total: usize,
        reused: usize,
    },
    /// There is a run in the store for this instance.
    Done,
    /// Stopped between frames. **The finished prefix was kept** — see the
    /// module note.
    Cancelled,
    Failed(RotoFailure),
}

/// Why a propagation did not produce mattes. Every variant is a refusal rather
/// than a fault, and every one is a **closed** enum with no free text in it, so
/// the bridge can hand the interface a reason rather than an English sentence
/// (K-303).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RotoFailure {
    /// No resolved media fingerprint — nothing to key a cache with.
    #[error("the media is offline")]
    Offline,
    /// No GPU flow on this device. The CPU oracle at seconds a pair would
    /// misrepresent a minutes-long job as hung, and mixing backends would break
    /// the byte-identical rebuild claim, so the honest answer is the refusal.
    #[error("optical flow is not available on this device")]
    FlowUnavailable,
    /// One propagation at a time.
    #[error("another propagation is running")]
    Busy,
    /// Propagate pressed before any stroke.
    #[error("there is no base frame to propagate from")]
    NoBaseFrame,
    /// The media could not be opened, or carries no video.
    #[error("the media could not be read")]
    Unreadable,
    /// Opened, but with no frames or no raster.
    #[error("the media has no frames to propagate through")]
    NoFrames,
    /// The base frame's own solve had nothing to work from — every stroke fell
    /// outside the picture, or claimed one side only with no border to answer
    /// it.
    #[error("the base frame's strokes do not describe a subject")]
    NoSeeds,
}

/// What happened when a propagation was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    /// Accepted; watch [`progress`].
    Started,
    /// Refused, with the reason. [`RotoFailure::Busy`] is the ordinary one.
    Refused(RotoFailure),
}

// ---------------------------------------------------------------------------
// What comes back
// ---------------------------------------------------------------------------

/// One frame's matte as the sidecar keeps it: the strokes it depends on, boiled
/// down, the box it lives in, and the pixels inside that box.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct FrameRecord {
    frame: i64,
    /// [`lumit_core::roto::chain_hash`] for this frame — what decides, on its
    /// own, whether a later run may copy this matte instead of solving it.
    chain: [u8; 32],
    /// `(x, y, width, height)` of the matte's non-empty box. A matte that is
    /// empty everywhere has a zero-sized box and no pixels at all.
    bbox: [u32; 4],
    /// gray8 inside the box, LZ4. Long runs of 0 and 255 are what a matte is
    /// mostly made of, so this is tens of kilobytes where the raw plane is two
    /// megabytes.
    lz4: Vec<u8>,
}

/// One Roto brush instance's propagation, as the render path wants it.
#[derive(Debug)]
pub struct RotoRun {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// How many frames the **clip** has, against which the propagated span is a
    /// whole answer or a partial one.
    pub clip_frames: usize,
    /// The span actually propagated, inclusive. Outside it the effect is a
    /// passthrough — never a held neighbouring matte.
    pub first_frame: i64,
    pub last_frame: i64,
    /// Ascending by frame, so a lookup is a binary search.
    records: Vec<FrameRecord>,
    /// The last few frames decompressed, so scrubbing one region does not
    /// re-inflate a plane per repaint.
    ///
    /// ponytail: a four-entry ring walked linearly, which is an LRU at this
    /// size; a real one when somebody wants more than a handful of frames warm
    /// at once. The lock is held for the length of a `Vec` scan and never
    /// across a decompression that misses.
    warm: Mutex<Vec<(i64, Arc<Vec<u8>>)>>,
}

/// How many decompressed planes stay warm. Four covers a scrub back and forth
/// over a boundary; each is two megabytes at 1080p.
const WARM_FRAMES: usize = 4;

impl RotoRun {
    fn index(&self, frame: i64) -> Option<usize> {
        self.records.binary_search_by_key(&frame, |r| r.frame).ok()
    }

    /// Whether the clip runs on past what was propagated — cancelled part-way,
    /// or the frames stopped decoding.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        let clip = i64::try_from(self.clip_frames).unwrap_or(i64::MAX);
        self.first_frame > 0 || self.last_frame + 1 < clip
    }

    /// The chain hash frame `frame`'s matte was filed under, for the frame key
    /// and for the reuse test.
    #[must_use]
    pub fn chain(&self, frame: i64) -> Option<[u8; 32]> {
        Some(self.records.get(self.index(frame)?)?.chain)
    }

    /// Frame `frame`'s matte as a full-raster gray8 plane, or `None` outside the
    /// propagated span.
    ///
    /// The `Arc` is cloned out from under the lock, so nothing is held while the
    /// caller uploads it (docs/14 §1.3).
    #[must_use]
    pub fn matte(&self, frame: i64) -> Option<Arc<Vec<u8>>> {
        if let Ok(warm) = self.warm.lock() {
            if let Some((_, plane)) = warm.iter().find(|(f, _)| *f == frame) {
                return Some(Arc::clone(plane));
            }
        }
        let record = self.records.get(self.index(frame)?)?;
        let plane = Arc::new(expand(record, self.width, self.height));
        if let Ok(mut warm) = self.warm.lock() {
            warm.insert(0, (frame, Arc::clone(&plane)));
            warm.truncate(WARM_FRAMES);
        }
        Some(plane)
    }
}

/// Blow one record's boxed, compressed matte back out to the full raster.
/// Anything that will not decompress reads as an empty matte rather than a
/// panic — a corrupt cache costs a re-propagation, never a frame.
fn expand(record: &FrameRecord, width: u32, height: u32) -> Vec<u8> {
    let n = (width as usize) * (height as usize);
    let mut plane = vec![0u8; n];
    let [bx, by, bw, bh] = record.bbox;
    if bw == 0 || bh == 0 {
        return plane;
    }
    let Ok(boxed) = lz4_flex::decompress_size_prepended(&record.lz4) else {
        return plane;
    };
    if boxed.len() != (bw as usize) * (bh as usize) {
        return plane;
    }
    for row in 0..bh as usize {
        let dst = ((by as usize + row) * width as usize) + bx as usize;
        let src = row * bw as usize;
        let (Some(d), Some(s)) = (
            plane.get_mut(dst..dst + bw as usize),
            boxed.get(src..src + bw as usize),
        ) else {
            continue;
        };
        d.copy_from_slice(s);
    }
    plane
}

/// The inverse: crop a full-raster gray8 plane to its non-empty box and
/// compress it.
fn shrink(plane: &[u8], width: u32, height: u32) -> ([u32; 4], Vec<u8>) {
    let (w, h) = (width as usize, height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if plane.get(y * w + x).copied().unwrap_or(0) != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return ([0, 0, 0, 0], Vec::new());
    }
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut boxed = Vec::with_capacity(bw * bh);
    for y in y0..=y1 {
        let src = y * w + x0;
        if let Some(row) = plane.get(src..src + bw) {
            boxed.extend_from_slice(row);
        }
    }
    (
        [x0 as u32, y0 as u32, bw as u32, bh as u32],
        lz4_flex::compress_prepend_size(&boxed),
    )
}

// ---------------------------------------------------------------------------
// The sidecar
// ---------------------------------------------------------------------------

/// Bump when the record's meaning changes. Old files then hash to a different
/// name and are simply never asked for — the disposal the frame cache's
/// algorithm version performs.
const FORMAT_VERSION: u16 = 1;

/// `LUMROT\0` — read before anything is deserialised, so a file that is not one
/// of ours is refused rather than fed to a decoder.
const MAGIC: &[u8; 7] = b"LUMROT\0";

/// What one `.lrot` file holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    /// Repeated inside the file as well as in its name, so a collision or a
    /// renamed file is caught rather than believed.
    key: [u8; 32],
    width: u32,
    height: u32,
    fps: f64,
    clip_frames: u64,
    frames: Vec<FrameRecord>,
}

fn key_bytes(key: RotoKey) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&key.media);
    out[16..].copy_from_slice(&key.run);
    out
}

/// Serialise a run, in the crate's shared framing ([`crate::sidecar`]).
fn encode(key: RotoKey, run: &RotoRun) -> Option<Vec<u8>> {
    let body = bincode::serialize(&Record {
        key: key_bytes(key),
        width: run.width,
        height: run.height,
        fps: run.fps,
        clip_frames: run.clip_frames as u64,
        frames: run.records.clone(),
    })
    .ok()?;
    Some(sidecar::frame(MAGIC, FORMAT_VERSION, &body))
}

/// The inverse, refusing anything it cannot vouch for: wrong magic, a version
/// from the future, a body that will not parse, or — when `key` is given — a
/// stored key that is not the one asked for. Every refusal costs a
/// re-propagation and nothing else.
fn decode(bytes: &[u8], key: Option<RotoKey>) -> Option<Record> {
    let body = sidecar::unframe(bytes, MAGIC, FORMAT_VERSION)?;
    let record: Record = bincode::deserialize(body).ok()?;
    match key {
        Some(k) if record.key != key_bytes(k) => None,
        _ => Some(record),
    }
}

fn record_to_run(record: Record) -> Option<RotoRun> {
    let first = record.frames.first()?.frame;
    let last = record.frames.last()?.frame;
    Some(RotoRun {
        width: record.width,
        height: record.height,
        fps: record.fps,
        clip_frames: usize::try_from(record.clip_frames).unwrap_or(usize::MAX),
        first_frame: first,
        last_frame: last,
        records: record.frames,
        warm: Mutex::new(Vec::new()),
    })
}

fn read_sidecar(dir: &Path, key: RotoKey) -> Option<RotoRun> {
    let bytes = std::fs::read(dir.join(key.file_name())).ok()?;
    record_to_run(decode(&bytes, Some(key))?)
}

fn write_sidecar(dir: &Path, key: RotoKey, run: &RotoRun) {
    if let Some(bytes) = encode(key, run) {
        sidecar::write(dir, &key.file_name(), &bytes);
    }
}

/// Every frame any **other** run of the same media can lend this one, by chain
/// hash. The whole of prefix reuse: a frame whose contributing strokes did not
/// change keeps its chain hash, and a matte filed under that hash is that
/// frame's answer whichever run made it.
fn lendable(dir: &Path, key: RotoKey) -> HashMap<[u8; 32], FrameRecord> {
    let mut out = HashMap::new();
    let prefix = key.prefix();
    let mine = key.file_name();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || !name.ends_with(".lrot") || name == mine {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        // No key check: the point is to read a *different* run's file. What
        // makes a frame safe to borrow is its chain hash, which already
        // covers the settings, the base and every stroke that decides it.
        let Some(record) = decode(&bytes, None) else {
            continue;
        };
        for frame in record.frames {
            out.entry(frame.chain).or_insert(frame);
        }
    }
    out
}

/// Where the sidecar lives. Overridable in tests, which must never write into
/// the user's own cache folder — the shape [`crate::track`] uses.
fn cache_dir() -> Option<PathBuf> {
    test_cache_dir().or_else(lumit_project::roto_cache_dir)
}

#[cfg(not(test))]
fn test_cache_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
static TEST_CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

// Process-wide rather than thread-local, because the propagation runs on a
// thread this module spawns and a thread-local would not follow it there.
#[cfg(test)]
fn test_cache_dir() -> Option<PathBuf> {
    TEST_CACHE_DIR.lock().ok().and_then(|dir| dir.clone())
}

/// Point the sidecar at a temporary folder for the length of a test.
#[cfg(test)]
pub(crate) fn set_test_cache_dir(dir: Option<PathBuf>) {
    if let Ok(mut held) = TEST_CACHE_DIR.lock() {
        *held = dir;
    }
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// Every propagation this session knows about, **by Roto brush instance**.
///
/// Keyed by the effect and not by the media, unlike a camera solve: two brushes
/// on one clip cut two different subjects, and the strokes that made each are
/// the effect's own.
fn runs() -> &'static RwLock<HashMap<Uuid, Arc<RotoRun>>> {
    static RUNS: OnceLock<RwLock<HashMap<Uuid, Arc<RotoRun>>>> = OnceLock::new();
    RUNS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The one propagation in flight, and what every instance's last one did.
struct Jobs {
    running: Option<(Uuid, Arc<AtomicBool>)>,
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

/// What has been propagated under the Roto brush instance `instance`. Cloned out
/// of the table so no lock is held while it is used.
#[must_use]
pub fn propagated(instance: Uuid) -> Option<Arc<RotoRun>> {
    runs().read().ok()?.get(&instance).cloned()
}

/// The span `instance`'s matte covers, inclusive — the panel's reading and the
/// render path's passthrough test.
#[must_use]
pub fn span(instance: Uuid) -> Option<(i64, i64)> {
    let run = propagated(instance)?;
    Some((run.first_frame, run.last_frame))
}

/// Frame `frame`'s matte for `instance`, or `None` outside the propagated span
/// — where the effect renders passthrough rather than holding a neighbour.
#[must_use]
pub fn matte(instance: Uuid, frame: i64) -> Option<(u32, u32, Arc<Vec<u8>>)> {
    let run = propagated(instance)?;
    let plane = run.matte(frame)?;
    Some((run.width, run.height, plane))
}

/// The chain hash naming frame `frame`'s matte as the store actually holds it.
#[must_use]
pub fn stored_chain(instance: Uuid, frame: i64) -> Option<[u8; 32]> {
    propagated(instance)?.chain(frame)
}

/// How far `instance`'s propagation has got.
#[must_use]
pub fn progress(instance: Uuid) -> Option<Progress> {
    jobs().lock().ok()?.progress.get(&instance).cloned()
}

/// Put a run in the store. Public because this is how one gets in and there is
/// exactly one way — a propagation finishing, a sidecar being read back — and
/// the bridge's own tests need one without an encoder to make it with.
pub fn publish(instance: Uuid, run: RotoRun) {
    if let Ok(mut held) = runs().write() {
        held.insert(instance, Arc::new(run));
    }
}

/// Forget everything: what closing a project does.
pub fn clear() {
    if let Ok(mut held) = runs().write() {
        held.clear();
    }
    if let Ok(mut held) = jobs().lock() {
        held.progress.clear();
    }
}

/// Build a run out of frames somebody wrote down — the test seam `publish` is
/// fed from, and the only way a matte enters the store without a media file.
#[must_use]
pub fn run_from_planes(
    width: u32,
    height: u32,
    fps: f64,
    clip_frames: usize,
    planes: &[(i64, [u8; 32], Vec<u8>)],
) -> Option<RotoRun> {
    let mut records: Vec<FrameRecord> = planes
        .iter()
        .map(|(frame, chain, plane)| {
            let (bbox, lz4) = shrink(plane, width, height);
            FrameRecord {
                frame: *frame,
                chain: *chain,
                bbox,
                lz4,
            }
        })
        .collect();
    records.sort_by_key(|r| r.frame);
    Some(RotoRun {
        width,
        height,
        fps,
        clip_frames,
        first_frame: records.first()?.frame,
        last_frame: records.last()?.frame,
        records,
        warm: Mutex::new(Vec::new()),
    })
}

// ---------------------------------------------------------------------------
// Asking for one
// ---------------------------------------------------------------------------

/// Start `job` on its own thread.
///
/// Returns as soon as the thread is spawned; the cache probe happens *there*, so
/// no caller — least of all the interface thread — ever waits on the disk
/// (docs/14 §1.1). The two refusals that can be answered without touching a
/// disk, [`RotoFailure::Offline`] and [`RotoFailure::NoBaseFrame`], are answered
/// here so a button can say why it did nothing.
pub fn request(job: RotoJob) -> Requested {
    let instance = job.instance;
    if job.key.is_none() {
        return Requested::Refused(RotoFailure::Offline);
    }
    if job.propagate && job.block.base_frame.is_none() {
        return Requested::Refused(RotoFailure::NoBaseFrame);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut held) = jobs().lock() else {
            return Requested::Refused(RotoFailure::Busy);
        };
        if held.running.is_some() {
            return Requested::Refused(RotoFailure::Busy);
        }
        held.running = Some((instance, Arc::clone(&cancel)));
        held.progress.insert(instance, Progress::Queued);
    }
    let spawned = std::thread::Builder::new()
        .name("lumit-roto".into())
        .spawn(move || run(job, &cancel));
    if spawned.is_err() {
        // The slot was claimed above and nothing would ever release it.
        finish(instance, None);
        return Requested::Refused(RotoFailure::Busy);
    }
    Requested::Started
}

/// Stop `instance`'s propagation. The flag is raised and the run ends **between
/// frames**, keeping and filing every frame it had finished (K-540).
pub fn cancel(instance: Uuid) {
    let Ok(mut held) = jobs().lock() else {
        return;
    };
    if let Some((running, flag)) = &held.running {
        if *running == instance {
            flag.store(true, Ordering::Relaxed);
            return;
        }
    }
    held.progress.insert(instance, Progress::Cancelled);
}

/// Record a refusal [`request`] answered without ever spawning a thread, so the
/// status row can say *why* the button did nothing.
///
/// A press is an event and has nothing to poll against (the camera track's §5c
/// first deviation), so the reason has to be left somewhere the next status read
/// will find it. This is that somewhere, and it is the same map every other
/// reading comes out of.
pub fn note_refusal(instance: Uuid, failure: RotoFailure) {
    report(instance, Progress::Failed(failure));
}

fn report(instance: Uuid, step: Progress) {
    if let Ok(mut held) = jobs().lock() {
        held.progress.insert(instance, step);
    }
}

/// Take the job off the running slot and publish its outcome. `None` **forgets**
/// the instance: a warm pass that found nothing has nothing to say about it.
fn finish(instance: Uuid, outcome: Option<Progress>) {
    if let Ok(mut held) = jobs().lock() {
        if held.running.as_ref().is_some_and(|(i, _)| *i == instance) {
            held.running = None;
        }
        match outcome {
            Some(step) => held.progress.insert(instance, step),
            None => held.progress.remove(&instance),
        };
    }
}

/// The whole of one job, on the propagation thread: read the sidecar, and only
/// if there is nothing there, propagate.
fn run(job: RotoJob, cancel: &AtomicBool) {
    let instance = job.instance;
    let key = job.key;
    let dir = cache_dir();

    if let Some(run) = key
        .zip(dir.as_deref())
        .and_then(|(key, d)| read_sidecar(d, key))
    {
        publish(instance, run);
        finish(instance, Some(Progress::Done));
        return;
    }
    if !job.propagate {
        // A warm pass found nothing. That is not a failure and must not look
        // like one: nobody asked for this shot to be cut.
        finish(instance, None);
        return;
    }

    match propagate(job, cancel, &|step| report(instance, step)) {
        Ok((run, cancelled)) => {
            // Written before the store is filled, so a run the interface can see
            // is a run the next session will find. A **cancelled** run is
            // cached like any other: its frames are correct and correctly
            // named, and re-deriving them would take the same minutes to reach
            // the same place.
            if let (Some(key), Some(dir)) = (key, dir.as_deref()) {
                write_sidecar(dir, key, &run);
            }
            publish(instance, run);
            finish(
                instance,
                Some(if cancelled {
                    Progress::Cancelled
                } else {
                    Progress::Done
                }),
            );
        }
        Err(e) => finish(instance, Some(Progress::Failed(e))),
    }
}

// ---------------------------------------------------------------------------
// The work
// ---------------------------------------------------------------------------

/// The document's stroke, as the arithmetic crate wants it. Two types for one
/// idea, because `lumit-core` sits below `lumit-roto` and may not depend on it
/// (docs/05 §1.1) — the split the Camera track's density table already takes.
fn to_engine(stroke: &lumit_core::roto::RotoStroke) -> RotoStroke {
    RotoStroke {
        id: stroke.id,
        points: stroke.points.clone(),
        radius: stroke.radius,
        kind: match stroke.kind {
            RotoStrokeKind::Foreground => StrokeKind::Foreground,
            RotoStrokeKind::Background => StrokeKind::Background,
            RotoStrokeKind::Refine => StrokeKind::Refine,
        },
        frame: stroke.frame,
    }
}

/// Encoded RGBA8 to the interleaved encoded f32 RGB the solve reads.
fn to_rgb(rgba: &[u8], out: &mut Vec<f32>) {
    out.clear();
    out.reserve(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        out.push(f32::from(px[0]) / 255.0);
        out.push(f32::from(px[1]) / 255.0);
        out.push(f32::from(px[2]) / 255.0);
    }
}

/// The flow settings one Roto brush's rows mean to the flow engine.
fn flow_settings(settings: RotoSettings) -> lumit_flow::FlowSettings {
    lumit_flow::FlowSettings {
        divisor: lumit_core::fx::effects::roto_brush::flow_divisor(settings.flow_resolution),
        smoothness: settings.flow_smoothness,
        ..lumit_flow::FlowSettings::default()
    }
}

/// The arithmetic crate's settings from the document's.
fn solver_settings(settings: RotoSettings) -> lumit_roto::RotoSettings {
    lumit_roto::RotoSettings {
        guide_radius: settings.refine_radius.clamp(0.0, 256.0) as u32,
        ..lumit_roto::RotoSettings::default()
    }
}

/// Decode, cut, carry. The whole cost of a propagation, and — given the same
/// frames — a pure function of its inputs.
///
/// Separated from [`run`] so the engine tests can drive it directly with a
/// deterministic progress log, rather than racing a thread to observe one.
/// Answers `(the run, whether it was cancelled)`: a cancelled run is an answer,
/// not an error.
fn propagate(
    job: RotoJob,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(RotoRun, bool), RotoFailure> {
    let base = job.block.base_frame.ok_or(RotoFailure::NoBaseFrame)?;
    let mut frames = (job.open)().ok_or(RotoFailure::Unreadable)?;
    let (count, width, height, fps) = frames.info();
    if count == 0 || width == 0 || height == 0 {
        return Err(RotoFailure::NoFrames);
    }
    let last_index = i64::try_from(count.saturating_sub(1)).unwrap_or(i64::MAX);
    let base = base.clamp(0, last_index);

    // The flow engine, and the one refusal only it can answer. `new_auto` opens
    // a headless device of its own; a build with no GPU flow degrades to the CPU
    // oracle, which is exactly what this job may not use (§8).
    let mut flow = lumit_flow::FlowEngine::new_auto();
    if !flow.backend().starts_with("dis-gpu") {
        return Err(RotoFailure::FlowUnavailable);
    }
    let flow_set = flow_settings(job.settings);

    let dir = cache_dir();
    let lend = match (job.key, dir.as_deref()) {
        (Some(key), Some(d)) => lendable(d, key),
        _ => HashMap::new(),
    };

    let mut solver = RotoSolver::new(solver_settings(job.settings));
    let mut seeds = Seeds::new(width, height).map_err(|_| RotoFailure::NoFrames)?;
    let mut rgb: Vec<f32> = Vec::new();
    let n = (width as usize) * (height as usize);
    let mut matte = vec![0f32; n];
    let mut records: Vec<FrameRecord> = Vec::new();
    let mut reused = 0usize;
    let mut cancelled = false;

    // The base frame first: both directions start from its answer.
    let base_chain = chain_hash(&job.block, job.settings, base).ok_or(RotoFailure::NoBaseFrame)?;
    let base_plane = match lend.get(&base_chain) {
        Some(record) => {
            reused += 1;
            expand(record, width, height)
        }
        None => {
            let rgba = frames.rgba(base as usize).ok_or(RotoFailure::NoFrames)?;
            to_rgb(&rgba, &mut rgb);
            let strokes: Vec<RotoStroke> = job
                .block
                .contributing(base)
                .into_iter()
                .map(to_engine)
                .collect();
            let base_field =
                base_seeds(width, height, &strokes).map_err(|_| RotoFailure::NoSeeds)?;
            let frame = FrameRgb::new(&rgb, width, height).map_err(|_| RotoFailure::NoFrames)?;
            solver
                .solve(frame, &base_field, &mut matte)
                .map_err(|_| RotoFailure::NoSeeds)?;
            to_gray8(&matte)
        }
    };
    records.push(record_of(base, base_chain, &base_plane, width, height));
    report(Progress::Solving {
        done: 1,
        total: count,
        reused,
    });

    // Then outward, one direction at a time. The two walks are the same four
    // stages with the pair reversed, so they share one closure rather than two
    // copies (docs/impl/roto.md §3).
    let mut done = 1usize;
    // Hoisted beside `rgb` and `matte`, and for the same reason: a solved frame
    // refills them rather than allocating them. `validity` never changes at all
    // — every pixel of a full-raster field is valid — so it is built once here.
    let mut interleaved: Vec<f32> = Vec::with_capacity(n * 2);
    let mut prev_f: Vec<f32> = Vec::with_capacity(n);
    let validity = vec![1u8; n];
    for direction in [1i64, -1] {
        let mut prev_plane = base_plane.clone();
        let mut prev_rgba: Option<Vec<u8>> = None;
        let mut cursor = base;
        loop {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let next = cursor + direction;
            if next < 0 || next > last_index {
                break;
            }
            let Some(chain) = chain_hash(&job.block, job.settings, next) else {
                break;
            };
            if let Some(record) = lend.get(&chain) {
                // **Copied, not re-solved.** No decode, no flow, no sweep —
                // which is what makes a correction at frame 200 cost a hundred
                // solves rather than three hundred.
                prev_plane = expand(record, width, height);
                records.push(record.clone());
                reused += 1;
                prev_rgba = None;
            } else {
                let previous = match prev_rgba.take() {
                    Some(bytes) => bytes,
                    None => match frames.rgba(cursor as usize) {
                        Some(bytes) => bytes,
                        None => break,
                    },
                };
                let Some(current) = frames.rgba(next as usize) else {
                    break;
                };
                if previous.len() != n * 4 || current.len() != n * 4 {
                    break;
                }
                let a = lumit_flow::to_gray(&previous, width as usize, height as usize);
                let b = lumit_flow::to_gray(&current, width as usize, height as usize);
                // `a → b` is previous → next; the field a backward warp of the
                // *next* frame needs is `b → a`, and its confidence is measured
                // with the forward field as the reference.
                let (fwd, bwd) = flow.flow_pair_with(&a, &b, &flow_set);
                let conf = lumit_flow::confidence(&bwd, &fwd);
                let (u, v, conf) =
                    lumit_flow::field_to_size(&bwd, &conf, width as usize, height as usize);
                interleaved.clear();
                for i in 0..n {
                    interleaved.push(u.get(i).copied().unwrap_or(0.0));
                    interleaved.push(v.get(i).copied().unwrap_or(0.0));
                }
                prev_f.clear();
                prev_f.extend(prev_plane.iter().map(|&b| f32::from(b) / 255.0));
                let field = FlowField::new(&interleaved, &validity, &conf, width, height)
                    .map_err(|_| RotoFailure::NoFrames)?;
                if lumit_roto::warp_and_seed(
                    &prev_f,
                    &field,
                    solver_settings(job.settings).confidence_floor,
                    &mut seeds,
                )
                .is_err()
                {
                    break;
                }
                // The frame's own corrections outrank the warped seeds, per
                // pixel, which is how the user outranks the machine.
                let corrections: Vec<RotoStroke> = job
                    .block
                    .strokes
                    .iter()
                    .filter(|s| s.frame == next)
                    .map(to_engine)
                    .collect();
                seeds.stamp_all(&corrections);
                to_rgb(&current, &mut rgb);
                let Ok(frame) = FrameRgb::new(&rgb, width, height) else {
                    break;
                };
                if solver.solve(frame, &seeds, &mut matte).is_err() {
                    // No seed of one kind survived — the subject left the
                    // frame, or the flow trusted nothing. The span ends here
                    // rather than inventing an answer.
                    break;
                }
                prev_plane = to_gray8(&matte);
                records.push(record_of(next, chain, &prev_plane, width, height));
                prev_rgba = Some(current);
            }
            cursor = next;
            done += 1;
            report(Progress::Solving {
                done,
                total: count,
                reused,
            });
        }
        if cancelled {
            break;
        }
    }

    records.sort_by_key(|r| r.frame);
    let first_frame = records.first().map_or(base, |r| r.frame);
    let last_frame = records.last().map_or(base, |r| r.frame);
    Ok((
        RotoRun {
            width,
            height,
            fps,
            clip_frames: count,
            first_frame,
            last_frame,
            records,
            warm: Mutex::new(Vec::new()),
        },
        cancelled,
    ))
}

fn to_gray8(matte: &[f32]) -> Vec<u8> {
    matte
        .iter()
        .map(|a| (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        .collect()
}

fn record_of(frame: i64, chain: [u8; 32], plane: &[u8], width: u32, height: u32) -> FrameRecord {
    let (bbox, lz4) = shrink(plane, width, height);
    FrameRecord {
        frame,
        chain,
        bbox,
        lz4,
    }
}

// ---------------------------------------------------------------------------
// Reading a document
// ---------------------------------------------------------------------------

/// The real frames, through the same frame index and decoder every other
/// consumer opens (docs/impl/media-io.md §2).
pub struct MediaRgba {
    decoder: lumit_media::VideoDecoder,
    frames: usize,
    width: u32,
    height: u32,
    fps: f64,
}

impl MediaRgba {
    /// Open `path` for propagation. Costs a frame-index build the first time the
    /// file is seen, which is why this is called on the propagation thread.
    #[must_use]
    pub fn open(path: &Path) -> Option<Self> {
        let video = lumit_media::probe::probe(path).ok()?.video?;
        let index = crate::media_index::load_or_build_index(path).ok()?;
        let frames = index.frame_count();
        let decoder = lumit_media::VideoDecoder::open(path, index).ok()?;
        Some(MediaRgba {
            decoder,
            frames,
            width: video.width,
            height: video.height,
            fps: video.fps(),
        })
    }
}

impl RotoFrames for MediaRgba {
    fn info(&self) -> (usize, u32, u32, f64) {
        (self.frames, self.width, self.height, self.fps)
    }

    fn rgba(&mut self, n: usize) -> Option<Vec<u8>> {
        // **The source's own raster, never a preview tier** (K-248): a stroke is
        // in source pixels and a matte describes the file's frames.
        let frame = self.decoder.frame_rgba(n, None).ok()?;
        (frame.width == self.width && frame.height == self.height).then_some(frame.rgba)
    }
}

/// The job one Roto brush instance on one footage layer describes, or `None`
/// when there is nothing to key a cache with.
#[must_use]
pub fn job_for(
    fx: &EffectInstance,
    path: PathBuf,
    fingerprint: &Fingerprint,
    propagate: bool,
) -> Option<RotoJob> {
    let block = fx.roto.clone().unwrap_or_default();
    let settings = RotoSettings::of(fx);
    Some(RotoJob {
        instance: fx.id,
        key: Some(RotoKey::new(fingerprint, &block, settings)),
        settings,
        block,
        open: Box::new(move || MediaRgba::open(&path).map(|f| Box::new(f) as Box<dyn RotoFrames>)),
        propagate,
    })
}

/// Every cached run a document could be holding, as warm-pass jobs: one per
/// enabled Roto brush on a footage layer whose media has a fingerprint.
///
/// One job per **instance**, because that is what a run is filed under. A
/// footage item with no fingerprint or no resolved path is skipped: it is
/// offline, and there is nothing to name a run with.
#[must_use]
pub fn warm_jobs(doc: &Document) -> Vec<RotoJob> {
    let mut out = Vec::new();
    for item in &doc.items {
        let lumit_core::model::ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &comp.layers {
            let LayerKind::Footage { item: media, .. } = layer.kind else {
                continue;
            };
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
            for fx in lumit_core::roto::brushes(&layer.effects) {
                if fx.roto.as_ref().is_none_or(RotoBlock::is_empty) {
                    continue;
                }
                let path = PathBuf::from(&footage.media.absolute_path);
                if let Some(job) = job_for(fx, path, fingerprint, false) {
                    out.push(job);
                }
            }
        }
    }
    out
}

/// Read every one of `jobs` back out of the sidecar, on one thread, filling the
/// store with whatever is already there. What opening a project does.
///
/// **Not [`request`], deliberately** — `request` owns the one-at-a-time slot, so
/// warming the second brush of a project would answer `Busy` and simply not
/// happen. A warm pass is a file read per brush and nothing else.
pub fn warm(jobs: Vec<RotoJob>) {
    if jobs.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("lumit-roto-warm".into())
        .spawn(move || {
            let never = AtomicBool::new(false);
            for mut job in jobs {
                job.propagate = false;
                run(job, &never);
            }
        });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

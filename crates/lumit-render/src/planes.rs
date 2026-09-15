//! The planes analysis job, the `planes/` sidecar, and the store the render
//! path reads (docs/impl/addons.md §6.1, §7, §9).
//!
//! # In plain terms
//!
//! A model reads the shot one frame at a time and hands back a plane: how far
//! away every pixel is, or how much of each pixel is the subject. **Analyse** is
//! what runs it over the whole clip, and this file is the whole of that: the
//! work, where it happens, where its answers are kept, and what happens when
//! somebody stops it half way.
//!
//! **It happens somewhere else.** One analysis at a time, on a thread spawned
//! for it and named `lumit-planes`. Its own slot, separate from the tracker's
//! and the Roto brush's, and the reason is a different one: those two exist
//! because a disk-bound job halves another on the same drive, and this one is
//! graphics-bound and contends with the compositor's own device instead. Two
//! model runs on one card halve each other just as surely, so one slot serves
//! every model job and a second ask is answered `Busy` rather than queued.
//!
//! **The model is opened here and dropped here.** A model run is an FFI call
//! that may hold the GPU, so the model is owned by the analysis thread and
//! never put behind a shared lock (docs/14 §1.3); the job drops it the moment
//! the run ends, so nothing is held between analyses. A warm pass never opens
//! one at all: it reads the sidecar and stops.
//!
//! **It can be stopped, and stopping keeps what it had.** The Roto brush's
//! stance, for its reason: every frame reached is correct and correctly named,
//! so they are written, the span says how far it got, and a later Analyse
//! carries on from the sidecar rather than starting again. That last part
//! holds wherever the model reads each frame on its own; one that carries its
//! state from frame to frame starts again, and says so.
//!
//! **It refuses rather than pretends.** No runtime installed; no pack for the
//! task; a model that will not run; one at a time; media that will not open.
//! Every one is a refusal and none is a fault, and while one stands the effect
//! wears a calm badge and renders identity.
//!
//! # What the store hands back
//!
//! A [`PlaneRun`] per **effect instance**, not per media: two Depth effects on
//! one clip at two model settings are two answers, and only one of them is any
//! given instance's. The render path asks for one frame's plane, gets the bytes
//! back through a small decompressed-frame cache, and holds no lock while it
//! uses them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use lumit_core::model::{Document, EffectInstance, Fingerprint, LayerKind};
use lumit_core::planes::{PlaneTask, TIER_VERSION};
use uuid::Uuid;

use crate::frames::RotoFrames;
use crate::sidecar;

// ---------------------------------------------------------------------------
// What an analysis is asked for
// ---------------------------------------------------------------------------

/// What one plane holds: one per effect on this tier (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlaneKind {
    /// How far away every pixel is: one little-endian `u16` per pixel, nearer
    /// larger, scaled to the frame's own range.
    Depth,
    /// How much of each pixel is the subject: one gray8 byte per pixel.
    Matte,
}

impl PlaneKind {
    /// How many bytes one pixel of this kind takes.
    #[must_use]
    pub fn stride(self) -> usize {
        match self {
            PlaneKind::Depth => 2,
            PlaneKind::Matte => 1,
        }
    }
}

/// The settings that change what a plane **is**, read off an instance and the
/// machine together.
///
/// `Copy` and `Eq` for [`AnalysisSettings`](crate::track::AnalysisSettings)'s
/// reason, and that is why the model is a thirty-two byte identity and a small
/// code rather than its name: a `String` here would break both (§13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneSettings {
    /// What the effect asks a model for.
    pub task: PlaneTask,
    /// Which architecture, as the effect's own model Choice index.
    pub arch: u32,
    /// Which pack, and which version of it, from the manifest's own digest
    /// (§7). The pack the effect's own Model row names, not whichever one does
    /// the task: two packs do matting, and a name made from the other one's
    /// digest would not move when the chosen pack is updated. Zeros when
    /// nothing is installed, which is a run that will refuse before it starts.
    pub identity: [u8; 32],
    /// Which provider ran it, as a small code: an answer made on one and
    /// served for another is the thing §7 exists to stop.
    pub provider: u32,
    /// The rows of the effect that change the answer and not the look. Today
    /// only Robust Video Matting's detail row, which is how much of the frame
    /// it works at.
    pub downsample: u32,
}

impl PlaneSettings {
    /// Read the settings off one instance and the installed packs together.
    ///
    /// `None` for anything that is not a planes-tier effect. The identity is
    /// zeros where nothing is installed rather than a refusal, so a key can
    /// still be made for a warm pass that will find nothing.
    #[must_use]
    pub fn of(fx: &EffectInstance) -> Option<Self> {
        let task = lumit_core::planes::task_of(fx)?;
        let rows = lumit_core::planes::PlaneSettings::of(fx);
        Some(PlaneSettings {
            task,
            arch: rows.model,
            // Asked of the family the Model row names, for the reason the
            // badge asks the same question that way: the store's own reading
            // answers with whichever pack of the task sorts first, so with both
            // matte packs installed every run would be named after one of them
            // and updating the other would rename nothing.
            identity: match task {
                PlaneTask::Depth => lumit_ml::store::installed_identity(ml_task(task)),
                PlaneTask::Matte => lumit_ml::matte::identity(matte_arch(rows.model)),
            }
            .unwrap_or_default(),
            provider: provider_code(),
            downsample: rows.detail,
        })
    }

    /// Feed the settings into a hash, in a fixed order.
    fn feed(&self, h: &mut blake3::Hasher) {
        h.update(&[self.task.tag()]);
        h.update(&self.arch.to_le_bytes());
        h.update(&self.identity);
        h.update(&self.provider.to_le_bytes());
        h.update(&self.downsample.to_le_bytes());
    }
}

/// The tier's own copy of a task, in the crate that opens a model.
#[must_use]
pub fn ml_task(task: PlaneTask) -> lumit_ml::Task {
    match task {
        PlaneTask::Depth => lumit_ml::Task::Depth,
        PlaneTask::Matte => lumit_ml::Task::Matte,
    }
}

/// Which provider a plane made now is made by, as a code the settings can
/// carry. A word would break their `Copy` and `Eq`.
///
/// The provider that **ran**, once a session has been opened on this machine,
/// and the one the platform asks for first before that (§7): an answer made on
/// the processor because the accelerator would not register is never served
/// under the accelerator's name.
fn provider_code() -> u32 {
    match lumit_ml::runtime::provider() {
        "DirectML" => 1,
        "CoreML" => 2,
        _ => 0,
    }
}

/// What names one analysis's file: the media's own content, and everything the
/// settings cover.
///
/// Two halves rather than one hash, in the Roto brush's shape and for its
/// reason: the **media prefix** can be enumerated, so a run under new settings
/// can still find what an earlier one left beside it. Filed under the effect
/// instance in the store, and under this in the folder, so two projects on the
/// same rushes with the same settings find the same file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlaneKey {
    media: [u8; 16],
    run: [u8; 16],
}

/// Bump when a record's meaning changes. Old files then hash to a different
/// name and are simply never asked for. Two, because a record now keeps the box
/// its bytes cover rather than always the whole plane.
const FORMAT_VERSION: u16 = 2;

/// `LUMPLN\0` - read before anything is deserialised, so a file that is not one
/// of ours is refused rather than fed to a decoder.
const MAGIC: &[u8; 7] = b"LUMPLN\0";

impl PlaneKey {
    /// The key for `fingerprint` analysed under `settings`.
    #[must_use]
    pub fn new(fingerprint: &Fingerprint, settings: PlaneSettings) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"lumit-planes/media/");
        h.update(&fingerprint.size.to_le_bytes());
        h.update(fingerprint.head_tail_hash.as_bytes());
        let mut media = [0u8; 16];
        media.copy_from_slice(&h.finalize().as_bytes()[..16]);

        let mut h = blake3::Hasher::new();
        h.update(b"lumit-planes/run/");
        h.update(&FORMAT_VERSION.to_le_bytes());
        h.update(&TIER_VERSION.to_le_bytes());
        settings.feed(&mut h);
        let mut run = [0u8; 16];
        run.copy_from_slice(&h.finalize().as_bytes()[..16]);
        PlaneKey { media, run }
    }

    fn prefix(&self) -> String {
        self.media.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn file_name(&self) -> String {
        let run: String = self.run.iter().map(|b| format!("{b:02x}")).collect();
        format!("{}-{run}.lpln", self.prefix())
    }
}

/// One analysis, as handed to the worker.
pub struct PlaneJob {
    /// The **effect instance** the answer is filed under, and what
    /// [`progress`] is read by.
    pub instance: Uuid,
    /// What the sidecar calls it, or `None` for a source with no fingerprint,
    /// which is refused before a thread is spawned.
    pub key: Option<PlaneKey>,
    pub settings: PlaneSettings,
    /// Opens the frames, **on the worker thread**.
    pub open: Box<dyn FnOnce() -> Option<Box<dyn RotoFrames>> + Send>,
    /// `false` asks only for a cache hit: the warm pass a project open makes,
    /// which must never start a model nobody asked for.
    pub analyse: bool,
    /// Whether this job holds the tier's one-at-a-time slot, and may therefore
    /// give it back. Set by [`request`], which is the only thing that claims
    /// it; a warm pass runs beside whatever is analysing and carries the same
    /// instance ids out of the file, so one handing the slot back would let a
    /// second model job start on the same card (§9).
    pub owns_slot: bool,
    /// Read no further than this frame. Nothing presses for it today; it is
    /// here because the shape is the Roto brush's and a single frame's
    /// feedback is the obvious next ask.
    pub stop_after: Option<i64>,
}

/// How far an analysis has got. Read, never subscribed to - the interface
/// samples it as it repaints, exactly as it samples the cache bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Accepted, not started.
    Queued,
    /// Reading: `done` of `total` frames.
    Solving {
        done: usize,
        total: usize,
    },
    /// There is a run in the store for this instance.
    Done,
    /// Stopped between frames. **The finished prefix was kept.**
    Cancelled,
    Failed(PlaneFailure),
}

/// Why an analysis produced no planes. Every variant is a refusal rather than a
/// fault, and the enum is **closed** with no free text in it, so the bridge
/// hands the interface a reason rather than an English sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlaneFailure {
    /// No model runtime is installed, so nothing can run at all.
    #[error("no model runtime is installed")]
    RuntimeMissing,
    /// Nothing installed does this task.
    #[error("no model pack is installed for this task")]
    PackMissing,
    /// A pack that is there and would not open, or a run the model refused.
    #[error("the model would not run")]
    ModelFailed,
    /// One model job at a time.
    #[error("another analysis is running")]
    Busy,
    /// The media could not be opened, or carries no video.
    #[error("the media could not be read")]
    Unreadable,
    /// Opened, but with no frames or no raster.
    #[error("the media has no frames to read")]
    NoFrames,
    /// Stopped between frames.
    #[error("cancelled")]
    Cancelled,
}

impl PlaneFailure {
    /// The tier's own reading of what the model crate answered. The detail a
    /// library wrote is dropped here on purpose: the badge reads the missing
    /// pack's name off the effect, and a sentence from ONNX Runtime is not
    /// something a reason enum can carry (§9).
    fn of(e: &lumit_ml::MlError) -> Self {
        use lumit_ml::MlError as E;
        match e {
            E::RuntimeMissing | E::RuntimeFailed(_) | E::RuntimeInUse => {
                PlaneFailure::RuntimeMissing
            }
            E::PackMissing(_) | E::NotInstalled => PlaneFailure::PackMissing,
            E::Cancelled => PlaneFailure::Cancelled,
            E::PackUnreadable | E::Invalid(_) | E::Busy | E::ModelFailed(_) | E::ShapeMismatch => {
                PlaneFailure::ModelFailed
            }
        }
    }
}

/// What happened when an analysis was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    /// Accepted; watch [`progress`].
    Started,
    /// Refused, with the reason. [`PlaneFailure::Busy`] is the ordinary one.
    Refused(PlaneFailure),
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// One frame's answer, as a model hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneOut {
    pub width: u32,
    pub height: u32,
    pub kind: PlaneKind,
    /// The plane itself, [`PlaneKind::stride`] bytes a pixel, row-major. A
    /// depth plane's pairs are little-endian.
    pub data: Vec<u8>,
}

/// A model, as this job uses one.
///
/// A trait and not the concrete type, so the whole job - the loop, the
/// progress, the cancel, the records, the sidecar - is tested on every machine
/// with a model the test wrote down, and none of it waits on a runtime nobody
/// installed on a CI runner. The real one is [`open_model`].
pub trait PlaneModel {
    /// Read one frame of RGBA bytes at `w` by `h`.
    fn run(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<PlaneOut, PlaneFailure>;
    /// What made these planes, for the record's provenance (§7): the provider
    /// first, then the pack and the runtime. The provider is read back off the
    /// front of it by [`PlaneRun::provider`].
    fn made_with(&self) -> String;
}

/// The depth model, wrapped so the job never names `lumit_ml` itself.
struct DepthModel(lumit_ml::Depth);

impl PlaneModel for DepthModel {
    fn run(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<PlaneOut, PlaneFailure> {
        let plane = self.0.run(rgba, w, h).map_err(|e| PlaneFailure::of(&e))?;
        let mut data = Vec::with_capacity(plane.data.len() * 2);
        for value in &plane.data {
            data.extend_from_slice(&value.to_le_bytes());
        }
        Ok(PlaneOut {
            width: plane.width,
            height: plane.height,
            kind: PlaneKind::Depth,
            data,
        })
    }

    fn made_with(&self) -> String {
        // The pack and its hash as well as the provider, because §7's whole
        // point is that a plane can be asked later which pack made it, and an
        // updated pack under the same id answers that only by its hash.
        made_with(self.0.provider(), self.0.pack(), self.0.identity())
    }
}

/// The matte model, wrapped the same way.
struct MatteModel(lumit_ml::Matte);

impl PlaneModel for MatteModel {
    fn run(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<PlaneOut, PlaneFailure> {
        let coverage = self.0.run(rgba, w, h).map_err(|e| PlaneFailure::of(&e))?;
        Ok(PlaneOut {
            width: coverage.width,
            height: coverage.height,
            kind: PlaneKind::Matte,
            data: coverage.data,
        })
    }

    fn made_with(&self) -> String {
        made_with(self.0.provider(), self.0.pack(), self.0.identity())
    }
}

/// The provenance a run carries (§7): the provider first, because
/// [`PlaneRun::provider`] reads it back off the front, then which pack and
/// which version of it, then the runtime that ran it.
fn made_with(provider: &str, pack: &str, identity: [u8; 32]) -> String {
    let runtime = match lumit_ml::runtime::status() {
        lumit_ml::RuntimeStatus::Loaded { version, .. } => version,
        _ => String::new(),
    };
    let hash: String = identity.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{provider}; {pack}; {hash}; ONNX Runtime {runtime}")
}

/// Which family of matte model the effect's Model row names.
///
/// The row's options are the effect's own (`remove_background::MODEL_OPTIONS`)
/// and are read here rather than there, for the Camera track's reason: the
/// crate that owns the control cannot depend on the crate that opens a model.
/// An index this build does not know reads as the first, which is what the
/// effect's own table does with one.
fn matte_arch(index: u32) -> lumit_ml::matte::MatteArch {
    match index {
        1 => lumit_ml::matte::MatteArch::Birefnet,
        _ => lumit_ml::matte::MatteArch::Rvm,
    }
}

/// How much of the frame Robust Video Matting works at, off the effect's own
/// Detail row.
fn matte_detail(index: u32) -> lumit_ml::tensor::Detail {
    match index {
        1 => lumit_ml::tensor::Detail::FullBody,
        _ => lumit_ml::tensor::Detail::Portrait,
    }
}

/// Open the model `settings` names, on the thread that will run it.
fn open_model(settings: PlaneSettings) -> Result<Box<dyn PlaneModel>, PlaneFailure> {
    match settings.task {
        PlaneTask::Depth => lumit_ml::Depth::open()
            .map(|model| Box::new(DepthModel(model)) as Box<dyn PlaneModel>)
            .map_err(|e| PlaneFailure::of(&e)),
        PlaneTask::Matte => {
            lumit_ml::Matte::open(matte_arch(settings.arch), matte_detail(settings.downsample))
                .map(|model| Box::new(MatteModel(model)) as Box<dyn PlaneModel>)
                .map_err(|e| PlaneFailure::of(&e))
        }
    }
}

/// Why an instance's model cannot run on this machine, or `None` when it can.
///
/// Asked before a job is spawned, so a press can say why it did nothing, and
/// asked again by the badge, so the effect says the same thing sitting still.
/// Nothing here opens a model: it reads the installed-packs snapshot, which is
/// what the Addons page writes and the frame key reads.
#[must_use]
pub fn refusal_for(fx: &EffectInstance) -> Option<PlaneFailure> {
    let task = lumit_core::planes::task_of(fx)?;
    if matches!(
        lumit_ml::runtime::status(),
        lumit_ml::RuntimeStatus::Missing
    ) {
        return Some(PlaneFailure::RuntimeMissing);
    }
    let rows = lumit_core::planes::PlaneSettings::of(fx);
    let installed = match task {
        PlaneTask::Depth => lumit_ml::store::find(ml_task(task)).is_some(),
        // Asked of the family the Model row names rather than of the task,
        // because two packs do matting and the other one is a different answer
        // rather than a substitute for the one that was chosen.
        PlaneTask::Matte => lumit_ml::matte::installed(matte_arch(rows.model)),
    };
    (!installed).then_some(PlaneFailure::PackMissing)
}

/// What an instance is missing, as the badge's detail slot wants it, or `None`
/// when the machine can run its model.
///
/// The one place a name is put to a missing addon. The runtime has one name;
/// a pack is named by the effect's own model row, because that is the pack the
/// user asked for and the store cannot name one it has not got.
#[must_use]
pub fn addon_missing(fx: &EffectInstance) -> Option<String> {
    match refusal_for(fx)? {
        PlaneFailure::RuntimeMissing => Some(lumit_ml::runtime::RUNTIME_ID.to_owned()),
        _ => {
            let rows = lumit_core::planes::PlaneSettings::of(fx);
            Some(
                match lumit_core::planes::task_of(fx)? {
                    PlaneTask::Depth => lumit_core::fx::effects::depth::model_pack(rows.model),
                    PlaneTask::Matte => {
                        lumit_core::fx::effects::remove_background::model_pack(rows.model)
                    }
                }
                .to_owned(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// What comes back
// ---------------------------------------------------------------------------

/// One frame's plane as the sidecar keeps it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct FrameRecord {
    frame: i64,
    kind: PlaneKind,
    /// The plane's own raster, which is the **model's** rather than the
    /// frame's: a depth model answers at a few hundred pixels on the long side
    /// and produced nothing finer, so growing it here would be inventing detail
    /// and then storing it. The draw resamples on the card.
    width: u32,
    height: u32,
    /// `(x, y, width, height)` of the part of the plane the bytes below cover.
    /// Everything outside it is nought.
    bbox: [u32; 4],
    /// The plane inside the box, LZ4. A depth plane is smooth over most of a
    /// frame and a matte is mostly runs of nothing and everything, so this is a
    /// small fraction of the raw bytes either way.
    lz4: Vec<u8>,
}

/// One effect instance's analysis, as the render path wants it.
#[derive(Debug)]
pub struct PlaneRun {
    pub fps: f64,
    /// How many frames the **clip** has, against which the analysed span is a
    /// whole answer or a partial one.
    pub clip_frames: usize,
    /// The span actually analysed, inclusive. Outside it the effect is a
    /// passthrough - never a held neighbouring plane.
    pub first_frame: i64,
    pub last_frame: i64,
    /// What made these planes (§7): the provider, the pack and its hash, and
    /// the runtime version, written when the run was made and read back
    /// whenever it is.
    pub made_with: String,
    /// A short name for what this run holds, over every record in it.
    ///
    /// Carried down to the draw, so the frame a plane was drawn into and the
    /// texture it was uploaded as both know which analysis made the bytes they
    /// hold: nothing else in a frame's name moves when an analysis lands, and
    /// a frame banked before one would be served back with no depth in it
    /// forever (§13). Taken off the records rather than counted, because the
    /// disk tier keeps a frame's name across restarts and a number counted in
    /// one run of the application means nothing in the next.
    pub content: u64,
    /// Ascending by frame, so a lookup is a binary search.
    records: Vec<FrameRecord>,
    /// The last few planes decompressed, so scrubbing one region does not
    /// re-inflate a plane per repaint. A four-entry ring walked linearly, which
    /// is an LRU at this size; the lock is held for a `Vec` scan and never
    /// across a decompression that misses.
    warm: Mutex<Vec<(i64, Arc<Vec<u8>>)>>,
}

/// How many decompressed planes stay warm. Four covers a scrub back and forth
/// over a boundary, and a depth plane at the model's own raster is a few
/// hundred kilobytes rather than the frame's own megabytes.
const WARM_FRAMES: usize = 4;

impl PlaneRun {
    fn index(&self, frame: i64) -> Option<usize> {
        self.records.binary_search_by_key(&frame, |r| r.frame).ok()
    }

    /// Whether the clip runs on past what was analysed - cancelled part-way, or
    /// the frames stopped decoding.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        let clip = i64::try_from(self.clip_frames).unwrap_or(i64::MAX);
        self.first_frame > 0 || self.last_frame + 1 < clip
    }

    /// Which provider made these planes: the first field of [`Self::made_with`].
    #[must_use]
    pub fn provider(&self) -> &str {
        self.made_with
            .split(';')
            .next()
            .unwrap_or(&self.made_with)
            .trim()
    }

    /// Frame `frame`'s plane, or `None` outside the analysed span.
    ///
    /// The `Arc` is cloned out from under the lock, so nothing is held while
    /// the caller uploads it (docs/14 §1.3).
    #[must_use]
    pub fn plane(&self, frame: i64) -> Option<(u32, u32, PlaneKind, Arc<Vec<u8>>)> {
        let record = self.records.get(self.index(frame)?)?;
        if let Ok(warm) = self.warm.lock() {
            if let Some((_, plane)) = warm.iter().find(|(f, _)| *f == frame) {
                return Some((record.width, record.height, record.kind, Arc::clone(plane)));
            }
        }
        let plane = Arc::new(expand(record));
        if let Ok(mut warm) = self.warm.lock() {
            warm.insert(0, (frame, Arc::clone(&plane)));
            warm.truncate(WARM_FRAMES);
        }
        Some((record.width, record.height, record.kind, plane))
    }
}

/// Blow one record's boxed, compressed plane back out to its whole raster.
/// Anything that will not decompress, or comes back the wrong length, reads as
/// an empty plane rather than a panic - a corrupt cache costs an Analyse, never
/// a frame.
fn expand(record: &FrameRecord) -> Vec<u8> {
    let stride = record.kind.stride();
    let (width, height) = (record.width as usize, record.height as usize);
    let want = width * height * stride;
    let [bx, by, bw, bh] = record.bbox.map(|n| n as usize);
    if bw == 0 || bh == 0 {
        return vec![0u8; want];
    }
    let Ok(boxed) = lz4_flex::decompress_size_prepended(&record.lz4) else {
        return vec![0u8; want];
    };
    if boxed.len() != bw * bh * stride {
        return vec![0u8; want];
    }
    if (bx, by, bw, bh) == (0, 0, width, height) {
        return boxed;
    }
    let mut plane = vec![0u8; want];
    for row in 0..bh {
        let into = ((by + row) * width + bx) * stride;
        let from = row * bw * stride;
        let (Some(d), Some(s)) = (
            plane.get_mut(into..into + bw * stride),
            boxed.get(from..from + bw * stride),
        ) else {
            continue;
        };
        d.copy_from_slice(s);
    }
    plane
}

fn record_of(frame: i64, out: &PlaneOut) -> FrameRecord {
    let (bbox, lz4) = match out.kind {
        // A matte is a subject in a box with nothing around it, so only the
        // box is kept - the Roto brush's own arithmetic, shared with it.
        PlaneKind::Matte => crate::roto::shrink(&out.data, out.width, out.height),
        // A depth plane is a reading of every pixel and nought there means the
        // furthest thing in the frame rather than nothing at all, so there is
        // no empty margin to cut off.
        PlaneKind::Depth => (
            [0, 0, out.width, out.height],
            lz4_flex::compress_prepend_size(&out.data),
        ),
    };
    FrameRecord {
        frame,
        kind: out.kind,
        width: out.width,
        height: out.height,
        bbox,
        lz4,
    }
}

// ---------------------------------------------------------------------------
// The sidecar
// ---------------------------------------------------------------------------

/// What one `.lpln` file holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    /// Repeated inside the file as well as in its name, so a collision or a
    /// renamed file is caught rather than believed.
    key: [u8; 32],
    fps: f64,
    clip_frames: u64,
    made_with: String,
    frames: Vec<FrameRecord>,
}

fn key_bytes(key: PlaneKey) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&key.media);
    out[16..].copy_from_slice(&key.run);
    out
}

/// Serialise a run, in the crate's shared framing ([`crate::sidecar`]).
fn encode(key: PlaneKey, run: &PlaneRun) -> Option<Vec<u8>> {
    let body = bincode::serialize(&Record {
        key: key_bytes(key),
        fps: run.fps,
        clip_frames: run.clip_frames as u64,
        made_with: run.made_with.clone(),
        frames: run.records.clone(),
    })
    .ok()?;
    Some(sidecar::frame(MAGIC, FORMAT_VERSION, &body))
}

/// The inverse, refusing anything it cannot vouch for: wrong magic, a version
/// from the future, a body that will not parse, or a stored key that is not the
/// one asked for. Every refusal costs one Analyse and nothing else.
fn decode(bytes: &[u8], key: PlaneKey) -> Option<Record> {
    let body = sidecar::unframe(bytes, MAGIC, FORMAT_VERSION)?;
    let record: Record = bincode::deserialize(body).ok()?;
    (record.key == key_bytes(key)).then_some(record)
}

fn record_to_run(record: Record) -> Option<PlaneRun> {
    let first = record.frames.first()?.frame;
    let last = record.frames.last()?.frame;
    Some(PlaneRun {
        fps: record.fps,
        clip_frames: usize::try_from(record.clip_frames).unwrap_or(usize::MAX),
        first_frame: first,
        last_frame: last,
        content: content_of(&record.frames, &record.made_with),
        made_with: record.made_with,
        records: record.frames,
        warm: Mutex::new(Vec::new()),
    })
}

fn read_sidecar(dir: &Path, key: PlaneKey) -> Option<PlaneRun> {
    let bytes = std::fs::read(dir.join(key.file_name())).ok()?;
    record_to_run(decode(&bytes, key)?)
}

fn write_sidecar(dir: &Path, key: PlaneKey, run: &PlaneRun) {
    if let Some(bytes) = encode(key, run) {
        sidecar::write(dir, &key.file_name(), &bytes);
    }
}

/// Where the sidecar lives. Overridable in tests, which must never write into
/// the user's own cache folder - the shape [`crate::track`] and [`crate::roto`]
/// both use, and process-wide rather than per thread because the analysis runs
/// on a thread this module spawns.
fn cache_dir() -> Option<PathBuf> {
    test_cache_dir().or_else(lumit_project::planes_cache_dir)
}

#[cfg(not(test))]
fn test_cache_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
static TEST_CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

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

/// Every analysis Lumit has in hand, **by effect instance**.
fn runs() -> &'static RwLock<HashMap<Uuid, Arc<PlaneRun>>> {
    static RUNS: OnceLock<RwLock<HashMap<Uuid, Arc<PlaneRun>>>> = OnceLock::new();
    RUNS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The one analysis in flight, and what every instance's last one did.
struct Jobs {
    running: Option<(Uuid, Arc<AtomicBool>)>,
    progress: HashMap<Uuid, Progress>,
}

/// This tier's own slot, separate from the tracker's and the Roto brush's. The
/// module note says why it is one slot for every analysis a model reads.
///
/// The Roto brush's segmentation deliberately does not take it: that is one
/// encode and one decode at the head of a propagation, and refusing a whole
/// shot over 77 ms of contention would cost minutes to save milliseconds
/// (docs/impl/addons.md §9).
fn jobs() -> &'static Mutex<Jobs> {
    static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
    JOBS.get_or_init(|| {
        Mutex::new(Jobs {
            running: None,
            progress: HashMap::new(),
        })
    })
}

/// What has been analysed under the effect instance `instance`. Cloned out of
/// the table so no lock is held while it is used.
#[must_use]
pub fn analysed(instance: Uuid) -> Option<Arc<PlaneRun>> {
    runs().read().ok()?.get(&instance).cloned()
}

/// The span `instance`'s planes cover, inclusive.
#[must_use]
pub fn span(instance: Uuid) -> Option<(i64, i64)> {
    let run = analysed(instance)?;
    Some((run.first_frame, run.last_frame))
}

/// One frame's plane as the draw builder takes it: the model's own raster,
/// what kind of plane it is, the bytes, and the name of the run they came
/// out of.
pub type PlaneRead = (u32, u32, PlaneKind, Arc<Vec<u8>>, u64);

/// Frame `frame`'s plane for `instance`, and the name of the run it came out
/// of, or `None` outside the analysed span, where the effect renders
/// passthrough rather than holding a neighbour.
///
/// The name comes off the same run the bytes did, so a publish landing between
/// the two readings cannot label one analysis's plane with another's name.
#[must_use]
pub fn plane(instance: Uuid, frame: i64) -> Option<PlaneRead> {
    let run = analysed(instance)?;
    let (width, height, kind, data) = run.plane(frame)?;
    Some((width, height, kind, data, run.content))
}

/// What `instance`'s analysis holds, as one number, or `None` when there is no
/// run for it at all.
///
/// This is what the frame key folds in (`crate::cache`'s stamper), because an
/// analysis landing moves nothing else in a frame's name: the document is
/// untouched and the installed pack is the one it always was, so without it
/// every frame banked before the run would be served back with no depth in it
/// (§13's "the frame key is the easiest thing to forget"). Read out from under
/// the guard as a number, so nothing is cloned to answer it.
#[must_use]
pub fn content(instance: Uuid) -> Option<u64> {
    Some(runs().read().ok()?.get(&instance)?.content)
}

/// Name what a run holds, once, where the run is made.
///
/// Over the compressed planes themselves rather than over the span alone: two
/// runs of one clip can cover the same frames and hold different pictures, and
/// which of those the viewer is showing is the whole question. One pass over
/// bytes that have just been read or just been made, and never in a render
/// loop.
fn content_of(records: &[FrameRecord], made_with: &str) -> u64 {
    let mut h = blake3::Hasher::new();
    h.update(b"lumit-planes/content/");
    h.update(made_with.as_bytes());
    for record in records {
        h.update(&record.frame.to_le_bytes());
        h.update(&[record.kind.stride() as u8]);
        h.update(&record.width.to_le_bytes());
        h.update(&record.height.to_le_bytes());
        for side in record.bbox {
            h.update(&side.to_le_bytes());
        }
        h.update(&record.lz4);
    }
    let mut out = [0u8; 8];
    out.copy_from_slice(&h.finalize().as_bytes()[..8]);
    u64::from_le_bytes(out)
}

/// How far `instance`'s analysis has got.
#[must_use]
pub fn progress(instance: Uuid) -> Option<Progress> {
    jobs().lock().ok()?.progress.get(&instance).cloned()
}

/// Put a run in the store. Public because this is how one gets in and there is
/// exactly one way - an analysis finishing, a sidecar being read back - and the
/// bridge's own tests need one without a model to make it with.
pub fn publish(instance: Uuid, run: PlaneRun) {
    if let Ok(mut held) = runs().write() {
        held.insert(instance, Arc::new(run));
    }
}

/// Forget everything: what closing the last project does.
pub fn clear() {
    if let Ok(mut held) = runs().write() {
        held.clear();
    }
    if let Ok(mut held) = jobs().lock() {
        held.progress.clear();
    }
}

/// Forget the runs and readings stored under `ids`, and no others. What closing
/// a project does with its own instances (see [`owned_ids`]), since the store is
/// shared by every project in the process. The sidecar is untouched, so
/// reopening reads them straight back.
pub fn forget(ids: &[Uuid]) {
    if let Ok(mut held) = runs().write() {
        held.retain(|id, _| !ids.contains(id));
    }
    if let Ok(mut held) = jobs().lock() {
        held.progress.retain(|id, _| !ids.contains(id));
    }
}

/// Every id a document's planes are stored under: its planes-tier effect
/// instances, enabled or not.
#[must_use]
pub fn owned_ids(doc: &Document) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for item in &doc.items {
        let lumit_core::model::ProjectItem::Composition(comp) = item else {
            continue;
        };
        ids.extend(
            comp.layers
                .iter()
                .flat_map(|layer| &layer.effects)
                .filter(|e| lumit_core::planes::task_of(e).is_some())
                .map(|e| e.id),
        );
    }
    ids
}

/// Build a run out of planes somebody wrote down - the test seam [`publish`] is
/// fed from, and the only way a plane enters the store without a model.
#[must_use]
pub fn run_from_planes(
    fps: f64,
    clip_frames: usize,
    made_with: &str,
    planes: &[(i64, PlaneOut)],
) -> Option<PlaneRun> {
    let mut records: Vec<FrameRecord> = planes
        .iter()
        .map(|(frame, out)| record_of(*frame, out))
        .collect();
    records.sort_by_key(|r| r.frame);
    Some(PlaneRun {
        fps,
        clip_frames,
        first_frame: records.first()?.frame,
        last_frame: records.last()?.frame,
        made_with: made_with.to_owned(),
        content: content_of(&records, made_with),
        records,
        warm: Mutex::new(Vec::new()),
    })
}

// ---------------------------------------------------------------------------
// Asking for one
// ---------------------------------------------------------------------------

/// Start `job` on its own thread.
///
/// Returns as soon as the thread is spawned; the cache probe and the model both
/// happen *there*, so no caller - least of all the interface thread - ever waits
/// on the disk or on a graph compile (docs/14 §1.1).
pub fn request(mut job: PlaneJob) -> Requested {
    let instance = job.instance;
    if job.key.is_none() {
        return Requested::Refused(PlaneFailure::Unreadable);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut held) = jobs().lock() else {
            return Requested::Refused(PlaneFailure::Busy);
        };
        if held.running.is_some() {
            return Requested::Refused(PlaneFailure::Busy);
        }
        held.running = Some((instance, Arc::clone(&cancel)));
        held.progress.insert(instance, Progress::Queued);
    }
    // Claimed just above, so this job and no other may give it back.
    job.owns_slot = true;
    let spawned = std::thread::Builder::new()
        .name("lumit-planes".into())
        .spawn(move || run(job, &cancel));
    if spawned.is_err() {
        // The slot was claimed above and nothing would ever release it.
        finish(instance, None, true);
        return Requested::Refused(PlaneFailure::Busy);
    }
    Requested::Started
}

/// Stop `instance`'s analysis. The flag is raised and the run ends **between
/// frames**, keeping and filing every frame it had finished.
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

/// Record a refusal answered without ever spawning a thread, so the status row
/// can say *why* the button did nothing. A press is an event and has nothing to
/// poll against, so the reason is left where the next status read will find it.
pub fn note_refusal(instance: Uuid, failure: PlaneFailure) {
    report(instance, Progress::Failed(failure));
}

fn report(instance: Uuid, step: Progress) {
    if let Ok(mut held) = jobs().lock() {
        held.progress.insert(instance, step);
    }
}

/// Take the job off the running slot and publish its outcome. `None` **forgets**
/// the instance: a warm pass that found nothing has nothing to say about it.
///
/// Only the job that claimed the slot may give it back, and only it may wipe a
/// reading. A project reopened part-way through an analysis warms the very
/// instance being read, out of the same file the ids came from, and a warm pass
/// answering for it used to hand the running job's slot away and delete its
/// progress (§9).
fn finish(instance: Uuid, outcome: Option<Progress>, owns_slot: bool) {
    if let Ok(mut held) = jobs().lock() {
        if owns_slot && held.running.as_ref().is_some_and(|(i, _)| *i == instance) {
            held.running = None;
        }
        match outcome {
            Some(step) => {
                held.progress.insert(instance, step);
            }
            None if owns_slot => {
                held.progress.remove(&instance);
            }
            None => {}
        }
    }
}

/// The whole of one job, on the analysis thread: read the sidecar, and only if
/// there is nothing there, open a model and read the clip.
fn run(job: PlaneJob, cancel: &AtomicBool) {
    let instance = job.instance;
    let owns_slot = job.owns_slot;
    let key = job.key;
    let dir = cache_dir();

    // A warm pass says nothing at all about an instance that is being read
    // right now: the run in flight is the newer answer and publishes its own.
    if !owns_slot
        && jobs()
            .lock()
            .is_ok_and(|held| held.running.as_ref().is_some_and(|(i, _)| *i == instance))
    {
        return;
    }

    // What a cancelled run left behind, when this Analyse is the one that
    // carries on from it.
    let mut resume = None;
    if let Some(hit) = key
        .zip(dir.as_deref())
        .and_then(|(key, d)| read_sidecar(d, key))
    {
        // A whole run answers anybody, and a partial one answers a warm pass.
        // An **Analyse** over a partial run falls through instead, which is the
        // resume a cancelled run is owed.
        let satisfied = match job.stop_after {
            Some(s) => hit.plane(s).is_some(),
            None => !hit.is_partial(),
        };
        if !job.analyse || satisfied {
            publish(instance, hit);
            finish(instance, Some(Progress::Done), owns_slot);
            return;
        }
        // Its frames were read by this model under this key, which is the whole
        // of what makes a plane what it is, so they are offered to the run that
        // carries on from them. What may actually be kept is [`analyse`]'s to
        // say.
        resume = Some(hit);
    }
    if !job.analyse {
        // A warm pass found nothing. That is not a failure and must not look
        // like one: nobody asked for this shot to be read.
        finish(instance, None, owns_slot);
        return;
    }

    // Opened here, on this thread, and dropped at the end of this function,
    // never held between analyses and never behind a shared lock (§6.1).
    let mut model = match open_model(job.settings) {
        Ok(model) => model,
        Err(why) => {
            finish(instance, Some(Progress::Failed(why)), owns_slot);
            return;
        }
    };

    match analyse(job, resume, model.as_mut(), cancel, &|step| {
        report(instance, step);
    }) {
        Ok((run, cancelled)) => {
            // Written before the store is filled, so a run the interface can
            // see is a run the next time the project is opened will find. A
            // **cancelled** run is cached like any other: its frames are
            // correct and correctly named, and re-deriving them would take the
            // same minutes to reach the same place.
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
                owns_slot,
            );
        }
        Err(e) => finish(instance, Some(Progress::Failed(e)), owns_slot),
    }
}

/// Whether a run's frames can be carried on from where a cancelled one
/// stopped.
///
/// Asked of the model rather than of the task, because the two matte models
/// answer it differently. Depth and BiRefNet read each frame on its own, so a
/// kept prefix is exactly what a fresh run would have made of those frames.
/// Robust Video Matting carries its state from frame to frame and the record
/// does not keep it, so a resume there is a fresh run that copies nothing
/// (§13's "Robust Video Matting is a sequence, not a set").
fn resumes(settings: PlaneSettings) -> bool {
    match settings.task {
        PlaneTask::Depth => true,
        PlaneTask::Matte => !matches!(matte_arch(settings.arch), lumit_ml::matte::MatteArch::Rvm),
    }
}

// ---------------------------------------------------------------------------
// The work
// ---------------------------------------------------------------------------

/// Decode, read, record. The whole cost of an analysis.
///
/// Separated from [`run`] so the engine tests can drive it directly with a
/// model they wrote down and a deterministic progress log, rather than racing a
/// thread to observe one. Answers `(the run, whether it was cancelled)`: a
/// cancelled run is an answer, not an error.
///
/// **Cancellation is once per frame** (§9). A single model run of one frame is
/// not interruptible and the note says so rather than implying finer.
///
/// `resume` is what a cancelled run of this same key left behind, and the read
/// starts after the last frame of it: those frames are correct and correctly
/// named, so reading them again would spend the same minutes reaching the same
/// place. `None` reads the clip from its first frame, which is what a model
/// that carries its state between frames always does ([`resumes`]).
pub fn analyse(
    job: PlaneJob,
    resume: Option<PlaneRun>,
    model: &mut dyn PlaneModel,
    cancel: &AtomicBool,
    report: &dyn Fn(Progress),
) -> Result<(PlaneRun, bool), PlaneFailure> {
    let mut frames = (job.open)().ok_or(PlaneFailure::Unreadable)?;
    let (count, width, height, fps) = frames.info();
    if count == 0 || width == 0 || height == 0 {
        return Err(PlaneFailure::NoFrames);
    }
    let last_index = i64::try_from(count.saturating_sub(1)).unwrap_or(i64::MAX);
    let stop = job.stop_after.map(|f| f.clamp(0, last_index));

    let made_with = model.made_with();
    let mut records: Vec<FrameRecord> = resume
        // A model that carries its state from frame to frame cannot be handed
        // the middle of a shot: the record does not keep that state, so a
        // resume there is a fresh run that copies nothing (§13).
        .filter(|_| resumes(job.settings))
        // A prefix that does not start at the first frame is nothing to carry
        // on from either: the run would come back with a hole in it.
        .filter(|run| run.first_frame == 0)
        // And a prefix something else made is not this run's to carry on from:
        // the provenance names one run, and half a run made by a runtime that
        // has been updated since would be filed under the new one's name (§7).
        .filter(|run| run.made_with == made_with)
        .map_or_else(Vec::new, |run| run.records);
    let from = records
        .last()
        .map_or(0, |r| usize::try_from(r.frame + 1).unwrap_or(usize::MAX));
    records.reserve(count.saturating_sub(from));
    let mut cancelled = false;
    for n in from..count {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let Some(rgba) = frames.rgba(n) else {
            // A clip that stops decoding part-way is analysed as far as it
            // went, exactly as a partial track is.
            break;
        };
        let out = model.run(&rgba, width, height)?;
        let at = i64::try_from(n).unwrap_or(i64::MAX);
        records.push(record_of(at, &out));
        report(Progress::Solving {
            done: n + 1,
            total: count,
        });
        if stop == Some(at) {
            break;
        }
    }
    if records.is_empty() {
        return Err(if cancelled {
            PlaneFailure::Cancelled
        } else {
            PlaneFailure::NoFrames
        });
    }

    let first_frame = records.first().map_or(0, |r| r.frame);
    let last_frame = records.last().map_or(0, |r| r.frame);
    Ok((
        PlaneRun {
            fps,
            clip_frames: count,
            first_frame,
            last_frame,
            content: content_of(&records, &made_with),
            made_with,
            records,
            warm: Mutex::new(Vec::new()),
        },
        cancelled,
    ))
}

// ---------------------------------------------------------------------------
// Reading a document
// ---------------------------------------------------------------------------

/// The job one planes-tier instance on one footage layer describes, or `None`
/// when it is not one or there is nothing to key a cache with.
#[must_use]
pub fn job_for(
    fx: &EffectInstance,
    path: PathBuf,
    fingerprint: &Fingerprint,
    analyse: bool,
) -> Option<PlaneJob> {
    let settings = PlaneSettings::of(fx)?;
    Some(PlaneJob {
        instance: fx.id,
        key: Some(PlaneKey::new(fingerprint, settings)),
        settings,
        open: Box::new(move || {
            crate::roto::MediaRgba::open(&path).map(|f| Box::new(f) as Box<dyn RotoFrames>)
        }),
        analyse,
        // Claimed by `request` and by nothing else, so a job made here starts
        // out holding nothing.
        owns_slot: false,
        stop_after: None,
    })
}

/// Every cached run a document could be holding, as warm-pass jobs: one per
/// enabled planes-tier effect on a footage layer whose media has a fingerprint.
#[must_use]
pub fn warm_jobs(doc: &Document) -> Vec<PlaneJob> {
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
            for fx in lumit_core::planes::analyses(&layer.effects) {
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
/// **Not [`request`], deliberately** - `request` owns the one-at-a-time slot, so
/// warming the second effect of a project would answer `Busy` and simply not
/// happen. A warm pass is a file read per instance and opens no model at all.
pub fn warm(jobs: Vec<PlaneJob>) {
    if jobs.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("lumit-planes-warm".into())
        .spawn(move || {
            let never = AtomicBool::new(false);
            for mut job in jobs {
                job.analyse = false;
                job.owns_slot = false;
                run(job, &never);
            }
        });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

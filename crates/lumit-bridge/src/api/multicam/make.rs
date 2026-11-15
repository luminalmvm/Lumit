//! **New multicam comp**: footage made into a multicam comp, its files synced
//! by their sound, their timecode, their In marks or their first markers.
//!
//! # In plain terms
//!
//! Syncing by sound decodes every file's sound and compares them, which takes
//! seconds to minutes, so it is driven the way a proxy is made: `start`, then
//! `poll` on the interface's own tick, then `cancel` if the wait is not worth
//! it. The files are read and compared on a thread of their own. When they
//! have been placed, the next poll builds the comp and commits it as one undo
//! step, on the UI thread where every edit is made (docs/14 §1).
//!
//! One at a time, for the whole process.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use lumit_core::model::{FootageItem, Layer, LayerKind, ProjectItem};
use lumit_core::multicam::Multicam;
use lumit_core::ops::Op;
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use uuid::Uuid;

use crate::api::{project::ProjectReference, BridgeError};

/// What a multicam comp's files are lined up by.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMulticamSync {
    Sound,
    Timecode,
    /// Each file's In mark is the same moment.
    InMarks,
    /// Each file's Out mark is the same moment.
    OutMarks,
    /// Each file's first marker is the same moment.
    Markers,
    /// Each angle's first file starts with the comp, its others following.
    Starts,
}

/// What makes an angle.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMulticamAngles {
    /// One angle to a file.
    EachFile,
    /// One angle to a folder on disk, as a camera's card is, its files one
    /// after another on the row.
    EachFolder,
}

/// How a multicam comp is made.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMulticamNew {
    /// Blank takes the next free comp name.
    pub name: String,
    pub sync: BridgeMulticamSync,
    pub angles: BridgeMulticamAngles,
    /// Whether a sound angle that runs fast or slow against the cameras is
    /// played at the speed that keeps it in step. Sound sync only.
    pub drift: bool,
}

/// How the job is getting on.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeMulticamJob {
    Idle,
    /// Nought to one.
    Working {
        done: f64,
    },
    /// The comp is made. `unsynced` names the files that had nothing to line
    /// up by and went in after the last clip on their row; `uncertain` those
    /// placed on weak evidence.
    Done {
        comp: Uuid,
        synced: u32,
        uncertain: Vec<String>,
        unsynced: Vec<String>,
    },
    Failed,
}

/// One file as the job reads it.
#[frb(ignore)]
#[derive(Clone)]
struct File {
    item: Uuid,
    name: String,
    /// The folder on disk the file is in, for a camera's card.
    folder: PathBuf,
    footage: FootageItem,
    src: Option<lumit_media::sequence::MediaSource>,
    in_mark: Option<Rational>,
    out_mark: Option<Rational>,
    marker: Option<Rational>,
    /// The device it came from: files of one group are never compared with
    /// each other and never overlap.
    group: u32,
}

/// What the thread found of one file.
#[frb(ignore)]
#[derive(Clone, Debug)]
struct Found {
    /// The file's length, seconds.
    length: Rational,
    /// Its picture's size and rate, where it has one.
    picture: Option<(u32, u32, u32, u32)>,
    /// Where its own time 0 sits on the shared clock, where it could be lined
    /// up, and whether that was on weak evidence.
    start: Option<Rational>,
    uncertain: bool,
    /// How much faster than the others its sound runs.
    speed: f64,
}

/// What the job is for.
#[frb(ignore)]
enum Kind {
    Make(BridgeMulticamNew),
    /// One angle's files synced again by their sound against another's,
    /// inside multicam comp `comp`: `layer`'s files come first in the job's
    /// list, `count` of them, then `to`'s.
    Resync {
        comp: Uuid,
        layer: Uuid,
        to: Uuid,
        count: usize,
    },
}

#[frb(ignore)]
struct Job {
    project: Uuid,
    kind: Kind,
    files: Vec<File>,
    cancel: Arc<AtomicBool>,
    /// Nought to one, as an f32's bits.
    progress: Arc<AtomicU32>,
    found: Arc<Mutex<Option<Option<Vec<Found>>>>>,
}

#[frb(ignore)]
static JOB: Mutex<Option<Job>> = Mutex::new(None);

/// Whether the project a job is for has closed.
#[frb(ignore)]
fn orphaned(job: &Job) -> bool {
    matches!(
        ProjectReference::new(job.project).state(),
        Err(BridgeError::InvalidProject)
    )
}

impl ProjectReference {
    /// Start making a multicam comp of the footage `items`, in that order.
    /// Refused while another is being made, and with no footage among them.
    #[frb(sync)]
    pub fn multicam_new(
        &self,
        items: Vec<Uuid>,
        settings: BridgeMulticamNew,
    ) -> Result<(), BridgeError> {
        let mut files = self.files_of(&items)?;
        // A device is a file of its own, or a folder on disk, as a camera's
        // card is.
        for n in 0..files.len() {
            let group = match settings.angles {
                BridgeMulticamAngles::EachFile => n,
                BridgeMulticamAngles::EachFolder => files
                    .iter()
                    .position(|f| f.folder == files[n].folder)
                    .unwrap_or(n),
            };
            files[n].group = u32::try_from(group).unwrap_or(u32::MAX);
        }
        let (sync, drift) = (settings.sync, settings.drift);
        self.start_job(Kind::Make(settings), files, sync, drift)
    }

    /// The footage `items` as the job reads them, in that order, each its
    /// own group to begin with. Refused with no footage among them, and when
    /// none of it can be read, as in a build that reads no media.
    #[frb(ignore)]
    fn files_of(&self, items: &[Uuid]) -> Result<Vec<File>, BridgeError> {
        let state = self.state()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let mut files = Vec::new();
        for id in items {
            let Some(ProjectItem::Footage(f)) = doc.item(*id) else {
                continue;
            };
            let path = crate::api::footage::FootageReference::resolve_path(&state, f);
            let marker = doc
                .footage_markers
                .get(id)
                .and_then(|m| m.iter().map(|m| m.time.0).min());
            #[cfg(feature = "media")]
            let src = crate::api::footage::FootageReference::resolve_source(&state, f);
            #[cfg(not(feature = "media"))]
            let src = None;
            files.push(File {
                item: *id,
                name: f.name.clone(),
                folder: path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(PathBuf::from)
                    .unwrap_or_default(),
                footage: f.clone(),
                src,
                in_mark: doc.footage_trims.get(id).and_then(|t| t.in_point),
                out_mark: doc.footage_trims.get(id).and_then(|t| t.out_point),
                marker,
                group: u32::try_from(files.len()).unwrap_or(u32::MAX),
            });
        }
        if files.is_empty() {
            return Err(BridgeError::InvalidItem);
        }
        if files.iter().all(|f| f.src.is_none()) {
            return Err(BridgeError::MediaPathUnresolved);
        }
        Ok(files)
    }

    /// Start the job's thread. Refused while another job is running.
    #[frb(ignore)]
    fn start_job(
        &self,
        kind: Kind,
        files: Vec<File>,
        sync: BridgeMulticamSync,
        drift: bool,
    ) -> Result<(), BridgeError> {
        let mut guard = JOB.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(job) = guard.as_ref() {
            // A job whose project has closed has no one to land for.
            if !orphaned(job) {
                return Err(BridgeError::InvalidWorkerState);
            }
            job.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU32::new(0));
        let found = Arc::new(Mutex::new(None));
        *guard = Some(Job {
            project: self.id,
            kind,
            files: files.clone(),
            cancel: cancel.clone(),
            progress: progress.clone(),
            found: found.clone(),
        });
        drop(guard);
        std::thread::Builder::new()
            .name("multicam".into())
            .spawn(move || {
                let answer = find(&files, sync, drift, &cancel, &progress);
                *found.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
            })
            .map(|_| ())
            .map_err(|_| {
                // No thread, so nothing will ever land: free the slot.
                *JOB.lock().unwrap_or_else(|p| p.into_inner()) = None;
                BridgeError::InvalidWorkerState
            })
    }

    /// How the multicam comp being made is getting on. Once its files are
    /// placed, this makes the comp, one undo step, and says so.
    #[frb(sync)]
    pub fn multicam_poll(&self) -> Result<BridgeMulticamJob, BridgeError> {
        let mut guard = JOB.lock().unwrap_or_else(|p| p.into_inner());
        let Some(job) = guard.as_ref() else {
            return Ok(BridgeMulticamJob::Idle);
        };
        if job.project != self.id {
            // Another project's job is left alone, unless that project has
            // closed, when the slot is freed.
            if orphaned(job) {
                if let Some(job) = guard.take() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
            return Ok(BridgeMulticamJob::Idle);
        }
        let landed = job.found.lock().unwrap_or_else(|p| p.into_inner()).take();
        let Some(landed) = landed else {
            let done = f32::from_bits(job.progress.load(Ordering::Relaxed));
            return Ok(BridgeMulticamJob::Working {
                done: f64::from(done),
            });
        };
        let Some(job) = guard.take() else {
            return Ok(BridgeMulticamJob::Idle);
        };
        drop(guard);
        let Some(found) = landed else {
            return Ok(BridgeMulticamJob::Failed);
        };
        match &job.kind {
            Kind::Make(settings) => self.commit_multicam(&job, settings, &found),
            Kind::Resync {
                comp,
                layer,
                to,
                count,
            } => self.commit_resync(&job.files, &found, *comp, *layer, *to, *count),
        }
    }

    /// Stop making the multicam comp. Nothing is made.
    #[frb(sync)]
    pub fn multicam_cancel(&self) {
        // Whichever project asks: there is one job for the whole process, and
        // the interface may have moved to another project since it started.
        if let Some(job) = JOB.lock().unwrap_or_else(|p| p.into_inner()).take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Build the comp the thread's findings describe, and commit it.
    #[frb(ignore)]
    fn commit_multicam(
        &self,
        job: &Job,
        settings: &BridgeMulticamNew,
        found: &[Found],
    ) -> Result<BridgeMulticamJob, BridgeError> {
        let state = self.state()?;
        let state = state.write().map_err(|_| BridgeError::WriteFailed)?;
        let doc = state.store.snapshot();
        let starts = settings.sync == BridgeMulticamSync::Starts;
        let plan = plan(&job.files, found, settings.angles, starts);
        let (comp_id, mut ops) = lumit_core::edits::new_comp_ops(
            &doc,
            lumit_core::edits::CompSettings {
                name: settings.name.trim().to_owned(),
                width: plan.width,
                height: plan.height,
                frame_rate: plan.rate,
                duration: Duration(plan.length),
                background: [0.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
            None,
        );
        let mut layers: Vec<Layer> = Vec::new();
        for (n, row) in plan.rows.iter().enumerate() {
            let transform = match row.size {
                Some((w, h)) => fitted(w, h, plan.width, plan.height),
                None => lumit_core::edits::centred_transform(
                    f64::from(plan.width),
                    f64::from(plan.height),
                    plan.width,
                    plan.height,
                ),
            };
            let mut layer = lumit_core::edits::base_layer(
                row.name.clone(),
                LayerKind::Sequence {
                    clips: row.clips.clone(),
                },
                plan.length,
                transform,
            );
            layer.audio_only = row.size.is_none();
            // Each angle wears a colour of its own, so the cuts that use it
            // can be told apart at a glance. The neutral first chip is
            // skipped, as a person's colour in a shared project skips it.
            layer.label = 1 + (n % 8) as u8;
            if let Some((start, end)) = lumit_core::sequence::clips_span(&row.clips) {
                layer.in_point = CompTime(start);
                layer.out_point = CompTime(end);
            }
            layers.push(layer);
        }
        // A recorder of its own is what a new clip sounds, as the reason it
        // was rolling.
        let sound = layers.iter().find(|l| l.audio_only).map(|l| l.id);
        for op in &mut ops {
            if let Op::AddItem { item, .. } = op {
                if let ProjectItem::Composition(c) = item.as_mut() {
                    if c.id == comp_id {
                        c.layers = std::mem::take(&mut layers);
                        c.set_multicam(Some(&Multicam {
                            sound,
                            ..Multicam::default()
                        }));
                    }
                }
            }
        }
        state
            .store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(BridgeMulticamJob::Done {
            comp: comp_id,
            synced: plan.synced,
            uncertain: plan.uncertain,
            unsynced: plan.unsynced,
        })
    }
}

/// A picture `w` by `h` fitted into a `cw` by `ch` frame and centred, kept to
/// its shape.
#[frb(ignore)]
fn fitted(w: u32, h: u32, cw: u32, ch: u32) -> lumit_core::model::TransformGroup {
    let mut tr = lumit_core::edits::centred_transform(f64::from(w), f64::from(h), cw, ch);
    if (w, h) != (cw, ch) && w > 0 && h > 0 {
        let k = (f64::from(cw) / f64::from(w)).min(f64::from(ch) / f64::from(h)) * 100.0;
        tr.scale_x = lumit_core::anim::Property::fixed(k);
        tr.scale_y = lumit_core::anim::Property::fixed(k);
    }
    tr
}

/// Read every file and line them up, on the job's thread. `None` when it was
/// cancelled.
#[frb(ignore)]
fn find(
    files: &[File],
    sync: BridgeMulticamSync,
    drift: bool,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Option<Vec<Found>> {
    // Told from several decoding threads at once.
    let progress: &AtomicU32 = progress;
    let tell = |done: f32| progress.store(done.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    let mut found: Vec<Found> = Vec::with_capacity(files.len());
    for (n, file) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        found.push(probe(file, sync));
        tell(0.05 * (n + 1) as f32 / files.len() as f32);
    }
    if sync == BridgeMulticamSync::Sound {
        sound::sync(files, &mut found, drift, cancel, &tell)?;
    }
    tell(1.0);
    Some(found)
}

/// One file's length and picture, and where its own time 0 sits when it is
/// lined up by something it carries.
#[frb(ignore)]
fn probe(file: &File, sync: BridgeMulticamSync) -> Found {
    let mut out = Found {
        length: Rational::ZERO,
        picture: None,
        start: None,
        uncertain: false,
        speed: 1.0,
    };
    let Some(src) = &file.src else {
        return out;
    };
    let Some(info) = probed(src) else {
        return out;
    };
    out.length = Rational::from_f64_on_grid(info.duration_seconds.max(0.0), 48_000)
        .unwrap_or(Rational::ZERO);
    out.picture = info.video.as_ref().and_then(|v| {
        let num = u32::try_from(v.fps_num).ok()?;
        let den = u32::try_from(v.fps_den).ok()?;
        (v.width > 0 && v.height > 0).then_some((v.width, v.height, num, den))
    });
    let back = |t: Rational| Rational::ZERO.checked_sub(t).ok();
    // A file's timecode counts its own frames; a sound file has none to count.
    let timecode = || {
        let frame = start_frame(file, src)?;
        let (num, den) = out.picture.map(|(_, _, n, d)| (n, d))?;
        Rational::from_i128(i128::from(frame) * i128::from(den), i128::from(num.max(1))).ok()
    };
    out.start = match sync {
        BridgeMulticamSync::Sound | BridgeMulticamSync::Starts => None,
        BridgeMulticamSync::Timecode => timecode(),
        BridgeMulticamSync::InMarks => file.in_mark.and_then(back),
        BridgeMulticamSync::OutMarks => file.out_mark.and_then(back),
        BridgeMulticamSync::Markers => file.marker.and_then(back),
    };
    out
}

/// What a file's probe found. Nothing in a build that reads no media.
#[frb(ignore)]
fn probed(src: &lumit_media::sequence::MediaSource) -> Option<Arc<lumit_media::probe::MediaProbe>> {
    #[cfg(feature = "media")]
    {
        crate::probe::ensure_probed(src.clone())
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = src;
        None
    }
}

/// The frame a file's timecode starts on, counted in its own frames. Nothing
/// in a build that reads no media.
#[frb(ignore)]
fn start_frame(file: &File, src: &lumit_media::sequence::MediaSource) -> Option<u64> {
    #[cfg(feature = "media")]
    {
        crate::api::interpret::start_timecode(&file.footage, src, None).map(|(frame, _)| frame)
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = (&file.footage, src);
        None
    }
}

/// The comp to make: its frame, its length, and its rows.
#[frb(ignore)]
struct Plan {
    width: u32,
    height: u32,
    rate: FrameRate,
    length: Rational,
    rows: Vec<Row>,
    synced: u32,
    uncertain: Vec<String>,
    unsynced: Vec<String>,
}

/// One angle to make: its name, its picture's size (none for a sound angle)
/// and its clips.
#[frb(ignore)]
struct Row {
    name: String,
    size: Option<(u32, u32)>,
    clips: Vec<Clip>,
}

/// Lay the files out as rows: one a file, or one a folder with its files one
/// after another and a second row where two of them overlap. Picture rows
/// first, then sound rows, each in the order the files came. A file that
/// could not be lined up goes after the last clip on its row, as every file
/// does by `starts`, which counts them lined up.
#[frb(ignore)]
fn plan(files: &[File], found: &[Found], angles: BridgeMulticamAngles, starts: bool) -> Plan {
    // The frame and rate most of the cameras have, each counted once by its
    // first file with a picture, or the first camera's on a tie.
    let mut cameras: Vec<u32> = Vec::new();
    let pictures: Vec<(u32, u32, u32, u32)> = files
        .iter()
        .zip(found)
        .filter_map(|(file, f)| {
            let picture = f.picture?;
            if cameras.contains(&file.group) {
                return None;
            }
            cameras.push(file.group);
            Some(picture)
        })
        .collect();
    let most = |pick: &dyn Fn(&(u32, u32, u32, u32)) -> (u32, u32)| {
        let mut best: Option<((u32, u32), usize)> = None;
        for p in &pictures {
            let key = pick(p);
            let n = pictures.iter().filter(|q| pick(q) == key).count();
            if best.is_none_or(|(_, m)| n > m) {
                best = Some((key, n));
            }
        }
        best.map(|(k, _)| k)
    };
    let (width, height) = most(&|p| (p.0, p.1)).unwrap_or((1920, 1080));
    let rate = most(&|p| (p.2, p.3))
        .and_then(|(n, d)| FrameRate::new(n, d).ok())
        .unwrap_or(FrameRate::FPS_25);
    // A camera starts on a frame of the comp, so its frames meet the comp's
    // whole and not a hair either side. A sound file keeps its sample.
    let on_frame = |t: Rational| {
        let fps = f64::from(rate.num()) / f64::from(rate.den().max(1));
        let n = (t.to_f64() * fps).round() as i64;
        rate.time_of_frame(n).map_or(t, |c| c.0)
    };
    // The shared clock from the earliest file lined up.
    let earliest = found
        .iter()
        .filter_map(|f| f.start)
        .min()
        .unwrap_or(Rational::ZERO);
    let mut rows: Vec<Row> = Vec::new();
    // Which group each row is for, so a folder's files find their row.
    let mut keys: Vec<(usize, Option<usize>)> = Vec::new();
    let (mut synced, mut uncertain, mut unsynced) = (0, Vec::new(), Vec::new());
    let mut order: Vec<usize> = (0..files.len()).collect();
    // Picture first, then by where each starts, the unplaced last.
    order.sort_by_key(|i| {
        let f = &found[*i];
        (
            f.picture.is_none(),
            f.start.is_none(),
            f.start.unwrap_or(Rational::ZERO),
            *i,
        )
    });
    for i in order {
        let (file, f) = (&files[i], &found[i]);
        if f.length <= Rational::ZERO {
            unsynced.push(file.name.clone());
            continue;
        }
        let group = file.group as usize;
        let at = f.start.and_then(|s| s.checked_sub(earliest).ok());
        let at = if f.picture.is_some() {
            at.map(on_frame)
        } else {
            at
        };
        match (at, f.uncertain) {
            (Some(_), false) => synced += 1,
            (Some(_), true) => uncertain.push(file.name.clone()),
            (None, _) if starts => synced += 1,
            (None, _) => unsynced.push(file.name.clone()),
        }
        let length = match f.speed {
            s if (s - 1.0).abs() > 1e-9 && s > 0.0 => {
                Rational::from_f64_on_grid(f.length.to_f64() / s, Rational::FLICK_DEN)
                    .unwrap_or(f.length)
            }
            _ => f.length,
        };
        let size = f.picture.map(|(w, h, _, _)| (w, h));
        // The row for this file: its group's first row with room for it.
        let fits = |row: &Row, start: Rational| {
            let end = start.checked_add(length).unwrap_or(start);
            !row.clips
                .iter()
                .any(|c| c.place_start < end && start < c.place_end())
        };
        let free_at = |row: &Row| {
            row.clips
                .iter()
                .map(Clip::place_end)
                .max()
                .unwrap_or(Rational::ZERO)
        };
        let mut chosen = None;
        for (r, (g, _)) in keys.iter().enumerate() {
            let same_kind = rows[r].size.is_some() == size.is_some();
            if *g != group || !same_kind {
                continue;
            }
            let start = at.unwrap_or_else(|| free_at(&rows[r]));
            if fits(&rows[r], start) {
                chosen = Some((r, start));
                break;
            }
        }
        let (r, start) = match chosen {
            Some(found) => found,
            None => {
                let name = match angles {
                    BridgeMulticamAngles::EachFile => file.name.clone(),
                    BridgeMulticamAngles::EachFolder => {
                        let base = file
                            .folder
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| file.name.clone());
                        let taken = keys.iter().filter(|(g, _)| *g == group).count();
                        if taken == 0 {
                            base
                        } else {
                            format!("{base} {}", taken + 1)
                        }
                    }
                };
                rows.push(Row {
                    name,
                    size,
                    clips: Vec::new(),
                });
                keys.push((group, Some(i)));
                (rows.len() - 1, at.unwrap_or(Rational::ZERO))
            }
        };
        let mut clip = Clip::new(
            ClipSource::Footage(file.item),
            Rational::ZERO,
            f.length,
            start,
            f.length,
        );
        if length != f.length {
            let end = start.checked_add(length).unwrap_or(start);
            clip = clip.stretched(start, end).unwrap_or(clip);
        }
        rows[r].clips.push(clip);
        if rows[r].size.is_none() {
            rows[r].size = size;
        }
    }
    // Picture rows above sound rows, each kind in the order its first file
    // was given.
    let mut ordered: Vec<(Row, usize)> = rows
        .into_iter()
        .zip(keys.iter().map(|(_, first)| first.unwrap_or(usize::MAX)))
        .collect();
    ordered.sort_by_key(|(row, first)| (row.size.is_none(), *first));
    let rows: Vec<Row> = ordered.into_iter().map(|(row, _)| row).collect();
    let length = rows
        .iter()
        .flat_map(|r| r.clips.iter().map(Clip::place_end))
        .max()
        .unwrap_or(Rational::ONE);
    let length = whole_frames(rate, length);
    Plan {
        width,
        height,
        rate,
        length,
        rows,
        synced,
        uncertain,
        unsynced,
    }
}

/// `length` taken on to the next whole frame at `rate`, so a comp ends on one.
#[frb(ignore)]
fn whole_frames(rate: FrameRate, length: Rational) -> Rational {
    let frames = rate.frame_at(CompTime(length));
    let short = rate.time_of_frame(frames).is_ok_and(|t| t.0 < length);
    rate.time_of_frame(frames + i64::from(short))
        .map_or(length, |t| t.0)
}

impl crate::api::composition::CompositionReference {
    /// **Sync to angle**: line `layer`'s files up again by their sound
    /// against `to`'s, inside this multicam comp. It runs as the job
    /// [`ProjectReference::multicam_poll`] reports on, and moves `layer`'s
    /// clips in one undo step when it lands.
    #[frb(sync)]
    pub fn multicam_sync_angle(&self, layer: Uuid, to: Uuid) -> Result<(), BridgeError> {
        if layer == to {
            return Err(BridgeError::InvalidLayer);
        }
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let items_of = |id: Uuid| -> Vec<Uuid> {
            let mut items = Vec::new();
            let row = comp.layers.iter().find(|l| l.id == id);
            if let Some(LayerKind::Sequence { clips }) = row.map(|l| &l.kind) {
                for clip in clips {
                    if let ClipSource::Footage(item) = clip.source {
                        if !items.contains(&item) {
                            items.push(item);
                        }
                    }
                }
            }
            items
        };
        let ours = items_of(layer);
        let theirs: Vec<Uuid> = items_of(to)
            .into_iter()
            .filter(|i| !ours.contains(i))
            .collect();
        if ours.is_empty() || theirs.is_empty() {
            return Err(BridgeError::InvalidLayer);
        }
        let project = ProjectReference::new(self.project);
        let all: Vec<Uuid> = ours.iter().chain(&theirs).copied().collect();
        let mut files = project.files_of(&all)?;
        let count = files.iter().filter(|f| ours.contains(&f.item)).count();
        for f in &mut files {
            f.group = u32::from(!ours.contains(&f.item));
        }
        project.start_job(
            Kind::Resync {
                comp: self.id,
                layer,
                to,
                count,
            },
            files,
            BridgeMulticamSync::Sound,
            false,
        )
    }
}

impl ProjectReference {
    /// Move `layer`'s clips where the sync put their files, against where
    /// `to`'s files already are, and commit it.
    #[frb(ignore)]
    fn commit_resync(
        &self,
        files: &[File],
        found: &[Found],
        comp: Uuid,
        layer: Uuid,
        to: Uuid,
        count: usize,
    ) -> Result<BridgeMulticamJob, BridgeError> {
        let state = self.state()?;
        let doc = state
            .read()
            .map_err(|_| BridgeError::ReadFailed)?
            .store
            .snapshot();
        let mc = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
        let row = |id: Uuid| mc.layers.iter().find(|l| l.id == id);
        let (Some(ours), Some(theirs)) = (row(layer), row(to)) else {
            return Ok(BridgeMulticamJob::Failed);
        };
        let (LayerKind::Sequence { clips: our_clips }, LayerKind::Sequence { clips: their_clips }) =
            (&ours.kind, &theirs.kind)
        else {
            return Ok(BridgeMulticamJob::Failed);
        };
        // Where a file's own time 0 sits in the comp, by its first clip on a
        // row.
        let zero = |clips: &[Clip], offset: Rational, item: Uuid| -> Option<Rational> {
            let c = clips
                .iter()
                .find(|c| c.source == ClipSource::Footage(item))?;
            c.place_start
                .checked_add(offset)
                .ok()?
                .checked_sub(c.source_in)
                .ok()
        };
        let (mine, refs) = (
            files.split_at(count.min(files.len())),
            found.split_at(count.min(found.len())),
        );
        // The shared clock against the comp's own, from where the other
        // angle's files already are: the middle of what they say.
        let mut shifts: Vec<Rational> = mine
            .1
            .iter()
            .zip(refs.1)
            .filter_map(|(f, d)| {
                zero(their_clips, theirs.start_offset.0, f.item)?
                    .checked_sub(d.start?)
                    .ok()
            })
            .collect();
        shifts.sort();
        let Some(&shift) = shifts.get(shifts.len() / 2) else {
            return Ok(BridgeMulticamJob::Done {
                comp,
                synced: 0,
                uncertain: Vec::new(),
                unsynced: mine.0.iter().map(|f| f.name.clone()).collect(),
            });
        };
        let mut clips = our_clips.clone();
        let (mut synced, mut uncertain, mut unsynced) = (0, Vec::new(), Vec::new());
        for (f, d) in mine.0.iter().zip(refs.0) {
            let now = zero(our_clips, ours.start_offset.0, f.item);
            let by = d
                .start
                .zip(now)
                .and_then(|(start, now)| start.checked_add(shift).ok()?.checked_sub(now).ok());
            let own = ClipSource::Footage(f.item);
            // The file's clips where the sync puts them. A file that would
            // go before the row's own zero, or onto another file's clip,
            // stays where it was and is not synced.
            let moved: Option<Vec<Clip>> = by.and_then(|by| {
                clips
                    .iter()
                    .filter(|c| c.source == own)
                    .map(|c| {
                        let mut c = c.clone();
                        c.place_start = c.place_start.checked_add(by).ok()?;
                        (!c.place_start.is_negative()).then_some(c)
                    })
                    .collect()
            });
            let clear = |m: &Clip| {
                !clips.iter().any(|o| {
                    o.source != own
                        && o.place_start < m.place_end()
                        && m.place_start < o.place_end()
                })
            };
            let Some(moved) = moved.filter(|m| m.iter().all(clear)) else {
                unsynced.push(f.name.clone());
                continue;
            };
            clips.retain(|c| c.source != own);
            clips.extend(moved);
            if d.uncertain {
                uncertain.push(f.name.clone());
            } else {
                synced += 1;
            }
        }
        clips.sort_by_key(|c| c.place_start);
        if clips != *our_clips {
            // The row's span follows its clips and the comp grows to hold
            // them, as every other clip edit does, in one undo step.
            let end = lumit_core::sequence::clips_span(&clips)
                .and_then(|(_, end)| end.checked_add(ours.start_offset.0).ok());
            let layer = crate::api::layer::LayerReference::new(self.id, comp, layer);
            let mut ops = layer.clip_ops(clips, ours.start_offset)?;
            if let Some(end) = end.filter(|end| *end > mc.duration.0) {
                ops.push(Op::SetCompSettings {
                    comp,
                    name: mc.name.clone(),
                    width: mc.width,
                    height: mc.height,
                    frame_rate: mc.frame_rate,
                    duration: Duration(whole_frames(mc.frame_rate, end)),
                    background: mc.background,
                });
            }
            state
                .write()
                .map_err(|_| BridgeError::WriteFailed)?
                .store
                .commit(Op::Batch { ops })
                .map_err(BridgeError::OpError)?;
        }
        Ok(BridgeMulticamJob::Done {
            comp,
            synced,
            uncertain,
            unsynced,
        })
    }
}

mod sound;

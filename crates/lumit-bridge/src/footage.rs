//! Footage in a shared project, for whoever has not got a file the others
//! are cutting with.
//!
//! # In plain terms
//!
//! Nobody sends anyone a whole recording unless they ask for it. What goes
//! by itself is a **stand-in**: the whole clip at its own size and length,
//! with its sound, squeezed small. A machine that lacks a file reads the
//! stand-in as if it were the file, so the project looks the same to
//! everyone and only the picture is softer.
//!
//! An export wants better than that. It can fetch the originals of what it
//! lacks, or fetch only the frames it reads at delivery quality, or ask the
//! machine that has the originals to do the export and send the result back.
//!
//! `lumit-share` carries the files. This is everything either side of that:
//! which files this machine has, which it wants, making what it is asked
//! for, and putting what arrives where the project reads it from.
//!
//! Threads: one of its own for each shared project, which is where anything
//! that touches the document or the list of shared projects happens. The
//! share threads call in to have a file made, which is an encode and runs on
//! theirs, and to say what happened, which is handed straight to that one.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use lumit_core::model::{FootageItem, ProjectItem};
use lumit_core::{Document, DocumentStore};
use lumit_share::{Footage, Held, Limits, News, Wanted};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::export::BridgeExportSpec;

/// How fast this machine sends and takes footage. One for the machine, kept
/// to across every shared project.
pub(crate) static LIMITS: LazyLock<Arc<Limits>> = LazyLock::new(|| Arc::new(Limits::default()));

/// Whether this machine makes stand-ins of its footage for the others, and
/// whether it asks for stand-ins of what it lacks without being told to.
static GIVE: AtomicBool = AtomicBool::new(true);
static TAKE: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_sharing(give: bool, take: bool) {
    GIVE.store(give, Ordering::Relaxed);
    TAKE.store(take, Ordering::Relaxed);
}

/// The carrier of each shared project, by project. An entry goes when
/// sharing stops or the project closes.
static CARRIERS: LazyLock<Mutex<BTreeMap<Uuid, Arc<Carrier>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(crate) fn carrier(project: Uuid) -> Option<Arc<Carrier>> {
    CARRIERS.lock().ok()?.get(&project).cloned()
}

/// What tells the interface something about footage changed.
pub(crate) type Tell = Arc<dyn Fn(Told) + Send + Sync>;

/// What the interface is told. Everything else it reads when it is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Told {
    /// A file arrived, a transfer moved, or who has what changed.
    Changed,
    /// A file this machine now reads a footage item from arrived, so what
    /// is on screen of it is stale.
    Placed,
    /// Someone asks this machine to export for them.
    Asked { job: Uuid, from: u32, comp: String },
}

/// Where a footage item is on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Here {
    Missing,
    /// A stand-in someone sent.
    StandIn(PathBuf),
    /// The file itself, the person's own or one fetched.
    Original(PathBuf),
}

/// The folder everything sent is kept under: `root` when a carrier was
/// given one of its own, and otherwise the machine's.
fn kept_under(root: Option<&Path>) -> Option<PathBuf> {
    root.map(Path::to_path_buf)
        .or_else(lumit_project::shared_footage_dir)
}

/// The folder files sent for `footage` are kept in. Named by the file's
/// fingerprint, so a file used by two projects is fetched once.
fn folder(root: Option<&Path>, footage: &FootageItem) -> Option<PathBuf> {
    // The fingerprint comes with the item from another machine, and this is
    // a folder name made of it. Only what a hash is made of is let through,
    // so nothing in it can name a place outside the folder.
    let hex = |print: &&lumit_core::model::Fingerprint| {
        !print.head_tail_hash.is_empty()
            && print.head_tail_hash.bytes().all(|b| b.is_ascii_hexdigit())
    };
    let key = match footage.media.fingerprint.as_ref().filter(hex) {
        Some(print) => {
            let hash: String = print.head_tail_hash.chars().take(32).collect();
            format!("{hash}-{}", print.size)
        }
        None => footage.id.to_string(),
    };
    Some(kept_under(root)?.join(key))
}

/// The item's file name with nothing of a path about it.
fn file_name(footage: &FootageItem) -> String {
    let name = footage.media.relative_path.rsplit(['/', '\\', ':']).next();
    let name = name.filter(|name| !name.is_empty() && *name != "." && *name != "..");
    name.unwrap_or("footage").to_owned()
}

/// Whether a file of this name is one a stand-in is the file itself for: a
/// still, a layered picture or sound, which are small or have no frames to
/// squeeze.
fn sent_whole(name: &str) -> bool {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "tif"
            | "tiff"
            | "exr"
            | "bmp"
            | "webp"
            | "tga"
            | "dpx"
            | "hdr"
            | "psd"
            | "ai"
            | "svg"
            | "wav"
            | "mp3"
            | "m4a"
            | "aac"
            | "flac"
            | "ogg"
            | "aif"
            | "aiff"
            | "opus"
    )
}

fn stand_in_path(root: Option<&Path>, footage: &FootageItem) -> Option<PathBuf> {
    let name = file_name(footage);
    let ext = if sent_whole(&name) {
        name.rsplit('.')
            .next()
            .unwrap_or("bin")
            .to_ascii_lowercase()
    } else {
        "mp4".to_owned()
    };
    Some(folder(root, footage)?.join(format!("standin.{ext}")))
}

fn parts_path(root: Option<&Path>, footage: &FootageItem, runs: &[(u64, u64)]) -> Option<PathBuf> {
    let (first, count) = runs.first()?;
    Some(folder(root, footage)?.join(format!("parts-{first}-{count}.mp4")))
}

fn original_path(root: Option<&Path>, footage: &FootageItem) -> Option<PathBuf> {
    Some(
        folder(root, footage)?
            .join("original")
            .join(file_name(footage)),
    )
}

/// The name of the folder sent files are kept under, which is how a path
/// is known for one of them.
const SENT: &str = "shared-footage";

pub(crate) fn here(footage: &FootageItem) -> Here {
    let path = PathBuf::from(&footage.media.absolute_path);
    if path.as_os_str().is_empty() || !path.is_file() {
        return Here::Missing;
    }
    // A stand-in sits in its file's own folder under the sent folder, and
    // an original that was fetched one folder further down.
    let above = |levels: usize| path.ancestors().nth(levels).and_then(Path::file_name);
    if above(2) == Some(SENT.as_ref()) {
        Here::StandIn(path)
    } else {
        Here::Original(path)
    }
}

fn footage(doc: &Document, item: Uuid) -> Option<&FootageItem> {
    match doc.item(item)? {
        ProjectItem::Footage(footage) => Some(footage),
        _ => None,
    }
}

/// Point what this machine lacks at what it was sent before, in a document
/// that is being opened. A fetched original before a stand-in.
pub(crate) fn restore(doc: &mut Document) {
    for item in &mut doc.items {
        let ProjectItem::Footage(footage) = item else {
            continue;
        };
        if here(footage) != Here::Missing {
            continue;
        }
        let sent = [original_path(None, footage), stand_in_path(None, footage)];
        if let Some(path) = sent.into_iter().flatten().find(|path| path.is_file()) {
            footage.media.absolute_path = path.to_string_lossy().into_owned();
        }
    }
}

/// The footage items of `comp` this machine has no original of.
pub(crate) fn lacking(doc: &Document, comp: Uuid) -> Vec<Uuid> {
    let Some(comp) = doc.comp(comp) else {
        return Vec::new();
    };
    let used = lumit_core::model::comp_footage_items(doc, comp);
    let lacks =
        |id: &Uuid| footage(doc, *id).is_some_and(|f| !matches!(here(f), Here::Original(_)));
    used.into_iter().filter(lacks).collect()
}

/// Whether `path` is a moving picture, which a stand-in is an encode of. A
/// still, a layered picture or sound alone is sent as it is.
#[cfg(feature = "media")]
fn moves(footage: &FootageItem, path: &Path) -> bool {
    if footage.sequence.is_some() || sent_whole(&file_name(footage)) {
        return false;
    }
    let source = lumit_media::MediaSource::file(path.to_path_buf());
    let pictured = lumit_media::probe::probe(&source).is_ok_and(|probe| probe.video.is_some());
    pictured
        && lumit_render::media_index::load_or_build_index(&source)
            .is_ok_and(|index| index.frame_count() > 1)
}

/// Encode a stand-in of `source` at `dest`, to one side first so a file at
/// `dest` is always a whole one.
#[cfg(feature = "media")]
fn encode(source: &Path, dest: &Path, parts: Option<&[(usize, usize)]>, stop: &AtomicBool) -> bool {
    let Some(folder) = dest.parent() else {
        return false;
    };
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let making = folder.join(format!("making-{name}"));
    let made = std::fs::create_dir_all(folder).is_ok()
        && lumit_render::standin::transcode(source, &making, parts, stop, &mut |_, _| {}).is_ok()
        && !stop.load(Ordering::Relaxed)
        && std::fs::rename(&making, dest).is_ok();
    if !made {
        let _ = std::fs::remove_file(&making);
    }
    made
}

#[cfg(not(feature = "media"))]
fn moves(_: &FootageItem, _: &Path) -> bool {
    false
}

/// How many frames the moving picture at `path` has.
#[cfg(feature = "media")]
fn frames_of(path: &Path) -> Option<u64> {
    let source = lumit_media::MediaSource::file(path.to_path_buf());
    let index = lumit_render::media_index::load_or_build_index(&source).ok()?;
    Some(index.frame_count() as u64)
}

#[cfg(not(feature = "media"))]
fn frames_of(_: &Path) -> Option<u64> {
    None
}

#[cfg(not(feature = "media"))]
fn encode(_: &Path, _: &Path, _: Option<&[(usize, usize)]>, _: &AtomicBool) -> bool {
    false
}

/// What the carrier's own thread is handed.
enum Job {
    News(News),
    /// The document changed, so what this machine has and wants may have.
    Changed,
    Note {
        from: u32,
        body: Value,
    },
}

/// How an export asked of someone else is getting on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Asking {
    Idle,
    /// Sent, and the other person has not said yes yet.
    Waiting,
    Running {
        frame: u64,
        total: u64,
    },
    /// Their export is done and the file is on its way.
    Fetching {
        done: u64,
        total: u64,
    },
    Done {
        path: String,
    },
    /// They said no, or their export stopped. In their words, or empty.
    Failed {
        why: String,
    },
}

/// How fetching what an export lacks is getting on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fetching {
    Idle,
    Working {
        done: u64,
        total: u64,
    },
    /// Everything is here and the export was queued under `id`.
    Queued {
        id: u32,
    },
    /// Nobody could send something, or the export would not queue.
    Failed,
}

/// An export someone asked this machine to do.
struct Ask {
    from: u32,
    comp: Uuid,
    spec: Value,
}

/// An export this machine is doing for someone.
struct Doing {
    job: Uuid,
    from: u32,
    queued: u32,
    path: PathBuf,
    told: Instant,
}

/// One shared project's footage on this machine.
pub(crate) struct Carrier {
    project: Uuid,
    store: Arc<DocumentStore>,
    /// Where what is sent is kept, when it is not the machine's own folder
    /// for it: a folder named [`SENT`]. For two carriers in one process,
    /// which only a test has.
    root: Option<PathBuf>,
    jobs: SyncSender<Job>,
    stop: AtomicBool,
    /// Transfers in flight: how much has crossed, of how much, and which
    /// way. An entry goes when its file is whole or refused.
    moving: Mutex<HashMap<Wanted, (u64, u64, bool)>>,
    /// What nobody could send. Asked for again only when who has what
    /// changes, which empties it.
    refused: Mutex<HashSet<Wanted>>,
    held: Mutex<Vec<Held>>,
    /// Exports asked of this machine and not answered yet, by job. Bounded:
    /// one more than [`ASKS`] is turned away.
    asks: Mutex<HashMap<Uuid, Ask>>,
    doing: Mutex<Vec<Doing>>,
    /// Exports this machine did for someone, for them to fetch. Emptied with
    /// the carrier.
    made: Mutex<HashMap<Uuid, PathBuf>>,
    /// The one export asked of someone else: its job, where its file goes,
    /// and how it is getting on.
    asked: Mutex<(Uuid, PathBuf, Asking)>,
    fetching: Mutex<Fetching>,
    cancel_fetch: AtomicBool,
}

/// The most exports that can be waiting on this person's answer.
const ASKS: usize = 4;

/// The most taken for a file somebody else made: a stand-in, a run of
/// frames, an export.
const MOST_MADE: u64 = 64 << 30;

impl Footage for Carrier {
    fn make(&self, wanted: &Wanted, stop: &AtomicBool) -> Option<PathBuf> {
        if let Wanted::Export { job } = wanted {
            return self.made.lock().ok()?.get(job).cloned();
        }
        let doc = self.store.snapshot();
        let item = footage(&doc, wanted.item()?)?;
        let give = GIVE.load(Ordering::Relaxed);
        // Only ever the file the item is of. Anyone in the project can add an
        // item by any name, and a file of that name beside the project is
        // found for it. Its fingerprint is what they could not have made up
        // without holding the file already. An item with none was brought in
        // on this machine and not saved yet: one that came from someone else
        // with none is never pointed at a file here (`lumit-share`'s `find`).
        let own = |path: &Path| {
            item.media.fingerprint.as_ref().is_none_or(|print| {
                lumit_project::fingerprint_path(path)
                    .is_ok_and(|found| found.likely_same_content(print))
            })
        };
        match (wanted, here(item)) {
            // What someone else sent is passed on whoever asks.
            (Wanted::StandIn { .. }, Here::StandIn(path)) => Some(path),
            (Wanted::StandIn { .. }, Here::Original(path)) if give && own(&path) => {
                if !moves(item, &path) {
                    return (item.sequence.is_none()).then_some(path);
                }
                let dest = stand_in_path(self.root.as_deref(), item)?;
                (dest.is_file() || encode(&path, &dest, None, stop)).then_some(dest)
            }
            (Wanted::Part { first, count, .. }, state) => {
                let dest = parts_path(self.root.as_deref(), item, &[(*first, *count)])?;
                if dest.is_file() {
                    return Some(dest);
                }
                let Here::Original(path) = state else {
                    return None;
                };
                // A run that starts inside the clip, and no other: each one
                // asked for is an encode, and the numbers are the asker's. It
                // may run past the end, as a layer longer than its clip does.
                let within = *count > 0 && frames_of(&path).is_some_and(|frames| *first < frames);
                let runs = runs_of(*first, *count);
                (give
                    && own(&path)
                    && within
                    && moves(item, &path)
                    && encode(&path, &dest, Some(&runs), stop))
                .then_some(dest)
            }
            (Wanted::Original { .. }, Here::Original(path)) if give && own(&path) => Some(path),
            _ => None,
        }
    }

    fn most(&self, wanted: &Wanted) -> u64 {
        let doc = self.store.snapshot();
        let size = wanted
            .item()
            .and_then(|item| footage(&doc, item))
            .and_then(|item| item.media.fingerprint.as_ref())
            .map(|print| print.size);
        match (wanted, size) {
            // The file itself is as big as the item says it is.
            (Wanted::Original { .. }, Some(size)) => size,
            (Wanted::Original { .. }, None) => 0,
            // An encode has no size to hold it to but a ceiling.
            // ponytail: one flat ceiling. Work it out from the frames asked
            // for and the encode's rate if a disk is ever filled under it.
            _ => MOST_MADE,
        }
    }

    fn room(&self, wanted: &Wanted) -> Option<PathBuf> {
        if let Wanted::Export { job } = wanted {
            let asked = self.asked.lock().ok()?;
            let ext = asked.1.extension()?.to_string_lossy().into_owned();
            let folder = kept_under(self.root.as_deref())?.join("exports");
            return (asked.0 == *job).then(|| folder.join(format!("{job}.{ext}")));
        }
        let doc = self.store.snapshot();
        let item = footage(&doc, wanted.item()?)?;
        match wanted {
            Wanted::StandIn { .. } => stand_in_path(self.root.as_deref(), item),
            Wanted::Part { first, count, .. } => {
                parts_path(self.root.as_deref(), item, &[(*first, *count)])
            }
            Wanted::Original { .. } => original_path(self.root.as_deref(), item),
            Wanted::Export { .. } => None,
        }
    }

    fn told(&self, news: News) {
        // Progress that cannot be queued is dropped: more is coming. The
        // rest waits its turn, which is never long.
        match news {
            News::Moving { .. } => {
                let _ = self.jobs.try_send(Job::News(news));
            }
            news => {
                let _ = self.jobs.send(Job::News(news));
            }
        }
    }
}

/// The frames a [`Wanted::Part`] names, as the one run the encoder keeps.
fn runs_of(first: u64, count: u64) -> Vec<(usize, usize)> {
    vec![(first as usize, first.saturating_add(count) as usize)]
}

impl Carrier {
    /// Start carrying footage for a shared project. `tell` is how the
    /// interface hears of it.
    pub(crate) fn start(project: Uuid, store: Arc<DocumentStore>, tell: Tell) -> Arc<Carrier> {
        Self::start_under(project, store, tell, None)
    }

    /// [`Self::start`], keeping what is sent under `root`.
    pub(crate) fn start_under(
        project: Uuid,
        store: Arc<DocumentStore>,
        tell: Tell,
        root: Option<PathBuf>,
    ) -> Arc<Carrier> {
        let (jobs, queue) = sync_channel(256);
        let carrier = Arc::new(Carrier {
            project,
            store,
            root,
            jobs,
            stop: AtomicBool::new(false),
            moving: Mutex::new(HashMap::new()),
            refused: Mutex::new(HashSet::new()),
            held: Mutex::new(Vec::new()),
            asks: Mutex::new(HashMap::new()),
            doing: Mutex::new(Vec::new()),
            made: Mutex::new(HashMap::new()),
            asked: Mutex::new((Uuid::nil(), PathBuf::new(), Asking::Idle)),
            fetching: Mutex::new(Fetching::Idle),
            cancel_fetch: AtomicBool::new(false),
        });
        let running = carrier.clone();
        let spawned = std::thread::Builder::new()
            .name("lumit-footage".into())
            .spawn(move || running.run(&queue, &tell));
        if spawned.is_ok() {
            if let Ok(mut carriers) = CARRIERS.lock() {
                carriers.insert(project, carrier.clone());
            }
        }
        carrier
    }

    /// Stop carrying footage for `project`, whose sharing is over.
    pub(crate) fn stop(project: Uuid) {
        let carrier = CARRIERS
            .lock()
            .ok()
            .and_then(|mut all| all.remove(&project));
        if let Some(carrier) = carrier {
            carrier.stop.store(true, Ordering::Relaxed);
            carrier.cancel_fetch.store(true, Ordering::Relaxed);
        }
    }

    /// The document changed. Looked at on the carrier's own thread.
    pub(crate) fn changed(project: Uuid) {
        if let Some(carrier) = carrier(project) {
            let _ = carrier.jobs.try_send(Job::Changed);
        }
    }

    /// Another person's Lumit said something to this one.
    pub(crate) fn note(&self, from: u32, body: Value) {
        let _ = self.jobs.try_send(Job::Note { from, body });
    }

    fn sharing<T>(&self, with: impl FnOnce(&lumit_share::Sharing) -> T) -> Option<T> {
        crate::api::share::with_sharing(self.project, with)
    }

    fn want(&self, wanted: Wanted) {
        self.sharing(|sharing| sharing.want(wanted));
    }

    fn say(&self, to: u32, body: Value) {
        self.sharing(|sharing| sharing.note(to, body));
    }

    fn run(&self, queue: &Receiver<Job>, tell: &Tell) {
        self.settle(tell);
        // An edit says the document changed, and a drag is a great many
        // edits. What this machine has and wants is looked at again once
        // they have stopped coming for a moment, not once for each.
        let (mut changed, mut settled) = (false, Instant::now());
        while !self.stop.load(Ordering::Relaxed) {
            match queue.recv_timeout(Duration::from_millis(250)) {
                Ok(Job::News(news)) => self.news(news, tell),
                Ok(Job::Changed) => changed = true,
                Ok(Job::Note { from, body }) => self.heard(from, &body, tell),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if changed && settled.elapsed() >= Duration::from_millis(500) {
                (changed, settled) = (false, Instant::now());
                self.settle(tell);
            }
            self.watch_exports();
        }
    }

    /// Say which originals this machine has, if that has changed, and ask
    /// for a stand-in of whatever it lacks that a composition uses.
    fn settle(&self, tell: &Tell) {
        let doc = self.store.snapshot();
        let mut held = Vec::new();
        let mut wants = Vec::new();
        for item in &doc.items {
            let ProjectItem::Footage(item) = item else {
                continue;
            };
            match here(item) {
                Here::Original(path) => held.push(Held {
                    item: item.id,
                    bytes: std::fs::metadata(path).map_or(0, |m| m.len()),
                }),
                Here::Missing if doc.item_is_used(item.id) => wants.push(item.id),
                _ => {}
            }
        }
        let changed = self.held.lock().is_ok_and(|mut was| {
            let changed = *was != held;
            was.clone_from(&held);
            changed
        });
        if changed {
            self.sharing(|sharing| sharing.set_holds(held));
            tell(Told::Changed);
        }
        if !TAKE.load(Ordering::Relaxed) {
            return;
        }
        for item in wants {
            let wanted = Wanted::StandIn { item };
            let refused = self.refused.lock().is_ok_and(|r| r.contains(&wanted));
            let someone = self.sharing(|s| !s.holders(item).is_empty()) == Some(true);
            if someone && !refused {
                self.want(wanted);
            }
        }
    }

    fn news(&self, news: News, tell: &Tell) {
        match news {
            News::Moving {
                wanted,
                done,
                total,
                sending,
            } => {
                if let Ok(mut moving) = self.moving.lock() {
                    if done >= total && sending {
                        moving.remove(&wanted);
                    } else {
                        moving.insert(wanted.clone(), (done, total, sending));
                    }
                }
                // An export done for someone has gone to them, and the copy
                // of it here was only ever theirs.
                if let (Wanted::Export { job }, true) = (&wanted, done >= total && sending) {
                    let sent = self.made.lock().ok().and_then(|mut made| made.remove(job));
                    if let Some(path) = sent {
                        let _ = std::fs::remove_file(path);
                    }
                }
                // So is a run of frames cut for someone's export. It is made
                // again if it is asked for again.
                if let (Wanted::Part { item, first, count }, true) =
                    (&wanted, done >= total && sending)
                {
                    let doc = self.store.snapshot();
                    let sent = footage(&doc, *item)
                        .and_then(|f| parts_path(self.root.as_deref(), f, &[(*first, *count)]));
                    if let Some(path) = sent {
                        let _ = std::fs::remove_file(path);
                    }
                }
                if let (Wanted::Export { job }, Ok(mut asked)) = (&wanted, self.asked.lock()) {
                    if asked.0 == *job && !sending {
                        asked.2 = Asking::Fetching { done, total };
                    }
                }
                tell(Told::Changed);
            }
            News::Refused { wanted } => {
                if let Ok(mut moving) = self.moving.lock() {
                    moving.remove(&wanted);
                }
                if let (Wanted::Export { job }, Ok(mut asked)) = (&wanted, self.asked.lock()) {
                    if asked.0 == *job {
                        asked.2 = Asking::Failed { why: String::new() };
                    }
                }
                if let Ok(mut refused) = self.refused.lock() {
                    refused.insert(wanted);
                }
                tell(Told::Changed);
            }
            News::Holders => {
                // Someone new may have what was refused before.
                if let Ok(mut refused) = self.refused.lock() {
                    refused.clear();
                }
                self.settle(tell);
                tell(Told::Changed);
            }
            News::Arrived { wanted, path } => {
                if let Ok(mut moving) = self.moving.lock() {
                    moving.remove(&wanted);
                }
                self.arrived(&wanted, &path, tell);
            }
        }
    }

    /// A file is here, whole. Put it where the project reads it from.
    fn arrived(&self, wanted: &Wanted, path: &Path, tell: &Tell) {
        let place = |item: Uuid| {
            let path = path.to_string_lossy().into_owned();
            self.store.place_media(&[(item, path)]);
            self.settle(tell);
            tell(Told::Placed);
        };
        let doc = self.store.snapshot();
        match wanted {
            Wanted::StandIn { item } => {
                // Only where there was nothing: the original turning up
                // meanwhile is not swapped for a stand-in of itself.
                if footage(&doc, *item).is_some_and(|f| here(f) == Here::Missing) {
                    place(*item);
                }
            }
            Wanted::Original { item } => {
                // The file it says it is, or it is not kept.
                let print = footage(&doc, *item).and_then(|f| f.media.fingerprint.clone());
                let same = match (print, lumit_project::fingerprint_path(path)) {
                    (Some(wanted), Ok(got)) => wanted.likely_same_content(&got),
                    (None, Ok(_)) => true,
                    _ => false,
                };
                if same {
                    place(*item);
                } else {
                    let _ = std::fs::remove_file(path);
                    if let Ok(mut refused) = self.refused.lock() {
                        refused.insert(wanted.clone());
                    }
                }
            }
            Wanted::Part { .. } => {}
            Wanted::Export { job } => {
                if let Ok(mut asked) = self.asked.lock() {
                    if asked.0 == *job {
                        let dest = asked.1.clone();
                        let moved = std::fs::rename(path, &dest).is_ok()
                            || (std::fs::copy(path, &dest).is_ok()
                                && std::fs::remove_file(path).is_ok());
                        asked.2 = if moved {
                            Asking::Done {
                                path: dest.to_string_lossy().into_owned(),
                            }
                        } else {
                            Asking::Failed { why: String::new() }
                        };
                    }
                }
            }
        }
        tell(Told::Changed);
    }

    // --- What the interface asks -----------------------------------------

    /// Transfers in flight: what, how much of how much, and which way.
    pub(crate) fn moving(&self) -> Vec<(Wanted, u64, u64, bool)> {
        let Ok(moving) = self.moving.lock() else {
            return Vec::new();
        };
        let all = moving.iter();
        all.map(|(wanted, (done, total, sending))| (wanted.clone(), *done, *total, *sending))
            .collect()
    }

    /// Whether nobody could send `wanted` when it was last asked for.
    pub(crate) fn was_refused(&self, wanted: &Wanted) -> bool {
        self.refused.lock().is_ok_and(|r| r.contains(wanted))
    }

    /// Ask for a stand-in of `item`, or its original, whatever the settings.
    pub(crate) fn fetch(&self, item: Uuid, original: bool) {
        let wanted = if original {
            Wanted::Original { item }
        } else {
            Wanted::StandIn { item }
        };
        if let Ok(mut refused) = self.refused.lock() {
            refused.remove(&wanted);
        }
        self.want(wanted);
    }

    pub(crate) fn fetching(&self) -> Fetching {
        self.fetching.lock().map_or(Fetching::Idle, |f| f.clone())
    }

    pub(crate) fn cancel_fetch(&self) {
        self.cancel_fetch.store(true, Ordering::Relaxed);
    }

    /// Fetch what an export of `comp` lacks and then queue it: the originals
    /// whole, or with `parts` only the frames the export reads. Answers at
    /// once, and [`Self::fetching`] says how it is going.
    pub(crate) fn fetch_then_export(
        self: &Arc<Self>,
        comp: Uuid,
        comp_name: String,
        spec: BridgeExportSpec,
        path: String,
        parts: bool,
        start: bool,
    ) {
        if let Ok(mut fetching) = self.fetching.lock() {
            if matches!(*fetching, Fetching::Working { .. }) {
                return;
            }
            *fetching = Fetching::Working { done: 0, total: 0 };
        }
        self.cancel_fetch.store(false, Ordering::Relaxed);
        let carrier = self.clone();
        let spawned = std::thread::Builder::new()
            .name("lumit-footage-fetch".into())
            .spawn(move || {
                let queued = carrier.gather(comp, &spec, parts).and_then(|doc| {
                    crate::export::queue_add(doc, comp, comp_name, &spec, &path, start).ok()
                });
                if let Ok(mut fetching) = carrier.fetching.lock() {
                    *fetching = queued.map_or(Fetching::Failed, |id| Fetching::Queued { id });
                }
            });
        if spawned.is_err() {
            if let Ok(mut fetching) = self.fetching.lock() {
                *fetching = Fetching::Failed;
            }
        }
    }

    /// Get what an export of `comp` lacks, and answer the document to export
    /// from once it is all here. `None` when something could not be had.
    fn gather(&self, comp: Uuid, spec: &BridgeExportSpec, parts: bool) -> Option<Arc<Document>> {
        let mut doc = self.store.snapshot();
        if parts {
            // A clip with nothing of it here has no length to say which
            // frames of it are read. Its stand-in comes first, which is
            // small, and the frames are worked out against that.
            let nothing = |doc: &Document, item: &Uuid| {
                footage(doc, *item).is_some_and(|f| here(f) == Here::Missing)
            };
            let bare: Vec<Uuid> = lacking(&doc, comp)
                .into_iter()
                .filter(|item| nothing(&doc, item))
                .collect();
            for item in &bare {
                self.fetch(*item, false);
            }
            loop {
                if self.stop.load(Ordering::Relaxed) || self.cancel_fetch.load(Ordering::Relaxed) {
                    return None;
                }
                let now = self.store.snapshot();
                let waiting = |item: &Uuid| {
                    nothing(&now, item) && !self.was_refused(&Wanted::StandIn { item: *item })
                };
                if !bare.iter().any(waiting) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            doc = self.store.snapshot();
        }
        let mut wants: Vec<Wanted> = Vec::new();
        let lacks = lacking(&doc, comp);
        let used = if parts {
            self.frames_used(&doc, comp, spec)
        } else {
            HashMap::new()
        };
        for item in lacks {
            // The frames an export reads of a clip it has a stand-in of. A
            // clip with no stand-in yet has no length to cut runs from, so
            // it is fetched whole.
            let run = used.get(&item).filter(|_| {
                footage(&doc, item).is_some_and(|f| matches!(here(f), Here::StandIn(_)))
            });
            match run {
                Some(&(first, end)) if end > first => wants.push(Wanted::Part {
                    item,
                    first,
                    count: end - first,
                }),
                Some(_) => {}
                None => wants.push(Wanted::Original { item }),
            }
        }
        if let Ok(mut refused) = self.refused.lock() {
            refused.retain(|wanted| !wants.contains(wanted));
        }
        for wanted in &wants {
            self.want(wanted.clone());
        }
        // Wait for them all, saying how far along the bytes are.
        loop {
            if self.stop.load(Ordering::Relaxed) || self.cancel_fetch.load(Ordering::Relaxed) {
                // Told to whoever is sending, so the fetch stops at their
                // end too and not only the waiting here.
                for wanted in &wants {
                    self.sharing(|sharing| sharing.unwant(wanted));
                }
                return None;
            }
            let now = self.store.snapshot();
            let here_now = |wanted: &Wanted| match wanted {
                Wanted::Part { item, first, count } => footage(&now, *item)
                    .and_then(|f| parts_path(self.root.as_deref(), f, &[(*first, *count)]))
                    .is_some_and(|path| path.is_file()),
                Wanted::Original { item } => {
                    footage(&now, *item).is_some_and(|f| matches!(here(f), Here::Original(_)))
                }
                _ => true,
            };
            if wants.iter().any(|wanted| self.was_refused(wanted)) {
                return None;
            }
            if wants.iter().all(here_now) {
                break;
            }
            let (mut done, mut total) = (0, 0);
            for (wanted, moved, of, sending) in self.moving() {
                if !sending && wants.contains(&wanted) {
                    done += moved;
                    total += of;
                }
            }
            if let Ok(mut fetching) = self.fetching.lock() {
                *fetching = Fetching::Working { done, total };
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        // The document to export: this one, with each clip that was sent in
        // parts read from its parts. A copy, so the project keeps reading
        // the stand-in, which unlike the parts has every frame.
        let mut doc = Document::clone(&self.store.snapshot());
        for wanted in &wants {
            let Wanted::Part { item, first, count } = wanted else {
                continue;
            };
            let Some(ProjectItem::Footage(footage)) = doc.item_mut(*item) else {
                continue;
            };
            if let Some(path) = parts_path(self.root.as_deref(), footage, &[(*first, *count)]) {
                footage.media.absolute_path = path.to_string_lossy().into_owned();
            }
        }
        Some(Arc::new(doc))
    }

    /// The frames an export of `comp` reads of each clip, as one run from
    /// the first it reads to the last.
    #[cfg(feature = "media")]
    fn frames_used(
        &self,
        doc: &Arc<Document>,
        comp: Uuid,
        spec: &BridgeExportSpec,
    ) -> HashMap<Uuid, (u64, u64)> {
        let Some(size) = doc.comp(comp).map(|c| (c.width, c.height)) else {
            return HashMap::new();
        };
        let Ok(spec) = crate::export::to_export_spec(spec, size.0, size.1) else {
            return HashMap::new();
        };
        let used = lumit_render::headless::source_frames_used(doc, comp, &spec, &self.cancel_fetch);
        let mut runs = HashMap::new();
        for (item, frames) in used {
            let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
                continue;
            };
            // Whole runs with room round them, joined into one: a clip is
            // cut from in one place far more often than in two, and one run
            // is one file to make, send and keep.
            let padded = lumit_render::standin::ranges(&frames, usize::MAX);
            let first = padded.first().map_or(*first, |run| run.0);
            let end = padded.last().map_or(*last + 1, |run| run.1);
            runs.insert(item, (first as u64, end as u64));
        }
        runs
    }

    #[cfg(not(feature = "media"))]
    fn frames_used(
        &self,
        _: &Arc<Document>,
        _: Uuid,
        _: &BridgeExportSpec,
    ) -> HashMap<Uuid, (u64, u64)> {
        HashMap::new()
    }

    // --- An export done by someone else ----------------------------------

    pub(crate) fn asking(&self) -> Asking {
        self.asked.lock().map_or(Asking::Idle, |a| a.2.clone())
    }

    /// Ask the person numbered `to` to export `comp` by `spec` and send the
    /// file back, to be written at `path`.
    pub(crate) fn ask_export(&self, to: u32, comp: Uuid, spec: &BridgeExportSpec, path: &str) {
        let doc = self.store.snapshot();
        let Some(size) = doc.comp(comp).map(|c| (c.width, c.height)) else {
            return;
        };
        let Ok(spec) = spec_value(spec, size) else {
            return;
        };
        let job = Uuid::now_v7();
        if let Ok(mut asked) = self.asked.lock() {
            *asked = (job, PathBuf::from(path), Asking::Waiting);
        }
        self.say(
            to,
            json!({ "lumit": "export-ask", "job": job, "comp": comp, "spec": spec }),
        );
    }

    /// Answer an export someone asked of this machine. Saying yes queues it.
    pub(crate) fn answer_export(&self, job: Uuid, yes: bool) {
        let Some(ask) = self.asks.lock().ok().and_then(|mut asks| asks.remove(&job)) else {
            return;
        };
        let no = |why: &str| {
            self.say(
                ask.from,
                json!({ "lumit": "export-failed", "job": job, "why": why }),
            );
        };
        if !yes {
            return no("refused");
        }
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(ask.comp) else {
            return no("gone");
        };
        // Asked because this machine has the footage. If it has not, the
        // file it made would be no better than the one they can make.
        if !lacking(&doc, ask.comp).is_empty() {
            return no("lacking");
        }
        let Some(mut spec) = spec_from(&ask.spec) else {
            return no("spec");
        };
        // What happens when it lands is this person's to choose, not theirs.
        spec.make_a_noise = false;
        spec.open_folder = false;
        let Some(folder) = kept_under(self.root.as_deref()).map(|dir| dir.join("exports")) else {
            return no("nowhere");
        };
        let _ = std::fs::create_dir_all(&folder);
        let path = folder.join(format!("{job}-made"));
        let path = path.to_string_lossy().into_owned();
        let name = comp.name.clone();
        match crate::export::queue_add(doc.clone(), ask.comp, name, &spec, &path, true) {
            Ok(queued) => {
                let path = crate::export::queue_list()
                    .into_iter()
                    .find(|row| row.id == queued)
                    .map_or(path, |row| row.out_path);
                if let Ok(mut doing) = self.doing.lock() {
                    doing.push(Doing {
                        job,
                        from: ask.from,
                        queued,
                        path: PathBuf::from(path),
                        told: Instant::now(),
                    });
                }
            }
            Err(_) => no("spec"),
        }
    }

    /// Tell whoever asked how the exports this machine is doing for them
    /// are getting on.
    fn watch_exports(&self) {
        use crate::export::QueueRowState;
        let Ok(mut doing) = self.doing.lock() else {
            return;
        };
        if doing.is_empty() {
            return;
        }
        let rows = crate::export::queue_list();
        doing.retain_mut(|doing| {
            let state = rows.iter().find(|row| row.id == doing.queued);
            let (job, from) = (doing.job, doing.from);
            match state.map(|row| &row.state) {
                Some(QueueRowState::Running { frame, total, .. }) => {
                    if doing.told.elapsed() >= Duration::from_secs(1) {
                        doing.told = Instant::now();
                        let body = json!({ "lumit": "export-progress", "job": job,
                            "frame": frame, "total": total });
                        self.say(from, body);
                    }
                    true
                }
                Some(QueueRowState::Waiting) => true,
                Some(QueueRowState::Done) => {
                    if let Ok(mut made) = self.made.lock() {
                        made.insert(job, doing.path.clone());
                    }
                    self.say(from, json!({ "lumit": "export-done", "job": job }));
                    false
                }
                Some(QueueRowState::Failed(_)) | None => {
                    let body = json!({ "lumit": "export-failed", "job": job, "why": "stopped" });
                    self.say(from, body);
                    false
                }
            }
        });
    }

    /// Another person's Lumit said something. Nothing in it is believed
    /// further than it has to be: it names a job and, to ask, a composition
    /// and how to export it.
    fn heard(&self, from: u32, body: &Value, tell: &Tell) {
        let job = body.get("job").and_then(Value::as_str);
        let Some(job) = job.and_then(|job| job.parse::<Uuid>().ok()) else {
            return;
        };
        let mine = |asked: &(Uuid, PathBuf, Asking)| asked.0 == job;
        match body.get("lumit").and_then(Value::as_str) {
            Some("export-ask") => {
                let comp = body.get("comp").and_then(Value::as_str);
                let comp = comp.and_then(|comp| comp.parse::<Uuid>().ok());
                let (Some(comp), Some(spec)) = (comp, body.get("spec")) else {
                    return;
                };
                let name = self.store.snapshot().comp(comp).map(|c| c.name.clone());
                let (Some(name), Ok(mut asks)) = (name, self.asks.lock()) else {
                    return;
                };
                if asks.len() >= ASKS {
                    drop(asks);
                    let body = json!({ "lumit": "export-failed", "job": job, "why": "refused" });
                    self.say(from, body);
                    return;
                }
                let spec = spec.clone();
                asks.insert(job, Ask { from, comp, spec });
                drop(asks);
                tell(Told::Asked {
                    job,
                    from,
                    comp: name,
                });
            }
            Some("export-progress") => {
                let number = |name: &str| body.get(name).and_then(Value::as_u64).unwrap_or(0);
                if let Ok(mut asked) = self.asked.lock() {
                    if mine(&asked) {
                        asked.2 = Asking::Running {
                            frame: number("frame"),
                            total: number("total"),
                        };
                    }
                }
                tell(Told::Changed);
            }
            Some("export-done") => {
                let waiting = self.asked.lock().is_ok_and(|asked| mine(&asked));
                if waiting {
                    if let Ok(mut asked) = self.asked.lock() {
                        asked.2 = Asking::Fetching { done: 0, total: 0 };
                    }
                    self.want(Wanted::Export { job });
                }
                tell(Told::Changed);
            }
            Some("export-failed") => {
                let why = body.get("why").and_then(Value::as_str).unwrap_or_default();
                if let Ok(mut asked) = self.asked.lock() {
                    if mine(&asked) {
                        asked.2 = Asking::Failed {
                            why: why.chars().take(32).collect(),
                        };
                    }
                }
                tell(Told::Changed);
            }
            _ => {}
        }
    }
}

/// An export's settings as they cross to another machine.
#[cfg(feature = "media")]
fn spec_value(spec: &BridgeExportSpec, size: (u32, u32)) -> Result<Value, String> {
    let spec = crate::export::to_export_spec(spec, size.0, size.1)?;
    serde_json::to_value(spec).map_err(|e| e.to_string())
}

/// The settings another machine sent, as this one's own.
#[cfg(feature = "media")]
fn spec_from(value: &Value) -> Option<BridgeExportSpec> {
    let spec: lumit_render::export::ExportSpec = serde_json::from_value(value.clone()).ok()?;
    crate::export::from_export_spec(&spec)
}

#[cfg(not(feature = "media"))]
fn spec_value(_: &BridgeExportSpec, _: (u32, u32)) -> Result<Value, String> {
    Err("no media".into())
}

#[cfg(not(feature = "media"))]
fn spec_from(_: &Value) -> Option<BridgeExportSpec> {
    None
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::MediaRef;
    use lumit_core::Op;
    use lumit_media::encode::{ColourTags, Encoder, Metadata, VideoCodec, VideoSettings};
    use std::net::{IpAddr, Ipv4Addr};

    fn until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(60), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A clip one person has and the other has not is read by the other
    /// from a stand-in the first makes when asked: a file of the clip's own
    /// size with every one of its frames. Asked for the original, it is
    /// the file itself that arrives, and the stand-in gives way to it.
    #[test]
    fn a_clip_missing_here_is_read_from_what_another_person_sends() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        let settings = VideoSettings {
            codec: VideoCodec::H264,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
            colour: ColourTags::default(),
        };
        let mut encoder = Encoder::open(&clip, Some(&settings), None, &Metadata::new()).unwrap();
        for n in 0..12u8 {
            let frame: Vec<u8> = [n * 20, 90, 200, 255].repeat(320 * 240);
            encoder.write_rgba(&frame).unwrap();
        }
        encoder.finish().unwrap();

        // The host has the clip in its project.
        let item = Uuid::now_v7();
        let value = json!({ "id": item, "name": "clip", "media": { "relative_path": "clip.mp4" } });
        let mut clip_item: FootageItem = serde_json::from_value(value).unwrap();
        clip_item.media = MediaRef {
            absolute_path: clip.to_string_lossy().into_owned(),
            fingerprint: lumit_project::fingerprint_path(&clip).ok(),
            ..clip_item.media
        };
        let hosted = Arc::new(DocumentStore::new(Document::new()));
        let added = Op::AddItem {
            index: 0,
            item: Box::new(ProjectItem::Footage(clip_item)),
        };
        hosted.commit(added).unwrap();

        // Two machines in one process: each with a project, an end of the
        // sharing, and a folder of its own for what it is sent.
        let quiet: lumit_share::Events = Arc::new(|_| {});
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let host = lumit_share::host(
            hosted.clone(),
            "Host",
            loopback,
            0,
            None,
            None,
            None,
            quiet.clone(),
        )
        .unwrap();
        let (document, joining) =
            lumit_share::join(host.invite("127.0.0.1"), "Guest", None).unwrap();
        let joined = Arc::new(DocumentStore::new(document));
        let guest = joining.start(joined.clone(), quiet).unwrap();
        let (host_project, guest_project) = (Uuid::now_v7(), Uuid::now_v7());
        let tell: Tell = Arc::new(|_| {});
        let machine = |project: Uuid, store: &Arc<DocumentStore>, sharing, name: &str| {
            crate::api::share::share_for_test(project, sharing);
            let root = dir.path().join(name).join(SENT);
            let carrier = Carrier::start_under(project, store.clone(), tell.clone(), Some(root));
            crate::api::share::with_sharing(project, |sharing| {
                sharing.carry_footage(carrier.clone(), LIMITS.clone());
            });
            carrier
        };
        let _hosts = machine(
            host_project,
            &hosted,
            lumit_share::Sharing::Host(host),
            "host",
        );
        let guests = machine(
            guest_project,
            &joined,
            lumit_share::Sharing::Guest(guest),
            "guest",
        );
        let at_guest = || {
            let doc = joined.snapshot();
            here(footage(&doc, item).unwrap())
        };
        assert_eq!(at_guest(), Here::Missing);
        until("the guest hears who has the clip", || {
            crate::api::share::with_sharing(guest_project, |s| !s.holders(item).is_empty())
                == Some(true)
        });

        guests.fetch(item, false);
        until("a stand-in arrives and is read from", || {
            matches!(at_guest(), Here::StandIn(_))
        });
        let Here::StandIn(stand_in) = at_guest() else {
            panic!("a stand-in");
        };
        let frames = |path: &Path| {
            let source = lumit_media::MediaSource::file(path.to_path_buf());
            lumit_render::media_index::load_or_build_index(&source)
                .unwrap()
                .frame_count()
        };
        assert_eq!(frames(&stand_in), frames(&clip));
        assert!(stand_in.starts_with(dir.path().join("guest")));

        guests.fetch(item, true);
        until("the original arrives and takes over", || {
            matches!(at_guest(), Here::Original(_))
        });
        let Here::Original(original) = at_guest() else {
            panic!("the original");
        };
        assert_eq!(
            std::fs::read(original).unwrap(),
            std::fs::read(&clip).unwrap()
        );

        crate::api::share::stop(host_project);
        crate::api::share::stop(guest_project);
    }

    /// Two machines sharing a project that has a clip in a composition: the
    /// host has the clip and the guest has not.
    struct Rig {
        dir: tempfile::TempDir,
        hosted: Arc<DocumentStore>,
        joined: Arc<DocumentStore>,
        hosts: Arc<Carrier>,
        guests: Arc<Carrier>,
        projects: (Uuid, Uuid),
        item: Uuid,
        comp: Uuid,
        /// The exports the host has been asked to do.
        asked: Arc<Mutex<Vec<Uuid>>>,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            crate::api::share::stop(self.projects.0);
            crate::api::share::stop(self.projects.1);
        }
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        let settings = VideoSettings {
            codec: VideoCodec::H264,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
            colour: ColourTags::default(),
        };
        let mut encoder = Encoder::open(&clip, Some(&settings), None, &Metadata::new()).unwrap();
        for n in 0..12u8 {
            let frame: Vec<u8> = [n * 20, 90, 200, 255].repeat(320 * 240);
            encoder.write_rgba(&frame).unwrap();
        }
        encoder.finish().unwrap();

        // A project with the clip in a composition, made the way the
        // interface makes one, and then its document on its own.
        let project = crate::api::state::LumitBridgeState::new_project(None).unwrap();
        let comp = project.new_composition("Scene".into(), None).unwrap();
        let placed = project
            .import_footage(clip.to_string_lossy().into_owned())
            .unwrap();
        comp.add_footage_layer(&placed, false, None).unwrap();
        let (item, comp) = (placed.id, comp.id);
        let document = {
            let state = project.state().unwrap();
            let state = state.read().unwrap();
            Document::clone(&state.store.snapshot())
        };
        project.close().unwrap();

        let hosted = Arc::new(DocumentStore::new(document));
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let projects = (Uuid::now_v7(), Uuid::now_v7());
        // A note reaches a machine's carrier as the bridge hands it on.
        let notes_for = |project: Uuid| -> lumit_share::Events {
            Arc::new(move |event| {
                if let lumit_share::Event::Note { from, body } = event {
                    if let Some(carrier) = carrier(project) {
                        carrier.note(from, body);
                    }
                }
            })
        };
        let events = notes_for(projects.0);
        let host = lumit_share::host(
            hosted.clone(),
            "Host",
            loopback,
            0,
            None,
            None,
            None,
            events,
        )
        .unwrap();
        let (document, joining) =
            lumit_share::join(host.invite("127.0.0.1"), "Guest", None).unwrap();
        let joined = Arc::new(DocumentStore::new(document));
        let guest = joining
            .start(joined.clone(), notes_for(projects.1))
            .unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = asked.clone();
        let tell: Tell = Arc::new(move |told| {
            if let Told::Asked { job, .. } = told {
                heard.lock().unwrap().push(job);
            }
        });
        let machine = |project: Uuid, store: &Arc<DocumentStore>, sharing, name: &str| {
            crate::api::share::share_for_test(project, sharing);
            let root = dir.path().join(name).join(SENT);
            let carrier = Carrier::start_under(project, store.clone(), tell.clone(), Some(root));
            crate::api::share::with_sharing(project, |sharing| {
                sharing.carry_footage(carrier.clone(), LIMITS.clone());
            });
            carrier
        };
        let hosts = machine(
            projects.0,
            &hosted,
            lumit_share::Sharing::Host(host),
            "host",
        );
        let guests = machine(
            projects.1,
            &joined,
            lumit_share::Sharing::Guest(guest),
            "guest",
        );
        Rig {
            dir,
            hosted,
            joined,
            hosts,
            guests,
            projects,
            item,
            comp,
            asked,
        }
    }

    /// An export on a machine without the clip fetches the frames it reads
    /// and is queued from them. Asking the other machine to do it instead
    /// reaches that machine as a question, and its no comes back.
    #[test]
    fn an_export_fetches_its_frames_or_is_asked_of_whoever_has_them() {
        let _queue = crate::api::tests::export_queue_test();
        let rig = rig();
        let (item, comp) = (rig.item, rig.comp);
        // The clip is in a composition, so the guest asks for a stand-in of
        // it without being told to.
        until("a stand-in arrives by itself", || {
            let doc = rig.joined.snapshot();
            matches!(here(footage(&doc, item).unwrap()), Here::StandIn(_))
        });
        assert_eq!(lacking(&rig.joined.snapshot(), comp), [item]);
        assert!(lacking(&rig.hosted.snapshot(), comp).is_empty());

        let out = rig.dir.path().join("out.mp4");
        let out = out.to_string_lossy().into_owned();
        let spec = BridgeExportSpec::default();
        let name = String::from("Scene");
        rig.guests
            .fetch_then_export(comp, name, spec.clone(), out.clone(), true, false);
        until("the frames arrive and the export is queued", || {
            assert_ne!(rig.guests.fetching(), Fetching::Failed);
            matches!(rig.guests.fetching(), Fetching::Queued { .. })
        });
        let Fetching::Queued { id } = rig.guests.fetching() else {
            panic!("queued");
        };
        crate::export::queue_remove(id);
        let parts = std::fs::read_dir(rig.dir.path().join("guest").join(SENT))
            .unwrap()
            .flatten()
            .flat_map(|folder| {
                std::fs::read_dir(folder.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .any(|file| file.file_name().to_string_lossy().starts_with("parts-"));
        assert!(
            parts,
            "the frames the export reads came as a file of their own"
        );

        // Asked of the host instead, the host is asked, and says no.
        rig.guests.ask_export(0, comp, &spec, &out);
        until("the host is asked", || {
            !rig.asked.lock().unwrap().is_empty()
        });
        assert_eq!(rig.guests.asking(), Asking::Waiting);
        let job = rig.asked.lock().unwrap()[0];
        rig.hosts.answer_export(job, false);
        until("the no comes back", || {
            let refused = Asking::Failed {
                why: "refused".into(),
            };
            rig.guests.asking() == refused
        });
    }

    /// The whole of an export done by another machine: asked, agreed to,
    /// exported there from the originals, and the file sent back to where
    /// the person who asked wanted it. It runs a real export, which needs a
    /// GPU and takes the queue, so it is run by hand:
    /// `cargo test -p lumit_bridge --lib asked_of_another -- --ignored`.
    #[test]
    #[ignore = "runs a real export"]
    fn an_export_asked_of_another_machine_comes_back() {
        let _queue = crate::api::tests::export_queue_test();
        let rig = rig();
        let out = rig.dir.path().join("out.mp4");
        // A dozen frames: it is the journey that is under test.
        let spec = BridgeExportSpec {
            range_start_frame: 0,
            range_end_frame: 12,
            ..BridgeExportSpec::default()
        };
        rig.guests
            .ask_export(0, rig.comp, &spec, &out.to_string_lossy());
        until("the host is asked", || {
            !rig.asked.lock().unwrap().is_empty()
        });
        let job = rig.asked.lock().unwrap()[0];
        rig.hosts.answer_export(job, true);
        crate::export::queue_start();
        until("the file comes back", || {
            // The queue is turned over by being read, as the interface does.
            let _ = crate::export::queue_list();
            assert!(
                !matches!(rig.guests.asking(), Asking::Failed { .. }),
                "{:?}",
                rig.guests.asking()
            );
            matches!(rig.guests.asking(), Asking::Done { .. })
        });
        assert!(std::fs::metadata(&out).is_ok_and(|file| file.len() > 1000));
    }
}

//! Remote export: one of the account's computers exporting for another
//! (docs/01-GLOSSARY.md). The **sender** has the project and asks. The
//! **worker** does the export, and sends the file back or keeps it.
//!
//! # In plain terms
//!
//! The interface holds the line to the account's room on Lumit's server and
//! the keys that seal what crosses it. It hands on what it hears, and says
//! what this module gives it. Nothing here sees a private key or the server.
//!
//! The sender works out every file the export reads and what each one is by
//! its hash, then offers the job. A worker that takes it lists the files it
//! has already, in its store, at the same place, or in folders it was told
//! to look in, and asks the sender for the rest over a link of the job's own
//! (`lumit_share::remote`). Once they are all here it lays them out in the
//! job's folder and the job takes its turn in the export queue, where it runs
//! in a `lumit-cli` of its own. The finished file goes back, or stays.
//!
//! Both computers keep every job on disk, so either can sleep, lose the
//! network or quit, and the job carries on when it is back. A job that has
//! not ended in a week is given up on.
//!
//! Threads: one of its own that hears what the room and the links said and
//! turns the jobs over four times a second, and one for disk work: hashing,
//! laying out and landing files. Links have their own. The interface's calls
//! only read and write what is held here, and hand the rest to those two.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};

use lumit_share::bulk::News;
use serde_json::Value;
use uuid::Uuid;
use web_time::{Duration, Instant};

use crate::api::composition::CompositionReference;
use crate::api::export::BridgeExportSpec;
use crate::api::remote::{
    BridgeComputer, BridgeExportNews, BridgeExportNewsKind, BridgeRemoteFit, BridgeRemoteJob,
    BridgeRemoteLands, BridgeRemoteSay,
};
use crate::api::BridgeError;
use crate::frb_generated::StreamSink;

mod either;
mod end;
mod files;
mod job;
mod needs;
mod room;
mod sender;
mod worker;

use job::{Job, Stage};

/// How often the jobs are turned over.
const TURN: Duration = Duration::from_millis(250);

/// How long a job may go without ending before it is given up on.
const WEEK: i64 = 7 * 24 * 60 * 60;

/// What the jobs' thread is handed.
enum Event {
    Heard(String),
    Command {
        id: String,
        from: String,
        body: String,
    },
    Reply {
        id: String,
        body: String,
        error: String,
    },
    /// A transfer of a job's moved on.
    Told(Uuid, News),
    /// Whether the computers of a fit in hand can export a composition.
    Check {
        computer: String,
        comp: Uuid,
        revision: u64,
        doc: Arc<lumit_core::model::Document>,
    },
}

/// What the disk thread is handed.
enum Io {
    /// Work out a sent job's files.
    Prepare(Uuid, sender::Snapshot),
    /// A file of a job arrived at `path`.
    Arrived(Uuid, Uuid, PathBuf),
    /// Work out which of a job's files the worker has.
    Gather(Uuid),
    /// Delete what is left of a job.
    Forget(Uuid),
    /// Count, or with `true` empty, the store.
    Store(bool),
}

/// The files the sender gives for one job, by the item each is asked for
/// as: where it is and its size.
type Holding = HashMap<Uuid, (PathBuf, u64)>;

type Say = Box<dyn Fn(BridgeRemoteSay) + Send + Sync>;
type Tell = Box<dyn Fn(BridgeExportNews) + Send + Sync>;

/// Transfers of one job: how much of each file has crossed, of how much, and
/// the last few readings of the whole, for a rate.
#[derive(Default)]
struct Moving {
    files: HashMap<Uuid, (u64, u64)>,
    readings: Vec<(Instant, u64)>,
}

/// Remote exports on this computer.
pub(crate) struct Remote {
    /// `<cache>/remote`: the jobs, the store and the hashes.
    root: PathBuf,
    outbox: Mutex<Option<Say>>,
    newsbox: Mutex<Option<Tell>>,
    account: Mutex<String>,
    takes: AtomicBool,
    folders: Mutex<Vec<PathBuf>>,
    room: Mutex<room::Room>,
    said: Mutex<Vec<room::Said>>,
    jobs: Mutex<BTreeMap<Uuid, Job>>,
    /// Jobs changed since they were last written.
    dirty: Mutex<HashSet<Uuid>>,
    /// Each job's link while it has one. Locked alone: a link calls back in.
    links: Mutex<HashMap<Uuid, Arc<lumit_share::remote::Link>>>,
    moving: Mutex<HashMap<Uuid, Moving>>,
    /// The worker's side of each job's files while it gathers them.
    gathering: Mutex<HashMap<Uuid, worker::Gathering>>,
    /// The sender's files of each job by the item each is asked for as:
    /// where it is and its size.
    holding: Mutex<HashMap<Uuid, Arc<Holding>>>,
    /// When the sender last asked for a finished export's files, and last
    /// asked how a job stands, by job.
    asked: Mutex<HashMap<Uuid, Instant>>,
    polled: Mutex<HashMap<Uuid, Instant>>,
    /// Sent jobs whose files this process has begun to work out.
    job_started: Mutex<HashSet<Uuid>>,
    /// Worker's jobs with an item in this process's export queue.
    queued: Mutex<HashSet<Uuid>>,
    /// What each computer said about each composition, by revision. Emptied
    /// when it grows past a few dozen.
    fits: Mutex<HashMap<(String, Uuid, u64), BridgeRemoteFit>>,
    hashes: files::Hashes,
    events: SyncSender<Event>,
    io: SyncSender<Io>,
    stop: AtomicBool,
    next_mirror: AtomicU32,
    kept: AtomicU64,
    awake: Mutex<Option<crate::awake::Awake>>,
}

/// The one for this process, started when first asked for.
static HERE: OnceLock<Arc<Remote>> = OnceLock::new();

fn here() -> &'static Arc<Remote> {
    HERE.get_or_init(|| {
        let root = lumit_project::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("remote");
        Remote::start(root)
    })
}

impl Remote {
    /// Start remote exports kept under `root`, with the jobs found there.
    pub(crate) fn start(root: PathBuf) -> Arc<Remote> {
        let (events, heard) = sync_channel(1024);
        let (io, work) = sync_channel(4096);
        let jobs: BTreeMap<Uuid, Job> = job::load(&root).into_iter().map(|j| (j.job, j)).collect();
        let mirror = jobs
            .values()
            .filter(|j| j.sent)
            .map(|j| j.item + 1)
            .max()
            .unwrap_or(0)
            .max(1 << 30);
        let remote = Arc::new(Remote {
            hashes: files::Hashes::open(root.join("hashes.json")),
            root,
            outbox: Mutex::new(None),
            newsbox: Mutex::new(None),
            account: Mutex::new(String::new()),
            takes: AtomicBool::new(true),
            folders: Mutex::new(Vec::new()),
            room: Mutex::new(room::Room::default()),
            said: Mutex::new(Vec::new()),
            jobs: Mutex::new(jobs),
            dirty: Mutex::new(HashSet::new()),
            links: Mutex::new(HashMap::new()),
            moving: Mutex::new(HashMap::new()),
            gathering: Mutex::new(HashMap::new()),
            holding: Mutex::new(HashMap::new()),
            asked: Mutex::new(HashMap::new()),
            polled: Mutex::new(HashMap::new()),
            job_started: Mutex::new(HashSet::new()),
            queued: Mutex::new(HashSet::new()),
            fits: Mutex::new(HashMap::new()),
            events,
            io,
            stop: AtomicBool::new(false),
            next_mirror: AtomicU32::new(mirror),
            kept: AtomicU64::new(0),
            awake: Mutex::new(None),
        });
        let turning = remote.clone();
        let _ = std::thread::Builder::new()
            .name("lumit-remote".into())
            .spawn(move || turning.run(&heard));
        let working = remote.clone();
        let _ = std::thread::Builder::new()
            .name("lumit-remote-io".into())
            .spawn(move || working.work(&work));
        let _ = remote.io.try_send(Io::Store(false));
        remote
    }

    /// Stop every link and both threads. For a test, which starts its own.
    #[cfg(all(test, feature = "media"))]
    pub(crate) fn shut(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let links: Vec<_> = self
            .links
            .lock()
            .map(|mut l| l.drain().collect())
            .unwrap_or_default();
        for (_, link) in links {
            link.stop();
        }
    }

    pub(crate) fn set_outbox(&self, say: Say) {
        if let Ok(mut outbox) = self.outbox.lock() {
            *outbox = Some(say);
        }
    }

    pub(crate) fn set_news(&self, tell: Tell) {
        if let Ok(mut newsbox) = self.newsbox.lock() {
            *newsbox = Some(tell);
        }
    }

    pub(crate) fn heard(&self, said: &str) {
        let _ = self.events.try_send(Event::Heard(said.to_owned()));
    }

    pub(crate) fn command(&self, id: &str, from: &str, body: &str) {
        let _ = self.events.try_send(Event::Command {
            id: id.to_owned(),
            from: from.to_owned(),
            body: body.to_owned(),
        });
    }

    pub(crate) fn reply(&self, id: &str, body: &str, error: &str) {
        let _ = self.events.try_send(Event::Reply {
            id: id.to_owned(),
            body: body.to_owned(),
            error: error.to_owned(),
        });
    }

    /// A copy of `job` as it stands.
    fn job(&self, job: Uuid) -> Option<Job> {
        self.jobs.lock().ok()?.get(&job).cloned()
    }

    /// Whether `job` has ended or gone, or this is stopping. What takes a
    /// while for a job asks it as it goes.
    fn over(&self, job: Uuid) -> bool {
        self.stop.load(Ordering::Relaxed)
            || self
                .jobs
                .lock()
                .map_or(true, |jobs| jobs.get(&job).is_none_or(|j| j.stage.ended()))
    }

    /// Change `job`, and write it down on the next turn.
    fn update<T>(&self, job: Uuid, change: impl FnOnce(&mut Job) -> T) -> Option<T> {
        let changed = self.jobs.lock().ok()?.get_mut(&job).map(change);
        if changed.is_some() {
            if let Ok(mut dirty) = self.dirty.lock() {
                dirty.insert(job);
            }
        }
        changed
    }

    /// Change how far `job` has got. Not written down for that alone: it is
    /// worked out again after a restart.
    fn progress<T>(&self, job: Uuid, change: impl FnOnce(&mut Job) -> T) -> Option<T> {
        self.jobs.lock().ok()?.get_mut(&job).map(change)
    }

    /// The link of `job`, if it has one now.
    fn link(&self, job: Uuid) -> Option<Arc<lumit_share::remote::Link>> {
        self.links.lock().ok()?.get(&job).cloned()
    }

    /// Stop the link of `job`, if it has one.
    fn unlink(&self, job: Uuid) {
        let link = self.links.lock().ok().and_then(|mut l| l.remove(&job));
        if let Some(link) = link {
            link.stop();
        }
        if let Ok(mut moving) = self.moving.lock() {
            moving.remove(&job);
        }
    }

    /// Tell the interface something about `job`.
    fn news(&self, kind: BridgeExportNewsKind, job: &Job) {
        let here = matches!(kind, BridgeExportNewsKind::Landed)
            || (!job.sent && job.lands == job::Lands::Worker);
        self.say_news(BridgeExportNews {
            kind,
            comp_name: job.comp_name.clone(),
            file: job.file.clone(),
            path: if here {
                job.path.clone()
            } else {
                String::new()
            },
            other_name: job.other_name.clone(),
            error: job.error.clone(),
            job: job.job.to_string(),
        });
    }

    fn say_news(&self, news: BridgeExportNews) {
        if let Ok(newsbox) = self.newsbox.lock() {
            if let Some(tell) = newsbox.as_ref() {
                tell(news);
            }
        }
    }

    /// The jobs' thread.
    fn run(self: &Arc<Self>, heard: &Receiver<Event>) {
        let mut turned = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            match heard.recv_timeout(TURN) {
                Ok(event) => self.event(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if turned.elapsed() >= TURN {
                turned = Instant::now();
                self.turn();
            }
        }
    }

    fn event(self: &Arc<Self>, event: Event) {
        match event {
            Event::Heard(said) => {
                if let Ok(said) = serde_json::from_str::<Value>(&said) {
                    self.hear(&said);
                    self.follow_room();
                    self.follow_mirrors();
                }
            }
            Event::Command { id, from, body } => {
                let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                self.obey(&id, &from, &body);
            }
            Event::Reply { id, body, error } => self.settle(&id, &body, &error),
            Event::Told(job, news) => self.told(job, news),
            Event::Check {
                computer,
                comp,
                revision,
                doc,
            } => {
                let needs = needs::of(&doc, comp);
                let body = serde_json::json!({ "lumit": "remote-check", "needs": needs });
                self.say(&computer, body, room::For::Check { comp, revision });
            }
        }
    }

    /// A command another computer sent this one. Always answered once.
    fn obey(self: &Arc<Self>, id: &str, from: &str, body: &Value) {
        let job = body
            .get("job")
            .and_then(Value::as_str)
            .and_then(|j| j.parse::<Uuid>().ok());
        let ours = |job: Uuid| self.job(job).filter(|j| j.other == from);
        let lumit = body
            .get("lumit")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let answer: Result<Value, &str> = match (lumit, job) {
            ("remote-check", _) => Ok(self.check(body)),
            ("remote-offer", Some(job)) => self.offer(job, from, body),
            ("remote-status", Some(job)) => match ours(job) {
                Some(j) => {
                    let fetching = body.get("fetching").and_then(Value::as_bool) == Some(true);
                    Ok(self.status_here(&j, fetching))
                }
                None => Err("not_found"),
            },
            // Ended there: cancelled, or given up on for the reason it gives.
            ("remote-cancel", Some(job)) => {
                let error = body
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match ours(job) {
                    // A Bring here that stopped short: its link here goes.
                    Some(j) if j.stage.ended() => self.unlink(job),
                    Some(_) => self.ended_there(job, error),
                    None => {}
                }
                Ok(Value::Null)
            }
            ("remote-pause", Some(job)) => {
                let paused = body.get("paused").and_then(Value::as_bool).unwrap_or(false);
                if ours(job).is_some() {
                    self.pause_here(job, paused);
                }
                Ok(Value::Null)
            }
            ("remote-landed", Some(job)) => {
                if ours(job).is_some() {
                    self.landed_there(job);
                }
                Ok(Value::Null)
            }
            ("remote-fetch", Some(job)) => match ours(job) {
                Some(_) => self.fetch_here(job),
                None => Err("not_found"),
            },
            _ => Err("bad_request"),
        };
        match answer {
            Ok(Value::Null) => self.answer(id, from, None, ""),
            Ok(body) => self.answer(id, from, Some(body), ""),
            Err(error) => self.answer(id, from, None, error),
        }
    }

    /// Turn every job over once.
    fn turn(self: &Arc<Self>) {
        self.resay();
        let account = self.account.lock().map(|a| a.clone()).unwrap_or_default();
        let live: Vec<Job> = self.jobs.lock().map_or_else(
            |_| Vec::new(),
            |jobs| {
                let ours = jobs.values().filter(|j| j.account == account);
                ours.filter(|j| !j.stage.ended()).cloned().collect()
            },
        );
        let now = job::now();
        for old in live
            .iter()
            .filter(|j| now - j.created.max(j.fetched) > WEEK)
        {
            self.give_up(old.job, "lost");
        }
        // The queue turns over when it is read. A worker with no window open
        // still runs what it was sent.
        if live.iter().any(|j| !j.sent) {
            let rows = crate::export::queue_list();
            self.follow_queue(&rows);
        }
        for job in live.iter().filter(|j| j.sent) {
            self.drive(job);
        }
        for job in live.iter().filter(|j| !j.sent) {
            self.keep_linked(job);
        }
        // Awake while files move, which needs the other computer there, or
        // a job's export waits here to run.
        let there = |j: &Job| self.computer(&j.other).is_some_and(|c| c.online);
        let moving = live.iter().any(|j| {
            !j.paused
                && ((matches!(j.stage, Stage::Sending | Stage::Returning) && there(j))
                    || (!j.sent && j.stage == Stage::Queued))
        });
        if let Ok(mut awake) = self.awake.lock() {
            if moving && awake.is_none() {
                *awake = Some(crate::awake::hold());
            } else if !moving {
                *awake = None;
            }
        }
        self.write_down();
    }

    /// Write the jobs changed since the last turn.
    fn write_down(&self) {
        let dirty: Vec<Uuid> = self
            .dirty
            .lock()
            .map(|mut d| d.drain().collect())
            .unwrap_or_default();
        for id in dirty {
            if let Some(job) = self.job(id) {
                job::save(&self.root, &job);
            }
        }
    }

    /// End `job` here as failed with `why`, and tell the other computer.
    fn give_up(&self, job: Uuid, why: &str) {
        // One that ended meanwhile keeps the end it had.
        let Some(j) = self.job(job).filter(|j| !j.stage.ended()) else {
            return;
        };
        if self.unfetch(&j, why) {
            return;
        }
        let Some(other) = self.update(job, |j| {
            j.end(Stage::Failed, why);
            j.other.clone()
        }) else {
            return;
        };
        self.stopped(job);
        self.tell(
            &other,
            "remote-cancel",
            job,
            serde_json::json!({ "error": why }),
        );
        if let Some(j) = self.job(job) {
            self.news(BridgeExportNewsKind::RemoteFailed, &j);
        }
    }

    /// What is left of an ended job's work here: its link, its commands and,
    /// on the worker, its item in the queue and its files.
    fn stopped(&self, job: Uuid) {
        self.unlink(job);
        self.unsay(job);
        if let Some(j) = self.job(job) {
            if !j.sent {
                crate::export::queue_cancel(j.item);
                if let Ok(mut gathering) = self.gathering.lock() {
                    gathering.remove(&job);
                }
                if j.stage != Stage::Done {
                    let _ = self.io.try_send(Io::Forget(job));
                }
            }
        }
    }

    /// The disk thread.
    fn work(self: &Arc<Self>, work: &Receiver<Io>) {
        while !self.stop.load(Ordering::Relaxed) {
            let Ok(next) = work.recv_timeout(TURN) else {
                continue;
            };
            match next {
                Io::Prepare(job, from) => self.prepare(job, from),
                Io::Arrived(job, item, path) => self.arrived(job, item, &path),
                Io::Gather(job) => self.gather(job),
                Io::Forget(job) => self.forget(job),
                Io::Store(all) => {
                    let keep = self.store_needed();
                    if all {
                        files::trim(&self.root, &keep, true);
                    }
                    self.kept
                        .store(files::store_bytes(&self.root), Ordering::Relaxed);
                }
            }
            self.hashes.keep();
        }
    }

    /// Delete what a job left in its folder, besides its record, its log
    /// and, on the sender, its snapshot.
    fn forget(&self, job: Uuid) {
        let dir = job::folder(&self.root, job);
        let keep = ["record.json", "log.txt"];
        let sent = self.job(job).is_some_and(|j| j.sent);
        let gone = self.job(job).is_none();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let snapshot = sent && matches!(name.as_str(), "document.json" | "manifest.json");
            if gone || !(keep.contains(&name.as_str()) || snapshot) {
                let path = entry.path();
                let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
            }
        }
        if gone {
            let _ = std::fs::remove_dir(&dir);
        }
    }

    /// Hashes of the store's files a job that has not ended needs.
    fn store_needed(&self) -> HashSet<String> {
        let live: Vec<Uuid> = self.jobs.lock().map_or_else(
            |_| Vec::new(),
            |jobs| {
                jobs.values()
                    .filter(|j| !j.sent && !j.stage.ended())
                    .map(|j| j.job)
                    .collect()
            },
        );
        let gathering = self.gathering.lock();
        let mut keep = HashSet::new();
        if let Ok(gathering) = gathering {
            for job in live {
                if let Some(g) = gathering.get(&job) {
                    keep.extend(g.manifest.files.iter().map(|f| f.hash.clone()));
                }
            }
        }
        keep
    }
}

// --- What the interface calls -------------------------------------------

pub(crate) fn heard(said: &str) {
    here().heard(said);
}

pub(crate) fn command(id: &str, from: &str, body: &str) {
    here().command(id, from, body);
}

pub(crate) fn reply(id: &str, body: &str, error: &str) {
    here().reply(id, body, error);
}

pub(crate) fn outbox(sink: StreamSink<BridgeRemoteSay>) {
    let remote = here();
    remote.set_outbox(Box::new(move |say| {
        let _ = sink.add(say);
    }));
    // What waited for a way out goes now.
    remote.resay();
}

pub(crate) fn news(sink: StreamSink<BridgeExportNews>) {
    here().set_news(Box::new(move |news| {
        let _ = sink.add(news);
    }));
}

/// An export this computer did for itself ended: written, or `error`.
pub(crate) fn exported(comp_name: &str, path: &str, error: Option<&str>) {
    let Some(remote) = HERE.get() else {
        return;
    };
    let file = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    remote.say_news(BridgeExportNews {
        kind: if error.is_some() {
            BridgeExportNewsKind::Failed
        } else {
            BridgeExportNewsKind::Exported
        },
        comp_name: comp_name.to_owned(),
        file: file.unwrap_or_default(),
        path: path.to_owned(),
        other_name: String::new(),
        error: error.unwrap_or_default().to_owned(),
        job: String::new(),
    });
}

pub(crate) fn state_fields() -> String {
    needs::state_fields()
}

pub(crate) fn account(account: Option<String>) {
    if let Ok(mut now) = here().account.lock() {
        *now = account.unwrap_or_default();
    }
}

pub(crate) fn set_takes(on: bool) {
    here().takes.store(on, Ordering::Relaxed);
}

pub(crate) fn set_folders(folders: Vec<String>) {
    if let Ok(mut now) = here().folders.lock() {
        *now = folders.into_iter().map(PathBuf::from).collect();
    }
}

pub(crate) fn computers() -> Vec<BridgeComputer> {
    here().computers()
}

pub(crate) fn fit(comp: &CompositionReference, computer: &str) -> BridgeRemoteFit {
    here().fit(comp, computer)
}

pub(crate) fn send(
    comp: &CompositionReference,
    computer: &str,
    spec: &BridgeExportSpec,
    path: &str,
    lands: BridgeRemoteLands,
) -> Result<String, BridgeError> {
    here().send(comp, computer, spec, path, lands.into())
}

pub(crate) fn jobs() -> Vec<BridgeRemoteJob> {
    here().jobs()
}

pub(crate) fn cancel(job: &str) {
    if let Ok(job) = job.parse() {
        here().cancel_here(job, true);
    }
}

pub(crate) fn pause(job: &str, paused: bool) {
    if let Ok(job) = job.parse() {
        here().pause(job, paused);
    }
}

pub(crate) fn remove(job: &str) {
    if let Ok(job) = job.parse() {
        here().remove(job);
    }
}

pub(crate) fn again(job: &str) -> Result<String, BridgeError> {
    let job = job.parse().map_err(|_| BridgeError::InvalidItem)?;
    here().again(job, None)
}

pub(crate) fn move_to(job: &str, computer: &str) -> Result<String, BridgeError> {
    let job = job.parse().map_err(|_| BridgeError::InvalidItem)?;
    here().move_to(job, computer)
}

pub(crate) fn fetch(job: &str, path: &str) -> Result<(), BridgeError> {
    let job = job.parse().map_err(|_| BridgeError::InvalidItem)?;
    here().fetch(job, path)
}

pub(crate) fn log(job: &str) -> String {
    let Ok(job) = job.parse() else {
        return String::new();
    };
    let log = job::folder(&here().root, job).join("log.txt");
    if here().job(job).is_some_and(|j| !j.sent) && log.is_file() {
        log.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

pub(crate) fn kept_bytes() -> u64 {
    let remote = here();
    let _ = remote.io.try_send(Io::Store(false));
    remote.kept.load(Ordering::Relaxed)
}

/// A remote export's document as this Lumit reads it, for its export.
pub(crate) fn job_document(folder: &Path, schema: &str) -> Option<lumit_core::model::Document> {
    worker::read_job_document(folder, schema)
}

pub(crate) fn kept_clear() {
    let _ = here().io.try_send(Io::Store(true));
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

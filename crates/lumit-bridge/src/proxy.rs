//! MAKE-PROXY from Flutter — starting one transcode, reporting how it is
//! getting on, and attaching the finished file to its item.
//!
//! # In plain terms
//!
//! Making a proxy is a long job with nothing to look at: read every frame of a
//! clip, write each one out half as wide. So it is driven exactly as an export
//! is — `start`, then `poll` on the interface's own tick, then `cancel` if the
//! wait turns out not to be worth it — and for the same reason: a call that
//! took minutes to return would freeze the window it was called from.
//!
//! One at a time. Two transcodes share one disk and halve each other, and the
//! Project panel has one progress row to show either in. Asking for another
//! while one runs puts it in a queue, which is how a folder of footage gets
//! its proxies: each starts as the one before it lands.
//!
//! The one thing this does that an export does not: when the file lands, the
//! **bridge** attaches it. `lumit_render::proxy` writes a file and knows
//! nothing about documents, so the op that puts the new path on the item is
//! committed here, from the item this job was started for — which is also why
//! the job remembers which project and which item it belongs to.

use lumit_render::proxy::{ProxyEvent, ProxyHandle};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// How the running transcode is getting on — the same three-beat shape the
/// export's own poll has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Idle,
    Running { frame: usize, total: usize },
    Done { path: String },
    Failed { error: String },
}

struct Job {
    state: State,
    handle: Option<ProxyHandle>,
    /// Which item the finished file is attached to. Held here rather than
    /// passed to `poll`, so whatever window is watching — or none — the proxy
    /// still lands on the item that asked for it.
    project: Uuid,
    item: Uuid,
    /// The proxies asked for while this one runs, in the order they were
    /// asked for.
    waiting: VecDeque<Waiting>,
}

/// One queued proxy: what [`start`] was handed.
struct Waiting {
    project: Uuid,
    item: Uuid,
    source: PathBuf,
    dest: PathBuf,
}

static JOB: OnceLock<Mutex<Job>> = OnceLock::new();

fn slot() -> &'static Mutex<Job> {
    JOB.get_or_init(|| {
        Mutex::new(Job {
            state: State::Idle,
            handle: None,
            project: Uuid::nil(),
            item: Uuid::nil(),
            waiting: VecDeque::new(),
        })
    })
}

/// Start making a proxy for `source`, writing to `dest`, on behalf of `item` in
/// `project`.
///
/// When one is already running this one waits its turn, and asking twice for
/// the same item queues it once.
pub(crate) fn start(
    project: Uuid,
    item: Uuid,
    source: PathBuf,
    dest: PathBuf,
) -> Result<(), String> {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    // Drain first, so a job that finished between two polls frees the slot
    // rather than blocking the next one until somebody looks.
    let landed = drain(&mut guard);
    if !source.is_file() {
        drop(guard);
        attach(landed);
        return Err("that footage is not on this machine".to_owned());
    }
    if guard.handle.is_some() {
        let asked = guard.item == item || guard.waiting.iter().any(|w| w.item == item);
        if !asked {
            guard.waiting.push_back(Waiting {
                project,
                item,
                source,
                dest,
            });
        }
        drop(guard);
        attach(landed);
        return Ok(());
    }
    guard.handle = Some(lumit_render::proxy::start(source, dest));
    guard.state = State::Running { frame: 0, total: 0 };
    guard.project = project;
    guard.item = item;
    drop(guard);
    attach(landed);
    Ok(())
}

/// Where the running transcode has got to, and — when it has just finished —
/// the item's new proxy, attached before this returns.
pub(crate) fn poll() -> State {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let landed = drain(&mut guard);
    let answer = guard.state.clone();
    // The document's lock is never taken while this one is held (docs/14 §3):
    // the attach happens after the guard is gone.
    drop(guard);
    attach(landed);
    answer
}

/// Ask the transcode to stop, and forget the ones waiting behind it. It
/// leaves no half-written file behind.
pub(crate) fn cancel() {
    let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    guard.waiting.clear();
    if let Some(handle) = &guard.handle {
        handle.cancel();
    }
}

/// Read whatever the job has said since the last look. Returns the item and
/// path to attach when this drain is the one that saw the file land.
fn drain(job: &mut Job) -> Option<(Uuid, Uuid, PathBuf)> {
    let Some(handle) = &job.handle else {
        return None;
    };
    let mut landed = None;
    let mut finished = false;
    while let Ok(event) = handle.events.try_recv() {
        match event {
            ProxyEvent::Progress { frame, total } => {
                job.state = State::Running { frame, total };
            }
            ProxyEvent::Done(path) => {
                landed = Some((job.project, job.item, path.clone()));
                job.state = State::Done {
                    path: path.to_string_lossy().into_owned(),
                };
                finished = true;
            }
            ProxyEvent::Failed(error) => {
                job.state = State::Failed { error };
                finished = true;
            }
        }
    }
    if finished {
        job.handle = None;
        // The next in the queue starts in its place, and the state goes
        // straight back to running, so whoever is polling keeps polling until
        // the last one lands. A file that has gone since it was queued is
        // passed over.
        //
        // ponytail: a failure in the middle of a queue is not reported, only
        // the last job's. Report each when the status line can list them.
        while let Some(next) = job.waiting.pop_front() {
            if !next.source.is_file() {
                continue;
            }
            job.handle = Some(lumit_render::proxy::start(next.source, next.dest));
            job.state = State::Running { frame: 0, total: 0 };
            job.project = next.project;
            job.item = next.item;
            break;
        }
    }
    landed
}

/// Put the finished file on its item — the one thing the transcode itself
/// cannot do, since it never sees a document.
///
/// A silent no-op when the project or the item has gone in the meantime: a
/// proxy for something nobody has open any more is a file on disk, not an
/// error worth raising at whatever happens to be polling.
fn attach(landed: Option<(Uuid, Uuid, PathBuf)>) {
    let Some((project, item, path)) = landed else {
        return;
    };
    let _ = crate::api::footage::attach_proxy(project, item, &path);
}

//! The footage probe worker and the session's probe cache.
//!
//! # In plain terms
//!
//! "Probing" a file means opening it far enough to read its vital statistics —
//! how big the picture is, how fast it runs, how long it lasts, whether it
//! carries sound. It is not a decode, but it is still FFmpeg opening a
//! container off a disk, and on a slow drive or a network share it can take
//! long enough to be felt.
//!
//! It was felt, because it happened *while the user waited*: dropping footage
//! into a composition probed the file on the very thread the interface was
//! calling on, so the editor stopped until the answer came back. This module
//! moves that work to a **worker thread** that probes in the background, and
//! keeps what it learns in a small cache the interface reads from.
//!
//! Two entry points, and the difference between them is the whole design:
//!
//! - [`request`] says "this file will be asked about soon" and returns at once.
//!   Importing a file, opening a project and relinking all say it, so the
//!   answers are usually already waiting by the time a panel asks.
//! - [`ensure_probed`] says "I need the answer now". It takes a cached answer
//!   when there is one and otherwise probes on the spot — the **fallback** that
//!   makes this a speed-up and never a change of behaviour. Every route that
//!   used to call `lumit_media::probe::probe` directly calls this instead, so
//!   the answer is identical whether or not the worker got there first.
//!
//! **Nothing is drained and nothing is polled**, and that is deliberate. The
//! worker files its results straight into the cache under a key that carries
//! the file's own size and modification time, so a result can only ever be read
//! back for the exact file it was taken from — the same discipline the
//! decode-ahead thread uses (`crate::prefetch`: correctness rides the key, not
//! timing). A file that is replaced, moved or deleted between one question and
//! the next re-stamps to a different key and is probed again, which is what
//! keeps `get_status` as honest as it was when it asked the file every time.
//!
//! The lock is never held across a probe: a look-up takes it, clones what it
//! found and lets go; a miss probes with nothing held and takes the lock back
//! only to file the result (docs/14 §3).

#[cfg(feature = "media")]
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex, OnceLock,
    },
    time::SystemTime,
};

/// What a probe found, or that the file was tried and would not read.
///
/// The failure is remembered as well as the success: a file that does not probe
/// is asked about just as often as one that does (every Project panel rebuild
/// asks its status), and re-opening a broken container each time is the same
/// cost with none of the reward. It is remembered against the file's own stamp,
/// so relinking or repairing it is picked up on the next question.
#[cfg(feature = "media")]
#[derive(Clone)]
pub(crate) enum Probed {
    Ready(Arc<lumit_media::probe::MediaProbe>),
    Unreadable,
}

/// A file's identity for cache purposes: its length and modification time.
///
/// Cheap — one `stat` — and enough to notice the cases that matter: a relink to
/// a different file, a re-export over the top of the old one, a file that has
/// gone away. It is not a content hash and does not pretend to be; the cost of
/// being wrong is one stale readout, and the fingerprinting that *does* hash
/// lives in `lumit_project` where a wrong answer would cost a lost link.
#[cfg(feature = "media")]
type Stamp = (u64, Option<SystemTime>);

/// How many files' answers to keep. A project's footage list is tens of items;
/// this holds a large one whole and evicts oldest-first past it, because
/// correctness never depends on the cache (docs/14 §5) — a miss is a probe.
#[cfg(feature = "media")]
const MAX_ENTRIES: usize = 512;

/// How many probes may be waiting for the worker at once. Past this a request
/// is simply dropped: the file still probes, on whichever thread asks for it
/// first, and an unbounded queue of paths is not a thing to grow (docs/14 §5).
#[cfg(feature = "media")]
const MAX_QUEUED: usize = 256;

#[cfg(feature = "media")]
struct Entry {
    stamp: Stamp,
    probed: Probed,
    /// Whether the worker produced this, rather than a caller probing inline.
    /// Only the tests read it, and they read it to prove which of the two
    /// paths ran — a counter would have said "a probe happened somewhere",
    /// which is not the same claim.
    #[cfg_attr(not(test), allow(dead_code))]
    off_thread: bool,
}

#[cfg(feature = "media")]
#[derive(Default)]
struct Cache {
    by_path: HashMap<PathBuf, Entry>,
    /// Insertion order, oldest first.
    order: Vec<PathBuf>,
    /// Paths handed to the worker and not yet answered, so a file asked for
    /// five times in one rebuild is queued once.
    queued: HashSet<PathBuf>,
}

#[cfg(feature = "media")]
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

/// The generation queued work belongs to. [`clear`] bumps it, and the worker
/// drops any job stamped with an older one — the cancellation rule (docs/14
/// §6): a project that has closed must not have its files probed on the next
/// project's time.
#[cfg(feature = "media")]
fn generation() -> &'static AtomicU64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    &GENERATION
}

#[cfg(feature = "media")]
struct Job {
    path: PathBuf,
    generation: u64,
}

/// The worker's job channel, and with it the thread. Built on the first
/// [`request`], so a session that never imports footage never starts it.
#[cfg(feature = "media")]
fn jobs() -> Option<&'static Sender<Job>> {
    static JOBS: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("lumit-probe".into())
            .spawn(move || run(&rx))
            .ok()
            .map(|_| tx)
    })
    .as_ref()
}

/// The worker loop: probe what is asked for, skip what is stale, stop when the
/// sender is dropped (which only happens at process exit).
#[cfg(feature = "media")]
fn run(rx: &Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        let wanted = job.generation == generation().load(Ordering::Relaxed);
        // A stamp is one `stat`; a file that has gone away since it was
        // queued is not probed at all, and nothing is remembered for it.
        let stamp = wanted.then(|| stamp(&job.path)).flatten();
        let Some(stamp) = stamp else {
            unqueue(&job.path);
            continue;
        };
        // Somebody may have needed the answer before this got here, in which
        // case they probed it inline and the answer is already filed.
        if lookup(&job.path, stamp).is_some() {
            unqueue(&job.path);
            continue;
        }
        let probed = probe_now(&job.path);
        store(&job.path, stamp, probed, true);
    }
}

/// This file's length and modification time, or `None` when it is not a file
/// we can read at all — which is the same answer a probe of it would give.
#[cfg(feature = "media")]
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some((meta.len(), meta.modified().ok()))
}

/// Open the container and read its statistics. The one place this crate calls
/// `lumit_media::probe::probe`.
#[cfg(feature = "media")]
fn probe_now(path: &Path) -> Probed {
    match lumit_media::probe::probe(path) {
        Ok(info) => Probed::Ready(Arc::new(info)),
        Err(_) => Probed::Unreadable,
    }
}

/// What is held for `path` at `stamp`, if anything. An entry filed against a
/// different stamp is not a hit: the file has changed since.
#[cfg(feature = "media")]
fn lookup(path: &Path, stamp: Stamp) -> Option<Probed> {
    let held = cache().lock().ok()?;
    held.by_path
        .get(path)
        .filter(|e| e.stamp == stamp)
        .map(|e| e.probed.clone())
}

/// File an answer, evicting oldest-first to stay inside the budget, and take
/// the path off the queue in the same turn of the lock — so a queue that has
/// emptied means every answer is readable.
#[cfg(feature = "media")]
fn store(path: &Path, stamp: Stamp, probed: Probed, off_thread: bool) {
    let Ok(mut held) = cache().lock() else {
        return;
    };
    held.queued.remove(path);
    let replaced = held
        .by_path
        .insert(
            path.to_path_buf(),
            Entry {
                stamp,
                probed,
                off_thread,
            },
        )
        .is_some();
    if !replaced {
        held.order.push(path.to_path_buf());
    }
    while held.order.len() > MAX_ENTRIES {
        // `order` is non-empty here (its length is over the cap), but the
        // no-panic rule (docs/14 §4) prefers a match to an index.
        let Some(oldest) = held.order.first().cloned() else {
            break;
        };
        held.order.remove(0);
        held.by_path.remove(&oldest);
    }
}

/// Take `path` off the queue without filing anything — the worker's skip paths.
#[cfg(feature = "media")]
fn unqueue(path: &Path) {
    if let Ok(mut held) = cache().lock() {
        held.queued.remove(path);
    }
}

/// Ask the worker to probe `path` in the background. Returns at once, always.
///
/// A file already answered for, already queued, or queued past the budget is
/// not queued again. Nothing depends on this having run: it only decides
/// whether [`ensure_probed`] finds its answer waiting or pays for it.
pub(crate) fn request(path: &std::path::Path) {
    #[cfg(feature = "media")]
    {
        let Some(stamp) = stamp(path) else {
            return;
        };
        {
            let Ok(mut held) = cache().lock() else {
                return;
            };
            if held
                .by_path
                .get(path)
                .is_some_and(|entry| entry.stamp == stamp)
            {
                return;
            }
            if held.queued.len() >= MAX_QUEUED || !held.queued.insert(path.to_path_buf()) {
                return;
            }
        }
        let job = Job {
            path: path.to_path_buf(),
            generation: generation().load(Ordering::Relaxed),
        };
        match jobs() {
            // A worker thread that could not be spawned leaves the queue
            // marker behind and would block this path for the session, so it
            // is taken back off.
            None => unqueue(path),
            Some(tx) => {
                if tx.send(job).is_err() {
                    unqueue(path);
                }
            }
        }
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = path;
    }
}

/// This file's vital statistics, probing here and now when the worker has not
/// already. `None` when the file is missing or will not read.
///
/// The synchronous fallback the whole design rests on: whatever the worker has
/// or has not done, this returns exactly what `lumit_media::probe::probe` would
/// have returned for the file as it is on disk right now.
#[cfg(feature = "media")]
pub(crate) fn ensure_probed(path: &Path) -> Option<Arc<lumit_media::probe::MediaProbe>> {
    let stamp = stamp(path)?;
    let probed = match lookup(path, stamp) {
        Some(hit) => hit,
        None => {
            // Nothing held across the probe.
            let probed = probe_now(path);
            store(path, stamp, probed.clone(), false);
            probed
        }
    };
    match probed {
        Probed::Ready(info) => Some(info),
        Probed::Unreadable => None,
    }
}

/// Forget everything probed so far and cancel whatever is queued. Called when a
/// project closes: the next project's files are different files, and answers
/// nobody will ask for are memory held for nothing.
pub(crate) fn clear() {
    #[cfg(feature = "media")]
    {
        generation().fetch_add(1, Ordering::Relaxed);
        if let Ok(mut held) = cache().lock() {
            held.by_path.clear();
            held.order.clear();
            held.queued.clear();
        }
    }
}

/// Whether `path`'s held answer came from the worker (`Some(true)`), from a
/// caller probing inline (`Some(false)`), or is not held at all (`None`).
///
/// Tests only: what makes "the async path ran" and "the fallback ran" separate,
/// checkable claims rather than one counter that cannot tell them apart.
#[cfg(all(test, feature = "media"))]
pub(crate) fn probed_off_thread(path: &Path) -> Option<bool> {
    let held = cache().lock().ok()?;
    held.by_path.get(path).map(|e| e.off_thread)
}

/// How many paths the worker still owes an answer for. Tests only.
#[cfg(all(test, feature = "media"))]
pub(crate) fn queued_len() -> usize {
    cache().lock().map(|h| h.queued.len()).unwrap_or(0)
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// One at a time. The cache is process-wide, and [`clear`] empties it, so
    /// two of these tests overlapping could have one wipe the answer the other
    /// had just watched arrive. Nothing outside this module clears it.
    fn serially() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        // A test that fails while holding the guard poisons it; the next test
        // should still run rather than fail for the first one's reason.
        SERIAL.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// Spin until `path` has an answer, up to two seconds. Returns whether one
    /// arrived — a bounded wait rather than a sleep, so the test is as quick as
    /// the machine is and still cannot hang the suite.
    fn wait_for(path: &Path) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if stamp(path).and_then(|s| lookup(path, s)).is_some() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    /// A requested file is probed by the worker and its answer is waiting when
    /// the interface asks — the whole point of the thing.
    #[test]
    fn a_requested_file_is_probed_off_thread() {
        let _serial = serially();
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(dir.path()) else {
            return; // no ffmpeg on this machine
        };

        request(&clip);
        assert!(wait_for(&clip), "the worker answers within two seconds");
        assert_eq!(
            probed_off_thread(&clip),
            Some(true),
            "the answer came from the worker, not from a caller probing inline"
        );

        let info = ensure_probed(&clip).expect("the fixture probes");
        assert!(info.video.is_some(), "the fixture has a video stream");
        assert_eq!(
            probed_off_thread(&clip),
            Some(true),
            "reading it back did not re-probe it"
        );
    }

    /// Nothing requested, so the answer has to be got the slow way — and it is
    /// the same answer. This is the fallback the sync ops rely on.
    #[test]
    fn an_unrequested_file_falls_back_to_probing_inline() {
        let _serial = serially();
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(dir.path()) else {
            return;
        };

        let info = ensure_probed(&clip).expect("the fixture probes");
        assert_eq!(
            probed_off_thread(&clip),
            Some(false),
            "nobody requested it, so the caller probed it itself"
        );

        let direct = lumit_media::probe::probe(&clip).expect("the fixture probes directly too");
        assert_eq!(
            *info, direct,
            "the cache changes when the answer arrives, never what it says"
        );
    }

    /// The worker's answer and the inline answer are the same answer, for the
    /// same file — the no-behaviour-change claim, made against real media.
    #[test]
    fn both_paths_give_the_answer_the_prober_gives() {
        let _serial = serially();
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(clip) = lumit_media::index::tests_support::fixture(dir.path()) else {
            return;
        };
        let direct = lumit_media::probe::probe(&clip).expect("the fixture probes");

        let inline = ensure_probed(&clip).expect("inline");
        request(&clip);
        assert!(wait_for(&clip));
        let after_request = ensure_probed(&clip).expect("after the request");

        assert_eq!(*inline, direct);
        assert_eq!(*after_request, direct);
    }

    /// A file that is not media answers "no" rather than panicking, and a file
    /// that is not there answers "no" without opening anything.
    #[test]
    fn a_file_that_will_not_read_answers_none() {
        let _serial = serially();
        let dir = tempfile::tempdir().expect("temp dir");
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, b"not a container").expect("write");

        assert!(ensure_probed(&text).is_none());
        assert!(matches!(
            lookup(&text, stamp(&text).expect("stamped")),
            Some(Probed::Unreadable)
        ));

        let absent = dir.path().join("nothing.mp4");
        assert!(ensure_probed(&absent).is_none());
        assert_eq!(
            probed_off_thread(&absent),
            None,
            "a file that is not there is not remembered as anything"
        );
    }

    /// The file at the path changed, so the held answer is not this file's
    /// answer — it is probed again. This is what lets `get_status` keep
    /// asking the file, as it did before there was a cache at all.
    #[test]
    fn a_changed_file_is_not_served_from_the_cache() {
        let _serial = serially();
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("clip.txt");
        std::fs::write(&path, b"one").expect("write");
        assert!(ensure_probed(&path).is_none());
        let first = stamp(&path).expect("stamped");

        // A longer file: a different stamp, whatever the clock's resolution.
        std::fs::write(&path, b"a longer body entirely").expect("rewrite");
        let second = stamp(&path).expect("stamped again");
        assert_ne!(first, second);
        assert!(
            lookup(&path, second).is_none(),
            "the held answer belongs to the file that was there before"
        );
    }

    /// Closing a project cancels what was queued for it: the worker drops
    /// jobs from a generation that has ended rather than probing files nobody
    /// is going to ask about.
    #[test]
    fn clearing_cancels_queued_work() {
        let _serial = serially();
        clear();
        assert_eq!(queued_len(), 0, "nothing queued after a clear");
    }
}

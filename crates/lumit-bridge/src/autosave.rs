//! The autosave scheduler: the timer behind the rotating copies
//! (docs/10-FILE-FORMAT.md §4).
//!
//! # In plain terms
//!
//! `lumit_project::autosave` knows *how* to write a spare copy of a project;
//! nothing until now knew *when*. This is the when. One thread wakes up every
//! so often, looks at every open project, and writes a copy of the ones that
//! have actually changed — so a crash costs at most the interval, and a session
//! spent reading a project costs nothing at all.
//!
//! Three rules it keeps, and they are the whole design:
//!
//! - **Nothing is written unless the document moved.** A project matching its
//!   last save, or matching its own last autosave, is skipped. Otherwise an
//!   editor left open overnight would rewrite the same five copies until the
//!   real work was rotated off the end of them — the opposite of a safety net.
//! - **A project with nowhere to write is skipped.** Autosaves live beside the
//!   project file, so an unsaved project has no folder to put them in. The
//!   crash journal is what covers that case (docs/10 §4).
//! - **No lock is held across the write.** The decision and the snapshot are
//!   taken under the project's read guard, the guard is dropped, and only then
//!   does anything touch the disk — the rule from docs/14 §5, and the shape
//!   `measure_document` was corrected into (9d96a24f). A save is seconds of
//!   serialising and an fsync; a read guard held across it is the interface
//!   frozen, because a Rust `RwLock` turns new readers away once a writer is
//!   waiting.
//!
//! The interval and the keep count are **application settings, not project
//! data** — the same arrangement as the cache budgets and the audio device
//! (K-586). The frontend owns the file they live in and hands them over at boot
//! and on every change ([`schedule`]); the engine holds the live values with no
//! store behind them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, Once};
use std::time::Duration;

use uuid::Uuid;

/// How long the thread will sleep in one go. An interval shorter than this is
/// slept whole, which is what lets a test use a clock of a few milliseconds
/// without a fake one; a longer interval is slept a second at a time so that
/// turning autosave off, or shortening it, is honoured within the second rather
/// than at the end of a wait that could be half an hour.
const MAX_TICK: Duration = Duration::from_secs(1);

/// The interval and the keep count, as the frontend last set them.
struct Schedule {
    /// Zero means off, and off is a setting a user is entitled to hold.
    every: Duration,
    keep: u32,
}

/// docs/10 §4's defaults, in force until the frontend says otherwise.
static SCHEDULE: LazyLock<Mutex<Schedule>> = LazyLock::new(|| {
    Mutex::new(Schedule {
        every: Duration::from_secs(5 * 60),
        keep: 5,
    })
});

/// The store revision each project's last autosave was written from. A leaf
/// lock: nothing taken here ever reaches back for a project's lock, and it is
/// released before one is taken (the lock order in `api/state.rs`).
static LAST: LazyLock<Mutex<BTreeMap<Uuid, u64>>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

static THREAD: Once = Once::new();

/// A poisoned leaf mutex is not a reason to stop autosaving: the data behind
/// both of these is bookkeeping, and the worst a panicking thread can have left
/// is a stale revision number, which costs one extra copy.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Set the cadence and start the timer if it is not already running.
///
/// `minutes` of 0 turns autosave off; the thread stays alive and idle, because
/// the setting can be turned back on and a thread that had exited could not
/// hear it.
pub(crate) fn schedule(minutes: u32, keep: u32) {
    schedule_every(Duration::from_secs(u64::from(minutes) * 60), keep);
}

/// The same, in the unit the timer actually works in. Whole minutes is all the
/// setting offers, so this is also the shortened clock the timer's test runs
/// on — a test that waited for a real interval would take a real minute.
fn schedule_every(every: Duration, keep: u32) {
    {
        let mut sched = lock(&SCHEDULE);
        sched.every = every;
        sched.keep = keep.max(1);
    }
    THREAD.call_once(|| {
        // Named, because a thread that appears in a debugger as `<unnamed>` is
        // a thread somebody has to identify by what it is doing.
        let _ = std::thread::Builder::new()
            .name("lumit-autosave".into())
            .spawn(run);
    });
}

/// The timer. Sleeps, counts, sweeps — for the life of the process.
fn run() {
    let mut waited = Duration::ZERO;
    loop {
        // How long to sleep is decided from the setting as it stands; whether
        // to write is decided from the setting as it stands *afterwards*. The
        // two readings matter: a copy written on the strength of an interval
        // the user turned off while the thread was asleep is a copy they asked
        // not to have.
        let tick = match now().0 {
            every if every.is_zero() => MAX_TICK,
            every => every.min(MAX_TICK),
        };
        std::thread::sleep(tick);

        let (every, keep) = now();
        if every.is_zero() {
            waited = Duration::ZERO;
            continue;
        }
        waited += tick;
        if waited >= every {
            waited = Duration::ZERO;
            sweep(keep);
        }
    }
}

/// The cadence as it stands.
fn now() -> (Duration, u32) {
    let sched = lock(&SCHEDULE);
    (sched.every, sched.keep)
}

/// One round: every open project considered, the moved ones written.
///
/// Returns what was written, which is what the tests read. The registry guard
/// is taken, copied out and dropped before any project's own lock — the lock
/// order rule in `api/state.rs`.
pub(crate) fn sweep(keep: u32) -> Vec<PathBuf> {
    let open: Vec<Uuid> = match crate::api::state::PROJECTS.read() {
        Ok(projects) => projects.keys().copied().collect(),
        Err(_) => return Vec::new(),
    };
    // A closed project's mark is dead weight; without this a process that
    // opened projects all day would carry a revision for every one of them.
    lock(&LAST).retain(|id, _| open.contains(id));

    open.into_iter()
        .filter_map(|id| sweep_one(id, keep))
        .collect()
}

/// Consider one project. `None` when there was nothing to write, or when the
/// write failed — a failed autosave is not an error anybody can act on, and the
/// next round will try again.
///
// ponytail: an autosave does not refresh the welcome screen's thumbnail
// (K-667). The file it would write is named by a digest the *frontend* owns
// (`Workspace.thumbnailKey`) in a folder the frontend owns, so writing it here
// would make one filename two sources of truth. The trigger for changing that
// is somebody minding that a picture can be an editing session stale; the
// upgrade is an "autosaved" event on the change stream that the frontend
// answers by drawing one, not a write made from this thread.
pub(crate) fn sweep_one(id: Uuid, keep: u32) -> Option<PathBuf> {
    let state = {
        let projects = crate::api::state::PROJECTS.read().ok()?;
        projects.get(&id).cloned()?
    };
    let marked = lock(&LAST).get(&id).copied();

    // Under the guard: the decision, the path, and an `Arc` clone of the
    // document. Nothing slow, and nothing that touches a disk.
    let (document, target, revision) = {
        let state = state.read().ok()?;
        let revision = state.store.revision();
        // Unmoved since the last save, or since the last autosave: there is
        // nothing a copy would preserve.
        if revision == state.saved_revision || marked == Some(revision) {
            return None;
        }
        let target = state.path.clone()?;
        (state.store.snapshot(), target, revision)
    };

    // Everything from here is outside the lock.
    let dir = target.parent().unwrap_or(Path::new(""));
    let document = lumit_project::rebase_for_save(&document, dir);
    let written = lumit_project::autosave(&document, &target, keep.max(1) as usize).ok()?;
    lock(&LAST).insert(id, revision);
    Some(written)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::api::state::LumitBridgeState;

    /// The tests in this module run one at a time. The schedule and the timer
    /// thread are process-wide, and a timer running at test speed would write
    /// copies of the projects the other tests are counting.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// A project saved to `dir/scene.lum`, with a comp in it.
    fn saved_project(dir: &Path) -> crate::api::project::ProjectReference {
        std::fs::create_dir_all(dir).expect("temp dir");
        let project = LumitBridgeState::new_project(None).expect("a new project");
        project.new_composition("Scene".into(), None).expect("comp");
        project
            .save(dir.join("scene.lum").to_string_lossy().into_owned())
            .expect("saved");
        project
    }

    /// **The revision is the gate.** A project that has not changed since its
    /// last save is not copied, and neither is one that has not changed since
    /// its own last autosave — otherwise an editor left open would rotate the
    /// real work off the end of the copies it was supposed to be protecting.
    #[test]
    fn nothing_is_written_until_the_document_moves() {
        let _turn = lock(&ONE_AT_A_TIME);
        let dir = std::env::temp_dir().join("lumit-autosave-gate");
        std::fs::remove_dir_all(&dir).ok();
        let project = saved_project(&dir);

        assert_eq!(
            sweep_one(project.id, 3),
            None,
            "saved a moment ago: a copy would preserve nothing"
        );

        project.new_composition("Later".into(), None).expect("comp");
        let written = sweep_one(project.id, 3).expect("the document moved, so it is copied");
        assert!(written.is_file());

        assert_eq!(
            sweep_one(project.id, 3),
            None,
            "and it is not copied twice for the same edit"
        );

        project.close().expect("closed");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **A project with nowhere to write is skipped**, calmly: autosaves live
    /// beside the project file, and one that has never been saved has no
    /// folder to put them in. The crash journal covers that case.
    #[test]
    fn a_project_that_was_never_saved_is_left_alone() {
        let project = LumitBridgeState::new_project(None).expect("a new project");
        project.new_composition("Scene".into(), None).expect("comp");
        assert_eq!(sweep_one(project.id, 3), None);
        project.close().expect("closed");
    }

    /// **The rotation is the one the recovery dialogue reads** (K-488): slots
    /// numbered from 1 in an `autosaves/` folder, 1 the newest, contiguous —
    /// which is what `list_autosaves` walks and what `latest_autosave` offers.
    /// The keep count is honoured, so the folder never grows without end.
    #[test]
    fn the_rotation_keeps_what_it_was_told_and_recovery_finds_it() {
        let _turn = lock(&ONE_AT_A_TIME);
        let dir = std::env::temp_dir().join("lumit-autosave-rotation");
        std::fs::remove_dir_all(&dir).ok();
        let project = saved_project(&dir);

        for n in 0..4 {
            project
                .new_composition(format!("Comp {n}"), None)
                .expect("comp");
            sweep_one(project.id, 2).expect("each edit is copied");
        }

        let listed =
            crate::api::shell::list_autosaves(dir.join("scene.lum").to_string_lossy().into_owned());
        assert_eq!(listed.len(), 2, "keep 2 means two slots, not four");
        assert_eq!(listed[0].slot, 1, "and slot 1 is the newest");
        assert!(
            lumit_project::latest_autosave(&dir.join("scene.lum")).is_some(),
            "the recovery dialogue's own reader finds it"
        );

        project.close().expect("closed");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **The interval is honoured**, on a clock shortened to milliseconds: the
    /// timer writes nothing before it is due, writes once it is, and writes
    /// nothing at all once the interval is set to zero — which is what a user
    /// turning autosave off is entitled to expect.
    #[test]
    fn the_timer_waits_for_its_interval_and_off_means_off() {
        let _turn = lock(&ONE_AT_A_TIME);
        let dir = std::env::temp_dir().join("lumit-autosave-clock");
        std::fs::remove_dir_all(&dir).ok();
        let project = saved_project(&dir);
        let autosaves = dir.join("autosaves");

        // The thread reads the schedule every round, so this is the whole
        // clock: a fifth of a second stands in for five minutes.
        schedule_every(Duration::from_millis(200), 3);
        project
            .new_composition("Waiting".into(), None)
            .expect("comp");
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            !autosaves.exists(),
            "nothing is written before the interval is up"
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(autosaves.is_dir(), "and it is written once it is");

        // Off: no further copy, however much the document moves. The count is
        // taken after a round has gone by, so a sweep already under way when
        // the setting changed is behind us and what follows is the answer to
        // "off", not to the last tick of "on".
        schedule_every(Duration::ZERO, 3);
        std::thread::sleep(Duration::from_millis(300));
        let before = std::fs::read_dir(&autosaves).expect("the folder").count();
        project
            .new_composition("Ignored".into(), None)
            .expect("comp");
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(
            std::fs::read_dir(&autosaves).expect("the folder").count(),
            before,
            "off is off"
        );

        project.close().expect("closed");
        std::fs::remove_dir_all(&dir).ok();
    }
}

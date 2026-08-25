//! The evaluation worker pool (docs/05-ARCHITECTURE.md §2,
//! docs/impl/playback-scheduler.md §2): the fixed set of threads that runs
//! evaluation-graph jobs, with two priority classes — *interactive* work
//! (the frame under the playhead, a scrub, audio-adjacent) always taken
//! before *background* work (cache warming, thumbnails) at every job
//! boundary.
//!
//! In plain terms: the pool is a small crew of workers and two in-trays —
//! an "urgent" tray and an "everything else" tray. Whenever a worker
//! finishes a job it always empties the urgent tray first, so a scrub never
//! queues behind cache warming. The trays have fixed sizes on purpose
//! (docs/impl/playback-scheduler.md: *bounded everything*): when a tray is
//! full, submitting more work fails fast and the caller decides what to do —
//! work can never pile up unboundedly behind a stall. Cancellation is not
//! the pool's job: every job carries an [`crate::epoch::EpochToken`] and
//! stops itself between small steps.
//!
//! The threads themselves belong to a dedicated [`rayon`] pool (never the
//! global one), as docs/impl/playback-scheduler.md pins: rayon's
//! work-stealing is the right substrate once graph evaluation fans out into
//! per-node and per-tile subtasks (`rayon::scope` inside a job steals across
//! the same threads). The priority discipline sits in front of rayon: jobs
//! wait in the class queues, and up to `threads` *pump* tasks drain them,
//! urgent-first, one job at a time.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

/// Queue capacities (documented constants per docs/impl/playback-scheduler.md
/// §2: choose capacities once, in one place). Interactive stays small — it
/// only ever holds the handful of jobs one interaction can produce; a full
/// interactive queue means the pool is hopelessly behind and the caller
/// should drop, not queue. Background is the cache-warming horizon.
pub const INTERACTIVE_QUEUE_CAP: usize = 64;
/// See [`INTERACTIVE_QUEUE_CAP`].
pub const BACKGROUND_QUEUE_CAP: usize = 256;

/// Worker-thread count for a machine with `cores` logical cores:
/// `cores − 3, min 2` (docs/impl/playback-scheduler.md §2 — the three
/// reserved cores are the UI thread, the GPU-submit thread and the OS/audio
/// headroom).
#[must_use]
pub fn worker_threads(cores: usize) -> usize {
    cores.saturating_sub(3).max(2)
}

/// The two priority classes (docs/05-ARCHITECTURE.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobClass {
    /// The current Viewer frame, scrub, audio-adjacent work. Always taken
    /// before background at every job boundary.
    Interactive,
    /// Cache warming, thumbnails, proxy checks.
    Background,
}

/// The class queue was full — back-pressure by design. The caller decides
/// whether to drop the work, retry later, or degrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolFull(pub JobClass);

impl std::fmt::Display for PoolFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            JobClass::Interactive => f.write_str("interactive job queue full"),
            JobClass::Background => f.write_str("background job queue full"),
        }
    }
}

impl std::error::Error for PoolFull {}

/// The pool could not be built (thread spawn failure at startup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolBuildError(String);

impl std::fmt::Display for PoolBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "worker pool: {}", self.0)
    }
}

impl std::error::Error for PoolBuildError {}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Everything the pumps and submitters share, behind one mutex. `pumps` is
/// bookkept under the same lock as the queues so a pump deciding to exit and
/// a submitter deciding whether to start one can never miss each other.
struct Inner {
    interactive: VecDeque<Job>,
    background: VecDeque<Job>,
    /// Pump tasks currently alive (≤ `threads`).
    pumps: usize,
    /// Jobs that panicked (contained; the pump survives). Engine code is
    /// panic-free by lint, so this counting is defensive, not expected.
    panicked: u64,
}

/// Recover from mutex poisoning instead of panicking: the guarded state is a
/// pair of queues and two counters, all valid at every instant a panic could
/// have interrupted (no multi-step invariants), so continuing is sound — and
/// the no-panics rule (docs/14 §4) leaves no alternative.
fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The two-priority evaluation worker pool. See the module docs.
pub struct WorkerPool {
    threads: usize,
    pool: rayon::ThreadPool,
    inner: Arc<Mutex<Inner>>,
}

impl WorkerPool {
    /// A pool with exactly `threads` workers (floored at 1). Prefer
    /// [`WorkerPool::with_default_threads`] outside tests.
    pub fn new(threads: usize) -> Result<Self, PoolBuildError> {
        let threads = threads.max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("lumit-eval-worker-{i}"))
            .build()
            .map_err(|e| PoolBuildError(e.to_string()))?;
        Ok(Self {
            threads,
            pool,
            inner: Arc::new(Mutex::new(Inner {
                interactive: VecDeque::new(),
                background: VecDeque::new(),
                pumps: 0,
                panicked: 0,
            })),
        })
    }

    /// A pool sized for this machine: `cores − 3, min 2`
    /// (docs/impl/playback-scheduler.md §2).
    pub fn with_default_threads() -> Result<Self, PoolBuildError> {
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        Self::new(worker_threads(cores))
    }

    /// Number of worker threads.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Submit a job, failing fast when its class queue is full. Jobs carry
    /// their own [`crate::epoch::EpochToken`] for cancellation; the pool
    /// never kills anything.
    pub fn try_spawn(
        &self,
        class: JobClass,
        job: impl FnOnce() + Send + 'static,
    ) -> Result<(), PoolFull> {
        let start_pump = {
            let mut inner = lock(&self.inner);
            let (queue, cap) = match class {
                JobClass::Interactive => (&mut inner.interactive, INTERACTIVE_QUEUE_CAP),
                JobClass::Background => (&mut inner.background, BACKGROUND_QUEUE_CAP),
            };
            if queue.len() >= cap {
                return Err(PoolFull(class));
            }
            queue.push_back(Box::new(job));
            // Start another pump only when one can actually run concurrently;
            // decided under the same lock a retiring pump decrements under, so
            // a job can never be left queued with no pump due to a race.
            if inner.pumps < self.threads {
                inner.pumps += 1;
                true
            } else {
                false
            }
        };
        if start_pump {
            let inner = Arc::clone(&self.inner);
            self.pool.spawn(move || pump(&inner));
        }
        Ok(())
    }

    /// Currently queued (interactive, background) — diagnostics and tests.
    #[must_use]
    pub fn queued(&self) -> (usize, usize) {
        let inner = lock(&self.inner);
        (inner.interactive.len(), inner.background.len())
    }

    /// Jobs whose panic was contained by a pump. Always 0 in panic-free
    /// engine code; nonzero means a bug worth chasing.
    #[must_use]
    pub fn panicked_jobs(&self) -> u64 {
        lock(&self.inner).panicked
    }
}

/// One pump: drain jobs urgent-first until both queues are empty, then
/// retire. Runs on a rayon worker thread, so `rayon::scope` inside a job
/// fans out across the same pool (work-stealing does the rest).
fn pump(inner: &Mutex<Inner>) {
    loop {
        let job = {
            let mut g = lock(inner);
            match g
                .interactive
                .pop_front()
                .or_else(|| g.background.pop_front())
            {
                Some(job) => job,
                None => {
                    g.pumps -= 1;
                    return;
                }
            }
        };
        // Contain a panicking job so the pump (and its bookkeeping) survives.
        // Sound to continue past: the job owned its state; the shared Inner is
        // not held across the call.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            lock(inner).panicked += 1;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn thread_count_is_cores_minus_three_floored_at_two() {
        assert_eq!(worker_threads(32), 29);
        assert_eq!(worker_threads(8), 5);
        assert_eq!(worker_threads(5), 2);
        assert_eq!(worker_threads(4), 2);
        assert_eq!(worker_threads(2), 2);
        assert_eq!(worker_threads(1), 2);
    }

    #[test]
    fn every_submitted_job_runs() {
        let pool = WorkerPool::new(2).unwrap();
        let ran = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        for i in 0..50 {
            let ran = Arc::clone(&ran);
            let tx = tx.clone();
            let class = if i % 2 == 0 {
                JobClass::Interactive
            } else {
                JobClass::Background
            };
            pool.try_spawn(class, move || {
                ran.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(());
            })
            .unwrap();
        }
        for _ in 0..50 {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(ran.load(Ordering::Relaxed), 50);
        assert_eq!(pool.queued(), (0, 0));
    }

    /// The load-bearing rule: at a job boundary, queued interactive work runs
    /// before queued background work, whatever order it arrived in. A
    /// single-worker pool is held busy while both classes queue up, then
    /// released — the interactive job must come out first.
    #[test]
    fn interactive_preempts_background_at_job_boundaries() {
        let pool = WorkerPool::new(1).unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let (blocker_started_tx, blocker_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel();

        pool.try_spawn(JobClass::Background, move || {
            let _ = blocker_started_tx.send(());
            let _ = release_rx.recv();
        })
        .unwrap();
        blocker_started_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        // Queue background first, interactive second — arrival order must not win.
        for (class, label) in [
            (JobClass::Background, "background"),
            (JobClass::Interactive, "interactive"),
        ] {
            let order = Arc::clone(&order);
            let done_tx = done_tx.clone();
            pool.try_spawn(class, move || {
                order.lock().unwrap().push(label);
                let _ = done_tx.send(());
            })
            .unwrap();
        }
        release_tx.send(()).unwrap();
        for _ in 0..2 {
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec!["interactive", "background"]);
    }

    /// Bounded everything: a full class queue rejects instead of growing.
    #[test]
    fn a_full_queue_fails_fast() {
        let pool = WorkerPool::new(1).unwrap();
        let (blocker_started_tx, blocker_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        pool.try_spawn(JobClass::Interactive, move || {
            let _ = blocker_started_tx.send(());
            let _ = release_rx.recv();
        })
        .unwrap();
        blocker_started_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        for _ in 0..INTERACTIVE_QUEUE_CAP {
            pool.try_spawn(JobClass::Interactive, || {}).unwrap();
        }
        assert_eq!(
            pool.try_spawn(JobClass::Interactive, || {}),
            Err(PoolFull(JobClass::Interactive))
        );
        // The other class is unaffected by this one's back-pressure.
        pool.try_spawn(JobClass::Background, || {}).unwrap();
        release_tx.send(()).unwrap();
    }

    /// A panicking job is contained: the pump survives, later jobs still run,
    /// and the panic is counted.
    #[test]
    fn a_panicking_job_does_not_take_the_worker_down() {
        let pool = WorkerPool::new(1).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        pool.try_spawn(JobClass::Interactive, || panic!("job bug"))
            .unwrap();
        pool.try_spawn(JobClass::Interactive, move || {
            let _ = done_tx.send(());
        })
        .unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(pool.panicked_jobs(), 1);
    }
}

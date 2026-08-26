//! `OfxMultiThreadSuiteV1` — the host lending a plugin its threads.
//!
//! # In plain terms
//!
//! A plugin that wants to use more than one core does not start threads of its
//! own; it hands the host a function and says "run this *n* times, and tell
//! each run which one it is". The host runs them on **its** threads, which is
//! the whole point: Lumit already knows how many cores it is willing to spend
//! and has a pool sized for it, and a plugin starting eight threads of its own
//! inside a pool that is already eight wide is how an editor stops responding.
//!
//! Two of the entry points look trivial and are not:
//!
//! * **`multiThreadNumCPUs` must be honest.** A plugin sizes its scratch
//!   buffers from this number, so answering "however many cores this machine
//!   has" when the host will only ever run four of them wastes memory the
//!   governor then has to find. The answer here is the same sum the evaluation
//!   pool uses — `cores − 3`, floored at two (docs/impl/playback-scheduler.md
//!   §2) — because that is the truth about how wide the host actually goes.
//! * **`multiThreadIndex` must be right.** Plugins index per-thread scratch by
//!   it: two threads told they are both number three write over each other, and
//!   the picture comes out with a band in it on some runs and not others. It is
//!   set once, on the thread, for the duration of the one call.
//!
//! The mutex half is exactly what it looks like: thin wrappers over
//! `parking_lot`. They are here because a plugin may not assume the host is
//! POSIX or Windows, not because a lock is hard.

use std::ffi::{c_int, c_uint, c_void};
use std::sync::{Arc, OnceLock};

use parking_lot::lock_api::RawMutex as RawMutexApi;
use parking_lot::RawMutex;

use crate::ffi::{OfxMultiThreadSuiteV1, OfxMutexHandle, OfxThreadFunctionV1};
use crate::handles::{Handle, HandleKind};
use crate::host::state;
use crate::status::Status;
use crate::suites::{guard, guard_code, out_handle};

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxMultiThreadSuiteV1 = OfxMultiThreadSuiteV1 {
    multi_thread,
    multi_thread_num_cpus,
    multi_thread_index,
    multi_thread_is_spawned_thread,
    mutex_create,
    mutex_destroy,
    mutex_lock,
    mutex_un_lock,
    mutex_try_lock,
};

thread_local! {
    /// Which of a `multiThread` fan-out this thread is, while it is inside one.
    static THREAD_INDEX: std::cell::Cell<Option<c_uint>> = const { std::cell::Cell::new(None) };
}

/// The pool the fan-out runs on: a dedicated one, never rayon's global, for the
/// same reason the evaluation pool is dedicated — a plugin's work must not be
/// able to starve anything else that reaches for rayon
/// (docs/impl/playback-scheduler.md §2).
///
/// `None` means the pool would not build, and the fan-out runs on the calling
/// thread instead. That is slower and completely correct; a plugin cannot tell
/// the difference except by the clock.
static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();

fn pool() -> Option<&'static rayon::ThreadPool> {
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(host_thread_count())
            .thread_name(|index| format!("lumit-ofx-plugin-{index}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// How wide this host will actually go, which is what `multiThreadNumCPUs`
/// answers.
#[must_use]
pub fn host_thread_count() -> usize {
    let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
    lumit_eval::pool::worker_threads(cores)
}

/// The plugin's `customArg`, carried across threads.
///
/// It is the plugin's own pointer and this host never follows it — it is passed
/// straight back into the plugin's own function on each thread, which is what
/// the argument is for.
struct CustomArg(*mut c_void);

impl CustomArg {
    /// The address, for handing back. A method rather than a field read so
    /// that a closure capturing it captures the *wrapper* — capturing the bare
    /// pointer field would drop the `Send` this type exists to provide.
    const fn get(&self) -> *mut c_void {
        self.0
    }
}

// SAFETY: the pointer is opaque to the host: it is never read, written or
// freed here, only handed back to the plugin that supplied it. OFX requires
// `customArg` to be safe for the plugin's own function to use from every thread
// of the fan-out — that is the contract of `multiThread` — so sharing the
// address is sharing exactly what the plugin asked us to share.
unsafe impl Send for CustomArg {}
// SAFETY: as above.
unsafe impl Sync for CustomArg {}

unsafe extern "C" fn multi_thread(
    func: OfxThreadFunctionV1,
    n_threads: c_uint,
    custom_arg: *mut c_void,
) -> c_int {
    guard(|| {
        if n_threads == 0 {
            return Err(Status::ErrValue);
        }
        // Nesting a fan-out inside a fan-out is the plugin's mistake and OFX
        // names the code for it. Running it anyway would square the thread
        // count against a pool that is already full.
        if THREAD_INDEX.with(std::cell::Cell::get).is_some() {
            return Err(Status::ErrExists);
        }
        let arg = CustomArg(custom_arg);

        // One thread is not a fan-out, and a plugin that asks for one is
        // asking to be called: doing it here saves a hop and keeps the
        // no-pool fallback path and this path the same shape.
        let run = move || {
            let body = |index: c_uint| {
                THREAD_INDEX.with(|slot| slot.set(Some(index)));
                // SAFETY: the plugin's own function, called with the arguments
                // OFX declares for it and the pointer it gave us. It may panic
                // only by being Rust, which it is not; a foreign unwind cannot
                // cross back through here because `guard` is outside the scope.
                unsafe { func(index, n_threads, arg.get()) };
                THREAD_INDEX.with(|slot| slot.set(None));
            };
            if n_threads == 1 {
                body(0);
                return;
            }
            rayon::scope(|scope| {
                for index in 0..n_threads {
                    let body = &body;
                    scope.spawn(move |_| body(index));
                }
            });
        };

        match pool() {
            Some(pool) => pool.install(run),
            None => run(),
        }
        Ok(())
    })
}

unsafe extern "C" fn multi_thread_num_cpus(n_cpus: *mut c_uint) -> c_int {
    guard(|| {
        if n_cpus.is_null() {
            return Err(Status::ErrValue);
        }
        let count = c_uint::try_from(host_thread_count()).unwrap_or(1).max(1);
        // SAFETY: the plugin's out-parameter, checked non-null above.
        unsafe { *n_cpus = count };
        Ok(())
    })
}

unsafe extern "C" fn multi_thread_index(thread_index: *mut c_uint) -> c_int {
    guard(|| {
        if thread_index.is_null() {
            return Err(Status::ErrValue);
        }
        // A thread that is not one of ours has no index, and saying "nought"
        // would be the wrong answer twice over: it is not true, and nought is
        // the one index a plugin is most likely to treat as special.
        let index = THREAD_INDEX
            .with(std::cell::Cell::get)
            .ok_or(Status::ErrBadIndex)?;
        // SAFETY: the plugin's out-parameter, checked non-null above.
        unsafe { *thread_index = index };
        Ok(())
    })
}

unsafe extern "C" fn multi_thread_is_spawned_thread() -> c_int {
    match std::panic::catch_unwind(|| THREAD_INDEX.with(std::cell::Cell::get).is_some()) {
        Ok(true) => 1,
        _ => 0,
    }
}

// ------------------------------------------------------------------ mutexes --

/// One lock the host holds for a plugin.
///
/// The raw lock rather than `parking_lot::Mutex` because OFX separates locking
/// from unlocking into two calls: there is no guard to hold between them, and
/// nothing for the lock to guard on this side — the plugin's data is the
/// plugin's.
pub struct HostMutex {
    raw: RawMutex,
}

fn mutex_of(handle: OfxMutexHandle) -> Result<Arc<HostMutex>, Status> {
    let handle = Handle::from_ptr(handle);
    if handle.kind() != Some(HandleKind::Mutex) {
        return Err(Status::ErrBadHandle);
    }
    // Cloned out and the guard dropped before anything blocks: the host lock is
    // never held across a wait (docs/14 §7).
    Ok(Arc::clone(state().mutexes.get(handle)?))
}

unsafe extern "C" fn mutex_create(mutex: *mut OfxMutexHandle, lock_count: c_int) -> c_int {
    guard(|| {
        let created = Arc::new(HostMutex {
            raw: RawMutex::INIT,
        });
        // `lockCount` is how many times the mutex arrives already locked.
        // Anything past one would need a recursive lock, which no plugin in
        // the test bench asks for; one is honoured and more is refused rather
        // than silently rounded down.
        if lock_count > 1 {
            return Err(Status::ErrUnsupported);
        }
        if lock_count == 1 {
            created.raw.lock();
        }
        let handle = state().mutexes.insert(created)?;
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(mutex, handle) }
    })
}

unsafe extern "C" fn mutex_destroy(mutex: OfxMutexHandle) -> c_int {
    guard(|| {
        let handle = Handle::from_ptr(mutex);
        if handle.kind() != Some(HandleKind::Mutex) {
            return Err(Status::ErrBadHandle);
        }
        state().mutexes.remove(handle)?;
        Ok(())
    })
}

unsafe extern "C" fn mutex_lock(mutex: OfxMutexHandle) -> c_int {
    guard(|| {
        mutex_of(mutex)?.raw.lock();
        Ok(())
    })
}

unsafe extern "C" fn mutex_un_lock(mutex: OfxMutexHandle) -> c_int {
    guard(|| {
        let mutex = mutex_of(mutex)?;
        if !mutex.raw.is_locked() {
            // Unlocking a lock nobody holds is the plugin's bug, and unlocking
            // it anyway is undefined behaviour. A status is the only safe
            // answer.
            return Err(Status::ErrValue);
        }
        // SAFETY: `unlock` requires the lock to be held. It is: the check
        // above saw it locked, and the only code that can unlock one of these
        // is this function — so a race here can only be a plugin unlocking the
        // same mutex from two threads, which is already the plugin breaking the
        // contract it was handed, and which `is_locked` catches on every
        // ordering that leaves the lock free.
        unsafe { mutex.raw.unlock() };
        Ok(())
    })
}

unsafe extern "C" fn mutex_try_lock(mutex: OfxMutexHandle) -> c_int {
    guard_code(|| match mutex_of(mutex) {
        Ok(mutex) => {
            if mutex.raw.try_lock() {
                Status::Ok
            } else {
                // The spec's answer for "somebody else has it".
                Status::Failed
            }
        }
        Err(status) => status,
    })
}

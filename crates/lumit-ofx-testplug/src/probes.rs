//! The exports no real plugin has, all named `LumitTestPlug`: how a test asks
//! what the plugins here were given and did.

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, PoisonError};

use lumit_ofx::ffi::OfxMultiThreadSuiteV1;
use lumit_ofx::status::Status;

use crate::{
    suite, ACTION_LOG, HOST_SEEN_BEFORE_LOAD, MAX_RENDERS_IN_FLIGHT, RENDERS_IN_FLIGHT,
    RENDER_FAILS, RENDER_RENDEZVOUS, ROD_SAW_SOURCE, SET_HOST_CALLS, SUITE_MASK,
};

/// How many times a plugin here was given a host.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetHostCalls() -> c_int {
    SET_HOST_CALLS.load(Ordering::SeqCst) as c_int
}

/// Whether a host was already in hand at the first load.
#[no_mangle]
pub extern "C" fn LumitTestPlugHostSeenBeforeLoad() -> c_int {
    HOST_SEEN_BEFORE_LOAD.load(Ordering::SeqCst) as c_int
}

/// Which suites the host handed over, as [`crate::SUITE_PROPERTY`] and friends.
#[no_mangle]
pub extern "C" fn LumitTestPlugSuiteMask() -> c_int {
    SUITE_MASK.load(Ordering::SeqCst) as c_int
}

/// Forget the action log and the concurrency high-water mark, and turn the
/// rendezvous off. A test calls this immediately before the stretch it means to
/// observe.
#[no_mangle]
pub extern "C" fn LumitTestPlugResetProbes() {
    ACTION_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    RENDERS_IN_FLIGHT.store(0, Ordering::SeqCst);
    MAX_RENDERS_IN_FLIGHT.store(0, Ordering::SeqCst);
    RENDER_RENDEZVOUS.store(0, Ordering::SeqCst);
    RENDER_FAILS.store(0, Ordering::SeqCst);
    ROD_SAW_SOURCE.store(0, Ordering::SeqCst);
}

/// What the host said when the plugin asked its Source clip's size during
/// `getRegionOfDefinition`: nought never asked, one answered, two could not.
#[no_mangle]
pub extern "C" fn LumitTestPlugRodSawSource() -> c_int {
    ROD_SAW_SOURCE.load(Ordering::SeqCst) as c_int
}

/// Make every render answer `kOfxStatFailed`, or stop doing so.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetRenderFails(fails: c_int) {
    RENDER_FAILS.store(fails.max(0) as usize, Ordering::SeqCst);
}

/// Copy the action log into `buffer` as a NUL-terminated string, and answer
/// with how long the log actually is. A buffer too small is truncated, not
/// overrun.
///
/// # Safety
///
/// `buffer` must be null or point at `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn LumitTestPlugActionLog(buffer: *mut c_char, capacity: c_int) -> c_int {
    let log = ACTION_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let bytes = log.as_bytes();
    let length = c_int::try_from(bytes.len()).unwrap_or(c_int::MAX);
    if buffer.is_null() || capacity <= 0 {
        return length;
    }
    let room = (capacity as usize).saturating_sub(1).min(bytes.len());
    for (index, byte) in bytes.iter().take(room).enumerate() {
        // SAFETY: the caller's contract; `index` is below `capacity - 1`.
        unsafe { *buffer.add(index) = *byte as c_char };
    }
    // SAFETY: as above; `room` is at most `capacity - 1`.
    unsafe { *buffer.add(room) = 0 };
    length
}

/// The most renders that were ever in flight at one moment since the last
/// reset. One means the host serialised them.
#[no_mangle]
pub extern "C" fn LumitTestPlugMaxConcurrentRenders() -> c_int {
    c_int::try_from(MAX_RENDERS_IN_FLIGHT.load(Ordering::SeqCst)).unwrap_or(c_int::MAX)
}

/// Make every render wait until `count` of them are in flight before any
/// finishes, so "did these two overlap?" is a question with a definite answer
/// rather than a race with a sleep in it. Nought turns it off.
#[no_mangle]
pub extern "C" fn LumitTestPlugSetRenderRendezvous(count: c_int) {
    RENDER_RENDEZVOUS.store(count.max(0) as usize, Ordering::SeqCst);
}

/// What the multi-thread suite told this plugin, so a test can read the host's
/// answers from the plugin's side rather than from the host's.
///
/// Answers `numCPUs`, and then runs a fan-out of that many threads which
/// records the indices it was given: the return is the number of *distinct*
/// indices seen, which equals the thread count exactly when
/// `multiThreadIndex` is right.
///
/// # Safety
///
/// `num_cpus_out` must be null or point at writable storage for one `int`.
#[no_mangle]
pub unsafe extern "C" fn LumitTestPlugFanOut(num_cpus_out: *mut c_int) -> c_int {
    // SAFETY: `OfxMultiThreadSuiteV1` is what the host declares for this name.
    let Some(suite) = (unsafe { suite::<OfxMultiThreadSuiteV1>(c"OfxMultiThreadSuite") }) else {
        return -1;
    };
    let mut cpus: c_uint = 0;
    // SAFETY: the host's own function, given a valid out-parameter.
    if unsafe { (suite.multi_thread_num_cpus)(&raw mut cpus) } != Status::Ok.code() {
        return -1;
    }
    if !num_cpus_out.is_null() {
        // SAFETY: the caller's out-parameter, checked non-null.
        unsafe { *num_cpus_out = cpus as c_int };
    }

    FAN_OUT_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    // SAFETY: the host's own function, given this plugin's own thread body and
    // a null custom argument, which the body does not follow.
    let status = unsafe { (suite.multi_thread)(fan_out_body, cpus, std::ptr::null_mut()) };
    if status != Status::Ok.code() {
        return -1;
    }
    let mut seen = FAN_OUT_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    seen.sort_unstable();
    seen.dedup();
    c_int::try_from(seen.len()).unwrap_or(c_int::MAX)
}

/// The indices the last fan-out handed out.
static FAN_OUT_SEEN: Mutex<Vec<c_uint>> = Mutex::new(Vec::new());

/// One thread of [`LumitTestPlugFanOut`]: note the index the host says this is.
unsafe extern "C" fn fan_out_body(thread_index: c_uint, _thread_max: c_uint, _arg: *mut c_void) {
    FAN_OUT_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(thread_index);
}

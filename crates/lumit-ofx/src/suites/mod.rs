//! The suites: the function tables a plugin fetches from the host.
//!
//! # In plain terms
//!
//! A suite is a struct of function pointers — the host's side of the deal.
//! The plugin asks for one by name and version and then calls straight into
//! Rust across a C boundary, which is where two rules bite (docs/14 §7):
//!
//! * **Nothing may unwind out of these functions.** A Rust panic crossing a C
//!   frame is undefined behaviour, so every entry point's body runs inside
//!   [`guard`], which turns a panic into `kOfxStatErrFatal`.
//! * **Every pointer is checked before it is followed**, including the
//!   handles, which are checked without being followed at all.

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::handles::Handle;
use crate::status::{finish, OfxStatus, Status, StatusResult};

pub mod image_effect;
pub mod memory;
pub mod message;
pub mod parameter;
pub mod property;

/// Run a suite entry point's body, converting its answer — or its panic —
/// into the status code the plugin is waiting for.
pub(crate) fn guard(body: impl FnOnce() -> StatusResult) -> OfxStatus {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => finish(result),
        Err(_) => Status::ErrFatal.code(),
    }
}

/// As [`guard`], for the entry points whose successful answer is a code other
/// than `kOfxStatOK` — the message suite answers a question with a reply.
pub(crate) fn guard_code(body: impl FnOnce() -> Status) -> OfxStatus {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status.code(),
        Err(_) => Status::ErrFatal.code(),
    }
}

/// Borrow a C string the plugin passed in.
///
/// # Safety
///
/// `ptr` must be null or a NUL-terminated string that stays alive for the
/// duration of the call, which is what the OFX API promises for every string
/// argument.
pub(crate) unsafe fn cstr<'a>(ptr: *const c_char) -> Result<&'a str, Status> {
    if ptr.is_null() {
        return Err(Status::ErrValue);
    }
    // SAFETY: the caller's contract, plus the null check above.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| Status::ErrValue)
}

/// Write a handle into a plugin's out-parameter.
///
/// A null out-parameter is [`Status::ErrValue`], not a silent success: a plugin
/// that asks for a handle and passes nowhere to put it has a bug, and the whole
/// call did nothing useful.
///
/// # Safety
///
/// `slot` must be null or point to writable storage for one handle, which is
/// what every OFX out-parameter is.
pub(crate) unsafe fn out_handle(slot: *mut *mut c_void, handle: Handle) -> StatusResult {
    if slot.is_null() {
        return Err(Status::ErrValue);
    }
    // SAFETY: the caller's contract, plus the null check above.
    unsafe { *slot = handle.as_ptr() };
    Ok(())
}

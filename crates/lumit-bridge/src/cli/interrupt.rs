//! Ctrl+C as a request to stop.
//!
//! The first Ctrl+C sets the stop flag, so a script stops at its next step
//! and its edits come off, and an export stops at its next frame and takes
//! its half-written file with it. A second one ends the program the usual
//! way, for whatever does not stop.
//!
//! The standard library has no way to hear Ctrl+C, so each system is asked
//! directly. The handler only sets a flag, which is all a signal handler may
//! safely do.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// The flag Ctrl+C sets, with the handler in place.
pub fn stop_flag() -> Arc<AtomicBool> {
    let flag = STOP.get_or_init(Arc::default).clone();
    install();
    flag
}

/// Set the flag, and answer whether it was already set.
fn stop() -> bool {
    STOP.get()
        .is_none_or(|stop| stop.swap(true, Ordering::SeqCst))
}

#[cfg(windows)]
fn install() {
    type Handler = unsafe extern "system" fn(u32) -> i32;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }
    /// `CTRL_C_EVENT` and `CTRL_BREAK_EVENT` are 0 and 1. Answering 0
    /// passes the event on to Windows, which ends the program.
    unsafe extern "system" fn on_break(kind: u32) -> i32 {
        i32::from(kind <= 1 && !stop())
    }
    // SAFETY: `on_break` has the signature Windows calls a handler with, and
    // touches nothing but an atomic. Windows runs it on a thread of its own.
    unsafe {
        SetConsoleCtrlHandler(Some(on_break), 1);
    }
}

#[cfg(unix)]
fn install() {
    use std::ffi::c_int;
    extern "C" {
        fn signal(signum: c_int, handler: usize) -> usize;
        fn _exit(status: c_int) -> !;
    }
    /// The same number on Linux and macOS.
    const SIGINT: c_int = 2;
    extern "C" fn on_interrupt(_: c_int) {
        if stop() {
            // SAFETY: `_exit` is one of the calls a signal handler may make.
            // 130 is what a shell reports for a program ended by Ctrl+C.
            unsafe { _exit(130) }
        }
    }
    // SAFETY: `on_interrupt` has the signature a signal handler has, and
    // does nothing a handler may not.
    unsafe {
        signal(SIGINT, on_interrupt as extern "C" fn(c_int) as usize);
    }
}

#[cfg(not(any(windows, unix)))]
fn install() {}

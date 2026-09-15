//! A test plugin that only loads when the DLL beside it is found.
//!
//! It imports from `lumit_side_dep.dll`, which the test puts next to it as a
//! copy of the normal test plugin. It holds no plugins of its own.
#![cfg(target_os = "windows")]

use std::ffi::{c_int, c_void};

#[link(name = "lumit_side_dep", kind = "raw-dylib")]
extern "C" {
    fn LumitTestPlugSetHostCalls() -> c_int;
}

/// Asks the DLL beside it something, then says it holds nothing.
#[no_mangle]
pub extern "C" fn OfxGetNumberOfPlugins() -> c_int {
    // SAFETY: the side DLL's own export, which takes no arguments.
    let _ = unsafe { LumitTestPlugSetHostCalls() };
    0
}

/// Never asked, since there are no plugins.
#[no_mangle]
pub extern "C" fn OfxGetPlugin(_index: c_int) -> *const c_void {
    std::ptr::null()
}

//! `OfxMemorySuiteV1` — the allocator plugins are asked to use.
//!
//! # In plain terms
//!
//! `memoryFree` is handed a bare address and nothing else, so a host that
//! trusts it will happily free a number a plugin invented. This one keeps a
//! list of the addresses it handed out: freeing anything else is
//! `kOfxStatErrBadHandle` and **nothing is read from the address at all**.
//! The list also holds each block's size, which is what Rust's allocator needs
//! back at free time and what a later package will bill against a budget.

use std::alloc::{alloc, dealloc, Layout};
use std::ffi::{c_int, c_void};

use crate::ffi::OfxMemorySuiteV1;
use crate::host::state;
use crate::status::Status;
use crate::suites::guard;

/// Alignment for every block handed to a plugin. Sixteen bytes is what a
/// plugin's SIMD load expects of a pixel buffer and costs nothing here.
const ALIGN: usize = 16;

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxMemorySuiteV1 = OfxMemorySuiteV1 {
    memory_alloc,
    memory_free,
};

unsafe extern "C" fn memory_alloc(
    _handle: *mut c_void,
    n_bytes: usize,
    allocated_data: *mut *mut c_void,
) -> c_int {
    guard(|| {
        if allocated_data.is_null() {
            return Err(Status::ErrValue);
        }
        // A zero-byte request still has to come back with an address the
        // plugin can free, so it gets the smallest real block.
        let size = n_bytes.max(1);
        let layout = Layout::from_size_align(size, ALIGN).map_err(|_| Status::ErrMemory)?;
        // SAFETY: the layout has a non-zero size, which is `alloc`'s one
        // requirement; the null return is handled below.
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err(Status::ErrMemory);
        }
        state().allocations.insert(ptr as usize, size);
        // SAFETY: the plugin's out-parameter, checked non-null above.
        unsafe { *allocated_data = ptr.cast() };
        Ok(())
    })
}

unsafe extern "C" fn memory_free(allocated_data: *mut c_void) -> c_int {
    guard(|| {
        if allocated_data.is_null() {
            return Err(Status::ErrBadHandle);
        }
        let address = allocated_data as usize;
        let size = state()
            .allocations
            .remove(&address)
            .ok_or(Status::ErrBadHandle)?;
        let layout = Layout::from_size_align(size, ALIGN).map_err(|_| Status::ErrMemory)?;
        // SAFETY: the address came out of our own list, was allocated with
        // exactly this layout, and has just been removed from the list, so no
        // second free can find it.
        unsafe { dealloc(allocated_data.cast(), layout) };
        Ok(())
    })
}

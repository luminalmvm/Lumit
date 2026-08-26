//! `OfxMemorySuiteV1` — the allocator plugins are asked to use — and the
//! host's own image arena beside it.
//!
//! # In plain terms
//!
//! `memoryFree` is handed a bare address and nothing else, so a host that
//! trusts it will happily free a number a plugin invented. This one keeps a
//! list of the addresses it handed out: freeing anything else is
//! `kOfxStatErrBadHandle` and **nothing is read from the address at all**.
//! The list also holds each block's size, which is what Rust's allocator needs
//! back at free time and what a later package will bill against a budget.
//!
//! The **arena** is the other half: the blocks the host allocates for the
//! pictures it hands a plugin ([`crate::image`]). They are deliberately in a
//! second list, not the plugin's one, so `memoryFree` cannot reach them — a
//! plugin handed a pointer to an input frame must not be able to free the
//! frame. Each [`Block`] frees itself when it is dropped, which happens when
//! the render that made it finishes, so nothing a plugin kept a pointer to can
//! outlive the action it was given in. [`image_bytes_live`] is what a test
//! reads to prove none of them leaked.

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::collections::BTreeMap;
use std::ffi::{c_int, c_void};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use crate::ffi::OfxMemorySuiteV1;
use crate::host::state;
use crate::status::Status;
use crate::suites::guard;

/// Alignment for every block handed to a plugin. Sixteen bytes is what a
/// plugin's SIMD load expects of a pixel buffer and costs nothing here.
const ALIGN: usize = 16;

/// The arena's own list, deliberately behind its **own** lock rather than the
/// host state's. A block is freed when its [`Block`] is dropped, and a drop can
/// happen anywhere — including while a suite call holds the host lock — so
/// sharing that lock would be a deadlock waiting for the right day
/// (docs/14 §7).
static IMAGE_BLOCKS: OnceLock<Mutex<BTreeMap<usize, usize>>> = OnceLock::new();

fn image_blocks() -> MutexGuard<'static, BTreeMap<usize, usize>> {
    IMAGE_BLOCKS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// One block of host-owned image memory. Freed when it is dropped, and never
/// reachable through `memoryFree`.
#[derive(Debug)]
pub struct Block {
    ptr: *mut u8,
    size: usize,
}

// SAFETY: a `Block` owns its allocation outright — nothing else holds a
// pointer to it except a plugin, for the duration of one action, and the block
// is not read or written except through `&self`/`&mut self`. Moving that
// ownership between threads is what the render driver does when it hands a
// render to the worker pool.
unsafe impl Send for Block {}

impl Block {
    /// A zeroed block of `size` bytes from the arena.
    ///
    /// Zeroed rather than uninitialised because the pixels are read as floats
    /// the moment the block exists, and "transparent black" is the right thing
    /// for a picture nobody has written yet.
    ///
    /// # Errors
    ///
    /// [`Status::ErrMemory`] if the size is impossible or the allocator says
    /// no; [`Status::ErrValue`] for a block of nothing.
    pub fn zeroed(size: usize) -> Result<Self, Status> {
        if size == 0 {
            return Err(Status::ErrValue);
        }
        let layout = Layout::from_size_align(size, ALIGN).map_err(|_| Status::ErrMemory)?;
        // SAFETY: the layout has a non-zero size, which is `alloc_zeroed`'s one
        // requirement; the null return is handled below.
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            return Err(Status::ErrMemory);
        }
        image_blocks().insert(ptr as usize, size);
        Ok(Self { ptr, size })
    }

    /// The address, for handing to a plugin as `kOfxImagePropData`.
    #[must_use]
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        image_blocks().remove(&(self.ptr as usize));
        let Ok(layout) = Layout::from_size_align(self.size, ALIGN) else {
            return;
        };
        // SAFETY: this block was allocated with exactly this layout by
        // `zeroed`, has just been struck off the arena's list, and is dropped
        // once — `Block` is not `Clone` and its pointer is private.
        unsafe { dealloc(self.ptr, layout) };
    }
}

/// How many bytes of image arena are outstanding. Nought once every render has
/// finished; a test reads it to prove a plugin's pictures were let go of.
#[must_use]
pub fn image_bytes_live() -> usize {
    image_blocks().values().sum()
}

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

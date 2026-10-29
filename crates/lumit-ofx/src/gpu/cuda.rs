//! The CUDA driver API, opened at run time.
//!
//! Nothing here is needed to build Lumit. A machine with an NVIDIA driver has
//! `nvcuda.dll` (or `libcuda.so.1`), and one without simply has no CUDA. The
//! test plugin opens the same table to run its own kernel, so the two sides
//! cannot disagree about a signature.
//!
//! Broker main thread only, like everything in [`crate::gpu`].

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};

use libloading::Library;

/// `CUresult`. Nought is success.
pub type CuResult = c_int;
/// `CUdevice`, an ordinal.
pub type CuDevice = c_int;
/// `CUcontext`.
pub type CuContext = *mut c_void;
/// `CUstream`.
pub type CuStream = *mut c_void;
/// `CUdeviceptr`: an address on the card, as a number.
pub type CuDevicePtr = u64;
/// `CUmodule`.
pub type CuModule = *mut c_void;
/// `CUfunction`.
pub type CuFunction = *mut c_void;

/// The driver's own file.
#[cfg(windows)]
const LIBRARY: &str = "nvcuda.dll";
#[cfg(not(windows))]
const LIBRARY: &str = "libcuda.so.1";

/// One entry point out of a library, copied out so the library can move.
///
/// # Safety
///
/// `T` must be the function type the library exports under `name`.
pub(crate) unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Option<T> {
    // SAFETY: the caller's contract on `T`.
    unsafe { library.get::<T>(name) }.ok().map(|found| *found)
}

/// The entry points the host and the test plugin call. `CUDAAPI` is
/// `__stdcall` on Windows, which `"system"` spells on every target.
pub struct Driver {
    pub init: unsafe extern "system" fn(c_uint) -> CuResult,
    pub device_get_count: unsafe extern "system" fn(*mut c_int) -> CuResult,
    pub device_get: unsafe extern "system" fn(*mut CuDevice, c_int) -> CuResult,
    pub device_get_name: unsafe extern "system" fn(*mut c_char, c_int, CuDevice) -> CuResult,
    pub primary_ctx_retain: unsafe extern "system" fn(*mut CuContext, CuDevice) -> CuResult,
    pub primary_ctx_release: unsafe extern "system" fn(CuDevice) -> CuResult,
    pub ctx_set_current: unsafe extern "system" fn(CuContext) -> CuResult,
    pub ctx_get_current: unsafe extern "system" fn(*mut CuContext) -> CuResult,
    pub ctx_synchronize: unsafe extern "system" fn() -> CuResult,
    pub stream_create: unsafe extern "system" fn(*mut CuStream, c_uint) -> CuResult,
    pub stream_destroy: unsafe extern "system" fn(CuStream) -> CuResult,
    pub mem_alloc: unsafe extern "system" fn(*mut CuDevicePtr, usize) -> CuResult,
    pub mem_free: unsafe extern "system" fn(CuDevicePtr) -> CuResult,
    pub memcpy_htod: unsafe extern "system" fn(CuDevicePtr, *const c_void, usize) -> CuResult,
    pub memcpy_dtoh: unsafe extern "system" fn(*mut c_void, CuDevicePtr, usize) -> CuResult,
    pub module_load_data: unsafe extern "system" fn(*mut CuModule, *const c_void) -> CuResult,
    pub module_get_function:
        unsafe extern "system" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult,
    #[allow(clippy::type_complexity)]
    pub launch_kernel: unsafe extern "system" fn(
        CuFunction,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        CuStream,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> CuResult,
    /// Last, so it is dropped after nothing can call into it.
    _library: Library,
}

impl Driver {
    /// Open the driver and initialise it, or `None` on a machine without one.
    #[must_use]
    pub fn load() -> Option<Self> {
        // SAFETY: loading the vendor's driver runs its own start-up code, which
        // is what every CUDA program does first.
        let library = unsafe { Library::new(LIBRARY) }.ok()?;
        // SAFETY: each type is the one cuda.h declares for that name; the `_v2`
        // names are the ones the header's macros select on 64-bit builds.
        let driver = unsafe {
            Self {
                init: symbol(&library, b"cuInit\0")?,
                device_get_count: symbol(&library, b"cuDeviceGetCount\0")?,
                device_get: symbol(&library, b"cuDeviceGet\0")?,
                device_get_name: symbol(&library, b"cuDeviceGetName\0")?,
                primary_ctx_retain: symbol(&library, b"cuDevicePrimaryCtxRetain\0")?,
                primary_ctx_release: symbol(&library, b"cuDevicePrimaryCtxRelease_v2\0")?,
                ctx_set_current: symbol(&library, b"cuCtxSetCurrent\0")?,
                ctx_get_current: symbol(&library, b"cuCtxGetCurrent\0")?,
                ctx_synchronize: symbol(&library, b"cuCtxSynchronize\0")?,
                stream_create: symbol(&library, b"cuStreamCreate\0")?,
                stream_destroy: symbol(&library, b"cuStreamDestroy_v2\0")?,
                mem_alloc: symbol(&library, b"cuMemAlloc_v2\0")?,
                mem_free: symbol(&library, b"cuMemFree_v2\0")?,
                memcpy_htod: symbol(&library, b"cuMemcpyHtoD_v2\0")?,
                memcpy_dtoh: symbol(&library, b"cuMemcpyDtoH_v2\0")?,
                module_load_data: symbol(&library, b"cuModuleLoadData\0")?,
                module_get_function: symbol(&library, b"cuModuleGetFunction\0")?,
                launch_kernel: symbol(&library, b"cuLaunchKernel\0")?,
                _library: library,
            }
        };
        // SAFETY: the driver's own entry point; nought is the only flag value.
        (unsafe { (driver.init)(0) } == 0).then_some(driver)
    }
}

/// A driver answer as a sentence, for the badge.
fn check(what: &str, code: CuResult) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("CUDA {what} failed ({code})"))
    }
}

/// One card, opened through its primary context, with a stream of our own.
///
/// The primary context is the one the CUDA runtime uses, so a plugin built
/// against the runtime finds our buffers valid in its own calls.
pub(crate) struct Cuda {
    driver: Driver,
    device: CuDevice,
    context: CuContext,
    stream: CuStream,
}

// SAFETY: the context and stream are driver handles, not Rust memory. The
// broker renders one frame at a time on one thread, and `make_current` binds
// the context to whichever thread asks before any call that needs it.
unsafe impl Send for Cuda {}
// SAFETY: as above; nothing here is mutated after `open`.
unsafe impl Sync for Cuda {}

impl Cuda {
    /// Whether this machine has a CUDA card, without making a context.
    pub(crate) fn present() -> bool {
        let Some(driver) = Driver::load() else {
            return false;
        };
        let mut count = 0;
        // SAFETY: the driver's own entry point, given a valid out-parameter.
        unsafe { (driver.device_get_count)(&raw mut count) == 0 && count > 0 }
    }

    /// Open the card whose name Lumit's adapter starts with, or the first.
    pub(crate) fn open(adapter: Option<&str>) -> Option<Self> {
        let driver = Driver::load()?;
        let mut count = 0;
        // SAFETY: the driver's own entry point, given a valid out-parameter.
        if unsafe { (driver.device_get_count)(&raw mut count) } != 0 || count < 1 {
            return None;
        }
        let mut chosen = None;
        for ordinal in 0..count {
            let mut device = 0;
            // SAFETY: as above.
            if unsafe { (driver.device_get)(&raw mut device, ordinal) } != 0 {
                continue;
            }
            let mut name = [0 as c_char; 256];
            // SAFETY: the buffer is as long as the length it is given.
            unsafe { (driver.device_get_name)(name.as_mut_ptr(), 255, device) };
            // SAFETY: the driver NUL-terminates within the length, and the last
            // byte is nought whatever it wrote.
            let name = unsafe { CStr::from_ptr(name.as_ptr()) }.to_string_lossy();
            let matches = adapter.is_some_and(|summary| super::names_card(summary, &name));
            if chosen.is_none() || matches {
                chosen = Some(device);
            }
            if matches {
                break;
            }
        }
        let device = chosen?;
        let mut context = std::ptr::null_mut();
        // SAFETY: as above.
        if unsafe { (driver.primary_ctx_retain)(&raw mut context, device) } != 0 {
            return None;
        }
        let mut cuda = Self {
            driver,
            device,
            context,
            stream: std::ptr::null_mut(),
        };
        cuda.make_current().ok()?;
        let mut stream = std::ptr::null_mut();
        // SAFETY: as above; flag nought is a stream that waits on the default
        // one, so the blocking copies below are ordered after the plugin's work.
        if unsafe { (cuda.driver.stream_create)(&raw mut stream, 0) } != 0 {
            return None;
        }
        cuda.stream = stream;
        Some(cuda)
    }

    /// Bind the context to this thread, which is the thread the plugin renders on.
    pub(crate) fn make_current(&self) -> Result<(), String> {
        // SAFETY: a context this value retained and still holds.
        check("context", unsafe {
            (self.driver.ctx_set_current)(self.context)
        })
    }

    /// `kOfxImageEffectPropCudaStream`.
    pub(crate) fn queue(&self) -> usize {
        self.stream as usize
    }

    pub(crate) fn alloc(&self, bytes: usize) -> Result<usize, String> {
        let mut at: CuDevicePtr = 0;
        // SAFETY: a valid out-parameter, on the current context.
        check("allocation", unsafe {
            (self.driver.mem_alloc)(&raw mut at, bytes.max(1))
        })?;
        usize::try_from(at).map_err(|_| "CUDA gave an address out of range".to_owned())
    }

    pub(crate) fn upload(&self, at: usize, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: `at` is a buffer this value allocated at least this long.
        check("upload", unsafe {
            (self.driver.memcpy_htod)(at as CuDevicePtr, bytes.as_ptr().cast(), bytes.len())
        })
    }

    pub(crate) fn download(&self, at: usize, into: &mut [u8]) -> Result<(), String> {
        // SAFETY: as above, into a host block exactly this long.
        check("download", unsafe {
            (self.driver.memcpy_dtoh)(into.as_mut_ptr().cast(), at as CuDevicePtr, into.len())
        })
    }

    pub(crate) fn free(&self, at: usize) {
        // SAFETY: a buffer this value allocated and nothing else frees.
        unsafe { (self.driver.mem_free)(at as CuDevicePtr) };
    }

    /// Wait for everything the plugin queued, on any stream.
    pub(crate) fn finish(&self) -> Result<(), String> {
        self.make_current()?;
        // SAFETY: the driver's own entry point, on the current context.
        check("render", unsafe { (self.driver.ctx_synchronize)() })
    }
}

impl Drop for Cuda {
    fn drop(&mut self) {
        // SAFETY: the stream and the retain are this value's own.
        unsafe {
            if !self.stream.is_null() {
                (self.driver.stream_destroy)(self.stream);
            }
            (self.driver.primary_ctx_release)(self.device);
        }
    }
}

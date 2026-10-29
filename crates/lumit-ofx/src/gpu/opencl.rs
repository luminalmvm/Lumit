//! OpenCL, through the ICD loader opened at run time.
//!
//! The loader (`OpenCL.dll`, `libOpenCL.so.1`) comes with every GPU driver
//! that has OpenCL at all, so nothing is needed to build this. Images go to a
//! plugin as OpenCL buffers, the 1.5 "buffers" path, whose `cl_mem` is the
//! image's data pointer. The test plugin opens the same table to build its own
//! kernel.
//!
//! Broker main thread only, like everything in [`crate::gpu`].

use std::ffi::{c_char, c_void};

use libloading::Library;

use super::cuda::symbol;

/// `cl_int`. Nought is `CL_SUCCESS`.
pub type ClInt = i32;
/// `cl_uint`.
pub type ClUint = u32;
/// Every `cl_*` object handle.
pub type ClHandle = *mut c_void;

/// `CL_DEVICE_TYPE_GPU`.
pub const DEVICE_TYPE_GPU: u64 = 1 << 2;
/// `CL_DEVICE_NAME`.
pub const DEVICE_NAME: ClUint = 0x102B;
/// `CL_DEVICE_BOARD_NAME_AMD`: AMD's marketing name, where `CL_DEVICE_NAME` is
/// a code name.
pub const DEVICE_BOARD_NAME_AMD: ClUint = 0x4038;
/// `CL_CONTEXT_PLATFORM`.
pub const CONTEXT_PLATFORM: isize = 0x1084;
/// `CL_MEM_READ_WRITE`.
pub const MEM_READ_WRITE: u64 = 1;
/// `CL_QUEUE_CONTEXT`.
pub const QUEUE_CONTEXT: ClUint = 0x1090;
/// `CL_QUEUE_DEVICE`.
pub const QUEUE_DEVICE: ClUint = 0x1091;

/// The ICD loader's own file.
#[cfg(windows)]
const LIBRARY: &str = "OpenCL.dll";
#[cfg(not(windows))]
const LIBRARY: &str = "libOpenCL.so.1";

/// The entry points the host and the test plugin call. `CL_API_CALL` is
/// `__stdcall` on Windows, which `"system"` spells on every target.
#[allow(clippy::type_complexity)]
pub struct Api {
    pub get_platform_ids: unsafe extern "system" fn(ClUint, *mut ClHandle, *mut ClUint) -> ClInt,
    pub get_device_ids:
        unsafe extern "system" fn(ClHandle, u64, ClUint, *mut ClHandle, *mut ClUint) -> ClInt,
    pub get_device_info:
        unsafe extern "system" fn(ClHandle, ClUint, usize, *mut c_void, *mut usize) -> ClInt,
    pub create_context: unsafe extern "system" fn(
        *const isize,
        ClUint,
        *const ClHandle,
        *const c_void,
        *mut c_void,
        *mut ClInt,
    ) -> ClHandle,
    pub create_command_queue:
        unsafe extern "system" fn(ClHandle, ClHandle, u64, *mut ClInt) -> ClHandle,
    pub create_buffer:
        unsafe extern "system" fn(ClHandle, u64, usize, *mut c_void, *mut ClInt) -> ClHandle,
    pub enqueue_write_buffer: unsafe extern "system" fn(
        ClHandle,
        ClHandle,
        ClUint,
        usize,
        usize,
        *const c_void,
        ClUint,
        *const ClHandle,
        *mut ClHandle,
    ) -> ClInt,
    pub enqueue_read_buffer: unsafe extern "system" fn(
        ClHandle,
        ClHandle,
        ClUint,
        usize,
        usize,
        *mut c_void,
        ClUint,
        *const ClHandle,
        *mut ClHandle,
    ) -> ClInt,
    pub finish: unsafe extern "system" fn(ClHandle) -> ClInt,
    pub release_mem_object: unsafe extern "system" fn(ClHandle) -> ClInt,
    pub release_command_queue: unsafe extern "system" fn(ClHandle) -> ClInt,
    pub release_context: unsafe extern "system" fn(ClHandle) -> ClInt,
    pub get_command_queue_info:
        unsafe extern "system" fn(ClHandle, ClUint, usize, *mut c_void, *mut usize) -> ClInt,
    pub create_program_with_source: unsafe extern "system" fn(
        ClHandle,
        ClUint,
        *const *const c_char,
        *const usize,
        *mut ClInt,
    ) -> ClHandle,
    pub build_program: unsafe extern "system" fn(
        ClHandle,
        ClUint,
        *const ClHandle,
        *const c_char,
        *const c_void,
        *mut c_void,
    ) -> ClInt,
    pub create_kernel: unsafe extern "system" fn(ClHandle, *const c_char, *mut ClInt) -> ClHandle,
    pub set_kernel_arg: unsafe extern "system" fn(ClHandle, ClUint, usize, *const c_void) -> ClInt,
    pub enqueue_nd_range_kernel: unsafe extern "system" fn(
        ClHandle,
        ClHandle,
        ClUint,
        *const usize,
        *const usize,
        *const usize,
        ClUint,
        *const ClHandle,
        *mut ClHandle,
    ) -> ClInt,
    /// Last, so it is dropped after nothing can call into it.
    _library: Library,
}

impl Api {
    /// Open the loader, or `None` on a machine without one.
    #[must_use]
    pub fn load() -> Option<Self> {
        // SAFETY: loading the loader runs its own start-up code, which is what
        // every OpenCL program does first.
        let library = unsafe { Library::new(LIBRARY) }.ok()?;
        // SAFETY: each type is the one cl.h declares for that name.
        unsafe {
            Some(Self {
                get_platform_ids: symbol(&library, b"clGetPlatformIDs\0")?,
                get_device_ids: symbol(&library, b"clGetDeviceIDs\0")?,
                get_device_info: symbol(&library, b"clGetDeviceInfo\0")?,
                create_context: symbol(&library, b"clCreateContext\0")?,
                create_command_queue: symbol(&library, b"clCreateCommandQueue\0")?,
                create_buffer: symbol(&library, b"clCreateBuffer\0")?,
                enqueue_write_buffer: symbol(&library, b"clEnqueueWriteBuffer\0")?,
                enqueue_read_buffer: symbol(&library, b"clEnqueueReadBuffer\0")?,
                finish: symbol(&library, b"clFinish\0")?,
                release_mem_object: symbol(&library, b"clReleaseMemObject\0")?,
                release_command_queue: symbol(&library, b"clReleaseCommandQueue\0")?,
                release_context: symbol(&library, b"clReleaseContext\0")?,
                get_command_queue_info: symbol(&library, b"clGetCommandQueueInfo\0")?,
                create_program_with_source: symbol(&library, b"clCreateProgramWithSource\0")?,
                build_program: symbol(&library, b"clBuildProgram\0")?,
                create_kernel: symbol(&library, b"clCreateKernel\0")?,
                set_kernel_arg: symbol(&library, b"clSetKernelArg\0")?,
                enqueue_nd_range_kernel: symbol(&library, b"clEnqueueNDRangeKernel\0")?,
                _library: library,
            })
        }
    }

    /// One text property of a device, or empty.
    fn device_text(&self, device: ClHandle, what: ClUint) -> String {
        let mut text = [0_u8; 256];
        let mut written = 0;
        // SAFETY: the buffer is as long as the size it is given.
        let code = unsafe {
            (self.get_device_info)(
                device,
                what,
                text.len(),
                text.as_mut_ptr().cast(),
                &raw mut written,
            )
        };
        if code != 0 {
            return String::new();
        }
        let end = text.iter().position(|byte| *byte == 0).unwrap_or(0);
        String::from_utf8_lossy(&text[..end]).into_owned()
    }
}

/// A sentence for the badge, from an OpenCL answer.
fn check(what: &str, code: ClInt) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("OpenCL {what} failed ({code})"))
    }
}

/// One GPU device, with a context and a command queue of our own.
pub(crate) struct OpenCl {
    api: Api,
    context: ClHandle,
    queue: ClHandle,
}

// SAFETY: the context and queue are driver handles, not Rust memory, and
// OpenCL objects may be used from any thread. The broker uses them from one.
unsafe impl Send for OpenCl {}
// SAFETY: as above; nothing here is mutated after `open`.
unsafe impl Sync for OpenCl {}

impl OpenCl {
    /// Whether this machine has an OpenCL GPU, without making a context.
    pub(crate) fn present() -> bool {
        let Some(api) = Api::load() else {
            return false;
        };
        let mut platforms = [std::ptr::null_mut(); 8];
        let mut found = 0;
        // SAFETY: the array is as long as the count it is given.
        if unsafe { (api.get_platform_ids)(8, platforms.as_mut_ptr(), &raw mut found) } != 0 {
            return false;
        }
        platforms
            .iter()
            .take(found.min(8) as usize)
            .any(|&platform| {
                let mut count = 0;
                // SAFETY: a count-only query, no array.
                let code = unsafe {
                    (api.get_device_ids)(
                        platform,
                        DEVICE_TYPE_GPU,
                        0,
                        std::ptr::null_mut(),
                        &raw mut count,
                    )
                };
                code == 0 && count > 0
            })
    }

    /// Open the GPU whose name Lumit's adapter starts with, or the first.
    pub(crate) fn open(adapter: Option<&str>) -> Option<Self> {
        let api = Api::load()?;
        let mut platforms = [std::ptr::null_mut(); 8];
        let mut found = 0;
        // SAFETY: the array is as long as the count it is given.
        let code = unsafe { (api.get_platform_ids)(8, platforms.as_mut_ptr(), &raw mut found) };
        if code != 0 {
            return None;
        }
        let mut chosen = None;
        'platforms: for &platform in platforms.iter().take(found.min(8) as usize) {
            let mut devices = [std::ptr::null_mut(); 8];
            let mut count = 0;
            // SAFETY: as above.
            let code = unsafe {
                (api.get_device_ids)(
                    platform,
                    DEVICE_TYPE_GPU,
                    8,
                    devices.as_mut_ptr(),
                    &raw mut count,
                )
            };
            if code != 0 {
                continue;
            }
            for &device in devices.iter().take(count.min(8) as usize) {
                let matches = adapter.is_some_and(|summary| {
                    [DEVICE_NAME, DEVICE_BOARD_NAME_AMD]
                        .iter()
                        .any(|what| super::names_card(summary, &api.device_text(device, *what)))
                });
                if chosen.is_none() || matches {
                    chosen = Some((platform, device));
                }
                if matches {
                    break 'platforms;
                }
            }
        }
        let (platform, device) = chosen?;
        let properties = [CONTEXT_PLATFORM, platform as isize, 0];
        let mut code = 0;
        // SAFETY: a nought-terminated property list and one device.
        let context = unsafe {
            (api.create_context)(
                properties.as_ptr(),
                1,
                &raw const device,
                std::ptr::null(),
                std::ptr::null_mut(),
                &raw mut code,
            )
        };
        if code != 0 || context.is_null() {
            return None;
        }
        // SAFETY: the context just made, on its own device.
        let queue = unsafe { (api.create_command_queue)(context, device, 0, &raw mut code) };
        if code != 0 || queue.is_null() {
            // SAFETY: the context just made, released once.
            unsafe { (api.release_context)(context) };
            return None;
        }
        Some(Self {
            api,
            context,
            queue,
        })
    }

    /// `kOfxImageEffectPropOpenCLCommandQueue`.
    pub(crate) fn queue(&self) -> usize {
        self.queue as usize
    }

    pub(crate) fn alloc(&self, bytes: usize) -> Result<usize, String> {
        let mut code = 0;
        // SAFETY: a plain device buffer in this value's context.
        let buffer = unsafe {
            (self.api.create_buffer)(
                self.context,
                MEM_READ_WRITE,
                bytes.max(1),
                std::ptr::null_mut(),
                &raw mut code,
            )
        };
        check("allocation", code)?;
        if buffer.is_null() {
            return Err("OpenCL gave no buffer".to_owned());
        }
        Ok(buffer as usize)
    }

    pub(crate) fn upload(&self, at: usize, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: a blocking write into a buffer this value made at least
        // this long.
        check("upload", unsafe {
            (self.api.enqueue_write_buffer)(
                self.queue,
                at as ClHandle,
                1,
                0,
                bytes.len(),
                bytes.as_ptr().cast(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        })
    }

    pub(crate) fn download(&self, at: usize, into: &mut [u8]) -> Result<(), String> {
        // SAFETY: a blocking read into a host block exactly this long.
        check("download", unsafe {
            (self.api.enqueue_read_buffer)(
                self.queue,
                at as ClHandle,
                1,
                0,
                into.len(),
                into.as_mut_ptr().cast(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        })
    }

    pub(crate) fn free(&self, at: usize) {
        // SAFETY: a buffer this value made and nothing else releases.
        unsafe { (self.api.release_mem_object)(at as ClHandle) };
    }

    /// Wait for everything the plugin queued.
    pub(crate) fn finish(&self) -> Result<(), String> {
        // SAFETY: this value's own queue.
        check("render", unsafe { (self.api.finish)(self.queue) })
    }
}

impl Drop for OpenCl {
    fn drop(&mut self) {
        // SAFETY: the queue and context are this value's own, released once.
        unsafe {
            (self.api.release_command_queue)(self.queue);
            (self.api.release_context)(self.context);
        }
    }
}

//! The gain of `com.lumitlab.testplug` on the graphics card, so a test can
//! compare it with the CPU's: CUDA as PTX text the driver compiles, and OpenCL
//! C built at run time. Neither needs a compiler here, and a machine with
//! neither simply never declares them.
//!
//! The driver tables are the host's own ([`lumit_ofx::gpu`]), so the two sides
//! cannot disagree about a signature.

use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

use lumit_ofx::ffi::{OfxPropertySetHandle, OfxPropertySuiteV1};
use lumit_ofx::gpu::cuda::{CuFunction, CuModule, CuStream, Driver};
use lumit_ofx::gpu::opencl::{Api, ClHandle, QUEUE_CONTEXT, QUEUE_DEVICE};
use lumit_ofx::status::{OfxStatus, Status};

use crate::{set_string, ImageView};

/// Declare the card paths, as `true`, or as `needed`, which also makes this
/// plugin refuse a CPU render so a test knows the card drew what came back.
/// Set in the broker's environment.
pub const GPU_ENV: &str = "LUMIT_TESTPLUG_GPU";

/// What [`GPU_ENV`] asks for, if it asks.
fn declared() -> Option<&'static CStr> {
    match std::env::var(GPU_ENV).ok()?.as_str() {
        "true" => Some(c"true"),
        "needed" => Some(c"needed"),
        _ => None,
    }
}

/// Say on the descriptor which card paths the gain can take.
pub(crate) fn declare(props_suite: &OfxPropertySuiteV1, props: OfxPropertySetHandle) {
    let Some(value) = declared() else {
        return;
    };
    for key in [
        c"OfxImageEffectPropCudaRenderSupported",
        c"OfxImageEffectPropOpenCLRenderSupported",
    ] {
        set_string(props_suite, props, key, 0, value);
    }
    set_string(
        props_suite,
        props,
        c"OfxImageEffectPropCudaStreamSupported",
        0,
        c"true",
    );
}

/// One render argument, nought or null when the host did not set it.
fn argument(props_suite: &OfxPropertySuiteV1, in_args: *mut c_void, key: &CStr) -> usize {
    let mut flag = 0;
    // SAFETY: the host's own function, given the `inArgs` it passed us.
    unsafe { (props_suite.prop_get_int)(in_args, key.as_ptr(), 0, &raw mut flag) };
    flag.max(0) as usize
}

/// The same, for a pointer.
fn pointer(props_suite: &OfxPropertySuiteV1, in_args: *mut c_void, key: &CStr) -> *mut c_void {
    let mut value = std::ptr::null_mut();
    // SAFETY: as above.
    unsafe { (props_suite.prop_get_pointer)(in_args, key.as_ptr(), 0, &raw mut value) };
    value
}

/// Render the gain on the card when the host asked for it, answering with the
/// status. `None` means a CPU render, which the caller does itself.
pub(crate) fn render(
    props_suite: &OfxPropertySuiteV1,
    in_args: *mut c_void,
    destination: &ImageView,
    source: Option<&ImageView>,
    temporal: bool,
    gain: f32,
) -> Option<OfxStatus> {
    let cuda = argument(props_suite, in_args, c"OfxImageEffectPropCudaEnabled") != 0;
    let opencl = argument(props_suite, in_args, c"OfxImageEffectPropOpenCLEnabled") != 0;
    if !cuda && !opencl {
        return (declared() == Some(c"needed")).then_some(Status::Failed.code());
    }
    // The card path is the gain on one frame of the same size, nothing more.
    let source = source.filter(|source| {
        !temporal
            && source.width() == destination.width()
            && source.height() == destination.height()
    });
    let Some(source) = source else {
        return Some(Status::Failed.code());
    };
    let Ok(floats) = u32::try_from(destination.width() * destination.height() * 4) else {
        return Some(Status::Failed.code());
    };
    let done = if cuda {
        let stream = pointer(props_suite, in_args, c"OfxImageEffectPropCudaStream");
        cuda_gain(stream, source.data, destination.data, floats, gain)
    } else {
        let queue = pointer(
            props_suite,
            in_args,
            c"OfxImageEffectPropOpenCLCommandQueue",
        );
        opencl_gain(queue, source.data, destination.data, floats, gain)
    };
    Some(if done {
        Status::Ok.code()
    } else {
        Status::Failed.code()
    })
}

// ------------------------------------------------------------------ CUDA --

/// `out[i] = in[i] * gain`, one thread a float, in PTX the driver compiles for
/// whatever card it is on.
const PTX: &CStr = c"
.version 7.0
.target sm_52
.address_size 64

.visible .entry gain(.param .u64 src, .param .u64 dst, .param .u32 n, .param .f32 g)
{
    .reg .pred %p<2>;
    .reg .b32 %r<6>;
    .reg .f32 %f<3>;
    .reg .b64 %rd<8>;

    ld.param.u64 %rd1, [src];
    ld.param.u64 %rd2, [dst];
    ld.param.u32 %r1, [n];
    ld.param.f32 %f1, [g];
    mov.u32 %r2, %ctaid.x;
    mov.u32 %r3, %ntid.x;
    mov.u32 %r4, %tid.x;
    mad.lo.s32 %r5, %r2, %r3, %r4;
    setp.ge.u32 %p1, %r5, %r1;
    @%p1 bra DONE;
    cvta.to.global.u64 %rd3, %rd1;
    cvta.to.global.u64 %rd4, %rd2;
    mul.wide.u32 %rd5, %r5, 4;
    add.s64 %rd6, %rd3, %rd5;
    add.s64 %rd7, %rd4, %rd5;
    ld.global.f32 %f2, [%rd6];
    mul.f32 %f2, %f2, %f1;
    st.global.f32 [%rd7], %f2;
DONE:
    ret;
}
";

/// The driver and the loaded kernel, made once on the render thread, where the
/// host has made its context current.
static CUDA: OnceLock<Option<(Driver, usize)>> = OnceLock::new();

fn cuda_kernel() -> Option<(&'static Driver, CuFunction)> {
    let (driver, function) = CUDA
        .get_or_init(|| {
            let driver = Driver::load()?;
            let mut module: CuModule = std::ptr::null_mut();
            // SAFETY: NUL-terminated PTX text, on the host's current context.
            if unsafe { (driver.module_load_data)(&raw mut module, PTX.as_ptr().cast()) } != 0 {
                return None;
            }
            let mut function: CuFunction = std::ptr::null_mut();
            // SAFETY: the module just loaded, and a name it defines.
            let code = unsafe {
                (driver.module_get_function)(&raw mut function, module, c"gain".as_ptr())
            };
            (code == 0).then_some((driver, function as usize))
        })
        .as_ref()?;
    Some((driver, *function as CuFunction))
}

fn cuda_gain(stream: *mut c_void, from: *mut c_void, to: *mut c_void, n: u32, gain: f32) -> bool {
    let Some((driver, function)) = cuda_kernel() else {
        return false;
    };
    let (mut from, mut to, mut n_arg, mut gain) = (from as u64, to as u64, n, gain);
    let mut args = [
        (&raw mut from).cast::<c_void>(),
        (&raw mut to).cast(),
        (&raw mut n_arg).cast(),
        (&raw mut gain).cast(),
    ];
    let stream: CuStream = stream;
    // SAFETY: four arguments of the kernel's four types, on device buffers the
    // host made at least `n` floats long.
    let launched = unsafe {
        (driver.launch_kernel)(
            function,
            n.div_ceil(256),
            1,
            1,
            256,
            1,
            1,
            0,
            stream,
            args.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    } == 0;
    // Without the host's stream the spec has the plugin finish before it
    // answers; with it, the host waits.
    // SAFETY: the driver's own entry point, on the current context.
    launched && (!stream.is_null() || unsafe { (driver.ctx_synchronize)() } == 0)
}

// ---------------------------------------------------------------- OpenCL --

/// The same gain in OpenCL C.
const OPENCL_C: &CStr = c"
__kernel void gain(__global const float* src, __global float* dst, uint n, float g) {
    size_t i = get_global_id(0);
    if (i < n) dst[i] = src[i] * g;
}
";

/// The loader and the built kernel, for the one context a broker has.
static OPENCL: OnceLock<Option<(Api, usize)>> = OnceLock::new();

fn opencl_kernel(queue: ClHandle) -> Option<(&'static Api, ClHandle)> {
    let (api, kernel) = OPENCL
        .get_or_init(|| {
            let api = Api::load()?;
            let mut context: ClHandle = std::ptr::null_mut();
            let mut device: ClHandle = std::ptr::null_mut();
            let size = size_of::<ClHandle>();
            // SAFETY: the host's queue, asked for two handles into handles.
            let asked = unsafe {
                (api.get_command_queue_info)(
                    queue,
                    QUEUE_CONTEXT,
                    size,
                    (&raw mut context).cast(),
                    std::ptr::null_mut(),
                ) | (api.get_command_queue_info)(
                    queue,
                    QUEUE_DEVICE,
                    size,
                    (&raw mut device).cast(),
                    std::ptr::null_mut(),
                )
            };
            if asked != 0 {
                return None;
            }
            let mut code = 0;
            let text: *const c_char = OPENCL_C.as_ptr();
            // SAFETY: one NUL-terminated source string, in the host's context.
            let program = unsafe {
                (api.create_program_with_source)(
                    context,
                    1,
                    &raw const text,
                    std::ptr::null(),
                    &raw mut code,
                )
            };
            if code != 0 || program.is_null() {
                return None;
            }
            // SAFETY: the program just made, for the queue's own device.
            let built = unsafe {
                (api.build_program)(
                    program,
                    1,
                    &raw const device,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
            };
            if built != 0 {
                return None;
            }
            // SAFETY: a kernel the program defines.
            let kernel = unsafe { (api.create_kernel)(program, c"gain".as_ptr(), &raw mut code) };
            (code == 0 && !kernel.is_null()).then_some((api, kernel as usize))
        })
        .as_ref()?;
    Some((api, *kernel as ClHandle))
}

fn opencl_gain(queue: *mut c_void, from: *mut c_void, to: *mut c_void, n: u32, gain: f32) -> bool {
    if queue.is_null() {
        return false;
    }
    let Some((api, kernel)) = opencl_kernel(queue) else {
        return false;
    };
    let global = n as usize;
    // SAFETY: each argument is the size and type the kernel declares, the two
    // buffers are the host's `cl_mem`s of at least `n` floats, and the broker
    // renders one frame at a time, so nobody else sets these arguments.
    let code = unsafe {
        (api.set_kernel_arg)(kernel, 0, size_of::<ClHandle>(), (&raw const from).cast())
            | (api.set_kernel_arg)(kernel, 1, size_of::<ClHandle>(), (&raw const to).cast())
            | (api.set_kernel_arg)(kernel, 2, size_of::<u32>(), (&raw const n).cast())
            | (api.set_kernel_arg)(kernel, 3, size_of::<f32>(), (&raw const gain).cast())
            | (api.enqueue_nd_range_kernel)(
                queue,
                kernel,
                1,
                std::ptr::null(),
                &raw const global,
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
    };
    // Not waited for: the host finishes its own queue.
    code == 0
}

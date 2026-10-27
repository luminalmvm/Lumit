//! Rendering a plugin on the graphics card: OFX 1.5's CUDA, OpenCL and Metal
//! render paths, the way Resolve offers them.
//!
//! # In plain terms
//!
//! A broker opens whatever this machine really has when it starts: CUDA and
//! OpenCL on Windows and Linux, Metal on macOS, each through the driver's own
//! library opened at run time, so a machine without one is a machine on the
//! CPU. The host table says yes to what opened and no to the rest, and each
//! plugin is given the best path it declared it can take. For that plugin's
//! begin, render and end, every image is a device buffer: the frames come off
//! the ring onto the card, the plugin renders there, and the output comes
//! back down into the ring.
//!
//! Lumit's own process opens nothing here. It hands its brokers two things as
//! they start, the "Render plugins on the graphics card" setting and the name
//! of the card Lumit draws with, and it keeps which path each plugin renders
//! through so the frame key can tell the two apart.
//!
//! # Thread role
//!
//! The card is used by the broker's main thread, one frame at a time. No lock
//! is held across a driver call.

// ponytail: every frame crosses the CPU twice, ring to card and back. Sharing
// memory with Lumit's own device removes both copies (D3D12 shared handles
// into CUDA, Vulkan fds, Metal on unified memory, as lumit-gpu's shared.rs and
// shared_linux.rs already export for Flutter).

pub mod cuda;
#[cfg(target_os = "macos")]
mod metal;
pub mod opencl;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::ffi::prop_keys as keys;
use crate::handles::Handle;
use crate::image::{Image, RowOrder};
use crate::instance::{set_images, take_all_images};
use crate::props::{PropValue, PropertySet};
use crate::render::{RenderError, OUTPUT_CLIP};

/// The broker's environment for the setting: `0` keeps it on the CPU, a name
/// (`cuda`, `opencl`, `metal`) opens only that one, and anything else opens
/// whatever there is.
pub const GPU_ENV: &str = "LUMIT_OFX_GPU";
/// The broker's environment for the card Lumit draws with, as the line
/// `lumit_gpu::adapter_summary` gives, whose start is the card's name.
pub const ADAPTER_ENV: &str = "LUMIT_OFX_GPU_ADAPTER";

/// How many times a plugin may fail on the card before it stays on the CPU for
/// the life of its broker.
pub const GPU_FAILURES_ALLOWED: u32 = 2;

/// One of the three ways a plugin can render on the card.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum GpuApi {
    Cuda,
    OpenCl,
    Metal,
}

impl GpuApi {
    /// The number the frame key carries for a path, nought being the CPU.
    #[must_use]
    pub const fn key(path: Option<Self>) -> i32 {
        match path {
            None => 0,
            Some(Self::Cuda) => 1,
            Some(Self::OpenCl) => 2,
            Some(Self::Metal) => 3,
        }
    }
}

/// What a plugin's descriptor says it can render with: `"true"` or `"needed"`.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct GpuSupport {
    pub cuda: bool,
    pub cuda_stream: bool,
    pub opencl: bool,
    pub metal: bool,
}

impl GpuSupport {
    /// Read off a plugin descriptor.
    pub(crate) fn declared(props: &PropertySet) -> Self {
        let yes = |key: &str| {
            props
                .get_string(key, 0)
                .is_ok_and(|value| matches!(value.to_bytes(), b"true" | b"needed"))
        };
        Self {
            cuda: yes(keys::CUDA_RENDER_SUPPORTED),
            cuda_stream: yes(keys::CUDA_STREAM_SUPPORTED),
            opencl: yes(keys::OPENCL_RENDER_SUPPORTED),
            metal: yes(keys::METAL_RENDER_SUPPORTED),
        }
    }
}

/// How one frame is rendered on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuRender {
    pub api: GpuApi,
    /// Whether the plugin is handed our CUDA stream, which it is only when it
    /// declared it can take one.
    pub stream: bool,
}

// ------------------------------------------------------------ in Lumit --

/// Set when the person has turned "Render plugins on the graphics card" off.
static CPU_ONLY: AtomicBool = AtomicBool::new(false);
/// The card Lumit draws with, for its brokers to find.
static ADAPTER: Mutex<Option<String>> = Mutex::new(None);

/// The setting, for brokers started from now on.
pub fn set_enabled(on: bool) {
    CPU_ONLY.store(!on, Ordering::Relaxed);
}

/// The card Lumit draws with, as `lumit_gpu::adapter_summary` gives it.
pub fn set_adapter(summary: Option<&str>) {
    *ADAPTER.lock() = summary.map(str::to_owned);
}

/// The environment a broker is started with for both.
pub(crate) fn broker_env() -> Vec<(String, String)> {
    let mut env = Vec::new();
    if CPU_ONLY.load(Ordering::Relaxed) {
        env.push((GPU_ENV.to_owned(), "0".to_owned()));
    }
    if let Some(adapter) = ADAPTER.lock().clone() {
        env.push((ADAPTER_ENV.to_owned(), adapter));
    }
    env
}

/// A plugin, by identifier and version.
type PluginKey = (String, (u32, u32));

/// The path each plugin renders through, as its broker last said. One entry
/// per plugin a scan found, replaced in place.
static PATHS: Mutex<BTreeMap<PluginKey, GpuApi>> = Mutex::new(BTreeMap::new());

/// Keep what a broker said about one plugin's path.
pub(crate) fn note_path(identifier: &str, version: (u32, u32), path: Option<GpuApi>) {
    let key = (identifier.to_owned(), version);
    let mut paths = PATHS.lock();
    match path {
        Some(api) => paths.insert(key, api),
        None => paths.remove(&key),
    };
}

/// The path a plugin renders through, `None` being the CPU and every plugin
/// hosted in Lumit's own process.
#[must_use]
pub fn path_of(identifier: &str, version: (u32, u32)) -> Option<GpuApi> {
    PATHS.lock().get(&(identifier.to_owned(), version)).copied()
}

/// Whether a driver's device name is the card the adapter line starts with.
/// The name must end where the line's bracket starts, so a 5080 is not a
/// 5080 Ti.
fn names_card(summary: &str, name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && summary
            .strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(" ("))
}

// ------------------------------------------------------- in the broker --

/// One opened API, as the render needs it.
trait Device: Send + Sync {
    /// Ready the API on this thread before a render.
    fn prepare(&self) -> Result<(), String> {
        Ok(())
    }
    /// The stream or queue a plugin enqueues on.
    fn queue(&self) -> usize;
    fn alloc(&self, bytes: usize) -> Result<usize, String>;
    fn upload(&self, at: usize, bytes: &[u8]) -> Result<(), String>;
    fn download(&self, at: usize, into: &mut [u8]) -> Result<(), String>;
    fn free(&self, at: usize);
    /// Wait for everything the plugin queued.
    fn finish(&self) -> Result<(), String>;
}

/// Spelled out once per API: each one's own methods, behind the one face.
macro_rules! device {
    ($api:ty $(, $prepare:ident)?) => {
        impl Device for $api {
            $(fn prepare(&self) -> Result<(), String> {
                self.$prepare()
            })?
            fn queue(&self) -> usize {
                <$api>::queue(self)
            }
            fn alloc(&self, bytes: usize) -> Result<usize, String> {
                <$api>::alloc(self, bytes)
            }
            fn upload(&self, at: usize, bytes: &[u8]) -> Result<(), String> {
                <$api>::upload(self, at, bytes)
            }
            fn download(&self, at: usize, into: &mut [u8]) -> Result<(), String> {
                <$api>::download(self, at, into)
            }
            fn free(&self, at: usize) {
                <$api>::free(self, at);
            }
            fn finish(&self) -> Result<(), String> {
                <$api>::finish(self)
            }
        }
    };
}
device!(cuda::Cuda, make_current);
device!(opencl::OpenCl);
#[cfg(target_os = "macos")]
device!(metal::Metal);

/// One API this broker found. Its context is only made at the first frame a
/// plugin renders through it, since every broker a scan starts would otherwise
/// hold a context on the card and a dozen of them can use up its memory.
struct Lazy<T> {
    here: bool,
    opened: OnceLock<Option<T>>,
}

impl<T> Lazy<T> {
    fn new(here: bool) -> Self {
        Self {
            here,
            opened: OnceLock::new(),
        }
    }

    /// The opened API, made the first time. Only the broker's main thread asks,
    /// so the driver call runs outside the cell and nothing waits on it.
    fn get(&self, open: impl FnOnce() -> Option<T>) -> Option<&T> {
        if !self.here {
            return None;
        }
        if self.opened.get().is_none() {
            let _ = self.opened.set(open());
        }
        self.opened.get().and_then(Option::as_ref)
    }
}

/// What this broker found.
struct Card {
    adapter: Option<String>,
    cuda: Lazy<cuda::Cuda>,
    opencl: Lazy<opencl::OpenCl>,
    #[cfg(target_os = "macos")]
    metal: Lazy<metal::Metal>,
    /// Lumit draws with an NVIDIA card, or did not say which, so CUDA goes first.
    nvidia: bool,
}

impl Card {
    fn device(&self, api: GpuApi) -> Option<&dyn Device> {
        let adapter = self.adapter.as_deref();
        match api {
            GpuApi::Cuda => self
                .cuda
                .get(|| cuda::Cuda::open(adapter))
                .map(|cuda| cuda as &dyn Device),
            GpuApi::OpenCl => self
                .opencl
                .get(|| opencl::OpenCl::open(adapter))
                .map(|cl| cl as &dyn Device),
            #[cfg(target_os = "macos")]
            GpuApi::Metal => self
                .metal
                .get(|| metal::Metal::open(adapter))
                .map(|metal| metal as &dyn Device),
            #[cfg(not(target_os = "macos"))]
            GpuApi::Metal => None,
        }
    }
}

static CARD: OnceLock<Card> = OnceLock::new();

/// Look for the card, once, before the bundle is loaded: the host table a
/// plugin first reads says what is there. A broker that never calls this, and
/// Lumit's own process, stays on the CPU.
pub fn open_in_broker() {
    let wanted = std::env::var(GPU_ENV).unwrap_or_default();
    let adapter = std::env::var(ADAPTER_ENV)
        .ok()
        .filter(|line| !line.is_empty());
    let adapter = adapter.as_deref();
    let off = wanted == "0";
    let named = |api: &str| {
        !off && (wanted.is_empty() || wanted.eq_ignore_ascii_case(api) || wanted == "1")
    };
    let desktop = cfg!(any(windows, target_os = "linux"));
    let card = Card {
        cuda: Lazy::new(desktop && named("cuda") && cuda::Cuda::present()),
        opencl: Lazy::new(desktop && named("opencl") && opencl::OpenCl::present()),
        #[cfg(target_os = "macos")]
        metal: Lazy::new(named("metal") && metal::Metal::present()),
        nvidia: adapter.is_none_or(|line| line.to_ascii_lowercase().contains("nvidia")),
        adapter: adapter.map(str::to_owned),
    };
    let _ = CARD.set(card);
}

/// What the host table offers: what is there.
pub(crate) fn available() -> GpuSupport {
    let Some(card) = CARD.get() else {
        return GpuSupport::default();
    };
    GpuSupport {
        cuda: card.cuda.here,
        cuda_stream: card.cuda.here,
        opencl: card.opencl.here,
        #[cfg(target_os = "macos")]
        metal: card.metal.here,
        #[cfg(not(target_os = "macos"))]
        metal: false,
    }
}

/// What each plugin described in this process declared about the card, by
/// identifier and version. One entry per plugin in the bundle, replaced when
/// a restart describes it again.
static DECLARED: Mutex<BTreeMap<PluginKey, GpuSupport>> = Mutex::new(BTreeMap::new());

/// Keep what a describe read off one plugin's descriptor.
pub(crate) fn note_declared(identifier: &str, version: (u32, u32), declared: GpuSupport) {
    DECLARED
        .lock()
        .insert((identifier.to_owned(), version), declared);
}

/// The path a plugin described in this process renders through: what it
/// declared, against what this broker opened.
#[must_use]
pub fn path_for(identifier: &str, version: (u32, u32)) -> Option<GpuRender> {
    let declared = DECLARED
        .lock()
        .get(&(identifier.to_owned(), version))
        .copied()?;
    choose(declared)
}

/// The best path for a plugin that declared `declared`: CUDA first on an
/// NVIDIA card, then OpenCL, then CUDA on any other, then Metal.
fn choose(declared: GpuSupport) -> Option<GpuRender> {
    let open = available();
    let cuda = declared.cuda && open.cuda;
    let opencl = declared.opencl && open.opencl;
    let nvidia = CARD.get().is_some_and(|card| card.nvidia);
    let api = if cuda && (nvidia || !opencl) {
        GpuApi::Cuda
    } else if opencl {
        GpuApi::OpenCl
    } else if declared.metal && open.metal {
        GpuApi::Metal
    } else {
        return None;
    };
    Some(GpuRender {
        api,
        stream: api == GpuApi::Cuda && declared.cuda_stream,
    })
}

/// The GPU render arguments of a begin, a render or an end: all off, which is
/// what a CPU render carries, or the one path this frame takes.
pub(crate) fn render_args(args: &mut PropertySet, on: Option<&DeviceImages>) {
    for key in [
        keys::OPENGL_ENABLED,
        keys::CUDA_ENABLED,
        keys::METAL_ENABLED,
        keys::OPENCL_ENABLED,
    ] {
        args.seed(key, PropValue::int(0));
    }
    for key in [
        keys::CUDA_STREAM,
        keys::METAL_COMMAND_QUEUE,
        keys::OPENCL_COMMAND_QUEUE,
    ] {
        args.seed(key, PropValue::Pointer(vec![0]));
    }
    let Some(on) = on else {
        return;
    };
    let queue = on.device.queue();
    let (enabled, queue) = match on.render.api {
        GpuApi::Cuda => (
            keys::CUDA_ENABLED,
            on.render.stream.then_some((keys::CUDA_STREAM, queue)),
        ),
        GpuApi::OpenCl => (
            keys::OPENCL_ENABLED,
            Some((keys::OPENCL_COMMAND_QUEUE, queue)),
        ),
        GpuApi::Metal => (
            keys::METAL_ENABLED,
            Some((keys::METAL_COMMAND_QUEUE, queue)),
        ),
    };
    args.seed(enabled, PropValue::int(1));
    if let Some((key, queue)) = queue {
        args.seed(key, PropValue::Pointer(vec![queue]));
    }
}

/// Every image of one render, on the card. Made before the begin, read back
/// after the end, and freed when dropped, once the card is done with them.
pub(crate) struct DeviceImages {
    device: &'static dyn Device,
    render: GpuRender,
    buffers: Vec<usize>,
    output: Option<usize>,
}

/// A GPU fault as the render's error, which the broker answers with a CPU
/// render of the same frame.
fn gpu_error(why: String) -> RenderError {
    RenderError::Gpu(why)
}

impl DeviceImages {
    /// Put every image on the instance onto the card, and its device address
    /// where the plugin will read its data pointer.
    pub(crate) fn upload(render: GpuRender, handle: Handle) -> Result<Self, RenderError> {
        let device = CARD
            .get()
            .and_then(|card| card.device(render.api))
            .ok_or_else(|| gpu_error("the graphics card is not open".to_owned()))?;
        device.prepare().map_err(gpu_error)?;
        // Off the instance for the copies, so no host lock is held across the
        // driver, and back on whatever happened.
        let (mut images, mut images_at) = take_all_images(handle)?;
        let mut frames = Self {
            device,
            render,
            buffers: Vec::new(),
            output: None,
        };
        let mut uploaded = Ok(());
        for (name, image) in &mut images {
            match frames.put(image) {
                Ok(at) if name == OUTPUT_CLIP => frames.output = Some(at),
                Ok(_) => {}
                Err(why) => uploaded = Err(why),
            }
        }
        for image in images_at.values_mut() {
            if let Err(why) = frames.put(image) {
                uploaded = Err(why);
            }
        }
        set_images(handle, images, images_at)?;
        uploaded.map_err(gpu_error)?;
        Ok(frames)
    }

    /// One image onto the card.
    fn put(&mut self, image: &mut Image) -> Result<usize, String> {
        // The broker only ever sends bottom-up pictures, and a device buffer
        // has no address inside it to start a top-down one at.
        if image.order() != RowOrder::BottomUp {
            return Err("a top-down image cannot go to the card".to_owned());
        }
        let at = self.device.alloc(image.bytes().len())?;
        self.buffers.push(at);
        self.device.upload(at, image.bytes())?;
        image.set_device(Some(at));
        Ok(at)
    }

    /// Wait for the plugin, and bring the output down into its host block.
    pub(crate) fn download(&self, handle: Handle) -> Result<(), RenderError> {
        self.device.finish().map_err(gpu_error)?;
        let (mut images, mut images_at) = take_all_images(handle)?;
        let read = match (self.output, images.get_mut(OUTPUT_CLIP)) {
            (Some(at), Some(output)) => self.device.download(at, output.bytes_mut()),
            _ => Err("the output never went to the card".to_owned()),
        };
        for image in images.values_mut().chain(images_at.values_mut()) {
            image.set_device(None);
        }
        set_images(handle, images, images_at)?;
        read.map_err(gpu_error)
    }
}

impl Drop for DeviceImages {
    fn drop(&mut self) {
        // A failed render may still have work queued on these buffers.
        let _ = self.device.finish();
        for at in self.buffers.drain(..) {
            self.device.free(at);
        }
    }
}

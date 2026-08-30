//! One live copy of one plugin: its whole life, in the order CLAP requires.
//!
//! # In plain terms
//!
//! A plugin has a life with stages, and the stages have to happen in order or
//! well-written plugins break. Lumit's order is written down once, here and in
//! [`crate::HOST_ACTIONS`]:
//!
//! create → init → **load the saved state** → set the parameters → activate →
//! start processing → block, block, block… → stop processing → deactivate →
//! destroy.
//!
//! Two things in that list are easy to get backwards and both matter. The state
//! blob is loaded **while the plugin is deactivated**, because a plugin
//! re-reads its own parameter ranges out of it and doing that mid-stream is
//! undefined. And the parameters are set **after** the state, because a saved
//! blob is last year's answer and the project's own keyframes are this year's:
//! **properties win over stale state** (docs/impl/audio-plugins.md §4). Loading
//! them the other way round is how a keyframed cutoff silently reverts to
//! whatever the preset had.
//!
//! # Threading
//!
//! Everything here is main-thread except [`Instance::process`], which is the
//! audio-thread half. One instance is processed single-threaded; parallelism is
//! across layers (§5). Nothing here takes a lock, so nothing can hold one
//! across a call into a plugin — the host callbacks a plugin can reach from
//! inside `process` ([`HostFlags`]) are three atomic flags and nothing else.

use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::params::{clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::render::{
    clap_plugin_render, CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE, CLAP_RENDER_REALTIME,
};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;
use thiserror::Error;

use crate::describe::{ParamDescription, PortInfo, Ports};
use crate::module::{text, Module, ModuleError};
use crate::process::{
    input_events, output_events, to_clap, Block, Denormals, BLOCK_FRAMES, CHANNELS, SAMPLE_RATE,
};
use crate::ParamEvent;

/// Everything a plugin can refuse to do, as a value.
///
/// None of these stops a composition: docs/12 §2.3's rule is that a plugin's
/// failure costs the plugin, and the seam that reads these turns them into a
/// calm badge on the layer.
#[derive(Debug, Error)]
pub enum HostError {
    /// The `.clap` file itself was the problem.
    #[error(transparent)]
    Module(#[from] ModuleError),
    /// The module has no plugin under that id.
    #[error("the module has no plugin called {0:?}")]
    NoSuchPlugin(String),
    /// The plugin id holds a nul byte, so it cannot cross the C boundary.
    #[error("the plugin id is not a C string")]
    BadId,
    /// The factory would not build it.
    #[error("the factory would not create the plugin")]
    NotCreated,
    /// `clap_plugin.init` answered false.
    #[error("the plugin refused to initialise")]
    InitRefused,
    /// The plugin does not offer an extension this host needs.
    #[error("the plugin implements no {0} extension")]
    NoExtension(&'static str),
    /// `activate` answered false.
    #[error("the plugin refused to activate at {SAMPLE_RATE} Hz in blocks of {BLOCK_FRAMES}")]
    ActivateRefused,
    /// `start_processing` answered false.
    #[error("the plugin refused to start processing")]
    StartRefused,
    /// `process` answered `CLAP_PROCESS_ERROR`.
    #[error("the plugin answered its block with an error")]
    ProcessFailed,
    /// `state.load` answered false — a blob from another version, most likely.
    #[error("the plugin refused the saved state")]
    StateRefused,
    /// `state.save` answered false.
    #[error("the plugin would not save its state")]
    StateUnsaved,
    /// A call that must happen while deactivated was made while processing
    /// (§9: never call state or describe functions while processing).
    #[error("that call may not be made while the plugin is processing")]
    WhileProcessing,
    /// A block was asked for before `start_processing`.
    #[error("the plugin is not processing")]
    NotProcessing,
    /// The block did not come back at all: the plugin crashed, hung past its
    /// deadline, or has been put away for the session. Only a brokered plugin
    /// can answer this way, and the caller's answer to it is one **dry** block
    /// with a ramp either side of the splice — never a stopped mix
    /// (docs/impl/audio-plugins.md §3).
    #[error("the block did not come back: {0}")]
    Failed(String),
}

/// The three things a plugin can ask of the host from inside a callback.
///
/// Flags rather than actions, and atomics rather than a lock, because a plugin
/// may set any of them **from the audio thread, during `process`** — a host
/// that took a lock there would be a host that deadlocks against its own
/// processing loop (docs/14 §7).
#[derive(Debug, Default)]
pub struct HostFlags {
    /// The plugin wants to be deactivated and activated again — how CLAP says
    /// "my latency changed" (§4). AP3 acts on it; AP1 records it.
    pub restart: AtomicBool,
    /// The plugin wants `process` called even though it said it was asleep.
    pub wants_process: AtomicBool,
    /// The plugin wants `on_main_thread` called.
    pub wants_callback: AtomicBool,
}

/// The host structure a plugin is handed, and the flags it can set.
///
/// Boxed and never moved: CLAP hands this exact pointer back on every callback,
/// for as long as the plugin lives.
pub struct HostBox {
    host: clap_host,
    flags: HostFlags,
}

impl HostBox {
    /// A host to hand one plugin.
    fn new() -> Box<Self> {
        let mut boxed = Box::new(Self {
            host: clap_host {
                clap_version: CLAP_VERSION,
                host_data: std::ptr::null_mut(),
                name: c"Lumit".as_ptr(),
                vendor: c"Lumit".as_ptr(),
                url: c"https://lumitlab.com".as_ptr(),
                version: c"0.2.0".as_ptr(),
                get_extension: Some(host_get_extension),
                request_restart: Some(host_request_restart),
                request_process: Some(host_request_process),
                request_callback: Some(host_request_callback),
            },
            flags: HostFlags::default(),
        });
        boxed.host.host_data = std::ptr::from_mut(boxed.as_mut()).cast();
        boxed
    }

    /// What the plugin has asked for since it was created.
    #[must_use]
    pub fn flags(&self) -> &HostFlags {
        &self.flags
    }
}

/// The host implements **no extensions** in v1.
///
/// That is a complete, honest answer rather than a gap: CLAP is built so a
/// missing host extension degrades cleanly, and every host extension worth
/// having (`params.rescan`, `latency.changed`, `gui`) belongs to a package that
/// has not landed — the panel surface AP5, the editor window after it (§6). A
/// host that claimed them and then ignored the calls would be worse than one
/// that says no.
unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    _id: *const std::ffi::c_char,
) -> *const c_void {
    std::ptr::null()
}

/// Recover the host box behind the pointer a plugin was given.
///
/// # Safety
///
/// `host` must be null or the `clap_host` a [`HostBox`] owns.
unsafe fn flags<'a>(host: *const clap_host) -> Option<&'a HostFlags> {
    if host.is_null() {
        return None;
    }
    // SAFETY: `host_data` is the box address `HostBox::new` wrote, and the box
    // outlives every plugin that holds it.
    unsafe {
        let data = (*host).host_data.cast::<HostBox>();
        data.as_ref().map(|boxed| &boxed.flags)
    }
}

unsafe extern "C" fn host_request_restart(host: *const clap_host) {
    // SAFETY: the plugin hands back the host it was given.
    if let Some(flags) = unsafe { flags(host) } {
        flags.restart.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_request_process(host: *const clap_host) {
    // SAFETY: as `host_request_restart`.
    if let Some(flags) = unsafe { flags(host) } {
        flags.wants_process.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_request_callback(host: *const clap_host) {
    // SAFETY: as `host_request_restart`.
    if let Some(flags) = unsafe { flags(host) } {
        flags.wants_callback.store(true, Ordering::Relaxed);
    }
}

/// One live plugin.
pub struct Instance {
    /// Kept so the module outlives the plugin: unloading a library with a live
    /// plugin in it is the crash nobody can read a stack trace for.
    module: Arc<Module>,
    /// Kept so the host structure outlives the plugin.
    host: Box<HostBox>,
    plugin: *const clap_plugin,
    activated: bool,
    processing: bool,
}

// SAFETY: an instance owns everything it points at — the module keeps the
// library loaded, the boxed host outlives the plugin — so moving it to the
// thread that will process it is moving the whole plugin, which is exactly what
// AP3's per-layer chain worker does.
//
// **Deliberately not `Sync`.** CLAP splits its functions into a main-thread and
// an audio-thread half, and an instance shared between two threads is a plugin
// asked to do both at once. One instance is processed single-threaded;
// parallelism is across layers (§5).
unsafe impl Send for Instance {}

impl Instance {
    /// Create one plugin from a module and initialise it.
    ///
    /// # Errors
    ///
    /// [`HostError::NoSuchPlugin`] when the module does not declare that id,
    /// [`HostError::NotCreated`] when the factory refuses, and
    /// [`HostError::InitRefused`] when the plugin will not start.
    pub fn create(module: Arc<Module>, plugin_id: &str) -> Result<Self, HostError> {
        if !module.entries().iter().any(|entry| entry.id == plugin_id) {
            return Err(HostError::NoSuchPlugin(plugin_id.to_owned()));
        }
        let id = CString::new(plugin_id).map_err(|_| HostError::BadId)?;
        let host = HostBox::new();
        // SAFETY: the host box is owned by the instance being built and is
        // never moved, so the pointer stays good until `destroy`.
        let plugin = unsafe { module.create(&id, std::ptr::addr_of!(host.host)) }
            .ok_or(HostError::NotCreated)?;

        let instance = Self {
            module,
            host,
            plugin,
            activated: false,
            processing: false,
        };
        let Some(init) = instance.vtable().init else {
            return Err(HostError::InitRefused);
        };
        // SAFETY: the plugin's own function, called once, before anything else.
        if !unsafe { init(instance.plugin) } {
            return Err(HostError::InitRefused);
        }
        Ok(instance)
    }

    /// The flags this plugin has raised.
    #[must_use]
    pub fn host_flags(&self) -> &HostFlags {
        self.host.flags()
    }

    /// The module this plugin came out of.
    #[must_use]
    pub fn module(&self) -> &Arc<Module> {
        &self.module
    }

    /// The plugin's own vtable.
    fn vtable(&self) -> &clap_plugin {
        // SAFETY: the factory answered with a live plugin and nothing has
        // destroyed it: `destroy` runs in `Drop` and nulls nothing after,
        // because the instance is gone at that point.
        unsafe { &*self.plugin }
    }

    /// One extension, or null.
    fn extension(&self, id: &CStr) -> *const c_void {
        let Some(get) = self.vtable().get_extension else {
            return std::ptr::null();
        };
        // SAFETY: the plugin's own function, with the plugin it belongs to and
        // a nul-terminated id.
        unsafe { get(self.plugin, id.as_ptr()) }
    }

    /// The plugin's audio ports, both directions.
    #[must_use]
    pub fn ports(&self) -> Ports {
        let table = self
            .extension(CLAP_EXT_AUDIO_PORTS)
            .cast::<clap_plugin_audio_ports>();
        if table.is_null() {
            return Ports::default();
        }
        // SAFETY: a non-null extension pointer is the plugin's own static
        // table, valid while the plugin lives.
        let table = unsafe { &*table };
        Ports {
            inputs: self.ports_in_direction(table, true),
            outputs: self.ports_in_direction(table, false),
        }
    }

    fn ports_in_direction(&self, table: &clap_plugin_audio_ports, is_input: bool) -> Vec<PortInfo> {
        let (Some(count), Some(get)) = (table.count, table.get) else {
            return Vec::new();
        };
        // SAFETY: the extension's own functions, with the plugin they belong to.
        let total = unsafe { count(self.plugin, is_input) };
        let mut ports = Vec::with_capacity(total as usize);
        for index in 0..total {
            let mut info = blank_port();
            // SAFETY: `index` is below the count just reported and `info` is a
            // writable `clap_audio_port_info`.
            if !unsafe { get(self.plugin, index, is_input, &mut info) } {
                continue;
            }
            ports.push(PortInfo {
                id: info.id,
                // SAFETY: the plugin filled a fixed array this host zeroed, so
                // the nul it must end in is there whatever the plugin wrote.
                name: unsafe { text(info.name.as_ptr()) },
                main: info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0,
                channels: info.channel_count,
            });
        }
        ports
    }

    /// Every parameter the plugin declares, in its own order.
    #[must_use]
    pub fn params(&self) -> Vec<ParamDescription> {
        let table = self.extension(CLAP_EXT_PARAMS).cast::<clap_plugin_params>();
        if table.is_null() {
            return Vec::new();
        }
        // SAFETY: as `ports`.
        let table = unsafe { &*table };
        let (Some(count), Some(get)) = (table.count, table.get_info) else {
            return Vec::new();
        };
        // SAFETY: the extension's own functions.
        let total = unsafe { count(self.plugin) };
        let mut params = Vec::with_capacity(total as usize);
        for index in 0..total {
            let mut info = blank_param();
            // SAFETY: `index` is below the count just reported.
            if !unsafe { get(self.plugin, index, &mut info) } {
                continue;
            }
            params.push(ParamDescription {
                id: info.id,
                // SAFETY: as the port names above.
                name: unsafe { text(info.name.as_ptr()) },
                // SAFETY: as above.
                module: unsafe { text(info.module.as_ptr()) },
                min: info.min_value,
                max: info.max_value,
                default: info.default_value,
                flags: info.flags,
            });
        }
        params
    }

    /// One parameter's live value, straight from the plugin.
    #[must_use]
    pub fn param_value(&self, id: u32) -> Option<f64> {
        let table = self.extension(CLAP_EXT_PARAMS).cast::<clap_plugin_params>();
        if table.is_null() {
            return None;
        }
        // SAFETY: as `ports`.
        let get = unsafe { &*table }.get_value?;
        let mut value = 0.0f64;
        // SAFETY: the extension's own function, with a writable double.
        unsafe { get(self.plugin, id, &mut value) }.then_some(value)
    }

    /// Whether the plugin implements the `latency` extension at all — the
    /// question a **describe** may ask, because it needs no active plugin.
    #[must_use]
    pub fn reports_latency(&self) -> bool {
        !self.extension(CLAP_EXT_LATENCY).is_null()
    }

    /// The latency the plugin reports, in samples. Nought when it implements
    /// no `latency` extension, which is most effects.
    ///
    /// CLAP calls this an **active-state** function, so it is asked only after
    /// [`Instance::activate`] — a describe reads
    /// [`Instance::reports_latency`] instead.
    #[must_use]
    pub fn latency(&self) -> u32 {
        let table = self
            .extension(CLAP_EXT_LATENCY)
            .cast::<clap_plugin_latency>();
        if table.is_null() {
            return 0;
        }
        // SAFETY: as `ports`.
        let Some(get) = unsafe { &*table }.get else {
            return 0;
        };
        // SAFETY: the extension's own function.
        unsafe { get(self.plugin) }
    }

    /// Tell the plugin whether this is an export or a preview.
    ///
    /// Offline is the export's mode: no deadline, and a plugin may take the
    /// slower, better path (§3). Answers false when the plugin implements no
    /// `render` extension, which is not a failure — most do not.
    pub fn set_offline(&mut self, offline: bool) -> bool {
        let table = self.extension(CLAP_EXT_RENDER).cast::<clap_plugin_render>();
        if table.is_null() {
            return false;
        }
        // SAFETY: as `ports`.
        let Some(set) = unsafe { &*table }.set else {
            return false;
        };
        let mode = if offline {
            CLAP_RENDER_OFFLINE
        } else {
            CLAP_RENDER_REALTIME
        };
        // SAFETY: the extension's own function.
        unsafe { set(self.plugin, mode) }
    }

    /// Hand the plugin the blob the project saved.
    ///
    /// # Errors
    ///
    /// [`HostError::WhileProcessing`] if the plugin is mid-stream — the trap
    /// §9 names — [`HostError::NoExtension`] if it saves no state, and
    /// [`HostError::StateRefused`] if it will not take this blob.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        if self.processing {
            return Err(HostError::WhileProcessing);
        }
        let table = self.extension(CLAP_EXT_STATE).cast::<clap_plugin_state>();
        if table.is_null() {
            return Err(HostError::NoExtension("state"));
        }
        // SAFETY: as `ports`.
        let load = unsafe { &*table }
            .load
            .ok_or(HostError::NoExtension("state"))?;

        let mut cursor = ReadCursor { bytes, read: 0 };
        let stream = clap_istream {
            ctx: std::ptr::from_mut(&mut cursor).cast(),
            read: Some(stream_read),
        };
        // SAFETY: the extension's own function, with a stream whose context
        // outlives the call.
        if unsafe { load(self.plugin, &stream) } {
            Ok(())
        } else {
            Err(HostError::StateRefused)
        }
    }

    /// The blob to write into the `.lum`. Never parsed, always round-tripped
    /// (§4).
    ///
    /// # Errors
    ///
    /// As [`Instance::load_state`], plus [`HostError::StateUnsaved`].
    pub fn save_state(&self) -> Result<Vec<u8>, HostError> {
        if self.processing {
            return Err(HostError::WhileProcessing);
        }
        let table = self.extension(CLAP_EXT_STATE).cast::<clap_plugin_state>();
        if table.is_null() {
            return Err(HostError::NoExtension("state"));
        }
        // SAFETY: as `ports`.
        let save = unsafe { &*table }
            .save
            .ok_or(HostError::NoExtension("state"))?;

        let mut sink: Vec<u8> = Vec::new();
        let stream = clap_ostream {
            ctx: std::ptr::from_mut(&mut sink).cast(),
            write: Some(stream_write),
        };
        // SAFETY: the extension's own function, with a stream whose context
        // outlives the call.
        if unsafe { save(self.plugin, &stream) } {
            Ok(sink)
        } else {
            Err(HostError::StateUnsaved)
        }
    }

    /// Set parameters outside a block — the "properties win" step, run after
    /// the state has been loaded and before the plugin is activated.
    ///
    /// # Errors
    ///
    /// [`HostError::WhileProcessing`] mid-stream, and
    /// [`HostError::NoExtension`] when the plugin has no parameters at all.
    pub fn flush_params(&mut self, events: &[ParamEvent]) -> Result<(), HostError> {
        if self.processing {
            return Err(HostError::WhileProcessing);
        }
        let table = self.extension(CLAP_EXT_PARAMS).cast::<clap_plugin_params>();
        if table.is_null() {
            return Err(HostError::NoExtension("params"));
        }
        // SAFETY: as `ports`.
        let flush = unsafe { &*table }
            .flush
            .ok_or(HostError::NoExtension("params"))?;

        let mut list: Vec<_> = events.iter().map(to_clap).collect();
        list.sort_by_key(|event| event.header.time);
        let mut slot: &[_] = &list;
        let incoming = input_events(&mut slot);
        let outgoing = output_events();
        // SAFETY: the extension's own function, with two lists that outlive the
        // call.
        unsafe { flush(self.plugin, &incoming, &outgoing) };
        Ok(())
    }

    /// Prepare the plugin for 512-frame blocks at 48 kHz.
    ///
    /// The minimum and maximum are the same number on purpose: Lumit's block
    /// size never varies, which is what makes two exports identical (§3).
    ///
    /// # Errors
    ///
    /// [`HostError::ActivateRefused`].
    pub fn activate(&mut self) -> Result<(), HostError> {
        if self.activated {
            return Ok(());
        }
        let frames = u32::try_from(BLOCK_FRAMES).unwrap_or(u32::MAX);
        let Some(activate) = self.vtable().activate else {
            return Err(HostError::ActivateRefused);
        };
        // SAFETY: the plugin's own function, after a successful `init`.
        if !unsafe { activate(self.plugin, SAMPLE_RATE, frames, frames) } {
            return Err(HostError::ActivateRefused);
        }
        self.activated = true;
        Ok(())
    }

    /// Enter the processing state.
    ///
    /// # Errors
    ///
    /// [`HostError::StartRefused`], including when the plugin was never
    /// activated.
    pub fn start_processing(&mut self) -> Result<(), HostError> {
        if self.processing {
            return Ok(());
        }
        if !self.activated {
            return Err(HostError::StartRefused);
        }
        let Some(start) = self.vtable().start_processing else {
            return Err(HostError::StartRefused);
        };
        // SAFETY: the plugin's own function, after a successful `activate`.
        if !unsafe { start(self.plugin) } {
            return Err(HostError::StartRefused);
        }
        self.processing = true;
        Ok(())
    }

    /// One block: the input planes in, the output planes out.
    ///
    /// `steady` is the running count of frames since the chain started, which
    /// is what CLAP's `steady_time` means and what lets a tempo-synced plugin
    /// know where it is.
    ///
    /// # Errors
    ///
    /// [`HostError::NotProcessing`] before `start_processing`, and
    /// [`HostError::ProcessFailed`] when the plugin answers with an error.
    pub fn process(&mut self, block: &mut Block, steady: i64) -> Result<(), HostError> {
        if !self.processing {
            return Err(HostError::NotProcessing);
        }
        let Some(process) = self.vtable().process else {
            return Err(HostError::NotProcessing);
        };

        let (input, output, events) = block.parts();
        let mut slot: &[_] = events;
        let incoming = input_events(&mut slot);
        let outgoing = output_events();

        // Separate in and out, always: in-place is where plugin bugs live (§9).
        let (in_left, in_right) = input.split_at_mut(BLOCK_FRAMES);
        let (out_left, out_right) = output.split_at_mut(BLOCK_FRAMES);
        let mut in_planes: [*mut f32; CHANNELS] = [in_left.as_mut_ptr(), in_right.as_mut_ptr()];
        let mut out_planes: [*mut f32; CHANNELS] = [out_left.as_mut_ptr(), out_right.as_mut_ptr()];

        let incoming_audio = clap_audio_buffer {
            data32: in_planes.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: u32::try_from(CHANNELS).unwrap_or(2),
            latency: 0,
            constant_mask: 0,
        };
        let mut outgoing_audio = clap_audio_buffer {
            data32: out_planes.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: u32::try_from(CHANNELS).unwrap_or(2),
            latency: 0,
            constant_mask: 0,
        };

        let call = clap_process {
            steady_time: steady,
            frames_count: u32::try_from(BLOCK_FRAMES).unwrap_or(u32::MAX),
            // No transport in v1: tempo is supplied only where the comp has a
            // confirmed BPM grid (§3), and that grid arrives with the mix seam.
            transport: std::ptr::null(),
            audio_inputs: &incoming_audio,
            audio_outputs: &mut outgoing_audio,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &incoming,
            out_events: &outgoing,
        };

        // Denormals off for the call and restored after it. Two instructions
        // per block, which is nothing beside the block itself, and it means a
        // caller cannot forget (§3).
        let _denormals = Denormals::on();
        // SAFETY: every pointer in `call` is to a live local that outlives this
        // call, the planes are `frames_count` long, and the plugin is in the
        // processing state CLAP requires for `process`.
        let status = unsafe { process(self.plugin, &call) };
        if status == CLAP_PROCESS_ERROR {
            return Err(HostError::ProcessFailed);
        }
        Ok(())
    }

    /// Leave the processing state.
    pub fn stop_processing(&mut self) {
        if !self.processing {
            return;
        }
        self.processing = false;
        if let Some(stop) = self.vtable().stop_processing {
            // SAFETY: the plugin's own function, paired with the
            // `start_processing` that succeeded.
            unsafe { stop(self.plugin) };
        }
    }

    /// Undo [`Instance::activate`]. Stops processing first, because CLAP says
    /// a plugin may not be deactivated while it is processing.
    pub fn deactivate(&mut self) {
        self.stop_processing();
        if !self.activated {
            return;
        }
        self.activated = false;
        if let Some(deactivate) = self.vtable().deactivate {
            // SAFETY: the plugin's own function, paired with the `activate`
            // that succeeded.
            unsafe { deactivate(self.plugin) };
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.deactivate();
        if let Some(destroy) = self.vtable().destroy {
            // SAFETY: the plugin's own function, called once, after it has been
            // deactivated. The module `Arc` is dropped after this, so the
            // library is still loaded while the plugin tears itself down.
            unsafe { destroy(self.plugin) };
        }
        let _ = &self.host;
    }
}

/// An empty port description for the plugin to fill in.
fn blank_port() -> clap_audio_port_info {
    clap_audio_port_info {
        id: 0,
        name: [0; 256],
        flags: 0,
        channel_count: 0,
        port_type: std::ptr::null(),
        in_place_pair: clap_sys::id::CLAP_INVALID_ID,
    }
}

/// An empty parameter description for the plugin to fill in.
fn blank_param() -> clap_param_info {
    clap_param_info {
        id: 0,
        flags: 0,
        cookie: std::ptr::null_mut(),
        name: [0; 256],
        module: [0; 1024],
        min_value: 0.0,
        max_value: 0.0,
        default_value: 0.0,
    }
}

// ---------------------------------------------------------------- streams --

/// Where a state load has got to in the blob.
struct ReadCursor<'a> {
    bytes: &'a [u8],
    read: usize,
}

/// # Safety
///
/// `stream.ctx` must be the [`ReadCursor`] [`Instance::load_state`] put there,
/// and `buffer` writable for `size` bytes.
unsafe extern "C" fn stream_read(
    stream: *const clap_istream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    if stream.is_null() || buffer.is_null() {
        return -1;
    }
    // SAFETY: the host built this stream and set `ctx` from a live cursor.
    let cursor = unsafe { &mut *(*stream).ctx.cast::<ReadCursor<'_>>() };
    let left = cursor.bytes.len().saturating_sub(cursor.read);
    let wanted = usize::try_from(size).unwrap_or(usize::MAX).min(left);
    if wanted == 0 {
        return 0;
    }
    // SAFETY: `wanted` bytes remain in the blob and the caller guarantees the
    // buffer holds them.
    unsafe {
        std::ptr::copy_nonoverlapping(
            cursor.bytes.as_ptr().add(cursor.read),
            buffer.cast::<u8>(),
            wanted,
        );
    }
    cursor.read = cursor.read.saturating_add(wanted);
    i64::try_from(wanted).unwrap_or(i64::MAX)
}

/// # Safety
///
/// `stream.ctx` must be the `Vec<u8>` [`Instance::save_state`] put there, and
/// `buffer` readable for `size` bytes.
unsafe extern "C" fn stream_write(
    stream: *const clap_ostream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    if stream.is_null() || buffer.is_null() {
        return -1;
    }
    // SAFETY: the host built this stream and set `ctx` from a live vector.
    let sink = unsafe { &mut *(*stream).ctx.cast::<Vec<u8>>() };
    let count = usize::try_from(size).unwrap_or(usize::MAX);
    if count == 0 {
        return 0;
    }
    // A plugin in a loop must not fill our memory: a state blob beyond this is
    // not a state blob (docs/12 §2.3's rule about a plugin's appetite).
    const CAP: usize = 64 * 1024 * 1024;
    if sink.len().saturating_add(count) > CAP {
        return -1;
    }
    // SAFETY: the caller guarantees `count` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), count) };
    sink.extend_from_slice(bytes);
    i64::try_from(count).unwrap_or(i64::MAX)
}

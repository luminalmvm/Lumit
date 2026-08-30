//! `lumit-aplug-testplug` — minimal CLAP plugins, for testing the host.
//!
//! # In plain terms
//!
//! A host cannot be tested against nothing, and it should not be tested only
//! against somebody else's plugin: a commercial plugin cannot be shipped in a
//! repository, and a free one changes underneath the tests. So these are
//! plugins of our own — the smallest things that are genuinely CLAP plugins.
//! A `.clap` file is a shared library exporting one symbol, `clap_entry`, and
//! that is exactly what this crate builds.
//!
//! There are eight, because the host has eight kinds of answer to give and each
//! needs something to give it to (see [`Kind`]): a plugin that multiplies by a
//! number, a plugin that writes down every call the host made, a plugin that
//! claims latency, a plugin that dies mid-block, a plugin that never comes
//! back, a plugin whose saved state is whatever it was handed, a plugin that
//! writes down the parameter events it was sent, and a plugin with no audio
//! input at all — an instrument, which the host must turn away at scan time.
//!
//! Three extra exports of their own — names beginning `LumitTestPlug` — are how
//! a test asks what was seen. They are read by opening the same library a
//! second time: the host keeps its handle private, and a second `LoadLibrary`
//! of the same path answers with the same module, so the statics are the ones
//! the host's copy has been writing to.
//!
//! **The dangerous two are disarmed by default.** [`Kind::Crash`] and
//! [`Kind::Hang`] pass audio straight through unless [`CRASH_ON_BLOCK_ENV`] or
//! [`HANG_ENV`] is set in the environment, because a scan describes every
//! plugin in the module and a plugin that hangs at describe would hang the
//! scan. AP2 sets them in the broker's environment, which is the only way to
//! reach a plugin that is not in the test's own process any more.

#![allow(non_upper_case_globals)]

use std::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, OnceLock};

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    clap_event_param_value, clap_input_events, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS,
    CLAP_PORT_STEREO,
};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_AUTOMATABLE,
    CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
};
use clap_sys::ext::render::{clap_plugin_render, CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::id::{clap_id, CLAP_INVALID_ID};
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;

// ------------------------------------------------------------- the eight --

/// Which of the eight this plugin is. The order is the factory's order, so an
/// index and a kind are the same fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Multiplies both channels by its Gain parameter. The sample-exact one.
    Gain,
    /// Writes every call the host made into [`LumitTestPlugLog`]'s log.
    Reporter,
    /// Reports [`latency`] samples of latency and nothing else.
    Latency,
    /// Aborts the process during block [`CRASH_ON_BLOCK_ENV`], if it is set.
    Crash,
    /// Never returns from `process`, if [`HANG_ENV`] is set.
    Hang,
    /// Hands back, byte for byte, whatever state it was loaded with.
    StateEcho,
    /// Writes every parameter event it was sent into the parameter log.
    ParamEcho,
    /// An instrument: no audio input at all, which the host must refuse.
    Instrument,
}

/// The eight, in factory order.
pub const KINDS: [Kind; 8] = [
    Kind::Gain,
    Kind::Reporter,
    Kind::Latency,
    Kind::Crash,
    Kind::Hang,
    Kind::StateEcho,
    Kind::ParamEcho,
    Kind::Instrument,
];

impl Kind {
    /// The plugin id the factory answers to, nul-terminated.
    #[must_use]
    pub const fn id(self) -> &'static [u8] {
        match self {
            Kind::Gain => b"com.lumit.aplug.testplug.gain\0",
            Kind::Reporter => b"com.lumit.aplug.testplug.reporter\0",
            Kind::Latency => b"com.lumit.aplug.testplug.latency\0",
            Kind::Crash => b"com.lumit.aplug.testplug.crash\0",
            Kind::Hang => b"com.lumit.aplug.testplug.hang\0",
            Kind::StateEcho => b"com.lumit.aplug.testplug.state\0",
            Kind::ParamEcho => b"com.lumit.aplug.testplug.paramecho\0",
            Kind::Instrument => b"com.lumit.aplug.testplug.instrument\0",
        }
    }

    /// The name a person would see.
    #[must_use]
    pub const fn name(self) -> &'static [u8] {
        match self {
            Kind::Gain => b"Lumit test gain\0",
            Kind::Reporter => b"Lumit test reporter\0",
            Kind::Latency => b"Lumit test latency\0",
            Kind::Crash => b"Lumit test crash\0",
            Kind::Hang => b"Lumit test hang\0",
            Kind::StateEcho => b"Lumit test state echo\0",
            Kind::ParamEcho => b"Lumit test param echo\0",
            Kind::Instrument => b"Lumit test instrument\0",
        }
    }

    /// The parameters this kind declares: id, name, range, default, flags.
    #[must_use]
    fn params(self) -> &'static [ParamDecl] {
        match self {
            Kind::Gain => &[ParamDecl {
                id: PARAM_GAIN,
                name: b"Gain\0",
                min: 0.0,
                max: 4.0,
                default: 1.0,
                flags: CLAP_PARAM_IS_AUTOMATABLE,
            }],
            Kind::Reporter => &[ParamDecl {
                id: PARAM_KNOB,
                name: b"Knob\0",
                min: 0.0,
                max: 1.0,
                default: 0.5,
                flags: CLAP_PARAM_IS_AUTOMATABLE,
            }],
            Kind::ParamEcho => &[
                ParamDecl {
                    id: PARAM_SWEEP,
                    name: b"Sweep\0",
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    flags: CLAP_PARAM_IS_AUTOMATABLE,
                },
                ParamDecl {
                    id: PARAM_SECRET,
                    name: b"Secret\0",
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    flags: CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_HIDDEN,
                },
                ParamDecl {
                    id: PARAM_FIXED,
                    name: b"Fixed\0",
                    min: 0.0,
                    max: 1.0,
                    default: 0.25,
                    flags: CLAP_PARAM_IS_READONLY,
                },
            ],
            _ => &[],
        }
    }

    /// Whether this kind has an audio input. Only the instrument has none.
    #[must_use]
    const fn has_input(self) -> bool {
        !matches!(self, Kind::Instrument)
    }
}

/// One declared parameter.
struct ParamDecl {
    id: clap_id,
    name: &'static [u8],
    min: f64,
    max: f64,
    default: f64,
    flags: u32,
}

/// [`Kind::Gain`]'s only parameter — the multiplier.
pub const PARAM_GAIN: clap_id = 1;
/// [`Kind::Reporter`]'s only parameter, so a `params.flush` has something to
/// flush.
pub const PARAM_KNOB: clap_id = 5;
/// [`Kind::ParamEcho`]'s automatable parameter — the one that gets a row.
pub const PARAM_SWEEP: clap_id = 7;
/// [`Kind::ParamEcho`]'s hidden parameter, which must get **no** row.
pub const PARAM_SECRET: clap_id = 8;
/// [`Kind::ParamEcho`]'s read-only parameter, which must get **no** row.
pub const PARAM_FIXED: clap_id = 9;

/// How many samples of latency [`Kind::Latency`] claims, unless
/// [`LATENCY_ENV`] says otherwise.
pub const LATENCY_DEFAULT: u32 = 64;

/// Overrides [`LATENCY_DEFAULT`].
pub const LATENCY_ENV: &str = "LUMIT_APLUG_LATENCY";

/// Abort the process partway through this block index (nought-based). Unset
/// leaves [`Kind::Crash`] a passthrough.
pub const CRASH_ON_BLOCK_ENV: &str = "LUMIT_APLUG_CRASH_ON_BLOCK";

/// Any value makes [`Kind::Hang`] never return from `process`. Unset leaves it
/// a passthrough.
pub const HANG_ENV: &str = "LUMIT_APLUG_HANG";

/// What [`Kind::StateEcho`] saves when it has never been loaded.
pub const STATE_ECHO_DEFAULT: &[u8] = b"lumit-aplug-testplug/state";

// -------------------------------------------------------------- the logs --

/// Every call the host made to [`Kind::Reporter`], in order.
static LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Every parameter event [`Kind::ParamEcho`] was sent, as
/// `block:time:id:value`.
static PARAM_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Record one host call. Only the reporter writes: a test that reads the log
/// would otherwise see whatever else the suite happened to be doing.
fn note(kind: Kind, what: &'static str) {
    if kind == Kind::Reporter {
        if let Ok(mut log) = LOG.lock() {
            log.push(what);
        }
    }
}

/// Empty both logs. Call before the run whose calls are being counted.
#[no_mangle]
pub extern "C" fn LumitTestPlugResetLog() {
    if let Ok(mut log) = LOG.lock() {
        log.clear();
    }
    if let Ok(mut log) = PARAM_LOG.lock() {
        log.clear();
    }
}

/// Copy the host-call log into `buf` as comma-separated ASCII and answer how
/// many bytes were written. A `buf` of null, or a capacity too small, writes
/// nothing and answers the length that would have been needed.
///
/// # Safety
///
/// `buf` must be writable for `cap` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn LumitTestPlugLog(buf: *mut c_char, cap: u32) -> u32 {
    let text = LOG.lock().map(|log| log.join(",")).unwrap_or_default();
    copy_out(&text, buf, cap)
}

/// The parameter-event log, comma separated, each entry
/// `block:time:id:value`.
///
/// # Safety
///
/// `buf` must be writable for `cap` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn LumitTestPlugParamLog(buf: *mut c_char, cap: u32) -> u32 {
    let text = PARAM_LOG
        .lock()
        .map(|log| log.join(","))
        .unwrap_or_default();
    copy_out(&text, buf, cap)
}

/// Write `text` into `buf`, nul-terminated, and answer its byte length.
unsafe fn copy_out(text: &str, buf: *mut c_char, cap: u32) -> u32 {
    let len = text.len();
    let wanted = u32::try_from(len).unwrap_or(u32::MAX);
    if buf.is_null() || cap as usize <= len {
        return wanted;
    }
    // SAFETY: the caller guarantees `cap` writable bytes, and `len < cap`.
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr().cast::<c_char>(), buf, len);
        *buf.add(len) = 0;
    }
    wanted
}

// -------------------------------------------------------------- the entry --

/// The one symbol a `.clap` file must export.
#[no_mangle]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};

unsafe extern "C" fn entry_init(_path: *const c_char) -> bool {
    true
}

unsafe extern "C" fn entry_deinit() {}

unsafe extern "C" fn entry_get_factory(id: *const c_char) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: CLAP guarantees a nul-terminated string here.
    let asked = unsafe { CStr::from_ptr(id) };
    if asked == CLAP_PLUGIN_FACTORY_ID {
        return std::ptr::addr_of!(FACTORY).cast();
    }
    std::ptr::null()
}

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

/// The eight descriptors, built once and never moved — the host keeps the
/// pointers for as long as the module is loaded.
fn descriptors() -> &'static [clap_plugin_descriptor; 8] {
    static ONCE: OnceLock<[clap_plugin_descriptor; 8]> = OnceLock::new();
    ONCE.get_or_init(|| KINDS.map(descriptor_of))
}

/// The feature list every one of them declares: an audio effect, in stereo.
/// Null-terminated, as CLAP requires.
static FEATURES: Features = Features([
    c"audio-effect".as_ptr(),
    c"stereo".as_ptr(),
    std::ptr::null(),
]);

/// A list of C strings, safe to share because nothing ever writes it.
struct Features([*const c_char; 3]);
// SAFETY: the pointers are to `'static` byte literals and the array is never
// mutated, so every thread reads the same immutable bytes.
unsafe impl Sync for Features {}

fn descriptor_of(kind: Kind) -> clap_plugin_descriptor {
    clap_plugin_descriptor {
        clap_version: CLAP_VERSION,
        id: kind.id().as_ptr().cast(),
        name: kind.name().as_ptr().cast(),
        vendor: c"Lumit".as_ptr(),
        url: c"https://lumitlab.com".as_ptr(),
        manual_url: std::ptr::null(),
        support_url: std::ptr::null(),
        version: c"2.0.0".as_ptr(),
        description: c"A host test fixture, not an effect".as_ptr(),
        features: FEATURES.0.as_ptr(),
    }
}

unsafe extern "C" fn factory_count(_factory: *const clap_plugin_factory) -> u32 {
    8
}

unsafe extern "C" fn factory_descriptor(
    _factory: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    let Some(kind) = KINDS.get(index as usize).copied() else {
        return std::ptr::null();
    };
    note(kind, "factory");
    &descriptors()[index as usize]
}

unsafe extern "C" fn factory_create(
    _factory: *const clap_plugin_factory,
    host: *const clap_host,
    plugin_id: *const c_char,
) -> *const clap_plugin {
    if plugin_id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: CLAP guarantees a nul-terminated string here.
    let asked = unsafe { CStr::from_ptr(plugin_id) }.to_bytes_with_nul();
    let Some(index) = KINDS.iter().position(|kind| kind.id() == asked) else {
        return std::ptr::null();
    };
    let kind = KINDS[index];
    note(kind, "create");

    let mut plug = Box::new(Plug {
        vtable: VTABLE,
        kind,
        host,
        gain: 1.0,
        state: Vec::new(),
        loaded: false,
        block: 0,
        activated: false,
    });
    plug.vtable.desc = &descriptors()[index];
    plug.vtable.plugin_data = std::ptr::from_mut(plug.as_mut()).cast();
    let raw = Box::into_raw(plug);
    // SAFETY: `raw` was just built by `Box::into_raw` and is not null.
    unsafe { std::ptr::addr_of!((*raw).vtable) }
}

// ------------------------------------------------------------- the plugin --

/// One live copy of one of the eight.
struct Plug {
    /// What the host holds. Its `plugin_data` points back at this struct.
    vtable: clap_plugin,
    kind: Kind,
    /// The host, kept so a plugin *could* call back. None of the eight does.
    #[allow(dead_code)]
    host: *const clap_host,
    /// [`Kind::Gain`]'s multiplier, and every other kind's ignored number.
    gain: f64,
    /// What [`Kind::StateEcho`] was handed, byte for byte.
    state: Vec<u8>,
    /// Whether `state` came from a load rather than from nothing.
    loaded: bool,
    /// How many blocks have been processed since activation.
    block: u32,
    activated: bool,
}

static VTABLE: clap_plugin = clap_plugin {
    desc: std::ptr::null(),
    plugin_data: std::ptr::null_mut(),
    init: Some(plugin_init),
    destroy: Some(plugin_destroy),
    activate: Some(plugin_activate),
    deactivate: Some(plugin_deactivate),
    start_processing: Some(plugin_start_processing),
    stop_processing: Some(plugin_stop_processing),
    reset: Some(plugin_reset),
    process: Some(plugin_process),
    get_extension: Some(plugin_get_extension),
    on_main_thread: Some(plugin_on_main_thread),
};

/// Recover the plugin behind a vtable pointer, or `None` for a bad one.
///
/// # Safety
///
/// `plugin` must be a pointer this crate handed the host, or null.
unsafe fn plug<'a>(plugin: *const clap_plugin) -> Option<&'a mut Plug> {
    if plugin.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees one of ours; `plugin_data` is the `Box`
    // address `factory_create` wrote and the box outlives every call but
    // `destroy`, which takes it back.
    unsafe {
        let data = (*plugin).plugin_data.cast::<Plug>();
        data.as_mut()
    }
}

unsafe extern "C" fn plugin_init(plugin: *const clap_plugin) -> bool {
    // SAFETY: the host hands back what `factory_create` gave it.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "init");
    for decl in this.kind.params() {
        if decl.id == PARAM_GAIN {
            this.gain = decl.default;
        }
    }
    true
}

unsafe extern "C" fn plugin_destroy(plugin: *const clap_plugin) {
    if plugin.is_null() {
        return;
    }
    // SAFETY: the host hands back what `factory_create` gave it, once.
    unsafe {
        let data = (*plugin).plugin_data.cast::<Plug>();
        if data.is_null() {
            return;
        }
        note((*data).kind, "destroy");
        drop(Box::from_raw(data));
    }
}

unsafe extern "C" fn plugin_activate(
    plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "activate");
    this.activated = true;
    this.block = 0;
    true
}

unsafe extern "C" fn plugin_deactivate(plugin: *const clap_plugin) {
    // SAFETY: as `plugin_init`.
    if let Some(this) = unsafe { plug(plugin) } {
        note(this.kind, "deactivate");
        this.activated = false;
    }
}

unsafe extern "C" fn plugin_start_processing(plugin: *const clap_plugin) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "start_processing");
    this.activated
}

unsafe extern "C" fn plugin_stop_processing(plugin: *const clap_plugin) {
    // SAFETY: as `plugin_init`.
    if let Some(this) = unsafe { plug(plugin) } {
        note(this.kind, "stop_processing");
    }
}

unsafe extern "C" fn plugin_reset(plugin: *const clap_plugin) {
    // SAFETY: as `plugin_init`.
    if let Some(this) = unsafe { plug(plugin) } {
        this.block = 0;
    }
}

unsafe extern "C" fn plugin_on_main_thread(_plugin: *const clap_plugin) {}

unsafe extern "C" fn plugin_process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return clap_sys::process::CLAP_PROCESS_ERROR;
    };
    note(this.kind, "process");
    if process.is_null() {
        return clap_sys::process::CLAP_PROCESS_ERROR;
    }
    // SAFETY: CLAP guarantees a live `clap_process` for the call's duration.
    let p = unsafe { &*process };
    let block = this.block;
    this.block = this.block.saturating_add(1);

    // SAFETY: the event list, when present, is the host's and lives for the
    // call.
    unsafe { this.read_events(p.in_events, block) };

    if this.kind == Kind::Hang && std::env::var_os(HANG_ENV).is_some() {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    if this.kind == Kind::Crash {
        let at = std::env::var(CRASH_ON_BLOCK_ENV)
            .ok()
            .and_then(|text| text.parse::<u32>().ok());
        if at == Some(block) {
            std::process::abort();
        }
    }

    // SAFETY: the port arrays are the host's and live for the call.
    unsafe { this.copy_audio(p) };
    CLAP_PROCESS_CONTINUE
}

impl Plug {
    /// Take every parameter value out of the host's list.
    ///
    /// # Safety
    ///
    /// `events` must be null or a live `clap_input_events`.
    unsafe fn read_events(&mut self, events: *const clap_input_events, block: u32) {
        if events.is_null() {
            return;
        }
        // SAFETY: the caller guarantees a live list.
        let list = unsafe { &*events };
        let (Some(size), Some(get)) = (list.size, list.get) else {
            return;
        };
        // SAFETY: the two are the list's own functions, called with the list.
        let count = unsafe { size(events) };
        for index in 0..count {
            // SAFETY: `index` is below the size the list just reported.
            let header = unsafe { get(events, index) };
            if header.is_null() {
                continue;
            }
            // SAFETY: a non-null header is a live event for the call.
            let head = unsafe { *header };
            if head.space_id != CLAP_CORE_EVENT_SPACE_ID || head.type_ != CLAP_EVENT_PARAM_VALUE {
                continue;
            }
            // SAFETY: the type says this header opens a `clap_event_param_value`.
            let event = unsafe { *header.cast::<clap_event_param_value>() };
            self.apply(event.param_id, event.value);
            if self.kind == Kind::ParamEcho {
                if let Ok(mut log) = PARAM_LOG.lock() {
                    log.push(format!(
                        "{block}:{}:{}:{:.6}",
                        head.time, event.param_id, event.value
                    ));
                }
            }
        }
    }

    /// One parameter, set.
    fn apply(&mut self, id: clap_id, value: f64) {
        if id == PARAM_GAIN {
            self.gain = value;
        }
    }

    /// Input to output, times the gain. Silence when there is no input.
    ///
    /// # Safety
    ///
    /// The port arrays and their channel pointers must be the host's, live for
    /// the call, and `frames_count` long.
    unsafe fn copy_audio(&self, p: &clap_process) {
        let frames = p.frames_count as usize;
        let gain = if self.kind == Kind::Gain {
            self.gain as f32
        } else {
            1.0
        };
        if p.audio_outputs.is_null() || p.audio_outputs_count == 0 {
            return;
        }
        // SAFETY: the host declared at least one output port.
        let out: &clap_audio_buffer = unsafe { &*p.audio_outputs };
        let input: Option<&clap_audio_buffer> =
            if p.audio_inputs.is_null() || p.audio_inputs_count == 0 {
                None
            } else {
                // SAFETY: the host declared at least one input port.
                Some(unsafe { &*p.audio_inputs })
            };
        for channel in 0..out.channel_count as usize {
            if out.data32.is_null() {
                return;
            }
            // SAFETY: `channel` is below the port's declared channel count.
            let dst = unsafe { *out.data32.add(channel) };
            if dst.is_null() {
                continue;
            }
            let src = input.and_then(|buffer| {
                if buffer.data32.is_null() || channel >= buffer.channel_count as usize {
                    return None;
                }
                // SAFETY: `channel` is below the input port's channel count.
                let plane = unsafe { *buffer.data32.add(channel) };
                (!plane.is_null()).then_some(plane)
            });
            for frame in 0..frames {
                // SAFETY: both planes are `frames_count` long by contract.
                unsafe {
                    let value = src.map_or(0.0, |plane| *plane.add(frame));
                    *dst.add(frame) = value * gain;
                }
            }
        }
    }
}

// --------------------------------------------------------- the extensions --

unsafe extern "C" fn plugin_get_extension(
    plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return std::ptr::null();
    };
    // SAFETY: CLAP guarantees a nul-terminated string here.
    let asked = unsafe { CStr::from_ptr(id) };
    if asked == CLAP_EXT_AUDIO_PORTS {
        return std::ptr::addr_of!(AUDIO_PORTS).cast();
    }
    if asked == CLAP_EXT_PARAMS && !this.kind.params().is_empty() {
        return std::ptr::addr_of!(PARAMS).cast();
    }
    if asked == CLAP_EXT_STATE {
        return std::ptr::addr_of!(STATE).cast();
    }
    // Every kind offers the extension, because every real host asks every
    // plugin. Only [`Kind::Latency`] answers with a number.
    if asked == CLAP_EXT_LATENCY {
        return std::ptr::addr_of!(LATENCY).cast();
    }
    if asked == CLAP_EXT_RENDER {
        return std::ptr::addr_of!(RENDER).cast();
    }
    std::ptr::null()
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(ports_count),
    get: Some(ports_get),
};

unsafe extern "C" fn ports_count(plugin: *const clap_plugin, is_input: bool) -> u32 {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return 0;
    };
    note(this.kind, "audio_ports.count");
    u32::from(!is_input || this.kind.has_input())
}

unsafe extern "C" fn ports_get(
    plugin: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "audio_ports.get");
    if index != 0 || info.is_null() || (is_input && !this.kind.has_input()) {
        return false;
    }
    let mut filled = clap_audio_port_info {
        id: 0,
        name: [0; 256],
        flags: CLAP_AUDIO_PORT_IS_MAIN,
        channel_count: 2,
        port_type: CLAP_PORT_STEREO.as_ptr(),
        in_place_pair: CLAP_INVALID_ID,
    };
    fill(&mut filled.name, if is_input { b"In" } else { b"Out" });
    // SAFETY: the host gave a writable `clap_audio_port_info`.
    unsafe { *info = filled };
    true
}

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_get_info),
    get_value: Some(params_get_value),
    value_to_text: None,
    text_to_value: None,
    flush: Some(params_flush),
};

unsafe extern "C" fn params_count(plugin: *const clap_plugin) -> u32 {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return 0;
    };
    note(this.kind, "params.count");
    u32::try_from(this.kind.params().len()).unwrap_or(0)
}

unsafe extern "C" fn params_get_info(
    plugin: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "params.get_info");
    let Some(decl) = this.kind.params().get(index as usize) else {
        return false;
    };
    if info.is_null() {
        return false;
    }
    let mut filled = clap_param_info {
        id: decl.id,
        flags: decl.flags,
        cookie: std::ptr::null_mut(),
        name: [0; 256],
        module: [0; 1024],
        min_value: decl.min,
        max_value: decl.max,
        default_value: decl.default,
    };
    fill(&mut filled.name, decl.name);
    // SAFETY: the host gave a writable `clap_param_info`.
    unsafe { *info = filled };
    true
}

unsafe extern "C" fn params_get_value(
    plugin: *const clap_plugin,
    id: clap_id,
    out: *mut f64,
) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    if out.is_null() || id != PARAM_GAIN {
        return false;
    }
    // SAFETY: the host gave a writable double.
    unsafe { *out = this.gain };
    true
}

unsafe extern "C" fn params_flush(
    plugin: *const clap_plugin,
    in_: *const clap_input_events,
    _out: *const clap_sys::events::clap_output_events,
) {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return;
    };
    note(this.kind, "params.flush");
    // SAFETY: the host's list is live for the call.
    unsafe { this.read_events(in_, u32::MAX) };
}

static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

unsafe extern "C" fn state_save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "state.save");
    if stream.is_null() {
        return false;
    }
    let bytes: Vec<u8> = match this.kind {
        Kind::StateEcho if this.loaded => this.state.clone(),
        Kind::StateEcho => STATE_ECHO_DEFAULT.to_vec(),
        _ => this.gain.to_le_bytes().to_vec(),
    };
    // SAFETY: the host gave a live output stream.
    let stream_ref = unsafe { &*stream };
    let Some(write) = stream_ref.write else {
        return false;
    };
    let mut sent = 0usize;
    while sent < bytes.len() {
        // SAFETY: the slice is live and `bytes.len() - sent` long from `sent`.
        let wrote = unsafe {
            write(
                stream,
                bytes.as_ptr().add(sent).cast(),
                (bytes.len() - sent) as u64,
            )
        };
        if wrote <= 0 {
            return false;
        }
        sent = sent.saturating_add(wrote as usize);
    }
    true
}

unsafe extern "C" fn state_load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(this.kind, "state.load");
    if stream.is_null() {
        return false;
    }
    // SAFETY: the host gave a live input stream.
    let stream_ref = unsafe { &*stream };
    let Some(read) = stream_ref.read else {
        return false;
    };
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        // SAFETY: `chunk` is writable for its own length.
        let got = unsafe { read(stream, chunk.as_mut_ptr().cast(), chunk.len() as u64) };
        if got < 0 {
            return false;
        }
        if got == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..got as usize]);
    }
    if bytes.len() == 8 && this.kind != Kind::StateEcho {
        let mut eight = [0u8; 8];
        eight.copy_from_slice(&bytes);
        this.gain = f64::from_le_bytes(eight);
    }
    this.state = bytes;
    this.loaded = true;
    true
}

static LATENCY: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};

unsafe extern "C" fn latency_get(plugin: *const clap_plugin) -> u32 {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return 0;
    };
    note(this.kind, "latency.get");
    if this.kind != Kind::Latency {
        return 0;
    }
    std::env::var(LATENCY_ENV)
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(LATENCY_DEFAULT)
}

static RENDER: clap_plugin_render = clap_plugin_render {
    has_hard_realtime_requirement: Some(render_hard),
    set: Some(render_set),
};

unsafe extern "C" fn render_hard(_plugin: *const clap_plugin) -> bool {
    false
}

unsafe extern "C" fn render_set(
    plugin: *const clap_plugin,
    mode: clap_sys::ext::render::clap_plugin_render_mode,
) -> bool {
    // SAFETY: as `plugin_init`.
    let Some(this) = (unsafe { plug(plugin) }) else {
        return false;
    };
    note(
        this.kind,
        if mode == CLAP_RENDER_OFFLINE {
            "render.offline"
        } else {
            "render.realtime"
        },
    );
    true
}

/// Copy `text` into a fixed C char array, nul-terminated and never overrun.
fn fill(dst: &mut [c_char], text: &[u8]) {
    for (slot, byte) in dst.iter_mut().zip(text.iter()) {
        *slot = *byte as c_char;
    }
    let cut = text.len().min(dst.len().saturating_sub(1));
    if let Some(slot) = dst.get_mut(cut) {
        *slot = 0;
    }
}

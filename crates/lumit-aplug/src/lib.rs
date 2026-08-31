//! `lumit-aplug` — Lumit's audio plugin host (K-683; docs/12 §4a says what it
//! must do, docs/impl/audio-plugins.md says how).
//!
//! # In plain terms
//!
//! An audio plugin is somebody else's compiled code — a compressor, an EQ, a
//! reverb — shipped as a shared library that Lumit loads while it is running.
//! It takes a short run of sound and hands back a processed run. Hosting one
//! means five things, and everything difficult here is one of them refusing to
//! be simple: find it on disk, load it, describe its knobs as ordinary Lumit
//! properties, feed it the layer's sound in fixed blocks, and put what comes
//! back where the dry sound would have gone.
//!
//! This crate is the first four, for **both** standards. CLAP went first
//! because it is the honest easier one: a `.clap` file is a plain DLL exporting
//! one symbol, and everything beyond a tiny core is a named **extension** the
//! host and plugin negotiate, so a host that implements five of them is a
//! complete host rather than a partial one. The five are `audio-ports`,
//! `params`, `state`, `latency` and `render`.
//!
//! [`vst3`] is the second front end onto the same road (K-707): a `.vst3` bundle
//! whose classes are COM-style objects, split into a component that makes the
//! sound and a controller that owns the knobs. It answers with the same
//! [`PluginDescriptor`] and plays the same 512 frames, and [`abi`] is where the
//! two stop being two — everything past describe is written once.
//!
//! The road through the crate, in the order a plugin travels it:
//!
//! 1. [`discover`] reads the standard CLAP folders and finds the files.
//! 2. [`module`] opens one file and asks its factory what is inside.
//! 3. [`describe`] creates one plugin, asks what sound it wants and what knobs
//!    it has, and throws it away — turning away instruments and anything that
//!    is not stereo, each with a report line.
//! 4. [`schema`] writes that answer down as the same declaration a built-in
//!    effect carries, so a plugin's Threshold keyframes exactly as a built-in's
//!    Radius does.
//! 5. [`def`] is the catalogue entry ([`AudioEffectDef`], which VST3 fills
//!    too) and the driver ([`AudioHost`], [`LocalHost`]).
//! 6. [`instance`] and [`process`] are one live plugin and one block of sound.
//! 7. [`ipc`] moves all of that into **another process**, which is the shipping
//!    arrangement: [`LocalHost`] loads the plugin here and is for tests,
//!    [`BrokerHost`] talks to a `lumit-aplug-broker` that has it at arm's
//!    length. [`quirks`] is where a plugin's deviations live as data.
//!
//! # What is here and what is elsewhere
//!
//! The mix seam — where a layer's chain is baked, a dead block goes dry and the
//! latency shift happens — is `lumit_core::fx::audio_chain` (K-700), and it
//! knows nothing about either standard. What is here is provable on its own: the
//! order of actions, the sound, the state, the events, and a plugin dying
//! without taking anything with it.
//!
//! # Thread role and contract
//!
//! Plugin-facing, and therefore full of raw pointers: the C API hands a
//! function no context but its arguments. Every entry point checks its pointers
//! before dereferencing them, no Rust panic may cross back into C, and no lock
//! is ever held across a call into a plugin — the three host callbacks a plugin
//! can reach are atomic flags (docs/14 §7). Nothing here blocks on the GPU or
//! sleeps.

pub mod abi;
pub mod def;
pub mod describe;
pub mod discover;
pub mod instance;
pub mod ipc;
pub mod module;
pub mod process;
pub mod quirks;
pub mod schema;
pub mod vst3;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests_vst3;

pub use abi::{Abi, AnyInstance, AnyModule};
pub use def::{
    AudioEffectDef, AudioHost, BlockJob, BrokerHost, HostedAudio, InstanceSetup, LocalHost,
    BLOCK_SAMPLES, LOOKAHEAD_MARGIN,
};
pub use describe::{
    describe, describe_module, describe_module_except, ParamDescription, PluginDescriptor,
    PortInfo, Ports, Refusal, Rejection, ScanReport, STEREO,
};
pub use discover::{
    clap_search_paths, scan, scan_brokered, scan_dir, search_paths, vst3_search_paths,
    DiscoveredPlugin, ScanOptions, ScanOutcome,
};
pub use instance::{HostError, HostFlags, Instance};
pub use ipc::broker::{
    module_broker, nothing_disabled, session_disabled, set_disabled, set_enabled, Broker,
    BrokerConfig, BrokerError, DisableList, STRIKES_BEFORE_DISABLED,
};
pub use module::{Module, ModuleEntry, ModuleError};
pub use process::{
    Block, Denormals, ParamEvent, BLOCK_FRAMES, CHANNELS, INTERLEAVED_LEN, SAMPLE_RATE,
};
pub use quirks::{Quirks, QuirksTable, BLOCK_PERIOD};
pub use schema::{match_name, row_id, schema_of, value_routes, ValueRoute, MATCH_PREFIX};
pub use vst3::{Vst3Instance, Vst3Module};

/// Every call this host makes to a plugin, from the factory to the grave, in
/// the order it makes them.
///
/// Written down so the order is a thing a test can compare against rather than
/// a thing spread across three functions (docs/impl/audio-plugins.md §7 plan 2,
/// the same discipline as the OFX host's `RENDER_ACTIONS`, K-591). Getting this
/// order wrong is how a host breaks plugins that are otherwise blameless: a
/// state blob loaded while the plugin is active is undefined, and parameters
/// set *before* the state are parameters the state then overwrites.
///
/// The two halves are the two moments a plugin is created. The first is
/// **describe**, at scan time: CLAP's audio ports and parameters are extensions
/// of a live plugin, so describing one means making one and throwing it away.
/// The second is the instance a layer actually holds.
///
/// The list assumes one plugin with one input port, one output port, one
/// parameter and one block; the reporter personality of the test plugin records
/// exactly this.
pub const HOST_ACTIONS: [&str; 21] = [
    // Scan: the factory names what is in the file.
    "factory",
    // Describe: create it, ask, destroy it. The latency is deliberately *not*
    // asked for here — CLAP's `latency.get` is an active-state call, and a
    // describe never activates.
    "create",
    "init",
    "audio_ports.count",
    "audio_ports.get",
    "audio_ports.count",
    "audio_ports.get",
    "params.count",
    "params.get_info",
    "destroy",
    // The instance a layer holds.
    "create",
    "init",
    // The plugin's own memory of itself first…
    "state.load",
    // …then the project's values over the top of it: properties win.
    "params.flush",
    "activate",
    "start_processing",
    // Now that it is active, and only now, it can be asked how far behind it
    // runs.
    "latency.get",
    "process",
    "stop_processing",
    "deactivate",
    "destroy",
];

/// The same list for **VST3**, whose words are its own (K-707).
///
/// It is a separate list rather than a translation of [`HOST_ACTIONS`] because
/// the two standards genuinely differ in what the host does, not only in what it
/// calls it: a VST3 plugin is two objects that are created, initialised and
/// terminated separately, its buses are negotiated at activate rather than
/// declared at describe, and it has no `flush` — a value set outside a block
/// goes to the **controller**, and the processor learns it from the queue that
/// rides with the next block ([`vst3`]). Pretending one order covered both would
/// hide exactly the differences a host gets wrong.
///
/// The list assumes one plugin with one input bus, one output bus, one
/// parameter and one block, and a **split** plugin — a component and a
/// controller of its own — which is the harder and more common shape. Calls the
/// controller answers are prefixed with its name, because both halves have an
/// `initialize`, a `terminate` and a `setState` and a log that did not say which
/// object answered would prove nothing.
pub const VST3_HOST_ACTIONS: [&str; 33] = [
    // Scan: the factory names the classes in the bundle.
    "factory",
    // Describe: create both halves, ask, throw them away. The latency is
    // deliberately *not* asked for here — it is an active-state number, and a
    // describe never activates.
    "create",
    "initialize",
    "controller.initialize",
    "getBusCount",
    "getBusInfo",
    "getBusCount",
    "getBusInfo",
    "getParameterCount",
    "getParameterInfo",
    "controller.terminate",
    "terminate",
    // The instance a layer holds.
    "create",
    "initialize",
    "controller.initialize",
    // The plugin's own memory of itself, into **both** halves: the processor's
    // own blob, the same blob to the controller so the two agree, then the
    // controller's own.
    "setState",
    "setComponentState",
    "controller.setState",
    // …then the project's values over the top of it: properties win.
    "setParamNormalized",
    // The buses, which are an inactive-state question in VST3 rather than a
    // describe-time declaration.
    "setBusArrangements",
    "getBusCount",
    "activateBus",
    "getBusCount",
    "activateBus",
    "setupProcessing",
    "setActive",
    "setProcessing",
    // Active at last, and only now, it can be asked how far behind it runs.
    "getLatencySamples",
    "process",
    "setProcessing",
    "setActive",
    "controller.terminate",
    "terminate",
];

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
//! This crate is the first four. CLAP goes first because it is the honest
//! easier one: a `.clap` file is a plain DLL exporting one symbol, and
//! everything beyond a tiny core is a named **extension** the host and plugin
//! negotiate, so a host that implements five of them is a complete host rather
//! than a partial one. The five are `audio-ports`, `params`, `state`, `latency`
//! and `render`.
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
//! 5. [`def`] is the catalogue entry ([`AudioEffectDef`], which VST3 will fill
//!    too) and the driver ([`AudioHost`], [`LocalHost`]).
//! 6. [`instance`] and [`process`] are one live plugin and one block of sound.
//! 7. [`ipc`] moves all of that into **another process**, which is the shipping
//!    arrangement: [`LocalHost`] loads the plugin here and is for tests,
//!    [`BrokerHost`] talks to a `lumit-aplug-broker` that has it at arm's
//!    length. [`quirks`] is where a plugin's deviations live as data.
//!
//! # What this package is not, yet
//!
//! **Nothing is wired into the mix.** The chain worker, the lookahead ring, the
//! dry-block fallback and the latency shift are AP3. What is here is provable
//! on its own: the order of actions, the sound, the state, the events, and a
//! plugin dying without taking anything with it.
//!
//! # Thread role and contract
//!
//! Plugin-facing, and therefore full of raw pointers: the C API hands a
//! function no context but its arguments. Every entry point checks its pointers
//! before dereferencing them, no Rust panic may cross back into C, and no lock
//! is ever held across a call into a plugin — the three host callbacks a plugin
//! can reach are atomic flags (docs/14 §7). Nothing here blocks on the GPU or
//! sleeps.

pub mod def;
pub mod describe;
pub mod discover;
pub mod instance;
pub mod ipc;
pub mod module;
pub mod process;
pub mod quirks;
pub mod schema;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub use def::{
    AudioEffectDef, AudioHost, BlockJob, BrokerHost, InstanceSetup, LocalHost, BLOCK_SAMPLES,
    LOOKAHEAD_MARGIN,
};
pub use describe::{
    describe, describe_module, describe_module_except, ParamDescription, PluginDescriptor,
    PortInfo, Ports, Refusal, Rejection, ScanReport, STEREO,
};
pub use discover::{
    scan, scan_brokered, scan_dir, search_paths, DiscoveredPlugin, ScanOptions, ScanOutcome,
};
pub use instance::{HostError, HostFlags, Instance};
pub use ipc::broker::{
    nothing_disabled, Broker, BrokerConfig, BrokerError, DisableList, STRIKES_BEFORE_DISABLED,
};
pub use module::{Module, ModuleEntry, ModuleError};
pub use process::{
    Block, Denormals, ParamEvent, BLOCK_FRAMES, CHANNELS, INTERLEAVED_LEN, SAMPLE_RATE,
};
pub use quirks::{Quirks, QuirksTable, BLOCK_PERIOD};
pub use schema::{row_id, schema_of, value_routes, ValueRoute, MATCH_PREFIX};

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

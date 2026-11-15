//! `lumit-ofx` — Lumit's OpenFX host (docs/12 §2 says what it must do,
//! docs/impl/ofx-host.md says how).
//!
//! # In plain terms
//!
//! An OFX plugin is somebody else's compiled code — Twixtor, Sapphire, one of
//! the eighty free ones — shipped as a shared library that Lumit loads while
//! it is running. It cannot be inspected, it was written against other hosts,
//! and it will be handed pictures and asked for pictures back. This crate is
//! the side of that conversation Lumit speaks.
//!
//! The ground floor is opening a bundle, telling the plugin honestly what sort
//! of host it has landed in, and answering the three simplest of its
//! questions — read and write properties, allocate memory, say something to the
//! user. On top of that sits **describe** ([`describe`]): the conversation
//! where a plugin says what it is called, what shapes of effect it can be, and
//! what controls it has, and where Lumit writes that answer down as the same
//! declaration a built-in effect carries ([`schema`]).
//!
//! On top of *that* sits the frame. An [`instance`] is one live copy of the
//! effect, with a value in every control; an [`image`] is the picture crossing
//! the boundary, widened to float and possibly upside-down on purpose; and
//! [`render`] is the fixed order of actions that turns the two into a frame.
//!
//! **Honesty.** The host describes itself in a table of properties, and every
//! answer in it is one the rest of Lumit keeps. Claiming to support tiled
//! rendering when the pipeline is full-frame is the classic way to break a
//! plugin that believed you.
//!
//! **Handles.** Everything the plugin refers to, it refers to by an opaque
//! number the host invents. Plugins forge them, keep them too long, and pass
//! the wrong sort. None of ours is a real pointer; each is checked three ways
//! on every use, and the answer to a bad one is an error code the plugin is
//! required to expect, not a crash. See [`handles`].
//!
//! # Thread role and contract
//!
//! Plugin-facing, and therefore not free of shared state: the C API gives a
//! suite function no context but its arguments, so the host's property sets
//! live behind one process-wide mutex ([`host::state`]). It is taken and
//! released inside a single suite call and **never held across a call into a
//! plugin**, which would deadlock the moment that plugin read a property
//! (docs/14 §7). Nothing here blocks, sleeps, or touches the GPU.
//!
//! No Rust panic may cross back into C, so every entry point a plugin can
//! reach runs its body inside a catch (see [`suites`]).

#[cfg(not(target_arch = "wasm32"))]
pub mod bundle;
#[cfg(not(target_arch = "wasm32"))]
pub mod def;
#[cfg(not(target_arch = "wasm32"))]
pub mod describe;
#[cfg(not(target_arch = "wasm32"))]
pub mod discover;
#[cfg(not(target_arch = "wasm32"))]
pub mod ffi;
#[cfg(not(target_arch = "wasm32"))]
pub mod gpu;
#[cfg(not(target_arch = "wasm32"))]
pub mod handles;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
#[cfg(not(target_arch = "wasm32"))]
pub mod image;
#[cfg(not(target_arch = "wasm32"))]
pub mod instance;
#[cfg(not(target_arch = "wasm32"))]
pub mod ipc;
#[cfg(not(target_arch = "wasm32"))]
pub mod props;
#[cfg(not(target_arch = "wasm32"))]
pub mod quirks;
#[cfg(not(target_arch = "wasm32"))]
pub mod render;
#[cfg(not(target_arch = "wasm32"))]
pub mod schema;
#[cfg(not(target_arch = "wasm32"))]
pub mod status;
#[cfg(not(target_arch = "wasm32"))]
pub mod suites;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

// The browser build has no OFX plugins. These are the few calls the bridge
// makes, answering as a machine with none installed would.

#[cfg(target_arch = "wasm32")]
pub mod discover {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use lumit_core::fx::registry::EffectDef;

    pub const DISABLED_REASON: &str = "plugin_disabled";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Hosting {
        Broker,
        InProcess,
    }

    pub struct ScanOptions {
        pub paths: Vec<PathBuf>,
        pub disabled: BTreeSet<String>,
        pub hosting: Hosting,
    }

    impl ScanOptions {
        #[must_use]
        pub fn standard() -> Self {
            Self {
                paths: Vec::new(),
                disabled: BTreeSet::new(),
                hosting: Hosting::Broker,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct DiscoveredPlugin {
        pub match_name: String,
        pub identifier: String,
        pub label: String,
        pub grouping: String,
        pub bundle: PathBuf,
    }

    #[derive(Debug, Default)]
    pub struct ScanOutcome {
        pub registered: Vec<DiscoveredPlugin>,
        pub skipped: Vec<String>,
    }

    pub fn stop_brokers(_folder: &Path) {}

    pub fn follow_gpu_setting() {}

    #[must_use]
    pub fn plugin_of(_match_name: &str) -> Option<DiscoveredPlugin> {
        None
    }

    pub fn set_enabled(_identifier: &str, _enabled: bool) {}

    pub fn set_disabled(_list: &BTreeSet<String>) {}

    pub fn remember_empty_bundles_in(_file: PathBuf) {}

    pub fn scan(
        _options: &ScanOptions,
        _register: &mut dyn FnMut(&'static dyn EffectDef) -> bool,
    ) -> ScanOutcome {
        ScanOutcome::default()
    }

    pub fn scan_reporting(
        _options: &ScanOptions,
        _register: &mut dyn FnMut(&'static dyn EffectDef) -> bool,
        _bundle: &mut dyn FnMut(&Path, usize, usize),
    ) -> ScanOutcome {
        ScanOutcome::default()
    }
}

#[cfg(target_arch = "wasm32")]
pub mod gpu {
    pub fn set_enabled(_on: bool) {}

    pub fn set_adapter(_summary: Option<&str>) {}
}

#[cfg(target_arch = "wasm32")]
pub mod host {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HostMessage {
        pub message_type: String,
        pub message_id: String,
        pub text: String,
    }

    pub struct HostState;

    impl HostState {
        pub fn take_messages(&mut self) -> Vec<HostMessage> {
            Vec::new()
        }
    }

    #[must_use]
    pub fn state() -> HostState {
        HostState
    }
}

#[cfg(target_arch = "wasm32")]
pub mod ipc {
    pub mod broker {
        pub const BROKER_EXE_ENV: &str = "LUMIT_OFX_BROKER";

        #[must_use]
        pub fn broker_exe_name() -> &'static str {
            "lumit-ofx-broker"
        }
    }
}

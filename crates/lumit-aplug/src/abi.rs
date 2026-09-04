//! Which standard a file speaks, and the one module and the one plugin the rest
//! of the crate holds.
//!
//! # In plain terms
//!
//! There are two plugin standards, and Lumit hosts both. Everything *after*
//! describe is meant not to know which one it is dealing with — the effect
//! declaration, the broker, the ring, the deadline, the switched-off list and
//! the mix seam are all written once. This file is where the not-knowing starts:
//! two files on disk, two ways of opening them, and from here on one
//! [`AnyModule`] and one [`AnyInstance`] with the same fourteen questions asked
//! of both.
//!
//! It is an enum rather than a trait on purpose. There are exactly two
//! standards, both are known at compile time, and a trait object would buy an
//! open set nobody wants — VST2 is dead for us and recorded as such.
//! What the enum costs is a `match` per question, which is the shortest honest
//! spelling of "these two things answer the same fourteen questions".

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::describe::{ParamDescription, Ports};
use crate::instance::{HostError, Instance};
use crate::module::{Module, ModuleEntry, ModuleError};
use crate::process::{Block, ParamEvent};
use crate::vst3::{Vst3Instance, Vst3Module, BUNDLE_EXTENSION};

/// Which standard a plugin speaks.
///
/// Carried on the descriptor and across the pipe, because exactly one thing
/// downstream of describe needs it: the prefix its effect's match name is
/// spelled with, which is how a saved project names the plugin it wants back.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Abi {
    /// CLAP (clap.audio), the MIT-licensed one AP1 built against.
    #[default]
    Clap,
    /// VST3 (Steinberg), hosted under the SDK's GPLv3 branch.
    Vst3,
}

impl Abi {
    /// What a plugin of this standard's `match_name` begins with.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Clap => lumit_core::fx::CLAP_MATCH_PREFIX,
            Self::Vst3 => lumit_core::fx::VST3_MATCH_PREFIX,
        }
    }

    /// Which standard a file on disk speaks, from its name. `None` for
    /// anything that is neither.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("clap") => Some(Self::Clap),
            Some(BUNDLE_EXTENSION) => Some(Self::Vst3),
            _ => None,
        }
    }
}

/// One opened plugin file, whichever standard it speaks.
#[derive(Clone)]
pub enum AnyModule {
    /// A `.clap` file.
    Clap(Arc<Module>),
    /// A `.vst3` bundle.
    Vst3(Arc<Vst3Module>),
}

impl AnyModule {
    /// Open a plugin file and read its list, choosing the standard by the file's
    /// own name.
    ///
    /// # Errors
    ///
    /// [`ModuleError`] — every way a third party's file can disappoint us, plus
    /// [`ModuleError::BadPath`] for a name that is neither standard's.
    pub fn open(path: &Path) -> Result<Self, ModuleError> {
        match Abi::of(path) {
            Some(Abi::Clap) => Module::open(path).map(|module| Self::Clap(Arc::new(module))),
            Some(Abi::Vst3) => Vst3Module::open(path).map(|module| Self::Vst3(Arc::new(module))),
            None => Err(ModuleError::BadPath),
        }
    }

    /// Which standard this one speaks.
    #[must_use]
    pub const fn abi(&self) -> Abi {
        match self {
            Self::Clap(_) => Abi::Clap,
            Self::Vst3(_) => Abi::Vst3,
        }
    }

    /// The plugins this file declares, in its own order.
    #[must_use]
    pub fn entries(&self) -> &[ModuleEntry] {
        match self {
            Self::Clap(module) => module.entries(),
            Self::Vst3(module) => module.entries(),
        }
    }

    /// The file this module came out of.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Clap(module) => module.path(),
            Self::Vst3(module) => module.path(),
        }
    }

    /// The file, owned — what a definition keeps so it can open a broker later.
    #[must_use]
    pub fn to_path(&self) -> PathBuf {
        self.path().to_path_buf()
    }

    /// Make one plugin, by its own id.
    ///
    /// # Errors
    ///
    /// [`HostError`] — the file does not declare that id, or the plugin refused
    /// to be created or initialised.
    pub fn create(&self, plugin_id: &str) -> Result<AnyInstance, HostError> {
        match self {
            Self::Clap(module) => {
                Instance::create(Arc::clone(module), plugin_id).map(AnyInstance::Clap)
            }
            Self::Vst3(module) => {
                Vst3Instance::create(Arc::clone(module), plugin_id).map(AnyInstance::Vst3)
            }
        }
    }
}

/// One live plugin, whichever standard it speaks.
///
/// The fourteen questions below are every question the host asks a plugin, and
/// they are asked in the order [`crate::HOST_ACTIONS`] and
/// [`crate::VST3_HOST_ACTIONS`] pin.
pub enum AnyInstance {
    /// A CLAP plugin.
    Clap(Instance),
    /// A VST3 plugin.
    Vst3(Vst3Instance),
}

impl AnyInstance {
    /// The plugin's audio ports, both directions.
    #[must_use]
    pub fn ports(&self) -> Ports {
        match self {
            Self::Clap(instance) => instance.ports(),
            Self::Vst3(instance) => instance.ports(),
        }
    }

    /// Every parameter the plugin declares, in its own order.
    #[must_use]
    pub fn params(&self) -> Vec<ParamDescription> {
        match self {
            Self::Clap(instance) => instance.params(),
            Self::Vst3(instance) => instance.params(),
        }
    }

    /// Whether the plugin can report latency at all — the question a describe
    /// may ask, because it needs no active plugin.
    #[must_use]
    pub fn reports_latency(&self) -> bool {
        match self {
            Self::Clap(instance) => instance.reports_latency(),
            Self::Vst3(instance) => instance.reports_latency(),
        }
    }

    /// The latency the plugin reports, in samples. Asked only while active.
    #[must_use]
    pub fn latency(&self) -> u32 {
        match self {
            Self::Clap(instance) => instance.latency(),
            Self::Vst3(instance) => instance.latency(),
        }
    }

    /// Whether the plugin has asked to be brought up again — how both standards
    /// say "my latency changed".
    #[must_use]
    pub fn wants_restart(&self) -> bool {
        match self {
            Self::Clap(instance) => instance
                .host_flags()
                .restart
                .load(std::sync::atomic::Ordering::Relaxed),
            Self::Vst3(instance) => instance.wants_restart(),
        }
    }

    /// Tell the plugin whether this is an export or a preview.
    pub fn set_offline(&mut self, offline: bool) -> bool {
        match self {
            Self::Clap(instance) => instance.set_offline(offline),
            Self::Vst3(instance) => instance.set_offline(offline),
        }
    }

    /// Hand the plugin the blob the project saved, while it is deactivated.
    ///
    /// # Errors
    ///
    /// Whatever the plugin said about it; a refusal is a warning on the way up,
    /// never a lost effect.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        match self {
            Self::Clap(instance) => instance.load_state(bytes),
            Self::Vst3(instance) => instance.load_state(bytes),
        }
    }

    /// The blob to write into the `.lum`. Never parsed, always round-tripped.
    ///
    /// # Errors
    ///
    /// Whatever the plugin said about it.
    pub fn save_state(&self) -> Result<Vec<u8>, HostError> {
        match self {
            Self::Clap(instance) => instance.save_state(),
            Self::Vst3(instance) => instance.save_state(),
        }
    }

    /// Set parameters outside a block — the "properties win" step, run after the
    /// state and before the plugin is activated.
    ///
    /// # Errors
    ///
    /// Whatever the plugin said about it.
    pub fn flush_params(&mut self, events: &[ParamEvent]) -> Result<(), HostError> {
        match self {
            Self::Clap(instance) => instance.flush_params(events),
            Self::Vst3(instance) => instance.flush_params(events),
        }
    }

    /// Prepare the plugin for 512-frame blocks at 48 kHz, in stereo.
    ///
    /// # Errors
    ///
    /// The plugin refused the rate, the block size or the stereo pair.
    pub fn activate(&mut self) -> Result<(), HostError> {
        match self {
            Self::Clap(instance) => instance.activate(),
            Self::Vst3(instance) => instance.activate(),
        }
    }

    /// Enter the processing state.
    ///
    /// # Errors
    ///
    /// The plugin refused, including when it was never activated.
    pub fn start_processing(&mut self) -> Result<(), HostError> {
        match self {
            Self::Clap(instance) => instance.start_processing(),
            Self::Vst3(instance) => instance.start_processing(),
        }
    }

    /// One block: the input planes in, the output planes out.
    ///
    /// `events` need not be sorted — both boundaries sort, because CLAP calls an
    /// unsorted list undefined and real plugins crash on one. `steady` is the
    /// running frame count since the chain started, which CLAP carries in the
    /// call; VST3 carries time on a transport this host does not supply in v1.
    ///
    /// # Errors
    ///
    /// The plugin answered its block with an error, or was never started.
    pub fn process(
        &mut self,
        block: &mut Block,
        events: &[ParamEvent],
        steady: i64,
    ) -> Result<(), HostError> {
        match self {
            Self::Clap(instance) => {
                block.set_events(events);
                instance.process(block, steady)
            }
            Self::Vst3(instance) => instance.process(block, events),
        }
    }

    /// Leave the processing state.
    pub fn stop_processing(&mut self) {
        match self {
            Self::Clap(instance) => instance.stop_processing(),
            Self::Vst3(instance) => instance.stop_processing(),
        }
    }

    /// Undo [`AnyInstance::activate`], stopping first.
    pub fn deactivate(&mut self) {
        match self {
            Self::Clap(instance) => instance.deactivate(),
            Self::Vst3(instance) => instance.deactivate(),
        }
    }
}

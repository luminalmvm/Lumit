//! Describe: asking a plugin what it is, and writing the answer down.
//!
//! # In plain terms
//!
//! A freshly opened module has told us a name and a vendor and nothing else.
//! **Describe** is the conversation where one plugin says what sound it wants,
//! what knobs it has, and how far behind its output runs. In CLAP that
//! conversation needs a live plugin — the audio ports and the parameters are
//! *extensions*, and only a created plugin has any — so describe creates one,
//! asks, and throws it away. It happens once per plugin, at scan time, before
//! any copy of the effect exists on any layer.
//!
//! # Who is turned away, and why it is a report line
//!
//! v1 hosts **stereo effect plugins** (docs/impl/audio-plugins.md §4). Three
//! kinds of plugin are refused here, each with a reason:
//!
//! * an **instrument** — no audio input at all, so there is nothing for a
//!   layer's sound to go into;
//! * a plugin whose main ports are not two channels, because Lumit's mix is
//!   stereo and guessing an up-mix on somebody else's behalf is a guess;
//! * a plugin with no `audio-ports` extension, which cannot say what it wants.
//!
//! None of the three is an error the person who opened Lumit did anything
//! about. Each is one calm line in a report, exactly as the OFX scan's are
//! (docs/12 §2.6), and the scan carries on to the next plugin in the file.

use std::collections::BTreeSet;
use std::sync::Arc;

use clap_sys::ext::params::{
    CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::instance::{HostError, Instance};
use crate::module::Module;

/// Channels a v1 main port must have.
pub const STEREO: u32 = 2;

/// One audio port a plugin declared.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PortInfo {
    /// The plugin's own id for the port.
    pub id: u32,
    /// The name it gave the port.
    pub name: String,
    /// Whether this is the **main** port — the one a layer's sound goes
    /// through. Anything else is an aux or sidechain, left inactive in v1.
    pub main: bool,
    /// How many channels it carries.
    pub channels: u32,
}

/// A plugin's audio ports, both directions.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Ports {
    /// Inputs, in the plugin's own order.
    pub inputs: Vec<PortInfo>,
    /// Outputs, in the plugin's own order.
    pub outputs: Vec<PortInfo>,
}

impl Ports {
    /// The main input, or the first input where none says it is main.
    #[must_use]
    pub fn main_input(&self) -> Option<&PortInfo> {
        main_of(&self.inputs)
    }

    /// The main output, likewise.
    #[must_use]
    pub fn main_output(&self) -> Option<&PortInfo> {
        main_of(&self.outputs)
    }
}

/// The port flagged main, or the first one. A plugin with exactly one port
/// often does not bother to flag it, and refusing that plugin would be
/// pedantry rather than safety.
fn main_of(ports: &[PortInfo]) -> Option<&PortInfo> {
    ports
        .iter()
        .find(|port| port.main)
        .or_else(|| ports.first())
}

/// One parameter a plugin declared.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParamDescription {
    /// The plugin's own **stable** id. The persistent key, never the index:
    /// plugins reorder and insert parameters across versions (§4).
    pub id: u32,
    /// The name a person sees.
    pub name: String,
    /// The plugin's own grouping path, `/` separated. Empty for a top-level
    /// parameter.
    pub module: String,
    /// The lowest legal value.
    pub min: f64,
    /// The highest legal value.
    pub max: f64,
    /// What the plugin starts it at.
    pub default: f64,
    /// CLAP's own flag word.
    pub flags: u32,
}

impl ParamDescription {
    /// Whether the plugin hides this parameter from the host's own surfaces.
    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.flags & CLAP_PARAM_IS_HIDDEN != 0
    }

    /// Whether an automation event may set it.
    #[must_use]
    pub const fn automatable(&self) -> bool {
        self.flags & CLAP_PARAM_IS_AUTOMATABLE != 0
    }

    /// Whether it takes whole numbers only.
    #[must_use]
    pub const fn stepped(&self) -> bool {
        self.flags & CLAP_PARAM_IS_STEPPED != 0
    }

    /// Whether the plugin refuses to let anything set it.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.flags & CLAP_PARAM_IS_READONLY != 0
    }

    /// Whether it is the plugin's own bypass switch. Lumit has its own — the
    /// effect's enable switch — so the plugin's gets no row of its own.
    #[must_use]
    pub const fn bypass(&self) -> bool {
        self.flags & CLAP_PARAM_IS_BYPASS != 0
    }

    /// Whether this parameter becomes a **row** in Effect controls.
    ///
    /// Hidden and non-automatable parameters get none (§4): they are not
    /// controls anybody could keyframe, and they live in the state blob, which
    /// is round-tripped whole. A read-only parameter is a readout, not a
    /// control, and Lumit has no readout row; a bypass is Lumit's own switch.
    #[must_use]
    pub const fn row_worthy(&self) -> bool {
        self.automatable() && !self.hidden() && !self.read_only() && !self.bypass()
    }
}

/// Everything one plugin said about itself.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginDescriptor {
    /// The plugin's own stable identifier.
    pub id: String,
    /// The name a person sees.
    pub label: String,
    /// Who wrote it.
    pub vendor: String,
    /// Its own version string, verbatim.
    pub version: String,
    /// The feature words it declares.
    pub features: Vec<String>,
    /// Its audio ports.
    pub ports: Ports,
    /// Its parameters, in its own order.
    pub params: Vec<ParamDescription>,
    /// Whether it implements the `latency` extension at all.
    ///
    /// **Not the number.** CLAP's `latency.get` is an active-state call, and a
    /// describe never activates — asking an inactive plugin is out of contract
    /// and the answer would be stale anyway, because latency changes with the
    /// parameters (§4). The chain reads the number off the live instance, which
    /// is where it is needed.
    pub reports_latency: bool,
}

impl PluginDescriptor {
    /// The parameters that become rows, in declaration order.
    pub fn rows(&self) -> impl Iterator<Item = &ParamDescription> {
        self.params.iter().filter(|param| param.row_worthy())
    }

    /// The value every row starts at — the plugin's own defaults, which is
    /// what a fresh instance of the effect holds.
    #[must_use]
    pub fn defaults(&self) -> Vec<(u32, f64)> {
        self.rows().map(|param| (param.id, param.default)).collect()
    }
}

/// Why a plugin was not offered as an effect.
#[derive(Debug, Error)]
pub enum Rejection {
    /// It could not be created or initialised.
    #[error("it could not be created ({0})")]
    NotCreated(#[from] HostError),
    /// It has no audio input: an instrument, not an effect (§4).
    #[error("it is an instrument — it has no audio input")]
    Instrument,
    /// It declares no audio ports at all, so it cannot say what it wants.
    #[error("it declares no audio ports")]
    NoAudioPorts,
    /// Its main ports are not stereo.
    #[error("its main ports are {inputs} in and {outputs} out, and this host is stereo")]
    NotStereo {
        /// Channels on the main input.
        inputs: u32,
        /// Channels on the main output.
        outputs: u32,
    },
    /// Two rows would land on the same schema id — the silent collision
    /// docs/impl/effect-registry.md §5 warns about, made loud because a
    /// plugin's parameter ids are not ours to choose.
    #[error("two parameters share the row id {first:?} (the second is {second:?})")]
    DuplicateParamId {
        /// The row that got there first.
        first: String,
        /// The one that collided with it.
        second: String,
    },
}

/// One plugin turned away, and why. A line in a report, never a dialogue.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Refusal {
    /// The plugin's own id.
    pub id: String,
    /// The reason, as a sentence.
    pub reason: String,
}

/// What describing one module found.
#[derive(Debug, Default)]
pub struct ScanReport {
    /// The plugins that can be hosted.
    pub described: Vec<PluginDescriptor>,
    /// The ones that cannot, each with its reason.
    pub rejected: Vec<Refusal>,
}

/// Describe every plugin in one module.
///
/// Blocking, and not to be called from the interface's thread: it runs
/// somebody else's start-up code once per plugin in the file.
#[must_use]
pub fn describe_module(module: &Arc<Module>) -> ScanReport {
    describe_module_except(module, &BTreeSet::new())
}

/// Describe every plugin in one module **except** the ones the user has
/// switched off.
///
/// The list is consulted *before* describe rather than after, so a switched-off
/// plugin is never created and its code never runs at all (K-594). That is the
/// whole difference between a disable that means something and a filter on a
/// list.
#[must_use]
pub fn describe_module_except(module: &Arc<Module>, disabled: &BTreeSet<String>) -> ScanReport {
    let mut report = ScanReport::default();
    for entry in module.entries() {
        if disabled.contains(&entry.id) {
            continue;
        }
        match describe(module, &entry.id) {
            Ok(descriptor) => report.described.push(descriptor),
            Err(reason) => report.rejected.push(Refusal {
                id: entry.id.clone(),
                reason: reason.to_string(),
            }),
        }
    }
    report
}

/// Describe one plugin: create it, ask, throw it away.
///
/// # Errors
///
/// A [`Rejection`], which is a report line rather than a failure.
pub fn describe(module: &Arc<Module>, plugin_id: &str) -> Result<PluginDescriptor, Rejection> {
    let entry = module
        .entries()
        .iter()
        .find(|entry| entry.id == plugin_id)
        .ok_or_else(|| Rejection::NotCreated(HostError::NoSuchPlugin(plugin_id.to_owned())))?
        .clone();

    let instance = Instance::create(Arc::clone(module), plugin_id)?;
    let ports = instance.ports();
    if ports.inputs.is_empty() && ports.outputs.is_empty() {
        return Err(Rejection::NoAudioPorts);
    }
    let Some(input) = ports.main_input() else {
        return Err(Rejection::Instrument);
    };
    let outputs = ports.main_output().map_or(0, |port| port.channels);
    if input.channels != STEREO || outputs != STEREO {
        return Err(Rejection::NotStereo {
            inputs: input.channels,
            outputs,
        });
    }

    Ok(PluginDescriptor {
        id: entry.id,
        label: entry.name,
        vendor: entry.vendor,
        version: entry.version,
        features: entry.features,
        params: instance.params(),
        reports_latency: instance.reports_latency(),
        ports,
    })
}

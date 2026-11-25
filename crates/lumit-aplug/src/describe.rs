//! Describe: asking a plugin what it is, and writing the answer down.
//!
//! # In plain terms
//!
//! A freshly opened module has told us a name and a vendor and nothing else.
//! **Describe** is the conversation where one plugin says what sound it wants,
//! what knobs it has, and how far behind its output runs. Both standards need a
//! live plugin for it — CLAP's audio ports and parameters are *extensions* only
//! a created plugin has, and VST3's buses and parameters live on an initialised
//! component and its controller — so describe creates one, asks, and throws it
//! away. It happens once per plugin, at scan time, before any copy of the effect
//! exists on any layer.
//!
//! **One answer for both**: everything below is written once and filled
//! by either front end, so nothing downstream of this file knows which standard
//! a plugin speaks except by the [`Abi`] on the descriptor, which exists for one
//! reason — the prefix its match name is spelled with.
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
//! * a plugin that declares no audio buses at all, which cannot say what it
//!   wants — a CLAP plugin with no `audio-ports` extension, or a VST3 component
//!   with no buses.
//!
//! None of the three is an error the person who opened Lumit did anything
//! about. Each is one calm line in a report, exactly as the OFX scan's are
//! (docs/12 §2.6), and the scan carries on to the next plugin in the file.

use std::collections::BTreeSet;

use clap_sys::ext::params::{
    CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::abi::{Abi, AnyModule};
use crate::instance::HostError;

pub const STEREO: u32 = 2;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PortInfo {
    pub id: u32,
    pub name: String,
    /// Whether this is the **main** port — the one a layer's sound goes
    /// through. Anything else is an aux or sidechain, left inactive in v1.
    pub main: bool,
    pub channels: u32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Ports {
    pub inputs: Vec<PortInfo>,
    pub outputs: Vec<PortInfo>,
}

impl Ports {
    /// The main input, or the first input where none says it is main.
    #[must_use]
    pub fn main_input(&self) -> Option<&PortInfo> {
        main_of(&self.inputs)
    }

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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParamDescription {
    /// The plugin's own **stable** id. The persistent key, never the index:
    /// plugins reorder and insert parameters across versions (§4).
    pub id: u32,
    pub name: String,
    /// The plugin's own grouping path, `/` separated. Empty for a top-level
    /// parameter.
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub flags: u32,
}

impl ParamDescription {
    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.flags & CLAP_PARAM_IS_HIDDEN != 0
    }

    #[must_use]
    pub const fn automatable(&self) -> bool {
        self.flags & CLAP_PARAM_IS_AUTOMATABLE != 0
    }

    #[must_use]
    pub const fn stepped(&self) -> bool {
        self.flags & CLAP_PARAM_IS_STEPPED != 0
    }

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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginDescriptor {
    /// Which standard it speaks. The one fact about a plugin's
    /// provenance that survives describe, because the effect's match name is
    /// spelled from it and a saved project has to name the plugin it wants back.
    #[serde(default)]
    pub abi: Abi,
    /// The plugin's own stable identifier. A CLAP plugin's own id string; a
    /// VST3 class id, as the 32 hex digits that name it.
    pub id: String,
    pub label: String,
    pub vendor: String,
    pub version: String,
    pub features: Vec<String>,
    pub ports: Ports,
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
    pub fn rows(&self) -> impl Iterator<Item = &ParamDescription> {
        self.params.iter().filter(|param| param.row_worthy())
    }

    #[must_use]
    pub fn defaults(&self) -> Vec<(u32, f64)> {
        self.rows().map(|param| (param.id, param.default)).collect()
    }
}

#[derive(Debug, Error)]
pub enum Rejection {
    #[error("it could not be created ({0})")]
    NotCreated(#[from] HostError),
    #[error("it is an instrument — it has no audio input")]
    Instrument,
    #[error("it declares no audio ports")]
    NoAudioPorts,
    #[error("its main ports are {inputs} in and {outputs} out, and this host is stereo")]
    NotStereo { inputs: u32, outputs: u32 },
    /// Two rows would land on the same schema id — the silent collision
    /// docs/impl/effect-registry.md §5 warns about, made loud because a
    /// plugin's parameter ids are not ours to choose.
    #[error("two parameters share the row id {first:?} (the second is {second:?})")]
    DuplicateParamId { first: String, second: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Refusal {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub described: Vec<PluginDescriptor>,
    pub rejected: Vec<Refusal>,
}

/// Describe every plugin in one module.
///
/// Blocking, and not to be called from the interface's thread: it runs
/// somebody else's start-up code once per plugin in the file.
#[must_use]
pub fn describe_module(module: &AnyModule) -> ScanReport {
    describe_module_except(module, &BTreeSet::new())
}

/// Describe every plugin in one module **except** the ones the user has
/// switched off.
///
/// The list is consulted *before* describe rather than after, so a switched-off
/// plugin is never created and its code never runs at all. That is the
/// whole difference between a disable that means something and a filter on a
/// list.
#[must_use]
pub fn describe_module_except(module: &AnyModule, disabled: &BTreeSet<String>) -> ScanReport {
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

pub fn describe(module: &AnyModule, plugin_id: &str) -> Result<PluginDescriptor, Rejection> {
    let entry = module
        .entries()
        .iter()
        .find(|entry| entry.id == plugin_id)
        .ok_or_else(|| Rejection::NotCreated(HostError::NoSuchPlugin(plugin_id.to_owned())))?
        .clone();

    let instance = module.create(plugin_id)?;
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
        abi: module.abi(),
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

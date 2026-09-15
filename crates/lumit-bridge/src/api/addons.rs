//! The Addons page's whole surface across the seam: what is installed, what
//! the runtime is doing, and the three gestures that change either
//! (docs/impl/addons.md §5).
//!
//! # In plain terms
//!
//! The download belongs to Dart, which already streams a file to disk, shows a
//! bar and checks a digest. Everything after that belongs here: the manifest is
//! judged, the files are unpacked into a folder of their own and renamed into
//! place, and the folder is the only record of what is installed.
//!
//! **Nothing here does any work of its own.** Every function turns one
//! `lumit_ml` answer into a flat struct or a typed refusal, so the page reads
//! fields rather than unwrapping a shape, and a failure crosses as a
//! `BridgeError` variant the panel has a sentence for.

use flutter_rust_bridge::frb;
use lumit_ml::{store, MlError};

use crate::api::BridgeError;

/// Which of the two things an addon is.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAddonKind {
    /// ONNX Runtime and its provider libraries. Exactly one may be installed.
    Runtime,
    /// One analysis model, with its licence and its tensor contract.
    Model,
}

/// One installed addon, as the page's row draws it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAddon {
    /// Its folder name, and what [`addon_remove`] takes.
    pub id: String,
    pub kind: BridgeAddonKind,
    /// What the row is titled.
    pub name: String,
    /// The addon's own version, beside the name.
    pub version: String,
    /// One line saying what it does.
    pub summary: String,
    /// An SPDX expression or a short name; shown with [`BridgeAddon::licence_url`].
    pub licence: String,
    pub licence_url: String,
    /// What the download was, in bytes, for the size on the description line.
    pub size_bytes: u64,
    /// `synthesis`, `depth`, `matte` or `segmentation`, and empty for the
    /// runtime. A word the engine sends, so it has an `engine_labels` entry.
    pub task: String,
    /// A file the manifest lists is missing or the wrong size. The row stays,
    /// marked, because the fix is to install it again.
    pub broken: bool,
}

/// What the runtime row says.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRuntimeState {
    /// Nothing is installed. Every model row's button reads "Needs the
    /// runtime" until this changes.
    Missing,
    /// The folder is there and nothing has asked for it yet.
    Present,
    /// The library has loaded since Lumit started.
    Loaded,
    /// The library was asked and refused; see [`BridgeRuntimeStatus::detail`].
    Failed,
}

/// The runtime row's whole reading, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRuntimeStatus {
    pub state: BridgeRuntimeState,
    /// Which execution provider a session tries first: `DirectML`, `CoreML`
    /// or `CPU`. Empty until the library has loaded.
    pub provider: String,
    /// What the installed runtime addon pins itself at. Empty until then.
    pub version: String,
    /// The library's own sentence when it would not load, untranslated, and
    /// marked as its own words where it is shown. Empty otherwise.
    pub detail: String,
}

/// Where addons live, so Dart and Rust cannot disagree about the folder.
/// `None` only when the platform has no home directory.
#[frb(sync)]
#[must_use]
pub fn addons_dir() -> Option<String> {
    store::dir().map(|dir| dir.to_string_lossy().into_owned())
}

/// Every installed addon, by id, with the staging leftovers of a build that
/// died halfway swept on the way past.
#[frb(sync)]
#[must_use]
pub fn addon_list() -> Vec<BridgeAddon> {
    store::scan().into_iter().map(flatten).collect()
}

/// Install an addon from its manifest and the files Dart fetched and verified,
/// one file per download of this platform's block, in the manifest's order.
///
/// Deliberately not `#[frb(sync)]`: unpacking the largest pack is a couple of
/// seconds, so it rides the frb worker pool the way `rescan_plugins` does
/// rather than holding the interface thread.
///
/// # Errors
///
/// [`BridgeError::AddonBusy`] while another install is running, and
/// [`BridgeError::AddonInvalid`] with the engine's own sentence for a manifest
/// this build will not take, a file that is not the size the manifest says, or
/// a download the unpack could not finish.
pub fn addon_install(manifest: String, files: Vec<String>) -> Result<(), BridgeError> {
    let paths: Vec<std::path::PathBuf> = files.into_iter().map(Into::into).collect();
    store::install(&manifest, &paths)
        .map(|_| ())
        .map_err(refusal)
}

/// Delete an addon's folder. The row goes; nothing else is touched.
///
/// # Errors
///
/// [`BridgeError::AddonMissing`] when nothing of that id is installed, and
/// [`BridgeError::AddonInvalid`] when the folder will not delete.
#[frb(sync)]
pub fn addon_remove(id: String) -> Result<(), BridgeError> {
    store::remove(&id).map_err(refusal)
}

/// What the runtime row reads right now, without asking the library for
/// anything.
#[frb(sync)]
#[must_use]
pub fn addon_runtime() -> BridgeRuntimeStatus {
    flatten_runtime(&lumit_ml::runtime::status())
}

/// Load the runtime library now and say what happened.
///
/// Not `#[frb(sync)]`: opening a shared library and initialising a provider is
/// disk and driver work, and it happens once per run.
#[must_use]
pub fn addon_runtime_load() -> BridgeRuntimeStatus {
    let _ = lumit_ml::runtime::load_installed();
    flatten_runtime(&lumit_ml::runtime::status())
}

/// One installed addon, flattened.
fn flatten(installed: store::Installed) -> BridgeAddon {
    let manifest = installed.manifest;
    BridgeAddon {
        id: manifest.id,
        kind: match manifest.kind {
            lumit_ml::Kind::Runtime => BridgeAddonKind::Runtime,
            lumit_ml::Kind::Model => BridgeAddonKind::Model,
        },
        name: manifest.name,
        version: manifest.version,
        summary: manifest.summary,
        licence: manifest.licence,
        licence_url: manifest.licence_url,
        size_bytes: manifest.size,
        task: manifest
            .model
            .map(|model| model.task.to_string())
            .unwrap_or_default(),
        broken: installed.broken,
    }
}

/// The runtime's own status, flattened.
fn flatten_runtime(status: &lumit_ml::RuntimeStatus) -> BridgeRuntimeStatus {
    let (state, provider, version, detail) = match status {
        lumit_ml::RuntimeStatus::Missing => (BridgeRuntimeState::Missing, "", "", ""),
        lumit_ml::RuntimeStatus::Present => (BridgeRuntimeState::Present, "", "", ""),
        lumit_ml::RuntimeStatus::Loaded { provider, version } => (
            BridgeRuntimeState::Loaded,
            provider.as_str(),
            version.as_str(),
            "",
        ),
        lumit_ml::RuntimeStatus::Failed { detail } => {
            (BridgeRuntimeState::Failed, "", "", detail.as_str())
        }
    };
    BridgeRuntimeStatus {
        state,
        provider: provider.to_owned(),
        version: version.to_owned(),
        detail: detail.to_owned(),
    }
}

/// Every way the store can refuse, as the one the panel has a sentence for.
fn refusal(why: MlError) -> BridgeError {
    match why {
        MlError::Busy => BridgeError::AddonBusy,
        MlError::NotInstalled
        | MlError::RuntimeMissing
        | MlError::PackMissing(_)
        | MlError::PackUnreadable => BridgeError::AddonMissing,
        other => BridgeError::AddonInvalid(other.to_string()),
    }
}

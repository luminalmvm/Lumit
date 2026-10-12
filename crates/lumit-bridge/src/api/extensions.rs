//! Extensions across the seam: what is installed, and the gestures that
//! install and remove one. `lumit-extensions` judges and copies. The
//! frontend shows an extension's page and answers what it asks, and reads
//! here what that extension is allowed.

use std::path::Path;

use flutter_rust_bridge::frb;
use lumit_extensions::{ExtensionError, Installed, Manifest, Permission};

/// One thing an extension asked to be allowed, which the person agreed to
/// by installing it.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeExtensionPermission {
    /// What the open project is called, and when one opens, saves or closes.
    Project,
    /// When the person edits, and what each step is called.
    Activity,
    /// When an export starts and finishes, and the file it made.
    Export,
    /// Bring files into the project, and watch a folder for new ones.
    Import,
    /// Share the project, join someone else's, and see who is here.
    Share,
}

/// One extension, installed or about to be.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeExtension {
    /// Its folder name, and what [`extension_remove`] takes.
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Who made it, in their own words. Nothing checks it.
    pub author: String,
    pub homepage: String,
    /// The folder its page is served from. Empty for one not installed yet.
    pub folder: String,
    /// The page its panel shows, as a path inside `folder` with forward
    /// slashes.
    pub entry: String,
    pub permissions: Vec<BridgeExtensionPermission>,
    /// The sites it may ask the frontend to fetch from. A name stands for
    /// the sites under it too.
    pub hosts: Vec<String>,
    /// The page its manifest names is missing. It lists so it can be
    /// installed again or removed.
    pub broken: bool,
}

/// How reading or installing an extension went.
#[frb(non_opaque)]
#[derive(Debug, Clone)]
pub enum BridgeExtensionOutcome {
    Ready {
        extension: BridgeExtension,
    },
    /// The folder has no `extension.json`.
    NotAnExtension,
    /// The manifest breaks a rule, which `why` says in the engine's English.
    Invalid {
        why: String,
    },
    /// It needs a newer Lumit, which `version` names.
    Newer {
        version: String,
    },
    TooLarge,
    /// Another install is running.
    Busy,
    /// The files could not be copied, or this machine has nowhere for them.
    Failed,
}

#[frb(ignore)]
fn flatten(manifest: Manifest, folder: &Path, broken: bool) -> BridgeExtension {
    let permission = |permission: &Permission| match permission {
        Permission::Project => BridgeExtensionPermission::Project,
        Permission::Activity => BridgeExtensionPermission::Activity,
        Permission::Export => BridgeExtensionPermission::Export,
        Permission::Import => BridgeExtensionPermission::Import,
        Permission::Share => BridgeExtensionPermission::Share,
    };
    BridgeExtension {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        summary: manifest.summary,
        author: manifest.author,
        homepage: manifest.homepage,
        folder: folder.to_string_lossy().into_owned(),
        entry: manifest.entry.replace('\\', "/"),
        permissions: manifest.permissions.iter().map(permission).collect(),
        hosts: manifest.hosts,
        broken,
    }
}

#[frb(ignore)]
fn outcome(done: Result<Installed, ExtensionError>) -> BridgeExtensionOutcome {
    match done {
        Ok(installed) => BridgeExtensionOutcome::Ready {
            extension: flatten(installed.manifest, &installed.path, installed.broken),
        },
        Err(ExtensionError::NotAnExtension) => BridgeExtensionOutcome::NotAnExtension,
        Err(ExtensionError::Invalid(why)) => BridgeExtensionOutcome::Invalid { why },
        Err(ExtensionError::Newer(version)) => BridgeExtensionOutcome::Newer { version },
        Err(ExtensionError::TooLarge) => BridgeExtensionOutcome::TooLarge,
        Err(ExtensionError::Busy) => BridgeExtensionOutcome::Busy,
        Err(ExtensionError::NoFolder | ExtensionError::Missing | ExtensionError::Io(_)) => {
            BridgeExtensionOutcome::Failed
        }
    }
}

/// Every extension installed, by id.
#[frb(sync)]
#[must_use]
pub fn extension_list() -> Vec<BridgeExtension> {
    let installed = lumit_extensions::scan().into_iter();
    installed
        .map(|e| flatten(e.manifest, &e.path, e.broken))
        .collect()
}

/// What the extension in `folder` says about itself and asks to be allowed,
/// for the person to read before they agree. Nothing is installed.
#[frb(sync)]
#[must_use]
pub fn extension_inspect(folder: String) -> BridgeExtensionOutcome {
    let read = lumit_extensions::inspect(Path::new(&folder)).map(|manifest| Installed {
        manifest,
        path: Default::default(),
        broken: false,
    });
    outcome(read)
}

/// Install the extension in `folder`, over one of the same id if there is
/// one. Its files are copied, so the folder can go afterwards.
///
/// Not sync: a folder of pictures takes a moment to copy.
#[must_use]
pub fn extension_install(folder: String) -> BridgeExtensionOutcome {
    outcome(lumit_extensions::install(Path::new(&folder)))
}

/// Remove an extension and its folder. False when none of that id is
/// installed or its folder would not delete.
#[frb(sync)]
#[must_use]
pub fn extension_remove(id: String) -> bool {
    lumit_extensions::remove(&id).is_ok()
}

/// Where the extension `id` keeps what its page stores, which outlives an
/// update of the extension. `None` when the platform has no home directory.
#[frb(sync)]
#[must_use]
pub fn extension_data_dir(id: String) -> Option<String> {
    if !lumit_extensions::is_id(&id) {
        return None;
    }
    lumit_project::extension_data_dir(&id).map(|dir| dir.to_string_lossy().into_owned())
}

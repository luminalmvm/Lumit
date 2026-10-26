//! Extensions across the seam: what is installed, and the gestures that
//! install and remove one, from a folder or an `.lfx` package. `lumit-extensions`
//! judges and copies. The frontend shows an extension's page and answers
//! what it asks, and reads here what that extension is allowed. The LFX
//! effects an extension carries are registered here, after every gesture
//! that changes what is installed. An extension that may edit runs Lumit's
//! commands on the open project through here too.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

use flutter_rust_bridge::frb;
use lumit_core::fx::lfx;
use lumit_extensions::{ExtensionError, Installed, Manifest, Permission};

use crate::api::project::ProjectReference;

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
    /// Edit the open project through Lumit's commands, each edit one undo
    /// step. Never save or export it.
    Edit,
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
    /// Whether it has a panel of its own. An extension may carry only
    /// effects.
    pub panel: bool,
    /// The page its panel shows, as a path inside `folder` with forward
    /// slashes. Empty when it has no panel.
    pub entry: String,
    pub permissions: Vec<BridgeExtensionPermission>,
    /// The sites it may ask the frontend to fetch from. A name stands for
    /// the sites under it too.
    pub hosts: Vec<String>,
    /// A page or shader its manifest names is missing. It lists so it can
    /// be installed again or removed.
    pub broken: bool,
    /// The LFX effects it carries.
    pub effects: Vec<BridgeExtensionEffect>,
}

/// One LFX effect an extension carries.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeExtensionEffect {
    /// Its id within the extension.
    pub id: String,
    pub name: String,
    /// `lfx:<extension>/<effect>`, what adding it takes.
    pub match_name: String,
    /// Whether it has an interface page of its own.
    pub interface: bool,
    /// That page, as a path inside the extension's folder with forward
    /// slashes. Empty when it has none.
    pub entry: String,
    /// The interface panel's opening size in logical pixels. Nought when it
    /// has none.
    pub width: u32,
    pub height: u32,
}

/// The folders extensions are found in, in the order they are scanned. The
/// first extension of an id wins.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeExtensionFolders {
    /// Lumit's own, where an install goes unless another is picked. None
    /// when the machine has no home folder.
    pub own: Option<String>,
    /// The machine-wide ones, for authors who ship their own installer.
    pub machine: Vec<String>,
    /// The ones the person added.
    pub extra: Vec<String>,
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
        Permission::Edit => BridgeExtensionPermission::Edit,
    };
    let effects = manifest
        .effects
        .iter()
        .map(|effect| BridgeExtensionEffect {
            id: effect.id.clone(),
            name: effect.name.clone(),
            match_name: lfx::match_name(&manifest.id, &effect.id),
            interface: effect.interface.is_some(),
            entry: effect
                .interface
                .as_ref()
                .map(|page| page.entry.replace('\\', "/"))
                .unwrap_or_default(),
            width: effect.interface.as_ref().map_or(0, |page| page.width),
            height: effect.interface.as_ref().map_or(0, |page| page.height),
        })
        .collect();
    BridgeExtension {
        panel: manifest.entry.is_some(),
        entry: manifest.entry.unwrap_or_default().replace('\\', "/"),
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        summary: manifest.summary,
        author: manifest.author,
        homepage: manifest.homepage,
        folder: folder.to_string_lossy().into_owned(),
        permissions: manifest.permissions.iter().map(permission).collect(),
        hosts: manifest.hosts,
        broken,
        effects,
    }
}

/// A manifest read but not installed, as an outcome.
#[frb(ignore)]
fn read(manifest: Result<Manifest, ExtensionError>) -> BridgeExtensionOutcome {
    outcome(manifest.map(|manifest| Installed {
        manifest,
        path: Default::default(),
        broken: false,
    }))
}

/// One rescan of the effects at a time.
static REGISTERING: Mutex<()> = Mutex::new(());

/// Register every installed extension's effects, and take out the ones no
/// extension carries any more, so they read as missing. A changed shader is
/// picked up at once. Answers the labels of the effects new since Lumit started
/// and one line per effect turned away, for the plugin scan's report.
#[frb(ignore)]
pub(crate) fn register_effects() -> (Vec<String>, Vec<String>) {
    let _one = REGISTERING.lock();
    let (mut added, mut skipped) = (Vec::new(), Vec::new());
    let mut kept = BTreeSet::new();
    for installed in lumit_extensions::scan() {
        for effect in lumit_extensions::effects(&installed) {
            let name = lfx::match_name(&effect.extension, &effect.id);
            let (label, new) = (effect.label.clone(), !lfx::is_installed(&name));
            match lumit_render::gpufx::lfx::register(effect) {
                Ok(_) => {
                    kept.insert(name);
                    if new {
                        added.push(label);
                    }
                }
                Err(why) => skipped.push(format!("{name}: {why}")),
            }
        }
    }
    for gone in lfx::all() {
        if !kept.contains(gone.match_name) {
            lfx::uninstall(gone.match_name);
        }
    }
    (added, skipped)
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
        Err(
            ExtensionError::NoFolder
            | ExtensionError::Missing
            | ExtensionError::NotOurs
            | ExtensionError::Io(_),
        ) => BridgeExtensionOutcome::Failed,
    }
}

/// Every extension installed, by id.
///
/// Not sync: it reads every folder's manifests.
#[must_use]
pub fn extension_list() -> Vec<BridgeExtension> {
    let installed = lumit_extensions::scan().into_iter();
    installed
        .map(|e| flatten(e.manifest, &e.path, e.broken))
        .collect()
}

/// What the extension in `folder` says about itself and asks to be allowed,
/// for the person to read before they agree. Nothing is installed.
///
/// Not sync: it reads the folder.
#[must_use]
pub fn extension_inspect(folder: String) -> BridgeExtensionOutcome {
    read(lumit_extensions::inspect(Path::new(&folder)))
}

/// What the `.lfx` package at `path` holds and asks to be allowed, for the
/// install dialogue. Nothing is unpacked.
///
/// Not sync: a large package takes a moment to read.
#[must_use]
pub fn extension_inspect_package(path: String) -> BridgeExtensionOutcome {
    read(lumit_extensions::package::read_package(Path::new(&path)))
}

/// Install the extension in `folder`, over one of the same id if there is
/// one. Its files are copied, so the folder can go afterwards. Its effects
/// are registered.
///
/// Not sync: a folder of pictures takes a moment to copy.
#[must_use]
pub fn extension_install(folder: String) -> BridgeExtensionOutcome {
    let done = outcome(lumit_extensions::install(Path::new(&folder)));
    register_effects();
    done
}

/// Install the `.lfx` package at `path` into `folder`, or Lumit's own
/// folder when None, over one of the same id there. A folder Lumit does not
/// scan already is added to the extra folders. Its effects are registered.
///
/// Not sync: a package of pictures takes a moment to unpack.
#[must_use]
pub fn extension_install_package(path: String, folder: Option<String>) -> BridgeExtensionOutcome {
    let into = folder.as_deref().map(Path::new);
    let done = outcome(lumit_extensions::package::install_package(
        Path::new(&path),
        into,
    ));
    if let (BridgeExtensionOutcome::Ready { .. }, Some(into)) = (&done, into) {
        // The extension is in place either way; a folder that would not be
        // remembered only means it is not found after a restart.
        let _ = lumit_extensions::add_dir(into);
    }
    register_effects();
    done
}

/// Make a package of the extension in `folder` at `out`, checked by the
/// same reader an install uses. `Ready` carries what it holds. The effects
/// are rescanned, so an author's changed shader is picked up.
///
/// Not sync: a folder of pictures takes a moment to pack.
#[must_use]
pub fn extension_make_package(folder: String, out: String) -> BridgeExtensionOutcome {
    let done = read(lumit_extensions::package::pack(
        Path::new(&folder),
        Path::new(&out),
    ));
    register_effects();
    done
}

/// How removing an extension went.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeExtensionRemoval {
    Removed,
    /// None of that id is installed.
    Missing,
    /// It is in the machine's or an added folder, which Lumit leaves alone.
    NotOurs,
    /// Its folder would not delete.
    Failed,
}

/// Remove an extension and its folder, from Lumit's own folder only. Its
/// effects stay in any project that uses them, as missing.
///
/// Not sync: a folder of pictures takes a moment to delete.
#[must_use]
pub fn extension_remove(id: String) -> BridgeExtensionRemoval {
    let removed = match lumit_extensions::remove(&id) {
        Ok(()) => BridgeExtensionRemoval::Removed,
        Err(ExtensionError::Missing) => BridgeExtensionRemoval::Missing,
        Err(ExtensionError::NotOurs) => BridgeExtensionRemoval::NotOurs,
        Err(_) => BridgeExtensionRemoval::Failed,
    };
    register_effects();
    removed
}

/// Register what the folders carry now, so an extension another Lumit
/// installed is picked up without a restart.
///
/// Not sync: registering reads every extension's shaders.
pub fn extension_rescan() {
    register_effects();
}

/// The folders extensions are found in.
///
/// Not sync: the added ones are read from the settings file.
#[must_use]
pub fn extension_folders() -> BridgeExtensionFolders {
    let text = |dir: std::path::PathBuf| dir.to_string_lossy().into_owned();
    BridgeExtensionFolders {
        own: lumit_extensions::dir().map(text),
        machine: lumit_extensions::machine_dirs()
            .into_iter()
            .map(text)
            .collect(),
        extra: lumit_extensions::extra_dirs()
            .into_iter()
            .map(text)
            .collect(),
    }
}

/// Scan `folder` for extensions too, and register what it carries. False
/// when it is scanned already or would not be remembered.
///
/// Not sync: registering reads every extension's shaders.
#[must_use]
pub fn extension_add_folder(folder: String) -> bool {
    let added = lumit_extensions::add_dir(Path::new(&folder)).unwrap_or(false);
    register_effects();
    added
}

/// Stop scanning a folder the person added. Nothing in it is deleted, and
/// its effects read as missing. False when it was not one of theirs.
///
/// Not sync: registering reads every extension's shaders.
#[must_use]
pub fn extension_remove_folder(folder: String) -> bool {
    let removed = lumit_extensions::remove_dir(Path::new(&folder));
    register_effects();
    removed
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

/// One of Lumit's commands, as an extension's page lists it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeExtensionCommand {
    pub name: String,
    /// One line saying what it does.
    pub summary: String,
    /// A JSON Schema for its arguments, as JSON text.
    pub schema: String,
}

/// What an extension's edit came to.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeExtensionEdit {
    /// It ran. `result` is the command's answer as JSON text.
    Done { result: String },
    /// It did nothing. `code` is one a page's refused call carries.
    Refused { code: String, message: String },
}

/// The commands an extension with the Edit permission may run. `import` is
/// whether it has the Import permission too.
#[frb(sync)]
#[must_use]
pub fn extension_commands(import: bool) -> Vec<BridgeExtensionCommand> {
    crate::extension_edit::commands(import)
}

impl ProjectReference {
    /// Run command `command` with `args`, JSON text, on this project for the
    /// extension called `extension`, as one undo step named after it.
    /// Saving, exporting, undo and redo are refused, and importing unless
    /// `import`. Waits a moment for a drag in the window to end.
    ///
    /// Not sync: it may wait for that drag.
    #[must_use]
    pub fn extension_edit(
        &self,
        extension: String,
        import: bool,
        command: String,
        args: String,
    ) -> BridgeExtensionEdit {
        crate::extension_edit::run(self, &extension, import, &command, &args)
    }
}

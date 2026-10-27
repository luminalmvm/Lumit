//! Extensions: panels other people make, shown inside Lumit.
//!
//! An extension is a folder: an `extension.json` that says what it is and
//! what it asks to be allowed, the web page it names, with whatever that
//! page loads, and the effects it carries. The interface shows the page in a
//! panel of its own. The page can do nothing to Lumit by itself. It asks
//! through the interface, which answers only what the extension's manifest
//! asked for and the person agreed to when they installed it. An effect is a
//! WGSL shader (an LFX effect), which the engine checks before it ever runs,
//! or an OFX plugin's bundle, which runs in a process of its own like any
//! other OFX plugin.
//!
//! This crate is the folder's side of that: reading and judging a manifest,
//! the folders extensions are found in, `.lfx` packages, and installing,
//! listing and removing. It runs nothing and shows nothing. Called from the
//! bridge, off the UI thread for an install.

use lumit_core::fx::lfx::{LfxEffect, LfxInput, LfxInputKind, LfxPass};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

pub mod package;

/// The file that makes a folder an extension.
pub const MANIFEST_FILE: &str = "extension.json";

/// The manifest shape this build reads. Raised when a field changes meaning.
pub const FORMAT: u32 = 1;

/// Where an install is put together before it is moved into place.
const STAGING: &str = ".staging";

/// The most an extension's folder holds. A page, its scripts and pictures.
const MAX_FILES: usize = 4096;
const MAX_BYTES: u64 = 512 << 20;
const MAX_DEPTH: usize = 16;

/// The longest a manifest is read.
const MAX_MANIFEST: u64 = 1 << 20;

/// The longest an effect's shader is read.
const MAX_SHADER: u64 = 1 << 20;

/// The most effects one extension carries.
const MAX_EFFECTS: usize = 64;

/// One thing an extension may ask to be allowed. Each is a family of
/// questions it can ask the interface and of things it is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
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

impl Permission {
    /// The word a manifest spells it with.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Permission::Project => "project",
            Permission::Activity => "activity",
            Permission::Export => "export",
            Permission::Import => "import",
            Permission::Share => "share",
            Permission::Edit => "edit",
        }
    }
}

/// What an `extension.json` says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// Its folder's name, and how everything knows it. Lower-case letters,
    /// digits and hyphens.
    pub id: String,
    pub name: String,
    pub version: String,
    /// One line saying what it does.
    #[serde(default)]
    pub summary: String,
    /// Who made it, as they call themselves. Nothing checks it.
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: String,
    /// The oldest Lumit it works with, as `0.7.0`. Empty for any.
    #[serde(default)]
    pub requires: String,
    /// The page the panel shows, as a path inside the folder. None for an
    /// extension that only carries effects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// The sites it may ask the interface to fetch from, as bare names such
    /// as `example.org`. A name stands for the sites under it too.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The LFX effects it carries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ManifestEffect>,
}

/// One effect an extension carries: a shader written to the Custom shader's
/// contract, and what Lumit needs to offer it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEffect {
    /// Unique within the extension. Lower-case letters, digits and hyphens.
    pub id: String,
    pub name: String,
    /// One of Lumit's effect categories, by name, such as `Stylise`.
    pub category: String,
    /// Bumped by the author when the pixels change.
    pub version: u32,
    /// The shader contract it was written to. 1 for an OFX plugin.
    pub api: u32,
    /// The `.wgsl` file, as a path inside the folder. Exactly one of this
    /// and `ofx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<String>,
    /// An OFX plugin's bundle folder inside the extension, such as
    /// `Sort.ofx.bundle`. The plugin scan finds it like any other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ofx: Option<String>,
    /// The OFX plugin's identifier, so the effect is `ofx:<plugin>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// None means one pass called `shade`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<ManifestPass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ManifestInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<ManifestInterface>,
}

/// One pass: a function in the shader, drawn at `scale` of the picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestPass {
    pub name: String,
    /// 1, 0.5 or 0.25.
    #[serde(default = "full_size")]
    pub scale: f32,
}

fn full_size() -> f32 {
    1.0
}

/// The extra picture an effect takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestInput {
    pub label: String,
    pub kind: InputKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    Image,
    /// A Depth socket in the node graph.
    Depth,
}

/// The page an effect's own panel shows, and the panel's opening size in
/// logical pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestInterface {
    pub entry: String,
    pub width: u32,
    pub height: u32,
}

/// What the engine keeps of `effect`, with its shader's text.
fn lfx_effect(extension: &str, effect: &ManifestEffect, shader: String) -> LfxEffect {
    LfxEffect {
        extension: extension.to_owned(),
        id: effect.id.clone(),
        label: effect.name.clone(),
        category: effect.category.clone(),
        version: effect.version,
        api: effect.api,
        passes: effect
            .passes
            .iter()
            .map(|p| LfxPass {
                name: p.name.clone(),
                scale: p.scale,
            })
            .collect(),
        input: effect.input.as_ref().map(|input| LfxInput {
            label: input.label.clone(),
            kind: match input.kind {
                InputKind::Image => LfxInputKind::Image,
                InputKind::Depth => LfxInputKind::Depth,
            },
        }),
        shader,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("this computer has no folder to keep extensions in")]
    NoFolder,
    #[error("that folder has no extension.json in it")]
    NotAnExtension,
    #[error("{0}")]
    Invalid(String),
    #[error("this extension needs Lumit {0} or newer")]
    Newer(String),
    #[error("an extension's folder holds at most {MAX_FILES} files and 512 MB")]
    TooLarge,
    #[error("another extension is being installed")]
    Busy,
    #[error("no extension of that name is installed")]
    Missing,
    #[error("this extension is in a folder Lumit doesn't own, so it stays there")]
    NotOurs,
    #[error("the extension's files could not be copied: {0}")]
    Io(#[from] std::io::Error),
}

fn invalid(why: &str) -> ExtensionError {
    ExtensionError::Invalid(why.to_owned())
}

/// Whether `id` can name an extension, and so a folder.
#[must_use]
pub fn is_id(id: &str) -> bool {
    let plain = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    (1..=64).contains(&id.len()) && id.bytes().all(plain)
}

/// Whether `host` is a bare site name: no scheme, port, path or wildcard.
fn is_host(host: &str) -> bool {
    let label = |label: &str| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    host.len() <= 253 && host.contains('.') && host.split('.').all(label)
}

/// `entry` as a path that stays inside the folder it is joined to.
fn inside(entry: &str) -> Option<PathBuf> {
    let path = Path::new(entry);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_)));
    (plain && !entry.is_empty() && !entry.contains(['\\', ':'])).then(|| path.to_path_buf())
}

/// The three numbers of a version, with anything after them left off.
fn version(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.trim().split(['.', '-', '+']);
    let mut next = || parts.next()?.parse().ok();
    Some([next()?, next()?, next().unwrap_or(0)])
}

/// Read a manifest and judge it. What is refused here is refused before a
/// single file is copied.
pub fn parse(text: &str) -> Result<Manifest, ExtensionError> {
    // `requires` is read before anything else, so a manifest that names a
    // newer Lumit is turned away as that, whatever fields it has that this
    // one does not know.
    #[derive(Deserialize)]
    struct Requires {
        #[serde(default)]
        requires: String,
    }
    if let Ok(Requires { requires }) = serde_json::from_str(text) {
        at_least(&requires)?;
    }
    let manifest: Manifest = serde_json::from_str(text).map_err(|e| {
        // Its own sentence names the field, which is what the author needs.
        ExtensionError::Invalid(format!("extension.json could not be read: {e}"))
    })?;
    if manifest.format == 0 || manifest.format > FORMAT {
        return Err(invalid(
            "extension.json is in a format this Lumit does not read",
        ));
    }
    if !is_id(&manifest.id) {
        return Err(invalid(
            "an extension's id is lower-case letters, digits and hyphens",
        ));
    }
    if manifest.name.trim().is_empty() || manifest.name.len() > 64 {
        return Err(invalid("an extension needs a name of at most 64 letters"));
    }
    if manifest.version.trim().is_empty() || manifest.version.len() > 32 {
        return Err(invalid("an extension needs a version"));
    }
    if manifest.summary.len() > 300 || manifest.author.len() > 64 || manifest.homepage.len() > 200 {
        return Err(invalid("extension.json says too much about itself"));
    }
    if manifest.entry.is_none() && manifest.effects.is_empty() {
        return Err(invalid("an extension adds a panel, effects or both"));
    }
    if manifest
        .entry
        .as_deref()
        .is_some_and(|e| inside(e).is_none())
    {
        return Err(invalid(
            "an extension's entry is a page inside its own folder",
        ));
    }
    if manifest.effects.len() > MAX_EFFECTS {
        return Err(invalid("an extension carries at most 64 effects"));
    }
    for (i, effect) in manifest.effects.iter().enumerate() {
        judge_effect(&manifest, effect)?;
        if manifest.effects[..i].iter().any(|e| e.id == effect.id) {
            return Err(ExtensionError::Invalid(format!(
                "two effects are called {}",
                effect.id
            )));
        }
    }
    if manifest.hosts.len() > 16 || !manifest.hosts.iter().all(|host| is_host(host)) {
        return Err(invalid(
            "an extension's hosts are at most 16 bare site names, such as example.org",
        ));
    }
    Ok(manifest)
}

/// Whether this Lumit is at least `requires`, which may be empty for any.
fn at_least(requires: &str) -> Result<(), ExtensionError> {
    if requires.is_empty() {
        return Ok(());
    }
    let needs =
        version(requires).ok_or_else(|| invalid("requires is a Lumit version, such as 0.7.0"))?;
    if version(env!("CARGO_PKG_VERSION")).is_some_and(|have| have < needs) {
        return Err(ExtensionError::Newer(requires.to_owned()));
    }
    Ok(())
}

/// One effect's entry, judged. The engine's own check has the last word on
/// the category, the api and the passes, so the two can never disagree.
fn judge_effect(manifest: &Manifest, effect: &ManifestEffect) -> Result<(), ExtensionError> {
    let id = &effect.id;
    if !is_id(id) {
        return Err(invalid(
            "an effect's id is lower-case letters, digits and hyphens",
        ));
    }
    let named = |name: &str| !name.trim().is_empty() && name.len() <= 64;
    if !named(&effect.name) {
        return Err(invalid("an effect needs a name of at most 64 letters"));
    }
    match (&effect.shader, &effect.ofx) {
        (Some(shader), None) if inside(shader).is_none() => {
            return Err(ExtensionError::Invalid(format!(
                "the {id} effect's shader is a file inside the extension's own folder"
            )));
        }
        (Some(_), None) => {}
        (None, Some(bundle)) => {
            if !bundle.ends_with(".ofx.bundle") || inside(bundle).is_none() {
                return Err(ExtensionError::Invalid(format!(
                    "the {id} effect's ofx is a .ofx.bundle folder inside the extension's own folder"
                )));
            }
            let plugin = effect.plugin.as_deref().unwrap_or_default();
            if plugin.is_empty() || plugin.len() > 256 || plugin.contains(char::is_whitespace) {
                return Err(ExtensionError::Invalid(format!(
                    "the {id} effect needs its OFX plugin's identifier as plugin"
                )));
            }
        }
        _ => {
            return Err(ExtensionError::Invalid(format!(
                "the {id} effect has a shader or an ofx bundle, and not both"
            )));
        }
    }
    if effect
        .input
        .as_ref()
        .is_some_and(|input| !named(&input.label))
    {
        return Err(ExtensionError::Invalid(format!(
            "the {id} effect's input needs a label of at most 64 letters"
        )));
    }
    if let Some(page) = &effect.interface {
        if inside(&page.entry).is_none() {
            return Err(ExtensionError::Invalid(format!(
                "the {id} effect's interface is a page inside the extension's own folder"
            )));
        }
        let size = 100..=4096;
        if !size.contains(&page.width) || !size.contains(&page.height) {
            return Err(ExtensionError::Invalid(format!(
                "the {id} effect's interface is 100 to 4096 pixels each way"
            )));
        }
    }
    lumit_core::fx::lfx::check(&lfx_effect(&manifest.id, effect, String::new()))
        .map(|_| ())
        .map_err(|why| ExtensionError::Invalid(format!("the {id} effect: {why}")))
}

/// Somewhere other than the user's own extensions folder, when a test has
/// said so.
static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The one install in flight.
static SLOT: Mutex<()> = Mutex::new(());

fn overridden() -> Option<PathBuf> {
    OVERRIDE.lock().ok().and_then(|over| over.clone())
}

/// Lumit's own extensions folder, where an install goes unless the person
/// picks another: what [`with_dir`] was last given, else the platform's own
/// folder.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    overridden().or_else(lumit_project::extensions_dir)
}

/// Point [`dir`] at `dir` instead, or at nothing to put it back. While it
/// points somewhere, that is the only folder scanned. For tests, the
/// bridge's included, which must not write into the user's own folder.
pub fn with_dir(dir: Option<PathBuf>) {
    if let Ok(mut over) = OVERRIDE.lock() {
        *over = dir;
    }
}

/// The machine-wide folders, for authors who ship their own installer.
#[cfg(windows)]
#[must_use]
pub fn machine_dirs() -> Vec<PathBuf> {
    std::env::var_os("CommonProgramFiles")
        .map(|common| PathBuf::from(common).join("Lumit").join("Extensions"))
        .into_iter()
        .collect()
}

/// The machine-wide folders, for authors who ship their own installer.
#[cfg(target_os = "macos")]
#[must_use]
pub fn machine_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from(
        "/Library/Application Support/Lumit/Extensions",
    )]
}

/// The machine-wide folders, for authors who ship their own installer. Only
/// a native install sees it: inside the Flatpak `/usr/share` is the
/// runtime's, and the sandbox grants no host path for it.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub fn machine_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("/usr/share/lumit/extensions")]
}

#[cfg(not(any(windows, unix)))]
#[must_use]
pub fn machine_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// The folders the person added, in the order they added them.
#[must_use]
pub fn extra_dirs() -> Vec<PathBuf> {
    lumit_project::plugins::PluginPrefs::load_default().extension_folders
}

/// Every folder extensions are found in, in the order they are scanned:
/// Lumit's own, the machine's, then the ones the person added. The first
/// extension of an id wins.
#[must_use]
pub fn dirs() -> Vec<PathBuf> {
    if let Some(over) = overridden() {
        return vec![over];
    }
    let mut all: Vec<PathBuf> = Vec::new();
    let own = lumit_project::extensions_dir().into_iter();
    for dir in own.chain(machine_dirs()).chain(extra_dirs()) {
        if !all.contains(&dir) {
            all.push(dir);
        }
    }
    all
}

/// Add `folder` to the ones scanned. False when it is scanned already.
///
/// # Errors
/// A folder given by a relative path, or preferences that would not save.
pub fn add_dir(folder: &Path) -> Result<bool, ExtensionError> {
    if !folder.is_absolute() {
        return Err(invalid("an extensions folder is given by its full path"));
    }
    if dirs().iter().any(|dir| dir == folder) {
        return Ok(false);
    }
    let mut prefs = lumit_project::plugins::PluginPrefs::load_default();
    prefs.extension_folders.push(folder.to_path_buf());
    prefs
        .save_default()
        .map_err(|why| ExtensionError::Io(std::io::Error::other(why)))?;
    Ok(true)
}

/// Stop scanning a folder the person added. Nothing in it is deleted. False
/// when it was not one of theirs.
pub fn remove_dir(folder: &Path) -> bool {
    let mut prefs = lumit_project::plugins::PluginPrefs::load_default();
    let before = prefs.extension_folders.len();
    prefs.extension_folders.retain(|dir| dir != folder);
    prefs.extension_folders.len() != before && prefs.save_default().is_ok()
}

/// One extension as its folder has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub manifest: Manifest,
    /// Its folder, which the panel's page is served from.
    pub path: PathBuf,
    /// A page or shader the manifest names is not there. It still lists, so
    /// it can be installed again or removed.
    pub broken: bool,
}

/// Whether every file the manifest names is there, asking `size` for the
/// size of a file by its path inside the extension, or None when it is not
/// a file. Shared by a folder and a package, so both are judged alike.
fn check_files(
    manifest: &Manifest,
    size: &dyn Fn(&Path) -> Option<u64>,
) -> Result<(), ExtensionError> {
    let there = |entry: &str| inside(entry).is_some_and(|path| size(&path).is_some());
    if manifest.entry.as_deref().is_some_and(|entry| !there(entry)) {
        return Err(invalid(
            "the page extension.json names is not in the folder",
        ));
    }
    for effect in &manifest.effects {
        let id = &effect.id;
        // An OFX bundle is a folder, and one without a binary for this
        // computer only means its plugin is not found here.
        let shader = effect.shader.as_deref();
        match shader.map(|shader| inside(shader).and_then(|path| size(&path))) {
            Some(None) => {
                return Err(ExtensionError::Invalid(format!(
                    "the {id} effect's shader is not in the folder"
                )))
            }
            Some(Some(bytes)) if bytes > MAX_SHADER => {
                return Err(ExtensionError::Invalid(format!(
                    "the {id} effect's shader is longer than 1 MB"
                )))
            }
            _ => {}
        }
        if effect
            .interface
            .as_ref()
            .is_some_and(|page| !there(&page.entry))
        {
            return Err(ExtensionError::Invalid(format!(
                "the {id} effect's interface page is not in the folder"
            )));
        }
    }
    Ok(())
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .filter(fs::Metadata::is_file)
        .map(|m| m.len())
}

fn read_manifest(folder: &Path) -> Result<Manifest, ExtensionError> {
    let file = folder.join(MANIFEST_FILE);
    let size = fs::metadata(&file)
        .map_err(|_| ExtensionError::NotAnExtension)?
        .len();
    if size > MAX_MANIFEST {
        return Err(invalid("extension.json is too long"));
    }
    parse(&fs::read_to_string(file)?)
}

/// What the extension in `folder` says about itself, judged, for the person
/// to read before they agree to it. Nothing is copied.
pub fn inspect(folder: &Path) -> Result<Manifest, ExtensionError> {
    let manifest = read_manifest(folder)?;
    check_files(&manifest, &|path| file_size(&folder.join(path)))?;
    Ok(manifest)
}

/// Every extension installed, by id, from every folder in [`dirs`]; the
/// first of an id wins. A folder with no manifest that reads, or one whose
/// manifest names another id, is not one and is not listed.
#[must_use]
pub fn scan() -> Vec<Installed> {
    // What an install left half done, but never under one running now, and
    // only in Lumit's own folder: the others aren't ours to tidy.
    if let (Ok(_slot), Some(own)) = (SLOT.try_lock(), dir()) {
        let _ = fs::remove_dir_all(own.join(STAGING));
    }
    let mut found: Vec<Installed> = Vec::new();
    for dir in dirs() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(manifest) = read_manifest(&path) else {
                continue;
            };
            let named = path.file_name().and_then(|n| n.to_str()) == Some(&manifest.id);
            if !named || found.iter().any(|f| f.manifest.id == manifest.id) {
                continue;
            }
            let broken = check_files(&manifest, &|p| file_size(&path.join(p))).is_err();
            found.push(Installed {
                manifest,
                path,
                broken,
            });
        }
    }
    found.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    found
}

/// The effects an installed extension carries, each with its shader read,
/// for the engine to register. One whose shader is gone or too long is left
/// out, and so reads as missing.
#[must_use]
pub fn effects(installed: &Installed) -> Vec<LfxEffect> {
    let manifest = &installed.manifest;
    let read = |effect: &ManifestEffect| {
        let path = installed.path.join(inside(effect.shader.as_deref()?)?);
        if file_size(&path)? > MAX_SHADER {
            return None;
        }
        let text = fs::read_to_string(path).ok()?;
        Some(lfx_effect(&manifest.id, effect, text))
    };
    manifest.effects.iter().filter_map(read).collect()
}

/// The OFX bundle folders an installed extension carries, for the plugin
/// scan. Two effects may share one bundle.
#[must_use]
pub fn bundles(installed: &Installed) -> Vec<PathBuf> {
    let bundles: std::collections::BTreeSet<PathBuf> = installed
        .manifest
        .effects
        .iter()
        .filter_map(|effect| Some(installed.path.join(inside(effect.ofx.as_deref()?)?)))
        .collect();
    bundles.into_iter().collect()
}

/// What a copy has left to spend.
struct Room {
    files: usize,
    bytes: u64,
}

/// Copy the files under `from` to `to`. Links are not followed and not
/// copied, so nothing outside the folder comes with it.
fn copy_tree(from: &Path, to: &Path, depth: usize, room: &mut Room) -> Result<(), ExtensionError> {
    if depth > MAX_DEPTH {
        return Err(ExtensionError::TooLarge);
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if kind.is_dir() {
            copy_tree(&source, &target, depth + 1, room)?;
        } else if kind.is_file() {
            let size = entry.metadata()?.len();
            if room.files == 0 || size > room.bytes {
                return Err(ExtensionError::TooLarge);
            }
            room.files -= 1;
            room.bytes -= size;
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// Put the extension `id` together to one side of `into` with `fill`, then
/// move it into place whole, over one of the same id there. An install that
/// fails leaves what was there.
fn put_in_place(
    into: &Path,
    id: &str,
    fill: impl FnOnce(&Path) -> Result<(), ExtensionError>,
) -> Result<PathBuf, ExtensionError> {
    let place = into.join(id);
    let mut nonce = [0u8; 8];
    let _ = getrandom::fill(&mut nonce);
    let staging = into.join(STAGING);
    let built = staging.join(format!("{id}-{:016x}", u64::from_le_bytes(nonce)));
    let done = fs::create_dir_all(&built)
        .map_err(ExtensionError::from)
        .and_then(|()| fill(&built))
        .and_then(|()| {
            let old = staging.join(format!("{id}-old"));
            let _ = fs::remove_dir_all(&old);
            if place.exists() {
                fs::rename(&place, &old)?;
            }
            fs::rename(&built, &place)?;
            Ok(())
        });
    let _ = fs::remove_dir_all(&staging);
    done.map(|()| place)
}

/// Install the extension in `folder` into Lumit's own folder, over one of
/// the same id if there is one. Its files are copied, so the folder can go
/// afterwards.
pub fn install(folder: &Path) -> Result<Installed, ExtensionError> {
    let _slot = SLOT.try_lock().map_err(|_| ExtensionError::Busy)?;
    let manifest = inspect(folder)?;
    let dir = dir().ok_or(ExtensionError::NoFolder)?;
    let place = dir.join(&manifest.id);
    // Installing an extension from where it is installed would copy it over
    // itself. It is there already.
    let same = |a: &Path, b: &Path| fs::canonicalize(a).ok() == fs::canonicalize(b).ok();
    if place.exists() && same(folder, &place) {
        return Ok(Installed {
            manifest,
            path: place,
            broken: false,
        });
    }
    let mut room = Room {
        files: MAX_FILES,
        bytes: MAX_BYTES,
    };
    let path = put_in_place(&dir, &manifest.id, |built| {
        copy_tree(folder, built, 0, &mut room)
    })?;
    Ok(Installed {
        manifest,
        path,
        broken: false,
    })
}

/// Remove the extension `id` and its folder. Only from Lumit's own folder:
/// one found first in the machine's or an added folder is someone else's to
/// remove, and is left as it is.
pub fn remove(id: &str) -> Result<(), ExtensionError> {
    if !is_id(id) {
        return Err(ExtensionError::Missing);
    }
    let found = dirs()
        .into_iter()
        .find(|dir| dir.join(id).join(MANIFEST_FILE).is_file())
        .ok_or(ExtensionError::Missing)?;
    if dir().as_ref() != Some(&found) {
        return Err(ExtensionError::NotOurs);
    }
    fs::remove_dir_all(found.join(id))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::package::{install_package, pack, read_package};
    use super::*;

    fn manifest(change: impl Fn(&mut serde_json::Value)) -> String {
        let mut value = serde_json::json!({
            "format": 1,
            "id": "timer",
            "name": "Timer",
            "version": "1.0.0",
            "entry": "ui/index.html",
            "permissions": ["activity", "share"],
            "hosts": ["example.org"],
            "effects": [{
                "id": "tape",
                "name": "Tape",
                "category": "Stylise",
                "version": 1,
                "api": 1,
                "shader": "tape.wgsl",
                "passes": [{ "name": "bleed", "scale": 0.5 }, { "name": "shade" }],
                "input": { "label": "Second input", "kind": "image" },
                "interface": { "entry": "ui/tape.html", "width": 420, "height": 260 },
            }],
        });
        change(&mut value);
        value.to_string()
    }

    /// An extension can ask only for what this Lumit knows how to refuse,
    /// can name only pages and shaders inside its own folder and sites by
    /// name, and carries only effects this Lumit can draw. The Tape sample
    /// in examples/ has to stay one of those.
    #[test]
    fn a_manifest_that_reaches_past_its_folder_is_refused() {
        let tape = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/tape");
        let installed = Installed {
            manifest: inspect(&tape).unwrap(),
            path: tape,
            broken: false,
        };
        let carried = effects(&installed);
        assert_eq!(carried.len(), 1);
        for effect in carried {
            let passes: Vec<&str> = effect.passes.iter().map(|p| p.name.as_str()).collect();
            let program = lumit_core::fx::shader::build_passes(&effect.shader, &passes).unwrap();
            for (_, text) in &program.passes {
                if let Err(why) = lumit_gpu::fx::custom_shader::validate(text) {
                    panic!("{}", program.remap_error(&why));
                }
            }
        }

        assert!(parse(&manifest(|_| {})).is_ok());
        let effects_only = manifest(|m| {
            m.as_object_mut().unwrap().remove("entry");
        });
        assert!(parse(&effects_only).is_ok());
        let neither = manifest(|m| {
            m.as_object_mut().unwrap().remove("entry");
            m["effects"] = serde_json::json!([]);
        });
        assert!(parse(&neither).is_err());
        let refused = [
            ("entry", serde_json::json!("../outside.html")),
            ("entry", serde_json::json!("C:\\outside.html")),
            ("entry", serde_json::json!("/etc/passwd")),
            ("id", serde_json::json!("../timer")),
            ("permissions", serde_json::json!(["everything"])),
            ("hosts", serde_json::json!(["https://example.org/path"])),
            ("hosts", serde_json::json!(["*"])),
            ("format", serde_json::json!(2)),
            ("requires", serde_json::json!("99.0.0")),
        ];
        for (field, value) in refused {
            let text = manifest(|m| m[field] = value.clone());
            assert!(parse(&text).is_err(), "{field}: {value}");
        }
        let refused = [
            ("shader", serde_json::json!("../tape.wgsl")),
            ("id", serde_json::json!("Tape Deck")),
            ("category", serde_json::json!("Sparkles")),
            ("api", serde_json::json!(2)),
            ("passes", serde_json::json!([{ "name": "a", "scale": 0.3 }])),
            (
                "interface",
                serde_json::json!({ "entry": "../tape.html", "width": 420, "height": 260 }),
            ),
        ];
        for (field, value) in refused {
            let text = manifest(|m| m["effects"][0][field] = value.clone());
            assert!(parse(&text).is_err(), "effect {field}: {value}");
        }

        // An OFX plugin's bundle loads native code, so it too stays inside.
        let ofx = |bundle: &str, plugin: &str| {
            manifest(|m| {
                let effect = m["effects"][0].as_object_mut().unwrap();
                effect.remove("shader");
                effect.insert("ofx".into(), bundle.into());
                effect.insert("plugin".into(), plugin.into());
            })
        };
        assert!(parse(&ofx("Tape.ofx.bundle", "com.example.tape")).is_ok());
        let refused = [
            ("../Tape.ofx.bundle", "com.example.tape"),
            ("C:\\Tape.ofx.bundle", "com.example.tape"),
            ("tape.dll", "com.example.tape"),
            ("Tape.ofx.bundle", ""),
        ];
        for (bundle, plugin) in refused {
            assert!(
                parse(&ofx(bundle, plugin)).is_err(),
                "ofx {bundle}: {plugin}"
            );
        }
        let both = manifest(|m| m["effects"][0]["ofx"] = "Tape.ofx.bundle".into());
        assert!(parse(&both).is_err());
    }

    /// Installing copies the folder whole, an install over another replaces
    /// it whole, a package installs the same as its folder, and a removed
    /// extension leaves nothing.
    #[test]
    fn an_extension_is_installed_listed_replaced_and_removed() {
        let root = std::env::temp_dir().join(format!("lumit-ext-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (source, store) = (root.join("source"), root.join("store"));
        fs::create_dir_all(source.join("ui")).unwrap();
        fs::create_dir_all(&store).unwrap();
        with_dir(Some(store.clone()));

        assert!(matches!(
            install(&source),
            Err(ExtensionError::NotAnExtension)
        ));
        fs::write(source.join(MANIFEST_FILE), manifest(|_| {})).unwrap();
        assert!(install(&source).is_err(), "the page is not there yet");
        fs::write(source.join("ui/index.html"), "one").unwrap();
        fs::write(source.join("ui/tape.html"), "deck").unwrap();
        assert!(install(&source).is_err(), "the shader is not there yet");
        fs::write(
            source.join("tape.wgsl"),
            "fn shade(uv: vec2<f32>) -> vec4<f32> {}",
        )
        .unwrap();
        fs::write(source.join("ui/old.js"), "old").unwrap();
        let installed = install(&source).unwrap();
        assert_eq!(installed.path, store.join("timer"));
        assert_eq!(scan(), std::slice::from_ref(&installed));
        let carried = effects(&installed);
        assert_eq!(carried.len(), 1);
        assert!(carried[0].shader.starts_with("fn shade"));

        fs::remove_file(source.join("ui/old.js")).unwrap();
        fs::write(source.join("ui/index.html"), "two").unwrap();
        install(&source).unwrap();
        let page = store.join("timer/ui/index.html");
        assert_eq!(fs::read_to_string(page).unwrap(), "two");
        assert!(!store.join("timer/ui/old.js").exists());

        let made = root.join("timer.lfx");
        fs::write(source.join("ui/index.html"), "three").unwrap();
        assert_eq!(pack(&source, &made).unwrap().id, "timer");
        assert_eq!(read_package(&made).unwrap().id, "timer");
        install_package(&made, None).unwrap();
        let page = store.join("timer/ui/index.html");
        assert_eq!(fs::read_to_string(page).unwrap(), "three");

        remove("timer").unwrap();
        assert!(scan().is_empty());
        assert!(matches!(remove("timer"), Err(ExtensionError::Missing)));
        with_dir(None);
        let _ = fs::remove_dir_all(&root);
    }
}

//! Extensions: panels other people make, shown inside Lumit.
//!
//! An extension is a folder: an `extension.json` that says what it is and
//! what it asks to be allowed, and the web page it names, with whatever that
//! page loads. The interface shows the page in a panel of its own. The page
//! can do nothing to Lumit by itself. It asks through the interface, which
//! answers only what the extension's manifest asked for and the person
//! agreed to when they installed it.
//!
//! This crate is the folder's side of that: reading and judging a manifest,
//! and installing, listing and removing. It runs nothing and shows nothing.
//! Called from the bridge, off the UI thread for an install.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

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
        }
    }
}

/// What an `extension.json` says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The page the panel shows, as a path inside the folder.
    pub entry: String,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// The sites it may ask the interface to fetch from, as bare names such
    /// as `example.org`. A name stands for the sites under it too.
    #[serde(default)]
    pub hosts: Vec<String>,
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
    if inside(&manifest.entry).is_none() {
        return Err(invalid(
            "an extension's entry is a page inside its own folder",
        ));
    }
    if manifest.hosts.len() > 16 || !manifest.hosts.iter().all(|host| is_host(host)) {
        return Err(invalid(
            "an extension's hosts are at most 16 bare site names, such as example.org",
        ));
    }
    if !manifest.requires.is_empty() {
        let needs = version(&manifest.requires)
            .ok_or_else(|| invalid("requires is a Lumit version, such as 0.7.0"))?;
        if version(env!("CARGO_PKG_VERSION")).is_some_and(|have| have < needs) {
            return Err(ExtensionError::Newer(manifest.requires));
        }
    }
    Ok(manifest)
}

/// Somewhere other than the user's own extensions folder, when a test has
/// said so.
static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The one install in flight.
static SLOT: Mutex<()> = Mutex::new(());

/// Where extensions live: what [`with_dir`] was last given, else the
/// platform's own folder.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    let over = OVERRIDE.lock().ok().and_then(|over| over.clone());
    over.or_else(lumit_project::extensions_dir)
}

/// Point [`dir`] at `dir` instead, or at nothing to put it back. For tests,
/// the bridge's included, which must not write into the user's own folder.
pub fn with_dir(dir: Option<PathBuf>) {
    if let Ok(mut over) = OVERRIDE.lock() {
        *over = dir;
    }
}

/// One extension as its folder has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub manifest: Manifest,
    /// Its folder, which the panel's page is served from.
    pub path: PathBuf,
    /// The page the manifest names is not there. It still lists, so it can
    /// be installed again or removed.
    pub broken: bool,
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
    let entry = inside(&manifest.entry).map(|entry| folder.join(entry));
    if !entry.is_some_and(|entry| entry.is_file()) {
        return Err(invalid(
            "the page extension.json names is not in the folder",
        ));
    }
    Ok(manifest)
}

/// Every extension installed, by id. A folder with no manifest that reads,
/// or one whose manifest names another id, is not one and is not listed.
#[must_use]
pub fn scan() -> Vec<Installed> {
    let Some(dir) = dir() else {
        return Vec::new();
    };
    let _ = fs::remove_dir_all(dir.join(STAGING));
    let mut found: Vec<Installed> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let manifest = read_manifest(&path).ok()?;
            if path.file_name()?.to_str()? != manifest.id {
                return None;
            }
            let entry = inside(&manifest.entry).map(|entry| path.join(entry));
            let broken = !entry.is_some_and(|entry| entry.is_file());
            Some(Installed {
                manifest,
                path,
                broken,
            })
        })
        .collect();
    found.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    found
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

/// Install the extension in `folder`, over one of the same id if there is
/// one. Its files are copied, so the folder can go afterwards. Put together
/// to one side and moved into place whole, so an install that fails leaves
/// what was there.
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
    let mut nonce = [0u8; 8];
    let _ = getrandom::fill(&mut nonce);
    let staging = dir.join(STAGING);
    let built = staging.join(format!(
        "{}-{:016x}",
        manifest.id,
        u64::from_le_bytes(nonce)
    ));
    let mut room = Room {
        files: MAX_FILES,
        bytes: MAX_BYTES,
    };
    let copied = copy_tree(folder, &built, 0, &mut room).and_then(|()| {
        let old = staging.join(format!("{}-old", manifest.id));
        let _ = fs::remove_dir_all(&old);
        if place.exists() {
            fs::rename(&place, &old)?;
        }
        fs::rename(&built, &place)?;
        Ok(())
    });
    let _ = fs::remove_dir_all(&staging);
    copied?;
    Ok(Installed {
        manifest,
        path: place,
        broken: false,
    })
}

/// Remove the extension `id` and its folder.
pub fn remove(id: &str) -> Result<(), ExtensionError> {
    let dir = dir().ok_or(ExtensionError::NoFolder)?;
    let place = dir.join(id);
    if !is_id(id) || !place.join(MANIFEST_FILE).is_file() {
        return Err(ExtensionError::Missing);
    }
    fs::remove_dir_all(place)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
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
        });
        change(&mut value);
        value.to_string()
    }

    /// An extension can ask only for what this Lumit knows how to refuse,
    /// and can name only a page inside its own folder and sites by name.
    #[test]
    fn a_manifest_that_reaches_past_its_folder_is_refused() {
        assert!(parse(&manifest(|_| {})).is_ok());
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
    }

    /// Installing copies the folder whole, an install over another replaces
    /// it whole, and a removed extension leaves nothing.
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
        fs::write(source.join("ui/old.js"), "old").unwrap();
        let installed = install(&source).unwrap();
        assert_eq!(installed.path, store.join("timer"));
        assert_eq!(scan(), [installed]);

        fs::remove_file(source.join("ui/old.js")).unwrap();
        fs::write(source.join("ui/index.html"), "two").unwrap();
        install(&source).unwrap();
        let page = store.join("timer/ui/index.html");
        assert_eq!(fs::read_to_string(page).unwrap(), "two");
        assert!(!store.join("timer/ui/old.js").exists());

        remove("timer").unwrap();
        assert!(scan().is_empty());
        assert!(matches!(remove("timer"), Err(ExtensionError::Missing)));
        with_dir(None);
        let _ = fs::remove_dir_all(&root);
    }
}

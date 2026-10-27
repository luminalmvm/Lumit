//! Packages: one `.lfx` file holding an extension's folder, zipped, with its
//! manifest at the root. Making one judges the folder first, and reading one
//! applies the same caps as a folder and refuses any file that would land
//! outside it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{
    check_files, dir, inspect, invalid, parse, put_in_place, ExtensionError, Installed, Manifest,
    MANIFEST_FILE, MAX_BYTES, MAX_DEPTH, MAX_FILES, MAX_MANIFEST, SLOT,
};

/// A package's file extension.
pub const PACKAGE_EXTENSION: &str = "lfx";

fn not_a_package() -> ExtensionError {
    invalid("this file is not an extension package")
}

/// A name inside a package as a plain path below its root, with forward
/// slashes, or None for one that is absolute, climbs out with `..`, is too
/// deep, or that Windows can't write: a device name such as `con.txt`, or
/// one ending in a dot or a space.
fn plain(name: &str) -> Option<String> {
    let name = name.strip_suffix('/').unwrap_or(name);
    let parts: Vec<&str> = name.split('/').collect();
    let device = |p: &str| {
        let stem = p.split('.').next().unwrap_or(p).to_ascii_uppercase();
        let numbered = |prefix: &str| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        };
        ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str()) || numbered("COM") || numbered("LPT")
    };
    let part = |p: &&str| {
        !p.is_empty()
            && *p != "."
            && *p != ".."
            && !p.contains(['\\', ':', '\0'])
            && !p.ends_with(['.', ' '])
            && !device(p)
    };
    (parts.len() <= MAX_DEPTH + 1 && parts.iter().all(part)).then(|| parts.join("/"))
}

/// A path inside an extension as a package names it.
fn slashed(path: &Path) -> String {
    let parts: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}

/// A package opened and judged: its files by name, as (index, size), and
/// its manifest.
struct Package {
    zip: ZipArchive<File>,
    files: BTreeMap<String, (usize, u64)>,
    manifest: Manifest,
}

fn open(path: &Path) -> Result<Package, ExtensionError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_BYTES {
        return Err(ExtensionError::TooLarge);
    }
    let mut zip = ZipArchive::new(file).map_err(|_| not_a_package())?;
    if zip.len() > MAX_FILES {
        return Err(ExtensionError::TooLarge);
    }
    let mut files = BTreeMap::new();
    // Names as a case-blind disk sees them, so two can't land on one file.
    let mut folded = BTreeSet::new();
    let mut bytes = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|_| not_a_package())?;
        if entry.is_symlink() {
            return Err(invalid("a package cannot hold links"));
        }
        let name = entry.name().map_err(|_| not_a_package())?;
        let Some(name) = plain(&name) else {
            return Err(invalid("the package has a file outside its own folder"));
        };
        if entry.is_dir() {
            continue;
        }
        bytes = bytes.saturating_add(entry.size());
        if bytes > MAX_BYTES {
            return Err(ExtensionError::TooLarge);
        }
        if !folded.insert(name.to_lowercase()) {
            return Err(invalid("the package has two files of one name"));
        }
        files.insert(name, (index, entry.size()));
    }
    let &(index, size) = files
        .get(MANIFEST_FILE)
        .ok_or(ExtensionError::NotAnExtension)?;
    if size > MAX_MANIFEST {
        return Err(invalid("extension.json is too long"));
    }
    let mut text = String::new();
    zip.by_index(index)
        .map_err(|_| not_a_package())?
        .take(MAX_MANIFEST)
        .read_to_string(&mut text)
        .map_err(|_| invalid("extension.json could not be read"))?;
    let manifest = parse(&text)?;
    check_files(
        &manifest,
        &|path| files.get(&slashed(path)).map(|&(_, size)| size),
        // A package holds files only, so a folder is there when one is in it.
        &|path| {
            let under = format!("{}/", slashed(path));
            files.keys().any(|name| name.starts_with(&under))
        },
    )?;
    Ok(Package {
        zip,
        files,
        manifest,
    })
}

/// What the package at `path` holds, judged, for the person to read before
/// they agree to it. Nothing is unpacked.
///
/// # Errors
/// A file that is not a package, or one that breaks a rule.
pub fn read_package(path: &Path) -> Result<Manifest, ExtensionError> {
    open(path).map(|package| package.manifest)
}

/// Install the package at `path` into `into`, or Lumit's own folder, over
/// one of the same id there. Unpacked to one side and moved into place
/// whole, as a folder's install is.
///
/// # Errors
/// What [`read_package`] refuses, another install running, or files that
/// would not write.
pub fn install_package(path: &Path, into: Option<&Path>) -> Result<Installed, ExtensionError> {
    let Package {
        mut zip,
        files,
        manifest,
    } = open(path)?;
    let _slot = SLOT.try_lock().map_err(|_| ExtensionError::Busy)?;
    let into = match into {
        Some(into) => into.to_path_buf(),
        None => dir().ok_or(ExtensionError::NoFolder)?,
    };
    let place = put_in_place(&into, &manifest.id, |built| {
        for (name, &(index, size)) in &files {
            let target = built.join(name);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let entry = zip.by_index(index).map_err(|_| not_a_package())?;
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            // No more than it said it holds, whatever it really holds.
            if io::copy(&mut entry.take(size), &mut out)? != size {
                return Err(invalid("the package is damaged"));
            }
        }
        Ok(())
    })?;
    Ok(Installed {
        manifest,
        path: place,
        broken: false,
    })
}

/// The files under `folder` a package takes, by their names in it. Links
/// and anything whose name starts with a dot (`.git`, `.DS_Store`) stay out.
fn gather(
    folder: &Path,
    under: &str,
    depth: usize,
    into: &mut Vec<String>,
    bytes: &mut u64,
) -> Result<(), ExtensionError> {
    if depth > MAX_DEPTH {
        return Err(ExtensionError::TooLarge);
    }
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(invalid("a file's name in the folder is not readable text"));
        };
        if name.starts_with('.') {
            continue;
        }
        let path = if under.is_empty() {
            name
        } else {
            format!("{under}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            gather(&entry.path(), &path, depth + 1, into, bytes)?;
        } else if kind.is_file() {
            *bytes = bytes.saturating_add(entry.metadata()?.len());
            into.push(path);
            if into.len() > MAX_FILES || *bytes > MAX_BYTES {
                return Err(ExtensionError::TooLarge);
            }
        }
    }
    Ok(())
}

fn write_zip(folder: &Path, files: &[String], out: &Path) -> Result<(), ExtensionError> {
    let failed = |e: zip::result::ZipError| ExtensionError::Io(io::Error::other(e));
    let mut zip = ZipWriter::new(io::BufWriter::new(File::create(out)?));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for name in files {
        zip.start_file(name.as_str(), options).map_err(failed)?;
        io::copy(&mut File::open(folder.join(name))?, &mut zip)?;
    }
    zip.finish().map_err(failed)?.flush()?;
    Ok(())
}

/// Make a package of the extension in `folder` at `out`, after judging the
/// folder as an install would and then reading the package back the way an
/// install will, so a bad one is caught by its author.
///
/// # Errors
/// What [`inspect`] or [`read_package`] refuses, or a file that would not
/// write. Nothing is left at `out` then.
pub fn pack(folder: &Path, out: &Path) -> Result<Manifest, ExtensionError> {
    let manifest = inspect(folder)?;
    let mut files = Vec::new();
    gather(folder, "", 0, &mut files, &mut 0)?;
    let mut part = out.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let made = write_zip(folder, &files, &part)
        .and_then(|()| read_package(&part))
        .and_then(|_| fs::rename(&part, out).map_err(ExtensionError::from));
    if made.is_err() {
        let _ = fs::remove_file(&part);
    }
    made.map(|()| manifest)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn package(path: &Path, names: &[String]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default();
        for name in names {
            zip.start_file(name.as_str(), options).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
    }

    /// A package someone else made is untrusted: a file that would land
    /// outside its folder or on another file, one Windows can't write, or
    /// more files than any extension holds, is refused before anything is
    /// written. Guards untrusted input.
    #[test]
    fn a_hostile_package_is_refused() {
        let root = std::env::temp_dir().join(format!("lumit-lfx-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = root.join("store");
        fs::create_dir_all(&store).unwrap();
        let manifest =
            r#"{"format":1,"id":"tape","name":"Tape","version":"1.0.0","entry":"index.html"}"#;

        let hostile = |file: &str, name: &str| {
            let path = root.join(file);
            let mut zip = ZipWriter::new(File::create(&path).unwrap());
            zip.start_file(MANIFEST_FILE, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(manifest.as_bytes()).unwrap();
            for name in ["index.html", name] {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
            path
        };
        let climbs = hostile("climbs.lfx", "../outside.txt");
        let clashes = hostile("clashes.lfx", "Index.html");
        let device = hostile("device.lfx", "ui/con.txt");

        let crowded = root.join("crowded.lfx");
        let names: Vec<String> = (0..=MAX_FILES).map(|i| format!("f{i}")).collect();
        package(&crowded, &names);

        assert!(matches!(
            read_package(&climbs),
            Err(ExtensionError::Invalid(_))
        ));
        for refused in [&climbs, &clashes, &device] {
            assert!(matches!(
                install_package(refused, Some(&store)),
                Err(ExtensionError::Invalid(_))
            ));
        }
        assert!(matches!(
            install_package(&crowded, Some(&store)),
            Err(ExtensionError::TooLarge)
        ));
        assert!(!root.join("outside.txt").exists());
        assert!(!store.join("outside.txt").exists());
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&root);
    }
}

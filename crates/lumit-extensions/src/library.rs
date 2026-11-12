//! Library packages: things from the user's library zipped into one
//! `.lumlibrary` file, to carry them to another computer by hand. Each item
//! is stored as `<kind>/<name>`, a folder item with everything in it, and a
//! small `library.json` at the root lists the items. A package is held to an
//! extension package's caps, links are refused, and any file that would land
//! outside the folder it is unpacked into is refused.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::package::{gather, plain};
use crate::{invalid, ExtensionError, MAX_BYTES, MAX_DEPTH, MAX_FILES, MAX_MANIFEST};

/// A library package's file extension.
pub const LIBRARY_EXTENSION: &str = "lumlibrary";

/// The list at a package's root.
const LIST_FILE: &str = "library.json";

/// The library's kinds, by id. An item of any other kind is skipped.
const KINDS: [&str; 10] = [
    "presets",
    "styles",
    "themes",
    "fonts",
    "shortcuts",
    "lenses",
    "flares",
    "exports",
    "luts",
    "templates",
];

/// One thing in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItem {
    /// The kind's id, such as `presets`.
    pub kind: String,
    /// The file or folder on this computer.
    pub path: PathBuf,
    /// Where it sits inside its kind's folder, `/` separated.
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct List {
    format: String,
    version: u32,
    items: Vec<Listed>,
}

#[derive(Serialize, Deserialize)]
struct Listed {
    kind: String,
    name: String,
}

const FORMAT: &str = "lumit-library";

/// Pack `items` into one package at `out`. Written beside `out` as `.part`
/// and moved into place, so a failure leaves nothing. Answers how many items
/// went in.
///
/// # Errors
/// An unknown kind or a name that isn't a plain path, a link, more than the
/// caps, or a file that would not read or write.
pub fn pack(items: &[LibraryItem], out: &Path) -> Result<u32, ExtensionError> {
    // (name in the package, the file it is read from), and the folders.
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    let mut bytes = 0u64;
    let mut listed = Vec::new();
    for item in items {
        if !KINDS.contains(&item.kind.as_str()) {
            return Err(invalid("that is not a kind the library keeps"));
        }
        let name = plain(&item.name).ok_or_else(|| invalid("that name leaves its folder"))?;
        let inside = format!("{}/{name}", item.kind);
        let kind = fs::symlink_metadata(&item.path)?.file_type();
        if kind.is_dir() {
            let mut found = Vec::new();
            gather(
                &item.path,
                "",
                inside.split('/').count(),
                &mut found,
                &mut bytes,
            )?;
            folders.push(format!("{inside}/"));
            files.extend(
                found
                    .into_iter()
                    .map(|rel| (format!("{inside}/{rel}"), item.path.join(rel))),
            );
        } else if kind.is_file() {
            bytes = bytes.saturating_add(fs::metadata(&item.path)?.len());
            files.push((inside, item.path.clone()));
        } else {
            return Err(invalid("a package cannot hold links"));
        }
        if files.len() > MAX_FILES || bytes > MAX_BYTES {
            return Err(ExtensionError::TooLarge);
        }
        listed.push(Listed {
            kind: item.kind.clone(),
            name,
        });
    }
    let count = u32::try_from(listed.len()).map_err(|_| ExtensionError::TooLarge)?;
    let list = serde_json::to_vec(&List {
        format: FORMAT.to_owned(),
        version: 1,
        items: listed,
    })
    .map_err(|e| ExtensionError::Io(io::Error::other(e)))?;

    let mut part = out.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let made = write(&part, &list, &folders, &files).and_then(|()| Ok(fs::rename(&part, out)?));
    if made.is_err() {
        let _ = fs::remove_file(&part);
    }
    made.map(|()| count)
}

fn write(
    out: &Path,
    list: &[u8],
    folders: &[String],
    files: &[(String, PathBuf)],
) -> Result<(), ExtensionError> {
    let failed = |e: zip::result::ZipError| ExtensionError::Io(io::Error::other(e));
    let mut zip = ZipWriter::new(io::BufWriter::new(File::create(out)?));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(LIST_FILE, options).map_err(failed)?;
    zip.write_all(list)?;
    // An empty folder is an item too.
    for folder in folders {
        zip.add_directory(folder.as_str(), options)
            .map_err(failed)?;
    }
    for (name, source) in files {
        zip.start_file(name.as_str(), options).map_err(failed)?;
        io::copy(&mut File::open(source)?, &mut zip)?;
    }
    zip.finish().map_err(failed)?.flush()?;
    Ok(())
}

/// Unpack the package at `file` into the folder `into`, made if missing and
/// empty if not, as `<kind>/<name>`. Every entry is judged before anything is
/// written, and a failure part way clears what was written. Items of a kind
/// this version doesn't know are skipped. Answers the items, with `path`
/// where each landed.
///
/// # Errors
/// A file that is not a library package, a link or a name that would land
/// outside `into`, more than the caps, a folder that isn't empty, or files
/// that would not write.
pub fn unpack(file: &Path, into: &Path) -> Result<Vec<LibraryItem>, ExtensionError> {
    let not_a_library = || invalid("this file is not a library package");
    let archive = File::open(file)?;
    if archive.metadata()?.len() > MAX_BYTES {
        return Err(ExtensionError::TooLarge);
    }
    let mut zip = ZipArchive::new(archive).map_err(|_| not_a_library())?;
    if zip.len() > MAX_FILES {
        return Err(ExtensionError::TooLarge);
    }
    // (index, name, size) of every file, and the folders.
    let mut entries: Vec<(usize, String, u64)> = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    let mut folded = BTreeSet::new();
    let mut bytes = 0u64;
    let mut list_at = None;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|_| not_a_library())?;
        if entry.is_symlink() {
            return Err(invalid("a package cannot hold links"));
        }
        let name = entry.name().map_err(|_| not_a_library())?;
        let name = plain(&name.replace('\\', "/"))
            .ok_or_else(|| invalid("the package has a file outside its own folder"))?;
        if entry.is_dir() {
            folders.push(name);
            continue;
        }
        bytes = bytes.saturating_add(entry.size());
        if bytes > MAX_BYTES {
            return Err(ExtensionError::TooLarge);
        }
        // Names as a case-blind disk sees them, so two can't land on one file.
        if !folded.insert(name.to_lowercase()) {
            return Err(invalid("the package has two files of one name"));
        }
        if name == LIST_FILE {
            list_at = Some((index, entry.size()));
        } else {
            entries.push((index, name, entry.size()));
        }
    }
    let (index, size) = list_at.ok_or_else(not_a_library)?;
    if size > MAX_MANIFEST {
        return Err(not_a_library());
    }
    let mut text = String::new();
    zip.by_index(index)
        .map_err(|_| not_a_library())?
        .take(MAX_MANIFEST)
        .read_to_string(&mut text)
        .map_err(|_| not_a_library())?;
    let list: List = serde_json::from_str(&text).map_err(|_| not_a_library())?;
    if list.format != FORMAT || list.version != 1 {
        return Err(not_a_library());
    }
    let mut items = Vec::new();
    for listed in list.items {
        if !KINDS.contains(&listed.kind.as_str()) {
            continue;
        }
        let name = plain(&listed.name)
            .filter(|n| n.split('/').count() < MAX_DEPTH)
            .ok_or_else(|| invalid("the package has a file outside its own folder"))?;
        items.push(LibraryItem {
            path: into.join(&listed.kind).join(&name),
            kind: listed.kind,
            name,
        });
    }
    // Only what belongs to a listed item is written.
    let owned = |name: &str| {
        items.iter().any(|item| {
            let inside = format!("{}/{}", item.kind, item.name);
            name == inside || name.starts_with(&format!("{inside}/"))
        })
    };

    folders.retain(|f| owned(f));
    entries.retain(|(_, n, _)| owned(n));
    fs::create_dir_all(into)?;
    if fs::read_dir(into)?.next().is_some() {
        return Err(invalid("the folder to unpack into isn't empty"));
    }
    if let Err(e) = put(&mut zip, into, &folders, &entries) {
        for entry in fs::read_dir(into).into_iter().flatten().flatten() {
            let _ = fs::remove_dir_all(entry.path()).or_else(|_| fs::remove_file(entry.path()));
        }
        return Err(e);
    }
    Ok(items)
}

/// Write the judged `folders` and files `entries` of `zip` into `into`.
fn put(
    zip: &mut ZipArchive<File>,
    into: &Path,
    folders: &[String],
    entries: &[(usize, String, u64)],
) -> Result<(), ExtensionError> {
    for folder in folders {
        fs::create_dir_all(into.join(folder))?;
    }
    for (index, name, size) in entries {
        let target = into.join(name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let entry = zip
            .by_index(*index)
            .map_err(|_| invalid("this file is not a library package"))?;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        // No more than it said it holds, whatever it really holds.
        if io::copy(&mut entry.take(*size), &mut out)? != *size {
            return Err(invalid("the package is damaged"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **A package unpacks to the bytes that went in, and a hostile one
    /// writes nothing.** A preset in a group, a style folder and an empty
    /// group go in and come out the same. A package naming `../` is refused
    /// and leaves nothing outside the folder or in it.
    #[test]
    fn a_package_round_trips_and_a_hostile_one_writes_nothing() {
        let root = std::env::temp_dir().join(format!("lumit-library-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.as_path();
        let home = dir.join("home");
        fs::create_dir_all(home.join("presets/Looks")).unwrap();
        fs::create_dir_all(home.join("presets/Empty")).unwrap();
        fs::create_dir_all(home.join("styles/Soft glass/fonts")).unwrap();
        fs::write(home.join("presets/Looks/Teal.lumfx"), b"{\"teal\":1}").unwrap();
        fs::write(
            home.join("styles/Soft glass/Soft glass.lumstyle"),
            b"@base;",
        )
        .unwrap();
        fs::write(
            home.join("styles/Soft glass/fonts/Face.ttf"),
            [0u8, 1, 2, 255],
        )
        .unwrap();
        let item = |kind: &str, name: &str| LibraryItem {
            kind: kind.into(),
            path: home.join(kind).join(name),
            name: name.into(),
        };
        let items = [
            item("presets", "Looks/Teal.lumfx"),
            item("presets", "Empty"),
            item("styles", "Soft glass"),
        ];
        let package = dir.join("mine.lumlibrary");
        assert_eq!(pack(&items, &package).unwrap(), 3);

        let into = dir.join("in");
        let back = unpack(&package, &into).unwrap();
        assert_eq!(back.len(), 3);
        for (went, came) in items.iter().zip(&back) {
            assert_eq!((&went.kind, &went.name), (&came.kind, &came.name));
        }
        for file in [
            "presets/Looks/Teal.lumfx",
            "styles/Soft glass/Soft glass.lumstyle",
            "styles/Soft glass/fonts/Face.ttf",
        ] {
            assert_eq!(
                fs::read(into.join(file)).unwrap(),
                fs::read(home.join(file)).unwrap()
            );
        }
        assert!(into.join("presets/Empty").is_dir());

        let hostile = dir.join("bad.lumlibrary");
        let mut zip = ZipWriter::new(File::create(&hostile).unwrap());
        let options = SimpleFileOptions::default();
        zip.start_file(LIST_FILE, options).unwrap();
        zip.write_all(
            br#"{"format":"lumit-library","version":1,"items":[{"kind":"presets","name":"a"}]}"#,
        )
        .unwrap();
        zip.start_file("presets/a", options).unwrap();
        zip.write_all(b"fine").unwrap();
        zip.start_file("presets/../../escape.txt", options).unwrap();
        zip.write_all(b"out").unwrap();
        zip.finish().unwrap();
        let into = dir.join("hostile");
        assert!(unpack(&hostile, &into).is_err());
        assert!(!dir.join("escape.txt").exists());
        assert!(!into.exists() || fs::read_dir(&into).unwrap().next().is_none());
        let _ = fs::remove_dir_all(&root);
    }
}

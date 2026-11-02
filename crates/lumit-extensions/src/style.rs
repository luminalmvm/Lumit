//! Style packages: a style's folder zipped into one `.lumstyle` file, to share
//! it with its pictures, icons and fonts. The style is the one `.lumstyle` at
//! the zip's root, or in the one folder there, as zipping a folder makes it.
//! A package is held to an extension package's caps, and any file that would
//! land outside its own folder is refused.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use zip::ZipArchive;

use crate::package::{gather, plain, write_zip};
use crate::{put_in_place, ExtensionError, MAX_BYTES, MAX_FILES};

/// A style file's extension, and a package's.
pub const STYLE_EXTENSION: &str = "lumstyle";

#[derive(Debug, thiserror::Error)]
pub enum StyleError {
    #[error("this file is not a style package")]
    NotAStyle,
    #[error("a style package holds at most {MAX_FILES} files and 512 MB")]
    TooLarge,
    #[error("the package has a link, or a file outside its own folder")]
    Unsafe,
    #[error("the style's files could not be written: {0}")]
    Io(#[from] io::Error),
}

impl From<ExtensionError> for StyleError {
    fn from(e: ExtensionError) -> Self {
        match e {
            ExtensionError::TooLarge => Self::TooLarge,
            ExtensionError::Io(e) => Self::Io(e),
            _ => Self::NotAStyle,
        }
    }
}

/// A package opened and judged: its files as (index, name inside the
/// style's folder, size), and the style's name.
struct Package {
    zip: ZipArchive<File>,
    files: Vec<(usize, String, u64)>,
    name: String,
}

fn is_style(name: &str) -> bool {
    !name.contains('/')
        && name
            .to_ascii_lowercase()
            .ends_with(&format!(".{STYLE_EXTENSION}"))
}

fn open(path: &Path) -> Result<Package, StyleError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_BYTES {
        return Err(StyleError::TooLarge);
    }
    let mut zip = ZipArchive::new(file).map_err(|_| StyleError::NotAStyle)?;
    if zip.len() > MAX_FILES {
        return Err(StyleError::TooLarge);
    }
    let mut files = Vec::new();
    let mut bytes = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|_| StyleError::NotAStyle)?;
        if entry.is_symlink() {
            return Err(StyleError::Unsafe);
        }
        let name = entry.name().map_err(|_| StyleError::NotAStyle)?;
        // Windows PowerShell's Compress-Archive writes backslashes.
        let name = plain(&name.replace('\\', "/")).ok_or(StyleError::Unsafe)?;
        // A folder's hidden files, and what a Mac's Compress puts beside it.
        let hidden = name
            .split('/')
            .any(|part| part.starts_with('.') || part == "__MACOSX");
        if entry.is_dir() || hidden {
            continue;
        }
        bytes = bytes.saturating_add(entry.size());
        if bytes > MAX_BYTES {
            return Err(StyleError::TooLarge);
        }
        files.push((index, name, entry.size()));
    }
    // Zipping a folder puts everything inside one folder of its name.
    if !files.iter().any(|(_, name, _)| is_style(name)) {
        let top = files
            .first()
            .and_then(|(_, name, _)| name.split_once('/'))
            .map(|(top, _)| format!("{top}/"))
            .ok_or(StyleError::NotAStyle)?;
        if !files.iter().all(|(_, name, _)| name.starts_with(&top)) {
            return Err(StyleError::NotAStyle);
        }
        for (_, name, _) in &mut files {
            name.drain(..top.len());
        }
    }
    let mut styles = files.iter().filter(|(_, name, _)| is_style(name));
    let (Some((_, style, _)), None) = (styles.next(), styles.next()) else {
        return Err(StyleError::NotAStyle);
    };
    let name = style[..style.len() - STYLE_EXTENSION.len() - 1].to_owned();
    Ok(Package { zip, files, name })
}

/// The name of the style in the package at `path`, judged as an install
/// would judge it. Nothing is unpacked.
///
/// # Errors
/// A file that is not a style package, or one that breaks a rule.
pub fn read_style_package(path: &Path) -> Result<String, StyleError> {
    open(path).map(|package| package.name)
}

/// Unpack the style package at `path` into a folder of the style's name in
/// `styles`, over a style of the same name there. Unpacked to one side and
/// moved into place whole, so a failure leaves what was there. Answers the
/// style's name and its folder.
///
/// # Errors
/// What [`read_style_package`] refuses, or files that would not write.
pub fn install_style_package(path: &Path, styles: &Path) -> Result<(String, PathBuf), StyleError> {
    // One at a time: two installs would share the staging folder.
    static SLOT: Mutex<()> = Mutex::new(());
    let _slot = SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Package {
        mut zip,
        files,
        name,
    } = open(path)?;
    fs::create_dir_all(styles)?;
    let place = put_in_place(styles, &name, |built| {
        for (index, inside, size) in &files {
            let target = built.join(inside);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let entry = zip
                .by_index(*index)
                .map_err(|_| ExtensionError::Invalid(String::new()))?;
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            // No more than it said it holds, whatever it really holds.
            if io::copy(&mut io::Read::take(entry, *size), &mut out)? != *size {
                return Err(ExtensionError::Invalid(String::new()));
            }
        }
        Ok(())
    })?;
    Ok((name, place))
}

/// Make a package of the style file at `style` at `out`: the file alone, or
/// with `whole_folder` everything in the folder it sits in, as the folder.
/// Read back as an install would, so a bad one is caught by its author.
///
/// # Errors
/// A folder over the caps, or a file that would not write. Nothing is left
/// at `out` then.
pub fn pack_style(style: &Path, whole_folder: bool, out: &Path) -> Result<String, StyleError> {
    let folder = style.parent().ok_or(StyleError::NotAStyle)?;
    let own = style
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StyleError::NotAStyle)?;
    if fs::canonicalize(out).ok() == fs::canonicalize(style).ok() {
        return Err(StyleError::Io(io::Error::other(
            "a package can't be saved over its own style",
        )));
    }
    let files = if whole_folder {
        let mut files = Vec::new();
        gather(folder, "", 0, &mut files, &mut 0)?;
        // Packages saved beside it before, and any other style there.
        files.retain(|name| !is_style(name) || name == own);
        files
    } else {
        vec![own.to_owned()]
    };
    let mut part = out.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let made = write_zip(folder, &files, &part)
        .map_err(StyleError::from)
        .and_then(|()| read_style_package(&part))
        .and_then(|name| {
            fs::rename(&part, out)
                .map(|()| name)
                .map_err(StyleError::from)
        });
    if made.is_err() {
        let _ = fs::remove_file(&part);
    }
    made
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn zip_of(path: &Path, files: &[(&str, &str)]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, text) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    // A zipped folder unpacks into the styles folder under the style's own
    // name, Mac leftovers dropped, and one that climbs out is refused.
    #[test]
    fn a_zipped_style_folder_installs_and_a_hostile_one_does_not() {
        let dir = std::env::temp_dir().join(format!("lumit-style-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let package = dir.join("Soft glass.zip");
        zip_of(
            &package,
            &[
                ("Glass/Soft glass.lumstyle", "@base lantern;"),
                ("Glass/fonts/Face.ttf", "font"),
                ("__MACOSX/Glass/._Soft glass.lumstyle", "junk"),
            ],
        );
        let styles = dir.join("styles");
        let (name, place) = install_style_package(&package, &styles).unwrap();
        assert_eq!(name, "Soft glass");
        assert_eq!(
            fs::read_to_string(place.join("Soft glass.lumstyle")).unwrap(),
            "@base lantern;"
        );
        assert!(place.join("fonts/Face.ttf").is_file());
        assert!(!styles.join("__MACOSX").exists());

        let hostile = dir.join("bad.lumstyle");
        zip_of(&hostile, &[("bad.lumstyle", ""), ("../escape.txt", "x")]);
        assert!(matches!(
            install_style_package(&hostile, &styles),
            Err(StyleError::Unsafe)
        ));
        assert!(!dir.join("escape.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

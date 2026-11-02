//! The interface's door to the files the engine reads and writes.
//!
//! The browser build has no file system, so its files are kept in memory in
//! `lumit_ingress::files`, and the page's `File` and `Directory` come here for
//! them. That way a file picked in the page is one the engine can open, and a
//! project the engine saved is one the page can hand over. On a desktop
//! nothing calls these, and each would be the system's own call if it did.

use std::path::Path;

use flutter_rust_bridge::frb;
use lumit_ingress::files;

/// What [`web_file_stat`] answers for something that is there.
pub struct WebFileStat {
    pub folder: bool,
    pub size: u32,
    /// When it was last written, in milliseconds since 1970.
    pub changed_ms: f64,
}

/// A file's bytes, or nothing when there is no file there.
#[frb(sync)]
pub fn web_file_read(path: String) -> Option<Vec<u8>> {
    files::read(Path::new(&path)).ok()
}

/// Write a file, making the folders above it.
#[frb(sync)]
pub fn web_file_write(path: String, bytes: Vec<u8>) -> bool {
    let path = Path::new(&path);
    path.parent()
        .is_none_or(|dir| files::create_dir_all(dir).is_ok())
        && files::write(path, &bytes).is_ok()
}

/// Delete a file, or a folder and everything in it.
#[frb(sync)]
pub fn web_file_remove(path: String) {
    let path = Path::new(&path);
    let _ = if files::is_dir(path) {
        files::remove_dir_all(path)
    } else {
        files::remove_file(path)
    };
}

/// Make a folder, and the folders above it.
#[frb(sync)]
pub fn web_file_make_folder(path: String) -> bool {
    files::create_dir_all(Path::new(&path)).is_ok()
}

/// Whether there is a file or a folder at `path`, and what it is.
#[frb(sync)]
pub fn web_file_stat(path: String) -> Option<WebFileStat> {
    let path = Path::new(&path);
    if files::is_dir(path) {
        return Some(WebFileStat {
            folder: true,
            size: 0,
            changed_ms: 0.0,
        });
    }
    let changed = files::modified(path).ok()?;
    let since = changed
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Some(WebFileStat {
        folder: false,
        size: u32::try_from(files::len(path).ok()?).unwrap_or(u32::MAX),
        changed_ms: since.as_secs_f64() * 1000.0,
    })
}

/// The names in a folder, each folder's with a slash at the end.
#[frb(sync)]
pub fn web_file_list(path: String) -> Vec<String> {
    let listed = files::list(Path::new(&path)).unwrap_or_default();
    let named = |(name, folder): (String, bool)| if folder { format!("{name}/") } else { name };
    listed.into_iter().map(named).collect()
}

//! Lumit's own folders on this machine, which every path in this crate is
//! built from.

use std::path::{Path, PathBuf};

/// The three folders Lumit keeps things in.
pub(crate) struct Dirs {
    cache: PathBuf,
    data: PathBuf,
    data_local: PathBuf,
}

impl Dirs {
    pub(crate) fn cache_dir(&self) -> &Path {
        &self.cache
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data
    }

    pub(crate) fn data_local_dir(&self) -> &Path {
        &self.data_local
    }
}

/// The platform's folders for Lumit.
///
/// `LUMIT_USER_DIRS` moves all of them under one folder. `cargo test` sets it
/// in `.cargo/config.toml`, so tests never write into the real ones.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn project_dirs() -> Option<Dirs> {
    // This crate's own tests check the real layout and write nothing there.
    let moved = std::env::var_os("LUMIT_USER_DIRS")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute() && !cfg!(test));
    let dirs = match moved {
        // Joining an absolute path replaces the platform's base folder.
        Some(root) => directories::ProjectDirs::from_path(root),
        None => directories::ProjectDirs::from("dev", "Lumit", "Lumit"),
    }?;
    Some(Dirs {
        cache: dirs.cache_dir().to_path_buf(),
        data: dirs.data_dir().to_path_buf(),
        data_local: dirs.data_local_dir().to_path_buf(),
    })
}

/// The browser build's, in the page's own files, laid out as on Linux under
/// the `HOME` the front end gives the page.
#[cfg(target_arch = "wasm32")]
pub(crate) fn project_dirs() -> Option<Dirs> {
    Some(Dirs {
        cache: PathBuf::from("/home/.cache/lumit"),
        data: PathBuf::from("/home/.local/share/lumit"),
        data_local: PathBuf::from("/home/.local/share/lumit"),
    })
}

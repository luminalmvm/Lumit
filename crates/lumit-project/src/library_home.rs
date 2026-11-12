//! Where the user's library lives: presets, lenses, flares, templates, fonts,
//! LUTs and the export presets, each in a folder (or file) of its own name.
//!
//! The first profile on a machine keeps them in the platform's roaming
//! app-data area, beside the config. Every other profile has a library home of
//! its own, set on start and whenever the profile changes, and every library
//! path follows it. Nothing is moved or made on disk here: callers make a
//! folder when they first write one.

use std::path::PathBuf;
use std::sync::{PoisonError, RwLock};

use crate::dirs;

/// The library home the profile in force uses, when it isn't the first
/// profile's. Read for every library path, so held only to clone.
static LIBRARY_HOME: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Moves the library under `home`, or back to the roaming app-data area with
/// `None`.
pub fn set_library_home(home: Option<PathBuf>) {
    *LIBRARY_HOME.write().unwrap_or_else(PoisonError::into_inner) = home;
}

/// The library home set by [`set_library_home`], or `None` for the first
/// profile's, which is the roaming app-data area.
#[must_use]
pub fn library_home() -> Option<PathBuf> {
    LIBRARY_HOME
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// `name` inside the library home in force. `None` only when the platform
/// has no home directory.
#[must_use]
pub fn in_library(name: &str) -> Option<PathBuf> {
    match library_home() {
        Some(home) => Some(home.join(name)),
        None => Some(dirs::project_dirs()?.data_dir().join(name)),
    }
}

/// The fonts kept in the library, as `.ttf`, `.otf`, `.ttc` or `.otc` files,
/// top level or in sub-folders, drawn with beside the system's own.
#[must_use]
pub fn fonts_dir() -> Option<PathBuf> {
    in_library("fonts")
}

/// The user's own LUT files, such as `.cube`. A LUT or OCIO row whose file is
/// missing on this machine finds one of the same name here.
#[must_use]
pub fn luts_dir() -> Option<PathBuf> {
    in_library("luts")
}

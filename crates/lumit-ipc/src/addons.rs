//! The folder Lumit's own plugin installer writes into, and every host looks
//! in (docs/impl/lfx.md §5.2, §6.1).
//!
//! # In plain terms
//!
//! A plugin bought from a vendor lands in one of the standard folders that
//! vendor's installer knows about. A plugin Lumit installs itself has nowhere
//! like that to go - Lumit will not write into `C:\Program Files\Common
//! Files\OFX\Plugins`, and on Linux `/usr/lib/lfx` belongs to the distribution -
//! so it gets a directory of its own, and every host searches it.
//!
//! On Linux that directory is not a convenience. Inside a Flatpak `/usr` is the
//! GNOME runtime's own, so a system-installed plugin is invisible whatever the
//! standard paths say; the Lumit-owned folder redirects into
//! `~/.var/app/…`, which is writable, and is the only route a plugin has into a
//! sandboxed Lumit at all. Settings ▸ Addons lists the search paths read-only
//! so that a person can see this rather than deduce it.
//!
//! **`data_local_dir()`, not `data_dir()`, and that is the whole of the trap**
//! (§11 item 5). Every other Lumit directory uses `data_dir()`, which on
//! Windows is roaming `%APPDATA%`: presets are a few kilobytes of JSON and roam
//! deliberately. A folder of **native plugin binaries** would roam too, between
//! machines and across architectures, and the frame-cache's own comment already
//! warns what a roaming profile does with bytes at logoff. On macOS and Linux
//! the two resolve to the same place, so the difference is Windows-only and
//! silent everywhere it is not a bug.
//!
//! # Why it is here rather than beside the other directories
//!
//! docs/impl/lfx.md §6.1 spells it `lumit_project::addons_dir()` and asks for it
//! to be appended **inside each host's own `search_paths()`** - and the three
//! hosts deliberately depend on no project-format crate: each reads the
//! switched-off list it is handed rather than reading the preference file
//! itself. `lumit-ipc` is the crate all three already share for exactly this
//! class of answer - the handful of things every host must answer the *same
//! way* - so the directory is recorded here and the hosts reach it without
//! growing a dependency on the `.lum` container.

use std::path::PathBuf;

/// The folder's name under the platform's machine-local application data.
pub const ADDONS_DIR_NAME: &str = "addons";

/// The folder an install unpacks into before it lands, a **sibling** of
/// [`ADDONS_DIR_NAME`] rather than a directory inside it.
///
/// The sibling is the whole of the point (docs/impl/lfx.md §6.2 step 6, §11
/// item 17). Every host's walk descends into every directory it finds,
/// dot-prefixed ones included - it filters on the bundle suffix and the depth
/// and on nothing else - and the start-up scan fires at every launch. So an
/// `addons/.staging/<token>/Name.lfx.bundle` is discoverable while it is
/// half-written: a scan racing an install would open it, and an install killed
/// between the unpack and the rename would leave it discoverable for ever.
/// Staging outside the searched directory is what makes "never looks
/// installed" true; the rename alone does not.
///
/// Same volume as [`ADDONS_DIR_NAME`], so landing an installed bundle is still
/// one rename rather than a copy.
pub const STAGING_DIR_NAME: &str = "addons.staging";

/// Where a plugin Lumit installed itself lives.
///
/// `None` only when the platform has no home directory, in which case nothing
/// is installed and nothing is an error - the same answer every other Lumit
/// directory gives. The folder is **not** created here: a search path that does
/// not exist costs a failed `read_dir` and nothing else, and the one caller
/// that has to make it is the installer.
#[must_use]
pub fn addons_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_local_dir().join(ADDONS_DIR_NAME))
}

/// Where an install unpacks a pack before it lands.
///
/// Beside [`addons_dir`] rather than inside it, for the reason
/// [`STAGING_DIR_NAME`] gives, and on the same volume, so landing is a rename.
/// `None` on the same platform [`addons_dir`] answers `None` on, and for the
/// same reason.
#[must_use]
pub fn staging_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_local_dir().join(STAGING_DIR_NAME))
}

//! Which installed plugins the user has switched off (docs/12 §2.6).
//!
//! # In plain terms
//!
//! Lumit scans the machine's OFX folders at start-up and offers everything it
//! finds. Not everything found is wanted: a vendor's demo build that nags, a
//! plugin that crashes on this machine, forty effects from a suite the user
//! bought for one. So there is a list of the ones to leave alone, and it is a
//! **preference** — a fact about this person's machine, never about a project —
//! so it lives beside the export defaults in the application's own data area.
//!
//! It holds identifiers and nothing else. A plugin is named by the
//! reverse-domain identifier it declares (`net.sf.openfx.invertPlugin`), which
//! is the one thing about it that does not change when it is upgraded or moved
//! to another folder.
//!
//! Like every other preference file here, losing it costs a re-tick and never
//! an error: an absent or damaged file reads as "nothing has been switched
//! off", which is what Lumit did before the file existed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the list is kept. `None` only when the platform has no home
/// directory, in which case nothing is remembered and nothing is an error.
#[must_use]
pub fn plugin_prefs_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "Lumit", "Lumit")?;
    Some(dirs.data_dir().join("plugins.json"))
}

/// The plugins this user does not want offered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginPrefs {
    /// Plugin identifiers to skip, sorted — a set rather than a list so the
    /// file cannot grow a duplicate and the order cannot drift between saves.
    pub disabled: BTreeSet<String>,
}

impl PluginPrefs {
    /// Read the list from `path`. An absent or damaged file is "nothing has
    /// been switched off".
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default()
    }

    /// Read from [`plugin_prefs_path`], or an empty list when there is none.
    #[must_use]
    pub fn load_default() -> Self {
        plugin_prefs_path()
            .map(|path| Self::load(&path))
            .unwrap_or_default()
    }

    /// Whether `identifier` is switched off.
    #[must_use]
    pub fn is_disabled(&self, identifier: &str) -> bool {
        self.disabled.contains(identifier)
    }

    /// Switch a plugin on or off. `true` when this changed anything, so a
    /// caller can skip a write that would rewrite the same file.
    pub fn set_enabled(&mut self, identifier: &str, enabled: bool) -> bool {
        if enabled {
            self.disabled.remove(identifier)
        } else {
            self.disabled.insert(identifier.to_owned())
        }
    }

    /// Write the list to `path`, creating the directory.
    ///
    /// # Errors
    ///
    /// A sentence naming what went wrong: the directory could not be made, the
    /// list would not serialise, or the file would not be written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("plugin preferences: {e}"))?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| format!("plugin preferences: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("plugin preferences: {e}"))
    }

    /// Save to [`plugin_prefs_path`]; a platform with no home directory keeps
    /// the answer for the session and says so calmly.
    ///
    /// # Errors
    ///
    /// As [`PluginPrefs::save`], plus the machine having nowhere to keep it.
    pub fn save_default(&self) -> Result<(), String> {
        match plugin_prefs_path() {
            Some(path) => self.save(&path),
            None => Err("this machine has no place to keep plugin preferences".to_owned()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_disabled_list_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("plugins.json");

        let mut prefs = PluginPrefs::default();
        assert!(prefs.set_enabled("com.vendor.nagware", false));
        assert!(!prefs.set_enabled("com.vendor.nagware", false), "no change");
        prefs.save(&path).unwrap();

        let read = PluginPrefs::load(&path);
        assert_eq!(read, prefs);
        assert!(read.is_disabled("com.vendor.nagware"));
        assert!(!read.is_disabled("com.vendor.good"));
    }

    #[test]
    fn a_missing_or_damaged_file_switches_nothing_off() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            PluginPrefs::load(&dir.path().join("not-there.json")),
            PluginPrefs::default()
        );

        let damaged = dir.path().join("damaged.json");
        std::fs::write(&damaged, "{ not json").unwrap();
        assert_eq!(PluginPrefs::load(&damaged), PluginPrefs::default());
    }
}

//! What the export dialogue opens on, remembered between sessions (docs/07 §15).
//!
//! # In plain terms
//!
//! Most people export the same way most of the time: the same preset, into the
//! same folder, under names built the same way. Until now the dialogue forgot
//! all of that the moment it closed — it opened on the first built-in preset
//! with no destination at all, every time, in every session.
//!
//! This is the small file that remembers four answers:
//!
//! - **preset** — the named preset the dialogue starts from;
//! - **codec** — the output format, kept separately because a person may want
//!   their preset's settings in a different container;
//! - **filename template** — the pattern a suggested file name is built from,
//!   in the tokens the exporter already understands (`{comp}`, `{preset}`,
//!   `{date}`); blank means each preset's own suggested name;
//! - **destination** — whether to ask every time, write beside the project, or
//!   always write into one chosen folder.
//!
//! It is **not** part of a project. It is a preference about how this person
//! exports, so it sits beside the export-preset library in the application's
//! own data area and follows the user between projects.
//!
//! Like the preset library, the file is a convenience and never a correctness
//! matter: a missing or damaged one reads as "nothing has been said", which is
//! exactly the behaviour Lumit had before the file existed. It is written with
//! `serde(default)` and unknown fields ignored, so a file written by a newer
//! Lumit still opens here — the answers this version understands are kept and
//! the rest are simply not read.

use std::path::{Path, PathBuf};

/// Ask where to write, every time — the answer that cannot put a file
/// somewhere the user did not look at.
pub const DESTINATION_ASK: &str = "ask";
/// Write beside the project file. Falls back to asking when the project has
/// never been saved and so has no folder of its own.
pub const DESTINATION_PROJECT: &str = "project";
/// Always write into [`ExportDefaults::folder`].
pub const DESTINATION_FOLDER: &str = "folder";

/// Where the defaults are kept. `None` only when the platform has no home
/// directory, in which case nothing is remembered and nothing is an error.
pub fn default_path() -> Option<PathBuf> {
    lumit_project::export_defaults_path()
}

/// The four answers the export dialogue opens on.
///
/// Every field is a string, and every empty string means "nothing has been
/// said" — which is what makes [`Default`] the same thing as an absent file.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExportDefaults {
    /// A preset name from the library, or empty for "whatever the dialogue
    /// would have opened on".
    pub preset: String,
    /// The output format key (`h264`, `png`, …), or empty for the preset's own.
    pub codec: String,
    /// The filename pattern, in `{comp}`/`{preset}`/`{date}`. Empty gives each
    /// preset's own suggested name, byte for byte.
    pub filename_template: String,
    /// [`DESTINATION_ASK`], [`DESTINATION_PROJECT`] or [`DESTINATION_FOLDER`].
    /// Read through [`ExportDefaults::policy`], never directly: an answer a
    /// newer Lumit wrote must not send this one hunting for a folder it cannot
    /// name.
    pub destination: String,
    /// The folder [`DESTINATION_FOLDER`] means. Empty is the same as asking.
    pub folder: String,
}

impl ExportDefaults {
    /// The destination policy this file actually states. An unrecognised
    /// answer — a newer Lumit's, or a hand-edited file's — reads as
    /// [`DESTINATION_ASK`], which is the one answer that cannot write a file
    /// somewhere surprising.
    ///
    /// A folder policy with **no folder yet** is still a folder policy: that is
    /// the state Settings ▸ Export is in between choosing the option and
    /// choosing the folder, and the dialogue treats an empty folder as asking
    /// anyway.
    pub fn policy(&self) -> &'static str {
        match self.destination.as_str() {
            DESTINATION_PROJECT => DESTINATION_PROJECT,
            DESTINATION_FOLDER => DESTINATION_FOLDER,
            _ => DESTINATION_ASK,
        }
    }

    /// Read the defaults from `path`. An absent or damaged file is "nothing
    /// has been said", never an error: losing this must cost a re-save, not an
    /// export.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default()
    }

    /// Read from [`default_path`], or the built-in answers when there is none.
    pub fn load_default() -> Self {
        default_path().map(|p| Self::load(&p)).unwrap_or_default()
    }

    /// Write the defaults to `path`, creating the directory.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("export defaults: {e}"))?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| format!("export defaults: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("export defaults: {e}"))
    }

    /// Save to [`default_path`]; a platform with no home directory keeps the
    /// answers for the session and says so calmly.
    pub fn save_default(&self) -> Result<(), String> {
        match default_path() {
            Some(path) => self.save(&path),
            None => Err("this machine has no place to keep export defaults".to_owned()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn some_defaults() -> ExportDefaults {
        ExportDefaults {
            preset: "My delivery".to_owned(),
            codec: "hevc".to_owned(),
            filename_template: "{comp}-{date}".to_owned(),
            destination: DESTINATION_FOLDER.to_owned(),
            folder: "/deliveries".to_owned(),
        }
    }

    /// Every answer survives a save and a reload — the round trip the export
    /// dialogue's opening state depends on.
    #[test]
    fn the_defaults_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("export-defaults.json");

        some_defaults().save(&path).unwrap();

        assert_eq!(ExportDefaults::load(&path), some_defaults());
    }

    /// A missing or damaged file is "nothing has been said", never an error.
    #[test]
    fn a_missing_or_damaged_file_reads_as_nothing_said() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-there.json");
        assert_eq!(ExportDefaults::load(&missing), ExportDefaults::default());

        let damaged = dir.path().join("damaged.json");
        std::fs::write(&damaged, "{ this is not json").unwrap();
        assert_eq!(ExportDefaults::load(&damaged), ExportDefaults::default());
    }

    /// A file from a newer Lumit still opens: fields nobody here knows are
    /// ignored, and fields nobody there wrote take their own defaults.
    #[test]
    fn a_file_from_a_newer_lumit_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("newer.json");
        std::fs::write(
            &path,
            r#"{"preset":"Master","when_done":"upload","priority":3}"#,
        )
        .unwrap();

        let read = ExportDefaults::load(&path);
        assert_eq!(read.preset, "Master");
        assert!(
            read.codec.is_empty(),
            "an unwritten field takes its default"
        );
        assert_eq!(read.policy(), DESTINATION_ASK);
    }

    /// The policy is read through [`ExportDefaults::policy`], which answers
    /// *ask* for everything it does not recognise — but keeps a folder policy
    /// that has not been given its folder yet, which is what Settings is in
    /// between the two clicks.
    #[test]
    fn an_unrecognised_policy_reads_as_ask() {
        let mut d = some_defaults();
        assert_eq!(d.policy(), DESTINATION_FOLDER);

        d.folder = String::new();
        assert_eq!(d.policy(), DESTINATION_FOLDER, "still a folder policy");

        d.destination = "sftp".to_owned();
        assert_eq!(d.policy(), DESTINATION_ASK);

        d.destination = DESTINATION_PROJECT.to_owned();
        assert_eq!(d.policy(), DESTINATION_PROJECT);
    }
}

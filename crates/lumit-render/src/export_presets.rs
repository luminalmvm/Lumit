//! Named export presets, saved between sessions (docs/07 §11).
//!
//! # In plain terms
//!
//! An export has a lot of settings, and most of the time a person exports the
//! same way twice. A **preset** is that whole set of settings under a name:
//! pick it and every field fills in at once.
//!
//! Two kinds live side by side. **Built-ins** ship with Lumit — "Master" and
//! the delivery presets of docs/06 §7.5 — and are read-only: they mean a fixed
//! thing, and a project that says "YouTube 1080p60" must mean the same thing on
//! someone else's machine. **User presets** are saved from the dialog and live
//! in one small JSON file in the application's data directory, so they follow
//! the user between projects.
//!
//! The file is a convenience, never a correctness matter: a missing or damaged
//! one reads as an empty library rather than an error, because losing your
//! saved presets should cost you a re-save, not an export.

use crate::export::{Bitrate, ExportFormat, ExportPreset, ExportSpec, PRESET_AUDIO_BPS};
use std::path::{Path, PathBuf};

/// One preset: a name and the whole settings payload behind it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NamedPreset {
    pub name: String,
    pub spec: ExportSpec,
}

/// Where the user's own presets are kept. `None` only when the platform has
/// no home directory, in which case the library is in memory for the session
/// and nothing is written — the same answer every other store here gives.
pub fn default_path() -> Option<PathBuf> {
    lumit_project::export_presets_path()
}

/// The presets that ship with Lumit, in the order the list shows them. Every
/// one is read-only: [`PresetLibrary::put`] and [`PresetLibrary::delete`]
/// refuse these names.
pub fn built_ins() -> Vec<NamedPreset> {
    // "Master": the composition's own frame and rate, HEVC at a bitrate
    // worked out from the size, everything on. What you export when the file
    // is for you rather than for a platform.
    let master = NamedPreset {
        name: "Master".to_owned(),
        spec: ExportSpec {
            format: ExportFormat::Video(lumit_media::encode::VideoCodec::Hevc),
            target: None,
            bitrate: Bitrate::Auto,
            ..ExportSpec::default()
        },
    };
    let mut out = vec![master];
    for preset in ExportPreset::ALL {
        // Custom is the *absence* of a preset stamp, not a preset.
        let Some(params) = preset.params() else {
            continue;
        };
        out.push(NamedPreset {
            name: preset.label().to_owned(),
            spec: ExportSpec {
                format: ExportFormat::Video(params.codec),
                target: Some(params.size),
                bitrate: Bitrate::Manual {
                    target_bps: params.target_bps,
                    peak_bps: Some(params.peak_bps),
                },
                fps: Some(60.0),
                audio_bit_rate: PRESET_AUDIO_BPS,
                ..ExportSpec::default()
            },
        });
    }
    out
}

/// Whether `name` belongs to a built-in — the one question the read-only rule
/// hangs off, asked in one place so the dialog and the store cannot disagree.
pub fn is_built_in(name: &str) -> bool {
    built_ins().iter().any(|p| p.name == name)
}

/// The built-ins plus whatever the user has saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetLibrary {
    user: Vec<NamedPreset>,
}

impl PresetLibrary {
    /// Read the user's presets from `path`. An absent file is an empty
    /// library; so is a damaged one, because a preset file is a convenience
    /// and refusing to open the export dialog over it would be absurd.
    pub fn load(path: &Path) -> Self {
        let user = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<NamedPreset>>(&text).ok())
            .unwrap_or_default();
        Self { user }
    }

    /// Read from [`default_path`], or an empty library when there is none.
    pub fn load_default() -> Self {
        default_path().map(|p| Self::load(&p)).unwrap_or_default()
    }

    /// Write the user's presets to `path`, creating the directory. Built-ins
    /// are never written — they are code, and a copy on disk would go stale.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("export presets: {e}"))?;
        }
        let text =
            serde_json::to_string_pretty(&self.user).map_err(|e| format!("export presets: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("export presets: {e}"))
    }

    /// Save to [`default_path`]; a platform with no home directory keeps the
    /// library for the session and says so calmly.
    pub fn save_default(&self) -> Result<(), String> {
        match default_path() {
            Some(path) => self.save(&path),
            None => Err("this machine has no place to keep presets".to_owned()),
        }
    }

    /// Every preset the list shows, as `(name, read_only)` — built-ins first,
    /// then the user's own in the order they were saved.
    pub fn list(&self) -> Vec<(String, bool)> {
        built_ins()
            .into_iter()
            .map(|p| (p.name, true))
            .chain(self.user.iter().map(|p| (p.name.clone(), false)))
            .collect()
    }

    /// The settings behind a name, built-in or the user's.
    pub fn get(&self, name: &str) -> Option<ExportSpec> {
        self.user
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.spec.clone())
            .or_else(|| {
                built_ins()
                    .into_iter()
                    .find(|p| p.name == name)
                    .map(|p| p.spec)
            })
    }

    /// Save `spec` under `name`, replacing a preset of that name in place —
    /// the *Save as…* button, which is also the overwrite. A built-in's name
    /// is refused rather than shadowed: two presets called "Master" meaning
    /// different things is worse than being told to pick another name.
    pub fn put(&mut self, name: &str, spec: ExportSpec) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a preset needs a name".to_owned());
        }
        if is_built_in(name) {
            return Err(format!(
                "\"{name}\" is a built-in preset and cannot be replaced"
            ));
        }
        match self.user.iter_mut().find(|p| p.name == name) {
            Some(existing) => existing.spec = spec,
            None => self.user.push(NamedPreset {
                name: name.to_owned(),
                spec,
            }),
        }
        Ok(())
    }

    /// Remove a preset of one's own. A built-in and an unknown name both
    /// answer an error rather than a silent no-op, so the dialog can say why.
    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        if is_built_in(name) {
            return Err(format!(
                "\"{name}\" is a built-in preset and cannot be deleted"
            ));
        }
        let before = self.user.len();
        self.user.retain(|p| p.name != name);
        if self.user.len() == before {
            return Err(format!("there is no preset called \"{name}\""));
        }
        Ok(())
    }

    /// How many presets the user has saved (the built-ins are not counted —
    /// they are always there).
    pub fn user_count(&self) -> usize {
        self.user.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::export::{Channels, Crop, WhenDone};

    fn a_spec() -> ExportSpec {
        let mut spec = ExportSpec {
            format: ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            target: Some((640, 360)),
            channels: Channels::RgbAlpha,
            depth: lumit_media::encode::BitDepth::Sixteen,
            crop: Crop {
                top: 10,
                left: 20,
                bottom: 30,
                right: 40,
            },
            when_done: WhenDone::OpenFolder,
            ..ExportSpec::default()
        };
        spec.metadata
            .set(lumit_media::encode::Metadata::TITLE, "Scene 1");
        spec
    }

    /// The whole settings payload survives a save and a reload — every field,
    /// not just the ones the dialog happens to show today. A preset that
    /// silently dropped a setting would be worse than no preset at all.
    #[test]
    fn a_user_preset_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("export-presets.json");

        let mut lib = PresetLibrary::default();
        lib.put("My delivery", a_spec()).unwrap();
        lib.save(&path).unwrap();

        let back = PresetLibrary::load(&path);
        assert_eq!(back.user_count(), 1);
        let spec = back.get("My delivery").expect("saved preset is there");
        assert_eq!(spec, a_spec());
    }

    /// Saving under an existing name replaces it in place rather than making
    /// a second row of the same name.
    #[test]
    fn saving_over_a_name_replaces_it_and_keeps_its_place() {
        let mut lib = PresetLibrary::default();
        lib.put("A", a_spec()).unwrap();
        lib.put("B", ExportSpec::default()).unwrap();
        lib.put("A", ExportSpec::default()).unwrap();
        assert_eq!(lib.user_count(), 2);
        assert_eq!(
            lib.list()
                .into_iter()
                .filter(|(_, read_only)| !read_only)
                .map(|(n, _)| n)
                .collect::<Vec<_>>(),
            ["A", "B"],
            "the replaced preset kept its row"
        );
        assert_eq!(lib.get("A"), Some(ExportSpec::default()));
    }

    /// Built-ins are read-only in both directions, and a nameless preset is
    /// refused before it can become an unnameable row.
    #[test]
    fn built_ins_cannot_be_replaced_or_deleted() {
        let mut lib = PresetLibrary::default();
        assert!(lib.put("Master", ExportSpec::default()).is_err());
        assert!(lib.delete("Master").is_err());
        assert!(lib.put("  ", ExportSpec::default()).is_err());
        assert!(lib.delete("never existed").is_err());
        // A user preset deletes cleanly.
        lib.put("Mine", ExportSpec::default()).unwrap();
        assert!(lib.delete("Mine").is_ok());
        assert_eq!(lib.user_count(), 0);
    }

    /// The list is built-ins first, all marked read-only, and every one of
    /// them resolves to a spec that would actually run.
    #[test]
    fn the_built_ins_lead_the_list_and_every_one_resolves() {
        let lib = PresetLibrary::default();
        let list = lib.list();
        assert_eq!(list.first().map(|(n, _)| n.as_str()), Some("Master"));
        assert!(list.iter().all(|(_, read_only)| *read_only));
        for (name, _) in &list {
            let spec = lib.get(name).unwrap_or_else(|| panic!("{name} resolves"));
            spec.check()
                .unwrap_or_else(|e| panic!("built-in {name} is not exportable: {e}"));
            assert!(is_built_in(name));
        }
        // "Master" follows the composition's own frame.
        assert_eq!(lib.get("Master").unwrap().target, None);
    }

    /// A missing or damaged file is an empty library, never an error: losing
    /// saved presets must cost a re-save, not an export.
    #[test]
    fn a_missing_or_damaged_file_reads_as_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-there.json");
        assert_eq!(PresetLibrary::load(&missing).user_count(), 0);

        let damaged = dir.path().join("damaged.json");
        std::fs::write(&damaged, "{ this is not json").unwrap();
        assert_eq!(PresetLibrary::load(&damaged).user_count(), 0);
    }
}

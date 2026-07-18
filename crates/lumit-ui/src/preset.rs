//! Effect presets (docs/07-UI-SPEC.md §6/§7, K-065): save a layer's whole
//! effect stack to a file and load it onto another layer.
//!
//! In plain terms: an effect preset is just the list of effects on a layer,
//! with their settings, written to a small `.lumfx` JSON file so it can be
//! reused or shared. Loading one gives every effect a fresh id, so applying
//! the same preset to two layers never makes them share an instance.

use lumit_core::model::EffectInstance;

/// A saved effect stack. `format` is bumped if the on-disk shape changes;
/// the effects are exactly the model's `EffectInstance`s, so a preset always
/// round-trips whatever a project does.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EffectPreset {
    pub format: u32,
    pub name: String,
    pub effects: Vec<EffectInstance>,
}

/// The current on-disk format version.
pub const PRESET_FORMAT: u32 = 1;

/// The file extension presets use (a plain JSON document inside).
pub const PRESET_EXTENSION: &str = "lumfx";

/// Serialise a stack to the preset JSON text.
pub fn to_json(name: &str, effects: &[EffectInstance]) -> Result<String, String> {
    serde_json::to_string_pretty(&EffectPreset {
        format: PRESET_FORMAT,
        name: name.to_owned(),
        effects: effects.to_vec(),
    })
    .map_err(|e| e.to_string())
}

/// Parse preset JSON text back to a preset. A newer `format` still loads:
/// unknown fields ride along in each effect's `extra` map, matching how the
/// project file tolerates forward-compatible additions.
pub fn from_json(text: &str) -> Result<EffectPreset, String> {
    serde_json::from_str::<EffectPreset>(text).map_err(|e| e.to_string())
}

/// The preset's effects with fresh instance ids — what actually lands on a
/// layer, so applying one preset to several layers never shares an instance
/// id (ids are instance identity only; they never feed a cache key).
pub fn instantiated(preset: &EffectPreset) -> Vec<EffectInstance> {
    preset
        .effects
        .iter()
        .cloned()
        .map(|mut e| {
            e.id = uuid::Uuid::now_v7();
            e
        })
        .collect()
}

/// One preset shown in the Effects & Presets browser (docs/07-UI-SPEC.md §7):
/// its file path and the name to display — the preset's own `name` when the
/// file parses, otherwise the file stem, so a hand-copied or partly written
/// file still lists under a sensible label rather than vanishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    pub path: std::path::PathBuf,
    pub name: String,
}

/// Scan `dir` for `*.lumfx` presets and return them for the browser, sorted by
/// display name (case-insensitive) so the list is stable between paints. A
/// missing directory or an unreadable entry yields fewer results, never an
/// error — the browser then shows a hint rather than a failure. Each entry's
/// display name is the preset's own `name` when the file parses, else the file
/// stem.
pub fn list_presets(dir: &std::path::Path) -> Vec<PresetEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PresetEntry> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        // Match the extension case-insensitively; skip anything else.
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !e.eq_ignore_ascii_case(PRESET_EXTENSION))
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("preset")
            .to_owned();
        let name = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| from_json(&t).ok())
            .map(|p| p.name)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(stem);
        out.push(PresetEntry { path, name });
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

/// Read a preset file and return its effects with fresh ids, ready to append
/// to a layer's stack. `None` on any read or parse error, so the browser can
/// show a hint and leave the document untouched (applying a preset is never a
/// half-done edit).
pub fn load_instantiated(path: &std::path::Path) -> Option<Vec<EffectInstance>> {
    let text = std::fs::read_to_string(path).ok()?;
    let preset = from_json(&text).ok()?;
    Some(instantiated(&preset))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stack() -> Vec<EffectInstance> {
        vec![
            lumit_core::fx::instantiate("blur").unwrap(),
            lumit_core::fx::instantiate("glow").unwrap(),
        ]
    }

    #[test]
    fn a_preset_round_trips_through_json() {
        let effects = stack();
        let json = to_json("My look", &effects).unwrap();
        let back = from_json(&json).unwrap();
        assert_eq!(back.format, PRESET_FORMAT);
        assert_eq!(back.name, "My look");
        assert_eq!(back.effects, effects);
    }

    #[test]
    fn instantiating_gives_fresh_ids_but_keeps_the_effects() {
        let preset = from_json(&to_json("look", &stack()).unwrap()).unwrap();
        let a = instantiated(&preset);
        let b = instantiated(&preset);
        // Same effects and params, but every instance id is unique.
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].effect, preset.effects[0].effect);
        assert_ne!(a[0].id, preset.effects[0].id);
        assert_ne!(a[0].id, b[0].id);
    }

    #[test]
    fn list_presets_reads_names_sorts_and_ignores_non_lumfx() {
        let dir = tempfile::tempdir().unwrap();
        // Two valid presets whose display names differ from their file stems.
        std::fs::write(
            dir.path().join("z-file.lumfx"),
            to_json("Alpha look", &stack()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("a-file.lumfx"),
            to_json("Beta look", &stack()).unwrap(),
        )
        .unwrap();
        // A non-preset file and a garbage .lumfx (kept, listed by its stem).
        std::fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();
        std::fs::write(dir.path().join("broken.lumfx"), "{ not json").unwrap();

        let entries = list_presets(dir.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        // Sorted by display name (case-insensitive): the parsed names win over
        // the stems, and the unreadable file falls back to its stem.
        assert_eq!(names, vec!["Alpha look", "Beta look", "broken"]);
    }

    #[test]
    fn list_presets_of_a_missing_directory_is_empty_not_an_error() {
        let missing = std::path::Path::new("definitely-not-a-real-dir-xyz");
        assert!(list_presets(missing).is_empty());
    }

    #[test]
    fn load_instantiated_round_trips_a_saved_preset_with_fresh_ids() {
        let dir = tempfile::tempdir().unwrap();
        let effects = stack();
        let path = dir.path().join("look.lumfx");
        std::fs::write(&path, to_json("look", &effects).unwrap()).unwrap();

        let loaded = load_instantiated(&path).unwrap();
        assert_eq!(loaded.len(), effects.len());
        assert_eq!(loaded[0].effect, effects[0].effect);
        assert_ne!(loaded[0].id, effects[0].id);
        // A broken file loads to None rather than panicking.
        std::fs::write(&path, "not a preset").unwrap();
        assert!(load_instantiated(&path).is_none());
    }

    #[test]
    fn a_newer_format_still_loads() {
        // A preset written by a hypothetical newer Lumit, with an unknown
        // top-level field, still parses — serde ignores what it doesn't know.
        let effects = stack();
        let mut v = serde_json::to_value(EffectPreset {
            format: 99,
            name: "future".into(),
            effects: effects.clone(),
        })
        .unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("future_field".into(), serde_json::json!(true));
        let back = from_json(&v.to_string()).unwrap();
        assert_eq!(back.effects, effects);
    }
}

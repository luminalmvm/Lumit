//! Effect presets (docs/07-UI-SPEC.md §6/§7, K-065): save a layer's whole
//! effect stack to a file and load it onto another layer.
//!
//! In plain terms: an effect preset is just the list of effects on a layer,
//! with their settings, written to a small `.lumfx` JSON file so it can be
//! reused or shared. Loading one gives every effect a fresh id, so applying
//! the same preset to two layers never makes them share an instance.

use crate::model::EffectInstance;

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

/// The earliest keyframe time across `effects`, in **layer-local** seconds, or
/// `None` when nothing in them is animated (K-275).
///
/// Copying an effect and pasting it somewhere else is copying a piece of
/// *timing* as much as a look, so the paste has to know where that timing
/// starts before it can land it under the playhead.
#[must_use]
pub fn first_key_time(effects: &[EffectInstance]) -> Option<crate::time::Rational> {
    let mut earliest: Option<crate::time::Rational> = None;
    for_each_property(effects, &mut |property| {
        if let crate::anim::Animation::Keyframed(keys) = &property.animation {
            if let Some(first) = keys.first() {
                earliest = Some(match earliest {
                    Some(held) if held <= first.time => held,
                    _ => first.time,
                });
            }
        }
    });
    earliest
}

/// Shift every keyframe in `effects` by `delta` layer-local seconds (K-275).
///
/// A key whose time cannot be moved without overflowing is left where it is
/// rather than wrapping — an engine crate does not panic, and a paste that
/// silently misplaced one key would be worse than one that refused to move it.
pub fn shift_keys(effects: &mut [EffectInstance], delta: crate::time::Rational) {
    if delta.is_zero() {
        return;
    }
    for_each_property_mut(effects, &mut |property| {
        if let crate::anim::Animation::Keyframed(keys) = &mut property.animation {
            for key in keys.iter_mut() {
                if let Ok(moved) = key.time.checked_add(delta) {
                    key.time = moved;
                }
            }
        }
    });
}

/// Visit every animatable [`crate::anim::Property`] in `effects`.
///
/// **Exhaustive on purpose**, like `fx::rescale_px`: a new `EffectValue`
/// variant must decide here whether it carries animation, so a parameter added
/// later cannot quietly stop being shifted by a paste.
fn for_each_property(effects: &[EffectInstance], visit: &mut impl FnMut(&crate::anim::Property)) {
    for effect in effects {
        for param in &effect.params {
            match &param.value {
                crate::model::EffectValue::Float(p) => visit(p),
                crate::model::EffectValue::Point(x, y) => {
                    visit(x);
                    visit(y);
                }
                crate::model::EffectValue::Colour(channels) => {
                    for channel in channels {
                        visit(channel);
                    }
                }
                crate::model::EffectValue::File(f) => visit(&f.index),
                // Carry no animation: a bool, a dropdown choice, a random
                // seed, a layer reference and a mask-path reference are all
                // static in v1 (docs/03 §8). A mask path's *shape* animates,
                // but it animates on the mask, not here — this value is only
                // which mask (K-408).
                crate::model::EffectValue::Bool(_)
                | crate::model::EffectValue::Choice(_)
                | crate::model::EffectValue::Seed(_)
                | crate::model::EffectValue::Layer(_)
                | crate::model::EffectValue::MaskPath(_) => {}
            }
        }
    }
}

/// The mutable twin of [`for_each_property`], same exhaustive match.
fn for_each_property_mut(
    effects: &mut [EffectInstance],
    visit: &mut impl FnMut(&mut crate::anim::Property),
) {
    for effect in effects {
        for param in &mut effect.params {
            match &mut param.value {
                crate::model::EffectValue::Float(p) => visit(p),
                crate::model::EffectValue::Point(x, y) => {
                    visit(x);
                    visit(y);
                }
                crate::model::EffectValue::Colour(channels) => {
                    for channel in channels {
                        visit(channel);
                    }
                }
                crate::model::EffectValue::File(f) => visit(&mut f.index),
                crate::model::EffectValue::Bool(_)
                | crate::model::EffectValue::Choice(_)
                | crate::model::EffectValue::Seed(_)
                | crate::model::EffectValue::Layer(_)
                | crate::model::EffectValue::MaskPath(_) => {}
            }
        }
    }
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

/// The effects a "Save stack as preset" writes, given the current selection
/// (docs/07-UI-SPEC.md §6/§7, K-156). Pure so it can be tested without egui.
///
/// - `effects` is the layer's whole effect stack.
/// - `selected_effects` are the stack indices whose parameter rows are
///   highlighted — the effect-row selection (`selected_prop`/`selected_props`).
/// - `selected_keys` names the keyframes picked out on the lanes: for each
///   `(effect index, parameter index)`, the exact key times highlighted.
///
/// The rule:
/// - nothing highlighted → the whole stack, so today's behaviour is unchanged;
/// - otherwise every effect the selection touches (a highlighted row, or a
///   highlighted key), in stack order, and within each of those effects any
///   Float parameter that has highlighted keys is trimmed to just those keys.
///   A parameter with no highlighted keys keeps its value exactly as set —
///   including any full animation the user did not single a key out of.
///
/// Key times match exactly: `selected_keys` carries each key's own rational
/// time (that is what the lane selection stores), so a stale selection whose
/// key was edited away simply matches nothing and the parameter is left whole.
pub fn selection_subset(
    effects: &[EffectInstance],
    selected_effects: &std::collections::BTreeSet<usize>,
    selected_keys: &std::collections::BTreeMap<
        (usize, usize),
        std::collections::BTreeSet<crate::Rational>,
    >,
) -> Vec<EffectInstance> {
    use crate::anim::Animation;
    use crate::model::EffectValue;

    // Nothing highlighted anywhere: keep the whole-stack behaviour.
    if selected_effects.is_empty() && selected_keys.is_empty() {
        return effects.to_vec();
    }

    // Every effect the selection touches, in stack order (BTreeSet iterates
    // sorted, so the saved stack keeps its original order).
    let mut include: std::collections::BTreeSet<usize> = selected_effects.clone();
    for (effect, _param) in selected_keys.keys() {
        include.insert(*effect);
    }

    let mut out = Vec::with_capacity(include.len());
    for &ei in &include {
        let Some(src) = effects.get(ei) else {
            continue; // a stale index (effect removed) contributes nothing
        };
        let mut inst = src.clone();
        for (pi, param) in inst.params.iter_mut().enumerate() {
            let Some(times) = selected_keys.get(&(ei, pi)) else {
                continue; // this parameter has no highlighted keys: keep as set
            };
            let EffectValue::Float(prop) = &mut param.value else {
                continue; // only Float parameters carry lane keys today
            };
            if let Animation::Keyframed(keys) = &prop.animation {
                let kept: Vec<crate::anim::Keyframe> = keys
                    .iter()
                    .filter(|k| times.contains(&k.time))
                    .copied()
                    .collect();
                // Filtering the already-sorted, unique keys keeps that invariant.
                // If nothing matched (a stale selection) leave the animation
                // whole rather than emptying it.
                if !kept.is_empty() {
                    prop.animation = Animation::Keyframed(kept);
                }
            }
        }
        out.push(inst);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn stack() -> Vec<EffectInstance> {
        vec![
            crate::fx::instantiate("blur").unwrap(),
            crate::fx::instantiate("glow").unwrap(),
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

    /// A stack of three effects; effect 1's first Float parameter is keyframed
    /// at the given times so the subset filtering has real keys to trim.
    fn keyed_stack(times: &[f64]) -> (Vec<EffectInstance>, usize) {
        use crate::anim::{Animation, Keyframe, SideInterp};
        use crate::model::EffectValue;
        let keys: Vec<Keyframe> = times
            .iter()
            .map(|&t| Keyframe {
                time: crate::Rational::from_f64_on_grid(t, crate::Rational::FLICK_DEN).unwrap(),
                value: t,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            })
            .collect();
        let mut effects = vec![
            crate::fx::instantiate("blur").unwrap(),
            crate::fx::instantiate("glow").unwrap(),
            crate::fx::instantiate("blur").unwrap(),
        ];
        // The first Float parameter on effect 1 becomes keyframed.
        let pi = effects[1]
            .params
            .iter()
            .position(|p| matches!(p.value, EffectValue::Float(_)))
            .unwrap();
        effects[1].params[pi].value = EffectValue::Float(crate::anim::Property {
            animation: Animation::Keyframed(keys),
            extra: serde_json::Map::new(),
        });
        (effects, pi)
    }

    fn rat(t: f64) -> crate::Rational {
        crate::Rational::from_f64_on_grid(t, crate::Rational::FLICK_DEN).unwrap()
    }

    #[test]
    fn selection_subset_with_no_selection_saves_the_whole_stack() {
        let (effects, _pi) = keyed_stack(&[0.0, 1.0, 2.0]);
        let out = selection_subset(
            &effects,
            &std::collections::BTreeSet::new(),
            &std::collections::BTreeMap::new(),
        );
        // Byte-for-byte the whole stack — the unchanged fallback behaviour.
        assert_eq!(out, effects);
    }

    #[test]
    fn selection_subset_of_effect_rows_keeps_those_effects_whole_in_order() {
        let (effects, _pi) = keyed_stack(&[0.0, 1.0, 2.0]);
        // Highlight effects 2 and 0 (out of order): the subset keeps them in
        // stack order and carries every parameter and keyframe untouched.
        let sel: std::collections::BTreeSet<usize> = [2usize, 0].into_iter().collect();
        let out = selection_subset(&effects, &sel, &std::collections::BTreeMap::new());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], effects[0]);
        assert_eq!(out[1], effects[2]);
    }

    #[test]
    fn selection_subset_of_keyframes_trims_to_just_those_keys_and_effects() {
        use crate::anim::Animation;
        use crate::model::EffectValue;
        let (effects, pi) = keyed_stack(&[0.0, 1.0, 2.0]);
        // Only two of effect 1's three keys are highlighted; no other effect.
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(
            (1usize, pi),
            [rat(0.0), rat(2.0)]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
        );
        let out = selection_subset(&effects, &std::collections::BTreeSet::new(), &keys);
        // Only the keyed effect is saved.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].effect, effects[1].effect);
        // Its keyframed parameter now holds exactly the two selected keys.
        let EffectValue::Float(prop) = &out[0].params[pi].value else {
            panic!("expected a Float parameter");
        };
        let Animation::Keyframed(kept) = &prop.animation else {
            panic!("expected a keyframed parameter");
        };
        let got: Vec<f64> = kept.iter().map(|k| k.time.to_f64()).collect();
        assert_eq!(got, vec![0.0, 2.0]);
    }

    #[test]
    fn selection_subset_combines_a_row_and_a_key_selection() {
        use crate::anim::Animation;
        use crate::model::EffectValue;
        let (effects, pi) = keyed_stack(&[0.0, 1.0, 2.0]);
        // Effect 0 is row-selected (saved whole); effect 1 has one key selected
        // (trimmed to it). Effect 2 is untouched and must not appear.
        let sel: std::collections::BTreeSet<usize> = [0usize].into_iter().collect();
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(
            (1usize, pi),
            [rat(1.0)]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
        );
        let out = selection_subset(&effects, &sel, &keys);
        assert_eq!(out.len(), 2);
        // Effect 0 unchanged.
        assert_eq!(out[0], effects[0]);
        // Effect 1 trimmed to its single highlighted key.
        assert_eq!(out[1].effect, effects[1].effect);
        let EffectValue::Float(prop) = &out[1].params[pi].value else {
            panic!("expected a Float parameter");
        };
        let Animation::Keyframed(kept) = &prop.animation else {
            panic!("expected a keyframed parameter");
        };
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].time.to_f64(), 1.0);
    }

    #[test]
    fn selection_subset_ignores_stale_key_times_and_indices() {
        let (effects, pi) = keyed_stack(&[0.0, 1.0, 2.0]);
        // A key time that no key has, plus an effect index past the stack end.
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(
            (1usize, pi),
            [rat(9.0)]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
        );
        keys.insert(
            (99usize, 0),
            [rat(0.0)]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
        );
        let out = selection_subset(&effects, &std::collections::BTreeSet::new(), &keys);
        // Effect 1 is still included (it was touched) but, since no key matched,
        // its animation is left whole rather than emptied; the bad index is
        // dropped silently.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].params[pi].value, effects[1].params[pi].value);
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

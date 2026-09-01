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
                // seed, a layer reference, a mask-path reference and a tone
                // curve are all static in v1 (docs/03 §8). A mask path's
                // *shape* animates, but it animates on the mask, not here —
                // this value is only which mask (K-408); a curve's shape is
                // right here and still does not animate, because a list that
                // grows has nothing to interpolate (K-412).
                crate::model::EffectValue::Bool(_)
                | crate::model::EffectValue::Choice(_)
                | crate::model::EffectValue::Seed(_)
                | crate::model::EffectValue::Layer(_)
                | crate::model::EffectValue::MaskPath(_)
                | crate::model::EffectValue::Curve(_) => {}
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
                | crate::model::EffectValue::MaskPath(_)
                | crate::model::EffectValue::Curve(_) => {}
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

// --- Node groups (K-651) --------------------------------------------------
//
// **In plain terms.** A group preset is to the Graph panel what an effect
// preset is to the effect stack: pick a few boxes, give the set a name, and it
// goes in the same library folder so it can be dropped into another layer's
// graph later. What it saves is the boxes, the wires **between** them, and
// where they sat relative to one another — so a rig that took five minutes to
// wire comes back wired.
//
// A wire that left the group is not saved: it named something the group does
// not carry, and inventing an end for it on the way back in would be a guess.

/// A saved set of driver boxes and the wires between them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupPreset {
    pub format: u32,
    pub name: String,
    /// Which chip of the label palette the group wears (K-188's set, indexed).
    #[serde(default)]
    pub colour: u32,
    /// The driver instances, in the order the graph carried them.
    pub nodes: Vec<EffectInstance>,
    /// The wires with both ends inside the set, naming the saved nodes' ids.
    #[serde(default)]
    pub edges: Vec<crate::graph::Edge>,
    /// One position per node, **relative to the set's top-left corner**, so a
    /// group dropped anywhere keeps the shape it was saved in.
    #[serde(default)]
    pub layout: Vec<[f64; 2]>,
}

/// The current on-disk format version for a group.
pub const GROUP_FORMAT: u32 = 1;

/// The file extension node groups use — a plain JSON document, beside the
/// `.lumfx` effect presets in the same per-user library.
pub const GROUP_EXTENSION: &str = "lumgrp";

/// The id of a node this group could carry — `None` for the derived boxes and
/// for the stack's effects, which belong to the effect list rather than here.
fn driver_id(node: crate::graph::NodeRef) -> Option<uuid::Uuid> {
    match node {
        crate::graph::NodeRef::Driver(id) => Some(id),
        _ => None,
    }
}

/// Gather `members` out of `graph` as a saveable group.
///
/// Only the driver boxes are taken: the Source, the Layer out and the stack's
/// effects are *derived* from the layer, so a saved group naming one could only
/// ever be re-inserted onto a layer that happened to have the same stack.
///
/// Deterministic: the nodes come out in the graph's own order, never the
/// selection's, so saving the same three boxes twice writes the same file.
#[must_use]
pub fn group_from_graph(
    graph: &crate::graph::LayerGraph,
    name: &str,
    colour: u32,
    members: &[crate::graph::NodeRef],
) -> GroupPreset {
    let wanted: std::collections::BTreeSet<uuid::Uuid> =
        members.iter().copied().filter_map(driver_id).collect();
    let nodes: Vec<EffectInstance> = graph
        .nodes
        .iter()
        .filter(|n| wanted.contains(&n.id))
        .cloned()
        .collect();

    let at = |id: uuid::Uuid| {
        graph
            .layout
            .iter()
            .find(|(node, _)| driver_id(*node) == Some(id))
            .map_or([0.0, 0.0], |(_, xy)| *xy)
    };
    let places: Vec<[f64; 2]> = nodes.iter().map(|n| at(n.id)).collect();
    // The set's top-left corner. An empty set has no corner and no layout, so
    // the fold's identity never reaches the subtraction below.
    let origin = places.iter().fold([f64::MAX, f64::MAX], |acc, p| {
        [acc[0].min(p[0]), acc[1].min(p[1])]
    });
    let layout: Vec<[f64; 2]> = places
        .iter()
        .map(|p| [p[0] - origin[0], p[1] - origin[1]])
        .collect();

    let inside =
        |node: crate::graph::NodeRef| driver_id(node).is_some_and(|id| wanted.contains(&id));
    let edges: Vec<crate::graph::Edge> = graph
        .edges
        .iter()
        .filter(|e| {
            let from = match &e.from {
                crate::graph::OutputRef::Driver { node, .. } => wanted.contains(node),
                _ => false,
            };
            let to = match &e.to {
                crate::graph::InputRef::Param { node, .. } => inside(*node),
                crate::graph::InputRef::Matte { .. } => false,
            };
            from && to
        })
        .cloned()
        .collect();

    GroupPreset {
        format: GROUP_FORMAT,
        name: name.to_owned(),
        colour,
        nodes,
        edges,
        layout,
    }
}

/// Serialise a group to its JSON text.
pub fn group_to_json(preset: &GroupPreset) -> Result<String, String> {
    serde_json::to_string_pretty(preset).map_err(|e| e.to_string())
}

/// Parse group JSON back. A newer `format` still loads, for the reason
/// [`from_json`] gives.
pub fn group_from_json(text: &str) -> Result<GroupPreset, String> {
    serde_json::from_str::<GroupPreset>(text).map_err(|e| e.to_string())
}

/// Everything one insert adds to a graph — handed over in a piece so the caller
/// extends four lists and commits once.
#[derive(Debug, Clone)]
pub struct InsertedGroup {
    pub nodes: Vec<EffectInstance>,
    pub edges: Vec<crate::graph::Edge>,
    pub layout: Vec<(crate::graph::NodeRef, [f64; 2])>,
    pub group: crate::graph::NodeGroup,
}

/// What inserting `preset` at canvas point `at` adds to a graph: fresh nodes,
/// the wires between them re-pointed at those fresh ids, where each one sits,
/// and the group that names them.
///
/// Every id is minted here, so inserting one group twice never makes two boxes
/// share an instance — the same rule [`instantiated`] follows for an effect
/// preset, and for the same reason.
#[must_use]
pub fn group_instantiated(preset: &GroupPreset, at: [f64; 2]) -> InsertedGroup {
    let mut fresh = std::collections::BTreeMap::new();
    let nodes: Vec<EffectInstance> = preset
        .nodes
        .iter()
        .cloned()
        .map(|mut n| {
            let id = uuid::Uuid::now_v7();
            fresh.insert(n.id, id);
            n.id = id;
            n
        })
        .collect();

    let renamed = |node: crate::graph::NodeRef| match node {
        crate::graph::NodeRef::Driver(id) => {
            fresh.get(&id).copied().map(crate::graph::NodeRef::Driver)
        }
        other => Some(other),
    };
    let edges: Vec<crate::graph::Edge> = preset
        .edges
        .iter()
        .filter_map(|e| {
            // A wire naming a node the file does not carry is dropped rather
            // than landing dangling: the graph would refuse the whole insert
            // for it, and one bad wire must not cost the rig.
            let from = match &e.from {
                crate::graph::OutputRef::Driver { node, port } => crate::graph::OutputRef::Driver {
                    node: *fresh.get(node)?,
                    port: port.clone(),
                },
                _ => return None,
            };
            let to = match &e.to {
                crate::graph::InputRef::Param { node, port } => crate::graph::InputRef::Param {
                    node: renamed(*node)?,
                    port: port.clone(),
                },
                crate::graph::InputRef::Matte { .. } => return None,
            };
            Some(crate::graph::Edge { from, to })
        })
        .collect();

    let layout: Vec<(crate::graph::NodeRef, [f64; 2])> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let rel = preset.layout.get(i).copied().unwrap_or([0.0, 0.0]);
            (
                crate::graph::NodeRef::Driver(n.id),
                [at[0] + rel[0], at[1] + rel[1]],
            )
        })
        .collect();

    let group = crate::graph::NodeGroup {
        name: preset.name.clone(),
        colour: preset.colour,
        members: nodes
            .iter()
            .map(|n| crate::graph::NodeRef::Driver(n.id))
            .collect(),
    };
    InsertedGroup {
        nodes,
        edges,
        layout,
        group,
    }
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

    // --- Node groups ------------------------------------------------------

    /// Two drivers wired to each other, a third outside them, and one wire
    /// leaving the pair — the shape every group claim is made against.
    fn wired_graph() -> (crate::graph::LayerGraph, Vec<crate::graph::NodeRef>) {
        use crate::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
        let wiggle = crate::fx::instantiate("wiggle").unwrap();
        let smooth = crate::fx::instantiate("smooth").unwrap();
        let outside = crate::fx::instantiate("wiggle").unwrap();
        let (a, b, c) = (wiggle.id, smooth.id, outside.id);
        let graph = LayerGraph {
            out_unwired: false,
            nodes: vec![wiggle, smooth, outside],
            edges: vec![
                // Inside the pair.
                Edge {
                    from: OutputRef::Driver {
                        node: a,
                        port: "value".into(),
                    },
                    to: InputRef::Param {
                        node: NodeRef::Driver(b),
                        port: "value".into(),
                    },
                },
                // Out of the pair, into the box left behind.
                Edge {
                    from: OutputRef::Driver {
                        node: b,
                        port: "value".into(),
                    },
                    to: InputRef::Param {
                        node: NodeRef::Driver(c),
                        port: "amount".into(),
                    },
                },
            ],
            layout: vec![
                (NodeRef::Driver(a), [100.0, 60.0]),
                (NodeRef::Driver(b), [260.0, 90.0]),
                (NodeRef::Driver(c), [500.0, 300.0]),
            ],
            exposed: Vec::new(),
            groups: Vec::new(),
        };
        (graph, vec![NodeRef::Driver(a), NodeRef::Driver(b)])
    }

    #[test]
    fn a_group_saves_its_members_the_wires_between_them_and_their_shape() {
        let (graph, members) = wired_graph();
        let preset = group_from_graph(&graph, "Audio rig", 4, &members);

        assert_eq!(preset.nodes.len(), 2, "the two picked boxes, no more");
        assert_eq!(
            preset.edges.len(),
            1,
            "the wire between them is kept and the one leaving them is not"
        );
        // Relative to the set's own top-left, so the pair keeps its shape
        // wherever it is dropped.
        assert_eq!(preset.layout, vec![[0.0, 0.0], [160.0, 30.0]]);
        assert_eq!(preset.colour, 4);
    }

    #[test]
    fn inserting_a_group_mints_fresh_ids_and_re_points_the_wires() {
        let (graph, members) = wired_graph();
        let preset = group_from_json(
            &group_to_json(&group_from_graph(&graph, "Audio rig", 4, &members)).unwrap(),
        )
        .unwrap();

        let added = group_instantiated(&preset, [400.0, 200.0]);
        let (nodes, edges, layout, group) = (added.nodes, added.edges, added.layout, added.group);
        assert_eq!(nodes.len(), 2);
        assert!(
            nodes
                .iter()
                .all(|n| !preset.nodes.iter().any(|s| s.id == n.id)),
            "every instance id is fresh, so inserting twice never shares one"
        );
        // The wire came back pointing at the *new* boxes, not the saved ones.
        assert_eq!(edges.len(), 1);
        let crate::graph::OutputRef::Driver { node: from, .. } = edges[0].from else {
            panic!("a driver wire");
        };
        let crate::graph::InputRef::Param {
            node: crate::graph::NodeRef::Driver(to),
            ..
        } = edges[0].to
        else {
            panic!("into a driver parameter");
        };
        assert_eq!(from, nodes[0].id);
        assert_eq!(to, nodes[1].id);
        // Dropped where it was asked for, keeping the saved shape.
        assert_eq!(layout[0].1, [400.0, 200.0]);
        assert_eq!(layout[1].1, [560.0, 230.0]);
        assert_eq!(group.name, "Audio rig");
        assert_eq!(group.members.len(), 2);
    }

    /// A second insert of the same file shares nothing with the first — the
    /// claim that makes a group library safe to lean on.
    #[test]
    fn two_inserts_of_one_group_share_no_ids() {
        let (graph, members) = wired_graph();
        let preset = group_from_graph(&graph, "Rig", 0, &members);
        let first = group_instantiated(&preset, [0.0, 0.0]).nodes;
        let second = group_instantiated(&preset, [0.0, 0.0]).nodes;
        assert!(first.iter().all(|a| !second.iter().any(|b| b.id == a.id)));
    }
}

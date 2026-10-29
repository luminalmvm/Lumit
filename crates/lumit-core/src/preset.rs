//! Effect presets (docs/07-UI-SPEC.md §6/§7): save a layer's whole
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
    /// The wires between the copied effects and the drivers that
    /// feed them, as a node group, so effects copied off one layer and pasted
    /// on another arrive wired ([`wiring_of`]). Absent when nothing is wired,
    /// and from every preset file, which reads and writes as it did; a Lumit
    /// without it reads the effects and leaves this be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiring: Option<GroupPreset>,
}

/// The current on-disk format version.
pub const PRESET_FORMAT: u32 = 1;

/// The file extension presets use (a plain JSON document inside).
pub const PRESET_EXTENSION: &str = "lumfx";

/// Serialise a stack to the preset JSON text.
pub fn to_json(name: &str, effects: &[EffectInstance]) -> Result<String, String> {
    to_json_wired(name, effects, None)
}

/// [`to_json`] with the wiring the effects were copied with.
pub fn to_json_wired(
    name: &str,
    effects: &[EffectInstance],
    wiring: Option<GroupPreset>,
) -> Result<String, String> {
    serde_json::to_string_pretty(&EffectPreset {
        format: PRESET_FORMAT,
        name: name.to_owned(),
        effects: effects.to_vec(),
        wiring,
    })
    .map_err(|e| e.to_string())
}

/// What copying `effects` off a layer takes along of its `graph`:
/// the wires between them, and every driver that feeds one of them, directly
/// or through other drivers, with that driver's own wires. `None` when no
/// wire is among them, so a plain copy stays a plain preset.
///
/// A wire from something not taken (another effect, the source's matte) is
/// left behind, as a node group leaves a wire that leaves the set.
#[must_use]
pub fn wiring_of(
    graph: &crate::graph::LayerGraph,
    stack: &[EffectInstance],
    effects: &[uuid::Uuid],
) -> Option<GroupPreset> {
    use crate::graph::{InputRef, NodeRef, OutputRef};
    let mut members: Vec<NodeRef> = effects.iter().map(|id| NodeRef::Effect(*id)).collect();
    // The drivers upstream, until a pass finds no new one.
    loop {
        let found: Vec<NodeRef> = graph
            .edges
            .iter()
            .filter_map(|e| match (&e.from, &e.to) {
                (OutputRef::Driver { node, .. }, InputRef::Param { node: to, .. })
                    if members.contains(to) && !members.contains(&NodeRef::Driver(*node)) =>
                {
                    Some(NodeRef::Driver(*node))
                }
                _ => None,
            })
            .collect();
        if found.is_empty() {
            break;
        }
        for node in found {
            if !members.contains(&node) {
                members.push(node);
            }
        }
    }
    let wiring = group_from_graph(graph, stack, "", 0, &members);
    (!wiring.edges.is_empty()).then_some(wiring)
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
/// `None` when nothing in them is animated.
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

/// Shift every keyframe in `effects` by `delta` layer-local seconds.
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
                // seed, a layer reference, a clip reference, a mask-path
                // reference, a tone
                // curve and a colour name are all static in v1 (docs/03 §8).
                // A mask path's *shape* animates, but it animates on the mask,
                // not here - this value is only which mask; a curve's shape is
                // right here and still does not animate, because a list that
                // grows has nothing to interpolate.
                crate::model::EffectValue::Bool(_)
                | crate::model::EffectValue::Choice(_)
                | crate::model::EffectValue::Seed(_)
                | crate::model::EffectValue::Layer(_)
                | crate::model::EffectValue::Clip(_)
                | crate::model::EffectValue::MaskPath(_)
                | crate::model::EffectValue::Curve(_)
                | crate::model::EffectValue::Text(_) => {}
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
                | crate::model::EffectValue::Clip(_)
                | crate::model::EffectValue::MaskPath(_)
                | crate::model::EffectValue::Curve(_)
                | crate::model::EffectValue::Text(_) => {}
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

// --- Node groups ----------------------------------------------------------
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

/// A saved set of a layer graph's boxes and the wires between them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupPreset {
    pub format: u32,
    pub name: String,
    /// Which chip of the label palette the group wears, as an index.
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
    /// The stack's effects in the set, in stack order. They join the end of
    /// the stack they are dropped on, as an effect preset's do. Absent from a
    /// group of drivers alone, so a file saved before effects could be
    /// grouped reads and writes as it did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectInstance>,
    /// One position per effect, from the same corner as [`Self::layout`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect_layout: Vec<[f64; 2]>,
    /// The groups inside this one. A member is a place in `nodes`, counting
    /// on into `effects`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inner: Vec<InnerGroup>,
}

/// A group nested inside a saved one, naming its boxes by where they sit in
/// the file, since every id is minted afresh on the way back in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InnerGroup {
    pub name: String,
    #[serde(default)]
    pub colour: u32,
    pub members: Vec<usize>,
}

/// The groups of `all` that sit wholly inside the saved set without being the
/// whole of it. `saved` is the set's boxes in file order.
fn inner_groups<T: PartialEq>(
    saved: &[T],
    all: impl Iterator<Item = (String, u32, Vec<T>)>,
) -> Vec<InnerGroup> {
    all.filter_map(|(name, colour, members)| {
        let mut places: Vec<usize> = members
            .iter()
            .map(|m| saved.iter().position(|s| s == m))
            .collect::<Option<_>>()?;
        places.sort_unstable();
        places.dedup();
        (!places.is_empty() && places.len() < saved.len()).then_some(InnerGroup {
            name,
            colour,
            members: places,
        })
    })
    .collect()
}

/// The current on-disk format version for a group.
pub const GROUP_FORMAT: u32 = 1;

/// The file extension node groups use — a plain JSON document, beside the
/// `.lumfx` effect presets in the same per-user library.
pub const GROUP_EXTENSION: &str = "lumgrp";

/// Gather `members` out of `graph` and the stack it sits beside as a saveable
/// group.
///
/// The drivers and the stack's effects are taken. The Source and the Layer out
/// are not: every layer has its own, so there is nothing of them to carry.
///
/// Deterministic: the drivers come out in the graph's own order and the
/// effects in the stack's, never the selection's, so saving the same three
/// boxes twice writes the same file.
#[must_use]
pub fn group_from_graph(
    graph: &crate::graph::LayerGraph,
    stack: &[EffectInstance],
    name: &str,
    colour: u32,
    members: &[crate::graph::NodeRef],
) -> GroupPreset {
    use crate::graph::{InputRef, NodeRef, OutputRef};
    let wanted: std::collections::BTreeSet<NodeRef> = members
        .iter()
        .copied()
        .filter(|node| matches!(node, NodeRef::Driver(_) | NodeRef::Effect(_)))
        .collect();
    let taken = |from: &[EffectInstance], as_ref: fn(uuid::Uuid) -> NodeRef| {
        from.iter()
            .filter(|n| wanted.contains(&as_ref(n.id)))
            .cloned()
            .collect::<Vec<EffectInstance>>()
    };
    let nodes = taken(&graph.nodes, NodeRef::Driver);
    let effects = taken(stack, NodeRef::Effect);

    let at = |node: NodeRef| {
        graph
            .layout
            .iter()
            .find(|(placed, _)| *placed == node)
            .map_or([0.0, 0.0], |(_, xy)| *xy)
    };
    let places: Vec<[f64; 2]> = nodes.iter().map(|n| at(NodeRef::Driver(n.id))).collect();
    let effect_places: Vec<[f64; 2]> = effects.iter().map(|n| at(NodeRef::Effect(n.id))).collect();
    // The set's top-left corner. An empty set has no corner and no layout, so
    // the fold's identity never reaches the subtraction below.
    let origin = places
        .iter()
        .chain(&effect_places)
        .fold([f64::MAX, f64::MAX], |acc, p| {
            [acc[0].min(p[0]), acc[1].min(p[1])]
        });
    let from_corner = |places: &[[f64; 2]]| -> Vec<[f64; 2]> {
        places
            .iter()
            .map(|p| [p[0] - origin[0], p[1] - origin[1]])
            .collect()
    };

    let edges: Vec<crate::graph::Edge> = graph
        .edges
        .iter()
        .filter(|e| {
            let from = match &e.from {
                OutputRef::Driver { node, .. } => wanted.contains(&NodeRef::Driver(*node)),
                OutputRef::EffectData { effect, .. } => wanted.contains(&NodeRef::Effect(*effect)),
                OutputRef::SourceMatte | OutputRef::EffectOutput { .. } => false,
            };
            let to = match &e.to {
                InputRef::Param { node, .. } => wanted.contains(node),
                InputRef::Matte { .. } => false,
            };
            from && to
        })
        .cloned()
        .collect();

    let saved: Vec<NodeRef> = nodes
        .iter()
        .map(|n| NodeRef::Driver(n.id))
        .chain(effects.iter().map(|n| NodeRef::Effect(n.id)))
        .collect();
    let inner = inner_groups(
        &saved,
        graph
            .groups
            .iter()
            .map(|g| (g.name.clone(), g.colour, g.members.clone())),
    );

    GroupPreset {
        format: GROUP_FORMAT,
        name: name.to_owned(),
        colour,
        layout: from_corner(&places),
        effect_layout: from_corner(&effect_places),
        nodes,
        edges,
        effects,
        inner,
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

/// Everything one insert adds to a layer, handed over in a piece so the caller
/// extends its lists and commits once.
#[derive(Debug, Clone)]
pub struct InsertedGroup {
    /// What joins the end of the layer's effect stack.
    pub effects: Vec<EffectInstance>,
    pub nodes: Vec<EffectInstance>,
    pub edges: Vec<crate::graph::Edge>,
    pub layout: Vec<(crate::graph::NodeRef, [f64; 2])>,
    pub group: crate::graph::NodeGroup,
    /// The groups nested inside [`Self::group`].
    pub inner: Vec<crate::graph::NodeGroup>,
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
    use crate::graph::{InputRef, NodeRef, OutputRef};
    let mut fresh = std::collections::BTreeMap::new();
    let mut minted = |saved: &[EffectInstance]| -> Vec<EffectInstance> {
        saved
            .iter()
            .cloned()
            .map(|mut n| {
                let id = uuid::Uuid::now_v7();
                fresh.insert(n.id, id);
                n.id = id;
                n
            })
            .collect()
    };
    let nodes = minted(&preset.nodes);
    let effects = minted(&preset.effects);

    let edges: Vec<crate::graph::Edge> = preset
        .edges
        .iter()
        .filter_map(|e| {
            // A wire naming a node the file does not carry is dropped rather
            // than landing dangling: the graph would refuse the whole insert
            // for it, and one bad wire must not cost the rig.
            let from = match &e.from {
                OutputRef::Driver { node, port } => OutputRef::Driver {
                    node: *fresh.get(node)?,
                    port: port.clone(),
                },
                OutputRef::EffectData { effect, port } => OutputRef::EffectData {
                    effect: *fresh.get(effect)?,
                    port: port.clone(),
                },
                OutputRef::SourceMatte | OutputRef::EffectOutput { .. } => return None,
            };
            let to = match &e.to {
                InputRef::Param { node, port } => InputRef::Param {
                    node: match node {
                        NodeRef::Driver(id) => NodeRef::Driver(*fresh.get(id)?),
                        NodeRef::Effect(id) => NodeRef::Effect(*fresh.get(id)?),
                        NodeRef::Source | NodeRef::Out => return None,
                    },
                    port: port.clone(),
                },
                InputRef::Matte { .. } => return None,
            };
            Some(crate::graph::Edge { from, to })
        })
        .collect();

    let placed =
        |made: &[EffectInstance], saved: &[[f64; 2]], as_ref: fn(uuid::Uuid) -> NodeRef| {
            made.iter()
                .enumerate()
                .map(|(i, n)| {
                    let rel = saved.get(i).copied().unwrap_or([0.0, 0.0]);
                    (as_ref(n.id), [at[0] + rel[0], at[1] + rel[1]])
                })
                .collect::<Vec<(NodeRef, [f64; 2])>>()
        };
    let mut layout = placed(&nodes, &preset.layout, NodeRef::Driver);
    layout.extend(placed(&effects, &preset.effect_layout, NodeRef::Effect));

    let group = crate::graph::NodeGroup {
        name: preset.name.clone(),
        colour: preset.colour,
        members: layout.iter().map(|(node, _)| *node).collect(),
    };
    let inner = preset
        .inner
        .iter()
        .map(|g| crate::graph::NodeGroup {
            name: g.name.clone(),
            colour: g.colour,
            members: g
                .members
                .iter()
                .filter_map(|&i| group.members.get(i).copied())
                .collect(),
        })
        .collect();
    InsertedGroup {
        effects,
        nodes,
        edges,
        layout,
        group,
        inner,
    }
}

// --- Node graph groups ----------------------------------------------------
//
// **In plain terms.** The same idea as a node group, for the other canvas: pick
// a few boxes out of a node graph composition, name the set, and drop it into
// another graph later with its wires still on. What is saved is the boxes, the
// wires between them and where they sat relative to one another.
//
// A different extension from the layer graph's, so neither listing can offer
// the other's file: the two canvases hold different kinds of box and a group of
// one means nothing to the other.

/// A saved set of a node graph's boxes and the wires between them (§5.8).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompGroupPreset {
    pub format: u32,
    pub name: String,
    /// Which chip of the label palette the group wears, as an index.
    #[serde(default)]
    pub colour: u32,
    /// The boxes, in the order the graph carried them. **Never the Output**:
    /// every graph has exactly one and it cannot be deleted, so a second one
    /// arriving in a group would be a graph the validator refuses.
    pub nodes: Vec<crate::comp_graph::GraphNode>,
    /// The wires with both ends inside the set.
    #[serde(default)]
    pub edges: Vec<crate::comp_graph::GraphEdge>,
    /// One position per box, **relative to the set's top-left corner**, so a
    /// group dropped anywhere keeps the shape it was saved in.
    #[serde(default)]
    pub layout: Vec<[f64; 2]>,
    /// The groups inside this one. A member is a place in `nodes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inner: Vec<InnerGroup>,
}

/// The current on-disk format version for a node graph group.
pub const COMP_GROUP_FORMAT: u32 = 1;

/// The file extension node graph groups use - a plain JSON document, beside the
/// `.lumgrp` driver groups in the same per-user library.
pub const COMP_GROUP_EXTENSION: &str = "lumngrp";

/// Gather `members` out of `graph` as a saveable group.
///
/// Deterministic: the boxes come out in the graph's own order, never the
/// selection's, so saving the same three twice writes the same file.
#[must_use]
pub fn comp_group_from_graph(
    graph: &crate::comp_graph::CompGraph,
    name: &str,
    colour: u32,
    members: &[uuid::Uuid],
) -> CompGroupPreset {
    use crate::comp_graph::GraphNode;
    let wanted: Vec<uuid::Uuid> = graph
        .nodes
        .iter()
        .map(GraphNode::id)
        .filter(|id| members.contains(id))
        .filter(|id| !matches!(graph.node(*id), Some(GraphNode::Output { .. })))
        .collect();
    let nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| wanted.contains(&n.id()))
        .cloned()
        .collect();

    let at = |id: uuid::Uuid| {
        graph
            .layout
            .iter()
            .find(|(node, _)| *node == id)
            .map_or([0.0, 0.0], |(_, xy)| *xy)
    };
    let places: Vec<[f64; 2]> = nodes.iter().map(|n| at(n.id())).collect();
    // The set's top-left corner. An empty set has no corner and no layout, so
    // the fold's identity never reaches the subtraction below.
    let origin = places.iter().fold([f64::MAX, f64::MAX], |acc, p| {
        [acc[0].min(p[0]), acc[1].min(p[1])]
    });
    let layout: Vec<[f64; 2]> = places
        .iter()
        .map(|p| [p[0] - origin[0], p[1] - origin[1]])
        .collect();

    CompGroupPreset {
        format: COMP_GROUP_FORMAT,
        name: name.to_owned(),
        colour,
        inner: inner_groups(
            &wanted,
            graph
                .groups
                .iter()
                .map(|g| (g.name.clone(), g.colour, g.members.clone())),
        ),
        edges: graph
            .edges
            .iter()
            .filter(|e| wanted.contains(&e.from) && wanted.contains(&e.to))
            .cloned()
            .collect(),
        nodes,
        layout,
    }
}

/// Serialise a node graph group to its JSON text.
pub fn comp_group_to_json(preset: &CompGroupPreset) -> Result<String, String> {
    serde_json::to_string_pretty(preset).map_err(|e| e.to_string())
}

/// Parse node graph group JSON back. A newer `format` still loads, for the
/// reason [`from_json`] gives.
pub fn comp_group_from_json(text: &str) -> Result<CompGroupPreset, String> {
    serde_json::from_str::<CompGroupPreset>(text).map_err(|e| e.to_string())
}

/// Everything one insert adds to a node graph - handed over in a piece so the
/// caller extends four lists and commits once.
#[derive(Debug, Clone)]
pub struct InsertedCompGroup {
    pub nodes: Vec<crate::comp_graph::GraphNode>,
    pub edges: Vec<crate::comp_graph::GraphEdge>,
    pub layout: Vec<(uuid::Uuid, [f64; 2])>,
    pub group: crate::comp_graph::GraphGroup,
    /// The groups nested inside [`Self::group`].
    pub inner: Vec<crate::comp_graph::GraphGroup>,
}

/// What inserting `preset` at canvas point `at` adds to a graph: fresh boxes,
/// the wires between them re-pointed at those fresh ids, where each one sits,
/// and the group that names them.
///
/// Every id is minted here, so inserting one group twice never makes two boxes
/// share an id - the rule [`instantiated`] follows for an effect preset, and an
/// Fx box takes a fresh instance id for the same reason. A Read keeps its item:
/// the item is the project's, not the group's, and a Read of one this project
/// lacks lands wearing the missing mark, which is how a deleted item already
/// reads.
#[must_use]
pub fn comp_group_instantiated(preset: &CompGroupPreset, at: [f64; 2]) -> InsertedCompGroup {
    use crate::comp_graph::{GraphEdge, GraphGroup, GraphNode};
    let mut fresh = std::collections::BTreeMap::new();
    let nodes: Vec<GraphNode> = preset
        .nodes
        .iter()
        .cloned()
        .map(|node| {
            let id = uuid::Uuid::now_v7();
            fresh.insert(node.id(), id);
            match node {
                GraphNode::Read {
                    item, custom_name, ..
                } => GraphNode::Read {
                    id,
                    item,
                    custom_name,
                },
                GraphNode::Input { input, .. } => GraphNode::Input { id, input },
                GraphNode::Output { .. } => GraphNode::Output { id },
                GraphNode::Fx(mut inst) => {
                    inst.id = id;
                    GraphNode::Fx(inst)
                }
            }
        })
        .collect();

    let edges: Vec<GraphEdge> = preset
        .edges
        .iter()
        .filter_map(|e| {
            // A wire naming a box the file does not carry is dropped rather
            // than landing dangling: the graph would refuse the whole insert
            // for it, and one bad wire must not cost the rig.
            Some(GraphEdge {
                from: *fresh.get(&e.from)?,
                from_port: e.from_port.clone(),
                to: *fresh.get(&e.to)?,
                to_port: e.to_port.clone(),
            })
        })
        .collect();

    let layout: Vec<(uuid::Uuid, [f64; 2])> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let rel = preset.layout.get(i).copied().unwrap_or([0.0, 0.0]);
            (n.id(), [at[0] + rel[0], at[1] + rel[1]])
        })
        .collect();

    let group = GraphGroup {
        name: preset.name.clone(),
        colour: preset.colour,
        members: nodes.iter().map(GraphNode::id).collect(),
    };
    let inner = preset
        .inner
        .iter()
        .map(|g| GraphGroup {
            name: g.name.clone(),
            colour: g.colour,
            members: g
                .members
                .iter()
                .filter_map(|&i| group.members.get(i).copied())
                .collect(),
        })
        .collect();
    InsertedCompGroup {
        nodes,
        edges,
        layout,
        group,
        inner,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn stack() -> Vec<EffectInstance> {
        vec![
            crate::fx::builtins::instantiate("blur").unwrap(),
            crate::fx::builtins::instantiate("glow").unwrap(),
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
    fn a_newer_format_still_loads() {
        // A preset written by a hypothetical newer Lumit, with an unknown
        // top-level field, still parses — serde ignores what it doesn't know.
        let effects = stack();
        let mut v = serde_json::to_value(EffectPreset {
            format: 99,
            name: "future".into(),
            effects: effects.clone(),
            wiring: None,
        })
        .unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("future_field".into(), serde_json::json!(true));
        let back = from_json(&v.to_string()).unwrap();
        assert_eq!(back.effects, effects);
    }

    // --- Node groups ------------------------------------------------------

    /// Two drivers wired to each other and on into a blur in the stack, a
    /// third outside them, and one wire leaving the set: the shape every
    /// group claim is made against.
    fn wired_graph() -> (
        crate::graph::LayerGraph,
        Vec<EffectInstance>,
        Vec<crate::graph::NodeRef>,
    ) {
        use crate::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
        let wiggle = crate::fx::builtins::instantiate("wiggle").unwrap();
        let smooth = crate::fx::builtins::instantiate("smooth").unwrap();
        let outside = crate::fx::builtins::instantiate("wiggle").unwrap();
        let blur = crate::fx::builtins::instantiate("blur").unwrap();
        let (a, b, c, d) = (wiggle.id, smooth.id, outside.id, blur.id);
        let graph = LayerGraph {
            notes: Vec::new(),
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
                // Out of the set, into the box left behind.
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
                // Inside the set, into the stack's blur.
                Edge {
                    from: OutputRef::Driver {
                        node: b,
                        port: "value".into(),
                    },
                    to: InputRef::Param {
                        node: NodeRef::Effect(d),
                        port: "radius".into(),
                    },
                },
            ],
            layout: vec![
                (NodeRef::Driver(a), [100.0, 60.0]),
                (NodeRef::Driver(b), [260.0, 90.0]),
                (NodeRef::Driver(c), [500.0, 300.0]),
                (NodeRef::Effect(d), [420.0, 60.0]),
            ],
            exposed: Vec::new(),
            // The two drivers are a group of their own, inside the set.
            groups: vec![crate::graph::NodeGroup {
                name: "Pair".into(),
                colour: 2,
                members: vec![NodeRef::Driver(a), NodeRef::Driver(b)],
            }],
        };
        let members = vec![NodeRef::Driver(a), NodeRef::Driver(b), NodeRef::Effect(d)];
        (graph, vec![blur], members)
    }

    /// Copying the blur takes the smooth that feeds it and the
    /// wiggle that feeds the smooth, with the two wires between them, and
    /// leaves the third driver and the wire to it behind; the wiring rides
    /// in the copied text and comes back with every id fresh. Copying
    /// nothing wired stays a plain preset. Guards effects that lose their
    /// drivers when copied to another layer.
    #[test]
    fn copied_effects_take_the_drivers_that_feed_them() {
        use crate::graph::{InputRef, NodeRef, OutputRef};
        let (graph, stack, _) = wired_graph();
        let blur = stack[0].id;
        let wiring = wiring_of(&graph, &stack, &[blur]).unwrap();
        let taken: Vec<_> = wiring.nodes.iter().map(|n| n.id).collect();
        assert_eq!(taken, vec![graph.nodes[0].id, graph.nodes[1].id]);
        assert_eq!(wiring.edges.len(), 2);
        assert_eq!(wiring.effects.len(), 1);

        let text = to_json_wired("look", &stack, Some(wiring)).unwrap();
        let back = from_json(&text).unwrap();
        let added = group_instantiated(back.wiring.as_ref().unwrap(), [0.0, 0.0]);
        assert_ne!(added.effects[0].id, blur);
        let into_blur = added.edges.iter().find(|e| {
            matches!(&e.to, InputRef::Param { node: NodeRef::Effect(id), .. } if *id == added.effects[0].id)
        });
        assert!(
            matches!(into_blur.map(|e| &e.from), Some(OutputRef::Driver { node, .. }) if *node == added.nodes[1].id),
            "the fresh smooth feeds the fresh blur"
        );

        let plain = vec![crate::fx::builtins::instantiate("glow").unwrap()];
        assert!(wiring_of(&graph, &plain, &[plain[0].id]).is_none());
        assert!(from_json(&to_json("x", &plain).unwrap())
            .unwrap()
            .wiring
            .is_none());
    }

    #[test]
    fn inserting_a_group_mints_fresh_ids_and_re_points_the_wires() {
        let (graph, stack, members) = wired_graph();
        let preset = group_from_json(
            &group_to_json(&group_from_graph(&graph, &stack, "Audio rig", 4, &members)).unwrap(),
        )
        .unwrap();

        let added = group_instantiated(&preset, [400.0, 200.0]);
        let (nodes, edges, layout, group) = (added.nodes, added.edges, added.layout, added.group);
        assert_eq!(nodes.len(), 2);
        // The blur came too, as a fresh effect for the stack, still driven.
        assert_eq!(added.effects.len(), 1);
        assert_ne!(added.effects[0].id, stack[0].id);
        assert_eq!(
            edges[1].to,
            crate::graph::InputRef::Param {
                node: crate::graph::NodeRef::Effect(added.effects[0].id),
                port: "radius".into(),
            }
        );
        assert_eq!(layout[2].1, [720.0, 200.0]);
        assert!(
            nodes
                .iter()
                .all(|n| !preset.nodes.iter().any(|s| s.id == n.id)),
            "every instance id is fresh, so inserting twice never shares one"
        );
        // The wires came back pointing at the *new* boxes, not the saved ones.
        assert_eq!(edges.len(), 2);
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
        assert_eq!(group.members.len(), 3);
        // The group inside it came back round the two fresh drivers.
        assert_eq!(added.inner.len(), 1);
        assert_eq!(added.inner[0].name, "Pair");
        assert_eq!(
            added.inner[0].members,
            vec![
                crate::graph::NodeRef::Driver(nodes[0].id),
                crate::graph::NodeRef::Driver(nodes[1].id),
            ]
        );
    }

    // -- 5.8, a node graph's groups -------------------------------------------

    /// A Read, a blur and an Output, wired in a line, with the first two
    /// picked. The Output is picked too, to prove it is never saved.
    fn wired_comp_graph() -> (crate::comp_graph::CompGraph, Vec<uuid::Uuid>, uuid::Uuid) {
        use crate::comp_graph::{CompGraph, GraphEdge, GraphNode};
        use crate::graph::{INPUT_PORT, OUTPUT_PORT};

        let item = uuid::Uuid::now_v7();
        let read = GraphNode::Read {
            id: uuid::Uuid::now_v7(),
            item,
            custom_name: None,
        };
        let blur = GraphNode::Fx(crate::fx::builtins::instantiate("blur").unwrap());
        let out = GraphNode::Output {
            id: uuid::Uuid::now_v7(),
        };
        let wire = |from: &GraphNode, to: &GraphNode| GraphEdge {
            from: from.id(),
            from_port: OUTPUT_PORT.id.to_owned(),
            to: to.id(),
            to_port: INPUT_PORT.id.to_owned(),
        };
        let members = vec![read.id(), blur.id(), out.id()];
        let graph = CompGraph {
            notes: Vec::new(),
            edges: vec![wire(&read, &blur), wire(&blur, &out)],
            layout: vec![
                (read.id(), [100.0, 60.0]),
                (blur.id(), [260.0, 90.0]),
                (out.id(), [500.0, 300.0]),
            ],
            nodes: vec![read, blur, out],
            exposed: Vec::new(),
            groups: Vec::new(),
        };
        (graph, members, item)
    }

    #[test]
    fn inserting_a_comp_group_mints_fresh_ids_and_keeps_a_reads_item() {
        use crate::comp_graph::GraphNode;

        let (graph, members, item) = wired_comp_graph();
        let mut preset = comp_group_from_json(
            &comp_group_to_json(&comp_group_from_graph(&graph, "Plate rig", 4, &members)).unwrap(),
        )
        .unwrap();
        // A wire naming a box the file does not carry is dropped rather than
        // landing dangling, which would cost the whole insert.
        preset.edges.push(crate::comp_graph::GraphEdge {
            from: uuid::Uuid::now_v7(),
            from_port: "output".into(),
            to: preset.nodes[1].id(),
            to_port: "input".into(),
        });

        let added = comp_group_instantiated(&preset, [400.0, 200.0]);
        assert_eq!(added.nodes.len(), 2);
        assert!(
            added
                .nodes
                .iter()
                .all(|n| !preset.nodes.iter().any(|s| s.id() == n.id())),
            "every id is fresh, so inserting twice never makes two boxes one"
        );
        // A Read keeps the item it names: the item is the project's, not the
        // group's, and one this project lacks wears the missing mark.
        match &added.nodes[0] {
            GraphNode::Read { item: named, .. } => assert_eq!(*named, item),
            other => panic!("the Read came back as {other:?}"),
        }
        assert_eq!(added.edges.len(), 1, "the dangling wire was dropped");
        assert_eq!(added.edges[0].from, added.nodes[0].id());
        assert_eq!(added.edges[0].to, added.nodes[1].id());
        assert_eq!(added.layout[0].1, [400.0, 200.0]);
        assert_eq!(added.layout[1].1, [560.0, 230.0]);
        assert_eq!(added.group.name, "Plate rig");
        assert_eq!(added.group.members.len(), 2);

        // A second insert shares nothing with the first.
        let again = comp_group_instantiated(&preset, [0.0, 0.0]);
        assert!(added
            .nodes
            .iter()
            .all(|a| !again.nodes.iter().any(|b| b.id() == a.id())));
    }
}

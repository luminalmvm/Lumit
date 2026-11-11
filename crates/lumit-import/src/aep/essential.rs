//! Essential Graphics in the project file: a composition's controllers, and
//! what a layer placing it has set for itself.
//!
//! # In plain terms
//!
//! After Effects lets a composition offer some of its properties as
//! controllers, and a layer placing that composition can then set each one
//! for itself. The file keeps the two halves apart.
//!
//! **The controllers** sit in the composition's own item, in `LIST CIF3` (and
//! in `CIF2` and `CIFO` beside it, the same list as older versions wrote it):
//!
//! | chunk | holds |
//! |---|---|
//! | `LIST CpS2` ▸ `Utf8` | the template's name |
//! | `CcCt` | how many controllers follow (u32) |
//! | `LIST CCtl` | one controller |
//!
//! and inside each `LIST CCtl`:
//!
//! | chunk | holds |
//! |---|---|
//! | `LIST CpS2` ▸ `Utf8` | its name |
//! | `Utf8` | its id, a GUID as text |
//! | `CTyp` | its kind (u32): 1 checkbox, 2 slider, 3 angle, 4 colour, 5 point, 6 text, 8 comment, 9 scale, 13 menu, 14 replaceable media |
//! | `Smin`, `Smax` | a slider's travel (f64 each) |
//! | `LIST StVc` ▸ `Utf8`… | a menu's options |
//! | `Utf8` of JSON with `capPropFontEdit` | whether a text controller lets the font and the size be changed |
//! | `LIST CPrp` | the source: `CCId` its composition's item id, `CLId` its layer's id, and a `Utf8` of JSON listing the match names from the layer down |
//!
//! `CVal` and `CDef` hold the value and the default again. They are not read:
//! the source property has the value.
//!
//! **What a layer set** sits in that layer's property tree under the match
//! name `ADBE Layer Overrides`: a `LIST OvG2` naming the controllers by id in
//! order, then a `LIST tdgp` holding an ordinary property for each. A property
//! the layer set itself has the low bit of its `tdsb` set. Replaceable media
//! keeps the item the layer put there in `blsi`.
//!
//! All of it was read off projects saved by After Effects 2026 beside what
//! its scripting said of the same projects. Two things were not: no project
//! with a **group** was to hand, as scripting cannot make one, so a group is
//! read on the guess that it is a controller holding controllers; and kinds 7,
//! 10, 11 and 12 were never seen, and arrive as their number.

use std::collections::HashMap;

use super::props::{self, Ctx};
use super::rifx::{bit, text_of, u32_at, u8_at, Chunk};
use crate::capture::{Comp, Controller, Essential, Item, Property, Step, Unreadable};

/// The match name a layer's Essential Properties sit under.
pub(super) const OVERRIDES: &str = "ADBE Layer Overrides";
/// The match name of a replaceable media property.
const ALTERNATE: &str = "ADBE Layer Source Alternate";
/// What a path step's index holds when its parent is not a list.
const NO_INDEX: u64 = 0xFFFF_FFFF;

/// A composition's Essential Graphics out of its item's chunks, or `None`
/// for a composition with no controllers. `layers` turns a layer id into its
/// stacking index.
pub(super) fn read(inside: &[Chunk<'_>], layers: &HashMap<u32, u32>) -> Option<Essential> {
    // The newest list the file has.
    let list = [b"CIF3", b"CIF2", b"CIFO"]
        .into_iter()
        .find_map(|kind| inside.iter().find(|chunk| chunk.is_list(kind)))?;
    let within: Vec<Chunk<'_>> = list.children().ok().collect();
    let mut controllers = Vec::new();
    for chunk in within.iter().filter(|chunk| chunk.is_list(b"CCtl")) {
        controller(chunk, None, layers, &mut controllers);
    }
    if controllers.is_empty() {
        return None;
    }
    Some(Essential {
        name: caption(&within),
        controllers,
    })
}

/// The name in a `LIST CpS2`.
fn caption(within: &[Chunk<'_>]) -> Option<String> {
    within
        .iter()
        .find(|chunk| chunk.is_list(b"CpS2"))?
        .children()
        .ok()
        .find(|chunk| chunk.id == *b"Utf8")
        .map(|chunk| chunk.text())
}

/// One `LIST CCtl`, and the controllers inside it when it is a group.
fn controller(
    chunk: &Chunk<'_>,
    group: Option<&str>,
    layers: &HashMap<u32, u32>,
    out: &mut Vec<Controller>,
) {
    let within: Vec<Chunk<'_>> = chunk.children().ok().collect();
    let name = caption(&within);
    let members: Vec<&Chunk<'_>> = within.iter().filter(|c| c.is_list(b"CCtl")).collect();
    if !members.is_empty() {
        for member in members {
            controller(member, name.as_deref(), layers, out);
        }
        return;
    }

    let texts: Vec<String> = within
        .iter()
        .filter(|c| c.id == *b"Utf8")
        .map(Chunk::text)
        .collect();
    let number = |id: &[u8; 4]| {
        let body = within.iter().find(|c| c.id == *id)?.body;
        Some(f64::from_be_bytes(body.get(..8)?.try_into().ok()?))
    };
    let code = within
        .iter()
        .find(|c| c.id == *b"CTyp")
        .and_then(|c| u32_at(c.body, 0));
    let flags: Option<serde_json::Value> = texts
        .iter()
        .filter(|text| text.contains("capPropFontEdit"))
        .find_map(|text| serde_json::from_str(text).ok());
    let flag = |key: &str| flags.as_ref()?.get(key)?.as_bool();
    let source = within.iter().find(|c| c.is_list(b"CPrp")).map(source_of);
    let (comp_id, layer_id, path) = source.unwrap_or_default();

    out.push(Controller {
        id: texts.first().cloned(),
        name,
        kind: code.map(kind_of),
        comp_id,
        layer_index: layer_id.and_then(|id| layers.get(&id).copied()),
        path,
        range: number(b"Smin").zip(number(b"Smax")),
        options: within
            .iter()
            .find(|c| c.is_list(b"StVc"))
            .map(|list| {
                list.children()
                    .ok()
                    .filter(|c| c.id == *b"Utf8")
                    .map(|c| c.text())
                    .collect()
            })
            .unwrap_or_default(),
        font: flag("capPropFontEdit"),
        size: flag("capPropFontSizeEdit"),
        group: group.map(str::to_string),
    });
}

/// A controller's kind by the name the capture uses, or the file's number
/// written out for one this reader has not met.
fn kind_of(code: u32) -> String {
    match code {
        1 => "checkbox",
        2 => "slider",
        3 => "angle",
        4 => "colour",
        5 => "point",
        6 => "text",
        8 => "comment",
        9 => "scale",
        // Never seen in a file. A group's kind in an exported template.
        10 => "group",
        13 => "menu",
        14 => "media",
        other => return other.to_string(),
    }
    .to_string()
}

/// A `LIST CPrp`: the source's composition, its layer's id, and the path from
/// the layer down.
fn source_of(chunk: &Chunk<'_>) -> (Option<i64>, Option<u32>, Vec<Step>) {
    let within: Vec<Chunk<'_>> = chunk.children().ok().collect();
    let word = |id: &[u8; 4]| {
        within
            .iter()
            .find(|c| c.id == *id)
            .and_then(|c| u32_at(c.body, 0))
    };
    // The steps are an object keyed "0", "1", "2".
    let steps: Option<serde_json::Value> = within
        .iter()
        .find(|c| c.id == *b"Utf8")
        .and_then(|c| serde_json::from_str(&c.text()).ok());
    let mut path = Vec::new();
    while let Some(step) = steps.as_ref().and_then(|s| s.get(path.len().to_string())) {
        path.push(Step {
            match_name: step
                .get("matchName")
                .and_then(|name| name.as_str())
                .map(str::to_string),
            index: step
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .filter(|index| *index != NO_INDEX)
                .and_then(|index| u32::try_from(index).ok()),
        });
    }
    (word(b"CCId").map(i64::from), word(b"CLId"), path)
}

/// A layer's Essential Properties: the `ADBE Layer Overrides` run, whose
/// `LIST OvG2` names each controller and whose `LIST tdgp` holds a property
/// for each, in the same order.
///
/// The values are read as they are stored. One kept as a share of a layer is
/// a share of the layer the controller's source is on, in another
/// composition, so [`resolve`] sizes those once every composition is read.
pub(super) fn read_overrides(
    names: &Chunk<'_>,
    group: &Chunk<'_>,
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let ids: Vec<String> = names
        .children()
        .ok()
        .filter(|chunk| chunk.is_list(b"CPrp"))
        .filter_map(|chunk| chunk.children().ok().find(|c| c.id == *b"Utf8"))
        .map(|chunk| chunk.text())
        .collect();
    let plain = Ctx {
        params: None,
        has_source: false,
        in_effect: false,
        ..ctx
    };
    let mut leaves = Vec::new();
    for (at, (match_name, chunks)) in props::runs(group).into_iter().enumerate() {
        let Some((mut node, _)) = props::read_node(&match_name, &chunks, plain, skipped) else {
            continue;
        };
        node.controller_id = ids.get(at).cloned();
        if match_name == ALTERNATE {
            let item = chunks
                .iter()
                .find(|chunk| chunk.id == *b"blsi")
                .and_then(|chunk| u32_at(chunk.body, 0))
                .filter(|id| *id != 0);
            node.alternate_id = item.map(i64::from);
            node.overridden = Some(item.is_some());
        } else {
            node.overridden = Some(set_by_layer(&chunks));
        }
        leaves.push(node);
    }
    Property {
        match_name: Some(OVERRIDES.to_string()),
        enabled: Some(true),
        group: Some(leaves),
        ..Property::default()
    }
}

/// Whether the layer set this property itself: the low bit of the `tdsb` in
/// its `LIST tdbs`, which a text property keeps one list further in.
fn set_by_layer(chunks: &[Chunk<'_>]) -> bool {
    fn tdbs<'a>(chunks: impl Iterator<Item = Chunk<'a>>, depth: u8) -> Option<Chunk<'a>> {
        for chunk in chunks {
            if chunk.is_list(b"tdbs") {
                return Some(chunk);
            }
            if depth > 0 && chunk.list_type.is_some() {
                if let Some(found) = tdbs(chunk.children().ok(), depth - 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    tdbs(chunks.iter().copied(), 1)
        .and_then(|list| list.children().ok().find(|chunk| chunk.id == *b"tdsb"))
        .and_then(|chunk| u8_at(chunk.body, 3))
        .is_some_and(|flags| bit(flags, 0))
}

/// Put the Essential Property values that the file keeps as a share of a
/// layer back into pixels.
///
/// An effect's point and an anchor point are shares of the layer the
/// controller's source is on. That layer is in the composition being placed,
/// so its size is only known once every composition has been read.
pub(super) fn resolve(comps: &mut [Comp], items: &[Item]) {
    // Each composition's controllers by id, with the size their values are a
    // share of, where they are one.
    let mut sizes: HashMap<(i64, String), (f64, f64)> = HashMap::new();
    for comp in comps.iter() {
        let (Some(comp_id), Some(essential)) = (comp.id, comp.essential.as_ref()) else {
            continue;
        };
        for controller in &essential.controllers {
            let first = controller
                .path
                .first()
                .and_then(|s| s.match_name.as_deref());
            let last = controller.path.last().and_then(|s| s.match_name.as_deref());
            let layer = comp
                .layers
                .iter()
                .find(|layer| layer.index.is_some() && layer.index == controller.layer_index);
            let source = layer.and_then(|layer| layer.source_id);
            let shared = match (first, last) {
                (Some("ADBE Effect Parade"), _) => true,
                (_, Some("ADBE Anchor Point")) => source.is_some(),
                _ => false,
            };
            let (Some(id), true) = (controller.id.clone(), shared) else {
                continue;
            };
            let own = source.and_then(|source| size_of(source, comps, items));
            let whole = (
                f64::from(comp.width.unwrap_or_default()),
                f64::from(comp.height.unwrap_or_default()),
            );
            sizes.insert((comp_id, id), own.unwrap_or(whole));
        }
    }
    if sizes.is_empty() {
        return;
    }
    for comp in comps.iter_mut() {
        for layer in &mut comp.layers {
            let Some(source) = layer.source_id else {
                continue;
            };
            let leaves = layer
                .properties
                .iter_mut()
                .find(|node| node.match_name.as_deref() == Some(OVERRIDES))
                .and_then(|node| node.group.as_mut());
            for leaf in leaves.into_iter().flatten() {
                let size = leaf
                    .controller_id
                    .clone()
                    .and_then(|id| sizes.get(&(source, id)));
                // A point only: a slider of the same effect is a plain number.
                if let (Some(size), Some("point" | "point3")) = (size, leaf.value_type.as_deref()) {
                    sized(leaf, *size);
                }
            }
        }
    }
}

/// The size of the item a layer draws from.
fn size_of(source: i64, comps: &[Comp], items: &[Item]) -> Option<(f64, f64)> {
    let (width, height) = match comps.iter().find(|comp| comp.id == Some(source)) {
        Some(comp) => (comp.width?, comp.height?),
        None => {
            let item = items.iter().find(|item| item.id == Some(source))?;
            (item.width?, item.height?)
        }
    };
    Some((f64::from(width), f64::from(height)))
}

/// Scale a point's value and its keys across and down.
// ponytail: a keyed point's ease speeds and tangents stay in shares of the
// layer. Scale them here when a keyed effect point on a copy is met.
fn sized(leaf: &mut Property, (width, height): (f64, f64)) {
    let scale = |value: &mut serde_json::Value| {
        if let Some(axes) = value.as_array_mut() {
            for (axis, by) in axes.iter_mut().zip([width, height]) {
                if let Some(v) = axis.as_f64() {
                    *axis = serde_json::json!(v * by);
                }
            }
        }
    };
    if let Some(value) = leaf.value.as_mut() {
        scale(value);
    }
    for key in leaf.keyframes.iter_mut().flatten() {
        if let Some(value) = key.v.as_mut() {
            scale(value);
        }
    }
}

/// A popup's options out of the `pdnm` after its definition: a `Utf8` of the
/// names with a bar between them.
pub(super) fn options_of(pdnm: &Chunk<'_>) -> Vec<String> {
    let text = pdnm
        .children()
        .ok()
        .find(|chunk| chunk.id == *b"Utf8")
        .map_or_else(|| text_of(pdnm.body), |chunk| chunk.text());
    text.split('|')
        .map(str::to_string)
        .filter(|option| !option.is_empty())
        .collect()
}

//! Turning a layered image file into a composition.
//!
//! # In plain terms
//!
//! A Photoshop document comes in as a folder of footage items, one per layer,
//! and a composition the size of the document holding a Footage layer for
//! each. Every layer is read at the document's size, so each one sits where it
//! sat in Photoshop with a centred transform and nothing else.
//!
//! Pure, like `edits`: it returns ops and commits nothing.

use std::path::Path;

use lumit_core::anim::Property;
use lumit_core::group::LayerGroup;
use lumit_core::model::{BlendMode, Folder, FootageItem, LayerKind, MediaRef, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::time::Rational;
use lumit_media::psd::{PsdDocument, Section};
use uuid::Uuid;

/// Lumit's blend mode for one of Photoshop's four-letter keys. Dissolve, and
/// any key newer than this table, comes in as Normal.
fn blend_of(key: [u8; 4]) -> BlendMode {
    match &key {
        b"dark" => BlendMode::Darken,
        b"mul " => BlendMode::Multiply,
        b"idiv" => BlendMode::ColourBurn,
        b"lbrn" => BlendMode::LinearBurn,
        b"dkCl" => BlendMode::DarkerColour,
        b"lddg" => BlendMode::Add,
        b"lite" => BlendMode::Lighten,
        b"scrn" => BlendMode::Screen,
        b"div " => BlendMode::ColourDodge,
        b"lgCl" => BlendMode::LighterColour,
        b"over" => BlendMode::Overlay,
        b"sLit" => BlendMode::SoftLight,
        b"hLit" => BlendMode::HardLight,
        b"lLit" => BlendMode::LinearLight,
        b"vLit" => BlendMode::VividLight,
        b"pLit" => BlendMode::PinLight,
        b"hMix" => BlendMode::HardMix,
        b"diff" => BlendMode::Difference,
        b"smud" => BlendMode::Exclusion,
        b"fsub" => BlendMode::Subtract,
        b"fdiv" => BlendMode::Divide,
        b"hue " => BlendMode::Hue,
        b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Colour,
        b"lum " => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// The ops that fill `comp` with `psd`'s layers, and how many records were
/// left out because they hold no picture: an adjustment layer, a fill layer,
/// an empty layer.
///
/// `comp` is new and empty. `comp_size` and `out` are its size and length, and
/// `first_index` is where the next project item goes.
// ponytail: a group inside a group folds into the outer one, and a group's own
// opacity and blend mode are dropped. Layer styles are not read at all.
pub(crate) fn psd_ops(
    psd: &PsdDocument,
    path: &Path,
    comp: Uuid,
    comp_size: (u32, u32),
    out: Rational,
    first_index: usize,
) -> (Vec<Op>, u32) {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut items = Vec::new();
    let mut layers = Vec::new();
    let mut groups: Vec<LayerGroup> = Vec::new();
    // Whether each group the walk is inside is switched on.
    let mut inside: Vec<bool> = Vec::new();
    let mut left_out = 0u32;

    // The file lists its layers bottom first, and a comp lists them top first.
    for (index, record) in psd.layers.iter().enumerate().rev() {
        match record.section {
            Section::Group => {
                if inside.is_empty() {
                    groups.push(LayerGroup {
                        id: Uuid::now_v7(),
                        name: record.name.clone(),
                        label: 0,
                        members: Vec::new(),
                        effects: Vec::new(),
                    });
                }
                inside.push(record.visible);
            }
            Section::GroupEnd => {
                inside.pop();
            }
            Section::Layer if !record.has_pixels() => left_out += 1,
            Section::Layer => {
                let item = FootageItem {
                    id: Uuid::now_v7(),
                    name: format!("{}/{file_name}", record.name),
                    media: MediaRef {
                        relative_path: file_name.clone(),
                        absolute_path: path.to_string_lossy().into_owned(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    colour_space: None,
                    sequence: None,
                    source_layer: u32::try_from(index).ok(),
                    extra: serde_json::Map::new(),
                };
                let mut layer = crate::edits::base_layer(
                    record.name.clone(),
                    LayerKind::Footage { item: item.id },
                    out,
                    crate::edits::centred_transform(
                        f64::from(psd.width),
                        f64::from(psd.height),
                        comp_size.0,
                        comp_size.1,
                    ),
                );
                layer.transform.opacity = Property::fixed(f64::from(record.opacity) / 2.55);
                layer.blend = blend_of(record.blend);
                layer.switches.visible = record.visible && inside.iter().all(|on| *on);
                if !inside.is_empty() {
                    if let Some(group) = groups.last_mut() {
                        group.members.push(layer.id);
                    }
                }
                items.push(item);
                layers.push(layer);
            }
        }
    }

    let mut ops = Vec::new();
    let folder = Uuid::now_v7();
    let children: Vec<Uuid> = items.iter().map(|item| item.id).collect();
    for (at, item) in items.into_iter().enumerate() {
        ops.push(Op::AddItem {
            index: first_index + at,
            item: Box::new(ProjectItem::Footage(item)),
        });
    }
    ops.push(Op::AddItem {
        index: first_index + children.len(),
        item: Box::new(ProjectItem::Folder(Folder {
            id: folder,
            name: format!("{stem} layers"),
            children: Vec::new(),
            extra: serde_json::Map::new(),
        })),
    });
    ops.push(Op::SetFolderChildren { folder, children });
    for (index, layer) in layers.into_iter().enumerate() {
        ops.push(Op::AddLayer {
            comp,
            index,
            layer: Box::new(layer),
        });
    }
    // A group whose every member was left out has nothing to hold.
    for (index, group) in groups
        .into_iter()
        .filter(|group| !group.members.is_empty())
        .enumerate()
    {
        ops.push(Op::GroupLayers {
            comp,
            index,
            group: Box::new(group),
        });
    }
    (ops, left_out)
}

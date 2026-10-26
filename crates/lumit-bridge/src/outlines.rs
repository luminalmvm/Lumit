//! Shape layers made from another layer's outlines: Create ▸ Shapes from
//! text, Shapes from vector layer, and Split into shape layers.

use lumit_core::model::{Layer, LayerKind, TextDocument};
use lumit_core::shape::modifier::ShapeEntry;
use uuid::Uuid;

use crate::api::assets::shift_property;
use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// Add `contents` as a shape layer directly above `source`, or one for each
/// part when `apart`, and switch `source` off, as After Effects does. One
/// `Batch`, so one undo step. Hands back the topmost new layer.
///
/// The art is in `source`'s own layer pixels, whose corner is `origin`, read
/// at layer time `t`.
pub(crate) fn add_above(
    source: &LayerReference,
    contents: Vec<ShapeEntry>,
    origin: (f64, f64),
    apart: bool,
    t: f64,
) -> Result<LayerReference, BridgeError> {
    let comp = source.composition()?;
    let layer = source.item()?;
    let index = comp
        .layers
        .iter()
        .position(|l| l.id == source.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;
    let parts = if apart {
        parts(&contents)
    } else {
        vec![contents]
    };

    let mut ops = Vec::new();
    let mut top = source.layer_id;
    for (n, part) in parts.into_iter().enumerate() {
        let mut copy = layer.clone();
        copy.id = Uuid::now_v7();
        copy.name = match (apart, part.first()) {
            (false, _) => format!("{} outlines", layer.name),
            (true, Some(ShapeEntry::Group(group))) => group.name.clone(),
            (true, _) => format!("{} {}", layer.name, n + 1),
        };
        for effect in &mut copy.effects {
            effect.id = Uuid::now_v7();
        }
        // Masks and paint are drawn from the layer's box corner, and a shape
        // layer's corner is its art's box, so they would land somewhere else.
        copy.masks.clear();
        copy.paint.clear();
        // The anchor moves by that corner too, so the art sits where it was.
        if let Some((x0, y0, _, _)) = lumit_core::shape::contents_bounds(&part, t) {
            shift_property(&mut copy.transform.anchor_x, origin.0 - x0);
            shift_property(&mut copy.transform.anchor_y, origin.1 - y0);
        }
        copy.kind = LayerKind::Shape { contents: part };
        crate::edits::solo_on_arrival(&mut copy, comp.layers.iter());
        top = copy.id;
        // Each part lands above the one before, so later art stays in front
        // as it was drawn.
        ops.push(lumit_core::Op::AddLayer {
            comp: source.comp_id,
            index,
            layer: Box::new(copy),
        });
    }
    if ops.is_empty() {
        return Err(BridgeError::NothingToConvert);
    }
    ops.push(lumit_core::Op::SetLayerVisible {
        comp: source.comp_id,
        layer: source.layer_id,
        visible: false,
    });
    source.commit(lumit_core::Op::Batch { ops })?;
    Ok(LayerReference::new(source.project_id, source.comp_id, top))
}

/// `contents` cut into one list for each piece of art at the top, an item
/// with the items combined into it, each with the modifiers after it that act
/// on it.
fn parts(contents: &[ShapeEntry]) -> Vec<Vec<ShapeEntry>> {
    let mut runs: Vec<(Vec<ShapeEntry>, usize)> = Vec::new();
    for (i, entry) in contents.iter().enumerate() {
        let joins = matches!(entry, ShapeEntry::Item(item) if item.combine != 0)
            && i > 0
            && matches!(contents[i - 1], ShapeEntry::Item(_));
        match (entry, runs.last_mut()) {
            (ShapeEntry::Modifier(_), _) => {}
            (_, Some((run, last))) if joins => {
                run.push(entry.clone());
                *last = i;
            }
            _ => runs.push((vec![entry.clone()], i)),
        }
    }
    runs.into_iter()
        .map(|(mut run, last)| {
            let after = contents[last + 1..].iter();
            run.extend(
                after
                    .filter(|e| matches!(e, ShapeEntry::Modifier(_)))
                    .cloned(),
            );
            run
        })
        .collect()
}

/// A Type layer's letters as shape art at layer time `t`, a group for each.
pub(crate) fn text_art(
    source: &LayerReference,
    layer: &Layer,
    document: &TextDocument,
    comp_time: f64,
    t: f64,
) -> Result<Vec<ShapeEntry>, BridgeError> {
    // The words at this moment, through the one resolver the rasteriser and
    // the frame key read, so a converted caption says what it was saying.
    let doc = {
        let project = source.project()?;
        let project = project.read().map_err(|_| BridgeError::ReadFailed)?;
        project.store.snapshot()
    };
    let words = document
        .resolved_text(std::sync::Arc::new(
            lumit_core::expression::ExpressionContext {
                document: doc,
                comp: Some(source.comp_id),
                layer: Some(source.layer_id),
                comp_time,
                current_depth: 0,
                inputs: None,
            },
        ))
        .into_owned();
    // A line on a path converts curved, placed by the rasteriser's own walk.
    let spine = document
        .path
        .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, t))
        .filter(|p| !p.is_empty());
    Ok(lumit_text::shape_items(
        &lumit_text::TextBlock::of(document, &words),
        spine.as_ref(),
        document.path_offset.value_at(t) as f32,
    ))
}

/// The Illustrator file a footage layer reads, and which of its layers.
#[cfg(feature = "media")]
pub(crate) fn vector_source(
    source: &LayerReference,
    layer: &Layer,
) -> Option<(std::path::PathBuf, Option<u32>)> {
    let LayerKind::Footage { item } = layer.kind else {
        return None;
    };
    let project = source.project().ok()?;
    let state = project.read().ok()?;
    let doc = state.store.snapshot();
    let lumit_core::model::ProjectItem::Footage(footage) = doc.item(item)? else {
        return None;
    };
    let path = crate::api::footage::FootageReference::resolve_path(&state, footage)?;
    lumit_media::ai::is_ai(&path).then_some((path, footage.source_layer))
}

/// An Illustrator layer's paths as shape art, a group for each path, in
/// the artboard's pixels.
#[cfg(feature = "media")]
pub(crate) fn vector_art(paths: Vec<lumit_media::ai::paths::AiPath>) -> Vec<ShapeEntry> {
    use lumit_core::mask::{BezierPath, Vertex};
    use lumit_core::pixels::srgb_decode;
    use lumit_core::shape::{colour, ShapeItem};
    let linear = |c: [u8; 4]| {
        lumit_core::model::LinearColour([
            srgb_decode(c[0]),
            srgb_decode(c[1]),
            srgb_decode(c[2]),
            f32::from(c[3]) / 255.0,
        ])
    };
    let mut groups = Vec::new();
    for path in paths {
        let mut items = Vec::new();
        for (i, contour) in path.contours.into_iter().enumerate() {
            let vertices = contour.points.iter().map(|p| Vertex {
                pos: (p[0], p[1]),
                tan_in: (p[2], p[3]),
                tan_out: (p[4], p[5]),
            });
            let shape = BezierPath {
                vertices: vertices.collect(),
                closed: contour.closed,
            };
            let mut item = ShapeItem::filled(format!("Path {}", i + 1), shape, linear([0; 4]));
            item.fill = path.fill.map(|c| colour::still(linear(c)));
            if let Some(c) = path.stroke {
                item.stroke = Some(colour::still(linear(c)));
                item.stroke_width = lumit_core::anim::Property::fixed(path.stroke_width);
            }
            // ponytail: holes are cut by even-odd, as a letter's are. A path
            // that crosses itself and is filled by winding comes out hollow
            // where it crosses; the upgrade is reading the path's own rule.
            item.combine = u32::from(path.fill.is_some() && i > 0) * 4;
            items.push(ShapeEntry::Item(item));
        }
        if !items.is_empty() {
            let name = format!("Group {}", groups.len() + 1);
            groups.push(ShapeEntry::Group(
                lumit_core::shape::modifier::ShapeGroup::new(name, items),
            ));
        }
    }
    groups
}

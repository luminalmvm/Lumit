//! Shape layers made from another layer's outlines: Create ▸ Shapes from
//! text.

use lumit_core::model::LayerKind;
use lumit_core::shape::modifier::ShapeEntry;
use uuid::Uuid;

use crate::api::assets::shift_property;
use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// Add `contents` as a shape layer directly above `source`, and switch
/// `source` off, as After Effects does. One `Batch`, so one undo step.
///
/// The art is in `source`'s own layer pixels, read at layer time `t`.
pub(crate) fn add_above(
    source: &LayerReference,
    contents: Vec<ShapeEntry>,
    t: f64,
) -> Result<LayerReference, BridgeError> {
    let comp = source.composition()?;
    let layer = source.item()?;
    let index = comp
        .layers
        .iter()
        .position(|l| l.id == source.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;

    let mut copy = layer.clone();
    copy.id = Uuid::now_v7();
    copy.name = format!("{} outlines", layer.name);
    for effect in &mut copy.effects {
        effect.id = Uuid::now_v7();
    }
    // Masks and paint are drawn from the layer's box corner, and a shape
    // layer's corner is its art's box, so they would land somewhere else.
    copy.masks.clear();
    copy.paint.clear();
    // The anchor moves by that corner too, so the art sits where it was.
    if let Some((x0, y0, _, _)) = lumit_core::shape::contents_bounds(&contents, t) {
        shift_property(&mut copy.transform.anchor_x, -x0);
        shift_property(&mut copy.transform.anchor_y, -y0);
    }
    copy.kind = LayerKind::Shape { contents };
    crate::edits::solo_on_arrival(&mut copy, comp.layers.iter());

    let id = copy.id;
    source.commit(lumit_core::Op::Batch {
        ops: vec![
            lumit_core::Op::AddLayer {
                comp: source.comp_id,
                index,
                layer: Box::new(copy),
            },
            lumit_core::Op::SetLayerVisible {
                comp: source.comp_id,
                layer: source.layer_id,
                visible: false,
            },
        ],
    })?;
    Ok(LayerReference::new(source.project_id, source.comp_id, id))
}

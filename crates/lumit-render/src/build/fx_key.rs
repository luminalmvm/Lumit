//! What names the picture a layer's effects run on.

use std::collections::HashMap;

use lumit_core::fx::builtins::{LFX_MATCH_PREFIX, OFX_MATCH_PREFIX};
use lumit_core::fx::params::ResolvedStack;

use uuid::Uuid;

use crate::decode::CompLayerPixels;

/// The content name of the picture a layer's effect stack runs on
/// ([`crate::draw::CompLayerDraw::fx_input_key`]): what the source is, then everything
/// `pixels_for` bakes into it before the stack sees it — the paint strokes,
/// the masks at this layer time — and the raster it was made at. Footage and
/// Sequence layers name their source by the decode job's identity
/// ([`CompLayerPixels::source_key`]); a solid by its colour and size. `None`
/// for the kinds nothing names yet (text, shapes), which run uncached.
///
/// The masks and paint go in serialised, as the file format writes them: a
/// keyframed mask is named by its whole animation plus the time it is read
/// at, which is coarser than the evaluated shape but never wrong.
pub(super) fn fx_input_key(
    doc: &lumit_core::model::Document,
    layer: &lumit_core::model::Layer,
    pixels_by_layer: &HashMap<Uuid, &CompLayerPixels>,
    lt: f64,
    tex_w: u32,
    tex_h: u32,
) -> Option<u128> {
    use lumit_core::model::LayerKind;
    let mut h = blake3::Hasher::new();
    h.update(b"fxinput/1/");
    match &layer.kind {
        LayerKind::Footage { .. } | LayerKind::Sequence { .. } => {
            h.update(b"footage/");
            h.update(&pixels_by_layer.get(&layer.id)?.source_key.to_le_bytes());
        }
        LayerKind::Solid { def } => {
            let sd = doc.solid(*def)?;
            h.update(b"solid/");
            for ch in sd.colour.0 {
                h.update(&ch.to_le_bytes());
            }
            h.update(&sd.width.to_le_bytes());
            h.update(&sd.height.to_le_bytes());
        }
        _ => return None,
    }
    h.update(&bincode::serialize(&layer.masks).ok()?);
    h.update(&bincode::serialize(&layer.paint).ok()?);
    h.update(&lt.to_le_bytes());
    h.update(&tex_w.to_le_bytes());
    h.update(&tex_h.to_le_bytes());
    let mut k = [0u8; 16];
    k.copy_from_slice(&h.finalize().as_bytes()[..16]);
    Some(u128::from_le_bytes(k))
}

/// [`fx_input_key`] for what reads a picture only when it changes, so with no
/// masks or paint a still is named the same at every moment. Not for the
/// effect cache: a custom shader or a plugin is told the time beside its
/// parameters, so `None` when `stack` holds one.
pub(crate) fn still_input_key(
    doc: &lumit_core::model::Document,
    layer: &lumit_core::model::Layer,
    pixels_by_layer: &HashMap<Uuid, &CompLayerPixels>,
    t_comp: f64,
    (tex_w, tex_h): (u32, u32),
    stack: &ResolvedStack,
) -> Option<u128> {
    let clocked = stack.iter().any(|op| {
        let name = op.def.schema().match_name;
        name == "custom_shader"
            || name.starts_with(OFX_MATCH_PREFIX)
            || name.starts_with(LFX_MATCH_PREFIX)
    });
    if clocked {
        return None;
    }
    let moves = !layer.masks.is_empty() || !layer.paint.is_empty();
    let lt = match moves {
        true => lumit_core::time::layer_time(t_comp, layer.start_offset.0),
        false => 0.0,
    };
    fx_input_key(doc, layer, pixels_by_layer, lt, tex_w, tex_h)
}

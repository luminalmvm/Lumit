//! What Energy line is handed beside its op: the layer's masks, a Shape
//! layer's paths or a Text layer's letters, as one run of paths in the
//! layer's pixels. The Line source carries its path in its rows and is handed
//! nothing.

use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::fx::effects::energy_line::{SOURCE_MASK, SOURCE_SHAPE, SOURCE_TEXT};
use lumit_core::mask::{mask_path_at, MaskPolyline};
use lumit_core::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};

#[allow(clippy::too_many_arguments)]
pub(super) fn path(
    doc: &Arc<Document>,
    comp: &Composition,
    e: &EffectInstance,
    param: &str,
    self_default: bool,
    layer: Option<&Layer>,
    lt: f64,
    t_comp: f64,
) -> MaskPolyline {
    let Some(layer) = layer else {
        return MaskPolyline::default();
    };
    let source = match e.param("source") {
        Some(EffectValue::Choice(v)) => *v,
        _ => 0,
    };
    match (source, &layer.kind) {
        (SOURCE_MASK, _) if e.bool_of("all_masks").unwrap_or(true) => {
            let mut run = MaskPolyline::default();
            for mask in &layer.masks {
                run.append(&mask_path_at(&layer.masks, Some(mask.id), false, lt));
            }
            run
        }
        (SOURCE_MASK, _) => mask_path_at(&layer.masks, e.mask_ref(param), self_default, lt),
        (SOURCE_SHAPE, LayerKind::Shape { contents }) => {
            lumit_core::shape::outline_at(contents, lt)
        }
        (SOURCE_TEXT, LayerKind::Text { document }) => {
            // The words and the path the layer's own picture is drawn with.
            let words = document.resolved_text(Arc::new(ExpressionContext {
                document: doc.clone(),
                comp: Some(comp.id),
                layer: Some(layer.id),
                comp_time: t_comp,
                current_depth: 0,
                inputs: None,
            }));
            let spine = document
                .path
                .map(|id| mask_path_at(&layer.masks, Some(id), false, lt))
                .filter(|p| !p.is_empty());
            let items = lumit_text::shape_items(
                &lumit_text::TextBlock::of(document, &words),
                spine.as_ref(),
                document.path_offset.value_at(lt) as f32,
            );
            let mut run = lumit_core::shape::outline_at(&items, lt);
            // The outline is measured from the letters' own corner, and the
            // layer draws them from its origin.
            if let Some((x0, y0, ..)) = lumit_core::shape::contents_bounds(&items, lt) {
                for p in &mut run.points {
                    p[0] += x0 as f32;
                    p[1] += y0 as f32;
                }
            }
            run
        }
        _ => MaskPolyline::default(),
    }
}

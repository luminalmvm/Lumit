//! A Text or Shape layer's own picture, rasterised.

use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, Layer, TextDocument};
use lumit_core::shape::modifier::ShapeEntry;

use crate::placement::world_placement;

use super::{raster_scale, SHAPE_RASTER_PIXELS};

/// A layer's raster: the bytes, the raster size and the natural size.
pub(super) type LayerRaster = (Arc<Vec<u8>>, u32, u32, (f32, f32));

/// A Text layer's words at layer time `lt`, comp time `t_comp`, rasterised.
pub(super) fn text(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    document: &TextDocument,
    (lt, t_comp): (f64, f64),
    context: &Arc<ExpressionContext>,
) -> LayerRaster {
    // The words at *this* layer time — a plain document hands back
    // what was typed; an expression-driven one is evaluated here,
    // which is what makes a caption able to print a live value.

    let (line, spoken, _) = lumit_core::captions::show::spoken(document, context.clone());
    let size = document.size as f32;
    // **Text on a path**. The named mask's own flattened
    // curve, read through the carriage every path-walking effect
    // reads — so the glyphs and a Stroke on the same mask
    // agree about where the curve is. A layer whose row names
    // nothing, or a mask since deleted, flattens to an empty
    // polyline and the line lays straight: refuse calmly, never
    // an empty layer (docs/14 §4).
    let spine = document
        .path
        .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
        .filter(|p| !p.is_empty());
    // **The letters move separately**. What each one is
    // asked to do is worked out here, from the animators' own
    // keyframed numbers at this layer time; a layer with none hands
    // back an empty list and the rasteriser takes the path it
    // always took, byte for byte.
    let xforms = lumit_core::text::ports::layer_glyph_xforms(
        doc, comp, layer, document, &line, t_comp, &spoken,
    );
    // An unstyled single line draws the way it always has. A
    // styled one, or one with a line break, is set by the shaper.
    let block = lumit_text::TextBlock::of(document, &line);
    // **Continuously rasterised**, as a shape layer is: the words
    // are drawn as large as the layer and its parents show them,
    // so a small text scaled up never blurs. The placement keeps
    // the layer's own size, and a raster too big to hold is drawn
    // at the most it can be.
    let shown =
        raster_scale::shape_raster_scale(world_placement(comp, layer, t_comp, context.clone()));
    let room = |w: u32, h: u32| {
        (SHAPE_RASTER_PIXELS / (f64::from(w.max(1)) * f64::from(h.max(1))))
            .sqrt()
            .max(1.0)
    };
    let (r, natural) = match &spine {
        Some(path) => {
            let (w, h) = lumit_text::path_box(path, size);
            let zoom = f64::from(shown).clamp(1.0, room(w, h)) as f32;
            let r = lumit_text::rasterise_along_at(
                &block,
                path,
                document.path_offset.value_at(lt) as f32,
                w,
                h,
                &xforms,
                zoom,
            );
            (r, (w.max(1), h.max(1)))
        }
        // Shown at its own size or smaller, the layer is drawn as it
        // always was, without laying it out twice.
        None if shown <= 1.0 => {
            let r = lumit_text::rasterise(&block, &xforms);
            let size = (r.width, r.height);
            (r, size)
        }
        None => {
            let own = lumit_text::layout(&block, !xforms.is_empty());
            let zoom = f64::from(shown).clamp(1.0, room(own.width, own.height)) as f32;
            lumit_text::rasterise_at(&block, &xforms, zoom)
        }
    };
    (
        Arc::new(r.rgba),
        r.width,
        r.height,
        (natural.0 as f32, natural.1 as f32),
    )
}

/// A Shape layer's art at layer time `lt`, rasterised. `None` when it has
/// nothing to draw.
pub(super) fn shape(
    comp: &Composition,
    layer: &Layer,
    contents: &[ShapeEntry],
    (lt, t_comp): (f64, f64),
    context: &Arc<ExpressionContext>,
) -> Option<LayerRaster> {
    let contents = lumit_core::shape::contents_at(contents, lt, context);
    let (x0, y0, x1, y1) = lumit_core::shape::contents_bounds(&contents, lt)?;
    let natural_w = (x1 - x0).max(1.0);
    let natural_h = (y1 - y0).max(1.0);
    // The same reduced-resolution rule the other rasterised
    // kinds follow: draw at the working scale, and let the
    // placement matrix carry the natural size.
    //
    // **Continuously rasterised**, as After Effects draws a
    // shape layer: the art is drawn as large as the layer and
    // its parents show it, so scaling it up never blurs it.
    // The effects read the raster factor off the texture, so
    // they keep their size.
    let shown =
        raster_scale::shape_raster_scale(world_placement(comp, layer, t_comp, context.clone()));
    let most = (SHAPE_RASTER_PIXELS / (natural_w * natural_h))
        .sqrt()
        .max(1.0);
    let k = f64::from(shown).clamp(1.0, most);
    let w = (natural_w * k).round().max(1.0) as u32;
    let h = (natural_h * k).round().max(1.0) as u32;
    Some((
        Arc::new(lumit_core::shape::rasterise_contents(
            &contents, w, h, x0, y0, x1, y1, lt,
        )),
        w,
        h,
        (natural_w as f32, natural_h as f32),
    ))
}

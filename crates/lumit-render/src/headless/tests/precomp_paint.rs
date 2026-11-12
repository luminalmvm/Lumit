//! Tests for paint strokes on a Precomp layer.

use super::*;

/// **A paint stroke on a Precomp layer must reach the picture**.
///
/// Every other kind of layer is painted in its own raster before it is
/// uploaded; a Precomp has no raster until it is rendered, so its strokes
/// were built into the draw list, carried nowhere, and quietly dropped —
/// the brush drew a Timeline row and no pixels. The realiser now stamps
/// them into the nested picture with the same `apply_strokes` every other
/// layer goes through.
///
/// The scene: a 32×32 precomp of solid white inside a black comp, with one
/// fat red dab in the middle of the precomp layer. The middle is red; a
/// corner of the precomp is still white.
#[test]
fn a_paint_stroke_on_a_precomp_layer_reaches_the_picture() {
    let mut r = match HeadlessRenderer::shared() {
        Ok(r) => r,
        Err(_) => {
            lumit_gpu::no_adapter();
            return;
        }
    };
    let (cw, ch) = (32u32, 32u32);
    let white = LinearColour([1.0, 1.0, 1.0, 1.0]);
    let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.0, 0.0, 0.0, 1.0]));
    let (child_doc, child_id, _) = matrix_base(cw, ch, white);
    for item in child_doc.items {
        doc.items.push(item);
    }
    let mut pre = matrix_layer("Nested", LayerKind::Precomp { comp: child_id }, cw, ch);
    let mut stroke = lumit_core::paint::PaintStroke::new("Brush 1", vec![(16.0, 16.0)]);
    stroke.width = 12.0;
    stroke.hardness = 1.0;
    stroke.colour = LinearColour([1.0, 0.0, 0.0, 1.0]);
    pre.paint.push(stroke);
    // Index 0 is the top of the stack, over the black base.
    doc.comp_mut(comp_id).unwrap().layers.insert(0, pre);

    let (rgba, w, _h) = r
        .render_rgba(&std::sync::Arc::new(doc.clone()), comp_id, 0, 1.0)
        .expect("render");
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        (rgba[i], rgba[i + 1], rgba[i + 2])
    };
    let (mr, mg, mb) = at(16, 16);
    assert!(
        mr > 200 && mg < 60 && mb < 60,
        "the dab must land on the precomp's picture, got {:?}",
        (mr, mg, mb)
    );
    let (cr, cg, cb) = at(2, 2);
    assert!(
        cr > 200 && cg > 200 && cb > 200,
        "and only under the brush — the corner stays white, got {:?}",
        (cr, cg, cb)
    );
}

//! A track matte and a paint stroke on a Precomp layer reach the picture.

use super::*;

/// **A precomp set as a track matte must actually gate the layer**.
///
/// The regression: a comp has no pixels until it is rendered, so the draw
/// builder's `pixels_for` answered None for a Precomp matte source and the
/// whole matte quietly disappeared — the consumer drew everywhere, as if
/// no matte had been set. An earlier fix closed the same hole for the
/// *layer-input* mattes (a flare source, a DoF depth pass); the track
/// matte, which is how everyone actually reaches for a precomp matte,
/// still had it.
///
/// The scene: a full-frame red solid matted by a hidden precomp layer whose
/// own 16×16 blue solid covers the LEFT half of a 32×16 comp. Red survives
/// where the precomp has alpha and nowhere else.
///
/// Also at half scale, where the comp is realised at half size. It used to
/// be padded back to full size rather than stretched, so the matte shrank
/// into the middle of its square and the corner went dark.
#[test]
fn a_precomp_track_matte_gates_the_layer() {
    let mut r = match HeadlessRenderer::shared() {
        Ok(r) => r,
        Err(_) => {
            lumit_gpu::no_adapter();
            return;
        }
    };
    let (cw, ch) = (32u32, 16u32);
    let (mut doc, comp_id, _) = matrix_base(cw, ch, LinearColour([0.8, 0.1, 0.1, 1.0]));
    let (child_doc, child_id, _) = matrix_base(16, 16, LinearColour([0.1, 0.2, 0.9, 1.0]));
    for item in child_doc.items {
        doc.items.push(item);
    }
    // The matte itself is hidden, as a matte source always is.
    let mut matte = matrix_layer("Matte", LayerKind::Precomp { comp: child_id }, 16, 16);
    matte.switches.visible = false;
    let matte_id = matte.id;
    {
        let comp = doc.comp_mut(comp_id).unwrap();
        comp.layers[0].matte = Some(lumit_core::model::MatteRef {
            layer: matte_id,
            channel: lumit_core::model::MatteChannel::Alpha,
            inverted: false,
            source: lumit_core::model::LayerInputSource::default(),
        });
        comp.layers.push(matte);
    }

    let doc = std::sync::Arc::new(doc);
    let half = Quality {
        auto_res: true,
        display_scale: 0.5,
        ..Quality::default()
    };
    for quality in [Quality::default(), half] {
        let (rgba, w, h) = r
            .render_preview(&doc, comp_id, 0, quality, 1.0)
            .expect("render");
        // Comp pixels, whatever size the frame came back at.
        let at = |x: u32, y: u32| {
            let i = (((y * h / ch) * w + x * w / cw) * 4) as usize;
            (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3])
        };
        // Left half: inside the precomp's opaque square, so the red shows.
        // The middle and a corner, so a shrunken matte shows too.
        for (x, y) in [(8, 8), (2, 2)] {
            let (lr, _lg, _lb, la) = at(x, y);
            assert!(
                lr > 150,
                "red should survive under the matte at {x},{y}, got {lr}"
            );
            assert_eq!(la, 255, "and it should be opaque there");
        }
        // Right half: the precomp is transparent there, so nothing is drawn —
        // this is the pixel that stayed red while the matte was being dropped.
        let (rr, rg, rb, _ra) = at(24, 8);
        assert!(
            rr < 30 && rg < 30 && rb < 30,
            "outside the precomp matte the layer must be gated out, got {:?}",
            (rr, rg, rb)
        );
    }
}

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

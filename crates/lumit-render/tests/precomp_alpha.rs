//! A Precomp layer draws the same pixels as its layers do without it. The
//! compositor made its picture, so the colour already has its alpha in it, and
//! multiplying by that alpha again is what turned anything see-through dark.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::edits::{base_layer, centred_transform, new_comp_ops, CompSettings};
use lumit_core::model::{
    BlendMode, ColourDepth, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
};
use lumit_core::ops::{apply, Op};
use lumit_core::time::Rational;
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 16;
const GROUND: [f32; 4] = [0.8, 0.3, 0.1, 1.0];

fn comp(doc: &mut Document) -> Uuid {
    let settings = CompSettings {
        width: SIZE,
        height: SIZE,
        background: [0.0; 4],
        ..CompSettings::default()
    };
    let (id, ops) = new_comp_ops(doc, settings, None);
    for op in &ops {
        apply(doc, op).unwrap();
    }
    id
}

/// A comp-sized solid, added under whatever `comp` holds.
fn solid(doc: &mut Document, comp: Uuid, colour: [f32; 4], edit: impl Fn(&mut Layer)) {
    let def = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Solid".into(),
        colour: LinearColour(colour),
        width: SIZE,
        height: SIZE,
        extra: serde_json::Map::new(),
    }));
    let mut layer = layer(doc, comp, LayerKind::Solid { def });
    edit(&mut layer);
    push(doc, comp, layer);
}

fn layer(doc: &Document, comp: Uuid, kind: LayerKind) -> Layer {
    base_layer(
        "Layer".into(),
        kind,
        doc.comp(comp).unwrap().duration.0,
        centred_transform(f64::from(SIZE), f64::from(SIZE), SIZE, SIZE),
    )
}

fn push(doc: &mut Document, comp: Uuid, layer: Layer) {
    let index = doc.comp(comp).unwrap().layers.len();
    let layer = Box::new(layer);
    apply(doc, &Op::AddLayer { comp, index, layer }).unwrap();
}

fn frame(r: &mut HeadlessRenderer, doc: Document, comp: Uuid, frame: u64) -> Vec<f32> {
    r.render_preview_linear(&Arc::new(doc), comp, frame, Default::default())
        .unwrap()
        .0
}

fn assert_close(a: &[f32], b: &[f32], what: &str) {
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for (a, b) in a.iter().zip(b) {
            assert!((a - b).abs() < 0.01, "{what}: {a} against {b}");
        }
    }
}

#[test]
fn a_precomp_over_a_ground_draws_what_its_layers_draw() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let half = |l: &mut Layer| l.transform.opacity = Property::fixed(50.0);
    for depth in [
        ColourDepth::Eight,
        ColourDepth::Sixteen,
        ColourDepth::ThirtyTwo,
    ] {
        for (blend, collapse) in [
            (BlendMode::Normal, false),
            (BlendMode::Normal, true),
            (BlendMode::Multiply, false),
            (BlendMode::Screen, false),
        ] {
            let mut doc = Document::new();
            doc.colour_depth = depth;
            let flat = comp(&mut doc);
            let inner = comp(&mut doc);
            let parent = comp(&mut doc);
            let fill = [0.2, 0.6, 0.9, 1.0];
            solid(&mut doc, flat, fill, |l| {
                half(l);
                l.blend = blend;
            });
            solid(&mut doc, flat, GROUND, |_| {});
            solid(&mut doc, inner, fill, half);
            let mut placed = layer(&doc, parent, LayerKind::Precomp { comp: inner });
            placed.blend = blend;
            placed.switches.collapse = collapse;
            push(&mut doc, parent, placed);
            solid(&mut doc, parent, GROUND, |_| {});
            let flat = frame(&mut r, doc.clone(), flat, 0);
            let nested = frame(&mut r, doc, parent, 0);
            assert_close(&nested, &flat, &format!("{depth:?} {blend:?} {collapse}"));
        }
    }

    // The average of a layer's motion blurred copies is premultiplied as well:
    // a smear the colour of the ground can't show against it.
    let mut doc = Document::new();
    let main = comp(&mut doc);
    solid(&mut doc, main, GROUND, |l| {
        l.switches.motion_blur = true;
        let key = |time, value| Keyframe {
            time,
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        l.transform.position_x = Property {
            animation: Animation::Keyframed(vec![
                key(Rational::ZERO, 0.0),
                key(Rational::new(1, 1).unwrap(), 600.0),
            ]),
            extra: serde_json::Map::new(),
        };
    });
    solid(&mut doc, main, GROUND, |_| {});
    for item in &mut doc.items {
        if let ProjectItem::Composition(c) = item {
            c.motion_blur.enabled = true;
        }
    }
    let ground: Vec<f32> = GROUND.repeat((SIZE * SIZE) as usize);
    assert_close(&frame(&mut r, doc, main, 1), &ground, "motion blur");
}

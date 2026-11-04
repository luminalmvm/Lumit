//! Text and Shape layers with Geometry options, through the walk the Viewer
//! and export share. At nought they draw the bytes they always drew. With
//! depth, the front lands where the flat picture's ink did, and the side shows
//! when the layer turns.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::edits::{new_comp_ops, CompSettings};
use lumit_core::mask::{BezierPath, Vertex};
use lumit_core::model::{
    Document, EffectInstance, EffectValue, Layer, LayerKind, LightDef, LightKind, LinearColour,
    TextDocument, TransformGroup,
};
use lumit_core::ops::{apply, Op};
use lumit_core::shape::{modifier::ShapeEntry, ShapeItem};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 320;

fn comp() -> (Document, Uuid) {
    let mut doc = Document::new();
    let settings = CompSettings {
        width: SIZE,
        height: SIZE,
        ..CompSettings::default()
    };
    let (id, ops) = new_comp_ops(&doc, settings, None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    (doc, id)
}

fn add(doc: &mut Document, comp: Uuid, layer: Layer) {
    apply(
        doc,
        &Op::AddLayer {
            comp,
            index: 0,
            layer: Box::new(layer),
        },
    )
    .unwrap();
}

fn base(doc: &Document, comp: Uuid, name: &str, kind: LayerKind) -> Layer {
    let duration = doc.comp(comp).unwrap().duration.0;
    let f = f64::from(SIZE);
    let mut layer = lumit_core::edits::base_layer(
        name.into(),
        kind,
        duration,
        TransformGroup {
            position_x: Property::fixed(f * 0.5),
            position_y: Property::fixed(f * 0.5),
            anchor_x: Property::fixed(f * 0.25),
            anchor_y: Property::fixed(f * 0.25),
            ..TransformGroup::default()
        },
    );
    layer.switches.three_d = true;
    layer
}

fn square() -> Vec<ShapeEntry> {
    let corner = |x: f64, y: f64| Vertex {
        pos: (x, y),
        tan_in: (0.0, 0.0),
        tan_out: (0.0, 0.0),
    };
    let path = BezierPath {
        vertices: vec![
            corner(0.0, 0.0),
            corner(160.0, 0.0),
            corner(160.0, 160.0),
            corner(0.0, 160.0),
        ],
        closed: true,
    };
    vec![ShapeEntry::Item(ShapeItem::filled(
        "Square",
        path,
        LinearColour([0.8, 0.2, 0.1, 1.0]),
    ))]
}

fn geometry(depth: f64, bevel: f64) -> Vec<Option<EffectInstance>> {
    let mut rows = lumit_core::fx::builtins::instantiate(lumit_core::extrude::MATCH_NAME).unwrap();
    for (id, value) in [("extrusion_depth", depth), ("bevel_depth", bevel)] {
        let p = rows.params.iter_mut().find(|p| p.id == id).unwrap();
        p.value = EffectValue::Float(Property::fixed(value));
    }
    vec![Some(rows)]
}

/// A light from up and to the left.
fn lit(doc: &mut Document, comp: Uuid) {
    let kind = LayerKind::Light {
        light: Box::new(LightDef {
            kind: LightKind::Parallel,
            ..LightDef::default()
        }),
    };
    let mut key = base(doc, comp, "Key", kind);
    let f = f64::from(SIZE);
    key.transform.position_x = Property::fixed(f * 0.2);
    key.transform.position_y = Property::fixed(f * 0.1);
    key.transform.position_z = Property::fixed(-f);
    add(doc, comp, key);
}

fn render(doc: Document, comp: Uuid) -> Vec<u8> {
    let doc = Arc::new(doc);
    let mut shared = HeadlessRenderer::shared().unwrap();
    let (rgba, ..) = shared.render_rgba(&doc, comp, 0, 1.0).unwrap();
    rgba
}

/// The box round the pixels that aren't the ground, as (left, top, right,
/// bottom). The ground is the corner's colour.
fn ink(rgba: &[u8]) -> (u32, u32, u32, u32) {
    let (mut a, mut b, mut c, mut d) = (SIZE, SIZE, 0, 0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let at = ((y * SIZE + x) * 4) as usize;
            if rgba[at..at + 3]
                .iter()
                .zip(rgba)
                .any(|(p, g)| p.abs_diff(*g) > 24)
            {
                (a, b, c, d) = (a.min(x), b.min(y), c.max(x), d.max(y));
            }
        }
    }
    (a, b, c, d)
}

/// Rows at nought draw the bytes a layer with none draws, whichever way the
/// layer came to have them.
#[test]
fn nought_is_the_flat_picture() {
    if HeadlessRenderer::shared().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let (mut doc, id) = comp();
    let kind = LayerKind::Shape { contents: square() };
    let layer = base(&doc, id, "Flat", kind);
    add(&mut doc, id, layer);
    let flat = render(doc, id);

    let (mut doc, id) = comp();
    let kind = LayerKind::Shape { contents: square() };
    let mut layer = base(&doc, id, "Nought", kind);
    layer.transform.material.geometry = geometry(0.0, 0.0);
    add(&mut doc, id, layer);
    assert_eq!(flat, render(doc, id));
}

/// With depth, a square facing the camera keeps its outline, and a text keeps
/// its place. Turned, the same square shows more than its front, shaded
/// differently from the front, because the side catches the light otherwise.
#[test]
fn depth_keeps_the_front_and_shows_a_side() {
    if HeadlessRenderer::shared().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let (mut doc, id) = comp();
    let mut flat = base(&doc, id, "Flat", LayerKind::Shape { contents: square() });
    add(&mut doc, id, flat.clone());
    let outline = ink(&render(doc, id));

    let (mut doc, id) = comp();
    flat.transform.material.geometry = geometry(80.0, 0.0);
    add(&mut doc, id, flat.clone());
    lit(&mut doc, id);
    let front = ink(&render(doc.clone(), id));
    for (a, b) in [
        (outline.0, front.0),
        (outline.1, front.1),
        (outline.2, front.2),
        (outline.3, front.3),
    ] {
        assert!(a.abs_diff(b) <= 1, "{outline:?} became {front:?}");
    }

    let layer = doc.comp_mut(id).unwrap().layers.iter_mut();
    for l in layer.filter(|l| l.name == "Flat") {
        l.transform.rotation_y = Property::fixed(40.0);
    }
    let turned = render(doc.clone(), id);
    let wide = ink(&turned);
    assert!(
        wide.2 - wide.0 > 160 * 3 / 4,
        "a turned solid shows little: {wide:?}"
    );
    // The front and the side aren't the same colour under one light.
    let mut shades: Vec<u8> = turned.chunks_exact(4).map(|p| p[0]).collect();
    shades.sort_unstable();
    shades.dedup();
    assert!(shades.len() > 2, "one flat colour: {shades:?}");
}

/// A title whose outline crosses itself, or has nothing in it at all, draws
/// what it can. Text with no glyphs draws nothing and doesn't fail the frame.
#[test]
fn a_text_draws_and_an_empty_one_is_nothing() {
    if HeadlessRenderer::shared().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let document = |text: &str| TextDocument {
        text: text.into(),
        expression: None,
        size: 120.0,
        fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
        path: None,
        path_offset: Property::zero(),
        animators: Vec::new(),
        style: Box::default(),
        paragraph: Default::default(),
        extra: serde_json::Map::new(),
    };
    for (text, shows) in [("Ae", true), ("", false)] {
        let (mut doc, id) = comp();
        let kind = LayerKind::Text {
            document: document(text),
        };
        let mut layer = base(&doc, id, "Title", kind);
        layer.transform.material.geometry = geometry(30.0, 4.0);
        add(&mut doc, id, layer);
        lit(&mut doc, id);
        let picture = render(doc, id);
        assert_eq!(ink(&picture).2 > 0, shows);
    }
}

/// Overlapping art in one Shape layer stacks as it paints when the layer
/// turns, its later fill in front of its earlier one.
#[test]
fn overlapping_fills_keep_their_order_when_turned() {
    if HeadlessRenderer::shared().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let corner = |x: f64, y: f64| Vertex {
        pos: (x, y),
        tan_in: (0.0, 0.0),
        tan_out: (0.0, 0.0),
    };
    let block = |x: f64, c: [f32; 4]| {
        let vertices = vec![
            corner(x, 0.0),
            corner(x + 120.0, 0.0),
            corner(x + 120.0, 120.0),
            corner(x, 120.0),
        ];
        let path = BezierPath {
            vertices,
            closed: true,
        };
        ShapeEntry::Item(ShapeItem::filled("Block", path, LinearColour(c)))
    };
    let contents = vec![
        block(0.0, [1.0, 0.0, 0.0, 1.0]),
        block(60.0, [0.0, 0.0, 1.0, 1.0]),
    ];
    let (mut doc, id) = comp();
    let mut layer = base(&doc, id, "Blocks", LayerKind::Shape { contents });
    layer.transform.anchor_x = Property::fixed(90.0);
    layer.transform.anchor_y = Property::fixed(60.0);
    layer.transform.rotation_y = Property::fixed(-30.0);
    layer.transform.material.geometry = geometry(10.0, 0.0);
    add(&mut doc, id, layer);
    let picture = render(doc, id);
    // Where the two overlap is the later one's blue, all the way down.
    let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    let (r, b) = (picture[at], picture[at + 2]);
    assert!(b > r, "red came through blue: r {r}, b {b}");
}

/// A title with animators keeps its place when it gains depth, with its
/// letters turned and scaled about the same point.
#[test]
fn an_animated_title_stays_put_when_it_gains_depth() {
    if HeadlessRenderer::shared().is_err() {
        lumit_gpu::no_adapter();
        return;
    }
    let kind = || LayerKind::Text {
        document: TextDocument {
            text: "Hold".into(),
            expression: None,
            size: 100.0,
            fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
            path: None,
            path_offset: Property::zero(),
            animators: vec![{
                let mut a = lumit_core::text::TextAnimator::new("Turned");
                a.rotation = Property::fixed(40.0);
                a.scale_x = Property::fixed(150.0);
                a.scale_y = Property::fixed(70.0);
                a
            }],
            style: Box::default(),
            paragraph: Default::default(),
            extra: serde_json::Map::new(),
        },
    };
    let (mut doc, id) = comp();
    let flat = base(&doc, id, "Flat", kind());
    add(&mut doc, id, flat);
    let flat = ink(&render(doc, id));
    let (mut doc, id) = comp();
    let mut solid = base(&doc, id, "Solid", kind());
    solid.transform.material.geometry = geometry(0.0, 0.0);
    solid.transform.material.geometry = geometry(1.0, 0.0);
    add(&mut doc, id, solid);
    let solid = ink(&render(doc, id));
    for (a, b) in [
        (flat.0, solid.0),
        (flat.1, solid.1),
        (flat.2, solid.2),
        (flat.3, solid.3),
    ] {
        assert!(a.abs_diff(b) <= 3, "{flat:?} became {solid:?}");
    }
}

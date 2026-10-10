//! **A depth plane is drawn in a real render** (docs/impl/addons.md §6.1 and
//! §11 test 8).
//!
//! # In plain terms
//!
//! The unit tests beside this one prove the analysis reads a clip, files its
//! planes and names them; none of them renders anything. This one does. It puts
//! a Depth effect on a layer, files a plane for that layer's source frame under
//! the effect's own id, and pushes the document through the same public entry
//! the Viewer and the exporter use, so it crosses every seam at once: the
//! store, the draw builder's planes carriage, the upload and the pass in
//! `run_ops`.
//!
//! The picture is arranged so the answer is unmissable. A flat white layer
//! fills the frame; the plane is nought on the left half and the top of its
//! range on the right. In Depth view the frame is black on the left and white
//! on the right, nearer being brighter; Invert turns it over; Source view
//! leaves the white layer alone; and a plane filed one frame away leaves it
//! alone too, because outside the analysed span there is nothing to draw.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, EffectParam, EffectValue, Layer, LayerKind, LinearColour, ProjectItem,
    SolidDef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::planes::{PlaneKind, PlaneOut};
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;
/// The model's own raster, deliberately not the comp's: the draw resamples.
const PLANE: u32 = 16;
/// Well inside the plane's far half.
const FAR: u32 = 12;
/// Well inside its near half.
const NEAR: u32 = 52;

/// A white layer wearing a Depth effect, with `view` and `invert` as asked, and
/// the effect's own id so a plane can be filed under it.
fn project(view: u32, invert: bool) -> (Arc<Document>, Uuid, Uuid) {
    let white = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: white,
        name: "white".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: COMP,
        height: COMP,
        extra: serde_json::Map::new(),
    }));

    let mut depth = lumit_core::fx::instantiate("depth").expect("depth is a built-in");
    for p in &mut depth.params {
        match p.id.as_str() {
            "view" => p.value = EffectValue::Choice(view),
            "invert" => p.value = EffectValue::Bool(invert),
            _ => {}
        }
    }
    let instance = depth.id;

    let mut base = layer("read me", LayerKind::Solid { def: white });
    base.effects = vec![depth];

    let comp = comp_of(vec![base]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id, instance)
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

fn comp_of(layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: COMP,
        height: COMP,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

/// A plane that is the bottom of its range on the left half of the frame and
/// the top on the right.
fn half_plane() -> PlaneOut {
    let mut data = Vec::with_capacity((PLANE as usize) * (PLANE as usize) * 2);
    for _ in 0..PLANE {
        for x in 0..PLANE {
            let v: u16 = if x < PLANE / 2 { 0 } else { u16::MAX };
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    PlaneOut {
        width: PLANE,
        height: PLANE,
        kind: PlaneKind::Depth,
        data,
    }
}

fn px(rgba: &[u8], w: u32, x: u32) -> [u8; 4] {
    let y = COMP / 2;
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// File `half_plane` for source frame 0 under `instance`.
fn publish(instance: Uuid, frame: i64) {
    let run = lumit_render::planes::run_from_planes(
        60.0,
        1,
        "Written; depth-test 1.0; ONNX Runtime none",
        &[(frame, half_plane())],
    )
    .expect("a run");
    lumit_render::planes::publish(instance, run);
}

#[test]
fn a_depth_plane_is_drawn_and_the_effect_passes_through_without_one() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };

    // Nothing analysed yet: the effect is a passthrough and the layer is whole.
    lumit_render::planes::clear();
    let (doc, comp, instance) = project(0, false);
    let (before, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("the plain render");
    assert_eq!(
        px(&before, w, FAR)[0],
        255,
        "with nothing analysed the Depth effect changes no pixel"
    );

    // The plane, filed for source frame 0 under this effect instance.
    publish(instance, 0);
    assert_eq!(lumit_render::planes::span(instance), Some((0, 0)));

    let (drawn, w, h) = r.render_rgba(&doc, comp, 0, 1.0).expect("the depth render");
    assert_eq!((w, h), (COMP, COMP), "rendered at comp size");
    let far = px(&drawn, w, FAR);
    let near = px(&drawn, w, NEAR);
    assert_eq!(far, [0, 0, 0, 255], "the far half is black and opaque");
    assert_eq!(near, [255, 255, 255, 255], "and the near half is white");

    // **Outside the span, passthrough.** A Solid layer's source frame is
    // nought whatever the playhead reads, so the plane is filed one frame away
    // instead of the playhead being moved.
    lumit_render::planes::clear();
    publish(instance, 1);
    let (missed, w, _) = r
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("the passthrough render");
    assert_eq!(
        px(&missed, w, FAR)[0],
        255,
        "a frame outside the analysed span wears a neighbour's plane"
    );
    lumit_render::planes::clear();
}

/// A red layer wearing a Remove background effect, with `view` and `invert` as
/// asked, and the effect's own id so a matte can be filed under it.
fn cutting(view: u32, invert: bool) -> (Arc<Document>, Uuid, Uuid) {
    let red = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: red,
        name: "red".into(),
        colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
        width: COMP,
        height: COMP,
        extra: serde_json::Map::new(),
    }));

    let mut cut = lumit_core::fx::instantiate("remove_background").expect("a built-in");
    for p in &mut cut.params {
        match p.id.as_str() {
            "view" => p.value = EffectValue::Choice(view),
            "invert" => p.value = EffectValue::Bool(invert),
            _ => {}
        }
    }
    let instance = cut.id;

    let mut base = layer("cut me out", LayerKind::Solid { def: red });
    base.effects = vec![cut];

    let comp = comp_of(vec![base]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id, instance)
}

/// A coverage that is nothing on the left half of the frame and all of the
/// subject on the right.
fn half_matte() -> PlaneOut {
    let mut data = Vec::with_capacity((PLANE as usize) * (PLANE as usize));
    for _ in 0..PLANE {
        for x in 0..PLANE {
            data.push(if x < PLANE / 2 { 0u8 } else { 255 });
        }
    }
    PlaneOut {
        width: PLANE,
        height: PLANE,
        kind: PlaneKind::Matte,
        data,
    }
}

/// File `half_matte` for source frame 0 under `instance`.
fn publish_matte(instance: Uuid) {
    let run = lumit_render::planes::run_from_planes(
        60.0,
        1,
        "Written; rvm-test 1.0; ONNX Runtime none",
        &[(0, half_matte())],
    )
    .expect("a run");
    lumit_render::planes::publish(instance, run);
}

/// **Composite view cuts the layer by the matte and Matte view draws the matte
/// itself** (docs/impl/addons.md §6.1, §11 test 8).
///
/// The layer is red and the coverage is nothing on the left and everything on
/// the right. Composited, the left half of the layer is gone and the right half
/// is the red it was; in Matte view the picture is the coverage as a grey one,
/// black on the left and white on the right; Invert swaps which half survives;
/// and with nothing analysed the effect changes no pixel.
#[test]
fn composite_view_cuts_the_layer_and_matte_view_draws_the_coverage() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };

    // Nothing analysed yet: the layer is whole.
    lumit_render::planes::clear();
    let (doc, comp, instance) = cutting(0, false);
    let (before, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("the plain render");
    assert_eq!(
        px(&before, w, FAR)[0],
        255,
        "with nothing analysed the effect cuts nothing"
    );

    publish_matte(instance);
    let (cut, w, h) = r.render_rgba(&doc, comp, 0, 1.0).expect("the cut render");
    assert_eq!((w, h), (COMP, COMP), "rendered at comp size");
    assert_eq!(
        px(&cut, w, FAR)[0],
        0,
        "the half the matte does not cover is cut away"
    );
    assert_eq!(
        px(&cut, w, NEAR),
        [255, 0, 0, 255],
        "and the half it covers is the layer's own red"
    );

    // Inverted: the background is what is kept.
    lumit_render::planes::clear();
    let (doc, comp, instance) = cutting(0, true);
    publish_matte(instance);
    let (turned, w, _) = r
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("the inverted render");
    assert_eq!(
        px(&turned, w, FAR),
        [255, 0, 0, 255],
        "inverted, the far half"
    );
    assert_eq!(px(&turned, w, NEAR)[0], 0);

    // Matte view: the coverage itself, as an opaque grey picture.
    lumit_render::planes::clear();
    let (doc, comp, instance) = cutting(1, false);
    publish_matte(instance);
    let (shown, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("the matte render");
    assert_eq!(px(&shown, w, FAR), [0, 0, 0, 255], "no coverage is black");
    assert_eq!(
        px(&shown, w, NEAR),
        [255, 255, 255, 255],
        "and all of it is white"
    );
    lumit_render::planes::clear();
}

/// **A layer read as another effect's matte source runs its own analysis
/// effects** (docs/impl/addons.md §13, and §11 test 7's second half).
///
/// A referenced layer's stack used to run with every side carriage empty, so a
/// Depth or a Roto brush on the layer somebody pointed a Matte row at rendered
/// as a passthrough and the consumer read the plain footage instead. Here the
/// source layer is a white solid wearing a Depth effect whose plane is black on
/// the left and white on the right, and Set matte reads its luminance: with the
/// carriage threaded through, the consuming layer is cut away on the left and
/// kept on the right. Without it the source reads as plain white and the
/// consumer is untouched.
#[test]
fn a_layer_read_as_a_matte_source_draws_its_own_plane() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    lumit_render::planes::clear();

    let white = Uuid::now_v7();
    let red = Uuid::now_v7();
    let mut doc = Document::new();
    for (id, name, colour) in [
        (white, "white", LinearColour([1.0, 1.0, 1.0, 1.0])),
        (red, "red", LinearColour([1.0, 0.0, 0.0, 1.0])),
    ] {
        doc.items.push(ProjectItem::Solid(SolidDef {
            id,
            name: name.into(),
            colour,
            width: COMP,
            height: COMP,
            extra: serde_json::Map::new(),
        }));
    }

    let mut depth = lumit_core::fx::instantiate("depth").expect("depth is a built-in");
    let instance = depth.id;
    for p in &mut depth.params {
        if p.id == "view" {
            p.value = EffectValue::Choice(0);
        }
    }
    let mut source = layer("the depth", LayerKind::Solid { def: white });
    source.effects = vec![depth];
    let source_id = source.id;

    let mut set_matte = lumit_core::fx::instantiate("set_matte").expect("a built-in");
    match set_matte.params.iter_mut().find(|p| p.id == "matte") {
        Some(p) => p.value = EffectValue::Layer(Some(source_id)),
        None => set_matte.params.push(EffectParam {
            id: "matte".into(),
            value: EffectValue::Layer(Some(source_id)),
            extra: serde_json::Map::new(),
        }),
    }
    assert_eq!(
        set_matte.layer_ref("matte"),
        Some(source_id),
        "the consumer is not pointed at the depth layer"
    );
    let mut consumer = layer("the consumer", LayerKind::Solid { def: red });
    consumer.effects = vec![set_matte];

    // The consumer on top, the depth layer under it and therefore behind it.
    let comp = comp_of(vec![consumer, source]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);

    publish(instance, 0);
    let (cut, w, _) = r.render_rgba(&doc, comp_id, 0, 1.0).expect("the render");
    // Where the plane is white the consumer is kept; where it is black the
    // consumer is cut away and the depth layer behind it shows, which on its
    // own far half is black. Without the carriage the source would read as
    // plain white everywhere, the consumer would be kept everywhere, and the
    // far half would be red.
    assert_eq!(
        px(&cut, w, NEAR),
        [255, 0, 0, 255],
        "the near half kept the consuming layer"
    );
    assert_eq!(
        px(&cut, w, FAR),
        [0, 0, 0, 255],
        "the far half is still red, so the referenced layer's own plane did \
         not reach its stack"
    );
    lumit_render::planes::clear();
}

/// A node graph whose picture is a red solid `size` pixels square, read
/// through a Transform box at its defaults into a Remove background box, and
/// that box's own id so a matte can be filed under it.
fn graph_cutting(size: u32) -> (Arc<Document>, Uuid, Uuid) {
    use lumit_core::comp_graph::{CompGraph, GraphEdge, GraphNode};

    let red = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: red,
        name: "red".into(),
        colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
        width: size,
        height: size,
        extra: serde_json::Map::new(),
    }));

    let cut = lumit_core::fx::instantiate("remove_background").expect("a built-in");
    let instance = cut.id;
    // A box between the Read and the one that cuts, so the file is found
    // through it rather than only beside it.
    let pass = lumit_core::fx::instantiate("transform").expect("a built-in");
    let (read, through, out) = (Uuid::now_v7(), pass.id, Uuid::now_v7());
    let wire = |from: Uuid, to: Uuid| GraphEdge {
        from,
        from_port: "output".into(),
        to,
        to_port: "input".into(),
    };

    let mut comp = comp_of(Vec::new());
    comp.graph = Some(CompGraph {
        nodes: vec![
            GraphNode::Read {
                id: read,
                item: red,
                custom_name: None,
            },
            GraphNode::Fx(pass),
            GraphNode::Fx(cut),
            GraphNode::Output { id: out },
        ],
        edges: vec![
            wire(read, through),
            wire(through, instance),
            wire(instance, out),
        ],
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    });
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id, instance)
}

/// **A Remove background box in a node graph cuts by the matte of the file its
/// picture comes from**, and lays that matte where the file's picture is.
///
/// A box has no layer, so nothing used to carry a matte to it: Analyse did
/// nothing and the box passed its picture through. Here the picture is a red
/// solid read through one box into the one that cuts, and the coverage is
/// nothing on the left and everything on the right, as it is for the layer
/// above.
///
/// The second half is the placement. A Read lays its item centred at its own
/// size, so a solid half the frame's width sits in the middle of it, and the
/// matte made from that solid has to sit there with it: the cut falls down the
/// middle of the solid, which is the middle of the frame, and not down the
/// middle of some stretched copy.
#[test]
fn a_box_in_a_node_graph_cuts_by_the_matte_of_the_file_behind_it() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };

    // Nothing analysed yet: the box changes no pixel.
    lumit_render::planes::clear();
    let (doc, comp, instance) = graph_cutting(COMP);
    let (before, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("the plain render");
    assert_eq!(px(&before, w, FAR), [255, 0, 0, 255]);

    publish_matte(instance);
    let (cut, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("the cut render");
    assert_eq!(
        px(&cut, w, FAR)[0],
        0,
        "the half the matte does not cover is cut away"
    );
    assert_eq!(
        px(&cut, w, NEAR),
        [255, 0, 0, 255],
        "and the half it covers is the solid's own red"
    );

    // A solid half the frame wide: it sits from a quarter to three quarters of
    // the way across, and its own left half is what goes.
    lumit_render::planes::clear();
    let (doc, comp, instance) = graph_cutting(COMP / 2);
    publish_matte(instance);
    let (placed, w, _) = r
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("the placed render");
    assert_eq!(
        px(&placed, w, COMP / 2 - 8)[0],
        0,
        "the solid's left half is cut, where the solid is"
    );
    assert_eq!(
        px(&placed, w, COMP / 2 + 8),
        [255, 0, 0, 255],
        "and its right half is kept"
    );
    lumit_render::planes::clear();
}

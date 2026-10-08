//! **The node graph composition, end to end** (docs/impl/node-graph-comp.md
//! §7 item 9).
//!
//! # In plain terms
//!
//! A node graph is a composition whose picture is made by boxes and wires. These
//! tests build such comps the way a user would and push them through the same
//! public entries the Viewer and the exporter use, because what they are really
//! asking is whether the new walk agrees with the old one:
//!
//! - **A Read box is the layer it behaves like.** A solid wired to the Output is
//!   that solid, and moving the Output's wire shows the other one.
//! - **A fork and a Merge are a stack.** One Read into two effects, merged with
//!   a blend mode, is the picture two Precomp layers of the same content with
//!   the same effects make - at full size and at half preview resolution, which
//!   is the render scale riding the same equivalence.
//! - **Merge and Switch behave as the note says**, including a driver wire into
//!   a Switch's Index moving the picture between frames.
//! - **A node graph is a comp**, so it can be placed as a Precomp layer, used as
//!   a matte source and fed to an effect, with nothing written for those three.
//! - **The Node graph effect is the same walk**, applied to a layer: equal to
//!   the effects inline, taking a second picture from a layer row, taking a
//!   keyed value from its host, and equal to the flattened graph when nested.
//! - **A cycle degrades**, and the frame still renders.
//! - **The picture at a box** (§4.5) is a different picture with a name of its
//!   own.
//!
//! The oracles are deliberately Precomp layers rather than plain solids. Every
//! box's picture is the graph's own frame (§1.6), while a layer's picture is its
//! own raster, so only a comp-sized intermediate makes the two comparable at a
//! reduced preview - which is exactly what a Precomp layer is.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::comp_graph::{CompGraph, GraphEdge, GraphInput, GraphNode, InputKind};
use lumit_core::model::{
    BlendMode, Composition, Document, EffectInstance, EffectParam, EffectValue, Layer, LayerKind,
    LinearColour, MatteChannel, MatteRef, ProjectItem, SolidDef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;

fn solid(def: Uuid, name: &str, colour: [f32; 4], w: u32, h: u32) -> ProjectItem {
    ProjectItem::Solid(SolidDef {
        id: def,
        name: name.into(),
        colour: LinearColour(colour),
        width: w,
        height: h,
        extra: serde_json::Map::new(),
    })
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

fn comp_of(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
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

/// A composition that holds a graph instead of layers.
fn graph_comp(name: &str, nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> Composition {
    let mut c = comp_of(name, Vec::new());
    c.graph = Some(CompGraph {
        nodes,
        edges,
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    });
    c
}

fn wire(from: Uuid, from_port: &str, to: Uuid, to_port: &str) -> GraphEdge {
    GraphEdge {
        from,
        from_port: from_port.to_owned(),
        to,
        to_port: to_port.to_owned(),
    }
}

fn read(item: Uuid) -> GraphNode {
    GraphNode::Read {
        id: Uuid::now_v7(),
        item,
        custom_name: None,
    }
}

fn picture_input(id: &str) -> GraphNode {
    GraphNode::Input {
        id: Uuid::now_v7(),
        input: GraphInput {
            id: id.into(),
            label: id.into(),
            kind: InputKind::Picture,
            default: [0.0; 4],
            min: 0.0,
            max: 1.0,
            unit: lumit_core::fx::Unit::Raw,
            preview: None,
        },
    }
}

fn number_input(id: &str, default: f64) -> GraphNode {
    GraphNode::Input {
        id: Uuid::now_v7(),
        input: GraphInput {
            id: id.into(),
            label: id.into(),
            kind: InputKind::Number,
            default: [default, 0.0, 0.0, 0.0],
            min: -8.0,
            max: 8.0,
            unit: lumit_core::fx::Unit::Raw,
            preview: None,
        },
    }
}

fn output() -> GraphNode {
    GraphNode::Output { id: Uuid::now_v7() }
}

/// A fresh builtin instance with `edits` applied to its float rows.
fn effect(name: &str, edits: &[(&str, f64)]) -> EffectInstance {
    let mut inst =
        lumit_core::fx::instantiate(name).unwrap_or_else(|| panic!("{name} is a builtin"));
    for (id, value) in edits {
        set_float(&mut inst, id, *value);
    }
    inst
}

/// Edit a row the instance already carries. A row it does not carry is a name
/// this test got wrong, and pushing one would hide that.
fn set_float(inst: &mut EffectInstance, id: &str, value: f64) {
    let row = inst
        .params
        .iter_mut()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("{id} is a row of this effect"));
    row.value = EffectValue::Float(lumit_core::anim::Property::fixed(value));
}

fn set_value(inst: &mut EffectInstance, id: &str, value: EffectValue) {
    match inst.params.iter_mut().find(|p| p.id == id) {
        Some(p) => p.value = value,
        None => inst.params.push(EffectParam {
            id: id.into(),
            value,
            extra: serde_json::Map::new(),
        }),
    }
}

/// A Node graph effect bound to `comp`, with the Inputs copy the panel writes.
fn node_graph_effect(comp: &Composition) -> EffectInstance {
    let mut inst = lumit_core::fx::instantiate("node_graph").unwrap();
    let graph = comp.graph.as_ref().expect("a node graph comp");
    lumit_core::fx::effects::node_graph::bind(&mut inst, comp.id, graph);
    inst
}

fn rgb(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let d = ((y * w + x) * 4) as usize;
    [rgba[d], rgba[d + 1], rgba[d + 2]]
}

struct Stub;
impl lumit_eval::SourceStamper for Stub {
    fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
        Some((format!("stub:{item}"), (lt * 60.0).round().max(0.0) as u64))
    }
}

fn key_of(doc: &Arc<Document>, comp: Uuid) -> lumit_eval::FrameKey {
    let comp = doc.comp(comp).expect("the comp is filed").clone();
    lumit_eval::comp_frame_key(doc, &comp, 0.0, lumit_eval::Quality::default(), &Stub)
        .expect("a solid comp is always keyable")
}

/// Half preview resolution: the render scale the Viewer's Auto setting reaches,
/// which is the one `realise_graph` carries through its Read placements.
fn half() -> lumit_render::Quality {
    lumit_render::Quality {
        auto_res: true,
        display_scale: 0.5,
        ..lumit_render::Quality::default()
    }
}

/// **Test 9b - a fork and a Merge are a stack.** One Read into two effects,
/// merged with a blend mode and an opacity, is byte for byte the picture two
/// Precomp layers of the same content wearing the same effects make - at full
/// size and at half preview resolution.
#[test]
fn a_fork_merged_is_the_two_layer_comp_it_claims_to_be() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let plate = |doc: &mut Document| -> Uuid {
        let id = Uuid::now_v7();
        doc.items
            .push(solid(id, "plate", [0.4, 0.6, 0.2, 1.0], COMP, COMP));
        id
    };
    let (over, under) = (
        || effect("blur", &[("radius", 4.0)]),
        || effect("exposure", &[("stops", 1.0)]),
    );

    // The graph: one Read forked into the two effects, merged A over B.
    let mut doc_g = Document::new();
    let item = plate(&mut doc_g);
    let (src, a, b, merge, out) = (
        read(item),
        GraphNode::Fx(over()),
        GraphNode::Fx(under()),
        GraphNode::Fx(effect("merge", &[("opacity", 60.0)])),
        output(),
    );
    let (src_id, a_id, b_id, merge_id, out_id) = (src.id(), a.id(), b.id(), merge.id(), out.id());
    // Mode is a Choice row, not a float: Multiply is index 2 of BlendMode::ALL.
    let GraphNode::Fx(mut merge_inst) = merge else {
        unreachable!()
    };
    set_value(&mut merge_inst, "mode", EffectValue::Choice(2));
    let comp_g = graph_comp(
        "graph",
        vec![src, a, b, GraphNode::Fx(merge_inst), out],
        vec![
            wire(src_id, "output", a_id, "input"),
            wire(src_id, "output", b_id, "input"),
            wire(a_id, "output", merge_id, "input"),
            wire(b_id, "output", merge_id, "background"),
            wire(merge_id, "output", out_id, "input"),
        ],
    );
    let comp_g_id = comp_g.id;
    doc_g.items.push(ProjectItem::Composition(comp_g));
    let doc_g = Arc::new(doc_g);

    // The oracle: the same plate packed into a comp, two Precomp layers of it,
    // the same effects, the top one Multiply at 60.
    let mut doc_p = Document::new();
    let item = plate(&mut doc_p);
    let nested = comp_of(
        "packed",
        vec![layer("plate", LayerKind::Solid { def: item })],
    );
    let nested_id = nested.id;
    doc_p.items.push(ProjectItem::Composition(nested));
    let mut top = layer("A", LayerKind::Precomp { comp: nested_id });
    top.effects = vec![over()];
    top.blend = BlendMode::Multiply;
    top.transform.opacity = lumit_core::anim::Property::fixed(60.0);
    let mut bottom = layer("B", LayerKind::Precomp { comp: nested_id });
    bottom.effects = vec![under()];
    let comp_p = comp_of("stacked", vec![top, bottom]);
    let comp_p_id = comp_p.id;
    doc_p.items.push(ProjectItem::Composition(comp_p));
    let doc_p = Arc::new(doc_p);

    let (g, w, _) = r.render_rgba(&doc_g, comp_g_id, 0, 1.0).unwrap();
    let (p, pw, _) = r.render_rgba(&doc_p, comp_p_id, 0, 1.0).unwrap();
    assert_eq!((w, pw), (COMP, COMP));
    assert_eq!(g, p, "a fork merged must be the two-layer picture exactly");

    // And at half the render scale, where the graph's frame and the Precomp's
    // intermediate are both half size.
    let (g, ..) = r.render_preview(&doc_g, comp_g_id, 0, half(), 1.0).unwrap();
    let (p, ..) = r.render_preview(&doc_p, comp_p_id, 0, half(), 1.0).unwrap();
    assert_eq!(g, p, "and the same at half preview resolution");
}

/// **Test 9d - the Switch.** It shows the picture its Index names, transparent
/// out of range, and a Wiggle wired into the Index moves the picture between
/// frames.
#[test]
fn a_switch_shows_the_picture_its_index_names() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let build = |index: i32, wiggle: bool| {
        let mut doc = Document::new();
        let (red, blue) = (Uuid::now_v7(), Uuid::now_v7());
        doc.items
            .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
        doc.items
            .push(solid(blue, "blue", [0.0, 0.0, 1.0, 1.0], COMP, COMP));
        let switch = effect("switch", &[("index", f64::from(index))]);
        let (ra, rb, sw, out) = (read(red), read(blue), GraphNode::Fx(switch), output());
        let (ra_id, rb_id, sw_id, out_id) = (ra.id(), rb.id(), sw.id(), out.id());
        let mut nodes = vec![ra, rb, sw, out];
        let mut edges = vec![
            wire(ra_id, "output", sw_id, "in0"),
            wire(rb_id, "output", sw_id, "in1"),
            wire(sw_id, "output", out_id, "input"),
        ];
        if wiggle {
            // A wobble about the stored index 0, wide enough and quick
            // enough to cross the boundary between the two pictures as the
            // frames go by. **The id is pinned**: Wiggle seeds its noise from
            // the box's id, so a fresh one each run would be a fresh wobble
            // each run, and this test would pass or fail by luck.
            let mut w = effect("wiggle", &[("amount", 4.0), ("frequency", 20.0)]);
            w.id = Uuid::from_u128(0x5377_6974_6368_5f77_6f62_626c_655f_3031);
            let w_id = w.id;
            nodes.push(GraphNode::Fx(w));
            edges.push(wire(w_id, "value", sw_id, "index"));
        }
        let comp = graph_comp("graph", nodes, edges);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    let frame = |r: &mut HeadlessRenderer, index, wiggle, f| {
        let (doc, comp) = build(index, wiggle);
        let (px, w, _) = r.render_rgba(&doc, comp, f, 1.0).unwrap();
        rgb(&px, w, 32, 32)
    };

    let zero = frame(&mut r, 0, false, 0);
    assert!(zero[0] > 200, "Index 0 shows the first picture: {zero:?}");
    let one = frame(&mut r, 1, false, 0);
    assert!(one[2] > 200, "Index 1 shows the second: {one:?}");
    let past = frame(&mut r, 7, false, 0);
    assert_eq!(past, [0, 0, 0], "out of range reads transparent");

    // A driver wired into the Index: somewhere across half a second the wobble
    // must cross from one picture to the other.
    let (doc, comp) = build(0, true);
    let wobbled: Vec<[u8; 3]> = (0..30)
        .map(|f| {
            let (px, w, _) = r.render_rgba(&doc, comp, f, 1.0).unwrap();
            rgb(&px, w, 32, 32)
        })
        .collect();
    assert!(
        wobbled.iter().any(|p| *p != wobbled[0]),
        "a Wiggle into the Index must change the picture between frames: {wobbled:?}"
    );
}

/// **Test 9e - a node graph is a comp.** Placed as a Precomp layer, used as a
/// matte source and fed to an effect's layer row, it renders exactly as a layer
/// comp of the same picture does - none of which has a line written for it.
#[test]
fn a_node_graph_is_read_wherever_a_comp_is() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // Two documents that differ only in whether the inner comp is a graph or a
    // stack. `use_it` builds the parent's layers from the inner comp's id and
    // hands back the items its own layers name, so both documents hold the
    // same project and only the inner comp differs.
    type Parent = dyn Fn(Uuid) -> (Vec<Layer>, Vec<ProjectItem>);
    let pair = |use_it: &Parent| -> [(Arc<Document>, Uuid); 2] {
        let mut built = Vec::new();
        for graphed in [false, true] {
            let mut doc = Document::new();
            let grey = Uuid::now_v7();
            doc.items
                .push(solid(grey, "grey", [0.6, 0.6, 0.6, 1.0], 24, 24));
            let inner = if graphed {
                let (src, o) = (read(grey), output());
                let (src_id, o_id) = (src.id(), o.id());
                graph_comp(
                    "inner",
                    vec![src, o],
                    vec![wire(src_id, "output", o_id, "input")],
                )
            } else {
                // A Read box places its item centred at its natural size,
                // which is what a fresh layer of it gets.
                let mut l = layer("grey", LayerKind::Solid { def: grey });
                l.transform.anchor_x = lumit_core::anim::Property::fixed(12.0);
                l.transform.anchor_y = lumit_core::anim::Property::fixed(12.0);
                l.transform.position_x = lumit_core::anim::Property::fixed(f64::from(COMP) / 2.0);
                l.transform.position_y = lumit_core::anim::Property::fixed(f64::from(COMP) / 2.0);
                comp_of("inner", vec![l])
            };
            let inner_id = inner.id;
            doc.items.push(ProjectItem::Composition(inner));
            let (layers, items) = use_it(inner_id);
            doc.items.extend(items);
            let comp = comp_of("parent", layers);
            let id = comp.id;
            doc.items.push(ProjectItem::Composition(comp));
            built.push((Arc::new(doc), id));
        }
        [built.remove(0), built.remove(0)]
    };
    let same = |r: &mut HeadlessRenderer, what: &str, use_it: &Parent| {
        let [(sd, sc), (gd, gc)] = pair(use_it);
        let (stacked, w, _) = r.render_rgba(&sd, sc, 0, 1.0).unwrap();
        let (graphed, gw, _) = r.render_rgba(&gd, gc, 0, 1.0).unwrap();
        assert_eq!((w, gw), (COMP, COMP));
        assert_eq!(
            stacked, graphed,
            "{what}: a node graph must read as any comp"
        );
        assert_ne!(
            rgb(&graphed, w, 32, 32),
            [0, 0, 0],
            "{what}: and there must be a picture to compare"
        );
    };

    // Placed as a Precomp layer.
    same(&mut r, "placed", &|inner| {
        (
            vec![layer("pre", LayerKind::Precomp { comp: inner })],
            Vec::new(),
        )
    });

    // As a matte source: a full-frame red gated by the box's alpha.
    same(&mut r, "matted", &|inner| {
        let source = layer("source", LayerKind::Precomp { comp: inner });
        let red = Uuid::now_v7();
        let mut over = layer("over", LayerKind::Solid { def: red });
        over.matte = Some(MatteRef {
            layer: source.id,
            channel: MatteChannel::Alpha,
            inverted: false,
            source: Default::default(),
        });
        (
            vec![over, source],
            vec![solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP)],
        )
    });

    // As a Light wrap's background plate, which is the layer-input carriage.
    same(&mut r, "as a plate", &|inner| {
        let plate = layer("plate", LayerKind::Precomp { comp: inner });
        let dark = Uuid::now_v7();
        let mut host = layer("host", LayerKind::Solid { def: dark });
        let mut wrap = effect("light_wrap", &[]);
        set_value(&mut wrap, "background", EffectValue::Layer(Some(plate.id)));
        host.effects = vec![wrap];
        (
            vec![host, plate],
            vec![solid(dark, "dark", [0.05, 0.05, 0.05, 1.0], COMP, COMP)],
        )
    });
}

/// **Test 9f - the Node graph effect on a solid equals the same effects
/// applied inline.** A graph with no Read boxes is a reusable effect with a
/// picture in and a picture out, and this is that claim, pinned.
#[test]
fn the_node_graph_effect_equals_the_same_effects_inline() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let fx = || effect("exposure", &[("stops", 1.5)]);
    let build = |graphed: bool| {
        let mut doc = Document::new();
        let grey = Uuid::now_v7();
        doc.items
            .push(solid(grey, "grey", [0.3, 0.4, 0.5, 1.0], COMP, COMP));
        let mut host = layer("host", LayerKind::Solid { def: grey });
        if graphed {
            // A graph with no Read boxes: a reusable effect with a picture in
            // and a picture out (§1.5).
            let (src, e, o) = (picture_input("src"), GraphNode::Fx(fx()), output());
            let (src_id, e_id, o_id) = (src.id(), e.id(), o.id());
            let inner = graph_comp(
                "preset",
                vec![src, e, o],
                vec![
                    wire(src_id, "output", e_id, "input"),
                    wire(e_id, "output", o_id, "input"),
                ],
            );
            host.effects = vec![node_graph_effect(&inner)];
            doc.items.push(ProjectItem::Composition(inner));
        } else {
            host.effects = vec![fx()];
        }
        let comp = comp_of("parent", vec![host]);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    let (id, ic) = build(false);
    let (gd, gc) = build(true);
    let (inline, w, _) = r.render_rgba(&id, ic, 0, 1.0).unwrap();
    let (graphed, gw, _) = r.render_rgba(&gd, gc, 0, 1.0).unwrap();
    assert_eq!((w, gw), (COMP, COMP));
    assert_eq!(
        inline, graphed,
        "a graph applied as an effect is the effects inside it"
    );
    assert!(
        rgb(&inline, w, 32, 32) != [0, 0, 0],
        "and the exposure did something to compare"
    );
}

/// **Test 9f, the rest.** A second picture Input fed from a layer row arrives
/// on the depth-pass carriage; a value Input keyed on the host moves the
/// picture; and a graph nested in a graph is the flattened graph.
#[test]
fn a_graphs_inputs_reach_it_from_the_host_and_from_the_graph_above() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };

    // A second picture Input: the graph lays the named layer over its own
    // input, so the finished picture is that layer alone.
    let mut doc = Document::new();
    let (red, blue) = (Uuid::now_v7(), Uuid::now_v7());
    doc.items
        .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
    doc.items
        .push(solid(blue, "blue", [0.0, 0.0, 1.0, 1.0], COMP, COMP));
    let (src, plate, merge, o) = (
        picture_input("src"),
        picture_input("plate"),
        GraphNode::Fx(effect("merge", &[])),
        output(),
    );
    let (src_id, plate_id, merge_id, o_id) = (src.id(), plate.id(), merge.id(), o.id());
    let inner = graph_comp(
        "over",
        vec![src, plate, merge, o],
        vec![
            wire(src_id, "output", merge_id, "background"),
            wire(plate_id, "output", merge_id, "input"),
            wire(merge_id, "output", o_id, "input"),
        ],
    );
    let mut host = layer("host", LayerKind::Solid { def: red });
    let mut inst = node_graph_effect(&inner);
    let blue_layer = layer("blue", LayerKind::Solid { def: blue });
    set_value(&mut inst, "plate", EffectValue::Layer(Some(blue_layer.id)));
    host.effects = vec![inst];
    doc.items.push(ProjectItem::Composition(inner));
    let mut blue_hidden = blue_layer.clone();
    blue_hidden.switches.visible = false;
    let comp = comp_of("parent", vec![host, blue_hidden]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let (px, w, _) = r.render_rgba(&Arc::new(doc), comp_id, 0, 1.0).unwrap();
    assert!(
        rgb(&px, w, 32, 32)[2] > 200 && rgb(&px, w, 32, 32)[0] < 60,
        "the second picture Input must arrive from the layer row: {:?}",
        rgb(&px, w, 32, 32)
    );

    // A value Input keyed on the host: two frames, two pictures.
    let mut doc = Document::new();
    let grey = Uuid::now_v7();
    doc.items
        .push(solid(grey, "grey", [0.3, 0.3, 0.3, 1.0], COMP, COMP));
    let (src, amount, e, o) = (
        picture_input("src"),
        number_input("amount", 0.0),
        GraphNode::Fx(effect("exposure", &[])),
        output(),
    );
    let (src_id, amount_id, e_id, o_id) = (src.id(), amount.id(), e.id(), o.id());
    let inner = graph_comp(
        "graded",
        vec![src, amount, e, o],
        vec![
            wire(src_id, "output", e_id, "input"),
            wire(amount_id, "value", e_id, "stops"),
            wire(e_id, "output", o_id, "input"),
        ],
    );
    let mut inst = node_graph_effect(&inner);
    set_value(
        &mut inst,
        "amount",
        EffectValue::Float(lumit_core::anim::Property {
            animation: lumit_core::anim::Animation::Keyframed(vec![
                lumit_core::anim::Keyframe {
                    time: Rational::new(0, 1).unwrap(),
                    value: 0.0,
                    interp_in: lumit_core::anim::SideInterp::Linear,
                    interp_out: lumit_core::anim::SideInterp::Linear,
                },
                lumit_core::anim::Keyframe {
                    time: Rational::new(1, 1).unwrap(),
                    value: 2.0,
                    interp_in: lumit_core::anim::SideInterp::Linear,
                    interp_out: lumit_core::anim::SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        }),
    );
    let mut host = layer("host", LayerKind::Solid { def: grey });
    host.effects = vec![inst];
    doc.items.push(ProjectItem::Composition(inner));
    let comp = comp_of("parent", vec![host]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let (first, w, _) = r.render_rgba(&doc, comp_id, 0, 1.0).unwrap();
    let (later, ..) = r.render_rgba(&doc, comp_id, 45, 1.0).unwrap();
    assert_ne!(
        rgb(&first, w, 32, 32),
        rgb(&later, w, 32, 32),
        "a keyed value Input on the host must move the picture"
    );

    // And a driver on the host wired into that same row: the wire substitutes
    // where the keyframes would have been read, exactly as it does on any
    // other effect's parameter.
    let mut driven = (*doc).clone();
    if let Some(comp) = driven.comp_mut(comp_id) {
        let host = &mut comp.layers[0];
        set_value(
            &mut host.effects[0],
            "amount",
            EffectValue::Float(lumit_core::anim::Property::fixed(0.0)),
        );
        let mut wiggle = effect("wiggle", &[("amount", 3.0), ("frequency", 4.0)]);
        // The id seeds the wobble, and about 3% of random ids stay white on every frame.
        wiggle.id = Uuid::from_u128(3);
        let wiggle_id = wiggle.id;
        let target = host.effects[0].id;
        host.graph = lumit_core::graph::LayerGraph {
            out_unwired: false,
            groups: Vec::new(),
            nodes: vec![wiggle],
            edges: vec![lumit_core::graph::Edge {
                from: lumit_core::graph::OutputRef::Driver {
                    node: wiggle_id,
                    port: "value".into(),
                },
                to: lumit_core::graph::InputRef::Param {
                    node: lumit_core::graph::NodeRef::Effect(target),
                    port: "amount".into(),
                },
            }],
            layout: Vec::new(),
            exposed: Vec::new(),
        };
    }
    let driven = Arc::new(driven);
    let wobbled: Vec<[u8; 3]> = (0..20)
        .map(|f| {
            let (px, w, _) = r.render_rgba(&driven, comp_id, f, 1.0).unwrap();
            rgb(&px, w, 32, 32)
        })
        .collect();
    assert!(
        wobbled.iter().any(|p| *p != wobbled[0]),
        "a driver on the host must reach the graph's own Input"
    );

    // A graph nested in a graph is the flattened graph.
    let flat = |nested: bool| {
        let mut doc = Document::new();
        let grey = Uuid::now_v7();
        doc.items
            .push(solid(grey, "grey", [0.3, 0.4, 0.2, 1.0], COMP, COMP));
        let e = || effect("exposure", &[("stops", 1.0)]);
        let comp = if nested {
            let (src, fx, o) = (picture_input("src"), GraphNode::Fx(e()), output());
            let (src_id, fx_id, o_id) = (src.id(), fx.id(), o.id());
            let inner = graph_comp(
                "inner",
                vec![src, fx, o],
                vec![
                    wire(src_id, "output", fx_id, "input"),
                    wire(fx_id, "output", o_id, "input"),
                ],
            );
            let (src, box_, o) = (
                read(grey),
                GraphNode::Fx(node_graph_effect(&inner)),
                output(),
            );
            let (src_id, box_id, o_id) = (src.id(), box_.id(), o.id());
            doc.items.push(ProjectItem::Composition(inner));
            graph_comp(
                "outer",
                vec![src, box_, o],
                vec![
                    wire(src_id, "output", box_id, "input"),
                    wire(box_id, "output", o_id, "input"),
                ],
            )
        } else {
            let (src, fx, o) = (read(grey), GraphNode::Fx(e()), output());
            let (src_id, fx_id, o_id) = (src.id(), fx.id(), o.id());
            graph_comp(
                "flat",
                vec![src, fx, o],
                vec![
                    wire(src_id, "output", fx_id, "input"),
                    wire(fx_id, "output", o_id, "input"),
                ],
            )
        };
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    let (fd, fc) = flat(false);
    let (nd, nc) = flat(true);
    let (flattened, w, _) = r.render_rgba(&fd, fc, 0, 1.0).unwrap();
    let (nested, ..) = r.render_rgba(&nd, nc, 0, 1.0).unwrap();
    assert_eq!(
        flattened, nested,
        "a graph nested in a graph is the flattened graph"
    );
    assert!(
        rgb(&flattened, w, 32, 32) != [0, 0, 0],
        "and there is a picture to compare"
    );
}

/// **Test 9g - a cycle degrades to a passthrough and renders.** A graph whose
/// own box names the comp it sits in hands its input on, exactly as a dangling
/// reference does everywhere else.
#[test]
fn a_cycle_through_a_node_graph_box_renders_a_passthrough() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut doc = Document::new();
    let red = Uuid::now_v7();
    doc.items
        .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
    // The comp's own id is minted first, so its graph can name itself.
    let mut comp = graph_comp("self", Vec::new(), Vec::new());
    let comp_id = comp.id;
    let mut inst = lumit_core::fx::instantiate("node_graph").unwrap();
    lumit_core::fx::effects::node_graph::bind(&mut inst, comp_id, &CompGraph::new_with_output());
    let (src, box_, o) = (read(red), GraphNode::Fx(inst), output());
    let (src_id, box_id, o_id) = (src.id(), box_.id(), o.id());
    comp.graph = Some(CompGraph {
        nodes: vec![src, box_, o],
        edges: vec![
            wire(src_id, "output", box_id, "input"),
            wire(box_id, "output", o_id, "input"),
        ],
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    });
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);
    let (px, w, _) = r.render_rgba(&doc, comp_id, 0, 1.0).unwrap();
    assert!(
        rgb(&px, w, 32, 32)[0] > 200,
        "a cycle hands the picture on rather than faulting: {:?}",
        rgb(&px, w, 32, 32)
    );
    // And the frame still has a name.
    key_of(&doc, comp_id);
}

/// **Test 21 - a placed graph reads the values the layer hands it** (§5.3). A
/// node graph placed as a Precomp layer used to be stuck on its own defaults:
/// the layer had nowhere to put a value and the lowering had nowhere to read
/// one. Now the layer carries a `node_graph` instance of its own, and the two
/// claims are that a keyed value moves the picture over time, and that the
/// picture is the very one the same graph applied as an effect makes with the
/// same values - the two roads into one lowering.
#[test]
fn a_placed_graphs_own_input_values_move_its_picture() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // Stops ramp nought to three over the comp's first second, so which frame
    // is on screen is a different brightness.
    let keyed = || {
        EffectValue::Float(lumit_core::anim::Property {
            animation: lumit_core::anim::Animation::Keyframed(vec![
                lumit_core::anim::Keyframe {
                    time: Rational::ZERO,
                    value: 0.0,
                    interp_in: lumit_core::anim::SideInterp::Linear,
                    interp_out: lumit_core::anim::SideInterp::Linear,
                },
                lumit_core::anim::Keyframe {
                    time: Rational::new(1, 1).unwrap(),
                    value: 3.0,
                    interp_in: lumit_core::anim::SideInterp::Linear,
                    interp_out: lumit_core::anim::SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        })
    };

    // The graph: a Read of a grey solid through an Exposure whose Stops is an
    // Input, so the host's value is the whole of what the picture shows.
    let build = |placed: bool| {
        let mut doc = Document::new();
        let grey = Uuid::now_v7();
        doc.items
            .push(solid(grey, "grey", [0.2, 0.25, 0.3, 1.0], COMP, COMP));
        let (r_node, amount, fx, out) = (
            read(grey),
            number_input("amount", 0.0),
            GraphNode::Fx(effect("exposure", &[("stops", 0.0)])),
            output(),
        );
        let (r_id, a_id, fx_id, o_id) = (r_node.id(), amount.id(), fx.id(), out.id());
        let inner = graph_comp(
            "graph",
            vec![r_node, amount, fx, out],
            vec![
                wire(r_id, "output", fx_id, "input"),
                wire(a_id, "value", fx_id, "stops"),
                wire(fx_id, "output", o_id, "input"),
            ],
        );
        let mut inst = node_graph_effect(&inner);
        set_value(&mut inst, "amount", keyed());
        let host = if placed {
            // Placed as a Precomp layer, with the values on the layer.
            let mut l = layer("placed", LayerKind::Precomp { comp: inner.id });
            l.graph_inputs = Some(inst);
            l
        } else {
            // Applied as an effect on a solid. The graph's Output comes off its
            // own Read, so the host's own picture is replaced whole and the two
            // roads must land on one frame.
            let mut l = layer("host", LayerKind::Solid { def: grey });
            l.effects = vec![inst];
            l
        };
        doc.items.push(ProjectItem::Composition(inner));
        let comp = comp_of("parent", vec![host]);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };

    let (pd, pc) = build(true);
    let (ad, ac) = build(false);
    let shot = |r: &mut HeadlessRenderer, doc: &Arc<Document>, comp: Uuid, frame: u64| {
        r.render_rgba(doc, comp, frame, 1.0).expect("the render").0
    };
    let early = shot(&mut r, &pd, pc, 0);
    let late = shot(&mut r, &pd, pc, 30);
    assert_ne!(
        early, late,
        "a keyed Input value on the layer must move the picture"
    );
    assert_eq!(
        early,
        shot(&mut r, &ad, ac, 0),
        "and the placed graph is the applied graph with the same values"
    );
    assert_eq!(late, shot(&mut r, &ad, ac, 30));

    // And at half the render scale, where the graph's frame and the Precomp's
    // intermediate are both half size.
    let (placed, ..) = r.render_preview(&pd, pc, 30, half(), 1.0).unwrap();
    let (applied, ..) = r.render_preview(&ad, ac, 30, half(), 1.0).unwrap();
    assert_eq!(placed, applied, "and the same at half preview resolution");
}

/// A half-transparent plate, so a straight channel and a premultiplied one
/// differ.
const PLATE: [f32; 4] = [0.8, 0.4, 0.2, 0.5];

/// The plate read into a Split channels box, then `rest` adds the boxes and
/// wires after it, handed the Split's id and the Output's.
fn split_graph(
    enabled: bool,
    rest: impl FnOnce(Uuid, Uuid, &mut Vec<GraphNode>, &mut Vec<GraphEdge>),
) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let plate = Uuid::now_v7();
    doc.items.push(solid(plate, "plate", PLATE, COMP, COMP));
    let mut split = effect("split_channels", &[]);
    split.enabled = enabled;
    let (src, split, out) = (read(plate), GraphNode::Fx(split), output());
    let (split_id, out_id) = (split.id(), out.id());
    let mut edges = vec![wire(src.id(), "output", split_id, "input")];
    let mut nodes = vec![src, split, out];
    rest(split_id, out_id, &mut nodes, &mut edges);
    let mut comp = graph_comp("graph", nodes, edges);
    comp.background = LinearColour([0.0; 4]);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

/// The plate read straight into the Output, for the pictures above to be
/// measured against.
fn plate_alone() -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let plate = Uuid::now_v7();
    doc.items.push(solid(plate, "plate", PLATE, COMP, COMP));
    let (src, out) = (read(plate), output());
    let edges = vec![wire(src.id(), "output", out.id(), "input")];
    let mut comp = graph_comp("graph", vec![src, out], edges);
    comp.background = LinearColour([0.0; 4]);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

/// The middle pixel in scene-linear floats, premultiplied as the composite is.
fn linear_at(r: &mut HeadlessRenderer, (doc, comp): (Arc<Document>, Uuid)) -> [f32; 4] {
    let (px, w, _) = r
        .render_preview_linear(&doc, comp, 0, lumit_render::Quality::default())
        .unwrap();
    let d = ((32 * w + 32) * 4) as usize;
    [px[d], px[d + 1], px[d + 2], px[d + 3]]
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

/// **Split then Combine is the identity** on a pixel with any alpha, and a
/// Combine with its Alpha unwired is opaque.
#[test]
fn a_split_into_a_combine_gives_the_picture_back() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let plate = linear_at(&mut r, plate_alone());
    let combined = |alpha: bool| {
        split_graph(true, |split, out, nodes, edges| {
            let combine = GraphNode::Fx(effect("combine_channels", &[]));
            let combine_id = combine.id();
            nodes.push(combine);
            for (from, to) in lumit_core::comp_graph::SPLIT_OUTPUTS
                .into_iter()
                .zip(lumit_core::comp_graph::COMBINE_INPUTS)
            {
                if alpha || to != "alpha" {
                    edges.push(wire(split, from, combine_id, to));
                }
            }
            edges.push(wire(combine_id, "output", out, "input"));
        })
    };

    let whole = linear_at(&mut r, combined(true));
    assert!(
        near(whole, plate),
        "all four back in is the plate: {whole:?} against {plate:?}"
    );

    let opaque = linear_at(&mut r, combined(false));
    let straight = [PLATE[0], PLATE[1], PLATE[2], 1.0];
    assert!(
        near(opaque, straight),
        "an unwired Alpha is full on: {opaque:?} against {straight:?}"
    );
}

/// The plate read into a Matte key with its View dropdown on `dropdown`, and
/// `rest` wiring the key's sockets on to the Output.
fn keyed_graph(
    dropdown: u32,
    rest: impl FnOnce(Uuid, Uuid, &mut Vec<GraphNode>, &mut Vec<GraphEdge>),
) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let plate = Uuid::now_v7();
    doc.items.push(solid(plate, "plate", PLATE, COMP, COMP));
    let mut key = effect("matte_key", &[]);
    set_value(&mut key, "view", EffectValue::Choice(dropdown));
    let (src, key, out) = (read(plate), GraphNode::Fx(key), output());
    let (key_id, out_id) = (key.id(), out.id());
    let mut edges = vec![wire(src.id(), "output", key_id, "input")];
    let mut nodes = vec![src, key, out];
    rest(key_id, out_id, &mut nodes, &mut edges);
    let mut comp = graph_comp("graph", nodes, edges);
    comp.background = LinearColour([0.0; 4]);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

/// **A view socket is the effect with its dropdown on that option.** The
/// Screen matte socket draws what `output` draws with the dropdown on Screen
/// matte, `output` goes on following the dropdown, and both can be wired at
/// once.
#[test]
fn a_view_socket_draws_the_view_it_is_named_for() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    const FINAL: u32 = 0;
    const SCREEN_MATTE: u32 = 1;
    let from = |dropdown: u32, port: &'static str| {
        keyed_graph(dropdown, move |key, out, _, edges| {
            edges.push(wire(key, port, out, "input"));
        })
    };
    let keyed = linear_at(&mut r, from(FINAL, "output"));
    let matte = linear_at(&mut r, from(SCREEN_MATTE, "output"));
    assert!(
        !near(keyed, matte),
        "the two views are different pictures: {keyed:?} and {matte:?}"
    );

    let socket = linear_at(&mut r, from(FINAL, "view_1"));
    assert!(
        near(socket, matte),
        "the Screen matte socket is the Screen matte view: {socket:?} against {matte:?}"
    );

    // Both wired at once, into a Switch that shows one or the other.
    let both = |index: f64| {
        keyed_graph(FINAL, move |key, out, nodes, edges| {
            let switch = GraphNode::Fx(effect("switch", &[("index", index)]));
            let switch_id = switch.id();
            nodes.push(switch);
            edges.push(wire(key, "output", switch_id, "in0"));
            edges.push(wire(key, "view_1", switch_id, "in1"));
            edges.push(wire(switch_id, "output", out, "input"));
        })
    };
    let first = linear_at(&mut r, both(0.0));
    let second = linear_at(&mut r, both(1.0));
    assert!(near(first, keyed), "{first:?} against {keyed:?}");
    assert!(near(second, matte), "{second:?} against {matte:?}");
}

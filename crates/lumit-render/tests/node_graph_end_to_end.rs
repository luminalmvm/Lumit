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

/// **Test 9a - a Read box is the layer it behaves like.** A solid wired to the
/// Output is that solid, and moving the wire to the other Read shows the other
/// solid.
#[test]
fn a_read_wired_to_the_output_renders_its_item() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let build = |blue_wins: bool| {
        let mut doc = Document::new();
        let (red, blue) = (Uuid::now_v7(), Uuid::now_v7());
        doc.items
            .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
        doc.items
            .push(solid(blue, "blue", [0.0, 0.0, 1.0, 1.0], COMP, COMP));
        let (a, b, out) = (read(red), read(blue), output());
        let shown = if blue_wins { b.id() } else { a.id() };
        let comp = graph_comp(
            "graph",
            vec![a, b, out.clone()],
            vec![wire(shown, "output", out.id(), "input")],
        );
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };

    let (doc, comp) = build(false);
    let (px, w, _) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
    assert!(
        rgb(&px, w, 32, 32)[0] > 200 && rgb(&px, w, 32, 32)[2] < 60,
        "the Read wired to the Output is the picture: {:?}",
        rgb(&px, w, 32, 32)
    );

    let (doc, comp) = build(true);
    let (px, w, _) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
    assert!(
        rgb(&px, w, 32, 32)[2] > 200 && rgb(&px, w, 32, 32)[0] < 60,
        "moving the Output's wire shows the other box: {:?}",
        rgb(&px, w, 32, 32)
    );
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

/// **Test 9c - Merge's own arithmetic.** An unwired A gives B, an unwired B
/// gives A, and Opacity 50 lays half of A on.
#[test]
fn merge_reads_an_unwired_socket_as_nothing_at_all() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // `a`/`b` say which sockets are wired; `opacity` is A's own row.
    let build = |a: bool, b: bool, opacity: f64| {
        let mut doc = Document::new();
        let (red, blue) = (Uuid::now_v7(), Uuid::now_v7());
        doc.items
            .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
        doc.items
            .push(solid(blue, "blue", [0.0, 0.0, 1.0, 1.0], COMP, COMP));
        let (ra, rb, merge, out) = (
            read(red),
            read(blue),
            GraphNode::Fx(effect("merge", &[("opacity", opacity)])),
            output(),
        );
        let (ra_id, rb_id, merge_id, out_id) = (ra.id(), rb.id(), merge.id(), out.id());
        let mut edges = vec![wire(merge_id, "output", out_id, "input")];
        if a {
            edges.push(wire(ra_id, "output", merge_id, "input"));
        }
        if b {
            edges.push(wire(rb_id, "output", merge_id, "background"));
        }
        let comp = graph_comp("graph", vec![ra, rb, merge, out], edges);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    let pixel = |r: &mut HeadlessRenderer, a, b, opacity| {
        let (doc, comp) = build(a, b, opacity);
        let (px, w, _) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
        rgb(&px, w, 32, 32)
    };

    let only_b = pixel(&mut r, false, true, 100.0);
    assert!(
        only_b[2] > 200 && only_b[0] < 60,
        "unwired A gives B: {only_b:?}"
    );
    let only_a = pixel(&mut r, true, false, 100.0);
    assert!(
        only_a[0] > 200 && only_a[2] < 60,
        "unwired B gives A: {only_a:?}"
    );
    let half_a = pixel(&mut r, true, true, 50.0);
    assert!(
        half_a[0] > 30 && half_a[2] > 30,
        "Opacity 50 lays half of A over B: {half_a:?}"
    );
    let all_a = pixel(&mut r, true, true, 100.0);
    assert!(
        all_a[0] > half_a[0] && all_a[2] < half_a[2],
        "and Opacity 100 covers B entirely: {all_a:?} against {half_a:?}"
    );
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
        let wiggle = effect("wiggle", &[("amount", 3.0), ("frequency", 4.0)]);
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

/// **Test 11 - the picture at a box** (§4.5). The patched copy renders the
/// picked box's own output rather than the Output's, and names its own frame.
#[test]
fn the_picture_at_a_box_is_its_own_frame() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut doc = Document::new();
    let grey = Uuid::now_v7();
    doc.items
        .push(solid(grey, "grey", [0.3, 0.3, 0.3, 1.0], COMP, COMP));
    let (src, fx, o) = (
        read(grey),
        GraphNode::Fx(effect("exposure", &[("stops", 2.0)])),
        output(),
    );
    let (src_id, fx_id, o_id) = (src.id(), fx.id(), o.id());
    let comp = graph_comp(
        "graph",
        vec![src, fx, o],
        vec![
            wire(src_id, "output", fx_id, "input"),
            wire(fx_id, "output", o_id, "input"),
        ],
    );
    let comp_id = comp.id;
    let graph = comp.graph.clone().unwrap();
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);

    // The Output already shows the Exposure, so there is nothing to cut to.
    assert!(
        graph.viewed_at(fx_id).is_none(),
        "the box already feeding the Output offers no cut"
    );
    assert!(
        graph.viewed_at(o_id).is_none(),
        "and the Output itself offers none"
    );

    let mut patched_doc = (*doc).clone();
    let patched = graph.viewed_at(src_id).expect("the Read offers a cut");
    if let Some(c) = patched_doc.comp_mut(comp_id) {
        c.graph = Some(patched);
    }
    let patched_doc = Arc::new(patched_doc);

    let (whole, w, _) = r.render_rgba(&doc, comp_id, 0, 1.0).unwrap();
    let (at_box, ..) = r.render_rgba(&patched_doc, comp_id, 0, 1.0).unwrap();
    assert_ne!(
        rgb(&whole, w, 32, 32),
        rgb(&at_box, w, 32, 32),
        "the picked box's output is a different picture"
    );
    assert_ne!(
        key_of(&doc, comp_id),
        key_of(&patched_doc, comp_id),
        "and it names its own frame by construction"
    );
}

/// **The bypass tick reaches the graph's own boxes** (§1.2). Merge and Switch
/// have no op for the stack resolver to leave out, so each hands on its main
/// picture itself when it is switched off - and the frame's name moves with the
/// picture.
#[test]
fn a_bypassed_merge_or_switch_hands_on_the_picture_it_was_given() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // Red into the box's main picture socket, blue into its second, each box
    // set so that switched on it shows the blue.
    let build = |switch: bool, enabled: bool| {
        let mut doc = Document::new();
        let (red, blue) = (Uuid::now_v7(), Uuid::now_v7());
        doc.items
            .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
        doc.items
            .push(solid(blue, "blue", [0.0, 0.0, 1.0, 1.0], COMP, COMP));
        let (main, second) = if switch {
            ("in0", "in1")
        } else {
            ("input", "background")
        };
        let mut inst = if switch {
            effect("switch", &[("index", 1.0)])
        } else {
            // A Merge laying nothing of A on is B alone.
            effect("merge", &[("opacity", 0.0)])
        };
        inst.enabled = enabled;
        let (ra, rb, node, out) = (read(red), read(blue), GraphNode::Fx(inst), output());
        let (ra_id, rb_id, node_id, out_id) = (ra.id(), rb.id(), node.id(), out.id());
        let comp = graph_comp(
            "graph",
            vec![ra, rb, node, out],
            vec![
                wire(ra_id, "output", node_id, main),
                wire(rb_id, "output", node_id, second),
                wire(node_id, "output", out_id, "input"),
            ],
        );
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    for switch in [false, true] {
        let (doc, comp) = build(switch, true);
        let (px, w, _) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
        let on = rgb(&px, w, 32, 32);
        assert!(
            on[2] > 200 && on[0] < 60,
            "switched on the box shows its second picture: {on:?}"
        );
        let on_key = key_of(&doc, comp);

        let (doc, comp) = build(switch, false);
        let (px, w, _) = r.render_rgba(&doc, comp, 0, 1.0).unwrap();
        let off = rgb(&px, w, 32, 32);
        assert!(
            off[0] > 200 && off[2] < 60,
            "bypassed it hands on the picture it was given: {off:?}"
        );
        assert_ne!(
            on_key,
            key_of(&doc, comp),
            "and a bypassed box names its own frame"
        );
    }
}

/// **A Read box has no in and out points** (§2.1). Applied as an effect on a
/// longer comp, a graph is asked at times past its own duration, and its Read
/// boxes are its picture there as much as anywhere.
#[test]
fn a_read_box_draws_past_the_graph_comps_own_duration() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut doc = Document::new();
    let (grey, red) = (Uuid::now_v7(), Uuid::now_v7());
    doc.items
        .push(solid(grey, "grey", [0.3, 0.3, 0.3, 1.0], COMP, COMP));
    doc.items
        .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
    let (src, out) = (read(red), output());
    let (src_id, out_id) = (src.id(), out.id());
    let mut inner = graph_comp(
        "graph",
        vec![src, out],
        vec![wire(src_id, "output", out_id, "input")],
    );
    // Half a second long: every frame this test renders is past its end.
    inner.duration = Duration(Rational::new(1, 2).unwrap());
    let mut host = layer("host", LayerKind::Solid { def: grey });
    host.effects = vec![node_graph_effect(&inner)];
    doc.items.push(ProjectItem::Composition(inner));
    let comp = comp_of("parent", vec![host]);
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    let doc = Arc::new(doc);

    let (px, w, _) = r.render_rgba(&doc, comp_id, 120, 1.0).unwrap();
    let at_two_seconds = rgb(&px, w, 32, 32);
    assert!(
        at_two_seconds[0] > 200 && at_two_seconds[2] < 60,
        "the Read box draws whenever the graph is asked: {at_two_seconds:?}"
    );
}

/// **An Input under one of the effect's own ids bakes its default** (§1.5). The
/// Node graph effect declares Mix, Blend and the matte trio, so an Input that
/// collides with one of them derives no row of its own - and the effect's Mix
/// must not be handed to the graph in its place.
#[test]
fn an_input_named_like_one_of_the_effects_rows_bakes_its_own_default() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // A small white square inside the graph's own frame, so a blur radius is
    // plain to see; the Input is the radius.
    let build = |r: &mut HeadlessRenderer, id: &str, radius: f64| {
        let mut doc = Document::new();
        let (grey, square) = (Uuid::now_v7(), Uuid::now_v7());
        doc.items
            .push(solid(grey, "grey", [0.2, 0.2, 0.2, 1.0], COMP, COMP));
        doc.items
            .push(solid(square, "square", [1.0, 1.0, 1.0, 1.0], 24, 24));
        let (src, amount, blur, o) = (
            read(square),
            number_input(id, radius),
            GraphNode::Fx(effect("blur", &[])),
            output(),
        );
        let (src_id, amount_id, blur_id, o_id) = (src.id(), amount.id(), blur.id(), o.id());
        let inner = graph_comp(
            "blurred",
            vec![src, amount, blur, o],
            vec![
                wire(src_id, "output", blur_id, "input"),
                wire(amount_id, "value", blur_id, "radius"),
                wire(blur_id, "output", o_id, "input"),
            ],
        );
        let mut host = layer("host", LayerKind::Solid { def: grey });
        host.effects = vec![node_graph_effect(&inner)];
        doc.items.push(ProjectItem::Composition(inner));
        let comp = comp_of("parent", vec![host]);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        let (px, ..) = r
            .render_rgba(&Arc::new(doc), id, 0, 1.0)
            .expect("a solid comp renders");
        px
    };
    let under_mix = build(&mut r, "mix", 4.0);
    let under_amount = build(&mut r, "amount", 4.0);
    let wider = build(&mut r, "amount", 20.0);
    assert_eq!(
        under_mix, under_amount,
        "an Input called mix must render as the same Input under any other name"
    );
    assert_ne!(
        under_mix, wider,
        "and its own default is what the graph reads"
    );
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

/// **Test 21 - a picture Input's preview item** (§5.11). It stands in for the
/// picture where nothing feeds the Input, which is a graph viewed on its own or
/// placed as a Precomp layer, and nowhere else: applied to a layer the picture
/// arrives on the socket, and the stand-in must be neither drawn nor decoded.
#[test]
fn a_preview_item_shows_only_where_nothing_feeds_the_input() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // The graph is a picture Input straight into the Output, so what it shows
    // is exactly what feeds that Input and nothing else.
    let build = |preview: bool, applied: bool| {
        let mut doc = Document::new();
        let plate = Uuid::now_v7();
        let grey = Uuid::now_v7();
        doc.items
            .push(solid(plate, "plate", [0.9, 0.1, 0.1, 1.0], COMP, COMP));
        doc.items
            .push(solid(grey, "grey", [0.2, 0.2, 0.2, 1.0], COMP, COMP));
        let mut src = picture_input("src");
        if preview {
            if let GraphNode::Input { input, .. } = &mut src {
                input.preview = Some(plate);
            }
        }
        let out = output();
        let (src_id, o_id) = (src.id(), out.id());
        let inner = graph_comp(
            "graph",
            vec![src, out],
            vec![wire(src_id, "output", o_id, "input")],
        );
        let inner_id = inner.id;
        let host = if applied {
            let mut l = layer("host", LayerKind::Solid { def: grey });
            l.effects = vec![node_graph_effect(&inner)];
            l
        } else {
            layer("placed", LayerKind::Precomp { comp: inner_id })
        };
        doc.items.push(ProjectItem::Composition(inner));
        let comp = comp_of("parent", vec![host]);
        let id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    };
    let shot = |r: &mut HeadlessRenderer, (doc, comp): (Arc<Document>, Uuid)| {
        r.render_rgba(&doc, comp, 0, 1.0).expect("the render").0
    };

    // Placed as a Precomp layer: nothing feeds the Input, so the stand-in is
    // the picture.
    let bare = shot(&mut r, build(false, false));
    let shown = shot(&mut r, build(true, false));
    assert_ne!(bare, shown, "the preview item is what a viewed graph shows");
    assert_eq!(
        rgb(&shown, COMP, 32, 32),
        rgb(
            &shot(&mut r, {
                // The plate on its own, for the colour to compare against.
                let mut doc = Document::new();
                let plate = Uuid::now_v7();
                doc.items
                    .push(solid(plate, "plate", [0.9, 0.1, 0.1, 1.0], COMP, COMP));
                let comp = comp_of("plate", vec![layer("p", LayerKind::Solid { def: plate })]);
                let id = comp.id;
                doc.items.push(ProjectItem::Composition(comp));
                (Arc::new(doc), id)
            }),
            COMP,
            32,
            32
        ),
        "and it is the item itself, at default placement"
    );

    // Applied as an effect: the host's own picture feeds the Input, so the
    // stand-in changes no pixel.
    assert_eq!(
        shot(&mut r, build(false, true)),
        shot(&mut r, build(true, true)),
        "an applied graph never shows a preview item"
    );
}

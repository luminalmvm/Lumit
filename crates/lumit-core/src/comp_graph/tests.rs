#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::fx::builtins::instantiate;
use crate::graph::GraphError;
use crate::model::{
    FootageItem, LayerKind, LinearColour, MediaRef, MotionBlur, ProjectItem, SolidDef,
};
use crate::time::{Duration, FrameRate, Rational};

fn fx(match_name: &str) -> GraphNode {
    GraphNode::Fx(instantiate(match_name).expect("the catalogue knows it"))
}

/// An Fx box with one of its numbers typed in.
fn fx_with(match_name: &str, param: &str, value: f64) -> GraphNode {
    let mut inst = instantiate(match_name).expect("the catalogue knows it");
    set_param(
        &mut inst,
        param,
        EffectValue::Float(crate::anim::Property::fixed(value)),
    );
    GraphNode::Fx(inst)
}

fn read(item: Uuid) -> GraphNode {
    GraphNode::Read {
        id: Uuid::now_v7(),
        item,
        custom_name: None,
    }
}

fn input(id: &str, kind: InputKind, default: f64) -> GraphNode {
    GraphNode::Input {
        id: Uuid::now_v7(),
        input: GraphInput {
            id: id.to_owned(),
            label: "Amount".to_owned(),
            kind,
            default: [default, 0.0, 0.0, 1.0],
            min: 0.0,
            max: 100.0,
            unit: Unit::Raw,
            preview: None,
        },
    }
}

fn wire(from: &GraphNode, from_port: &str, to: &GraphNode, to_port: &str) -> GraphEdge {
    GraphEdge {
        from: from.id(),
        from_port: from_port.to_owned(),
        to: to.id(),
        to_port: to_port.to_owned(),
    }
}

fn built(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> CompGraph {
    CompGraph {
        notes: Vec::new(),
        ties: Vec::new(),
        looks: Default::default(),
        nodes,
        edges,
        layout: Vec::new(),
        widths: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    }
}

/// An old Depth of field pro box's depth wires land on its one Depth
/// socket, metres winning over a pass as they did, and a box already moved
/// on keeps a mask wired to its matte.
#[test]
fn dof_pro_depth_wires_merge_on_open() {
    let (pass, metres) = (fx("depth"), fx("depth"));
    let mut old = instantiate("dof_pro").unwrap();
    old.effect.version = 1;
    let (old, new) = (GraphNode::Fx(old), fx("dof_pro"));
    let mut g = built(
        vec![pass.clone(), metres.clone(), old.clone(), new.clone()],
        vec![
            wire(&pass, OUTPUT_PORT.id, &old, MATTE_PORT.id),
            wire(&metres, OUTPUT_PORT.id, &old, "depth_m"),
            wire(&pass, OUTPUT_PORT.id, &new, MATTE_PORT.id),
        ],
    );
    g.migrate_dof_pro_depth();
    assert_eq!(
        g.wire_into(old.id(), "depth").map(|w| *w.0),
        Some(metres.id())
    );
    assert_eq!(g.wire_into(old.id(), MATTE_PORT.id), None);
    assert_eq!(
        g.wire_into(new.id(), MATTE_PORT.id).map(|w| *w.0),
        Some(pass.id())
    );
}

/// The shape every other test bends: one Read through a blur into the
/// Output.
fn read_blur_output() -> (CompGraph, Uuid, Uuid, Uuid) {
    let item = Uuid::now_v7();
    let source = read(item);
    let blur = fx("blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let (source_id, blur_id, out_id) = (source.id(), blur.id(), out.id());
    let edges = vec![
        wire(&source, OUTPUT_PORT.id, &blur, INPUT_PORT.id),
        wire(&blur, OUTPUT_PORT.id, &out, INPUT_PORT.id),
    ];
    (
        built(vec![source, blur, out], edges),
        source_id,
        blur_id,
        out_id,
    )
}

fn comp() -> Composition {
    Composition {
        id: Uuid::now_v7(),
        name: "Node graph 1".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(25, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        groups: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        beat_grid: None,
        graph: None,
        extra: serde_json::Map::new(),
    }
}

fn footage(name: &str) -> FootageItem {
    FootageItem {
        id: Uuid::now_v7(),
        name: name.to_owned(),
        media: MediaRef {
            relative_path: format!("{name}.mov"),
            absolute_path: String::new(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: None,
        sequence: None,
        extra: serde_json::Map::new(),
        source_layer: None,
        interpretation: Default::default(),
    }
}

// -- 1.4, the rules ------------------------------------------------------

/// Types must match, with **one exception**: a matte socket takes a
/// picture as well as a matte, because what the row means by matte is the
/// picture's own channel.
#[test]
fn a_mistyped_wire_is_refused_and_a_picture_into_a_matte_is_not() {
    let (mut graph, source_id, blur_id, _) = read_blur_output();
    graph.edges.push(GraphEdge {
        from: source_id,
        from_port: OUTPUT_PORT.id.to_owned(),
        to: blur_id,
        to_port: "radius".to_owned(),
    });
    assert_eq!(graph.validate(None), Err(GraphError::PortTypeMismatch));

    let (mut graph, source_id, blur_id, _) = read_blur_output();
    graph.edges.push(GraphEdge {
        from: source_id,
        from_port: OUTPUT_PORT.id.to_owned(),
        to: blur_id,
        to_port: MATTE_PORT.id.to_owned(),
    });
    graph
        .validate(None)
        .expect("a picture is what a matte socket is reached for");
}

#[test]
fn a_loop_through_image_wires_is_refused() {
    let first = fx("blur");
    let second = fx("blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let edges = vec![
        wire(&first, OUTPUT_PORT.id, &second, INPUT_PORT.id),
        wire(&second, OUTPUT_PORT.id, &first, INPUT_PORT.id),
        wire(&second, OUTPUT_PORT.id, &out, INPUT_PORT.id),
    ];
    let graph = built(vec![first, second, out], edges);
    assert_eq!(graph.validate(None), Err(GraphError::Cycle));

    // The smallest loop of all: a box wired into itself.
    let alone = fx("blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let edges = vec![wire(&alone, OUTPUT_PORT.id, &alone, INPUT_PORT.id)];
    let graph = built(vec![alone, out], edges);
    assert_eq!(graph.validate(None), Err(GraphError::Cycle));
}

// -- 1.4, the sockets ----------------------------------------------------

fn ids(ports: &[GraphPort]) -> Vec<&str> {
    ports.iter().map(|p| p.id.as_str()).collect()
}

/// An image effect with a matte row and a layer row: its picture, its
/// matte, the layer row as an **image socket**, then its numbers.
#[test]
fn an_image_effects_sockets_are_its_picture_its_matte_and_its_rows() {
    let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());
    let wrap = fx("light_wrap");
    let (ins, outs) = ports_of(&graph, &wrap, None);
    assert_eq!(
        ids(&ins),
        vec!["input", "matte", "background", "width", "intensity", "mix"]
    );
    assert_eq!(
        ins[2].ty,
        PortType::Image,
        "a layer reference is a wire here, not a dropdown"
    );
    assert_eq!(ins[3].ty, PortType::Number);
    assert_eq!(ids(&outs), vec!["output"]);
}

/// A view menu gives the box a socket per option beside `output`, and a
/// graph that only ever wired `output` is still a sound one.
#[test]
fn a_view_menu_gives_the_box_a_socket_per_option() {
    let (source, dof, out) = (
        read(Uuid::now_v7()),
        fx("dof"),
        GraphNode::Output { id: Uuid::now_v7() },
    );
    let graph = |from: &str| {
        built(
            vec![source.clone(), dof.clone(), out.clone()],
            vec![
                wire(&source, OUTPUT_PORT.id, &dof, INPUT_PORT.id),
                wire(&dof, from, &out, INPUT_PORT.id),
            ],
        )
    };
    let (_, outs) = ports_of(&graph("output"), &dof, None);
    assert_eq!(
        ids(&outs),
        vec!["output", "display_0", "display_1", "display_2"]
    );
    assert_eq!(outs[3].label, "Focus map");
    assert_eq!(outs[3].ty, PortType::Image);

    assert_eq!(graph("output").validate(None), Ok(()));
    assert_eq!(graph("display_2").validate(None), Ok(()));
    assert_eq!(
        graph("display_3").validate(None),
        Err(GraphError::UnknownPort),
        "the menu has three options"
    );

    let GraphNode::Fx(inst) = &dof else {
        unreachable!()
    };
    let held = held_on_view(inst, "display_2").expect("a view socket");
    let display = held.params.iter().find(|p| p.id == "display");
    assert_eq!(display.map(|p| &p.value), Some(&EffectValue::Choice(2)));
    assert!(held_on_view(inst, "output").is_none());
    // An effect with no view menu has the one picture output it always had.
    let (_, plain) = ports_of(&graph("output"), &fx("blur"), None);
    assert_eq!(ids(&plain), vec!["output"]);
}

/// A Switch always draws **one spare socket** beyond the last one wired,
/// which is what makes it grow by being used.
#[test]
fn a_switch_shows_one_spare_socket_beyond_the_last_wired() {
    let switch = fx("switch");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let first = fx("blur");
    let second = fx("blur");

    let bare = built(vec![switch.clone(), out.clone()], Vec::new());
    let (ins, _) = ports_of(&bare, &switch, None);
    assert_eq!(ids(&ins), vec!["in0", "index"]);
    assert_eq!(ins[0].label, "", "the canvas draws the index itself");

    let edges = vec![
        wire(&first, OUTPUT_PORT.id, &switch, "in0"),
        wire(&second, OUTPUT_PORT.id, &switch, "in1"),
    ];
    let wired = built(vec![first, second, switch.clone(), out], edges);
    let (ins, _) = ports_of(&wired, &switch, None);
    assert_eq!(ids(&ins), vec!["in0", "in1", "in2", "index"]);
}

/// A Node graph box's sockets are the Inputs of the comp it names: the
/// first picture is its own `input`, then its Matte, then every other
/// Input a socket of its own.
#[test]
fn a_node_graph_box_shows_the_inputs_of_the_comp_it_names() {
    let inner = built(
        vec![
            input("plate", InputKind::Picture, 0.0),
            input("second", InputKind::Picture, 0.0),
            input("amount", InputKind::Number, 4.0),
            GraphNode::Output { id: Uuid::now_v7() },
        ],
        Vec::new(),
    );
    let mut inner_comp = comp();
    let inner_id = inner_comp.id;
    inner_comp.graph = Some(inner.clone());
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(inner_comp));

    let mut instance = instantiate("node_graph").expect("the catalogue knows it");
    crate::fx::effects::node_graph::bind(&mut instance, inner_id, &inner);
    let node = GraphNode::Fx(instance);
    let outer = built(
        vec![node.clone(), GraphNode::Output { id: Uuid::now_v7() }],
        Vec::new(),
    );

    let (ins, outs) = ports_of(&outer, &node, Some(&doc));
    assert_eq!(ids(&ins), vec!["input", "matte", "second", "amount"]);
    assert_eq!(ins[2].ty, PortType::Image);
    assert_eq!(ins[3].ty, PortType::Number);
    assert_eq!(ids(&outs), vec!["output"]);

    // A picture wired into its Matte is taken.
    let source = read(Uuid::now_v7());
    let matted = built(
        vec![
            source.clone(),
            node.clone(),
            GraphNode::Output { id: Uuid::now_v7() },
        ],
        vec![wire(&source, OUTPUT_PORT.id, &node, MATTE_PORT.id)],
    );
    matted
        .validate(Some(&doc))
        .expect("the matte socket takes a picture");
}

// -- 2.2, the projection -------------------------------------------------

/// A driver into an Fx parameter comes out as the very edge the layer's
/// driver walk already resolves.
#[test]
fn a_driver_wire_projects_onto_the_layer_graph() {
    let wiggle = fx("wiggle");
    let blur = fx("blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let (wiggle_id, blur_id) = (wiggle.id(), blur.id());
    let edges = vec![wire(&wiggle, "value", &blur, "radius")];
    let graph = built(vec![wiggle, blur, out], edges);

    let projected = graph.project(&[]);
    assert_eq!(
        projected.effects.len(),
        1,
        "the blur is the only picture op"
    );
    assert_eq!(projected.effects[0].id, blur_id);
    assert_eq!(projected.drivers.nodes.len(), 1);
    assert_eq!(projected.drivers.nodes[0].id, wiggle_id);
    assert_eq!(
        projected.drivers.edges,
        vec![crate::graph::Edge {
            from: crate::graph::OutputRef::Driver {
                node: wiggle_id,
                port: "value".into(),
            },
            to: crate::graph::InputRef::Param {
                node: crate::graph::NodeRef::Effect(blur_id),
                port: "radius".into(),
            },
        }]
    );
}

#[test]
fn an_input_bakes_its_default_and_the_override_when_one_is_given() {
    let amount = input("amount", InputKind::Number, 7.0);
    let blur = fx("blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let edges = vec![wire(&amount, "value", &blur, "radius")];
    let graph = built(vec![amount, blur, out], edges);

    let radius = |p: &Projection| match p.effects[0].param("radius") {
        Some(EffectValue::Float(v)) => v.value_at(0.0),
        other => panic!("radius is a number, not {other:?}"),
    };
    assert_eq!(radius(&graph.project(&[])), 7.0, "nobody overrode it");

    let over = vec![(
        "amount".to_owned(),
        EffectValue::Float(crate::anim::Property::fixed(3.0)),
    )];
    assert_eq!(radius(&graph.project(&over)), 3.0);
}

// -- 5.2 and 5.4, the cone and the times ---------------------------------

/// The cone is the boxes that make the picture, in an order where a box's
/// sources are always earlier than the box.
#[test]
fn the_cone_is_topological_and_leaves_an_idle_branch_out() {
    let read = read(Uuid::now_v7());
    let blur = fx("blur");
    let glow = fx("glow");
    let idle = fx("invert");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let (read_id, blur_id, glow_id, out_id) = (read.id(), blur.id(), glow.id(), out.id());
    let edges = vec![
        wire(&read, OUTPUT_PORT.id, &blur, INPUT_PORT.id),
        wire(&blur, OUTPUT_PORT.id, &glow, INPUT_PORT.id),
        wire(&glow, OUTPUT_PORT.id, &out, INPUT_PORT.id),
    ];
    let graph = built(vec![glow, idle, blur, read, out], edges);

    assert_eq!(
        graph.cone_of(out_id),
        vec![read_id, blur_id, glow_id, out_id],
        "sources first, and the idle branch is not in it"
    );
    assert_eq!(graph.cone_of(blur_id), vec![read_id, blur_id]);

    // A Switch on its second socket leaves the first socket's branch out,
    // unless something else in the cone reads that branch too.
    let (a, b) = (self::read(Uuid::now_v7()), self::read(Uuid::now_v7()));
    let heavy = fx("blur");
    let switch = fx(SWITCH);
    let merge = fx(MERGE);
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let (a_id, b_id, heavy_id, switch_id) = (a.id(), b.id(), heavy.id(), switch.id());
    let mut edges = vec![
        wire(&a, OUTPUT_PORT.id, &heavy, INPUT_PORT.id),
        wire(&heavy, OUTPUT_PORT.id, &switch, "in0"),
        wire(&b, OUTPUT_PORT.id, &switch, "in1"),
        wire(&switch, OUTPUT_PORT.id, &merge, INPUT_PORT.id),
        wire(&merge, OUTPUT_PORT.id, &out, INPUT_PORT.id),
    ];
    let shared = wire(&heavy, OUTPUT_PORT.id, &merge, BACKGROUND_PORT.id);
    let nodes = vec![a, heavy, b, switch, merge, out];
    let out_id = nodes[5].id();
    let alone = built(nodes.clone(), edges.clone());
    let cone = alone.cone_picked(out_id, &[(switch_id, 1)]);
    assert!(cone.contains(&b_id) && !cone.contains(&heavy_id) && !cone.contains(&a_id));
    let cone = alone.cone_picked(out_id, &[(switch_id, 0)]);
    assert!(cone.contains(&heavy_id) && !cone.contains(&b_id));
    edges.push(shared);
    let cone = built(nodes, edges).cone_picked(out_id, &[(switch_id, 1)]);
    assert!(cone.contains(&heavy_id) && cone.contains(&b_id), "{cone:?}");
}

/// A Time offset before a temporal box shifts every neighbour that box asks
/// for, and a temporal box inside a neighbour holds a still (§5.2).
#[test]
fn a_shift_reaches_every_neighbour_and_a_window_never_nests() {
    let read = read(Uuid::now_v7());
    let inner = fx("datamosh");
    let shift = fx_with("time_offset", "offset", 1.0);
    let outer = fx("motion_blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let (read_id, inner_id, out_id) = (read.id(), inner.id(), out.id());
    let edges = vec![
        wire(&read, OUTPUT_PORT.id, &inner, INPUT_PORT.id),
        wire(&inner, OUTPUT_PORT.id, &shift, INPUT_PORT.id),
        wire(&shift, OUTPUT_PORT.id, &outer, INPUT_PORT.id),
        wire(&outer, OUTPUT_PORT.id, &out, INPUT_PORT.id),
    ];
    let graph = built(vec![read, inner, shift, outer, out], edges);

    let demands = graph.time_demands(out_id, 4.0, &|inst, at| {
        crate::fx::temporal::input_times(inst, at, 0.5)
    });
    let times = |node: Uuid| -> Vec<f64> {
        let mut ts: Vec<f64> = demands
            .iter()
            .filter(|(n, _)| *n == node)
            .map(|(_, at)| *at)
            .collect();
        ts.sort_by(f64::total_cmp);
        ts
    };
    // Motion blur asks its input at 4 and at the frame after it; the shift
    // moves both of them on by its second.
    assert_eq!(times(inner_id), vec![5.0, 5.5]);
    // Datamosh is only ever asked at a time other than the graph's own, so
    // it holds a still: no window opens inside a window.
    assert_eq!(times(read_id), vec![5.0, 5.5]);
}

// -- 2.1, a Read box is a layer ------------------------------------------

#[test]
fn a_read_box_is_a_layer_at_default_placement() {
    let host = comp();
    let solid = SolidDef {
        id: Uuid::now_v7(),
        name: "White solid 1".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: 640,
        height: 480,
        extra: serde_json::Map::new(),
    };
    let node_id = Uuid::now_v7();
    let layer =
        read_layer(node_id, &ProjectItem::Solid(solid.clone()), &host).expect("a solid is a layer");
    assert_eq!(layer.id, node_id, "the box names the layer");
    assert_eq!(layer.name, "White solid 1");
    assert_eq!(layer.kind, LayerKind::Solid { def: solid.id });
    assert_eq!(layer.in_point.0, Rational::ZERO);
    assert_eq!(layer.out_point.0, host.duration.0);
    assert_eq!(layer.start_offset.0, Rational::ZERO);
    // Centred at its natural size: the anchor is the item's middle, the
    // position the comp's.
    assert_eq!(layer.transform.anchor_x.value_at(0.0), 320.0);
    assert_eq!(layer.transform.anchor_y.value_at(0.0), 240.0);
    assert_eq!(layer.transform.position_x.value_at(0.0), 960.0);
    assert_eq!(layer.transform.position_y.value_at(0.0), 540.0);

    // Footage has no size in the document, so it is centred at comp size -
    // what a fresh footage layer of unprobed media already gets.
    let item = footage("plate");
    let layer = read_layer(Uuid::now_v7(), &ProjectItem::Footage(item.clone()), &host)
        .expect("footage is a layer");
    assert_eq!(layer.kind, LayerKind::Footage { item: item.id });
    assert_eq!(layer.transform.anchor_x.value_at(0.0), 960.0);

    let nested = comp();
    let layer = read_layer(
        Uuid::now_v7(),
        &ProjectItem::Composition(nested.clone()),
        &host,
    )
    .expect("a comp is a layer");
    assert_eq!(layer.kind, LayerKind::Precomp { comp: nested.id });

    let folder = crate::model::Folder {
        id: Uuid::now_v7(),
        name: "Stills".into(),
        children: Vec::new(),
        extra: serde_json::Map::new(),
    };
    assert!(
        read_layer(Uuid::now_v7(), &ProjectItem::Folder(folder), &host).is_none(),
        "a folder is the one item no layer can hold"
    );
}

/// The export's footage list follows a **Node graph effect** wherever it
/// sits: on a layer's stack, or as a box inside another graph. Without
/// that the renderer never probes the file and the Read box draws
/// transparent until somebody opens the graph.
#[test]
fn the_footage_walk_follows_a_node_graph_effect() {
    let plate = footage("plate");
    let mut graph_comp = comp();
    graph_comp.graph = Some(built(
        vec![read(plate.id), GraphNode::Output { id: Uuid::now_v7() }],
        Vec::new(),
    ));
    let graph_id = graph_comp.id;
    let inner = graph_comp.graph.clone().expect("the graph just set");

    let mut doc = Document::new();
    doc.items.push(ProjectItem::Footage(plate.clone()));
    doc.items.push(ProjectItem::Composition(graph_comp));

    let bound = |comp_id: Uuid| {
        let mut inst = instantiate("node_graph").expect("the catalogue knows it");
        crate::fx::effects::node_graph::bind(&mut inst, comp_id, &inner);
        inst
    };

    // On a layer of a comp with no source of its own.
    let mut host = comp();
    let solid = SolidDef {
        id: Uuid::now_v7(),
        name: "grey".to_owned(),
        colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
        width: 32,
        height: 32,
        extra: serde_json::Map::new(),
    };
    let mut host_layer =
        read_layer(Uuid::now_v7(), &ProjectItem::Solid(solid), &host).expect("a solid is a layer");
    host_layer.effects = vec![bound(graph_id)];
    host.layers.push(host_layer);
    assert_eq!(
        crate::model::comp_footage_items(&doc, &host),
        vec![plate.id],
        "a graph applied as an effect brings its own footage"
    );

    // The same box, inside another graph.
    let mut outer = comp();
    outer.graph = Some(built(
        vec![
            GraphNode::Fx(bound(graph_id)),
            GraphNode::Output { id: Uuid::now_v7() },
        ],
        Vec::new(),
    ));
    assert_eq!(
        crate::model::comp_footage_items(&doc, &outer),
        vec![plate.id],
        "and so does a nested Node graph box"
    );

    // A graph that applies itself stops at the guard.
    let mut looped = comp();
    let looped_id = looped.id;
    looped.graph = Some(built(
        vec![
            read(plate.id),
            GraphNode::Fx(bound(looped_id)),
            GraphNode::Output { id: Uuid::now_v7() },
        ],
        Vec::new(),
    ));
    doc.items.push(ProjectItem::Composition(looped.clone()));
    assert_eq!(
        crate::model::comp_footage_items(&doc, &looped),
        vec![plate.id],
        "a graph that reads itself terminates"
    );
}

// -- the copies ----------------------------------------------------------

#[test]
fn viewing_at_a_box_rewires_the_output_to_it() {
    let (graph, source_id, blur_id, out_id) = read_blur_output();

    let at_source = graph.viewed_at(source_id).expect("a Read makes a picture");
    assert_eq!(
        at_source.wire_into(out_id, INPUT_PORT.id),
        Some((&source_id, OUTPUT_PORT.id))
    );
    at_source.validate(None).expect("still a sound graph");
    assert_eq!(
        at_source.edges.len(),
        graph.edges.len(),
        "the Output's old wire went with the new one"
    );

    assert!(
        graph.viewed_at(blur_id).is_none(),
        "the blur already feeds the Output"
    );
    assert!(
        graph.viewed_at(out_id).is_none(),
        "the Output is the picture the Viewer already shows"
    );

    let mut with_driver = graph.clone();
    let wiggle = fx("wiggle");
    let wiggle_id = wiggle.id();
    with_driver.nodes.push(wiggle);
    assert!(
        with_driver.viewed_at(wiggle_id).is_none(),
        "a driver makes a number, not a picture"
    );
}

// -- the file ------------------------------------------------------------

#[test]
fn a_whole_graph_round_trips_through_json() {
    let (mut graph, source_id, blur_id, _) = read_blur_output();
    graph.nodes.push(input("amount", InputKind::Number, 7.0));
    graph.nodes.push(fx("wiggle"));
    graph.nodes.push(fx("merge"));
    graph.layout = vec![(source_id, [10.5, -20.25])];
    graph.exposed = vec![blur_id];
    graph.groups = vec![GraphGroup {
        name: "The plate".into(),
        colour: 3,
        members: vec![source_id],
    }];

    let json = serde_json::to_string(&graph).expect("it serialises");
    let back: CompGraph = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(back, graph);
}

/// **A Repeat pair holds the boxes between it, and nothing leaves them
/// but through the end.** A nested pair is inside, a wire from inside to
/// the Output is refused, and an end with no start of its own is refused.
#[test]
fn a_repeat_pair_holds_the_boxes_between_it() {
    let src = read(Uuid::now_v7());
    let (start, end) = (fx(REPEAT_START), fx(REPEAT_END));
    let (inner_start, inner_end) = (fx(REPEAT_START), fx(REPEAT_END));
    let blur = fx("blur");
    let out = GraphNode::Output { id: Uuid::now_v7() };
    let nodes = vec![
        src.clone(),
        start.clone(),
        inner_start.clone(),
        blur.clone(),
        inner_end.clone(),
        end.clone(),
        out.clone(),
    ];
    let mut edges = vec![
        wire(&src, "output", &start, "input"),
        wire(&start, "output", &inner_start, "input"),
        wire(&inner_start, "output", &blur, "input"),
        wire(&blur, "output", &inner_end, "input"),
        wire(&inner_end, "output", &end, "input"),
        wire(&end, "output", &out, "input"),
    ];
    let graph = built(nodes.clone(), edges.clone());
    assert_eq!(graph.validate(None), Ok(()));
    let pairs = graph.repeats().expect("two pairs");
    let outer = pairs.iter().find(|r| r.start == start.id()).expect("outer");
    assert_eq!(outer.end, end.id());
    assert_eq!(
        outer.body,
        vec![inner_start.id(), blur.id(), inner_end.id()]
    );
    let inner = pairs
        .iter()
        .find(|r| r.start == inner_start.id())
        .expect("inner");
    assert_eq!(inner.body, vec![blur.id()]);

    // One time of the outer pair is a graph of its own: the start an
    // Input under its own id, the end its Output, and nothing wired in
    // from outside. Its numbers are named for the start.
    let (time, fed) = graph.repeat_graph(outer);
    assert!(fed.is_empty());
    assert_eq!(time.output_id(), Some(end.id()));
    assert_eq!(time.validate(None), Ok(()));
    assert_eq!(time.repeats().map(|r| r.len()), Ok(1));
    let GraphNode::Fx(start_box) = &start else {
        panic!("the start is a box");
    };
    let values = repeat_values(start_box, 2, 5, &Default::default());
    assert_eq!(values.len(), crate::fx::effects::repeat::PASS_PORTS.len());
    assert!(values
        .iter()
        .all(|(id, _)| time.inputs().any(|i| i.id == *id)));

    // Out of the middle straight to the Output, past both ends.
    edges[5] = wire(&blur, "output", &out, "input");
    assert_eq!(
        built(nodes.clone(), edges).validate(None),
        Err(GraphError::RepeatLeak)
    );

    // An end on its own hands its picture on, and two ends on one start
    // are refused.
    let lone = fx(REPEAT_END);
    let graph = built(
        vec![src.clone(), lone.clone(), out.clone()],
        vec![
            wire(&src, "output", &lone, "input"),
            wire(&lone, "output", &out, "input"),
        ],
    );
    assert_eq!(graph.validate(None), Ok(()));
    assert_eq!(graph.repeats(), Ok(Vec::new()));
    let merge = fx("merge");
    let graph = built(
        vec![
            src.clone(),
            start.clone(),
            lone.clone(),
            end.clone(),
            merge.clone(),
            out.clone(),
        ],
        vec![
            wire(&src, "output", &start, "input"),
            wire(&start, "output", &lone, "input"),
            wire(&start, "output", &end, "input"),
            wire(&lone, "output", &merge, "input"),
            wire(&end, "output", &merge, "background"),
            wire(&merge, "output", &out, "input"),
        ],
    );
    assert_eq!(graph.validate(None), Err(GraphError::RepeatUnpaired));

    // Each path: a Read of a comp hands on a box round each path of its
    // shape layers, laid where the Read lays the comp.
    let mut art = comp();
    let square = |x: f64| {
        crate::shape::ShapeItem::filled(
            "Rectangle",
            crate::mask::Mask::rectangle(x, 100.0, 40.0, 40.0).path,
            LinearColour([1.0; 4]),
        )
    };
    let item = ProjectItem::Composition(art.clone());
    let mut layer = read_layer(Uuid::now_v7(), &item, &art).expect("a layer");
    layer.kind = LayerKind::Shape {
        contents: vec![
            crate::shape::modifier::ShapeEntry::Item(square(100.0)),
            crate::shape::modifier::ShapeEntry::Item(square(300.0)),
        ],
    };
    layer.transform = crate::model::TransformGroup {
        position_x: crate::anim::Property::fixed(100.0),
        position_y: crate::anim::Property::fixed(100.0),
        ..Default::default()
    };
    art.layers.push(layer);
    let art_id = art.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(art));
    let context = crate::expression::ExpressionContext {
        document: std::sync::Arc::new(doc),
        comp: None,
        layer: None,
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    };
    let boxes = read_paths(&context, art_id, [1920.0, 1080.0], 0.0);
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    assert!((boxes[0][0] - 100.0).abs() < 1.0 && (boxes[1][2] - 340.0).abs() < 1.0);
}

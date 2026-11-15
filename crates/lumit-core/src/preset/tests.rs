//! Tests for presets.

use super::*;

fn stack() -> Vec<EffectInstance> {
    vec![
        crate::fx::builtins::instantiate("blur").unwrap(),
        crate::fx::builtins::instantiate("glow").unwrap(),
    ]
}

#[test]
fn a_preset_round_trips_through_json() {
    let effects = stack();
    let json = to_json("My look", &effects).unwrap();
    let back = from_json(&json).unwrap();
    assert_eq!(back.format, PRESET_FORMAT);
    assert_eq!(back.name, "My look");
    assert_eq!(back.effects, effects);
}

#[test]
fn instantiating_gives_fresh_ids_but_keeps_the_effects() {
    let preset = from_json(&to_json("look", &stack()).unwrap()).unwrap();
    let a = instantiated(&preset);
    let b = instantiated(&preset);
    // Same effects and params, but every instance id is unique.
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].effect, preset.effects[0].effect);
    assert_ne!(a[0].id, preset.effects[0].id);
    assert_ne!(a[0].id, b[0].id);
}

#[test]
fn list_presets_reads_names_sorts_and_ignores_non_lumfx() {
    let dir = tempfile::tempdir().unwrap();
    // Two valid presets whose display names differ from their file stems.
    std::fs::write(
        dir.path().join("z-file.lumfx"),
        to_json("Alpha look", &stack()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("a-file.lumfx"),
        to_json("Beta look", &stack()).unwrap(),
    )
    .unwrap();
    // A non-preset file and a garbage .lumfx (kept, listed by its stem).
    std::fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();
    std::fs::write(dir.path().join("broken.lumfx"), "{ not json").unwrap();

    let entries = list_presets(dir.path());
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    // Sorted by display name (case-insensitive): the parsed names win over
    // the stems, and the unreadable file falls back to its stem.
    assert_eq!(names, vec!["Alpha look", "Beta look", "broken"]);
}

#[test]
fn a_newer_format_still_loads() {
    // A preset written by a hypothetical newer Lumit, with an unknown
    // top-level field, still parses — serde ignores what it doesn't know.
    let effects = stack();
    let mut v = serde_json::to_value(EffectPreset {
        format: 99,
        name: "future".into(),
        effects: effects.clone(),
        wiring: None,
    })
    .unwrap();
    v.as_object_mut()
        .unwrap()
        .insert("future_field".into(), serde_json::json!(true));
    let back = from_json(&v.to_string()).unwrap();
    assert_eq!(back.effects, effects);
}

// --- Node groups ------------------------------------------------------

/// Two drivers wired to each other and on into a blur in the stack, a
/// third outside them, and one wire leaving the set: the shape every
/// group claim is made against.
fn wired_graph() -> (
    crate::graph::LayerGraph,
    Vec<EffectInstance>,
    Vec<crate::graph::NodeRef>,
) {
    use crate::graph::{Edge, InputRef, LayerGraph, NodeRef, OutputRef};
    let wiggle = crate::fx::builtins::instantiate("wiggle").unwrap();
    let smooth = crate::fx::builtins::instantiate("smooth").unwrap();
    let outside = crate::fx::builtins::instantiate("wiggle").unwrap();
    let blur = crate::fx::builtins::instantiate("blur").unwrap();
    let (a, b, c, d) = (wiggle.id, smooth.id, outside.id, blur.id);
    let graph = LayerGraph {
        widths: Vec::new(),
        notes: Vec::new(),
        ties: Vec::new(),
        looks: Default::default(),
        out_unwired: false,
        nodes: vec![wiggle, smooth, outside],
        edges: vec![
            // Inside the pair.
            Edge {
                from: OutputRef::Driver {
                    node: a,
                    port: "value".into(),
                },
                to: InputRef::Param {
                    node: NodeRef::Driver(b),
                    port: "value".into(),
                },
            },
            // Out of the set, into the box left behind.
            Edge {
                from: OutputRef::Driver {
                    node: b,
                    port: "value".into(),
                },
                to: InputRef::Param {
                    node: NodeRef::Driver(c),
                    port: "amount".into(),
                },
            },
            // Inside the set, into the stack's blur.
            Edge {
                from: OutputRef::Driver {
                    node: b,
                    port: "value".into(),
                },
                to: InputRef::Param {
                    node: NodeRef::Effect(d),
                    port: "radius".into(),
                },
            },
        ],
        layout: vec![
            (NodeRef::Driver(a), [100.0, 60.0]),
            (NodeRef::Driver(b), [260.0, 90.0]),
            (NodeRef::Driver(c), [500.0, 300.0]),
            (NodeRef::Effect(d), [420.0, 60.0]),
        ],
        exposed: Vec::new(),
        // The two drivers are a group of their own, inside the set.
        groups: vec![crate::graph::NodeGroup {
            name: "Pair".into(),
            colour: 2,
            members: vec![NodeRef::Driver(a), NodeRef::Driver(b)],
        }],
    };
    let members = vec![NodeRef::Driver(a), NodeRef::Driver(b), NodeRef::Effect(d)];
    (graph, vec![blur], members)
}

/// Copying the blur takes the smooth that feeds it and the
/// wiggle that feeds the smooth, with the two wires between them, and
/// leaves the third driver and the wire to it behind; the wiring rides
/// in the copied text and comes back with every id fresh. Copying
/// nothing wired stays a plain preset. Guards effects that lose their
/// drivers when copied to another layer.
#[test]
fn copied_effects_take_the_drivers_that_feed_them() {
    use crate::graph::{InputRef, NodeRef, OutputRef};
    let (graph, stack, _) = wired_graph();
    let blur = stack[0].id;
    let wiring = wiring_of(&graph, &stack, &[blur]).unwrap();
    let taken: Vec<_> = wiring.nodes.iter().map(|n| n.id).collect();
    assert_eq!(taken, vec![graph.nodes[0].id, graph.nodes[1].id]);
    assert_eq!(wiring.edges.len(), 2);
    assert_eq!(wiring.effects.len(), 1);

    let text = to_json_wired("look", &stack, Some(wiring)).unwrap();
    let back = from_json(&text).unwrap();
    let added = group_instantiated(back.wiring.as_ref().unwrap(), [0.0, 0.0]);
    assert_ne!(added.effects[0].id, blur);
    let into_blur = added.edges.iter().find(|e| {
            matches!(&e.to, InputRef::Param { node: NodeRef::Effect(id), .. } if *id == added.effects[0].id)
        });
    assert!(
        matches!(into_blur.map(|e| &e.from), Some(OutputRef::Driver { node, .. }) if *node == added.nodes[1].id),
        "the fresh smooth feeds the fresh blur"
    );

    let plain = vec![crate::fx::builtins::instantiate("glow").unwrap()];
    assert!(wiring_of(&graph, &plain, &[plain[0].id]).is_none());
    assert!(from_json(&to_json("x", &plain).unwrap())
        .unwrap()
        .wiring
        .is_none());
}

#[test]
fn inserting_a_group_mints_fresh_ids_and_re_points_the_wires() {
    let (graph, stack, members) = wired_graph();
    let preset = group_from_json(
        &group_to_json(&group_from_graph(&graph, &stack, "Audio rig", 4, &members)).unwrap(),
    )
    .unwrap();

    let added = group_instantiated(&preset, [400.0, 200.0]);
    let (nodes, edges, layout, group) = (added.nodes, added.edges, added.layout, added.group);
    assert_eq!(nodes.len(), 2);
    // The blur came too, as a fresh effect for the stack, still driven.
    assert_eq!(added.effects.len(), 1);
    assert_ne!(added.effects[0].id, stack[0].id);
    assert_eq!(
        edges[1].to,
        crate::graph::InputRef::Param {
            node: crate::graph::NodeRef::Effect(added.effects[0].id),
            port: "radius".into(),
        }
    );
    assert_eq!(layout[2].1, [720.0, 200.0]);
    assert!(
        nodes
            .iter()
            .all(|n| !preset.nodes.iter().any(|s| s.id == n.id)),
        "every instance id is fresh, so inserting twice never shares one"
    );
    // The wires came back pointing at the *new* boxes, not the saved ones.
    assert_eq!(edges.len(), 2);
    let crate::graph::OutputRef::Driver { node: from, .. } = edges[0].from else {
        panic!("a driver wire");
    };
    let crate::graph::InputRef::Param {
        node: crate::graph::NodeRef::Driver(to),
        ..
    } = edges[0].to
    else {
        panic!("into a driver parameter");
    };
    assert_eq!(from, nodes[0].id);
    assert_eq!(to, nodes[1].id);
    // Dropped where it was asked for, keeping the saved shape.
    assert_eq!(layout[0].1, [400.0, 200.0]);
    assert_eq!(layout[1].1, [560.0, 230.0]);
    assert_eq!(group.name, "Audio rig");
    assert_eq!(group.members.len(), 3);
    // The group inside it came back round the two fresh drivers.
    assert_eq!(added.inner.len(), 1);
    assert_eq!(added.inner[0].name, "Pair");
    assert_eq!(
        added.inner[0].members,
        vec![
            crate::graph::NodeRef::Driver(nodes[0].id),
            crate::graph::NodeRef::Driver(nodes[1].id),
        ]
    );
}

// -- 5.8, a node graph's groups -------------------------------------------

/// A Read, a blur and an Output, wired in a line, with the first two
/// picked. The Output is picked too, to prove it is never saved.
fn wired_comp_graph() -> (crate::comp_graph::CompGraph, Vec<uuid::Uuid>, uuid::Uuid) {
    use crate::comp_graph::{CompGraph, GraphEdge, GraphNode};
    use crate::graph::{INPUT_PORT, OUTPUT_PORT};

    let item = uuid::Uuid::now_v7();
    let read = GraphNode::Read {
        id: uuid::Uuid::now_v7(),
        item,
        custom_name: None,
    };
    let blur = GraphNode::Fx(crate::fx::builtins::instantiate("blur").unwrap());
    let out = GraphNode::Output {
        id: uuid::Uuid::now_v7(),
    };
    let wire = |from: &GraphNode, to: &GraphNode| GraphEdge {
        from: from.id(),
        from_port: OUTPUT_PORT.id.to_owned(),
        to: to.id(),
        to_port: INPUT_PORT.id.to_owned(),
    };
    let members = vec![read.id(), blur.id(), out.id()];
    let graph = CompGraph {
        widths: Vec::new(),
        notes: Vec::new(),
        ties: Vec::new(),
        looks: Default::default(),
        edges: vec![wire(&read, &blur), wire(&blur, &out)],
        layout: vec![
            (read.id(), [100.0, 60.0]),
            (blur.id(), [260.0, 90.0]),
            (out.id(), [500.0, 300.0]),
        ],
        nodes: vec![read, blur, out],
        exposed: Vec::new(),
        groups: Vec::new(),
    };
    (graph, members, item)
}

#[test]
fn inserting_a_comp_group_mints_fresh_ids_and_keeps_a_reads_item() {
    use crate::comp_graph::GraphNode;

    let (graph, members, item) = wired_comp_graph();
    let mut preset = comp_group_from_json(
        &comp_group_to_json(&comp_group_from_graph(&graph, "Plate rig", 4, &members)).unwrap(),
    )
    .unwrap();
    // A wire naming a box the file does not carry is dropped rather than
    // landing dangling, which would cost the whole insert.
    preset.edges.push(crate::comp_graph::GraphEdge {
        from: uuid::Uuid::now_v7(),
        from_port: "output".into(),
        to: preset.nodes[1].id(),
        to_port: "input".into(),
    });

    let added = comp_group_instantiated(&preset, [400.0, 200.0]);
    assert_eq!(added.nodes.len(), 2);
    assert!(
        added
            .nodes
            .iter()
            .all(|n| !preset.nodes.iter().any(|s| s.id() == n.id())),
        "every id is fresh, so inserting twice never makes two boxes one"
    );
    // A Read keeps the item it names: the item is the project's, not the
    // group's, and one this project lacks wears the missing mark.
    match &added.nodes[0] {
        GraphNode::Read { item: named, .. } => assert_eq!(*named, item),
        other => panic!("the Read came back as {other:?}"),
    }
    assert_eq!(added.edges.len(), 1, "the dangling wire was dropped");
    assert_eq!(added.edges[0].from, added.nodes[0].id());
    assert_eq!(added.edges[0].to, added.nodes[1].id());
    assert_eq!(added.layout[0].1, [400.0, 200.0]);
    assert_eq!(added.layout[1].1, [560.0, 230.0]);
    assert_eq!(added.group.name, "Plate rig");
    assert_eq!(added.group.members.len(), 2);

    // A second insert shares nothing with the first.
    let again = comp_group_instantiated(&preset, [0.0, 0.0]);
    assert!(added
        .nodes
        .iter()
        .all(|a| !again.nodes.iter().any(|b| b.id() == a.id())));
}

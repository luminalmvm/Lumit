#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::fx::builtins::instantiate;

/// A driver or effect instance to hang wires off.
fn inst(match_name: &str) -> EffectInstance {
    instantiate(match_name).expect("the catalogue knows it")
}

fn param_edge(from: &EffectInstance, port: &str, to: NodeRef, socket: &str) -> Edge {
    Edge {
        from: OutputRef::Driver {
            node: from.id,
            port: port.to_owned(),
        },
        to: InputRef::Param {
            node: to,
            port: socket.to_owned(),
        },
    }
}

/// The shape every other test bends: a Wiggle driving a Gaussian blur's
/// radius.
fn wiggle_into_blur() -> (LayerGraph, Vec<EffectInstance>) {
    let blur = inst("blur");
    let wiggle = inst("wiggle");
    let edge = param_edge(&wiggle, "value", NodeRef::Effect(blur.id), "radius");
    (
        LayerGraph {
            notes: Vec::new(),
            ties: Vec::new(),
            looks: Default::default(),
            out_unwired: false,
            nodes: vec![wiggle],
            edges: vec![edge],
            layout: vec![(NodeRef::Source, [0.0, 0.0])],
            exposed: Vec::new(),
            groups: Vec::new(),
        },
        vec![blur],
    )
}

/// The Node preview's prefix: the picture at a node is the stack
/// cut off there, so naming the node names a length. A driver has no
/// length because it has no picture.
#[test]
fn a_nodes_prefix_is_the_stack_cut_off_at_it() {
    let first = inst("blur");
    let second = inst("blur");
    let wiggle = inst("wiggle");
    let effects = vec![first.clone(), second.clone()];

    assert_eq!(prefix_len(&effects, NodeRef::Source), Some(0));
    assert_eq!(prefix_len(&effects, NodeRef::Effect(first.id)), Some(1));
    assert_eq!(prefix_len(&effects, NodeRef::Effect(second.id)), Some(2));
    assert_eq!(prefix_len(&effects, NodeRef::Out), Some(2));
    assert_eq!(
        prefix_len(&effects, NodeRef::Driver(wiggle.id)),
        None,
        "a driver makes a number, not a picture"
    );
    assert_eq!(
        prefix_len(&effects, NodeRef::Effect(wiggle.id)),
        None,
        "an effect this layer does not carry names no prefix"
    );
    assert_eq!(prefix_len(&[], NodeRef::Out), Some(0));
}

/// Number accepts number and colour accepts colour. The two crossed over
/// are refused in both directions.
#[test]
fn a_number_into_a_colour_is_refused_and_so_is_a_colour_into_a_number() {
    let fill = inst("fill");
    let wiggle = inst("wiggle");
    let cycle = inst("colour_cycle");

    let graph = LayerGraph {
        out_unwired: false,
        nodes: vec![wiggle.clone()],
        edges: vec![param_edge(
            &wiggle,
            "value",
            NodeRef::Effect(fill.id),
            "colour",
        )],
        ..LayerGraph::default()
    };
    assert_eq!(
        graph.validate(std::slice::from_ref(&fill)),
        Err(GraphError::PortTypeMismatch)
    );

    let blur = inst("blur");
    let graph = LayerGraph {
        out_unwired: false,
        nodes: vec![cycle.clone()],
        edges: vec![param_edge(
            &cycle,
            "colour",
            NodeRef::Effect(blur.id),
            "radius",
        )],
        ..LayerGraph::default()
    };
    assert_eq!(
        graph.validate(std::slice::from_ref(&blur)),
        Err(GraphError::PortTypeMismatch)
    );

    // And the accepted pair, so the refusals above are about the types and
    // not about the wiring.
    let graph = LayerGraph {
        out_unwired: false,
        nodes: vec![cycle.clone()],
        edges: vec![param_edge(
            &cycle,
            "colour",
            NodeRef::Effect(fill.id),
            "colour",
        )],
        ..LayerGraph::default()
    };
    graph
        .validate(std::slice::from_ref(&fill))
        .expect("colour into colour");
}

/// A driver feeding itself, and a pair feeding each other: both refused
/// before anything is swapped into the document.
#[test]
fn a_loop_among_drivers_is_refused() {
    let a = inst("smooth");
    let graph = LayerGraph {
        out_unwired: false,
        edges: vec![param_edge(&a, "value", NodeRef::Driver(a.id), "value")],
        nodes: vec![a],
        ..LayerGraph::default()
    };
    assert_eq!(graph.validate(&[]), Err(GraphError::Cycle));

    let a = inst("smooth");
    let b = inst("remap");
    let graph = LayerGraph {
        out_unwired: false,
        edges: vec![
            param_edge(&a, "value", NodeRef::Driver(b.id), "value"),
            param_edge(&b, "value", NodeRef::Driver(a.id), "value"),
        ],
        nodes: vec![a, b],
        ..LayerGraph::default()
    };
    assert_eq!(graph.validate(&[]), Err(GraphError::Cycle));
}

/// §1.4: the layer's own source alpha is a Matte output, and an effect's
/// matte input takes it.
#[test]
fn the_source_matte_feeds_an_effects_matte_input() {
    let blur = inst("blur");
    let graph = LayerGraph {
        out_unwired: false,
        edges: vec![Edge {
            from: OutputRef::SourceMatte,
            to: InputRef::Matte { effect: blur.id },
        }],
        ..LayerGraph::default()
    };
    graph
        .validate(std::slice::from_ref(&blur))
        .expect("every effect with a matte row takes one");
    assert!(graph.source_matte(blur.id));
    assert!(!graph.source_matte(Uuid::now_v7()));

    // An effect that carries no matte row at all has no socket to wire.
    let set_matte = inst("set_matte");
    let graph = LayerGraph {
        out_unwired: false,
        edges: vec![Edge {
            from: OutputRef::SourceMatte,
            to: InputRef::Matte {
                effect: set_matte.id,
            },
        }],
        ..LayerGraph::default()
    };
    assert_eq!(
        graph.validate(std::slice::from_ref(&set_matte)),
        Err(GraphError::UnknownPort)
    );
}

/// The one derived socket a wire *may* land on: the Layer out's Volume
/// (the Audio panel's *Duck under* writes it). A number is accepted;
/// anything else is the ordinary type refusal, and every other made-up
/// port on the derived nodes stays refused above.
#[test]
fn a_number_wire_may_land_on_the_layer_outs_volume() {
    let wiggle = inst("wiggle");
    let graph = LayerGraph {
        out_unwired: false,
        edges: vec![param_edge(
            &wiggle,
            "value",
            NodeRef::Out,
            OUT_VOLUME_PORT.id,
        )],
        nodes: vec![wiggle],
        ..LayerGraph::default()
    };
    graph
        .validate(&[])
        .expect("a number onto the Volume socket is a legal duck");
}

/// §4: the whole graph survives a trip through the file format — wires,
/// positions and all.
#[test]
fn a_graph_round_trips_through_json() {
    let (mut graph, _) = wiggle_into_blur();
    graph
        .layout
        .push((NodeRef::Driver(graph.nodes[0].id), [12.5, -3.0]));
    graph.edges.push(Edge {
        from: OutputRef::SourceMatte,
        to: InputRef::Matte {
            effect: Uuid::now_v7(),
        },
    });
    graph.looks.set_colour(NodeRef::Source, 3);
    graph
        .looks
        .set_collapsed(NodeRef::Driver(graph.nodes[0].id), true);
    let json = serde_json::to_string(&graph).expect("serialises");
    let back: LayerGraph = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, graph);
    // A box's colour alone is enough to keep the graph in the file.
    let looks_only = LayerGraph {
        looks: graph.looks.clone(),
        ..LayerGraph::default()
    };
    assert!(!looks_only.is_empty());
}

// -----------------------------------------------------------------------
// The points edge (points-stream.md §1).
// -----------------------------------------------------------------------

/// A wire out of a stack effect's declared data output.
fn points_edge(producer: &EffectInstance, to: InputRef) -> Edge {
    Edge {
        from: OutputRef::EffectData {
            effect: producer.id,
            port: "points".to_owned(),
        },
        to,
    }
}

/// A wire onto a stack effect's parameter socket.
fn onto(effect: &EffectInstance, port: &str) -> InputRef {
    InputRef::Param {
        node: NodeRef::Effect(effect.id),
        port: port.to_owned(),
    }
}

/// §1.2's sharpest risk: the cycle check must walk **through** effect data
/// sources, or the demand-driven driver walk that PS4 builds on it would not
/// terminate. The v1 loop is real — a driver reads Particulate's stream and
/// its output is wired back into a Particulate parameter.
///
/// Checked against the walk itself rather than through `validate`, because
/// the driver that declares a Points *input* arrives with Points sample and
/// the type check would refuse these graphs first. The link set is the part
/// under test and it is the part that would rot.
#[test]
fn the_cycle_walk_goes_through_an_effects_data_output() {
    let particulate = inst("particulate");
    let smooth = inst("smooth");
    let stack = vec![particulate.clone()];

    let stream_into = |driver: &EffectInstance| {
        points_edge(
            &particulate,
            InputRef::Param {
                node: NodeRef::Driver(driver.id),
                port: "points".to_owned(),
            },
        )
    };
    let back_into_producer = |driver: &EffectInstance| {
        param_edge(
            driver,
            "value",
            NodeRef::Effect(particulate.id),
            "emit_rate",
        )
    };

    // One leg only: the stream feeds a driver, and nothing returns.
    let open = LayerGraph {
        out_unwired: false,
        nodes: vec![smooth.clone()],
        edges: vec![stream_into(&smooth)],
        ..LayerGraph::default()
    };
    open.check_acyclic(&stack).expect("a line is not a loop");

    // Both legs: the stream depends on the parameters and the parameters on
    // the stream.
    let closed = LayerGraph {
        out_unwired: false,
        nodes: vec![smooth.clone()],
        edges: vec![stream_into(&smooth), back_into_producer(&smooth)],
        ..LayerGraph::default()
    };
    assert_eq!(closed.check_acyclic(&stack), Err(GraphError::Cycle));

    // The driver's output alone, with no stream read, is not a loop — so
    // the refusal above is about the round trip and not about an effect
    // merely being in the walk.
    let driven = LayerGraph {
        out_unwired: false,
        nodes: vec![smooth.clone()],
        edges: vec![back_into_producer(&smooth)],
        ..LayerGraph::default()
    };
    driven
        .check_acyclic(&stack)
        .expect("a driven parameter is not a loop");
}

/// The healing half of the carve-out: a reorder that inverts a
/// points wire drops it, because a **stack** edit cannot be refused on the
/// wiring's behalf. Without this the document would hold a graph its own
/// validator refuses, and the next wire anybody drew would be refused for
/// it.
#[test]
fn an_inverting_reorder_heals_rather_than_refusing() {
    let particulate = inst("particulate");
    let blur = inst("blur");
    let down = vec![particulate.clone(), blur.clone()];
    let up = vec![blur.clone(), particulate.clone()];
    let wired = LayerGraph {
        out_unwired: false,
        edges: vec![points_edge(&particulate, onto(&blur, "points_in"))],
        ..LayerGraph::default()
    };

    // Down the stack: nothing to heal.
    let mut graph = wired.clone();
    assert!(!graph.prune_to(&down));
    assert_eq!(graph.edges.len(), 1);

    // Dragged above its consumer: the wire goes.
    let mut graph = wired.clone();
    assert!(graph.prune_to(&up), "the reorder inverted the wire");
    assert!(graph.edges.is_empty());

    // And the state the heal prevents is genuinely unreachable: left in
    // place, that same graph is what `validate` refuses.
    assert_eq!(wired.validate(&up), Err(GraphError::Cycle));
    // The pruned one is accepted against the new order, which is the
    // property that keeps the next `SetLayerGraph` from being refused for a
    // wire nobody drew.
    graph.validate(&up).expect("healed, and so committable");
}

/// **A real stack-to-stack points wire**: the arrangement
/// §1.2 was written for, now that there is a consumer to draw one into.
///
/// Everything it exercises was landed by PS3 against a made-up port name;
/// this is the same rules against two effects that genuinely declare the
/// sockets — so a signature that stopped answering, on either end, would
/// show up here rather than in a panel.
#[test]
fn a_points_wire_between_two_stack_effects_is_accepted_down_the_stack() {
    let producer = inst("particulate");
    let consumer = inst("clone_to_points");
    let down = vec![producer.clone(), consumer.clone()];
    let graph = LayerGraph {
        out_unwired: false,
        edges: vec![points_edge(&producer, onto(&consumer, "points"))],
        ..LayerGraph::default()
    };
    graph
        .validate(&down)
        .expect("a producer above its consumer is the arrangement §1.2 asks for");

    // Back up the stack: the loop sentence, because that is what such a
    // wire is asking for.
    let up = vec![consumer.clone(), producer.clone()];
    assert_eq!(graph.validate(&up), Err(GraphError::Cycle));

    // Two consumers off one producer, both down the stack: a stream is not
    // spent by being read, and the one-wire-per-*input* rule is about the
    // socket rather than about the source.
    let trail = inst("trail");
    let both = LayerGraph {
        out_unwired: false,
        edges: vec![
            points_edge(&producer, onto(&consumer, "points")),
            points_edge(&producer, onto(&trail, "points")),
        ],
        ..LayerGraph::default()
    };
    both.validate(&[producer.clone(), consumer.clone(), trail])
        .expect("one producer may feed two consumers");

    // A generator feeds the same socket — the wire does not know which
    // producer it came from, which is the whole point of one port type.
    let grid = inst("grid");
    let from_grid = LayerGraph {
        out_unwired: false,
        edges: vec![points_edge(&grid, onto(&consumer, "points"))],
        ..LayerGraph::default()
    };
    from_grid
        .validate(&[grid, consumer.clone()])
        .expect("a lattice is a points stream like any other");

    // And a number is not a stream, on this end as on the driver's.
    let wiggle = inst("wiggle");
    let mistyped = LayerGraph {
        out_unwired: false,
        nodes: vec![wiggle.clone()],
        edges: vec![Edge {
            from: OutputRef::Driver {
                node: wiggle.id,
                port: "value".to_owned(),
            },
            to: onto(&consumer, "points"),
        }],
        ..LayerGraph::default()
    };
    assert_eq!(
        mistyped.validate(&[producer, consumer]),
        Err(GraphError::PortTypeMismatch)
    );
}

#[test]
fn disconnected_output_makes_the_graph_non_empty_and_persists() {
    let graph = LayerGraph {
        out_unwired: true,
        ..LayerGraph::default()
    };
    assert!(!graph.is_empty());
    let json = serde_json::to_string(&graph).expect("serialises");
    assert!(json.contains("out_unwired"));
    assert!(
        serde_json::from_str::<LayerGraph>(&json)
            .expect("deserialises")
            .out_unwired
    );
}

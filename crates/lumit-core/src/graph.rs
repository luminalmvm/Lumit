//! The layer driver graph: the additive wiring a layer carries beside its
//! effect stack (K-471, K-472, [impl/node-graph.md](../../../docs/impl/node-graph.md)).
//!
//! # In plain terms
//!
//! A layer's effects are a list: the picture goes in at the top, each effect
//! changes it, the result comes out at the bottom. **That list stays the only
//! authority for the picture.** What this module adds is a second kind of box —
//! a **driver**. A driver makes no picture; it makes a *value*: a wobbling
//! number (Wiggle), the loudness of the music (Audio level), a slowly turning
//! colour (Colour cycle). A wire from a driver into an effect's socket makes
//! that effect's parameter follow the value instead of its keyframes. "The glow
//! pulses with the music" becomes one wire you can see, instead of an
//! expression you have to write.
//!
//! So a layer carries its drivers ([`LayerGraph::nodes`]), the wires between
//! them ([`LayerGraph::edges`]), and where the boxes sit on the canvas
//! ([`LayerGraph::layout`]). A layer that never opens the Graph panel carries
//! none of it, and the field is left out of the saved file entirely — which is
//! what lets every project written before drivers existed open, and save back
//! the same bytes.
//!
//! The image chain itself is **not** stored here. The Source node, one node per
//! effect in stack order and the Layer out node are all *derived* from
//! `Layer::effects`, and every image-wire gesture lowers to the existing
//! whole-stack commit (§1.1 of the note). There is no graph you can build that
//! the ordinary Effect controls has to lie about.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fx::{Port, PortType};
use crate::model::EffectInstance;

/// The picture leaving a Source node, entering the Layer out node, or the
/// image a node hands on.
pub const IMAGE_PORT: Port = Port::new("image", "Image", PortType::Image);
/// Coverage: the layer's own masked source alpha, and the socket on an effect
/// that reads one.
pub const MATTE_PORT: Port = Port::new("matte", "Matte", PortType::Matte);
/// An effect node's one picture in — by construction the previous stack
/// entry's output (§1.1), which is why no gesture can branch the chain.
pub const INPUT_PORT: Port = Port::new("input", "Input", PortType::Image);
/// An effect node's one picture out.
pub const OUTPUT_PORT: Port = Port::new("output", "Output", PortType::Image);
/// The Layer out node's sound. Drawn and unfilled: audio comes only from a
/// footage layer's own stream (K-435), so this accepts no wire in this phase
/// (§7) — listed rather than faked.
pub const AUDIO_PORT: Port = Port::new("audio", "Audio", PortType::Audio);

/// Every port the *derived* nodes draw — the ones no schema declares, because
/// the Source node, an effect's picture in and out and the Layer out node are
/// all worked out from the effect stack rather than stored.
///
/// Gathered in one list so the K-303 label walk ([`crate::fx::labels`]) can
/// find their words the same way it finds an effect's.
pub const DERIVED_PORTS: [Port; 5] = [IMAGE_PORT, MATTE_PORT, INPUT_PORT, OUTPUT_PORT, AUDIO_PORT];

/// What the canvas calls [`NodeRef::Source`] — display text (K-303).
pub const SOURCE_LABEL: &str = "Source";
/// What the canvas calls [`NodeRef::Out`] — display text (K-303).
pub const OUT_LABEL: &str = "Layer out";

/// Names anything the graph canvas draws (K-471 §1.2).
///
/// The image-path nodes are **derived** from the effect stack rather than
/// stored, so they get stable synthetic refs: there is exactly one Source and
/// one Out per layer, and an effect node is named by the id of its
/// [`EffectInstance`] in `Layer::effects`. A driver node is named by the id of
/// its instance in [`LayerGraph::nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodeRef {
    /// The layer's own source — its picture and its matte outputs.
    Source,
    /// An [`EffectInstance`] in `Layer::effects`.
    Effect(Uuid),
    /// An [`EffectInstance`] in [`LayerGraph::nodes`] — a driver.
    Driver(Uuid),
    /// The layer's output.
    Out,
}

/// Where a wire comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputRef {
    /// A driver node's declared output port
    /// ([`Signature::Data`](crate::fx::Signature::Data)).
    Driver {
        node: Uuid,
        /// The port's stable id, as the driver's signature declares it.
        port: String,
    },
    /// The layer's **own** masked source alpha at that point in the chain — a
    /// texture the pipeline has already computed, so wiring it costs no second
    /// render (§1.4). It overrides the effect's Matte *parameter* while it
    /// exists.
    SourceMatte,
}

/// Where a wire goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputRef {
    /// A parameter socket: the destination parameter follows the source output
    /// instead of its own keyframes. At most one wire per socket.
    Param {
        node: NodeRef,
        /// The parameter's stable snake_case id.
        port: String,
    },
    /// An effect's matte input, fed by [`OutputRef::SourceMatte`].
    Matte { effect: Uuid },
}

/// One wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: OutputRef,
    pub to: InputRef,
}

/// The additive wiring a layer carries beside its effect stack (K-471).
///
/// Empty by default and **absent from the saved file when empty**, which is
/// what makes every pre-K-471 project load unchanged and re-save byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerGraph {
    /// The driver nodes — ordinary [`EffectInstance`]s whose definition
    /// declares a data signature rather than an image kernel (§1.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<EffectInstance>,
    /// The wires.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    /// Canvas positions. **Document data**: they persist and travel, and are
    /// edited through the same whole-graph commit as everything else. A node
    /// with no entry is auto-placed by the panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<(NodeRef, [f64; 2])>,
    /// The nodes whose `E` badge is on — grown to show one socket per
    /// parameter (§1.4). Presentation state, beside [`Self::layout`] rather
    /// than on the instance, so a *derived* effect node can carry it without
    /// the whole effect stack gaining a field; it changes no pixel and so
    /// reaches no frame key. A wired socket draws regardless of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed: Vec<NodeRef>,
}

/// Why a graph was refused (§1.5).
///
/// **Refusal, not degradation.** Unlike a dangling matte, none of these states
/// can be reached by deleting some *other* entity — a deleted driver takes its
/// edges with it inside the same commit — so every one of them is an edit we
/// control, and the honest answer is a calm message rather than a silently
/// different picture. A dangling **layer reference** on a driver's parameter
/// still degrades exactly as a matte does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    #[error("a wire names a node this layer does not have")]
    UnknownNode,
    #[error("a wire names a port that does not exist")]
    UnknownPort,
    #[error("a wire joins two ports of different types")]
    PortTypeMismatch,
    #[error("a socket cannot take a second wire")]
    InputAlreadyWired,
    #[error("the wire would close a loop")]
    Cycle,
}

impl LayerGraph {
    /// Whether this layer carries no wiring at all — the overwhelming case, and
    /// what keeps the field out of the saved file.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.edges.is_empty()
            && self.layout.is_empty()
            && self.exposed.is_empty()
    }

    /// The driver instance named by `id`, if this graph carries one.
    #[must_use]
    pub fn node(&self, id: Uuid) -> Option<&EffectInstance> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The wire feeding `to`, if any.
    #[must_use]
    pub fn wire_into(&self, to: &InputRef) -> Option<&OutputRef> {
        self.edges.iter().find(|e| &e.to == to).map(|e| &e.from)
    }

    /// Whether the effect named by `effect` takes the layer's own source alpha
    /// as its matte (§1.4) — the one in-graph feed the Matte row can carry.
    #[must_use]
    pub fn source_matte(&self, effect: Uuid) -> bool {
        self.edges
            .iter()
            .any(|e| e.from == OutputRef::SourceMatte && e.to == InputRef::Matte { effect })
    }

    /// Check every rule `SetLayerGraph` enforces (§1.5), against the effect
    /// stack the graph sits beside.
    ///
    /// Missing node or port, a type mismatch, a doubled input, or a loop among
    /// the driver nodes — each is refused, because each can only be reached by
    /// an edit this application made.
    pub fn validate(&self, effects: &[EffectInstance]) -> Result<(), GraphError> {
        for (i, edge) in self.edges.iter().enumerate() {
            // One wire per socket. Comparing against the edges *before* this
            // one reports the second wire, which is the one being added.
            if self.edges[..i].iter().any(|e| e.to == edge.to) {
                return Err(GraphError::InputAlreadyWired);
            }
            let from = self.output_type(&edge.from)?;
            let to = self.input_type(&edge.to, effects)?;
            if from != to {
                return Err(GraphError::PortTypeMismatch);
            }
        }
        self.check_acyclic()
    }

    /// The type a wire's source carries.
    fn output_type(&self, from: &OutputRef) -> Result<PortType, GraphError> {
        match from {
            OutputRef::SourceMatte => Ok(PortType::Matte),
            OutputRef::Driver { node, port } => {
                let inst = self.node(*node).ok_or(GraphError::UnknownNode)?;
                let def = crate::fx::BUILTIN_DEFS
                    .get(&inst.effect.match_name)
                    .ok_or(GraphError::UnknownNode)?;
                def.signature().output(port).ok_or(GraphError::UnknownPort)
            }
        }
    }

    /// The type a wire's destination accepts.
    fn input_type(
        &self,
        to: &InputRef,
        effects: &[EffectInstance],
    ) -> Result<PortType, GraphError> {
        match to {
            InputRef::Matte { effect } => {
                // An effect that declares no matte row has no matte socket to
                // wire, which is the same refusal as a missing port.
                let inst = effects
                    .iter()
                    .find(|e| e.id == *effect)
                    .ok_or(GraphError::UnknownNode)?;
                let def = crate::fx::BUILTIN_DEFS
                    .get(&inst.effect.match_name)
                    .ok_or(GraphError::UnknownNode)?;
                def.schema()
                    .matte
                    .param()
                    .map(|_| PortType::Matte)
                    .ok_or(GraphError::UnknownPort)
            }
            InputRef::Param { node, port } => {
                // Source and Out draw ports but hold no parameters, so naming
                // one of them as a parameter destination names nothing.
                let inst = match node {
                    NodeRef::Effect(id) => effects.iter().find(|e| e.id == *id),
                    NodeRef::Driver(id) => self.node(*id),
                    NodeRef::Source | NodeRef::Out => None,
                }
                .ok_or(GraphError::UnknownNode)?;
                let def = crate::fx::BUILTIN_DEFS
                    .get(&inst.effect.match_name)
                    .ok_or(GraphError::UnknownNode)?;
                def.schema()
                    .params
                    .iter()
                    .find(|p| p.id == port)
                    .and_then(|p| p.kind.port_type())
                    .ok_or(GraphError::UnknownPort)
            }
        }
    }

    /// Refuse a loop among the driver nodes.
    ///
    /// Kahn's algorithm over the driver-to-driver wires; anything left when no
    /// node has an unwired input is in a loop. Only drivers can close one — an
    /// effect node is never a wire's *source*, so the image chain cannot take
    /// part.
    fn check_acyclic(&self) -> Result<(), GraphError> {
        // (source driver, destination driver) for every wire between two of
        // them, in document order — no map iteration anywhere, so the walk is
        // the same on every machine.
        let links: Vec<(Uuid, Uuid)> = self
            .edges
            .iter()
            .filter_map(|e| match (&e.from, &e.to) {
                (
                    OutputRef::Driver { node: from, .. },
                    InputRef::Param {
                        node: NodeRef::Driver(to),
                        ..
                    },
                ) => Some((*from, *to)),
                _ => None,
            })
            .collect();
        let mut settled: Vec<Uuid> = Vec::with_capacity(self.nodes.len());
        loop {
            let ready: Vec<Uuid> = self
                .nodes
                .iter()
                .map(|n| n.id)
                .filter(|id| !settled.contains(id))
                .filter(|id| {
                    !links
                        .iter()
                        .any(|(from, to)| to == id && !settled.contains(from))
                })
                .collect();
            if ready.is_empty() {
                return if settled.len() == self.nodes.len() {
                    Ok(())
                } else {
                    Err(GraphError::Cycle)
                };
            }
            settled.extend(ready);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::instantiate;

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
                nodes: vec![wiggle],
                edges: vec![edge],
                layout: vec![(NodeRef::Source, [0.0, 0.0])],
                exposed: Vec::new(),
            },
            vec![blur],
        )
    }

    #[test]
    fn a_well_formed_graph_is_accepted() {
        let (graph, effects) = wiggle_into_blur();
        graph.validate(&effects).expect("a number into a number");
    }

    /// §1.5: a wire naming a node the layer does not have is refused, not
    /// quietly dropped.
    #[test]
    fn a_wire_to_a_node_that_is_not_there_is_refused() {
        let (mut graph, effects) = wiggle_into_blur();
        graph.nodes.clear();
        assert_eq!(graph.validate(&effects), Err(GraphError::UnknownNode));

        let (graph, _) = wiggle_into_blur();
        assert_eq!(graph.validate(&[]), Err(GraphError::UnknownNode));
    }

    #[test]
    fn a_wire_to_a_port_that_is_not_there_is_refused() {
        let (mut graph, effects) = wiggle_into_blur();
        let InputRef::Param { port, .. } = &mut graph.edges[0].to else {
            unreachable!()
        };
        *port = "no_such_parameter".into();
        assert_eq!(graph.validate(&effects), Err(GraphError::UnknownPort));

        let (mut graph, effects) = wiggle_into_blur();
        let OutputRef::Driver { port, .. } = &mut graph.edges[0].from else {
            unreachable!()
        };
        *port = "no_such_output".into();
        assert_eq!(graph.validate(&effects), Err(GraphError::UnknownPort));
    }

    /// Number accepts number and colour accepts colour (K-472 §6.1); the two
    /// crossed over are refused in both directions.
    #[test]
    fn a_number_into_a_colour_is_refused_and_so_is_a_colour_into_a_number() {
        let fill = inst("fill");
        let wiggle = inst("wiggle");
        let cycle = inst("colour_cycle");

        let graph = LayerGraph {
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

    /// A switch is not a socket, so a wire onto one names a port that does not
    /// exist — the same answer a typo gets.
    #[test]
    fn a_wire_onto_a_switch_finds_no_socket() {
        let remap = inst("remap");
        let wiggle = inst("wiggle");
        let graph = LayerGraph {
            edges: vec![param_edge(
                &wiggle,
                "value",
                NodeRef::Driver(remap.id),
                "clamp",
            )],
            nodes: vec![wiggle, remap],
            ..LayerGraph::default()
        };
        assert_eq!(graph.validate(&[]), Err(GraphError::UnknownPort));
    }

    #[test]
    fn a_socket_refuses_a_second_wire() {
        let (mut graph, effects) = wiggle_into_blur();
        let second = inst("wiggle");
        graph.edges.push(param_edge(
            &second,
            "value",
            NodeRef::Effect(effects[0].id),
            "radius",
        ));
        graph.nodes.push(second);
        assert_eq!(graph.validate(&effects), Err(GraphError::InputAlreadyWired));
    }

    /// A driver feeding itself, and a pair feeding each other: both refused
    /// before anything is swapped into the document.
    #[test]
    fn a_loop_among_drivers_is_refused() {
        let a = inst("smooth");
        let graph = LayerGraph {
            edges: vec![param_edge(&a, "value", NodeRef::Driver(a.id), "value")],
            nodes: vec![a],
            ..LayerGraph::default()
        };
        assert_eq!(graph.validate(&[]), Err(GraphError::Cycle));

        let a = inst("smooth");
        let b = inst("remap");
        let graph = LayerGraph {
            edges: vec![
                param_edge(&a, "value", NodeRef::Driver(b.id), "value"),
                param_edge(&b, "value", NodeRef::Driver(a.id), "value"),
            ],
            nodes: vec![a, b],
            ..LayerGraph::default()
        };
        assert_eq!(graph.validate(&[]), Err(GraphError::Cycle));
    }

    /// The same drivers wired in a line, which is not a loop — and declared in
    /// the reverse of evaluation order, so the check cannot be relying on the
    /// order they happen to sit in.
    #[test]
    fn a_chain_of_drivers_is_accepted() {
        let a = inst("wiggle");
        let b = inst("remap");
        let c = inst("smooth");
        let graph = LayerGraph {
            edges: vec![
                param_edge(&a, "value", NodeRef::Driver(b.id), "value"),
                param_edge(&b, "value", NodeRef::Driver(c.id), "value"),
            ],
            nodes: vec![c, b, a],
            ..LayerGraph::default()
        };
        graph.validate(&[]).expect("a line is not a loop");
    }

    /// §1.4: the layer's own source alpha is a Matte output, and an effect's
    /// matte input takes it.
    #[test]
    fn the_source_matte_feeds_an_effects_matte_input() {
        let blur = inst("blur");
        let graph = LayerGraph {
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

    /// Source and Out draw ports but hold no parameters, so naming one as a
    /// parameter destination names nothing.
    #[test]
    fn the_derived_nodes_have_no_parameters_to_drive() {
        let wiggle = inst("wiggle");
        for node in [NodeRef::Source, NodeRef::Out] {
            let graph = LayerGraph {
                edges: vec![param_edge(&wiggle, "value", node, "anything")],
                nodes: vec![wiggle.clone()],
                ..LayerGraph::default()
            };
            assert_eq!(graph.validate(&[]), Err(GraphError::UnknownNode));
        }
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
        let json = serde_json::to_string(&graph).expect("serialises");
        let back: LayerGraph = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, graph);
    }

    /// §4 and K-471's promise: an empty graph writes nothing at all, so a layer
    /// that never opened the Graph panel carries no `graph` key — which is what
    /// makes an untouched document re-save byte for byte.
    #[test]
    fn an_empty_graph_is_absent_from_the_file() {
        assert!(LayerGraph::default().is_empty());
        assert_eq!(
            serde_json::to_string(&LayerGraph::default()).expect("serialises"),
            "{}"
        );
    }
}

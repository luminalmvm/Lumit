//! The layer driver graph as the Graph panel reads and writes it (K-471,
//! K-472, docs/impl/node-graph.md §5).
//!
//! # In plain terms
//!
//! A layer's effects are a list, and that list is still the only thing that
//! makes the picture. The Graph panel draws that list as boxes joined by wires,
//! and adds a second kind of box — a **driver**, which makes a value rather
//! than a picture. This module is the doorway between the two halves of the
//! application for all of that.
//!
//! Two shapes cross, and the split between them is the whole design:
//!
//! - [`BridgeGraphNode`] is **derived**. The Source box, one box per effect in
//!   stack order, the Layer out box and one box per driver, each with the
//!   sockets it draws. Nothing here is stored anywhere; it is worked out from
//!   the layer every time it is asked for, which is why it is read whole in one
//!   call and never written back.
//! - [`BridgeGraphWiring`] is **stored**. The wires, where the boxes sit, and
//!   which boxes are grown to show every socket. This is the half the user
//!   edits, and it is handed back to `LayerReference::set_graph` exactly as it
//!   came — one gesture, one `SetLayerGraph`, one undo step.
//!
//! **One call, not one per node.** `LayerReference::get_graph` answers the
//! whole structure at once. A panel fetches it when the selection or the
//! document changes and holds it; asking per node per rebuild is the traffic
//! the budget test forbids (K-183).
//!
//! **No colour crosses.** A port carries its *type*
//! ([`BridgePortType`]) and the frontend maps that to a `port.*` theme token
//! (K-472 §6.1). The engine has never known what a colour is.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::graph::{self, Edge, InputRef, LayerGraph, NodeRef, OutputRef};
use lumit_core::model::Layer;

/// What a socket carries, and so what colour the frontend draws it
/// (K-472 §6.1). Seven types, five colours: image with matte, number, colour,
/// shape with points, audio.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePortType {
    Image,
    Matte,
    Number,
    Colour,
    Shape,
    Points,
    Audio,
}

impl BridgePortType {
    #[frb(ignore)]
    fn of(ty: lumit_core::fx::PortType) -> BridgePortType {
        use lumit_core::fx::PortType;
        match ty {
            PortType::Image => BridgePortType::Image,
            PortType::Matte => BridgePortType::Matte,
            PortType::Number => BridgePortType::Number,
            PortType::Colour => BridgePortType::Colour,
            PortType::Shape => BridgePortType::Shape,
            PortType::Points => BridgePortType::Points,
            PortType::Audio => BridgePortType::Audio,
        }
    }
}

/// Names one box on the canvas.
///
/// `Source`, `Effect` and `Out` are **derived** from the effect stack and carry
/// no storage of their own — an effect box is named by the id of its instance
/// in the layer's stack. A `Driver` is named by the id of its instance in the
/// graph's own node list.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeNodeRef {
    Source,
    Effect(Uuid),
    Driver(Uuid),
    Out,
}

impl BridgeNodeRef {
    #[frb(ignore)]
    fn of(node: NodeRef) -> BridgeNodeRef {
        match node {
            NodeRef::Source => BridgeNodeRef::Source,
            NodeRef::Effect(id) => BridgeNodeRef::Effect(id),
            NodeRef::Driver(id) => BridgeNodeRef::Driver(id),
            NodeRef::Out => BridgeNodeRef::Out,
        }
    }

    #[frb(ignore)]
    pub(crate) fn core(self) -> NodeRef {
        match self {
            BridgeNodeRef::Source => NodeRef::Source,
            BridgeNodeRef::Effect(id) => NodeRef::Effect(id),
            BridgeNodeRef::Driver(id) => NodeRef::Driver(id),
            BridgeNodeRef::Out => NodeRef::Out,
        }
    }
}

/// One socket on a box: the id the document writes down, the English word drawn
/// beside it (K-303), what it carries, and whether a wire is on it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePort {
    pub id: String,
    pub label: String,
    pub port_type: BridgePortType,
    /// Whether a wire lands on (an input) or leaves from (an output) this
    /// socket. The image chain's own sockets are wired by construction — the
    /// picture always flows straight down the stack.
    pub wired: bool,
}

impl BridgePort {
    #[frb(ignore)]
    fn of(port: lumit_core::fx::Port, wired: bool) -> BridgePort {
        BridgePort {
            id: port.id.to_owned(),
            label: port.label.to_owned(),
            port_type: BridgePortType::of(port.ty),
            wired,
        }
    }
}

/// One box on the canvas, as it is drawn.
///
/// Derived, not stored: the engine works this out from the layer each time it
/// is asked, so there is nothing here to write back. What the user *edits*
/// lives in [`BridgeGraphWiring`], and a driver's parameters ride the ordinary
/// staged-instance path (`LayerReference::get_graph_drivers`).
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeGraphNode {
    pub node: BridgeNodeRef,
    /// The effect's schema key, empty for the two derived boxes. Not display
    /// text — `label` is.
    pub match_name: String,
    /// What the box is called, in English (K-303).
    pub label: String,
    /// The user's own name for this instance (K-321), shown in place of
    /// `label`. Always `None` for the derived boxes.
    pub custom_name: Option<String>,
    /// False draws the border dashed — the `B` badge. Always true for the
    /// derived boxes, which cannot be bypassed.
    pub enabled: bool,
    pub inputs: Vec<BridgePort>,
    pub outputs: Vec<BridgePort>,
}

/// Where a wire comes from.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeOutputRef {
    /// A driver's declared output port.
    Driver { node: Uuid, port: String },
    /// The layer's own masked source alpha at that point in the chain (§1.4).
    SourceMatte,
    /// A **stack effect's** declared data output — Particulate's Points stream
    /// (K-492). The effect goes on making its picture for the chain; this taps
    /// the data it declares beside it, and carries no picture itself.
    EffectData { effect: Uuid, port: String },
}

/// Where a wire goes.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeInputRef {
    /// A parameter socket: the parameter follows the wire instead of its own
    /// keyframes. At most one wire per socket.
    Param { node: BridgeNodeRef, port: String },
    /// An effect's matte input.
    Matte { effect: Uuid },
}

/// One wire.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGraphEdge {
    pub from: BridgeOutputRef,
    pub to: BridgeInputRef,
}

impl BridgeGraphEdge {
    #[frb(ignore)]
    fn of(edge: &Edge) -> BridgeGraphEdge {
        BridgeGraphEdge {
            from: match &edge.from {
                OutputRef::Driver { node, port } => BridgeOutputRef::Driver {
                    node: *node,
                    port: port.clone(),
                },
                OutputRef::SourceMatte => BridgeOutputRef::SourceMatte,
                OutputRef::EffectData { effect, port } => BridgeOutputRef::EffectData {
                    effect: *effect,
                    port: port.clone(),
                },
            },
            to: match &edge.to {
                InputRef::Param { node, port } => BridgeInputRef::Param {
                    node: BridgeNodeRef::of(*node),
                    port: port.clone(),
                },
                InputRef::Matte { effect } => BridgeInputRef::Matte { effect: *effect },
            },
        }
    }

    #[frb(ignore)]
    fn core(self) -> Edge {
        Edge {
            from: match self.from {
                BridgeOutputRef::Driver { node, port } => OutputRef::Driver { node, port },
                BridgeOutputRef::SourceMatte => OutputRef::SourceMatte,
                BridgeOutputRef::EffectData { effect, port } => {
                    OutputRef::EffectData { effect, port }
                }
            },
            to: match self.to {
                BridgeInputRef::Param { node, port } => InputRef::Param {
                    node: node.core(),
                    port,
                },
                BridgeInputRef::Matte { effect } => InputRef::Matte { effect },
            },
        }
    }
}

/// Where one box sits on the canvas, in canvas units.
///
/// Document data: positions persist and travel with the project, and are
/// committed by the same whole-graph write as the wires. A box with no entry is
/// auto-placed by the panel.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeNodePosition {
    pub node: BridgeNodeRef,
    pub x: f64,
    pub y: f64,
}

/// A named region of the canvas: a tinted wash behind a set of boxes (K-651).
///
/// **No rectangle crosses**, and no colour: the wash is worked out from where
/// the members are sitting, and `colour` is an index into the frontend's own
/// label palette taken modulo its length. The engine has never known what a
/// colour is, and it does not know what a node card is either.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeNodeGroup {
    pub name: String,
    pub colour: u32,
    pub members: Vec<BridgeNodeRef>,
}

/// The half of the graph the user edits: wires, positions, which boxes are
/// grown to show every socket, and the named regions behind them.
///
/// Read out of [`BridgeLayerGraph`], changed, and handed straight back to
/// `LayerReference::set_graph` — one gesture, one op, one undo step. There is
/// deliberately no per-wire call: adding a driver *and* auto-wiring it is one
/// write, which is what makes it one undo step (docs/impl/node-graph.md §3).
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeGraphWiring {
    pub edges: Vec<BridgeGraphEdge>,
    pub layout: Vec<BridgeNodePosition>,
    /// The boxes whose `E` badge is on. A wired socket is drawn whether its box
    /// is exposed or not.
    pub exposed: Vec<BridgeNodeRef>,
    /// The named regions (K-651).
    pub groups: Vec<BridgeNodeGroup>,
}

impl BridgeGraphWiring {
    #[frb(ignore)]
    fn of(g: &LayerGraph) -> BridgeGraphWiring {
        BridgeGraphWiring {
            edges: g.edges.iter().map(BridgeGraphEdge::of).collect(),
            layout: g
                .layout
                .iter()
                .map(|(node, [x, y])| BridgeNodePosition {
                    node: BridgeNodeRef::of(*node),
                    x: *x,
                    y: *y,
                })
                .collect(),
            exposed: g.exposed.iter().copied().map(BridgeNodeRef::of).collect(),
            groups: g
                .groups
                .iter()
                .map(|group| BridgeNodeGroup {
                    name: group.name.clone(),
                    colour: group.colour,
                    members: group
                        .members
                        .iter()
                        .copied()
                        .map(BridgeNodeRef::of)
                        .collect(),
                })
                .collect(),
        }
    }
}

/// A layer's whole graph, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLayerGraph {
    /// Every box, in draw order: the Source, one per effect **in stack order**,
    /// the Layer out, then the drivers in document order. Filtering this to the
    /// `Effect` boxes gives the effect stack exactly as the Effect controls
    /// panel lists it — the graph has no second opinion about the picture's
    /// path, because the stack *is* the path (§1.1).
    pub nodes: Vec<BridgeGraphNode>,
    pub wiring: BridgeGraphWiring,
}

/// Build one layer's whole graph — the body of `LayerReference::get_graph`.
#[frb(ignore)]
pub(crate) fn read_layer_graph(layer: &Layer) -> BridgeLayerGraph {
    let g = &layer.graph;
    // A socket is wired when some edge names it. Linear over a handful of
    // wires, in document order — no map iteration, so two machines report the
    // same graph.
    let input_wired = |to: InputRef| g.edges.iter().any(|e| e.to == to);
    let output_wired = |from: &OutputRef| g.edges.iter().any(|e| &e.from == from);
    let driver_out = |node: Uuid, port: &str| OutputRef::Driver {
        node,
        port: port.to_owned(),
    };
    let effect_out = |effect: Uuid, port: &str| OutputRef::EffectData {
        effect,
        port: port.to_owned(),
    };

    let mut nodes = Vec::with_capacity(layer.effects.len() + g.nodes.len() + 2);

    nodes.push(BridgeGraphNode {
        node: BridgeNodeRef::Source,
        match_name: String::new(),
        label: graph::SOURCE_LABEL.to_owned(),
        custom_name: None,
        enabled: true,
        inputs: Vec::new(),
        outputs: vec![
            BridgePort::of(graph::IMAGE_PORT, true),
            BridgePort::of(graph::MATTE_PORT, output_wired(&OutputRef::SourceMatte)),
        ],
    });

    for effect in &layer.effects {
        let def = lumit_core::fx::BUILTIN_DEFS.get(&effect.effect.match_name);
        let schema = def.map(|d| d.schema());
        let mut inputs = vec![BridgePort::of(graph::INPUT_PORT, true)];
        // The matte socket exists only where the effect declares a matte row
        // (K-395) — the same question `LayerGraph::validate` asks before it
        // accepts a wire onto one.
        if schema.is_some_and(|s| s.matte.param().is_some()) {
            inputs.push(BridgePort::of(
                graph::MATTE_PORT,
                input_wired(InputRef::Matte { effect: effect.id }),
            ));
        }
        inputs.extend(param_ports(
            schema,
            NodeRef::Effect(effect.id),
            &input_wired,
        ));
        // The signature's **data inputs** last (K-492, K-600): wire-only, with
        // no stored value and no panel row, of which Clone to points' Points is
        // the first on a stack effect. Read from the same method a driver box
        // reads, so the socket that lets a consumer be wired and the one the
        // validator accepts a wire onto cannot disagree — and so an unwired one
        // reaches the canvas as the "no stream" mark (K-509) with no
        // effect-specific code at the seam.
        inputs.extend(
            def.map(|d| d.signature().inputs())
                .unwrap_or_default()
                .iter()
                .map(|port| {
                    BridgePort::of(
                        *port,
                        input_wired(InputRef::Param {
                            node: NodeRef::Effect(effect.id),
                            port: port.id.to_owned(),
                        }),
                    )
                }),
        );
        nodes.push(BridgeGraphNode {
            node: BridgeNodeRef::Effect(effect.id),
            match_name: effect.effect.match_name.clone(),
            // An effect this build does not know (a placeholder, an OFX that is
            // not installed) draws under its own key rather than under nothing.
            label: schema.map_or_else(|| effect.effect.match_name.clone(), |s| s.label.to_owned()),
            custom_name: effect.custom_name.clone(),
            enabled: effect.enabled,
            inputs,
            // The picture, always — plus whatever **data** outputs the effect's
            // signature declares beside it (K-492). That second half is how
            // Particulate's teal Points socket reaches the canvas with no
            // Particulate-specific code at the seam: it declares the port, and
            // every effect that declares none is untouched.
            outputs: std::iter::once(BridgePort::of(graph::OUTPUT_PORT, true))
                .chain(
                    def.map(|d| d.signature().outputs())
                        .unwrap_or_default()
                        .iter()
                        .map(|port| {
                            BridgePort::of(*port, output_wired(&effect_out(effect.id, port.id)))
                        }),
                )
                .collect(),
        });
    }

    nodes.push(BridgeGraphNode {
        node: BridgeNodeRef::Out,
        match_name: String::new(),
        label: graph::OUT_LABEL.to_owned(),
        custom_name: None,
        enabled: true,
        inputs: vec![
            BridgePort::of(graph::IMAGE_PORT, true),
            // Drawn, unfilled, honest: audio comes only from a footage layer's
            // own stream (K-435), so nothing may be wired here in this phase.
            BridgePort::of(graph::AUDIO_PORT, false),
        ],
        outputs: Vec::new(),
    });

    for driver in &g.nodes {
        let def = lumit_core::fx::BUILTIN_DEFS.get(&driver.effect.match_name);
        let schema = def.map(|d| d.schema());
        nodes.push(BridgeGraphNode {
            node: BridgeNodeRef::Driver(driver.id),
            match_name: driver.effect.match_name.clone(),
            label: schema.map_or_else(|| driver.effect.match_name.clone(), |s| s.label.to_owned()),
            custom_name: driver.custom_name.clone(),
            enabled: driver.enabled,
            // The parameter sockets, then the signature's **data inputs** — the
            // wire-only ones with no stored value and no panel row (K-492), of
            // which Points sample's Points is the first.
            inputs: param_ports(schema, NodeRef::Driver(driver.id), &input_wired)
                .chain(
                    def.map(|d| d.signature().inputs())
                        .unwrap_or_default()
                        .iter()
                        .map(|port| {
                            BridgePort::of(
                                *port,
                                input_wired(InputRef::Param {
                                    node: NodeRef::Driver(driver.id),
                                    port: port.id.to_owned(),
                                }),
                            )
                        }),
                )
                .collect(),
            outputs: def
                .map(|d| d.signature().outputs())
                .unwrap_or_default()
                .iter()
                .map(|port| BridgePort::of(*port, output_wired(&driver_out(driver.id, port.id))))
                .collect(),
        });
    }

    BridgeLayerGraph {
        nodes,
        wiring: BridgeGraphWiring::of(g),
    }
}

/// The sockets an entry of the catalogue *declares*, before any instance of it
/// exists: `(inputs, outputs)`.
///
/// The Graph panel needs these before the node is in the document — to fold an
/// auto-wire into the same commit as the add, and to filter the Tab search to
/// the entries a dragged wire could actually land on. Nothing is ever wired on
/// a catalogue entry, so the `wired` flag is `false` throughout and the node
/// reference `param_ports` needs is never consulted.
#[frb(ignore)]
pub(crate) fn catalogue_ports(match_name: &str) -> (Vec<BridgePort>, Vec<BridgePort>) {
    let Some(def) = lumit_core::fx::BUILTIN_DEFS.get(match_name) else {
        return (Vec::new(), Vec::new());
    };
    let inputs = param_ports(Some(def.schema()), NodeRef::Source, &|_| false)
        .chain(
            def.signature()
                .inputs()
                .iter()
                .map(|port| BridgePort::of(*port, false)),
        )
        .collect();
    let outputs = def
        .signature()
        .outputs()
        .iter()
        .map(|port| BridgePort::of(*port, false))
        .collect();
    (inputs, outputs)
}

/// One socket per parameter that can take a wire.
///
/// `ParamKind::port_type` is the authority (a switch, a dropdown or a file
/// picker has nothing a driver could hand it), so the sockets a node draws and
/// the wires `LayerGraph::validate` accepts cannot disagree.
#[frb(ignore)]
fn param_ports<'a>(
    schema: Option<&'static lumit_core::fx::EffectSchema>,
    node: NodeRef,
    wired: &'a impl Fn(InputRef) -> bool,
) -> impl Iterator<Item = BridgePort> + 'a {
    schema
        .map(|s| s.params)
        .unwrap_or_default()
        .iter()
        .filter_map(move |param| {
            let ty = param.kind.port_type()?;
            Some(BridgePort {
                id: param.id.to_owned(),
                label: param.label.to_owned(),
                port_type: BridgePortType::of(ty),
                wired: wired(InputRef::Param {
                    node,
                    port: param.id.to_owned(),
                }),
            })
        })
}

/// The document form of an edited wiring — the body of
/// `LayerReference::set_graph`, which pairs it with the staged driver nodes.
#[frb(ignore)]
pub(crate) fn wiring_into(
    wiring: BridgeGraphWiring,
    nodes: Vec<lumit_core::model::EffectInstance>,
) -> LayerGraph {
    LayerGraph {
        nodes,
        edges: wiring
            .edges
            .into_iter()
            .map(BridgeGraphEdge::core)
            .collect(),
        layout: wiring
            .layout
            .into_iter()
            .map(|p| (p.node.core(), [p.x, p.y]))
            .collect(),
        exposed: wiring
            .exposed
            .into_iter()
            .map(BridgeNodeRef::core)
            .collect(),
        groups: wiring
            .groups
            .into_iter()
            .map(|group| lumit_core::graph::NodeGroup {
                name: group.name,
                colour: group.colour,
                members: group.members.into_iter().map(BridgeNodeRef::core).collect(),
            })
            .collect(),
    }
}

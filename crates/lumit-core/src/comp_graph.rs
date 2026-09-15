//! The node graph composition: what a comp holds **instead of** a layer stack
//! ([impl/node-graph-comp.md](../../../docs/impl/node-graph-comp.md)).
//!
//! # In plain terms
//!
//! A composition normally stacks layers. A **node graph** is the other kind:
//! the same comp, with the same size, rate and duration, whose picture is made
//! by boxes joined with wires. Read boxes bring footage, solids and comps in.
//! Fx boxes are ordinary catalogue entries, effects and drivers alike. A Merge
//! lays one picture over another and a Switch picks one of several, so a
//! picture can fork and come back together, which a layer stack cannot say. An
//! Input box stands for a value or a picture handed in from outside, and the
//! one Output box is the picture the comp shows.
//!
//! The two kinds never mix: a comp has layers or it has a graph, and
//! [`crate::ops`] refuses every edit that would give it both.
//!
//! **None of the layer graph's types are reused** ([`crate::graph`]). That
//! graph's boxes are *derived* from an effect list and its image chain cannot
//! branch; sharing one edge type between a graph that may branch and one that
//! must not would give the type two meanings. What is shared is everything
//! below the type: the port types, the effect registry, [`EffectInstance`], the
//! driver walk and the resolve path.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fx::{FxCategory, ParamKind, Port, PortType, Unit};
use crate::graph::{GraphError, INPUT_PORT, MATTE_PORT, OUTPUT_PORT};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer, ProjectItem};

/// The socket a value Input hands its number or colour out of. The id and the
/// word every driver's own output already uses, so no new word is minted.
pub const VALUE_PORT: Port = Port::new("value", "Value", PortType::Number);

/// The Merge's second picture: what its A is laid over.
pub const BACKGROUND_PORT: Port = Port::new("background", "B", PortType::Image);

/// The match name of the Merge node (§1.3).
pub const MERGE: &str = "merge";
/// The match name of the Switch node (§1.3).
pub const SWITCH: &str = "switch";
/// The match name of the Node graph effect (§1.3).
pub const NODE_GRAPH: &str = "node_graph";
/// The match name of the Split channels node (§1.3).
pub const SPLIT_CHANNELS: &str = "split_channels";
/// The match name of the Combine channels node (§1.3).
pub const COMBINE_CHANNELS: &str = "combine_channels";

/// Split channels' outputs and Combine channels' inputs, red to alpha. Red
/// keeps the `output` and `input` ids, as Merge's A does, so Auto-wire and Heal
/// still find it.
pub const SPLIT_OUTPUTS: [&str; 4] = ["output", "green", "blue", "alpha"];
/// See [`SPLIT_OUTPUTS`].
pub const COMBINE_INPUTS: [&str; 4] = ["input", "green", "blue", "alpha"];
const CHANNEL_LABELS: [&str; 4] = ["Red", "Green", "Blue", "Alpha"];

/// What a node graph composition holds instead of layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompGraph {
    /// The boxes, in document order. Never empty: the Output is among them.
    pub nodes: Vec<GraphNode>,
    /// The wires (§1.4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<GraphEdge>,
    /// Canvas positions. **Document data**, as the layer graph's are: they
    /// persist and travel, and a box with no entry is auto-placed by the panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<(Uuid, [f64; 2])>,
    /// The boxes twirled open to show every socket. Presentation state, so it
    /// changes no picture and reaches no frame key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed: Vec<Uuid>,
    /// The named washes over regions of the canvas, beside [`Self::layout`] and
    /// for the same reasons.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GraphGroup>,
}

/// One box on a node graph's canvas.
///
/// Every box is **stored** and every id is a [`Uuid`], so this graph needs none
/// of the layer graph's derived sentinels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphNode {
    /// A project item brought in: footage, a solid or a composition, a node
    /// graph included.
    Read {
        id: Uuid,
        /// The project item this box reads. An item somebody deleted draws
        /// transparent and wears the missing mark, exactly as a Precomp layer
        /// of a deleted comp does.
        item: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_name: Option<String>,
    },
    /// A value or picture handed in from outside the graph (§1.5).
    Input { id: Uuid, input: GraphInput },
    /// Any catalogue entry: an image effect, a driver, a Merge, a Switch, a
    /// Node graph effect. An ordinary [`EffectInstance`], which is what gives a
    /// box keyframes, expressions, driver wires, a bypass tick and a name for
    /// nothing.
    Fx(EffectInstance),
    /// The one box whose picture the comp shows.
    Output { id: Uuid },
}

impl GraphNode {
    /// This box's id - the name every wire, position and group uses.
    #[must_use]
    pub fn id(&self) -> Uuid {
        match self {
            GraphNode::Read { id, .. } | GraphNode::Input { id, .. } | GraphNode::Output { id } => {
                *id
            }
            GraphNode::Fx(inst) => inst.id,
        }
    }

    /// The user's own name for this box, where they have given it one.
    #[must_use]
    pub fn custom_name(&self) -> Option<&str> {
        match self {
            GraphNode::Read { custom_name, .. } => custom_name.as_deref(),
            GraphNode::Fx(inst) => inst.custom_name.as_deref(),
            GraphNode::Input { .. } | GraphNode::Output { .. } => None,
        }
    }
}

/// One wire: an output socket of one box into an input socket of another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: Uuid,
    /// One of the source box's output sockets, as [`ports_of`] lists them.
    pub from_port: String,
    pub to: Uuid,
    /// One of the destination box's input sockets.
    pub to_port: String,
}

/// A named set of boxes drawn on one tinted wash.
///
/// No geometry is stored: the rectangle is worked out from the members' own
/// positions every time it is drawn, which is what keeps the wash following the
/// boxes rather than trapping them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphGroup {
    /// What the group is called, drawn as the wash's kicker.
    pub name: String,
    /// Which chip of the label palette tints it. An index, not a colour: no
    /// colour has ever crossed the bridge.
    pub colour: u32,
    /// The boxes inside it.
    pub members: Vec<Uuid>,
}

/// What an Input box stands for (§1.5) - the five facts the Custom shader's
/// Parameter node carries, with the same names, so one form edits both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphInput {
    /// snake_case, unique in the graph, and the parameter id outside it.
    pub id: String,
    /// The row's word.
    pub label: String,
    pub kind: InputKind,
    /// One number, or four for a colour.
    pub default: [f64; 4],
    pub min: f64,
    pub max: f64,
    pub unit: Unit,
    /// A project item to stand in for the picture when nothing feeds this
    /// Input (§5.11): viewed on its own or placed as a Precomp layer, never
    /// applied to a layer or nested, where the picture arrives on a socket.
    ///
    /// A value Input carries none, and a graph written before it existed opens
    /// and re-saves byte for byte without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Uuid>,
}

/// What an Input carries. A Point is two Number inputs; there is no Checkbox,
/// because a switch has no socket to feed and nothing inside the graph could
/// read one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputKind {
    Picture,
    Number,
    Angle,
    Colour,
}

impl InputKind {
    /// The type this Input's output socket carries, or `None` for a picture,
    /// whose socket is an image rather than a value.
    #[must_use]
    const fn value_type(self) -> Option<PortType> {
        match self {
            InputKind::Picture => None,
            InputKind::Number | InputKind::Angle => Some(PortType::Number),
            InputKind::Colour => Some(PortType::Colour),
        }
    }
}

/// One socket on a node graph's canvas.
///
/// [`Port`]'s shape with owned ids, because a node graph's socket names can
/// only be worked out at run time: a layer-reference row's socket is named by
/// the row's id and a Switch's by its index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphPort {
    pub id: String,
    /// The word drawn beside the plug, in British English. Empty where the
    /// canvas draws the socket's own meaning instead, as it does a Switch's
    /// index.
    pub label: String,
    pub ty: PortType,
}

impl GraphPort {
    fn of(port: Port) -> GraphPort {
        GraphPort {
            id: port.id.to_owned(),
            label: port.label.to_owned(),
            ty: port.ty,
        }
    }

    fn new(id: impl Into<String>, label: impl Into<String>, ty: PortType) -> GraphPort {
        GraphPort {
            id: id.into(),
            label: label.into(),
            ty,
        }
    }
}

/// The value half of a graph, turned into shapes the engine already evaluates
/// (§2.2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Projection {
    /// The Fx boxes that make pictures, in document order, with every wired
    /// Input baked into the parameter it feeds.
    pub effects: Vec<EffectInstance>,
    /// The driver boxes and the wires out of them, as the layer graph the
    /// existing driver walk resolves.
    pub drivers: crate::graph::LayerGraph,
}

/// Whether this catalogue entry makes a value rather than a picture.
///
/// The declared category, which is how the bridge already tells one apart: a
/// driver declares [`FxCategory::Drivers`] and a data signature together.
#[must_use]
fn is_driver(inst: &EffectInstance) -> bool {
    crate::fx::def(&inst.effect.match_name)
        .is_some_and(|def| def.schema().category == FxCategory::Drivers)
}

/// Whether this catalogue entry is offered in a node graph's console (§5.1).
///
/// Layer points is the one that is not: it taps another *layer's* first
/// producer, and a graph has no layers, so the box could only ever hand out the
/// empty stream. Inside a graph the wire is the tap.
#[must_use]
pub fn offered_in_graph(match_name: &str) -> bool {
    match_name != crate::fx::drivers::layer_points::MATCH_NAME
}

/// The sockets `node` draws, as inputs and outputs (§1.4).
///
/// **One function, one table.** The validator, the bridge's read model, the
/// label walk and the walk that lowers a graph to draws all read this, so they
/// cannot disagree about what a box has.
///
/// `graph` is the graph the box sits in, which a Switch needs: its spare socket
/// is one beyond the last one wired. `doc` is the project, which a Node graph
/// box needs to find the Inputs of the comp it names; with `None` it shows its
/// picture and matte sockets alone.
#[must_use]
pub fn ports_of(
    graph: &CompGraph,
    node: &GraphNode,
    doc: Option<&Document>,
) -> (Vec<GraphPort>, Vec<GraphPort>) {
    match node {
        // A Read is a source: it brings a picture in and takes nothing.
        GraphNode::Read { .. } => (Vec::new(), vec![GraphPort::of(OUTPUT_PORT)]),
        GraphNode::Input { input, .. } => {
            let out = match input.kind.value_type() {
                None => GraphPort::of(OUTPUT_PORT),
                Some(ty) => GraphPort::new(VALUE_PORT.id, VALUE_PORT.label, ty),
            };
            (Vec::new(), vec![out])
        }
        GraphNode::Output { .. } => (vec![GraphPort::of(INPUT_PORT)], Vec::new()),
        GraphNode::Fx(inst) => fx_ports(graph, inst, doc),
    }
}

/// [`ports_of`] for an Fx box: the three node-graph natives first, then the
/// ordinary rule an effect or a driver follows.
fn fx_ports(
    graph: &CompGraph,
    inst: &EffectInstance,
    doc: Option<&Document>,
) -> (Vec<GraphPort>, Vec<GraphPort>) {
    let out = vec![GraphPort::of(OUTPUT_PORT)];
    match inst.effect.match_name.as_str() {
        MERGE => {
            let ins = vec![
                GraphPort::new(INPUT_PORT.id, "A", PortType::Image),
                GraphPort::of(BACKGROUND_PORT),
                GraphPort::new("opacity", "Opacity", PortType::Number),
            ];
            return (ins, out);
        }
        SWITCH => {
            let mut ins: Vec<GraphPort> = (0..=spare_switch_socket(graph, inst.id))
                .map(|i| GraphPort::new(format!("in{i}"), "", PortType::Image))
                .collect();
            ins.push(GraphPort::new("index", "Index", PortType::Number));
            return (ins, out);
        }
        SPLIT_CHANNELS | COMBINE_CHANNELS => {
            let channels = |ids: [&str; 4]| {
                ids.into_iter()
                    .zip(CHANNEL_LABELS)
                    .map(|(id, label)| GraphPort::new(id, label, PortType::Image))
                    .collect()
            };
            return if inst.effect.match_name == SPLIT_CHANNELS {
                (vec![GraphPort::of(INPUT_PORT)], channels(SPLIT_OUTPUTS))
            } else {
                (channels(COMBINE_INPUTS), out)
            };
        }
        NODE_GRAPH => {
            let mut ins = vec![GraphPort::of(INPUT_PORT)];
            let inner = doc
                .zip(crate::fx::effects::node_graph::comp_of(inst))
                .and_then(|(d, c)| d.comp(c))
                .and_then(|c| c.graph.as_ref());
            if let Some(inner) = inner {
                // The first picture Input is the box's own `input`; every
                // further one is a socket of its own, named by the Input's id.
                let mut seen_picture = false;
                for input in inner.inputs() {
                    match input.kind.value_type() {
                        None if seen_picture => {
                            ins.push(GraphPort::new(
                                input.id.clone(),
                                input.label.clone(),
                                PortType::Image,
                            ));
                        }
                        None => seen_picture = true,
                        Some(ty) => {
                            ins.push(GraphPort::new(input.id.clone(), input.label.clone(), ty));
                        }
                    }
                }
            }
            // No matte socket in v1: the walk binds `input` and the picture
            // Inputs, so a wire into a matte here would change no pixel. The
            // Node graph *effect* on a layer keeps its Matte row, which the
            // stack's matte dissolve honours.
            return (ins, out);
        }
        _ => {}
    }

    let Some(def) = crate::fx::def(&inst.effect.match_name) else {
        // An entry this build does not know draws no sockets, so no wire can
        // name one - the same answer a missing box gets.
        return (Vec::new(), Vec::new());
    };
    let schema = def.schema();
    let signature = def.signature();
    let driver = schema.category == FxCategory::Drivers;

    let mut ins = Vec::new();
    if !driver {
        ins.push(GraphPort::of(INPUT_PORT));
        if schema.matte.param().is_some() {
            ins.push(GraphPort::of(MATTE_PORT));
        }
    }
    // **A layer-reference row is an image socket here.** On a layer that row is
    // a dropdown of the comp's layers; a node graph has no layers, so the wire
    // is the reference. Light wrap's Background and Texturize's Texture all
    // arrive this way, with nothing per effect at the seam.
    if !driver {
        for param in schema.params {
            let is_matte = schema.matte.param() == Some(param.id);
            if matches!(param.kind, ParamKind::Layer { .. }) && !is_matte {
                ins.push(GraphPort::new(param.id, param.label, PortType::Image));
            }
        }
    }
    for param in schema.params {
        if let Some(ty) = param.kind.port_type() {
            ins.push(GraphPort::new(param.id, param.label, ty));
        }
    }
    // **Every socket a signature declares is drawn** (§5.1): a points wire is
    // carried through the projection into the driver walk, so there is nothing
    // left for a type skip to protect.
    for port in signature.inputs() {
        ins.push(GraphPort::of(*port));
    }

    let mut outs = if driver { Vec::new() } else { out };
    for port in signature.outputs() {
        outs.push(GraphPort::of(*port));
    }
    (ins, outs)
}

/// As many pictures as one Switch may choose between.
///
/// A ceiling rather than a design: the index comes out of the document, and a
/// hand-edited `in4000000000` would otherwise ask for four billion sockets.
/// Nothing a hand builds comes near it, and a wire past it is refused as a
/// socket that is not there.
const MAX_SWITCH_SOCKETS: u32 = 64;

/// The index of a Switch's **spare** socket: one beyond the highest `in<n>`
/// anything is wired to, and zero on a Switch nobody has wired.
///
/// The canvas always draws one empty socket at the bottom, which is what makes
/// a Switch grow by being used rather than by a count somebody has to set.
fn spare_switch_socket(graph: &CompGraph, node: Uuid) -> u32 {
    graph
        .edges
        .iter()
        .filter(|e| e.to == node)
        .filter_map(|e| e.to_port.strip_prefix("in")?.parse::<u32>().ok())
        .max()
        .map_or(0, |highest| highest.saturating_add(1))
        .min(MAX_SWITCH_SOCKETS)
}

impl CompGraph {
    /// A fresh node graph: one Output box at the canvas origin, which is what
    /// **New node graph** seeds.
    #[must_use]
    pub fn new_with_output() -> CompGraph {
        let id = Uuid::now_v7();
        CompGraph {
            nodes: vec![GraphNode::Output { id }],
            edges: Vec::new(),
            layout: vec![(id, [0.0, 0.0])],
            exposed: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// The box named by `id`, if this graph carries one.
    #[must_use]
    pub fn node(&self, id: Uuid) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id() == id)
    }

    /// The one Output box's id, or `None` for a graph that has none - a state
    /// [`Self::validate`] refuses, and one a hand-edited file can still be in.
    #[must_use]
    pub fn output_id(&self) -> Option<Uuid> {
        self.nodes.iter().find_map(|n| match n {
            GraphNode::Output { id } => Some(*id),
            _ => None,
        })
    }

    /// This graph's Input boxes, in document order - which is the row order
    /// outside it (§1.5).
    pub fn inputs(&self) -> impl Iterator<Item = &GraphInput> {
        self.nodes.iter().filter_map(|n| match n {
            GraphNode::Input { input, .. } => Some(input),
            _ => None,
        })
    }

    /// What feeds `port` on `node`: the box the wire comes from and its output
    /// socket, or `None` for a socket nothing is wired to.
    #[must_use]
    pub fn wire_into(&self, node: Uuid, port: &str) -> Option<(&Uuid, &str)> {
        self.edges
            .iter()
            .find(|e| e.to == node && e.to_port == port)
            .map(|e| (&e.from, e.from_port.as_str()))
    }

    /// Whether any Read box brings project item `id` in - the node graph's twin
    /// of `layer_names_item`, so the Project panel's *in use* badge and the
    /// export's footage list cannot under-report a graph.
    #[must_use]
    pub fn read_names_item(&self, item: Uuid) -> bool {
        self.nodes
            .iter()
            .any(|n| matches!(n, GraphNode::Read { item: named, .. } if *named == item))
    }

    /// A copy with fresh ids for every box, its wires, positions, exposure and
    /// groups all re-pointed at them.
    ///
    /// What duplicating or pasting a node graph makes. A copy that kept its
    /// ids would alias its original's boxes, because a node graph's ids are
    /// resolved across the whole comp.
    #[must_use]
    pub fn fresh_copy(&self) -> CompGraph {
        let fresh: std::collections::BTreeMap<Uuid, Uuid> = self
            .nodes
            .iter()
            .map(|n| (n.id(), Uuid::now_v7()))
            .collect();
        let renamed = |id: &Uuid| fresh.get(id).copied().unwrap_or(*id);
        let nodes = self
            .nodes
            .iter()
            .cloned()
            .map(|node| match node {
                GraphNode::Read {
                    id,
                    item,
                    custom_name,
                } => GraphNode::Read {
                    id: renamed(&id),
                    item,
                    custom_name,
                },
                GraphNode::Input { id, input } => GraphNode::Input {
                    id: renamed(&id),
                    input,
                },
                GraphNode::Output { id } => GraphNode::Output { id: renamed(&id) },
                GraphNode::Fx(mut inst) => {
                    inst.id = renamed(&inst.id);
                    GraphNode::Fx(inst)
                }
            })
            .collect();
        CompGraph {
            nodes,
            edges: self
                .edges
                .iter()
                .map(|e| GraphEdge {
                    from: renamed(&e.from),
                    from_port: e.from_port.clone(),
                    to: renamed(&e.to),
                    to_port: e.to_port.clone(),
                })
                .collect(),
            layout: self
                .layout
                .iter()
                .map(|(id, at)| (renamed(id), *at))
                .collect(),
            exposed: self.exposed.iter().map(renamed).collect(),
            groups: self
                .groups
                .iter()
                .map(|g| GraphGroup {
                    name: g.name.clone(),
                    colour: g.colour,
                    members: g.members.iter().map(renamed).collect(),
                })
                .collect(),
        }
    }

    /// A copy whose Output shows `node`'s picture - the Viewer's *at this box*
    /// reading (§4.5).
    ///
    /// `None` when there is nothing to show differently: `node` is the Output
    /// itself, makes no picture, or already feeds the Output. The Viewer then
    /// rides the frame it has, which is the same one.
    #[must_use]
    pub fn viewed_at(&self, node: Uuid) -> Option<CompGraph> {
        let output = self.output_id()?;
        if node == output || !self.node(node).is_some_and(makes_a_picture) {
            return None;
        }
        if self.wire_into(output, INPUT_PORT.id) == Some((&node, OUTPUT_PORT.id)) {
            return None;
        }
        let mut copy = self.clone();
        copy.edges
            .retain(|e| !(e.to == output && e.to_port == INPUT_PORT.id));
        copy.edges.push(GraphEdge {
            from: node,
            from_port: OUTPUT_PORT.id.to_owned(),
            to: output,
            to_port: INPUT_PORT.id.to_owned(),
        });
        Some(copy)
    }

    /// Check every rule `SetCompGraph` enforces (§1.4).
    ///
    /// Each is an edit this application made rather than a state some other
    /// entity produced, so each is refused with a calm sentence rather than
    /// quietly rendering something different. A Read box whose item somebody
    /// deleted is the other case and degrades: it draws transparent and wears
    /// the missing mark.
    ///
    /// # Errors
    /// A wire naming a box or a socket that is not there, a wire joining two
    /// types, a second wire on one input, a loop anywhere, no Output box, or a
    /// second one.
    pub fn validate(&self, doc: Option<&Document>) -> Result<(), GraphError> {
        let mut outputs = self
            .nodes
            .iter()
            .filter(|n| matches!(n, GraphNode::Output { .. }));
        if outputs.next().is_none() {
            return Err(GraphError::NoOutput);
        }
        if outputs.next().is_some() {
            return Err(GraphError::SecondOutput);
        }
        for (i, edge) in self.edges.iter().enumerate() {
            // One wire per socket. Comparing against the wires *before* this
            // one reports the second, which is the one being added.
            if self.edges[..i]
                .iter()
                .any(|e| e.to == edge.to && e.to_port == edge.to_port)
            {
                return Err(GraphError::InputAlreadyWired);
            }
            let from = self.node(edge.from).ok_or(GraphError::UnknownNode)?;
            let to = self.node(edge.to).ok_or(GraphError::UnknownNode)?;
            let source = ports_of(self, from, doc)
                .1
                .into_iter()
                .find(|p| p.id == edge.from_port)
                .ok_or(GraphError::UnknownPort)?;
            let sink = ports_of(self, to, doc)
                .0
                .into_iter()
                .find(|p| p.id == edge.to_port)
                .ok_or(GraphError::UnknownPort)?;
            // **A matte socket takes a picture as well as a matte**: what the
            // row means by matte is the picture's own channel, chosen by its
            // Channel setting, and refusing a picture there would refuse the
            // thing people reach for first. Everywhere else a wire's type must
            // equal its socket's.
            let matched = source.ty == sink.ty
                || (sink.ty == PortType::Matte && source.ty == PortType::Image);
            if !matched {
                return Err(GraphError::PortTypeMismatch);
            }
        }
        self.check_acyclic()
    }

    /// Refuse a loop anywhere in the graph, image wires and value wires alike.
    ///
    /// Kahn's walk in document order: the lowering pulls a box's inputs before
    /// the box, so a loop would never end. Vectors rather than maps throughout,
    /// so the answer is the same on every machine.
    fn check_acyclic(&self) -> Result<(), GraphError> {
        let all: Vec<Uuid> = self.nodes.iter().map(GraphNode::id).collect();
        let mut settled: Vec<Uuid> = Vec::with_capacity(all.len());
        loop {
            let ready: Vec<Uuid> = all
                .iter()
                .copied()
                .filter(|id| !settled.contains(id))
                .filter(|id| {
                    !self
                        .edges
                        .iter()
                        .any(|e| e.to == *id && !settled.contains(&e.from))
                })
                .collect();
            if ready.is_empty() {
                return if settled.len() == all.len() {
                    Ok(())
                } else {
                    Err(GraphError::Cycle)
                };
            }
            settled.extend(ready);
        }
    }

    /// Turn the value half of this graph into the shapes the engine already
    /// evaluates (§2.2).
    ///
    /// **Driver wires become a [`LayerGraph`](crate::graph::LayerGraph)**, so
    /// `resolve_drivers` answers for a node graph without a line of the driver
    /// walk changing, and a Wiggle into a Switch's Index works on the first
    /// day.
    ///
    /// **An Input's value is baked.** A wire from a value Input into a
    /// parameter means "this parameter is that value", which is what overriding
    /// keyframes means, so the projection writes the value into that parameter
    /// and drops the wire. `overrides` is what a host instance supplies, by
    /// Input id; an Input nobody overrides bakes its default.
    ///
    /// Image wires and matte wires are untouched: the render half owns them.
    #[must_use]
    pub fn project(&self, overrides: &[(String, EffectValue)]) -> Projection {
        let mut out = Projection::default();
        for node in &self.nodes {
            let GraphNode::Fx(inst) = node else { continue };
            let id = inst.id;
            let mut inst = inst.clone();
            for edge in self.edges.iter().filter(|e| e.to == id) {
                let Some(GraphNode::Input { input, .. }) = self.node(edge.from) else {
                    continue;
                };
                if let Some(value) = baked(input, overrides) {
                    set_param(&mut inst, &edge.to_port, value);
                }
            }
            if is_driver(&inst) {
                out.drivers.nodes.push(inst);
            } else {
                out.effects.push(inst);
            }
        }
        for edge in &self.edges {
            let Some(GraphNode::Fx(source)) = self.node(edge.from) else {
                continue;
            };
            let Some(GraphNode::Fx(target)) = self.node(edge.to) else {
                continue;
            };
            // A driver hands out a value; a picture effect hands out whatever
            // its signature declares beside its picture, which today is
            // Particulate's Points. Both are wires the driver walk reads, in
            // the two shapes a layer stores them in. An image wire belongs to
            // the render half and is left alone.
            let from = if is_driver(source) {
                crate::graph::OutputRef::Driver {
                    node: source.id,
                    port: edge.from_port.clone(),
                }
            } else if data_output(source, &edge.from_port) {
                crate::graph::OutputRef::EffectData {
                    effect: source.id,
                    port: edge.from_port.clone(),
                }
            } else {
                continue;
            };
            let node = if is_driver(target) {
                crate::graph::NodeRef::Driver(target.id)
            } else {
                crate::graph::NodeRef::Effect(target.id)
            };
            out.drivers.edges.push(crate::graph::Edge {
                from,
                to: crate::graph::InputRef::Param {
                    node,
                    port: edge.to_port.clone(),
                },
            });
        }
        out
    }

    /// The boxes `node`'s picture is made from, `node` among them, in the order
    /// the lowering steps in (§5.4).
    ///
    /// A backwards flood over every incoming wire, then Kahn's walk in document
    /// order: a box lands once everything wired into it has, so a step's inputs
    /// are always earlier than the step. Vectors throughout, never a map, so
    /// the answer is the same on every machine. A box inside a loop - refused
    /// at the edit, and reachable only through a hand-edited file - is simply
    /// left out.
    #[must_use]
    pub fn cone_of(&self, node: Uuid) -> Vec<Uuid> {
        let mut wanted: Vec<Uuid> = vec![node];
        let mut seen = 0usize;
        while seen < wanted.len() {
            let at = wanted[seen];
            seen += 1;
            for e in self.edges.iter().filter(|e| e.to == at) {
                if !wanted.contains(&e.from) {
                    wanted.push(e.from);
                }
            }
        }
        let mut order: Vec<Uuid> = Vec::with_capacity(wanted.len());
        loop {
            let ready: Vec<Uuid> = self
                .nodes
                .iter()
                .map(GraphNode::id)
                .filter(|id| wanted.contains(id) && !order.contains(id))
                .filter(|id| {
                    !self.edges.iter().any(|e| {
                        e.to == *id && wanted.contains(&e.from) && !order.contains(&e.from)
                    })
                })
                .collect();
            if ready.is_empty() {
                return order;
            }
            order.extend(ready);
        }
    }

    /// Every `(box, time)` the picture at `root` and time `t` is made from
    /// (§5.2), the root itself first.
    ///
    /// **One rule**: a box's input at another time is its input cone evaluated
    /// at that time. `input_times` answers the times one box asks its input at,
    /// its own time first; a box that is not an Fx box asks its sources at its
    /// own time. **The bound** is the layer path's `strip_temporal_inputs` rule
    /// stated once: at any time other than the graph's own `t`, only the first
    /// of those times is taken, so a shift always applies and a window never
    /// nests. Deduplicated on the exact bits, in first-seen order, so the
    /// lowering, the planner and the frame key cannot disagree.
    ///
    /// The frame `dt` a window is measured in is the caller's, held by the
    /// closure it hands in: [`crate::fx::input_times`] wants it and this walk
    /// has no other use for it.
    #[must_use]
    pub fn time_demands(
        &self,
        root: Uuid,
        t: f64,
        input_times: &dyn Fn(&EffectInstance, f64) -> Vec<f64>,
    ) -> Vec<(Uuid, f64)> {
        let mut out: Vec<(Uuid, f64)> = vec![(root, t)];
        let mut seen = 0usize;
        while seen < out.len() {
            let (node, tau) = out[seen];
            seen += 1;
            let mut times = match self.node(node) {
                Some(GraphNode::Fx(inst)) => input_times(inst, tau),
                _ => vec![tau],
            };
            if times.is_empty() {
                times.push(tau);
            }
            if tau.to_bits() != t.to_bits() {
                times.truncate(1);
            }
            for e in self.edges.iter().filter(|e| e.to == node) {
                for at in &times {
                    let demand = (e.from, *at);
                    if !out
                        .iter()
                        .any(|(n, a)| *n == demand.0 && a.to_bits() == at.to_bits())
                    {
                        out.push(demand);
                    }
                }
            }
        }
        out
    }
}

/// Whether `port` is a data output this effect's signature declares - the wire
/// a points producer hands its stream out of.
///
/// The image output is not one: every picture operation has one and no
/// signature repeats it.
#[must_use]
fn data_output(inst: &EffectInstance, port: &str) -> bool {
    crate::fx::def(&inst.effect.match_name)
        .is_some_and(|def| def.signature().output(port).is_some())
}

/// How a graph is being looked at (§5.3): the values a host hands its Inputs,
/// and whether the pictures those Inputs stand for are fed at all.
///
/// A graph viewed on its own or placed as a Precomp layer has no host to feed
/// its picture Inputs, which is what a preview item stands in for (§5.11); a
/// graph applied as an effect or nested in another does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphView<'a> {
    /// The host's values, by Input id, as `node_graph::overrides_of` reads them
    /// off the instance.
    pub values: &'a [(String, EffectValue)],
    /// Whether the picture Inputs are fed from outside.
    pub pictures_fed: bool,
}

impl GraphView<'_> {
    /// A graph looked at on its own: no values and no pictures.
    pub const DEFAULTS: GraphView<'static> = GraphView {
        values: &[],
        pictures_fed: false,
    };
}

/// Whether this box hands a picture out - which is what the Viewer's *at this
/// box* chip is offered for.
fn makes_a_picture(node: &GraphNode) -> bool {
    match node {
        GraphNode::Read { .. } => true,
        GraphNode::Input { input, .. } => input.kind == InputKind::Picture,
        GraphNode::Fx(inst) => {
            crate::fx::def(&inst.effect.match_name).is_some() && !is_driver(inst)
        }
        GraphNode::Output { .. } => false,
    }
}

/// The value a wired Input bakes into the parameter it feeds: the host's
/// override where there is one, and the Input's own default otherwise.
///
/// `None` for a picture Input, which carries a texture rather than a number and
/// bakes nothing.
fn baked(input: &GraphInput, overrides: &[(String, EffectValue)]) -> Option<EffectValue> {
    if let Some((_, value)) = overrides.iter().find(|(id, _)| *id == input.id) {
        return Some(value.clone());
    }
    let d = input.default;
    match input.kind {
        InputKind::Picture => None,
        InputKind::Number | InputKind::Angle => {
            Some(EffectValue::Float(crate::anim::Property::fixed(d[0])))
        }
        InputKind::Colour => Some(EffectValue::Colour(d.map(crate::anim::Property::fixed))),
    }
}

/// Write `value` into `inst`'s parameter `id`, adding the row when the instance
/// has none - a project saved before a parameter existed carries no entry for
/// it, and a baked value still has to land.
fn set_param(inst: &mut EffectInstance, id: &str, value: EffectValue) {
    match inst.params.iter_mut().find(|p| p.id == id) {
        Some(param) => param.value = value,
        None => inst.params.push(crate::model::EffectParam {
            id: id.to_owned(),
            value,
            extra: serde_json::Map::new(),
        }),
    }
}

/// The layer a Read box behaves like: `item` at default placement, spanning the
/// whole comp (§2.1).
///
/// **Nothing keeps it.** It is built where it is needed and dropped, which is
/// what lets the decode planner, the pixel fetch, the colour space tag and the
/// frame key treat a Read box as the layer it behaves like with no second code
/// path.
///
/// `None` for a folder, which is the one project item no layer can hold.
/// Footage has no size in the document, so it is centred at comp size, which is
/// what a fresh footage layer of unprobed media already gets.
#[must_use]
pub fn read_layer(id: Uuid, item: &ProjectItem, comp: &Composition) -> Option<Layer> {
    use crate::anim::Property;
    use crate::model::{LayerKind, TransformGroup};

    let comp_w = f64::from(comp.width);
    let comp_h = f64::from(comp.height);
    let (kind, nat_w, nat_h) = match item {
        ProjectItem::Footage(f) => (LayerKind::Footage { item: f.id }, comp_w, comp_h),
        ProjectItem::Solid(s) => (
            LayerKind::Solid { def: s.id },
            f64::from(s.width),
            f64::from(s.height),
        ),
        ProjectItem::Composition(c) => (
            LayerKind::Precomp { comp: c.id },
            f64::from(c.width),
            f64::from(c.height),
        ),
        ProjectItem::Folder(_) => return None,
    };
    Some(Layer {
        id,
        name: item.name().to_owned(),
        kind,
        in_point: crate::CompTime(crate::Rational::ZERO),
        out_point: crate::CompTime(comp.duration.0),
        start_offset: crate::CompTime(crate::Rational::ZERO),
        transform: TransformGroup {
            anchor_x: Property::fixed(nat_w * 0.5),
            anchor_y: Property::fixed(nat_h * 0.5),
            position_x: Property::fixed(comp_w * 0.5),
            position_y: Property::fixed(comp_h * 0.5),
            ..TransformGroup::default()
        },
        matte: None,
        parent: None,
        label: 0,
        markers: Vec::new(),
        volume_db: Property::zero(),
        pan: Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: crate::retime::Interpolation::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: crate::model::BlendMode::Normal,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        graph: crate::graph::LayerGraph::default(),
        switches: crate::model::Switches::default(),
        extra: serde_json::Map::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::instantiate;
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
            nodes,
            edges,
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        }
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
        }
    }

    // -- 1.4, the rules ------------------------------------------------------

    #[test]
    fn a_well_formed_graph_is_accepted() {
        let (graph, ..) = read_blur_output();
        graph.validate(None).expect("a picture into a picture");
    }

    #[test]
    fn a_wire_to_a_box_that_is_not_there_is_refused() {
        let (mut graph, source_id, ..) = read_blur_output();
        graph.nodes.retain(|n| n.id() != source_id);
        assert_eq!(graph.validate(None), Err(GraphError::UnknownNode));
    }

    #[test]
    fn a_wire_to_a_socket_that_is_not_there_is_refused() {
        let (mut graph, ..) = read_blur_output();
        graph.edges[0].to_port = "no_such_socket".into();
        assert_eq!(graph.validate(None), Err(GraphError::UnknownPort));

        let (mut graph, ..) = read_blur_output();
        graph.edges[0].from_port = "no_such_socket".into();
        assert_eq!(graph.validate(None), Err(GraphError::UnknownPort));
    }

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
    fn a_socket_cannot_take_a_second_wire() {
        let (mut graph, _, blur_id, out_id) = read_blur_output();
        graph.edges.push(GraphEdge {
            from: blur_id,
            from_port: OUTPUT_PORT.id.to_owned(),
            to: out_id,
            to_port: INPUT_PORT.id.to_owned(),
        });
        assert_eq!(graph.validate(None), Err(GraphError::InputAlreadyWired));
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

    /// A value loop is a loop too: the walk pulls a box's inputs before the
    /// box, whichever kind of wire brought them.
    #[test]
    fn a_loop_through_two_drivers_is_refused() {
        let wiggle = fx("wiggle");
        let math = fx("math");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let edges = vec![
            wire(&wiggle, "value", &math, "a"),
            wire(&math, "value", &wiggle, "amount"),
        ];
        let graph = built(vec![wiggle, math, out], edges);
        assert_eq!(graph.validate(None), Err(GraphError::Cycle));
    }

    #[test]
    fn a_node_graph_has_exactly_one_output() {
        let (mut graph, ..) = read_blur_output();
        let out_id = graph.output_id().expect("the Output");
        graph.nodes.retain(|n| n.id() != out_id);
        graph.edges.retain(|e| e.to != out_id);
        assert_eq!(graph.validate(None), Err(GraphError::NoOutput));

        let (mut graph, ..) = read_blur_output();
        graph.nodes.push(GraphNode::Output { id: Uuid::now_v7() });
        assert_eq!(graph.validate(None), Err(GraphError::SecondOutput));
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

    #[test]
    fn a_read_an_input_and_the_output_show_what_they_carry() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());

        let (ins, outs) = ports_of(&graph, &read(Uuid::now_v7()), None);
        assert!(ins.is_empty(), "a Read takes nothing");
        assert_eq!(ids(&outs), vec!["output"]);

        let picture = input("plate", InputKind::Picture, 0.0);
        let (ins, outs) = ports_of(&graph, &picture, None);
        assert!(ins.is_empty());
        assert_eq!(ids(&outs), vec!["output"]);
        assert_eq!(outs[0].ty, PortType::Image);

        let number = input("amount", InputKind::Number, 0.0);
        let (_, outs) = ports_of(&graph, &number, None);
        assert_eq!(ids(&outs), vec!["value"]);
        assert_eq!(outs[0].ty, PortType::Number);

        let colour = input("tint", InputKind::Colour, 0.0);
        let (_, outs) = ports_of(&graph, &colour, None);
        assert_eq!(outs[0].ty, PortType::Colour);

        let out = GraphNode::Output { id: Uuid::now_v7() };
        let (ins, outs) = ports_of(&graph, &out, None);
        assert_eq!(ids(&ins), vec!["input"]);
        assert!(outs.is_empty(), "the Output hands nothing on");
    }

    #[test]
    fn a_merge_shows_a_b_and_opacity() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());
        let (ins, outs) = ports_of(&graph, &fx("merge"), None);
        assert_eq!(ids(&ins), vec!["input", "background", "opacity"]);
        assert_eq!(ins[0].label, "A");
        assert_eq!(ins[1].label, "B");
        assert_eq!(ins[2].ty, PortType::Number);
        assert_eq!(ids(&outs), vec!["output"]);
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

    #[test]
    fn a_driver_shows_what_it_shows_on_a_layer() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());
        let (ins, outs) = ports_of(&graph, &fx("wiggle"), None);
        assert_eq!(ids(&ins), vec!["amount", "frequency"]);
        assert_eq!(ids(&outs), vec!["value"], "a driver makes no picture");
    }

    /// **Every socket a signature declares is drawn** (§5.1): a producer's
    /// Points output and a consumer's Points input, one apiece.
    #[test]
    fn a_points_socket_is_drawn() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());

        let (_, outs) = ports_of(&graph, &fx("particulate"), None);
        let points: Vec<&GraphPort> = outs.iter().filter(|p| p.ty == PortType::Points).collect();
        assert_eq!(points.len(), 1, "its declared Points output, once");
        assert_eq!(ids(&outs), vec!["output", "points"]);

        let (ins, _) = ports_of(&graph, &fx("clone_to_points"), None);
        let points: Vec<&GraphPort> = ins.iter().filter(|p| p.ty == PortType::Points).collect();
        assert_eq!(points.len(), 1, "its declared Points input, once");
        assert_eq!(points[0].id, "points");
    }

    /// Split channels hands out a picture per channel and Combine channels
    /// takes one per channel, red on the `output` and `input` ids.
    #[test]
    fn split_and_combine_show_a_socket_per_channel() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());
        let labels = |ports: &[GraphPort]| -> Vec<String> {
            ports.iter().map(|p| p.label.clone()).collect()
        };

        let (ins, outs) = ports_of(&graph, &fx(SPLIT_CHANNELS), None);
        assert_eq!(ids(&ins), vec!["input"]);
        assert_eq!(ids(&outs), vec!["output", "green", "blue", "alpha"]);
        assert_eq!(labels(&outs), vec!["Red", "Green", "Blue", "Alpha"]);
        assert!(outs.iter().all(|p| p.ty == PortType::Image));

        // Its four pickers are rows, never sockets.
        let combine = fx(COMBINE_CHANNELS);
        let GraphNode::Fx(inst) = &combine else {
            unreachable!()
        };
        let rows: Vec<&str> = inst.params.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(rows, ["red_from", "green_from", "blue_from", "alpha_from"]);
        let (ins, outs) = ports_of(&graph, &combine, None);
        assert_eq!(ids(&ins), vec!["input", "green", "blue", "alpha"]);
        assert_eq!(labels(&ins), vec!["Red", "Green", "Blue", "Alpha"]);
        assert!(ins.iter().all(|p| p.ty == PortType::Image));
        assert_eq!(ids(&outs), vec!["output"]);
    }

    /// Any Split output is an ordinary picture: it validates into a Combine's
    /// socket and into a matte socket as any picture does.
    #[test]
    fn a_split_output_wires_like_any_picture() {
        let source = read(Uuid::now_v7());
        let split = fx(SPLIT_CHANNELS);
        let combine = fx(COMBINE_CHANNELS);
        let blur = fx("blur");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let edges = vec![
            wire(&source, OUTPUT_PORT.id, &split, INPUT_PORT.id),
            wire(&split, "green", &combine, "green"),
            wire(&combine, OUTPUT_PORT.id, &blur, INPUT_PORT.id),
            wire(&split, "alpha", &blur, MATTE_PORT.id),
            wire(&blur, OUTPUT_PORT.id, &out, INPUT_PORT.id),
        ];
        let mut graph = built(vec![source, split, combine, blur, out], edges);
        graph.validate(None).expect("a channel is a picture");

        graph.edges[1].from_port = "luminance".into();
        assert_eq!(graph.validate(None), Err(GraphError::UnknownPort));
    }

    /// A Time offset takes a picture and a number and hands a picture on: the
    /// ordinary rule, with nothing per effect at the seam (§5.2).
    #[test]
    fn a_time_offset_shows_its_picture_and_its_number() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());
        let (ins, outs) = ports_of(&graph, &fx("time_offset"), None);
        assert_eq!(ids(&ins), vec!["input", "offset"]);
        assert_eq!(ins[0].ty, PortType::Image);
        assert_eq!(ins[1].ty, PortType::Number);
        assert_eq!(ids(&outs), vec!["output"]);
        assert_eq!(outs[0].ty, PortType::Image);
    }

    /// **Layer points has no home in a graph** (§5.1): a graph has no layers to
    /// tap, and the wire is the tap. Every other entry is offered.
    #[test]
    fn layer_points_is_not_offered_in_a_graph() {
        assert!(!offered_in_graph("layer_points"));
        assert!(offered_in_graph("points_sample"));
        assert!(offered_in_graph(MERGE));
    }

    /// A points wire is an ordinary wire: the types match, so it validates;
    /// they do not, so an image socket refuses it; and a loop through one is
    /// refused as any loop is.
    #[test]
    fn a_points_wire_validates_and_a_mistyped_or_looping_one_does_not() {
        let producer = fx("particulate");
        let consumer = fx("clone_to_points");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let nodes = vec![producer.clone(), consumer.clone(), out.clone()];
        let good = vec![
            wire(&producer, "points", &consumer, "points"),
            wire(&consumer, OUTPUT_PORT.id, &out, INPUT_PORT.id),
        ];
        assert_eq!(built(nodes.clone(), good.clone()).validate(None), Ok(()));

        let mistyped = vec![wire(&producer, "points", &consumer, INPUT_PORT.id)];
        assert_eq!(
            built(nodes.clone(), mistyped).validate(None),
            Err(GraphError::PortTypeMismatch),
            "a stream is not a picture"
        );

        // The consumer's picture back into the producer closes the loop the
        // points wire opened.
        let mut looped = good;
        looped.push(wire(&consumer, OUTPUT_PORT.id, &producer, INPUT_PORT.id));
        assert_eq!(
            built(nodes, looped).validate(None),
            Err(GraphError::Cycle),
            "a loop through a points wire is still a loop"
        );
    }

    /// A Node graph box's sockets are the Inputs of the comp it names: the
    /// first picture is its own `input`, every other Input a socket of its own.
    ///
    /// And no matte socket, because the walk has nothing to bind one to: a
    /// wire into it is refused rather than accepted and ignored.
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
        assert_eq!(ids(&ins), vec!["input", "second", "amount"]);
        assert_eq!(ins[1].ty, PortType::Image);
        assert_eq!(ins[2].ty, PortType::Number);
        assert_eq!(ids(&outs), vec!["output"]);

        // A wire into the matte it does not have is a wire to a socket that is
        // not there.
        let source = read(Uuid::now_v7());
        let mut matted = built(
            vec![
                source.clone(),
                node.clone(),
                GraphNode::Output { id: Uuid::now_v7() },
            ],
            vec![wire(&source, OUTPUT_PORT.id, &node, MATTE_PORT.id)],
        );
        assert_eq!(matted.validate(Some(&doc)), Err(GraphError::UnknownPort));
        matted.edges[0].to_port = INPUT_PORT.id.to_owned();
        matted
            .validate(Some(&doc))
            .expect("and the picture socket takes it");
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

    #[test]
    fn an_input_wired_into_a_driver_bakes_into_the_driver() {
        let amount = input("amount", InputKind::Number, 12.0);
        let wiggle = fx("wiggle");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let edges = vec![wire(&amount, "value", &wiggle, "amount")];
        let graph = built(vec![amount, wiggle, out], edges);

        let projected = graph.project(&[]);
        assert!(projected.effects.is_empty());
        match projected.drivers.nodes[0].param("amount") {
            Some(EffectValue::Float(v)) => assert_eq!(v.value_at(0.0), 12.0),
            other => panic!("amount is a number, not {other:?}"),
        }
    }

    /// A picture Input carries a texture, so there is nothing to bake and the
    /// image wiring is left exactly as it was.
    #[test]
    fn a_picture_input_bakes_nothing() {
        let plate = input("plate", InputKind::Picture, 0.0);
        let blur = fx("blur");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let before = match &blur {
            GraphNode::Fx(inst) => inst.params.clone(),
            _ => unreachable!(),
        };
        let edges = vec![wire(&plate, OUTPUT_PORT.id, &blur, INPUT_PORT.id)];
        let graph = built(vec![plate, blur, out], edges);

        let projected = graph.project(&[]);
        assert_eq!(projected.effects[0].params, before);
        assert!(projected.drivers.edges.is_empty());
    }

    /// A points wire comes out as the `EffectData` edge a layer stores one in,
    /// so [`crate::fx::effect_stream_in`] finds the producer through it (§5.1).
    #[test]
    fn a_points_wire_projects_as_an_effect_data_edge() {
        let producer = fx("particulate");
        let consumer = fx("clone_to_points");
        let sample = fx("points_sample");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let (producer_id, consumer_id, sample_id) = (producer.id(), consumer.id(), sample.id());
        let edges = vec![
            wire(&producer, "points", &consumer, "points"),
            wire(&producer, "points", &sample, "points"),
            // An image wire belongs to the render half and projects nothing.
            wire(&producer, OUTPUT_PORT.id, &consumer, INPUT_PORT.id),
        ];
        let graph = built(vec![producer, consumer, sample, out], edges);

        let data = |effect: Uuid| crate::graph::OutputRef::EffectData {
            effect,
            port: "points".into(),
        };
        assert_eq!(
            graph.project(&[]).drivers.edges,
            vec![
                crate::graph::Edge {
                    from: data(producer_id),
                    to: crate::graph::InputRef::Param {
                        node: crate::graph::NodeRef::Effect(consumer_id),
                        port: "points".into(),
                    },
                },
                crate::graph::Edge {
                    from: data(producer_id),
                    to: crate::graph::InputRef::Param {
                        node: crate::graph::NodeRef::Driver(sample_id),
                        port: "points".into(),
                    },
                },
            ]
        );
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
    }

    /// Every box asks its own time, and a fork asks the shared source once.
    #[test]
    fn a_fork_demands_its_shared_source_once() {
        let read = read(Uuid::now_v7());
        let left = fx("blur");
        let right = fx("glow");
        let merge = fx("merge");
        let out = GraphNode::Output { id: Uuid::now_v7() };
        let out_id = out.id();
        let read_id = read.id();
        let edges = vec![
            wire(&read, OUTPUT_PORT.id, &left, INPUT_PORT.id),
            wire(&read, OUTPUT_PORT.id, &right, INPUT_PORT.id),
            wire(&left, OUTPUT_PORT.id, &merge, INPUT_PORT.id),
            wire(&right, OUTPUT_PORT.id, &merge, BACKGROUND_PORT.id),
            wire(&merge, OUTPUT_PORT.id, &out, INPUT_PORT.id),
        ];
        let graph = built(vec![read, left, right, merge, out], edges);

        let demands = graph.time_demands(out_id, 2.0, &|inst, at| {
            crate::fx::input_times(inst, at, 0.04)
        });
        assert!(
            demands.iter().all(|(_, at)| *at == 2.0),
            "no box asks another time"
        );
        assert_eq!(
            demands.iter().filter(|(n, _)| *n == read_id).count(),
            1,
            "two demands of one Read at one time are one demand"
        );
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
            crate::fx::input_times(inst, at, 0.5)
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
        let layer = read_layer(node_id, &ProjectItem::Solid(solid.clone()), &host)
            .expect("a solid is a layer");
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

    /// The *in use* badge and the export's footage list both see a Read box.
    #[test]
    fn the_in_use_walks_see_a_read_box() {
        let plate = footage("plate");
        let inner_plate = footage("inner");
        let mut inner = comp();
        inner.layers.push({
            let mut l = read_layer(
                Uuid::now_v7(),
                &ProjectItem::Footage(inner_plate.clone()),
                &inner,
            )
            .expect("a layer");
            l.name = "inner".into();
            l
        });

        let mut graph_comp = comp();
        graph_comp.graph = Some(built(
            vec![
                read(plate.id),
                read(inner.id),
                GraphNode::Output { id: Uuid::now_v7() },
            ],
            Vec::new(),
        ));

        let mut doc = Document::new();
        doc.items.push(ProjectItem::Footage(plate.clone()));
        doc.items.push(ProjectItem::Footage(inner_plate.clone()));
        doc.items.push(ProjectItem::Composition(inner.clone()));
        doc.items.push(ProjectItem::Composition(graph_comp.clone()));

        assert!(doc.item_is_used(plate.id), "a Read box places the item");
        assert!(doc.item_is_used(inner.id), "and so does a Read of a comp");
        assert!(!doc.item_is_used(graph_comp.id), "nothing places the graph");

        let found = crate::model::comp_footage_items(&doc, &graph_comp);
        assert_eq!(
            found,
            vec![plate.id, inner_plate.id],
            "the export sees the graph's own footage and the comp's it reads"
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
        let mut host_layer = read_layer(Uuid::now_v7(), &ProjectItem::Solid(solid), &host)
            .expect("a solid is a layer");
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

    #[test]
    fn a_fresh_copy_mints_new_ids_and_repoints_everything() {
        let (mut graph, source_id, blur_id, out_id) = read_blur_output();
        graph.layout = vec![(source_id, [10.0, 20.0]), (blur_id, [30.0, 40.0])];
        graph.exposed = vec![blur_id];
        graph.groups = vec![GraphGroup {
            name: "The plate".into(),
            colour: 2,
            members: vec![source_id, blur_id],
        }];

        let copy = graph.fresh_copy();
        let old: Vec<Uuid> = vec![source_id, blur_id, out_id];
        for node in &copy.nodes {
            assert!(!old.contains(&node.id()), "every box is a fresh box");
        }
        copy.validate(None).expect("the wires found their boxes");
        let ids: Vec<Uuid> = copy.nodes.iter().map(GraphNode::id).collect();
        for edge in &copy.edges {
            assert!(ids.contains(&edge.from) && ids.contains(&edge.to));
        }
        assert!(copy.layout.iter().all(|(id, _)| ids.contains(id)));
        assert!(copy.exposed.iter().all(|id| ids.contains(id)));
        assert!(copy.groups[0].members.iter().all(|id| ids.contains(id)));
        assert_eq!(copy.groups[0].name, "The plate");
        assert_eq!(copy.layout.len(), 2);
    }

    // -- the file ------------------------------------------------------------

    /// The presentation lists are skipped while empty, so a graph nobody has
    /// arranged writes nothing for them.
    #[test]
    fn an_unarranged_graph_writes_no_layout_exposure_or_groups() {
        let graph = built(vec![GraphNode::Output { id: Uuid::now_v7() }], Vec::new());
        let json = serde_json::to_string(&graph).expect("it serialises");
        assert!(json.contains("\"nodes\""));
        for key in ["layout", "exposed", "groups", "edges"] {
            assert!(!json.contains(key), "an empty {key} must not be written");
        }
    }

    /// A picture Input's preview item is document state (§5.11): it survives a
    /// save and a load, and an Input without one writes no key at all, so every
    /// graph written before the field opens unchanged.
    #[test]
    fn a_picture_inputs_preview_item_round_trips_and_is_absent_when_there_is_none() {
        let mut plate = input("plate", InputKind::Picture, 0.0);
        let json = serde_json::to_string(&plate).expect("it serialises");
        assert!(!json.contains("preview"), "no item, nothing written");
        assert_eq!(
            serde_json::from_str::<GraphNode>(&json).expect("and reads back"),
            plate
        );

        let item = Uuid::now_v7();
        if let GraphNode::Input { input, .. } = &mut plate {
            input.preview = Some(item);
        }
        let json = serde_json::to_string(&plate).expect("it serialises");
        let back: GraphNode = serde_json::from_str(&json).expect("and reads back");
        assert_eq!(back, plate);
        match back {
            GraphNode::Input { input, .. } => assert_eq!(input.preview, Some(item)),
            other => panic!("an Input came back as {other:?}"),
        }
    }

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

    /// The project panel's "in use" badge: a node graph is placed by the
    /// effect that applies it as much as by a layer or a Read box, on a
    /// layer, on a group header and inside another graph, bypassed or not.
    #[test]
    fn a_graph_applied_as_an_effect_counts_as_in_use() {
        let mut graph_comp = comp();
        graph_comp.graph = Some(built(
            vec![GraphNode::Output { id: Uuid::now_v7() }],
            Vec::new(),
        ));
        let graph_id = graph_comp.id;
        let inner = graph_comp.graph.clone().expect("the graph just set");
        let bound = || {
            let mut inst = instantiate("node_graph").expect("the catalogue knows it");
            crate::fx::effects::node_graph::bind(&mut inst, graph_id, &inner);
            inst
        };
        let solid = SolidDef {
            id: Uuid::now_v7(),
            name: "grey".to_owned(),
            colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
            width: 32,
            height: 32,
            extra: serde_json::Map::new(),
        };

        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(graph_comp));
        assert!(!doc.item_is_used(graph_id), "nothing applies it yet");

        // On a layer, switched off: a bypassed effect still places the graph.
        let mut host = comp();
        let mut layer = read_layer(Uuid::now_v7(), &ProjectItem::Solid(solid), &host)
            .expect("a solid is a layer");
        let mut off = bound();
        off.enabled = false;
        layer.effects = vec![off];
        host.layers.push(layer);
        let host_id = host.id;
        doc.items.push(ProjectItem::Composition(host));
        assert!(doc.item_is_used(graph_id), "a layer's effect applies it");

        // On a group header.
        doc.items.retain(|i| i.id() != host_id);
        let mut grouped = comp();
        grouped.groups.push(crate::group::LayerGroup {
            id: Uuid::now_v7(),
            name: "band".to_owned(),
            label: 0,
            members: Vec::new(),
            effects: vec![bound()],
        });
        let grouped_id = grouped.id;
        doc.items.push(ProjectItem::Composition(grouped));
        assert!(doc.item_is_used(graph_id), "a header's effect applies it");

        // Inside another graph.
        doc.items.retain(|i| i.id() != grouped_id);
        let mut outer = comp();
        outer.graph = Some(built(
            vec![
                GraphNode::Fx(bound()),
                GraphNode::Output { id: Uuid::now_v7() },
            ],
            Vec::new(),
        ));
        doc.items.push(ProjectItem::Composition(outer));
        assert!(doc.item_is_used(graph_id), "a nested box applies it");
    }
}

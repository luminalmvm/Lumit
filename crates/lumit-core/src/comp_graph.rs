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

use crate::fx::{params::Unit, schema::ParamKind, schema::Port, schema::PortType};
use crate::graph::looks::BoxLooks;
use crate::graph::{INPUT_PORT, MATTE_PORT, OUTPUT_PORT};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer, ProjectItem};

mod project;
mod repeat;
#[cfg(test)]
mod tests;
mod validate;

/// The socket a value Input hands its number or colour out of. The id and the
/// word every driver's own output already uses, so no new word is minted.
pub const VALUE_PORT: Port = Port::new("value", "Value", PortType::Number);

/// A Read's second output: how far from its camera each pixel of a comp's
/// 3D layers is, metres (Scene depth).
pub const READ_DEPTH_PORT: Port = Port::new("depth", "Depth", PortType::Depth);

/// A Read's paths: the shape layers of the comp it reads, for a Repeat start
/// to run once a path.
pub const READ_SHAPES_PORT: Port = Port::new("shapes", "Shapes", PortType::Shape);

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
/// The match name of the Repeat start box.
pub const REPEAT_START: &str = "repeat_start";
/// The match name of the Repeat end box.
pub const REPEAT_END: &str = "repeat_end";

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
    /// The notes left on the canvas, as a layer's graph keeps them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<crate::graph::Note>,
    /// The ties on its wires, as a layer's graph keeps them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ties: Vec<crate::wire_tie::WireTie>,
    /// Each box's label colour and fold, as a layer's graph keeps them.
    #[serde(default, skip_serializing_if = "BoxLooks::is_empty")]
    pub looks: BoxLooks<Uuid>,
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
/// driver declares a driver category
/// ([`crate::fx::schema::FxCategory::is_driver`]) and a data
/// signature together.
#[must_use]
fn is_driver(inst: &EffectInstance) -> bool {
    crate::fx::builtins::def(&inst.effect.match_name)
        .is_some_and(|def| def.schema().category.is_driver())
}

/// Whether this catalogue entry is offered in a node graph's console (§5.1).
/// A graph has no layers, so never the three that read one.
#[must_use]
pub fn offered_in_graph(match_name: &str) -> bool {
    !matches!(
        match_name,
        crate::fx::drivers::layer_points::MATCH_NAME | "scene_pass" | "part_matte"
    )
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
        // A Read of a comp hands on its shape layers' paths too.
        GraphNode::Read { item, .. } => {
            let mut outs = vec![GraphPort::of(OUTPUT_PORT), GraphPort::of(READ_DEPTH_PORT)];
            if doc.is_some_and(|d| d.comp(*item).is_some()) {
                outs.push(GraphPort::of(READ_SHAPES_PORT));
            }
            (Vec::new(), outs)
        }
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
            // The Matte, as every effect has, dissolving back to `input` where
            // the matte is dark.
            if crate::fx::builtins::def(NODE_GRAPH)
                .is_some_and(|d| d.schema().matte.param().is_some())
            {
                ins.insert(1, GraphPort::of(MATTE_PORT));
            }
            return (ins, out);
        }
        _ => {}
    }

    let Some(def) = crate::fx::builtins::def(&inst.effect.match_name) else {
        // An entry this build does not know draws no sockets, so no wire can
        // name one - the same answer a missing box gets.
        return (Vec::new(), Vec::new());
    };
    let schema = def.schema();
    let signature = def.signature();
    let driver = schema.category.is_driver();

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
            if matches!(param.kind, ParamKind::Layer { .. })
                && !is_matte
                && !def.shape_inputs().contains(&param.id)
            {
                let ty = if def.depth_inputs().contains(&param.id) {
                    PortType::Depth
                } else {
                    PortType::Image
                };
                ins.push(GraphPort::new(param.id, param.label, ty));
            }
        }
    }
    // A Custom shader's own rows get sockets too.
    for param in schema.params.iter().chain(def.derived(inst)) {
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
    outs.extend(view_ports(inst));
    for port in signature.outputs() {
        outs.push(GraphPort::of(*port));
    }
    (ins, outs)
}

/// The view menu `inst`'s effect declares: its parameter id and its options.
fn view_menu(inst: &EffectInstance) -> Option<(&'static str, &'static [&'static str])> {
    let def = crate::fx::builtins::def(&inst.effect.match_name)?;
    let param = def.view()?;
    def.schema().params.iter().find_map(|p| match p.kind {
        ParamKind::Choice { options, .. } if p.id == param => Some((param, options)),
        _ => None,
    })
}

/// The output sockets a view menu gives a box: one picture per option, named
/// by the menu's parameter and the option's place in it, `display_2` for Depth
/// of field's Focus map.
///
/// They sit beside `output`, which still draws whatever the dropdown says, so a
/// graph saved before these existed reads as it did. Empty for every effect
/// that declares no view.
#[must_use]
pub fn view_ports(inst: &EffectInstance) -> Vec<GraphPort> {
    let Some((param, options)) = view_menu(inst) else {
        return Vec::new();
    };
    let types =
        crate::fx::builtins::def(&inst.effect.match_name).map_or(&[][..], |d| d.view_types());
    options
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let ty = types.get(i).copied().unwrap_or(PortType::Image);
            GraphPort::new(format!("{param}_{i}"), *label, ty)
        })
        .collect()
}

/// `inst` with its view menu held on the option `port` stands for, which is
/// the effect that socket draws. `None` for a socket that is not a view.
///
/// The held value is fixed, so keyframes on the dropdown move `output` alone.
#[must_use]
pub fn held_on_view(inst: &EffectInstance, port: &str) -> Option<EffectInstance> {
    let (param, options) = view_menu(inst)?;
    let option: u32 = port.strip_prefix(param)?.strip_prefix('_')?.parse().ok()?;
    if option as usize >= options.len() {
        return None;
    }
    let mut held = inst.clone();
    set_param(&mut held, param, EffectValue::Choice(option));
    Some(held)
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
    /// Rewire Depth of field pro boxes saved before their two depth sockets
    /// became one. The matte socket held a depth pass and `depth_m` held
    /// metres, which won when both were wired. Both now go into `depth`, and
    /// the matte socket is a mask, so the box is moved on to version 2 and a
    /// mask wired later is left alone.
    pub fn migrate_dof_pro_depth(&mut self) {
        for node in &mut self.nodes {
            let GraphNode::Fx(inst) = node else {
                continue;
            };
            if inst.effect.match_name != "dof_pro" || inst.effect.version >= 2 {
                continue;
            }
            inst.effect.version = 2;
            let id = inst.id;
            let metres = self
                .edges
                .iter()
                .any(|e| e.to == id && e.to_port == "depth_m");
            self.edges
                .retain(|e| !(metres && e.to == id && e.to_port == MATTE_PORT.id));
            for e in &mut self.edges {
                if e.to == id && (e.to_port == "depth_m" || e.to_port == MATTE_PORT.id) {
                    e.to_port = "depth".to_owned();
                }
            }
        }
    }

    /// Put back what an edited wiring doesn't carry, from `held`, the graph
    /// before the edit. A box the edit took away takes its looks with it.
    pub fn keep_canvas(&mut self, held: &CompGraph) {
        self.notes.clone_from(&held.notes);
        self.ties.clone_from(&held.ties);
        self.looks.clone_from(&held.looks);
        let nodes = &self.nodes;
        self.looks.retain(|id| nodes.iter().any(|n| n.id() == id));
    }

    /// A fresh node graph: one Output box at the canvas origin, which is what
    /// **New node graph** seeds.
    #[must_use]
    pub fn new_with_output() -> CompGraph {
        let id = Uuid::now_v7();
        CompGraph {
            notes: Vec::new(),
            ties: Vec::new(),
            looks: Default::default(),
            nodes: vec![GraphNode::Output { id }],
            edges: Vec::new(),
            // Down and to the right, where a chain ends, with the top-left
            // free for the boxes that feed it and the socket clear of the
            // panel's edge.
            layout: vec![(id, [560.0, 500.0])],
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

    /// The Read box `node`'s picture comes from, as `(read, item)`: its main
    /// picture socket followed upstream, box by box, to the first Read.
    ///
    /// # In plain terms
    ///
    /// Remove background, Depth and the Roto brush do not work on the picture
    /// they are handed. They read a **file**, frame by frame, and keep what
    /// they made of it beside that file. On a layer the file is the layer's
    /// own. A box has no layer, so its file is whichever Read is wired, through
    /// any number of boxes, into its picture socket.
    ///
    /// A Switch is followed down its first socket, the picture it hands on
    /// when it is bypassed. `None` where the chain ends at an Input, at
    /// nothing, or goes round.
    #[must_use]
    pub fn read_behind(&self, node: Uuid) -> Option<(Uuid, Uuid)> {
        let mut at = node;
        // One step per box at most, so a loop a hand-edited file smuggled in
        // stops rather than spins.
        for _ in 0..self.nodes.len() {
            let port = match self.node(at)? {
                GraphNode::Read { id, item, .. } => return Some((*id, *item)),
                GraphNode::Fx(inst) if inst.effect.match_name == SWITCH => "in0",
                GraphNode::Fx(_) => INPUT_PORT.id,
                GraphNode::Input { .. } | GraphNode::Output { .. } => return None,
            };
            at = *self.wire_into(at, port)?.0;
        }
        None
    }

    /// Every box that is switched on and reads a file through an analysis - a
    /// planes-tier effect or a Roto brush - with the Read it reads, as
    /// `(box, read, item)`. The graph's answer to walking a layer's effects
    /// for them.
    pub fn analysis_boxes(&self) -> impl Iterator<Item = (&EffectInstance, Uuid, Uuid)> + '_ {
        self.nodes.iter().filter_map(|n| {
            let GraphNode::Fx(inst) = n else { return None };
            let reads = inst.enabled
                && (crate::planes::task_of(inst).is_some()
                    || inst.effect.match_name == crate::roto::ROTO_BRUSH);
            let (read, item) = reads.then(|| self.read_behind(inst.id)).flatten()?;
            Some((inst, read, item))
        })
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
}

/// Whether `port` is a data output this effect's signature declares - the wire
/// a points producer hands its stream out of.
///
/// The image output is not one: every picture operation has one and no
/// signature repeats it.
#[must_use]
fn data_output(inst: &EffectInstance, port: &str) -> bool {
    crate::fx::builtins::def(&inst.effect.match_name)
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
            crate::fx::builtins::def(&inst.effect.match_name).is_some() && !is_driver(inst)
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
    let label = kind.default_label();
    Some(Layer {
        id,
        name: item.name().to_owned(),
        kind,
        in_point: crate::time::CompTime(crate::time::Rational::ZERO),
        out_point: crate::time::CompTime(comp.duration.0),
        start_offset: crate::time::CompTime(crate::time::Rational::ZERO),
        transform: TransformGroup {
            anchor_x: Property::fixed(nat_w * 0.5),
            anchor_y: Property::fixed(nat_h * 0.5),
            position_x: Property::fixed(comp_w * 0.5),
            position_y: Property::fixed(comp_h * 0.5),
            ..TransformGroup::default()
        },
        matte: None,
        parent: None,
        label,
        markers: Vec::new(),
        volume_db: Property::zero(),
        pan: Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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

/// One Repeat start, the Repeat end it pairs with, and the boxes between them:
/// every box downstream of the start that the end is downstream of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    pub start: Uuid,
    pub end: Uuid,
    /// In document order, nested pairs included.
    pub body: Vec<Uuid>,
}

/// The numbers time `index` of `count` of the pair starting at `start` reads,
/// with its point's or path's, by the Input ids [`CompGraph::repeat_graph`]
/// gave them.
#[must_use]
pub fn repeat_values(
    start: &EffectInstance,
    index: u32,
    count: u32,
    element: &crate::fx::effects::repeat::Element,
) -> Vec<(String, EffectValue)> {
    use crate::anim::Property;
    use crate::fx::effects::repeat::{RepeatStart, PASS_PORTS};
    let seed = match start.param("seed") {
        Some(EffectValue::Seed(s)) => *s,
        _ => 0,
    };
    let [i, n, u, r] = RepeatStart { seed }.pass(index, count);
    let numbers = [
        i,
        n,
        u,
        r,
        element.at[0],
        element.at[1],
        element.size,
        element.rotation,
    ];
    let float = |v: f32| EffectValue::Float(Property::fixed(f64::from(v)));
    PASS_PORTS
        .iter()
        .enumerate()
        .map(|(k, port)| {
            let value = match numbers.get(k) {
                Some(v) => float(*v),
                None => EffectValue::Colour(element.colour.map(|c| Property::fixed(f64::from(c)))),
            };
            (repeat::repeat_value_id(start.id, port.id), value)
        })
        .collect()
}

/// The box round each path of the shape layers in comp `item` at comp time
/// `t`, as a Read box of it lays them in a graph `size` across and down:
/// left, top, right, bottom. What a Read's Shapes socket hands on.
#[must_use]
pub fn read_paths(
    context: &crate::expression::ExpressionContext,
    item: Uuid,
    size: [f32; 2],
    t: f64,
) -> Vec<[f32; 4]> {
    use crate::model::LayerKind;
    let Some(comp) = context.document.comp(item) else {
        return Vec::new();
    };
    // A Read lays its comp centred in the graph's frame.
    let off = [
        (size[0] - comp.width as f32) * 0.5,
        (size[1] - comp.height as f32) * 0.5,
    ];
    let context = crate::expression::ExpressionContext {
        comp: Some(comp.id),
        layer: None,
        comp_time: t,
        inputs: None,
        ..context.clone()
    };
    let mut out = Vec::new();
    for layer in comp.layers.iter().rev() {
        let LayerKind::Shape { contents } = &layer.kind else {
            continue;
        };
        if !layer.switches.visible {
            continue;
        }
        let lt = crate::time::layer_time(t, layer.start_offset.0);
        let line = crate::shape::outline_at(contents, lt);
        let world = crate::fx::drivers::measure::world(comp, layer, t, &context);
        let mut bounds: Option<[f32; 4]> = None;
        for i in 0..line.points.len() {
            // A step with no length joins two paths of one run.
            if i > 0 && line.arc[i] <= line.arc[i - 1] {
                out.extend(bounds.take());
            }
            let p = line.points[i];
            let (x, y) = world.apply((f64::from(p[0]), f64::from(p[1])));
            let (x, y) = (x as f32 + off[0], y as f32 + off[1]);
            bounds = Some(bounds.map_or([x, y, x, y], |b| {
                [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
            }));
        }
        out.extend(bounds);
    }
    out
}

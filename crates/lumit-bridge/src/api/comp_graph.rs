//! The node graph composition as the Graph panel reads and writes it
//! (docs/impl/node-graph-comp.md §4.1).
//!
//! # In plain terms
//!
//! A node graph is a composition whose picture is made by boxes and wires
//! instead of a layer stack. This module is the doorway to it, and it is the
//! layer graph's shape ([`crate::api::graph`]) moved onto the comp, so the two
//! canvases read the same kinds of thing:
//!
//! - [`BridgeCompNode`] is **derived**. One box per node with the sockets it
//!   draws, worked out from the document every time it is asked for, so there
//!   is nothing here to write back.
//! - [`BridgeCompWiring`] is **stored**. The Read, Input and Output boxes as
//!   data, the wires, where the boxes sit, which are twirled open and how they
//!   are grouped. This is the half the user edits, and it is handed straight
//!   back to `CompositionReference::set_node_graph`: one gesture, one
//!   [`lumit_core::Op::SetCompGraph`], one undo step.
//!
//! The Fx boxes are not in the wiring. They are ordinary
//! [`EffectInstance`](lumit_core::model::EffectInstance)s and ride the staged
//! path every other instance rides (`get_node_graph_instances`), so a box's
//! parameters keyframe, express and take driver wires with no surface of their
//! own.
//!
//! **One call, not one per node**, and **no colour crosses**: both for the
//! reasons the layer graph gives.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use lumit_core::comp_graph::{
    self, CompGraph, GraphEdge, GraphGroup, GraphInput, GraphNode, GraphPort, InputKind,
};
use lumit_core::model::{Document, EffectInstance};

use crate::api::composition::CompositionReference;
use crate::api::effect::{bridge_unit, core_unit, BridgeUnit};
use crate::api::graph::{BridgePort, BridgePortType};
use crate::api::planes::BridgePlaneStatus;
use crate::api::project_item::{item_reference, ItemReference};
use crate::api::roto::BridgeRotoStatus;
use crate::api::BridgeError;

/// Which kind of box this is, and so what the canvas draws it as.
///
/// The four the model has: a project item brought in, a value or picture handed
/// in from outside, a catalogue entry, and the one box whose picture the comp
/// shows.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCompNodeKind {
    Read,
    Input,
    Fx,
    Output,
}

/// One box on the canvas, as it is drawn.
///
/// Derived, not stored: the engine works this out from the graph and the
/// project each time it is asked, so there is nothing here to write back. What
/// the user edits lives in [`BridgeCompWiring`], and an Fx box's parameters ride
/// the ordinary staged-instance path.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCompNode {
    pub id: Uuid,
    pub kind: BridgeCompNodeKind,
    /// The project item a Read box brings in, so the panel can open it or draw
    /// its kind. `None` on every other kind, and on a Read whose item somebody
    /// deleted.
    pub item: Option<ItemReference>,
    /// True for a Read box whose item is no longer in the project. It draws
    /// transparent and wears the missing mark, exactly as a Precomp layer of a
    /// deleted comp does.
    pub missing: bool,
    /// The effect's schema key, empty for every box that is not an Fx. Not
    /// display text: `label` is.
    pub match_name: String,
    /// What the box is called, in English: the item's name for a Read, the
    /// Input's own word, the effect's label for an Fx, and *Output* for the
    /// Output. Empty for a Read whose item has gone, there being no name left
    /// to draw.
    pub label: String,
    /// The user's own name for this box, shown in place of `label`.
    pub custom_name: Option<String>,
    /// False draws the border dashed, the bypass tick being off. Always true
    /// for the three boxes that cannot be bypassed.
    pub enabled: bool,
    pub inputs: Vec<BridgePort>,
    pub outputs: Vec<BridgePort>,
}

/// What an Input box stands for: the five facts the Node panel's form edits
/// (docs/impl/node-graph-comp.md §1.5).
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeInputKind {
    Picture,
    Number,
    Angle,
    Colour,
}

impl BridgeInputKind {
    #[frb(ignore)]
    fn of(kind: InputKind) -> BridgeInputKind {
        match kind {
            InputKind::Picture => BridgeInputKind::Picture,
            InputKind::Number => BridgeInputKind::Number,
            InputKind::Angle => BridgeInputKind::Angle,
            InputKind::Colour => BridgeInputKind::Colour,
        }
    }

    #[frb(ignore)]
    fn core(self) -> InputKind {
        match self {
            BridgeInputKind::Picture => InputKind::Picture,
            BridgeInputKind::Number => InputKind::Number,
            BridgeInputKind::Angle => InputKind::Angle,
            BridgeInputKind::Colour => InputKind::Colour,
        }
    }
}

/// One Input box's declaration: what it is called, what it carries, and the
/// range and unit its row gets outside the graph.
///
/// The same five facts the Custom shader's Parameter node carries, with the
/// same names, so one form edits both.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeGraphInput {
    /// snake_case, unique in the graph, and the parameter id outside it.
    pub id: String,
    pub label: String,
    pub kind: BridgeInputKind,
    /// One number, or four for a colour.
    pub default: [f64; 4],
    pub min: f64,
    pub max: f64,
    pub unit: BridgeUnit,
    /// A project item to stand in for the picture when nothing feeds a picture
    /// Input (§5.11) - what the Node panel's preview picker sets. It crosses
    /// both ways, so a graph read out and written back keeps the one it has.
    pub preview: Option<Uuid>,
}

impl BridgeGraphInput {
    #[frb(ignore)]
    fn of(input: &GraphInput) -> BridgeGraphInput {
        BridgeGraphInput {
            id: input.id.clone(),
            label: input.label.clone(),
            kind: BridgeInputKind::of(input.kind),
            default: input.default,
            min: input.min,
            max: input.max,
            unit: bridge_unit(input.unit),
            preview: input.preview,
        }
    }

    #[frb(ignore)]
    fn core(self) -> GraphInput {
        GraphInput {
            id: self.id,
            label: self.label,
            kind: self.kind.core(),
            default: self.default,
            min: self.min,
            max: self.max,
            unit: core_unit(self.unit),
            preview: self.preview,
        }
    }
}

/// A Read box as it is stored: which item it brings in, under which name.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeReadNode {
    pub id: Uuid,
    pub item: Uuid,
    pub custom_name: Option<String>,
}

/// An Input box as it is stored: its id on the canvas, and its declaration.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeInputNode {
    pub id: Uuid,
    pub input: BridgeGraphInput,
}

/// One wire: an output socket of one box into an input socket of another.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCompEdge {
    pub from: Uuid,
    pub from_port: String,
    pub to: Uuid,
    pub to_port: String,
}

/// Where one box sits on the canvas, in canvas units.
///
/// Document data, as the layer graph's positions are: they persist and travel,
/// and a box with no entry is auto-placed by the panel.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeCompNodePosition {
    pub node: Uuid,
    pub x: f64,
    pub y: f64,
}

/// A named region of the canvas: a tinted wash behind a set of boxes.
///
/// No rectangle crosses and no colour: the wash follows the members' own
/// positions, and `colour` is an index into the frontend's label palette.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCompNodeGroup {
    pub name: String,
    pub colour: u32,
    pub members: Vec<Uuid>,
}

/// The half of the graph the user edits, read out of [`BridgeCompGraph`],
/// changed, and handed straight back to
/// `CompositionReference::set_node_graph`.
///
/// The Fx boxes are deliberately absent: they are staged instances and travel
/// beside this, which is what lets a parameter edit and a wiring edit be the
/// same one commit.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCompWiring {
    pub reads: Vec<BridgeReadNode>,
    pub inputs: Vec<BridgeInputNode>,
    /// The one Output box's id. A graph always has exactly one, and
    /// [`lumit_core::Op::SetCompGraph`] refuses a write that has none or two.
    pub output: Uuid,
    pub edges: Vec<BridgeCompEdge>,
    pub layout: Vec<BridgeCompNodePosition>,
    /// The boxes twirled open to show every socket. A wired socket is drawn
    /// whether its box is exposed or not.
    pub exposed: Vec<Uuid>,
    pub groups: Vec<BridgeCompNodeGroup>,
}

/// A node graph's whole structure, in one crossing.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCompGraph {
    /// Every box, in document order: the order the wiring was written in, and
    /// the order an Input's row is drawn in outside the graph.
    pub nodes: Vec<BridgeCompNode>,
    pub wiring: BridgeCompWiring,
}

/// One socket, with whether a wire is on it.
#[frb(ignore)]
fn port_of(port: GraphPort, wired: bool) -> BridgePort {
    BridgePort {
        id: port.id,
        label: port.label,
        port_type: BridgePortType::of(port.ty),
        wired,
    }
}

/// Build one node graph's whole structure: the body of
/// `CompositionReference::get_node_graph`.
///
/// `doc` is the project, which the Read boxes need for their items and a Node
/// graph box needs for the Inputs of the comp it names.
#[frb(ignore)]
pub(crate) fn read_comp_graph(project: Uuid, doc: &Document, graph: &CompGraph) -> BridgeCompGraph {
    BridgeCompGraph {
        nodes: graph
            .nodes
            .iter()
            .map(|node| read_node(project, doc, graph, node))
            .collect(),
        wiring: wiring_of(graph),
    }
}

/// One box as the canvas draws it.
#[frb(ignore)]
fn read_node(project: Uuid, doc: &Document, graph: &CompGraph, node: &GraphNode) -> BridgeCompNode {
    // A socket is wired when some edge names it. Linear over the wires, in
    // document order, so two machines report the same graph.
    let id = node.id();
    let (inputs, outputs) = comp_graph::ports_of(graph, node, Some(doc));
    let inputs = inputs
        .into_iter()
        .map(|port| {
            let wired = graph
                .edges
                .iter()
                .any(|e| e.to == id && e.to_port == port.id);
            port_of(port, wired)
        })
        .collect();
    let outputs = outputs
        .into_iter()
        .map(|port| {
            let wired = graph
                .edges
                .iter()
                .any(|e| e.from == id && e.from_port == port.id);
            port_of(port, wired)
        })
        .collect();

    let mut drawn = BridgeCompNode {
        id,
        kind: BridgeCompNodeKind::Output,
        item: None,
        missing: false,
        match_name: String::new(),
        label: String::new(),
        custom_name: node.custom_name().map(str::to_owned),
        enabled: true,
        inputs,
        outputs,
    };
    match node {
        GraphNode::Read { item, .. } => {
            drawn.kind = BridgeCompNodeKind::Read;
            match doc.item(*item) {
                Some(found) => {
                    drawn.label = found.name().to_owned();
                    drawn.item = Some(item_reference(project, found));
                }
                // Somebody deleted the item. The box stays, wearing the missing
                // mark, exactly as a Precomp layer of a deleted comp does.
                None => drawn.missing = true,
            }
        }
        GraphNode::Input { input, .. } => {
            drawn.kind = BridgeCompNodeKind::Input;
            drawn.label = input.label.clone();
        }
        GraphNode::Fx(inst) => {
            drawn.kind = BridgeCompNodeKind::Fx;
            drawn.match_name = inst.effect.match_name.clone();
            // An entry this build does not know draws under its own key rather
            // than under nothing, as the layer canvas has it.
            drawn.label = lumit_core::fx::def(&inst.effect.match_name).map_or_else(
                || inst.effect.match_name.clone(),
                |d| d.schema().label.to_owned(),
            );
            drawn.enabled = inst.enabled;
        }
        GraphNode::Output { .. } => {
            drawn.label = lumit_core::graph::OUTPUT_PORT.label.to_owned();
        }
    }
    drawn
}

/// The stored half of `graph`, as the panel edits it.
#[frb(ignore)]
fn wiring_of(graph: &CompGraph) -> BridgeCompWiring {
    BridgeCompWiring {
        reads: graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Read {
                    id,
                    item,
                    custom_name,
                } => Some(BridgeReadNode {
                    id: *id,
                    item: *item,
                    custom_name: custom_name.clone(),
                }),
                _ => None,
            })
            .collect(),
        inputs: graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Input { id, input } => Some(BridgeInputNode {
                    id: *id,
                    input: BridgeGraphInput::of(input),
                }),
                _ => None,
            })
            .collect(),
        // A graph with no Output is a state a hand-edited file can be in and
        // the ops refuse to write. The nil id names no box, so a panel that
        // hands it straight back is refused rather than quietly given one.
        output: graph.output_id().unwrap_or(Uuid::nil()),
        edges: graph
            .edges
            .iter()
            .map(|e| BridgeCompEdge {
                from: e.from,
                from_port: e.from_port.clone(),
                to: e.to,
                to_port: e.to_port.clone(),
            })
            .collect(),
        layout: graph
            .layout
            .iter()
            .map(|(node, [x, y])| BridgeCompNodePosition {
                node: *node,
                x: *x,
                y: *y,
            })
            .collect(),
        exposed: graph.exposed.clone(),
        groups: graph
            .groups
            .iter()
            .map(|g| BridgeCompNodeGroup {
                name: g.name.clone(),
                colour: g.colour,
                members: g.members.clone(),
            })
            .collect(),
    }
}

/// The document form of an edited wiring: the body of
/// `CompositionReference::set_node_graph`, which pairs it with the staged Fx
/// instances.
///
/// The node list is assembled Reads, then Inputs, then the instances, then the
/// Output, each in the order it was given. **That is the document order from
/// then on**, so the Inputs' order is the row order the panel sent, and a
/// panel that reorders the rows reorders this list.
#[frb(ignore)]
pub(crate) fn wiring_into(wiring: BridgeCompWiring, instances: Vec<EffectInstance>) -> CompGraph {
    let mut nodes =
        Vec::with_capacity(wiring.reads.len() + wiring.inputs.len() + instances.len() + 1);
    for read in wiring.reads {
        nodes.push(GraphNode::Read {
            id: read.id,
            item: read.item,
            custom_name: read.custom_name,
        });
    }
    for input in wiring.inputs {
        nodes.push(GraphNode::Input {
            id: input.id,
            input: input.input.core(),
        });
    }
    nodes.extend(instances.into_iter().map(GraphNode::Fx));
    nodes.push(GraphNode::Output { id: wiring.output });

    CompGraph {
        nodes,
        edges: wiring
            .edges
            .into_iter()
            .map(|e| GraphEdge {
                from: e.from,
                from_port: e.from_port,
                to: e.to,
                to_port: e.to_port,
            })
            .collect(),
        layout: wiring
            .layout
            .into_iter()
            .map(|p| (p.node, [p.x, p.y]))
            .collect(),
        exposed: wiring.exposed,
        groups: wiring
            .groups
            .into_iter()
            .map(|g| GraphGroup {
                name: g.name,
                colour: g.colour,
                members: g.members,
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// A box that reads a file
// ---------------------------------------------------------------------------

/// The file a Roto brush box reads, as the Viewer needs it to carry a scribble
/// onto the picture: which frame of it is on screen, and how big its picture
/// is, which is what says where the Read laid it in the graph's frame.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeBoxSource {
    /// The **source** frame showing at the composition frame asked about, or
    /// `None` where the file will not read: it is offline, or carries no
    /// video. Footage is wired in either way, which is what the caller needs
    /// to tell that from a box with nothing behind it.
    pub frame: Option<i64>,
    /// The file's own picture size, in pixels. Nought where it will not read.
    pub width: u32,
    pub height: u32,
}

/// Remove background, Depth and the Roto brush as boxes
/// (docs/impl/addons.md §6.1, docs/impl/roto.md §5).
///
/// # In plain terms
///
/// These three read a **file** and keep what they made of it beside that file.
/// On a layer the file is the layer's own, and `fire_effect_action`,
/// `plane_status` and the `roto_*` calls all name it through the layer. A box
/// has no layer. Its file is the footage its picture comes from, found by
/// following its picture socket upstream to a Read
/// ([`CompGraph::read_behind`]), and these are the same calls with the
/// composition and the box in the layer's place.
impl CompositionReference {
    /// The Fx box named `effect`, cloned out of the current snapshot.
    #[frb(ignore)]
    fn graph_fx(&self, effect: Uuid) -> Result<EffectInstance, BridgeError> {
        let doc = self.document()?;
        match self.graph_of(&doc)?.node(effect) {
            Some(GraphNode::Fx(inst)) => Ok(inst.clone()),
            _ => Err(BridgeError::InvalidEffect),
        }
    }

    /// The file box `effect` reads. `NotFootage` where its picture comes from
    /// no footage: nothing is wired in, or what is wired in is a solid, a
    /// composition or an Input.
    #[frb(ignore)]
    fn box_source(&self, effect: Uuid) -> Result<crate::api::roto::Source, BridgeError> {
        let doc = self.document()?;
        let (_, item) = self
            .graph_of(&doc)?
            .read_behind(effect)
            .ok_or(BridgeError::NotFootage)?;
        match doc.item(item) {
            Some(lumit_core::model::ProjectItem::Footage(_)) => Ok((self.project, item)),
            _ => Err(BridgeError::NotFootage),
        }
    }

    /// Press one of a box's Action parameters: `fire_effect_action` for a box.
    ///
    /// The boxes with an engine-side button are the planes tier's two and the
    /// Roto brush. Anything else is refused rather than ignored, because a
    /// button that silently does nothing is the hardest kind of fault to see.
    #[frb(sync)]
    pub fn fire_graph_action(&self, effect: Uuid, param: String) -> Result<(), BridgeError> {
        let fx = self.graph_fx(effect)?;
        if fx.effect.match_name == lumit_core::roto::ROTO_BRUSH {
            return crate::api::roto::press(self.box_source(effect), &fx, &param);
        }
        if lumit_core::planes::task_of(&fx).is_some() {
            return crate::api::planes::press(self.box_source(effect), &fx, &param);
        }
        Err(BridgeError::InvalidParam)
    }

    /// A planes-tier box as its status row draws it: `plane_status` for a box.
    #[frb(sync)]
    pub fn graph_plane_status(&self, effect: Uuid) -> Result<BridgePlaneStatus, BridgeError> {
        crate::api::planes::status_of(&self.graph_fx(effect)?)
    }

    /// A Roto brush box as its status row draws it: `roto_status` for a box.
    #[frb(sync)]
    pub fn graph_roto_status(&self, effect: Uuid) -> Result<BridgeRotoStatus, BridgeError> {
        crate::api::roto::status_of(&self.graph_fx(effect)?)
    }

    /// Which frame of its file box `effect` is showing at composition frame
    /// `frame`, and that file's picture size: `roto_source_frame` for a box.
    ///
    /// A Read has no start offset and no Retime, so the file's time is the
    /// graph's own. Read on a frame change and held, never per rebuild.
    ///
    /// `NotFootage` where no footage is wired into the box. A file that is
    /// wired in and will not read answers with no frame instead, so the two
    /// can be told apart and said in different words.
    #[frb(sync)]
    pub fn graph_box_source(
        &self,
        effect: Uuid,
        frame: i64,
    ) -> Result<BridgeBoxSource, BridgeError> {
        let source = self.box_source(effect)?;
        let t = self
            .composition()?
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidParam)?;
        let read = crate::api::roto::media_rate(source)
            .and_then(|(_, _, size)| {
                Ok((
                    crate::api::roto::source_frame_at(source, t.0.to_f64())?,
                    size,
                ))
            })
            .ok();
        Ok(match read {
            Some((frame, (width, height))) => BridgeBoxSource {
                frame: Some(frame),
                width,
                height,
            },
            None => BridgeBoxSource {
                frame: None,
                width: 0,
                height: 0,
            },
        })
    }

    /// Solve the scribbled frame's own matte, now: `roto_solve_frame` for a
    /// box, with the same quiet `false` where the job did not start.
    #[frb(sync)]
    pub fn graph_roto_solve_frame(&self, effect: Uuid, frame: i64) -> Result<bool, BridgeError> {
        crate::api::roto::solve_frame(self.box_source(effect)?, &self.graph_fx(effect)?, frame)
    }
}

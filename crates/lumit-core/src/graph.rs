//! The layer driver graph: the additive wiring a layer carries beside its
//! effect stack ([impl/node-graph.md](../../../docs/impl/node-graph.md)).
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

use crate::fx::{schema::Port, schema::PortType};
use crate::model::EffectInstance;

pub mod looks;

pub const IMAGE_PORT: Port = Port::new("image", "Image", PortType::Image);
pub const MATTE_PORT: Port = Port::new("matte", "Matte", PortType::Matte);
/// An effect node's one picture in — by construction the previous stack
/// entry's output (§1.1), which is why no gesture can branch the chain.
pub const INPUT_PORT: Port = Port::new("input", "Input", PortType::Image);
pub const OUTPUT_PORT: Port = Port::new("output", "Output", PortType::Image);
pub const KEY_PORT: Port = Port::new("key", "Key", PortType::Matte);
pub const AUDIO_PORT: Port = Port::new("audio", "Audio", PortType::Audio);
/// The Layer out node's **Volume** — the one layer property a wire may land on
/// (the Audio workspace board's *Duck under*). A number in decibels: driven, it
/// overrides the layer's own Volume keyframes exactly as a wired effect
/// parameter overrides its own, and the mixer bakes the chain's answer into the
/// same gain stage the keyframes would have fed
/// ([`crate::fx::drivers::driven_volume_db`]).
pub const OUT_VOLUME_PORT: Port = Port::new("volume", "Volume", PortType::Number);

/// Every port the *derived* nodes draw — the ones no schema declares, because
/// the Source node, an effect's picture in and out and the Layer out node are
/// all worked out from the effect stack rather than stored.
///
/// Gathered in one list so the label walk ([`crate::fx::labels`]) can
/// find their words the same way it finds an effect's.
pub const DERIVED_PORTS: [Port; 7] = [
    IMAGE_PORT,
    MATTE_PORT,
    INPUT_PORT,
    OUTPUT_PORT,
    KEY_PORT,
    AUDIO_PORT,
    OUT_VOLUME_PORT,
];

pub const SOURCE_LABEL: &str = "Source";
pub const OUT_LABEL: &str = "Layer out";

/// Names anything the graph canvas draws.
///
/// The image-path nodes are **derived** from the effect stack rather than
/// stored, so they get stable synthetic refs: there is exactly one Source and
/// one Out per layer, and an effect node is named by the id of its
/// [`EffectInstance`] in `Layer::effects`. A driver node is named by the id of
/// its instance in [`LayerGraph::nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodeRef {
    Source,
    Effect(Uuid),
    Driver(Uuid),
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputRef {
    Driver {
        node: Uuid,
        port: String,
    },
    /// The layer's **own** masked source alpha at that point in the chain — a
    /// texture the pipeline has already computed, so wiring it costs no second
    /// render (§1.4). It overrides the effect's Matte *parameter* while it
    /// exists.
    SourceMatte,
    /// A **stack** effect's declared data output — the first wire whose source
    /// is an effect rather than a driver (points-stream.md §1.1).
    ///
    /// The effect goes on making its picture for the chain; this taps the data
    /// it declares beside it, which today means Particulate's Points stream.
    /// A data edge, never an image edge: it cannot reorder, branch or skip the
    /// chain, so `Layer::effects` is still the picture's only authority.
    EffectData {
        effect: Uuid,
        port: String,
    },
    /// A stack effect's picture, as it leaves that effect, into the Matte of an
    /// effect further down. The chain is untouched: the picture still flows on
    /// down the stack, and this hands a copy of it to the matte as well.
    EffectOutput {
        effect: Uuid,
    },
    /// A Correction's key, as it leaves that node, into the Matte of an effect
    /// further down: white where the node grades, black where it leaves the
    /// picture alone.
    EffectKey {
        effect: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputRef {
    /// A parameter socket: the destination parameter follows the source output
    /// instead of its own keyframes. At most one wire per socket.
    Param {
        node: NodeRef,
        port: String,
    },
    Matte {
        effect: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: OutputRef,
    pub to: InputRef,
}

/// The additive wiring a layer carries beside its effect stack.
///
/// Empty by default and **absent from the saved file when empty**, which is
/// what makes a project saved before graphs existed load unchanged and re-save
/// byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerGraph {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<EffectInstance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    /// Canvas positions. **Document data**: they persist and travel, and are
    /// edited through the same whole-graph commit as everything else. A node
    /// with no entry is auto-placed by the panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<(NodeRef, [f64; 2])>,
    /// The boxes dragged wider or narrower by an edge, in canvas units.
    /// Presentation state beside [`Self::layout`]: a box with no entry keeps
    /// the panel's own width.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widths: Vec<(NodeRef, f64)>,
    /// The nodes whose `E` badge is on — grown to show one socket per
    /// parameter (§1.4). Presentation state, beside [`Self::layout`] rather
    /// than on the instance, so a *derived* effect node can carry it without
    /// the whole effect stack gaining a field; it changes no pixel and so
    /// reaches no frame key. A wired socket draws regardless of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed: Vec<NodeRef>,
    /// The named regions of the canvas. Presentation state beside
    /// [`Self::layout`] and for the same reasons: it persists and travels, it
    /// is committed by the same whole-graph write, and it reaches no frame key
    /// because it changes no pixel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<NodeGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ties: Vec<crate::wire_tie::WireTie>,
    #[serde(default, skip_serializing_if = "looks::BoxLooks::is_empty")]
    pub looks: looks::BoxLooks<NodeRef>,
    /// **The Layer out box's image socket is unplugged**: the layer
    /// makes no picture at all.
    ///
    /// The one piece of this graph that is not presentation. Every other
    /// disconnection in the panel lowers to a flag that already existed - an
    /// effect pulled out of the chain is the `enabled` flag its own tick sets -
    /// but the layer's own output had nothing to lower to, and "a layer that
    /// draws nothing because it is not wired up" is a state the owner asked
    /// for by name. It is deliberately *not* the visibility switch: the eye is
    /// the timeline's control and a user who hid a layer there should not find
    /// the graph rewired behind their back, nor the reverse.
    ///
    /// It changes the picture, so unlike its neighbours here it reaches the
    /// frame key - by the same road `switches.visible` does, which is that a
    /// layer skipped for it is fed to the key not at all
    /// (`lumit-eval`'s `feed_comp`, `lumit-render`'s `build`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub out_unwired: bool,
}

/// A named set of boxes drawn on one tinted wash.
///
/// # In plain terms
///
/// A graph of any size grows regions that belong together — the three boxes
/// that make the music drive the glow, say. A group is a name written on that
/// region and a colour behind it: the canvas draws a rectangle around whatever
/// its members happen to be sitting on, so the wash follows the boxes rather
/// than the boxes being trapped in a box.
///
/// **No geometry is stored.** The rectangle is worked out from the members'
/// own positions every time it is drawn, which is what keeps a group honest
/// when a member is dragged, and what keeps this out of the way of every other
/// edit — dragging a box is still one `layout` write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeGroup {
    pub name: String,
    /// Which chip of the label palette tints it. An index, not a colour: no
    /// colour has ever crossed the bridge, and the frontend takes this modulo
    /// its own palette length.
    pub colour: u32,
    /// The boxes inside it. A box may sit in one group or none — a member
    /// listed twice is the same as listed once, and a member the layer no
    /// longer carries is dropped by [`LayerGraph::prune_to`].
    pub members: Vec<NodeRef>,
}

/// A note on a graph's canvas: a few words, where somebody put them.
///
/// Not a box. It has no sockets and nothing reads it, so it changes no pixel
/// and reaches no frame key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub text: String,
    pub colour: u32,
    pub at: [f64; 2],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
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
    /// A **node graph** with nothing to show
    /// ([`crate::comp_graph`]). Here rather than in its own error type so
    /// [`crate::ops::OpError::InvalidGraph`] carries both graph kinds' sentences.
    #[error("a node graph needs an Output box")]
    NoOutput,
    #[error("a node graph has one Output box")]
    SecondOutput,
    #[error("a Repeat end needs a Repeat start of its own before it")]
    RepeatUnpaired,
    #[error("a wire leaves a repeat other than through its Repeat end")]
    RepeatLeak,
}

/// How many of `effects` lie at or above `node` in the image chain — the
/// **prefix** a Node preview shows (note §8 WP5).
///
/// The picture at a node is the picture the layer makes with its stack cut off
/// there: nothing at the Source, the first *n* effects at the *n*th effect
/// node, the whole stack at Layer out. Since the chain is the stack (§1.1),
/// naming the point is naming a length.
///
/// `None` for a [`NodeRef::Driver`] — a driver makes a number, not a picture,
/// so there is nothing to preview — and for an effect this layer does not
/// carry.
#[must_use]
pub fn prefix_len(effects: &[EffectInstance], node: NodeRef) -> Option<usize> {
    match node {
        NodeRef::Source => Some(0),
        NodeRef::Effect(id) => effects.iter().position(|e| e.id == id).map(|i| i + 1),
        NodeRef::Driver(_) => None,
        NodeRef::Out => Some(effects.len()),
    }
}

/// `doc` with `layer`'s effect stack cut to its first `keep` entries — the
/// document a Node preview renders.
///
/// A patched **copy**, never anything the document remembers: exactly the shape
/// the drag previews and the dropper's solo read already use. Because the frame
/// key hashes each layer's effects, the shortened stack names a different frame
/// on its own — the prefix point folds into the key without a field being added
/// to it.
///
/// `None` when nothing would change (the whole stack is kept, or the comp or
/// layer is not there), so a preview of the Layer out node costs no clone and
/// rides the Viewer's own cached frame.
#[must_use]
pub fn truncated_effects(
    doc: &std::sync::Arc<crate::model::Document>,
    comp_id: Uuid,
    layer_id: Uuid,
    keep: usize,
) -> Option<std::sync::Arc<crate::model::Document>> {
    let layer = doc
        .comp(comp_id)?
        .layers
        .iter()
        .find(|l| l.id == layer_id)?;
    if keep >= layer.effects.len() {
        return None;
    }
    let mut copy = crate::model::Document::clone(doc);
    let layer = copy
        .comp_mut(comp_id)?
        .layers
        .iter_mut()
        .find(|l| l.id == layer_id)?;
    layer.effects.truncate(keep);
    Some(std::sync::Arc::new(copy))
}

/// Whether `edge` obeys the **downstream-only** rule for a stack-to-stack data
/// wire (points-stream.md §1.2): the producing effect must sit strictly
/// earlier in `effects` than the consuming one.
///
/// **Why the rule exists.** A points stream is data, but a stack consumer reads
/// it *at its own position in the chain*. When an emit-from-image producer makes
/// its stream depend on the picture arriving at it, a wire pointing back up the
/// stack would ask for a stream that is not defined yet — the consumer's own
/// output would be part of its input. Requiring the producer to be upstream is
/// what keeps that question answerable, and is the recorded carve-out.
///
/// An effect's picture into a later effect's Matte follows the same rule, as
/// the picture below the consumer isn't made yet when the consumer runs.
///
/// `true` for everything else: a driver's wire, the source matte, and a data
/// wire into a *driver*, which is not in the chain and so has no position in it.
/// An edge naming an effect the stack does not carry is `true` here too —
/// dangling is [`LayerGraph::prune_to`]'s business and `UnknownNode`'s, not this
/// rule's.
fn flows_down_the_stack(edge: &Edge, effects: &[EffectInstance]) -> bool {
    let (producer, consumer) = match (&edge.from, &edge.to) {
        (
            OutputRef::EffectData { effect, .. },
            InputRef::Param {
                node: NodeRef::Effect(consumer),
                ..
            },
        )
        | (
            OutputRef::EffectOutput { effect } | OutputRef::EffectKey { effect },
            InputRef::Matte { effect: consumer },
        ) => (effect, consumer),
        _ => return true,
    };
    let at = |id: &Uuid| effects.iter().position(|e| e.id == *id);
    match (at(producer), at(consumer)) {
        // Strictly earlier — so an effect can never feed its own data input
        // either, which is the smallest loop of all.
        (Some(from), Some(to)) => from < to,
        _ => true,
    }
}

impl LayerGraph {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.edges.is_empty()
            && self.layout.is_empty()
            && self.widths.is_empty()
            && self.exposed.is_empty()
            && self.groups.is_empty()
            && self.notes.is_empty()
            && self.looks.is_empty()
            && !self.out_unwired
    }

    /// Put back what an edited wiring doesn't carry: the notes, the ties and
    /// the boxes' looks, from `held`, the graph before the edit. A driver the
    /// edit took away takes its looks with it.
    pub fn keep_canvas(&mut self, held: LayerGraph) {
        self.notes = held.notes;
        self.ties = held.ties;
        self.looks = held.looks;
        let nodes = &self.nodes;
        self.looks.retain(
            |node| !matches!(node, NodeRef::Driver(id) if !nodes.iter().any(|n| n.id == id)),
        );
    }

    #[must_use]
    pub fn node(&self, id: Uuid) -> Option<&EffectInstance> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Drop everything that names an effect the stack no longer carries, and
    /// say whether anything went.
    ///
    /// **Why the graph heals here rather than refusing.** Every other broken
    /// state in this file is an edit somebody made *to the graph*, so refusing
    /// it is the honest answer. A removed effect is not: it is an edit to the
    /// **stack**, which knows nothing of wires and cannot be refused on their
    /// behalf. Left alone, the wire would name a box that is not there and the
    /// next `SetLayerGraph` — a box dragged, a wire drawn, anything — would be
    /// refused for a dangling edge nobody drew. So `SetLayerEffects` prunes,
    /// and takes the wires with the box exactly as deleting a driver does.
    ///
    /// Only `NodeRef::Effect` entries are considered: a driver, the Source and
    /// the Layer out are not the effect stack's to remove.
    ///
    /// Two things dangle now that a wire can *source* from an effect.
    /// A removed producer takes its outgoing data wires with it, exactly as a
    /// removed consumer takes its incoming ones. And a **reorder** can break a
    /// wire without removing anything at all: a stack-to-stack points wire must
    /// flow down the stack (§1.2), so dragging the producer below its consumer
    /// inverts it. That is still a stack edit, still not refusable on the
    /// wiring's behalf, so it heals here the same way — the edge is dropped,
    /// inside the same commit and so inside the same undo step.
    pub fn prune_to(&mut self, effects: &[EffectInstance]) -> bool {
        let alive = |id: &Uuid| effects.iter().any(|e| e.id == *id);
        let gone = |node: &NodeRef| matches!(node, NodeRef::Effect(id) if !alive(id));
        // Members rather than groups: a group that merely *lost* one has still
        // changed, and counting the groups alone would call that no change.
        let members = |g: &Self| {
            g.groups
                .iter()
                .map(|group| group.members.len())
                .sum::<usize>()
        };
        let before = (
            self.edges.len(),
            self.layout.len(),
            self.exposed.len(),
            members(self),
        );
        self.edges.retain(|e| {
            let source_stands = match &e.from {
                OutputRef::Driver { .. } | OutputRef::SourceMatte => true,
                OutputRef::EffectData { effect, .. }
                | OutputRef::EffectOutput { effect }
                | OutputRef::EffectKey { effect } => alive(effect),
            };
            let destination_stands = match &e.to {
                InputRef::Param { node, .. } => !gone(node),
                InputRef::Matte { effect } => alive(effect),
            };
            source_stands && destination_stands && flows_down_the_stack(e, effects)
        });
        self.layout.retain(|(node, _)| !gone(node));
        self.widths.retain(|(node, _)| !gone(node));
        self.exposed.retain(|node| !gone(node));
        // A group loses the members the stack removed, and a group that has
        // lost all of them goes with them: a wash around nothing is not a
        // region, and leaving one behind would be a name with nothing under it.
        for group in &mut self.groups {
            group.members.retain(|node| !gone(node));
        }
        self.groups.retain(|group| !group.members.is_empty());
        let looks_went = self.looks.retain(|node| !gone(&node));
        looks_went
            || before
                != (
                    self.edges.len(),
                    self.layout.len(),
                    self.exposed.len(),
                    members(self),
                )
    }

    #[must_use]
    pub fn wire_into(&self, to: &InputRef) -> Option<&OutputRef> {
        self.edges.iter().find(|e| &e.to == to).map(|e| &e.from)
    }

    pub fn wire_points_from_above(&mut self, effects: &[EffectInstance], consumer: Uuid) -> bool {
        let points_ports = |e: &EffectInstance, outputs: bool| {
            let sig = crate::fx::catalogue::BUILTIN_DEFS
                .get(&e.effect.match_name)
                .map(|def| def.signature());
            sig.map_or(&[][..], |s| if outputs { s.outputs() } else { s.inputs() })
                .iter()
                .filter(|p| p.ty == PortType::Points)
                .map(|p| p.id)
        };
        let Some(at) = effects.iter().position(|e| e.id == consumer) else {
            return false;
        };
        // Nearest first. An effect with several inputs takes a different
        // producer on each, and never one whose points already reach it
        // through a producer it has taken, so a stream is not merged with
        // its own source.
        let mut above = effects
            .iter()
            .take(at)
            .rev()
            .filter_map(|e| points_ports(e, true).next().map(|port| (e.id, port)));
        let mut taken: Vec<Uuid> = Vec::new();
        let mut added = false;
        for port in effects
            .get(at)
            .into_iter()
            .flat_map(|e| points_ports(e, false))
        {
            let to = InputRef::Param {
                node: NodeRef::Effect(consumer),
                port: port.to_owned(),
            };
            if self.wire_into(&to).is_some() {
                continue;
            }
            let Some((producer, out)) = above.find(|(id, _)| !self.feeds_points(*id, &taken))
            else {
                break;
            };
            taken.push(producer);
            self.edges.push(Edge {
                from: OutputRef::EffectData {
                    effect: producer,
                    port: out.to_owned(),
                },
                to,
            });
            added = true;
        }
        added
    }

    fn feeds_points(&self, effect: Uuid, targets: &[Uuid]) -> bool {
        let mut seen = vec![effect];
        let mut next = 0;
        // Each effect is looked at once, so a loop in a hand-edited file ends.
        while let Some(from) = seen.get(next).copied() {
            next += 1;
            if targets.contains(&from) {
                return true;
            }
            for edge in &self.edges {
                let (
                    OutputRef::EffectData { effect: source, .. },
                    InputRef::Param {
                        node: NodeRef::Effect(to),
                        ..
                    },
                ) = (&edge.from, &edge.to)
                else {
                    continue;
                };
                if *source == from && !seen.contains(to) {
                    seen.push(*to);
                }
            }
        }
        false
    }

    #[must_use]
    pub fn source_matte(&self, effect: Uuid) -> bool {
        self.edges
            .iter()
            .any(|e| e.from == OutputRef::SourceMatte && e.to == InputRef::Matte { effect })
    }

    #[must_use]
    pub fn matte_from(&self, effect: Uuid) -> Option<Uuid> {
        self.edges.iter().find_map(|e| match (&e.from, &e.to) {
            (OutputRef::EffectOutput { effect: from }, InputRef::Matte { effect: to })
                if *to == effect =>
            {
                Some(*from)
            }
            _ => None,
        })
    }

    #[must_use]
    pub fn key_from(&self, effect: Uuid) -> Option<Uuid> {
        self.edges.iter().find_map(|e| match (&e.from, &e.to) {
            (OutputRef::EffectKey { effect: from }, InputRef::Matte { effect: to })
                if *to == effect =>
            {
                Some(*from)
            }
            _ => None,
        })
    }

    pub fn validate(&self, effects: &[EffectInstance]) -> Result<(), GraphError> {
        for (i, edge) in self.edges.iter().enumerate() {
            // One wire per socket. Comparing against the edges *before* this
            // one reports the second wire, which is the one being added.
            if self.edges[..i].iter().any(|e| e.to == edge.to) {
                return Err(GraphError::InputAlreadyWired);
            }
            // The downstream-only rule is about where the two *boxes*
            // sit, not about what their sockets carry, so it is answered before
            // any port is looked up — a wire drawn back up the stack gets the
            // loop sentence whether or not its ports would have matched, which
            // is the honest reading of what such a wire asks for.
            if !flows_down_the_stack(edge, effects) {
                return Err(GraphError::Cycle);
            }
            let from = self.output_type(&edge.from, effects)?;
            let to = self.input_type(&edge.to, effects)?;
            if from != to && !(from == PortType::Image && to == PortType::Matte) {
                return Err(GraphError::PortTypeMismatch);
            }
        }
        self.check_acyclic(effects)
    }

    /// Drop the wires from or into a socket the boxes no longer have, and say
    /// whether any went. A row a newer build took away leaves its wire naming
    /// nothing, and every `SetLayerGraph` after would be refused for it, so a
    /// stack that comes in from a file or the clipboard is pruned here.
    pub fn prune_rows(&mut self, effects: &[EffectInstance]) -> bool {
        let before = self.edges.len();
        let edges = std::mem::take(&mut self.edges);
        let gone = |r: Result<PortType, GraphError>| matches!(r, Err(GraphError::UnknownPort));
        self.edges = (edges.into_iter())
            .filter(|e| {
                !gone(self.output_type(&e.from, effects)) && !gone(self.input_type(&e.to, effects))
            })
            .collect();
        self.edges.len() != before
    }

    fn output_type(
        &self,
        from: &OutputRef,
        effects: &[EffectInstance],
    ) -> Result<PortType, GraphError> {
        match from {
            OutputRef::SourceMatte => Ok(PortType::Matte),
            OutputRef::EffectOutput { effect } => effects
                .iter()
                .any(|e| e.id == *effect)
                .then_some(PortType::Image)
                .ok_or(GraphError::UnknownNode),
            OutputRef::EffectKey { effect } => {
                let inst = effects
                    .iter()
                    .find(|e| e.id == *effect)
                    .ok_or(GraphError::UnknownNode)?;
                (inst.effect.match_name == "correction")
                    .then_some(PortType::Matte)
                    .ok_or(GraphError::UnknownPort)
            }
            OutputRef::Driver { node, port } => {
                let inst = self.node(*node).ok_or(GraphError::UnknownNode)?;
                Self::def_of(inst)?
                    .signature()
                    .output(port)
                    .ok_or(GraphError::UnknownPort)
            }
            // A stack effect's declared data output — the signature answers for
            // it exactly as it answers for a driver's, which is why the seam and
            // the validator read one method whichever kind an entry is.
            OutputRef::EffectData { effect, port } => {
                let inst = effects
                    .iter()
                    .find(|e| e.id == *effect)
                    .ok_or(GraphError::UnknownNode)?;
                Self::def_of(inst)?
                    .signature()
                    .output(port)
                    .ok_or(GraphError::UnknownPort)
            }
        }
    }

    /// The catalogue entry behind an instance. An effect this build does not
    /// know has no ports to wire, which is the same answer a missing node gets.
    fn def_of(
        inst: &EffectInstance,
    ) -> Result<&'static dyn crate::fx::registry::EffectDef, GraphError> {
        crate::fx::catalogue::BUILTIN_DEFS
            .get(&inst.effect.match_name)
            .ok_or(GraphError::UnknownNode)
    }

    fn input_type(
        &self,
        to: &InputRef,
        effects: &[EffectInstance],
    ) -> Result<PortType, GraphError> {
        match to {
            InputRef::Matte { effect } => {
                let inst = effects
                    .iter()
                    .find(|e| e.id == *effect)
                    .ok_or(GraphError::UnknownNode)?;
                Self::def_of(inst)?
                    .schema()
                    .matte
                    .param()
                    .map(|_| PortType::Matte)
                    .ok_or(GraphError::UnknownPort)
            }
            InputRef::Param { node, port } => {
                // The Layer out's Volume socket — the one derived port a wire
                // may land on. A layer property rather than a schema parameter,
                // so it is answered here rather than through an instance.
                if matches!(node, NodeRef::Out) && port == OUT_VOLUME_PORT.id {
                    return Ok(PortType::Number);
                }
                // A Text layer's animator numbers, on its Source box. Which
                // ones the layer has is the document's to say; a port that
                // names none is ignored when the letters are worked out.
                if matches!(node, NodeRef::Source)
                    && port.starts_with(crate::text::ports::TEXT_PORT_PREFIX)
                {
                    return Ok(PortType::Number);
                }
                let inst = match node {
                    NodeRef::Effect(id) => effects.iter().find(|e| e.id == *id),
                    NodeRef::Driver(id) => self.node(*id),
                    NodeRef::Source | NodeRef::Out => None,
                }
                .ok_or(GraphError::UnknownNode)?;
                let def = Self::def_of(inst)?;
                def.schema()
                    .params
                    .iter()
                    .chain(def.derived(inst))
                    .find(|p| p.id == port)
                    .and_then(|p| p.kind.port_type())
                    // A **declared data input** beside the schema's
                    // parameters: wire-only, with no stored value to fall
                    // back on, which is why `InputRef::Param` needs no new arm
                    // to reach it.
                    .or_else(|| def.signature().input(port))
                    .ok_or(GraphError::UnknownPort)
            }
        }
    }

    /// Refuse a loop among the driver nodes **and the effects that hand out
    /// data** (points-stream.md §1.2).
    ///
    /// Kahn's algorithm; anything left when no node has an unwired input is in
    /// a loop. Until points wires existed only drivers could close one, because
    /// an effect was never a wire's *source*. `OutputRef::EffectData` makes a
    /// genuine loop constructible: Points sample reads Particulate's stream and
    /// its Count is wired back into Particulate's Emit rate — the stream depends
    /// on the parameters and the parameters on the stream. So the walk grows
    /// effect nodes into it, and an effect gets a link in each direction: out of
    /// it for the data it hands over, into it for a wire onto one of its
    /// parameters. This is what makes the demand-driven driver walk (§1.3)
    /// terminate, so it has to be airtight rather than nearly right.
    ///
    /// **The image chain is deliberately not in this link set.** An effect's
    /// *picture* depends on the previous effect's picture, but a stream depends
    /// only on the producer's own parameters and the time, so adding
    /// chain links would refuse the perfectly sound arrangement of a Points
    /// sample driving a parameter of an effect *above* its producer. The chain's
    /// own constraint on data wires is the positional one
    /// ([`flows_down_the_stack`]), which is checked separately and exactly where
    /// it applies.
    fn check_acyclic(&self, effects: &[EffectInstance]) -> Result<(), GraphError> {
        // (source, destination) for every wire between two evaluated nodes, in
        // document order — no map iteration anywhere, so the walk is the same on
        // every machine.
        let links: Vec<(Uuid, Uuid)> = self
            .edges
            .iter()
            .filter_map(|e| {
                let from = match &e.from {
                    OutputRef::Driver { node, .. } => *node,
                    OutputRef::EffectData { effect, .. } => *effect,
                    OutputRef::SourceMatte
                    | OutputRef::EffectOutput { .. }
                    | OutputRef::EffectKey { .. } => return None,
                };
                let to = match &e.to {
                    InputRef::Param {
                        node: NodeRef::Driver(id) | NodeRef::Effect(id),
                        ..
                    } => *id,
                    InputRef::Param {
                        node: NodeRef::Source | NodeRef::Out,
                        ..
                    }
                    | InputRef::Matte { .. } => return None,
                };
                Some((from, to))
            })
            .collect();
        let all: Vec<Uuid> = self.nodes.iter().chain(effects).map(|n| n.id).collect();
        let mut settled: Vec<Uuid> = Vec::with_capacity(all.len());
        loop {
            let ready: Vec<Uuid> = all
                .iter()
                .copied()
                .filter(|id| !settled.contains(id))
                .filter(|id| {
                    !links
                        .iter()
                        .any(|(from, to)| to == id && !settled.contains(from))
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
}

#[cfg(test)]
mod tests;

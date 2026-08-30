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
    /// A **stack** effect's declared data output — the first wire whose source
    /// is an effect rather than a driver (K-492, points-stream.md §1.1).
    ///
    /// The effect goes on making its picture for the chain; this taps the data
    /// it declares beside it, which today means Particulate's Points stream.
    /// A data edge, never an image edge: it cannot reorder, branch or skip the
    /// chain, so `Layer::effects` is still the picture's only authority.
    EffectData {
        effect: Uuid,
        /// The port's stable id, as the effect's signature declares it.
        port: String,
    },
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
    /// The named regions of the canvas (K-651). Presentation state beside
    /// [`Self::layout`] and for the same reasons: it persists and travels, it
    /// is committed by the same whole-graph write, and it reaches no frame key
    /// because it changes no pixel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<NodeGroup>,
}

/// A named set of boxes drawn on one tinted wash (K-651).
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
    /// What the group is called, drawn as the wash's kicker.
    pub name: String,
    /// Which chip of the label palette tints it. An index, not a colour: no
    /// colour has ever crossed the bridge, and the frontend takes this modulo
    /// its own palette length (K-188's set).
    pub colour: u32,
    /// The boxes inside it. A box may sit in one group or none — a member
    /// listed twice is the same as listed once, and a member the layer no
    /// longer carries is dropped by [`LayerGraph::prune_to`].
    pub members: Vec<NodeRef>,
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

/// How many of `effects` lie at or above `node` in the image chain — the
/// **prefix** a Node preview shows (K-486, note §8 WP5).
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
/// document a Node preview renders (K-486).
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
/// wire (K-492, points-stream.md §1.2): the producing effect must sit strictly
/// earlier in `effects` than the consuming one.
///
/// **Why the rule exists.** A points stream is data, but a stack consumer reads
/// it *at its own position in the chain*. When an emit-from-image producer makes
/// its stream depend on the picture arriving at it, a wire pointing back up the
/// stack would ask for a stream that is not defined yet — the consumer's own
/// output would be part of its input. Requiring the producer to be upstream is
/// what keeps that question answerable, and is the recorded carve-out.
///
/// `true` for everything else: a driver's wire, the source matte, and a data
/// wire into a *driver*, which is not in the chain and so has no position in it.
/// An edge naming an effect the stack does not carry is `true` here too —
/// dangling is [`LayerGraph::prune_to`]'s business and `UnknownNode`'s, not this
/// rule's.
fn flows_down_the_stack(edge: &Edge, effects: &[EffectInstance]) -> bool {
    let (
        OutputRef::EffectData {
            effect: producer, ..
        },
        InputRef::Param {
            node: NodeRef::Effect(consumer),
            ..
        },
    ) = (&edge.from, &edge.to)
    else {
        return true;
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
    /// Whether this layer carries no wiring at all — the overwhelming case, and
    /// what keeps the field out of the saved file.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.edges.is_empty()
            && self.layout.is_empty()
            && self.exposed.is_empty()
            && self.groups.is_empty()
    }

    /// The driver instance named by `id`, if this graph carries one.
    #[must_use]
    pub fn node(&self, id: Uuid) -> Option<&EffectInstance> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Drop everything that names an effect the stack no longer carries, and
    /// say whether anything went (K-471 §1.5).
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
    /// Two things dangle now that a wire can *source* from an effect (K-492).
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
            // The source: a driver or the layer's own alpha is not the stack's
            // to remove, but an effect is.
            let source_stands = match &e.from {
                OutputRef::Driver { .. } | OutputRef::SourceMatte => true,
                OutputRef::EffectData { effect, .. } => alive(effect),
            };
            let destination_stands = match &e.to {
                InputRef::Param { node, .. } => !gone(node),
                InputRef::Matte { effect } => alive(effect),
            };
            source_stands && destination_stands && flows_down_the_stack(e, effects)
        });
        self.layout.retain(|(node, _)| !gone(node));
        self.exposed.retain(|node| !gone(node));
        // A group loses the members the stack removed, and a group that has
        // lost all of them goes with them: a wash around nothing is not a
        // region, and leaving one behind would be a name with nothing under it.
        for group in &mut self.groups {
            group.members.retain(|node| !gone(node));
        }
        self.groups.retain(|group| !group.members.is_empty());
        before
            != (
                self.edges.len(),
                self.layout.len(),
                self.exposed.len(),
                members(self),
            )
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
            // The downstream-only rule (K-492) is about where the two *boxes*
            // sit, not about what their sockets carry, so it is answered before
            // any port is looked up — a wire drawn back up the stack gets the
            // loop sentence whether or not its ports would have matched, which
            // is the honest reading of what such a wire asks for.
            if !flows_down_the_stack(edge, effects) {
                return Err(GraphError::Cycle);
            }
            let from = self.output_type(&edge.from, effects)?;
            let to = self.input_type(&edge.to, effects)?;
            if from != to {
                return Err(GraphError::PortTypeMismatch);
            }
        }
        self.check_acyclic(effects)
    }

    /// The type a wire's source carries.
    fn output_type(
        &self,
        from: &OutputRef,
        effects: &[EffectInstance],
    ) -> Result<PortType, GraphError> {
        match from {
            OutputRef::SourceMatte => Ok(PortType::Matte),
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
    fn def_of(inst: &EffectInstance) -> Result<&'static dyn crate::fx::EffectDef, GraphError> {
        crate::fx::BUILTIN_DEFS
            .get(&inst.effect.match_name)
            .ok_or(GraphError::UnknownNode)
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
                Self::def_of(inst)?
                    .schema()
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
                let def = Self::def_of(inst)?;
                def.schema()
                    .params
                    .iter()
                    .find(|p| p.id == port)
                    .and_then(|p| p.kind.port_type())
                    // A **declared data input** beside the schema's parameters
                    // (K-492 §4.1): wire-only, with no stored value to fall
                    // back on, which is why `InputRef::Param` needs no new arm
                    // to reach it.
                    .or_else(|| def.signature().input(port))
                    .ok_or(GraphError::UnknownPort)
            }
        }
    }

    /// Refuse a loop among the driver nodes **and the effects that hand out
    /// data** (K-492, points-stream.md §1.2).
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
    /// only on the producer's own parameters and the time (K-474), so adding
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
                    OutputRef::SourceMatte => return None,
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
        // Drivers and effects alike: an effect that hands out data is a node in
        // this walk, not a fixed point outside it.
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
                groups: Vec::new(),
            },
            vec![blur],
        )
    }

    /// The Node preview's prefix (K-486): the picture at a node is the stack
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

    // -----------------------------------------------------------------------
    // The points edge (K-492, points-stream.md §1).
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

    /// §1.1: the first wire whose source is a *stack* effect survives the file
    /// format like every other, effect id and port id both.
    #[test]
    fn a_points_edge_round_trips_through_json() {
        let particulate = inst("particulate");
        let blur = inst("blur");
        let graph = LayerGraph {
            edges: vec![points_edge(&particulate, onto(&blur, "radius"))],
            layout: vec![(NodeRef::Effect(particulate.id), [8.0, 16.0])],
            ..LayerGraph::default()
        };
        let json = serde_json::to_string(&graph).expect("serialises");
        let back: LayerGraph = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, graph);
        assert_eq!(
            back.wire_into(&onto(&blur, "radius")),
            Some(&OutputRef::EffectData {
                effect: particulate.id,
                port: "points".to_owned(),
            }),
            "and the wire is findable by its destination, as a driver's is"
        );
    }

    /// §1.1 and §4.1: a stack effect's declared data output is looked up through
    /// its signature, exactly as a driver's output is — so a port Particulate
    /// does not declare is refused, and so is one on an effect that declares no
    /// data output at all.
    #[test]
    fn an_effect_data_wire_names_a_port_the_signature_declares() {
        let particulate = inst("particulate");
        let smooth = inst("smooth");
        let effects = vec![particulate.clone()];

        // The real port, into a driver socket of the wrong type: found, and
        // refused for the type rather than for the name.
        let graph = LayerGraph {
            nodes: vec![smooth.clone()],
            edges: vec![points_edge(
                &particulate,
                InputRef::Param {
                    node: NodeRef::Driver(smooth.id),
                    port: "value".to_owned(),
                },
            )],
            ..LayerGraph::default()
        };
        assert_eq!(
            graph.validate(&effects),
            Err(GraphError::PortTypeMismatch),
            "a points stream is not a number"
        );

        // A port name Particulate does not declare.
        let mut graph = graph;
        let OutputRef::EffectData { port, .. } = &mut graph.edges[0].from else {
            unreachable!()
        };
        *port = "no_such_output".into();
        assert_eq!(graph.validate(&effects), Err(GraphError::UnknownPort));

        // An effect that declares no data output has none to tap.
        let blur = inst("blur");
        let graph = LayerGraph {
            nodes: vec![smooth.clone()],
            edges: vec![points_edge(
                &blur,
                InputRef::Param {
                    node: NodeRef::Driver(smooth.id),
                    port: "value".to_owned(),
                },
            )],
            ..LayerGraph::default()
        };
        assert_eq!(
            graph.validate(std::slice::from_ref(&blur)),
            Err(GraphError::UnknownPort)
        );

        // And an effect the stack does not carry at all.
        let graph = LayerGraph {
            nodes: vec![smooth],
            edges: vec![points_edge(
                &particulate,
                InputRef::Param {
                    node: NodeRef::Driver(inst("smooth").id),
                    port: "value".to_owned(),
                },
            )],
            ..LayerGraph::default()
        };
        assert_eq!(graph.validate(&[]), Err(GraphError::UnknownNode));
    }

    /// The recorded carve-out (K-492, §1.2): a stack-to-stack points wire flows
    /// **down** the stack. Tested in both directions, and on the smallest loop
    /// of all — an effect wired into itself.
    #[test]
    fn a_stack_to_stack_points_wire_must_flow_down_the_stack() {
        let particulate = inst("particulate");
        let blur = inst("blur");
        let wire = |effects: Vec<EffectInstance>, to: &EffectInstance| {
            let graph = LayerGraph {
                edges: vec![points_edge(&particulate, onto(to, "points_in"))],
                ..LayerGraph::default()
            };
            graph.validate(&effects)
        };

        // Producer above consumer: not refused for its direction. (It is
        // refused for its port — no stack effect declares a Points input until
        // the family lands — which is exactly the answer that proves the
        // ordering rule let it through.)
        assert_eq!(
            wire(vec![particulate.clone(), blur.clone()], &blur),
            Err(GraphError::UnknownPort),
            "downstream is allowed to reach the port check"
        );

        // Producer below consumer: refused before any port is looked up.
        assert_eq!(
            wire(vec![blur.clone(), particulate.clone()], &blur),
            Err(GraphError::Cycle),
            "a points wire drawn back up the stack closes a loop"
        );

        // And an effect feeding its own data input: not strictly earlier than
        // itself, so the same refusal.
        assert_eq!(
            wire(vec![particulate.clone()], &particulate),
            Err(GraphError::Cycle),
            "a producer cannot feed itself"
        );
    }

    /// §1.2: the rule constrains the *stack*, not the whole graph. A points
    /// wire into a **driver** has no position in the image chain to be wrong
    /// about, wherever its producer sits.
    #[test]
    fn a_points_wire_into_a_driver_has_no_stack_position_to_break() {
        let particulate = inst("particulate");
        let blur = inst("blur");
        let smooth = inst("smooth");
        let graph = LayerGraph {
            nodes: vec![smooth.clone()],
            edges: vec![points_edge(
                &particulate,
                InputRef::Param {
                    node: NodeRef::Driver(smooth.id),
                    port: "value".to_owned(),
                },
            )],
            ..LayerGraph::default()
        };
        // The type is wrong (a stream is not a number), but never the ordering
        // — with the producer last in the stack as much as first.
        for effects in [
            vec![particulate.clone(), blur.clone()],
            vec![blur, particulate],
        ] {
            assert_eq!(graph.validate(&effects), Err(GraphError::PortTypeMismatch));
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
            nodes: vec![smooth.clone()],
            edges: vec![stream_into(&smooth)],
            ..LayerGraph::default()
        };
        open.check_acyclic(&stack).expect("a line is not a loop");

        // Both legs: the stream depends on the parameters and the parameters on
        // the stream.
        let closed = LayerGraph {
            nodes: vec![smooth.clone()],
            edges: vec![stream_into(&smooth), back_into_producer(&smooth)],
            ..LayerGraph::default()
        };
        assert_eq!(closed.check_acyclic(&stack), Err(GraphError::Cycle));

        // The driver's output alone, with no stream read, is not a loop — so
        // the refusal above is about the round trip and not about an effect
        // merely being in the walk.
        let driven = LayerGraph {
            nodes: vec![smooth.clone()],
            edges: vec![back_into_producer(&smooth)],
            ..LayerGraph::default()
        };
        driven
            .check_acyclic(&stack)
            .expect("a driven parameter is not a loop");
    }

    /// The same walk, adversarially: loops that close through **two** producers
    /// and two drivers, and a long line that only looks like one.
    #[test]
    fn a_cycle_through_two_producers_is_refused_and_a_long_line_is_not() {
        let (p1, p2) = (inst("particulate"), inst("particulate"));
        let (d1, d2) = (inst("smooth"), inst("remap"));
        let stack = vec![p1.clone(), p2.clone()];

        let stream = |from: &EffectInstance, to: &EffectInstance| Edge {
            from: OutputRef::EffectData {
                effect: from.id,
                port: "points".to_owned(),
            },
            to: InputRef::Param {
                node: NodeRef::Driver(to.id),
                port: "points".to_owned(),
            },
        };
        let drives = |from: &EffectInstance, to: &EffectInstance| {
            param_edge(from, "value", NodeRef::Effect(to.id), "emit_rate")
        };

        // p1 → d1 → p2 → d2 → p1: four hops, no two of them adjacent.
        let closed = LayerGraph {
            nodes: vec![d1.clone(), d2.clone()],
            edges: vec![
                stream(&p1, &d1),
                drives(&d1, &p2),
                stream(&p2, &d2),
                drives(&d2, &p1),
            ],
            ..LayerGraph::default()
        };
        assert_eq!(closed.check_acyclic(&stack), Err(GraphError::Cycle));

        // The same four boxes with the last hop landing on a third effect
        // instead: a line, however long.
        let blur = inst("blur");
        let open = LayerGraph {
            nodes: vec![d1.clone(), d2.clone()],
            edges: vec![
                stream(&p1, &d1),
                drives(&d1, &p2),
                stream(&p2, &d2),
                param_edge(&d2, "value", NodeRef::Effect(blur.id), "radius"),
            ],
            ..LayerGraph::default()
        };
        open.check_acyclic(&[p1.clone(), p2.clone(), blur])
            .expect("a line is not a loop");

        // Declared in reverse of evaluation order, so the walk cannot be
        // relying on the order the boxes happen to sit in.
        let reversed = LayerGraph {
            nodes: vec![d2, d1],
            edges: closed.edges.into_iter().rev().collect(),
            ..LayerGraph::default()
        };
        assert_eq!(
            reversed.check_acyclic(&[p2, p1]),
            Err(GraphError::Cycle),
            "a loop is a loop in whatever order it is written down"
        );
    }

    /// §1.2: `prune_to`'s old comment — "a wire's *source* is a driver or the
    /// layer's own alpha, neither of which the stack can remove" — stops being
    /// true the moment a wire can source from an effect. A removed producer
    /// takes its outgoing data wires with it.
    #[test]
    fn removing_a_producer_takes_its_data_wires_with_it() {
        let particulate = inst("particulate");
        let blur = inst("blur");
        let mut graph = LayerGraph {
            edges: vec![points_edge(&particulate, onto(&blur, "points_in"))],
            layout: vec![(NodeRef::Effect(particulate.id), [0.0, 0.0])],
            ..LayerGraph::default()
        };

        // The consumer alone stays: the producer is gone.
        assert!(graph.prune_to(std::slice::from_ref(&blur)));
        assert!(graph.edges.is_empty(), "the wire went with the box");
        assert!(graph.layout.is_empty());

        // And the whole stack still there prunes nothing.
        let mut graph = LayerGraph {
            edges: vec![points_edge(&particulate, onto(&blur, "points_in"))],
            ..LayerGraph::default()
        };
        assert!(!graph.prune_to(&[particulate, blur]));
        assert_eq!(graph.edges.len(), 1);
    }

    /// The healing half of the carve-out (K-492): a reorder that inverts a
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

    /// **A real stack-to-stack points wire** (K-492, K-600): the arrangement
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
            edges: vec![points_edge(&grid, onto(&consumer, "points"))],
            ..LayerGraph::default()
        };
        from_grid
            .validate(&[grid, consumer.clone()])
            .expect("a lattice is a points stream like any other");

        // And a number is not a stream, on this end as on the driver's.
        let wiggle = inst("wiggle");
        let mistyped = LayerGraph {
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

    /// **The cross-layer tap is an ordinary wire out of an ordinary node**
    /// (K-604, points-stream.md §1.2): the edge rules needed no arm for it,
    /// which is the point of settling the design as a *layer-reference
    /// parameter* rather than as an edge that crosses layers.
    ///
    /// What is asserted here is that nothing had to be relaxed to let it
    /// through, and that the taxonomy still refuses everything it refused.
    #[test]
    fn a_points_tap_wires_like_any_other_driver_and_refuses_like_one() {
        let tap = inst("layer_points");
        let consumer = inst("clone_to_points");
        let sample = inst("points_sample");
        let from_tap = |to: InputRef| Edge {
            from: OutputRef::Driver {
                node: tap.id,
                port: "points".to_owned(),
            },
            to,
        };

        // Into a stack effect's Points socket, and into a driver's: both are
        // the ordinary type match, through the ordinary `Driver` arm.
        for to in [
            onto(&consumer, "points"),
            InputRef::Param {
                node: NodeRef::Driver(sample.id),
                port: "points".to_owned(),
            },
        ] {
            let graph = LayerGraph {
                nodes: vec![tap.clone(), sample.clone()],
                edges: vec![from_tap(to)],
                ..LayerGraph::default()
            };
            graph
                .validate(std::slice::from_ref(&consumer))
                .expect("a tap is a Points source like any other");
        }

        // A stream is not a number, on this end as on the producer's.
        let blur = inst("blur");
        let mistyped = LayerGraph {
            nodes: vec![tap.clone()],
            edges: vec![from_tap(onto(&blur, "radius"))],
            ..LayerGraph::default()
        };
        assert_eq!(
            mistyped.validate(std::slice::from_ref(&blur)),
            Err(GraphError::PortTypeMismatch)
        );

        // A tap has no input of its own — nothing to wire *into*, so naming one
        // is naming a port that does not exist.
        let wiggle = inst("wiggle");
        let backwards = LayerGraph {
            nodes: vec![tap.clone(), wiggle.clone()],
            edges: vec![Edge {
                from: OutputRef::Driver {
                    node: wiggle.id,
                    port: "value".to_owned(),
                },
                to: InputRef::Param {
                    node: NodeRef::Driver(tap.id),
                    port: "points".to_owned(),
                },
            }],
            ..LayerGraph::default()
        };
        assert_eq!(backwards.validate(&[]), Err(GraphError::UnknownPort));

        // And the tap takes its wires with it when the node goes, exactly as
        // every other driver does — `prune_to` is the stack's business, so a
        // graph without the node is simply a graph with a missing node.
        let orphaned = LayerGraph {
            nodes: Vec::new(),
            edges: vec![from_tap(onto(&consumer, "points"))],
            ..LayerGraph::default()
        };
        assert_eq!(orphaned.validate(&[consumer]), Err(GraphError::UnknownNode));
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

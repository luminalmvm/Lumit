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

use crate::fx::PortType;
use crate::model::EffectInstance;

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
        self.nodes.is_empty() && self.edges.is_empty() && self.layout.is_empty()
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

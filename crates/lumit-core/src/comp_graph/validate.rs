//! The rules a node graph is checked against before it's stored.

use uuid::Uuid;

use super::{ports_of, CompGraph, GraphNode};
use crate::fx::schema::PortType;
use crate::graph::GraphError;
use crate::model::Document;

impl CompGraph {
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
            // thing people reach for first. A depth socket whose effect also
            // reads a grey depth pass takes a picture for the same reason.
            // Everywhere else a wire's type must equal its socket's.
            let takes_a_pass = || match to {
                GraphNode::Fx(inst) => crate::fx::builtins::def(&inst.effect.match_name)
                    .is_some_and(|d| d.depth_takes_a_pass()),
                _ => false,
            };
            let matched = source.ty == sink.ty
                || (sink.ty == PortType::Matte && source.ty == PortType::Image)
                || (sink.ty == PortType::Depth && source.ty == PortType::Image && takes_a_pass());
            if !matched {
                return Err(GraphError::PortTypeMismatch);
            }
        }
        self.check_acyclic()?;
        self.repeats().map(|_| ())
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
}

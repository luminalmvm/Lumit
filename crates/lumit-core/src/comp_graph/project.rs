//! A node graph's values turned into the shapes the engine already evaluates.

use uuid::Uuid;

use super::{
    baked, data_output, is_driver, set_param, CompGraph, GraphEdge, GraphNode, Projection,
};
use crate::model::{EffectInstance, EffectValue};

impl CompGraph {
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
        self.cone_picked(node, &[])
    }

    /// [`Self::cone_of`] with each Switch in `picks`, as `(box, index)`,
    /// followed down only the socket it shows. What sits behind the other
    /// sockets is left out, unless something else in the cone reads it too.
    #[must_use]
    pub fn cone_picked(&self, node: Uuid, picks: &[(Uuid, i64)]) -> Vec<Uuid> {
        let shown = |e: &GraphEdge| match picks.iter().find(|(id, _)| *id == e.to) {
            // The Index wire and anything else that is not a picture socket
            // is always followed.
            Some((_, index)) => e
                .to_port
                .strip_prefix("in")
                .and_then(|n| n.parse::<i64>().ok())
                .is_none_or(|n| n == *index),
            None => true,
        };
        let mut wanted: Vec<Uuid> = vec![node];
        let mut seen = 0usize;
        while seen < wanted.len() {
            let at = wanted[seen];
            seen += 1;
            for e in self.edges.iter().filter(|e| e.to == at && shown(e)) {
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

    /// The index the Switch box `inst` shows at `t`, when the box alone says:
    /// bypassed, which shows its first socket, or an Index that is still or
    /// keyed with nothing wired into it. `None` when a wire or an expression
    /// decides, which only the full resolve with its drivers can answer.
    #[must_use]
    pub fn plain_switch_index(&self, inst: &EffectInstance, t: f64) -> Option<i64> {
        if !inst.enabled {
            return Some(0);
        }
        if self.wire_into(inst.id, "index").is_some() {
            return None;
        }
        if let Some(EffectValue::Float(p)) = inst.param("index") {
            if matches!(p.animation, crate::anim::Animation::Expression(_)) {
                return None;
            }
        }
        // Resolved by the code the builder resolves it with, so the two
        // cannot round it differently.
        let rows = crate::fx::resolved::resolve_instance(
            inst,
            &crate::fx::drivers::ResolvedDrivers::default(),
            t,
            1.0,
            1.0,
            &crate::fx::markers::MarkerContext::NONE,
            std::sync::Arc::new(crate::expression::ExpressionContext::detached()),
        );
        let index = rows.iter().next().map_or(0, |op| {
            i64::from(op.params.int(crate::fx::params::ParamId::new("index"), 0))
        });
        Some(index)
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
    /// closure it hands in: [`crate::fx::temporal::input_times`] wants it and this walk
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

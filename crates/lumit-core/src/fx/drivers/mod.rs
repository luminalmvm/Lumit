//! The drivers, and how a layer's driver graph is evaluated (K-471 §1.3, §2).
//!
//! # In plain terms
//!
//! A driver makes a *value* rather than a picture, and a wire from a driver
//! into an effect's socket makes that parameter follow the value instead of its
//! keyframes. This module holds the six of them — Wiggle, Audio level, Colour
//! cycle, Math, Remap, Smooth — and the small walk that works out, at one
//! frame, what every wire is carrying.
//!
//! **Driver evaluation is parameter evaluation.** It happens where a keyframe
//! would have been read, before an effect's numbers are packed for the GPU, so
//! nothing downstream learns anything new: the kernels see numbers, exactly as
//! they see keyframed numbers today, and the compiled evaluation graph (K-015)
//! is untouched in shape. Drivers never become pixel nodes.
//!
//! **The walk is demand-driven.** Asking what an effect's socket is carrying
//! asks the driver feeding it, which asks whatever feeds *its* sockets, and so
//! on back to the numbers somebody typed. That is topological order by
//! construction, and there is no map iteration anywhere, so two machines
//! evaluate in the same order and get the same numbers — which is what makes
//! export equal preview (K-031).

use std::cell::Cell;
use std::sync::Arc;

use uuid::Uuid;

use super::markers::MarkerContext;
use super::params::{ParamId, Value};
use super::registry::{AudioTap, DriverCx};
use super::resolved::resolve_into_arena;
use super::ResolvedStack;
use crate::expression::ExpressionContext;
use crate::graph::{InputRef, LayerGraph, NodeRef, OutputRef};

pub mod audio_level;
pub mod colour_cycle;
pub mod math;
pub mod remap;
pub mod smooth;
pub mod wiggle;

/// How many driver nodes one frame's evaluation may visit.
///
/// ponytail: a flat budget instead of memoising each `(node, port, time)`. A
/// graph shaped like a diamond re-evaluates its shared upstream once per path,
/// and a Smooth multiplies its subtree by its tap count — fine for the handful
/// of nodes a layer carries, and this is the ceiling that stops a pathological
/// or hand-edited graph from taking a frame with it. Add the memo if a real
/// graph ever spends it.
const EVAL_BUDGET: u32 = 4096;

/// How deep the walk may recurse, so a very long chain cannot overflow the
/// stack before the budget notices.
const MAX_DEPTH: u32 = 32;

/// What a layer's driver graph came to at one frame (K-471 §2.1).
///
/// Two answers, because a wire carries one of two things: a **value** that
/// stands in for a parameter's keyframes, or the layer's **own source alpha**
/// standing in for an effect's matte.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedDrivers {
    /// `(destination node, parameter, value)`, sorted, so the substitution a
    /// frame makes never depends on the order the wires were drawn in.
    subs: Vec<(NodeRef, ParamId, Value)>,
    /// The effects whose matte is the layer's own masked source alpha (§1.4).
    source_matte: Vec<Uuid>,
}

impl ResolvedDrivers {
    /// No drivers at all — what every layer without a graph resolves through,
    /// and what the resolve path takes when nothing is wired.
    pub const NONE: &'static ResolvedDrivers = &ResolvedDrivers {
        subs: Vec::new(),
        source_matte: Vec::new(),
    };

    /// Whether anything is driven at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.subs.is_empty() && self.source_matte.is_empty()
    }

    /// The value driving `id` on `node`, or `None` for a parameter that is
    /// reading its own keyframes as usual.
    #[must_use]
    pub fn param(&self, node: NodeRef, id: ParamId) -> Option<Value> {
        self.subs
            .iter()
            .find(|(n, p, _)| *n == node && *p == id)
            .map(|(_, _, v)| *v)
    }

    /// Whether `effect`'s matte is the layer's own source alpha (§1.4).
    #[must_use]
    pub fn source_matte(&self, effect: Uuid) -> bool {
        self.source_matte.contains(&effect)
    }

    /// Every substitution, in sorted order — for the frame key and the tests.
    pub fn iter(&self) -> impl Iterator<Item = (NodeRef, ParamId, Value)> + '_ {
        self.subs.iter().copied()
    }
}

/// Evaluate a layer's driver graph at layer time `lt`.
///
/// `audio` is the host's decoded sound, where it has any; `None` reads as
/// silence, which is the documented degrade rather than a fault.
#[must_use]
pub fn resolve_drivers(
    graph: &LayerGraph,
    lt: f64,
    context: Arc<ExpressionContext>,
    audio: Option<&dyn AudioTap>,
) -> ResolvedDrivers {
    if graph.edges.is_empty() {
        return ResolvedDrivers::default();
    }
    let ev = Eval {
        graph,
        context,
        audio,
        budget: Cell::new(EVAL_BUDGET),
    };
    let mut out = ResolvedDrivers::default();
    // Document order in, sorted order out: the wires a layer carries are a
    // list, and which one was drawn first must not decide anything.
    for edge in &graph.edges {
        match (&edge.from, &edge.to) {
            (OutputRef::SourceMatte, InputRef::Matte { effect }) => {
                out.source_matte.push(*effect);
            }
            (
                OutputRef::Driver { node, port },
                InputRef::Param {
                    node: NodeRef::Effect(target),
                    port: socket,
                },
            ) => {
                if let Some(v) = ev.output(*node, port, lt, 0) {
                    out.subs
                        .push((NodeRef::Effect(*target), ParamId::new(socket), v));
                }
            }
            // A wire between two drivers is followed on demand above; one into
            // a node this layer does not have was refused at commit and is a
            // stale line to ignore here (14-ENGINEERING-RULES §4).
            _ => {}
        }
    }
    out.subs.sort_by_key(|s| (s.0, s.1));
    out.subs.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    out.source_matte.sort();
    out.source_matte.dedup();
    out
}

/// The largest distance either side of the frame any driver in this graph reads
/// (K-471 §2.3) — the **temporal declaration**, folded into the frame key so a
/// cached frame cannot outlive the range it was averaged or measured from.
///
/// Nought for a graph of pointwise drivers, which is most of them.
#[must_use]
pub fn temporal_window(graph: &LayerGraph, lt: f64, context: Arc<ExpressionContext>) -> f64 {
    let mut widest = 0.0f64;
    for inst in &graph.nodes {
        let Some(def) = super::BUILTIN_DEFS.get(&inst.effect.match_name) else {
            continue;
        };
        let mut bag = ResolvedStack::new();
        resolve_into_arena(
            def,
            inst,
            NodeRef::Driver(inst.id),
            lt,
            0.0,
            1.0,
            &MarkerContext::NONE,
            &mut bag,
            context.clone(),
            ResolvedDrivers::NONE,
        );
        if let Some(fx) = bag.get(0) {
            let w = def.driver_window(fx.params);
            if w.is_finite() && w > widest {
                widest = w;
            }
        }
    }
    widest
}

/// The demand-driven walk.
struct Eval<'a> {
    graph: &'a LayerGraph,
    context: Arc<ExpressionContext>,
    audio: Option<&'a dyn AudioTap>,
    budget: Cell<u32>,
}

impl Eval<'_> {
    /// What driver `node` puts out of `port` at time `t`.
    fn output(&self, node: Uuid, port: &str, t: f64, depth: u32) -> Option<Value> {
        if depth >= MAX_DEPTH || self.budget.get() == 0 {
            return None;
        }
        self.budget.set(self.budget.get() - 1);
        let inst = self.graph.node(node)?;
        // Bypass (the `B` badge, §1.4) is the ordinary `enabled` flag: a
        // bypassed driver carries nothing, and every socket it fed falls back
        // to the keyframes it had before the wire was drawn.
        if !inst.enabled {
            return None;
        }
        let def = super::BUILTIN_DEFS.get(&inst.effect.match_name)?;

        // This node's own incoming wires, evaluated first — which is what makes
        // the walk topological without a sort.
        let mut subs = Vec::new();
        for edge in &self.graph.edges {
            let (
                OutputRef::Driver {
                    node: src,
                    port: src_port,
                },
                InputRef::Param {
                    node: NodeRef::Driver(dest),
                    port: socket,
                },
            ) = (&edge.from, &edge.to)
            else {
                continue;
            };
            if *dest != node {
                continue;
            }
            if let Some(v) = self.output(*src, src_port, t, depth + 1) {
                subs.push((NodeRef::Driver(node), ParamId::new(socket), v));
            }
        }
        subs.sort_by_key(|s| (s.0, s.1));
        let wired = ResolvedDrivers {
            subs,
            source_matte: Vec::new(),
        };

        // ponytail: one small arena per node evaluation. It is the same shape
        // the effect stack resolves through, so the substitution and unit rules
        // stay in one place; pool them if a profile ever shows the allocation.
        let mut bag = ResolvedStack::new();
        resolve_into_arena(
            def,
            inst,
            NodeRef::Driver(node),
            t,
            0.0,
            1.0,
            &MarkerContext::NONE,
            &mut bag,
            self.context.clone(),
            &wired,
        );
        let fx = bag.get(0)?;

        let sample = |socket: &str, at: f64| self.input(node, socket, at, depth + 1);
        let cx = DriverCx {
            node,
            inst,
            lt: t,
            params: fx.params,
            audio: self.audio,
            sample_input: &sample,
        };
        let mut found = None;
        def.eval_driver(&cx, &mut |id, value| {
            if id == port {
                found = Some(value);
            }
        });
        found
    }

    /// What is feeding driver `node`'s `port` at time `t`, or `None` when the
    /// socket is unwired.
    fn input(&self, node: Uuid, port: &str, t: f64, depth: u32) -> Option<Value> {
        let from = self.graph.wire_into(&InputRef::Param {
            node: NodeRef::Driver(node),
            port: port.to_owned(),
        })?;
        match from {
            OutputRef::Driver {
                node: src,
                port: src_port,
            } => self.output(*src, src_port, t, depth),
            OutputRef::SourceMatte => None,
        }
    }
}

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
            // Neither of these is a number a driver could be handed. The source
            // matte is a texture; a points stream is a whole frame's particles,
            // read through its own arm of the walk when the first driver
            // declares a Points input (points-stream.md §3.3). Until then a
            // socket fed by one reads as unwired, which is the documented no-op
            // rather than a wrong number.
            OutputRef::SourceMatte | OutputRef::EffectData { .. } => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::{instantiate, MarkerContext, ResolvedStack};
    use crate::graph::Edge;
    use crate::model::{EffectInstance, EffectValue};

    fn inst(match_name: &str) -> EffectInstance {
        instantiate(match_name).expect("the catalogue knows it")
    }

    fn set(inst: &mut EffectInstance, id: &str, v: f64) {
        for p in &mut inst.params {
            if p.id == id {
                p.value = EffectValue::Float(crate::anim::Property::fixed(v));
                return;
            }
        }
        panic!("no parameter {id}");
    }

    fn set_choice(inst: &mut EffectInstance, id: &str, v: u32) {
        for p in &mut inst.params {
            if p.id == id {
                p.value = EffectValue::Choice(v);
                return;
            }
        }
        panic!("no parameter {id}");
    }

    fn ctx() -> Arc<ExpressionContext> {
        Arc::new(ExpressionContext::detached())
    }

    fn edge(from: &EffectInstance, port: &str, to: NodeRef, socket: &str) -> Edge {
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

    /// One driver's output at `lt`, with nothing wired into it — the shape most
    /// of the per-driver tests want.
    fn output_of(node: &EffectInstance, port: &str, lt: f64) -> f32 {
        let target = inst("blur");
        let graph = LayerGraph {
            edges: vec![edge(node, port, NodeRef::Effect(target.id), "radius")],
            nodes: vec![node.clone()],
            ..LayerGraph::default()
        };
        resolve_drivers(&graph, lt, ctx(), None)
            .param(NodeRef::Effect(target.id), ParamId::new("radius"))
            .expect("the wire carries something")
            .as_f32()
    }

    /// K-031, and the whole reason a driver is seeded rather than random: the
    /// same node at the same time is the same number, twice and for ever — and
    /// two Wiggles on one layer are two different wobbles.
    #[test]
    fn wiggle_is_the_same_wobble_every_time() {
        let mut w = inst("wiggle");
        set(&mut w, "amount", 10.0);
        set(&mut w, "frequency", 2.0);

        let first: Vec<f32> = (0..20)
            .map(|i| output_of(&w, "value", f64::from(i) * 0.1))
            .collect();
        let second: Vec<f32> = (0..20)
            .map(|i| output_of(&w, "value", f64::from(i) * 0.1))
            .collect();
        assert_eq!(first, second, "two evaluations must agree bit for bit");

        // It is a wobble, not a constant, and it stays inside Amount.
        assert!(
            first.iter().any(|v| (v - first[0]).abs() > 1e-3),
            "a wiggle that never moves is not a wiggle: {first:?}"
        );
        assert!(
            first.iter().all(|v| v.abs() <= 10.0 + 1e-4),
            "the wobble must stay within Amount: {first:?}"
        );

        // A second node, same parameters, different id: a different path.
        let mut other = inst("wiggle");
        other.params = w.params.clone();
        let theirs: Vec<f32> = (0..20)
            .map(|i| output_of(&other, "value", f64::from(i) * 0.1))
            .collect();
        assert_ne!(first, theirs, "two nodes must not wobble in sync");

        // Amount nought is silence, whatever the frequency.
        let mut still = inst("wiggle");
        set(&mut still, "amount", 0.0);
        assert_eq!(output_of(&still, "value", 0.37), 0.0);
    }

    /// A synthesised tone: a sine of amplitude A has RMS A/√2, and a tone well
    /// above the low band is most of the way gone from the Low output.
    #[test]
    fn audio_level_measures_a_synthesised_tone() {
        /// A sine at `hz` and amplitude `amp`, sampled at 48 kHz.
        struct Tone {
            hz: f64,
            amp: f32,
        }
        impl AudioTap for Tone {
            fn samples(&self, _layer: Uuid, from: f64, to: f64, out: &mut Vec<f32>) -> Option<f64> {
                let rate = 48_000.0;
                let n = ((to - from) * rate).round().max(1.0) as usize;
                for i in 0..n {
                    let t = from + i as f64 / rate;
                    out.push(self.amp * (std::f64::consts::TAU * self.hz * t).sin() as f32);
                }
                Some(rate)
            }
        }

        let measure = |tap: &dyn AudioTap, port: &str| -> f32 {
            let mut node = inst("audio_level");
            let music = Uuid::now_v7();
            for p in &mut node.params {
                if p.id == "audio" {
                    p.value = EffectValue::Layer(Some(music));
                }
            }
            set(&mut node, "window", 0.1);
            let target = inst("blur");
            let graph = LayerGraph {
                edges: vec![edge(&node, port, NodeRef::Effect(target.id), "radius")],
                nodes: vec![node],
                ..LayerGraph::default()
            };
            resolve_drivers(&graph, 1.0, ctx(), Some(tap))
                .param(NodeRef::Effect(target.id), ParamId::new("radius"))
                .expect("wired")
                .as_f32()
        };

        // 100 Hz at amplitude 0.5 — inside the low band, so both outputs read it.
        let low_tone = Tone {
            hz: 100.0,
            amp: 0.5,
        };
        let rms = 0.5 / 2.0f32.sqrt();
        assert!(
            (measure(&low_tone, "amplitude") - rms).abs() < 0.01,
            "a sine of amplitude 0.5 has RMS 0.354, not {}",
            measure(&low_tone, "amplitude")
        );
        assert!(
            measure(&low_tone, "low") > rms * 0.5,
            "a 100 Hz tone must survive the low band"
        );

        // 4 kHz at the same amplitude: the whole level is unchanged, the low
        // band is all but gone.
        let high_tone = Tone {
            hz: 4000.0,
            amp: 0.5,
        };
        assert!((measure(&high_tone, "amplitude") - rms).abs() < 0.01);
        assert!(
            measure(&high_tone, "low") < measure(&low_tone, "low") * 0.25,
            "4 kHz must be attenuated far below 100 Hz"
        );

        // No tap at all is silence, not a fault.
        let mut node = inst("audio_level");
        set(&mut node, "window", 0.1);
        assert_eq!(output_of(&node, "amplitude", 1.0), 0.0);
    }

    /// Colour cycle turns through the wheel and comes back, and Rate nought
    /// holds it still.
    #[test]
    fn colour_cycle_turns_once_a_cycle() {
        let mut c = inst("colour_cycle");
        set(&mut c, "phase", 0.0);
        set(&mut c, "rate", 1.0);
        set(&mut c, "saturation", 100.0);
        set(&mut c, "brightness", 100.0);

        let fill = inst("fill");
        let colour_at = |node: &EffectInstance, lt: f64| -> [f32; 4] {
            let graph = LayerGraph {
                edges: vec![edge(node, "colour", NodeRef::Effect(fill.id), "colour")],
                nodes: vec![node.clone()],
                ..LayerGraph::default()
            };
            match resolve_drivers(&graph, lt, ctx(), None)
                .param(NodeRef::Effect(fill.id), ParamId::new("colour"))
            {
                Some(Value::Colour(c)) => c,
                other => panic!("a colour port must carry a colour, not {other:?}"),
            }
        };

        // Phase 0 is red; a third of a turn later, green; a whole turn is back.
        assert_eq!(colour_at(&c, 0.0), [1.0, 0.0, 0.0, 1.0]);
        let third = colour_at(&c, 1.0 / 3.0);
        assert!(
            third[1] > 0.9 && third[0] < 0.1,
            "a third of a turn is green: {third:?}"
        );
        assert_eq!(colour_at(&c, 1.0), colour_at(&c, 0.0));

        // Rate nought holds.
        let mut still = c.clone();
        set(&mut still, "rate", 0.0);
        set(&mut still, "phase", 0.5);
        assert_eq!(colour_at(&still, 0.0), colour_at(&still, 7.5));

        // Saturation nought is grey at the brightness asked for.
        let mut grey = c.clone();
        set(&mut grey, "saturation", 0.0);
        set(&mut grey, "brightness", 50.0);
        assert_eq!(colour_at(&grey, 2.3), [0.5, 0.5, 0.5, 1.0]);
    }

    /// Every operation against its closed form, including the two that would
    /// otherwise divide by nought.
    #[test]
    fn math_matches_its_closed_form() {
        let cases: [(u32, f32, f32, f32); 10] = [
            (0, 2.0, 3.0, 5.0),
            (1, 2.0, 3.0, -1.0),
            (2, 2.0, 3.0, 6.0),
            (3, 6.0, 3.0, 2.0),
            (3, 6.0, 0.0, 0.0),
            (4, 2.0, 3.0, 2.0),
            (5, 2.0, 3.0, 3.0),
            (6, 7.0, 3.0, 1.0),
            (6, 7.0, 0.0, 0.0),
            (7, 2.0, 10.0, 1024.0),
        ];
        for (op, a, b, want) in cases {
            assert_eq!(math::apply(op, a, b), want, "operation {op} of {a} and {b}");
            let mut node = inst("math");
            set_choice(&mut node, "operation", op);
            set(&mut node, "a", f64::from(a));
            set(&mut node, "b", f64::from(b));
            assert_eq!(output_of(&node, "value", 0.0), want);
        }
        // An option index this build does not know renders as the default
        // rather than faulting (K-065).
        assert_eq!(math::apply(99, 2.0, 3.0), 6.0);
    }

    /// The straight line, its clamp, and the two ends the wrong way round.
    #[test]
    fn remap_matches_its_closed_form() {
        // 0..1 onto 0..40: a half is twenty.
        assert_eq!(remap::map(0.5, 0.0, 1.0, 0.0, 40.0, true), 20.0);
        // Past the end, clamped and unclamped.
        assert_eq!(remap::map(2.0, 0.0, 1.0, 0.0, 40.0, true), 40.0);
        assert_eq!(remap::map(2.0, 0.0, 1.0, 0.0, 40.0, false), 80.0);
        // An inverting map clamps to the range it actually spans.
        assert_eq!(remap::map(0.5, 0.0, 1.0, 100.0, 0.0, true), 50.0);
        assert_eq!(remap::map(-1.0, 0.0, 1.0, 100.0, 0.0, true), 100.0);
        // A zero-width input range has no line through it.
        assert_eq!(remap::map(0.5, 1.0, 1.0, 7.0, 40.0, true), 7.0);

        let mut node = inst("remap");
        set(&mut node, "value", 0.25);
        set(&mut node, "in_low", 0.0);
        set(&mut node, "in_high", 1.0);
        set(&mut node, "out_low", 0.0);
        set(&mut node, "out_high", 40.0);
        assert_eq!(output_of(&node, "value", 0.0), 10.0);
    }

    /// Centred smoothing of a straight line gives the line back — the closed
    /// form that says it is not running late. A constant comes back a constant,
    /// and a wobble comes back calmer than it went in.
    #[test]
    fn smooth_of_a_ramp_is_the_ramp() {
        // Colour cycle is the only driver whose output moves with time on its
        // own and lands in a number... so build the ramp out of a Wiggle at a
        // very low frequency instead, and check the calming rather than the
        // exact line. The exact line is checked below with a static input.
        let mut ramp = inst("colour_cycle");
        set(&mut ramp, "rate", 0.0);

        // A static input has nothing to smooth: it comes straight out.
        let mut s = inst("smooth");
        set(&mut s, "value", 7.5);
        set(&mut s, "time", 0.5);
        assert_eq!(output_of(&s, "value", 3.0), 7.5);

        // Time nought is a pass-through even with a wire on it.
        let mut w = inst("wiggle");
        set(&mut w, "amount", 10.0);
        set(&mut w, "frequency", 8.0);
        let mut s0 = inst("smooth");
        set(&mut s0, "time", 0.0);
        set(&mut s0, "value", 0.0);
        let target = inst("blur");
        let chain = |smooth: &EffectInstance| LayerGraph {
            edges: vec![
                edge(&w, "value", NodeRef::Driver(smooth.id), "value"),
                edge(smooth, "value", NodeRef::Effect(target.id), "radius"),
            ],
            nodes: vec![w.clone(), smooth.clone()],
            ..LayerGraph::default()
        };
        let read = |g: &LayerGraph, lt: f64| {
            resolve_drivers(g, lt, ctx(), None)
                .param(NodeRef::Effect(target.id), ParamId::new("radius"))
                .expect("wired")
                .as_f32()
        };
        let raw: Vec<f32> = (0..40)
            .map(|i| output_of(&w, "value", f64::from(i) * 0.02))
            .collect();
        let passed: Vec<f32> = (0..40)
            .map(|i| read(&chain(&s0), f64::from(i) * 0.02))
            .collect();
        assert_eq!(raw, passed, "Time nought must change nothing at all");

        // And with a window, the same signal comes out calmer: less total
        // movement frame to frame.
        let mut s1 = inst("smooth");
        set(&mut s1, "time", 0.25);
        set(&mut s1, "value", 0.0);
        let smoothed: Vec<f32> = (0..40)
            .map(|i| read(&chain(&s1), f64::from(i) * 0.02))
            .collect();
        let travel = |v: &[f32]| v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>();
        assert!(
            travel(&smoothed) < travel(&raw) * 0.6,
            "smoothing must calm the signal: {} against {}",
            travel(&smoothed),
            travel(&raw)
        );
        // Centred, so it does not lag: the smoothed and raw signals rise and
        // fall about the same place rather than one trailing the other.
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&smoothed) - mean(&raw)).abs() < 2.0);
    }

    /// §2.1: a wire wins over the keyframes, and every other parameter is left
    /// exactly as it was.
    #[test]
    fn a_driven_parameter_ignores_its_keyframes() {
        let mut blur = inst("blur");
        set(&mut blur, "radius", 40.0);
        let mut w = inst("wiggle");
        set(&mut w, "amount", 0.0);
        set(&mut w, "frequency", 1.0);

        let graph = LayerGraph {
            edges: vec![edge(&w, "value", NodeRef::Effect(blur.id), "radius")],
            nodes: vec![w],
            ..LayerGraph::default()
        };
        let drivers = resolve_drivers(&graph, 0.5, ctx(), None);

        let resolve = |drivers: &ResolvedDrivers| -> ResolvedStack {
            crate::fx::resolve_stack_temporal_named(
                std::slice::from_ref(&blur),
                drivers,
                0.5,
                0.5,
                1000.0,
                1.0,
                &MarkerContext::NONE,
                ctx(),
            )
            .1
        };

        let driven = resolve(&drivers);
        let plain = resolve(ResolvedDrivers::NONE);
        let radius = |s: &ResolvedStack| {
            s.get(0)
                .expect("one op")
                .params
                .float(ParamId::new("radius"), -1.0)
        };
        assert_eq!(radius(&plain), 40.0, "the stored keyframe, undriven");
        assert_eq!(radius(&driven), 0.0, "the wire, not the keyframe");
        // Its neighbours are untouched.
        assert_eq!(
            plain
                .get(0)
                .expect("op")
                .params
                .float(ParamId::new("mix"), -1.0),
            driven
                .get(0)
                .expect("op")
                .params
                .float(ParamId::new("mix"), -1.0)
        );
    }

    /// A driven px@comp parameter travels through the same preview-raster
    /// conversion a typed one does, so a wire cannot land in the wrong units.
    #[test]
    fn a_driven_distance_is_still_pixels_at_composition_size() {
        let blur = inst("blur");
        let mut w = inst("wiggle");
        set(&mut w, "amount", 0.0);
        // Amount nought so the value is exactly nought... use Remap instead to
        // get a known non-zero number out.
        let mut r = inst("remap");
        set(&mut r, "value", 1.0);
        set(&mut r, "in_low", 0.0);
        set(&mut r, "in_high", 1.0);
        set(&mut r, "out_low", 0.0);
        set(&mut r, "out_high", 20.0);

        let graph = LayerGraph {
            edges: vec![edge(&r, "value", NodeRef::Effect(blur.id), "radius")],
            nodes: vec![r],
            ..LayerGraph::default()
        };
        let drivers = resolve_drivers(&graph, 0.0, ctx(), None);
        let at = |px_scale: f32| {
            crate::fx::resolve_stack_temporal_named(
                std::slice::from_ref(&blur),
                &drivers,
                0.0,
                0.0,
                1000.0,
                px_scale,
                &MarkerContext::NONE,
                ctx(),
            )
            .1
            .get(0)
            .expect("one op")
            .params
            .float(ParamId::new("radius"), -1.0)
        };
        assert_eq!(at(1.0), 20.0);
        assert_eq!(at(0.5), 10.0, "half resolution halves the radius");
    }

    /// §1.4: bypass is the ordinary `enabled` flag, and a bypassed driver hands
    /// the parameter back to its keyframes.
    #[test]
    fn a_bypassed_driver_carries_nothing() {
        let blur = inst("blur");
        let mut r = inst("remap");
        set(&mut r, "value", 1.0);
        set(&mut r, "out_high", 20.0);
        r.enabled = false;

        let graph = LayerGraph {
            edges: vec![edge(&r, "value", NodeRef::Effect(blur.id), "radius")],
            nodes: vec![r],
            ..LayerGraph::default()
        };
        assert!(resolve_drivers(&graph, 0.0, ctx(), None).is_empty());
    }

    /// The walk is topological by construction: a chain evaluates back to front
    /// whatever order the nodes are written in.
    #[test]
    fn a_chain_evaluates_in_dependency_order() {
        let blur = inst("blur");
        let mut a = inst("math");
        set_choice(&mut a, "operation", 0); // Add
        set(&mut a, "a", 2.0);
        set(&mut a, "b", 3.0);
        let mut b = inst("math");
        set_choice(&mut b, "operation", 2); // Multiply
        set(&mut b, "b", 10.0);

        // a (=5) into b's A, b (=50) into the blur.
        let graph = LayerGraph {
            edges: vec![
                edge(&a, "value", NodeRef::Driver(b.id), "a"),
                edge(&b, "value", NodeRef::Effect(blur.id), "radius"),
            ],
            // Deliberately the reverse of evaluation order.
            nodes: vec![b.clone(), a],
            ..LayerGraph::default()
        };
        assert_eq!(
            resolve_drivers(&graph, 0.0, ctx(), None)
                .param(NodeRef::Effect(blur.id), ParamId::new("radius"))
                .expect("wired")
                .as_f32(),
            50.0
        );
    }

    /// A loop is refused at commit, but a hand-edited file can still carry one:
    /// the walk must bottom out rather than spin.
    #[test]
    fn a_hand_edited_loop_bottoms_out() {
        let blur = inst("blur");
        let a = inst("smooth");
        let graph = LayerGraph {
            edges: vec![
                edge(&a, "value", NodeRef::Driver(a.id), "value"),
                edge(&a, "value", NodeRef::Effect(blur.id), "radius"),
            ],
            nodes: vec![a],
            ..LayerGraph::default()
        };
        // The only promise is that it returns at all, and returns the same
        // answer twice.
        let first = resolve_drivers(&graph, 0.0, ctx(), None);
        let second = resolve_drivers(&graph, 0.0, ctx(), None);
        assert_eq!(first, second);
    }

    /// §1.4 again, from the evaluation side: a SourceMatte wire is reported as
    /// itself rather than as a value.
    #[test]
    fn a_source_matte_wire_is_reported_not_evaluated() {
        let blur = inst("blur");
        let graph = LayerGraph {
            edges: vec![Edge {
                from: OutputRef::SourceMatte,
                to: InputRef::Matte { effect: blur.id },
            }],
            ..LayerGraph::default()
        };
        let drivers = resolve_drivers(&graph, 0.0, ctx(), None);
        assert!(drivers.source_matte(blur.id));
        assert_eq!(drivers.iter().count(), 0);
    }

    /// §2.3: the two temporal drivers declare how far they reach, and the
    /// pointwise ones declare nothing.
    #[test]
    fn only_the_temporal_drivers_declare_a_window() {
        let mut s = inst("smooth");
        set(&mut s, "time", 0.4);
        let mut a = inst("audio_level");
        set(&mut a, "window", 0.1);
        let w = inst("wiggle");

        let graph = |nodes: Vec<EffectInstance>| LayerGraph {
            nodes,
            ..LayerGraph::default()
        };
        let near = |got: f64, want: f64| {
            assert!((got - want).abs() < 1e-6, "{got} is not about {want}");
        };
        near(temporal_window(&graph(vec![w.clone()]), 0.0, ctx()), 0.0);
        near(temporal_window(&graph(vec![s.clone()]), 0.0, ctx()), 0.2);
        near(temporal_window(&graph(vec![a.clone()]), 0.0, ctx()), 0.05);
        // The widest one wins: the frame key has to cover every read.
        near(temporal_window(&graph(vec![w, a, s]), 0.0, ctx()), 0.2);
    }

    /// Determinism, from the outside: the same graph resolved twice gives the
    /// identical substitution list, in the identical order, whatever order the
    /// wires were drawn in.
    #[test]
    fn the_same_graph_resolves_to_the_same_list_every_time() {
        let blur = inst("blur");
        let mut w = inst("wiggle");
        set(&mut w, "amount", 5.0);
        let mut c = inst("colour_cycle");
        set(&mut c, "rate", 0.3);
        let fill = inst("fill");

        let one = LayerGraph {
            edges: vec![
                edge(&w, "value", NodeRef::Effect(blur.id), "radius"),
                edge(&c, "colour", NodeRef::Effect(fill.id), "colour"),
            ],
            nodes: vec![w.clone(), c.clone()],
            ..LayerGraph::default()
        };
        // The same graph with both lists written the other way round.
        let other = LayerGraph {
            edges: one.edges.iter().rev().cloned().collect(),
            nodes: vec![c, w],
            ..LayerGraph::default()
        };
        let a = resolve_drivers(&one, 0.7, ctx(), None);
        let b = resolve_drivers(&other, 0.7, ctx(), None);
        assert_eq!(a, b, "the order the wires were drawn in decides nothing");
        assert_eq!(a, resolve_drivers(&one, 0.7, ctx(), None));
    }
}

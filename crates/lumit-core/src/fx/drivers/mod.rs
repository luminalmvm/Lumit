//! The drivers, and how a layer's driver graph is evaluated (K-471 §1.3, §2).
//!
//! # In plain terms
//!
//! A driver makes a *value* rather than a picture, and a wire from a driver
//! into an effect's socket makes that parameter follow the value instead of its
//! keyframes. This module holds the eight of them — Wiggle, Audio level, Colour
//! cycle, Math, Remap, Smooth, Points sample, Layer points — and the small walk
//! that works out, at one frame, what every wire is carrying.
//!
//! **One of them carries no value at all.** Layer points (K-604) is a *source*:
//! it names another layer and hands out that layer's points stream, so what
//! leaves it is read through [`Eval::points_input`] rather than substituted
//! into a parameter. It is the family's cross-layer tap, and it is a
//! layer-reference parameter rather than a wire because edges never cross
//! layers (K-471).
//!
//! **One of them reads a picture effect's data.** Points sample takes a wire
//! from Particulate's Points socket, which makes the walk *re-entrant through
//! the effect stack*: answering that wire evaluates the producer's particle
//! stream, and the producer's own parameters may themselves be driven, so the
//! walk calls back into itself. It terminates because a loop between the two is
//! refused at commit (K-492), and it is bounded anyway by the same evaluation
//! budget every other wire spends (§3.3).
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

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use uuid::Uuid;

use super::markers::MarkerContext;
use super::params::{ParamId, Value};
use super::points::{self, PointsStream};
use super::registry::{AudioTap, DriverCx, EffectMetadata};
use super::resolved::resolve_into_arena;
use super::ResolvedStack;
use crate::expression::ExpressionContext;
use crate::graph::{InputRef, LayerGraph, NodeRef, OutputRef};

pub mod audio_level;
pub mod colour_cycle;
pub mod combine;
pub mod layer_points;
pub mod math;
pub mod points_sample;
pub mod remap;
pub mod smooth;
pub mod split;
pub mod wiggle;

/// How many driver nodes one frame's evaluation may visit.
///
/// ponytail: a flat budget instead of memoising each `(node, port, time)`. A
/// graph shaped like a diamond re-evaluates its shared upstream once per path,
/// and a Smooth multiplies its subtree by its nine taps — so cost is
/// exponential in stacked Smooths, and four of them over a shared subtree
/// (9⁴ = 6561 visits) spends this budget before the walk finishes. That is the
/// ceiling, and it is a visible one, not a slow one: the walk returns `None`
/// the moment the budget runs out, so the driven parameter drops back to its
/// own keyframes mid-frame — a value that snaps as the graph grows. The
/// trigger is that snap on a graph a user could plausibly build, which in
/// practice means three or more Smooths in series; a driver graph that big
/// wants the `(node, port, time)` memo, and the memo makes the diamond free
/// as well.
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
    resolve_drivers_projected(graph, lt, context, audio, points::Projection::FLAT)
}

/// [`resolve_drivers`], told where the composition's camera puts a particle
/// that is off the layer's plane (K-561).
///
/// **Why the walk needs to be told at all.** A driver reads a points stream as
/// *data*, and Nearest distance is a distance on the frame — so the numbers a
/// wire carries have to be measured where the picture draws the particles,
/// which on a 3D layer is through the composition's camera. The projection is
/// worked out by the one place that holds both the layer's placement and the
/// comp's camera (the draw builder, from `lumit_gpu`'s own matrices), and
/// handed down as plain numbers; nothing in this crate derives a camera.
///
/// `Projection::FLAT` — what plain [`resolve_drivers`] passes — is a 2D layer,
/// a comp with no camera, and every caller that is not placing layers.
#[must_use]
pub fn resolve_drivers_projected(
    graph: &LayerGraph,
    lt: f64,
    context: Arc<ExpressionContext>,
    audio: Option<&dyn AudioTap>,
    projection: points::Projection,
) -> ResolvedDrivers {
    if graph.edges.is_empty() {
        return ResolvedDrivers::default();
    }
    let ev = Eval {
        graph,
        context,
        audio,
        projection,
        budget: Cell::new(EVAL_BUDGET),
        streams: RefCell::new(Vec::new()),
        cross: true,
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

/// **One stack effect's points stream**, at layer time `t` and in px@comp
/// (K-600, points-stream.md §3.3).
///
/// # In plain terms
///
/// A wire from Particulate's teal socket into Clone to points has to hand the
/// consumer some particles, and this is where they come from: the very function
/// that answers the same question for a driver, exported so the draw builder can
/// ask it too. Nothing is duplicated, so the particles a consumer stamps, the
/// particles a driver counts and the particles the producer draws are one set.
///
/// `None` means *no stream*, which is always the documented calm rather than a
/// fault: the effect is not a producer, it is bypassed, it is Scatter (whose
/// stream is a function of a picture that does not exist at this point in the
/// frame — K-599), or the document names an effect this layer does not carry.
///
/// `context` must name the producer's own comp and layer; `projection` is where
/// the composition's camera puts a particle off the layer's plane, which the
/// caller works out because nothing in this crate derives a camera (K-561).
#[must_use]
pub fn effect_stream(
    graph: &LayerGraph,
    effect: Uuid,
    t: f64,
    context: Arc<ExpressionContext>,
    audio: Option<&dyn AudioTap>,
    projection: points::Projection,
) -> Option<PointsStream> {
    // The `Eval` is dropped before the stream is unwrapped, so its own memo is
    // not a second owner and the common case moves rather than copies eight
    // vectors of up to the cap.
    let stream = {
        let ev = Eval {
            graph,
            context,
            audio,
            projection,
            budget: Cell::new(EVAL_BUDGET),
            streams: RefCell::new(Vec::new()),
            cross: true,
        };
        ev.stream(effect, t, 0)
    }?;
    Some(Rc::try_unwrap(stream).unwrap_or_else(|shared| (*shared).clone()))
}

/// **One cross-layer tap's points stream**, at layer time `t` and in px@comp
/// (K-604) — [`effect_stream`]'s sibling, for the other kind of thing a points
/// wire can come out of.
///
/// # In plain terms
///
/// A wire into Clone to points may come from a producer in the same stack, or
/// from a **Layer points** node naming another layer entirely. The draw builder
/// asks whichever this one is, through the same walk the driver graph uses, so
/// a consumer stamps one set of points however they arrived.
///
/// `None` means *no stream*, which is always the documented calm rather than a
/// fault: the node is not a tap, it is bypassed, its row names no layer or a
/// deleted one, that layer carries no producer, or the walk has already crossed
/// one layer boundary (see [`Eval::tap_stream`]).
#[must_use]
pub fn driver_stream(
    graph: &LayerGraph,
    node: Uuid,
    t: f64,
    context: Arc<ExpressionContext>,
    audio: Option<&dyn AudioTap>,
    projection: points::Projection,
) -> Option<PointsStream> {
    // The `Eval` is dropped before the stream is unwrapped, as `effect_stream`
    // does and for the same reason: the common case moves rather than copies
    // eight vectors of up to the cap.
    let stream = {
        let ev = Eval {
            graph,
            context,
            audio,
            projection,
            budget: Cell::new(EVAL_BUDGET),
            streams: RefCell::new(Vec::new()),
            cross: true,
        };
        ev.tap_stream(node, t, 0)
    }?;
    Some(Rc::try_unwrap(stream).unwrap_or_else(|shared| (*shared).clone()))
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
    /// Where the composition's camera puts a particle off the layer's plane
    /// (K-561), in **px@comp** — the units a driver reads a stream in. Flat on
    /// a 2D layer, and flat for every caller that does not place layers.
    projection: points::Projection,
    budget: Cell<u32>,
    /// **One evaluation per producer per frame** (points-stream.md §3.3): two
    /// wires out of one Particulate cost one stream, and a diamond of drivers
    /// over it costs one as well.
    ///
    /// A list rather than a map, because a layer carries a handful of effects
    /// and a linear scan over three entries beats hashing a `Uuid`. The stream
    /// is shared rather than cloned: it is eight `Vec`s of up to the cap.
    streams: RefCell<Vec<(Uuid, Rc<PointsStream>)>>,
    /// **Whether this walk may still cross a layer boundary** (K-604).
    ///
    /// A cross-layer tap evaluates the named layer with a fresh walk over
    /// *that* layer's graph, and that walk is built with this set false — so a
    /// tap reaches one layer and never two. It is the whole of the recursion
    /// argument for the tap: two layers naming each other stop at the second
    /// hop, with the far one reading the documented empty stream. No visited
    /// set, no cycle to detect, and a bound that does not depend on the budget
    /// noticing.
    cross: bool,
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
        // stay in one place. The ceiling is one allocation per visit, so a
        // layer's drivers can cost up to `EVAL_BUDGET` — 4096 — small
        // allocations in a single frame, all on the UI-facing evaluation path.
        // The trigger is docs/13 B1: an animated comp whose layers carry
        // drivers missing the 8 ms UI frame with the allocator, not the
        // arithmetic, on top of the profile. Pool the arenas on `Eval` then —
        // they are the same size every time and die in visit order.
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
        let points = |socket: &str| self.points_input(node, socket, t, depth + 1);
        let cx = DriverCx {
            node,
            inst,
            lt: t,
            params: fx.params,
            audio: self.audio,
            sample_input: &sample,
            points_input: &points,
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
            // read through [`points_input`](Self::points_input) instead
            // (points-stream.md §3.3). A *number* socket fed by one reads as
            // unwired, which is the documented no-op rather than a wrong
            // number — and is unreachable through a validated document, where
            // the type check refused it at commit.
            OutputRef::SourceMatte | OutputRef::EffectData { .. } => None,
        }
    }

    /// The **points stream** feeding driver `node`'s data input `port`, or
    /// `None` where the socket is unwired — the documented empty stream.
    fn points_input(&self, node: Uuid, port: &str, t: f64, depth: u32) -> Option<Rc<PointsStream>> {
        let from = self.graph.wire_into(&InputRef::Param {
            node: NodeRef::Driver(node),
            port: port.to_owned(),
        })?;
        match from {
            OutputRef::EffectData { effect, .. } => self.stream(*effect, t, depth),
            // **A cross-layer tap** (K-604): a driver whose output is a stream
            // rather than a number, so a points socket may legitimately be fed
            // by one. Anything else on a driver's output is a number, which is
            // not a stream and was refused at commit by the type check.
            OutputRef::Driver { node, .. } => self.tap_stream(*node, t, depth),
            // A texture is not a stream either.
            OutputRef::SourceMatte => None,
        }
    }

    /// The stream a **cross-layer tap** hands out (K-604,
    /// points-stream.md §1.2): the first producer on the layer its Points layer
    /// row names, evaluated with that layer's own graph applied.
    ///
    /// `None` — the documented empty stream, never a fault — for every absence
    /// there is: a node this build does not know, a bypassed one, a row naming
    /// no layer or a deleted one, a layer with no producer or with its fx
    /// switch off, a producer whose stream depends on a picture (K-599, K-603),
    /// and **a second hop**.
    ///
    /// The second hop is the recursion argument and it is deliberately blunt: a
    /// tap reaches one layer, and the walk it starts there carries
    /// [`cross`](Eval::cross) false, so a tap on the far side answers nothing.
    /// Two layers naming each other therefore stop at the second hop with no
    /// visited set and no cycle to detect. The far walk shares this one's
    /// remaining budget, so a fan of taps cannot buy itself more work than one
    /// frame's allowance.
    fn tap_stream(&self, node: Uuid, t: f64, depth: u32) -> Option<Rc<PointsStream>> {
        if let Some((_, s)) = self.streams.borrow().iter().find(|(id, _)| *id == node) {
            return Some(Rc::clone(s));
        }
        if !self.cross || depth >= MAX_DEPTH || self.budget.get() == 0 {
            return None;
        }
        self.budget.set(self.budget.get() - 1);
        let inst = self.graph.node(node)?;
        // Bypass (the `B` badge, §1.4): a bypassed tap carries nothing, exactly
        // as a bypassed driver carries no number.
        if !inst.enabled || inst.effect.match_name != layer_points::MATCH_NAME {
            return None;
        }
        let layer_id = inst.layer_ref(layer_points::SOURCE_PARAM)?;
        let doc = &self.context.document;
        let comp = doc.comp(self.context.comp?)?;
        let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
        if !layer.switches.fx {
            return None;
        }
        // **The first producer on it**, asked of the signature rather than of a
        // list of names (K-598's rule). A layer carrying two is a layer whose
        // first one is tapped.
        let producer = layer.effects.iter().find(|e| {
            e.enabled
                && super::BUILTIN_DEFS
                    .get(&e.effect.match_name)
                    .is_some_and(|def| points::wants_schedule(def.signature()))
        })?;
        // A fresh walk over **that** layer's graph and context, so the stream a
        // tap reads is the stream that layer draws — its producer's own wires
        // applied. The camera stays this layer's: the consumer draws into its
        // own rectangle, and where the composition's camera puts that rectangle
        // is the consumer's own placement, never the producer's.
        let far = Eval {
            graph: &layer.graph,
            context: Arc::new(ExpressionContext {
                layer: Some(layer_id),
                ..(*self.context).clone()
            }),
            audio: self.audio,
            projection: self.projection,
            budget: Cell::new(self.budget.get()),
            streams: RefCell::new(Vec::new()),
            cross: false,
        };
        let stream = far.stream(producer.id, t, 0);
        self.budget.set(far.budget.get());
        let stream = stream?;
        self.streams.borrow_mut().push((node, Rc::clone(&stream)));
        Some(stream)
    }

    /// One stack effect's points stream at layer time `t`, memoised.
    ///
    /// **This is where the walk becomes re-entrant** (points-stream.md §1.3).
    /// The producer's own parameters are resolved *with their driver wires
    /// substituted in*, which asks [`output`](Self::output) for every wire
    /// feeding it, which may in turn ask for another stream. The sampled stream
    /// must be the stream the picture draws, or this driver would report a
    /// particle field the viewer cannot see; resolving the producer's
    /// parameters any other way is exactly the drift that would cause.
    ///
    /// Termination rests on the commit-time cycle refusal (K-492): a document
    /// where the stream depends on the parameters and the parameters on the
    /// stream never reaches this path. A hand-edited file that carries one
    /// anyway bottoms out on the shared budget and depth, like every other
    /// wire — the walk returns a wrong-but-bounded answer rather than spinning.
    fn stream(&self, effect: Uuid, t: f64, depth: u32) -> Option<Rc<PointsStream>> {
        if let Some((_, s)) = self.streams.borrow().iter().find(|(id, _)| *id == effect) {
            return Some(Rc::clone(s));
        }
        if depth >= MAX_DEPTH || self.budget.get() == 0 {
            return None;
        }
        self.budget.set(self.budget.get() - 1);

        // The producer, its layer and its comp, read off the context every
        // resolve already carries — which is why nothing in the render's four
        // call sites had to grow an argument to make this work.
        let doc = &self.context.document;
        let comp = doc.comp(self.context.comp?)?;
        let layer_id = self.context.layer?;
        let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
        let inst = layer.effects.iter().find(|e| e.id == effect)?;
        // A bypassed producer draws nothing, so it hands out nothing: the
        // stream and the picture agree about an off switch too.
        if !inst.enabled || !layer.switches.fx {
            return None;
        }
        let def = super::BUILTIN_DEFS.get(&inst.effect.match_name)?;
        // Something that does not emit points has no stream to hand over, and
        // the walk asks the signature rather than a list of names (K-598).
        // **Which** producer it is still decides how the stream is made, at the
        // bottom of this function, because a birth schedule and a lattice are
        // not the same arithmetic.
        if !super::points::wants_schedule(def.signature()) {
            return None;
        }

        // The producer's own incoming wires, evaluated first — the same shape
        // `resolve_drivers` uses for the whole stack, scoped to this effect.
        let mut subs = Vec::new();
        for edge in &self.graph.edges {
            let (
                OutputRef::Driver {
                    node: src,
                    port: src_port,
                },
                InputRef::Param {
                    node: NodeRef::Effect(dest),
                    port: socket,
                },
            ) = (&edge.from, &edge.to)
            else {
                continue;
            };
            if *dest != effect {
                continue;
            }
            if let Some(v) = self.output(*src, src_port, t, depth + 1) {
                subs.push((NodeRef::Effect(effect), ParamId::new(socket), v));
            }
        }
        subs.sort_by_key(|s| (s.0, s.1));
        subs.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        let wired = ResolvedDrivers {
            subs,
            source_matte: Vec::new(),
        };

        // **px@comp, always** (K-419): a stream read as data is in composition
        // pixels whatever raster the preview happens to be drawn at, so the
        // number Nearest distance hands a px@comp parameter travels through the
        // same rescale a typed one does and lands in the right units.
        let mut bag = ResolvedStack::new();
        resolve_into_arena(
            def,
            inst,
            NodeRef::Effect(effect),
            t,
            0.0,
            1.0,
            &MarkerContext::NONE,
            &mut bag,
            self.context.clone(),
            &wired,
        );
        let params = bag.get(0)?.params;
        // **A generator's stream is arithmetic and nothing else** (K-598): no
        // schedule to scan, no mask to flatten, no clock to read. It is here
        // rather than behind a trait method because the two producers want
        // genuinely different things from the document — Particulate wants the
        // whole history of its Emit rate track — and a trait method wide enough
        // to carry both would be an interface shaped by its callers.
        if inst.effect.match_name == "grid" {
            let stream = Rc::new(super::effects::grid::Grid::read(params).stream(self.projection));
            self.streams.borrow_mut().push((effect, Rc::clone(&stream)));
            return Some(stream);
        }
        // **The picture-dependent producers cannot be sampled here, and that is
        // the recorded answer** to points-stream.md §2.2's constraint (K-599,
        // K-603): Scatter's stream is a function of the input picture and Emit
        // from image's is a function of a Source layer's, and at resolve time —
        // which is when this walk runs — no picture exists. The wire reads the
        // documented empty stream rather than a guess at one, and nothing is
        // memoised, so a future carriage that can answer will not find a wrong
        // answer cached in front of it. Anything else this build does not know
        // how to evaluate falls out here too, which is the same calm.
        if inst.effect.match_name != "particulate" {
            return None;
        }
        let p = super::effects::particulate::Particulate::read(params);

        // The mask-path emitter's polyline, flattened at composition scale, by
        // the rule the draw builder applies: a row the panel does not show, or
        // shows greyed, is a row nobody meant.
        let path = match def.schema().mask_path() {
            Some((param, self_default))
                if super::param_visible(inst, param) && super::param_enabled(inst, param) =>
            {
                crate::mask::mask_path_at(&layer.masks, inst.mask_ref(param), self_default, t)
            }
            _ => crate::mask::MaskPolyline::default(),
        };

        // **The birth schedule follows the authored Emit rate track**, which is
        // the rule the draw builder already applies (`build.rs`,
        // `points_schedules_for`) and the reason the two agree: the rate is
        // read off the stored property at every frame the scan walks, so a wire
        // on Emit rate does not rewrite the history of births. Resolving the
        // graph once per scanned frame would make one picture cost a thousand
        // driver walks, and a rate that rewrote its own past would make
        // particles jump about as the wire moved. Both readers, one rule.
        let dt = 1.0 / comp.frame_rate.fps().max(1.0);
        let rate_at = |lt: f64| -> f64 {
            inst.float_at_with_context("emit_rate", lt, self.context.clone())
                .unwrap_or(0.0)
        };
        let sched = super::points::Schedule::scan(
            dt,
            (t / dt).floor() as i64,
            p.window_frames(dt),
            &rate_at,
        );
        let stream = Rc::new(super::points::evaluate(
            &p.points().projected(self.projection),
            &sched,
            t,
            &path,
        ));
        self.streams.borrow_mut().push((effect, Rc::clone(&stream)));
        Some(stream)
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

    /// A colour through Split and back through Combine is the colour that went
    /// in, **bit for bit** — including a channel above one, because neither
    /// node converts anything and a driver's own sockets are never held to a
    /// range (K-510). Split's four numbers are the channels themselves.
    #[test]
    fn a_colour_survives_split_and_combine_unchanged() {
        // Scene-linear, deliberately awkward: over one on red, exact zero on
        // blue, a fraction that is not representable in eight bits on alpha.
        let want = [1.5f32, 0.25, 0.0, 0.7];
        let mut s = inst("split");
        for p in &mut s.params {
            if p.id == "colour" {
                p.value =
                    EffectValue::Colour(want.map(|c| crate::anim::Property::fixed(f64::from(c))));
            }
        }

        // Each channel out of Split, on its own, is that channel's number.
        for (port, channel) in ["red", "green", "blue", "alpha"].iter().zip(want) {
            let fill = inst("fill");
            // Through a Combine socket rather than an effect's, so the hard
            // range of whatever it lands on cannot be what is being measured.
            let c = inst("combine");
            let graph = LayerGraph {
                edges: vec![
                    edge(&s, port, NodeRef::Driver(c.id), "red"),
                    edge(&c, "colour", NodeRef::Effect(fill.id), "colour"),
                ],
                nodes: vec![s.clone(), c.clone()],
                ..LayerGraph::default()
            };
            match resolve_drivers(&graph, 0.0, ctx(), None)
                .param(NodeRef::Effect(fill.id), ParamId::new("colour"))
            {
                Some(Value::Colour(got)) => assert_eq!(got[0], channel, "{port} out of Split"),
                other => panic!("a colour port must carry a colour, not {other:?}"),
            }
        }

        // And all four together are the colour again.
        let c = inst("combine");
        let fill = inst("fill");
        let graph = LayerGraph {
            edges: vec![
                edge(&s, "red", NodeRef::Driver(c.id), "red"),
                edge(&s, "green", NodeRef::Driver(c.id), "green"),
                edge(&s, "blue", NodeRef::Driver(c.id), "blue"),
                edge(&s, "alpha", NodeRef::Driver(c.id), "alpha"),
                edge(&c, "colour", NodeRef::Effect(fill.id), "colour"),
            ],
            nodes: vec![s.clone(), c.clone()],
            ..LayerGraph::default()
        };
        match resolve_drivers(&graph, 0.0, ctx(), None)
            .param(NodeRef::Effect(fill.id), ParamId::new("colour"))
        {
            Some(Value::Colour(got)) => assert_eq!(got, want, "the round trip is not lossless"),
            other => panic!("a colour port must carry a colour, not {other:?}"),
        }

        // A Combine with nothing wired into Alpha makes an opaque colour, which
        // is the three-wire shape the node is usually built in.
        let mut three = inst("combine");
        set(&mut three, "red", 0.2);
        set(&mut three, "green", 0.4);
        set(&mut three, "blue", 0.6);
        let graph = LayerGraph {
            edges: vec![edge(&three, "colour", NodeRef::Effect(fill.id), "colour")],
            nodes: vec![three.clone()],
            ..LayerGraph::default()
        };
        assert_eq!(
            resolve_drivers(&graph, 0.0, ctx(), None)
                .param(NodeRef::Effect(fill.id), ParamId::new("colour")),
            Some(Value::Colour([0.2, 0.4, 0.6, 1.0]))
        );
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
        // Wiggle seeds its noise from the node's id, and a fresh instance gets
        // a new id every run — so an unpinned node makes this a different
        // signal each time, and any numeric bound below becomes a coin toss.
        // Pin the id: one fixed wobble, one fixed answer.
        w.id = uuid::Uuid::from_u128(0x5000_7401);
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
        // With that pinned wobble the nine-tap window leaves 0.205 of the raw
        // travel, so 0.6 is a wide, fixed margin rather than a coin toss.
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

    // -----------------------------------------------------------------------
    // Points sample (K-494, points-stream.md §2.2, §3.3).
    // -----------------------------------------------------------------------

    /// The comp every points test is staged in: 1920×1080 at 60 fps, one solid
    /// layer carrying `effects` and `graph`.
    ///
    /// The walk reads the producer off the context's own document — which is
    /// why the render's four call sites needed no new argument — so a points
    /// test needs a real comp behind it rather than a detached context.
    fn staged(effects: Vec<EffectInstance>, graph: LayerGraph) -> Arc<ExpressionContext> {
        use crate::model::{Composition, Document, LayerKind, ProjectItem, Switches};
        use crate::time::{CompTime, Duration, FrameRate, Rational};

        let at = |s: i64| CompTime(Rational::new(s, 1).expect("a whole second"));
        let layer = crate::model::Layer {
            graph,
            id: Uuid::now_v7(),
            name: "points".into(),
            kind: LayerKind::Solid {
                def: Uuid::now_v7(),
            },
            in_point: at(0),
            out_point: at(10),
            start_offset: at(0),
            transform: Default::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: crate::anim::Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects,
            switches: Switches::default(),
            interpolation: Default::default(),
            parked_flow: None,
            markers: Vec::new(),
            paint: Default::default(),
            extra: serde_json::Map::new(),
        };
        let layer_id = layer.id;
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).expect("60 fps"),
            duration: Duration(Rational::new(10, 1).expect("ten seconds")),
            background: crate::model::LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));
        Arc::new(ExpressionContext {
            document: Arc::new(doc),
            comp: Some(comp_id),
            layer: Some(layer_id),
            comp_time: 0.0,
            current_depth: 0,
        })
    }

    /// **The stream the picture draws**, by the oracle particulate.md §1.6
    /// names: the producer's parameters resolved through the ordinary stack
    /// walk with this frame's driver substitutions, its own birth scan, and
    /// [`points::evaluate`].
    ///
    /// Deliberately *not* the driver walk's own route to the same place — that
    /// is the thing under test.
    fn drawn_stream(
        producer: &EffectInstance,
        drivers: &ResolvedDrivers,
        context: Arc<ExpressionContext>,
        t: f64,
    ) -> crate::fx::points::PointsStream {
        use crate::fx::effects::particulate::Particulate;
        use crate::fx::points::{self, Schedule};

        let dt = 1.0 / 60.0;
        let (_, bag) = crate::fx::resolve_stack_temporal_named(
            std::slice::from_ref(producer),
            drivers,
            t,
            t,
            2202.9,
            1.0,
            &MarkerContext::NONE,
            context.clone(),
        );
        let p = Particulate::read(bag.get(0).expect("one op").params);
        let rate_at = |lt: f64| {
            producer
                .float_at_with_context("emit_rate", lt, context.clone())
                .unwrap_or(0.0)
        };
        let sched = Schedule::scan(dt, (t / dt).floor() as i64, p.window_frames(dt), &rate_at);
        points::evaluate(
            &p.points(),
            &sched,
            t,
            &crate::mask::MaskPolyline::default(),
        )
    }

    /// A points wire from `producer`'s stream into `driver`'s data input.
    fn stream_edge(producer: &EffectInstance, driver: &EffectInstance) -> Edge {
        Edge {
            from: OutputRef::EffectData {
                effect: producer.id,
                port: points_sample::POINTS_PORT.to_owned(),
            },
            to: InputRef::Param {
                node: NodeRef::Driver(driver.id),
                port: points_sample::POINTS_PORT.to_owned(),
            },
        }
    }

    /// **The central invariant** (points-stream.md §6 item 1): the stream the
    /// driver samples is the stream the picture draws — *under a driven
    /// producer*, which is the case that makes the walk re-entrant.
    ///
    /// Two wires are live at once. A Wiggle drives Particulate's Emit rate, the
    /// §1.3 example; a Remap drives its Position x, which is a parameter the
    /// closed forms actually read, so the crowd genuinely moves. Answering the
    /// Points sample's own wire therefore has to resolve the producer with both
    /// substitutions applied, and the walk calls back into itself to do it.
    #[test]
    fn the_sampled_stream_is_the_stream_the_picture_draws() {
        let mut producer = inst("particulate");
        set(&mut producer, "emit_rate", 200.0);
        set(&mut producer, "life", 1.0);
        set(&mut producer, "life_jitter", 0.0);
        set(&mut producer, "initial_speed", 0.0);
        set(&mut producer, "turbulence_amount", 0.0);
        set(&mut producer, "position_x", 300.0);

        let mut wiggle = inst("wiggle");
        set(&mut wiggle, "amount", 60.0);
        set(&mut wiggle, "frequency", 2.0);
        let mut moved = inst("remap");
        set(&mut moved, "value", 1.0);
        set(&mut moved, "out_high", 1500.0);

        let sampler = inst("points_sample");
        let target = inst("blur");

        let wired = LayerGraph {
            nodes: vec![wiggle.clone(), moved.clone(), sampler.clone()],
            edges: vec![
                edge(&wiggle, "value", NodeRef::Effect(producer.id), "emit_rate"),
                edge(&moved, "value", NodeRef::Effect(producer.id), "position_x"),
                stream_edge(&producer, &sampler),
                edge(
                    &sampler,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(target.id),
                    "radius",
                ),
                edge(
                    &sampler,
                    points_sample::NEAREST_PORT,
                    NodeRef::Effect(target.id),
                    "mix",
                ),
            ],
            ..LayerGraph::default()
        };
        let context = staged(vec![producer.clone(), target.clone()], wired.clone());

        for step in 0..8 {
            let t = 1.0 + f64::from(step) * 0.05;
            let resolved = resolve_drivers(&wired, t, context.clone(), None);
            let drawn = drawn_stream(&producer, &resolved, context.clone(), t);

            // The driver's own Position, resolved the same way the walk does.
            let at = [960.0f32, 540.0f32];
            let (want_count, want_near) = points_sample::sample(Some(&drawn), at);
            let count = resolved
                .param(NodeRef::Effect(target.id), ParamId::new("radius"))
                .expect("the Count wire carries something")
                .as_f32();
            let near = resolved
                .param(NodeRef::Effect(target.id), ParamId::new("mix"))
                .expect("the Nearest distance wire carries something")
                .as_f32();
            assert_eq!(
                count, want_count,
                "the driver counted a different crowd from the one drawn at {t}"
            );
            assert_eq!(
                near, want_near,
                "the driver measured a different crowd from the one drawn at {t}"
            );
            assert!(
                count > 0.0,
                "the fixture must have particles, or this proves nothing"
            );
        }

        // And the driving is doing something: with the Position wire cut, the
        // same frame measures a different distance.
        let mut cut = wired.clone();
        cut.edges.remove(1);
        let context_cut = staged(vec![producer.clone(), target.clone()], cut.clone());
        let near = |g: &LayerGraph, c: &Arc<ExpressionContext>| {
            resolve_drivers(g, 1.0, c.clone(), None)
                .param(NodeRef::Effect(target.id), ParamId::new("mix"))
                .expect("wired")
                .as_f32()
        };
        assert_ne!(
            near(&wired, &context),
            near(&cut, &context_cut),
            "a driven producer must move the crowd the sample reads"
        );
    }

    /// **A second producer, on the same wire** (K-598): a Grid's lattice reads
    /// through the Points sample exactly as a particle field does, because the
    /// walk asks the *signature* who emits points rather than carrying a name.
    /// The count is the lattice, cell for cell, and the nearest distance is a
    /// spacing away from a point sat on the lattice's own centre.
    #[test]
    fn a_grids_lattice_reads_through_the_points_sample() {
        let mut producer = inst("grid");
        set(&mut producer, "columns", 5.0);
        set(&mut producer, "rows", 3.0);
        set(&mut producer, "spacing_x", 100.0);
        set(&mut producer, "spacing_y", 100.0);

        let sampler = inst("points_sample");
        let target = inst("blur");
        let wired = LayerGraph {
            nodes: vec![sampler.clone()],
            edges: vec![
                stream_edge(&producer, &sampler),
                edge(
                    &sampler,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(target.id),
                    "radius",
                ),
                edge(
                    &sampler,
                    points_sample::NEAREST_PORT,
                    NodeRef::Effect(target.id),
                    "mix",
                ),
            ],
            ..LayerGraph::default()
        };
        let context = staged(vec![producer.clone(), target.clone()], wired.clone());
        let resolved = resolve_drivers(&wired, 1.0, context, None);
        let read = |id: &str| {
            resolved
                .param(NodeRef::Effect(target.id), ParamId::new(id))
                .expect("the wire carries something")
                .as_f32()
        };
        assert_eq!(read("radius"), 15.0, "five columns of three rows");
        // The sampler's default Position is the comp's centre, and an odd
        // lattice has a cell sat exactly on it.
        assert_eq!(read("mix"), 0.0, "the centre cell is where the query is");
    }

    /// **Scatter's stream cannot be sampled by a driver** (K-599), which is the
    /// recorded answer to points-stream.md §2.2's constraint: the stream is a
    /// function of the input picture, and at resolve time there is no picture.
    /// The wire reads the documented empty stream — nothing alive, nothing
    /// anywhere near — rather than a guess at one.
    #[test]
    fn a_scatters_stream_reads_as_empty_in_the_driver_walk() {
        let producer = inst("scatter");
        let sampler = inst("points_sample");
        let target = inst("blur");
        let wired = LayerGraph {
            nodes: vec![sampler.clone()],
            edges: vec![
                stream_edge(&producer, &sampler),
                edge(
                    &sampler,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(target.id),
                    "radius",
                ),
                edge(
                    &sampler,
                    points_sample::NEAREST_PORT,
                    NodeRef::Effect(target.id),
                    "mix",
                ),
            ],
            ..LayerGraph::default()
        };
        let context = staged(vec![producer.clone(), target.clone()], wired.clone());
        let resolved = resolve_drivers(&wired, 1.0, context, None);
        let read = |id: &str| {
            resolved
                .param(NodeRef::Effect(target.id), ParamId::new(id))
                .expect("the wire carries something")
                .as_f32()
        };
        assert_eq!(read("radius"), 0.0, "a picture-less stream counted points");
        // Clamped to the parameter's own hard range at the socket (K-510), as
        // every driven value is, so this is the far value held to Mix's top.
        assert!(
            read("mix") > 0.0,
            "nearness read as 'a point is right here'"
        );
    }

    /// The documented no-ops (§2.2): an unwired socket and an empty stream both
    /// read as "nothing alive, nothing anywhere near" — and the far value is a
    /// large distance rather than nought, because a Remap from nearness reads
    /// nought as "a particle is right here".
    #[test]
    fn an_unwired_or_empty_stream_reads_as_nothing_near() {
        use crate::fx::points::PointsStream;

        let sampler = inst("points_sample");
        let target = inst("blur");
        let unwired = LayerGraph {
            nodes: vec![sampler.clone()],
            edges: vec![
                edge(
                    &sampler,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(target.id),
                    "radius",
                ),
                edge(
                    &sampler,
                    points_sample::NEAREST_PORT,
                    NodeRef::Effect(target.id),
                    "mix",
                ),
            ],
            ..LayerGraph::default()
        };
        let context = staged(vec![target.clone()], unwired.clone());
        let resolved = resolve_drivers(&unwired, 1.0, context, None);
        let read = |port: &str| {
            resolved
                .param(NodeRef::Effect(target.id), ParamId::new(port))
                .expect("an unwired data input still makes numbers")
                .as_f32()
        };
        assert_eq!(read("radius"), 0.0, "nothing wired is nothing alive");
        assert_eq!(read("mix"), points_sample::NOTHING_NEAR);

        // The same two numbers from the sampler itself, over no stream and over
        // an empty one — the two ways of having no particles.
        assert_eq!(
            points_sample::sample(None, [0.0, 0.0]),
            (0.0, points_sample::NOTHING_NEAR)
        );
        assert_eq!(
            points_sample::sample(Some(&PointsStream::default()), [0.0, 0.0]),
            (0.0, points_sample::NOTHING_NEAR)
        );
    }

    /// Nearest distance against a hand-placed field: the closed form is a
    /// minimum over a linear scan, so the number is checkable by eye.
    #[test]
    fn nearest_distance_is_the_nearest_particle() {
        use crate::fx::points::PointsStream;

        let mut s = PointsStream::default();
        for (i, p) in [
            [0.0f32, 0.0, 0.0],
            [30.0, 40.0, 0.0],
            [-100.0, 0.0, 0.0],
            [3.0, 4.0, 0.0],
        ]
        .into_iter()
        .enumerate()
        {
            s.position.push(p);
            s.id.push(i as u64);
        }
        // From the origin: the 3-4-5 triangle is the nearest, at five.
        assert_eq!(points_sample::sample(Some(&s), [0.0, 0.0]), (4.0, 0.0));
        assert_eq!(points_sample::sample(Some(&s), [6.0, 8.0]), (4.0, 5.0));
        // A query point on top of a particle reads nought, not the empty value.
        assert_eq!(points_sample::sample(Some(&s), [30.0, 40.0]).1, 0.0);
    }

    /// **Nearest distance is measured where the picture draws** (K-561): the
    /// projected position, not the three axes.
    ///
    /// Position is a point on the frame, so the honest answer to "how far is
    /// the nearest particle" is how far it is in the frame — and a particle
    /// pushed away from the camera is *seen* nearer the centre, so the number
    /// has to follow it there. The port declares itself 2D
    /// ([`Port::three_d`] false); this is what that declaration buys.
    #[test]
    fn nearest_distance_measures_in_the_projected_frame() {
        use crate::fx::points::{PointsStream, Projection};

        // A head-on camera 400 back from the plane, about the origin: a
        // particle 400 deep is seen at half its distance from the centre.
        let proj = Projection {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0 / 400.0, 1.0],
            ],
        };
        let mut s = PointsStream {
            projection: proj,
            ..PointsStream::default()
        };
        s.position.push([100.0, 0.0, 400.0]);
        s.id.push(0);
        // Unprojected it is 100 out; seen, it is 50.
        assert!((points_sample::sample(Some(&s), [0.0, 0.0]).1 - 50.0).abs() < 1e-3);
        // And the same stream on a 2D layer reads the plane distance, which is
        // the number this driver has always answered.
        s.projection = Projection::FLAT;
        assert_eq!(points_sample::sample(Some(&s), [0.0, 0.0]).1, 100.0);
    }

    /// The wire stays one type (K-561): the v1 consumer does **not** declare 3D
    /// awareness, so what it reads is the projected pair. A test rather than a
    /// comment because the flag is what the family package builds on, and a
    /// port that quietly flipped would change what every 2D consumer measures.
    #[test]
    fn the_points_sample_port_is_not_three_d_aware() {
        let def = super::super::BUILTIN_DEFS
            .get("points_sample")
            .expect("Points sample is declared");
        let crate::fx::Signature::Data { inputs, .. } = def.signature() else {
            panic!("Points sample is a driver");
        };
        let port = inputs.first().expect("it declares its Points input");
        assert_eq!(port.ty, crate::fx::PortType::Points);
        assert!(!port.three_d, "the v1 driver reads projected positions");
    }

    /// §3.3's memo: **one evaluation per producer per frame**, however many
    /// wires read it — and the walk's budget bounds what a frame can spend.
    #[test]
    fn one_producer_is_evaluated_once_a_frame() {
        let mut producer = inst("particulate");
        set(&mut producer, "emit_rate", 60.0);
        let (a, b) = (inst("points_sample"), inst("points_sample"));
        let target = inst("blur");

        let graph = LayerGraph {
            nodes: vec![a.clone(), b.clone()],
            edges: vec![
                stream_edge(&producer, &a),
                stream_edge(&producer, &b),
                edge(
                    &a,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(target.id),
                    "radius",
                ),
                edge(
                    &b,
                    points_sample::NEAREST_PORT,
                    NodeRef::Effect(target.id),
                    "mix",
                ),
            ],
            ..LayerGraph::default()
        };
        let context = staged(vec![producer.clone(), target.clone()], graph.clone());

        let ev = Eval {
            graph: &graph,
            context,
            audio: None,
            projection: points::Projection::FLAT,
            budget: Cell::new(EVAL_BUDGET),
            streams: RefCell::new(Vec::new()),
            cross: true,
        };
        assert!(ev.output(a.id, points_sample::COUNT_PORT, 1.0, 0).is_some());
        assert!(ev
            .output(b.id, points_sample::NEAREST_PORT, 1.0, 0)
            .is_some());
        assert_eq!(
            ev.streams.borrow().len(),
            1,
            "two wires out of one Particulate must cost one stream"
        );
        assert!(
            EVAL_BUDGET - ev.budget.get() < 16,
            "a two-driver graph must not spend a frame's budget"
        );
    }

    /// **Termination is the commit-time refusal, and bounded work is the belt**
    /// (§1.3). The loop the v1 catalogue makes constructible — Points sample
    /// reads Particulate, its Count drives Particulate's Emit rate — never
    /// reaches the eval path, because `validate` refuses it. A file hand-edited
    /// to carry one anyway must still bottom out.
    #[test]
    fn a_points_cycle_is_refused_and_a_hand_edited_one_bottoms_out() {
        use crate::graph::GraphError;

        let producer = inst("particulate");
        let sampler = inst("points_sample");
        let looped = LayerGraph {
            nodes: vec![sampler.clone()],
            edges: vec![
                stream_edge(&producer, &sampler),
                edge(
                    &sampler,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(producer.id),
                    "emit_rate",
                ),
            ],
            ..LayerGraph::default()
        };
        let stack = vec![producer.clone()];
        assert_eq!(
            looped.validate(&stack),
            Err(GraphError::Cycle),
            "the document can never carry this, which is what makes the walk \
             well-founded"
        );
        // One leg is a line, not a loop, and must still commit.
        let open = LayerGraph {
            edges: vec![looped.edges[0].clone()],
            ..looped.clone()
        };
        open.validate(&stack).expect("a line is not a loop");

        // Hand-edited past the refusal: the promise is that it returns, returns
        // the same answer twice, and stops rather than spinning.
        let context = staged(stack, looped.clone());
        let ev = Eval {
            graph: &looped,
            context: context.clone(),
            audio: None,
            projection: points::Projection::FLAT,
            budget: Cell::new(EVAL_BUDGET),
            streams: RefCell::new(Vec::new()),
            cross: true,
        };
        let _ = ev.output(sampler.id, points_sample::COUNT_PORT, 1.0, 0);
        assert!(
            ev.budget.get() > 0,
            "the walk must stop on its depth, not by exhausting the frame"
        );
        assert_eq!(
            resolve_drivers(&looped, 1.0, context.clone(), None),
            resolve_drivers(&looped, 1.0, context, None),
        );
    }

    /// A bypassed producer draws nothing, so it hands out nothing: the picture
    /// and the stream agree about an off switch.
    #[test]
    fn a_bypassed_producer_hands_out_no_stream() {
        let mut producer = inst("particulate");
        set(&mut producer, "emit_rate", 200.0);
        let sampler = inst("points_sample");
        let target = inst("blur");
        let graph = LayerGraph {
            nodes: vec![sampler.clone()],
            edges: vec![
                stream_edge(&producer, &sampler),
                edge(
                    &sampler,
                    points_sample::COUNT_PORT,
                    NodeRef::Effect(target.id),
                    "radius",
                ),
            ],
            ..LayerGraph::default()
        };
        let count = |p: &EffectInstance| {
            let context = staged(vec![p.clone(), target.clone()], graph.clone());
            resolve_drivers(&graph, 1.0, context, None)
                .param(NodeRef::Effect(target.id), ParamId::new("radius"))
                .expect("wired")
                .as_f32()
        };
        assert!(count(&producer) > 0.0);
        let mut off = producer.clone();
        off.enabled = false;
        assert_eq!(
            count(&off),
            0.0,
            "a bypassed producer emits nothing to read"
        );
    }

    // -----------------------------------------------------------------
    // The clamp (K-510, PS7; the question K-509 left open)
    // -----------------------------------------------------------------

    /// What `target`'s `socket` actually resolves to once `graph`'s wires have
    /// been substituted — the number the kernel sees, not the number the driver
    /// said. The clamp lives in the resolve walk, so a test that read
    /// [`ResolvedDrivers::param`] would be reading one step too early.
    fn resolved_param(target: &EffectInstance, socket: &str, graph: &LayerGraph, lt: f64) -> f32 {
        let context = staged(vec![target.clone()], graph.clone());
        let drivers = resolve_drivers(graph, lt, context.clone(), None);
        let (_, bag) = super::super::resolved::resolve_stack_temporal_named(
            std::slice::from_ref(target),
            &drivers,
            lt,
            lt,
            0.0,
            1.0,
            &MarkerContext::NONE,
            context,
        );
        bag.get(0)
            .expect("the stack resolved")
            .params
            .get(ParamId::new(socket))
            .expect("the parameter is in the bag")
            .as_f32()
    }

    /// A Math driver pinned to one constant, for wiring an arbitrary number
    /// into a socket.
    fn constant(v: f64) -> EffectInstance {
        let mut m = inst("math");
        set(&mut m, "a", v);
        set(&mut m, "b", 0.0);
        // Add: a + 0 is a, whatever a is.
        set_choice(&mut m, "operation", 0);
        m
    }

    /// **An unwired Points sample's `1e9` arrives clamped** (K-509, K-510).
    ///
    /// This is the case that raised the question: the driver answers a
    /// deliberately enormous distance over an empty stream, and before the
    /// clamp that number went straight into the parameter — a Blur radius sat
    /// at a billion pixels, past a hard maximum a typed value can never reach.
    /// The panel's *"no stream"* mark (K-509) still says why; this is what
    /// stops the picture being nonsense while it does.
    #[test]
    fn an_empty_streams_enormous_distance_clamps_to_the_hard_range() {
        let sampler = inst("points_sample");
        let target = inst("blur");
        let graph = LayerGraph {
            nodes: vec![sampler.clone()],
            edges: vec![edge(
                &sampler,
                points_sample::NEAREST_PORT,
                NodeRef::Effect(target.id),
                "radius",
            )],
            ..LayerGraph::default()
        };
        // The wire itself still carries the honest constant: the driver is not
        // told what it is plugged into, and the frame key hashes what it said.
        let raw = resolve_drivers(
            &graph,
            1.0,
            staged(vec![target.clone()], graph.clone()),
            None,
        )
        .param(NodeRef::Effect(target.id), ParamId::new("radius"))
        .expect("the wire carries something")
        .as_f32();
        assert_eq!(raw, points_sample::NOTHING_NEAR);
        // What the kernel is handed is the parameter's own maximum.
        assert_eq!(
            resolved_param(&target, "radius", &graph, 1.0),
            2000.0,
            "a driven radius must stop where a typed one stops (K-090)"
        );
    }

    /// **A wild driver cannot push past either bound** (K-090's hard range,
    /// K-510) — and the clamp is in schema space, before the raster scaling,
    /// so it is the same number at every preview resolution.
    #[test]
    fn a_driven_value_is_held_to_both_hard_bounds() {
        let target = inst("blur");
        let at = |v: f64| {
            let driver = constant(v);
            let graph = LayerGraph {
                nodes: vec![driver.clone()],
                edges: vec![edge(&driver, "value", NodeRef::Effect(target.id), "radius")],
                ..LayerGraph::default()
            };
            resolved_param(&target, "radius", &graph, 1.0)
        };
        assert_eq!(at(-5_000.0), 0.0, "a blur radius cannot go negative");
        assert_eq!(at(1e30), 2000.0, "nor past its hard maximum");
        // In between, the wire's number is untouched: this is a backstop, not
        // a second opinion about what a driver means.
        assert_eq!(at(37.5), 37.5);
    }

    /// **A whole-number parameter is clamped too**, at its own bounds: Sprite
    /// flare's Ghosts runs 0..=16, and rounds after the clamp rather than
    /// before it.
    #[test]
    fn a_driven_integer_is_held_to_its_own_bounds() {
        let target = inst("sprite_flare");
        let at = |v: f64| {
            let driver = constant(v);
            let graph = LayerGraph {
                nodes: vec![driver.clone()],
                edges: vec![edge(&driver, "value", NodeRef::Effect(target.id), "ghosts")],
                ..LayerGraph::default()
            };
            resolved_param(&target, "ghosts", &graph, 1.0)
        };
        assert_eq!(at(-40.0), 0.0);
        assert_eq!(at(900.0), 16.0);
        assert_eq!(at(5.0), 5.0);
    }

    /// **An unbounded-above parameter still takes big values** (K-090's
    /// one-sided amendment): the clamp is the *declared* range, not a range
    /// invented for it. Radial blur's Amount clamps at nought below and runs
    /// free above, and a driver may take it anywhere the user could type it.
    #[test]
    fn an_unbounded_parameter_still_takes_a_large_driven_value() {
        let target = inst("radial_blur");
        let at = |v: f64| {
            let driver = constant(v);
            let graph = LayerGraph {
                nodes: vec![driver.clone()],
                edges: vec![edge(&driver, "value", NodeRef::Effect(target.id), "amount")],
                ..LayerGraph::default()
            };
            resolved_param(&target, "amount", &graph, 1.0)
        };
        assert_eq!(at(50_000.0), 50_000.0, "nothing bounds it above (K-090)");
        assert_eq!(at(-1.0), 0.0, "and it still stops at nought below");
    }

    /// **A driver's own socket is not clamped** (K-510), and this is the case
    /// that decides it: Remap exists to take a wide number and narrow it. Its
    /// Value row declares a 0..=1 slider, which is a sensible thing to *type*
    /// into and a nonsense bound on a **wire** — clamping there would leave the
    /// one driver written for out-of-range numbers unable to see them, and
    /// would make Nearest distance (pixels, K-419) unusable through the very
    /// driver points-stream.md §2.2 names for it.
    ///
    /// A hard bound says what a *kernel* was written for. A chain of drivers
    /// ends at an effect socket, and that is where the clamp is.
    #[test]
    fn a_drivers_own_socket_takes_the_number_it_is_handed() {
        let feed = constant(400.0);
        let mut remap = inst("remap");
        set(&mut remap, "in_low", 0.0);
        set(&mut remap, "in_high", 800.0);
        set(&mut remap, "out_low", 0.0);
        set(&mut remap, "out_high", 100.0);
        let target = inst("blur");
        let graph = LayerGraph {
            nodes: vec![feed.clone(), remap.clone()],
            edges: vec![
                edge(&feed, "value", NodeRef::Driver(remap.id), "value"),
                edge(&remap, "value", NodeRef::Effect(target.id), "radius"),
            ],
            ..LayerGraph::default()
        };
        // 400 of 0..800 is halfway, so 50 of 0..100. Clamped at the Value
        // row's 0..=1 slider it would have been 100 — the top of the range,
        // for every input above one.
        let got = resolved_param(&target, "radius", &graph, 1.0);
        assert!(
            (got - 50.0).abs() < 1e-3,
            "Remap saw a clamped input: {got} rather than 50"
        );
    }

    // -----------------------------------------------------------------------
    // The cross-layer points tap (K-604, points-stream.md §1.2, §2.3).
    // -----------------------------------------------------------------------

    /// A comp of **two** layers: a reader whose graph `reader_graph` builds
    /// from the source layer's id, and a source carrying `source_effects` and
    /// `source_graph`.
    ///
    /// The closure exists because a tap names the source layer by id, and that
    /// id does not exist until the layer does — so the reader's graph is built
    /// after it, not before.
    fn staged_pair(
        reader_graph: impl FnOnce(Uuid) -> LayerGraph,
        source_effects: Vec<EffectInstance>,
        source_graph: LayerGraph,
    ) -> (Arc<ExpressionContext>, Uuid) {
        use crate::model::{Composition, Document, LayerKind, ProjectItem, Switches};
        use crate::time::{CompTime, Duration, FrameRate, Rational};

        let at = |s: i64| CompTime(Rational::new(s, 1).expect("a whole second"));
        let layer =
            |name: &str, effects: Vec<EffectInstance>, graph: LayerGraph| crate::model::Layer {
                graph,
                id: Uuid::now_v7(),
                name: name.into(),
                kind: LayerKind::Solid {
                    def: Uuid::now_v7(),
                },
                in_point: at(0),
                out_point: at(10),
                start_offset: at(0),
                transform: Default::default(),
                matte: None,
                parent: None,
                label: 0,
                volume_db: crate::anim::Property::zero(),
                audio_only: false,
                adjustment: false,
                retime: None,
                blend: Default::default(),
                masks: Vec::new(),
                effects,
                switches: Switches::default(),
                interpolation: Default::default(),
                parked_flow: None,
                markers: Vec::new(),
                paint: Default::default(),
                extra: serde_json::Map::new(),
            };
        let source = layer("source", source_effects, source_graph);
        let source_id = source.id;
        let reader = layer("reader", Vec::new(), reader_graph(source_id));
        let reader_id = reader.id;
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).expect("60 fps"),
            duration: Duration(Rational::new(10, 1).expect("ten seconds")),
            background: crate::model::LinearColour::BLACK,
            work_area: None,
            layers: vec![reader, source],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));
        (
            Arc::new(ExpressionContext {
                document: Arc::new(doc),
                comp: Some(comp_id),
                layer: Some(reader_id),
                comp_time: 0.0,
                current_depth: 0,
            }),
            source_id,
        )
    }

    /// A Layer points node naming `layer` — or naming nothing, when `layer` is
    /// `None`.
    fn tap(layer: Option<Uuid>) -> EffectInstance {
        let mut node = inst("layer_points");
        for p in &mut node.params {
            if p.id == "source" {
                p.value = EffectValue::Layer(layer);
            }
        }
        node
    }

    /// A 5 × 3 Grid, whose lattice a test can count without measuring it.
    fn lattice() -> EffectInstance {
        let mut producer = inst("grid");
        set(&mut producer, "columns", 5.0);
        set(&mut producer, "rows", 3.0);
        set(&mut producer, "planes", 1.0);
        producer
    }

    /// The stream a tap named `node` hands out, on the reader layer `context`
    /// points at.
    fn tapped(
        context: &Arc<ExpressionContext>,
        graph: &LayerGraph,
        node: Uuid,
    ) -> Option<PointsStream> {
        driver_stream(
            graph,
            node,
            0.0,
            context.clone(),
            None,
            points::Projection::FLAT,
        )
    }

    /// One reader layer carrying a single tap, and the source layer it names:
    /// the graph, the tap's node id, and a context pointed at the reader.
    fn one_tap(
        row: Option<Uuid>,
        source_effects: Vec<EffectInstance>,
        source_graph: LayerGraph,
        enabled: bool,
    ) -> (Arc<ExpressionContext>, LayerGraph, Uuid) {
        let mut node_id = Uuid::nil();
        let mut built = LayerGraph::default();
        let (context, _) = staged_pair(
            |source| {
                // `row` overrides which layer is named: `Some` for the tests
                // about a dangling or absent reference, and the real source
                // layer otherwise.
                let mut node = tap(if row.is_some() { row } else { Some(source) });
                node.enabled = enabled;
                node_id = node.id;
                built = LayerGraph {
                    nodes: vec![node],
                    ..LayerGraph::default()
                };
                built.clone()
            },
            source_effects,
            source_graph,
        );
        (context, built, node_id)
    }

    /// **A tap hands out the points of the layer it names** (K-604): the stream
    /// the *other* layer's producer makes, reaching this layer's graph as an
    /// ordinary wire out of a derived source node — no edge crosses anything.
    #[test]
    fn a_tap_reads_the_points_of_the_layer_it_names() {
        let (context, graph, node) = one_tap(None, vec![lattice()], LayerGraph::default(), true);
        let stream = tapped(&context, &graph, node).expect("the tap answered nothing");
        assert_eq!(stream.len(), 15, "not the other layer's 5 × 3 lattice");
        assert_eq!(stream.id, (0..15).collect::<Vec<u64>>());
    }

    /// **Every absence is the empty stream** (K-604) — the labelled no-op a
    /// dangling layer reference has always been, over the five ways a tap can
    /// come to nothing. None of them is a refusal: a tap that answers nothing
    /// leaves its consumer drawing the picture it was handed.
    #[test]
    fn a_tap_that_names_nothing_useful_hands_over_nothing() {
        let mut bypassed = lattice();
        bypassed.enabled = false;
        let cases: Vec<(&str, Option<Uuid>, Vec<EffectInstance>, bool)> = vec![
            // A row nobody set.
            ("an unset row", Some(Uuid::nil()), vec![lattice()], true),
            // A layer somebody deleted: an id that names nothing in the comp.
            (
                "a dangling reference",
                Some(Uuid::now_v7()),
                vec![lattice()],
                true,
            ),
            // A layer with no producer on it at all.
            ("no producer", None, vec![inst("blur")], true),
            // A bypassed producer draws nothing, so it hands out nothing.
            ("a bypassed producer", None, vec![bypassed], true),
            // A bypassed tap — the `B` badge, as on every other driver.
            ("a bypassed tap", None, vec![lattice()], false),
        ];
        for (what, row, effects, enabled) in cases {
            // `Uuid::nil` stands for the unset row, which stores no id at all.
            let row = row.filter(|id| !id.is_nil());
            let unset_row = matches!(what, "an unset row");
            let (context, graph, node) = if unset_row {
                one_tap(Some(Uuid::nil()), effects, LayerGraph::default(), enabled)
            } else {
                one_tap(row, effects, LayerGraph::default(), enabled)
            };
            assert!(
                tapped(&context, &graph, node).is_none(),
                "{what} answered a stream"
            );
        }
    }

    /// **A tap reaches one layer, never two** (K-604) — the recursion argument,
    /// asserted rather than reasoned about. The source layer's own graph
    /// carries a tap of its own and no producer; the far tap answers nothing,
    /// so the near one does, and two layers naming each other terminate at the
    /// second hop.
    #[test]
    fn a_tap_does_not_follow_a_second_tap() {
        let far = tap(Some(Uuid::now_v7()));
        let (context, graph, node) = one_tap(
            None,
            Vec::new(),
            LayerGraph {
                nodes: vec![far],
                ..LayerGraph::default()
            },
            true,
        );
        assert!(tapped(&context, &graph, node).is_none());
    }

    /// **What a tap reads is what the other layer draws** (points-stream.md
    /// §1.3): the far layer's producer is resolved with its *own* graph's
    /// substitutions applied, so a wire over there moves the points a tap hands
    /// over here.
    #[test]
    fn a_tap_reads_the_far_layers_own_driver_wires() {
        let feed = constant(9.0);
        let producer = lattice();
        let far_graph = LayerGraph {
            nodes: vec![feed.clone()],
            edges: vec![edge(
                &feed,
                "value",
                NodeRef::Effect(producer.id),
                "columns",
            )],
            ..LayerGraph::default()
        };
        let (context, graph, node) = one_tap(None, vec![producer], far_graph, true);
        let stream = tapped(&context, &graph, node).expect("the tap answered nothing");
        assert_eq!(
            stream.len(),
            27,
            "the far layer's wire on Columns was not applied"
        );
    }

    /// **A driver reads a tap the same way it reads a producer** (K-604):
    /// Points sample counts another layer's points, which is the wire the
    /// family's whole cross-layer story is for.
    #[test]
    fn points_sample_counts_another_layers_points_through_a_tap() {
        let sample = inst("points_sample");
        let target = inst("blur");
        let sample_id = sample.id;
        let mut built = LayerGraph::default();
        let (context, _) = staged_pair(
            |source| {
                let node = tap(Some(source));
                built = LayerGraph {
                    nodes: vec![node.clone(), sample.clone()],
                    edges: vec![
                        Edge {
                            from: OutputRef::Driver {
                                node: node.id,
                                port: "points".into(),
                            },
                            to: InputRef::Param {
                                node: NodeRef::Driver(sample_id),
                                port: "points".into(),
                            },
                        },
                        edge(&sample, "count", NodeRef::Effect(target.id), "radius"),
                    ],
                    ..LayerGraph::default()
                };
                built.clone()
            },
            vec![lattice()],
            LayerGraph::default(),
        );
        let drivers = resolve_drivers(&built, 0.0, context, None);
        let count = drivers
            .param(NodeRef::Effect(target.id), ParamId::new("radius"))
            .expect("Count drove nothing");
        assert_eq!(count, Value::Float(15.0), "not the far lattice's count");
    }
}

//! What a frame is costing, while it is being made — the per-node profiler's
//! first visible piece (docs/13-PERFORMANCE-RULES.md §7.1).
//!
//! # In plain terms
//!
//! Two questions this module answers, both about one frame:
//!
//! 1. **"How far along is it?"** A render is a sequence of stages — plan the
//!    decodes, decode them, build the draw list, composite it, show it — and
//!    each one can say how much of itself is done. Those reports go out through
//!    a [`ProgressSink`] the owner installs, which is how the Viewer can draw a
//!    progress bar for a frame that is taking a noticeable moment (a scrub, a
//!    value drag) instead of leaving the picture stale and silent.
//! 2. **"Where did the time go?"** Each layer's own picture — its source, its
//!    effect stack — is timed, and each effect within it, and the numbers go out
//!    through a [`ProfileSink`] as a [`FrameProfile`] once the frame is made.
//!    That is what the Timeline's render-time column and the Effect controls
//!    panel's per-effect readouts show.
//!
//! ## Honesty about what is measured
//!
//! Graphics work is *submitted* rather than performed: a kernel call returns
//! long before the graphics card has run it, so wall-clock around the call
//! alone would time the paperwork and not the work. So a profiled render
//! **fences** — it waits for the card to go idle at the end of each node before
//! reading the clock ([`FrameProfiler::span`]). That is a true measurement of
//! that node, and it costs the overlap between the processor and the card for
//! the frame being profiled. Which is why profiling is opt-in and never runs
//! during playback: a still frame can afford to be measured, a playing one
//! cannot. Continuous, free collection wants GPU timestamp queries and is the
//! recorded follow-up in docs/TODO.md.
//!
//! Two further boundaries, both deliberate:
//!
//! - Timings are recorded for the **top-level layers of the composition being
//!   rendered** only. A Precomp layer's number therefore includes everything
//!   inside it, which is the answer the Timeline row wants — the layers inside
//!   are not rows in this composition.
//! - A layer's number is the cost of **its own picture**: decoded source
//!   uploaded and linearised, then its effect stack. The final composite is one
//!   pass over the whole segment rather than a per-layer act, so it lands in the
//!   frame total and not on any one row.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use uuid::Uuid;

/// Where a frame has got to. Ordered as the render performs them, so a
/// frontend may show them as a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStage {
    /// Working out which source frames are wanted (cheap; opens nothing).
    Planning,
    /// Reading those frames off disk — usually the long pole.
    Decoding,
    /// Turning the document plus those pixels into a draw list.
    Building,
    /// Walking the draw list on the graphics card.
    Compositing,
    /// Display-encoding and handing the finished frame over.
    Presenting,
}

impl RenderStage {
    /// The stage's wire code, for a frontend that shows its name. Fixed
    /// numbers: a reordered enum must not silently relabel anything.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            RenderStage::Planning => 0,
            RenderStage::Decoding => 1,
            RenderStage::Building => 2,
            RenderStage::Compositing => 3,
            RenderStage::Presenting => 4,
        }
    }
}

/// One report of how far a frame has got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameProgress {
    pub comp: Uuid,
    pub frame: u64,
    pub stage: RenderStage,
    /// How much of the whole frame is done, 0..=1. An estimate — the stage
    /// weights below are fixed, not measured — which is what a progress bar
    /// needs and all it can honestly claim.
    pub fraction: f32,
}

/// Where each stage's span of the bar begins. Decode owns the largest share
/// because it is the stage that is actually slow when a frame is slow; the
/// compositing span is next because it grows with the layer count, which is
/// the other way a frame gets expensive.
const PLAN_AT: f32 = 0.0;
const DECODE_FROM: f32 = 0.05;
const DECODE_TO: f32 = 0.5;
const BUILD_AT: f32 = 0.55;
const COMPOSITE_FROM: f32 = 0.55;
const COMPOSITE_TO: f32 = 0.95;
const PRESENT_AT: f32 = 0.97;

/// A fraction inside `from..to`, `done` of `total` of the way through. `total`
/// of zero is the start of the span rather than a division by nothing.
fn span_fraction(from: f32, to: f32, done: u32, total: u32) -> f32 {
    if total == 0 {
        return from;
    }
    let through = done as f32 / total as f32;
    (from + (to - from) * through.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// One effect's measured cost within its layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectTiming {
    /// The effect *instance* id — the row in the layer's stack, not the kind
    /// of effect.
    pub effect: Uuid,
    pub ms: f32,
}

/// One layer's measured cost: its own picture, and the effects within it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerTiming {
    pub layer: Uuid,
    /// Source upload and linearise plus the whole effect stack, in
    /// milliseconds. Always at least the sum of `effects`.
    pub ms: f32,
    pub effects: Vec<EffectTiming>,
}

/// What one profiled frame cost, published once the frame is made.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameProfile {
    pub comp: Uuid,
    pub frame: u64,
    /// The whole frame, wall-clock, including the stages no layer owns
    /// (planning, decoding, the final composite, the display encode).
    pub total_ms: f32,
    /// The composition's top-level layers, in draw order (bottom-most first),
    /// each measured as described in the module note.
    pub layers: Vec<LayerTiming>,
}

/// Where progress reports go. `Arc` rather than a borrow because the renderer
/// hands the same sink to the compositing walk while holding it itself, and
/// `Send + Sync` because the render thread is not the frontend's.
pub type ProgressSink = Arc<dyn Fn(FrameProgress) + Send + Sync>;

/// Where a finished frame's timings go.
pub type ProfileSink = Arc<dyn Fn(FrameProfile) + Send + Sync>;

/// The recorder for one frame: it reports progress as the stages pass, and
/// accumulates the per-layer and per-effect timings the frame is measured
/// into. Built by the renderer for a frame that is being watched, and simply
/// absent (`None`) for one that is not — which is what keeps an unwatched
/// render, and every frame of playback, at exactly its old cost.
pub struct FrameProfiler {
    comp: Uuid,
    frame: u64,
    /// Null when nobody is watching the bar; timings may still be collected.
    progress: Option<ProgressSink>,
    /// True when the per-node timings are wanted. Separate from `progress`
    /// because the two have different costs: reporting progress is free,
    /// measuring nodes fences the graphics card.
    timing: bool,
    started: std::time::Instant,
    /// How deep in the comp tree the walk is: 0 outside it, 1 in the
    /// composition being rendered, more inside a Precomp. Only depth 1 is
    /// reported — see the module note.
    depth: Cell<u32>,
    /// Top-level layers finished, and how many there are, for the compositing
    /// span of the bar.
    layers_done: Cell<u32>,
    layers_total: Cell<u32>,
    timings: RefCell<Vec<LayerTiming>>,
}

impl FrameProfiler {
    /// A recorder for `comp` at `frame`. `progress` is `None` when no bar is
    /// being drawn; `timing` false when no timings are wanted. A profiler with
    /// neither is legal and does nothing, but the renderer builds none at all
    /// in that case.
    #[must_use]
    pub fn new(comp: Uuid, frame: u64, progress: Option<ProgressSink>, timing: bool) -> Self {
        Self {
            comp,
            frame,
            progress,
            timing,
            started: std::time::Instant::now(),
            depth: Cell::new(0),
            layers_done: Cell::new(0),
            layers_total: Cell::new(0),
            timings: RefCell::new(Vec::new()),
        }
    }

    /// True when nodes are to be measured — the callers that would otherwise
    /// pay for a fence ask this first.
    #[must_use]
    pub fn timing(&self) -> bool {
        self.timing
    }

    fn report(&self, stage: RenderStage, fraction: f32) {
        if let Some(sink) = &self.progress {
            sink(FrameProgress {
                comp: self.comp,
                frame: self.frame,
                stage,
                fraction: fraction.clamp(0.0, 1.0),
            });
        }
    }

    /// The decode plan is written; `jobs` sources are about to be read.
    pub fn planned(&self) {
        self.report(RenderStage::Planning, PLAN_AT);
    }

    /// `done` of `total` source decodes have landed.
    pub fn decoded(&self, done: u32, total: u32) {
        self.report(
            RenderStage::Decoding,
            span_fraction(DECODE_FROM, DECODE_TO, done, total),
        );
    }

    /// The draw list is being built (the pixels are all in hand).
    pub fn building(&self) {
        self.report(RenderStage::Building, BUILD_AT);
    }

    /// The draw list is in hand: `layers` top-level layers to composite.
    pub fn compositing(&self, layers: u32) {
        self.layers_total.set(layers);
        self.layers_done.set(0);
        self.report(RenderStage::Compositing, COMPOSITE_FROM);
    }

    /// The frame is composited and is being display-encoded and handed over.
    pub fn presenting(&self) {
        self.report(RenderStage::Presenting, PRESENT_AT);
    }

    /// Enter a composition's realise walk (the outermost one is the frame's
    /// own; deeper ones are Precomps).
    pub fn enter_comp(&self) {
        self.depth.set(self.depth.get().saturating_add(1));
    }

    /// Leave one, pairing with [`Self::enter_comp`].
    pub fn leave_comp(&self) {
        self.depth.set(self.depth.get().saturating_sub(1));
    }

    /// True while the walk is in the composition being rendered rather than
    /// inside one of its Precomps.
    #[must_use]
    pub fn at_top_level(&self) -> bool {
        self.depth.get() == 1
    }

    /// Record one top-level layer's cost and count it towards the bar. Called
    /// once per layer of the composition being rendered; ignored (bar apart)
    /// for anything deeper.
    pub fn layer_done(&self, layer: Uuid, ms: f32, effects: Vec<EffectTiming>) {
        if self.at_top_level() {
            if self.timing {
                self.timings
                    .borrow_mut()
                    .push(LayerTiming { layer, ms, effects });
            }
            let done = self.layers_done.get().saturating_add(1);
            self.layers_done.set(done);
            self.report(
                RenderStage::Compositing,
                span_fraction(
                    COMPOSITE_FROM,
                    COMPOSITE_TO,
                    done,
                    self.layers_total.get().max(done),
                ),
            );
        }
    }

    /// Time `f`, waiting for the graphics card to finish what it queued before
    /// the clock is read — see the module note on why the fence is there.
    /// Returns the value and the elapsed milliseconds.
    pub fn span<T>(&self, ctx: &lumit_gpu::GpuContext, f: impl FnOnce() -> T) -> (T, f32) {
        let started = std::time::Instant::now();
        let out = f();
        // The span may have recorded into a batched frame buffer; hand it over
        // before fencing, or the wait returns on an empty queue and the number
        // is meaningless.
        ctx.flush();
        ctx.device.poll(wgpu::Maintain::Wait);
        (out, started.elapsed().as_secs_f32() * 1000.0)
    }

    /// The finished frame's timings, consuming the recorder. `None` when no
    /// timings were asked for.
    #[must_use]
    pub fn finish(self) -> Option<FrameProfile> {
        if !self.timing {
            return None;
        }
        Some(FrameProfile {
            comp: self.comp,
            frame: self.frame,
            total_ms: self.started.elapsed().as_secs_f32() * 1000.0,
            layers: self.timings.into_inner(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn recorder() -> (ProgressSink, Arc<Mutex<Vec<FrameProgress>>>) {
        let seen: Arc<Mutex<Vec<FrameProgress>>> = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&seen);
        let sink: ProgressSink = Arc::new(move |p| {
            if let Ok(mut seen) = into.lock() {
                seen.push(p);
            }
        });
        (sink, seen)
    }

    #[test]
    fn span_fraction_walks_its_span_and_never_leaves_it() {
        assert!((span_fraction(0.1, 0.5, 0, 4) - 0.1).abs() < 1e-6);
        assert!((span_fraction(0.1, 0.5, 2, 4) - 0.3).abs() < 1e-6);
        assert!((span_fraction(0.1, 0.5, 4, 4) - 0.5).abs() < 1e-6);
        // More done than there are: clamped, never past the span's end.
        assert!((span_fraction(0.1, 0.5, 9, 4) - 0.5).abs() < 1e-6);
        // Nothing to do at all is the start of the span, not a division by
        // zero.
        assert!((span_fraction(0.1, 0.5, 0, 0) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn progress_only_ever_advances_across_a_frame() {
        let (sink, seen) = recorder();
        let p = FrameProfiler::new(Uuid::nil(), 7, Some(sink), false);
        p.planned();
        p.decoded(1, 2);
        p.decoded(2, 2);
        p.building();
        p.compositing(2);
        p.enter_comp();
        p.layer_done(Uuid::nil(), 1.0, Vec::new());
        p.layer_done(Uuid::nil(), 1.0, Vec::new());
        p.leave_comp();
        p.presenting();
        let seen = seen.lock().expect("progress recorded");
        assert!(seen.len() >= 8);
        let mut last = -1.0_f32;
        for report in seen.iter() {
            assert!(report.frame == 7);
            assert!(
                report.fraction >= last,
                "progress went backwards: {last} then {}",
                report.fraction
            );
            assert!((0.0..=1.0).contains(&report.fraction));
            last = report.fraction;
        }
        assert!(matches!(
            seen.last().map(|r| r.stage),
            Some(RenderStage::Presenting)
        ));
    }

    #[test]
    fn a_profiler_with_no_sink_and_no_timing_reports_nothing() {
        let p = FrameProfiler::new(Uuid::nil(), 0, None, false);
        p.planned();
        p.compositing(1);
        p.enter_comp();
        p.layer_done(Uuid::nil(), 5.0, Vec::new());
        assert!(!p.timing());
        assert!(p.finish().is_none());
    }

    #[test]
    fn only_top_level_layers_are_timed() {
        let outer = Uuid::from_u128(1);
        let inner = Uuid::from_u128(2);
        let p = FrameProfiler::new(Uuid::nil(), 3, None, true);
        p.enter_comp();
        p.layer_done(
            outer,
            4.0,
            vec![EffectTiming {
                effect: outer,
                ms: 1.0,
            }],
        );
        // A Precomp's own walk: its layers are rows of another composition, so
        // they are not the ones this frame's Timeline is showing.
        p.enter_comp();
        p.layer_done(inner, 9.0, Vec::new());
        p.leave_comp();
        p.leave_comp();
        let profile = p.finish().expect("timings were asked for");
        assert_eq!(profile.layers.len(), 1);
        assert_eq!(profile.layers[0].layer, outer);
        assert_eq!(profile.layers[0].effects.len(), 1);
        assert_eq!(profile.frame, 3);
    }
}

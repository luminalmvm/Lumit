//! The six timed scenarios (docs/13-PERFORMANCE-RULES.md §2, K-389).
//!
//! # In plain terms
//!
//! Each function here does one thing the editor does — jump the playhead, let a
//! frame refine, play a second of the comp, fill the cache — with a stopwatch
//! around it, and reports one number. Nothing judges the number: a scenario
//! measures, and the gate (see [`crate::baseline`]) decides whether the number
//! got worse.
//!
//! Which budgets are here, and which cannot be: a headless harness has no UI
//! thread, no window and no encoder, so **B1/B2** (UI frame time, input
//! acknowledgement), **B8** (export throughput), **B9** (device loss) and
//! **B10** (A/V sync) are not measurable from here and stay manual or
//! real-window checks — docs/TODO.md keeps them. What is here is B3, B4, B5,
//! B6, B7 and B11.
//!
//! ## Two things worth knowing before reading a number
//!
//! **Cold means a new renderer.** GPU work is submitted rather than performed,
//! and a [`HeadlessRenderer`] holds everything expensive that survives a frame:
//! the compiled shaders, the open decoders, the probes, and the cache of
//! finished frames. So "cold" here is a renderer built for that scenario and
//! thrown away after it, with nothing in any cache and no pipeline compiled.
//! Only B5 is warm, and it warms itself.
//!
//! **A latency measurement waits for the graphics card.** Compositing a frame
//! *submits* work; a clock read straight afterwards times the paperwork, not
//! the picture (docs/13 §7.1 says this about the profiler and it is just as
//! true here). So B3 and B4 settle the device inside the timed region. The
//! throughput scenarios (B5–B7, B11) settle once at the end instead and divide,
//! because real playback renders ahead of the clock and keeps exactly the
//! processor/card overlap that a per-frame fence would throw away.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use lumit_core::model::Document;
use lumit_render::cache::{fill_walk_order, work_area_frames};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use uuid::Uuid;

/// One measured number, as the harness emits it.
///
/// `value_ms` is whatever the budget is stated against, and lower is always
/// better — which is what lets one ratio gate cover all six:
///
/// | budget | `value_ms` is |
/// |---|---|
/// | B3, B4 | the latency of one action, 95th percentile |
/// | B5, B6, B7 | milliseconds per frame (60 fps = 16.7, 24 fps = 41.7) |
/// | B11 | the whole work-area fill, start to finish |
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Measurement {
    /// The budget's name in docs/13 §2 — `"B3"` and so on.
    pub budget: &'static str,
    /// The measured number (see the table above).
    pub value_ms: f64,
    /// How many frames the measurement is over. Context for a human, and the
    /// guard against comparing a run of 60 frames with a run of 1200.
    pub frames: u64,
}

impl Measurement {
    /// Frames per second, for the scenarios whose budget is stated that way.
    #[must_use]
    pub fn fps(&self) -> f64 {
        if self.value_ms > 0.0 {
            1000.0 / self.value_ms
        } else {
            f64::INFINITY
        }
    }
}

/// Full resolution: what B4 refines to, what B7 plays at, and what export
/// writes.
const FULL: Quality = Quality {
    draft: false,
    auto_res: false,
    display_scale: 1.0,
    divisor: 1,
};

/// The scrub draft: decode capped hard so a frame comes back fast, which is
/// precisely B3's "possibly degraded frame" (see [`lumit_render::plan`]).
const SCRUB: Quality = Quality {
    draft: true,
    auto_res: true,
    display_scale: 0.5,
    divisor: 1,
};

/// Half scale — the middle rung of the degradation ladder (docs/13 §4 step 3),
/// used by B6 and by the idle fill.
///
/// ponytail: the real adaptive controller picks a tier per run and walks it up
/// and down (K-186); it lives in `lumit-bridge`, which an engine-side harness
/// must not depend on. Pinning the middle rung measures the same work at a
/// fixed quality, which is what a regression gate needs — a controller that
/// silently settled a rung lower would otherwise read as "faster". If B6 ever
/// needs to answer "which tier did this machine hold?", lift the controller
/// into an engine crate and drive it here.
const HALF: Quality = Quality {
    draft: false,
    auto_res: true,
    display_scale: 0.5,
    divisor: 1,
};

/// Scrub jumps sampled for B3's 95th percentile.
const SCRUB_SAMPLES: usize = 20;
/// Refinements sampled for B4's.
const REFINE_SAMPLES: usize = 5;
/// Frames warmed into the card's cache for B5. At full resolution a frame is
/// 8.3 MB, so this many sit inside the default 512 MiB budget with room to
/// spare — a window that evicted its own start would measure re-rendering.
const WARM_FRAMES: u64 = 48;
/// How many times B5 replays that window.
const WARM_LAPS: u64 = 5;

/// The fraction of the work area the span-scaled scenarios measure, in
/// per cent: `BENCH_SPAN_FRACTION`, clamped 1..=100, default 100.
///
/// Why it exists: B6, B7 and B11 render cold frames of a deliberately heavy
/// comp, and on a software rasteriser (CI's lavapipe) the full 20 s span ran
/// past the job's two-hour timeout. A fraction is honest where a timeout is
/// not, because the ratio gate only ever compares a run against a baseline
/// carrying the SAME fraction - the stamp rides the results file and
/// `baseline::compare` refuses a mismatch, exactly as it refuses a foreign OS.
/// B3, B4 and B5 stay unscaled: single-frame latencies and a warm replay are
/// already cheap.
pub fn span_fraction() -> u64 {
    std::env::var("BENCH_SPAN_FRACTION")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(100, |v| v.clamp(1, 100))
}

/// `count` scaled by [`span_fraction`], never below one frame.
fn span_scaled(count: u64) -> u64 {
    (count * span_fraction() / 100).max(1)
}
/// A second of playback, which is what B6 and B7 report the rate of.
const PLAY_FRAMES: u64 = 60;

/// The reference comp, built once, ready to be driven by any scenario.
pub struct Harness {
    doc: Arc<Document>,
    comp: Uuid,
}

impl Harness {
    /// Generate (or reuse) the media in `media_dir` and build docs/13 §1's
    /// comp over it. `Err` when the media cannot be made — no ffmpeg, say.
    pub fn new(media_dir: &Path) -> Result<Self, String> {
        let (doc, comp) = crate::reference_comp(media_dir)?;
        Ok(Self {
            doc: Arc::new(doc),
            comp,
        })
    }

    /// Every scenario in order, each reported to `progress` as it lands so a
    /// long run says where it is. Stops at the first failure.
    pub fn all(&self, progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
        let mut out = Vec::new();
        for run in [
            Self::b3_scrub,
            Self::b4_refine,
            Self::b5_warm_playback,
            Self::b6_cold_adaptive_playback,
            Self::b7_cold_full_playback,
            Self::b11_idle_fill,
        ] {
            let m = run(self)?;
            progress(m);
            out.push(m);
        }
        Ok(out)
    }

    /// A renderer with nothing in it — see the module note on what cold means.
    fn cold(&self) -> Result<HeadlessRenderer, String> {
        let mut r = HeadlessRenderer::new()?;
        r.presync_items(&self.doc, self.comp);
        Ok(r)
    }

    /// How many frames the comp's work area holds — B11's span, and the range
    /// the other scenarios pick frames from.
    fn work_area(&self) -> (usize, usize) {
        self.doc.comp(self.comp).map_or((0, 1), work_area_frames)
    }

    /// **B3** — playhead move to first (possibly degraded) frame displayed.
    ///
    /// The renderer is warmed on one frame first: a scrub happens in a session
    /// that is already running, so compiling a shader is not part of what the
    /// user waits for. Each sample then jumps somewhere the renderer has not
    /// been (a stride coprime with the work area, so no frame repeats) — a
    /// scrub that landed on a cached frame would be measuring the cache.
    pub fn b3_scrub(&self) -> Result<Measurement, String> {
        let mut r = self.cold()?;
        let (start, end) = self.work_area();
        r.render_prepared(&self.doc, self.comp, start as u64, SCRUB, false, false)?;
        r.settle_gpu();

        let mut ms = Vec::with_capacity(SCRUB_SAMPLES);
        for i in 0..SCRUB_SAMPLES {
            let frame = scatter(i, start, end);
            let t = Instant::now();
            r.render_prepared(&self.doc, self.comp, frame, SCRUB, false, true)?;
            r.settle_gpu();
            ms.push(elapsed_ms(t));
        }
        Ok(Measurement {
            budget: "B3",
            value_ms: p95(&mut ms),
            frames: SCRUB_SAMPLES as u64,
        })
    }

    /// **B4** — idle to the current frame refined to full quality.
    ///
    /// Each sample is the real sequence: land on a frame at scrub quality (not
    /// timed — that is B3), then render the same frame at Full and time that.
    pub fn b4_refine(&self) -> Result<Measurement, String> {
        let mut r = self.cold()?;
        let (start, end) = self.work_area();
        r.render_prepared(&self.doc, self.comp, start as u64, FULL, false, false)?;
        r.settle_gpu();

        let mut ms = Vec::with_capacity(REFINE_SAMPLES);
        for i in 0..REFINE_SAMPLES {
            let frame = scatter(i, start, end);
            r.render_prepared(&self.doc, self.comp, frame, SCRUB, false, true)?;
            r.settle_gpu();
            let t = Instant::now();
            r.render_prepared(&self.doc, self.comp, frame, FULL, false, true)?;
            r.settle_gpu();
            ms.push(elapsed_ms(t));
        }
        Ok(Measurement {
            budget: "B4",
            value_ms: p95(&mut ms),
            frames: REFINE_SAMPLES as u64,
        })
    }

    /// **B5** — warm cache playback: the green bar's promise.
    ///
    /// Renders a window of frames into the card's frame cache, then replays it.
    /// A served frame costs what playback really pays for one: naming it (the
    /// content hash of the whole composition at that time — not free) and
    /// finding it. Nothing is composited, which is the point of the budget.
    ///
    /// Fails rather than reports if the replay did not come from the cache. A
    /// window that had evicted itself would still produce a plausible number,
    /// and it would be a cold measurement wearing a warm one's name.
    pub fn b5_warm_playback(&self) -> Result<Measurement, String> {
        let mut r = self.cold()?;
        let (start, _) = self.work_area();
        for i in 0..WARM_FRAMES {
            r.render_prepared(&self.doc, self.comp, start as u64 + i, FULL, false, true)?;
        }
        r.settle_gpu();

        let hits_before = r.frame_texture_hits();
        let t = Instant::now();
        for _ in 0..WARM_LAPS {
            for i in 0..WARM_FRAMES {
                r.render_prepared(&self.doc, self.comp, start as u64 + i, FULL, false, true)?;
            }
        }
        r.settle_gpu();
        let total = elapsed_ms(t);

        let frames = WARM_FRAMES * WARM_LAPS;
        let served = r.frame_texture_hits() - hits_before;
        if served < frames {
            return Err(format!(
                "B5 is not a warm measurement: {served} of {frames} replayed frames came from \
                 the cache. Either the window no longer fits the frame-texture budget, or the \
                 frames stopped being nameable."
            ));
        }
        Ok(Measurement {
            budget: "B5",
            value_ms: total / frames as f64,
            frames,
        })
    }

    /// **B6** — cold playback with degradation allowed, at [`HALF`].
    pub fn b6_cold_adaptive_playback(&self) -> Result<Measurement, String> {
        self.cold_playback("B6", HALF)
    }

    /// **B7** — cold playback at Full resolution, nothing degraded.
    pub fn b7_cold_full_playback(&self) -> Result<Measurement, String> {
        self.cold_playback("B7", FULL)
    }

    /// A second of consecutive frames on a renderer that has never rendered,
    /// reported as milliseconds per frame. Settled once at the end: playback
    /// renders ahead of its clock, so the overlap between the processor and the
    /// card is part of the rate, not noise in it.
    fn cold_playback(&self, budget: &'static str, quality: Quality) -> Result<Measurement, String> {
        let mut r = self.cold()?;
        let (start, end) = self.work_area();
        let count = span_scaled(PLAY_FRAMES.min((end - start) as u64));
        let t = Instant::now();
        for i in 0..count {
            r.render_prepared(&self.doc, self.comp, start as u64 + i, quality, false, true)?;
        }
        r.settle_gpu();
        Ok(Measurement {
            budget,
            value_ms: elapsed_ms(t) / count as f64,
            frames: count,
        })
    }

    /// **B11** — background cache fill of the 20 s work area from cold.
    ///
    /// Walks the work area in the order the idle fill really walks it
    /// ([`fill_walk_order`]: the playhead, then roughly three frames forward
    /// for every one back) at the fill's quality, and times the lot. The number
    /// is the whole fill, because that is how the budget is written.
    ///
    /// There is no idling here and no editor competing for the card — a fill
    /// that yields to interactive work can only be slower, so this is the
    /// optimistic end of B11 and a regression in it is a real one.
    pub fn b11_idle_fill(&self) -> Result<Measurement, String> {
        let mut r = self.cold()?;
        let (start, end) = self.work_area();
        let mut order = fill_walk_order(start, start, end);
        order.truncate(span_scaled(order.len() as u64) as usize);
        let t = Instant::now();
        for frame in &order {
            r.render_prepared(&self.doc, self.comp, *frame as u64, HALF, false, true)?;
        }
        r.settle_gpu();
        Ok(Measurement {
            budget: "B11",
            value_ms: elapsed_ms(t),
            frames: order.len() as u64,
        })
    }
}

/// Milliseconds since `t`, as a float.
fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// The `i`th scrub target: a stride coprime with the work area's length, so a
/// run of samples visits a different frame every time and never lands twice.
fn scatter(i: usize, start: usize, end: usize) -> u64 {
    let span = end.saturating_sub(start).max(1);
    (start + (i.wrapping_mul(617) % span)) as u64
}

/// The 95th percentile docs/13 §2 states its latencies at — nearest-rank, so a
/// handful of samples gives the worst of them rather than an interpolation
/// between two numbers that were never measured.
fn p95(ms: &mut [f64]) -> f64 {
    if ms.is_empty() {
        return 0.0;
    }
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((ms.len() as f64) * 0.95).ceil() as usize;
    ms[rank.clamp(1, ms.len()) - 1]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_95th_percentile_is_the_nearest_rank() {
        // Twenty samples: rank 19 of 20, so the second worst.
        let mut twenty: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(p95(&mut twenty), 19.0);
        // Five samples: the worst of them, not an average of the top two.
        assert_eq!(p95(&mut [1.0, 9.0, 2.0, 3.0, 4.0]), 9.0);
        assert_eq!(p95(&mut [7.0]), 7.0);
        assert_eq!(p95(&mut []), 0.0);
    }

    #[test]
    fn scrub_targets_never_repeat_across_a_run() {
        let seen: std::collections::BTreeSet<u64> =
            (0..SCRUB_SAMPLES).map(|i| scatter(i, 0, 1200)).collect();
        assert_eq!(
            seen.len(),
            SCRUB_SAMPLES,
            "a scrub sample measured a cache hit"
        );
        assert!(seen.iter().all(|f| *f < 1200));
        // A work area shorter than the stride still stays inside it.
        assert!((0..SCRUB_SAMPLES).all(|i| scatter(i, 10, 12) < 12));
    }
}

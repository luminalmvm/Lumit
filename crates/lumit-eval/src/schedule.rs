//! The pure realtime tier controller for playback
//! (docs/impl/playback-scheduler.md §5 "Realtime mode").
//!
//! In plain terms: adaptive playback promises to keep time, and pays for it
//! with resolution. This module watches how long frames have actually been
//! taking and answers one question with plain arithmetic — what preview
//! divisor should the next frame be rendered at? Frames too slow for the frame
//! budget drop to a coarser resolution at once; comfortably fast frames earn
//! it back slowly, so the picture never flickers between qualities.
//!
//! That is all this module is. The live playback loop — the render-ahead ring,
//! the lookahead that sizes it, the present schedule, the pre-roll before
//! audio — lives in `lumit-bridge` (`playback.rs` and the worker thread),
//! where the clock, the GPU and the audio device it needs actually are;
//! `crates/lumit-bridge/src/realtime.rs` wraps the controller below and feeds
//! it measured render costs. This file once carried pure referee copies of the
//! ring, the lookahead and the scheduling scan; they were deleted rather than
//! kept as a second truth beside the wired ones.
//!
//! What stays here is deliberately pure: no threads, no clocks, no GPU. Costs
//! and frame rates arrive as plain numbers and the tier comes back as a plain
//! number, which is what makes every rule below provable by an ordinary
//! deterministic test.

/// Coarsest preview divisor the realtime controller will fall to (Quarter
/// resolution). Below Quarter the picture stops being judgeable.
pub const COARSEST_TIER: u32 = 4;

/// Finest tier (Full resolution).
pub const FINEST_TIER: u32 = 1;

/// Drop a tier when the smoothed cost exceeds this fraction of the frame
/// budget (`0.9 / fps` of headroom is gone). Starting point per
/// docs/impl/playback-scheduler.md §5 — tune on reference hardware.
pub const DROP_BUDGET_FRACTION: f64 = 0.9;

/// Rise a tier only when the smoothed cost sits below this fraction of the
/// frame budget. The wide gap between 0.4 and 0.9 is the hysteresis: a cost
/// in between changes nothing. Starting point — tune on reference hardware.
pub const RISE_BUDGET_FRACTION: f64 = 0.4;

/// How many consecutive comfortably-cheap frames it takes to earn a finer
/// tier. Starting point per the impl note — tune on reference hardware.
pub const RISE_SUSTAIN_FRAMES: u32 = 12;

/// Ceiling on the rise requirement after repeated flapping (see
/// [`RealtimeController`]'s anti-flap back-off).
pub const MAX_RISE_SUSTAIN_FRAMES: u32 = 96;

/// A rise that gets reversed within this many frames counts as a flap, and
/// doubles the sustain required for the next rise attempt. Holding the finer
/// tier this long clears the penalty. Starting point — tune on hardware.
pub const FLAP_WINDOW_FRAMES: u32 = 48;

/// How much one new measurement moves the smoothed cost (exponentially
/// weighted moving average). Higher reacts faster but jitters more.
/// Starting point — tune on reference hardware.
pub const COST_EWMA_ALPHA: f64 = 0.3;

/// The realtime-mode resolution picker
/// (docs/impl/playback-scheduler.md §5 "Realtime mode").
///
/// In plain terms: realtime mode promises smooth motion and pays for it with
/// resolution. This controller watches a smoothed average of how long frames
/// are taking at the current preview resolution. When frames get too slow
/// for the frame budget it *immediately* drops to a coarser resolution
/// (divisor 1 = Full, 2 = Half, 3 = Third, 4 = Quarter); when frames have
/// been comfortably fast for a sustained stretch it cautiously steps back up.
///
/// Quick to worsen, slow to improve — that asymmetry, plus the gap between
/// the two thresholds, is what stops the picture flickering between
/// resolutions. As a further guard, a rise that has to be reversed straight
/// away doubles the patience required before trying again.
#[derive(Debug, Clone)]
pub struct RealtimeController {
    /// Current preview divisor, [`FINEST_TIER`]..=[`COARSEST_TIER`].
    tier: u32,
    /// Smoothed render cost at the current tier, seconds. `None` right after
    /// a tier change: costs from another resolution describe different work,
    /// so the average restarts.
    cost_ewma: Option<f64>,
    /// Consecutive frames below the rise threshold so far.
    rise_streak: u32,
    /// Cheap frames currently required before rising (grows on flaps).
    required_rise_streak: u32,
    /// Frames spent at the current tier since the last change.
    frames_at_tier: u32,
    /// Whether the last tier change was a rise (needed to spot a flap).
    last_change_was_rise: bool,
}

impl Default for RealtimeController {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeController {
    /// Starts optimistic, at Full resolution; the first slow frames will
    /// walk it down to wherever the machine can keep up.
    pub fn new() -> Self {
        Self {
            tier: FINEST_TIER,
            cost_ewma: None,
            rise_streak: 0,
            required_rise_streak: RISE_SUSTAIN_FRAMES,
            frames_at_tier: 0,
            last_change_was_rise: false,
        }
    }

    /// The preview divisor currently in force (1 = Full … 4 = Quarter).
    pub fn tier(&self) -> u32 {
        self.tier
    }

    /// Feed in one frame's measured render cost (seconds) at the current
    /// frame rate; the answer is the divisor to render the *next* frame at.
    /// Nonsense measurements or frame rates change nothing.
    pub fn record(&mut self, cost_secs: f64, fps: f64) -> u32 {
        if !cost_secs.is_finite() || cost_secs < 0.0 || !fps.is_finite() || fps <= 0.0 {
            return self.tier;
        }
        let budget = 1.0 / fps;
        let ewma = match self.cost_ewma {
            None => cost_secs,
            Some(prev) => COST_EWMA_ALPHA * cost_secs + (1.0 - COST_EWMA_ALPHA) * prev,
        };
        self.cost_ewma = Some(ewma);
        self.frames_at_tier = self.frames_at_tier.saturating_add(1);

        // Holding a risen tier long enough proves it was earned; forgive
        // past flaps and restore normal patience.
        if self.last_change_was_rise && self.frames_at_tier >= FLAP_WINDOW_FRAMES {
            self.required_rise_streak = RISE_SUSTAIN_FRAMES;
            self.last_change_was_rise = false;
        }

        if ewma > DROP_BUDGET_FRACTION * budget && self.tier < COARSEST_TIER {
            // Too slow for the budget: coarsen immediately.
            if self.last_change_was_rise && self.frames_at_tier <= FLAP_WINDOW_FRAMES {
                // We only just rose and are already backing out — a flap.
                // Demand a longer proof of cheapness next time.
                self.required_rise_streak =
                    (self.required_rise_streak * 2).min(MAX_RISE_SUSTAIN_FRAMES);
            }
            self.tier += 1;
            self.after_tier_change(false);
        } else if ewma < RISE_BUDGET_FRACTION * budget && self.tier > FINEST_TIER {
            self.rise_streak += 1;
            if self.rise_streak >= self.required_rise_streak {
                // Comfortably fast for long enough: refine one step.
                self.tier -= 1;
                self.after_tier_change(true);
            }
        } else {
            // In the hysteresis band (or already at the finest tier): any
            // rise progress is void — cheapness must be *consecutive*.
            self.rise_streak = 0;
        }
        self.tier
    }

    /// Shared reset after any tier change: the smoothed cost belonged to the
    /// old resolution, so measurement starts over.
    fn after_tier_change(&mut self, was_rise: bool) {
        self.cost_ewma = None;
        self.rise_streak = 0;
        self.frames_at_tier = 0;
        self.last_change_was_rise = was_rise;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // ---- RealtimeController ----

    /// docs/impl/playback-scheduler.md §6 test #5, first half: a cost cliff
    /// (a heavy effect appears) drops the tier within 3 frames.
    #[test]
    fn realtime_cost_cliff_drops_tier_within_three_frames() {
        let fps = 60.0;
        let mut rc = RealtimeController::new();
        // Comfortable playback at Full for a while (4 ms per frame).
        for _ in 0..120 {
            assert_eq!(rc.record(0.004, fps), FINEST_TIER);
        }
        // The cliff: frames suddenly cost 50 ms (budget is ~16.7 ms).
        let mut frames_to_drop = 0;
        for frame in 1..=10 {
            if rc.record(0.05, fps) > FINEST_TIER {
                frames_to_drop = frame;
                break;
            }
        }
        assert!(
            (1..=3).contains(&frames_to_drop),
            "tier should drop within 3 frames of the cliff, took {frames_to_drop}"
        );
    }

    /// §6 test #5, second half: once settled on a steady cost, the tier does
    /// not flap — at most one change over a long run.
    #[test]
    fn realtime_settles_without_flapping_on_steady_cost() {
        let fps = 60.0;
        let mut rc = RealtimeController::new();
        // Heavy steady cost: walk down as far as needed, then settle.
        for _ in 0..60 {
            rc.record(0.05, fps);
        }
        let settled = rc.tier();
        let mut changes = 0;
        let mut prev = settled;
        for _ in 0..600 {
            let t = rc.record(0.05, fps);
            if t != prev {
                changes += 1;
                prev = t;
            }
        }
        assert!(changes <= 1, "tier flapped {changes} times on steady cost");
        // A cost inside the hysteresis band (between 0.4 and 0.9 of budget)
        // changes nothing at all, from either direction.
        let mut rc = RealtimeController::new();
        for _ in 0..600 {
            assert_eq!(rc.record(0.01, fps), FINEST_TIER); // 0.6 of budget
        }
    }

    /// Walk a fresh controller down to Quarter with brutal costs, stopping
    /// the moment it arrives (so its smoothed cost is freshly reset there).
    fn controller_forced_to_quarter(fps: f64) -> RealtimeController {
        let mut rc = RealtimeController::new();
        for _ in 0..10 {
            if rc.tier() == COARSEST_TIER {
                break;
            }
            rc.record(0.2, fps);
        }
        assert_eq!(rc.tier(), COARSEST_TIER, "brutal cost should reach Quarter");
        rc
    }

    /// Recovery: sustained comfortably-cheap frames earn the tier back after
    /// the 12-frame sustain, one step at a time.
    #[test]
    fn realtime_sustained_cheap_cost_rises_tier() {
        let fps = 60.0;
        let mut rc = controller_forced_to_quarter(fps);
        // Now frames are cheap (2 ms, well under 0.4 × 16.7 ms ≈ 6.7 ms).
        // Fewer than the sustain: no rise yet.
        for _ in 0..(RISE_SUSTAIN_FRAMES - 1) {
            rc.record(0.002, fps);
        }
        assert_eq!(rc.tier(), COARSEST_TIER, "must not rise before the sustain");
        // One more cheap frame completes the sustain.
        assert_eq!(rc.record(0.002, fps), COARSEST_TIER - 1);
        // Kept cheap long enough, it climbs all the way back to Full.
        for _ in 0..200 {
            rc.record(0.002, fps);
        }
        assert_eq!(rc.tier(), FINEST_TIER);
    }

    /// One frame landing the smoothed cost mid-band voids the streak:
    /// cheapness must be consecutive (the hysteresis in action).
    #[test]
    fn realtime_rise_streak_resets_on_a_mid_band_frame() {
        let fps = 60.0;
        let mut rc = controller_forced_to_quarter(fps);
        for _ in 0..(RISE_SUSTAIN_FRAMES - 1) {
            rc.record(0.002, fps);
        }
        // A 20 ms frame lifts the smoothed cost into the hysteresis band
        // (0.3 × 0.02 + 0.7 × ~0.002 ≈ 7.4 ms, between 6.7 and 15 ms).
        rc.record(0.02, fps);
        // Another 11 cheap frames: a fresh streak, still one short.
        for _ in 0..(RISE_SUSTAIN_FRAMES - 1) {
            rc.record(0.002, fps);
        }
        assert_eq!(
            rc.tier(),
            COARSEST_TIER,
            "streak must restart after a break"
        );
    }

    /// Nonsense measurements change nothing.
    #[test]
    fn realtime_ignores_nonsense_inputs() {
        let mut rc = RealtimeController::new();
        assert_eq!(rc.record(f64::NAN, 60.0), FINEST_TIER);
        assert_eq!(rc.record(-0.5, 60.0), FINEST_TIER);
        assert_eq!(rc.record(0.05, 0.0), FINEST_TIER);
        assert_eq!(rc.record(0.05, f64::NEG_INFINITY), FINEST_TIER);
        assert_eq!(rc.tier(), FINEST_TIER);
    }
}

//! The pure decision core of the playback frame scheduler
//! (docs/impl/playback-scheduler.md §5).
//!
//! In plain terms: during playback Lumit renders frames *ahead* of the
//! playhead and parks them in a small shelf (the [`FrameRing`]). Each screen
//! refresh takes the newest shelf frame whose time has come. Three questions
//! decide everything, and this module answers all three with plain
//! arithmetic:
//!
//! 1. **How far ahead should we render?** ([`Lookahead`]) Far enough that a
//!    slow frame doesn't cause a stutter, not so far that scrubbing throws
//!    lots of work away. The answer adapts to how long frames have actually
//!    been taking.
//! 2. **Which frame do we render next?** ([`next_frame_to_schedule`]) The
//!    earliest frame between the playhead and the lookahead target that is
//!    neither on the shelf nor already being rendered.
//! 3. **In realtime mode, at what resolution?** ([`RealtimeController`],
//!    K-030) Frames too slow for the frame budget drop to a coarser preview
//!    resolution; comfortably fast frames earn the resolution back, slowly,
//!    so the picture never flickers between qualities.
//!
//! Everything here is deliberately *pure*: no threads, no clocks, no audio
//! device, no GPU. The current playhead position and each frame's measured
//! render cost arrive as plain numbers, and the answers come back as plain
//! numbers. The messy real-world wiring (worker threads, the audio clock,
//! vsync) lives in the UI crate later and simply asks these types what to
//! do — which is what makes every rule in this file provable by an ordinary
//! deterministic test.

/// The fewest frames the scheduler will render ahead of the playhead
/// (docs/impl/playback-scheduler.md §5). Below this, one slow frame stutters.
pub const MIN_LOOKAHEAD_FRAMES: u32 = 8;

/// The most frames the scheduler will render ahead. Beyond this, pausing or
/// scrubbing throws away too much finished work for no smoothness gain.
pub const MAX_LOOKAHEAD_FRAMES: u32 = 16;

/// How many recent render costs [`Lookahead`] remembers when estimating how
/// slow frames have been lately. Small on purpose: old costs describe a comp
/// that may have changed.
pub const COST_WINDOW: usize = 32;

/// A bounded shelf of rendered frames waiting to be shown, ordered by frame
/// number. `T` is whatever a rendered frame is to the caller (in tests a
/// number; in the real pipeline a handle to GPU pixels).
///
/// The rules, in plain terms (docs/impl/playback-scheduler.md §4):
/// - The shelf has a fixed number of slots. When it is full, nothing more is
///   accepted — that is the back-pressure that stops workers racing ahead.
/// - At each screen refresh the presenter asks for the newest frame whose
///   time has already come ([`newest_ready`](Self::newest_ready)). Older due
///   frames are quietly discarded — the clock has passed them, showing them
///   now would be showing the past.
/// - If nothing on the shelf is due yet (the renderer is running late), the
///   answer is "nothing new" and the caller keeps showing the last frame it
///   was given. Audio never waits for video.
#[derive(Debug, Clone)]
pub struct FrameRing<T> {
    /// Waiting frames, kept sorted by frame number, oldest first.
    slots: Vec<(u64, T)>,
    /// Fixed number of slots; never changes after construction.
    capacity: usize,
}

impl<T> FrameRing<T> {
    /// A ring with `capacity` slots. A ring needs at least one slot to be a
    /// ring, so 0 is treated as 1 (the scrub "mailbox" is exactly this).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// How many more frames the ring can accept right now.
    pub fn free(&self) -> usize {
        self.capacity - self.slots.len()
    }

    /// How many frames are waiting on the shelf.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// True when a frame with this number is already on the shelf.
    pub fn contains(&self, frame_index: u64) -> bool {
        self.slots
            .binary_search_by_key(&frame_index, |(i, _)| *i)
            .is_ok()
    }

    /// Offer a finished frame to the shelf. Returns whether it was accepted:
    /// a full ring refuses (back-pressure), and a duplicate frame number
    /// refuses (the copy already shelved is just as good).
    pub fn push(&mut self, frame_index: u64, payload: T) -> bool {
        if self.slots.len() >= self.capacity {
            return false;
        }
        match self.slots.binary_search_by_key(&frame_index, |(i, _)| *i) {
            Ok(_) => false,
            Err(pos) => {
                self.slots.insert(pos, (frame_index, payload));
                true
            }
        }
    }

    /// Remove and return every frame whose time has come (frame number at or
    /// before `clock_frame`), oldest first, freeing their slots. The caller
    /// decides what to do with them; frames still in the future stay put.
    pub fn take_upto(&mut self, clock_frame: u64) -> Vec<(u64, T)> {
        let due = self.slots.partition_point(|(i, _)| *i <= clock_frame);
        self.slots.drain(..due).collect()
    }

    /// The present decision, exactly as §4 words it: return the newest frame
    /// whose time has come, discard the older due frames the clock has
    /// already passed, and leave future frames waiting.
    ///
    /// `None` means nothing on the shelf is due yet — the renderer is late,
    /// and the caller should *hold*: keep showing the last frame this method
    /// returned. Audio carries on regardless; video catches up when it can.
    pub fn newest_ready(&mut self, clock_frame: u64) -> Option<(u64, T)> {
        let mut due = self.take_upto(clock_frame);
        due.pop()
    }
}

/// Adaptive lookahead (docs/impl/playback-scheduler.md §5): how many frames
/// past the playhead the scheduler should be rendering right now.
///
/// The idea: watch how long recent frames took, take a near-worst-case
/// ("95th percentile" — slower than 19 frames in 20), and keep roughly twice
/// that much time already rendered as a cushion. Cheap comps settle at the
/// minimum cushion of [`MIN_LOOKAHEAD_FRAMES`]; heavy comps grow it, capped
/// at [`MAX_LOOKAHEAD_FRAMES`].
#[derive(Debug, Clone)]
pub struct Lookahead {
    /// The last [`COST_WINDOW`] render costs, in seconds, as a reusable
    /// circular buffer (oldest overwritten first).
    recent: [f64; COST_WINDOW],
    /// How many entries of `recent` hold real measurements so far.
    filled: usize,
    /// Where the next measurement will be written.
    next: usize,
}

impl Default for Lookahead {
    fn default() -> Self {
        Self::new()
    }
}

impl Lookahead {
    /// A lookahead with no history yet; answers the minimum until taught.
    pub fn new() -> Self {
        Self {
            recent: [0.0; COST_WINDOW],
            filled: 0,
            next: 0,
        }
    }

    /// Record how long one frame took to render, in seconds. Nonsense
    /// measurements (negative, infinite, not-a-number) are ignored rather
    /// than poisoning the estimate.
    pub fn record(&mut self, cost_secs: f64) {
        if !cost_secs.is_finite() || cost_secs < 0.0 {
            return;
        }
        self.recent[self.next] = cost_secs;
        self.next = (self.next + 1) % COST_WINDOW;
        self.filled = (self.filled + 1).min(COST_WINDOW);
    }

    /// How many frames ahead to render at the given frame rate:
    /// `clamp(round(2 × p95 cost × fps), 8, 16)`. With no history yet (or a
    /// nonsense frame rate) the answer is the safe minimum.
    pub fn frames(&self, fps: f64) -> u32 {
        if self.filled == 0 || !fps.is_finite() || fps <= 0.0 {
            return MIN_LOOKAHEAD_FRAMES;
        }
        // Approximate p95: sort a copy of the window and read near the top.
        let mut sorted = self.recent;
        let filled = &mut sorted[..self.filled];
        filled.sort_by(f64::total_cmp);
        let idx = (self.filled * 95 / 100).min(self.filled - 1);
        let p95 = filled[idx];
        let raw = (2.0 * p95 * fps).round();
        if raw >= f64::from(MAX_LOOKAHEAD_FRAMES) {
            MAX_LOOKAHEAD_FRAMES
        } else if raw > f64::from(MIN_LOOKAHEAD_FRAMES) {
            raw as u32
        } else {
            // Small values land here — and so would any NaN, safely.
            MIN_LOOKAHEAD_FRAMES
        }
    }
}

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

/// The realtime-mode resolution picker (K-030,
/// docs/impl/playback-scheduler.md §5 "Realtime mode").
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

/// Which frame should be rendered next (the §5 loop's
/// `n = next comp frame ≤ target not yet scheduled`).
///
/// Scans forward from the playhead (`clock_frame`) to the lookahead target
/// (`target_frame = clock + lookahead`, inclusive) and returns the first
/// frame number that is neither already on the shelf nor already being
/// rendered — `already_scheduled` is the caller's own "in flight" check,
/// passed as a function so this stays pure and collection-agnostic. `None`
/// means the whole window is covered and the scheduler can rest until the
/// clock moves. The scan is bounded by the lookahead window, so it is at
/// most [`MAX_LOOKAHEAD_FRAMES`] + 1 steps in practice.
pub fn next_frame_to_schedule<T>(
    clock_frame: u64,
    target_frame: u64,
    ring: &FrameRing<T>,
    already_scheduled: impl Fn(u64) -> bool,
) -> Option<u64> {
    if target_frame < clock_frame {
        return None;
    }
    (clock_frame..=target_frame).find(|&n| !ring.contains(n) && !already_scheduled(n))
}

/// How many upcoming frames must already be renderable before Cached-mode
/// audio plays — a quarter-second at any frame rate (rounded up, min 1). The
/// gate is *readiness ahead*, not replay history (owner): a fully-cached run
/// gets sound from its very first frame, a still-rendering stretch stays
/// silent (no start-stop flapping at the render's crawling edge), and after a
/// stall the sound rejoins the instant the next quarter-second is there.
#[must_use]
pub fn cached_audio_lookahead(fps: f64) -> usize {
    ((fps / 4.0).ceil() as usize).max(1)
}

/// What Cached-mode playback (K-171, docs/06 §6.5) should do this UI tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedStep {
    /// Move the playhead on to the next frame this tick.
    pub advance: bool,
    /// Kick off a render of the next frame now — it is not ready, so playback
    /// is render-gated (waits for it) rather than skipping it.
    pub request_next: bool,
    /// Whether comp audio should be running this tick. Audio plays exactly
    /// when the coming stretch is already renderable ([`cached_audio_lookahead`]),
    /// so it starts with the first shown frame of a ready run and pauses
    /// whenever a frame is being awaited — sound never runs ahead of a
    /// stalled picture.
    pub audio_playing: bool,
}

/// Decide the Cached-mode step: render every frame, never skip, paced to at
/// most realtime (K-171). The playhead advances to `next` only once `next` is
/// ready (cached) *and* a frame's worth of real time has passed since the last
/// advance — so a fully-cached span replays at true speed, and a span still
/// rendering advances exactly as fast as frames complete (slower than
/// realtime), never dropping one.
///
/// - `next_ready` — is the next frame already in the cache?
/// - `elapsed` / `frame_dur` — seconds since the last advance, and one frame's
///   duration (`1/fps`); their ratio is the realtime pace cap.
/// - `run_ready` — are the next [`cached_audio_lookahead`] frames all ready?
///   The audio gate: sound plays from the first frame of a ready run (owner)
///   instead of waiting out a warm-up streak.
#[must_use]
pub fn cached_step(next_ready: bool, elapsed: f64, frame_dur: f64, run_ready: bool) -> CachedStep {
    if !next_ready {
        // Render-gate: hold the picture, render the frame, and pause audio so
        // it never runs ahead of the stalled picture.
        return CachedStep {
            advance: false,
            request_next: true,
            audio_playing: false,
        };
    }
    // Ready: advance once a frame's worth of real time has passed (the
    // realtime pace cap), otherwise hold this frame for smooth replay. Either
    // way sound runs exactly when the stretch ahead is ready.
    CachedStep {
        advance: elapsed >= frame_dur,
        request_next: false,
        audio_playing: run_ready,
    }
}

/// The fixed-timestep remainder for Cached-mode pacing: how much of the time
/// beyond one frame the caller should carry into the next frame's pace
/// window, and whether the replay is still continuous (`false` = a UI hitch;
/// re-anchor the timer and rebuild the audio streak instead of
/// fast-forwarding through the owed frames).
///
/// In plain terms: the UI only gets to advance the playhead when a repaint
/// tick happens, and ticks never land exactly on a frame boundary. If the
/// pace timer restarts at "now" on every advance, the few milliseconds of
/// overshoot are thrown away each frame — a fully-cached 60 fps comp on
/// ~16 ms ticks replayed at roughly HALF speed — while the audio engine kept
/// its own hardware clock. Sound pulled ahead, the >2-frame resync yanked it
/// back, over and over: the reported "audio lags even when everything is
/// cached". Carrying the remainder makes the long-run replay pace exactly
/// realtime, so picture and sound agree.
#[must_use]
pub fn cached_pace_carry(elapsed: f64, frame_dur: f64) -> (f64, bool) {
    // Tolerate up to 50 ms of overshoot (or one frame, for comps slower than
    // 20 fps) — a few UI ticks' worth of jitter, briefly repaid by catch-up.
    // More than that is a hitch — the window was dragged, the app stalled —
    // and repaying it would fast-forward the picture; re-anchor instead.
    let slack = frame_dur.max(0.05);
    let over = (elapsed - frame_dur).max(0.0);
    if over <= slack {
        (over, true)
    } else {
        (0.0, false)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // ---- Cached-mode render-gated stepping (K-171) ----

    #[test]
    fn cached_replay_advances_at_realtime_pace_with_audio() {
        let fd = 1.0 / 60.0;
        // Frames all cached (ready, run ready). Before a frame's worth of
        // time: hold — with sound running (the stretch ahead is ready).
        let early = cached_step(true, fd * 0.5, fd, true);
        assert!(!early.advance && !early.request_next && early.audio_playing);
        // Once a frame's time has passed: advance, audio on.
        let s = cached_step(true, fd, fd, true);
        assert!(s.advance && !s.request_next && s.audio_playing);
    }

    #[test]
    fn cached_building_render_gates_and_mutes_until_ready() {
        let fd = 1.0 / 30.0;
        // Next frame not ready: hold, request it, audio paused — however long
        // we have been waiting.
        let waiting = cached_step(false, fd * 5.0, fd, false);
        assert!(!waiting.advance && waiting.request_next && !waiting.audio_playing);
        // It arrives (now ready) after a long wait: advance immediately, but
        // the stretch ahead is still rendering — sound stays paused (no
        // start-stop flapping at the render's crawling edge).
        let arrived = cached_step(true, fd * 5.0, fd, false);
        assert!(arrived.advance && !arrived.request_next && !arrived.audio_playing);
    }

    /// Regression (tester report): with every frame cached, replay must hold
    /// realtime pace long-run. Restarting the pace timer at "now" on each
    /// advance discarded the per-frame overshoot; on 16 ms ticks a 60 fps
    /// comp advanced every OTHER tick (~half speed), audio ran ahead on its
    /// own clock, and the >2-frame resync yanked it back for ever. The carry
    /// keeps the remainder, so the drift never accumulates.
    #[test]
    fn cached_replay_long_run_pace_is_exactly_realtime() {
        let fd = 1.0 / 60.0;
        let tick = 0.016; // the UI repaint cadence
        let mut baseline = 0.0; // when the shown frame's pace window opened
        let mut now = 0.0;
        let mut advances = 0u32;
        for _ in 0..625 {
            // 10 s of ticks
            now += tick;
            let s = cached_step(true, now - baseline, fd, true);
            if s.advance {
                let (carry, _continuous) = cached_pace_carry(now - baseline, fd);
                baseline = now - carry; // == baseline + fd while continuous
                advances += 1;
            }
        }
        // 10 s at 60 fps = 600 frames. The old restart-at-now pacing managed
        // ~312 here — half speed. Allow one frame of edge slack.
        assert!((599..=601).contains(&advances), "advances = {advances}");
    }

    /// The carry itself: ordinary tick jitter is repaid, a hitch is not.
    #[test]
    fn cached_pace_carry_repays_jitter_but_reanchors_on_a_hitch() {
        let fd = 1.0 / 60.0;
        // Landed 5 ms past the boundary: carry the 5 ms, still continuous.
        let (carry, cont) = cached_pace_carry(fd + 0.005, fd);
        assert!(cont && (carry - 0.005).abs() < 1e-9);
        // Landed exactly on it: nothing to carry.
        assert_eq!(cached_pace_carry(fd, fd), (0.0, true));
        // Landed 200 ms late (window drag, app stall): re-anchor, streak over.
        assert_eq!(cached_pace_carry(0.2, fd), (0.0, false));
        // A 120 fps comp on 16 ms ticks overshoots by more than a frame every
        // time — that is the tick cadence, not a hitch (the 50 ms floor).
        let fd120 = 1.0 / 120.0;
        let (_, cont) = cached_pace_carry(fd120 + 0.016, fd120);
        assert!(cont, "sub-tick frame durations must not read as hitches");
    }

    /// Regression (owner report): audio used to wait out a quarter-second
    /// warm-up streak even when every frame was already cached, so each run
    /// began with a few silent frames. The gate is readiness *ahead* now: a
    /// ready run gets sound from its very first frame; a still-rendering one
    /// gets none until the coming quarter-second is cached.
    #[test]
    fn cached_audio_starts_with_the_first_frame_of_a_ready_run() {
        let fd = 1.0 / 60.0;
        // The very first tick of playback, everything cached: sound at once —
        // even while holding frame 0 for pace, before any advance.
        let first = cached_step(true, 0.0, fd, true);
        assert!(first.audio_playing, "no warm-up silence on a cached run");
        // The same first tick with the run still rendering: silent.
        let building = cached_step(true, 0.0, fd, false);
        assert!(!building.audio_playing);
        // The lookahead is a quarter-second of frames at any rate.
        assert_eq!(cached_audio_lookahead(60.0), 15);
        assert_eq!(cached_audio_lookahead(24.0), 6);
        assert_eq!(cached_audio_lookahead(1.0), 1);
    }

    // ---- FrameRing ----

    /// The ring fills to its stated capacity and then refuses (that refusal
    /// is the scheduler's back-pressure).
    #[test]
    fn ring_fills_to_capacity_then_rejects() {
        let mut ring = FrameRing::with_capacity(3);
        assert_eq!(ring.free(), 3);
        assert!(ring.is_empty());
        assert!(ring.push(10, "a"));
        assert!(ring.push(11, "b"));
        assert!(ring.push(12, "c"));
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.free(), 0);
        assert!(!ring.push(13, "d"), "a full ring must refuse");
        // Duplicates refuse even with space free.
        let mut ring2 = FrameRing::<&str>::with_capacity(3);
        assert!(ring2.push(5, "x"));
        assert!(!ring2.push(5, "y"), "duplicate frame numbers must refuse");
        assert_eq!(ring2.len(), 1);
    }

    /// `take_upto` drains exactly the frames whose time has come, oldest
    /// first, and frees their slots; future frames stay shelved.
    #[test]
    fn take_upto_drains_only_due_frames_and_frees_slots() {
        let mut ring = FrameRing::with_capacity(4);
        // Pushed out of order on purpose — completion order isn't frame order.
        assert!(ring.push(7, "g"));
        assert!(ring.push(5, "e"));
        assert!(ring.push(8, "h"));
        assert!(ring.push(6, "f"));
        assert_eq!(ring.free(), 0);
        let due = ring.take_upto(6);
        assert_eq!(due, vec![(5, "e"), (6, "f")]);
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.free(), 2);
        assert!(ring.contains(7) && ring.contains(8));
        // Nothing due yet → nothing drained.
        assert!(ring.take_upto(6).is_empty());
    }

    /// The §4 present rule: newest due frame wins, older due frames are
    /// dropped (the clock passed them), and when nothing is due the answer
    /// is None — hold the last presented frame.
    #[test]
    fn newest_ready_presents_newest_due_drops_passed_holds_when_late() {
        let mut ring = FrameRing::with_capacity(5);
        for n in 10..15 {
            assert!(ring.push(n, n * 100));
        }
        // Clock at 12: frame 12 presents; 10 and 11 are dropped, not shown.
        assert_eq!(ring.newest_ready(12), Some((12, 1200)));
        assert!(!ring.contains(10) && !ring.contains(11));
        assert_eq!(ring.len(), 2); // 13 and 14 still wait their turn
                                   // Clock hasn't reached 13 yet: hold (None), shelf untouched.
        assert_eq!(ring.newest_ready(12), None);
        assert_eq!(ring.len(), 2);
        // Empty ring holds too.
        let mut empty = FrameRing::<u64>::with_capacity(2);
        assert_eq!(empty.newest_ready(99), None);
    }

    /// A zero-capacity request still yields a usable one-slot mailbox (the
    /// scrub configuration).
    #[test]
    fn ring_capacity_zero_becomes_one_slot_mailbox() {
        let mut ring = FrameRing::with_capacity(0);
        assert_eq!(ring.free(), 1);
        assert!(ring.push(1, "only"));
        assert!(!ring.push(2, "no room"));
    }

    // ---- Lookahead ----

    /// Consistently cheap frames settle at the minimum cushion of 8.
    #[test]
    fn lookahead_cheap_frames_clamp_to_minimum() {
        let mut la = Lookahead::new();
        for _ in 0..COST_WINDOW {
            la.record(0.001); // 1 ms frames
        }
        assert_eq!(la.frames(60.0), MIN_LOOKAHEAD_FRAMES);
        // No history at all also answers the minimum.
        assert_eq!(Lookahead::new().frames(60.0), MIN_LOOKAHEAD_FRAMES);
    }

    /// Consistently heavy frames grow the cushion, capped at 16.
    #[test]
    fn lookahead_expensive_frames_clamp_to_maximum() {
        let mut la = Lookahead::new();
        for _ in 0..COST_WINDOW {
            la.record(0.5); // half-second frames: 2 × 0.5 × 60 = 60 → cap
        }
        assert_eq!(la.frames(60.0), MAX_LOOKAHEAD_FRAMES);
    }

    /// Between the clamps the answer follows the formula and never shrinks
    /// as costs grow (monotone in the p95 cost).
    #[test]
    fn lookahead_tracks_cost_monotonically_between_clamps() {
        // 2 × cost × 60 fps: 0.1 s → 12 frames, exactly mid-band.
        let mut la = Lookahead::new();
        for _ in 0..COST_WINDOW {
            la.record(0.1);
        }
        assert_eq!(la.frames(60.0), 12);
        // Sweep rising costs; the answer must never decrease.
        let mut last = 0;
        for step in 1..=20 {
            let cost = 0.01 * f64::from(step);
            let mut la = Lookahead::new();
            for _ in 0..COST_WINDOW {
                la.record(cost);
            }
            let frames = la.frames(60.0);
            assert!(frames >= last, "lookahead shrank as cost grew");
            assert!((MIN_LOOKAHEAD_FRAMES..=MAX_LOOKAHEAD_FRAMES).contains(&frames));
            last = frames;
        }
        // p95 means one outlier in the window is *seen* (near-worst-case).
        let mut la = Lookahead::new();
        for _ in 0..COST_WINDOW - 2 {
            la.record(0.001);
        }
        la.record(0.1);
        la.record(0.1);
        assert_eq!(la.frames(60.0), 12);
        // Nonsense inputs are ignored / answered safely.
        let mut la = Lookahead::new();
        la.record(f64::NAN);
        la.record(-1.0);
        la.record(f64::INFINITY);
        assert_eq!(la.frames(60.0), MIN_LOOKAHEAD_FRAMES);
        la.record(0.1);
        assert_eq!(la.frames(0.0), MIN_LOOKAHEAD_FRAMES);
        assert_eq!(la.frames(f64::NAN), MIN_LOOKAHEAD_FRAMES);
    }

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

    // ---- Planner ----

    /// The planner returns the nearest frame in [clock, target] that is
    /// neither shelved nor in flight, and None when the window is covered.
    #[test]
    fn planner_finds_nearest_unscheduled_frame_or_none() {
        let mut ring = FrameRing::with_capacity(8);
        assert!(ring.push(100, ()));
        assert!(ring.push(101, ()));
        let mut in_flight: HashSet<u64> = HashSet::new();
        in_flight.insert(102);
        let scheduled = |n: u64| in_flight.contains(&n);

        // 100, 101 shelved; 102 in flight → 103 is next.
        assert_eq!(
            next_frame_to_schedule(100, 108, &ring, scheduled),
            Some(103)
        );
        // Window fully covered → None (the loop parks).
        let all = |_: u64| true;
        assert_eq!(next_frame_to_schedule(100, 108, &ring, all), None);
        // Single-frame window, uncovered → that frame.
        let none = |_: u64| false;
        let empty = FrameRing::<()>::with_capacity(1);
        assert_eq!(next_frame_to_schedule(7, 7, &empty, none), Some(7));
        // Degenerate window (target behind clock) → None.
        assert_eq!(next_frame_to_schedule(9, 8, &empty, none), None);
    }
}

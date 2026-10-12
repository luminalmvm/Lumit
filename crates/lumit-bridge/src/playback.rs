//! The playback scheduler's pure decisions (docs/impl/playback-scheduler.md §5).
//!
//! # In plain terms
//!
//! During playback the worker no longer renders one frame and shows it in the
//! same breath. It renders AHEAD of the clock into a small ring of finished
//! frames, and shows each one only when it is due. The point is slack: a span
//! of cheap (or cached) frames fills the ring, and when an expensive frame
//! comes along it can spend the banked time instead of stalling the picture.
//! How far ahead to render is not guessed — it adapts to what frames have
//! actually been costing, measured as they happen.
//!
//! This module holds the arithmetic of those decisions — the cost window and
//! the lookahead formula — kept free of the GPU and the worker loop so they
//! are testable on their own. The ring itself lives with the worker's
//! `Playback` state; its entries are `lumit_render::PreparedFrame`s.

use std::collections::VecDeque;

/// How many recent render costs the p95 is taken over. Small on purpose: the
/// lookahead should follow the comp the playhead is in NOW, not the average of
/// the whole session.
const COST_WINDOW: usize = 32;

/// How many frames past the one being rendered have their source decodes
/// posted to the decode-ahead thread. Enough to keep that thread busy through
/// one composite; more would just fill the decoded-frame cache further ahead
/// than the ring ever presents.
pub(crate) const PREFETCH_AHEAD: u64 = 4;

/// The frames whose edit points playback should ask about now, as "after this
/// one, up to and including that one": the clips starting in there have their
/// files opened by the decode-ahead threads
/// ([`lumit_render::HeadlessRenderer::cut_wants`]).
///
/// It reaches about a second past the frame being rendered. Four frames is
/// enough warning to decode the next frame of a file that is playing and
/// nowhere near enough to open one that is not, which costs tens of
/// milliseconds on a good day, so a cut to a new file used to play late.
///
/// `posted` is how far this run has already asked, so each edit point is
/// asked about once. One further on than this window reaches means the
/// playhead has gone back (a loop coming round) and the asking starts again.
/// `None` when there is nothing new to ask.
pub(crate) fn cut_window(
    frame: u64,
    posted: Option<u64>,
    fps: f64,
    speed: u64,
    last: u64,
) -> Option<(u64, u64)> {
    let second = (fps.round().max(1.0) as u64).saturating_mul(speed);
    let through = frame.saturating_add(second).min(last);
    // Nearer than this the ordinary read-ahead has already posted the frames
    // themselves.
    let near = frame.saturating_add(PREFETCH_AHEAD * speed);
    let after = posted
        .filter(|p| *p <= through)
        .map_or(near, |p| p.max(near));
    (after < through).then_some((after, through))
}

/// The measured cost of recent renders, for the scheduler's lookahead.
///
/// The impl note asks for the 95th percentile rather than the mean: lookahead
/// exists to absorb the OCCASIONAL slow frame, so it must be sized by what the
/// slow frames cost, not by the typical one.
#[derive(Default)]
pub(crate) struct CostWindow {
    samples: VecDeque<f64>,
}

impl CostWindow {
    /// Record one render's measured cost in seconds.
    pub(crate) fn push(&mut self, cost_secs: f64) {
        if !cost_secs.is_finite() || cost_secs < 0.0 {
            return;
        }
        if self.samples.len() == COST_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(cost_secs);
    }

    /// The 95th-percentile cost over the window, or `None` before any sample —
    /// a fresh run has nothing to size its lookahead by yet.
    pub(crate) fn p95(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = self.samples.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // Nearest-rank p95: the value 95% of samples sit at or below.
        let rank = ((sorted.len() as f64) * 0.95).ceil() as usize;
        Some(sorted[rank.saturating_sub(1).min(sorted.len() - 1)])
    }
}

/// How many frames ahead of the clock the scheduler renders — the ring
/// capacity. The impl note's formula, pinned: `clamp(2 × p95 cost × fps, 8,
/// 16)`. Before any cost has been measured the floor applies.
///
/// The floor is also the VRAM ceiling to know about: at worst 16 display
/// textures are held at once (~8 MB each at full 1080p, less at any preview
/// scale), freed the moment they are presented or playback stops.
pub(crate) fn lookahead_frames(p95_cost: Option<f64>, fps: f64) -> usize {
    let frames = match p95_cost {
        Some(cost) if fps > 0.0 => (2.0 * cost * fps).ceil() as usize,
        _ => 0,
    };
    frames.clamp(8, 16)
}

/// When the next every-frame present falls due, given when the one just shown
/// was scheduled and when it actually went out.
///
/// The schedule is a GRID, not a stopwatch. The old rule — "at least one comp
/// period since the last actual present" — re-anchored the clock at every
/// present, so every scrap of loop overhead (the sleep waking a little late,
/// the turn's bookkeeping) was added to every frame and never paid back: a
/// 60 fps comp could not play faster than about 55, cached or not, and the
/// shortfall grew with the rate. Keeping the due times on a grid means a
/// present that goes out a millisecond late leaves the NEXT one due at the
/// grid time, and the rate holds exactly.
///
/// A present more than one whole period late is a genuine stall, and there the
/// grid is re-anchored at now instead: every-frame never skips a frame and
/// never bursts faster than the comp's rate to catch up, so time lost
/// to a stall stays lost — playback continues at rate from where it is.
pub(crate) fn next_present_due(
    scheduled: Option<std::time::Instant>,
    now: std::time::Instant,
    period: std::time::Duration,
) -> std::time::Instant {
    match scheduled {
        Some(due) if now < due + period => due + period,
        _ => now + period,
    }
}

/// Whether the every-frame render turn should hold off compositing its next
/// frame because a copy of it is on its way up from disk.
///
/// `asked_ago` is how long ago the disk tier was asked for the frame, `None`
/// when it never was (nothing to wait for). A read plus decompression lands
/// within a few loop turns; [`DISK_LOAD_GRACE`] bounds the wait so a load that
/// never comes (file deleted underneath the session) degrades to a composite
/// rather than a hang. Only every-frame playback waits at all — it promises
/// every frame, not any particular arrival time — and only for frames not yet
/// held anywhere above disk; adaptive playback keeps chasing its clock.
pub(crate) fn wait_for_disk(asked_ago: Option<std::time::Duration>) -> bool {
    asked_ago.is_some_and(|ago| ago < DISK_LOAD_GRACE)
}

/// How long a pending disk copy is given before the frame is composited
/// anyway. Generous beside one read (a few milliseconds) so a queue of
/// pre-asked loads can drain; tiny beside the composite it saves.
pub(crate) const DISK_LOAD_GRACE: std::time::Duration = std::time::Duration::from_millis(50);

/// The order idle cache-fill visits frames around the playhead (docs/06 §5.5,
/// with the forward bias the owner asked for): two frames ahead for every one
/// behind — you are about to watch forward, but a small rewind should be warm
/// too. Yields every frame in `[first, last]` except the anchor itself,
/// nearest first per direction, and **both directions wrap**: playback loops
/// the work area, so the frame after the last one is the first, and that is
/// what the forward walk goes on to once it reaches the end. The walk ends
/// when every frame has been visited once, which is where the two ends meet.
pub(crate) fn fill_order(anchor: u64, first: u64, last: u64) -> impl Iterator<Item = u64> {
    // An empty or inverted range yields nothing rather than panicking. `clamp`
    // panics when its bounds cross, and they crossed for real: a work area
    // dragged before frame zero gave a negative first frame, which cast
    // unsigned became astronomically large, and the render worker died on
    // `min > max` — taking every later frame request with it. The op that
    // stores a work area now clamps it (lumit-core), so this is the second
    // line: a document from disk, or any future caller, cannot reach that
    // panic through here either (docs/14: no panics in engine crates).
    // A range of one frame yields nothing below, which is what an impossible
    // range should yield.
    let (first, last) = if first > last {
        (anchor, anchor)
    } else {
        (first, last)
    };
    let anchor = anchor.clamp(first, last);
    let mut ahead = anchor;
    let mut behind = anchor;
    let mut left = last - first;
    let mut step = 0u64;
    std::iter::from_fn(move || {
        if left == 0 {
            return None;
        }
        left -= 1;
        // The pattern: positions 0 and 1 of every three go forward, 2 back.
        // Neither direction ever runs out, because each wraps round the work
        // area; the count above is what stops the two from meeting.
        let forward = step % 3 != 2;
        step += 1;
        if forward {
            ahead = if ahead == last { first } else { ahead + 1 };
            Some(ahead)
        } else {
            behind = if behind == first { last } else { behind - 1 };
            Some(behind)
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The fill order walks outward from the playhead, two ahead for every
    /// one behind, covers every frame exactly once, and wraps at either end
    /// of the work area — playback loops it, so the frame after the last is
    /// the first — keeping the 2:1 interleave all the way round.
    #[test]
    fn the_fill_order_is_forward_biased_and_complete() {
        let order: Vec<u64> = fill_order(10, 0, 20).collect();
        assert_eq!(&order[..6], &[11, 12, 9, 13, 14, 8], "two ahead, one back");
        let mut all = order.clone();
        all.sort_unstable();
        let expected: Vec<u64> = (0..=20).filter(|&f| f != 10).collect();
        assert_eq!(all, expected, "every frame once, never the anchor");

        // Anchor near the end: the forward walk wraps to the start instead of
        // stopping, the backward walk wraps to the end, 2:1 throughout.
        let wrapped: Vec<u64> = fill_order(4, 0, 5).collect();
        assert_eq!(wrapped, vec![5, 0, 3, 1, 2], "ahead wraps 5 -> 0");
        let from_start: Vec<u64> = fill_order(0, 0, 5).collect();
        assert_eq!(from_start, vec![1, 2, 5, 3, 4], "behind wraps 0 -> 5");
        // A work area that does not start at zero wraps to ITS start.
        assert_eq!(fill_order(9, 7, 9).collect::<Vec<_>>(), vec![7, 8]);
        // Anchor clamped into range, single-frame comp yields nothing.
        assert_eq!(fill_order(99, 0, 0).count(), 0);
        // An impossible range yields nothing rather than panicking. This is the
        // shape a work area outside the comp used to take — a negative first
        // frame cast unsigned — and `clamp` panics when its bounds cross, which
        // killed the render worker outright.
        assert_eq!(fill_order(7, u64::MAX - 100, 363).count(), 0);
        assert_eq!(fill_order(0, 9, 4).count(), 0);
    }

    /// **The pacing-drift regression.** Presents are scheduled on a grid: a
    /// present that goes out a little late (the sleep woke late, the turn had
    /// bookkeeping) leaves the next one due at the grid time, so the overhead
    /// is absorbed instead of compounding. Under the old
    /// stopwatch-from-last-present rule each frame added its own lateness to
    /// the schedule and a 60 fps comp could never actually play at 60.
    #[test]
    fn the_present_grid_absorbs_loop_overhead_instead_of_compounding_it() {
        let period = std::time::Duration::from_micros(16_667);
        let start = std::time::Instant::now();

        // Ten frames, each presented 2 ms after its due time — the overhead the
        // old rule accumulated. The grid must stay exactly period-spaced.
        let mut due = next_present_due(None, start, period);
        assert_eq!(due, start + period, "the first present anchors the grid");
        for n in 2..=10u32 {
            let presented = due + std::time::Duration::from_millis(2);
            due = next_present_due(Some(due), presented, period);
            assert_eq!(
                due,
                start + period * n,
                "lateness within a period never moves the grid"
            );
        }

        // A genuine stall — later than one whole period — re-anchors: every-frame
        // never bursts to catch up, so lost time stays lost and playback
        // continues at rate from where it is.
        let stalled = due + period * 3;
        let after = next_present_due(Some(due), stalled, period);
        assert_eq!(after, stalled + period, "a stall re-anchors at now");
    }
}

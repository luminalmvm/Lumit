//! Epoch cancellation (docs/impl/playback-scheduler.md §1): the mechanism by
//! which stale work stops itself.
//!
//! In plain terms: every piece of scheduled work carries a ticket stamped
//! with the number that was on the wall when it started. Scrubbing, stopping,
//! or editing turns the wall number over; workers glance at the wall between
//! small steps ("is my ticket still current?") and quietly stop if it isn't.
//! Nothing is ever force-killed — force-killing is how state gets corrupted —
//! everything checks and steps aside. The engineering rule (14 §cancellation)
//! is that every loop over frames, rows, nodes, or dispatches checks its
//! token at least every ~10 ms of work.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A worker's job was invalidated mid-flight; unwind quietly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// One interaction context's epoch counter (scrub, playback, device…).
/// Cloning shares the counter; `bump` invalidates every outstanding token.
#[derive(Clone, Default)]
pub struct Epoch(Arc<AtomicU64>);

impl Epoch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Invalidate all outstanding tokens (playhead moved, stop pressed…).
    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Stamp a token for work scheduled now.
    pub fn token(&self) -> EpochToken {
        EpochToken {
            epoch: self.clone(),
            seen: self.0.load(Ordering::Relaxed),
        }
    }
}

/// The ticket a job carries; check it between small steps.
pub struct EpochToken {
    epoch: Epoch,
    seen: u64,
}

impl EpochToken {
    #[inline]
    pub fn cancelled(&self) -> bool {
        self.epoch.0.load(Ordering::Relaxed) != self.seen
    }

    /// `token.check()?` — the form every worker loop uses.
    #[inline]
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Tokens issued before a bump cancel; tokens issued after don't.
    #[test]
    fn bump_invalidates_outstanding_tokens_only() {
        let epoch = Epoch::new();
        let old = epoch.token();
        assert!(old.check().is_ok());
        epoch.bump();
        assert!(old.cancelled());
        assert_eq!(old.check(), Err(Cancelled));
        let fresh = epoch.token();
        assert!(fresh.check().is_ok());
    }

    /// The impl-note cancellation-latency drill: a deliberately slow job
    /// checking its token at working granularity stops within 15 ms of the
    /// bump (docs/impl/playback-scheduler.md test plan #1).
    #[test]
    fn workers_stop_within_fifteen_milliseconds_of_a_bump() {
        let epoch = Epoch::new();
        let token = epoch.token();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            // A "4 second render": 4000 × ~1 ms steps, token check each step.
            for _ in 0..4000 {
                if token.check().is_err() {
                    let _ = tx.send(Instant::now());
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = tx.send(Instant::now());
        });
        std::thread::sleep(Duration::from_millis(30)); // let it get going
        let bumped_at = Instant::now();
        epoch.bump();
        let stopped_at = rx.recv().unwrap();
        worker.join().unwrap();
        let latency = stopped_at.saturating_duration_since(bumped_at);
        assert!(
            latency <= Duration::from_millis(15),
            "cancellation took {latency:?} (budget 15 ms)"
        );
    }
}

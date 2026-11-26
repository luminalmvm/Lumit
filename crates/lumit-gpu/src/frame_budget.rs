//! A frame that can never fit the memory budget, told apart from one that
//! merely has to wait for a cache to give memory back.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A frame whose own need is more than the whole budget, so no amount of
/// giving back would make room for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverBudget {
    pub tier: lumit_budget::Tier,
    /// Bytes the frame needs at its peak.
    pub wanted: u64,
    /// Bytes Lumit may use on that tier.
    pub budget: u64,
}

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "this frame needs {} of {} and Lumit may use {}",
            readable(self.wanted),
            self.tier.name(),
            readable(self.budget)
        )
    }
}

/// Bytes in megabytes, or gigabytes from one up.
fn readable(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / GB)
    } else {
        format!("{} MB", bytes >> 20)
    }
}

/// Shared by every handle on a device, as the ledger is.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    refuse: AtomicBool,
    over: AtomicU64,
    last: std::sync::Mutex<Option<OverBudget>>,
}

impl crate::GpuContext {
    /// Skip, rather than draw, a frame that can never fit the budget. The
    /// Viewer's renderer does; an export draws it anyway, and only slows.
    pub fn refuse_frames_over_budget(&self, on: bool) {
        self.over_budget.refuse.store(on, Ordering::Relaxed);
    }

    /// Whether frames that can never fit are skipped.
    #[must_use]
    pub fn refuses_frames_over_budget(&self) -> bool {
        self.over_budget.refuse.load(Ordering::Relaxed)
    }

    /// Called when the ledger has turned a frame of `estimate` bytes away
    /// after the cheap rungs of the ladder. Notes it if the frame needs more
    /// than the whole budget, and answers whether to skip it.
    pub fn skip_over_budget(&self, estimate: u64) -> bool {
        let tier = lumit_budget::Tier::Vram;
        let budget = self.ledger.budget(tier);
        if estimate <= budget {
            return false;
        }
        if let Ok(mut last) = self.over_budget.last.lock() {
            *last = Some(OverBudget {
                tier,
                wanted: estimate,
                budget,
            });
        }
        self.over_budget.over.fetch_add(1, Ordering::Relaxed);
        self.refuses_frames_over_budget()
    }

    /// How many frames so far could never fit. Read it either side of a
    /// render to tell whether that one was one of them.
    #[must_use]
    pub fn frames_over_budget(&self) -> u64 {
        self.over_budget.over.load(Ordering::Relaxed)
    }

    /// The last frame that could never fit.
    #[must_use]
    pub fn over_budget(&self) -> Option<OverBudget> {
        self.over_budget.last.lock().ok().and_then(|last| *last)
    }
}

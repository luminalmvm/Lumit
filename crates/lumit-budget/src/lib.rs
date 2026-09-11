//! The resource governor's ledger: what the renderer is holding, and whether it
//! may hold more (docs/13-PERFORMANCE-RULES.md §3).
//!
//! # In plain terms
//!
//! Every frame-sized thing the renderer makes — a decoded picture, an effect's
//! intermediate, a shutter sample, a mask, a flow field — is tens or hundreds of
//! megabytes, and until now each was allocated on its own judgement. Individually
//! every one of them is reasonable. Together they are however many the project
//! happens to ask for, and a project is a file: a comp at 8K with forty effects
//! and a thirty-two-sample shutter is a thing somebody can build by accident on
//! a Tuesday, and it is also a thing somebody can build on purpose.
//!
//! So there is one ledger, and everything frame-sized goes through it. Asking is
//! `reserve`, which answers yes with a [`Reservation`] or no with a reason. The
//! reservation is the memory: holding it is what makes the memory yours, dropping
//! it is what gives it back, and there is no way to give it back twice or to
//! forget. That is the whole mechanism.
//!
//! # The two things it is not
//!
//! **It is not an allocator.** It counts; it does not hand out bytes. A caller
//! reserves and then allocates, and the ledger's honesty depends on the two
//! agreeing — which is why `reserve_raster` exists, so the number reserved is
//! computed by the same checked multiplication the allocation uses rather than
//! by a second one that might round differently.
//!
//! **It is not a promise that the memory exists.** A budget is a policy — 70% of
//! the card, 60% of physical RAM — and the machine may still be short. What the
//! ledger guarantees is that *Lumit* stops asking at its own ceiling rather than
//! discovering the machine's, which is the difference between a calm refusal and
//! a driver reset.
//!
//! # Why refusal is not the only answer
//!
//! A denied reservation is not usually a failure to report. It is the trigger
//! for the degradation ladder (docs/13 §4): drop the preview tier, tile the
//! frame, fall back to a CPU path. [`Ledger::pressure`] is what a caller reads to
//! decide, and it is deliberately readable without reserving anything, so a
//! kernel can trim its own work before it asks.
//!
//! # Thread role
//!
//! `Send + Sync` and lock-free: the meters are atomics and `reserve` is a
//! compare-and-swap, so two workers racing for the last hundred megabytes get one
//! grant and one refusal rather than two grants (docs/14 §1.2). Nothing here
//! blocks, so it may be called from the GPU-submit thread, a decode thread or a
//! pool worker without the lock-ordering question arising at all.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub use lumit_ingress::{checked_area, checked_raster_bytes};

/// Which pool a reservation is against.
///
/// Two, because they run out separately and for different reasons: video memory
/// is a fixed lump on the card that another application competes for, and system
/// memory is shared with everything else the machine is doing. A frame that
/// would fit in one and not the other has to be refused on the one that is
/// short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// The graphics card's memory: textures, buffers, the work-texture pool.
    Vram,
    /// System memory: decoded pictures, the frame arena, cache tiers.
    Ram,
}

impl Tier {
    /// The name a refusal prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Tier::Vram => "video memory",
            Tier::Ram => "memory",
        }
    }
}

/// Why a reservation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudgetError {
    /// The tier is full. `wanted` is what was asked for, `free` what was left.
    ///
    /// Both numbers are in the sentence on purpose: "out of memory" is not
    /// actionable and "this frame needs 1.2 GB of video memory and 400 MB is
    /// free" tells somebody to lower the preview resolution.
    #[error("this needs {wanted} bytes of {} and {free} is free", tier.name())]
    Denied { tier: Tier, wanted: u64, free: u64 },

    /// The sizes handed in did not multiply — a raster whose width times height
    /// times depth does not fit in a `u64`, which is a number out of a file
    /// rather than a picture.
    #[error("the sizes do not make sense together")]
    Overflow,
}

impl From<lumit_ingress::IngressError> for BudgetError {
    fn from(_: lumit_ingress::IngressError) -> Self {
        // Everything `checked_raster_bytes` can say is "these do not multiply";
        // the byte and item ceilings are the reader's business, not the
        // ledger's.
        BudgetError::Overflow
    }
}

// ---------------------------------------------------------------------------
// Default budgets
// ---------------------------------------------------------------------------

/// What the ledger starts at when nobody has said otherwise.
///
/// Deliberately conservative and deliberately *a number*: docs/13 §3 wants 70%
/// of the card and 60% of physical RAM, and getting those figures needs a
/// platform call per platform (DXGI's `DedicatedVideoMemory`, Metal's
/// `recommendedMaxWorkingSetSize`, the largest device-local Vulkan heap) that
/// wgpu does not offer portably today. Until each is wired,
/// [`Ledger::set_budget`] is how the real figure arrives — from the frontend's
/// preference, exactly as the frame cache's budget already does — and this is
/// what a build that has not been told operates at.
///
/// Two gigabytes of video memory is a 4K comp with a deep stack and room to
/// spare, and is inside the smallest card anybody runs this on. Four gigabytes
/// of system memory against the 16 GB reference machine is a quarter of it,
/// beside a frame cache that has its own budget on top.
pub const DEFAULT_VRAM_BUDGET: u64 = 2 << 30;
/// See [`DEFAULT_VRAM_BUDGET`].
pub const DEFAULT_RAM_BUDGET: u64 = 4 << 30;

/// The share of a reported pool the governor will spend, per docs/13 §3.
///
/// Used by whoever *can* ask the platform, so the percentage lives in one place
/// rather than at each call site that learns a figure.
#[must_use]
pub fn vram_budget_for(reported_bytes: u64) -> u64 {
    reported_bytes.saturating_mul(70) / 100
}

/// See [`vram_budget_for`].
#[must_use]
pub fn ram_budget_for(physical_bytes: u64) -> u64 {
    physical_bytes.saturating_mul(60) / 100
}

// ---------------------------------------------------------------------------
// The meter
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Meter {
    budget: AtomicU64,
    used: AtomicU64,
    /// The most that was ever held at once, for the readout and for the tests
    /// that assert a change lowered it.
    peak: AtomicU64,
    /// How many reservations this tier has refused, so "it went slowly" can be
    /// told apart from "it went slowly because it kept being refused".
    denials: AtomicU64,
}

impl Meter {
    fn new(budget: u64) -> Self {
        Meter {
            budget: AtomicU64::new(budget),
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            denials: AtomicU64::new(0),
        }
    }

    /// Take `bytes` if they fit, atomically.
    ///
    /// A compare-and-swap loop rather than a fetch-add and a check afterwards:
    /// two workers racing for the last hundred megabytes must produce one grant
    /// and one refusal, and a fetch-add would let both through and then have
    /// both discover it, by which point both have allocated.
    fn take(&self, bytes: u64) -> Result<(), (u64, u64)> {
        let budget = self.budget.load(Ordering::Relaxed);
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = used.checked_add(bytes) else {
                self.denials.fetch_add(1, Ordering::Relaxed);
                return Err((bytes, budget.saturating_sub(used)));
            };
            if next > budget {
                self.denials.fetch_add(1, Ordering::Relaxed);
                return Err((bytes, budget.saturating_sub(used)));
            }
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.peak.fetch_max(next, Ordering::Relaxed);
                    return Ok(());
                }
                Err(actual) => used = actual,
            }
        }
    }

    fn give_back(&self, bytes: u64) {
        // Saturating rather than wrapping: a double release would otherwise
        // make the ledger claim a negative amount is held, and a ledger that
        // reads below zero is worse than one that reads slightly high — the
        // first grants memory that is gone, the second only refuses early.
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let next = used.saturating_sub(bytes);
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(actual) => used = actual,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The ledger
// ---------------------------------------------------------------------------

/// One ledger per renderer. Shared by `Arc`, read and written from any thread.
#[derive(Debug)]
pub struct Ledger {
    vram: Meter,
    ram: Meter,
}

impl Default for Ledger {
    fn default() -> Self {
        Ledger {
            vram: Meter::new(DEFAULT_VRAM_BUDGET),
            ram: Meter::new(DEFAULT_RAM_BUDGET),
        }
    }
}

impl Ledger {
    /// A ledger at the default budgets.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Ledger::default())
    }

    /// A ledger at budgets the caller names — what a test uses to reach a
    /// ceiling without allocating a gigabyte to do it.
    #[must_use]
    pub fn with_budgets(vram: u64, ram: u64) -> Arc<Self> {
        Arc::new(Ledger {
            vram: Meter::new(vram),
            ram: Meter::new(ram),
        })
    }

    fn meter(&self, tier: Tier) -> &Meter {
        match tier {
            Tier::Vram => &self.vram,
            Tier::Ram => &self.ram,
        }
    }

    /// Ask for `bytes` of `tier`.
    ///
    /// The answer is a [`Reservation`] that holds the memory until it is
    /// dropped, or [`BudgetError::Denied`] naming what was wanted and what was
    /// free. A denial is the degradation ladder's trigger (docs/13 §4), not
    /// necessarily a failure to report.
    ///
    /// # Errors
    ///
    /// [`BudgetError::Denied`].
    pub fn reserve(self: &Arc<Self>, tier: Tier, bytes: u64) -> Result<Reservation, BudgetError> {
        self.meter(tier)
            .take(bytes)
            .map_err(|(wanted, free)| BudgetError::Denied { tier, wanted, free })?;
        Ok(Reservation {
            ledger: Arc::clone(self),
            tier,
            bytes,
        })
    }

    /// [`Ledger::reserve`] for a raster, worked out by the same checked
    /// multiplication the allocation will use.
    ///
    /// The pairing is the point: a caller that reserves `w * h * 4` and then
    /// allocates `w * h * 8` has a ledger that lies, and a ledger that lies is
    /// worse than none because it will confidently grant the frame that breaks
    /// the machine.
    ///
    /// # Errors
    ///
    /// [`BudgetError::Overflow`] if the sizes do not multiply;
    /// [`BudgetError::Denied`] if they do and there is no room.
    pub fn reserve_raster(
        self: &Arc<Self>,
        tier: Tier,
        width: u64,
        height: u64,
        channels: u64,
        bytes_per_sample: u64,
    ) -> Result<Reservation, BudgetError> {
        let bytes = checked_raster_bytes(width, height, channels, bytes_per_sample)?;
        self.reserve(tier, bytes)
    }

    /// Take `bytes` if they fit, and answer `None` rather than an error if they
    /// do not — for the caller whose response to "no" is to do less rather than
    /// to stop.
    #[must_use]
    pub fn try_reserve(self: &Arc<Self>, tier: Tier, bytes: u64) -> Option<Reservation> {
        self.reserve(tier, bytes).ok()
    }

    /// What is held right now.
    #[must_use]
    pub fn used(&self, tier: Tier) -> u64 {
        self.meter(tier).used.load(Ordering::Relaxed)
    }

    /// The ceiling.
    #[must_use]
    pub fn budget(&self, tier: Tier) -> u64 {
        self.meter(tier).budget.load(Ordering::Relaxed)
    }

    /// What is left.
    #[must_use]
    pub fn free(&self, tier: Tier) -> u64 {
        self.budget(tier).saturating_sub(self.used(tier))
    }

    /// The most that was ever held at once.
    #[must_use]
    pub fn peak(&self, tier: Tier) -> u64 {
        self.meter(tier).peak.load(Ordering::Relaxed)
    }

    /// How many reservations this tier has refused.
    #[must_use]
    pub fn denials(&self, tier: Tier) -> u64 {
        self.meter(tier).denials.load(Ordering::Relaxed)
    }

    /// Move the ceiling — the frontend's preference, or a platform figure once
    /// somebody has asked the platform.
    ///
    /// Lowering it below what is already held is allowed and does not take
    /// anything away: the reservations that exist are memory that exists. What
    /// it does is refuse everything new until enough has been given back, which
    /// is exactly what shrinking a budget under pressure should do.
    pub fn set_budget(&self, tier: Tier, bytes: u64) {
        self.meter(tier).budget.store(bytes, Ordering::Relaxed);
    }

    /// Forget the peak and the denial count. The readout's "since when", not a
    /// change to what is held.
    pub fn reset_statistics(&self) {
        for meter in [&self.vram, &self.ram] {
            meter
                .peak
                .store(meter.used.load(Ordering::Relaxed), Ordering::Relaxed);
            meter.denials.store(0, Ordering::Relaxed);
        }
    }

    /// How close to the ceiling this tier is, without reserving anything.
    ///
    /// Readable by a kernel that would rather trim its own work than be
    /// refused — the particle system's cap rung is the first of those
    /// (docs/13 §4 step 3). Deliberately cheap: two relaxed atomic loads, so
    /// asking once per effect costs nothing measurable.
    #[must_use]
    pub fn pressure(&self, tier: Tier) -> Pressure {
        let (used, budget) = (self.used(tier), self.budget(tier));
        if budget == 0 {
            return Pressure::Full;
        }
        // Integer percent rather than a float: this is compared against fixed
        // thresholds and reported in a status line, and neither wants a value
        // that differs in the last bit between machines (docs/14 §3).
        let percent = used.saturating_mul(100) / budget;
        match percent {
            0..=74 => Pressure::Easy,
            75..=89 => Pressure::Tight,
            90..=99 => Pressure::Severe,
            _ => Pressure::Full,
        }
    }
}

/// How close a tier is to its ceiling.
///
/// The signal the degradation ladder reads. Ordered, so `>=` is a meaningful
/// test and a caller can say "trim above Tight" in one comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    /// Under three quarters. Nothing to do.
    Easy,
    /// Three quarters to nine tenths. Cheap trims are worth making.
    Tight,
    /// Nine tenths and up. Trim properly; a refusal is close.
    Severe,
    /// At or past the ceiling. Anything new will be refused.
    Full,
}

impl Pressure {
    /// Whether a caller that can do less should.
    #[must_use]
    pub const fn should_trim(self) -> bool {
        matches!(self, Pressure::Severe | Pressure::Full)
    }
}

// ---------------------------------------------------------------------------
// The reservation
// ---------------------------------------------------------------------------

/// Memory the ledger has granted, held until this is dropped.
///
/// # In plain terms
///
/// This *is* the accounting. There is no "release" to call and no way to call it
/// twice: the memory is given back when the value goes out of scope, including
/// when that happens because something failed half way through, which is the
/// case a hand-written release is always eventually missing.
///
/// Keep it beside the thing it paid for — in the same struct, in the same `Vec`
/// — so that the two cannot be separated by a later edit. A reservation that
/// outlives its allocation only makes the ledger pessimistic; one that dies
/// first makes it wrong.
#[derive(Debug)]
pub struct Reservation {
    ledger: Arc<Ledger>,
    tier: Tier,
    bytes: u64,
}

impl Reservation {
    /// How much this holds.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Which tier it is against.
    #[must_use]
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Fold another reservation of the same tier into this one.
    ///
    /// For the caller accumulating a frame's worth and wanting one value to
    /// drop at the end rather than a growing `Vec` of them.
    ///
    /// Same-tier only, and a mismatch **hands the other back** rather than
    /// answering false and dropping it. The difference matters: this takes
    /// ownership, so a bare `bool` would mean a caller that got the tiers wrong
    /// silently released memory it still holds, and the ledger would then grant
    /// that memory to somebody else. Giving it back makes the mistake
    /// impossible to make quietly.
    ///
    /// # Errors
    ///
    /// The reservation that was handed in, unchanged, when its tier is not this
    /// one's.
    pub fn absorb(&mut self, other: Reservation) -> Result<(), Reservation> {
        if other.tier != self.tier {
            return Err(other);
        }
        // Take the bytes across and stop `other`'s destructor giving them back:
        // the memory has not been released, only re-filed.
        self.bytes = self.bytes.saturating_add(other.bytes);
        std::mem::forget(other);
        Ok(())
    }

    /// Give part of it back early, keeping the rest.
    ///
    /// For a caller that reserved a worst case and then found out the real one —
    /// a frame that reserved for eight shutter samples and drew three.
    pub fn shrink_to(&mut self, bytes: u64) {
        let Some(giving_back) = self.bytes.checked_sub(bytes) else {
            return;
        };
        self.ledger.meter(self.tier).give_back(giving_back);
        self.bytes = bytes;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.meter(self.tier).give_back(self.bytes);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_reservation_holds_its_bytes_and_gives_them_back_when_it_drops() {
        let ledger = Ledger::with_budgets(1000, 1000);
        assert_eq!(ledger.used(Tier::Vram), 0);

        {
            let held = ledger.reserve(Tier::Vram, 400).unwrap();
            assert_eq!(held.bytes(), 400);
            assert_eq!(ledger.used(Tier::Vram), 400);
            assert_eq!(ledger.free(Tier::Vram), 600);

            // The other tier is untouched: they run out separately.
            assert_eq!(ledger.used(Tier::Ram), 0);
        }
        assert_eq!(ledger.used(Tier::Vram), 0, "dropping must give it back");
        // And the peak remembers what happened.
        assert_eq!(ledger.peak(Tier::Vram), 400);
    }

    #[test]
    fn a_refusal_names_what_was_wanted_and_what_was_free() {
        let ledger = Ledger::with_budgets(1000, 1000);
        let _held = ledger.reserve(Tier::Vram, 700).unwrap();

        let refused = ledger.reserve(Tier::Vram, 400).unwrap_err();
        assert_eq!(
            refused,
            BudgetError::Denied {
                tier: Tier::Vram,
                wanted: 400,
                free: 300
            }
        );
        // Both numbers are in the sentence, because "out of memory" is not
        // something anybody can act on.
        let words = refused.to_string();
        assert!(words.contains("400") && words.contains("300"), "{words}");
        assert!(words.contains("video memory"), "{words}");

        // A refusal takes nothing: what was held before is exactly what is held.
        assert_eq!(ledger.used(Tier::Vram), 700);
        assert_eq!(ledger.denials(Tier::Vram), 1);

        // And what does fit still fits.
        assert!(ledger.reserve(Tier::Vram, 300).is_ok());
    }

    #[test]
    fn two_threads_racing_for_the_last_of_it_get_one_grant_between_them() {
        // The property a fetch-add would not have: both would be let through
        // and both would discover it afterwards, by which point both have
        // allocated.
        for _ in 0..200 {
            let ledger = Ledger::with_budgets(100, 100);
            let a = Arc::clone(&ledger);
            let b = Arc::clone(&ledger);
            // The reservations are carried back out rather than tested for
            // success inside the thread: dropping one at the end of the closure
            // would give its bytes back before the other had asked, and both
            // would then be granted for the entirely innocent reason that they
            // never overlapped.
            let one = std::thread::spawn(move || a.reserve(Tier::Vram, 60));
            let two = std::thread::spawn(move || b.reserve(Tier::Vram, 60));
            let (one, two) = (one.join().unwrap(), two.join().unwrap());
            let granted = usize::from(one.is_ok()) + usize::from(two.is_ok());
            assert_eq!(granted, 1, "exactly one of two racing asks may win");
            assert_eq!(ledger.used(Tier::Vram), 60);
            drop((one, two));
            assert_eq!(ledger.used(Tier::Vram), 0);
        }
    }

    #[test]
    fn a_raster_is_reserved_by_the_arithmetic_that_will_allocate_it() {
        let ledger = Ledger::with_budgets(1 << 30, 1 << 30);
        // 1920 x 1080 RGBA half-float.
        let held = ledger.reserve_raster(Tier::Vram, 1920, 1080, 4, 2).unwrap();
        assert_eq!(held.bytes(), 1920 * 1080 * 4 * 2);

        // A raster out of a file whose numbers do not multiply is an overflow
        // rather than a wrap into something small and plausible.
        assert_eq!(
            ledger
                .reserve_raster(Tier::Vram, u64::MAX, 2, 4, 4)
                .unwrap_err(),
            BudgetError::Overflow
        );
    }

    #[test]
    fn a_reservation_that_would_overflow_the_tally_is_refused() {
        let ledger = Ledger::with_budgets(u64::MAX, u64::MAX);
        let _most = ledger.reserve(Tier::Vram, u64::MAX - 10).unwrap();
        let refused = ledger.reserve(Tier::Vram, 100).unwrap_err();
        assert!(matches!(refused, BudgetError::Denied { .. }));
    }

    #[test]
    fn absorbing_folds_two_into_one_without_releasing_either() {
        let ledger = Ledger::with_budgets(1000, 1000);
        let mut a = ledger.reserve(Tier::Vram, 100).unwrap();
        let b = ledger.reserve(Tier::Vram, 250).unwrap();
        assert_eq!(ledger.used(Tier::Vram), 350);

        assert!(a.absorb(b).is_ok());
        assert_eq!(a.bytes(), 350);
        assert_eq!(
            ledger.used(Tier::Vram),
            350,
            "folding must not release anything"
        );

        // A different tier is handed back rather than mis-filed — and, the part
        // that matters, rather than dropped: a caller that got the tiers wrong
        // must not silently release memory it still holds.
        let ram = ledger.reserve(Tier::Ram, 10).unwrap();
        let ram = a.absorb(ram).expect_err("a mismatched tier comes back");
        assert_eq!(ram.bytes(), 10);
        assert_eq!(ledger.used(Tier::Ram), 10);
        drop(ram);
        assert_eq!(ledger.used(Tier::Ram), 0);

        drop(a);
        assert_eq!(ledger.used(Tier::Vram), 0);
    }

    #[test]
    fn shrinking_gives_back_the_part_that_was_not_needed() {
        let ledger = Ledger::with_budgets(1000, 1000);
        // Reserved for eight shutter samples, drew three.
        let mut held = ledger.reserve(Tier::Vram, 800).unwrap();
        held.shrink_to(300);
        assert_eq!(held.bytes(), 300);
        assert_eq!(ledger.used(Tier::Vram), 300);

        // Shrinking upward is not a way to get memory without asking.
        held.shrink_to(900);
        assert_eq!(held.bytes(), 300);
        assert_eq!(ledger.used(Tier::Vram), 300);
    }

    #[test]
    fn pressure_reads_the_ceiling_without_reserving_anything() {
        let ledger = Ledger::with_budgets(100, 100);
        assert_eq!(ledger.pressure(Tier::Vram), Pressure::Easy);

        let _a = ledger.reserve(Tier::Vram, 80).unwrap();
        assert_eq!(ledger.pressure(Tier::Vram), Pressure::Tight);
        assert!(!Pressure::Tight.should_trim());

        let _b = ledger.reserve(Tier::Vram, 12).unwrap();
        assert_eq!(ledger.pressure(Tier::Vram), Pressure::Severe);
        assert!(Pressure::Severe.should_trim());

        let _c = ledger.reserve(Tier::Vram, 8).unwrap();
        assert_eq!(ledger.pressure(Tier::Vram), Pressure::Full);

        // Reading it took nothing.
        assert_eq!(ledger.used(Tier::Vram), 100);
        // And the order is a usable comparison.
        assert!(Pressure::Full > Pressure::Easy);
    }

    #[test]
    fn lowering_the_budget_under_what_is_held_refuses_rather_than_confiscates() {
        let ledger = Ledger::with_budgets(1000, 1000);
        let held = ledger.reserve(Tier::Vram, 800).unwrap();

        // Another application took the card; the frontend shrinks the budget.
        ledger.set_budget(Tier::Vram, 500);
        assert_eq!(
            ledger.used(Tier::Vram),
            800,
            "memory that exists is not taken away by a policy change"
        );
        assert_eq!(ledger.free(Tier::Vram), 0);
        assert_eq!(ledger.pressure(Tier::Vram), Pressure::Full);
        assert!(ledger.reserve(Tier::Vram, 1).is_err());

        // And giving it back brings the ledger to where the new budget says.
        drop(held);
        assert_eq!(ledger.used(Tier::Vram), 0);
        assert!(ledger.reserve(Tier::Vram, 500).is_ok());
    }

    #[test]
    fn the_documented_shares_are_what_the_helpers_give() {
        // docs/13 §3: 70% of the card, 60% of physical RAM.
        assert_eq!(vram_budget_for(8 << 30), (8 << 30) * 70 / 100);
        assert_eq!(ram_budget_for(16 << 30), (16 << 30) * 60 / 100);
        // A platform that answers nothing gets nothing rather than a surprise.
        assert_eq!(vram_budget_for(0), 0);
        assert_eq!(ram_budget_for(0), 0);
    }

    #[test]
    fn a_ledger_with_no_budget_refuses_everything_rather_than_dividing_by_it() {
        let ledger = Ledger::with_budgets(0, 0);
        assert_eq!(ledger.pressure(Tier::Vram), Pressure::Full);
        assert!(ledger.reserve(Tier::Vram, 1).is_err());
        // Nought bytes is not a reservation worth making, but it is not a
        // division by zero either.
        assert!(ledger.reserve(Tier::Vram, 0).is_ok());
    }
}

//! The cache crate — **Nebula**: byte-budgeted stores per
//! docs/06-RENDER-PIPELINE.md §5. The RAM tier ([`ByteLru`]) is a
//! byte-budget store with cost-aware (GreedyDual-style) eviction and pinning
//! (§5.3); the disk tier ([`disk`]) parks frames in a cache folder and keeps an
//! [`index`] of them. The governor joins as the evaluator grows.
//!
//! The disk tier keeps an [`index`] of what it holds — size, recompute cost and
//! last use — so it can evict by the same rule the tiers above it use rather than
//! by the one thing a filesystem remembers.
//!
//! In plain terms: a cupboard with a strict size limit. When it's full and you
//! add something, it throws out the item that is the best bargain to lose —
//! one that hasn't been touched in a while, is big (frees the most room), and
//! is cheap to remake — while never touching anything you've *pinned* (the
//! frame on screen and its neighbours). "Budget by bytes, not by count" is the
//! point: one 4K frame costs what sixty thumbnails cost.

pub mod disk;
pub mod index;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub trait ByteSized {
    fn byte_size(&self) -> usize;
}

impl ByteSized for Vec<u8> {
    fn byte_size(&self) -> usize {
        self.len()
    }
}

/// Byte-budgeted store with cost-aware (GreedyDual-style) eviction
/// (docs/06-RENDER-PIPELINE.md §5.3).
///
/// When an insert would exceed the budget, the victim is the entry that scores
/// highest on **staleness × size ÷ recompute-cost** — the spec's "stale ×
/// cheap-to-recompute × large" preference. With equal sizes and a uniform cost
/// this is exactly least-recently-used; the size and cost terms only tilt the
/// choice when entries differ (evict the big cheap stale frame before the small
/// dear one). Cost is a caller-supplied hint via [`Self::insert_with_cost`];
/// plain [`Self::insert`] uses a uniform cost, so callers that don't measure
/// recompute cost keep size-aware LRU behaviour.
///
/// **Pinning** (docs §5.3): keys in the pin set are never chosen as victims —
/// the shell pins the displayed frame and a window around the playhead so
/// playback can't evict what it is about to show. If *only* pinned entries
/// remain and the store is still over budget, it is left slightly over rather
/// than dropping a pin (the pin set is small and short-lived, so the overage is
/// bounded); the excess clears as those keys are unpinned.
///
/// Eviction scans for the highest-scoring entry — O(n) on insert-over-budget,
/// fine at the hundreds-of-frames scale of the preview cache; the evaluator's
/// tier replaces the scan with a heap when n grows (documented debt).
pub struct ByteLru<K, V> {
    map: HashMap<K, Entry<V>>,
    pins: HashSet<K>,
    budget: usize,
    used: usize,
    tick: u64,
    /// Evicted entries the owner asked to see ([`ByteLru::collect_evictions`]),
    /// waiting to be drained. Empty — and never filled — unless it did.
    evicted: Vec<(K, V, u32)>,
    collect_evicted: bool,
    /// The governor's account of what this store holds, once somebody has
    /// registered it ([`ByteLru::account_against`]).
    account: Option<Account>,
}

/// A store's standing reservation against the resource governor (docs/13 §3).
///
/// # In plain terms
///
/// A byte budget of its own tells a store when *it* has had enough. It does
/// not tell it that four other stores, a decode queue and a frame in flight
/// have between them already spent the machine — and five private budgets that
/// cannot see each other is exactly how a process ends up holding five times
/// what any one of them would allow. This is the store saying what it holds
/// somewhere they can all be added up.
struct Account {
    ledger: std::sync::Arc<lumit_budget::Ledger>,
    tier: lumit_budget::Tier,
    /// What the ledger has granted for the current contents. Re-sized after
    /// every change; dropped, and so given back entire, when the store empties.
    held: Option<lumit_budget::Reservation>,
}

struct Entry<V> {
    value: V,
    bytes: usize,
    last_used: u64,
    /// Recompute-cost hint (arbitrary units, ≥ 1); higher means dearer to
    /// rebuild, so the eviction score divides by it. Uniform for plain inserts.
    cost: u32,
}

/// The GreedyDual eviction score (docs §5.3): higher = evict sooner. Stale
/// (large `now − last_used`), large (`bytes`) and cheap (small `cost`) all
/// raise it. `cost` is clamped ≥ 1 at insert, so this never divides by zero.
fn eviction_score<V>(e: &Entry<V>, now: u64) -> f64 {
    let staleness = now.saturating_sub(e.last_used) as f64;
    staleness * e.bytes as f64 / e.cost as f64
}

impl<K: Eq + Hash + Clone, V: ByteSized> ByteLru<K, V> {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            pins: HashSet::new(),
            budget: budget_bytes,
            used: 0,
            tick: 0,
            evicted: Vec::new(),
            collect_evicted: false,
            account: None,
        }
    }

    /// Account this store's contents against the governor's ledger, in `tier`
    /// (docs/13 §3: every frame-sized allocation is registered with size, tier
    /// and owner).
    ///
    /// Called once, by whoever builds the store, with the ledger of the device
    /// or machine the bytes are actually on. A store nobody registers keeps its
    /// private budget and nothing changes — which is what a test, and a store
    /// whose bytes are already counted somewhere else, both want.
    pub fn account_against(
        &mut self,
        ledger: std::sync::Arc<lumit_budget::Ledger>,
        tier: lumit_budget::Tier,
    ) {
        self.account = Some(Account {
            ledger,
            tier,
            held: None,
        });
        self.resync();
    }

    /// How close the tier this store is accounted in is to its ceiling, or
    /// `None` when nobody has registered it. The reading the degradation ladder
    /// steps on (docs/13 §4).
    #[must_use]
    pub fn pressure(&self) -> Option<lumit_budget::Pressure> {
        self.account.as_ref().map(|a| a.ledger.pressure(a.tier))
    }

    /// Bring the reservation into line with what the store now holds. Called
    /// after every change to the contents, so there is no path that changes
    /// them and forgets to say so.
    ///
    /// A refusal is **recorded, not obeyed**: the entries are already in
    /// memory when this runs, so the reservation simply falls short and the
    /// shortfall reads as pressure. Dropping one here instead would be the
    /// ledger deciding what the store holds, which is backwards — its job is
    /// to describe what is held truthfully enough that the ladder can act.
    fn resync(&mut self) {
        let want = self.used as u64;
        let Some(account) = self.account.as_mut() else {
            return;
        };
        let have = account
            .held
            .as_ref()
            .map_or(0, lumit_budget::Reservation::bytes);
        if want == have {
            return;
        }
        if want < have {
            match account.held.as_mut() {
                Some(held) if want > 0 => held.shrink_to(want),
                // Nothing held any more, so nothing to hold it with: dropping
                // the reservation gives the whole of it back at once.
                _ => drop(account.held.take()),
            }
            return;
        }
        let Some(more) = account.ledger.try_reserve(account.tier, want - have) else {
            return;
        };
        match account.held.as_mut() {
            Some(held) => {
                if let Err(unabsorbed) = held.absorb(more) {
                    // Cannot happen — one store, one tier — but keeping it is
                    // the safe direction: dropping it would hand back bytes
                    // the entries below are still sitting on.
                    account.held = Some(unabsorbed);
                }
            }
            None => account.held = Some(more),
        }
    }

    /// Keep evicted entries for the owner to collect with [`Self::take_evicted`]
    /// instead of dropping them — what a **demotion ladder** needs (docs/06 §5.3:
    /// a frame leaving VRAM falls to RAM, one leaving RAM falls to disk). Without
    /// this an eviction is invisible: the value is dropped inside the insert that
    /// displaced it and the tier below never hears that it exists.
    ///
    /// Off by default, and deliberately opt-in: a store nobody drains would grow
    /// a second copy of everything it evicted. The owner is expected to drain
    /// after each insert (the renderer does, so the log holds one turn's
    /// evictions at most).
    ///
    /// An explicit [`Self::clear`] is NOT logged: emptying a tier on purpose —
    /// the user's Clear cache — means the frames should go, not go downstairs.
    pub fn collect_evictions(&mut self) {
        self.collect_evicted = true;
    }

    /// Take the evicted entries logged since the last call, as
    /// `(key, value, recompute cost)`. Empty unless [`Self::collect_evictions`]
    /// was asked for. The cost travels with them because it is what decides
    /// whether the tier below is worth the trouble (docs §5.3: demote when
    /// recompute cost exceeds the cost of moving it down).
    pub fn take_evicted(&mut self) -> Vec<(K, V, u32)> {
        std::mem::take(&mut self.evicted)
    }

    /// The highest-scoring evictable (non-pinned) key, or None when every
    /// remaining entry is pinned. O(n) scan (see the type's note).
    fn victim(&self) -> Option<K> {
        let now = self.tick;
        self.map
            .iter()
            .filter(|(k, _)| !self.pins.contains(k))
            .max_by(|(_, a), (_, b)| {
                eviction_score(a, now)
                    .partial_cmp(&eviction_score(b, now))
                    .unwrap_or(Ordering::Equal)
            })
            .map(|(k, _)| k.clone())
    }

    /// Remove one entry, accounting for its bytes and — when the owner asked to
    /// see evictions — handing it to the log rather than dropping it. The one
    /// place an eviction happens, so the ladder cannot be bypassed by a second
    /// copy of this loop.
    fn evict(&mut self, victim: &K) {
        let Some(entry) = self.map.remove(victim) else {
            return;
        };
        self.used -= entry.bytes;
        if self.collect_evicted {
            self.evicted.push((victim.clone(), entry.value, entry.cost));
        }
    }

    /// Evict non-pinned entries by eviction score until within budget, or until
    /// only pinned entries remain (then stop — a pin is never dropped).
    fn evict_to_fit(&mut self) {
        while self.used > self.budget {
            let Some(victim) = self.victim() else {
                break; // only pins left; accept the bounded overage (§5.3)
            };
            self.evict(&victim);
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|e| {
            e.last_used = tick;
            &e.value
        })
    }

    /// Insert with a uniform recompute cost (size-aware LRU). See
    /// [`Self::insert_with_cost`] to supply a measured cost.
    pub fn insert(&mut self, key: K, value: V) -> bool {
        self.insert_with_cost(key, value, 1)
    }

    /// Insert with a recompute-cost hint (docs §5.3): dearer entries (higher
    /// `cost`) resist eviction, cheaper ones go first at equal staleness and
    /// size. Evicts non-pinned victims to make room; a value larger than the
    /// whole budget is not cached (returns false). If only pinned entries block
    /// the way, the store is left briefly over budget rather than dropping a pin.
    pub fn insert_with_cost(&mut self, key: K, value: V, cost: u32) -> bool {
        let bytes = value.byte_size();
        if bytes > self.budget {
            return false;
        }
        self.tick += 1;
        if let Some(old) = self.map.remove(&key) {
            self.used -= old.bytes;
        }
        // Make room before admitting, so the newcomer is never its own victim.
        while self.used + bytes > self.budget {
            let Some(victim) = self.victim() else {
                break; // only pins remain; accept the bounded overage (§5.3)
            };
            self.evict(&victim);
        }
        self.map.insert(
            key,
            Entry {
                value,
                bytes,
                last_used: self.tick,
                cost: cost.max(1),
            },
        );
        self.used += bytes;
        self.resync();
        true
    }

    /// Membership test that does not touch recency (cache-bar drawing polls
    /// every visible frame each paint; that must not distort eviction).
    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// Change the byte budget, evicting by eviction score until the store fits
    /// (Settings → Performance resizes the RAM cache live). Pins are respected.
    pub fn set_budget(&mut self, budget_bytes: usize) {
        self.budget = budget_bytes;
        self.evict_to_fit();
        self.resync();
    }

    /// Protect a key from eviction (docs §5.3): the shell pins the displayed
    /// frame and a window around the playhead. Pinning a key not present is
    /// remembered, so it also protects the frame once it lands. Idempotent.
    pub fn pin(&mut self, key: K) {
        self.pins.insert(key);
    }

    /// Lift a pin, letting the key be evicted again. Idempotent.
    pub fn unpin(&mut self, key: &K) {
        self.pins.remove(key);
    }

    /// Whether a key is currently pinned (present or not).
    pub fn is_pinned(&self, key: &K) -> bool {
        self.pins.contains(key)
    }

    /// Fetch without touching recency, for read-only per-paint consumers.
    /// The Scopes panel reads the current frame every paint to draw its
    /// waveform/histogram; like `contains_key`, that poll must not bump the
    /// frame's last-used tick and distort eviction. Use `get` where the read
    /// should count as a use (playback, scrubbing that should retain frames).
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|e| &e.value)
    }

    /// The same, to change a held value in place — without bumping recency and
    /// without changing what it costs.
    ///
    /// For bookkeeping the owner learns *after* the value went in: a frame that
    /// has since been copied to a lower tier, say. The value's size must not
    /// change, because the byte total was counted at insert time; nothing this
    /// answers with can change a size, which is why it is a note about the
    /// value rather than the value itself.
    pub fn peek_mut(&mut self, key: &K) -> Option<&mut V> {
        self.map.get_mut(key).map(|e| &mut e.value)
    }

    pub fn used_bytes(&self) -> usize {
        self.used
    }

    pub fn budget_bytes(&self) -> usize {
        self.budget
    }

    /// Every held key, in no particular order — for mirrors of the cache's
    /// contents (the Timeline's cache bar) that must never hold this cache's
    /// owner across a paint.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.used = 0;
        self.resync();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn v(n: usize) -> Vec<u8> {
        vec![0u8; n]
    }

    #[test]
    fn budget_is_enforced_in_bytes_and_lru_evicts_oldest() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(40)));
        assert!(lru.insert("b", v(40)));
        // Touch "a" so "b" is the oldest.
        assert!(lru.get(&"a").is_some());
        assert!(lru.insert("c", v(40)));
        assert!(lru.used_bytes() <= 100);
        assert!(lru.get(&"b").is_none(), "least-recently-used was evicted");
        assert!(lru.get(&"a").is_some());
        assert!(lru.get(&"c").is_some());
    }

    #[test]
    fn oversized_values_are_refused_not_thrashed() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(60)));
        assert!(!lru.insert("huge", v(1000)));
        assert!(lru.get(&"a").is_some(), "existing entries untouched");
    }

    #[test]
    fn reinserting_a_key_replaces_without_double_counting() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(60)));
        assert!(lru.insert("a", v(30)));
        assert_eq!(lru.used_bytes(), 30);
        assert_eq!(lru.len(), 1);
    }

    /// **The governor's ledger follows the contents** (docs/13 §3: "the ledger
    /// MUST equal reality"), through every way they can change — an insert, an
    /// eviction that insert caused, a replacement, a lowered budget, and the
    /// store being emptied.
    #[test]
    fn an_accounted_store_tells_the_ledger_exactly_what_it_holds() {
        let ledger = lumit_budget::Ledger::with_budgets(10_000, 10_000);
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        lru.account_against(std::sync::Arc::clone(&ledger), lumit_budget::Tier::Ram);
        let used = || ledger.used(lumit_budget::Tier::Ram);

        assert_eq!(used(), 0, "an empty store holds nothing");
        assert!(lru.insert("a", v(60)));
        assert_eq!(used(), 60);

        // The insert that evicts: the ledger is told the net, not the gross.
        assert!(lru.insert("b", v(60)));
        assert_eq!(lru.used_bytes(), 60, "\"a\" made room for \"b\"");
        assert_eq!(used(), 60);

        assert!(lru.insert("b", v(30)), "the same key, smaller");
        assert_eq!(used(), 30);

        lru.insert("c", v(30));
        assert_eq!(used(), 60);
        lru.set_budget(30);
        assert_eq!(lru.used_bytes(), 30, "the lower budget evicted one");
        assert_eq!(used(), 30);

        lru.clear();
        assert_eq!(used(), 0, "and emptying it gives back the whole of it");
    }

    /// A store nobody registered is exactly what it was: its own budget, and
    /// no opinion about anyone else's memory.
    #[test]
    fn an_unaccounted_store_has_no_pressure_and_charges_nobody() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(60)));
        assert_eq!(lru.used_bytes(), 60);
        assert_eq!(lru.pressure(), None);
    }

    /// The entries are already in memory by the time the store asks, so a
    /// refusal is **recorded, not obeyed**: the store keeps what it was given,
    /// the ledger counts the denial, and the pressure that reads from goes to
    /// the top — which is what makes whoever is watching act.
    #[test]
    fn a_refused_reservation_is_recorded_and_not_obeyed() {
        let ledger = lumit_budget::Ledger::with_budgets(10_000, 50);
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(1000);
        lru.account_against(std::sync::Arc::clone(&ledger), lumit_budget::Tier::Ram);

        assert!(lru.insert("a", v(40)));
        assert_eq!(ledger.used(lumit_budget::Tier::Ram), 40);

        assert!(lru.insert("b", v(40)), "the store's own budget has room");
        assert_eq!(lru.used_bytes(), 80, "and it kept both");
        assert_eq!(
            ledger.used(lumit_budget::Tier::Ram),
            40,
            "the second reservation was refused, not the entry"
        );
        assert_eq!(ledger.denials(lumit_budget::Tier::Ram), 1);
        // The denial is the honest signal, not the used figure: a refusal
        // leaves the ledger *under*-reporting by exactly the entry it could
        // not grant, and only the counter says so. A reader that watched the
        // bytes alone would see a tier with room to spare.
        assert!(lru
            .pressure()
            .is_some_and(|p| p > lumit_budget::Pressure::Easy));
    }

    #[test]
    fn eviction_cascades_until_it_fits() {
        let mut lru: ByteLru<u32, Vec<u8>> = ByteLru::new(100);
        for i in 0..10u32 {
            assert!(lru.insert(i, v(10)));
        }
        assert!(lru.insert(99, v(95)));
        assert!(lru.used_bytes() <= 100);
        assert!(lru.get(&99).is_some());
    }

    #[test]
    fn lowering_the_budget_evicts_until_it_fits() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(40)));
        assert!(lru.insert("b", v(40)));
        lru.get(&"b"); // make "a" the oldest
        lru.set_budget(50);
        assert!(lru.used_bytes() <= 50);
        assert!(lru.contains_key(&"b") && !lru.contains_key(&"a"));
        // Raising it again keeps what is there and admits more.
        lru.set_budget(100);
        assert!(lru.insert("c", v(40)));
        assert!(lru.contains_key(&"b") && lru.contains_key(&"c"));
    }

    #[test]
    fn peek_reads_without_rescuing_from_eviction() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(40)));
        assert!(lru.insert("b", v(40)));
        // Peeking "a" many times must not bump its recency: "a" was inserted
        // first, so it stays the least-recently-used and is the one evicted.
        for _ in 0..5 {
            assert!(lru.peek(&"a").is_some());
        }
        assert!(lru.insert("c", v(40)));
        assert!(
            lru.contains_key(&"b") && !lru.contains_key(&"a"),
            "peek did not distort eviction: the oldest entry still went"
        );
    }

    /// docs §5.3 "cheap-to-recompute": a dear entry resists eviction even when
    /// it is the *older* one. "dear" is inserted first (so it is staler), yet
    /// its high recompute cost keeps it while the cheap, newer entry goes.
    #[test]
    fn cost_aware_eviction_keeps_the_dear_frame() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert_with_cost("dear", v(40), 100));
        assert!(lru.insert_with_cost("cheap", v(40), 1));
        assert!(lru.insert("c", v(40))); // forces one eviction
        assert!(
            lru.contains_key(&"dear") && !lru.contains_key(&"cheap"),
            "the cheap-to-recompute frame is evicted before the dear one"
        );
    }

    /// docs §5.3 "large": at equal cost, the bigger frame is reclaimed first —
    /// it frees the most room — even though the smaller one is staler here.
    #[test]
    fn size_aware_eviction_reclaims_the_big_frame() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("small", v(20)));
        assert!(lru.insert("big", v(60)));
        assert!(lru.insert("c", v(40)));
        assert!(
            lru.contains_key(&"small") && !lru.contains_key(&"big"),
            "the large frame is reclaimed first"
        );
        assert!(lru.contains_key(&"c"));
    }

    /// docs §5.3's demotion ladder needs eviction to be *visible*: the tier
    /// below cannot take a frame it never hears about. Opt-in, so a store whose
    /// owner does not drain keeps behaving exactly as before.
    #[test]
    fn evictions_can_be_collected_for_the_tier_below() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        lru.collect_evictions();
        // Equal cost, so plain staleness decides: "a" is the stalest and goes.
        assert!(lru.insert_with_cost("a", v(40), 7));
        assert!(lru.insert_with_cost("b", v(40), 7));
        lru.get(&"b");
        assert!(lru.insert_with_cost("c", v(40), 7));

        let evicted = lru.take_evicted();
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].0, "a");
        assert_eq!(evicted[0].1.len(), 40, "the value comes with it");
        assert_eq!(
            evicted[0].2, 7,
            "and its recompute cost, which is what decides whether the tier \
             below is worth the trouble"
        );
        assert!(lru.take_evicted().is_empty(), "drained once, not twice");

        // Lowering the budget is an eviction too — the frames squeezed out
        // should fall downstairs rather than vanish.
        lru.set_budget(40);
        assert_eq!(lru.take_evicted().len(), 1);

        // Clearing on purpose is NOT: the user asked for the tier to be empty.
        assert!(lru.insert("d", v(40)));
        lru.take_evicted(); // "d" displaced the survivor; that is an eviction
        lru.clear();
        assert!(
            lru.take_evicted().is_empty(),
            "an explicit clear means gone, not demoted"
        );

        // Replacing a key is a replacement, not an eviction.
        assert!(lru.insert("e", v(20)));
        assert!(lru.insert("e", v(30)));
        assert!(lru.take_evicted().is_empty());
    }

    /// Without the opt-in nothing is kept, so the default store cannot grow a
    /// second copy of everything it evicted.
    #[test]
    fn evictions_are_dropped_unless_asked_for() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(40);
        assert!(lru.insert("a", v(40)));
        assert!(lru.insert("b", v(40)));
        assert!(lru.take_evicted().is_empty());
    }

    /// docs §5.3 pinning: a pinned key is never the victim, so the eviction
    /// falls on a non-pinned entry instead — even though the pinned one would
    /// otherwise be chosen (it is the stalest here).
    #[test]
    fn pinned_entries_survive_eviction() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(40)));
        assert!(lru.insert("b", v(40)));
        lru.pin("a"); // "a" is the stalest, the natural victim
        assert!(lru.insert("c", v(40)));
        assert!(
            lru.contains_key(&"a") && !lru.contains_key(&"b"),
            "the pin protects the stalest frame; a non-pinned one goes instead"
        );
        // Lifting the pin lets it be evicted normally again.
        lru.unpin(&"a");
        assert!(lru.insert("d", v(40)));
        assert!(!lru.contains_key(&"a"), "unpinned, the stale frame can go");
    }

    /// docs §5.3: when only pinned entries remain, the store is left briefly
    /// over budget rather than dropping a pin (the pin set is small and clears
    /// as the playhead moves).
    #[test]
    fn only_pins_left_accepts_bounded_overage() {
        let mut lru: ByteLru<&str, Vec<u8>> = ByteLru::new(100);
        assert!(lru.insert("a", v(40)));
        assert!(lru.insert("b", v(40)));
        lru.pin("a");
        lru.pin("b");
        assert!(lru.insert("c", v(40))); // nothing evictable → overage
        assert!(lru.contains_key(&"a") && lru.contains_key(&"b") && lru.contains_key(&"c"));
        assert_eq!(
            lru.used_bytes(),
            120,
            "pins protected, budget briefly exceeded"
        );
        // Once a pin lifts, the next insert reclaims the overage.
        lru.unpin(&"a");
        assert!(lru.insert("d", v(40)));
        assert!(
            lru.used_bytes() <= 100,
            "overage clears once a pin is lifted"
        );
        assert!(!lru.contains_key(&"a"), "the unpinned frame was reclaimed");
    }
}

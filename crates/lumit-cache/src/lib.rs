//! The cache crate — **Nebula** (K-083): byte-budgeted stores per
//! docs/06-RENDER-PIPELINE.md §5 (K-016). The RAM tier ([`ByteLru`]) is a
//! byte-budget store with cost-aware (GreedyDual-style) eviction and pinning
//! (§5.3); the disk tier ([`disk`]) parks frames in the project's sidecar
//! folder. The VRAM tier, index.db and the governor join as the evaluator grows.
//!
//! In plain terms: a cupboard with a strict size limit. When it's full and you
//! add something, it throws out the item that is the best bargain to lose —
//! one that hasn't been touched in a while, is big (frees the most room), and
//! is cheap to remake — while never touching anything you've *pinned* (the
//! frame on screen and its neighbours). "Budget by bytes, not by count" is the
//! point: one 4K frame costs what sixty thumbnails cost.

pub mod disk;

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
        }
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

    /// Evict non-pinned entries by eviction score until within budget, or until
    /// only pinned entries remain (then stop — a pin is never dropped).
    fn evict_to_fit(&mut self) {
        while self.used > self.budget {
            let Some(victim) = self.victim() else {
                break; // only pins left; accept the bounded overage (§5.3)
            };
            if let Some(evicted) = self.map.remove(&victim) {
                self.used -= evicted.bytes;
            }
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
            if let Some(evicted) = self.map.remove(&victim) {
                self.used -= evicted.bytes;
            }
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

    pub fn used_bytes(&self) -> usize {
        self.used
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

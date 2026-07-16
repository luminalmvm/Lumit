//! The cache crate: byte-budgeted stores per docs/06-RENDER-PIPELINE.md §5
//! (K-016). Phase 1 seed: the RAM tier as a byte-budget LRU. VRAM and disk
//! tiers, content-hash keys and the governor join as the evaluator grows.
//!
//! In plain terms: a cupboard with a strict size limit — putting something in
//! when it's full throws out whatever was used longest ago. "Budget by bytes,
//! not by count" is the point: one 4K frame costs what sixty thumbnails cost.

use std::collections::HashMap;
use std::hash::Hash;

pub trait ByteSized {
    fn byte_size(&self) -> usize;
}

impl ByteSized for Vec<u8> {
    fn byte_size(&self) -> usize {
        self.len()
    }
}

/// Least-recently-used store with a byte budget.
/// Eviction scans for the oldest entry — O(n) on insert-over-budget, fine at
/// the hundreds-of-frames scale of the preview cache; the evaluator's tier
/// replaces the scan with an intrusive list when n grows (documented debt).
pub struct ByteLru<K, V> {
    map: HashMap<K, Entry<V>>,
    budget: usize,
    used: usize,
    tick: u64,
}

struct Entry<V> {
    value: V,
    bytes: usize,
    last_used: u64,
}

impl<K: Eq + Hash + Clone, V: ByteSized> ByteLru<K, V> {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            budget: budget_bytes,
            used: 0,
            tick: 0,
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

    /// Insert, evicting least-recently-used entries until within budget.
    /// A value larger than the whole budget is not cached (returns false).
    pub fn insert(&mut self, key: K, value: V) -> bool {
        let bytes = value.byte_size();
        if bytes > self.budget {
            return false;
        }
        self.tick += 1;
        if let Some(old) = self.map.remove(&key) {
            self.used -= old.bytes;
        }
        while self.used + bytes > self.budget {
            let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(evicted) = self.map.remove(&oldest) {
                self.used -= evicted.bytes;
            }
        }
        self.map.insert(
            key,
            Entry {
                value,
                bytes,
                last_used: self.tick,
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
}

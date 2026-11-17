//! The decode pool's measured motion: the pairs every flow reader shares, and
//! beside them what Denoise keeps so that a frame met again costs nothing.
//!
//! Denoise reads up to five frames either side and chains the motion between
//! neighbours to reach the far ones (`lumit_flow::chain`). It keeps each pair
//! with the confidence of both directions already worked out, since the pair a
//! frame measures going forward is the one the next frame reads going back,
//! and each frame's luma, since the next pair starts from it. Playing forward,
//! that leaves one decode, one luma and one measurement a frame.

use std::sync::Arc;

use lumit_flow::chain::Leg;

use super::{CachedFlow, FlowKey};

/// How a frame was read, beside its number: the channels a layer extracts
/// and the stamp of a frame in a run of stills, the two things besides the
/// number that the decoded-frame cache names a frame by.
pub(super) type Read = (
    Option<[Option<String>; 4]>,
    Option<(u64, std::time::SystemTime)>,
);

/// A pair of frames of one source, earlier first, each as it was read,
/// measured at one setting and one decoded size.
pub(super) type LinkKey = (
    uuid::Uuid,
    (usize, Read),
    (usize, Read),
    u64,
    (usize, usize),
);

/// One frame's luma, by source, frame as it was read, setting, decoded size
/// and whether it was read as floats.
pub(super) type GrayKey = (uuid::Uuid, (usize, Read), u64, (usize, usize), bool);

/// The motion both ways between two frames, each with its confidence.
pub(super) struct Link {
    /// Earlier frame to later.
    pub(super) on: Arc<Leg>,
    /// Later frame to earlier.
    pub(super) back: Arc<Leg>,
}

/// A pair as the cache holds it, shared with whoever is reading it.
pub(super) struct HeldLink(pub(super) Arc<Link>);

impl lumit_cache::ByteSized for HeldLink {
    fn byte_size(&self) -> usize {
        self.0.on.bytes() + self.0.back.bytes()
    }
}

/// A frame's luma, held for the next pair that starts from it.
pub(super) struct HeldGray(pub(super) Arc<lumit_flow::Gray>);

impl lumit_cache::ByteSized for HeldGray {
    fn byte_size(&self) -> usize {
        self.0.data.len() * 4 + 32
    }
}

/// How many decoded frames' luma are kept: the frame a pair ends on, which the
/// next pair starts from, with room for the two directions of a scrub.
const GRAYS_HELD: usize = 4;

/// Everything the pool has measured.
pub(super) struct FlowCache {
    /// The pairs as measured, which Motion blur, Datamosh and the retime
    /// policy share.
    pairs: lumit_cache::ByteLru<FlowKey, CachedFlow>,
    /// Denoise's pairs, with their confidences. Its budget follows how many
    /// pairs the widest Denoise playing needs live, so five frames either side
    /// of 4K footage is not measured again every frame ([`Self::want_links`]).
    pub(super) links: lumit_cache::ByteLru<LinkKey, HeldLink>,
    pub(super) grays: lumit_cache::ByteLru<GrayKey, HeldGray>,
    /// The budget `links` was built with, the least it is ever given.
    floor: usize,
    /// The governor, once the pool is accounted against it: `links` never
    /// grows past a quarter of its memory.
    ledger: Option<Arc<lumit_budget::Ledger>>,
}

impl FlowCache {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            pairs: lumit_cache::ByteLru::new(budget),
            links: lumit_cache::ByteLru::new(budget),
            grays: lumit_cache::ByteLru::new(budget / 8),
            floor: budget,
            ledger: None,
        }
    }

    pub(super) fn get(&mut self, key: &FlowKey) -> Option<&CachedFlow> {
        self.pairs.get(key)
    }

    pub(super) fn insert(&mut self, key: FlowKey, value: CachedFlow) -> bool {
        self.pairs.insert(key, value)
    }

    /// Room for `pairs` of Denoise's pairs of `bytes` each, and a frame's luma
    /// of `gray` bytes for [`GRAYS_HELD`] frames. Grows to what is playing, up
    /// to a quarter of the governor's memory, and never shrinks below where it
    /// started; the governor's trim still takes the cold half of it under
    /// pressure, as it does every store. Past the cap, the far pairs are
    /// measured again, which is slower and never wrong.
    pub(super) fn want_links(&mut self, pairs: usize, bytes: usize, gray: usize) {
        let cap = self.ledger.as_ref().map_or(usize::MAX, |l| {
            usize::try_from(l.budget(lumit_budget::Tier::Ram) / 4).unwrap_or(usize::MAX)
        });
        let links = pairs.saturating_mul(bytes).min(cap).max(self.floor);
        if links > self.links.budget_bytes() {
            self.links.set_budget(links);
        }
        let grays = (GRAYS_HELD * gray).max(self.floor / 8);
        if grays > self.grays.budget_bytes() {
            self.grays.set_budget(grays);
        }
    }

    pub(super) fn account_against(
        &mut self,
        ledger: &Arc<lumit_budget::Ledger>,
        tier: lumit_budget::Tier,
    ) {
        self.pairs.account_against(Arc::clone(ledger), tier);
        self.links.account_against(Arc::clone(ledger), tier);
        self.grays.account_against(Arc::clone(ledger), tier);
        self.ledger = Some(Arc::clone(ledger));
    }

    pub(super) fn evict_cold_half(&mut self) -> bool {
        // Every store, not the first that gives something back.
        let pairs = self.pairs.evict_cold_half();
        let links = self.links.evict_cold_half();
        self.grays.evict_cold_half() | pairs | links
    }

    pub(super) fn clear(&mut self) {
        self.pairs.clear();
        self.links.clear();
        self.grays.clear();
    }
}

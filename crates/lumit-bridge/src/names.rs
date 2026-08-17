//! A memo of frame names, so a name is computed once per edit rather than once
//! per ask.
//!
//! # In plain terms
//!
//! Every cached frame is filed under a content hash — a walk of the whole
//! composition at that frame's time (docs/06 §5.2). The walk is cheap beside a
//! composite, but it is not free, and several consumers ask for the same names
//! over and over: the cache bar names hundreds of frames each time it redraws,
//! playback names each coming frame to look it up in the tiers, the idle fill
//! names its whole window. During playback of a cached span those repeated
//! walks were most of what the worker thread spent its deadline on.
//!
//! The names are deterministic: for one committed document, one composition,
//! one frame and one preview quality, the walk always produces the same hash.
//! So the answer is remembered here, keyed by `(comp, frame, quality tag)`, and
//! the whole memo is dropped the moment the document's revision moves — an edit
//! renames an unknown set of frames, and recomputing is exactly what the memo
//! must not guess about. A frame that cannot be named yet (footage still being
//! probed) is never remembered, so a probe finishing later is picked up on the
//! next ask.

use std::collections::HashMap;
use uuid::Uuid;

/// The most names held before the memo is emptied and started afresh. A name
/// is 16 bytes plus its key; the cap keeps a long session with many zoom
/// levels (each its own quality tag) bounded at a few megabytes. Emptying is
/// crude but correct — everything is recomputable — and at this size it is
/// effectively never hit inside one revision.
const MAX_NAMES: usize = 65_536;

/// The memo. One per worker thread, owned by its state; never shared.
#[derive(Default)]
pub(crate) struct NameCache {
    /// The document revision the held names were computed against.
    revision: u64,
    map: HashMap<(Uuid, u64, u32), u128>,
}

impl NameCache {
    /// The name of `frame` of `comp` at the quality named by `tag`, remembered
    /// from an earlier ask or computed now by `compute`. `revision` is the
    /// document revision the caller read alongside its snapshot: a different
    /// revision empties the memo first, so a stale name can never be served.
    ///
    /// Forget everything, revision aside.
    ///
    /// For when the names themselves change meaning without the document
    /// moving: the Viewer's way of looking is folded into every name
    /// (`named_under_view` — exposure, tone map, the transparency grid, the
    /// region), so a look change renames every frame at the same revision. A
    /// memo keyed only by `(comp, frame, quality)` would go on serving the old
    /// look's names — which is how the cache bar read all-zero and the idle
    /// fill re-rendered for ever whenever the grid was up (its default).
    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }

    /// `compute` returning `None` (frame not nameable yet) is passed through
    /// and NOT remembered — the next ask tries again, which is what lets a
    /// finishing probe make a frame nameable mid-session.
    pub(crate) fn get_or_compute(
        &mut self,
        revision: u64,
        comp: Uuid,
        frame: u64,
        tag: u32,
        compute: impl FnOnce() -> Option<u128>,
    ) -> Option<u128> {
        if revision != self.revision {
            self.map.clear();
            self.revision = revision;
        }
        if let Some(&name) = self.map.get(&(comp, frame, tag)) {
            return Some(name);
        }
        let name = compute()?;
        if self.map.len() >= MAX_NAMES {
            self.map.clear();
        }
        self.map.insert((comp, frame, tag), name);
        Some(name)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The memo's whole point: the second identical ask answers without
    /// computing. The walk it saves is a hash of the entire composition, paid
    /// hundreds of times per redraw of the cache bar before this existed.
    #[test]
    fn a_name_is_computed_once_per_revision() {
        let mut names = NameCache::default();
        let comp = Uuid::now_v7();
        let computed = std::cell::Cell::new(0u32);
        let ask = |names: &mut NameCache, revision| {
            names.get_or_compute(revision, comp, 7, 1000, || {
                computed.set(computed.get() + 1);
                Some(42)
            })
        };
        assert_eq!(ask(&mut names, 1), Some(42));
        assert_eq!(ask(&mut names, 1), Some(42));
        assert_eq!(computed.get(), 1, "the second ask is remembered");

        // An edit moves the revision: everything is recomputed, because an edit
        // renames an unknown set of frames and a memo must never guess.
        assert_eq!(ask(&mut names, 2), Some(42));
        assert_eq!(computed.get(), 2, "a new revision recomputes");
    }

    /// Different frames, comps and quality tags are different names — and an
    /// unnameable frame is asked again rather than remembered as nothing, so a
    /// probe finishing mid-session is picked up.
    #[test]
    fn keys_are_distinct_and_none_is_never_remembered() {
        let mut names = NameCache::default();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(names.get_or_compute(1, a, 0, 1000, || Some(1)), Some(1));
        assert_eq!(names.get_or_compute(1, a, 1, 1000, || Some(2)), Some(2));
        assert_eq!(names.get_or_compute(1, a, 0, 1050, || Some(3)), Some(3));
        assert_eq!(names.get_or_compute(1, b, 0, 1000, || Some(4)), Some(4));
        assert_eq!(names.get_or_compute(1, a, 0, 1000, || Some(9)), Some(1));

        // Not nameable yet: passed through, tried again next ask.
        let mut tries = 0;
        for _ in 0..2 {
            let got = names.get_or_compute(1, b, 9, 1000, || {
                tries += 1;
                None
            });
            assert_eq!(got, None);
        }
        assert_eq!(tries, 2, "an unnameable frame is never memoised");
        // And once the probe lands, the name is served and then remembered.
        assert_eq!(names.get_or_compute(1, b, 9, 1000, || Some(5)), Some(5));
        assert_eq!(names.get_or_compute(1, b, 9, 1000, || None), Some(5));
    }

    /// A look change renames every frame at the same revision (the look is
    /// folded into the names), which the revision check cannot see — so the
    /// worker clears the memo when the look changes, and the next ask
    /// recomputes under the new look rather than serving the old one's name.
    #[test]
    fn a_cleared_memo_recomputes_at_the_same_revision() {
        let mut names = NameCache::default();
        let comp = Uuid::now_v7();
        assert_eq!(names.get_or_compute(1, comp, 0, 1000, || Some(1)), Some(1));
        names.clear();
        assert_eq!(
            names.get_or_compute(1, comp, 0, 1000, || Some(2)),
            Some(2),
            "after a clear, the same key is computed afresh"
        );
    }

    /// The cap empties rather than growing without bound — crude, correct, and
    /// effectively never hit inside one revision.
    #[test]
    fn the_cap_bounds_the_memo() {
        let mut names = NameCache::default();
        let comp = Uuid::now_v7();
        for frame in 0..(MAX_NAMES as u64 + 10) {
            names.get_or_compute(1, comp, frame, 1000, || Some(u128::from(frame)));
        }
        assert!(names.map.len() <= MAX_NAMES);
        // Still answers correctly after the clear-out.
        assert_eq!(
            names.get_or_compute(1, comp, 3, 1000, || Some(77)),
            Some(77),
            "a cleared name is simply recomputed"
        );
    }
}

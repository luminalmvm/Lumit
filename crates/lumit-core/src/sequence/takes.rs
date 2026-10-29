//! A clip's takes, as Vegas has them: one clip can hold several pieces of
//! media and show any one of them in the same place.
//!
//! The take a clip shows is its own `source` and `source_in`. The others wait
//! in [`Clip::takes`], in turn after it, so the clip and its takes are one
//! ring: number nought is the take shown, and number `n` is `takes[n - 1]`.
//! Showing take `n` turns the ring so it comes first and keeps the order.

use super::{Clip, ClipSource};
use crate::time::Rational;
use serde::{Deserialize, Serialize};

/// One more take a clip holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    pub source: ClipSource,
    /// Where its trim sits against the take shown: it starts this much later
    /// in its own media. Kept relative, so a trim, a slip or a cut moves every
    /// take with the one shown and they stay in step.
    pub offset: Rational,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Clip {
    /// Every source the clip holds: the take shown, then the rest. What a
    /// project counts as in use, and what a removed item is taken out of.
    pub fn sources(&self) -> impl Iterator<Item = ClipSource> + '_ {
        std::iter::once(self.source).chain(self.takes.iter().map(|t| t.source))
    }

    /// This clip showing its take number `n`, by the numbering above. Its place,
    /// length, Retime, effects and fades stay, and its trim moves to the take.
    /// None for take nought or one it does not hold, and where the trim would
    /// start before the top of the take's media.
    #[must_use]
    pub fn showing_take(&self, n: usize) -> Option<Clip> {
        let chosen = self.takes.get(n.checked_sub(1)?)?;
        let by = chosen.offset;
        let mut clip = self.slip(by)?;
        clip.source = chosen.source;
        let shown = Take {
            source: self.source,
            offset: Rational::ZERO,
            extra: serde_json::Map::new(),
        };
        let after = self.takes.iter().skip(n);
        let before = self.takes.iter().take(n - 1);
        let mut takes = Vec::with_capacity(self.takes.len());
        for take in after.chain([&shown]).chain(before) {
            takes.push(Take {
                offset: take.offset.checked_sub(by).ok()?,
                ..take.clone()
            });
        }
        clip.takes = takes;
        Some(clip)
    }

    /// This clip holding `source` as one more take, starting at `source_in`,
    /// and showing it. It goes in just after the take shown now, so takes
    /// added one after another go round in the order they came.
    #[must_use]
    pub fn with_take(&self, source: ClipSource, source_in: Rational) -> Option<Clip> {
        let mut clip = self.clone();
        let take = Take {
            source,
            offset: source_in.checked_sub(self.source_in).ok()?,
            extra: serde_json::Map::new(),
        };
        clip.takes.insert(0, take);
        clip.showing_take(1)
    }

    /// This clip without its take number `n`. Taking away the take shown
    /// shows the one after it. None when the clip has no other take, or no
    /// take `n`.
    #[must_use]
    pub fn without_take(&self, n: usize) -> Option<Clip> {
        let mut clip = self.clone();
        if n == 0 {
            // The take shown goes to the back of the ring, and off it.
            clip = self.showing_take(1)?;
            clip.takes.pop();
        } else if n <= clip.takes.len() {
            clip.takes.remove(n - 1);
        } else {
            return None;
        }
        Some(clip)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// Takes go round in one order and each keeps its own trim, and a clip
    /// showing another take is where it was, as long as it was.
    #[test]
    fn takes_turn_in_order_and_keep_their_trims() {
        let s = |n: i64| Rational::new(n, 1).unwrap();
        let (a, b, c) = (
            ClipSource::Footage(Uuid::now_v7()),
            ClipSource::Footage(Uuid::now_v7()),
            ClipSource::Footage(Uuid::now_v7()),
        );
        let clip = Clip::new(a, s(1), s(3), s(10), s(2));
        let clip = clip.with_take(b, s(5)).unwrap();
        let clip = clip.with_take(c, s(0)).unwrap();
        let shows = |clip: &Clip| (clip.source, clip.source_in, clip.source_out);
        assert_eq!(shows(&clip), (c, s(0), s(2)));
        assert_eq!((clip.place_start, clip.place_duration), (s(10), s(2)));
        // On from the last added is round to the first.
        let next = clip.showing_take(1).unwrap();
        assert_eq!(shows(&next), (a, s(1), s(3)));
        let next = next.showing_take(1).unwrap();
        assert_eq!(shows(&next), (b, s(5), s(7)));
        // Back one from the first is the last.
        let back = clip.showing_take(clip.takes.len()).unwrap();
        assert_eq!(shows(&back), (b, s(5), s(7)));
        // Taking away the take shown shows the next, and the other stays.
        let left = clip.without_take(0).unwrap();
        assert_eq!(shows(&left), (a, s(1), s(3)));
        assert_eq!(left.takes.len(), 1);
        assert_eq!(shows(&left.showing_take(1).unwrap()), (b, s(5), s(7)));
    }
}

//! Sequence layers: clips cut back-to-back on one row (docs/03-DATA-MODEL.md
//! §5.3, docs/04-RETIMING.md §1.3). This is Luminal's Vegas-style editing
//! surface.
//!
//! In plain terms: a Sequence layer is one timeline row holding a run of
//! **clips** laid end to end. Each clip points at a source (a footage item or
//! a comp), carries its own trim and its own [`Retime`] ramp, and sits at an
//! exact place on the row. Clips never overlap; a gap between them shows
//! through as transparent. To draw the layer at a given moment you ask "which
//! clip is under the playhead, and which moment of its source does that map
//! to?" — that resolution is all this module does. Turning that source moment
//! into pixels, and the layer's own masks/effects/transform, happen above.
//!
//! Scope note: this is the resolution model and its invariants only. Wiring
//! it into `LayerKind` and the render paths is the next step and lives
//! elsewhere; cutting (§8) and the graph lenses (§9) build on top.

use crate::retime::{Ease, Interpolation, Retime};
use crate::time::Rational;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a clip plays: one footage item or one nested composition
/// (docs/03-DATA-MODEL.md §5.3 ClipSource).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipSource {
    Footage(Uuid),
    Comp(Uuid),
}

/// One clip on a Sequence layer (docs/03-DATA-MODEL.md §5.3). Times are exact
/// rationals in seconds; `place_*` are on the layer's timeline, `source_*`
/// index into the clip's source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub source: ClipSource,
    /// Trim into the source (seconds).
    pub source_in: Rational,
    /// Exclusive trim end (seconds).
    pub source_out: Rational,
    /// Where the clip starts on the layer's timeline (seconds).
    pub place_start: Rational,
    /// How long the clip occupies on the layer's timeline (seconds).
    pub place_duration: Rational,
    /// The clip's retime map: clip-local time → source time. Its first
    /// boundary's source position is the clip's effective source in.
    pub retime: Retime,
    /// How fractional source moments become pixels (render policy).
    #[serde(default)]
    pub interpolation: Interpolation,
    /// Unknown fields from newer Luminal versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Clip {
    /// A plain (un-retimed) clip of `source` placed at `place_start` for
    /// `place_duration`, playing its source from `source_in` at natural rate.
    pub fn new(
        source: ClipSource,
        source_in: Rational,
        source_out: Rational,
        place_start: Rational,
        place_duration: Rational,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            source,
            source_in,
            source_out,
            place_start,
            place_duration,
            retime: Retime::identity(place_duration, source_in),
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// Where the clip ends on the layer timeline (exclusive).
    pub fn place_end(&self) -> Rational {
        self.place_start
            .checked_add(self.place_duration)
            .unwrap_or(self.place_start)
    }

    /// This clip at a new constant `speed` (1.0 = source rate), its place on the
    /// layer unchanged (the beat-sync covenant — edit points never move). The
    /// source range it covers follows from the speed, so `source_out` is
    /// re-derived from the retime; the clip id is preserved.
    pub fn with_speed(&self, speed: Rational) -> Clip {
        let retime = Retime::constant_speed(self.place_duration, self.source_in, speed);
        let source_out = retime.boundaries.last().map_or(self.source_out, |b| b.s);
        Clip {
            retime,
            source_out,
            ..self.clone()
        }
    }

    /// The clip's single constant speed (1.0 = source rate), or None if it holds
    /// a ramp or a more complex retime that the timeline can't show as one value.
    pub fn constant_speed(&self) -> Option<f64> {
        self.retime
            .single_ramp_view()
            .filter(|(v0, v1, _)| (v0 - v1).abs() < 1e-9)
            .map(|(v0, _, _)| v0)
    }

    /// This clip with a single speed *ramp* — speed eases from `v0` to `v1`
    /// across the clip — its place on the layer unchanged (beat-sync). The
    /// montage velocity gesture; `source_out` follows from the ramp integral.
    pub fn with_ramp(&self, v0: Rational, v1: Rational, ease: Ease) -> Clip {
        let retime = Retime::single_ramp(self.place_duration, self.source_in, v0, v1, ease);
        let source_out = retime.boundaries.last().map_or(self.source_out, |b| b.s);
        Clip {
            retime,
            source_out,
            ..self.clone()
        }
    }

    /// The clip's ramp as `(start speed, end speed, ease)` when it is a single
    /// Rate segment (constant or ramp); None for multi-segment / Map stores.
    pub fn ramp_view(&self) -> Option<(f64, f64, Ease)> {
        self.retime.single_ramp_view()
    }

    /// True when layer-local time `lt` (seconds) falls within this clip.
    pub fn contains(&self, lt: f64) -> bool {
        lt >= self.place_start.to_f64() && lt < self.place_end().to_f64()
    }

    /// The source time (seconds) shown at layer-local time `lt`, via the
    /// clip's retime (which maps clip-local time → source time). Only
    /// meaningful when [`Self::contains`] is true.
    pub fn source_time(&self, lt: f64) -> f64 {
        let clip_time = lt - self.place_start.to_f64();
        self.retime.evaluate(clip_time)
    }

    /// Cut this clip at layer-local time `at` into two clips whose retimes
    /// exactly partition the original (docs/03-DATA-MODEL.md §5.3, the
    /// beat-sync covenant: `place` never moves, source positions stay exact).
    /// None when `at` is not strictly inside the clip, or the retime can't be
    /// split exactly there ([`Retime::split_at`]).
    pub fn cut(&self, at: Rational) -> Option<(Clip, Clip)> {
        let tau_clip = at.checked_sub(self.place_start).ok()?;
        if tau_clip <= Rational::ZERO || tau_clip >= self.place_duration {
            return None;
        }
        let (left_retime, right_retime) = self.retime.split_at(tau_clip)?;
        // The shared cut source position — exact, C0 (both retimes agree).
        let s_cut = left_retime.boundaries.last()?.s;
        let right_duration = self.place_duration.checked_sub(tau_clip).ok()?;
        let left = Clip {
            id: Uuid::now_v7(),
            source: self.source,
            source_in: self.source_in,
            source_out: s_cut,
            place_start: self.place_start,
            place_duration: tau_clip,
            retime: left_retime,
            interpolation: self.interpolation.clone(),
            extra: self.extra.clone(),
        };
        let right = Clip {
            id: Uuid::now_v7(),
            source: self.source,
            source_in: s_cut,
            source_out: self.source_out,
            place_start: at,
            place_duration: right_duration,
            retime: right_retime,
            interpolation: self.interpolation.clone(),
            extra: self.extra.clone(),
        };
        Some((left, right))
    }

    /// Slide the clip along the Sequence layer by `delta` (docs/04-RETIMING.md
    /// §8.2): its position moves, but the source window, local time and retime
    /// are untouched — the same frames play, just earlier or later on the row.
    /// None if the clip would start before the layer origin, or on overflow.
    pub fn slide(&self, delta: Rational) -> Option<Clip> {
        let place_start = self.place_start.checked_add(delta).ok()?;
        if place_start.is_negative() {
            return None;
        }
        Some(Clip {
            place_start,
            ..self.clone()
        })
    }

    /// Slip the source under the fixed clip by `delta` (docs/04-RETIMING.md
    /// §8.2): the clip keeps its place and duration, but a different stretch of
    /// the source plays. The trim window and every retime source position shift
    /// by `delta` together, so the retime's shape is untouched; overrun is
    /// re-evaluated at render time. None if the slip would read before the
    /// source start, or on overflow.
    pub fn slip(&self, delta: Rational) -> Option<Clip> {
        let source_in = self.source_in.checked_add(delta).ok()?;
        if source_in.is_negative() {
            return None;
        }
        let source_out = self.source_out.checked_add(delta).ok()?;
        let retime = self.retime.shift_source(delta)?;
        Some(Clip {
            source_in,
            source_out,
            retime,
            ..self.clone()
        })
    }

    /// Trim the clip's tail inward to end at layer time `new_end`
    /// (docs/04-RETIMING.md §8.2, non-ripple): the retime is split at the new
    /// edge and the outside discarded, so the kept portion plays exactly as
    /// before. The clip keeps its identity and its start. None if `new_end` is
    /// not strictly inside the clip (trimming *outward* extends per §7.3, which
    /// needs the source's available length and is a separate op).
    pub fn trim_end(&self, new_end: Rational) -> Option<Clip> {
        let tau = new_end.checked_sub(self.place_start).ok()?;
        if tau <= Rational::ZERO || tau >= self.place_duration {
            return None;
        }
        let (left, _) = self.retime.split_at(tau)?;
        let source_out = left.boundaries.last()?.s;
        Some(Clip {
            source_out,
            place_duration: tau,
            retime: left,
            ..self.clone()
        })
    }

    /// Trim the clip's head inward to start at layer time `new_start`
    /// (docs/04-RETIMING.md §8.2, non-ripple): the retime is split at the new
    /// edge, the outside discarded, and the kept portion's local time re-based
    /// to zero — so it still plays exactly as before, just entered later. The
    /// clip keeps its identity. None if `new_start` is not strictly inside the
    /// clip (outward trims extend per §7.3, a separate op).
    pub fn trim_start(&self, new_start: Rational) -> Option<Clip> {
        let tau = new_start.checked_sub(self.place_start).ok()?;
        if tau <= Rational::ZERO || tau >= self.place_duration {
            return None;
        }
        let (_, right) = self.retime.split_at(tau)?;
        let source_in = right.boundaries.first()?.s;
        let place_duration = self.place_duration.checked_sub(tau).ok()?;
        Some(Clip {
            source_in,
            place_start: new_start,
            place_duration,
            retime: right,
            ..self.clone()
        })
    }

    /// Trim the out point to the last moment still inside the source extent
    /// (docs/04-RETIMING.md §7.4, non-ripple): when the retime runs the clip
    /// past its trimmed source end (tail overrun), crop the clip to the crossing
    /// point. The clip's start never moves and a gap is left after it (gaps are
    /// never auto-closed — the beat-sync covenant K-022). None when there is no
    /// tail overrun, so the command can report "nothing to trim".
    pub fn trim_to_source_end(&self) -> Option<Clip> {
        // Local time where the mapped source position first reaches source_out.
        let crossing = self.retime.overrun_local_time(self.source_out)?;
        let crossing = Rational::from_f64_on_grid(crossing, Rational::FLICK_DEN).ok()?;
        let new_end = self.place_start.checked_add(crossing).ok()?;
        self.trim_end(new_end)
    }
}

/// The clip active at layer-local time `lt`, or None if `lt` is in a gap
/// (transparent) or past the end. Clips must not overlap, so at most one
/// matches; the first match wins defensively.
pub fn active_clip(clips: &[Clip], lt: f64) -> Option<&Clip> {
    clips.iter().find(|c| c.contains(lt))
}

/// Resolve layer-local time `lt` to `(active clip id, source, source time)`,
/// or None in a gap. The one query the renderer needs.
pub fn resolve(clips: &[Clip], lt: f64) -> Option<(Uuid, ClipSource, f64)> {
    active_clip(clips, lt).map(|c| (c.id, c.source, c.source_time(lt)))
}

/// The single source shared by all clips, if they share one — a sequenced
/// layer is single-source (K-071). None when empty or mixed.
pub fn single_source(clips: &[Clip]) -> Option<ClipSource> {
    let first = clips.first()?.source;
    clips.iter().all(|c| c.source == first).then_some(first)
}

/// True when clips never jump backwards in the source as you read the layer
/// left to right — "no mixing footage time" (K-071): `source_in` is
/// non-decreasing by timeline position. Gaps are allowed; reordering is not.
pub fn is_source_ordered(clips: &[Clip]) -> bool {
    let mut by_place: Vec<&Clip> = clips.iter().collect();
    by_place.sort_by(|a, b| {
        a.place_start
            .to_f64()
            .partial_cmp(&b.place_start.to_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    by_place
        .windows(2)
        .all(|w| w[0].source_in <= w[1].source_in)
}

/// Do any two clips overlap on the layer timeline? (docs/03-DATA-MODEL.md
/// §5.3 invariant: clips MUST NOT overlap — this is the check editors run
/// after a move before committing.)
pub fn has_overlap(clips: &[Clip]) -> bool {
    let mut spans: Vec<(f64, f64)> = clips
        .iter()
        .map(|c| (c.place_start.to_f64(), c.place_end().to_f64()))
        .collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    spans.windows(2).any(|w| w[1].0 < w[0].1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    fn clip(src: Uuid, place_start: i64, place_dur: i64) -> Clip {
        Clip::new(
            ClipSource::Footage(src),
            rat(0, 1),
            rat(place_dur, 1),
            rat(place_start, 1),
            rat(place_dur, 1),
        )
    }

    #[test]
    fn with_speed_reprices_the_clip_without_moving_it() {
        // A 4 s clip of source [0,4). Play it at 2× → it still occupies 4 s on
        // the layer (place unchanged) but consumes 8 s of source.
        let base = clip(Uuid::now_v7(), 3, 4);
        let fast = base.with_speed(rat(2, 1));
        assert_eq!(fast.place_start, base.place_start); // edit point held
        assert_eq!(fast.place_duration, base.place_duration);
        assert_eq!(fast.source_out, rat(8, 1)); // 4 s × 2×
        assert_eq!(fast.id, base.id); // same clip
        assert_eq!(fast.constant_speed(), Some(2.0));
        // Half speed consumes half the source.
        let slow = base.with_speed(rat(1, 2));
        assert_eq!(slow.source_out, rat(2, 1));
        assert_eq!(slow.constant_speed(), Some(0.5));
        // A plain clip reads as 1×.
        assert_eq!(base.constant_speed(), Some(1.0));
    }

    #[test]
    fn with_ramp_sets_a_velocity_ramp() {
        use crate::retime::Ease;
        // 4 s clip from source 0, speed ramping 1× → 3× (Linear): source used =
        // 4·[1 + (3−1)·E(1)] = 4·(1 + 2·0.5) = 8.
        let base = clip(Uuid::now_v7(), 0, 4);
        let ramp = base.with_ramp(rat(1, 1), rat(3, 1), Ease::Linear);
        assert_eq!(ramp.place_duration, base.place_duration); // place held
        assert_eq!(ramp.source_out, rat(8, 1));
        let (v0, v1, ease) = ramp.ramp_view().unwrap();
        assert!((v0 - 1.0).abs() < 1e-9 && (v1 - 3.0).abs() < 1e-9);
        assert_eq!(ease, Ease::Linear);
        // A ramp has no single constant speed.
        assert_eq!(ramp.constant_speed(), None);
    }

    #[test]
    fn resolution_picks_the_clip_under_the_playhead() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        // Clip A [0,2), then a gap [2,3), then clip B [3,5).
        let clips = vec![clip(a, 0, 2), clip(b, 3, 2)];
        assert_eq!(resolve(&clips, 1.0).unwrap().1, ClipSource::Footage(a));
        assert_eq!(resolve(&clips, 4.0).unwrap().1, ClipSource::Footage(b));
        // The gap and past-the-end render transparent (None).
        assert!(resolve(&clips, 2.5).is_none());
        assert!(resolve(&clips, 5.0).is_none());
        // Boundaries: start inclusive, end exclusive.
        assert!(resolve(&clips, 0.0).is_some());
        assert!(resolve(&clips, 2.0).is_none());
        assert!(resolve(&clips, 3.0).is_some());
    }

    #[test]
    fn source_time_runs_through_the_clip_retime() {
        // A clip at layer [2,6) whose source starts at 10s, played at half
        // speed: at layer time 4 (clip-local 2) the source is 10 + 0.5·2 = 11.
        let src = Uuid::now_v7();
        let mut c = clip(src, 2, 4);
        c.retime = Retime::constant_speed(rat(4, 1), rat(10, 1), rat(1, 2));
        assert!((c.source_time(2.0) - 10.0).abs() < 1e-9); // clip start
        assert!((c.source_time(4.0) - 11.0).abs() < 1e-9); // half speed
        assert!((c.source_time(6.0) - 12.0).abs() < 1e-9); // clip end
    }

    #[test]
    fn sliding_moves_the_clip_but_not_its_content() {
        // Clip at layer [2,6), source [0,4). Slide +3 → layer [5,9), same source.
        let src = Uuid::now_v7();
        let c = clip(src, 2, 4);
        let s = c.slide(rat(3, 1)).unwrap();
        assert_eq!(s.place_start, rat(5, 1));
        assert_eq!(s.place_duration, c.place_duration); // duration unchanged
        assert_eq!(s.source_in, c.source_in); // source window untouched
        assert_eq!(s.source_out, c.source_out);
        // The same source moments play, just later on the row (map untouched).
        assert!((s.source_time(5.0) - c.source_time(2.0)).abs() < 1e-9);
        assert!((s.source_time(7.0) - c.source_time(4.0)).abs() < 1e-9);
        // Sliding before the layer origin is refused.
        assert!(c.slide(rat(-3, 1)).is_none());
    }

    #[test]
    fn slipping_changes_the_source_but_not_the_place() {
        // Clip at layer [2,6), source [0,4) at natural rate. Slip +1 shows
        // source [1,5); the place is unchanged and every moment shifts by +1.
        let src = Uuid::now_v7();
        let c = clip(src, 2, 4);
        let s = c.slip(rat(1, 1)).unwrap();
        assert_eq!(s.place_start, c.place_start); // place held
        assert_eq!(s.place_duration, c.place_duration);
        assert_eq!(s.source_in, rat(1, 1)); // window shifted
        assert_eq!(s.source_out, rat(5, 1));
        for &lt in &[2.0, 4.0, 5.9] {
            assert!(
                (s.source_time(lt) - (c.source_time(lt) + 1.0)).abs() < 1e-9,
                "@ {lt}"
            );
        }
        // Slipping before the source start is refused.
        assert!(c.slip(rat(-1, 1)).is_none());
    }

    #[test]
    fn trimming_an_edge_inward_keeps_the_rest_in_place() {
        let src = Uuid::now_v7();
        // Clip at layer [2,6), source [0,4) at natural rate.
        let c = clip(src, 2, 4);
        // Trim the tail to end at 5 → layer [2,5), source [0,3).
        let t = c.trim_end(rat(5, 1)).unwrap();
        assert_eq!(t.id, c.id); // same clip identity
        assert_eq!(t.place_start, rat(2, 1));
        assert_eq!(t.place_duration, rat(3, 1));
        assert_eq!(t.source_out, rat(3, 1));
        for &lt in &[2.0, 3.5, 4.9] {
            assert!(
                (t.source_time(lt) - c.source_time(lt)).abs() < 1e-9,
                "tail @ {lt}"
            );
        }
        // Trim the head to start at 4 → layer [4,6), source [2,4), re-based.
        let h = c.trim_start(rat(4, 1)).unwrap();
        assert_eq!(h.id, c.id);
        assert_eq!(h.place_start, rat(4, 1));
        assert_eq!(h.place_duration, rat(2, 1));
        assert_eq!(h.source_in, rat(2, 1));
        for &lt in &[4.0, 5.0, 5.9] {
            assert!(
                (h.source_time(lt) - c.source_time(lt)).abs() < 1e-9,
                "head @ {lt}"
            );
        }
        // Outward trims (need §7.3 extend) and out-of-range edges are refused.
        assert!(c.trim_end(rat(7, 1)).is_none());
        assert!(c.trim_start(rat(1, 1)).is_none());
        assert!(c.trim_end(rat(2, 1)).is_none()); // zero length
    }

    #[test]
    fn trim_to_source_end_crops_a_tail_overrun() {
        let src = Uuid::now_v7();
        // Clip at layer [0,4), source [0,4). Retime it to 2× so f(t) = 2t runs
        // out of the source (out = 4) at local time 2.
        let mut c = clip(src, 0, 4);
        c.retime = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(2, 1));
        let t = c.trim_to_source_end().expect("a tail overrun trims");
        assert_eq!(t.place_start, c.place_start); // non-ripple: start held
        assert!((t.place_duration.to_f64() - 2.0).abs() < 1e-6);
        assert!((t.source_out.to_f64() - 4.0).abs() < 1e-6); // ends at the source end
                                                             // A clip that fits inside its source has nothing to trim.
        assert!(clip(src, 0, 4).trim_to_source_end().is_none());
    }

    #[test]
    fn overlap_detection() {
        let s = Uuid::now_v7();
        // Back-to-back is fine (end-exclusive touching).
        assert!(!has_overlap(&[clip(s, 0, 2), clip(s, 2, 2)]));
        // A gap is fine.
        assert!(!has_overlap(&[clip(s, 0, 2), clip(s, 5, 2)]));
        // Genuine overlap is caught.
        assert!(has_overlap(&[clip(s, 0, 3), clip(s, 2, 2)]));
    }

    #[test]
    fn cutting_partitions_a_clip_without_moving_it() {
        let src = Uuid::now_v7();
        // A clip at layer [2,6), source 0→4 at natural rate. Cut at layer 4.
        let c = clip(src, 2, 4);
        let (l, r) = c.cut(rat(4, 1)).unwrap();
        // Places abut exactly and don't move (beat-sync).
        assert_eq!(l.place_start, rat(2, 1));
        assert_eq!(l.place_duration, rat(2, 1));
        assert_eq!(r.place_start, rat(4, 1));
        assert_eq!(r.place_duration, rat(2, 1));
        // Source trims partition at the cut (source time 2 at layer time 4).
        assert_eq!(l.source_out, rat(2, 1));
        assert_eq!(r.source_in, rat(2, 1));
        // Each half plays the same source moment as the original did.
        assert!((l.source_time(3.0) - c.source_time(3.0)).abs() < 1e-9);
        assert!((r.source_time(5.0) - c.source_time(5.0)).abs() < 1e-9);
        // A cut outside the clip refuses.
        assert!(c.cut(rat(2, 1)).is_none());
        assert!(c.cut(rat(6, 1)).is_none());
    }

    /// The frame-pinning invariant (Mack's note): a clip's first frame is its
    /// `source_in`, whatever its speed. So splitting a clip and re-speeding the
    /// second half (e.g. 200% → 100%) leaves the second clip's *starting*
    /// frame exactly where it was — the speed change ripples forward only.
    #[test]
    fn re_speeding_a_cut_clip_keeps_its_start_frame() {
        use crate::retime::{Ease, Retime};
        let src = Uuid::now_v7();
        // Clip [0,4), source 0→4 natural. Cut at layer 2 → right clip [2,4).
        let (_left, right) = clip(src, 0, 4).cut(rat(2, 1)).unwrap();
        let start_frame = right.source_in; // the source moment at the cut
        assert!((right.source_time(2.0) - start_frame.to_f64()).abs() < 1e-9);

        // Re-speed the right clip: 200% ramping to 100% over its 2 s, pinned at
        // its own source_in (this is exactly what per-clip speed editing must
        // build). Its first frame must NOT move.
        let mut respeed = right.clone();
        respeed.retime = Retime::single_ramp(
            respeed.place_duration,
            respeed.source_in,
            rat(2, 1),
            rat(1, 1),
            Ease::Linear,
        );
        // First frame unchanged; only later frames advance faster.
        assert!((respeed.source_time(2.0) - start_frame.to_f64()).abs() < 1e-9);
        assert!(respeed.source_time(3.0) > right.source_time(3.0));
        // And it holds after moving the whole clip later on the layer (the
        // place shifts, the retime domain is unchanged, so the start frame is
        // still source_in).
        let mut moved = respeed.clone();
        moved.place_start = rat(5, 1);
        assert!((moved.source_time(5.0) - start_frame.to_f64()).abs() < 1e-9);
    }

    #[test]
    fn single_source_and_ordering_invariants() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        // Two clips of the same source in order.
        let c0 = Clip::new(
            ClipSource::Footage(a),
            rat(0, 1),
            rat(2, 1),
            rat(0, 1),
            rat(2, 1),
        );
        let c1 = Clip::new(
            ClipSource::Footage(a),
            rat(2, 1),
            rat(4, 1),
            rat(3, 1),
            rat(2, 1),
        );
        assert_eq!(
            single_source(&[c0.clone(), c1.clone()]),
            Some(ClipSource::Footage(a))
        );
        assert!(is_source_ordered(&[c0.clone(), c1.clone()]));
        // A gap between them is fine (still ordered).
        assert!(is_source_ordered(&[c0.clone(), c1.clone()]));
        // Mixed sources → not single-source.
        let other = Clip::new(
            ClipSource::Footage(b),
            rat(0, 1),
            rat(2, 1),
            rat(5, 1),
            rat(2, 1),
        );
        assert_eq!(single_source(&[c0.clone(), other]), None);
        assert_eq!(single_source(&[]), None);
        // Reordered so a later timeline slot holds an earlier source moment →
        // "mixing footage time", rejected.
        let early_source_late_place = Clip::new(
            ClipSource::Footage(a),
            rat(0, 1),
            rat(1, 1),
            rat(6, 1),
            rat(1, 1),
        );
        assert!(!is_source_ordered(&[c1, early_source_late_place]));
    }

    #[test]
    fn clip_round_trips_through_serde() {
        let c = clip(Uuid::now_v7(), 1, 4);
        let json = serde_json::to_string(&c).unwrap();
        let back: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}

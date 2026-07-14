//! Keyframe evaluation — docs/impl/keyframe-eval.md (binding), Phase 1.
//!
//! In plain terms: a keyframe curve between two keys is a bezier described by
//! AE-style *speed* (units per second) and *influence* (how far the handle
//! reaches, as a fraction of the gap). The curve is parametric, so asking
//! "what's the value at time t?" means first solving "which point on the
//! curve has x = t?" — and doing that solve sloppily is precisely why some
//! editors' graph editors feel wrong near steep handles. We use the impl
//! note's bracketed-Newton method: fast like Newton, and mathematically
//! incapable of escaping the valid range like plain Newton can.

use crate::time::Rational;
use serde::{Deserialize, Serialize};

/// Per-side interpolation of a keyframe (docs/03-DATA-MODEL.md §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SideInterp {
    Hold,
    Linear,
    /// AE-compatible: speed in value-units/second, influence in (0, 1].
    Bezier {
        speed: f64,
        influence: f64,
    },
}

/// A scalar keyframe. Time lives in the owner's timebase (kept rational so
/// keyframes hash and serialise exactly; evaluation converts to f64 once).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time: Rational,
    pub value: f64,
    /// Approaching this key.
    pub interp_in: SideInterp,
    /// Leaving this key.
    pub interp_out: SideInterp,
}

/// Easy-ease preset: speed 0, influence 33.33% (the AE constant).
pub const EASY_EASE: SideInterp = SideInterp::Bezier {
    speed: 0.0,
    influence: 1.0 / 3.0,
};

impl Keyframe {
    /// This keyframe made linear on both sides — straight lines to its
    /// neighbours, the default for a fresh key.
    pub fn to_linear(self) -> Keyframe {
        Keyframe {
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
            ..self
        }
    }

    /// This keyframe eased on both sides — the After Effects "easy ease" (F9):
    /// speed 0, influence 1/3, so the curve arrives and leaves flat and the
    /// tangent handles can then be dragged. The value is unchanged, so the curve
    /// still passes exactly through the key.
    pub fn to_bezier(self) -> Keyframe {
        Keyframe {
            interp_in: EASY_EASE,
            interp_out: EASY_EASE,
            ..self
        }
    }

    /// True when either side is a bezier (an eased key), so the UI can show it
    /// as a circle and give it tangent handles.
    pub fn is_bezier(&self) -> bool {
        matches!(self.interp_in, SideInterp::Bezier { .. })
            || matches!(self.interp_out, SideInterp::Bezier { .. })
    }
}

/// Evaluate a sorted keyframe list at time `t` (seconds, f64 — evaluation
/// domain per the engineering rules; authoritative times stay rational).
pub fn evaluate(keys: &[Keyframe], t: f64) -> Option<f64> {
    let first = keys.first()?;
    let last = keys.last()?;
    if t <= first.time.to_f64() {
        return Some(first.value);
    }
    if t >= last.time.to_f64() {
        return Some(last.value);
    }
    // Find the span containing t.
    let idx = keys
        .windows(2)
        .position(|w| t < w[1].time.to_f64())
        .unwrap_or(keys.len() - 2);
    let (a, b) = (&keys[idx], &keys[idx + 1]);
    Some(evaluate_span(a, b, t))
}

/// One span, honouring the pair of adjacent sides. Hold-out wins the span
/// (docs/impl/keyframe-eval.md §2).
fn evaluate_span(a: &Keyframe, b: &Keyframe, t: f64) -> f64 {
    let (t1, t2) = (a.time.to_f64(), b.time.to_f64());
    let dt = t2 - t1;
    if dt <= 0.0 {
        return a.value;
    }
    match (a.interp_out, b.interp_in) {
        (SideInterp::Hold, _) => a.value,
        (SideInterp::Linear, SideInterp::Linear) => a.value + (b.value - a.value) * ((t - t1) / dt),
        (out_side, in_side) => {
            // Mixed linear/bezier sides: a linear side is a bezier whose
            // handle lies on the chord (speed = chord slope, influence ⅓).
            let chord = (b.value - a.value) / dt;
            let (s1, b1) = side_params(out_side, chord);
            let (s2, b2) = side_params(in_side, chord);
            let cubic = CubicSpan::from_ae(t1, a.value, t2, b.value, s1, b1, s2, b2);
            cubic.value_at(t)
        }
    }
}

fn side_params(side: SideInterp, chord_slope: f64) -> (f64, f64) {
    match side {
        SideInterp::Bezier { speed, influence } => (speed, influence.clamp(1e-3, 1.0)),
        // Linear (or hold-in, which only matters as an out-side) on the chord.
        _ => (chord_slope, 1.0 / 3.0),
    }
}

/// The cubic bezier for one span, built from AE parameters
/// (docs/impl/keyframe-eval.md §1):
///   P0=(t1,v1)  P1=(t1+b1·Δt, v1+s1·b1·Δt)  P2=(t2−b2·Δt, v2−s2·b2·Δt)  P3=(t2,v2)
pub struct CubicSpan {
    x: [f64; 4],
    y: [f64; 4],
}

impl CubicSpan {
    #[allow(clippy::too_many_arguments)]
    pub fn from_ae(
        t1: f64,
        v1: f64,
        t2: f64,
        v2: f64,
        speed_out: f64,
        infl_out: f64,
        speed_in: f64,
        infl_in: f64,
    ) -> Self {
        let dt = t2 - t1;
        Self {
            x: [t1, t1 + infl_out * dt, t2 - infl_in * dt, t2],
            y: [
                v1,
                v1 + speed_out * infl_out * dt,
                v2 - speed_in * infl_in * dt,
                v2,
            ],
        }
    }

    fn bezier(p: &[f64; 4], u: f64) -> f64 {
        let w = 1.0 - u;
        w * w * w * p[0] + 3.0 * w * w * u * p[1] + 3.0 * w * u * u * p[2] + u * u * u * p[3]
    }

    fn bezier_deriv(p: &[f64; 4], u: f64) -> f64 {
        let w = 1.0 - u;
        3.0 * w * w * (p[1] - p[0]) + 6.0 * w * u * (p[2] - p[1]) + 3.0 * u * u * (p[3] - p[2])
    }

    /// Solve x(u) = t by Newton inside a shrinking bracket
    /// (docs/impl/keyframe-eval.md §2 — binding; do not substitute).
    pub fn solve_u(&self, t: f64) -> f64 {
        let (x0, x3) = (self.x[0], self.x[3]);
        if x3 <= x0 {
            return 0.0;
        }
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        let mut u = ((t - x0) / (x3 - x0)).clamp(0.0, 1.0); // x ≈ identity guess
        for _ in 0..16 {
            let xu = Self::bezier(&self.x, u);
            if (xu - t).abs() < 1e-12 {
                break;
            }
            if xu < t {
                lo = u;
            } else {
                hi = u;
            }
            let dxu = Self::bezier_deriv(&self.x, u);
            let newton = u - (xu - t) / dxu;
            u = if dxu > 1e-12 && newton > lo && newton < hi {
                newton
            } else {
                0.5 * (lo + hi)
            };
        }
        u
    }

    pub fn value_at(&self, t: f64) -> f64 {
        Self::bezier(&self.y, self.solve_u(t))
    }
}

/// An animatable scalar slot (docs/03-DATA-MODEL.md §6.1; the expression slot
/// joins in Phase 4). Phase 1 starts with separated scalar dimensions —
/// coupled Vec2 spatial paths and roving keyframes arrive with the
/// motion-path work (status-noted in the data model doc).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Animation {
    Static(f64),
    /// Sorted by time, unique times (enforced by the editing ops).
    Keyframed(Vec<Keyframe>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    pub animation: Animation,
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Property {
    /// serde-default helper for 2.5D fields added after 1.0 projects existed.
    pub fn zero() -> Self {
        Self::fixed(0.0)
    }

    pub fn fixed(value: f64) -> Self {
        Self {
            animation: Animation::Static(value),
            extra: serde_json::Map::new(),
        }
    }

    /// Evaluate at a time in the owner's timebase (seconds).
    pub fn value_at(&self, t: f64) -> f64 {
        match &self.animation {
            Animation::Static(v) => *v,
            Animation::Keyframed(keys) => evaluate(keys, t).unwrap_or(0.0),
        }
    }

    pub fn is_animated(&self) -> bool {
        matches!(&self.animation, Animation::Keyframed(keys) if !keys.is_empty())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    fn key(t: Rational, v: f64, side: SideInterp) -> Keyframe {
        Keyframe {
            time: t,
            value: v,
            interp_in: side,
            interp_out: side,
        }
    }

    #[test]
    fn linear_hold_and_clamping() {
        let keys = [
            key(rat(0, 1), 0.0, SideInterp::Linear),
            key(rat(1, 1), 10.0, SideInterp::Linear),
            Keyframe {
                time: rat(2, 1),
                value: 20.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Hold,
            },
            key(rat(3, 1), 5.0, SideInterp::Linear),
        ];
        assert_eq!(evaluate(&keys, -1.0), Some(0.0)); // clamp before
        assert_eq!(evaluate(&keys, 0.5), Some(5.0)); // linear
        assert_eq!(evaluate(&keys, 2.5), Some(20.0)); // hold-out wins the span
        assert_eq!(evaluate(&keys, 9.0), Some(5.0)); // clamp after
    }

    #[test]
    fn easy_ease_is_flat_at_both_keys_and_monotone() {
        let keys = [
            key(rat(0, 1), 0.0, EASY_EASE),
            key(rat(1, 1), 100.0, EASY_EASE),
        ];
        // Flat tangents: near the keys the value barely moves.
        let near0 = evaluate(&keys, 0.01).unwrap();
        let near1 = evaluate(&keys, 0.99).unwrap();
        assert!(near0 < 0.5, "start not flat: {near0}");
        assert!(near1 > 99.5, "end not flat: {near1}");
        // Midpoint of a symmetric ease is the midpoint value.
        let mid = evaluate(&keys, 0.5).unwrap();
        assert!((mid - 50.0).abs() < 1e-9, "mid {mid}");
        // Monotone in, monotone out.
        let mut prev = f64::MIN;
        for i in 0..=1000 {
            let v = evaluate(&keys, i as f64 / 1000.0).unwrap();
            assert!(v >= prev - 1e-9, "not monotone at {i}: {v} < {prev}");
            prev = v;
        }
    }

    #[test]
    fn linear_bezier_conversion_keeps_the_key_values() {
        let k = key(rat(1, 1), 5.0, SideInterp::Linear);
        assert!(!k.is_bezier());
        let b = k.to_bezier();
        assert!(b.is_bezier());
        assert_eq!(b.interp_in, EASY_EASE);
        assert_eq!(b.interp_out, EASY_EASE);
        assert!((b.value - 5.0).abs() < 1e-12); // value unchanged
        let l = b.to_linear();
        assert!(!l.is_bezier());
        assert_eq!(l.interp_in, SideInterp::Linear);
        // Whether linear or eased, the curve still passes exactly through each key.
        let eased = [
            key(rat(0, 1), 0.0, SideInterp::Linear).to_bezier(),
            key(rat(1, 1), 10.0, SideInterp::Linear).to_bezier(),
            key(rat(2, 1), 0.0, SideInterp::Linear).to_bezier(),
        ];
        assert!((evaluate(&eased, 0.0).unwrap() - 0.0).abs() < 1e-9);
        assert!((evaluate(&eased, 1.0).unwrap() - 10.0).abs() < 1e-9);
        assert!((evaluate(&eased, 2.0).unwrap() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn full_influence_spike_is_exact_not_explosive() {
        // 100% influence both sides with zero speed: the x-curve has dx=0 at
        // the endpoints — the case that diverges under plain Newton.
        let spike = SideInterp::Bezier {
            speed: 0.0,
            influence: 1.0,
        };
        let keys = [key(rat(0, 1), 0.0, spike), key(rat(1, 1), 1.0, spike)];
        for i in 0..=100 {
            let t = i as f64 / 100.0;
            let v = evaluate(&keys, t).unwrap();
            assert!(v.is_finite());
            assert!((-1e-9..=1.0 + 1e-9).contains(&v), "t={t} v={v}");
        }
        assert!((evaluate(&keys, 0.5).unwrap() - 0.5).abs() < 1e-9);
    }

    proptest! {
        /// solve_u(x(u)) == u to 1e-10 over random monotone cubics,
        /// including dx = 0 endpoints (keyframe-eval.md test plan §2).
        #[test]
        fn solve_round_trips(
            b1 in 0.001f64..=1.0,
            b2 in 0.001f64..=1.0,
            s1 in -5.0f64..5.0,
            s2 in -5.0f64..5.0,
            u in 0.0f64..=1.0,
        ) {
            let cubic = CubicSpan::from_ae(0.0, 0.0, 1.0, 1.0, s1, b1, s2, b2);
            let t = CubicSpan::bezier(&cubic.x, u);
            let solved = cubic.solve_u(t);
            let t_back = CubicSpan::bezier(&cubic.x, solved);
            // Compare in x-space: distinct u can map to equal x at flat spots.
            prop_assert!((t_back - t).abs() < 1e-10, "t {t} → u {solved} → {t_back}");
        }

        /// Evaluation stays within the hull of the two key values whenever
        /// both handles point "inward" (no overshoot without overshooting
        /// handles) — and is always finite.
        #[test]
        fn no_spurious_overshoot(
            b1 in 0.001f64..=1.0,
            b2 in 0.001f64..=1.0,
            t in 0.0f64..=1.0,
        ) {
            let keys = [
                key(rat(0,1), 0.0, SideInterp::Bezier { speed: 0.0, influence: b1 }),
                key(rat(1,1), 1.0, SideInterp::Bezier { speed: 0.0, influence: b2 }),
            ];
            let v = evaluate(&keys, t).unwrap();
            prop_assert!(v.is_finite());
            prop_assert!((-1e-9..=1.0 + 1e-9).contains(&v));
        }
    }

    /// Perf sanity from the impl note: 10⁶ evaluations well under budget.
    #[test]
    fn million_evaluations_stay_cheap() {
        let keys = [
            key(rat(0, 1), 0.0, EASY_EASE),
            key(rat(1, 1), 100.0, EASY_EASE),
        ];
        let start = std::time::Instant::now();
        let mut acc = 0.0;
        for i in 0..1_000_000 {
            acc += evaluate(&keys, (i % 1000) as f64 / 1000.0).unwrap_or(0.0);
        }
        let elapsed = start.elapsed();
        assert!(acc.is_finite());
        // Debug-build headroom: impl note budgets 20 ms release; allow 40× debug.
        assert!(elapsed.as_millis() < 800, "1M evals took {elapsed:?}");
    }
}

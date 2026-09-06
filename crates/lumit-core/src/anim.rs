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

use std::sync::Arc;

use crate::{expression::ExpressionContext, time::Rational};
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
    /// A side whose **speed is computed from the key's neighbours** on every
    /// read rather than stored (docs/impl/keyframe-eval.md §6): Auto is the
    /// Catmull-Rom slope through the two neighbouring keys, and `clamped` is
    /// the same slope limited so the span cannot overshoot them.
    ///
    /// `speed` and `influence` are **not** evaluated. They are the ease this
    /// side carried when it was last Free, held here so that Free → Auto →
    /// Free hands the custom ease back exactly (the study's explicit bar,
    /// §6.3 of docs/impl/timeline-interaction.md). Carrying them inside the
    /// side is what lets a key list cross the bridge and come back without a
    /// merge step: the memory travels with the thing that owns it.
    Auto {
        clamped: bool,
        speed: f64,
        influence: f64,
    },
}

/// Which of the three tangent modes a side is in — the graph strip's
/// Auto / Clamp / Free (docs/impl/timeline-interaction.md §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TangentMode {
    Auto,
    Clamp,
    Free,
}

impl SideInterp {
    /// Which mode this side is in.
    #[must_use]
    pub fn tangent_mode(self) -> TangentMode {
        match self {
            SideInterp::Auto { clamped: false, .. } => TangentMode::Auto,
            SideInterp::Auto { clamped: true, .. } => TangentMode::Clamp,
            _ => TangentMode::Free,
        }
    }

    /// This side put into [`mode`](TangentMode) — the strip's three buttons.
    ///
    /// The ease is never destroyed by the switch: going automatic files the
    /// side's current speed and influence inside the [`SideInterp::Auto`] arm,
    /// and coming back to Free hands them out again. A side that was straight
    /// or held has no ease of its own to keep, so it files the easy-ease pair
    /// and returns as an eased key — which is what an automatic side has been
    /// in the meantime.
    #[must_use]
    pub fn with_tangent_mode(self, mode: TangentMode) -> SideInterp {
        let (speed, influence) = match self {
            SideInterp::Bezier { speed, influence }
            | SideInterp::Auto {
                speed, influence, ..
            } => (speed, influence),
            SideInterp::Hold | SideInterp::Linear => (0.0, 1.0 / 3.0),
        };
        match mode {
            TangentMode::Auto => SideInterp::Auto {
                clamped: false,
                speed,
                influence,
            },
            TangentMode::Clamp => SideInterp::Auto {
                clamped: true,
                speed,
                influence,
            },
            TangentMode::Free => match self {
                SideInterp::Auto { .. } => SideInterp::Bezier { speed, influence },
                other => other,
            },
        }
    }
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

    /// This keyframe held on both sides: the value stays exactly at the key
    /// until the next key's time, then steps to it — no interpolation in
    /// between. This is the discrete/stepped key, and the only animation a
    /// string-valued property (e.g. a File param) can carry, since two file
    /// paths cannot be blended.
    pub fn to_hold(self) -> Keyframe {
        Keyframe {
            interp_in: SideInterp::Hold,
            interp_out: SideInterp::Hold,
            ..self
        }
    }

    /// True when either side is a bezier (an eased key), so the UI can show it
    /// as a circle and give it tangent handles. An automatic side counts: it
    /// has a tangent, the neighbours simply choose where it points.
    pub fn is_bezier(&self) -> bool {
        matches!(
            self.interp_in,
            SideInterp::Bezier { .. } | SideInterp::Auto { .. }
        ) || matches!(
            self.interp_out,
            SideInterp::Bezier { .. } | SideInterp::Auto { .. }
        )
    }

    /// True when the out-side holds, so the value steps at the next key rather
    /// than blending toward it (the out-side is what governs the span leaving
    /// this key — see [`evaluate_span`]).
    pub fn is_hold(&self) -> bool {
        matches!(self.interp_out, SideInterp::Hold)
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
    Some(evaluate_span(keys, span_index(keys, t), t))
}

/// The speed an automatic side takes at `keys[i]` (docs/impl/keyframe-eval.md
/// §6), in value-units per second.
///
/// In plain terms: an automatic tangent points the way the curve is already
/// going. The plain one aims straight from the key before to the key after —
/// the Catmull-Rom slope — which is smooth but can bulge past a neighbour on
/// the way. The clamped one is the same aim with two limits that make the
/// bulge impossible: it lies flat where the key is a peak or a trough (the two
/// neighbours are on the same side of it, so any tilt would overshoot one of
/// them), and elsewhere it is held to three times the gentler of the two
/// chords, which is the classical monotone-cubic bound.
///
/// An end key has no pair to aim between, so its automatic tangent is flat —
/// the curve arrives and leaves level, which is what "auto ease" means there.
pub fn auto_speed(keys: &[Keyframe], i: usize, clamped: bool) -> f64 {
    if i == 0 || i + 1 >= keys.len() {
        return 0.0;
    }
    let (p, k, n) = (&keys[i - 1], &keys[i], &keys[i + 1]);
    let (tp, tk, tn) = (p.time.to_f64(), k.time.to_f64(), n.time.to_f64());
    if tn - tp <= 0.0 {
        return 0.0;
    }
    let smooth = (n.value - p.value) / (tn - tp);
    if !clamped {
        return smooth;
    }
    if tk - tp <= 0.0 || tn - tk <= 0.0 {
        return 0.0;
    }
    let before = (k.value - p.value) / (tk - tp);
    let after = (n.value - k.value) / (tn - tk);
    if before * after <= 0.0 {
        return 0.0;
    }
    let limit = 3.0 * before.abs().min(after.abs());
    smooth.clamp(-limit, limit)
}

/// One side of `keys[i]` as the evaluator sees it: an automatic side becomes
/// the bezier its neighbours dictate, everything else is itself.
///
/// The influence is the side's own — an automatic tangent decides which way
/// the handle points, not how far it reaches.
#[must_use]
pub fn resolved_side(keys: &[Keyframe], i: usize, out: bool) -> SideInterp {
    let side = if out {
        keys[i].interp_out
    } else {
        keys[i].interp_in
    };
    match side {
        SideInterp::Auto {
            clamped, influence, ..
        } => SideInterp::Bezier {
            speed: auto_speed(keys, i, clamped),
            influence,
        },
        other => other,
    }
}

/// The instantaneous speed dv/dt at time `t` (value-units per second) — the
/// exact derivative of what [`evaluate`] returns, so the speed lens draws the
/// true derivative of the value bezier rather than a finite-difference guess.
/// Held past the ends (the value is clamped there, so the slope is 0)
/// and 0 across a Hold-out span. `None` only when the key list is empty.
pub fn evaluate_speed(keys: &[Keyframe], t: f64) -> Option<f64> {
    let first = keys.first()?;
    let last = keys.last()?;
    if t <= first.time.to_f64() || t >= last.time.to_f64() {
        return Some(0.0);
    }
    Some(evaluate_speed_span(keys, span_index(keys, t), t))
}

/// The index of the span containing `t` — the first `i` with
/// `t < keys[i + 1].time` — by binary search, for callers that have already
/// handled `t` at or beyond the ends.
///
/// In plain terms: a curve with thousands of keys (an imported camera arrives
/// with one per frame) is sampled at least once per rendered frame per
/// property, and walking the list from the front made every sample cost the
/// whole list. Bisecting costs a dozen looks whatever the count, which is what
/// keeps the impl note's bench gate (10⁶ evaluations < 20 ms) true of dense
/// curves and not only of sparse ones.
fn span_index(keys: &[Keyframe], t: f64) -> usize {
    // The first key strictly past `t`; the guards in the callers put `t`
    // strictly inside the keyed range, so `p` is in `1..keys.len()` and the
    // span leaving `keys[p - 1]` is the one `t` sits on.
    let p = keys.partition_point(|k| k.time.to_f64() <= t);
    p.saturating_sub(1).min(keys.len().saturating_sub(2))
}

/// One span's slope, matching [`evaluate_span`]'s side handling: a Hold-out span
/// is flat (0), a straight span is the chord slope, and a bezier span is the
/// value curve's exact derivative.
fn evaluate_speed_span(keys: &[Keyframe], i: usize, t: f64) -> f64 {
    let (a, b) = (&keys[i], &keys[i + 1]);
    let (t1, t2) = (a.time.to_f64(), b.time.to_f64());
    let dt = t2 - t1;
    if dt <= 0.0 {
        return 0.0;
    }
    match (a.interp_out, b.interp_in) {
        (SideInterp::Hold, _) => 0.0,
        (SideInterp::Linear, SideInterp::Linear) => (b.value - a.value) / dt,
        _ => span_cubic(keys, i).speed_at(t),
    }
}

/// One span, honouring the pair of adjacent sides. Hold-out wins the span
/// (docs/impl/keyframe-eval.md §2).
fn evaluate_span(keys: &[Keyframe], i: usize, t: f64) -> f64 {
    let (a, b) = (&keys[i], &keys[i + 1]);
    let (t1, t2) = (a.time.to_f64(), b.time.to_f64());
    let dt = t2 - t1;
    if dt <= 0.0 {
        return a.value;
    }
    match (a.interp_out, b.interp_in) {
        (SideInterp::Hold, _) => a.value,
        (SideInterp::Linear, SideInterp::Linear) => a.value + (b.value - a.value) * ((t - t1) / dt),
        _ => span_cubic(keys, i).value_at(t),
    }
}

/// The cubic across the span leaving `keys[i]`, its two sides resolved
/// ([`resolved_side`]) so an automatic tangent is the one the neighbours
/// dictate.
fn span_cubic(keys: &[Keyframe], i: usize) -> CubicSpan {
    let (a, b) = (&keys[i], &keys[i + 1]);
    let (t1, t2) = (a.time.to_f64(), b.time.to_f64());
    // Mixed linear/bezier sides: a linear side is a bezier whose handle lies
    // on the chord (speed = chord slope, influence ⅓).
    let chord = (b.value - a.value) / (t2 - t1);
    let (s1, b1) = side_params(resolved_side(keys, i, true), chord);
    let (s2, b2) = side_params(resolved_side(keys, i + 1, false), chord);
    CubicSpan::from_ae(t1, a.value, t2, b.value, s1, b1, s2, b2)
}

/// A side's (speed, influence). Takes a side that has already been through
/// [`resolved_side`], so an automatic one never reaches here as itself; the
/// fallback treats one that does as straight, which is the calmest wrong
/// answer rather than a panic in an engine crate.
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

    /// A span from raw control points. The maths of a cubic does not care what
    /// its two axes mean, so this is also how a *geometric* cubic — a bezier
    /// path segment, x against y — borrows [`Self::split_at`] instead of the
    /// codebase carrying a second de Casteljau (see `mask::resample`). Only the
    /// time-flavoured methods (`solve_u`, `value_at`, the AE side conversions)
    /// assume x is time.
    #[must_use]
    pub fn from_points(x: [f64; 4], y: [f64; 4]) -> Self {
        Self { x, y }
    }

    /// The four control points, x and y.
    #[must_use]
    pub fn control_points(&self) -> ([f64; 4], [f64; 4]) {
        (self.x, self.y)
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

    /// De Casteljau: the two cubics that together *are* this one, meeting at
    /// parameter `u`. Exact, not an approximation — which is the whole
    /// reason a key can be inserted mid-span without the picture changing.
    #[must_use]
    pub fn split_at(&self, u: f64) -> (Self, Self) {
        fn split(p: &[f64; 4], u: f64) -> ([f64; 4], [f64; 4]) {
            let lerp = |a: f64, b: f64| a + (b - a) * u;
            let (q0, q1, q2) = (lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3]));
            let (r0, r1) = (lerp(q0, q1), lerp(q1, q2));
            let s = lerp(r0, r1);
            ([p[0], q0, r0, s], [s, r1, q2, p[3]])
        }
        let (lx, rx) = split(&self.x, u);
        let (ly, ry) = split(&self.y, u);
        (Self { x: lx, y: ly }, Self { x: rx, y: ry })
    }

    /// This span's leaving side, as the AE speed/influence pair a keyframe
    /// stores — the exact inverse of [`Self::from_ae`]'s first handle.
    ///
    /// Influence is clamped into the range [`side_params`] allows, so a handle
    /// that came out of a split at a very small parameter still describes a
    /// side the evaluator will accept.
    #[must_use]
    pub fn out_side(&self) -> SideInterp {
        let dt = self.x[3] - self.x[0];
        let dx = self.x[1] - self.x[0];
        if dt <= 0.0 || dx.abs() < 1e-12 {
            return SideInterp::Bezier {
                speed: 0.0,
                influence: 1e-3,
            };
        }
        SideInterp::Bezier {
            speed: (self.y[1] - self.y[0]) / dx,
            influence: (dx / dt).clamp(1e-3, 1.0),
        }
    }

    /// The same for the side approaching this span's end.
    #[must_use]
    pub fn in_side(&self) -> SideInterp {
        let dt = self.x[3] - self.x[0];
        let dx = self.x[3] - self.x[2];
        if dt <= 0.0 || dx.abs() < 1e-12 {
            return SideInterp::Bezier {
                speed: 0.0,
                influence: 1e-3,
            };
        }
        SideInterp::Bezier {
            speed: (self.y[3] - self.y[2]) / dx,
            influence: (dx / dt).clamp(1e-3, 1.0),
        }
    }

    /// The instantaneous slope dv/dt at time `t` — the value curve's derivative,
    /// `y′(u)/x′(u)` at the parameter with `x(u) = t`. `x′` can touch zero at a
    /// 100%-influence handle, so it is floored to keep the speed finite (the
    /// curve is simply "very fast" there). This is what the speed lens draws so
    /// its curve is the exact derivative of the value bezier.
    pub fn speed_at(&self, t: f64) -> f64 {
        let u = self.solve_u(t);
        Self::bezier_deriv(&self.y, u) / Self::bezier_deriv(&self.x, u).max(1e-12)
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
    Expression(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    pub animation: Animation,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
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
            Animation::Expression(expression) => crate::expression::evaluate(expression, None),
        }
    }

    pub fn value_at_with_context(&self, t: f64, context: Arc<ExpressionContext>) -> f64 {
        match &self.animation {
            Animation::Static(v) => *v,
            Animation::Keyframed(keys) => evaluate(keys, t).unwrap_or(0.0),
            Animation::Expression(expression) => {
                crate::expression::evaluate(expression, Some(context))
            }
        }
    }

    pub fn is_animated(&self) -> bool {
        matches!(&self.animation, Animation::Keyframed(keys) if !keys.is_empty())
    }

    /// Put a keyframe at `t` **without changing the curve**.
    ///
    /// The new key takes the value the curve already had there, and the two
    /// halves of the span it lands in are re-described so that every value
    /// before and after it is exactly what it was. That is what makes this safe
    /// to do behind the user's back — the razor does it on every cut of a
    /// retimed layer (docs/07 §4.4), and a cut that changed the speed ramp it
    /// was cutting would be worse than no cut at all.
    ///
    /// **How the shape survives.** A span is one cubic bezier
    /// (docs/impl/keyframe-eval.md §1). Splitting a cubic at a parameter with de
    /// Casteljau gives two cubics whose union *is* the original curve — not an
    /// approximation of it, the same curve — so all that is left is converting
    /// the four control points back into the AE speed/influence pair each side
    /// is stored as. Those conversions are the exact inverse of
    /// [`CubicSpan::from_ae`].
    ///
    /// A no-op when: the property is not keyframed (there is no curve to keep),
    /// a key already sits at `t`, or the span is a Hold — a held span has no
    /// shape to preserve, so the key is inserted flat and the hold continues.
    ///
    /// Returns whether a key was added.
    pub fn insert_key_preserving_shape(&mut self, t: Rational) -> bool {
        let Animation::Keyframed(keys) = &mut self.animation else {
            return false;
        };
        if keys.is_empty() {
            return false;
        }
        let tf = t.to_f64();
        if keys.iter().any(|k| (k.time.to_f64() - tf).abs() < 1e-12) {
            return false;
        }

        // Outside the keyed range the property holds its end value, so a key
        // there needs no shape work: it takes that value and the same sides.
        let first = keys[0];
        if tf < first.time.to_f64() {
            keys.insert(
                0,
                Keyframe {
                    time: t,
                    value: first.value,
                    interp_in: first.interp_in,
                    interp_out: SideInterp::Linear,
                },
            );
            return true;
        }
        let last = keys[keys.len() - 1];
        if tf > last.time.to_f64() {
            keys.push(Keyframe {
                time: t,
                value: last.value,
                interp_in: SideInterp::Linear,
                interp_out: last.interp_out,
            });
            return true;
        }

        let Some(i) = keys.windows(2).position(|w| {
            let (a, b) = (w[0].time.to_f64(), w[1].time.to_f64());
            tf > a && tf < b
        }) else {
            return false;
        };
        let a = keys[i];

        // A held span is flat by definition: the key takes the held value and
        // holds on, and neither neighbour needs touching.
        if matches!(a.interp_out, SideInterp::Hold) {
            keys.insert(
                i + 1,
                Keyframe {
                    time: t,
                    value: a.value,
                    interp_in: SideInterp::Hold,
                    interp_out: SideInterp::Hold,
                },
            );
            return true;
        }

        let cubic = span_cubic(keys, i);
        let u = cubic.solve_u(tf);
        let (left, right) = cubic.split_at(u);

        // The split point is the curve's own value there, taken from the split
        // rather than re-evaluated, so the two halves meet exactly.
        let value = left.y[3];
        // An **automatic** neighbour keeps its mode rather than being frozen
        // into the shape it happened to have: its tangent is a function of its
        // neighbours, and one of those has just changed. The shape either side
        // of the new key is preserved wherever the side stored one; where it
        // did not, the side goes on recomputing, which is what it is for.
        if !matches!(keys[i].interp_out, SideInterp::Auto { .. }) {
            keys[i].interp_out = left.out_side();
        }
        if !matches!(keys[i + 1].interp_in, SideInterp::Auto { .. }) {
            keys[i + 1].interp_in = right.in_side();
        }
        keys.insert(
            i + 1,
            Keyframe {
                time: t,
                value,
                interp_in: left.in_side(),
                interp_out: right.out_side(),
            },
        );
        true
    }

    /// Write a **fade** across `[from, to]` in this property's own clock: a
    /// pair of keys between `floor` and the level the property already holds
    /// at the loud end, shaped by `shape` (docs/09 §6).
    ///
    /// `rising` is a fade *in* — silence at `from`, level at `to`; false is a
    /// fade out. Any keys already inside the span are replaced, so running a
    /// fade twice over the same stretch reshapes it rather than layering two
    /// curves on top of each other; keys outside it are left exactly where
    /// they are, which is what lets a fade-in and a fade-out live on one
    /// property without knowing about each other.
    ///
    /// A no-op (returns `None`) when the span has no length, or when the
    /// property is an expression — a fade would have to rewrite what somebody
    /// typed, which is an edit of their work rather than a command.
    #[must_use]
    pub fn with_fade(
        &self,
        from: Rational,
        to: Rational,
        shape: FadeShape,
        floor: f64,
        rising: bool,
    ) -> Option<Property> {
        if to.to_f64() <= from.to_f64() || matches!(self.animation, Animation::Expression(_)) {
            return None;
        }
        let loud_at = if rising { to } else { from };
        let level = self.value_at(loud_at.to_f64());
        let (loud_side, quiet_side) = shape.sides();
        let key = |time: Rational, value: f64, side: SideInterp| Keyframe {
            time,
            value,
            interp_in: side,
            interp_out: side,
        };
        let pair = if rising {
            [key(from, floor, quiet_side), key(to, level, loud_side)]
        } else {
            [key(from, level, loud_side), key(to, floor, quiet_side)]
        };

        let mut keys: Vec<Keyframe> = match &self.animation {
            Animation::Keyframed(existing) => existing
                .iter()
                .filter(|k| k.time.to_f64() < from.to_f64() || k.time.to_f64() > to.to_f64())
                .copied()
                .collect(),
            // A static property has one level everywhere; the pair is the
            // whole animation, and the value outside the span still holds
            // because a keyed property holds its end values.
            _ => Vec::new(),
        };
        keys.extend(pair);
        keys.sort_by(|a, b| {
            a.time
                .to_f64()
                .partial_cmp(&b.time.to_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(Property {
            animation: Animation::Keyframed(keys),
            extra: self.extra.clone(),
        })
    }
}

/// The curve a fade command writes (docs/09 §6) — the board's three
/// chips, each built out of the easing vocabulary that already exists rather
/// than a new kind of span.
///
/// **Volume is in decibels**, which is roughly how loudness is heard, so a
/// straight line between silence and level is already the perceptually even
/// fade — that is why `Linear` is a real option here and not the naive one.
/// The other two bend it: `Ease` softens both ends, and `Exponential` holds
/// the level and then lets it fall away, which is the shape that spends the
/// most of a fade's length in the part you can actually hear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeShape {
    /// Straight in dB: flat handles nowhere, an even ramp.
    Linear,
    /// Easy-ease at both ends — the default, and the one a fade under a
    /// title wants.
    Ease,
    /// Eased at the loud end, straight at the silent one: the level hangs
    /// where it was and then drops away.
    Exponential,
}

impl FadeShape {
    /// `(the loud key's sides, the silent key's sides)`.
    fn sides(self) -> (SideInterp, SideInterp) {
        match self {
            FadeShape::Linear => (SideInterp::Linear, SideInterp::Linear),
            FadeShape::Ease => (EASY_EASE, EASY_EASE),
            FadeShape::Exponential => (EASY_EASE, SideInterp::Linear),
        }
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

    /// **A fade is a pair of ordinary keys** (docs/09 §6): between the
    /// −∞ knee and the level the property already held, shaped from the
    /// easing vocabulary that already exists, with everything outside the
    /// span left alone.
    #[test]
    fn a_fade_is_an_eased_pair_that_leaves_the_rest_of_the_curve_alone() {
        let floor = -100.0;
        let level = Property::fixed(-3.0);

        // Fade in: silence at the start of the span, the level at its end.
        let faded = level
            .with_fade(rat(0, 1), rat(1, 1), FadeShape::Ease, floor, true)
            .expect("a fade in");
        assert!((faded.value_at(0.0) - floor).abs() < 1e-9);
        assert!((faded.value_at(1.0) - -3.0).abs() < 1e-9);
        assert!(
            faded.value_at(5.0) == -3.0,
            "past the last key a property holds its level"
        );
        let Animation::Keyframed(keys) = &faded.animation else {
            panic!("a fade is keyframed");
        };
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].interp_out, EASY_EASE, "Ease eases both keys");
        assert_eq!(keys[1].interp_in, EASY_EASE);

        // Fade out: the level at the start of the span, silence at its end.
        let out = level
            .with_fade(rat(2, 1), rat(3, 1), FadeShape::Linear, floor, false)
            .expect("a fade out");
        assert!((out.value_at(2.0) - -3.0).abs() < 1e-9);
        assert!((out.value_at(3.0) - floor).abs() < 1e-9);
        let Animation::Keyframed(keys) = &out.animation else {
            panic!("keyframed");
        };
        assert!(keys.iter().all(|k| k.interp_in == SideInterp::Linear));

        // Exponential eases the loud key and leaves the silent one straight,
        // so the level hangs where it was and then falls away.
        let exp = level
            .with_fade(rat(0, 1), rat(1, 1), FadeShape::Exponential, floor, false)
            .expect("a fade out");
        let Animation::Keyframed(keys) = &exp.animation else {
            panic!("keyframed");
        };
        assert_eq!(keys[0].interp_out, EASY_EASE, "the loud key is eased");
        assert_eq!(keys[1].interp_in, SideInterp::Linear);
        assert!(
            exp.value_at(0.25) > out.value_at(2.25),
            "an exponential fade is still louder a quarter of the way down"
        );

        // The two halves coexist: fading a property that already has a fade
        // in leaves those keys where they are.
        let both = faded
            .with_fade(rat(3, 1), rat(4, 1), FadeShape::Ease, floor, false)
            .expect("a fade out over a fade in");
        let Animation::Keyframed(keys) = &both.animation else {
            panic!("keyframed");
        };
        assert_eq!(keys.len(), 4, "two keys each, in time order");
        assert!(keys
            .windows(2)
            .all(|w| w[0].time.to_f64() < w[1].time.to_f64()));
        assert!((both.value_at(0.0) - floor).abs() < 1e-9);
        assert!((both.value_at(4.0) - floor).abs() < 1e-9);
        assert!(
            (both.value_at(2.0) - -3.0).abs() < 1e-9,
            "full level between"
        );

        // Running one twice over the same span reshapes it rather than
        // stacking a second curve on top.
        let again = both
            .with_fade(rat(3, 1), rat(4, 1), FadeShape::Linear, floor, false)
            .expect("a reshape");
        let Animation::Keyframed(keys) = &again.animation else {
            panic!("keyframed");
        };
        assert_eq!(keys.len(), 4, "still four keys, not six");

        // Refusals: no span, and an expression (a fade would rewrite what
        // somebody typed).
        assert!(level
            .with_fade(rat(1, 1), rat(1, 1), FadeShape::Ease, floor, true)
            .is_none());
        let expression = Property {
            animation: Animation::Expression("time".into()),
            extra: serde_json::Map::new(),
        };
        assert!(expression
            .with_fade(rat(0, 1), rat(1, 1), FadeShape::Ease, floor, true)
            .is_none());
    }

    #[test]
    fn evaluate_speed_is_the_exact_derivative() {
        // Linear span 0→10 over 1 s: constant slope 10; flat/held outside.
        let lin = [
            key(rat(0, 1), 0.0, SideInterp::Linear),
            key(rat(1, 1), 10.0, SideInterp::Linear),
        ];
        assert!((evaluate_speed(&lin, 0.5).unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(evaluate_speed(&lin, -1.0), Some(0.0));
        assert_eq!(evaluate_speed(&lin, 2.0), Some(0.0));

        // Easy-ease span: speed ~0 at the keys, positive in the middle, and the
        // analytic derivative agrees with a central finite difference of value.
        let ease = [
            key(rat(0, 1), 0.0, EASY_EASE),
            key(rat(1, 1), 10.0, EASY_EASE),
        ];
        let (start, middle) = (
            evaluate_speed(&ease, 0.02).unwrap(),
            evaluate_speed(&ease, 0.5).unwrap(),
        );
        assert!((0.0..2.0).contains(&start) && start < middle * 0.2); // slow start
        assert!((middle - 15.0).abs() < 1e-6); // fast middle (peak of easy-ease)
        for &t in &[0.15_f64, 0.4, 0.6, 0.85] {
            let h = 1e-4;
            let fd =
                (evaluate(&ease, t + h).unwrap() - evaluate(&ease, t - h).unwrap()) / (2.0 * h);
            assert!(
                (evaluate_speed(&ease, t).unwrap() - fd).abs() < 1e-3,
                "at t={t}: analytic {} vs finite {fd}",
                evaluate_speed(&ease, t).unwrap()
            );
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
    fn hold_key_steps_at_the_next_key_not_before() {
        // A hold key keeps its exact value across the whole span, then the
        // value jumps to the next key at that key's time — the discrete/stepped
        // behaviour a File param relies on.
        let keys = [
            key(rat(0, 1), 3.0, SideInterp::Linear).to_hold(),
            key(rat(2, 1), 9.0, SideInterp::Linear).to_hold(),
            key(rat(4, 1), 1.0, SideInterp::Linear),
        ];
        assert!(keys[0].is_hold());
        assert_eq!(evaluate(&keys, 0.0), Some(3.0)); // at the key
        assert_eq!(evaluate(&keys, 1.999), Some(3.0)); // still held just before
        assert_eq!(evaluate(&keys, 2.0), Some(9.0)); // steps exactly at the key
        assert_eq!(evaluate(&keys, 3.5), Some(9.0)); // held across the next span
        assert_eq!(evaluate(&keys, 4.0), Some(1.0)); // and again at the last key
                                                     // A hold span has zero speed throughout (no blend to differentiate).
        assert_eq!(evaluate_speed(&keys, 1.0), Some(0.0));
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

    /// **A key inserted mid-span must not move the curve**. Sampled
    /// densely across the whole span, before and after, and compared: the razor
    /// puts one of these on every retimed layer it cuts, and a cut that changed
    /// the speed ramp would be a cut nobody could trust.
    #[test]
    fn inserting_a_key_preserves_the_curve() {
        for (out_side, in_side) in [
            (EASY_EASE, EASY_EASE),
            (
                SideInterp::Bezier {
                    speed: 4.0,
                    influence: 0.75,
                },
                SideInterp::Bezier {
                    speed: -2.0,
                    influence: 0.1,
                },
            ),
            (SideInterp::Linear, EASY_EASE),
            (SideInterp::Linear, SideInterp::Linear),
        ] {
            let mut p = Property {
                animation: Animation::Keyframed(vec![
                    Keyframe {
                        time: rat(0, 1),
                        value: 0.0,
                        interp_in: SideInterp::Linear,
                        interp_out: out_side,
                    },
                    Keyframe {
                        time: rat(2, 1),
                        value: 10.0,
                        interp_in: in_side,
                        interp_out: SideInterp::Linear,
                    },
                ]),
                extra: serde_json::Map::new(),
            };
            let before: Vec<f64> = (0..=200)
                .map(|i| p.value_at(f64::from(i) / 100.0))
                .collect();

            assert!(p.insert_key_preserving_shape(rat(3, 4)), "a key landed");
            let Animation::Keyframed(keys) = &p.animation else {
                panic!("still keyframed");
            };
            assert_eq!(keys.len(), 3, "one key added, in the middle");
            assert!((keys[1].time.to_f64() - 0.75).abs() < 1e-12);

            for (i, was) in before.iter().enumerate() {
                let t = f64::from(i as u32) / 100.0;
                let now = p.value_at(t);
                assert!(
                    (now - was).abs() < 1e-6,
                    "at t={t} the curve moved: {was} -> {now}"
                );
            }
        }
    }

    #[test]
    fn a_key_on_a_held_span_holds_and_one_on_an_existing_key_is_refused() {
        let mut p = Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: rat(0, 1),
                    value: 3.0,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Hold,
                },
                Keyframe {
                    time: rat(2, 1),
                    value: 9.0,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        };
        assert!(p.insert_key_preserving_shape(rat(1, 1)));
        assert_eq!(p.value_at(0.5), 3.0);
        assert_eq!(p.value_at(1.5), 3.0, "the hold still holds");

        // A second key at the same time would be two keys at one moment, which
        // the ops layer forbids and the evaluator cannot read.
        assert!(!p.insert_key_preserving_shape(rat(1, 1)));
    }

    #[test]
    fn a_key_outside_the_keyed_range_takes_the_held_end_value() {
        let mut p = Property {
            animation: Animation::Keyframed(vec![Keyframe {
                time: rat(1, 1),
                value: 5.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            }]),
            extra: serde_json::Map::new(),
        };
        assert!(p.insert_key_preserving_shape(rat(3, 1)));
        assert_eq!(p.value_at(2.0), 5.0);
        assert_eq!(p.value_at(9.0), 5.0);

        // And a static property has no curve to keep, so nothing happens.
        let mut fixed = Property::fixed(2.0);
        assert!(!fixed.insert_key_preserving_shape(rat(1, 1)));
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

    /// The same budget on a *dense* curve — an imported AE camera arrives with
    /// a key per frame, thousands per property. The linear span scan this
    /// replaced made each sample cost the whole list (~2,400 keys × 7 camera
    /// properties per rendered frame on the real project that found it), so a
    /// sparse-only bench held while dense curves missed the gate by orders of
    /// magnitude. Fails in minutes rather than milliseconds without bisection.
    #[test]
    fn million_evaluations_stay_cheap_on_a_dense_curve() {
        let keys: Vec<Keyframe> = (0..2_400)
            .map(|i| key(rat(i, 25), (i % 7) as f64, SideInterp::Linear))
            .collect();
        let start = std::time::Instant::now();
        let mut acc = 0.0;
        for i in 0..1_000_000u32 {
            acc += evaluate(&keys, f64::from(i % 2_399) / 25.0 + 0.017).unwrap_or(0.0);
        }
        let elapsed = start.elapsed();
        assert!(acc.is_finite());
        // The same 40× debug headroom as the sparse bench above.
        assert!(elapsed.as_millis() < 800, "1M dense evals took {elapsed:?}");
    }

    /// Bisection answers exactly what the linear walk answered, spans and
    /// speeds both — sampled between keys, exactly on interior keys, and at
    /// both ends — on a curve dense enough that an off-by-one lands on a
    /// different span with a different value.
    #[test]
    fn span_bisection_matches_the_linear_walk() {
        let sides = [
            SideInterp::Linear,
            SideInterp::Hold,
            EASY_EASE,
            SideInterp::Bezier {
                speed: 3.0,
                influence: 0.25,
            },
        ];
        let keys: Vec<Keyframe> = (0..500)
            .map(|i| key(rat(i, 10), ((i * 37) % 11) as f64, sides[(i % 4) as usize]))
            .collect();
        let linear_idx = |t: f64| {
            keys.windows(2)
                .position(|w| t < w[1].time.to_f64())
                .unwrap_or(keys.len() - 2)
        };
        for i in 0..4_990 {
            for t in [f64::from(i) / 100.0, f64::from(i) / 100.0 + 0.003] {
                if t <= keys[0].time.to_f64() || t >= keys[keys.len() - 1].time.to_f64() {
                    continue;
                }
                assert_eq!(
                    span_index(&keys, t),
                    linear_idx(t),
                    "span for t = {t} diverged"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tangent modes — Auto / Clamp / Free (docs/impl/keyframe-eval.md §6).
    // -----------------------------------------------------------------------

    fn auto(clamped: bool) -> SideInterp {
        SideInterp::Auto {
            clamped,
            speed: 0.0,
            influence: 1.0 / 3.0,
        }
    }

    /// Three keys 0, 10, 30 at one second apart: the automatic tangent at the
    /// middle one aims from the first to the last — (30 − 0) / 2 s = 15.
    #[test]
    fn an_automatic_tangent_aims_between_the_neighbours() {
        let keys = [
            key(rat(0, 1), 0.0, auto(false)),
            key(rat(1, 1), 10.0, auto(false)),
            key(rat(2, 1), 30.0, auto(false)),
        ];
        assert!((auto_speed(&keys, 1, false) - 15.0).abs() < 1e-12);
        // An end key has no pair to aim between, so it lies flat.
        assert_eq!(auto_speed(&keys, 0, false), 0.0);
        assert_eq!(auto_speed(&keys, 2, false), 0.0);
    }

    /// Auto's smooth slope is what the evaluator uses: the resolved side is a
    /// bezier at that speed, keeping the side's own influence.
    #[test]
    fn an_automatic_side_resolves_to_the_bezier_its_neighbours_dictate() {
        let keys = [
            key(rat(0, 1), 0.0, auto(false)),
            key(
                rat(1, 1),
                10.0,
                SideInterp::Auto {
                    clamped: false,
                    speed: 99.0, // remembered ease, never evaluated
                    influence: 0.5,
                },
            ),
            key(rat(2, 1), 30.0, auto(false)),
        ];
        assert_eq!(
            resolved_side(&keys, 1, true),
            SideInterp::Bezier {
                speed: 15.0,
                influence: 0.5
            }
        );
    }

    /// **Auto recomputes whenever a neighbour moves.** The same key, the same
    /// mode, a moved neighbour — a different tangent, and so a different
    /// curve. A free side would have gone on reading the speed it stored.
    #[test]
    fn an_automatic_tangent_follows_a_moved_neighbour() {
        let mut keys = [
            key(rat(0, 1), 0.0, auto(false)),
            key(rat(1, 1), 10.0, auto(false)),
            key(rat(2, 1), 30.0, auto(false)),
        ];
        let before = evaluate(&keys, 1.5).unwrap();
        keys[2].value = 200.0;
        let after = evaluate(&keys, 1.5).unwrap();
        assert!((auto_speed(&keys, 1, false) - 100.0).abs() < 1e-12);
        assert!((after - before).abs() > 1.0);

        // With both sides free the stored speed is all there is, so moving the
        // neighbour's *value* changes the span's ends but not the tangent.
        let free = SideInterp::Bezier {
            speed: 15.0,
            influence: 1.0 / 3.0,
        };
        let frozen = [
            key(rat(0, 1), 0.0, free),
            key(rat(1, 1), 10.0, free),
            key(rat(2, 1), 200.0, free),
        ];
        assert!((side_params(resolved_side(&frozen, 1, true), 0.0).0 - 15.0).abs() < 1e-12);
    }

    /// **Clamp is Auto with no overshoot.** A peak key — both neighbours
    /// below it — gets a flat tangent, so no sample in either span rises above
    /// it; the smooth tangent tilts and does overshoot.
    #[test]
    fn a_clamped_tangent_does_not_overshoot_its_neighbours() {
        let peak = |side: SideInterp| {
            [
                key(rat(0, 1), 0.0, SideInterp::Linear),
                key(rat(1, 1), 10.0, side),
                key(rat(2, 1), 9.0, SideInterp::Linear),
            ]
        };
        let clamped = peak(auto(true));
        assert_eq!(auto_speed(&clamped, 1, true), 0.0);
        for i in 0..=100 {
            let t = i as f64 / 50.0;
            assert!(
                evaluate(&clamped, t).unwrap() <= 10.0 + 1e-9,
                "clamped overshot at t={t}"
            );
        }
        // The unclamped one aims from 0 to 9 across two seconds — uphill past
        // the peak — and does rise above it.
        let smooth = peak(auto(false));
        assert!((auto_speed(&smooth, 1, false) - 4.5).abs() < 1e-12);
        let over = (0..=100)
            .map(|i| evaluate(&smooth, i as f64 / 50.0).unwrap())
            .fold(f64::MIN, f64::max);
        assert!(over > 10.0 + 1e-6, "smooth tangent should overshoot");
    }

    /// A clamped tangent that is *not* at a peak keeps the smooth aim, held to
    /// three times the gentler chord (the monotone bound).
    #[test]
    fn a_clamped_tangent_keeps_the_smooth_aim_where_it_is_safe() {
        let keys = [
            key(rat(0, 1), 0.0, auto(true)),
            key(rat(1, 1), 10.0, auto(true)),
            key(rat(2, 1), 30.0, auto(true)),
        ];
        assert!((auto_speed(&keys, 1, true) - 15.0).abs() < 1e-12);

        // A near-flat step followed by a leap: the smooth aim is steeper than
        // three times the gentler chord (0.1), so it is held to 0.3.
        let steep = [
            key(rat(0, 1), 0.0, auto(true)),
            key(rat(1, 1), 0.1, auto(true)),
            key(rat(2, 1), 100.0, auto(true)),
        ];
        assert!((auto_speed(&steep, 1, true) - 0.3).abs() < 1e-12);
    }

    /// **Free → Auto → Free keeps the custom ease** (the study's explicit
    /// bar). The mode switch files the ease inside the automatic side and
    /// hands it back untouched.
    #[test]
    fn a_round_trip_through_auto_keeps_the_custom_ease() {
        let custom = SideInterp::Bezier {
            speed: 7.25,
            influence: 0.82,
        };
        assert_eq!(custom.tangent_mode(), TangentMode::Free);
        let automatic = custom.with_tangent_mode(TangentMode::Auto);
        assert_eq!(automatic.tangent_mode(), TangentMode::Auto);
        let clamped = automatic.with_tangent_mode(TangentMode::Clamp);
        assert_eq!(clamped.tangent_mode(), TangentMode::Clamp);
        assert_eq!(clamped.with_tangent_mode(TangentMode::Free), custom);

        // A straight side has no ease of its own to keep; it returns eased,
        // which is what it has been while automatic.
        let straight = SideInterp::Linear.with_tangent_mode(TangentMode::Auto);
        assert_eq!(
            straight.with_tangent_mode(TangentMode::Free),
            SideInterp::Bezier {
                speed: 0.0,
                influence: 1.0 / 3.0
            }
        );
        // Free asked of an already-free side changes nothing at all.
        assert_eq!(
            SideInterp::Hold.with_tangent_mode(TangentMode::Free),
            SideInterp::Hold
        );
    }

    /// An automatic side survives the file format, and a key carrying one is
    /// an eased key as far as the interface is concerned.
    #[test]
    fn an_automatic_side_serialises_and_counts_as_eased() {
        let k = key(rat(1, 2), 5.0, auto(true));
        let text = serde_json::to_string(&k).unwrap();
        assert_eq!(serde_json::from_str::<Keyframe>(&text).unwrap(), k);
        assert!(k.is_bezier());
        assert!(!k.is_hold());
    }

    /// Planting a key inside a span leaves an automatic neighbour automatic —
    /// its tangent is a function of its neighbours, and one has just changed.
    #[test]
    fn inserting_a_key_leaves_an_automatic_neighbour_automatic() {
        let mut property = Property {
            animation: Animation::Keyframed(vec![
                key(rat(0, 1), 0.0, auto(false)),
                key(rat(1, 1), 10.0, auto(false)),
                key(rat(2, 1), 30.0, auto(false)),
            ]),
            extra: serde_json::Map::new(),
        };
        assert!(property.insert_key_preserving_shape(rat(1, 2)));
        let Animation::Keyframed(keys) = &property.animation else {
            panic!("still keyframed");
        };
        assert_eq!(keys.len(), 4);
        assert!(matches!(keys[0].interp_out, SideInterp::Auto { .. }));
        assert!(matches!(keys[2].interp_in, SideInterp::Auto { .. }));
        // The new key is an ordinary free one, taking the shape of the split.
        assert!(matches!(keys[1].interp_out, SideInterp::Bezier { .. }));
    }
}

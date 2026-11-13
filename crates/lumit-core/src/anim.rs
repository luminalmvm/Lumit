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
    /// AE-compatible: speed in value-units/second, influence a fraction of
    /// the span from [`MIN_INFLUENCE`] to [`MAX_INFLUENCE`].
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

/// The shortest a handle may reach, as a fraction of its span.
pub const MIN_INFLUENCE: f64 = 1e-3;

/// The furthest a handle may reach, as a fraction of its span: 1000%.
pub const MAX_INFLUENCE: f64 = 10.0;

/// The influences a span's cubic is built from, given the two its sides store.
///
/// A handle may reach past the neighbouring key, but time must never run
/// backwards along the span. That holds exactly when `a + b - sqrt(a * b)` is
/// at most 1. Inside the old box, both at most 1, it always holds, so those
/// pairs come back untouched and every older curve stays bit-identical. A pair
/// that reaches too far is scaled down in proportion onto that edge. The
/// stored handles stay where they were put.
#[must_use]
pub fn span_influences(a: f64, b: f64) -> (f64, f64) {
    let a = a.clamp(MIN_INFLUENCE, MAX_INFLUENCE);
    let b = b.clamp(MIN_INFLUENCE, MAX_INFLUENCE);
    if a <= 1.0 && b <= 1.0 {
        return (a, b);
    }
    let f = a + b - (a * b).sqrt();
    if f <= 1.0 {
        return (a, b);
    }
    (a / f, b / f)
}

/// One side of a key eased as After Effects describes it: a speed in the
/// property's units a second, and an influence as a per cent of the span,
/// 0.1 to 100. Lumit's influence is the same as a fraction. Either left out
/// is the After Effects default, no speed and a third of the span.
#[must_use]
pub fn ae_ease(speed: Option<f64>, influence: Option<f64>) -> SideInterp {
    SideInterp::Bezier {
        speed: speed.unwrap_or(0.0),
        influence: (influence.unwrap_or(100.0 / 3.0) / 100.0).clamp(1e-3, 1.0),
    }
}

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
    let (b1, b2) = span_influences(b1, b2);
    CubicSpan::from_ae(t1, a.value, t2, b.value, s1, b1, s2, b2)
}

/// A side's (speed, influence). Takes a side that has already been through
/// [`resolved_side`], so an automatic one never reaches here as itself; the
/// fallback treats one that does as straight, which is the calmest wrong
/// answer rather than a panic in an engine crate. The influence is as stored,
/// and [`span_influences`] bounds it.
fn side_params(side: SideInterp, chord_slope: f64) -> (f64, f64) {
    match side {
        SideInterp::Bezier { speed, influence } => (speed, influence),
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
    /// Influence is clamped to [`MIN_INFLUENCE`] and [`MAX_INFLUENCE`], so a
    /// handle that came out of a split at a very small parameter still
    /// describes a side the evaluator will accept. Half of a split span can
    /// need more than 1, so the planted key keeps the curve exactly.
    #[must_use]
    pub fn out_side(&self) -> SideInterp {
        let dt = self.x[3] - self.x[0];
        let dx = self.x[1] - self.x[0];
        if dt <= 0.0 || dx.abs() < 1e-12 {
            return SideInterp::Bezier {
                speed: 0.0,
                influence: MIN_INFLUENCE,
            };
        }
        SideInterp::Bezier {
            speed: (self.y[1] - self.y[0]) / dx,
            influence: (dx / dt).clamp(MIN_INFLUENCE, MAX_INFLUENCE),
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
                influence: MIN_INFLUENCE,
            };
        }
        SideInterp::Bezier {
            speed: (self.y[3] - self.y[2]) / dx,
            influence: (dx / dt).clamp(MIN_INFLUENCE, MAX_INFLUENCE),
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
            Animation::Expression(expression) => {
                crate::expression::evaluate_property(expression, None, &self.extra)
            }
        }
    }

    pub fn value_at_with_context(&self, t: f64, context: Arc<ExpressionContext>) -> f64 {
        match &self.animation {
            Animation::Static(v) => *v,
            Animation::Keyframed(keys) => evaluate(keys, t).unwrap_or(0.0),
            Animation::Expression(expression) => {
                crate::expression::evaluate_property(expression, Some(context), &self.extra)
            }
        }
    }

    /// Change what drives this property, and keep the note an expression reads
    /// its own value from ([`crate::expression::script::Slot`]) in step with it.
    ///
    /// **In plain terms.** `value` in an expression means what the property
    /// held before the expression, and `wiggle` wanders around that. So the
    /// moment an expression is put on a property is the moment to write the
    /// number down. Editing the text afterwards leaves the note alone, and
    /// taking the expression off removes it.
    pub fn set_animation(&mut self, animation: Animation) {
        use crate::expression::{script::Slot, script::SLOT_KEY};
        match (&self.animation, &animation) {
            (Animation::Expression(_), Animation::Expression(_)) => {}
            (_, Animation::Expression(_)) => {
                // A seed of its own, so two properties given the same text
                // do not wander in step.
                let seed = (uuid::Uuid::now_v7().as_u128() as u32).max(1);
                Slot::new(&[self.value_at(0.0)], 0, seed).write(&mut self.extra);
            }
            _ => {
                self.extra.remove(SLOT_KEY);
            }
        }
        self.animation = animation;
    }

    /// The language this property's expression is written in. Rhai when
    /// nothing says otherwise.
    #[must_use]
    pub fn expression_language(&self) -> crate::expression::script::Language {
        crate::expression::script::Language::of(&self.extra)
    }

    /// Say which language this property's expression is written in.
    pub fn set_expression_language(&mut self, language: crate::expression::script::Language) {
        use crate::expression::script::Slot;
        Slot::read(&self.extra)
            .in_language(language)
            .write(&mut self.extra);
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
mod tests;

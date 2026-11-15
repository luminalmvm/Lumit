//! Keys: a Lumit property's keyframes as an output key list, with the easing
//! kept exactly.
//!
//! # In plain terms
//!
//! Lumit eases a key the way After Effects does, with a speed and an
//! influence on each side. Under that is one cubic bezier per span, value
//! against time. A Lottie key (and an SVG `keySplines` entry) is the same
//! cubic, drawn in a unit square: the span runs from (0, 0) to (1, 1) and the
//! two handles are points inside it. So nothing is approximated: each span's
//! four control points are divided by the span's own width and height and
//! written as they are.
//!
//! Two cases need care.
//!
//! - **A span that starts and ends on the same value but moves in between**
//!   (a bump: out of 0 and back to 0 with speed on the way). The unit square
//!   has no height, so the handles can't be written. The span is cut at its
//!   peak with [`CubicSpan::split_at`], which is exact, so each piece rises or
//!   falls and is written normally.
//! - **A value with several numbers** (a scale's x and y, a gradient's two
//!   points). A Lottie key carries every number at one time, so the keys of
//!   all the numbers are merged: wherever one number has a key the others are
//!   cut there too, again exactly. Lottie holds a whole key, so when one number
//!   holds while another moves the merge gives up and the caller samples
//!   instead.
//!
//! Runs on the export's thread; pure maths, no allocation per frame.

use lumit_core::anim::{resolved_side, CubicSpan, Keyframe, SideInterp};

/// One dimension's curve, exactly: the spans between its keys, and its value
/// before the first key and after the last.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub spans: Vec<Span>,
    pub first: f64,
    pub last: f64,
}

/// One span of a curve, in seconds of its owner's clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Span {
    /// The value stays at `v` from `t0` until `t1`, then steps to the next.
    Hold { t0: f64, t1: f64, v: f64 },
    /// Control points: time in `x`, value in `y`.
    Cubic { x: [f64; 4], y: [f64; 4] },
}

impl Span {
    fn t0(&self) -> f64 {
        match self {
            Span::Hold { t0, .. } => *t0,
            Span::Cubic { x, .. } => x[0],
        }
    }

    fn t1(&self) -> f64 {
        match self {
            Span::Hold { t1, .. } => *t1,
            Span::Cubic { x, .. } => x[3],
        }
    }

    fn start_value(&self) -> f64 {
        match self {
            Span::Hold { v, .. } => *v,
            Span::Cubic { y, .. } => y[0],
        }
    }

    /// The two pieces this span is at time `t` strictly inside it.
    fn split(&self, t: f64) -> (Span, Span) {
        match *self {
            Span::Hold { t0, t1, v } => (Span::Hold { t0, t1: t, v }, Span::Hold { t0: t, t1, v }),
            Span::Cubic { x, y } => {
                let cubic = CubicSpan::from_points(x, y);
                let (l, r) = cubic.split_at(cubic.solve_u(t));
                let ((lx, ly), (rx, ry)) = (l.control_points(), r.control_points());
                // The cut lands exactly on `t`, whatever the solve's last bit,
                // so neighbouring spans share their ends.
                let (mut lx, mut rx) = (lx, rx);
                lx[3] = t;
                rx[0] = t;
                (Span::Cubic { x: lx, y: ly }, Span::Cubic { x: rx, y: ry })
            }
        }
    }

    /// True when the value does not move across the span at all.
    fn is_constant(&self) -> bool {
        match self {
            Span::Hold { .. } => true,
            Span::Cubic { y, .. } => {
                let tol = tolerance(y);
                (y[1] - y[0]).abs() <= tol
                    && (y[2] - y[0]).abs() <= tol
                    && (y[3] - y[0]).abs() <= tol
            }
        }
    }
}

/// How close two values are taken as equal, for a span from `y`.
fn tolerance(y: &[f64; 4]) -> f64 {
    1e-9 * (1.0 + y[0].abs() + y[3].abs())
}

/// The curve Lumit evaluates for `keys`: the same spans `anim::evaluate`
/// walks, each side resolved the same way.
#[must_use]
pub fn curve(keys: &[Keyframe]) -> Curve {
    let first = keys.first().map_or(0.0, |k| k.value);
    let last = keys.last().map_or(first, |k| k.value);
    let mut spans = Vec::with_capacity(keys.len().saturating_sub(1));
    for (i, pair) in keys.windows(2).enumerate() {
        let [a, b] = pair else { continue };
        let (t1, t2) = (a.time.to_f64(), b.time.to_f64());
        let dt = t2 - t1;
        if dt.is_nan() || dt <= 0.0 {
            continue;
        }
        if matches!(a.interp_out, SideInterp::Hold) {
            spans.push(Span::Hold {
                t0: t1,
                t1: t2,
                v: a.value,
            });
            continue;
        }
        // A straight side is a bezier whose handle lies on the chord, as the
        // evaluator has it: speed is the chord's slope, influence a third.
        let chord = (b.value - a.value) / dt;
        let (s1, i1) = side(resolved_side(keys, i, true), chord);
        let (s2, i2) = side(resolved_side(keys, i + 1, false), chord);
        let (x, y) = CubicSpan::from_ae(t1, a.value, t2, b.value, s1, i1, s2, i2).control_points();
        spans.extend(unbumped(x, y));
    }
    Curve { spans, first, last }
}

/// A side's speed and influence as `anim::side_params` reads it.
fn side(side: SideInterp, chord: f64) -> (f64, f64) {
    match side {
        SideInterp::Bezier { speed, influence } => (speed, influence.clamp(1e-3, 1.0)),
        _ => (chord, 1.0 / 3.0),
    }
}

/// The span `x`, `y` cut at each turning point of its value when its unit
/// square would have no height (a bump), or when its handles would stand a
/// thousand times taller than the span itself.
fn unbumped(x: [f64; 4], y: [f64; 4]) -> Vec<Span> {
    let dy = y[3] - y[0];
    let reach = (y[1] - y[0]).abs().max((y[2] - y[0]).abs());
    let flat = dy.abs() <= tolerance(&y);
    if !(flat && reach > tolerance(&y)) && !(reach > 1e3 * dy.abs() && !flat) {
        return vec![Span::Cubic { x, y }];
    }
    // y'(u)/3 = a·u² + b·u + c over the three handle deltas.
    let (p0, p1, p2) = (y[1] - y[0], y[2] - y[1], y[3] - y[2]);
    let (a, b, c) = (p0 - 2.0 * p1 + p2, 2.0 * (p1 - p0), p0);
    let mut roots: Vec<f64> = Vec::with_capacity(2);
    if a.abs() < 1e-15 {
        if b.abs() > 1e-15 {
            roots.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let s = disc.sqrt();
            roots.push((-b - s) / (2.0 * a));
            roots.push((-b + s) / (2.0 * a));
        }
    }
    roots.retain(|u| *u > 1e-6 && *u < 1.0 - 1e-6);
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let mut out = Vec::with_capacity(roots.len() + 1);
    let mut rest = CubicSpan::from_points(x, y);
    let mut used = 0.0;
    for u in roots {
        // `u` is on the whole span; the rest starts at `used`.
        let local = (u - used) / (1.0 - used);
        let (l, r) = rest.split_at(local);
        let (lx, ly) = l.control_points();
        out.push(Span::Cubic { x: lx, y: ly });
        rest = r;
        used = u;
    }
    let (rx, ry) = rest.control_points();
    out.push(Span::Cubic { x: rx, y: ry });
    out
}

/// One key of an output track: its time, its numbers, and the span that
/// leaves it.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackKey {
    /// Seconds on the property owner's clock (a layer's own time).
    pub seconds: f64,
    pub value: Vec<f64>,
    /// The whole key holds until the next.
    pub hold: bool,
    /// Per dimension, the handle leaving this key in the unit square
    /// (Lottie `o`, the first two numbers of a `keySplines` entry). Empty on a
    /// hold and on the last key.
    pub out: Vec<[f64; 2]>,
    /// Per dimension, the handle arriving at the next key (Lottie `i`).
    pub inn: Vec<[f64; 2]>,
}

/// A property as output keys: one or more, the values between them eased
/// exactly as Lumit eases them.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub keys: Vec<TrackKey>,
}

/// The straight line's handles, a third and two thirds along the diagonal.
pub const LINEAR_OUT: [f64; 2] = [1.0 / 3.0, 1.0 / 3.0];
pub const LINEAR_IN: [f64; 2] = [2.0 / 3.0, 2.0 / 3.0];

impl Track {
    /// A track holding `value` throughout.
    #[must_use]
    pub fn still(value: Vec<f64>) -> Track {
        Track {
            keys: vec![TrackKey {
                seconds: 0.0,
                value,
                hold: false,
                out: Vec::new(),
                inn: Vec::new(),
            }],
        }
    }

    /// True when every key holds the first one's value, so the track can be
    /// written as a still value.
    #[must_use]
    pub fn is_still(&self) -> bool {
        let Some(first) = self.keys.first() else {
            return true;
        };
        self.keys.iter().all(|k| {
            k.value.len() == first.value.len()
                && k.value
                    .iter()
                    .zip(&first.value)
                    .all(|(a, b)| (a - b).abs() <= 1e-9 * (1.0 + b.abs()))
        })
    }

    /// The first key's value.
    #[must_use]
    pub fn first_value(&self) -> Vec<f64> {
        self.keys
            .first()
            .map(|k| k.value.clone())
            .unwrap_or_default()
    }

    /// Every value put through `f(dimension, value)`. Only for a map of the
    /// form `a·v + b` with `a ≠ 0` (adding the art's shift to an anchor,
    /// turning a fraction into a per cent): the handles in the unit square
    /// don't change under one, so the easing stays exact.
    #[must_use]
    pub fn map_affine(mut self, f: impl Fn(usize, f64) -> f64) -> Track {
        for key in &mut self.keys {
            for (d, v) in key.value.iter_mut().enumerate() {
                *v = f(d, *v);
            }
        }
        self
    }

    /// Every key's time put through `f`, which must keep their order (a
    /// shift, or a change of clock).
    #[must_use]
    pub fn map_time(mut self, f: impl Fn(f64) -> f64) -> Track {
        for key in &mut self.keys {
            key.seconds = f(key.seconds);
        }
        self
    }

    /// The value at `seconds`, evaluated the way a player does: each span's
    /// handles as a unit-square bezier per dimension.
    #[must_use]
    pub fn value_at(&self, seconds: f64) -> Vec<f64> {
        let Some(first) = self.keys.first() else {
            return Vec::new();
        };
        if seconds <= first.seconds || self.keys.len() == 1 {
            return first.value.clone();
        }
        let p = self.keys.partition_point(|k| k.seconds <= seconds);
        let (Some(a), Some(b)) = (self.keys.get(p.wrapping_sub(1)), self.keys.get(p)) else {
            return self
                .keys
                .last()
                .map(|k| k.value.clone())
                .unwrap_or_default();
        };
        if a.hold {
            return a.value.clone();
        }
        let s = (seconds - a.seconds) / (b.seconds - a.seconds);
        (0..a.value.len())
            .map(|d| {
                let (o, i) = (
                    a.out.get(d).copied().unwrap_or(LINEAR_OUT),
                    a.inn.get(d).copied().unwrap_or(LINEAR_IN),
                );
                let k = CubicSpan::from_points([0.0, o[0], i[0], 1.0], [0.0, o[1], i[1], 1.0]);
                let e = k.value_at(s);
                let (v0, v1) = (
                    a.value.get(d).copied().unwrap_or(0.0),
                    b.value.get(d).copied().unwrap_or(0.0),
                );
                v0 + (v1 - v0) * e
            })
            .collect()
    }
}

/// Why several numbers' keys could not be merged into one track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmergeable;

/// The curves of a value's numbers as one track, every number keyed wherever
/// any of them is. `Err` when, across some span, one number holds while
/// another moves: a Lottie key holds all its numbers or none, so the caller
/// samples that value instead.
pub fn track(curves: &[Curve]) -> Result<Track, Unmergeable> {
    // Every time any number has a key, in order.
    let mut times: Vec<f64> = curves
        .iter()
        .flat_map(|c| c.spans.iter().flat_map(|s| [s.t0(), s.t1()]))
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() <= 1e-9 * (1.0 + b.abs()));
    if times.len() < 2 {
        let value: Vec<f64> = curves.iter().map(|c| c.first).collect();
        let mut still = Track::still(value);
        if let (Some(t), Some(key)) = (times.first(), still.keys.first_mut()) {
            key.seconds = *t;
        }
        return Ok(still);
    }
    // Each number's spans cut at every one of those times.
    let cut: Vec<Vec<Span>> = curves.iter().map(|c| on_grid(c, &times)).collect();
    let mut keys = Vec::with_capacity(times.len());
    for (k, t) in times.iter().enumerate() {
        let last = k + 1 == times.len();
        let spans: Vec<Option<&Span>> = cut.iter().map(|c| c.get(k)).collect();
        let value: Vec<f64> = spans
            .iter()
            .zip(curves)
            .map(|(s, c)| match s {
                Some(s) if !last => s.start_value(),
                _ => c.last,
            })
            .collect();
        if last {
            keys.push(TrackKey {
                seconds: *t,
                value,
                hold: false,
                out: Vec::new(),
                inn: Vec::new(),
            });
            continue;
        }
        let holds = spans.iter().any(|s| matches!(s, Some(Span::Hold { .. })));
        if holds {
            // Every other number has to sit still across this span, or the
            // key can't hold for all of them.
            if spans.iter().any(|s| s.is_some_and(|s| !s.is_constant())) {
                return Err(Unmergeable);
            }
            keys.push(TrackKey {
                seconds: *t,
                value,
                hold: true,
                out: Vec::new(),
                inn: Vec::new(),
            });
            continue;
        }
        // A number that stands still across the span draws the same whatever
        // its handles are, so it borrows those of the first number that moves.
        // Then a point with one still number reads as one easing, which every
        // player draws, rather than one easing per number.
        let moving = spans
            .iter()
            .find_map(|s| match s {
                Some(span @ Span::Cubic { x, y }) if !span.is_constant() => Some(handles(x, y)),
                _ => None,
            })
            .unwrap_or((LINEAR_OUT, LINEAR_IN));
        let (out, inn) = spans
            .iter()
            .map(|s| match s {
                Some(span @ Span::Cubic { x, y }) if !span.is_constant() => handles(x, y),
                _ => moving,
            })
            .unzip();
        keys.push(TrackKey {
            seconds: *t,
            value,
            hold: false,
            out,
            inn,
        });
    }
    Ok(Track { keys })
}

/// A cubic span's handles in its unit square. A span that doesn't move gets
/// the straight line's, which a player draws as standing still.
fn handles(x: &[f64; 4], y: &[f64; 4]) -> ([f64; 2], [f64; 2]) {
    let (dx, dy) = (x[3] - x[0], y[3] - y[0]);
    if dx <= 0.0 || dy.abs() <= tolerance(y) {
        return (LINEAR_OUT, LINEAR_IN);
    }
    (
        [(x[1] - x[0]) / dx, (y[1] - y[0]) / dy],
        [(x[2] - x[0]) / dx, (y[2] - y[0]) / dy],
    )
}

/// `curve`'s spans laid on `times`: one span per gap, cut wherever a time
/// falls inside one, and a still span wherever the curve has no key (before
/// its first, after its last).
fn on_grid(curve: &Curve, times: &[f64]) -> Vec<Span> {
    let mut out = Vec::with_capacity(times.len());
    let mut spans = curve.spans.iter().copied().peekable();
    let mut pending: Option<Span> = None;
    for gap in times.windows(2) {
        let [a, b] = gap else { continue };
        let (a, b) = (*a, *b);
        let near = |p: f64, q: f64| (p - q).abs() <= 1e-9 * (1.0 + q.abs());
        // Take the span that covers `a`, carrying any remainder of a cut one.
        let current = match pending.take() {
            Some(s) => Some(s),
            None => {
                while spans
                    .peek()
                    .is_some_and(|s| s.t1() <= a && !near(s.t1(), b))
                {
                    spans.next();
                }
                match spans.peek() {
                    Some(s) if s.t0() <= a || near(s.t0(), a) => spans.next(),
                    _ => None,
                }
            }
        };
        let Some(span) = current else {
            // Before the first key or after the last: the value stands still.
            let v = if curve.spans.first().is_some_and(|s| b <= s.t0() + 1e-12) {
                curve.first
            } else {
                curve.last
            };
            out.push(Span::Cubic {
                x: [a, a + (b - a) / 3.0, b - (b - a) / 3.0, b],
                y: [v, v, v, v],
            });
            continue;
        };
        if near(span.t1(), b) {
            out.push(span);
        } else {
            let (left, right) = span.split(b);
            out.push(left);
            pending = Some(right);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::{evaluate, Keyframe, SideInterp};
    use lumit_core::time::Rational;

    fn key(t: f64, v: f64, inn: SideInterp, out: SideInterp) -> Keyframe {
        Keyframe {
            time: Rational::new((t * 1000.0).round() as i64, 1000).unwrap(),
            value: v,
            interp_in: inn,
            interp_out: out,
        }
    }

    fn ease(speed: f64, influence: f64) -> SideInterp {
        SideInterp::Bezier { speed, influence }
    }

    /// A small deterministic generator, so the cases are the same every run.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }
    }

    /// The whole promise of this module: written keys, evaluated the way a
    /// player evaluates them, give the value Lumit gives, everywhere, for
    /// eased, straight, held, automatic and bumped spans alike.
    #[test]
    fn written_keys_play_back_exactly_what_lumit_evaluates() {
        let mut rng = Lcg(7);
        for case in 0..400 {
            let n = 2 + (rng.next() * 5.0) as usize;
            let mut t = 0.0;
            let keys: Vec<Keyframe> = (0..n)
                .map(|i| {
                    t += 0.1 + rng.next() * 2.0;
                    // Every fourth case repeats a value, to make bumps.
                    let v = if case % 4 == 0 && i % 2 == 1 {
                        10.0
                    } else {
                        (rng.next() - 0.5) * 400.0
                    };
                    let side = |r: f64, speed: f64, influence: f64| match (r * 6.0) as u32 {
                        0 => SideInterp::Linear,
                        1 => SideInterp::Hold,
                        2 => SideInterp::Auto {
                            clamped: r > 0.4,
                            speed: 0.0,
                            influence: 0.2 + r * 0.5,
                        },
                        _ => ease((speed - 0.5) * 300.0, 0.01 + influence * 0.99),
                    };
                    let r: [f64; 6] = std::array::from_fn(|_| rng.next());
                    let inn = side(r[0], r[1], r[2]);
                    let out = side(r[3], r[4], r[5]);
                    key(
                        t,
                        if case % 4 == 0 && i % 2 == 0 { 10.0 } else { v },
                        inn,
                        out,
                    )
                })
                .collect();
            let track = track(&[curve(&keys)]).expect("one number always merges");
            let (t0, t1) = (keys[0].time.to_f64(), keys[n - 1].time.to_f64());
            for s in 0..=200 {
                let at = t0 - 0.5 + (t1 - t0 + 1.0) * f64::from(s) / 200.0;
                let want = evaluate(&keys, at).unwrap();
                let got = track.value_at(at)[0];
                let scale = 1.0 + want.abs();
                assert!(
                    (want - got).abs() <= 1e-7 * scale,
                    "case {case}: at {at} Lumit {want} written {got}"
                );
            }
        }
    }

    /// Two numbers keyed at different times merge into one track that plays
    /// both back exactly.
    #[test]
    fn numbers_keyed_apart_merge_exactly() {
        let x = vec![
            key(0.0, 0.0, SideInterp::Linear, ease(0.0, 0.33)),
            key(2.0, 100.0, ease(0.0, 0.8), SideInterp::Linear),
        ];
        let y = vec![
            key(0.5, 50.0, SideInterp::Linear, ease(40.0, 0.5)),
            key(1.3, -20.0, ease(-10.0, 0.6), ease(5.0, 0.2)),
            key(3.0, 70.0, SideInterp::Linear, SideInterp::Linear),
        ];
        let merged = track(&[curve(&x), curve(&y)]).unwrap();
        for s in 0..=300 {
            let at = -0.2 + 3.5 * f64::from(s) / 300.0;
            let got = merged.value_at(at);
            assert!((got[0] - evaluate(&x, at).unwrap()).abs() < 1e-7);
            assert!((got[1] - evaluate(&y, at).unwrap()).abs() < 1e-7);
        }
    }
}

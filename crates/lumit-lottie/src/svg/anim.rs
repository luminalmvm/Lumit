//! SMIL timing: a value's keys as `values`, `keyTimes` and `keySplines` over
//! the file's span.
//!
//! # In plain terms
//!
//! Every animation in the file shares one clock: it starts with the file, runs
//! for the span and then loops or stops. A value's keys become a list of
//! values, the moments they fall at as fractions of the span, and one ease per
//! gap between them. That ease is the same unit-square cubic a Lumit key
//! already is ([`crate::keys`]), so a key's easing comes across exactly.
//!
//! SMIL asks one more thing: every number of an ease has to lie from 0 to 1.
//! An ease that overshoots, a bounce past its key, can't be written as one.
//! Such a span is cut where it turns round, which is exact, and each
//! piece cut again until it fits; what still doesn't fit is sampled frame by
//! frame and written as straight steps.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::anim::CubicSpan;

use crate::fit;
use crate::keys::Track;

/// How root seconds, the file's own, read on some owner's clock: a comp's, or
/// a layer's.
#[derive(Debug, Clone, PartialEq)]
pub enum Clock {
    /// The owner's seconds are the root's less this.
    Shift(f64),
    /// The owner's seconds at each frame of the grid, for a clock a Retime
    /// bends, which no shift describes.
    Map(Vec<f64>),
}

impl Clock {
    /// The owner's seconds at grid frame `i`, which falls at `root` seconds.
    #[must_use]
    pub fn at(&self, i: usize, root: f64) -> f64 {
        match self {
            Clock::Shift(s) => root - s,
            Clock::Map(v) => v.get(i).copied().unwrap_or(root),
        }
    }

    /// This clock read `by` seconds later, which is how a layer's own clock
    /// sits on its comp's.
    #[must_use]
    pub fn shifted(&self, by: f64) -> Clock {
        match self {
            Clock::Shift(s) => Clock::Shift(s + by),
            Clock::Map(v) => Clock::Map(v.iter().map(|t| t - by).collect()),
        }
    }
}

/// How one span eases from its first value to its last.
#[derive(Debug, Clone, PartialEq)]
pub enum Ease {
    /// The first value until the span ends.
    Hold,
    /// A unit-square cubic `[x1, y1, x2, y2]` per number, or one shared by
    /// every number.
    Curve(Vec<[f64; 4]>),
}

/// One span of a value, in root seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub t0: f64,
    pub t1: f64,
    pub v0: Vec<f64>,
    pub v1: Vec<f64>,
    pub ease: Ease,
}

/// The straight line, as a unit-square cubic.
pub const LINEAR: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

impl Seg {
    fn spline(&self, d: usize) -> [f64; 4] {
        match &self.ease {
            Ease::Curve(s) => s.get(d).or_else(|| s.first()).copied().unwrap_or(LINEAR),
            Ease::Hold => LINEAR,
        }
    }

    /// The value at `t`, inside the span.
    fn at(&self, t: f64) -> Vec<f64> {
        if matches!(self.ease, Ease::Hold) || self.t1 <= self.t0 {
            return if t < self.t1 {
                self.v0.clone()
            } else {
                self.v1.clone()
            };
        }
        let s = ((t - self.t0) / (self.t1 - self.t0)).clamp(0.0, 1.0);
        self.v0
            .iter()
            .zip(&self.v1)
            .enumerate()
            .map(|(d, (a, b))| a + (b - a) * eased(self.spline(d), s))
            .collect()
    }
}

/// The unit-square cubic `sp` read at `s`, the way a player reads it.
fn eased(sp: [f64; 4], s: f64) -> f64 {
    CubicSpan::from_points([0.0, sp[0], sp[2], 1.0], [0.0, sp[1], sp[3], 1.0]).value_at(s)
}

/// A value in root seconds: one that never moves, or its spans in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Still(Vec<f64>),
    Moving(Vec<Seg>),
}

impl Value {
    /// `track`, whose key times are already root seconds.
    #[must_use]
    pub fn from_track(track: &Track) -> Value {
        if track.keys.len() < 2 || track.is_still() {
            return Value::Still(track.first_value());
        }
        let segs: Vec<Seg> = track
            .keys
            .windows(2)
            .filter_map(|pair| {
                let [a, b] = pair else { return None };
                if b.seconds <= a.seconds || b.seconds.is_nan() {
                    return None;
                }
                let ease = if a.hold {
                    Ease::Hold
                } else {
                    Ease::Curve(
                        (0..a.value.len())
                            .map(|d| {
                                let o = a.out.get(d).copied().unwrap_or([1.0 / 3.0; 2]);
                                let i = a.inn.get(d).copied().unwrap_or([2.0 / 3.0; 2]);
                                [o[0], o[1], i[0], i[1]]
                            })
                            .collect(),
                    )
                };
                Some(Seg {
                    t0: a.seconds,
                    t1: b.seconds,
                    v0: a.value.clone(),
                    v1: b.value.clone(),
                    ease,
                })
            })
            .collect();
        if segs.is_empty() {
            return Value::Still(track.first_value());
        }
        Value::Moving(segs)
    }

    /// Samples taken at `times`, as the fewest straight keys within `tol`.
    #[must_use]
    pub fn sampled(times: &[f64], samples: Vec<Vec<f64>>, tol: &[f64]) -> Value {
        let pairs: Vec<(f64, Vec<f64>)> = times.iter().copied().zip(samples).collect();
        Value::from_track(&fit::linear(&pairs, tol))
    }

    /// The value at root second `t`.
    #[must_use]
    pub fn at(&self, t: f64) -> Vec<f64> {
        match self {
            Value::Still(v) => v.clone(),
            Value::Moving(segs) => {
                let (Some(first), Some(last)) = (segs.first(), segs.last()) else {
                    return Vec::new();
                };
                if t <= first.t0 {
                    return first.v0.clone();
                }
                if t >= last.t1 {
                    return last.v1.clone();
                }
                let i = segs.partition_point(|s| s.t1 <= t);
                segs.get(i).map_or_else(|| last.v1.clone(), |s| s.at(t))
            }
        }
    }

    /// Every number put through `f(dimension, value)`, which has to be of the
    /// form `a·v + b`: an ease in its unit square doesn't change under one.
    #[must_use]
    pub fn map(self, f: impl Fn(usize, f64) -> f64) -> Value {
        let each =
            |v: Vec<f64>| -> Vec<f64> { v.into_iter().enumerate().map(|(d, x)| f(d, x)).collect() };
        match self {
            Value::Still(v) => Value::Still(each(v)),
            Value::Moving(segs) => Value::Moving(
                segs.into_iter()
                    .map(|s| Seg {
                        v0: each(s.v0),
                        v1: each(s.v1),
                        ..s
                    })
                    .collect(),
            ),
        }
    }

    /// True for a value that never moves.
    #[must_use]
    pub fn is_still(&self) -> bool {
        matches!(self, Value::Still(_))
    }
}

/// The span of the file and the frames a value is sampled at, in root seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    /// Every output frame from `start` to `end`, both ends included.
    pub grid: Vec<f64>,
    pub looping: bool,
}

/// A value as SMIL takes it: the values at their times, and one ease per gap.
/// `splines` is one shorter than `values`.
#[derive(Debug, Clone, PartialEq)]
pub struct Smil {
    pub times: Vec<f64>,
    pub values: Vec<Vec<f64>>,
    pub splines: Vec<[f64; 4]>,
}

/// `value` across `span` as SMIL keys, or `None` when it doesn't move there.
/// `tol` is how closely a span that has to be sampled is kept, per number.
#[must_use]
pub fn smil(value: &Value, span: &Span, tol: &[f64]) -> Option<Smil> {
    let Value::Moving(segs) = value else {
        return None;
    };
    let (start, end) = (span.start, span.end);
    // The spans inside the file's span, those that cross its ends cut there.
    let mut inside: Vec<Seg> = Vec::with_capacity(segs.len() + 2);
    for seg in segs {
        if seg.t1 <= start || seg.t0 >= end {
            continue;
        }
        if seg.t0 >= start && seg.t1 <= end {
            inside.push(seg.clone());
            continue;
        }
        let (a, b) = (seg.t0.max(start), seg.t1.min(end));
        match seg.ease {
            Ease::Hold => inside.push(Seg {
                t0: a,
                t1: b,
                v0: seg.v0.clone(),
                v1: if b < seg.t1 {
                    seg.v0.clone()
                } else {
                    seg.v1.clone()
                },
                ease: Ease::Hold,
            }),
            Ease::Curve(_) => inside.extend(straightened(seg, a, b, &span.grid, tol)),
        }
    }
    let base = value.at(start);
    let moves = inside
        .iter()
        .any(|s| differs(&s.v0, &base) || differs(&s.v1, &base));
    if !moves {
        return None;
    }
    let mut out = Smil {
        times: vec![start],
        values: vec![base.clone()],
        splines: Vec::new(),
    };
    let push = |out: &mut Smil, t: f64, v: Vec<f64>, sp: [f64; 4]| {
        out.times.push(t);
        out.values.push(v);
        out.splines.push(sp);
    };
    for seg in &inside {
        let (last_t, last_v) = match (out.times.last(), out.values.last()) {
            (Some(t), Some(v)) => (*t, v.clone()),
            _ => continue,
        };
        // A step into this span: a hold ending, or the padding before it.
        if seg.t0 > last_t {
            push(&mut out, seg.t0, last_v.clone(), LINEAR);
        }
        if differs(&seg.v0, &last_v) {
            push(&mut out, seg.t0, seg.v0.clone(), LINEAR);
        }
        match &seg.ease {
            Ease::Hold => {
                push(&mut out, seg.t1, seg.v0.clone(), LINEAR);
                // The step at its end, which a last span has no next one for.
                if differs(&seg.v1, &seg.v0) {
                    push(&mut out, seg.t1, seg.v1.clone(), LINEAR);
                }
            }
            Ease::Curve(_) => match pieces(seg) {
                Some(pieces) => {
                    for (t, v, sp) in pieces {
                        push(&mut out, t, v, sp);
                    }
                }
                None => {
                    for s in straightened(seg, seg.t0, seg.t1, &span.grid, tol) {
                        push(&mut out, s.t1, s.v1, LINEAR);
                    }
                }
            },
        }
    }
    // Held to the end.
    if let (Some(t), Some(v)) = (out.times.last().copied(), out.values.last().cloned()) {
        if t < end {
            push(&mut out, end, v, LINEAR);
        }
    }
    Some(out)
}

/// True when two values differ by more than rounding.
fn differs(a: &[f64], b: &[f64]) -> bool {
    a.len() != b.len()
        || a.iter()
            .zip(b)
            .any(|(x, y)| (x - y).abs() > 1e-9 * (1.0 + x.abs() + y.abs()))
}

/// `seg` from `a` to `b` as straight spans through its values at the grid's
/// frames, thinned to `tol`.
fn straightened(seg: &Seg, a: f64, b: f64, grid: &[f64], tol: &[f64]) -> Vec<Seg> {
    let mut times = vec![a];
    times.extend(grid.iter().copied().filter(|t| *t > a && *t < b));
    times.push(b);
    let samples: Vec<Vec<f64>> = times.iter().map(|t| seg.at(*t)).collect();
    match Value::sampled(&times, samples, tol) {
        Value::Moving(segs) => segs,
        Value::Still(v) => vec![Seg {
            t0: a,
            t1: b,
            v0: v.clone(),
            v1: v,
            ease: Ease::Curve(vec![LINEAR]),
        }],
    }
}

/// One SMIL piece: its end time, its end value and its keySpline.
type Piece = (f64, Vec<f64>, [f64; 4]);

/// One eased span as SMIL pieces, each `(end time, end value, keySpline)`
/// with every number of the spline in 0..1. `None` when its numbers ease
/// differently from each other, or the ease can't be cut to fit.
fn pieces(seg: &Seg) -> Option<Vec<Piece>> {
    let moving: Vec<usize> = (0..seg.v0.len())
        .filter(|d| {
            let (a, b) = (
                seg.v0.get(*d).copied().unwrap_or(0.0),
                seg.v1.get(*d).copied().unwrap_or(0.0),
            );
            (b - a).abs() > 1e-9 * (1.0 + a.abs() + b.abs())
        })
        .collect();
    let Some(&first) = moving.first() else {
        return Some(vec![(seg.t1, seg.v1.clone(), LINEAR)]);
    };
    let sp = seg.spline(first);
    if moving.iter().any(|d| {
        seg.spline(*d)
            .iter()
            .zip(sp)
            .any(|(a, b)| (a - b).abs() > 1e-6)
    }) {
        return None;
    }
    let cut = fitted(sp)?;
    let dt = seg.t1 - seg.t0;
    Some(
        cut.into_iter()
            .map(|(x, y, s)| {
                let v = seg
                    .v0
                    .iter()
                    .zip(&seg.v1)
                    .map(|(a, b)| a + (b - a) * y)
                    .collect();
                (seg.t0 + dt * x, v, s)
            })
            .collect(),
    )
}

/// The unit-square ease `sp` as pieces SMIL can carry, each
/// `(end x, end y, keySpline)` in the whole span's unit square, or `None`
/// when it can't be cut to fit.
#[must_use]
pub fn fitted(sp: [f64; 4]) -> Option<Vec<(f64, f64, [f64; 4])>> {
    if in_range(&sp) {
        return Some(vec![(1.0, 1.0, sp)]);
    }
    let whole = CubicSpan::from_points([0.0, sp[0], sp[2], 1.0], [0.0, sp[1], sp[3], 1.0]);
    // Cut first where the value turns round: each piece then only rises or
    // only falls, and its handles settle inside its own square.
    let mut cuts = turning_points(sp);
    cuts.retain(|u| *u > 1e-6 && *u < 1.0 - 1e-6);
    let mut out = Vec::new();
    let mut rest = whole;
    let mut used = 0.0;
    for u in cuts {
        let (l, r) = rest.split_at((u - used) / (1.0 - used));
        settle(&l, 0, &mut out)?;
        rest = r;
        used = u;
    }
    settle(&rest, 0, &mut out)?;
    Some(out)
}

/// The deepest a piece is halved in looking for handles that fit: 2⁻¹² of the
/// span, far finer than a frame of any span worth easing.
const MAX_DEPTH: u32 = 12;

/// `piece` as pieces whose own handles fit their own square, halving it until
/// they do.
fn settle(piece: &CubicSpan, depth: u32, out: &mut Vec<(f64, f64, [f64; 4])>) -> Option<()> {
    let (x, y) = piece.control_points();
    let (dx, dy) = (x[3] - x[0], y[3] - y[0]);
    if dx <= 1e-12 || dx.is_nan() {
        return None;
    }
    let sp = if dy.abs() <= 1e-12 {
        // A piece that neither rises nor falls: it stands still, which any
        // spline draws.
        ((y[1] - y[0]).abs() <= 1e-9 && (y[2] - y[0]).abs() <= 1e-9).then_some(LINEAR)
    } else {
        Some([
            (x[1] - x[0]) / dx,
            (y[1] - y[0]) / dy,
            (x[2] - x[0]) / dx,
            (y[2] - y[0]) / dy,
        ])
    };
    match sp {
        Some(sp) if in_range(&sp) => {
            out.push((x[3], y[3], sp.map(|v| v.clamp(0.0, 1.0))));
            Some(())
        }
        _ if depth < MAX_DEPTH => {
            let (l, r) = piece.split_at(0.5);
            settle(&l, depth + 1, out)?;
            settle(&r, depth + 1, out)
        }
        _ => None,
    }
}

fn in_range(sp: &[f64; 4]) -> bool {
    sp.iter().all(|v| (-1e-9..=1.0 + 1e-9).contains(v))
}

/// Where the value of the unit-square ease `sp` turns round, as parameters,
/// in order.
fn turning_points(sp: [f64; 4]) -> Vec<f64> {
    // y'(u)/3 = a·u² + b·u + c over the handle deltas.
    let (p0, p1, p2) = (sp[1], sp[3] - sp[1], 1.0 - sp[3]);
    let (a, b, c) = (p0 - 2.0 * p1 + p2, 2.0 * (p1 - p0), p0);
    let mut roots = Vec::with_capacity(2);
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
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    roots
}

/// A number written to `decimals` places, without trailing zeros.
#[must_use]
pub fn num(v: f64, decimals: u32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let mut s = format!("{:.*}", decimals as usize, v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = String::from("0");
    }
    s
}

/// How the animation repeats and how long it runs.
fn timing(span: &Span) -> String {
    let dur = num(span.end - span.start, 6);
    if span.looping {
        format!(" dur=\"{dur}s\" repeatCount=\"indefinite\"")
    } else {
        format!(" dur=\"{dur}s\" fill=\"freeze\"")
    }
}

/// The key times of `times` as fractions of the span, the first 0 and the
/// last 1.
fn key_times(times: &[f64], span: &Span) -> String {
    let len = (span.end - span.start).max(1e-12);
    let n = times.len();
    times
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == 0 {
                String::from("0")
            } else if i + 1 == n {
                String::from("1")
            } else {
                num(((t - span.start) / len).clamp(0.0, 1.0), 6)
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// An `<animate>` (or, with `kind`, an `<animateTransform>` of that type) of
/// `attr` through `smil`, each value written by `fmt`.
pub fn write(
    out: &mut String,
    attr: &str,
    kind: Option<&str>,
    smil: &Smil,
    span: &Span,
    fmt: &dyn Fn(&[f64]) -> String,
) {
    let values: Vec<String> = smil.values.iter().map(|v| fmt(v)).collect();
    let straight = smil
        .splines
        .iter()
        .all(|s| (s[0] - s[1]).abs() < 1e-9 && (s[2] - s[3]).abs() < 1e-9);
    let element = if kind.is_some() {
        "animateTransform"
    } else {
        "animate"
    };
    let _ = write!(out, "<{element} attributeName=\"{attr}\"");
    if let Some(kind) = kind {
        let _ = write!(out, " type=\"{kind}\"");
    }
    let _ = write!(
        out,
        " values=\"{}\" keyTimes=\"{}\"",
        values.join(";"),
        key_times(&smil.times, span)
    );
    if !straight {
        let splines: Vec<String> = smil
            .splines
            .iter()
            .map(|s| s.map(|v| num(v.clamp(0.0, 1.0), 4)).join(" "))
            .collect();
        let _ = write!(
            out,
            " calcMode=\"spline\" keySplines=\"{}\"",
            splines.join(";")
        );
    }
    out.push_str(&timing(span));
    out.push_str("/>");
}

/// An `<animate>` of `attr` stepping through `values`, each from its time on
/// (`calcMode="discrete"`). The first time is the span's start.
pub fn write_steps(out: &mut String, attr: &str, times: &[f64], values: &[String], span: &Span) {
    if values.len() < 2 {
        return;
    }
    let len = (span.end - span.start).max(1e-12);
    let key_times: Vec<String> = times
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == 0 {
                String::from("0")
            } else {
                num(((t - span.start) / len).clamp(0.0, 1.0), 6)
            }
        })
        .collect();
    let _ = write!(
        out,
        "<animate attributeName=\"{attr}\" values=\"{}\" keyTimes=\"{}\" calcMode=\"discrete\"{}/>",
        values.join(";"),
        key_times.join(";"),
        timing(span)
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::{evaluate, Keyframe, SideInterp};
    use lumit_core::time::Rational;

    /// The one promise SMIL forces on us: an ease that overshoots its keys
    /// is written with every keySpline number inside 0..1, and the file,
    /// played the way SMIL plays it (one cubic per gap of keyTimes), still
    /// shows the value Lumit evaluates.
    #[test]
    fn an_overshooting_ease_fits_smil_and_plays_back() {
        let key = |t: i64, v: f64, inn: SideInterp, out: SideInterp| Keyframe {
            time: Rational::new(t, 10).unwrap(),
            value: v,
            interp_in: inn,
            interp_out: out,
        };
        // Leaves fast and arrives fast in the wrong direction: a bounce past
        // both keys, the shape a back ease makes.
        let keys = vec![
            key(
                0,
                0.0,
                SideInterp::Linear,
                SideInterp::Bezier {
                    speed: -400.0,
                    influence: 0.6,
                },
            ),
            key(
                10,
                100.0,
                SideInterp::Bezier {
                    speed: -400.0,
                    influence: 0.6,
                },
                SideInterp::Linear,
            ),
            key(20, 40.0, SideInterp::Linear, SideInterp::Linear),
        ];
        let track = crate::keys::track(&[crate::keys::curve(&keys)]).unwrap();
        let value = Value::from_track(&track);
        let span = Span {
            start: 0.0,
            end: 2.0,
            grid: (0..=60).map(|f| f64::from(f) / 30.0).collect(),
            looping: true,
        };
        let s = smil(&value, &span, &[0.01]).unwrap();
        assert_eq!(s.splines.len() + 1, s.values.len());
        assert!(s.splines.len() > 2, "the bounce was cut");
        for sp in &s.splines {
            assert!(sp.iter().all(|v| (0.0..=1.0).contains(v)), "{sp:?}");
        }
        // SMIL's own reading: find the gap, read its spline, mix the values.
        let play = |t: f64| -> f64 {
            let i = s
                .times
                .partition_point(|k| *k <= t)
                .saturating_sub(1)
                .min(s.splines.len() - 1);
            let (t0, t1) = (s.times[i], s.times[i + 1]);
            let (v0, v1) = (s.values[i][0], s.values[i + 1][0]);
            if t1 <= t0 {
                return v1;
            }
            v0 + (v1 - v0) * eased(s.splines[i], (t - t0) / (t1 - t0))
        };
        let mut worst = 0.0f64;
        for k in 0..=400 {
            let t = 2.0 * f64::from(k) / 400.0;
            let want = evaluate(&keys, t).unwrap();
            worst = worst.max((play(t) - want).abs());
        }
        assert!(worst < 1e-6, "off by {worst}");
        // And the overshoot really is there to be kept.
        let peak = (0..=100)
            .map(|k| evaluate(&keys, f64::from(k) / 100.0).unwrap())
            .fold(f64::MIN, f64::max);
        assert!(peak > 100.5);
    }
}

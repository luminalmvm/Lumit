//! A Lottie's keys as Lumit's: each span's unit-square handles turned back
//! into a speed and an influence on each side, exactly.
//!
//! # In plain terms
//!
//! A Lottie span is a cubic in a unit square: from (0, 0) to (1, 1), the handle
//! leaving the first key at `o` and the one arriving at the next at `i`.
//! Lumit's span is the same cubic described the way After Effects describes
//! it: how far each handle reaches (its influence, a fraction of the span) and
//! how steep it is (its speed, in units a second). So `o.x` is the influence
//! out, and `o.y` over `o.x`, scaled by the span's rise over its run, is the
//! speed out; the in side is the same from the other corner. Nothing is
//! approximated, and a key written by [`crate::keys`] reads back as it was.
//!
//! A position whose motion path curves (`to` and `ti` on its keys) has no
//! equal in Lumit, which animates each axis on its own, so it is sampled a
//! frame at a time along the path and fitted back into straight keys per
//! axis ([`crate::fit::linear`]), close enough that no frame moves.
//!
//! Runs on the import's thread.

use lumit_core::anim::{Animation, CubicSpan, Keyframe, Property, SideInterp};
use lumit_core::mask::{BezierPath, PathKeyframe};
use lumit_core::time::Rational;

use super::read::{Clock, Read};
use crate::keys::{Track, LINEAR_IN, LINEAR_OUT};

/// The most samples a curved motion path takes: about an hour at 60 fps.
const MAX_SAMPLES: usize = crate::time::MAX_SAMPLES;

/// The two sides of one span, from its handles in the unit square, its length
/// in seconds and its rise.
pub(super) fn sides(o: [f64; 2], i: [f64; 2], dt: f64, dv: f64) -> (SideInterp, SideInterp) {
    let diagonal = |h: [f64; 2]| (h[0] - h[1]).abs() < 1e-9;
    if diagonal(o) && diagonal(i) {
        return (SideInterp::Linear, SideInterp::Linear);
    }
    let dt = dt.max(1e-12);
    let infl_out = o[0].clamp(1e-3, 1.0);
    let infl_in = (1.0 - i[0]).clamp(1e-3, 1.0);
    let speed = |v: f64| if v.is_finite() { v } else { 0.0 };
    (
        SideInterp::Bezier {
            speed: speed(o[1] * dv / (infl_out * dt)),
            influence: infl_out,
        },
        SideInterp::Bezier {
            speed: speed((1.0 - i[1]) * dv / (infl_in * dt)),
            influence: infl_in,
        },
    )
}

/// The keys' times as exact seconds, each strictly after the one before: two
/// keys on one time (a step) are a thousandth of a frame apart.
fn times(track: &Track, clock: &Clock) -> Vec<Rational> {
    let step =
        Rational::new(1, (clock.fps * 1000.0).round().max(1.0) as i64).unwrap_or(Rational::ONE);
    let mut out: Vec<Rational> = Vec::with_capacity(track.keys.len());
    for key in &track.keys {
        let mut t = clock.time(key.seconds * clock.fps);
        if let Some(prev) = out.last() {
            if t <= *prev {
                t = prev.checked_add(step).unwrap_or(*prev);
            }
        }
        out.push(t);
    }
    out
}

/// Number `dim` of a track as Lumit's keys, on the owner's clock.
pub(super) fn keyframes(track: &Track, dim: usize, clock: &Clock, default: f64) -> Vec<Keyframe> {
    let at = times(track, clock);
    let value = |n: usize| {
        track
            .keys
            .get(n)
            .and_then(|k| k.value.get(dim))
            .copied()
            .unwrap_or(default)
    };
    let mut keys: Vec<Keyframe> = track
        .keys
        .iter()
        .enumerate()
        .map(|(n, _)| Keyframe {
            time: at.get(n).copied().unwrap_or(Rational::ZERO),
            value: value(n),
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        })
        .collect();
    for n in 0..track.keys.len().saturating_sub(1) {
        let (Some(a), Some(t0), Some(t1)) = (track.keys.get(n), at.get(n), at.get(n + 1)) else {
            continue;
        };
        let (out, inn) = if a.hold {
            (SideInterp::Hold, SideInterp::Linear)
        } else {
            let o = a
                .out
                .get(dim)
                .or(a.out.first())
                .copied()
                .unwrap_or(LINEAR_OUT);
            let i = a
                .inn
                .get(dim)
                .or(a.inn.first())
                .copied()
                .unwrap_or(LINEAR_IN);
            sides(o, i, t1.to_f64() - t0.to_f64(), value(n + 1) - value(n))
        };
        if let Some(k) = keys.get_mut(n) {
            k.interp_out = out;
        }
        if let Some(k) = keys.get_mut(n + 1) {
            k.interp_in = inn;
        }
    }
    keys
}

/// Number `dim` of a track as a property: still when it doesn't move.
pub(super) fn property(track: &Track, dim: usize, clock: &Clock, default: f64) -> Property {
    let first = track
        .keys
        .first()
        .and_then(|k| k.value.get(dim))
        .copied()
        .unwrap_or(default);
    let moves = track.keys.iter().any(|k| {
        k.value
            .get(dim)
            .is_some_and(|v| (v - first).abs() > 1e-9 * (1.0 + first.abs()))
    });
    if !moves {
        return Property::fixed(first);
    }
    Property {
        animation: Animation::Keyframed(keyframes(track, dim, clock, default)),
        extra: serde_json::Map::new(),
    }
}

/// Number `dim` of a read property, or `default` when there is none.
pub(super) fn one(read: Option<&Read>, dim: usize, clock: &Clock, default: f64) -> Property {
    read.map_or_else(
        || Property::fixed(default),
        |r| property(&r.track, dim, clock, default),
    )
}

/// A two-number value (a position, an anchor) as two properties. A position
/// whose motion path curves is sampled along it first.
pub(super) fn pair(read: Option<&Read>, clock: &Clock, default: [f64; 2]) -> [Property; 2] {
    let Some(read) = read else {
        return default.map(Property::fixed);
    };
    let track = if read.spatial.is_empty() {
        std::borrow::Cow::Borrowed(&read.track)
    } else {
        std::borrow::Cow::Owned(along_path(read, clock))
    };
    [
        property(&track, 0, clock, default[0]),
        property(&track, 1, clock, default[1]),
    ]
}

/// A curved motion path, sampled at every frame between its first key and its
/// last and at the keys themselves, and fitted per axis.
pub(super) fn along_path(read: &Read, clock: &Clock) -> Track {
    let keys = &read.track.keys;
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
        return read.track.clone();
    };
    let (a, b) = (first.seconds * clock.fps, last.seconds * clock.fps);
    let mut frames: Vec<f64> = Vec::new();
    let mut f = a.floor();
    while f <= b && frames.len() < MAX_SAMPLES {
        if f >= a {
            frames.push(f);
        }
        f += 1.0;
    }
    frames.extend(keys.iter().map(|k| k.seconds * clock.fps));
    frames.sort_by(f64::total_cmp);
    frames.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    let samples: Vec<(f64, Vec<f64>)> = frames
        .iter()
        .map(|f| {
            let s = clock.seconds(*f);
            (s, point_at(read, s).to_vec())
        })
        .collect();
    let tol = crate::fit::tolerance::PIXELS;
    crate::fit::linear(&samples, &[tol, tol])
}

/// Where a curved motion path is at `seconds`: the span's ease gives how far
/// along the curve, by length, as lottie-web reads it.
fn point_at(read: &Read, seconds: f64) -> [f64; 2] {
    let keys = &read.track.keys;
    let xy = |v: &[f64]| {
        [
            v.first().copied().unwrap_or(0.0),
            v.get(1).copied().unwrap_or(0.0),
        ]
    };
    let p = keys.partition_point(|k| k.seconds <= seconds);
    let (Some(k0), Some(k1)) = (keys.get(p.wrapping_sub(1)), keys.get(p)) else {
        let end = if p == 0 { keys.first() } else { keys.last() };
        return end.map_or([0.0, 0.0], |k| xy(&k.value));
    };
    let (p0, p3) = (xy(&k0.value), xy(&k1.value));
    if k0.hold {
        return p0;
    }
    let dt = (k1.seconds - k0.seconds).max(1e-12);
    let u = (seconds - k0.seconds) / dt;
    let o = k0.out.first().copied().unwrap_or(LINEAR_OUT);
    let i = k0.inn.first().copied().unwrap_or(LINEAR_IN);
    let ease = CubicSpan::from_points([0.0, o[0], i[0], 1.0], [0.0, o[1], i[1], 1.0]).value_at(u);
    let (to, ti) = read
        .spatial
        .get(p - 1)
        .copied()
        .unwrap_or(([0.0, 0.0], [0.0, 0.0]));
    let ctrl = [
        p0,
        [p0[0] + to[0], p0[1] + to[1]],
        [p3[0] + ti[0], p3[1] + ti[1]],
        p3,
    ];
    at_length(&ctrl, ease.clamp(0.0, 1.0))
}

/// The point a fraction `s` of the way along a cubic, by length.
fn at_length(c: &[[f64; 2]; 4], s: f64) -> [f64; 2] {
    const STEPS: usize = 64;
    let point = |u: f64| {
        let w = 1.0 - u;
        let (a, b, cc, d) = (w * w * w, 3.0 * w * w * u, 3.0 * w * u * u, u * u * u);
        [
            a * c[0][0] + b * c[1][0] + cc * c[2][0] + d * c[3][0],
            a * c[0][1] + b * c[1][1] + cc * c[2][1] + d * c[3][1],
        ]
    };
    let pts: Vec<[f64; 2]> = (0..=STEPS)
        .map(|n| point(n as f64 / STEPS as f64))
        .collect();
    let mut lengths = Vec::with_capacity(pts.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for w in pts.windows(2) {
        if let [p, q] = w {
            total += ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
        }
        lengths.push(total);
    }
    if total <= 1e-12 {
        return point(s);
    }
    let want = s * total;
    let n = lengths.partition_point(|l| *l < want).clamp(1, STEPS);
    let (Some(l0), Some(l1)) = (lengths.get(n - 1).copied(), lengths.get(n).copied()) else {
        return point(s);
    };
    let f = if l1 > l0 {
        (want - l0) / (l1 - l0)
    } else {
        0.0
    };
    point((n as f64 - 1.0 + f) / STEPS as f64)
}

/// One key of a path as the file has it: its frame, its shape, whether it
/// holds, and the handles leaving it and arriving at the next.
pub(super) type PathKey = (f64, BezierPath, bool, [f64; 2], [f64; 2]);

/// A keyed path's keys as Lumit's: the shape at each key, eased the same way
/// between them, on a 0 to 1 ramp.
pub(super) fn path_keys(keys: &[PathKey], clock: &Clock) -> Vec<PathKeyframe> {
    let track = Track {
        keys: keys
            .iter()
            .enumerate()
            .map(|(n, (t, _, hold, o, i))| crate::keys::TrackKey {
                seconds: clock.seconds(*t),
                value: vec![n as f64],
                hold: *hold,
                out: vec![*o],
                inn: vec![*i],
            })
            .collect(),
    };
    let at = times(&track, clock);
    let mut out: Vec<PathKeyframe> = keys
        .iter()
        .zip(&at)
        .map(|((_, path, ..), time)| PathKeyframe {
            time: *time,
            path: path.clone(),
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        })
        .collect();
    for n in 0..out.len().saturating_sub(1) {
        let (Some((_, _, hold, o, i)), Some(t0), Some(t1)) =
            (keys.get(n), at.get(n), at.get(n + 1))
        else {
            continue;
        };
        let (a, b) = if *hold {
            (SideInterp::Hold, SideInterp::Linear)
        } else {
            sides(*o, *i, t1.to_f64() - t0.to_f64(), 1.0)
        };
        if let Some(k) = out.get_mut(n) {
            k.interp_out = a;
        }
        if let Some(k) = out.get_mut(n + 1) {
            k.interp_in = b;
        }
    }
    out
}

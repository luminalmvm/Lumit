//! A layer's motion path: the way Position travels from key to key.
//!
//! Straight is each axis on its own curve, so the layer goes in a straight
//! line between two keys. Curved runs a smooth path through the keys, as After
//! Effects' auto bezier does, so a few keys round a circle make a circle. The
//! keys' eases still set how fast it goes: they say how far along the span it
//! is, and that share of the span is measured along the curve.
//!
//! Straight is every file from before this existed, and writes nothing.

use serde::{Deserialize, Serialize};

use crate::anim::{evaluate, Animation, Property};
use crate::model::{Document, TransformGroup};
use crate::ops::{Op, OpError};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPath {
    #[default]
    Straight,
    Curved,
}

impl MotionPath {
    /// For serde.
    #[must_use]
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde hands a reference
    pub fn is_straight(&self) -> bool {
        matches!(self, Self::Straight)
    }
}

/// Set a layer's motion path, handing back the op that puts it back.
pub(crate) fn set(
    doc: &mut Document,
    comp: Uuid,
    layer: Uuid,
    path: MotionPath,
) -> Result<Op, OpError> {
    let c = doc.comp_mut(comp).ok_or(OpError::UnknownComp)?;
    let l = c
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .ok_or(OpError::UnknownLayer)?;
    let previous = std::mem::replace(&mut l.transform.motion_path, path);
    Ok(Op::SetLayerMotionPath {
        comp,
        layer,
        path: previous,
    })
}

type Point = [f64; 3];

/// Position at layer time `lt`, each property read by `read`. A `flat` (2D)
/// layer counts its z as nought.
#[must_use]
pub fn position(
    tr: &TransformGroup,
    lt: f64,
    flat: bool,
    read: &dyn Fn(&Property) -> f64,
) -> (f64, f64, f64) {
    let z = if flat { 0.0 } else { read(&tr.position_z) };
    let straight = [read(&tr.position_x), read(&tr.position_y), z];
    let p = match tr.motion_path {
        MotionPath::Straight => straight,
        MotionPath::Curved => curved(tr, lt, flat, straight).unwrap_or(straight),
    };
    (p[0], p[1], p[2])
}

/// The curved position, or `None` where the path is the straight one: an
/// expression drives an axis, there are under three keys, a key holds, or
/// `lt` is outside the keys.
fn curved(tr: &TransformGroup, lt: f64, flat: bool, straight: Point) -> Option<Point> {
    let axes: &[&Property] = if flat {
        &[&tr.position_x, &tr.position_y]
    } else {
        &[&tr.position_x, &tr.position_y, &tr.position_z]
    };
    let mut times = Vec::new();
    for axis in axes {
        match &axis.animation {
            Animation::Expression(_) => return None,
            Animation::Static(_) => {}
            Animation::Keyframed(keys) => {
                // A held key steps, and a step has no path to curve.
                if keys.iter().any(|k| k.is_hold()) {
                    return None;
                }
                times.extend(keys.iter().map(|k| k.time.to_f64()));
            }
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    if times.len() < 3 {
        return None;
    }
    let i = times.windows(2).position(|w| w[0] <= lt && lt < w[1])?;
    let at = |j: usize| -> Point {
        let mut p = [0.0; 3];
        for (out, axis) in p.iter_mut().zip(axes) {
            *out = match &axis.animation {
                Animation::Static(v) => *v,
                Animation::Keyframed(keys) => evaluate(keys, times[j]).unwrap_or(0.0),
                Animation::Expression(_) => 0.0,
            };
        }
        p
    };
    let last = times.len() - 1;
    let (a, b) = (at(i), at(i + 1));
    let chord = sub(b, a);
    let length = dot(chord, chord);
    if length <= 1e-18 {
        return Some(a);
    }
    // How far along the span the eases have it: the straight position laid on
    // the chord. Exact for keys that ease alike on every axis.
    let share = dot(sub(straight, a), chord) / length;
    // A path that ends where it starts goes round smoothly through that key.
    let closed = dist(at(0), at(last)) <= 1e-6;
    let before = match i.checked_sub(1) {
        Some(j) => Some(at(j)),
        None => closed.then(|| at(last - 1)),
    };
    let after = if i + 1 < last {
        Some(at(i + 2))
    } else {
        closed.then(|| at(1))
    };
    let mut out = handle(before, a, Some(b), true);
    let mut back = handle(Some(a), b, after, false);
    // An end key's handle makes the path leave it with no bend, reaching for
    // the handle beside it.
    if before.is_none() {
        out = lerp(a, back, 0.5);
    }
    if after.is_none() {
        back = lerp(b, out, 0.5);
    }
    Some(along([a, out, back, b], share))
}

/// Where a key's handle on one side sits: along the line from the key before
/// to the key after, reaching a third of the way to the neighbour on that
/// side, and further the more the path turns there. That reach is the one a
/// circle's arc needs, so keys spaced round a circle draw a circle. An end
/// key gets its own handle in [`curved`].
fn handle(before: Option<Point>, key: Point, after: Option<Point>, out: bool) -> Point {
    let (Some(p), Some(n)) = (before, after) else {
        return key;
    };
    let aim = sub(n, p);
    let side = if out { sub(n, key) } else { sub(key, p) };
    let (aim_length, side_length) = (dot(aim, aim).sqrt(), dot(side, side).sqrt());
    if aim_length <= 1e-9 || side_length <= 1e-9 {
        return key;
    }
    // The cosine of the angle between the handle and the span: one on a
    // straight run, smaller round a bend. Past a right angle it stays at the
    // right angle's reach.
    let turn = (dot(aim, side) / (aim_length * side_length)).max(0.0);
    let reach = 2.0 * side_length / (3.0 * (1.0 + turn)) / aim_length;
    add(key, scale(aim, if out { reach } else { -reach }))
}

/// How many straight pieces a span is measured in, to find the point a share
/// of its length along. Fixed, so every machine gets the same answer.
const STEPS: usize = 32;

/// The point `share` of the way along the cubic `c`, by length. Past either
/// end it carries on along the end's own direction.
fn along(c: [Point; 4], share: f64) -> Point {
    // How far along the curve each step is.
    let mut lengths = [0.0; STEPS + 1];
    let (mut prev, mut so_far) = (c[0], 0.0);
    for (k, length) in lengths.iter_mut().enumerate().skip(1) {
        let p = bezier(c, k as f64 / STEPS as f64);
        so_far += dist(prev, p);
        *length = so_far;
        prev = p;
    }
    let total = lengths[STEPS];
    if total <= 1e-12 {
        return c[0];
    }
    if share <= 0.0 || share >= 1.0 {
        let (end, from) = if share <= 0.0 {
            (c[0], bezier(c, 1.0 / STEPS as f64))
        } else {
            (c[3], bezier(c, 1.0 - 1.0 / STEPS as f64))
        };
        let way = if share <= 0.0 {
            sub(end, from)
        } else {
            sub(from, end)
        };
        let way_length = dot(way, way).sqrt();
        if way_length <= 1e-12 {
            return end;
        }
        let past = if share <= 0.0 { share } else { share - 1.0 };
        return sub(end, scale(way, past * total / way_length));
    }
    let wanted = share * total;
    let k = lengths.partition_point(|l| *l < wanted).clamp(1, STEPS);
    let piece = lengths[k] - lengths[k - 1];
    let into = if piece > 0.0 {
        (wanted - lengths[k - 1]) / piece
    } else {
        0.0
    };
    bezier(c, (k as f64 - 1.0 + into) / STEPS as f64)
}

fn bezier(c: [Point; 4], u: f64) -> Point {
    let v = 1.0 - u;
    let w = [v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u];
    let mut p = [0.0; 3];
    for (axis, out) in p.iter_mut().enumerate() {
        *out = (0..4).map(|n| w[n] * c[n][axis]).sum();
    }
    p
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Point, s: f64) -> Point {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist(a: Point, b: Point) -> f64 {
    let d = sub(b, a);
    dot(d, d).sqrt()
}

fn lerp(a: Point, b: Point, u: f64) -> Point {
    add(a, scale(sub(b, a), u))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::{Keyframe, SideInterp};
    use crate::time::Rational;

    /// Eight linear keys round a circle of radius 100, one a second.
    fn round() -> TransformGroup {
        let axis = |f: fn(f64) -> f64| {
            let keys = (0..=8_i32)
                .map(|k| Keyframe {
                    time: Rational::new(i64::from(k), 1).unwrap(),
                    value: 100.0 * f(f64::from(k) * std::f64::consts::FRAC_PI_4),
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                })
                .collect();
            Property {
                animation: Animation::Keyframed(keys),
                extra: serde_json::Map::new(),
            }
        };
        TransformGroup {
            position_x: axis(f64::cos),
            position_y: axis(f64::sin),
            ..TransformGroup::default()
        }
    }

    fn radius(tr: &TransformGroup, t: f64) -> f64 {
        let (x, y, _) = position(tr, t, true, &|p| p.value_at(t));
        x.hypot(y)
    }

    #[test]
    fn a_curved_path_keeps_a_circle_round_and_a_straight_one_is_unchanged() {
        let mut tr = round();
        // Straight is the chord, as every older project has it.
        assert!((radius(&tr, 2.5) - 100.0 * (std::f64::consts::PI / 8.0).cos()).abs() < 1e-9);
        // An older file has no motion path and reads as straight, and straight
        // writes nothing.
        let json = serde_json::to_value(&tr).unwrap();
        assert!(json.get("motion_path").is_none());
        let back: TransformGroup = serde_json::from_value(json).unwrap();
        assert_eq!(back.motion_path, MotionPath::Straight);

        // Keys round the whole circle close the path, so it is round at its
        // ends too.
        tr.motion_path = MotionPath::Curved;
        for step in 0..=80 {
            let t = f64::from(step) / 10.0;
            let r = radius(&tr, t);
            assert!((r - 100.0).abs() < 0.01, "radius {r} at {t}");
        }
        // Through the keys exactly, and at an even pace along the way.
        let (x, y, _) = position(&tr, 3.0, true, &|p| p.value_at(3.0));
        assert!((x - 100.0 * (3.0 * std::f64::consts::FRAC_PI_4).cos()).abs() < 1e-9);
        assert!((y - 100.0 * (3.0 * std::f64::consts::FRAC_PI_4).sin()).abs() < 1e-9);
        let angle = |t: f64| {
            let (x, y, _) = position(&tr, t, true, &|p| p.value_at(t));
            y.atan2(x).to_degrees()
        };
        assert!((angle(3.5) - 157.5).abs() < 0.5, "{}", angle(3.5));
    }
}

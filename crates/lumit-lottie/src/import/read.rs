//! Reading a Lottie's JSON the way players read it, whatever exporter and
//! version wrote it.
//!
//! # In plain terms
//!
//! A Lottie has been written by many programs over many years, and the old
//! files are still about. A key may carry its value in `s` and the value it
//! runs to in `e` (the old form), or only `s` with the next key holding the
//! end (the new one). Easing handles may be one number or one per dimension.
//! Colours before version 4.1.9 run to 255, and paths before 4.4.18 keep
//! whether they are closed on the shape rather than on the path. This module
//! reads all of them into one form: a [`Track`] of keys in seconds, with each
//! span's handles in the unit square, which is what [`crate::keys`] writes.
//!
//! Nothing here trusts the file: every key and vertex is counted against the
//! import's budget before it is kept.
//!
//! Runs on the import's thread.

use lumit_core::mask::{BezierPath, Vertex};
use lumit_core::time::{FrameRate, Rational};
use lumit_ingress::{Budget, IngressError};
use serde_json::Value;

use crate::keys::{Track, TrackKey, LINEAR_IN, LINEAR_OUT};

/// The file's clock: its rate, exactly, and as a number.
#[derive(Debug, Clone, Copy)]
pub(super) struct Clock {
    pub rate: FrameRate,
    pub fps: f64,
}

impl Clock {
    /// The clock for a file's `fr`, or `None` when it isn't a rate.
    pub fn of(fr: f64) -> Option<Clock> {
        let rate = FrameRate::from_decimal(fr)?;
        Some(Clock {
            rate,
            fps: rate.fps(),
        })
    }

    /// A frame of the file as exact seconds. A time within a millionth of a
    /// frame is that frame; any other stays where it is, on the flick grid,
    /// so a key between frames is never moved onto one.
    pub fn time(&self, frames: f64) -> Rational {
        if !frames.is_finite() {
            return Rational::ZERO;
        }
        let nearest = frames.round();
        if (frames - nearest).abs() <= 1e-6 && nearest.abs() < 9e15 {
            if let Ok(exact) = self.rate.time_of_frame(nearest as i64) {
                return exact.0;
            }
        }
        Rational::from_f64_on_grid(frames / self.fps, Rational::FLICK_DEN).unwrap_or(Rational::ZERO)
    }

    /// Seconds as a number, for sampling.
    pub fn seconds(&self, frames: f64) -> f64 {
        frames / self.fps
    }
}

/// The version that wrote the file, as three numbers. A file that names none
/// is read as new, as players read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Version(pub [u64; 3]);

impl Version {
    pub fn of(v: Option<&Value>) -> Version {
        let Some(text) = v.and_then(Value::as_str) else {
            return Version([100, 100, 100]);
        };
        let mut parts = text
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0));
        Version([
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        ])
    }

    /// True for a file written before `at`.
    pub fn before(self, at: [u64; 3]) -> bool {
        self < Version(at)
    }
}

/// A number: a number, a boolean, or the first of a list.
pub(super) fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Array(list) => num(list.first()),
        _ => None,
    }
}

/// Numbers: a list of them, or one.
pub(super) fn nums(v: Option<&Value>) -> Vec<f64> {
    match v {
        Some(Value::Array(list)) => list.iter().filter_map(|x| num(Some(x))).collect(),
        Some(other) => num(Some(other)).into_iter().collect(),
        None => Vec::new(),
    }
}

/// A text field.
pub(super) fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// A switch written as `1`, `true` or nothing.
pub(super) fn on(v: Option<&Value>) -> bool {
    num(v).is_some_and(|n| n != 0.0)
}

/// A list field, or nothing.
pub(super) fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// A property as read: its keys, the expression it carried, and the spatial
/// tangents of a position whose motion path curves.
#[derive(Debug, Clone)]
pub(super) struct Read {
    pub track: Track,
    pub expression: Option<String>,
    /// Per key, the tangents leaving it (`to`) and arriving at the next (`ti`),
    /// relative to the two values. Empty unless some tangent is not zero.
    pub spatial: Vec<([f64; 2], [f64; 2])>,
}

impl Read {
    /// A still value.
    pub fn still(value: Vec<f64>) -> Read {
        Read {
            track: Track::still(value),
            expression: None,
            spatial: Vec::new(),
        }
    }

    /// True when the value moves.
    pub fn is_animated(&self) -> bool {
        !self.track.is_still()
    }

    /// The value at the first key.
    pub fn first(&self) -> Vec<f64> {
        self.track.first_value()
    }
}

/// One key as the file has it, its value read by the caller.
#[derive(Debug, Clone)]
pub(super) struct RawKey<T> {
    /// Frames on the owner's clock.
    pub t: f64,
    pub value: T,
    pub hold: bool,
    /// Per dimension, x then y, of the handle leaving this key and the one
    /// arriving at the next.
    pub out: (Vec<f64>, Vec<f64>),
    pub inn: (Vec<f64>, Vec<f64>),
    pub to: Vec<f64>,
    pub ti: Vec<f64>,
}

/// True when `k` is a list of keyframes rather than a value.
pub(super) fn is_keyed(k: Option<&Value>) -> bool {
    matches!(k, Some(Value::Array(list)) if list.first().is_some_and(|f| f.is_object() && f.get("t").is_some()))
}

/// The keys of a keyframed `k`, each value read by `value`. The old form's
/// `e` stands in for a key with no `s` of its own, and a key with neither
/// keeps the value before it.
pub(super) fn raw_keys<'a, T: Clone>(
    k: &'a [Value],
    budget: &mut Budget,
    value: impl Fn(&'a Value) -> Option<T>,
) -> Result<Vec<RawKey<T>>, IngressError> {
    budget.take_items(u64::try_from(k.len()).map_err(|_| IngressError::Overflow)?)?;
    let mut out: Vec<RawKey<T>> = budget.vec_with_capacity(k.len())?;
    let mut carried: Option<T> = None;
    for key in k {
        let Some(t) = num(key.get("t")) else {
            continue;
        };
        let v = key
            .get("s")
            .and_then(&value)
            .or_else(|| carried.clone())
            .or_else(|| out.last().map(|last| last.value.clone()));
        carried = key.get("e").and_then(&value);
        let Some(v) = v else {
            continue;
        };
        let handle = |side: &str, axis: &str| nums(key.get(side).and_then(|h| h.get(axis)));
        out.push(RawKey {
            t,
            value: v,
            hold: on(key.get("h")),
            out: (handle("o", "x"), handle("o", "y")),
            inn: (handle("i", "x"), handle("i", "y")),
            to: nums(key.get("to")),
            ti: nums(key.get("ti")),
        });
    }
    Ok(out)
}

/// Dimension `d` of a handle list: its own entry, else the first, as
/// lottie-web reads it.
pub(super) fn pick(list: &[f64], d: usize, fallback: f64) -> f64 {
    list.get(d)
        .or_else(|| list.first())
        .copied()
        .unwrap_or(fallback)
}

/// A numeric property as a track on the owner's clock, its values put
/// through `map` (a colour's 0..255, say). `None` when there's no property.
///
/// A key's `t` is a frame of the comp the layer sits in, whatever the
/// layer's start, as lottie-web reads it; `offset` is that start (`st`), so
/// the track's seconds are on the layer's own clock, as Lumit keeps them.
pub(super) fn property(
    p: Option<&Value>,
    clock: &Clock,
    offset: f64,
    budget: &mut Budget,
    map: &dyn Fn(f64) -> f64,
) -> Result<Option<Read>, IngressError> {
    let Some(p) = p else {
        return Ok(None);
    };
    // A bare value where a property belongs: old text fields wrote these.
    let (k, expression) = match p {
        Value::Object(_) => (
            p.get("k"),
            p.get("x")
                .and_then(Value::as_str)
                .filter(|x| !x.trim().is_empty())
                .map(str::to_owned),
        ),
        other => (Some(other), None),
    };
    let Some(k) = k else {
        return Ok(None);
    };
    if !is_keyed(Some(k)) {
        let values: Vec<f64> = nums(Some(k)).into_iter().map(map).collect();
        if values.is_empty() {
            return Ok(None);
        }
        let mut read = Read::still(values);
        read.expression = expression;
        return Ok(Some(read));
    }
    let keys = raw_keys(k.as_array().map_or(&[], Vec::as_slice), budget, |v| {
        let n = nums(Some(v));
        (!n.is_empty()).then_some(n)
    })?;
    let Some(first) = keys.first() else {
        return Ok(None);
    };
    let dims = first.value.len();
    let mut track = Track { keys: Vec::new() };
    let mut spatial = Vec::with_capacity(keys.len());
    let mut curved = false;
    for (j, key) in keys.iter().enumerate() {
        let last = j + 1 == keys.len();
        let value: Vec<f64> = (0..dims)
            .map(|d| map(key.value.get(d).copied().unwrap_or(0.0)))
            .collect();
        let (out, inn) = if last || key.hold {
            (Vec::new(), Vec::new())
        } else {
            (0..dims)
                .map(|d| {
                    (
                        [
                            pick(&key.out.0, d, LINEAR_OUT[0]),
                            pick(&key.out.1, d, LINEAR_OUT[1]),
                        ],
                        [
                            pick(&key.inn.0, d, LINEAR_IN[0]),
                            pick(&key.inn.1, d, LINEAR_IN[1]),
                        ],
                    )
                })
                .unzip()
        };
        let pair = |l: &[f64]| {
            [
                l.first().copied().unwrap_or(0.0),
                l.get(1).copied().unwrap_or(0.0),
            ]
        };
        let (to, ti) = (pair(&key.to), pair(&key.ti));
        curved |= !last && to.iter().chain(&ti).any(|v| v.abs() > 1e-9);
        spatial.push((to, ti));
        track.keys.push(TrackKey {
            seconds: clock.seconds(key.t - offset),
            value,
            hold: key.hold && !last,
            out,
            inn,
        });
    }
    Ok(Some(Read {
        track,
        expression,
        spatial: if curved { spatial } else { Vec::new() },
    }))
}

/// A path value: `{c, v, i, o}`, or a key's one-path list. `closed` is what an
/// old file said on the shape or the mask instead.
pub(super) fn bezier(
    v: &Value,
    closed: Option<bool>,
    budget: &mut Budget,
) -> Result<Option<BezierPath>, IngressError> {
    let v = match v {
        Value::Array(list) => match list.first() {
            Some(first) => first,
            None => return Ok(None),
        },
        other => other,
    };
    let points = list(v, "v");
    budget.take_items(u64::try_from(points.len()).map_err(|_| IngressError::Overflow)?)?;
    let at = |key: &str, n: usize| {
        let p = nums(list(v, key).get(n));
        (
            p.first().copied().unwrap_or(0.0),
            p.get(1).copied().unwrap_or(0.0),
        )
    };
    let mut vertices = budget.vec_with_capacity(points.len())?;
    vertices.extend((0..points.len()).map(|n| Vertex {
        pos: at("v", n),
        tan_in: at("i", n),
        tan_out: at("o", n),
    }));
    let closed = v
        .get("c")
        .and_then(Value::as_bool)
        .or(closed)
        .unwrap_or(false);
    Ok(Some(BezierPath { vertices, closed }))
}

/// A solid's `#rrggbb` as display numbers from 0 to 1.
pub(super) fn hex(code: &str) -> Option<[f64; 3]> {
    let code = code.trim().trim_start_matches('#');
    let channel = |n: usize| {
        code.get(n * 2..n * 2 + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(|b| f64::from(b) / 255.0)
    };
    Some([channel(0)?, channel(1)?, channel(2)?])
}

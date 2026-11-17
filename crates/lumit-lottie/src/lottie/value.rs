//! Numbers and animated values as a Lottie writes them.
//!
//! Every number goes through [`Fmt::num`], which rounds it for its kind:
//! three decimals, or with Minify two for pixels and degrees and three for
//! anything from 0 to 1. Every property goes through a [`Clock`], which reads
//! it on its layer's own time: keys are written exactly ([`keys`]), and what
//! only has a value per frame (an expression, a colour Lumit mixes in linear
//! light) is sampled on the output's frames and thinned ([`fit`]).
//!
//! Runs on the export's thread.

use std::cell::Cell;
use std::sync::Arc;

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::expression::ExpressionContext;
use lumit_core::mask::{lerp_paths, resample, BezierPath, PathKeyframe, Vertex};
use lumit_core::model::{Composition, Document, Layer};
use serde_json::{json, Map, Value};

use crate::fit;
use crate::keys::{self, Curve, Span, Track};
use crate::time::Timebase;
use crate::Host;

/// What kind of number a value is, which decides how finely it is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Pixels, per cent and anything else a person reads in whole units.
    Px,
    /// Degrees.
    Deg,
    /// A number from 0 to 1: a colour channel, an easing handle.
    Unit,
    /// Frames and seconds.
    Time,
}

/// How numbers are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fmt {
    pub minify: bool,
}

impl Fmt {
    /// `v` rounded for its class, as a JSON number.
    #[must_use]
    pub fn num(self, v: f64, class: Class) -> Value {
        let decimals = match class {
            Class::Px | Class::Deg if self.minify => 2,
            _ => 3,
        };
        number(v, decimals)
    }

    /// Several numbers of one class, as an array.
    #[must_use]
    pub fn nums(self, v: &[f64], class: Class) -> Value {
        Value::Array(v.iter().map(|x| self.num(*x, class)).collect())
    }

    /// A name, or nothing once minified.
    pub fn name(self, map: &mut Map<String, Value>, name: &str) {
        if !self.minify {
            map.insert("nm".into(), Value::from(name));
        }
    }
}

/// `v` rounded to `decimals`, written as an integer when it is one, so `100`
/// is not `100.0`. A number that isn't finite is written as 0.
#[must_use]
pub fn number(v: f64, decimals: i32) -> Value {
    if !v.is_finite() {
        return Value::from(0);
    }
    let p = 10f64.powi(decimals);
    let r = (v * p).round() / p;
    if r == 0.0 {
        return Value::from(0);
    }
    if r.fract() == 0.0 && r.abs() < 9e15 {
        // Whole and well inside i64: the cast is exact.
        return Value::from(r as i64);
    }
    Value::from(r)
}

/// A property's numbers over time, read the way the renderer reads it: on its
/// layer's own clock, with its expressions run in the layer's context.
pub struct Clock<'a> {
    pub doc: &'a Arc<Document>,
    pub comp: &'a Composition,
    pub layer: &'a Layer,
    /// The clock of the comp the layer is in.
    pub tb: &'a Timebase,
    pub host: &'a dyn Host,
    pub fmt: Fmt,
    /// The comp seconds sampled values are read across: the layer's own
    /// span in the file unless a wider one is asked for.
    pub span: (f64, f64),
    /// Set once something had to be sampled because an expression drives it.
    pub baked: Cell<bool>,
    /// Set once a value with several numbers was written with an easing for
    /// each of them.
    pub split: Cell<bool>,
}

impl<'a> Clock<'a> {
    #[must_use]
    pub fn new(
        doc: &'a Arc<Document>,
        comp: &'a Composition,
        layer: &'a Layer,
        tb: &'a Timebase,
        host: &'a dyn Host,
        fmt: Fmt,
    ) -> Self {
        let from = layer.in_point.0.to_f64().max(tb.start);
        Self {
            doc,
            comp,
            layer,
            tb,
            host,
            fmt,
            span: (from, layer.out_point.0.to_f64().min(tb.end).max(from)),
            baked: Cell::new(false),
            split: Cell::new(false),
        }
    }

    /// The layer's start offset in seconds: layer time 0 on the comp's clock.
    #[must_use]
    pub fn offset(&self) -> f64 {
        self.layer.start_offset.0.to_f64()
    }

    /// Layer seconds as the comp's output frames, which is what a Lottie key
    /// time counts in.
    #[must_use]
    pub fn frame(&self, lt: f64) -> f64 {
        self.tb.frame(lt + self.offset())
    }

    /// The layer times a sampled property is read at: every output frame
    /// across [`Self::span`], both ends kept.
    #[must_use]
    pub fn times(&self) -> Vec<f64> {
        let offset = self.offset();
        self.tb
            .grid(self.span.0, self.span.1)
            .into_iter()
            .map(|f| self.tb.seconds(f) - offset)
            .collect()
    }

    /// What the layer's expressions read at layer time `lt`.
    #[must_use]
    pub fn context(&self, lt: f64) -> Arc<ExpressionContext> {
        Arc::new(ExpressionContext {
            document: self.doc.clone(),
            comp: Some(self.comp.id),
            layer: Some(self.layer.id),
            comp_time: lt + self.offset(),
            current_depth: 0,
            inputs: None,
        })
    }

    /// `p` at layer time `lt`.
    #[must_use]
    pub fn value(&self, p: &Property, lt: f64) -> f64 {
        match p.animation {
            Animation::Expression(_) => p.value_at_with_context(lt, self.context(lt)),
            _ => p.value_at(lt),
        }
    }

    /// `f` read at every sampled time and thinned to the fewest straight
    /// keys within `tolerance`. Stops early once the export is cancelled; the
    /// caller notices between layers.
    pub fn sampled(&self, f: impl Fn(f64) -> Vec<f64>, tolerance: &[f64]) -> Track {
        let mut samples = Vec::new();
        for lt in self.times() {
            if self.host.cancelled() {
                break;
            }
            samples.push((lt, f(lt)));
        }
        fit::linear(&samples, tolerance)
    }

    /// One or more numbers as one track: their keys exactly where they all
    /// have keys a Lottie can hold, sampled where they can't be merged or an
    /// expression drives one of them.
    pub fn track(&self, props: &[&Property], tolerance: &[f64]) -> Track {
        let curves: Option<Vec<Curve>> = props.iter().map(|p| curve(p)).collect();
        if let Some(curves) = curves {
            if let Ok(track) = keys::track(&curves) {
                return track;
            }
        } else {
            self.baked.set(true);
        }
        self.sampled(
            |lt| props.iter().map(|p| self.value(p, lt)).collect(),
            tolerance,
        )
    }

    /// True when `p` moves at all.
    #[must_use]
    pub fn moves(p: &Property) -> bool {
        match &p.animation {
            Animation::Static(_) => false,
            Animation::Keyframed(keys) => keys.windows(2).any(|w| match w {
                [a, b] => a.value != b.value,
                _ => false,
            }),
            Animation::Expression(_) => true,
        }
    }

    /// A track as a Lottie property: `{a: 0, k}` when it never moves, else
    /// its keys. `vector` writes the value as an array even with one number;
    /// `pad` is appended to every value (a scale's third number).
    #[must_use]
    pub fn write(&self, track: &Track, class: Class, vector: bool, pad: &[f64]) -> Value {
        let fmt = self.fmt;
        let full = |v: &[f64]| {
            let mut all = v.to_vec();
            all.extend_from_slice(pad);
            all
        };
        if track.keys.len() <= 1 || track.is_still() {
            let v = full(&track.first_value());
            let k = match (vector, v.as_slice()) {
                (false, [one]) => fmt.num(*one, class),
                _ => fmt.nums(&v, class),
            };
            return json!({"a": 0, "k": k});
        }
        let last = track.keys.len() - 1;
        let mut out = Vec::with_capacity(track.keys.len());
        for (n, key) in track.keys.iter().enumerate() {
            let mut k = Map::new();
            k.insert("t".into(), fmt.num(self.frame(key.seconds), Class::Time));
            k.insert("s".into(), fmt.nums(&full(&key.value), class));
            if n < last {
                if key.hold {
                    k.insert("h".into(), Value::from(1));
                } else {
                    let (o, i) = (handles(&key.out), handles(&key.inn));
                    if o.len() > 1 || i.len() > 1 {
                        self.split.set(true);
                    }
                    k.insert("o".into(), easing(fmt, &o));
                    k.insert("i".into(), easing(fmt, &i));
                }
            }
            out.push(Value::Object(k));
        }
        json!({"a": 1, "k": out})
    }

    /// `props` as one Lottie property, keyed or sampled as [`Self::track`]
    /// decides.
    #[must_use]
    pub fn prop(&self, props: &[&Property], class: Class, tolerance: f64) -> Value {
        let track = self.track(props, &vec![tolerance; props.len()]);
        self.write(&track, class, props.len() > 1, &[])
    }

    /// A scalar property with every value put through `f`, an affine map
    /// that keeps the easing (per cent to a fraction, a shift).
    #[must_use]
    pub fn prop_mapped(
        &self,
        p: &Property,
        class: Class,
        tolerance: f64,
        f: impl Fn(f64) -> f64,
    ) -> Value {
        let track = self.track(&[p], &[tolerance]).map_affine(|_, v| f(v));
        self.write(&track, class, false, &[])
    }

    /// A colour in working linear light as the display sRGB a Lottie states,
    /// `[r, g, b]` from 0 to 1. A still one is converted once; a moving one
    /// is sampled each frame in linear light, converted and thinned, so the
    /// file shows the colours Lumit passes through.
    #[must_use]
    pub fn colour(&self, rgb: [&Property; 3]) -> Value {
        self.colour_padded(rgb, &[])
    }

    /// [`Self::colour`] with `pad` after the three channels (an effect's
    /// colour carries an alpha of 1).
    #[must_use]
    pub fn colour_padded(&self, rgb: [&Property; 3], pad: &[f64]) -> Value {
        if rgb
            .iter()
            .all(|p| matches!(p.animation, Animation::Static(_)))
        {
            let mut shown = self.host.display(rgb.map(|p| p.value_at(0.0))).to_vec();
            shown.extend_from_slice(pad);
            return json!({"a": 0, "k": self.fmt.nums(&shown, Class::Unit)});
        }
        if rgb
            .iter()
            .any(|p| matches!(p.animation, Animation::Expression(_)))
        {
            self.baked.set(true);
        }
        let track = self.sampled(
            |lt| self.host.display(rgb.map(|p| self.value(p, lt))).to_vec(),
            &[fit::tolerance::CHANNEL; 3],
        );
        self.write(&track, Class::Unit, true, pad)
    }

    /// A path as a Lottie shape property, `offset` added to every point.
    #[must_use]
    pub fn path(&self, path: &BezierPath, keys: &[PathKeyframe], offset: (f64, f64)) -> Value {
        if keys.len() < 2 {
            let still = keys.first().map_or(path, |k| &k.path);
            return json!({"a": 0, "k": bezier(self.fmt, still, offset)});
        }
        let n = keys
            .iter()
            .map(|k| k.path.vertices.len())
            .max()
            .unwrap_or(0);
        // Lumit blends each two keys at the larger of their own vertex
        // counts. Each pair is resampled so, then padded to the most any key
        // has, so every key a player reads has as many vertices.
        let pairs: Vec<(BezierPath, BezierPath)> = keys
            .windows(2)
            .filter_map(|w| {
                let [a, b] = w else { return None };
                let m = a.path.vertices.len().max(b.path.vertices.len());
                Some((
                    padded(resample(&a.path, m), n),
                    padded(resample(&b.path, m), n),
                ))
            })
            .collect();
        let times: Vec<f64> = keys.iter().map(|k| k.time.to_f64()).collect();
        // Each span eases 0 to 1 between its two keys, as Lumit's morph does;
        // laid end to end the value counts the keys passed, so one curve
        // carries every span's easing exactly.
        let mut spans = Vec::new();
        for (i, pair) in keys.windows(2).enumerate() {
            let [a, b] = pair else { continue };
            let ramp = [ramp_key(a, 0.0, true), ramp_key(b, 1.0, false)];
            let lift = i as f64;
            spans.extend(keys::curve(&ramp).spans.into_iter().map(|s| match s {
                Span::Hold { t0, t1, v } => Span::Hold {
                    t0,
                    t1,
                    v: v + lift,
                },
                Span::Cubic { x, y } => Span::Cubic {
                    x,
                    y: y.map(|v| v + lift),
                },
            }));
        }
        let curve = Curve {
            spans,
            first: 0.0,
            last: (keys.len() - 1) as f64,
        };
        let Ok(track) = keys::track(&[curve]) else {
            return json!({"a": 0, "k": bezier(self.fmt, path, offset)});
        };
        let at = |seconds: f64, v: f64| {
            // The span the key sits in, by time, so a value an overshoot
            // carries past the next key still blends this span's two shapes.
            let span = times
                .partition_point(|t| *t <= seconds + 1e-9)
                .saturating_sub(1)
                .min(pairs.len().saturating_sub(1));
            match pairs.get(span) {
                Some((a, b)) => lerp_paths(a, b, v - span as f64),
                None => path.clone(),
            }
        };
        // Where a key ends one pair on other vertices than it starts the
        // next, the end is held for no time just before it.
        let ends = |seconds: f64| {
            let k = times.iter().position(|t| (t - seconds).abs() <= 1e-9)?;
            let (end, start) = (&pairs.get(k.checked_sub(1)?)?.1, &pairs.get(k)?.0);
            (end != start).then(|| end.clone())
        };
        self.path_keys(&track, at, ends, offset)
    }

    /// Paths read at every sampled time, as straight keys. For a path no key
    /// list describes: a parametric shape whose numbers move, written as a
    /// path because a trim reaches it.
    #[must_use]
    pub fn path_sampled(&self, f: impl Fn(f64) -> BezierPath, offset: (f64, f64)) -> Value {
        let times = self.times();
        let paths: Vec<BezierPath> = times
            .iter()
            .take_while(|_| !self.host.cancelled())
            .map(|lt| f(*lt))
            .collect();
        let n = paths.iter().map(|p| p.vertices.len()).max().unwrap_or(0);
        let closed = paths.first().is_some_and(|p| p.closed);
        let flat: Vec<(f64, Vec<f64>)> = times
            .iter()
            .zip(&paths)
            .map(|(lt, p)| {
                let p = resample(p, n);
                let numbers = p
                    .vertices
                    .iter()
                    .flat_map(|v| {
                        [
                            v.pos.0,
                            v.pos.1,
                            v.tan_in.0,
                            v.tan_in.1,
                            v.tan_out.0,
                            v.tan_out.1,
                        ]
                    })
                    .collect();
                (*lt, numbers)
            })
            .collect();
        let dims = flat.first().map_or(0, |s| s.1.len());
        let track = fit::linear(&flat, &vec![fit::tolerance::PIXELS; dims]);
        let at = |_: f64, v: &[f64]| BezierPath {
            vertices: v
                .chunks_exact(6)
                .map(|c| lumit_core::mask::Vertex {
                    pos: (c[0], c[1]),
                    tan_in: (c[2], c[3]),
                    tan_out: (c[4], c[5]),
                })
                .collect(),
            closed,
        };
        if track.keys.len() <= 1 {
            let first = track.first_value();
            return json!({"a": 0, "k": bezier(self.fmt, &at(0.0, &first), offset)});
        }
        let last = track.keys.len() - 1;
        let keys: Vec<Value> = track
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let mut m = Map::new();
                m.insert("t".into(), self.fmt.num(self.frame(k.seconds), Class::Time));
                m.insert(
                    "s".into(),
                    json!([bezier(self.fmt, &at(k.seconds, &k.value), offset)]),
                );
                if i < last {
                    // Straight between samples.
                    m.insert("o".into(), json!({"x": 0, "y": 0}));
                    m.insert("i".into(), json!({"x": 1, "y": 1}));
                }
                Value::Object(m)
            })
            .collect();
        json!({"a": 1, "k": keys})
    }

    /// A one-number track whose values pick shapes through `at`, as Lottie
    /// shape keys with scalar easing. `ends` gives a shape to hold for no
    /// time just before a key.
    fn path_keys(
        &self,
        track: &Track,
        at: impl Fn(f64, f64) -> BezierPath,
        ends: impl Fn(f64) -> Option<BezierPath>,
        offset: (f64, f64),
    ) -> Value {
        let last = track.keys.len().saturating_sub(1);
        let keys: Vec<Value> = track
            .keys
            .iter()
            .enumerate()
            .flat_map(|(i, k)| {
                let v = k.value.first().copied().unwrap_or(0.0);
                let t = self.fmt.num(self.frame(k.seconds), Class::Time);
                let end = ends(k.seconds)
                    .map(|end| json!({"t": t, "s": [bezier(self.fmt, &end, offset)], "h": 1}));
                let mut m = Map::new();
                m.insert("t".into(), t);
                m.insert(
                    "s".into(),
                    json!([bezier(self.fmt, &at(k.seconds, v), offset)]),
                );
                if i < last {
                    if k.hold {
                        m.insert("h".into(), Value::from(1));
                    } else {
                        let o = k.out.first().copied().unwrap_or(keys::LINEAR_OUT);
                        let n = k.inn.first().copied().unwrap_or(keys::LINEAR_IN);
                        m.insert("o".into(), json!({"x": o[0], "y": o[1]}));
                        m.insert("i".into(), json!({"x": n[0], "y": n[1]}));
                        round_easing(self.fmt, &mut m);
                    }
                }
                end.into_iter().chain([Value::Object(m)])
            })
            .collect();
        json!({"a": 1, "k": keys})
    }
}

/// The curve `p` follows, or `None` when an expression drives it.
#[must_use]
pub fn curve(p: &Property) -> Option<Curve> {
    match &p.animation {
        Animation::Static(v) => Some(Curve {
            spans: Vec::new(),
            first: *v,
            last: *v,
        }),
        Animation::Keyframed(k) => Some(keys::curve(k)),
        Animation::Expression(_) => None,
    }
}

/// A path key's timing as a scalar key, for the 0 to 1 ramp of its span.
fn ramp_key(k: &PathKeyframe, value: f64, first: bool) -> Keyframe {
    // A ramp of two keys has no neighbours, so the outer sides never matter;
    // they are made straight so an automatic side can't read them.
    let (interp_in, interp_out) = if first {
        (SideInterp::Linear, k.interp_out)
    } else {
        (k.interp_in, SideInterp::Linear)
    };
    Keyframe {
        time: k.time,
        value,
        interp_in,
        interp_out,
    }
}

/// One handle per number, or one for all of them when they agree.
fn handles(h: &[[f64; 2]]) -> Vec<[f64; 2]> {
    match h.split_first() {
        Some((first, rest)) if rest.iter().all(|x| x == first) => vec![*first],
        Some(_) => h.to_vec(),
        None => vec![keys::LINEAR_OUT],
    }
}

/// An easing handle as Lottie's `{x: [..], y: [..]}`.
fn easing(fmt: Fmt, h: &[[f64; 2]]) -> Value {
    let x: Vec<f64> = h.iter().map(|p| p[0]).collect();
    let y: Vec<f64> = h.iter().map(|p| p[1]).collect();
    json!({"x": fmt.nums(&x, Class::Unit), "y": fmt.nums(&y, Class::Unit)})
}

/// Rounds the scalar easing handles of a shape key in place.
fn round_easing(fmt: Fmt, m: &mut Map<String, Value>) {
    for side in ["o", "i"] {
        if let Some(Value::Object(h)) = m.get_mut(side) {
            for axis in ["x", "y"] {
                if let Some(v) = h.get(axis).and_then(Value::as_f64) {
                    h.insert(axis.into(), fmt.num(v, Class::Unit));
                }
            }
        }
    }
}

/// `path` with its last vertex repeated until it has `n`: the same curve,
/// with as many vertices as a fuller key.
fn padded(mut path: BezierPath, n: usize) -> BezierPath {
    let Some(last) = path.vertices.last().cloned() else {
        return path;
    };
    if path.vertices.len() >= n {
        return path;
    }
    let still = (0.0, 0.0);
    if let Some(v) = path.vertices.last_mut() {
        v.tan_out = still;
    }
    while path.vertices.len() + 1 < n {
        path.vertices.push(Vertex {
            pos: last.pos,
            tan_in: still,
            tan_out: still,
        });
    }
    path.vertices.push(Vertex {
        pos: last.pos,
        tan_in: still,
        tan_out: last.tan_out,
    });
    path
}

/// `path` moved by `d`.
#[must_use]
pub fn moved(mut path: BezierPath, d: (f64, f64)) -> BezierPath {
    for v in &mut path.vertices {
        v.pos = (v.pos.0 + d.0, v.pos.1 + d.1);
    }
    path
}

/// A path as Lottie's bezier object, `offset` added to every point.
#[must_use]
pub fn bezier(fmt: Fmt, path: &BezierPath, offset: (f64, f64)) -> Value {
    let pt = |p: (f64, f64)| fmt.nums(&[p.0, p.1], Class::Px);
    let v: Vec<Value> = path
        .vertices
        .iter()
        .map(|v| pt((v.pos.0 + offset.0, v.pos.1 + offset.1)))
        .collect();
    let i: Vec<Value> = path.vertices.iter().map(|v| pt(v.tan_in)).collect();
    let o: Vec<Value> = path.vertices.iter().map(|v| pt(v.tan_out)).collect();
    json!({"c": path.closed, "v": v, "i": i, "o": o})
}

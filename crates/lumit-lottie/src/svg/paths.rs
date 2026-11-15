//! Paths as `d`: still, morphing between keys, or worked out frame by frame.
//!
//! A morph animates `d` itself, which SMIL allows only when every value has
//! the same commands. So every key of a morph is resampled to the most
//! vertices any key has ([`lumit_core::mask::resample`], which splits curves
//! without moving them) and every segment written as a cubic. The easing
//! between keys is the keys' own, exactly, as for any number.
//!
//! What has no keys to write (a parametric shape whose numbers move, a trim
//! that cuts a fill, a Lumit-only modifier) is drawn frame by frame: as
//! values in step when the outline keeps its commands, else as steps.
//!
//! Runs on the export's thread.

use lumit_core::anim::Keyframe;
use lumit_core::mask::{BezierPath, PathKeyframe};
use lumit_core::model::Layer;
use lumit_core::shape::primitive::ShapePrimitive;

use super::anim::{self, Clock, Ease, Seg, Value, LINEAR};
use super::{Scope, Svg};
use crate::fit::tolerance;

/// A path as the file writes it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PathValue {
    /// The path's numbers ([`numbers`]), still or animated, the commands the
    /// same throughout.
    Curves { value: Value, closed: bool },
    /// A `d` per change, from its root second on.
    Steps(Vec<(f64, String)>),
}

/// A path's numbers: per vertex its position, then its two handles as
/// points, all in the path's own coordinates.
#[must_use]
pub(super) fn numbers(path: &BezierPath) -> Vec<f64> {
    let mut out = Vec::with_capacity(path.vertices.len() * 6);
    for v in &path.vertices {
        let (x, y) = v.pos;
        out.extend([
            x,
            y,
            x + v.tan_in.0,
            y + v.tan_in.1,
            x + v.tan_out.0,
            y + v.tan_out.1,
        ]);
    }
    out
}

impl Svg<'_> {
    /// The `d` of [`numbers`]: every segment a cubic when `curves`, which an
    /// animated `d` needs, else straight lines where the handles lie on the
    /// vertices.
    pub(super) fn d(&self, n: &[f64], closed: bool, curves: bool) -> String {
        let count = n.len() / 6;
        if count == 0 {
            return String::from("M0,0");
        }
        let at = |i: usize, k: usize| n.get(i * 6 + k).copied().unwrap_or(0.0);
        let mut d = format!("M{},{}", self.px(at(0, 0)), self.px(at(0, 1)));
        let segments = if closed { count } else { count - 1 };
        for i in 0..segments {
            let j = (i + 1) % count;
            let straight = !curves
                && (at(i, 4) - at(i, 0)).abs() < 1e-9
                && (at(i, 5) - at(i, 1)).abs() < 1e-9
                && (at(j, 2) - at(j, 0)).abs() < 1e-9
                && (at(j, 3) - at(j, 1)).abs() < 1e-9;
            if straight {
                d.push_str(&format!("L{},{}", self.px(at(j, 0)), self.px(at(j, 1))));
            } else {
                d.push_str(&format!(
                    "C{},{} {},{} {},{}",
                    self.px(at(i, 4)),
                    self.px(at(i, 5)),
                    self.px(at(j, 2)),
                    self.px(at(j, 3)),
                    self.px(at(j, 0)),
                    self.px(at(j, 1))
                ));
            }
        }
        if closed {
            d.push('Z');
        }
        d
    }

    /// A path from polylines, each a run of points: what a bake hands back.
    pub(super) fn polyline_d(&self, lines: &[Vec<(f64, f64)>], closed: bool) -> String {
        let mut d = String::new();
        for line in lines {
            let line = thinned(line, BAKE_TOLERANCE);
            for (k, (x, y)) in line.iter().enumerate() {
                d.push(if k == 0 { 'M' } else { 'L' });
                d.push_str(&format!("{},{}", self.px(*x), self.px(*y)));
            }
            if closed && line.len() > 2 {
                d.push('Z');
            }
        }
        if d.is_empty() {
            d.push_str("M0,0");
        }
        d
    }

    /// The path a shape item or a mask draws, on `layer`'s clock.
    pub(super) fn path_value(
        &self,
        scope: &Scope<'_>,
        layer: &Layer,
        keys: &[PathKeyframe],
        still: &BezierPath,
        primitive: Option<&ShapePrimitive>,
    ) -> PathValue {
        if let Some(p) = primitive {
            if !p.is_animated() {
                return curves_of(&p.path_at(0.0));
            }
            if let Clock::Shift(shift) = Svg::layer_clock(scope, layer) {
                if let Some(keyed) = keyed_primitive(p, shift) {
                    return keyed;
                }
            }
            return self.sampled_path(scope, layer, &|lt| p.path_at(lt));
        }
        let (Some(first), true) = (keys.first(), keys.len() > 1) else {
            return curves_of(keys.first().map_or(still, |k| &k.path));
        };
        let most = keys
            .iter()
            .map(|k| k.path.vertices.len())
            .max()
            .unwrap_or(0);
        let closed = first.path.closed;
        if keys.iter().any(|k| k.path.closed != closed) {
            return self.sampled_path(scope, layer, &|lt| {
                lumit_core::mask::path_at(keys, still, lt).into_owned()
            });
        }
        match Svg::layer_clock(scope, layer) {
            Clock::Shift(shift) => PathValue::Curves {
                value: keyed(keys, most, shift),
                closed,
            },
            Clock::Map(_) => self.sampled_path(scope, layer, &|lt| {
                lumit_core::mask::resample(&lumit_core::mask::path_at(keys, still, lt), most)
            }),
        }
    }

    /// A path worked out at every frame: values in step when its commands
    /// never change, steps of `d` when they do.
    pub(super) fn sampled_path(
        &self,
        scope: &Scope<'_>,
        layer: &Layer,
        at: &dyn Fn(f64) -> BezierPath,
    ) -> PathValue {
        let clock = Svg::layer_clock(scope, layer);
        let paths: Vec<BezierPath> = self
            .span
            .grid
            .iter()
            .enumerate()
            .map(|(i, t)| at(clock.at(i, *t)))
            .collect();
        let Some(first) = paths.first() else {
            return PathValue::Steps(Vec::new());
        };
        let same = paths
            .iter()
            .all(|p| p.closed == first.closed && p.vertices.len() == first.vertices.len());
        if same {
            let samples: Vec<Vec<f64>> = paths.iter().map(numbers).collect();
            let tol = vec![tolerance::PIXELS; samples.first().map_or(0, Vec::len)];
            return PathValue::Curves {
                value: Value::sampled(&self.span.grid, samples, &tol),
                closed: first.closed,
            };
        }
        let mut steps: Vec<(f64, String)> = Vec::new();
        for (t, p) in self.span.grid.iter().zip(&paths) {
            let d = self.d(&numbers(p), p.closed, false);
            if steps.last().is_none_or(|(_, last)| *last != d) {
                steps.push((*t, d));
            }
        }
        PathValue::Steps(steps)
    }

    /// The base `d` of `path`, for the tag.
    pub(super) fn path_base(&self, path: &PathValue) -> String {
        match path {
            PathValue::Curves { value, closed } => {
                self.d(&value.at(self.span.start), *closed, !value.is_still())
            }
            PathValue::Steps(steps) => steps.first().map(|(_, d)| d.clone()).unwrap_or_default(),
        }
    }

    /// The `<animate attributeName="d">` of `path`, when it moves.
    pub(super) fn path_animation(&self, out: &mut String, path: &PathValue) {
        match path {
            PathValue::Curves { value, closed } => {
                let n = match value {
                    Value::Still(_) => return,
                    Value::Moving(segs) => segs.first().map_or(0, |s| s.v0.len()),
                };
                let tol = vec![tolerance::PIXELS; n];
                if let Some(smil) = anim::smil(value, &self.span, &tol) {
                    anim::write(out, "d", None, &smil, &self.span, &|v| {
                        self.d(v, *closed, true)
                    });
                }
            }
            PathValue::Steps(steps) => {
                let times: Vec<f64> = steps.iter().map(|(t, _)| *t).collect();
                let values: Vec<String> = steps.iter().map(|(_, d)| d.clone()).collect();
                anim::write_steps(out, "d", &times, &values, &self.span);
            }
        }
    }
}

/// A parametric shape whose numbers are keyed, as keys of its path, when
/// that is exact: every number that moves across a span eases the same way,
/// and the path moves in a straight line as the numbers do (a rectangle
/// growing, a star's radii), which is checked part way across each span.
/// `None` sends the caller to sampling.
fn keyed_primitive(p: &ShapePrimitive, shift: f64) -> Option<PathValue> {
    let props = [
        &p.position_x,
        &p.position_y,
        &p.width,
        &p.height,
        &p.roundness,
        &p.points,
        &p.rotation,
        &p.outer_radius,
        &p.inner_radius,
        &p.outer_roundness,
        &p.inner_roundness,
    ];
    if props
        .iter()
        .any(|q| matches!(q.animation, lumit_core::anim::Animation::Expression(_)))
    {
        return None;
    }
    let track =
        crate::keys::track(&props.iter().map(|q| super::curve(q)).collect::<Vec<_>>()).ok()?;
    let shape = |v: &[f64]| -> BezierPath {
        let mut q = p.clone();
        let fields = [
            &mut q.position_x,
            &mut q.position_y,
            &mut q.width,
            &mut q.height,
            &mut q.roundness,
            &mut q.points,
            &mut q.rotation,
            &mut q.outer_radius,
            &mut q.inner_radius,
            &mut q.outer_roundness,
            &mut q.inner_roundness,
        ];
        for (field, value) in fields.into_iter().zip(v) {
            *field = lumit_core::anim::Property::fixed(*value);
        }
        q.path_at(0.0)
    };
    let first = shape(&track.first_value());
    let mut segs = Vec::with_capacity(track.keys.len());
    for pair in track.keys.windows(2) {
        let [a, b] = pair else { continue };
        let (pa, pb) = (shape(&a.value), shape(&b.value));
        if pa.vertices.len() != first.vertices.len() || pb.vertices.len() != first.vertices.len() {
            return None;
        }
        let (na, nb) = (numbers(&pa), numbers(&pb));
        if a.hold {
            segs.push(Seg {
                t0: a.seconds + shift,
                t1: b.seconds + shift,
                v0: na,
                v1: nb,
                ease: Ease::Hold,
            });
            continue;
        }
        let moving: Vec<usize> = (0..a.value.len())
            .filter(|d| {
                (a.value.get(*d).copied().unwrap_or(0.0) - b.value.get(*d).copied().unwrap_or(0.0))
                    .abs()
                    > 1e-12
            })
            .collect();
        let handle = |d: usize| -> [f64; 4] {
            let o = a.out.get(d).copied().unwrap_or(crate::keys::LINEAR_OUT);
            let i = a.inn.get(d).copied().unwrap_or(crate::keys::LINEAR_IN);
            [o[0], o[1], i[0], i[1]]
        };
        let spline = moving.first().map_or(LINEAR, |d| handle(*d));
        if moving.iter().any(|d| {
            handle(*d)
                .iter()
                .zip(spline)
                .any(|(x, y)| (x - y).abs() > 1e-9)
        }) {
            return None;
        }
        // Straight in the numbers: the path part way is the paths' mix.
        for u in [0.25, 0.5] {
            let mid: Vec<f64> = a
                .value
                .iter()
                .zip(&b.value)
                .map(|(x, y)| x + (y - x) * u)
                .collect();
            let got = numbers(&shape(&mid));
            let off = got
                .iter()
                .zip(na.iter().zip(&nb))
                .any(|(g, (x, y))| (g - (x + (y - x) * u)).abs() > 1e-6 * (1.0 + g.abs()));
            if got.len() != na.len() || off {
                return None;
            }
        }
        segs.push(Seg {
            t0: a.seconds + shift,
            t1: b.seconds + shift,
            v0: na,
            v1: nb,
            ease: Ease::Curve(vec![spline]),
        });
    }
    let value = if segs.is_empty() {
        Value::Still(numbers(&first))
    } else {
        Value::Moving(segs)
    };
    Some(PathValue::Curves {
        value,
        closed: first.closed,
    })
}

/// How far a baked outline's points may move when the points it doesn't
/// need are dropped, in pixels: a twentieth of one.
const BAKE_TOLERANCE: f64 = 0.05;

/// `line` without the points that lie within `tol` of the straight line
/// between the points kept either side (Douglas and Peucker), with an
/// explicit stack so a long outline can't run the stack out.
fn thinned(line: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let mut keep = vec![false; line.len()];
    let last = line.len() - 1;
    keep[0] = true;
    keep[last] = true;
    let mut stack = vec![(0usize, last)];
    while let Some((i, j)) = stack.pop() {
        let (Some(a), Some(b)) = (line.get(i), line.get(j)) else {
            continue;
        };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        let mut worst = (0usize, tol);
        for (k, p) in line.iter().enumerate().take(j).skip(i + 1) {
            let off = if len > 1e-12 {
                ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len
            } else {
                (p.0 - a.0).hypot(p.1 - a.1)
            };
            if off > worst.1 {
                worst = (k, off);
            }
        }
        if worst.0 > i {
            keep[worst.0] = true;
            stack.push((i, worst.0));
            stack.push((worst.0, j));
        }
    }
    line.iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect()
}

/// A still path.
fn curves_of(path: &BezierPath) -> PathValue {
    PathValue::Curves {
        value: Value::Still(numbers(path)),
        closed: path.closed,
    }
}

/// `path` with `count` vertices or more, the extra ones repeats of its last
/// vertex joined by segments of no length, so it draws the same path.
fn padded(path: &BezierPath, count: usize) -> BezierPath {
    let mut out = path.clone();
    let Some(last) = path.vertices.last().copied() else {
        return out;
    };
    if count <= path.vertices.len() {
        return out;
    }
    let still = lumit_core::mask::Vertex {
        pos: last.pos,
        tan_in: (0.0, 0.0),
        tan_out: (0.0, 0.0),
    };
    if let Some(v) = out.vertices.last_mut() {
        v.tan_out = (0.0, 0.0);
    }
    out.vertices.resize(count, still);
    if let Some(v) = out.vertices.last_mut() {
        v.tan_out = last.tan_out;
    }
    out
}

/// A morph between `keys`, its times moved `shift` onto the root clock.
/// Each pair is resampled to its own larger vertex count, as
/// `mask::lerp_paths` does, then padded to `most` so every value has the same
/// commands; a key two pairs share may so take two values at one moment,
/// both drawing its path. Each pair eases the way `mask::path_at` eases it:
/// the keys' own sides on a ramp from 0 to 1.
fn keyed(keys: &[PathKeyframe], most: usize, shift: f64) -> Value {
    let shape =
        |p: &BezierPath, n: usize| numbers(&padded(&lumit_core::mask::resample(p, n), most));
    let mut segs = Vec::new();
    for pair in keys.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        let n = a.path.vertices.len().max(b.path.vertices.len());
        let (pa, pb) = (shape(&a.path, n), shape(&b.path, n));
        let lerp =
            |u: f64| -> Vec<f64> { pa.iter().zip(&pb).map(|(p, q)| p + (q - p) * u).collect() };
        let ramp = [
            Keyframe {
                time: a.time,
                value: 0.0,
                interp_in: a.interp_in,
                interp_out: a.interp_out,
            },
            Keyframe {
                time: b.time,
                value: 1.0,
                interp_in: b.interp_in,
                interp_out: b.interp_out,
            },
        ];
        for span in crate::keys::curve(&ramp).spans {
            segs.push(match span {
                crate::keys::Span::Hold { t0, t1, .. } => Seg {
                    t0: t0 + shift,
                    t1: t1 + shift,
                    v0: pa.clone(),
                    v1: pb.clone(),
                    ease: Ease::Hold,
                },
                crate::keys::Span::Cubic { x, y } => {
                    let (dx, dy) = (x[3] - x[0], y[3] - y[0]);
                    let spline = if dy.abs() < 1e-12 || dx <= 0.0 {
                        LINEAR
                    } else {
                        [
                            (x[1] - x[0]) / dx,
                            (y[1] - y[0]) / dy,
                            (x[2] - x[0]) / dx,
                            (y[2] - y[0]) / dy,
                        ]
                    };
                    Seg {
                        t0: x[0] + shift,
                        t1: x[3] + shift,
                        v0: lerp(y[0]),
                        v1: lerp(y[3]),
                        ease: Ease::Curve(vec![spline]),
                    }
                }
            });
        }
    }
    if segs.is_empty() {
        return Value::Still(keys.first().map_or_else(Vec::new, |k| shape(&k.path, most)));
    }
    Value::Moving(segs)
}

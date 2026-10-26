//! After Effects' shape operators that act on the paths above them, as Lumit
//! takes them.
//!
//! Merge Paths, Offset Paths and Wiggle Paths have a field of their own on a
//! Lumit item. Round Corners, Zig Zag and Pucker & Bloat do not, but each only
//! moves a path's points, so they are worked into the path itself on every
//! frame it is sampled at.
//!
//! What makes a path in the first place is here too: a rectangle, an ellipse
//! and a star, as After Effects draws them.

use lumit_core::anim::{Animation, Keyframe, Property as LumProperty, SideInterp};
use lumit_core::mask::{BezierPath, Vertex};
use lumit_core::time::Rational;

use crate::capture::Property;
use crate::report::ItemPath;

use super::props::{child, from_node, starting_value};
use super::Conv;

/// What one operator does to the paths above it.
pub(super) enum Op {
    /// Worked into the path's points.
    Bake(Bake),
    /// Lumit's combine, by its own numbers.
    Merge(u32),
    /// Lumit's offset, in pixels, and whether After Effects' own would look
    /// the same: Lumit's corners are always round, and it draws no copies.
    Offset(LumProperty, bool),
    /// Lumit's deform, standing in for Wiggle Paths.
    Wiggle(lumit_core::shape::modifier::Deform),
}

#[derive(Clone)]
pub(super) enum Bake {
    Round(LumProperty),
    ZigZag {
        size: LumProperty,
        ridges: LumProperty,
        smooth: bool,
    },
    Pucker(LumProperty),
}

/// The operator `node` names, or `None` for one Lumit has no place for.
pub(super) fn read(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<Op> {
    let props = node.children();
    let one = |conv: &mut Conv<'_>, name: &str, default: f64| {
        child(props, name).map_or(LumProperty::fixed(default), |n| {
            from_node(conv, path, n, 0, default)
        })
    };
    let still = |name: &str, default: f64| {
        child(props, name)
            .and_then(|n| starting_value(n).0)
            .unwrap_or(default)
    };
    Some(match super::props::match_name_of(node) {
        "ADBE Vector Filter - RC" => Op::Bake(Bake::Round(one(
            conv,
            "ADBE Vector RoundCorner Radius",
            10.0,
        ))),
        "ADBE Vector Filter - Zigzag" => Op::Bake(Bake::ZigZag {
            size: one(conv, "ADBE Vector Zigzag Size", 5.0),
            ridges: one(conv, "ADBE Vector Zigzag Detail", 10.0),
            smooth: still("ADBE Vector Zigzag Points", 1.0) == 2.0,
        }),
        "ADBE Vector Filter - PB" => Op::Bake(Bake::Pucker(one(
            conv,
            "ADBE Vector PuckerBloat Amount",
            10.0,
        ))),
        // Merge, Add, Subtract, Intersect, Exclude Intersections, where Lumit
        // has union for the first two.
        "ADBE Vector Filter - Merge" => {
            Op::Merge(match still("ADBE Vector Merge Type", 1.0) as u32 {
                3 => 2,
                4 => 3,
                5 => 4,
                _ => 1,
            })
        }
        "ADBE Vector Filter - Offset" => Op::Offset(
            one(conv, "ADBE Vector Offset Amount", 10.0),
            still("ADBE Vector Offset Line Join", 1.0) == 2.0
                && still("ADBE Vector Offset Copies", 1.0) <= 1.0,
        ),
        "ADBE Vector Filter - Roughen" => {
            let rate = still("ADBE Vector Temporal Freq", 2.0);
            let phase = still("ADBE Vector Temporal Phase", 0.0);
            // Wiggle Paths moves on its own, so the field's evolution runs
            // across the layer at its speed, a turn a wiggle.
            let (from, to) = conv.span;
            let turn = |t: Rational| phase + rate * 360.0 * t.to_f64();
            let key = |time: Rational| Keyframe {
                time,
                value: turn(time),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            };
            Op::Wiggle(lumit_core::shape::modifier::Deform {
                mode: 0,
                amount: one(conv, "ADBE Vector Roughen Size", 10.0),
                // Detail is After Effects' points to an inch.
                scale: LumProperty::fixed(72.0),
                evolution: LumProperty {
                    animation: Animation::Keyframed(vec![key(from), key(to)]),
                    extra: serde_json::Map::new(),
                },
                seed: LumProperty::fixed(still("ADBE Vector Random Seed", 0.0)),
                detail: one(conv, "ADBE Vector Roughen Detail", 10.0),
                extra: serde_json::Map::new(),
            })
        }
        _ => return None,
    })
}

impl Bake {
    /// The numbers that, keyed, make the path move.
    pub(super) fn props(&self) -> Vec<&LumProperty> {
        match self {
            Bake::Round(r) | Bake::Pucker(r) => vec![r],
            Bake::ZigZag { size, ridges, .. } => vec![size, ridges],
        }
    }

    pub(super) fn apply(&self, path: BezierPath, t: f64) -> BezierPath {
        match self {
            Bake::Round(radius) => round(path, radius.value_at(t)),
            Bake::ZigZag {
                size,
                ridges,
                smooth,
            } => zigzag(path, size.value_at(t), ridges.value_at(t), *smooth),
            Bake::Pucker(amount) => pucker(path, amount.value_at(t)),
        }
    }
}

/// The share of a corner's radius its two handles reach.
const ROUND: f64 = 0.551_9;

/// Every sharp corner cut back by `radius` along both its edges and rounded
/// across, never further than half of either edge. A point with handles is
/// already round and is left.
fn round(path: BezierPath, radius: f64) -> BezierPath {
    let n = path.vertices.len();
    if radius <= 0.0 || n < 2 {
        return path;
    }
    let mut out = Vec::with_capacity(n * 2);
    for (i, v) in path.vertices.iter().enumerate() {
        let sharp = v.tan_in == (0.0, 0.0) && v.tan_out == (0.0, 0.0);
        let end = !path.closed && (i == 0 || i == n - 1);
        if !sharp || end {
            out.push(*v);
            continue;
        }
        let prev = path.vertices[(i + n - 1) % n].pos;
        let next = path.vertices[(i + 1) % n].pos;
        let toward = |to: (f64, f64)| {
            let (dx, dy) = (to.0 - v.pos.0, to.1 - v.pos.1);
            let length = dx.hypot(dy);
            let r = radius.min(length / 2.0);
            if length <= 0.0 {
                return (v.pos, (0.0, 0.0));
            }
            let at = (v.pos.0 + dx / length * r, v.pos.1 + dy / length * r);
            (at, ((v.pos.0 - at.0) * ROUND, (v.pos.1 - at.1) * ROUND))
        };
        let (a, a_handle) = toward(prev);
        let (b, b_handle) = toward(next);
        out.push(Vertex {
            pos: a,
            tan_in: (0.0, 0.0),
            tan_out: a_handle,
        });
        out.push(Vertex {
            pos: b,
            tan_in: b_handle,
            tan_out: (0.0, 0.0),
        });
    }
    BezierPath {
        vertices: out,
        closed: path.closed,
    }
}

/// Each edge cut into `ridges` points, `size` pixels either side of the line
/// in turn.
fn zigzag(path: BezierPath, size: f64, ridges: f64, smooth: bool) -> BezierPath {
    let n = path.vertices.len();
    let ridges = ridges.round().clamp(0.0, 100.0) as usize;
    if n < 2 || ridges == 0 || size == 0.0 {
        return path;
    }
    let edges = if path.closed { n } else { n - 1 };
    // A ridge is one point either side of the line, so an edge of five
    // ridges has five points, alternating.
    let steps = ridges;
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(edges * steps + 1);
    let mut sign = 1.0;
    for e in 0..edges {
        let a = path.vertices[e];
        let b = path.vertices[(e + 1) % n];
        for s in 0..steps {
            let u = s as f64 / steps as f64;
            let (p, d) = cubic(&a, &b, u);
            let length = d.0.hypot(d.1).max(1e-9);
            let normal = (-d.1 / length, d.0 / length);
            let push = if s == 0 && e == 0 && !path.closed {
                0.0
            } else {
                sign
            };
            points.push((p.0 + normal.0 * size * push, p.1 + normal.1 * size * push));
            sign = -sign;
        }
    }
    if !path.closed {
        if let Some(last) = path.vertices.last() {
            points.push(last.pos);
        }
    }
    let m = points.len();
    let vertices = (0..m)
        .map(|i| {
            let handle = if smooth {
                let prev = points[(i + m - 1) % m];
                let next = points[(i + 1) % m];
                ((next.0 - prev.0) / 6.0, (next.1 - prev.1) / 6.0)
            } else {
                (0.0, 0.0)
            };
            Vertex {
                pos: points[i],
                tan_in: (-handle.0, -handle.1),
                tan_out: handle,
            }
        })
        .collect();
    BezierPath {
        vertices,
        closed: path.closed,
    }
}

/// Each point pulled towards the middle of the path by `amount` per cent and
/// its handles pushed the other way, which bloats the curves between them.
/// A negative amount puckers.
fn pucker(path: BezierPath, amount: f64) -> BezierPath {
    let n = path.vertices.len();
    if n == 0 || amount == 0.0 {
        return path;
    }
    let share = amount / 100.0;
    let centre = path.vertices.iter().fold((0.0, 0.0), |c, v| {
        (c.0 + v.pos.0 / n as f64, c.1 + v.pos.1 / n as f64)
    });
    let moved = |p: (f64, f64), by: f64| (p.0 + (centre.0 - p.0) * by, p.1 + (centre.1 - p.1) * by);
    BezierPath {
        vertices: path
            .vertices
            .iter()
            .map(|v| {
                let pos = moved(v.pos, share);
                let out = moved((v.pos.0 + v.tan_out.0, v.pos.1 + v.tan_out.1), -share);
                let inn = moved((v.pos.0 + v.tan_in.0, v.pos.1 + v.tan_in.1), -share);
                Vertex {
                    pos,
                    tan_in: (inn.0 - pos.0, inn.1 - pos.1),
                    tan_out: (out.0 - pos.0, out.1 - pos.1),
                }
            })
            .collect(),
        closed: path.closed,
    }
}

/// The point `u` of the way along the curve from `a` to `b`, and its
/// direction there.
fn cubic(a: &Vertex, b: &Vertex, u: f64) -> ((f64, f64), (f64, f64)) {
    let p0 = a.pos;
    let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
    let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
    let p3 = b.pos;
    let w = 1.0 - u;
    let at = |c: fn((f64, f64)) -> f64| {
        w * w * w * c(p0) + 3.0 * w * w * u * c(p1) + 3.0 * w * u * u * c(p2) + u * u * u * c(p3)
    };
    let slope = |c: fn((f64, f64)) -> f64| {
        3.0 * w * w * (c(p1) - c(p0))
            + 6.0 * w * u * (c(p2) - c(p1))
            + 3.0 * u * u * (c(p3) - c(p2))
    };
    let mut d = (slope(|p| p.0), slope(|p| p.1));
    if d.0.hypot(d.1) < 1e-9 {
        d = (p3.0 - p0.0, p3.1 - p0.1);
    }
    ((at(|p| p.0), at(|p| p.1)), d)
}

/// The share of a quarter circle's radius its bezier handles reach, so four
/// arcs make a circle.
const KAPPA: f64 = 0.552_284_749_8;

pub(super) fn rect(w: f64, h: f64, cx: f64, cy: f64, round: f64) -> BezierPath {
    let (l, t, r, b) = (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
    let round = round.clamp(0.0, w.abs().min(h.abs()) / 2.0);
    let point = |x: f64, y: f64, tan_in: (f64, f64), tan_out: (f64, f64)| Vertex {
        pos: (x, y),
        tan_in,
        tan_out,
    };
    let vertices = if round <= 0.0 {
        vec![
            point(r, t, (0.0, 0.0), (0.0, 0.0)),
            point(r, b, (0.0, 0.0), (0.0, 0.0)),
            point(l, b, (0.0, 0.0), (0.0, 0.0)),
            point(l, t, (0.0, 0.0), (0.0, 0.0)),
        ]
    } else {
        let k = round * KAPPA;
        vec![
            point(r - round, t, (0.0, 0.0), (k, 0.0)),
            point(r, t + round, (0.0, -k), (0.0, 0.0)),
            point(r, b - round, (0.0, 0.0), (0.0, k)),
            point(r - round, b, (k, 0.0), (0.0, 0.0)),
            point(l + round, b, (0.0, 0.0), (-k, 0.0)),
            point(l, b - round, (0.0, k), (0.0, 0.0)),
            point(l, t + round, (0.0, 0.0), (0.0, -k)),
            point(l + round, t, (-k, 0.0), (0.0, 0.0)),
        ]
    };
    BezierPath {
        vertices,
        closed: true,
    }
}

pub(super) fn ellipse(w: f64, h: f64, cx: f64, cy: f64) -> BezierPath {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    BezierPath {
        vertices: vec![
            Vertex {
                pos: (cx, cy - ry),
                tan_in: (-kx, 0.0),
                tan_out: (kx, 0.0),
            },
            Vertex {
                pos: (cx + rx, cy),
                tan_in: (0.0, -ky),
                tan_out: (0.0, ky),
            },
            Vertex {
                pos: (cx, cy + ry),
                tan_in: (kx, 0.0),
                tan_out: (-kx, 0.0),
            },
            Vertex {
                pos: (cx - rx, cy),
                tan_in: (0.0, ky),
                tan_out: (0.0, -ky),
            },
        ],
        closed: true,
    }
}

/// A star or a polygon, pointing up, its corners rounded the way After
/// Effects rounds them.
pub(super) fn star(s: &super::shapes::Star, t: f64) -> BezierPath {
    let points = s.points.value_at(t).floor().clamp(3.0, 1000.0);
    let corners = if s.polygon { points } else { points * 2.0 };
    let step = std::f64::consts::TAU / corners;
    let (cx, cy) = (s.at[0].value_at(t), s.at[1].value_at(t));
    let mut angle = -std::f64::consts::FRAC_PI_2 + s.rotation.value_at(t).to_radians();
    let mut vertices = Vec::new();
    for i in 0..corners as usize {
        let outer = s.polygon || i % 2 == 0;
        let (radius, round) = if outer {
            (s.outer.value_at(t), s.outer_round.value_at(t) / 100.0)
        } else {
            (s.inner.value_at(t), s.inner_round.value_at(t) / 100.0)
        };
        let (sin, cos) = angle.sin_cos();
        let reach = std::f64::consts::TAU * radius / (corners * 4.0) * round;
        // Along the edge, at right angles to the spoke.
        let (ox, oy) = (-sin, cos);
        vertices.push(Vertex {
            pos: (cx + radius * cos, cy + radius * sin),
            tan_in: (-ox * reach, -oy * reach),
            tan_out: (ox * reach, oy * reach),
        });
        angle += step;
    }
    BezierPath {
        vertices,
        closed: true,
    }
}

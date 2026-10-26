//! What each modifier does to a polyline: the twist, the deform, the
//! smooth, the offset, and one step of the repeater.

use super::geometry::{
    arc_of, box_of, control_points, point_at, ring_of, shoelace, Affine, FINE_STEP, MAX_FINE_POINTS,
};
use super::modifier::{Deform, Twist};
use crate::mask::BezierPath;

impl Twist {
    /// The axis's direction, the direction across it, and the centre.
    fn frame(&self, t: f64) -> ((f64, f64), (f64, f64), (f64, f64)) {
        let (sin, cos) = self.axis.value_at(t).to_radians().sin_cos();
        (
            (cos, sin),
            (-sin, cos),
            (self.centre_x.value_at(t), self.centre_y.value_at(t)),
        )
    }

    /// How far `path` reaches back and forward along the axis from the
    /// centre, and how far across it at most.
    fn reach(&self, path: &BezierPath, t: f64) -> (f64, f64, f64) {
        let (a, n, c) = self.frame(t);
        control_points(path).fold((0.0f64, 0.0f64, 0.0f64), |r, p| {
            let d = (p.0 - c.0, p.1 - c.1);
            let along = d.0 * a.0 + d.1 * a.1;
            let across = d.0 * n.0 + d.1 * n.1;
            (r.0.min(along), r.1.max(along), r.2.max(across.abs()))
        })
    }

    /// Twist `points` in place. Each point keeps its place along the axis;
    /// its distance across it is what you see of that distance once its
    /// slice of the shape has turned, which is none at the back end of
    /// `path` and the whole angle at the front, plus the rotation.
    pub(super) fn apply(&self, points: &mut [(f64, f64)], path: &BezierPath, t: f64) {
        let (a, n, c) = self.frame(t);
        let (back, forward, _) = self.reach(path, t);
        let span = forward - back;
        let per = if span > 1e-9 {
            self.angle.value_at(t).to_radians() / span
        } else {
            0.0
        };
        let rotation = self.rotation.value_at(t).to_radians();
        for p in points {
            let d = (p.0 - c.0, p.1 - c.1);
            let along = d.0 * a.0 + d.1 * a.1;
            let across = (d.0 * n.0 + d.1 * n.1) * ((along - back) * per + rotation).cos();
            *p = (
                c.0 + along * a.0 + across * n.0,
                c.1 + along * a.1 + across * n.1,
            );
        }
    }

    /// The box the twisted `path` stays inside: its reach along the axis, and
    /// as far across it either way as it ever reached.
    pub(super) fn bounds(&self, path: &BezierPath, t: f64) -> Option<(f64, f64, f64, f64)> {
        let (a, n, c) = self.frame(t);
        let (back, forward, across) = self.reach(path, t);
        let corner = |along: f64, side: f64| {
            (
                c.0 + along * a.0 + side * n.0,
                c.1 + along * a.1 + side * n.1,
            )
        };
        box_of(
            [
                corner(back, -across),
                corner(back, across),
                corner(forward, -across),
                corner(forward, across),
            ]
            .into_iter(),
        )
    }
}

/// The most places one contour reads a deform's field at.
const MAX_FIELD_SAMPLES: f64 = 4096.0;

impl Deform {
    /// The field's value at each of `count` samples, read where they fall
    /// on the path, in −1..=1.
    fn field(&self, ring: &[(f64, f64)], arc: &[f64], gap: f64, count: usize, t: f64) -> Vec<f64> {
        let scale = self.scale.value_at(t).max(1.0);
        let depth = (self.evolution.value_at(t) / 360.0) as f32;
        let seed = self.seed.value_at(t).round() as i64 as u32;
        (0..count)
            .map(|k| {
                let p = point_at(ring, arc, k as f64 * gap);
                let v = crate::fx::noise::perlin3(
                    seed,
                    0,
                    (p.0 / scale) as f32,
                    (p.1 / scale) as f32,
                    depth,
                    0,
                );
                // Held to one, which is what keeps the layer's box honest.
                f64::from(v).clamp(-1.0, 1.0)
            })
            .collect()
    }

    /// `points` moved by the field. `path` gives the middle a Scale deform
    /// measures from.
    pub(super) fn applied(
        &self,
        points: &[(f64, f64)],
        closed: bool,
        path: &BezierPath,
        t: f64,
    ) -> Vec<(f64, f64)> {
        let amount = self.amount.value_at(t);
        let (ring, repeat) = ring_of(points, closed);
        let n = ring.len();
        let (arc, total) = arc_of(ring, closed);
        if n < 2 || amount == 0.0 || total <= 0.0 {
            return points.to_vec();
        }
        let scale = self.scale.value_at(t).max(1.0);
        let detail = self.detail.value_at(t).clamp(0.1, 100.0);
        let pieces = (total * detail / scale)
            .ceil()
            .clamp(if closed { 3.0 } else { 1.0 }, MAX_FIELD_SAMPLES) as usize;
        let gap = total / pieces as f64;
        // A closed ring's last sample leads back into its first; an open path
        // has one at each end.
        let count = if closed { pieces } else { pieces + 1 };
        let field = self.field(ring, &arc, gap, count, t);
        let sample = |k: isize| {
            let k = if closed {
                k.rem_euclid(count as isize)
            } else {
                k.clamp(0, count as isize - 1)
            };
            field.get(k as usize).copied().unwrap_or(0.0)
        };
        // Eased between the samples, so the outline has no corner where one
        // sample hands over to the next.
        let read = |s: f64| {
            let u = s / gap;
            let k = u.floor() as isize;
            let f = u - u.floor();
            let (p0, p1, p2, p3) = (sample(k - 1), sample(k), sample(k + 1), sample(k + 2));
            let v = 0.5
                * (2.0 * p1
                    + (p2 - p0) * f
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * f * f
                    + (3.0 * (p1 - p2) + p3 - p0) * f * f * f);
            v.clamp(-1.0, 1.0)
        };
        let middle = box_of(control_points(path))
            .map(|b| ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0))
            .unwrap_or_default();
        // Out is decided by the ring's own winding, as the offset decides it.
        let out = if closed && shoelace(ring) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let mut moved: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let p = ring[i];
                let v = read(arc[i]);
                if self.mode == 1 {
                    let f = 1.0 + amount / 100.0 * v;
                    return (
                        middle.0 + (p.0 - middle.0) * f,
                        middle.1 + (p.1 - middle.1) * f,
                    );
                }
                let before = match i {
                    0 if closed => ring[n - 1],
                    0 => p,
                    _ => ring[i - 1],
                };
                let after = match ring.get(i + 1) {
                    Some(&q) => q,
                    None if closed => ring[0],
                    None => p,
                };
                let (dx, dy) = (after.0 - before.0, after.1 - before.1);
                let len = dx.hypot(dy);
                if len <= 1e-12 {
                    return p;
                }
                let push = out * amount * v / len;
                (p.0 + dy * push, p.1 - dx * push)
            })
            .collect();
        if repeat {
            moved.push(moved[0]);
        }
        moved
    }

    /// `twisted`, the box the art is in before the deform, grown to hold
    /// what the deform can do to it. `own` is the path's own box, whose middle
    /// a Scale deform measures from.
    pub(super) fn bounds(
        &self,
        twisted: (f64, f64, f64, f64),
        own: (f64, f64, f64, f64),
        t: f64,
    ) -> (f64, f64, f64, f64) {
        let amount = self.amount.value_at(t).abs();
        let (x0, y0, x1, y1) = twisted;
        if self.mode != 1 {
            return (x0 - amount, y0 - amount, x1 + amount, y1 + amount);
        }
        let (mx, my) = ((own.0 + own.2) / 2.0, (own.1 + own.3) / 2.0);
        let grow = 1.0 + amount / 100.0;
        let rx = (x0 - mx).abs().max((x1 - mx).abs()) * grow;
        let ry = (y0 - my).abs().max((y1 - my).abs()) * grow;
        (mx - rx, my - ry, mx + rx, my + ry)
    }
}

/// `points` with every point averaged with the path either side of it,
/// `amount` pixels each way: the path walked into evenly spaced points, then
/// three running averages, which together weigh the neighbours like a bell
/// curve. An open path's ends stay put, the window closing in towards them.
pub(super) fn smoothed(points: &[(f64, f64)], closed: bool, amount: f64) -> Vec<(f64, f64)> {
    let (ring, repeat) = ring_of(points, closed);
    let (arc, total) = arc_of(ring, closed);
    if amount <= 0.0 || ring.len() < 3 || total <= 0.0 {
        return points.to_vec();
    }
    let step = (amount / 8.0)
        .clamp(0.25, FINE_STEP)
        .max(total / MAX_FINE_POINTS);
    let pieces = (total / step).ceil().max(3.0) as usize;
    let n = if closed { pieces } else { pieces + 1 };
    let gap = total / pieces as f64;
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    for k in 0..n {
        let p = point_at(ring, &arc, k as f64 * gap);
        xs.push(p.0);
        ys.push(p.1);
    }
    let reach = ((amount / (3.0 * gap)).round() as usize).max(1);
    for _ in 0..3 {
        xs = averaged(&xs, reach, closed);
        ys = averaged(&ys, reach, closed);
    }
    let mut out: Vec<(f64, f64)> = xs.into_iter().zip(ys).collect();
    if repeat {
        out.push(out[0]);
    }
    out
}

/// Each of `values` replaced by the mean of itself and `reach` either side,
/// wrapping round when `closed`. Open, the window narrows towards each end so
/// the ends themselves never move.
fn averaged(values: &[f64], reach: usize, closed: bool) -> Vec<f64> {
    let n = values.len();
    // Running totals, so the cost does not grow with the reach.
    let mut sums = Vec::with_capacity(n + 1);
    sums.push(0.0);
    for v in values {
        sums.push(sums[sums.len() - 1] + v);
    }
    let all = sums[n];
    if closed {
        let reach = reach.min((n - 1) / 2);
        let width = (2 * reach + 1) as f64;
        (0..n)
            .map(|i| {
                let (from, to) = ((i + n - reach) % n, (i + reach) % n);
                let sum = if from <= to {
                    sums[to + 1] - sums[from]
                } else {
                    all - sums[from] + sums[to + 1]
                };
                sum / width
            })
            .collect()
    } else {
        (0..n)
            .map(|i| {
                let r = reach.min(i).min(n - 1 - i);
                (sums[i + r + 1] - sums[i - r]) / (2 * r + 1) as f64
            })
            .collect()
    }
}

/// How many straight steps a round join is drawn in, per quarter turn.
///
/// Four is smooth at the sizes an offset outline is drawn at, and a count taken
/// from the angle rather than the size keeps the answer identical on every
/// machine (docs/14 §determinism).
const JOIN_STEPS_PER_QUARTER: f64 = 4.0;

/// `points` pushed `amount` layer pixels **out** of itself, corners rounded —
/// After Effects' Offset Paths, with the one join this crate draws.
///
/// "Out" is decided by the ring's own winding for a closed path, so a positive
/// amount always grows the shape whichever way round its points were written.
/// An open path has no inside, so it is simply moved to the side its own
/// direction puts on the left.
///
/// **Self-intersections are left in.** Offsetting inwards by more than a curve
/// bends can fold the outline back through itself; the non-zero winding fill
/// swallows most of what that produces, and unpicking it properly is a
/// polygon-clipping library rather than thirty lines. The failure is visible
/// and local, which is why it is a limit rather than a trap.
pub(super) fn offset_polyline(points: &[(f64, f64)], amount: f64, closed: bool) -> Vec<(f64, f64)> {
    // A closed path flattens with its first point repeated at the end; the
    // offset works on the ring, and closes itself again at the end.
    let ring: &[(f64, f64)] = if closed && points.len() > 2 && points[0] == points[points.len() - 1]
    {
        &points[..points.len() - 1]
    } else {
        points
    };
    if ring.len() < 2 {
        return points.to_vec();
    }

    // Which side is "out". The shoelace area is positive for the winding a
    // shape tool draws, and a path written the other way round gets the sign
    // flipped so that a positive amount still grows it.
    let sign = if closed {
        let mut area = 0.0;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            area += a.0 * b.1 - b.0 * a.1;
        }
        if area < 0.0 {
            -1.0
        } else {
            1.0
        }
    } else {
        1.0
    };
    let d = amount * sign;

    // The normal of the segment starting at `i`, or `None` where the segment
    // has no length to take a direction from.
    let normal = |i: usize| -> Option<(f64, f64)> {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        (len > 1e-12).then(|| (dy / len, -dx / len))
    };

    let last = if closed { ring.len() } else { ring.len() - 1 };
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(ring.len() * 2);
    for i in 0..ring.len() {
        // The segments that meet at this point. An open path's two ends have
        // only one each, which is what moves an open path to the side.
        let before = (closed || i > 0)
            .then(|| normal((i + ring.len() - 1) % ring.len()))
            .flatten();
        let after = (i < last).then(|| normal(i)).flatten();
        let p = ring[i];
        match (before, after) {
            (Some(n0), Some(n1)) => {
                out.push((p.0 + d * n0.0, p.1 + d * n0.1));
                // The corner opens on the side the offset is going: that gap
                // is what a round join fills. On the other side the two
                // offset ends overlap, and joining them straight is the whole
                // of the self-intersection this leaves behind.
                let cross = n0.0 * n1.1 - n0.1 * n1.0;
                if cross * d > 0.0 {
                    let angle = (n0.0 * n1.0 + n0.1 * n1.1).clamp(-1.0, 1.0).acos();
                    let steps =
                        (angle / std::f64::consts::FRAC_PI_2 * JOIN_STEPS_PER_QUARTER).ceil();
                    let steps = steps.max(1.0) as usize;
                    for step in 1..steps {
                        let a = angle * (step as f64 / steps as f64) * cross.signum();
                        let (sin, cos) = a.sin_cos();
                        let (nx, ny) = (n0.0 * cos - n0.1 * sin, n0.0 * sin + n0.1 * cos);
                        out.push((p.0 + d * nx, p.1 + d * ny));
                    }
                }
                out.push((p.0 + d * n1.0, p.1 + d * n1.1));
            }
            (Some(n), None) | (None, Some(n)) => out.push((p.0 + d * n.0, p.1 + d * n.1)),
            (None, None) => out.push(p),
        }
    }
    if closed && out.len() > 1 {
        out.push(out[0]);
    }
    out
}

/// One step of the repeater: move by `position`, turn by `rotation` degrees and
/// scale by `scale`, all about `anchor`.
pub(super) fn repeat_step(
    position: (f64, f64),
    rotation: f64,
    scale: f64,
    anchor: (f64, f64),
) -> Affine {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (a, b, c, d) = (cos * scale, sin * scale, -sin * scale, cos * scale);
    // Move to the anchor, turn and scale there, move back, then translate.
    Affine([
        a,
        b,
        c,
        d,
        anchor.0 - (a * anchor.0 + c * anchor.1) + position.0,
        anchor.1 - (b * anchor.0 + d * anchor.1) + position.1,
    ])
}

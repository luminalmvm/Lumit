//! The polyline maths under a shape layer: walking a path into points,
//! measuring it, and the affine a copy is placed by.

use crate::mask::{BezierPath, Vertex};

/// How many straight steps each cubic segment becomes when a path is walked for
/// stroking.
///
/// Sixteen is smooth to well past the sizes a stroke is drawn at, and a fixed
/// count keeps the result identical on every machine (docs/14 §determinism).
const FLATTEN_STEPS: usize = 16;

/// The path as a polyline: each cubic segment walked in [`FLATTEN_STEPS`] steps.
/// A closed path returns to its first point, so its outline closes.
pub fn flatten_path(path: &BezierPath) -> Vec<(f64, f64)> {
    let n = path.vertices.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![path.vertices[0].pos];
    }
    let mut out = vec![path.vertices[0].pos];
    let last = if path.closed { n } else { n - 1 };
    for i in 0..last {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        let p0 = a.pos;
        let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
        let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
        let p3 = b.pos;
        for step in 1..=FLATTEN_STEPS {
            let t = step as f64 / FLATTEN_STEPS as f64;
            out.push(cubic_at(p0, p1, p2, p3, t));
        }
    }
    out
}

/// How far apart the points of a path being bent are, in layer pixels: close
/// enough that the bend reads as a curve and not as a run of facets.
pub(super) const FINE_STEP: f64 = 2.0;

/// The most points one bent contour is walked into. A path long enough to
/// need more is walked more coarsely instead, so the cost of a frame stays
/// bounded whatever size the art is.
pub(super) const MAX_FINE_POINTS: f64 = 16_384.0;

/// The path as a polyline fine enough to bend: like [`flatten_path`], but each
/// cubic is walked in steps of about [`FINE_STEP`] rather than a fixed sixteen.
pub(super) fn flatten_fine(path: &BezierPath) -> Vec<(f64, f64)> {
    let n = path.vertices.len();
    if n < 2 {
        return flatten_path(path);
    }
    let last = if path.closed { n } else { n - 1 };
    let segment = |i: usize| {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        [
            a.pos,
            (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1),
            (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1),
            b.pos,
        ]
    };
    // The control polygon is never shorter than the curve, so stepping it
    // keeps the steps no longer than asked.
    let hull = |c: [(f64, f64); 4]| dist(c[0], c[1]) + dist(c[1], c[2]) + dist(c[2], c[3]);
    let total: f64 = (0..last).map(|i| hull(segment(i))).sum();
    let step = FINE_STEP.max(total / MAX_FINE_POINTS);
    let mut out = vec![path.vertices[0].pos];
    for i in 0..last {
        let [p0, p1, p2, p3] = segment(i);
        let steps = ((hull(segment(i)) / step).ceil() as usize).max(FLATTEN_STEPS);
        for k in 1..=steps {
            out.push(cubic_at(p0, p1, p2, p3, k as f64 / steps as f64));
        }
    }
    out
}

/// `points` with every segment longer than `step` cut into equal pieces no
/// longer than it, so a straight edge has points along it to move.
pub(super) fn densified(points: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let total: f64 = points.windows(2).map(|p| dist(p[0], p[1])).sum();
    let step = step.max(total / MAX_FINE_POINTS);
    let mut out = Vec::with_capacity(points.len());
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        out.push(a);
        let pieces = (dist(a, b) / step).ceil() as usize;
        for k in 1..pieces {
            let f = k as f64 / pieces as f64;
            out.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
        }
    }
    if let Some(&last) = points.last() {
        out.push(last);
    }
    out
}

pub(super) fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Every vertex of `path` and both its handles' ends: the hull a cubic never
/// leaves.
pub(super) fn control_points(path: &BezierPath) -> impl Iterator<Item = (f64, f64)> + '_ {
    path.vertices.iter().flat_map(|v| {
        [
            v.pos,
            (v.pos.0 + v.tan_in.0, v.pos.1 + v.tan_in.1),
            (v.pos.0 + v.tan_out.0, v.pos.1 + v.tan_out.1),
        ]
    })
}

/// The box round `points`, or `None` for none.
pub(super) fn box_of(points: impl Iterator<Item = (f64, f64)>) -> Option<(f64, f64, f64, f64)> {
    points.fold(None, |out, (x, y)| {
        Some(match out {
            None => (x, y, x, y),
            Some(b) => (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y)),
        })
    })
}

/// A closed polyline's points without the copy of the first one it ends on,
/// and whether there was one, so it can be put back.
pub(super) fn ring_of(points: &[(f64, f64)], closed: bool) -> (&[(f64, f64)], bool) {
    match (points.first(), points.last()) {
        (Some(a), Some(b)) if closed && points.len() > 2 && a == b => {
            (&points[..points.len() - 1], true)
        }
        _ => (points, false),
    }
}

/// How far along `ring` each of its points is, and its whole length,
/// counting the way back to the start when it is closed.
pub(super) fn arc_of(ring: &[(f64, f64)], closed: bool) -> (Vec<f64>, f64) {
    let mut arc = Vec::with_capacity(ring.len());
    let mut far = 0.0;
    for (i, &p) in ring.iter().enumerate() {
        if i > 0 {
            far += dist(ring[i - 1], p);
        }
        arc.push(far);
    }
    let total = match (closed, ring.first(), ring.last()) {
        (true, Some(&a), Some(&b)) => far + dist(b, a),
        _ => far,
    };
    (arc, total)
}

/// The point `s` along `ring`, whose points sit at `arc`.
pub(super) fn point_at(ring: &[(f64, f64)], arc: &[f64], s: f64) -> (f64, f64) {
    let i = arc.partition_point(|&a| a <= s).max(1) - 1;
    let a = ring.get(i).copied().unwrap_or_default();
    // Past the last point of a closed ring is on the way back to the first.
    let (b, end) = match ring.get(i + 1) {
        Some(&b) => (b, arc.get(i + 1).copied().unwrap_or(s)),
        None => (ring[0], arc[i] + dist(a, ring[0])),
    };
    let span = end - arc[i];
    if span <= 0.0 {
        return a;
    }
    let f = ((s - arc[i]) / span).clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

/// Twice the signed area of a ring: positive for the winding a shape tool
/// draws.
pub(super) fn shoelace(ring: &[(f64, f64)]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

fn cubic_at(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// A polyline as a [`BezierPath`]: every handle zero, so the segments stay
/// straight and the one rasteriser in the crate can draw it.
pub(super) fn polyline_path(points: &[(f64, f64)], closed: bool) -> BezierPath {
    BezierPath {
        vertices: points
            .iter()
            .map(|&pos| Vertex {
                pos,
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            })
            .collect(),
        closed,
    }
}

/// `points` re-started `shift` per cent of its own length along, wrapping — the
/// same ring of points with the seam moved.
///
/// Only meaningful for a closed polyline (one whose last point is its first),
/// which is what a closed path flattens to. A shift of zero, or a path with no
/// length to measure, hands the points straight back.
pub(super) fn rotated(points: &[(f64, f64)], shift: f64) -> Vec<(f64, f64)> {
    let shift = shift.rem_euclid(100.0);
    if shift == 0.0 || points.len() < 2 {
        return points.to_vec();
    }
    // Walk to the point `shift` per cent along, cut there, and hand back the
    // two pieces the other way round. `trimmed` already knows how to cut a
    // polyline by length, so the two cuts are the two halves of the ring.
    let head = crate::paint::trimmed(points, 0.0, shift);
    let tail = crate::paint::trimmed(points, shift, 100.0);
    if head.len() < 2 || tail.len() < 2 {
        return points.to_vec();
    }
    let mut out = tail;
    // The join is one shared point, not two: the tail ends where the head
    // begins.
    out.extend_from_slice(&head[1..]);
    out
}

/// A 2-D affine transform, `[a, b, c, d, e, f]`, mapping
/// `(x, y)` to `(a x + c y + e, b x + d y + f)` — the order every 2-D graphics
/// library writes it in.
///
/// Six numbers rather than a matrix type from somewhere: this is the only place
/// in the crate that composes transforms in the *art's* own space, and a
/// dependency for six multiplications would be a dependency for six
/// multiplications.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// A move and nothing else.
    #[must_use]
    pub fn translation(dx: f64, dy: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, dx, dy])
    }

    /// How much bigger this transform draws things: the square root of the area
    /// it multiplies by, which is the one number a *width* can be scaled by
    /// whatever the rotation. A mirrored transform has a negative determinant
    /// and still draws at a size, hence the absolute value.
    #[must_use]
    pub fn scale(&self) -> f64 {
        let m = self.0;
        (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
    }

    /// This transform followed by `next` — read left to right, which is how the
    /// repeater stacks its steps.
    #[must_use]
    pub fn then(self, next: &Affine) -> Affine {
        let (m, n) = (self.0, next.0);
        Affine([
            m[0] * n[0] + m[1] * n[2],
            m[0] * n[1] + m[1] * n[3],
            m[2] * n[0] + m[3] * n[2],
            m[2] * n[1] + m[3] * n[3],
            m[4] * n[0] + m[5] * n[2] + n[4],
            m[4] * n[1] + m[5] * n[3] + n[5],
        ])
    }

    #[must_use]
    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let m = self.0;
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }

    /// The transform that undoes this one, or the identity where it cannot be
    /// undone — a copy scaled to nothing has no way back, and an identity is
    /// the answer that draws something rather than dividing by zero (docs/14).
    #[must_use]
    pub fn inverse(&self) -> Affine {
        let m = self.0;
        let det = m[0] * m[3] - m[1] * m[2];
        if det.abs() < 1e-12 {
            return Affine::IDENTITY;
        }
        let inv = 1.0 / det;
        let (a, b, c, d) = (m[3] * inv, -m[1] * inv, -m[2] * inv, m[0] * inv);
        Affine([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
    }
}

/// `path` with `m` applied to it.
///
/// A vertex's **position** is moved; its **handles** are turned and scaled but
/// not moved, because a handle is a direction from its vertex rather than a
/// place — moving it as well would translate the curve twice.
pub(super) fn transform_path(path: &BezierPath, m: &Affine) -> BezierPath {
    let vector = |(x, y): (f64, f64)| {
        let a = m.0;
        (a[0] * x + a[2] * y, a[1] * x + a[3] * y)
    };
    BezierPath {
        vertices: path
            .vertices
            .iter()
            .map(|v| crate::mask::Vertex {
                pos: m.apply(v.pos),
                tan_in: vector(v.tan_in),
                tan_out: vector(v.tan_out),
            })
            .collect(),
        closed: path.closed,
    }
}

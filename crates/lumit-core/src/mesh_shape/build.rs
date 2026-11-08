//! A 3D shape's triangles, in its layer's pixels: y down, z going away from
//! the eye, centred on the layer's anchor.
//!
//! Each is worked out y up first, the way a 3D model's file has it, then
//! turned half a turn about x, the way a model reaches its comp. Every step is
//! plain arithmetic in a fixed order, so the same rows give the same triangles
//! on any run.

use std::f64::consts::{FRAC_PI_4, PI, TAU};

use super::MeshShape;
use crate::anim::Property;
use crate::extrude::mesh::Face;
use crate::model::{EffectInstance, EffectValue};

/// Where neighbouring steps turn by more than this (30 degrees) the surface
/// shows a crease or flat facets, so 4 segments make a pyramid and 6 a hex
/// prism. Below it they shade as one smooth curve.
const SMOOTH: f64 = PI / 6.0;
/// Below this a length is nothing.
const TINY: f64 = 1e-6;

type V3 = [f64; 3];

/// `shape` as its rows `rows` have it, each number read by `read`, or as it
/// starts with none.
#[must_use]
pub fn mesh(
    shape: MeshShape,
    rows: Option<&EffectInstance>,
    read: &dyn Fn(&Property) -> f64,
) -> Face {
    let row = |id: &str, or: f64| match rows.and_then(|r| r.param(id)) {
        Some(EffectValue::Float(p)) => read(p),
        _ => or,
    };
    let size = |id: &str, or: f64| row(id, or).max(0.0);
    let count = |id: &str, or: f64, least: f64| row(id, or).round().clamp(least, 512.0) as u32;
    let slice = || {
        let start = row("slice_start", 0.0).to_radians();
        // An end before the start wraps round, as the angles do.
        let d = row("slice_end", 360.0).to_radians() - start;
        (start, if d >= TAU { TAU } else { d.rem_euclid(TAU) })
    };
    let mut b = Builder::default();
    match shape {
        MeshShape::Cube => b.cube(
            [
                size("width", 200.0),
                size("height", 200.0),
                size("depth", 200.0),
            ],
            size("bevel", 0.0),
            count("bevel_segments", 4.0, 1.0),
        ),
        MeshShape::Sphere => {
            let (r, n) = (size("radius", 100.0), count("segments", 48.0, 3.0));
            let rings = (n / 2).max(2);
            let profile = (0..=rings)
                .map(|i| {
                    let (s, c) = (PI * f64::from(i) / f64::from(rings)).sin_cos();
                    Pt::new([r * s, r * c], [s, c])
                })
                .collect::<Vec<_>>();
            if r > TINY {
                b.lathe(&profile, n, slice());
            }
        }
        MeshShape::Cylinder | MeshShape::Cone => {
            let (top, bottom) = match shape {
                MeshShape::Cone => (size("top_radius", 0.0), size("bottom_radius", 100.0)),
                _ => (size("radius", 100.0), size("radius", 100.0)),
            };
            let h = size("height", 200.0) / 2.0;
            let mut corners = vec![[0.0, h]];
            corners.extend((top > TINY).then_some([top, h]));
            corners.extend((bottom > TINY).then_some([bottom, -h]));
            corners.push([0.0, -h]);
            let bevel = (size("bevel", 0.0), count("bevel_segments", 4.0, 1.0));
            let profile = with_normals(&rounded(&corners, bevel.0, bevel.1, false));
            if h > TINY && corners.len() > 2 {
                b.lathe(&profile, count("segments", 48.0, 3.0), slice());
            }
        }
        MeshShape::Torus => {
            let (ring, pipe) = (size("radius", 100.0), size("pipe_radius", 30.0));
            let m = count("pipe_segments", 24.0, 3.0);
            // Round the pipe from its top, outside first, back to its top.
            let profile = (0..=m)
                .map(|i| {
                    let (s, c) = (PI / 2.0 - TAU * f64::from(i) / f64::from(m)).sin_cos();
                    Pt::new([ring + pipe * c, pipe * s], [c, s])
                })
                .collect::<Vec<_>>();
            if pipe > TINY {
                b.lathe(&profile, count("segments", 64.0, 3.0), slice());
            }
        }
        MeshShape::Plane => {
            let (w, h) = (size("width", 400.0) / 2.0, size("height", 400.0) / 2.0);
            let corners = [[-w, -h], [w, -h], [w, h], [-w, h]];
            let radius = size("corner_radius", 0.0);
            let ring = rounded(&corners, radius, count("corner_segments", 8.0, 1.0), true);
            if w > TINY && h > TINY {
                let at = |p: [f64; 2]| [p[0], p[1], 0.0];
                let uv = |p: [f64; 2]| [(p[0] + w) / (2.0 * w), (h - p[1]) / (2.0 * h)];
                let centre = b.push([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.5, 0.5]);
                let first = b.len();
                for p in &ring {
                    b.push(at(*p), [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], uv(*p));
                }
                let n = ring.len() as u32;
                for k in 0..n {
                    b.tri(centre, first + k, first + (k + 1) % n);
                }
            }
        }
    }
    b.turned()
}

/// A point of a profile turned round y: its radius and height, which way the
/// surface faces there, and how far along the profile it is, nought to one.
#[derive(Debug, Clone, Copy)]
struct Pt {
    at: [f64; 2],
    normal: [f64; 2],
    v: f64,
}

impl Pt {
    fn new(at: [f64; 2], normal: [f64; 2]) -> Self {
        Self { at, normal, v: 0.0 }
    }
}

/// `corners` with each corner rounded by an arc of `radius` in `segments`
/// steps. An open profile keeps its two ends sharp, as they sit on the axis.
fn rounded(corners: &[[f64; 2]], radius: f64, segments: u32, closed: bool) -> Vec<[f64; 2]> {
    let n = corners.len();
    let round = |k: usize| radius > TINY && n > 2 && (closed || (k > 0 && k + 1 < n));
    let mut out = Vec::new();
    for k in 0..n {
        let p = corners[k];
        if !round(k) {
            out.push(p);
            continue;
        }
        let (prev, next) = ((k + n - 1) % n, (k + 1) % n);
        let (a, c) = (corners[prev], corners[next]);
        let (la, lc) = (dist(a, p), dist(p, c));
        let (u, w) = (unit(sub(p, a)), unit(sub(c, p)));
        let turn = dot(u, w).clamp(-1.0, 1.0).acos();
        if la < TINY || lc < TINY || turn < TINY {
            out.push(p);
            continue;
        }
        // A side another rounded corner shares lends each half of itself.
        let room = |len: f64, other: usize| if round(other) { len / 2.0 } else { len };
        let reach = (radius * (turn / 2.0).tan())
            .min(room(la, prev))
            .min(room(lc, next));
        let r = reach / (turn / 2.0).tan();
        let start = sub(p, scale(u, reach));
        let inward = unit(sub(w, scale(u, dot(u, w))));
        let centre = add(start, scale(inward, r));
        let from = sub(start, centre);
        let sign = (u[0] * w[1] - u[1] * w[0]).signum();
        for s in 0..=segments {
            let (sn, cs) = (sign * turn * f64::from(s) / f64::from(segments)).sin_cos();
            out.push(add(
                centre,
                [from[0] * cs - from[1] * sn, from[0] * sn + from[1] * cs],
            ));
        }
    }
    out
}

/// An open profile from the axis at the top to the axis at the bottom, given
/// the way each part faces: smooth where it turns gently, a crease (the point
/// twice) where it doesn't.
fn with_normals(points: &[[f64; 2]]) -> Vec<Pt> {
    // Out from the solid: to the right of going down the profile.
    let side = |k: usize| {
        let d = unit(sub(points[k + 1], points[k]));
        [-d[1], d[0]]
    };
    let sides: Vec<[f64; 2]> = (0..points.len().saturating_sub(1))
        .filter(|&k| dist(points[k], points[k + 1]) > TINY)
        .map(side)
        .collect();
    let mut kept: Vec<[f64; 2]> = points.to_vec();
    kept.dedup_by(|a, b| dist(*a, *b) <= TINY);
    let mut out = Vec::new();
    for (k, &p) in kept.iter().enumerate() {
        let before = k.checked_sub(1).and_then(|k| sides.get(k)).copied();
        let after = sides.get(k).copied();
        match (before, after) {
            (Some(a), Some(b)) if dot(a, b).clamp(-1.0, 1.0).acos() <= SMOOTH => {
                out.push(Pt::new(p, unit(add(a, b))));
            }
            (Some(a), Some(b)) => {
                out.push(Pt::new(p, a));
                out.push(Pt::new(p, b));
            }
            // An end on the axis faces straight up or down when the part
            // leaving it is nearly flat, as a cap's middle does.
            (Some(n), None) | (None, Some(n)) => {
                let flat = n[1].abs() >= SMOOTH.cos();
                out.push(Pt::new(p, if flat { [0.0, n[1].signum()] } else { n }));
            }
            (None, None) => {}
        }
    }
    out
}

#[derive(Default)]
struct Builder {
    f: Face,
    /// The y-up positions, kept in f64 until the end.
    at: Vec<V3>,
}

impl Builder {
    fn len(&self) -> u32 {
        self.at.len() as u32
    }

    fn push(&mut self, p: V3, n: V3, t: V3, uv: [f64; 2]) -> u32 {
        let k = self.len();
        let f32s = |v: V3| v.map(|c| c as f32);
        self.at.push(p);
        self.f.normals.push(f32s(n));
        let t = f32s(t);
        self.f.tangents.push([t[0], t[1], t[2], 1.0]);
        self.f.uv0.push(uv.map(|c| c as f32));
        k
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.f.indices.extend([a, b, c]);
    }

    /// The face, turned half a turn about x into the layer's space.
    fn turned(mut self) -> Face {
        let flip = |v: [f32; 3]| [v[0], -v[1], -v[2]];
        self.f.positions = (self.at.iter())
            .map(|p| flip(p.map(|c| c as f32)))
            .collect();
        for n in &mut self.f.normals {
            *n = flip(*n);
        }
        for t in &mut self.f.tangents {
            let [x, y, z] = flip([t[0], t[1], t[2]]);
            *t = [x, y, z, t[3]];
        }
        self.f
    }

    /// A box `size` across, its edges rounded by `bevel` in about `segments`
    /// steps. Each face is a grid whose outer rows wrap half way round its
    /// edges, meeting the next face's at 45 degrees.
    fn cube(&mut self, size: V3, bevel: f64, segments: u32) {
        let half = size.map(|s| s / 2.0);
        if half.iter().any(|&h| h <= TINY) {
            return;
        }
        let bevel = bevel.min(half[0]).min(half[1]).min(half[2]);
        let inner = half.map(|h| h - bevel);
        let steps = if bevel > TINY {
            segments.div_ceil(2)
        } else {
            0
        };
        // Across a face's axis `a`, from one edge to the other.
        let across = |a: usize| -> Vec<f64> {
            let out = (0..=steps).map(|k| match steps {
                0 => half[a],
                _ => inner[a] + bevel * (FRAC_PI_4 * f64::from(k) / f64::from(steps)).tan(),
            });
            let out: Vec<f64> = out.collect();
            out.iter()
                .rev()
                .map(|v| -v)
                .chain(out.iter().copied())
                .collect()
        };
        // Each face's normal axis and sign, the way its pictures run across,
        // and up it.
        let faces: [(usize, f64, usize, f64, usize, f64); 6] = [
            (0, 1.0, 2, -1.0, 1, 1.0),
            (0, -1.0, 2, 1.0, 1, 1.0),
            (1, 1.0, 0, 1.0, 2, -1.0),
            (1, -1.0, 0, 1.0, 2, 1.0),
            (2, 1.0, 0, 1.0, 1, 1.0),
            (2, -1.0, 0, -1.0, 1, 1.0),
        ];
        for (na, ns, ta, ts, ba, bs) in faces {
            let (us, vs) = (across(ta), across(ba));
            let first = self.len();
            for &v in &vs {
                for &u in &us {
                    let mut p = [0.0; 3];
                    p[na] = ns * half[na];
                    p[ta] = ts * u;
                    p[ba] = bs * v;
                    let core: V3 = std::array::from_fn(|i| p[i].clamp(-inner[i], inner[i]));
                    let out = sub3(p, core);
                    let n = match len3(out) {
                        l if l > TINY && bevel > TINY => out.map(|c| c / l),
                        _ => {
                            let mut n = [0.0; 3];
                            n[na] = ns;
                            n
                        }
                    };
                    let at = match bevel > TINY {
                        true => std::array::from_fn(|i| core[i] + bevel * n[i]),
                        false => p,
                    };
                    let mut t = [0.0; 3];
                    t[ta] = ts;
                    let uv = [
                        (u + half[ta]) / (2.0 * half[ta]),
                        (half[ba] - v) / (2.0 * half[ba]),
                    ];
                    self.push(at, n, t, uv);
                }
            }
            let row = us.len() as u32;
            for j in 0..vs.len() as u32 - 1 {
                for i in 0..row - 1 {
                    let a = first + j * row + i;
                    let (b, c, d) = (a + 1, a + row + 1, a + row);
                    self.tri(a, b, c);
                    self.tri(a, c, d);
                }
            }
        }
    }

    /// `profile` turned round y in `sides` steps a full turn, over the sweep
    /// `(start, length)` in radians. A part turn has its cut faces filled.
    fn lathe(&mut self, profile: &[Pt], sides: u32, (start, sweep): (f64, f64)) {
        let mut profile = profile.to_vec();
        let mut total = 0.0;
        for k in 1..profile.len() {
            total += dist(profile[k - 1].at, profile[k].at);
            profile[k].v = total;
        }
        for p in &mut profile {
            p.v /= total.max(TINY);
        }
        let sweep = sweep.min(TAU);
        if sweep <= TINY || profile.len() < 2 {
            return;
        }
        let step = TAU / f64::from(sides);
        // The same angles at any slice, so cutting into a faceted shape never
        // turns its facets: half a step round from nought, plus the cut ends.
        let full = sweep >= TAU - TINY;
        let angles: Vec<f64> = if full {
            (0..=sides)
                .map(|k| step / 2.0 + step * f64::from(k))
                .collect()
        } else {
            let end = start + sweep;
            let first = ((start - step / 2.0) / step).floor() as i64 + 1;
            let inside = (first..)
                .map(|k| step / 2.0 + step * k as f64)
                .take_while(|a| *a < end - TINY)
                .filter(|a| *a > start + TINY);
            std::iter::once(start)
                .chain(inside)
                .chain(std::iter::once(end))
                .collect()
        };
        let (a0, span) = (angles[0], angles[angles.len() - 1] - angles[0]);
        let at = |p: &Pt, a: f64| [p.at[0] * a.cos(), p.at[1], -p.at[0] * a.sin()];
        let round = |a: f64| [-a.sin(), 0.0, -a.cos()];
        let faceted = step > SMOOTH + TINY
            || (profile.windows(2)).any(|w| {
                dist(w[0].at, w[1].at) > TINY
                    && dot(w[0].normal, w[1].normal).clamp(-1.0, 1.0).acos() > SMOOTH + TINY
            });
        let rows = profile.len();
        let cols = angles.len();
        if faceted {
            for i in 0..rows - 1 {
                let (p, q) = (&profile[i], &profile[i + 1]);
                if dist(p.at, q.at) <= TINY {
                    continue;
                }
                for j in 0..cols - 1 {
                    let (s, e) = (angles[j], angles[j + 1]);
                    let corners = [at(p, s), at(p, e), at(q, e), at(q, s)];
                    let n = cross3(sub3(corners[2], corners[0]), sub3(corners[1], corners[3]));
                    let Some(n) = norm3(n) else { continue };
                    let t = round((s + e) / 2.0);
                    let u = |a: f64| (a - a0) / span;
                    let uvs = [[u(s), p.v], [u(e), p.v], [u(e), q.v], [u(s), q.v]];
                    let k = self.len();
                    for (c, uv) in corners.iter().zip(uvs) {
                        self.push(*c, n, t, uv);
                    }
                    self.tri(k + 3, k + 2, k + 1);
                    self.tri(k + 3, k + 1, k);
                }
            }
        } else {
            let first = self.len();
            for p in &profile {
                for &a in &angles {
                    let n = [p.normal[0] * a.cos(), p.normal[1], -p.normal[0] * a.sin()];
                    self.push(at(p, a), n, round(a), [(a - a0) / span, p.v]);
                }
            }
            let cols32 = cols as u32;
            for i in 0..rows - 1 {
                if dist(profile[i].at, profile[i + 1].at) <= TINY {
                    continue;
                }
                for j in 0..cols32 - 1 {
                    let a = first + i as u32 * cols32 + j;
                    let (b, c, d) = (a + 1, a + cols32 + 1, a + cols32);
                    self.tri(d, c, b);
                    self.tri(d, b, a);
                }
            }
        }
        if !full {
            let end = angles[cols - 1];
            self.cap(&profile, a0, [a0.sin(), 0.0, a0.cos()]);
            self.cap(&profile, end, [-end.sin(), 0.0, -end.cos()]);
        }
    }

    /// The profile at angle `a` filled flat, facing `normal`. Every profile is
    /// convex, so a fan from its middle covers it.
    fn cap(&mut self, profile: &[Pt], a: f64, normal: V3) {
        let mut ring: Vec<[f64; 2]> = profile.iter().map(|p| p.at).collect();
        ring.dedup_by(|x, y| dist(*x, *y) <= TINY);
        if ring.len() > 2 && dist(ring[0], ring[ring.len() - 1]) <= TINY {
            ring.pop();
        }
        if ring.len() < 3 {
            return;
        }
        let n = ring.len() as f64;
        let mid = ring
            .iter()
            .fold([0.0; 2], |s, p| [s[0] + p[0] / n, s[1] + p[1] / n]);
        let (lo, hi) = ring.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        });
        let wide = ring.iter().fold(TINY, |w, p| w.max(p[0]));
        let at = |p: [f64; 2]| [p[0] * a.cos(), p[1], -p[0] * a.sin()];
        let uv = |p: [f64; 2]| [p[0] / wide, (hi - p[1]) / (hi - lo).max(TINY)];
        let along = [a.cos(), 0.0, -a.sin()];
        let centre = self.push(at(mid), normal, along, uv(mid));
        let first = self.len();
        for p in &ring {
            self.push(at(*p), normal, along, uv(*p));
        }
        // The fan winds whichever way faces `normal`.
        let facing = dot3(
            cross3(sub3(at(ring[0]), at(mid)), sub3(at(ring[1]), at(mid))),
            normal,
        ) >= 0.0;
        let count = ring.len() as u32;
        for k in 0..count {
            let (p, q) = (first + k, first + (k + 1) % count);
            match facing {
                true => self.tri(centre, p, q),
                false => self.tri(centre, q, p),
            }
        }
    }
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] * s, a[1] * s]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

fn unit(a: [f64; 2]) -> [f64; 2] {
    let l = dot(a, a).sqrt();
    if l > TINY {
        scale(a, 1.0 / l)
    } else {
        [0.0; 2]
    }
}

fn sub3(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len3(a: V3) -> f64 {
    dot3(a, a).sqrt()
}

fn norm3(a: V3) -> Option<V3> {
    let l = len3(a);
    (l > TINY).then(|| a.map(|c| c / l))
}

fn cross3(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn volume(shape: MeshShape, rows: &[(&str, f64)]) -> f64 {
        let mut options = crate::fx::builtins::instantiate(shape.options()).unwrap();
        for (id, v) in rows {
            let p = options.params.iter_mut().find(|p| p.id == *id).unwrap();
            p.value = EffectValue::Float(Property::fixed(*v));
        }
        let f = mesh(shape, Some(&options), &|p| p.value_at(0.0));
        assert!(f.positions.iter().flatten().all(|c| c.is_finite()));
        // Outward triangles enclose a positive volume, so one wound the wrong
        // way, which the mesh pass would cull, shows as a shortfall.
        f.indices
            .chunks(3)
            .map(|t| {
                let p = |k: usize| f.positions[t[k] as usize].map(f64::from);
                dot3(p(0), cross3(p(1), p(2))) / 6.0
            })
            .sum()
    }

    /// Every shape is closed and faces outward, whatever its rows: its
    /// triangles enclose the volume it should have.
    #[test]
    fn every_shape_is_closed_and_faces_out() {
        let near = |got: f64, want: f64| (got - want).abs() / want < 0.02;
        let ball = 4.0 / 3.0 * PI * 100f64.powi(3);
        let can = PI * 100f64.powi(2) * 200.0;
        type Case = (MeshShape, &'static [(&'static str, f64)], f64);
        let cases: [Case; 9] = [
            (MeshShape::Cube, &[], 200f64.powi(3)),
            (
                MeshShape::Cube,
                &[("bevel", 30.0)],
                200f64.powi(3)
                    - 30f64.powi(2) * 140.0 * 12.0 * (1.0 - PI / 4.0)
                    - 30f64.powi(3) * 8.0 * (1.0 - PI / 6.0),
            ),
            (MeshShape::Sphere, &[("segments", 128.0)], ball),
            (
                MeshShape::Sphere,
                &[
                    ("segments", 128.0),
                    ("slice_start", 90.0),
                    ("slice_end", 270.0),
                ],
                ball / 2.0,
            ),
            (MeshShape::Cylinder, &[("segments", 256.0)], can),
            (
                MeshShape::Cylinder,
                &[("segments", 4.0)],
                200.0 * 2.0 * 100f64.powi(2),
            ),
            (MeshShape::Cone, &[("segments", 256.0)], can / 3.0),
            (
                MeshShape::Torus,
                &[("segments", 256.0), ("pipe_segments", 64.0)],
                2.0 * PI * PI * 100.0 * 900.0,
            ),
            (
                MeshShape::Torus,
                &[
                    ("segments", 256.0),
                    ("pipe_segments", 64.0),
                    ("slice_end", 180.0),
                ],
                PI * PI * 100.0 * 900.0,
            ),
        ];
        for (shape, rows, want) in cases {
            let got = volume(shape, rows);
            assert!(near(got, want), "{shape:?} {rows:?}: {got} not {want}");
        }
        // A plane is flat, and nought across is nothing at all.
        assert!(volume(MeshShape::Plane, &[("corner_radius", 50.0)]).abs() < 1e-3);
        assert_eq!(volume(MeshShape::Sphere, &[("radius", 0.0)]), 0.0);
    }
}

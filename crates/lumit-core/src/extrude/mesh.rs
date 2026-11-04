//! Outlines made solid: a cap in front, a bevel round it, a side, a second
//! bevel and a cap behind.
//!
//! The space is the layer's own pixels, y down, and z going away from the eye
//! with the front cap on the layer's plane, so a layer keeps its place and its
//! size as it gains depth.
//!
//! Every step is plain arithmetic in a fixed order, so the same outline gives
//! the same triangles on any run. An outline that can't be cleaned or capped
//! loses that part and keeps the rest.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use super::{BevelStyle, Profile};

/// Which of a mesh's faces is which, and the order the materials come in.
pub const FRONT: usize = 0;
pub const BEVEL: usize = 1;
pub const SIDE: usize = 2;
pub const BACK: usize = 3;

/// Past this, a point's neighbours along the bevel may fold over each other.
const MITRE_LIMIT: f64 = 4.0;
/// Where a line turns by more than this (the cosine of 30 degrees) it's a
/// corner, with a crease, and not a curve.
const CREASE: f64 = 0.866;
/// How far apart coordinates can be before a path is called wild and left out.
const WILD: f64 = 1e8;

/// Triangles sharing one of a mesh's four materials. Three to a triangle, in
/// layer pixels. `uv0` is pixels too until the whole layer's box is known.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Face {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// One outline made solid, by [`FRONT`], [`BEVEL`], [`SIDE`] and [`BACK`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub faces: [Face; 4],
}

impl Mesh {
    /// Roughly the memory it holds.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        self.faces
            .iter()
            .map(|f| f.positions.len() * 48 + f.indices.len() * 4)
            .sum()
    }
}

/// `contours`, however they cross, as one solid: filled by the non-zero rule,
/// as a shape's fill and a letter are.
#[must_use]
pub fn solid(contours: &[Vec<[f64; 2]>], profile: &Profile) -> Mesh {
    let mut mesh = Mesh::default();
    for shape in clean(contours) {
        shape_into(&shape, profile, &mut mesh);
    }
    mesh
}

/// Separate polygons, each its outer ring and then its holes. The rings never
/// cross, and with the solid always on their left.
fn clean(contours: &[Vec<[f64; 2]>]) -> Vec<Vec<Vec<[f64; 2]>>> {
    let rings: Vec<Vec<[f64; 2]>> = contours
        .iter()
        .map(|c| {
            let finite = c.iter().filter(|p| p[0].is_finite() && p[1].is_finite());
            finite.copied().collect::<Vec<_>>()
        })
        .filter(|c| c.len() >= 3)
        .collect();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in rings.iter().flatten() {
        (lo[0], lo[1]) = (lo[0].min(p[0]), lo[1].min(p[1]));
        (hi[0], hi[1]) = (hi[0].max(p[0]), hi[1].max(p[1]));
    }
    // Nothing to fill, a line with no area, or numbers past all reason.
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    if rings.is_empty() || !(w > 1e-6 && h > 1e-6 && w < WILD && h < WILD) {
        return Vec::new();
    }
    let none: Vec<Vec<[f64; 2]>> = Vec::new();
    rings.overlay(&none, OverlayRule::Subject, FillRule::NonZero)
}

/// A ring ready to be walked: its points, the way out of the solid at and
/// between them, and how far each may move in for the bevel.
struct Ring {
    points: Vec<[f64; 2]>,
    /// The way out of the solid across the edge from each point to the next.
    out: Vec<[f64; 2]>,
    /// Between the edges either side of each point, for a curve.
    smooth: Vec<[f64; 2]>,
    crease: Vec<bool>,
    /// Where each point moves to for the cap, as an offset from it.
    inset: Vec<[f64; 2]>,
    /// How far in that is square to the outline, which sets the bevel's slope.
    square: Vec<f64>,
    /// How far along the ring each point is, then the whole way round.
    along: Vec<f64>,
    /// How deep its bevel is in z: the layer's, or a hole's share of it.
    bevel: f64,
}

impl Ring {
    fn new(raw: &[[f64; 2]], bevel: f64) -> Option<Self> {
        let mut points: Vec<[f64; 2]> = Vec::with_capacity(raw.len());
        for p in raw {
            if points.last() != Some(p) {
                points.push(*p);
            }
        }
        if points.first() == points.last() {
            points.pop();
        }
        let n = points.len();
        if n < 3 {
            return None;
        }
        let mut out = Vec::with_capacity(n);
        let mut along = Vec::with_capacity(n + 1);
        let mut run = 0.0;
        for i in 0..n {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy).max(1e-12);
            along.push(run);
            run += len;
            out.push([dy / len, -dx / len]);
        }
        along.push(run);
        let (mut smooth, mut crease, mut inset) = (
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        );
        for i in 0..n {
            let (a, b) = (out[(i + n - 1) % n], out[i]);
            let (sx, sy) = (a[0] + b[0], a[1] + b[1]);
            let len = sx.hypot(sy);
            // A line doubling straight back has no between: take the next edge's.
            let m = if len > 1e-9 { [sx / len, sy / len] } else { b };
            let cos = m[0] * b[0] + m[1] * b[1];
            smooth.push(m);
            crease.push(a[0] * b[0] + a[1] * b[1] < CREASE);
            let far = (1.0 / cos.max(1.0 / MITRE_LIMIT)) * bevel;
            inset.push([-m[0] * far, -m[1] * far]);
        }
        Some(Self {
            square: vec![bevel; n],
            points,
            out,
            smooth,
            crease,
            inset,
            along,
            bevel,
        })
    }

    /// The ring at its cap.
    fn capped(&self) -> Vec<[f64; 2]> {
        self.points
            .iter()
            .zip(&self.inset)
            .map(|(p, i)| [p[0] + i[0], p[1] + i[1]])
            .collect()
    }
}

/// Stop each ring's points where they would meet another part of the outline
/// moving in towards them, so a narrow stem's bevel rises to a ridge and never
/// folds through itself.
fn limit_insets(rings: &mut [Ring]) {
    // Every edge, by the left end of its box, so a point only looks at those
    // near it.
    let mut edges: Vec<(f64, f64, usize, usize)> = Vec::new();
    for (r, ring) in rings.iter().enumerate() {
        let n = ring.points.len();
        for i in 0..n {
            let (a, b) = (ring.points[i], ring.points[(i + 1) % n]);
            edges.push((a[0].min(b[0]), a[0].max(b[0]), r, i));
        }
    }
    edges.sort_by(|a, b| {
        (a.0.total_cmp(&b.0))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    let widest = edges.iter().map(|e| e.1 - e.0).fold(0.0, f64::max);
    let mut settled: Vec<Vec<f64>> = Vec::with_capacity(rings.len());
    for (r, ring) in rings.iter().enumerate() {
        let n = ring.points.len();
        let mut scale = vec![1.0f64; n];
        for (i, slot) in scale.iter_mut().enumerate() {
            let p = ring.points[i];
            let d = ring.inset[i];
            let wanted = d[0].hypot(d[1]);
            if wanted <= 0.0 {
                continue;
            }
            let u = [d[0] / wanted, d[1] / wanted];
            // A front moving in from the far side meets this one half way.
            let reach = wanted * 2.0;
            let from = p[0] - reach.max(0.0) - widest;
            let start = edges.partition_point(|e| e.0 < from);
            let mut nearest = f64::MAX;
            for e in edges[start..].iter().take_while(|e| e.0 <= p[0] + reach) {
                let (er, ei) = (e.2, e.3);
                let en = rings[er].points.len();
                if er == r && (ei == i || ei == (i + n - 1) % n) {
                    continue;
                }
                let (a, b) = (rings[er].points[ei], rings[er].points[(ei + 1) % en]);
                if let Some(s) = ray_meets(p, u, a, b) {
                    nearest = nearest.min(s);
                }
            }
            *slot = (nearest * 0.5 / wanted).clamp(0.0, 1.0);
        }
        // Neighbours can't differ by more than the edge between them, so the
        // cap's edge never turns over.
        let mut far: Vec<f64> = (0..n)
            .map(|i| ring.inset[i][0].hypot(ring.inset[i][1]) * scale[i])
            .collect();
        for _ in 0..2 {
            for i in 0..n {
                let prev = (i + n - 1) % n;
                let edge = ring.along[prev + 1] - ring.along[prev];
                far[i] = far[i].min(far[prev] + edge);
            }
            for i in (0..n).rev() {
                let next = (i + 1) % n;
                let edge = ring.along[i + 1] - ring.along[i];
                far[i] = far[i].min(far[next] + edge);
            }
        }
        // Where the cap's edge would run backwards against the outline's, as at
        // a tight inner join, both its ends come back until it doesn't.
        for _ in 0..16 {
            let mut turned = false;
            for i in 0..n {
                let next = (i + 1) % n;
                let (a, b) = (ring.points[i], ring.points[next]);
                let reach = |k: usize, far: &[f64]| {
                    let d = ring.inset[k];
                    let wanted = d[0].hypot(d[1]);
                    let share = if wanted > 0.0 { far[k] / wanted } else { 0.0 };
                    [
                        ring.points[k][0] + d[0] * share,
                        ring.points[k][1] + d[1] * share,
                    ]
                };
                let (ca, cb) = (reach(i, &far), reach(next, &far));
                let along = (cb[0] - ca[0]) * (b[0] - a[0]) + (cb[1] - ca[1]) * (b[1] - a[1]);
                if along < 0.0 {
                    far[i] *= 0.75;
                    far[next] *= 0.75;
                    turned = true;
                }
            }
            if !turned {
                break;
            }
        }
        settled.push(far);
    }
    for (ring, far) in rings.iter_mut().zip(settled) {
        for (i, far) in far.into_iter().enumerate() {
            let d = ring.inset[i];
            let wanted = d[0].hypot(d[1]);
            let share = if wanted > 0.0 { far / wanted } else { 0.0 };
            ring.inset[i] = [d[0] * share, d[1] * share];
            ring.square[i] = ring.bevel * share;
        }
    }
}

/// How far along `u` from `p` the edge from `a` to `b` is met, if it is.
fn ray_meets(p: [f64; 2], u: [f64; 2], a: [f64; 2], b: [f64; 2]) -> Option<f64> {
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let det = u[0] * ey - u[1] * ex;
    if det.abs() < 1e-12 {
        return None;
    }
    let (wx, wy) = (a[0] - p[0], a[1] - p[1]);
    let s = (wx * ey - wy * ex) / det;
    let k = (wx * u[1] - wy * u[0]) / det;
    (s > 1e-9 && (0.0..=1.0).contains(&k)).then_some(s)
}

/// One row of a strip across a ring.
#[derive(Clone, Copy)]
struct Row {
    /// How much of the way to its cap each point has moved.
    inset: f64,
    z: f64,
    /// The slope here, for the normal: z for each part of the inset.
    dz: f64,
    dout: f64,
    /// Facing the back rather than the front.
    back: bool,
}

/// A bevel's rows from cap to side, nearest the front first.
fn bevel_rows(p: &Profile, z_span: f64, total: f64, back: bool) -> Vec<Row> {
    let segments = match p.style {
        BevelStyle::Angular => 1,
        _ => p.segments.max(1),
    };
    let mut rows: Vec<Row> = (0..=segments)
        .map(|k| {
            let t = f64::from(k) / f64::from(segments);
            let a = t * std::f64::consts::FRAC_PI_2;
            let (s, c) = a.sin_cos();
            let (inset, z, dz, dout) = match p.style {
                BevelStyle::Angular => (1.0 - t, t * z_span, z_span, 1.0),
                BevelStyle::Convex => (1.0 - s, (1.0 - c) * z_span, s * z_span, c),
                BevelStyle::Concave => (c, s * z_span, c * z_span, s),
            };
            let z = if back { total - z } else { z };
            Row {
                inset,
                z,
                dz,
                dout,
                back,
            }
        })
        .collect();
    // The back's run from the side to the cap, still nearest the front first.
    if back {
        rows.reverse();
    }
    rows
}

/// A strip of quads round `ring` through `rows`, which run nearest the front
/// first, its normals smooth along a curve and creased at a corner.
fn strip(face: &mut Face, ring: &Ring, rows: &[Row]) {
    let n = ring.points.len();
    let Ok(base) = u32::try_from(face.positions.len()) else {
        return;
    };
    let across = rows.len() as u32;
    let mut column = 0u32;
    let mut starts = Vec::with_capacity(n);
    for walk in 0..=n {
        let i = walk % n;
        // Each edge starts at one column and ends at another, the same one
        // on a curve, and the first point closes the ring in a column of its
        // own so the uvs run on to the full length.
        let creased = ring.crease[i] || walk == n || walk == 0;
        let last = (i + n - 1) % n;
        let sides: &[[f64; 2]] = if !creased {
            &[ring.smooth[i]]
        } else if walk == n {
            &[ring.out[last]]
        } else if walk == 0 {
            &[ring.out[i]]
        } else {
            &[ring.out[last], ring.out[i]]
        };
        if walk < n {
            starts.push(column + sides.len() as u32 - 1);
        }
        for &out in sides {
            let edge = [-out[1], out[0]];
            for row in rows {
                let p = ring.points[i];
                let d = ring.inset[i];
                let (x, y) = (p[0] + d[0] * row.inset, p[1] + d[1] * row.inset);
                let slope = row.dout * ring.square[i];
                let along = row.dz;
                let len = along.hypot(slope);
                let (no, nz) = if len > 1e-9 {
                    (along / len, slope / len)
                } else {
                    (1.0, 0.0)
                };
                let nz = if row.back { nz } else { -nz };
                face.positions.push([x as f32, y as f32, row.z as f32]);
                face.normals
                    .push([(out[0] * no) as f32, (out[1] * no) as f32, nz as f32]);
                face.tangents
                    .push([edge[0] as f32, edge[1] as f32, 0.0, 1.0]);
                face.uv0
                    .push([ring.along[walk.min(n)] as f32, row.z as f32]);
            }
            column += 1;
        }
    }
    for (i, &start) in starts.iter().enumerate() {
        // The next point's last column is where this edge ends.
        let end = starts
            .get(i + 1)
            .map_or(column - 1, |&s| if ring.crease[i + 1] { s - 1 } else { s });
        for r in 0..across.saturating_sub(1) {
            let at = |c: u32, r: u32| base + c * across + r;
            let (a, b, c, d) = (at(start, r), at(end, r), at(end, r + 1), at(start, r + 1));
            face.indices.extend([a, b, c, a, c, d]);
        }
    }
}

type Cdt = ConstrainedDelaunayTriangulation<Point2<f64>>;
/// A cap's points and its triangles over them.
type Capped = (Vec<[f64; 2]>, Vec<[u32; 3]>);

/// `rings` triangulated, every triangle counter-clockwise in x and y, kept
/// where an odd number of rings surround it. `None` when two rings cross.
fn triangulated(rings: &[Vec<[f64; 2]>]) -> Option<Capped> {
    let mut cdt = Cdt::new();
    for ring in rings {
        let mut handles = Vec::with_capacity(ring.len());
        for p in ring {
            handles.push(cdt.insert(Point2::new(p[0], p[1])).ok()?);
        }
        for (i, &a) in handles.iter().enumerate() {
            let b = handles[(i + 1) % handles.len()];
            if a == b {
                continue;
            }
            // `add_constraint` panics on a crossing, so this is no courtesy.
            if !cdt.can_add_constraint(a, b) {
                return None;
            }
            cdt.add_constraint(a, b);
        }
    }
    if cdt.num_inner_faces() == 0 {
        return Some((Vec::new(), Vec::new()));
    }
    // In from the outside, crossing a ring each time the answer flips.
    let mut inside: Vec<Option<bool>> = vec![None; cdt.num_all_faces()];
    let mut stack = Vec::new();
    for edge in cdt.convex_hull() {
        if let Some(face) = edge.rev().face().as_inner() {
            let keep = edge.as_undirected().is_constraint_edge();
            if inside[face.fix().index()].is_none() {
                inside[face.fix().index()] = Some(keep);
                stack.push(face.fix());
            }
        }
    }
    while let Some(fixed) = stack.pop() {
        let face = cdt.face(fixed);
        let here = inside[fixed.index()].unwrap_or(false);
        for edge in face.adjacent_edges() {
            if let Some(next) = edge.rev().face().as_inner() {
                let slot = &mut inside[next.fix().index()];
                if slot.is_none() {
                    *slot = Some(here != edge.as_undirected().is_constraint_edge());
                    stack.push(next.fix());
                }
            }
        }
    }
    let mut remap = vec![u32::MAX; cdt.num_vertices()];
    let mut points = Vec::new();
    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        if inside[face.fix().index()] != Some(true) {
            continue;
        }
        let mut tri = [0u32; 3];
        for (slot, v) in tri.iter_mut().zip(face.vertices()) {
            let at = &mut remap[v.fix().index()];
            if *at == u32::MAX {
                *at = points.len() as u32;
                points.push([v.position().x, v.position().y]);
            }
            *slot = *at;
        }
        triangles.push(tri);
    }
    Some((points, triangles))
}

/// `rings` as a flat cap at depth `z`, facing the eye or away from it.
fn cap(face: &mut Face, points: &[[f64; 2]], triangles: &[[u32; 3]], z: f64, front: bool) {
    let Ok(base) = u32::try_from(face.positions.len()) else {
        return;
    };
    let (nz, w) = if front { (-1.0, -1.0) } else { (1.0, 1.0) };
    for p in points {
        face.positions.push([p[0] as f32, p[1] as f32, z as f32]);
        face.normals.push([0.0, 0.0, nz]);
        face.tangents.push([1.0, 0.0, 0.0, w]);
        face.uv0.push([p[0] as f32, p[1] as f32]);
    }
    for t in triangles {
        // Counter-clockwise in x and y looks away from the eye.
        let [a, b, c] = t.map(|i| base + i);
        face.indices
            .extend(if front { [a, c, b] } else { [a, b, c] });
    }
}

fn shape_into(shape: &[Vec<[f64; 2]>], p: &Profile, mesh: &mut Mesh) {
    let bevel = p.bevel.max(0.0);
    let total = p.depth.max(0.0) + bevel * 2.0;
    let mut rings: Vec<Ring> = shape
        .iter()
        .enumerate()
        .filter_map(|(i, raw)| {
            let share = if i == 0 { 1.0 } else { p.hole.clamp(0.0, 1.0) };
            Ring::new(raw, bevel * share)
        })
        .collect();
    if rings.is_empty() {
        return;
    }
    if bevel > 0.0 {
        limit_insets(&mut rings);
    }
    let capped: Vec<Vec<[f64; 2]>> = rings.iter().map(Ring::capped).collect();
    // Folded rings are cleaned for the cap alone. The bevel still meets them.
    let made = triangulated(&capped).or_else(|| {
        let flat: Vec<Vec<[f64; 2]>> = clean(&capped).into_iter().flatten().collect();
        triangulated(&flat)
    });
    let [front, bevels, side, back] = &mut mesh.faces;
    if let Some((points, triangles)) = made {
        cap(front, &points, &triangles, 0.0, true);
        if total > 0.0 {
            cap(back, &points, &triangles, total, false);
        }
    }
    if total <= 0.0 {
        return;
    }
    for ring in &rings {
        if ring.bevel > 0.0 {
            let span = ring.bevel;
            strip(bevels, ring, &bevel_rows(p, span, total, false));
            strip(bevels, ring, &bevel_rows(p, span, total, true));
        }
        let (near, far) = (ring.bevel, total - ring.bevel);
        if far > near {
            let row = |z| Row {
                inset: 0.0,
                z,
                dz: 1.0,
                dout: 0.0,
                back: false,
            };
            strip(side, ring, &[row(near), row(far)]);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// Every triangle turns the way its normals face, and so shows from
    /// outside, with the mesh's faces where they should be.
    #[test]
    fn a_ring_with_a_hole_makes_a_closed_solid() {
        let outer = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let hole = vec![[30.0, 70.0], [70.0, 70.0], [70.0, 30.0], [30.0, 30.0]];
        for style in [BevelStyle::Angular, BevelStyle::Convex, BevelStyle::Concave] {
            let profile = Profile {
                depth: 40.0,
                bevel: 8.0,
                style,
                segments: 3,
                hole: 1.0,
            };
            let mesh = solid(&[outer.clone(), hole.clone()], &profile);
            for (k, face) in mesh.faces.iter().enumerate() {
                assert!(!face.indices.is_empty(), "face {k} is empty");
                for t in face.indices.chunks_exact(3) {
                    let [a, b, c] = [t[0], t[1], t[2]].map(|i| face.positions[i as usize]);
                    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let g = cross(e1, e2);
                    let n: [f32; 3] = [t[0], t[1], t[2]]
                        .iter()
                        .map(|i| face.normals[*i as usize])
                        .fold([0.0; 3], |s, n| [s[0] + n[0], s[1] + n[1], s[2] + n[2]]);
                    let dot = g[0] * n[0] + g[1] * n[1] + g[2] * n[2];
                    assert!(dot > -1e-3, "face {k} shows from inside, {style:?}");
                }
            }
            // The caps sit at the front and the back, the front's inset.
            let z = |f: &Face| f.positions[0][2];
            assert_eq!(z(&mesh.faces[FRONT]), 0.0);
            assert_eq!(z(&mesh.faces[BACK]), 56.0);
            let left = mesh.faces[FRONT]
                .positions
                .iter()
                .map(|p| p[0])
                .fold(f32::MAX, f32::min);
            assert!((left - 8.0).abs() < 0.01, "cap starts at {left}");
            let hole_in = mesh.faces[FRONT]
                .positions
                .iter()
                .any(|p| (p[0] - 22.0).abs() < 0.01);
            assert!(hole_in, "the hole's bevel moved out from the hole");
        }
    }

    /// The same outline gives the same triangles, and no outline however bad
    /// panics or hangs: crossings, doubled points, slivers, lines and numbers
    /// past all reason each lose what can't be made and keep the rest.
    #[test]
    fn any_outline_is_safe_and_repeats() {
        let profile = Profile {
            depth: 30.0,
            bevel: 12.0,
            style: BevelStyle::Convex,
            segments: 4,
            hole: 0.5,
        };
        let mut state = 0x9e37_79b9u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f64::from(state % 1000) / 10.0
        };
        let mut scribble: Vec<Vec<[f64; 2]>> = Vec::new();
        for _ in 0..6 {
            scribble.push((0..40).map(|_| [next(), next()]).collect());
        }
        let bad: Vec<Vec<Vec<[f64; 2]>>> = vec![
            Vec::new(),
            vec![Vec::new()],
            vec![vec![[1.0, 1.0]; 8]],
            vec![vec![[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]]],
            vec![vec![[f64::NAN, 0.0], [f64::INFINITY, 1.0], [2.0, -3.0]]],
            vec![vec![[0.0, 0.0], [1e300, 0.0], [0.0, 1e300]]],
            // A bow tie, and a sliver far narrower than the bevel.
            vec![vec![[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]]],
            vec![vec![[0.0, 0.0], [300.0, 0.0], [300.0, 0.5], [0.0, 0.5]]],
            scribble.clone(),
        ];
        for outline in &bad {
            assert_eq!(solid(outline, &profile), solid(outline, &profile));
        }
        assert!(!solid(&scribble, &profile).faces[SIDE].indices.is_empty());
        // The sliver still draws, its bevel a ridge rather than a fold.
        let sliver = solid(&bad[7], &profile);
        assert!(sliver.faces[BEVEL].positions.iter().all(|p| p[1] >= -0.01));
    }

    /// With no depth and no bevel, only the front.
    #[test]
    fn nothing_but_a_front_at_nought() {
        let square = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]];
        let mesh = solid(
            &square,
            &Profile {
                depth: 0.0,
                bevel: 0.0,
                style: BevelStyle::Angular,
                segments: 1,
                hole: 1.0,
            },
        );
        assert_eq!(mesh.faces[FRONT].indices.len(), 6);
        for face in [BEVEL, SIDE, BACK] {
            assert!(mesh.faces[face].indices.is_empty());
        }
    }
}

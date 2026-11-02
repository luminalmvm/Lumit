//! How a run of 3D layers is ordered and cut where its layers meet.

use super::{Cut, CUT_LIFT, CUT_NEAR, CUT_PLANES, PIECE_CORNERS};
use glam::Mat4;

/// One flat part of a run of 3D layers, as the order sees it: a whole layer,
/// or a piece of one cut along the planes of the layers it crosses.
struct Shape {
    layer: usize,
    /// Its corners in the layer's own 0..1 space, and in comp pixels.
    uv: Vec<glam::Vec2>,
    pts: Vec<glam::Vec3>,
    /// Clip w at its middle, which is how far along the view it is.
    depth: f32,
    /// Where it lands on screen. None when a corner is behind the eye, which
    /// could reach anywhere.
    bounds: Option<[f32; 4]>,
}

impl Shape {
    fn new(layer: usize, uv: Vec<glam::Vec2>, world: Mat4, view_proj: Mat4) -> Self {
        let pts: Vec<glam::Vec3> = uv
            .iter()
            .map(|c| world.transform_point3(c.extend(0.0)))
            .collect();
        let middle = pts.iter().copied().sum::<glam::Vec3>() / pts.len().max(1) as f32;
        let bounds = pts
            .iter()
            .try_fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
                let q = view_proj * p.extend(1.0);
                (q.w > 0.0).then(|| {
                    let (x, y) = (q.x / q.w, q.y / q.w);
                    [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
                })
            });
        Self {
            layer,
            uv,
            depth: (view_proj * middle.extend(1.0)).w,
            pts,
            bounds,
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        match (self.bounds, other.bounds) {
            (Some(p), Some(q)) => p[0] < q[2] && q[0] < p[2] && p[1] < q[3] && q[1] < p[3],
            _ => true,
        }
    }

    /// Where this shape crosses `plane`, as a span along `along`.
    fn chord(
        &self,
        (normal, origin): (glam::Vec3, glam::Vec3),
        along: glam::Vec3,
    ) -> Option<(f32, f32)> {
        let mut span: Option<(f32, f32)> = None;
        for (k, &p) in self.pts.iter().enumerate() {
            let q = self.pts[(k + 1) % self.pts.len()];
            let (dp, dq) = (normal.dot(p - origin), normal.dot(q - origin));
            if (dp >= 0.0) != (dq >= 0.0) {
                let t = along.dot(p + (q - p) * (dp / (dp - dq)));
                span = Some(span.map_or((t, t), |(lo, hi)| (lo.min(t), hi.max(t))));
            }
        }
        span
    }
}

/// Farther first, then lower in the stack.
struct Far(f32, usize);

impl PartialEq for Far {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Far {}

impl PartialOrd for Far {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Far {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0).then(other.1.cmp(&self.1))
    }
}

/// Pairs of shapes, by index.
type Pairs = Vec<(usize, usize)>;

/// The order `shapes` draw in, farthest first, with the flush and crossing
/// pairs it found. Of two that overlap on screen, the one wholly behind the
/// other's plane draws first. Two that only straddle each other's planes,
/// without meeting, go by how far their middles are. `planes` is each layer's
/// normal and a point on it, and `eye` the camera's position.
fn arrange(
    shapes: &[Shape],
    planes: &[Option<(glam::Vec3, glam::Vec3)>],
    eye: glam::Vec3,
) -> (Vec<usize>, Pairs, Pairs) {
    // Closer than this to a plane, in comp pixels, is on it.
    const ON: f32 = 0.01;
    // Which side of `b`'s plane `a` lies on. A plane seen edge on decides
    // nothing.
    enum Side {
        Front,
        Behind,
        On,
        Across,
    }
    let side = |a: &Shape, b: &Shape| {
        let Some((normal, origin)) = planes[b.layer] else {
            return Side::Across;
        };
        let toward = normal.dot(eye - origin);
        if toward.abs() <= ON {
            return Side::Across;
        }
        let (mut front, mut behind) = (false, false);
        for p in &a.pts {
            let d = normal.dot(*p - origin) * toward.signum();
            front |= d > ON;
            behind |= d < -ON;
        }
        match (front, behind) {
            (false, false) => Side::On,
            (true, false) => Side::Front,
            (false, true) => Side::Behind,
            (true, true) => Side::Across,
        }
    };
    // Two that straddle each other's planes really cross when their chords
    // along the line the planes share overlap.
    let meet = |a: &Shape, b: &Shape| {
        let (Some(pa), Some(pb)) = (planes[a.layer], planes[b.layer]) else {
            return false;
        };
        let along = pa.0.cross(pb.0);
        match (a.chord(pb, along), b.chord(pa, along)) {
            (Some((a0, a1)), Some((b0, b1))) => a0 < b1 && b0 < a1,
            _ => false,
        }
    };
    let n = shapes.len();
    // Each shape's list of shapes that must wait for it, and how many it
    // waits for itself.
    let mut then: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut waits = vec![0usize; n];
    let (mut flush, mut cross) = (Vec::new(), Vec::new());
    for a in 0..n {
        for b in a + 1..n {
            let (sa, sb) = (&shapes[a], &shapes[b]);
            if sa.layer == sb.layer || !sa.overlaps(sb) {
                continue;
            }
            let (first, second) = match side(sa, sb) {
                Side::On => {
                    flush.push((a, b));
                    (a, b)
                }
                Side::Behind => (a, b),
                Side::Front => (b, a),
                Side::Across => match side(sb, sa) {
                    Side::Behind => (b, a),
                    Side::Front => (a, b),
                    Side::On | Side::Across => {
                        if meet(sa, sb) {
                            cross.push((a, b));
                        }
                        continue;
                    }
                },
            };
            then[first].push(second);
            waits[second] += 1;
        }
    }
    let mut free: std::collections::BinaryHeap<Far> = (0..n)
        .filter(|&i| waits[i] == 0)
        .map(|i| Far(shapes[i].depth, i))
        .collect();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        // The farthest shape free to go. When a loop leaves none free, the
        // farthest of those waiting on the fewest, so the shapes that only
        // follow the loop keep their place.
        let next = free.pop().map(|Far(_, i)| i).or_else(|| {
            (0..n)
                .filter(|&i| !done[i])
                .max_by_key(|&i| (std::cmp::Reverse(waits[i]), Far(shapes[i].depth, i)))
        });
        let Some(i) = next else {
            break;
        };
        done[i] = true;
        order.push(i);
        for &j in &then[i] {
            waits[j] = waits[j].saturating_sub(1);
            if waits[j] == 0 && !done[j] {
                free.push(Far(shapes[j].depth, j));
            }
        }
    }
    (order, flush, cross)
}

/// The part of the convex `poly` where `line.x * u + line.y * v + line.z` is
/// not negative. A crossing point is worked out from the same end of its edge
/// whichever side is kept, so the two halves meet without a crack, and it
/// stays exactly on an edge of the layer it falls on, so the outline keeps no
/// nick.
fn keep_side(poly: &[glam::Vec2], line: glam::Vec3) -> Vec<glam::Vec2> {
    let f = |p: glam::Vec2| line.x * p.x + line.y * p.y + line.z;
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (k, &p) in poly.iter().enumerate() {
        let q = poly[(k + 1) % poly.len()];
        let (fp, fq) = (f(p), f(q));
        if fp >= 0.0 {
            out.push(p);
        }
        if (fp >= 0.0) != (fq >= 0.0) {
            let (a, b, fa, fb) = if (p.x, p.y) < (q.x, q.y) {
                (p, q, fp, fq)
            } else {
                (q, p, fq, fp)
            };
            let mut at = a + (b - a) * (fa / (fa - fb));
            if a.x == b.x {
                at.x = a.x;
            }
            if a.y == b.y {
                at.y = a.y;
            }
            out.push(at);
        }
    }
    out
}

/// Twice the area of `poly`, in the layer's own 0..1 space.
fn area(poly: &[glam::Vec2]) -> f32 {
    (0..poly.len())
        .map(|k| poly[k].perp_dot(poly[(k + 1) % poly.len()]))
        .sum::<f32>()
        .abs()
}

/// How a run of adjacent 3D layers draws, as layers of `worlds` (each one's
/// [`super::CompositeLayer::world`], bottom of the stack first) farthest first, each
/// with how it is cut. `flat` marks a layer drawn as a flat smear (motion
/// blur), which can't be cut itself. `view_proj` is the [`super::camera_matrix`] and
/// `eye` the camera's position in comp pixels.
///
/// After Effects splits 3D layers where they pass through each other, so what
/// it shows is always the true order, not a sort by some one point. So here
/// of two layers that overlap on screen, the one wholly behind the other's
/// plane draws first, and two on one plane keep their stack order. Of two that
/// cross, the one that crosses fewer layers is cut into pieces along the
/// other's plane (both, when they cross as many), and the pieces take their
/// place the same way, which is exact at any opacity. A group of crossing
/// layers that would need too many pieces is cut by depth instead, which is
/// exact for what is solid.
///
/// ponytail: the eye-side test and the depth by clip w assume a perspective
/// camera. An orthographic view needs its view direction here instead.
pub fn depth_order(
    worlds: &[Mat4],
    flat: &[bool],
    view_proj: Mat4,
    eye: [f32; 3],
) -> Vec<(usize, Option<Cut>)> {
    // Past this many layers the pairs, and past this many pieces their pairs,
    // cost more than a frame can spare (a millisecond at 256), so the run is
    // cut by depth throughout.
    const PAIRED: usize = 256;
    const PIECES: usize = 1024;
    // A sliver of a piece, in twice the layer's own area, is left out.
    const SLIVER: f32 = 1e-7;
    let eye = glam::Vec3::from(eye);
    let square: Vec<glam::Vec2> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .map(|(u, v)| glam::vec2(u, v))
        .to_vec();
    let layers: Vec<Shape> = worlds
        .iter()
        .enumerate()
        .map(|(i, m)| Shape::new(i, square.clone(), *m, view_proj))
        .collect();
    let n = layers.len();
    if n > PAIRED {
        return cut_by_depth(&layers.iter().map(|s| s.depth).collect::<Vec<_>>(), flat);
    }
    let flat = |i: usize| flat.get(i) == Some(&true);
    let by_depth = |i: usize, lift: u32| depth_cut(layers[i].depth, flat(i), lift);
    let planes: Vec<Option<(glam::Vec3, glam::Vec3)>> = layers
        .iter()
        .map(|s| {
            let normal = (s.pts[1] - s.pts[0]).cross(s.pts[3] - s.pts[0]);
            Some((normal.try_normalize()?, s.pts[0]))
        })
        .collect();
    let (order, flush, cross) = arrange(&layers, &planes, eye);
    if cross.is_empty() {
        return order.into_iter().map(|i| (i, None)).collect();
    }
    let mut lift = vec![0u32; n];
    for &(a, b) in &flush {
        lift[b] = lift[b].max(lift[a] + 1);
    }
    // Of a crossing pair, the one that crosses fewer is cut along the other's
    // plane, so a floor that many cards dip into stays whole and each card is
    // cut once. A flat smear can't be cut, so its partner always is, and two
    // smears that cross just go by the order.
    let mut degree = vec![0usize; n];
    for &(a, b) in &cross {
        degree[a] += 1;
        degree[b] += 1;
    }
    let mut cuts: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &cross {
        if !flat(a) && (flat(b) || degree[a] <= degree[b]) {
            cuts[a].push(b);
        }
        if !flat(b) && (flat(a) || degree[b] <= degree[a]) {
            cuts[b].push(a);
        }
    }
    // Layers that cross each other, directly or through others, are one
    // group, and a group with a layer cut too many times is cut by depth.
    let mut group: Vec<usize> = (0..n).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for &(a, b) in &cross {
        let (ra, rb) = (root(&mut group, a), root(&mut group, b));
        group[ra.max(rb)] = ra.min(rb);
    }
    let mut tangled = vec![false; n];
    for (i, by) in cuts.iter().enumerate() {
        if by.len() > CUT_PLANES {
            let r = root(&mut group, i);
            tangled[r] = true;
        }
    }
    let tangled: Vec<bool> = (0..n).map(|i| tangled[root(&mut group, i)]).collect();
    let mut pieces = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let (origin, u, v) = (
            layer.pts[0],
            layer.pts[1] - layer.pts[0],
            layer.pts[3] - layer.pts[0],
        );
        let mut polys = vec![square.clone()];
        for &by in cuts[i].iter().filter(|_| !tangled[i]) {
            let Some((normal, at)) = planes[by] else {
                continue;
            };
            // That plane, as a line across this layer's own 0..1 space.
            let line = glam::vec3(normal.dot(u), normal.dot(v), normal.dot(origin - at));
            polys = polys
                .into_iter()
                .flat_map(|p| [keep_side(&p, line), keep_side(&p, -line)])
                .filter(|p| p.len() >= 3 && area(p) > SLIVER)
                .collect();
        }
        pieces.extend(
            polys
                .into_iter()
                .map(|p| Shape::new(i, p, worlds[i], view_proj)),
        );
    }
    if pieces.len() > PIECES {
        return order
            .into_iter()
            .map(|i| (i, Some(by_depth(i, lift[i]))))
            .collect();
    }
    let (pieces_order, _, _) = arrange(&pieces, &planes, eye);
    let mut drawn = vec![false; n];
    let mut out = Vec::with_capacity(pieces.len());
    for k in pieces_order {
        let piece = &pieces[k];
        let i = piece.layer;
        if tangled[i] {
            // A group cut by depth draws together where its first layer
            // comes, in its own order.
            if !drawn[i] {
                let r = root(&mut group, i);
                for &j in &order {
                    if root(&mut group, j) == r {
                        drawn[j] = true;
                        out.push((j, Some(by_depth(j, lift[j]))));
                    }
                }
            }
            continue;
        }
        let cut = (!cuts[i].is_empty()).then(|| {
            let mut corners = [[0.0; 2]; PIECE_CORNERS];
            for (c, p) in corners.iter_mut().zip(&piece.uv) {
                *c = p.to_array();
            }
            Cut::Piece {
                corners,
                count: piece.uv.len().min(PIECE_CORNERS),
            }
        });
        out.push((i, cut));
    }
    out
}

/// A run cut by depth throughout, as a run holding a model is: farthest
/// first by `depths`, each draw's clip w at its middle, and lifted by stack
/// order, so layers on one plane keep it. `flat` as for [`depth_order`].
pub fn cut_by_depth(depths: &[f32], flat: &[bool]) -> Vec<(usize, Option<Cut>)> {
    let mut order: Vec<usize> = (0..depths.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(Far(depths[i], i)));
    order
        .into_iter()
        .map(|i| {
            let cut = depth_cut(depths[i], flat.get(i) == Some(&true), i as u32);
            (i, Some(cut))
        })
        .collect()
}

/// How one draw is cut by depth: a smear as a card at its middle's distance,
/// anything else a hair nearer per `lift`.
fn depth_cut(depth: f32, flat: bool, lift: u32) -> Cut {
    Cut::Depth(if flat {
        CUT_NEAR / depth.max(CUT_NEAR)
    } else {
        CUT_NEAR * (1.0 + lift as f32 * CUT_LIFT)
    })
}

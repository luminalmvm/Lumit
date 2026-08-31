//! Puppet: the mesh, the deformer and the warp (docs/impl/puppet.md §1–§3).
//!
//! # In plain terms
//!
//! You have a picture of a character — a cutout with an arm, say — and you want
//! to wave the arm without cutting the layer apart. Puppet does it in three
//! steps, and all three live in this file.
//!
//! First it lays a **mesh** of small triangles over the opaque part of the
//! picture, like chicken wire bent to the silhouette. Finding the silhouette is
//! [`build_mesh`]: grow the opaque region by a few pixels, walk its outline,
//! straighten the outline's staircase into a handful of straight runs, and fill
//! what is inside with triangles of roughly one size.
//!
//! Then you place **pins**: a pin says "this spot of the picture is under my
//! thumb". [`solve`] moves the mesh so the pinned spots follow your thumbs while
//! every triangle in between tries as hard as it can to keep its own shape — to
//! turn and slide rather than stretch. That "tries to keep its shape" is the
//! whole trick; it is what makes an arm *bend at the elbow* instead of smearing
//! like taffy. The maths is two rounds of ordinary simultaneous equations
//! (Igarashi, Moscovich & Hughes 2005): the first round lets each triangle turn
//! *and* resize, the second round takes the resizing back out, which is what
//! stops limbs inflating as they bend.
//!
//! Finally [`apply_puppet`] redraws the picture through the moved mesh: each
//! triangle carries its patch of pixels to wherever it ended up.
//!
//! Three other pin kinds season that basic move. A **starch** pin stiffens a
//! region, so a torso stays rigid while the limbs bend — it multiplies the
//! "keep your shape" term for the triangles it covers. An **overlap** pin says
//! which part draws in front when the picture folds over itself. A **bend** pin
//! rotates and scales a region about itself, so a hand can wave from the wrist
//! without the wrist travelling.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use spade::{
    handles::FixedVertexHandle, AngleLimit, ConstrainedDelaunayTriangulation, Point2,
    RefinementParameters, Triangulation,
};
use uuid::Uuid;

use crate::anim::{Animation, Property};
use crate::linalg::{cholesky, cholesky_solve, Dense};
use crate::mask::signed_distance;
use crate::time::Rational;

/// Alpha at or above which a pixel is content (docs/impl/puppet.md §1.1). Ten
/// per cent, not fifty: faint content — smoke, glow, a soft antialiased edge —
/// is still content, and a mesh cut at half alpha drops the fringe of
/// everything.
pub const COVERAGE_ALPHA: u8 = 25;

/// The iso-value marching squares walks, on the 0..255 expanded coverage.
const ISO: f64 = 127.5;

/// Douglas–Peucker tolerance, layer px (docs/impl/puppet.md §1.2).
const SIMPLIFY_TOLERANCE: f64 = 1.25;

/// Most vertices a mesh may hold before it is coarsened, then refused
/// (docs/impl/puppet.md §1.3). The deformer's factorisation is dense, and this
/// cap is its budget.
pub const VERTEX_CAP: usize = 1500;

/// How many times the area bound may double before a build is refused.
const MAX_COARSEN_STEPS: u32 = 5;

/// Soft weight of a pin's own position constraint (docs/impl/puppet.md §2.2).
const PIN_WEIGHT: f64 = 1000.0;

/// Soft weight of a bend pin's rotation targets, before falloff. Below
/// [`PIN_WEIGHT`] on purpose: a position pin inside a bend region must win the
/// argument.
const BEND_WEIGHT: f64 = 100.0;

// --- The mesh ---------------------------------------------------------------

/// A puppet mesh: vertices in layer pixels at natural size, and the triangles
/// over them. Never stored in a project file — it is rebuilt from the layer's
/// alpha and cached by [`PuppetCache`] (docs/impl/puppet.md §1.4).
#[derive(Debug, Clone, PartialEq)]
pub struct PuppetMesh {
    pub vertices: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
    /// `blake3(alpha ‖ density bits ‖ expansion bits)` — the cache key, and the
    /// forward-compatibility pin: a future triangulator changes nothing in any
    /// saved project, because no project ever contained a triangle.
    pub hash: [u8; 32],
}

/// Why a mesh could not be built. Every one of these is a value, not a panic
/// (docs/14 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshError {
    /// Nothing at or above [`COVERAGE_ALPHA`] anywhere in the layer.
    Empty,
    /// Even at the coarsest auto-coarsened density the mesh is over
    /// [`VERTEX_CAP`]. Carries the vertex count that lost, so the refusal can
    /// name a density.
    TooDense { vertices: usize },
    /// The simplified contours still intersect at Douglas–Peucker tolerance 0,
    /// which should be impossible for level-set contours. Kept as a value so a
    /// surprise is a refusal rather than a crash.
    Intersecting,
    /// A degenerate request: zero-sized raster, or a density that is not a
    /// positive finite number.
    Degenerate,
}

/// The alpha channel of a premultiplied RGBA buffer, at the layer's **natural**
/// size — the space the mesh and the pins live in.
///
/// A frame drawn at a reduced preview resolution hands over a smaller buffer
/// than the layer's own size, so its alpha is sampled up to natural before the
/// mesh is walked; at full resolution — and for every kind that rasterises
/// itself at natural size — this is a plain copy of every fourth byte.
///
/// Nearest, not bilinear: the coverage is about to be thresholded at 10 % and
/// grown by three pixels, and a smoother resample would not move that boundary
/// enough to be worth a multiply per pixel.
#[must_use]
pub fn alpha_at_natural(rgba: &[u8], w: u32, h: u32, natural_w: u32, natural_h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let (nw, nh) = (natural_w as usize, natural_h as usize);
    if w == 0 || h == 0 || nw == 0 || nh == 0 || rgba.len() < w * h * 4 {
        return vec![0; nw * nh];
    }
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        let sy = (y * h / nh).min(h - 1);
        for x in 0..nw {
            let sx = (x * w / nw).min(w - 1);
            out[y * nw + x] = rgba[(sy * w + sx) * 4 + 3];
        }
    }
    out
}

/// Build the mesh over an alpha bitmap.
///
/// `alpha` is one byte per pixel, `w`×`h`, at the layer's natural size;
/// `density` is the target triangle edge in layer px and `expansion` how far
/// the coverage is grown before the outline is walked.
pub fn build_mesh(
    alpha: &[u8],
    w: u32,
    h: u32,
    density: f64,
    expansion: f64,
) -> Result<PuppetMesh, MeshError> {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || alpha.len() != w * h || !density.is_finite() || density <= 0.0 {
        return Err(MeshError::Degenerate);
    }
    let expansion = if expansion.is_finite() {
        expansion.max(0.0)
    } else {
        0.0
    };
    let hash = mesh_hash(alpha, density, expansion);

    let Some(cov) = expanded_coverage(alpha, w, h, expansion) else {
        return Err(MeshError::Empty);
    };
    let raw = contours(&cov);

    // The trap from §1.2: raw level-set contours never cross, but two
    // *simplified* ones (or one and itself, around a narrow neck) can. Retry
    // with the tolerance halved down to the raw contours, which cannot.
    let mut tolerance = SIMPLIFY_TOLERANCE;
    loop {
        let simplified = simplify_all(&raw, tolerance, density);
        match triangulate(&simplified, &cov, density) {
            Ok((vertices, triangles)) => {
                return Ok(PuppetMesh {
                    vertices: vertices
                        .into_iter()
                        .map(|p| [p[0] - cov.pad as f64, p[1] - cov.pad as f64])
                        .collect(),
                    triangles,
                    hash,
                })
            }
            Err(MeshError::Intersecting) if tolerance > 0.0 => {
                tolerance = if tolerance <= 0.05 {
                    0.0
                } else {
                    tolerance * 0.5
                };
            }
            Err(e) => return Err(e),
        }
    }
}

fn mesh_hash(alpha: &[u8], density: f64, expansion: f64) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(alpha);
    hasher.update(&density.to_bits().to_le_bytes());
    hasher.update(&expansion.to_bits().to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// The expanded coverage, on a grid padded with transparent margin so that
/// every contour closes even when the content runs to the edge of the frame.
struct Coverage {
    v: Vec<u8>,
    w: usize,
    h: usize,
    pad: usize,
}

impl Coverage {
    /// Bilinear sample in padded grid coordinates (a whole coordinate is a
    /// pixel centre). Outside the grid clamps, which is safe because the pad is
    /// transparent.
    fn at(&self, x: f64, y: f64) -> f64 {
        if self.w == 0 || self.h == 0 {
            return 0.0;
        }
        let fx = x.clamp(0.0, (self.w - 1) as f64);
        let fy = y.clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let g =
            |xx: usize, yy: usize| f64::from(self.v.get(yy * self.w + xx).copied().unwrap_or(0));
        let top = g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx;
        let bot = g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx;
        top * (1.0 - ty) + bot * ty
    }

    fn corner(&self, x: usize, y: usize) -> f64 {
        f64::from(self.v.get(y * self.w + x).copied().unwrap_or(0))
    }
}

/// Threshold at [`COVERAGE_ALPHA`], grow by `expansion`, and read the result
/// back as a soft 0..255 field — the same signed-distance machinery `mask.rs`
/// uses for mask expansion, read the same way (a one-pixel ramp across the
/// edge, so the crossing marching squares finds sits at the pixel boundary
/// rather than on a knife edge).
fn expanded_coverage(alpha: &[u8], w: usize, h: usize, expansion: f64) -> Option<Coverage> {
    let pad = (expansion.ceil() as usize).saturating_add(2);
    let (pw, ph) = (w + 2 * pad, h + 2 * pad);
    let mut hard = vec![0u8; pw * ph];
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if alpha.get(y * w + x).copied().unwrap_or(0) >= COVERAGE_ALPHA {
                any = true;
                if let Some(c) = hard.get_mut((y + pad) * pw + (x + pad)) {
                    *c = 255;
                }
            }
        }
    }
    if !any {
        return None;
    }
    let dist = signed_distance(&hard, pw, ph);
    let mut v = hard;
    for (o, d) in v.iter_mut().zip(dist) {
        *o = (((0.5 + f64::from(d) + expansion).clamp(0.0, 1.0)) * 255.0).round() as u8;
    }
    Some(Coverage {
        v,
        w: pw,
        h: ph,
        pad,
    })
}

// --- Marching squares -------------------------------------------------------

/// Which of a cell's four edges a segment end sits on.
#[derive(Clone, Copy)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// Closed contours of the coverage at [`ISO`], in scan-discovery order
/// (top-to-bottom, left-to-right) — the determinism anchor for everything
/// downstream (docs/impl/puppet.md §1.2).
fn contours(cov: &Coverage) -> Vec<Vec<[f64; 2]>> {
    let (w, h) = (cov.w, cov.h);
    if w < 2 || h < 2 {
        return Vec::new();
    }
    // Edge ids: the horizontal grid edges first, then the vertical ones.
    let hn = (w - 1) * h;
    let total = hn + w * (h - 1);
    let hid = |x: usize, y: usize| y * (w - 1) + x;
    let vid = |x: usize, y: usize| hn + y * w + x;

    let mut segs: Vec<[usize; 2]> = Vec::new();
    let mut incident: Vec<[u32; 2]> = vec![[u32::MAX; 2]; total];
    let push = |segs: &mut Vec<[usize; 2]>, incident: &mut Vec<[u32; 2]>, a: usize, b: usize| {
        let i = segs.len() as u32;
        segs.push([a, b]);
        for e in [a, b] {
            if let Some(slot) = incident.get_mut(e) {
                if slot[0] == u32::MAX {
                    slot[0] = i;
                } else if slot[1] == u32::MAX {
                    slot[1] = i;
                }
            }
        }
    };

    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let (tl, tr) = (cov.corner(x, y), cov.corner(x + 1, y));
            let (br, bl) = (cov.corner(x + 1, y + 1), cov.corner(x, y + 1));
            let case = usize::from(tl >= ISO)
                | usize::from(tr >= ISO) << 1
                | usize::from(br >= ISO) << 2
                | usize::from(bl >= ISO) << 3;
            // The saddles (5 and 10) are resolved by the average of the four
            // corners: an average above the iso joins the higher-valued pair,
            // so the contour isolates the *other* pair's two corners.
            let joined = (tl + tr + br + bl) * 0.25 >= ISO;
            let pairs: &[[Side; 2]] = match case {
                1 => &[[Side::Left, Side::Top]],
                2 => &[[Side::Top, Side::Right]],
                3 => &[[Side::Left, Side::Right]],
                4 => &[[Side::Right, Side::Bottom]],
                5 if joined => &[[Side::Top, Side::Right], [Side::Left, Side::Bottom]],
                5 => &[[Side::Left, Side::Top], [Side::Right, Side::Bottom]],
                6 => &[[Side::Top, Side::Bottom]],
                7 => &[[Side::Left, Side::Bottom]],
                8 => &[[Side::Left, Side::Bottom]],
                9 => &[[Side::Top, Side::Bottom]],
                10 if joined => &[[Side::Left, Side::Top], [Side::Right, Side::Bottom]],
                10 => &[[Side::Top, Side::Right], [Side::Left, Side::Bottom]],
                11 => &[[Side::Right, Side::Bottom]],
                12 => &[[Side::Left, Side::Right]],
                13 => &[[Side::Top, Side::Right]],
                14 => &[[Side::Left, Side::Top]],
                _ => &[],
            };
            let edge = |s: Side| match s {
                Side::Top => hid(x, y),
                Side::Bottom => hid(x, y + 1),
                Side::Left => vid(x, y),
                Side::Right => vid(x + 1, y),
            };
            for pair in pairs {
                push(&mut segs, &mut incident, edge(pair[0]), edge(pair[1]));
            }
        }
    }

    // Where each edge's crossing sits, interpolated along the edge.
    let point = |e: usize| -> [f64; 2] {
        let cross = |a: f64, b: f64| {
            let d = b - a;
            if d.abs() < 1e-12 {
                0.5
            } else {
                ((ISO - a) / d).clamp(0.0, 1.0)
            }
        };
        if e < hn {
            let (x, y) = (e % (w - 1), e / (w - 1));
            [
                x as f64 + cross(cov.corner(x, y), cov.corner(x + 1, y)),
                y as f64,
            ]
        } else {
            let (x, y) = ((e - hn) % w, (e - hn) / w);
            [
                x as f64,
                y as f64 + cross(cov.corner(x, y), cov.corner(x, y + 1)),
            ]
        }
    };

    let mut seen = vec![false; segs.len()];
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    for start in 0..segs.len() {
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let Some(&[first, mut edge]) = segs.get(start) else {
            continue;
        };
        let mut current = start;
        if let Some(s) = seen.get_mut(current) {
            *s = true;
        }
        let mut path = vec![point(first), point(edge)];
        // Each interior grid edge with a crossing is shared by two cells, so it
        // carries exactly two segment ends: walking from one to the other
        // always closes.
        while edge != first {
            let inc = incident.get(edge).copied().unwrap_or([u32::MAX; 2]);
            let next = if inc[0] as usize == current {
                inc[1]
            } else {
                inc[0]
            };
            if next == u32::MAX {
                break;
            }
            let next = next as usize;
            if seen.get(next).copied().unwrap_or(true) {
                break;
            }
            if let Some(s) = seen.get_mut(next) {
                *s = true;
            }
            let ends = segs.get(next).copied().unwrap_or([edge, edge]);
            edge = if ends[0] == edge { ends[1] } else { ends[0] };
            current = next;
            if edge == first {
                break;
            }
            path.push(point(edge));
        }
        if path.len() >= 3 {
            out.push(path);
        }
    }
    out
}

// --- Simplification ---------------------------------------------------------

fn simplify_all(raw: &[Vec<[f64; 2]>], tolerance: f64, density: f64) -> Vec<Vec<[f64; 2]>> {
    raw.iter()
        .filter_map(|c| {
            let s = simplify_closed(c, tolerance);
            let s = collapse(&s, density * 0.25);
            (s.len() >= 3).then_some(s)
        })
        .collect()
}

/// Douglas–Peucker over a *closed* polyline: split it at the vertex furthest
/// from vertex 0 and simplify the two chains, so the result does not depend on
/// where the walk happened to start beyond that one fixed choice.
fn simplify_closed(pts: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    if pts.len() < 4 || tolerance <= 0.0 {
        return pts.to_vec();
    }
    let Some(&anchor) = pts.first() else {
        return Vec::new();
    };
    let mut split = 0usize;
    let mut best = -1.0f64;
    for (i, p) in pts.iter().enumerate() {
        let d = (p[0] - anchor[0]).powi(2) + (p[1] - anchor[1]).powi(2);
        if d > best {
            best = d;
            split = i;
        }
    }
    let mut keep = vec![false; pts.len()];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.get_mut(split) {
        *k = true;
    }
    // Two chains: 0..=split, and split..=len (wrapping back to 0).
    dp(pts, 0, split, tolerance, &mut keep);
    dp(pts, split, pts.len(), tolerance, &mut keep);
    pts.iter()
        .zip(&keep)
        .filter_map(|(p, &k)| k.then_some(*p))
        .collect()
}

/// Mark the vertices of `pts[lo..=hi]` (index `hi` taken modulo the length, so
/// the last chain closes onto vertex 0) that survive at `tolerance`.
fn dp(pts: &[[f64; 2]], lo: usize, hi: usize, tolerance: f64, keep: &mut [bool]) {
    let mut stack = vec![(lo, hi)];
    let tol2 = tolerance * tolerance;
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let n = pts.len();
        let (pa, pb) = (
            pts.get(a % n).copied().unwrap_or([0.0; 2]),
            pts.get(b % n).copied().unwrap_or([0.0; 2]),
        );
        let (ex, ey) = (pb[0] - pa[0], pb[1] - pa[1]);
        let len2 = ex * ex + ey * ey;
        let mut worst = 0.0f64;
        let mut at = a;
        for i in a + 1..b {
            let p = pts.get(i % n).copied().unwrap_or([0.0; 2]);
            let (dx, dy) = (p[0] - pa[0], p[1] - pa[1]);
            let d2 = if len2 < 1e-18 {
                dx * dx + dy * dy
            } else {
                let t = ((dx * ex + dy * ey) / len2).clamp(0.0, 1.0);
                (dx - t * ex).powi(2) + (dy - t * ey).powi(2)
            };
            if d2 > worst {
                worst = d2;
                at = i;
            }
        }
        if worst > tol2 && at > a {
            if let Some(k) = keep.get_mut(at % n) {
                *k = true;
            }
            stack.push((a, at));
            stack.push((at, b));
        }
    }
}

/// Drop consecutive vertices closer than `min` along the contour, so constraint
/// edges do not force slivers into the triangulation.
fn collapse(pts: &[[f64; 2]], min: f64) -> Vec<[f64; 2]> {
    if pts.len() < 4 || !min.is_finite() || min <= 0.0 {
        return pts.to_vec();
    }
    let min2 = min * min;
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
    for p in pts {
        let far = match out.last() {
            Some(q) => (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) >= min2,
            None => true,
        };
        if far {
            out.push(*p);
        }
    }
    while out.len() > 3 {
        let (Some(first), Some(last)) = (out.first().copied(), out.last().copied()) else {
            break;
        };
        if (first[0] - last[0]).powi(2) + (first[1] - last[1]).powi(2) >= min2 {
            break;
        }
        out.pop();
    }
    out
}

// --- Triangulation ----------------------------------------------------------

type Cdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

/// What a triangulation pass hands back: the kept vertices and the triangles
/// over them, already densely re-indexed.
type Meshed = (Vec<[f64; 2]>, Vec<[u32; 3]>);

/// Constrained Delaunay plus quality refinement, then the outside thrown away
/// by sampling the coverage under each triangle's centroid.
fn triangulate(
    contours: &[Vec<[f64; 2]>],
    cov: &Coverage,
    density: f64,
) -> Result<Meshed, MeshError> {
    if contours.is_empty() {
        return Err(MeshError::Empty);
    }
    let base_area = density * density * 3f64.sqrt() * 0.25;
    let mut last_count = 0usize;
    for step in 0..=MAX_COARSEN_STEPS {
        let mut cdt = Cdt::new();
        for contour in contours {
            let mut handles: Vec<FixedVertexHandle> = Vec::with_capacity(contour.len());
            for p in contour {
                match cdt.insert(Point2::new(p[0], p[1])) {
                    Ok(v) => handles.push(v),
                    Err(_) => return Err(MeshError::Degenerate),
                }
            }
            for i in 0..handles.len() {
                let (a, b) = (
                    handles.get(i).copied(),
                    handles.get((i + 1) % handles.len()).copied(),
                );
                let (Some(a), Some(b)) = (a, b) else { continue };
                if a == b {
                    continue;
                }
                // `add_constraint` *panics* on an intersection, so the check is
                // not optional: this is where §1.2's retry is triggered.
                if !cdt.can_add_constraint(a, b) {
                    return Err(MeshError::Intersecting);
                }
                cdt.add_constraint(a, b);
            }
        }
        let params = RefinementParameters::<f64>::new()
            .with_angle_limit(AngleLimit::from_deg(30.0))
            .with_max_allowed_area(base_area * f64::from(1u32 << step))
            // Refining the faces outside the silhouette would spend the whole
            // vertex budget on triangles step 3 immediately discards. Which
            // triangles are *kept* is still the centroid sample below, never
            // this flag.
            .exclude_outer_faces(true)
            .with_max_additional_vertices(VERTEX_CAP * 2);
        let excluded = cdt.refine(params);
        let mut outer = vec![false; cdt.num_all_faces()];
        for f in &excluded.excluded_faces {
            if let Some(slot) = outer.get_mut(f.index()) {
                *slot = true;
            }
        }

        // Keep a triangle iff the expanded coverage under its centroid is
        // inside. Holes and the outside fail the sample and fall away; disjoint
        // blobs come out disjoint, which is why this is a conforming mesh and
        // not a clipped grid.
        //
        // The centroid sample alone is not quite enough, and the weld test
        // (§7.3) is where it shows: two blobs whose silhouettes share a
        // straight edge — two legs standing on the same ground line — leave the
        // hull triangulator a long, thin sliver bridging the gap, and that
        // sliver's centroid sits a fraction of a pixel *inside* the shared
        // edge. So the faces spade's own flood fill already called outer are
        // dropped first. Cheaper than sampling a triangle at several points,
        // and it is the same closed contours doing the deciding.
        let mut remap = vec![u32::MAX; cdt.num_vertices()];
        let mut vertices: Vec<[f64; 2]> = Vec::new();
        let mut triangles: Vec<[u32; 3]> = Vec::new();
        for face in cdt.inner_faces() {
            if outer.get(face.fix().index()).copied().unwrap_or(false) {
                continue;
            }
            let vs = face.vertices();
            let ps = vs.map(|v| v.position());
            let cx = (ps[0].x + ps[1].x + ps[2].x) / 3.0;
            let cy = (ps[0].y + ps[1].y + ps[2].y) / 3.0;
            if cov.at(cx, cy) < ISO {
                continue;
            }
            let mut tri = [0u32; 3];
            for (slot, v) in tri.iter_mut().zip(vs) {
                let fixed = v.fix().index();
                let existing = remap.get(fixed).copied().unwrap_or(u32::MAX);
                *slot = if existing == u32::MAX {
                    let id = vertices.len() as u32;
                    vertices.push([v.position().x, v.position().y]);
                    if let Some(r) = remap.get_mut(fixed) {
                        *r = id;
                    }
                    id
                } else {
                    existing
                };
            }
            triangles.push(tri);
        }
        if triangles.is_empty() {
            return Err(MeshError::Empty);
        }
        last_count = vertices.len();
        if last_count <= VERTEX_CAP {
            return Ok((vertices, triangles));
        }
    }
    Err(MeshError::TooDense {
        vertices: last_count,
    })
}

/// The triangle containing `p` and its barycentric coordinates there, or `None`
/// when the point falls outside the mesh — which is how a pin goes inert.
pub fn locate(mesh: &PuppetMesh, p: [f64; 2]) -> Option<(usize, [f64; 3])> {
    locate_in(&mesh.vertices, &mesh.triangles, p)
}

/// [`locate`] over loose vertices, so the same walk can be run over the
/// **deformed** positions rather than the rest ones.
///
/// That is what a click on the picture needs (docs/impl/puppet.md §5): the user
/// aims at the puppet where it is now, and the pin has to be stored where that
/// spot sits in the rest mesh — the same barycentric coordinates, read off the
/// rest triangle. `triangles` indexes `vertices`; an index past the end is
/// skipped rather than a panic (docs/14 §4).
pub fn locate_in(
    vertices: &[[f64; 2]],
    triangles: &[[u32; 3]],
    p: [f64; 2],
) -> Option<(usize, [f64; 3])> {
    for (i, tri) in triangles.iter().enumerate() {
        let Some(a) = vertices.get(tri[0] as usize).copied() else {
            continue;
        };
        let Some(b) = vertices.get(tri[1] as usize).copied() else {
            continue;
        };
        let Some(c) = vertices.get(tri[2] as usize).copied() else {
            continue;
        };
        if let Some(bary) = barycentric(a, b, c, p) {
            if bary.iter().all(|&v| v >= -1e-9) {
                return Some((i, bary));
            }
        }
    }
    None
}

fn tri_positions(mesh: &PuppetMesh, tri: &[u32; 3]) -> Option<[[f64; 2]; 3]> {
    Some([
        mesh.vertices.get(tri[0] as usize).copied()?,
        mesh.vertices.get(tri[1] as usize).copied()?,
        mesh.vertices.get(tri[2] as usize).copied()?,
    ])
}

fn barycentric(a: [f64; 2], b: [f64; 2], c: [f64; 2], p: [f64; 2]) -> Option<[f64; 3]> {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if !d.is_finite() || d.abs() < 1e-12 {
        return None;
    }
    let l0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let l1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    Some([l0, l1, 1.0 - l0 - l1])
}

// ponytail: dense Cholesky, O((2n)³) at (re)factorisation — a sparse
// factorisation (the matrices are mesh-Laplacian sparse) is the upgrade when
// capped-out meshes make pin placement feel sticky. Observable trigger:
// factorisation time > 250 ms in the PU1 bench at default density, or users
// hitting the 1500-vertex cap in anger.

// --- The deformer -----------------------------------------------------------

/// The four pin kinds (docs/07-UI-SPEC.md §1.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PuppetPinKind {
    Position,
    Starch,
    Overlap,
    Bend,
}

/// One pin, already evaluated at the frame. The document's `PuppetPin` (PU2)
/// maps onto this; the solver never reads Properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolvePin {
    pub id: Uuid,
    pub kind: PuppetPinKind,
    /// Where the pin sits in the rest mesh, layer px. Its binding and every
    /// falloff distance are read here, so they are rest-pose facts and the
    /// matrices factor once.
    pub rest: [f64; 2],
    /// Where the pin is at this frame, layer px.
    pub now: [f64; 2],
    /// Bend only: degrees.
    pub rotation: f64,
    /// Bend only: percent, 100 = natural.
    pub scale: f64,
    /// Starch 0..100, overlap −100..100.
    pub amount: f64,
    /// Starch / overlap / bend falloff radius, rest px.
    pub extent: f64,
}

impl SolvePin {
    /// A plain position pin at `p`, at rest.
    pub fn position(id: Uuid, p: [f64; 2]) -> SolvePin {
        SolvePin {
            id,
            kind: PuppetPinKind::Position,
            rest: p,
            now: p,
            rotation: 0.0,
            scale: 100.0,
            amount: 0.0,
            extent: DEFAULT_EXTENT,
        }
    }
}

// --- What the document stores (docs/impl/puppet.md §4) ----------------------

/// Default target triangle edge, px at natural size.
pub const DEFAULT_DENSITY: f64 = 24.0;
/// Default growth of the coverage before the outline is walked, px.
pub const DEFAULT_EXPANSION: f64 = 3.0;
/// Default falloff radius for a starch, overlap or bend pin, rest px.
pub const DEFAULT_EXTENT: f64 = 50.0;

fn default_density() -> f64 {
    DEFAULT_DENSITY
}
fn default_expansion() -> f64 {
    DEFAULT_EXPANSION
}
fn default_extent() -> f64 {
    DEFAULT_EXTENT
}
fn natural_scale() -> Property {
    Property::fixed(100.0)
}
fn is_natural_scale(p: &Property) -> bool {
    matches!(p.animation, Animation::Static(v) if v == 100.0) && p.extra.is_empty()
}

/// A layer's puppet: the pins the author placed, and the three numbers the mesh
/// is built from. One block per layer (docs/impl/puppet.md §4).
///
/// **No triangle is ever stored.** The mesh is rebuilt from the layer's own
/// alpha and cached by [`PuppetCache`], which is what lets a future, better
/// triangulator change nothing in any saved project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PuppetBlock {
    /// The layer time the mesh's alpha is taken at: when the first pin was
    /// placed. Rational, like a mask's keys, so a rate change cannot drift it.
    pub reference_time: Rational,
    /// Target triangle edge, px at natural size.
    #[serde(default = "default_density")]
    pub density: f64,
    /// Coverage growth before meshing, px.
    #[serde(default = "default_expansion")]
    pub expansion: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<PuppetPin>,
    /// Unknown fields from newer Lumit versions (docs/10 §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl PuppetBlock {
    /// An empty block whose mesh will be taken at `reference_time`.
    #[must_use]
    pub fn new(reference_time: Rational) -> PuppetBlock {
        PuppetBlock {
            reference_time,
            density: DEFAULT_DENSITY,
            expansion: DEFAULT_EXPANSION,
            pins: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    /// The pins as the solver wants them at layer time `lt`: **rest** read at
    /// the reference time, where the pin sits in the mesh, and **now** read at
    /// this frame. Everything else is the pin's own animated value.
    #[must_use]
    pub fn pins_at(&self, lt: f64) -> Vec<SolvePin> {
        let rest_t = self.reference_time.to_f64();
        self.pins
            .iter()
            .map(|p| SolvePin {
                id: p.id,
                kind: p.kind,
                rest: [p.x.value_at(rest_t), p.y.value_at(rest_t)],
                now: [p.x.value_at(lt), p.y.value_at(lt)],
                rotation: p.rotation.value_at(lt),
                scale: p.scale.value_at(lt),
                amount: p.amount.value_at(lt),
                extent: p.extent,
            })
            .collect()
    }
}

/// One pin. Every animatable field is an ordinary [`Property`] — the same
/// stopwatch, lanes and diamonds as everything else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PuppetPin {
    pub id: Uuid,
    pub name: String,
    pub kind: PuppetPinKind,
    /// Layer pixels, never per cent: a point parameter is pixels everywhere in
    /// Lumit, and the mesh lives in layer px at natural size.
    #[serde(with = "crate::mask::still_or_keyed")]
    pub x: Property,
    #[serde(with = "crate::mask::still_or_keyed")]
    pub y: Property,
    /// Bend only: degrees.
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub rotation: Property,
    /// Bend only: per cent, 100 = natural.
    #[serde(
        default = "natural_scale",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "is_natural_scale"
    )]
    pub scale: Property,
    /// Starch 0..100, overlap −100..100 (in front / behind).
    #[serde(
        default = "Property::zero",
        with = "crate::mask::still_or_keyed",
        skip_serializing_if = "crate::paint::is_static_zero"
    )]
    pub amount: Property,
    /// Starch / overlap / bend falloff radius, rest px. Not animatable: which
    /// vertices a pin reaches is a rest-pose fact, and that is what lets the
    /// systems factor once (docs/impl/puppet.md §2.2).
    #[serde(default = "default_extent")]
    pub extent: f64,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl PuppetPin {
    /// A pin of `kind` at `(x, y)` layer px, with every other value at its
    /// default.
    #[must_use]
    pub fn new(kind: PuppetPinKind, name: impl Into<String>, x: f64, y: f64) -> PuppetPin {
        PuppetPin {
            id: Uuid::now_v7(),
            name: name.into(),
            kind,
            x: Property::fixed(x),
            y: Property::fixed(y),
            rotation: Property::zero(),
            scale: natural_scale(),
            amount: Property::zero(),
            extent: DEFAULT_EXTENT,
            extra: serde_json::Map::new(),
        }
    }
}

/// What the solve produced.
#[derive(Debug, Clone, PartialEq)]
pub struct PuppetSolution {
    /// Deformed vertex positions, layer px, one per mesh vertex.
    pub vertices: Vec<[f64; 2]>,
    /// Overlap depth per vertex: more "in front" draws later.
    pub depth: Vec<f64>,
    /// Pins whose rest position fell outside the mesh. Kept in the document,
    /// drawn hollow, contributing nothing.
    pub inert: Vec<Uuid>,
    /// The mesh is unmoved, so the warp can be skipped entirely.
    pub identity: bool,
}

/// One soft constraint row: a barycentric point pulled towards a target.
#[derive(Clone)]
struct Row {
    verts: [u32; 3],
    bary: [f64; 3],
    weight: f64,
    target: Target,
}

#[derive(Clone, Copy)]
enum Target {
    /// The pin's own animated position.
    Pin(usize),
    /// A bend pin's rotation target: `now + s·R(θ)·offset`.
    Bend { pin: usize, offset: [f64; 2] },
}

/// The factored systems for one (mesh, pin structure) pair. Everything here
/// depends on the *rest* mesh and on which points are pinned — never on where
/// the pins are this frame — which is the whole reason the 2005 formulation was
/// chosen (docs/impl/puppet.md §2.1).
pub struct Factorisation {
    /// Mesh vertex → index into the solved systems, or `usize::MAX` for a
    /// vertex whose component has too few constraints to solve.
    slot: Vec<usize>,
    /// System index → mesh vertex.
    back: Vec<usize>,
    /// Mesh vertex → the component's single pin delta source, when that
    /// component carries exactly one constrained point.
    lone: Vec<Option<usize>>,
    l1: Option<Dense>,
    l2: Option<Dense>,
    rows: Vec<Row>,
    weights: Vec<f64>,
    inert: Vec<Uuid>,
    key: [u8; 32],
}

/// Connected components of the mesh, by union-find over triangle edges.
fn components(n: usize, triangles: &[[u32; 3]]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        loop {
            let p = parent.get(i).copied().unwrap_or(i);
            if p == i {
                return i;
            }
            let g = parent.get(p).copied().unwrap_or(p);
            if let Some(s) = parent.get_mut(i) {
                *s = g;
            }
            i = g;
        }
    }
    for tri in triangles {
        for k in 0..3 {
            let (a, b) = (tri[k] as usize, tri[(k + 1) % 3] as usize);
            if a >= n || b >= n {
                continue;
            }
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                if let Some(s) = parent.get_mut(ra) {
                    *s = rb;
                }
            }
        }
    }
    (0..n).map(|i| find(&mut parent, i)).collect()
}

/// Per-triangle stiffness from the starch pins: `1 + 9·influence(centroid)`,
/// influences combining by max rather than sum — three overlapping pins should
/// not stack to absurd stiffness.
fn starch_weights(mesh: &PuppetMesh, pins: &[SolvePin]) -> Vec<f64> {
    mesh.triangles
        .iter()
        .map(|tri| {
            let Some([a, b, c]) = tri_positions(mesh, tri) else {
                return 1.0;
            };
            let p = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
            let mut influence = 0.0f64;
            for pin in pins {
                if pin.kind != PuppetPinKind::Starch {
                    continue;
                }
                influence = influence.max(falloff(pin, p) * (pin.amount / 100.0).clamp(0.0, 1.0));
            }
            1.0 + 9.0 * influence
        })
        .collect()
}

/// Linear falloff from a pin's rest position across its extent: 1 at the pin, 0
/// at the rim and beyond.
fn falloff(pin: &SolvePin, p: [f64; 2]) -> f64 {
    if !pin.extent.is_finite() || pin.extent <= 0.0 {
        return 0.0;
    }
    let d = ((p[0] - pin.rest[0]).powi(2) + (p[1] - pin.rest[1]).powi(2)).sqrt();
    (1.0 - d / pin.extent).clamp(0.0, 1.0)
}

/// The key the factorisation cache is held under: the mesh, and everything
/// about the pins that changes the *matrices* — their identities, kinds, rest
/// positions, extents and starch amounts. Pin positions at the frame are
/// deliberately absent: they only move the right-hand side.
fn factor_key(mesh: &PuppetMesh, pins: &[SolvePin]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&mesh.hash);
    for pin in pins {
        hasher.update(pin.id.as_bytes());
        hasher.update(&[pin.kind as u8]);
        for v in [pin.rest[0], pin.rest[1], pin.extent] {
            hasher.update(&v.to_bits().to_le_bytes());
        }
        if pin.kind == PuppetPinKind::Starch {
            hasher.update(&pin.amount.to_bits().to_le_bytes());
        }
    }
    *hasher.finalize().as_bytes()
}

/// Assemble and factor the two systems for this mesh and pin structure.
pub fn factorise(mesh: &PuppetMesh, pins: &[SolvePin]) -> Factorisation {
    let n = mesh.vertices.len();
    let key = factor_key(mesh, pins);
    let weights = starch_weights(mesh, pins);

    // Bind every constraining pin, and collect its rows.
    let mut rows: Vec<Row> = Vec::new();
    let mut inert: Vec<Uuid> = Vec::new();
    for (i, pin) in pins.iter().enumerate() {
        if !matches!(pin.kind, PuppetPinKind::Position | PuppetPinKind::Bend) {
            continue;
        }
        let Some((tri, bary)) = locate(mesh, pin.rest) else {
            inert.push(pin.id);
            continue;
        };
        let verts = mesh.triangles.get(tri).copied().unwrap_or([0; 3]);
        rows.push(Row {
            verts,
            bary,
            weight: PIN_WEIGHT,
            target: Target::Pin(i),
        });
        if pin.kind == PuppetPinKind::Bend {
            for (v, p) in mesh.vertices.iter().enumerate() {
                let f = falloff(pin, *p);
                if f <= 0.0 {
                    continue;
                }
                rows.push(Row {
                    verts: [v as u32, v as u32, v as u32],
                    bary: [1.0, 0.0, 0.0],
                    weight: BEND_WEIGHT * f,
                    target: Target::Bend {
                        pin: i,
                        offset: [p[0] - pin.rest[0], p[1] - pin.rest[1]],
                    },
                });
            }
        }
    }

    // How many constrained points each component carries decides whether it is
    // solved, translated, or left at rest. A component with one constrained
    // point has a free rotation and scale, so solving it is not merely
    // underdetermined — it is not what the user meant (§2.3).
    let comp = components(n, &mesh.triangles);
    let mut count = vec![0usize; n];
    let mut lone_row = vec![usize::MAX; n];
    for (ri, row) in rows.iter().enumerate() {
        let root = comp.get(row.verts[0] as usize).copied().unwrap_or(0);
        if let Some(c) = count.get_mut(root) {
            *c += 1;
        }
        if let Some(s) = lone_row.get_mut(root) {
            if *s == usize::MAX {
                *s = ri;
            }
        }
    }

    let mut slot = vec![usize::MAX; n];
    let mut back: Vec<usize> = Vec::new();
    let mut lone: Vec<Option<usize>> = vec![None; n];
    for (v, lone) in lone.iter_mut().enumerate() {
        let root = comp.get(v).copied().unwrap_or(v);
        match count.get(root).copied().unwrap_or(0) {
            0 => {}
            1 => *lone = lone_row.get(root).copied().filter(|&r| r != usize::MAX),
            _ => {
                if let Some(s) = slot.get_mut(v) {
                    *s = back.len();
                }
                back.push(v);
            }
        }
    }

    let m = back.len();
    if m == 0 {
        return Factorisation {
            slot,
            back,
            lone,
            l1: None,
            l2: None,
            rows,
            weights,
            inert,
            key,
        };
    }

    // Step 1 — the similarity system, over 2m coordinates.
    let mut g = Dense::zero(2 * m);
    // Step 2 — the two decoupled m×m systems share one matrix.
    let mut a = Dense::zero(m);
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let Some(rest) = tri_positions(mesh, tri) else {
            continue;
        };
        if tri
            .iter()
            .any(|&v| slot.get(v as usize).copied().unwrap_or(usize::MAX) == usize::MAX)
        {
            continue;
        }
        let wt = weights.get(t).copied().unwrap_or(1.0);
        for k in 0..3 {
            let (i0, i1, i2) = (
                slot.get(tri[k] as usize).copied().unwrap_or(0),
                slot.get(tri[(k + 1) % 3] as usize).copied().unwrap_or(0),
                slot.get(tri[(k + 2) % 3] as usize).copied().unwrap_or(0),
            );
            let (v0, v1, v2) = (rest[k], rest[(k + 1) % 3], rest[(k + 2) % 3]);
            let (ex, ey) = (v1[0] - v0[0], v1[1] - v0[1]);
            let len2 = ex * ex + ey * ey;
            if len2 < 1e-18 {
                continue;
            }
            let (dx, dy) = (v2[0] - v0[0], v2[1] - v0[1]);
            let x = (ex * dx + ey * dy) / len2;
            let y = (ey * dx - ex * dy) / len2;
            // rx = v2x − v0x − x(v1x − v0x) − y(v1y − v0y)
            accumulate(
                &mut g,
                wt,
                &[
                    (2 * i0, x - 1.0),
                    (2 * i0 + 1, y),
                    (2 * i1, -x),
                    (2 * i1 + 1, -y),
                    (2 * i2, 1.0),
                ],
            );
            // ry = v2y − v0y − x(v1y − v0y) + y(v1x − v0x)
            accumulate(
                &mut g,
                wt,
                &[
                    (2 * i0, -y),
                    (2 * i0 + 1, x - 1.0),
                    (2 * i1, y),
                    (2 * i1 + 1, -x),
                    (2 * i2 + 1, 1.0),
                ],
            );
            // Step 2's edge row: one per triangle edge, the same matrix for
            // both coordinates.
            accumulate(&mut a, wt, &[(i0, -1.0), (i1, 1.0)]);
        }
    }
    for row in &rows {
        let Some(cols) = row_slots(&slot, row) else {
            continue;
        };
        let terms: Vec<(usize, f64)> = cols
            .iter()
            .enumerate()
            .map(|(k, &c)| (c, row.bary.get(k).copied().unwrap_or(0.0)))
            .collect();
        accumulate(&mut a, row.weight, &terms);
        let xs: Vec<(usize, f64)> = terms.iter().map(|&(c, b)| (2 * c, b)).collect();
        let ys: Vec<(usize, f64)> = terms.iter().map(|&(c, b)| (2 * c + 1, b)).collect();
        accumulate(&mut g, row.weight, &xs);
        accumulate(&mut g, row.weight, &ys);
    }

    Factorisation {
        slot,
        back,
        lone,
        l1: cholesky(&g),
        l2: cholesky(&a),
        rows,
        weights,
        inert,
        key,
    }
}

/// `M += w · cᵀc` for a sparse row `c`.
fn accumulate(m: &mut Dense, w: f64, c: &[(usize, f64)]) {
    for &(i, ci) in c {
        for &(j, cj) in c {
            m.add(i, j, w * ci * cj);
        }
    }
}

/// A constraint row's three system columns, or `None` when any of its vertices
/// is not in the solved set.
fn row_slots(slot: &[usize], row: &Row) -> Option<[usize; 3]> {
    let mut out = [0usize; 3];
    for (o, v) in out.iter_mut().zip(row.verts) {
        let s = slot.get(v as usize).copied().unwrap_or(usize::MAX);
        if s == usize::MAX {
            return None;
        }
        *o = s;
    }
    Some(out)
}

fn row_target(row: &Row, pins: &[SolvePin]) -> [f64; 2] {
    match row.target {
        Target::Pin(i) => pins.get(i).map(|p| p.now).unwrap_or([0.0; 2]),
        Target::Bend { pin, offset } => {
            let Some(p) = pins.get(pin) else {
                return [0.0; 2];
            };
            let th = p.rotation.to_radians();
            let s = if p.scale.is_finite() {
                p.scale / 100.0
            } else {
                1.0
            };
            let (c, sn) = (th.cos(), th.sin());
            [
                p.now[0] + s * (c * offset[0] - sn * offset[1]),
                p.now[1] + s * (sn * offset[0] + c * offset[1]),
            ]
        }
    }
}

/// Overlap depth per vertex: the sum over the overlap pins of
/// `amount · falloff`. More "in front" draws later (docs/impl/puppet.md §3).
fn overlap_depth(mesh: &PuppetMesh, pins: &[SolvePin]) -> Vec<f64> {
    mesh.vertices
        .iter()
        .map(|p| {
            pins.iter()
                .filter(|pin| pin.kind == PuppetPinKind::Overlap)
                .map(|pin| (pin.amount / 100.0).clamp(-1.0, 1.0) * falloff(pin, *p))
                .sum()
        })
        .collect()
}

/// Is every constraining pin exactly where it started, with no bend?
fn all_at_rest(pins: &[SolvePin]) -> bool {
    pins.iter().all(|p| match p.kind {
        PuppetPinKind::Position => p.now == p.rest,
        PuppetPinKind::Bend => p.now == p.rest && p.rotation == 0.0 && p.scale == 100.0,
        _ => true,
    })
}

/// Deform the mesh so the pins land where they are asked to, using the
/// factorisation for this mesh and pin structure.
pub fn solve(mesh: &PuppetMesh, pins: &[SolvePin], f: &Factorisation) -> PuppetSolution {
    let rest = mesh.vertices.clone();
    let depth = overlap_depth(mesh, pins);
    let identity = f.rows.is_empty() || all_at_rest(pins);
    if identity {
        return PuppetSolution {
            vertices: rest,
            depth,
            inert: f.inert.clone(),
            identity: true,
        };
    }

    let mut out = rest.clone();
    // Components with exactly one constrained point translate: the similarity
    // step would leave their rotation and scale free, and translation is what
    // the user meant anyway (§2.3).
    for (v, slot) in f.lone.iter().enumerate() {
        let Some(row) = slot.and_then(|r| f.rows.get(r)) else {
            continue;
        };
        let target = row_target(row, pins);
        let source = point_of(&rest, row);
        if let Some(p) = out.get_mut(v) {
            p[0] += target[0] - source[0];
            p[1] += target[1] - source[1];
        }
    }

    let m = f.back.len();
    if m == 0 {
        return PuppetSolution {
            vertices: out,
            depth,
            inert: f.inert.clone(),
            identity: false,
        };
    }

    let (Some(l1), Some(l2)) = (f.l1.as_ref(), f.l2.as_ref()) else {
        return fallback(mesh, pins, f, depth);
    };

    // Step 1 — the similarity solve. Only the constraint rows carry a
    // right-hand side; the rigidity rows are homogeneous.
    let mut b1 = vec![0.0f64; 2 * m];
    for row in &f.rows {
        let Some(cols) = row_slots(&f.slot, row) else {
            continue;
        };
        let target = row_target(row, pins);
        for (k, &c) in cols.iter().enumerate() {
            let bk = row.bary.get(k).copied().unwrap_or(0.0);
            if let Some(s) = b1.get_mut(2 * c) {
                *s += row.weight * bk * target[0];
            }
            if let Some(s) = b1.get_mut(2 * c + 1) {
                *s += row.weight * bk * target[1];
            }
        }
    }
    let v1 = cholesky_solve(l1, &b1);

    // Step 2 — divide the scale back out of each triangle and re-solve, x and
    // y decoupled. This is what stops limbs inflating as they bend.
    let mut bx = vec![0.0f64; m];
    let mut by = vec![0.0f64; m];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let Some(r) = tri_positions(mesh, tri) else {
            continue;
        };
        let Some(cols) = row_slots(
            &f.slot,
            &Row {
                verts: *tri,
                bary: [0.0; 3],
                weight: 0.0,
                target: Target::Pin(0),
            },
        ) else {
            continue;
        };
        let wt = f.weights.get(t).copied().unwrap_or(1.0);
        let step1: Vec<[f64; 2]> = cols
            .iter()
            .map(|&c| {
                [
                    v1.get(2 * c).copied().unwrap_or(0.0),
                    v1.get(2 * c + 1).copied().unwrap_or(0.0),
                ]
            })
            .collect();
        let (cos, sin) = fitted_rotation(&r, &step1);
        for k in 0..3 {
            let (i0, i1) = (
                cols.get(k).copied().unwrap_or(0),
                cols.get((k + 1) % 3).copied().unwrap_or(0),
            );
            let (ex, ey) = (
                r.get((k + 1) % 3).map_or(0.0, |p| p[0]) - r.get(k).map_or(0.0, |p| p[0]),
                r.get((k + 1) % 3).map_or(0.0, |p| p[1]) - r.get(k).map_or(0.0, |p| p[1]),
            );
            let (tx, ty) = (cos * ex - sin * ey, sin * ex + cos * ey);
            for (b, rhs) in [(&mut bx, tx), (&mut by, ty)] {
                if let Some(s) = b.get_mut(i0) {
                    *s -= wt * rhs;
                }
                if let Some(s) = b.get_mut(i1) {
                    *s += wt * rhs;
                }
            }
        }
    }
    for row in &f.rows {
        let Some(cols) = row_slots(&f.slot, row) else {
            continue;
        };
        let target = row_target(row, pins);
        for (k, &c) in cols.iter().enumerate() {
            let bk = row.bary.get(k).copied().unwrap_or(0.0);
            if let Some(s) = bx.get_mut(c) {
                *s += row.weight * bk * target[0];
            }
            if let Some(s) = by.get_mut(c) {
                *s += row.weight * bk * target[1];
            }
        }
    }
    let xs = cholesky_solve(l2, &bx);
    let ys = cholesky_solve(l2, &by);
    for (i, &v) in f.back.iter().enumerate() {
        let p = [
            xs.get(i).copied().unwrap_or(0.0),
            ys.get(i).copied().unwrap_or(0.0),
        ];
        if !p[0].is_finite() || !p[1].is_finite() {
            return fallback(mesh, pins, f, depth);
        }
        if let Some(slot) = out.get_mut(v) {
            *slot = p;
        }
    }

    PuppetSolution {
        vertices: out,
        depth,
        inert: f.inert.clone(),
        identity: false,
    }
}

/// The rest position of a constraint row's point.
fn point_of(verts: &[[f64; 2]], row: &Row) -> [f64; 2] {
    let mut p = [0.0f64; 2];
    for (k, &v) in row.verts.iter().enumerate() {
        let b = row.bary.get(k).copied().unwrap_or(0.0);
        let q = verts.get(v as usize).copied().unwrap_or([0.0; 2]);
        p[0] += b * q[0];
        p[1] += b * q[1];
    }
    p
}

/// The rotation half of the least-squares similarity carrying `rest` onto
/// `image`, as `(cos, sin)`: fit the similarity, then divide its scale out.
fn fitted_rotation(rest: &[[f64; 2]; 3], image: &[[f64; 2]]) -> (f64, f64) {
    let mean = |ps: &[[f64; 2]]| {
        let n = ps.len().max(1) as f64;
        [
            ps.iter().map(|p| p[0]).sum::<f64>() / n,
            ps.iter().map(|p| p[1]).sum::<f64>() / n,
        ]
    };
    let (cr, ci) = (mean(rest), mean(image));
    let (mut a, mut b) = (0.0f64, 0.0f64);
    for (p, q) in rest.iter().zip(image) {
        let (px, py) = (p[0] - cr[0], p[1] - cr[1]);
        let (qx, qy) = (q[0] - ci[0], q[1] - ci[1]);
        a += px * qx + py * qy;
        b += px * qy - py * qx;
    }
    let norm = (a * a + b * b).sqrt();
    if !norm.is_finite() || norm < 1e-12 {
        (1.0, 0.0)
    } else {
        (a / norm, b / norm)
    }
}

/// A failed factorisation — numerically conceivable with pathological starch
/// weights — translates the mesh by the mean pin delta, never a panic
/// (docs/14 §4).
fn fallback(
    mesh: &PuppetMesh,
    pins: &[SolvePin],
    f: &Factorisation,
    depth: Vec<f64>,
) -> PuppetSolution {
    let mut delta = [0.0f64; 2];
    let mut n = 0.0f64;
    for row in &f.rows {
        let target = row_target(row, pins);
        let source = point_of(&mesh.vertices, row);
        delta[0] += target[0] - source[0];
        delta[1] += target[1] - source[1];
        n += 1.0;
    }
    if n > 0.0 {
        delta[0] /= n;
        delta[1] /= n;
    }
    PuppetSolution {
        vertices: mesh
            .vertices
            .iter()
            .map(|p| [p[0] + delta[0], p[1] + delta[1]])
            .collect(),
        depth,
        inert: f.inert.clone(),
        identity: false,
    }
}

// --- The warp ---------------------------------------------------------------

/// Redraw `rgba` through the deformed mesh: each triangle carries its patch of
/// pixels to wherever the solve put it (docs/impl/puppet.md §3).
///
/// `rgba` is premultiplied, `w`×`h`; `natural_w`/`natural_h` are the layer's
/// natural size, which is the space the mesh lives in — the same convention
/// paint uses.
pub fn apply_puppet(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    natural_w: f64,
    natural_h: f64,
    mesh: &PuppetMesh,
    solution: &PuppetSolution,
) {
    if solution.identity || w == 0 || h == 0 {
        return;
    }
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return;
    }
    let sx = f64::from(w) / natural_w.max(1.0);
    let sy = f64::from(h) / natural_h.max(1.0);
    let src = rgba.to_vec();
    rgba.fill(0);

    // Painter's algorithm: more "in front" draws later, ties broken by triangle
    // index so the order is a fact about the mesh and not about the sort.
    let mut order: Vec<(f64, usize)> = mesh
        .triangles
        .iter()
        .enumerate()
        .map(|(i, tri)| {
            let d: f64 = tri
                .iter()
                .map(|&v| solution.depth.get(v as usize).copied().unwrap_or(0.0))
                .sum::<f64>()
                / 3.0;
            (d, i)
        })
        .collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));

    let (iw, ih) = (w as usize, h as usize);
    for (_, t) in order {
        let Some(tri) = mesh.triangles.get(t) else {
            continue;
        };
        let mut dst = [[0.0f64; 2]; 3];
        let mut rest = [[0.0f64; 2]; 3];
        let mut ok = true;
        for k in 0..3 {
            let v = tri[k] as usize;
            let (Some(d), Some(r)) = (solution.vertices.get(v), mesh.vertices.get(v)) else {
                ok = false;
                break;
            };
            dst[k] = [d[0] * sx, d[1] * sy];
            rest[k] = [r[0] * sx, r[1] * sy];
        }
        if !ok {
            continue;
        }
        let area2 = (dst[1][0] - dst[0][0]) * (dst[2][1] - dst[0][1])
            - (dst[2][0] - dst[0][0]) * (dst[1][1] - dst[0][1]);
        // A degenerate deformed triangle — crossed pins folding the mesh flat —
        // is skipped, so a fold draws honestly rather than as NaNs.
        if !area2.is_finite() || area2.abs() < 1e-9 {
            continue;
        }
        let minx = dst.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
        let maxx = dst.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let miny = dst.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        let maxy = dst.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
        if !minx.is_finite() || !miny.is_finite() {
            continue;
        }
        let x0 = minx.floor().max(0.0) as usize;
        let x1 = (maxx.ceil().max(0.0) as usize).min(iw);
        let y0 = miny.floor().max(0.0) as usize;
        let y1 = (maxy.ceil().max(0.0) as usize).min(ih);
        // The barycentric coordinates are an **affine** function of the pixel,
        // so everything about the triangle is worked out here rather than per
        // pixel: one reciprocal instead of two divisions a pixel, which is most
        // of what a full-frame warp costs.
        let d = (dst[1][1] - dst[2][1]) * (dst[0][0] - dst[2][0])
            + (dst[2][0] - dst[1][0]) * (dst[0][1] - dst[2][1]);
        if !d.is_finite() || d.abs() < 1e-12 {
            continue;
        }
        let inv_d = 1.0 / d;
        let (l0x, l0y) = (
            (dst[1][1] - dst[2][1]) * inv_d,
            (dst[2][0] - dst[1][0]) * inv_d,
        );
        let (l1x, l1y) = (
            (dst[2][1] - dst[0][1]) * inv_d,
            (dst[0][0] - dst[2][0]) * inv_d,
        );
        for py in y0..y1 {
            let dy = py as f64 + 0.5 - dst[2][1];
            let (row0, row1) = (l0y * dy, l1y * dy);
            for px in x0..x1 {
                let dx = px as f64 + 0.5 - dst[2][0];
                let l = [l0x * dx + row0, l1x * dx + row1, 0.0];
                let l = [l[0], l[1], 1.0 - l[0] - l[1]];
                if l[0] < -1e-9 || l[1] < -1e-9 || l[2] < -1e-9 {
                    continue;
                }
                let sxp = l[0] * rest[0][0] + l[1] * rest[1][0] + l[2] * rest[2][0] - 0.5;
                let syp = l[0] * rest[0][1] + l[1] * rest[1][1] + l[2] * rest[2][1] - 0.5;
                let texel = sample_rgba(&src, iw, ih, sxp, syp);
                let base = (py * iw + px) * 4;
                if let Some(slot) = rgba.get_mut(base..base + 4) {
                    slot.copy_from_slice(&texel);
                }
            }
        }
    }
}

/// Bilinear sample of a premultiplied RGBA buffer at pixel-index coordinates.
fn sample_rgba(src: &[u8], w: usize, h: usize, x: f64, y: f64) -> [u8; 4] {
    if w == 0 || h == 0 {
        return [0; 4];
    }
    let fx = x.clamp(0.0, (w - 1) as f64);
    let fy = y.clamp(0.0, (h - 1) as f64);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
    // The four texels' offsets, once for all four channels: the same sixteen
    // multiplies a channel used to redo.
    let (i00, i10) = ((y0 * w + x0) * 4, (y0 * w + x1) * 4);
    let (i01, i11) = ((y1 * w + x0) * 4, (y1 * w + x1) * 4);
    let mut out = [0u8; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let g = |i: usize| f64::from(src.get(i + c).copied().unwrap_or(0));
        let top = g(i00) * (1.0 - tx) + g(i10) * tx;
        let bot = g(i01) * (1.0 - tx) + g(i11) * tx;
        *o = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
    }
    out
}

// --- Caches -----------------------------------------------------------------

/// How many meshes and factorisations are held. Small on purpose: the cost of a
/// miss is a rebuild, and the cost of a hit that never comes is memory.
const CACHE_SLOTS: usize = 4;

/// The mesh and factorisation caches (docs/impl/puppet.md §1.4, §2.5), both
/// keyed deterministically so a cached frame is the frame that would have been
/// computed.
#[derive(Default)]
pub struct PuppetCache {
    meshes: Mutex<Vec<Arc<PuppetMesh>>>,
    factors: Mutex<Vec<Arc<Factorisation>>>,
}

impl PuppetCache {
    pub fn new() -> PuppetCache {
        PuppetCache::default()
    }

    /// The mesh for this alpha, density and expansion — built on the first ask
    /// and remembered after.
    pub fn mesh(
        &self,
        alpha: &[u8],
        w: u32,
        h: u32,
        density: f64,
        expansion: f64,
    ) -> Result<Arc<PuppetMesh>, MeshError> {
        let key = mesh_hash(alpha, density, expansion);
        {
            let mut held = self.meshes.lock();
            if let Some(i) = held.iter().position(|m| m.hash == key) {
                let hit = held.remove(i);
                held.insert(0, Arc::clone(&hit));
                return Ok(hit);
            }
        }
        let built = Arc::new(build_mesh(alpha, w, h, density, expansion)?);
        let mut held = self.meshes.lock();
        held.insert(0, Arc::clone(&built));
        held.truncate(CACHE_SLOTS);
        Ok(built)
    }

    /// The factored systems for this mesh and pin structure.
    pub fn factorisation(&self, mesh: &PuppetMesh, pins: &[SolvePin]) -> Arc<Factorisation> {
        let key = factor_key(mesh, pins);
        {
            let mut held = self.factors.lock();
            if let Some(i) = held.iter().position(|f| f.key == key) {
                let hit = held.remove(i);
                held.insert(0, Arc::clone(&hit));
                return hit;
            }
        }
        let built = Arc::new(factorise(mesh, pins));
        let mut held = self.factors.lock();
        held.insert(0, Arc::clone(&built));
        held.truncate(CACHE_SLOTS);
        built
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A filled axis-aligned rectangle of alpha in a `w`×`h` bitmap.
    fn rect(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<u8> {
        let mut a = vec![0u8; w * h];
        for y in y0..y1 {
            for x in x0..x1 {
                a[y * w + x] = 255;
            }
        }
        a
    }

    fn bar_mesh() -> PuppetMesh {
        let alpha = rect(160, 40, 10, 12, 150, 28);
        build_mesh(&alpha, 160, 40, 12.0, 0.0).unwrap()
    }

    fn solved(mesh: &PuppetMesh, pins: &[SolvePin]) -> PuppetSolution {
        let f = factorise(mesh, pins);
        solve(mesh, pins, &f)
    }

    /// **1 — a rectangle is one contour with four corners.** The staircase the
    /// marching squares walks must simplify back to the shape a human sees.
    #[test]
    fn a_rectangle_simplifies_to_four_corners() {
        let alpha = rect(40, 30, 10, 8, 30, 22);
        let cov = expanded_coverage(&alpha, 40, 30, 0.0).unwrap();
        let raw = contours(&cov);
        assert_eq!(raw.len(), 1, "one silhouette, one contour");
        let simple = simplify_all(&raw, SIMPLIFY_TOLERANCE, 8.0);
        assert_eq!(simple.len(), 1);
        let c = &simple[0];
        assert_eq!(c.len(), 4, "four corners, got {c:?}");
        let pad = cov.pad as f64;
        let corners: Vec<[f64; 2]> = c.iter().map(|p| [p[0] - pad, p[1] - pad]).collect();
        // Pixel centres are whole coordinates, so the edge of the filled block
        // sits half a pixel outside the outermost covered centre. Marching
        // squares chamfers a right angle across the corner cell, and
        // simplification keeps one end of that chamfer — so a corner lands
        // within one cell's diagonal of the true corner, not on the nose.
        for want in [[9.5, 7.5], [29.5, 7.5], [9.5, 21.5], [29.5, 21.5]] {
            assert!(
                corners
                    .iter()
                    .any(|g| (g[0] - want[0]).abs() <= 0.75 && (g[1] - want[1]).abs() <= 0.75),
                "no corner near {want:?} in {corners:?}"
            );
        }
    }

    /// **2 — a ring is two contours, and nothing is meshed over the hole.**
    #[test]
    fn a_ring_keeps_its_hole_empty() {
        let mut alpha = rect(80, 80, 8, 8, 72, 72);
        for y in 28..52 {
            for x in 28..52 {
                alpha[y * 80 + x] = 0;
            }
        }
        let cov = expanded_coverage(&alpha, 80, 80, 0.0).unwrap();
        assert_eq!(contours(&cov).len(), 2, "outer silhouette and hole");

        let mesh = build_mesh(&alpha, 80, 80, 10.0, 0.0).unwrap();
        for tri in &mesh.triangles {
            let [a, b, c] = tri_positions(&mesh, tri).unwrap();
            let cx = (a[0] + b[0] + c[0]) / 3.0;
            let cy = (a[1] + b[1] + c[1]) / 3.0;
            assert!(
                !(28.0..51.0).contains(&cx) || !(28.0..51.0).contains(&cy),
                "triangle centroid {cx},{cy} sits in the hole"
            );
        }
    }

    /// **3 — the weld test.** Two blobs with a gap between them must come out
    /// as two mesh components, and a drag in one must not move the other by so
    /// much as a bit.
    #[test]
    fn disjoint_blobs_are_never_welded() {
        let mut alpha = rect(160, 60, 10, 15, 60, 45);
        for y in 15..45 {
            for x in 100..150 {
                alpha[y * 160 + x] = 255;
            }
        }
        let mesh = build_mesh(&alpha, 160, 60, 10.0, 0.0).unwrap();
        let comp = components(mesh.vertices.len(), &mesh.triangles);
        let roots: std::collections::BTreeSet<usize> = comp.iter().copied().collect();
        assert_eq!(roots.len(), 2, "two blobs, two components");

        let mut pins = vec![
            SolvePin::position(Uuid::from_u128(1), [20.0, 30.0]),
            SolvePin::position(Uuid::from_u128(2), [50.0, 30.0]),
        ];
        pins[0].now = [20.0, 10.0];
        let out = solved(&mesh, &pins);
        assert!(out.inert.is_empty(), "both pins bound");
        let mut moved_a = 0;
        for (i, (r, d)) in mesh.vertices.iter().zip(&out.vertices).enumerate() {
            if r[0] > 90.0 {
                assert_eq!(r, d, "vertex {i} of the far blob moved");
            } else if r != d {
                moved_a += 1;
            }
        }
        assert!(moved_a > 0, "the dragged blob must actually deform");
    }

    /// **4 — the same input twice is the same bits twice.** The whole cache
    /// scheme rests on this.
    #[test]
    fn mesh_and_solve_are_deterministic() {
        let alpha = rect(120, 80, 12, 10, 108, 70);
        let one = build_mesh(&alpha, 120, 80, 11.0, 3.0).unwrap();
        let two = build_mesh(&alpha, 120, 80, 11.0, 3.0).unwrap();
        assert_eq!(one, two, "two builds, one mesh");

        let mut pins = vec![
            SolvePin::position(Uuid::from_u128(7), [20.0, 40.0]),
            SolvePin::position(Uuid::from_u128(8), [100.0, 40.0]),
        ];
        pins[1].now = [104.0, 21.0];
        let a = solved(&one, &pins);
        let b = solved(&two, &pins);
        assert_eq!(
            a.vertices
                .iter()
                .map(|p| [p[0].to_bits(), p[1].to_bits()])
                .collect::<Vec<_>>(),
            b.vertices
                .iter()
                .map(|p| [p[0].to_bits(), p[1].to_bits()])
                .collect::<Vec<_>>(),
            "two solves, one answer, bit for bit"
        );
    }

    /// **5 — a rigid motion is reproduced exactly.** Two pins moved by the same
    /// delta is a translation, and ARAP's whole promise is that it finds one
    /// when one exists.
    #[test]
    fn two_pins_translated_together_translate_the_mesh() {
        let mesh = bar_mesh();
        let d = [13.0, -7.0];
        let mut pins = vec![
            SolvePin::position(Uuid::from_u128(1), [20.0, 20.0]),
            SolvePin::position(Uuid::from_u128(2), [140.0, 20.0]),
        ];
        for p in pins.iter_mut() {
            p.now = [p.rest[0] + d[0], p.rest[1] + d[1]];
        }
        let out = solved(&mesh, &pins);
        for (r, v) in mesh.vertices.iter().zip(&out.vertices) {
            assert!(
                (v[0] - r[0] - d[0]).abs() < 1e-6 && (v[1] - r[1] - d[1]).abs() < 1e-6,
                "vertex {r:?} landed at {v:?}"
            );
        }
    }

    /// **6 — a quarter turn comes out a quarter turn**, not a shear.
    #[test]
    fn a_rotated_pin_rotates_the_mesh() {
        let mesh = bar_mesh();
        let pivot = [20.0, 20.0];
        let far = [140.0, 20.0];
        let turn = |p: [f64; 2]| {
            let (dx, dy) = (p[0] - pivot[0], p[1] - pivot[1]);
            [pivot[0] - dy, pivot[1] + dx]
        };
        let mut pins = vec![
            SolvePin::position(Uuid::from_u128(1), pivot),
            SolvePin::position(Uuid::from_u128(2), far),
        ];
        pins[1].now = turn(far);
        let out = solved(&mesh, &pins);
        for (r, v) in mesh.vertices.iter().zip(&out.vertices) {
            let want = turn(*r);
            assert!(
                (v[0] - want[0]).abs() < 0.5 && (v[1] - want[1]).abs() < 0.5,
                "vertex {r:?} wanted {want:?}, got {v:?}"
            );
        }
    }

    /// How far a triangle's deformation is from any similarity — the number
    /// starch is supposed to push down.
    fn deviation(mesh: &PuppetMesh, out: &[[f64; 2]], mid: (f64, f64)) -> f64 {
        let mut total = 0.0;
        for tri in &mesh.triangles {
            let rest = tri_positions(mesh, tri).unwrap();
            let cx = (rest[0][0] + rest[1][0] + rest[2][0]) / 3.0;
            if cx < mid.0 || cx > mid.1 {
                continue;
            }
            let now: Vec<[f64; 2]> = tri
                .iter()
                .map(|&v| out.get(v as usize).copied().unwrap_or([0.0; 2]))
                .collect();
            let (cos, sin) = fitted_rotation(&rest, &now);
            // Best uniform scale for that rotation, then the residual.
            let (mut num, mut den) = (0.0f64, 0.0f64);
            for k in 0..3 {
                let (ex, ey) = (
                    rest[(k + 1) % 3][0] - rest[k][0],
                    rest[(k + 1) % 3][1] - rest[k][1],
                );
                let (rx, ry) = (cos * ex - sin * ey, sin * ex + cos * ey);
                let (dx, dy) = (
                    now[(k + 1) % 3][0] - now[k][0],
                    now[(k + 1) % 3][1] - now[k][1],
                );
                num += rx * dx + ry * dy;
                den += rx * rx + ry * ry;
            }
            let s = if den > 1e-12 { num / den } else { 1.0 };
            for k in 0..3 {
                let (ex, ey) = (
                    rest[(k + 1) % 3][0] - rest[k][0],
                    rest[(k + 1) % 3][1] - rest[k][1],
                );
                let (rx, ry) = (s * (cos * ex - sin * ey), s * (sin * ex + cos * ey));
                let (dx, dy) = (
                    now[(k + 1) % 3][0] - now[k][0],
                    now[(k + 1) % 3][1] - now[k][1],
                );
                total += (dx - rx).powi(2) + (dy - ry).powi(2);
            }
        }
        total
    }

    /// **7 — starch stiffens.** The same bend, with a starch pin mid-bar, must
    /// leave the middle measurably closer to rigid.
    #[test]
    fn starch_reduces_deformation_in_its_region() {
        let mesh = bar_mesh();
        let mut pins = vec![
            SolvePin::position(Uuid::from_u128(1), [20.0, 20.0]),
            SolvePin::position(Uuid::from_u128(2), [80.0, 20.0]),
            SolvePin::position(Uuid::from_u128(3), [140.0, 20.0]),
        ];
        pins[1].now = [80.0, 2.0];
        let loose = solved(&mesh, &pins);

        let mut starched = pins.clone();
        starched.push(SolvePin {
            id: Uuid::from_u128(4),
            kind: PuppetPinKind::Starch,
            rest: [80.0, 20.0],
            now: [80.0, 20.0],
            rotation: 0.0,
            scale: 100.0,
            amount: 100.0,
            extent: 40.0,
        });
        let stiff = solved(&mesh, &starched);

        let a = deviation(&mesh, &loose.vertices, (55.0, 105.0));
        let b = deviation(&mesh, &stiff.vertices, (55.0, 105.0));
        assert!(b < a, "starch should stiffen the middle: {b} vs {a}");
    }

    /// **8 — overlap decides the fold.** With the bar folded back over itself,
    /// the pixel at the overlap belongs to whichever half is in front, and
    /// swapping the amounts swaps the pixel.
    #[test]
    fn overlap_amount_decides_which_half_draws_in_front() {
        let (w, h) = (160usize, 40usize);
        let alpha = rect(w, h, 10, 12, 150, 28);
        let mesh = build_mesh(&alpha, w as u32, h as u32, 12.0, 0.0).unwrap();

        // Left half red, right half blue, so the winning half is legible.
        let mut src = vec![0u8; w * h * 4];
        for y in 12..28 {
            for x in 10..150 {
                let px = (y * w + x) * 4;
                let c = if x < 80 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                src[px..px + 4].copy_from_slice(&c);
            }
        }
        // Fold the right half back over the left by hand: the solve's fold is
        // tested elsewhere; this test is about the draw order.
        let folded: Vec<[f64; 2]> = mesh
            .vertices
            .iter()
            .map(|p| {
                if p[0] > 80.0 {
                    [160.0 - p[0], p[1]]
                } else {
                    *p
                }
            })
            .collect();

        let read = |front_left: bool| {
            let pins = vec![
                SolvePin {
                    id: Uuid::from_u128(1),
                    kind: PuppetPinKind::Overlap,
                    rest: [40.0, 20.0],
                    now: [40.0, 20.0],
                    rotation: 0.0,
                    scale: 100.0,
                    amount: if front_left { 100.0 } else { -100.0 },
                    extent: 60.0,
                },
                SolvePin {
                    id: Uuid::from_u128(2),
                    kind: PuppetPinKind::Overlap,
                    rest: [120.0, 20.0],
                    now: [120.0, 20.0],
                    rotation: 0.0,
                    scale: 100.0,
                    amount: if front_left { -100.0 } else { 100.0 },
                    extent: 60.0,
                },
            ];
            let solution = PuppetSolution {
                vertices: folded.clone(),
                depth: overlap_depth(&mesh, &pins),
                inert: Vec::new(),
                identity: false,
            };
            let mut buf = src.clone();
            apply_puppet(
                &mut buf, w as u32, h as u32, w as f64, h as f64, &mesh, &solution,
            );
            let px = (20 * w + 40) * 4;
            [buf[px], buf[px + 2]]
        };

        let left_front = read(true);
        let right_front = read(false);
        assert!(
            left_front[0] > left_front[1],
            "left in front should read red, got {left_front:?}"
        );
        assert!(
            right_front[1] > right_front[0],
            "right in front should read blue, got {right_front:?}"
        );
    }

    /// **9 — a bend pin turns its own region.** Vertices inside the extent
    /// follow the rotation; a vertex outside it moves less than one inside.
    /// The far end is held by an ordinary position pin, which is what makes
    /// "outside the extent" mean anything: with nothing else pinned, the whole
    /// bar would swing round the bend and the far end would travel furthest.
    #[test]
    fn a_bend_pin_rotates_its_extent() {
        let mesh = bar_mesh();
        let centre = [80.0, 20.0];
        let pins = vec![
            SolvePin {
                id: Uuid::from_u128(1),
                kind: PuppetPinKind::Bend,
                rest: centre,
                now: centre,
                rotation: 30.0,
                scale: 100.0,
                amount: 0.0,
                extent: 30.0,
            },
            SolvePin::position(Uuid::from_u128(2), [145.0, 20.0]),
        ];
        let out = solved(&mesh, &pins);
        assert!(!out.identity, "a rotation is not a no-op");

        let th: f64 = 30f64.to_radians();
        let (c, s) = (th.cos(), th.sin());
        let mut inside = 0;
        for (r, v) in mesh.vertices.iter().zip(&out.vertices) {
            let (dx, dy) = (r[0] - centre[0], r[1] - centre[1]);
            if (dx * dx + dy * dy).sqrt() > 12.0 {
                continue;
            }
            inside += 1;
            let want = [centre[0] + c * dx - s * dy, centre[1] + s * dx + c * dy];
            assert!(
                (v[0] - want[0]).abs() < 2.0 && (v[1] - want[1]).abs() < 2.0,
                "vertex {r:?} wanted {want:?}, got {v:?}"
            );
        }
        assert!(inside > 0, "the extent has to contain vertices");

        let travel = |p: [f64; 2]| {
            let i = mesh
                .vertices
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let d = |q: &[f64; 2]| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                    d(a.1).total_cmp(&d(b.1))
                })
                .map(|(i, _)| i)
                .unwrap();
            let (r, v) = (mesh.vertices[i], out.vertices[i]);
            ((v[0] - r[0]).powi(2) + (v[1] - r[1]).powi(2)).sqrt()
        };
        assert!(
            travel([145.0, 20.0]) < travel([80.0, 26.0]),
            "a vertex outside the extent should move less than one inside"
        );
    }

    /// **10 — one pin is a translation**, not an underdetermined solve.
    #[test]
    fn one_pin_translates() {
        let mesh = bar_mesh();
        let mut pins = vec![SolvePin::position(Uuid::from_u128(1), [80.0, 20.0])];
        pins[0].now = [95.0, 5.0];
        let out = solved(&mesh, &pins);
        for (r, v) in mesh.vertices.iter().zip(&out.vertices) {
            assert!(
                (v[0] - r[0] - 15.0).abs() < 1e-9 && (v[1] - r[1] + 15.0).abs() < 1e-9,
                "vertex {r:?} landed at {v:?}"
            );
        }
    }

    /// **11 — the refusals are values.** A transparent layer, a click outside
    /// the mesh, and a density no coarsening can afford each come back as an
    /// answer rather than a crash.
    #[test]
    fn refusals_are_values_not_panics() {
        let empty = vec![0u8; 40 * 40];
        assert_eq!(
            build_mesh(&empty, 40, 40, 12.0, 3.0),
            Err(MeshError::Empty),
            "nothing opaque, no mesh"
        );

        let mesh = bar_mesh();
        assert!(locate(&mesh, [80.0, 20.0]).is_some(), "inside the bar");
        assert!(locate(&mesh, [80.0, 39.0]).is_none(), "outside the bar");

        let big = rect(220, 220, 5, 5, 215, 215);
        match build_mesh(&big, 220, 220, 0.5, 0.0) {
            Err(MeshError::TooDense { vertices }) => {
                assert!(vertices > VERTEX_CAP, "refused at {vertices} vertices")
            }
            other => panic!("a 0.5 px density over 210² px should refuse, got {other:?}"),
        }
    }

    /// **12 — an untouched puppet block is a no-op**, byte for byte, so a frame
    /// cache that includes it stays honest.
    #[test]
    fn pins_at_rest_leave_the_buffer_untouched() {
        let (w, h) = (160usize, 40usize);
        let mesh = bar_mesh();
        let pins = vec![
            SolvePin::position(Uuid::from_u128(1), [20.0, 20.0]),
            SolvePin::position(Uuid::from_u128(2), [140.0, 20.0]),
        ];
        let out = solved(&mesh, &pins);
        assert!(out.identity, "no pin moved, so nothing moves");

        let mut buf: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let before = buf.clone();
        apply_puppet(
            &mut buf, w as u32, h as u32, w as f64, h as f64, &mesh, &out,
        );
        assert_eq!(buf, before, "an identity warp must not touch a byte");
    }

    /// The caches hand back the same object rather than rebuilding, and a
    /// changed pin position does *not* invalidate the factorisation — the whole
    /// point of the 2005 formulation.
    #[test]
    fn caches_hit_on_the_keys_they_promise() {
        let alpha = rect(120, 60, 10, 10, 110, 50);
        let cache = PuppetCache::new();
        let a = cache.mesh(&alpha, 120, 60, 12.0, 0.0).unwrap();
        let b = cache.mesh(&alpha, 120, 60, 12.0, 0.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "same alpha, same mesh");
        let c = cache.mesh(&alpha, 120, 60, 16.0, 0.0).unwrap();
        assert!(!Arc::ptr_eq(&a, &c), "a new density is a new mesh");

        let mut pins = vec![
            SolvePin::position(Uuid::from_u128(1), [20.0, 30.0]),
            SolvePin::position(Uuid::from_u128(2), [100.0, 30.0]),
        ];
        let f1 = cache.factorisation(&a, &pins);
        pins[1].now = [100.0, 12.0];
        let f2 = cache.factorisation(&a, &pins);
        assert!(Arc::ptr_eq(&f1, &f2), "moving a pin only moves the RHS");
        pins.push(SolvePin::position(Uuid::from_u128(3), [60.0, 30.0]));
        let f3 = cache.factorisation(&a, &pins);
        assert!(!Arc::ptr_eq(&f1, &f3), "a new pin is a new factorisation");
    }

    /// The block survives a trip through the file, writes only what differs
    /// from the defaults, and keeps a newer version's unknown fields.
    #[test]
    fn the_block_round_trips_and_keeps_what_it_does_not_understand() {
        let mut block = PuppetBlock::new(Rational::new(3, 2).unwrap());
        block
            .pins
            .push(PuppetPin::new(PuppetPinKind::Position, "Pin 1", 12.5, 40.0));
        let mut bend = PuppetPin::new(PuppetPinKind::Bend, "Bend 1", 80.0, 20.0);
        bend.rotation = Property::fixed(35.0);
        bend.extent = 72.0;
        block.pins.push(bend);

        let text = serde_json::to_string(&block).unwrap();
        let back: PuppetBlock = serde_json::from_str(&text).unwrap();
        assert_eq!(back, block, "a puppet block is what it was written as");

        // A still pin is a bare number, and the values nobody moved are absent
        // altogether — the file-format rule every other animatable field keeps.
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["pins"][0]["x"], serde_json::json!(12.5));
        assert!(
            v["pins"][0].get("rotation").is_none() && v["pins"][0].get("scale").is_none(),
            "an unmoved position pin writes no bend values: {text}"
        );

        // A newer Lumit's field rides through untouched, and the defaults fill
        // in for a block written before they existed.
        let older: PuppetBlock =
            serde_json::from_str(r#"{"reference_time":[0,1],"pins":[],"wobble":7}"#).unwrap();
        assert_eq!(older.density, DEFAULT_DENSITY);
        assert_eq!(older.expansion, DEFAULT_EXPANSION);
        assert_eq!(older.extra.get("wobble"), Some(&serde_json::json!(7)));
        let again = serde_json::to_value(&older).unwrap();
        assert_eq!(again["wobble"], serde_json::json!(7));
    }

    /// A pin's rest position is read at the reference time and its live one at
    /// the frame — which is what makes a keyframed pin move at all.
    #[test]
    fn pins_rest_at_the_reference_time_and_move_at_the_frame() {
        let key = |t: i64, value: f64| crate::anim::Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value,
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        };
        let mut block = PuppetBlock::new(Rational::ONE);
        let mut pin = PuppetPin::new(PuppetPinKind::Position, "Pin 1", 10.0, 10.0);
        pin.x = Property {
            animation: Animation::Keyframed(vec![key(1, 10.0), key(2, 50.0)]),
            extra: serde_json::Map::new(),
        };
        block.pins.push(pin);
        let at_two = block.pins_at(2.0);
        assert_eq!(at_two[0].rest, [10.0, 10.0], "rest is the reference time");
        assert_eq!(at_two[0].now, [50.0, 10.0], "now is this frame");
    }
}

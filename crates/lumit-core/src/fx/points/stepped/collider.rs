//! What Particulate's particles hit: a layer's outline at one time, seen from
//! the layer the particles are on.

use crate::camera::Place;
use crate::expression::ExpressionContext;
use crate::fx::points::Contact;
use crate::mask::MaskPolyline;
use crate::model::{Composition, Document, Layer, LayerKind};

/// The most surfaces one particle comes off in one step.
const MAX_HITS: usize = 4;

/// How far off a surface a particle is left after hitting it, px, so its
/// next move does not start on the surface.
const NUDGE: f32 = 1e-3;

/// A collider's outline at one time, seen from the Particulate layer.
#[derive(Debug, Clone)]
pub struct Collider {
    /// Each edge's two ends: in this layer's pixels for walls, in the
    /// collider's own for a surface.
    edges: Vec<[[f32; 2]; 2]>,
    /// The collider layer's own pixels to this layer's, in 3D.
    pub(super) to_layer: Place,
    /// And back.
    pub(super) from_layer: Place,
    /// Whether it is a surface turned against this layer, rather than walls.
    surface: bool,
    grid: Grid,
}

impl Collider {
    /// `target`'s shapes and masks at comp time `t`, seen from `owner`'s
    /// pixels. A layer with neither collides with its rectangle.
    #[must_use]
    pub fn of(
        doc: &Document,
        comp: &Composition,
        owner: &Layer,
        target: &Layer,
        t: f64,
        context: &ExpressionContext,
    ) -> Self {
        Self::seen(doc, comp, owner, target, t, context, false)
    }

    /// As [`Collider::of`], but a 3D layer facing the way this one does is a
    /// surface too, as a simulation in 3D wants.
    #[must_use]
    pub fn of_in_3d(
        doc: &Document,
        comp: &Composition,
        owner: &Layer,
        target: &Layer,
        t: f64,
        context: &ExpressionContext,
    ) -> Self {
        let in_3d = owner.switches.three_d && target.switches.three_d;
        Self::seen(doc, comp, owner, target, t, context, in_3d)
    }

    fn seen(
        doc: &Document,
        comp: &Composition,
        owner: &Layer,
        target: &Layer,
        t: f64,
        context: &ExpressionContext,
        always_surface: bool,
    ) -> Self {
        use crate::fx::drivers::measure;
        let lt = crate::time::layer_time(t, target.start_offset.0);
        let mut lines: Vec<MaskPolyline> = target
            .masks
            .iter()
            .map(|m| crate::mask::flatten_path(&m.path_at(lt), crate::mask::MASK_PATH_TOLERANCE_PX))
            .collect();
        if let LayerKind::Shape { contents } = &target.kind {
            lines.push(crate::shape::outline_at(contents, lt));
        }
        if lines.iter().all(MaskPolyline::is_empty) {
            if let Some([l, t0, r, b]) = measure::layer_rect(doc, comp, target, t, context) {
                let points = [[l, t0], [r, t0], [r, b], [l, b], [l, t0]];
                let points: Vec<[f32; 2]> =
                    points.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                // Only the arc's steps are read, to tell a jump from an edge.
                let arc = (0..points.len()).map(|i| i as f32).collect();
                lines.push(MaskPolyline {
                    points,
                    arc,
                    closed: true,
                    ..MaskPolyline::default()
                });
            }
        }
        // Where both layers sit in the comp's 3D space, as toWorld reads it.
        let read = crate::expression::space::reader(context, comp.id, t);
        let world =
            |l: &Layer| crate::expression::space::space(&context.document, comp, l, t, &read).world;
        let owner = world(owner);
        let Some(back) = owner.invert() else {
            return Collider::new(Vec::new(), owner, false);
        };
        let to_layer = world(target).within(&back);
        // The card's normal here. Along this layer's z, it is walls.
        let m = &to_layer.m;
        let n = [
            m[1][0] * m[2][1] - m[2][0] * m[1][1],
            m[2][0] * m[0][1] - m[0][0] * m[2][1],
            m[0][0] * m[1][1] - m[1][0] * m[0][1],
        ];
        let surface = always_surface || n[0].hypot(n[1]) > 1e-6 * n[2].abs();
        let place = |p: [f32; 2]| {
            if surface {
                return p;
            }
            let (x, y, _) = to_layer.point((f64::from(p[0]), f64::from(p[1]), 0.0));
            [x as f32, y as f32]
        };
        let mut edges = Vec::new();
        let mut push = |a: [f32; 2], b: [f32; 2]| {
            let edge = [place(a), place(b)];
            if edge.iter().flatten().all(|v| v.is_finite()) {
                edges.push(edge);
            }
        };
        for line in &lines {
            let mut start = 0;
            for i in 0..line.points.len().saturating_sub(1) {
                // Two paths of one run are joined by a step with no length,
                // which is a jump between them and not an edge. A surface is
                // filled, so there each path closes.
                if line.arc[i + 1] <= line.arc[i] {
                    if surface {
                        push(line.points[i], line.points[start]);
                    }
                    start = i + 1;
                    continue;
                }
                push(line.points[i], line.points[i + 1]);
            }
            if let Some(&last) = line.points.last().filter(|_| surface) {
                push(last, line.points[start]);
            }
        }
        Collider::new(edges, to_layer, surface)
    }

    /// A collider of `edges` from a layer whose own pixels `to_layer` carries
    /// into this layer's. The edges are walls in this layer's pixels, or a
    /// `surface` in the collider's own.
    #[must_use]
    pub fn new(edges: Vec<[[f32; 2]; 2]>, to_layer: Place, surface: bool) -> Self {
        // A layer scaled to nothing has nothing to hit.
        let (edges, from_layer) = match to_layer.invert() {
            Some(back) => (edges, back),
            None => (Vec::new(), to_layer),
        };
        Collider {
            grid: Grid::of(&edges),
            edges,
            to_layer,
            from_layer,
            surface,
        }
    }

    /// The floor at height `y` in comp pixels, seen from `owner`'s pixels. In
    /// its own pixels the floor is where z is nought, and z grows upwards.
    #[must_use]
    pub fn ground(
        comp: &Composition,
        owner: &Layer,
        t: f64,
        context: &ExpressionContext,
        y: f64,
    ) -> Self {
        let read = crate::expression::space::reader(context, comp.id, t);
        let world = crate::expression::space::space(&context.document, comp, owner, t, &read).world;
        let floor = Place {
            m: [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]],
            t: (0.0, y, 0.0),
        };
        let to_layer = match world.invert() {
            Some(back) => floor.within(&back),
            None => floor,
        };
        Collider::new(Vec::new(), to_layer, true)
    }

    /// Each edge's two ends: in this layer's pixels for walls, in the
    /// collider's own for a surface.
    #[must_use]
    pub fn edges(&self) -> &[[[f32; 2]; 2]] {
        &self.edges
    }

    /// The collider's own pixels to this layer's.
    #[must_use]
    pub fn to_layer(&self) -> &Place {
        &self.to_layer
    }

    /// This layer's pixels to the collider's own.
    #[must_use]
    pub fn from_layer(&self) -> &Place {
        &self.from_layer
    }

    /// Whether it is a surface, rather than walls.
    #[must_use]
    pub fn is_surface(&self) -> bool {
        self.surface
    }

    /// Where a particle moving from `from` to `to` at `vel` ends up.
    pub(super) fn hit(
        &self,
        contact: Contact,
        keep: f32,
        slip: f32,
        from: [f32; 3],
        to: [f32; 3],
        vel: [f32; 3],
    ) -> Hit {
        let (mut from, mut to, mut vel) = (from, to, vel);
        // Walls are across and down only, so depth carries on as it was.
        let slip = [slip, slip, if self.surface { slip } else { 1.0 }];
        for _ in 0..MAX_HITS {
            let Some((s, n)) = self.first(from, to) else {
                return Hit::Clear(to, vel);
            };
            let at: [f32; 3] = std::array::from_fn(|k| from[k] + (to[k] - from[k]) * s);
            let off: [f32; 3] = std::array::from_fn(|k| at[k] + n[k] * NUDGE);
            let keep = match contact {
                Contact::Kill => return Hit::Gone(off),
                Contact::Stick => return Hit::Held(off),
                Contact::Bounce => keep,
                Contact::Slide => 0.0,
            };
            // Along the surface it keeps all but the friction, and off it the
            // bounciness, turned round.
            let turn = |d: [f32; 3]| -> [f32; 3] {
                let dn = d[0] * n[0] + d[1] * n[1] + d[2] * n[2];
                std::array::from_fn(|k| (d[k] - dn * n[k]) * slip[k] - dn * keep * n[k])
            };
            vel = turn(vel);
            let rest = turn(std::array::from_fn(|k| to[k] - at[k]));
            from = off;
            to = std::array::from_fn(|k| off[k] + rest[k]);
        }
        // Still hitting things: it stops where it last came off.
        Hit::Clear(from, vel)
    }

    /// The first thing the move from `a` to `b` hits: how far along the move,
    /// and the normal on `a`'s side.
    fn first(&self, a: [f32; 3], b: [f32; 3]) -> Option<(f32, [f32; 3])> {
        if self.surface {
            return self.through(a, b);
        }
        let (a, b) = ([a[0], a[1]], [b[0], b[1]]);
        let cross = |p: [f32; 2], q: [f32; 2]| p[0] * q[1] - p[1] * q[0];
        let r = [b[0] - a[0], b[1] - a[1]];
        let mut best: Option<(f32, usize)> = None;
        self.grid.near(a, b, |i| {
            let [c, d] = self.edges[i];
            let w = [d[0] - c[0], d[1] - c[1]];
            let den = cross(r, w);
            if den.abs() < 1e-12 {
                return;
            }
            let qp = [c[0] - a[0], c[1] - a[1]];
            let s = cross(qp, w) / den;
            let u = cross(qp, r) / den;
            if (0.0..=1.0).contains(&s)
                && (0.0..=1.0).contains(&u)
                && best.is_none_or(|(t, j)| s < t || (s == t && i < j))
            {
                best = Some((s, i));
            }
        });
        let (s, i) = best?;
        let [c, d] = self.edges[i];
        let len = (d[0] - c[0]).hypot(d[1] - c[1]);
        let mut n = [-(d[1] - c[1]) / len, (d[0] - c[0]) / len];
        if (a[0] - c[0]) * n[0] + (a[1] - c[1]) * n[1] < 0.0 {
            n = [-n[0], -n[1]];
        }
        Some((s, [n[0], n[1], 0.0]))
    }

    /// Where the move from `a` to `b` passes through the surface's shapes:
    /// how far along the move, and the surface's normal on `a`'s side.
    fn through(&self, a: [f32; 3], b: [f32; 3]) -> Option<(f32, [f32; 3])> {
        // In the collider's own pixels its card is where z is nought.
        let (p, q) = (
            self.from_layer.point(wide(a)),
            self.from_layer.point(wide(b)),
        );
        if !((p.2 > 0.0 && q.2 <= 0.0) || (p.2 < 0.0 && q.2 >= 0.0)) {
            return None;
        }
        let s = p.2 / (p.2 - q.2);
        let at = [p.0 + (q.0 - p.0) * s, p.1 + (q.1 - p.1) * s];
        if !self.inside([at[0] as f32, at[1] as f32]) {
            return None;
        }
        // The way its own z grows, seen from this layer.
        let z = self.from_layer.m[2];
        let len = (z[0] * z[0] + z[1] * z[1] + z[2] * z[2]).sqrt() * p.2.signum();
        Some((s as f32, z.map(|v| (v / len) as f32)))
    }

    /// Whether `p`, in the collider's own pixels, is inside its shapes,
    /// even-odd.
    fn inside(&self, p: [f32; 2]) -> bool {
        if self.grid.span(p, p).is_none() {
            return false;
        }
        let mut odd = false;
        for [c, d] in &self.edges {
            if (c[1] > p[1]) != (d[1] > p[1])
                && p[0] < c[0] + (p[1] - c[1]) / (d[1] - c[1]) * (d[0] - c[0])
            {
                odd = !odd;
            }
        }
        odd
    }
}

pub(super) fn wide(p: [f32; 3]) -> (f64, f64, f64) {
    (f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
}

pub(super) fn narrow(p: (f64, f64, f64)) -> [f32; 3] {
    [p.0 as f32, p.1 as f32, p.2 as f32]
}

/// What a move came to.
pub(super) enum Hit {
    /// It got here, going this fast.
    Clear([f32; 3], [f32; 3]),
    /// It stuck here.
    Held([f32; 3]),
    /// It died here.
    Gone([f32; 3]),
}

/// The edges by where they are, so a particle only looks at the few near it.
#[derive(Debug, Clone, Default)]
struct Grid {
    origin: [f32; 2],
    cell: f32,
    size: [usize; 2],
    /// Where each cell's edges start in `items`, and one past the last cell.
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    /// About this many cells across the longer side.
    const CELLS: f32 = 64.0;

    fn of(edges: &[[[f32; 2]; 2]]) -> Self {
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        for p in edges.iter().flatten() {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if edges.is_empty() {
            return Grid::default();
        }
        let cell = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / Self::CELLS).max(4.0);
        let size = [
            ((hi[0] - lo[0]) / cell) as usize + 1,
            ((hi[1] - lo[1]) / cell) as usize + 1,
        ];
        let mut grid = Grid {
            origin: lo,
            cell,
            size,
            starts: vec![0; size[0] * size[1] + 1],
            items: Vec::new(),
        };
        let cells = |g: &Grid, e: &[[f32; 2]; 2]| g.span(e[0], e[1]);
        for e in edges {
            if let Some((x, y)) = cells(&grid, e) {
                for cy in y.clone() {
                    for cx in x.clone() {
                        grid.starts[cy * size[0] + cx + 1] += 1;
                    }
                }
            }
        }
        for i in 1..grid.starts.len() {
            grid.starts[i] += grid.starts[i - 1];
        }
        let mut fill = grid.starts.clone();
        grid.items = vec![0; grid.starts.last().copied().unwrap_or(0) as usize];
        for (i, e) in edges.iter().enumerate() {
            if let Some((x, y)) = cells(&grid, e) {
                for cy in y.clone() {
                    for cx in x.clone() {
                        let slot = &mut fill[cy * size[0] + cx];
                        grid.items[*slot as usize] = i as u32;
                        *slot += 1;
                    }
                }
            }
        }
        grid
    }

    /// The cells the box round `a` and `b` covers, or `None` off the grid.
    fn span(
        &self,
        a: [f32; 2],
        b: [f32; 2],
    ) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
        if self.size[0] == 0 {
            return None;
        }
        let axis = |k: usize| {
            let lo = (a[k].min(b[k]) - self.origin[k]) / self.cell;
            let hi = (a[k].max(b[k]) - self.origin[k]) / self.cell;
            let last = self.size[k] as f32 - 1.0;
            // NaN fails both tests, so a move that is not a number hits nothing.
            if !(hi >= 0.0 && lo <= last) {
                return None;
            }
            Some(lo.max(0.0) as usize..hi.min(last) as usize + 1)
        };
        Some((axis(0)?, axis(1)?))
    }

    /// Hands `f` each edge near the move from `a` to `b`, perhaps twice.
    fn near(&self, a: [f32; 2], b: [f32; 2], mut f: impl FnMut(usize)) {
        let Some((x, y)) = self.span(a, b) else {
            return;
        };
        for cy in y {
            for cx in x.clone() {
                let c = cy * self.size[0] + cx;
                for &i in &self.items[self.starts[c] as usize..self.starts[c + 1] as usize] {
                    f(i as usize);
                }
            }
        }
    }
}

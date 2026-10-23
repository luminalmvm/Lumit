//! Particulate with a Collide with layer, a flock or sparks: particles worked
//! out frame by frame.
//!
//! # In plain terms
//!
//! The closed forms in [`super`] put a particle where it is by arithmetic, and
//! that cannot know about a wall, a neighbour or a death. Once Collide with
//! names a layer, or the particles flock or throw sparks, Particulate walks
//! instead: each frame starts from the particles of the frame before,
//! moves them on by one frame of the same forces, and then looks at what they
//! hit on the way. A particle remembers where it was born and how it has been
//! knocked about, so it can bounce, slide, stick or die.
//!
//! A frame is defined as **the walk from the layer's in point**, and every step
//! reads its rows, its wires and the collider at that step's own time. Where
//! the walk got to is kept every [`KEEP_EVERY`] frames and at the last frame
//! drawn, so playing on is one step and scrubbing back is a few. Kept states
//! are only ever a shortcut: a frame reached from one is the frame walked from
//! the start, which is what lets an export match the Viewer.
//!
//! The walk runs on the CPU in px@comp, so it is the same whatever the preview
//! size. A collider facing the way the layer does is a wall along its outline
//! through every depth, hit across and down. One turned against it in 3D is a
//! surface, hit where a particle passes through its shapes.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use rayon::prelude::*;

use super::{
    attr, birth_point, burst_base, draw, integrate, jitter, launch, look_of, outline_polyline,
    turbulence, wrapped, Burst, Contact, Edge, PointsParams, PointsStream, BURST_ID, BURST_MAX,
};
use crate::camera::Place;
use crate::expression::ExpressionContext;
use crate::fx::effects::particulate::Particulate;
use crate::mask::MaskPolyline;
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};

/// The most particles a walk carries, whatever Max particles says. Each is
/// carried from frame to frame and kept with the frame, so this is lower than
/// the closed forms' ceiling.
pub const STEPPED_CAP: u32 = 100_000;

/// How often a walk keeps where it got to, in frames. Scrubbing back costs at
/// most one fewer steps than this.
const KEEP_EVERY: i64 = 8;

/// The most bytes kept walks hold between them, over every Particulate.
const KEPT_BYTES: usize = 256 << 20;

/// The most frames one walk takes. A layer's time carried further than this by
/// a retime starts its walk late, as the birth schedule's scan does.
const MAX_WALK_FRAMES: i64 = super::MAX_SCAN_FRAMES;

/// The most surfaces one particle comes off in one step.
const MAX_HITS: usize = 4;

/// How many particles a step moves before it shares them between threads.
const PARALLEL_FROM: usize = 4096;

/// How far off a surface a particle is left after hitting it, px, so its
/// next move does not start on the surface.
const NUDGE: f32 = 1e-3;

/// The top bit a spark's index carries, above the bursts' [`BURST_ID`].
/// Below it sit its parent's index, with the burst bit brought down, and its
/// place among the parent's sparks, so a spark is the same on every walk.
pub const SPARK_ID: u64 = 1 << 63;

/// How many bits of a spark's index count its place.
const SPARK_SHIFT: u32 = 6;

/// The most sparks one death throws out.
const SPARKS_MAX: u32 = 50;

/// Where a parent's burst bit sits in a spark's index, before the shift.
const SPARK_BURST: u64 = 1 << 56;

/// How quickly a flock at full strength steers, per second.
const STEER: f32 = 4.0;

/// The most others one particle steers by. Past it, an even spread of them.
const NEIGHBOURS: usize = 64;

/// A placement that moves nothing: a collider already in this layer's pixels.
pub const SAME: Place = Place {
    m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    t: (0.0, 0.0, 0.0),
};

thread_local! {
    /// What says the frame this thread is drawing is no longer wanted. The
    /// renderer holds the token, so it puts its own check here.
    static STOP: Cell<Option<fn() -> bool>> = const { Cell::new(None) };
}

/// Makes `check` this thread's stop check, and hands back the one before.
pub fn stop_when(check: Option<fn() -> bool>) -> Option<fn() -> bool> {
    STOP.with(|stop| stop.replace(check))
}

fn stopping() -> bool {
    STOP.with(|stop| stop.get().is_some_and(|check| check()))
}

/// One particle as the walk carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Particle {
    /// Its birth index, as the closed forms number it.
    id: u64,
    /// Where it was born, across and down: where turbulence's noise is read.
    born: [f32; 2],
    /// Where it is drawn.
    pos: [f32; 3],
    /// How fast it is going, before turbulence.
    vel: [f32; 3],
    /// Turbulence's push at its age, already in `pos`.
    drift: [f32; 3],
    age: f32,
    life: f32,
    /// Where it stuck, in the collider layer's own pixels.
    held: Option<[f32; 3]>,
}

/// Where a walk has got to at the start of a frame.
#[derive(Debug, Clone, Default, PartialEq)]
struct State {
    /// The frame, from the layer's in point.
    frame: i64,
    /// Oldest first, by birth index.
    particles: Vec<Particle>,
    /// The part of a birth the steady flow owes, as the scan carries it.
    carry: f64,
    /// Steady births so far: the next one's index.
    births: u64,
    /// Whether Trigger was over a half on this frame.
    high: bool,
}

impl State {
    fn bytes(&self) -> usize {
        self.particles.len() * std::mem::size_of::<Particle>() + std::mem::size_of::<Self>()
    }
}

/// What one step reads at one layer time.
pub struct Inputs {
    /// Particulate's rows at this time, in px@comp.
    pub rows: Particulate,
    /// The same, as the closed forms read them, with the layer's frame.
    params: PointsParams,
    /// The polyline a Mask path emitter walks, px@comp.
    path: MaskPolyline,
    /// What the particles hit, if anything.
    collider: Option<Collider>,
}

impl Inputs {
    /// `frame` is the layer's picture size, px@comp, for the Edge row.
    #[must_use]
    pub fn new(
        rows: Particulate,
        frame: [f32; 2],
        path: MaskPolyline,
        collider: Option<Collider>,
    ) -> Self {
        let mut params = rows.points().framed(frame[0], frame[1]);
        params.cap = params.cap.min(STEPPED_CAP);
        Inputs {
            rows,
            params,
            path,
            collider,
        }
    }
}

/// A collider's outline at one time, seen from the Particulate layer.
#[derive(Debug, Clone)]
pub struct Collider {
    /// Each edge's two ends: in this layer's pixels for walls, in the
    /// collider's own for a surface.
    edges: Vec<[[f32; 2]; 2]>,
    /// The collider layer's own pixels to this layer's, in 3D.
    to_layer: Place,
    /// And back.
    from_layer: Place,
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
        let surface = n[0].hypot(n[1]) > 1e-6 * n[2].abs();
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

    /// Where a particle moving from `from` to `to` at `vel` ends up.
    fn hit(
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

fn wide(p: [f32; 3]) -> (f64, f64, f64) {
    (f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
}

fn narrow(p: (f64, f64, f64)) -> [f32; 3] {
    [p.0 as f32, p.1 as f32, p.2 as f32]
}

/// What a move came to.
enum Hit {
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

/// Everything a walk reads, as one name: its kept states are found by it.
///
/// `layer` is the layer Particulate is on and `target` the one it collides
/// with, if any. Their tracks, masks, shapes, wires and parents are read
/// whole, so an edit at any time renames every frame. What an expression or a
/// tap reads from somewhere else is not, as the frame keys already treat it.
#[must_use]
pub fn definition(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    target: Option<&Layer>,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    let _ = serde_json::to_writer(&mut h, &inst.params);
    let _ = serde_json::to_writer(&mut h, &layer.graph);
    let _ = serde_json::to_writer(&mut h, &layer.masks);
    let _ = serde_json::to_writer(&mut h, &layer.kind);
    let _ = serde_json::to_writer(&mut h, &target);
    for one in std::iter::once(layer).chain(target) {
        // A solid's size is the frame Edge acts at, and the rectangle a
        // layer with no shapes is hit at.
        if let LayerKind::Solid { def } = one.kind {
            if let Some(solid) = doc.solid(def) {
                h.update(&solid.width.to_le_bytes());
                h.update(&solid.height.to_le_bytes());
            }
        }
        h.update(&[u8::from(one.switches.three_d)]);
        let _ = serde_json::to_writer(&mut h, &one.transform);
        let _ = serde_json::to_writer(&mut h, &one.start_offset);
        for id in crate::model::layer_parent_chain(comp, one.id) {
            if let Some(parent) = comp.layers.iter().find(|l| l.id == id) {
                // A parent is placed in 3D by its switch, and a camera or a
                // light by its aim too.
                h.update(&[u8::from(parent.switches.three_d)]);
                if matches!(
                    parent.kind,
                    LayerKind::Camera { .. } | LayerKind::Light { .. }
                ) {
                    let _ = serde_json::to_writer(&mut h, &parent.kind);
                }
                let _ = serde_json::to_writer(&mut h, &parent.transform);
                let _ = serde_json::to_writer(&mut h, &parent.start_offset);
            }
        }
    }
    let _ = serde_json::to_writer(&mut h, &comp.frame_rate);
    *h.finalize().as_bytes()
}

/// The layer `inst` collides with in `comp`, if its Collide with row names
/// one that is there.
#[must_use]
pub fn target<'a>(comp: &'a Composition, inst: &EffectInstance) -> Option<&'a Layer> {
    let id = inst.layer_ref("collide_with")?;
    comp.layers.iter().find(|l| l.id == id)
}

/// The rows beside Collide with that walk once above nought.
const WALK_ROWS: [&str; 4] = ["separation", "alignment", "cohesion", "sparks"];

/// Whether Particulate `inst` walks in `comp`: Collide with names a layer
/// there, or a flock or sparks row is above nought. A keyed, expression or
/// wired row counts as above nought, as it may be at some time.
#[must_use]
pub fn wanted(comp: &Composition, inst: &EffectInstance) -> bool {
    inst.effect.match_name == "particulate"
        && (target(comp, inst).is_some() || WALK_ROWS.iter().any(|id| above(comp, inst, id)))
}

/// Whether Particulate `inst` keeps a picture between frames: Persistence is
/// above nought, at some time. It does not make the particles walk.
#[must_use]
pub fn trails(comp: &Composition, inst: &EffectInstance) -> bool {
    inst.effect.match_name == "particulate" && above(comp, inst, "persistence")
}

/// Whether row `id` is above nought, or keyed, an expression or wired, as it
/// may be at some time.
fn above(comp: &Composition, inst: &EffectInstance, id: &str) -> bool {
    use crate::anim::Animation;
    use crate::graph::{InputRef, NodeRef};
    let on = match inst.param(id) {
        Some(EffectValue::Float(track)) => {
            !matches!(track.animation, Animation::Static(v) if v <= 0.0)
        }
        _ => false,
    };
    on || comp
        .layers
        .iter()
        .find(|l| l.effects.iter().any(|e| e.id == inst.id))
        .is_some_and(|l| {
            l.graph.edges.iter().any(|e| {
                matches!(&e.to, InputRef::Param { node: NodeRef::Effect(n), port }
                    if *n == inst.id && port == id)
            })
        })
}

/// The particles at layer time `t`, walked from the layer's in point a frame
/// of `dt` seconds at a time. `at` answers the inputs at any layer time, and
/// `key` is the walk's [`definition`]. `None` when an input cannot be read,
/// or the frame is no longer wanted.
///
/// A time between frames is the frame before, moved on by the part of a
/// frame it is past it, so motion blur sees the particles move.
pub fn stream_at(
    key: [u8; 32],
    t: f64,
    dt: f64,
    at: &dyn Fn(f64) -> Option<Inputs>,
) -> Option<PointsStream> {
    if !(t >= 0.0 && dt.is_finite() && dt > 0.0) {
        return Some(PointsStream::default());
    }
    let target = (t / dt).floor() as i64;
    let from = target.saturating_sub(MAX_WALK_FRAMES).max(0);
    // A walk that started late is not the walk from the in point.
    let keeps = from == 0;
    let kept = keeps.then(|| kept(&key, target)).flatten();
    let mut now = at(kept.as_ref().map_or(from, |s| s.frame) as f64 * dt)?;
    let mut state = match kept {
        Some(state) => (*state).clone(),
        None => {
            let mut state = State {
                frame: from,
                ..State::default()
            };
            bursts(&mut state, &now);
            state
        }
    };
    let start = state.frame;
    while state.frame < target {
        // A frame nobody wants any more stops here, and keeps where it got
        // to for the next.
        if stopping() {
            break;
        }
        let then = at((state.frame + 1) as f64 * dt)?;
        step(&mut state, &now, &then, dt, dt);
        if keeps && state.frame % KEEP_EVERY == 0 {
            keep(key, &state, true);
        }
        now = then;
    }
    if keeps && state.frame > start && state.frame % KEEP_EVERY != 0 {
        keep(key, &state, false);
    }
    if state.frame < target {
        return None;
    }
    let h = t - target as f64 * dt;
    if h > 1e-9 {
        let then = at(t)?;
        step(&mut state, &now, &then, h, dt);
        return Some(stream_of(&state, &then, dt));
    }
    Some(stream_of(&state, &now, dt))
}

/// Moves `s` on by `h` seconds: a whole frame of `dt`, or the part of one a
/// time between frames asks for. `now` is read at the step's start and
/// `then` at its end.
fn step(s: &mut State, now: &Inputs, then: &Inputs, h: f64, dt: f64) {
    let whole = h >= dt;
    let p = &now.params;
    let hf = h as f32;
    let steer = flock(&s.particles, &now.rows);
    // A particle that dies hands back its id and where, for its sparks.
    let advance = |(i, mut q): (usize, Particle)| -> Result<Particle, (u64, [f32; 3])> {
        let age = q.age + hf;
        if age >= q.life {
            return Err((q.id, q.pos));
        }
        q.age = age;
        if let Some(local) = q.held {
            if let Some(c) = &then.collider {
                q.pos = narrow(c.to_layer.point(wide(local)));
            }
            return Ok(q);
        }
        if let Some(a) = steer.as_ref().and_then(|all| all.get(i)) {
            q.vel = std::array::from_fn(|k| q.vel[k] + a[k] * hf);
        }
        let base = [
            q.pos[0] - q.drift[0],
            q.pos[1] - q.drift[1],
            q.pos[2] - q.drift[2],
        ];
        let (next, vel) = integrate(base, q.vel, &p.forces, hf);
        let phase = draw(p.seed, q.id, attr::TURB_PHASE) * 1000.0;
        let drift = turbulence([q.born[0], q.born[1], 0.0], phase, &p.forces, p.seed, age);
        let to = [next[0] + drift[0], next[1] + drift[1], next[2] + drift[2]];
        q.drift = drift;
        settle(q, q.pos, to, vel, then).map_err(|at| (q.id, at))
    };
    let all = std::mem::take(&mut s.particles);
    // A few hundred particles are quicker on this thread than handed round.
    let moved: Vec<_> = if all.len() < PARALLEL_FROM {
        all.into_iter().enumerate().map(advance).collect()
    } else {
        all.into_par_iter().enumerate().map(advance).collect()
    };
    let mut deaths = Vec::new();
    s.particles.reserve(moved.len());
    for m in moved {
        match m {
            Ok(q) => s.particles.push(q),
            Err(death) => deaths.push(death),
        }
    }

    // The steady flow's births in this frame, spread through it as the
    // schedule spreads them, each moved on from its own moment.
    let rate = f64::from(now.rows.emit_rate);
    let rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
    let carry = s.carry + rate * dt;
    let n = carry.floor().clamp(0.0, f64::from(u32::MAX)) as u32;
    let outline = outline_polyline(&p.emitter);
    let walk = if p.emitter.shape.is_outline() {
        &outline
    } else {
        &now.path
    };
    // Only the newest Max particles could survive the cap.
    for j in n.saturating_sub(p.cap)..n {
        let off = (f64::from(j) + 0.5) * dt / f64::from(n);
        if off >= h {
            break;
        }
        let Some((q, start)) = born(p, walk, s.births + u64::from(j), (h - off) as f32) else {
            continue;
        };
        match settle(q, start, q.pos, q.vel, then) {
            Ok(q) => s.particles.push(q),
            Err(at) => deaths.push((q.id, at)),
        }
    }
    sparks(s, &mut deaths, then);
    if whole {
        s.carry = carry - f64::from(n);
        s.births += u64::from(n);
        s.frame += 1;
        bursts(s, then);
    }
    // Bursts and sparks number above the steady flow, so births can land out
    // of order.
    s.particles.sort_unstable_by_key(|q| q.id);
    let cap = then.params.cap as usize;
    if s.particles.len() > cap {
        s.particles.drain(..s.particles.len() - cap);
    }
}

/// The burst on `s`'s frame, if one goes off, read at that frame.
fn bursts(s: &mut State, now: &Inputs) {
    let rows = &now.rows;
    let f = s.frame;
    let high = rows.trigger > 0.5;
    let fires = match Burst::from_code(rows.burst) {
        Burst::Off => false,
        Burst::Once => f == 0,
        Burst::Every => {
            let n = rows.burst_interval;
            let n = if n.is_finite() {
                n.round().max(1.0) as i64
            } else {
                1
            };
            f % n == 0
        }
        Burst::Trigger => high && !s.high,
    };
    s.high = high;
    if !fires {
        return;
    }
    let n = rows.burst_count;
    let n = if n.is_finite() {
        n.round().clamp(0.0, BURST_MAX as f32) as u32
    } else {
        0
    };
    let p = &now.params;
    let outline = outline_polyline(&p.emitter);
    let walk = if p.emitter.shape.is_outline() {
        &outline
    } else {
        &now.path
    };
    for j in n.saturating_sub(p.cap)..n {
        if let Some((q, _)) = born(p, walk, burst_base(f) + u64::from(j), 0.0) {
            s.particles.push(q);
        }
    }
}

/// Spark `j` of the particle `parent`.
fn spark_id(parent: u64, j: u32) -> u64 {
    let burst = if parent & BURST_ID != 0 {
        SPARK_BURST
    } else {
        0
    };
    SPARK_ID | ((burst | (parent & (SPARK_BURST - 1))) << SPARK_SHIFT) | u64::from(j)
}

/// The particle spark `id` came from.
fn parent_of(id: u64) -> u64 {
    let field = (id & !SPARK_ID) >> SPARK_SHIFT;
    let burst = if field & SPARK_BURST != 0 {
        BURST_ID
    } else {
        0
    };
    burst | (field & (SPARK_BURST - 1))
}

/// The sparks of the particles in `deaths` (id and where), read at the end
/// of the step. Only the newest Max particles of them could survive the cap.
fn sparks(s: &mut State, deaths: &mut [(u64, [f32; 3])], now: &Inputs) {
    let rows = &now.rows;
    let n = if rows.sparks.is_finite() {
        rows.sparks.round().clamp(0.0, SPARKS_MAX as f32) as u32
    } else {
        0
    };
    let life = rows.spark_life;
    if n == 0 || life.is_nan() || life <= 0.0 {
        return;
    }
    let speed = if rows.spark_speed.is_finite() {
        rows.spark_speed.max(0.0)
    } else {
        0.0
    };
    let p = &now.params;
    // Newest first, as a spark's index follows its parent's.
    deaths.sort_unstable_by_key(|d| std::cmp::Reverse(d.0));
    let mut room = p.cap as usize;
    for &(parent, at) in deaths.iter().filter(|d| d.0 & SPARK_ID == 0) {
        for j in (0..n).rev() {
            if room == 0 {
                return;
            }
            room -= 1;
            let id = spark_id(parent, j);
            // Evenly over a sphere.
            let z = 2.0 * draw(p.seed, id, attr::DIRECTION_Z) - 1.0;
            let (sa, ca) = (draw(p.seed, id, attr::DIRECTION) * std::f32::consts::TAU).sin_cos();
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phase = draw(p.seed, id, attr::TURB_PHASE) * 1000.0;
            s.particles.push(Particle {
                id,
                born: [at[0], at[1]],
                pos: at,
                vel: [speed * r * ca, speed * r * sa, speed * z],
                drift: turbulence([at[0], at[1], 0.0], phase, &p.forces, p.seed, 0.0),
                age: 0.0,
                life,
                held: None,
            });
        }
    }
}

/// How each particle steers by its neighbours, px/s², read from where they
/// all are before any of them moves, so the order of work changes nothing.
/// `None` with the three strengths at nought.
fn flock(particles: &[Particle], rows: &Particulate) -> Option<Vec<[f32; 3]>> {
    let k = |v: f32| {
        if v.is_finite() {
            (v / 100.0).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let (apart, along, together) = (k(rows.separation), k(rows.alignment), k(rows.cohesion));
    let r = rows.flock_radius;
    if apart + along + together <= 0.0 || !(r > 0.0 && r.is_finite()) {
        return None;
    }
    // The free particles by the cell they are in, a radius on a side.
    let cell = |p: [f32; 3]| p.map(|v| (v / r).floor() as i64);
    let mut cells: Vec<([i64; 3], u32)> = particles
        .iter()
        .enumerate()
        .filter(|(_, q)| q.held.is_none())
        .map(|(i, q)| (cell(q.pos), i as u32))
        .collect();
    cells.sort_unstable();
    let steer = |(i, q): (usize, &Particle)| -> [f32; 3] {
        if q.held.is_some() {
            return [0.0; 3];
        }
        let c = cell(q.pos);
        let mut near = [(0, 0); 27];
        let mut total = 0;
        for (slot, ends) in near.iter_mut().enumerate() {
            let d = [slot % 3, slot / 3 % 3, slot / 9].map(|v| v as i64 - 1);
            let key: [i64; 3] = std::array::from_fn(|a| c[a].saturating_add(d[a]));
            let from = cells.partition_point(|e| e.0 < key);
            let to = from + cells[from..].partition_point(|e| e.0 == key);
            *ends = (from, to);
            total += to - from;
        }
        let every = total.div_ceil(NEIGHBOURS).max(1);
        let (mut push, mut vel, mut mid, mut n) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3], 0.0f32);
        for at in near.iter().flat_map(|&(a, b)| a..b).step_by(every) {
            let j = cells[at].1 as usize;
            let o = &particles[j];
            let d: [f32; 3] = std::array::from_fn(|a| q.pos[a] - o.pos[a]);
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if j == i || len.is_nan() || len >= r {
                continue;
            }
            if len > 0.0 {
                let w = (1.0 - len / r) / len;
                push = std::array::from_fn(|a| push[a] + d[a] * w);
            }
            vel = std::array::from_fn(|a| vel[a] + o.vel[a]);
            mid = std::array::from_fn(|a| mid[a] + o.pos[a]);
            n += 1.0;
        }
        if n == 0.0 {
            return [0.0; 3];
        }
        // Away from close ones, towards their speed and towards their middle.
        // A spread of the others stands for all of them, so the push counts
        // each one it looked at for those it passed over.
        let away = every as f32 * apart * STEER * STEER * r;
        std::array::from_fn(|a| {
            away * push[a]
                + along * STEER * (vel[a] / n - q.vel[a])
                + together * STEER * STEER * (mid[a] / n - q.pos[a])
        })
    };
    Some(if particles.len() < PARALLEL_FROM {
        particles.iter().enumerate().map(steer).collect()
    } else {
        particles.par_iter().enumerate().map(steer).collect()
    })
}

/// Particle `b`, born under `p` and moved on `age` seconds by the closed
/// form, and where it was drawn at birth. `None` when it has already died or
/// there is nowhere to be born.
fn born(p: &PointsParams, walk: &MaskPolyline, b: u64, age: f32) -> Option<(Particle, [f32; 3])> {
    let life = jitter(
        p.particle.life,
        p.particle.life_jitter,
        draw(p.seed, b, attr::LIFE),
    );
    if life <= 0.0 || age >= life {
        return None;
    }
    let p0 = birth_point(
        &p.emitter,
        walk,
        draw(p.seed, b, attr::EMIT_U),
        draw(p.seed, b, attr::EMIT_V),
        draw(p.seed, b, attr::EMIT_W),
    )?;
    let v0 = launch(p, b);
    let phase = draw(p.seed, b, attr::TURB_PHASE) * 1000.0;
    let (pos, vel) = integrate(p0, v0, &p.forces, age);
    let drift = turbulence(p0, phase, &p.forces, p.seed, age);
    let first = turbulence(p0, phase, &p.forces, p.seed, 0.0);
    let q = Particle {
        id: b,
        born: [p0[0], p0[1]],
        pos: [pos[0] + drift[0], pos[1] + drift[1], pos[2] + drift[2]],
        vel,
        drift,
        age,
        life,
        held: None,
    };
    Some((q, [p0[0] + first[0], p0[1] + first[1], p0[2] + first[2]]))
}

/// `q` moved from `from` to `to` at `vel`, after what it hit on the way and
/// the frame's edge. Where it died when it did.
fn settle(
    mut q: Particle,
    from: [f32; 3],
    to: [f32; 3],
    vel: [f32; 3],
    then: &Inputs,
) -> Result<Particle, [f32; 3]> {
    let rows = &then.rows;
    let contact = Contact::from_code(rows.on_contact);
    let keep = (rows.bounciness / 100.0).clamp(0.0, 1.0);
    let slip = 1.0 - (rows.friction / 100.0).clamp(0.0, 1.0);
    (q.pos, q.vel) = (to, vel);
    if let Some(c) = &then.collider {
        match c.hit(contact, keep, slip, from, to, vel) {
            Hit::Gone(at) => return Err(at),
            Hit::Held(at) => {
                q.pos = at;
                q.vel = [0.0; 3];
                q.held = Some(narrow(c.from_layer.point(wide(at))));
            }
            Hit::Clear(at, v) => {
                q.pos = at;
                q.vel = v;
            }
        }
    }
    let p = &then.params;
    if let Some(bx) = p.edge_box() {
        let out = q.pos[0] < bx[0] || q.pos[0] > bx[2] || q.pos[1] < bx[1] || q.pos[1] > bx[3];
        match p.edge {
            Edge::Kill if out => return Err(q.pos),
            Edge::Wrap => q.pos = wrapped(bx, q.pos),
            _ => {}
        }
    }
    Ok(q)
}

/// The stream `s` draws, looked at with `now`'s rows.
fn stream_of(s: &State, now: &Inputs, dt: f64) -> PointsStream {
    let p = &now.params;
    let eps = (dt as f32 * 0.5).max(1e-6);
    let small = (now.rows.spark_size / 100.0).max(0.0);
    let mut out = PointsStream::default();
    for q in &s.particles {
        let speed = if q.held.is_some() {
            [0.0; 3]
        } else {
            // Turbulence's own rate of change, as the closed forms add it.
            let born = [q.born[0], q.born[1], 0.0];
            let phase = draw(p.seed, q.id, attr::TURB_PHASE) * 1000.0;
            let next = turbulence(born, phase, &p.forces, p.seed, q.age + eps);
            let prev = turbulence(born, phase, &p.forces, p.seed, q.age - eps);
            [
                q.vel[0] + (next[0] - prev[0]) / (2.0 * eps),
                q.vel[1] + (next[1] - prev[1]) / (2.0 * eps),
                q.vel[2] + (next[2] - prev[2]) / (2.0 * eps),
            ]
        };
        // A spark looks like the particle it came from, at its own age, and
        // smaller.
        let (look, scale) = if q.id & SPARK_ID != 0 {
            (parent_of(q.id), small)
        } else {
            (q.id, 1.0)
        };
        let (size, colour, rotation) = look_of(p, look, q.age, q.life, speed);
        out.position.push(q.pos);
        out.speed.push(speed);
        out.age.push(q.age);
        out.life.push(q.life);
        out.size.push(size * scale);
        out.rotation.push(rotation);
        out.colour.push(colour);
        out.id.push(q.id);
    }
    out
}

/// A walk kept for the frames after it.
struct Kept {
    key: [u8; 32],
    /// Kept for good, as against the latest frame drawn, which the next
    /// frame drawn replaces.
    sticky: bool,
    state: Arc<State>,
}

/// Kept walks, the latest used first. Only ever a shortcut.
static KEPT: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// The furthest kept state of `key` at or before `frame`.
fn kept(key: &[u8; 32], frame: i64) -> Option<Arc<State>> {
    let mut all = KEPT.lock().ok()?;
    let found = all
        .iter()
        .enumerate()
        .filter(|(_, k)| k.key == *key && k.state.frame <= frame)
        .max_by_key(|(_, k)| k.state.frame)
        .map(|(n, _)| n)?;
    let k = all.remove(found);
    let state = Arc::clone(&k.state);
    all.insert(0, k);
    Some(state)
}

/// Keep `state` under `key`, and let go of the states used longest ago once
/// they hold too much.
fn keep(key: [u8; 32], state: &State, sticky: bool) {
    let Ok(mut all) = KEPT.lock() else {
        return;
    };
    all.retain(|k| !(k.key == key && (k.state.frame == state.frame || (!sticky && !k.sticky))));
    all.insert(
        0,
        Kept {
            key,
            sticky,
            state: Arc::new(state.clone()),
        },
    );
    let mut bytes = 0;
    all.retain(|k| {
        bytes += k.state.bytes();
        bytes <= KEPT_BYTES
    });
}

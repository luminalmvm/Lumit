//! Connect points: lines drawn between the points of a stream that are near
//! enough to each other — the plexus look.
//!
//! **In plain terms.** Wire a producer's teal Points socket into this effect and
//! every pair of points closer together than **Max distance** is joined by a
//! line. Particles drifting past each other web up and let go again; a Grid
//! becomes a mesh; a Scatter inside a silhouette becomes the constellation
//! everybody makes by hand out of a plugin they had to go and buy.
//!
//! Mode chooses the pairs instead: each point to the next, a mesh of
//! triangles, or only the lines round the outside. Max distance still drops a
//! line that is too long. Nearest joins each point to its closest few however
//! far off they are. Between keeps the lines to the picked points, or to the
//! ones that run from a picked point to one that is not.
//!
//! **A line is a capsule, and a capsule is a disc that has been stretched.**
//! Nothing new is drawn here: the shared points draw already runs a dab from a
//! head to a tail, so a segment is one entry in an ordinary stream whose
//! tail is somewhere other than its head. Three effects and one rasteriser,
//! still.
//!
//! **The pairing is deterministic and it is not a full comparison.** Naively
//! every point asks every other point how far away it is, which is
//! `n²/2` questions — a hundred thousand at a thousand points, and a hundred
//! million at twenty thousand, per frame. Instead the projected plane is cut
//! into squares of one Max distance and a point only asks the nine squares
//! around it, which is the whole of what can be within reach. Points are walked
//! in `id` order and their candidates ordered by distance with `id` breaking
//! every tie, so the same document draws the same web on every machine and from
//! any scrub direction.
//!
//! **Nothing wired draws nothing** — the picture passes through, and the box
//! wears the family's "no stream" mark.

use std::collections::HashMap;

use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    Port, PortType, ResolveCx, ShortText, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input (points-stream.md §4.1).
pub const POINTS_PORT: &str = "points";

/// What this effect consumes. Not `three_d`: a web is drawn on the layer's own
/// flat picture, and "near enough to join" is a nearness *in that picture* —
/// the same reading, and for the same reason, that makes Points sample's
/// Nearest distance a distance on the frame.
const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// Which pairs are joined, what the line between them looks like, and the
/// budget under the family's own heading.
pub const CONNECT_GROUPS: &[ParamGroup] = &[
    group(
        "Connections",
        &[
            "mode",
            "order_name",
            "between",
            "between_from",
            "between_to",
            "max_distance",
            "max_links",
            "closed",
            "depth",
            "taper",
            "fade",
        ],
    ),
    group("Line", &["width", "feather", "colour"]),
    group("Point", &["max_points"]),
];

const fn grey(param: &'static str, cond: EnabledCond) -> EnabledWhen {
    EnabledWhen {
        param,
        on: "mode",
        cond,
    }
}

/// Max connections is read by Nearby and Nearest, and the other modes name
/// their pairs. Nearest has no Max distance, and only In order can close.
pub const CONNECT_ENABLED_WHEN: &[EnabledWhen] = &[
    grey("max_links", EnabledCond::ChoiceIsNot(1)),
    grey("max_links", EnabledCond::ChoiceIsNot(2)),
    grey("max_links", EnabledCond::ChoiceIsNot(3)),
    grey("max_distance", EnabledCond::ChoiceIsNot(4)),
    grey("closed", EnabledCond::ChoiceIs(1)),
    grey("order_name", EnabledCond::ChoiceIs(1)),
    // The first group is read by both of Between's choices, the second only
    // by the one that joins two.
    EnabledWhen {
        param: "between_from",
        on: "between",
        cond: EnabledCond::ChoiceIsNot(0),
    },
    EnabledWhen {
        param: "between_to",
        on: "between",
        cond: EnabledCond::ChoiceIs(2),
    },
];

/// Which pairs of points are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectMode {
    /// Every pair within Max distance, up to Max connections at each point.
    #[default]
    Nearby,
    /// Each point to the next one, in the order of the number each carries,
    /// which is its place in the stream until something above sets it.
    InOrder,
    /// The edges of a triangulation of the points.
    Triangles,
    /// Only the edges round the outside of them.
    Outline,
    /// Each point to its Max connections nearest, however far off they are.
    Nearest,
}

impl ConnectMode {
    /// The Choice option labels, in code order. A Choice is stored as its
    /// index, so a new mode goes on the end.
    pub const OPTIONS: &'static [&'static str] =
        &["Nearby", "In order", "Triangles", "Outline", "Nearest"];

    /// The mode for a stored Choice index. Anything unknown is Nearby.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => ConnectMode::InOrder,
            2 => ConnectMode::Triangles,
            3 => ConnectMode::Outline,
            4 => ConnectMode::Nearest,
            _ => ConnectMode::Nearby,
        }
    }
}

/// Which points a line may run between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Between {
    /// Any two.
    #[default]
    All,
    /// Two picked points. The rest are left out altogether.
    Picked,
    /// A picked point and one that is not, so two sets join each other and
    /// neither joins itself.
    PickedAndRest,
}

impl Between {
    /// The Choice option labels, in code order.
    pub const OPTIONS: &'static [&'static str] = &["All points", "Picked", "Picked and the rest"];

    /// The rule for a stored Choice index. Anything unknown is All.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => Between::Picked,
            2 => Between::PickedAndRest,
            _ => Between::All,
        }
    }
}

/// Connect points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "connect_points",
    label = "Connect points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be anywhere, and a line reaches from one to another.
    roi = FullFrame,
    premultiplied = true,
    // Not seeded: nothing here is a function of time under constant parameters.
    // The producer's stream may well be, and that is the producer's own
    // declaration.
    seeded = false,
    groups = CONNECT_GROUPS,
    enabled_when = CONNECT_ENABLED_WHEN,
)]
pub struct ConnectPoints {
    /// Which pairs are joined. Max distance drops a line that is too long in
    /// every mode but Nearest.
    #[choice(label = "Mode", options = *ConnectMode::OPTIONS, default = 0)]
    pub mode: u32,

    /// What In order sorts the points by: a name written above, or one of
    /// the `@` names. Empty is the number each point carries.
    #[text(label = "Order by", default = "")]
    pub order_name: ShortText,

    /// Which points a line may run between, by what a Pick points above
    /// picked.
    #[choice(label = "Between", options = *Between::OPTIONS, default = 0)]
    pub between: u32,

    /// The group Picked reads, and the first of the two Picked and the rest
    /// joins: a name written above, or one of the `@` names. Empty is the
    /// picked points.
    #[text(label = "From group", default = "")]
    pub between_from: ShortText,

    /// The second of the two groups Picked and the rest joins. Empty is the
    /// points that are not picked. The same name in both joins that group
    /// to itself.
    #[text(label = "To group", default = "")]
    pub between_to: ShortText,

    /// How far apart two points may be and still be joined, px@comp measured on
    /// the frame. **Nought joins nothing**, which is the documented no-op.
    #[slider(
        label = "Max distance",
        min = 0.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub max_distance: f32,

    /// The most lines that may meet at any one point. A pair is joined only
    /// when **both** ends still have room, so the dial means what it says at
    /// every point rather than only at the one being walked. In Nearest it is
    /// how many neighbours each point reaches for.
    #[counter(
        label = "Max connections",
        min = 0,
        max = 32,
        default = 4,
        hard_min = 0,
        hard_max = 64,
        unit = Raw
    )]
    pub max_links: i32,

    /// Join the last point of In order back to the first.
    #[toggle(label = "Closed", default = false)]
    pub closed: bool,

    /// Measure Max distance between the points themselves, in all three axes,
    /// rather than between where the camera puts them on the frame.
    #[toggle(label = "Depth", default = false)]
    pub depth: bool,

    /// How much a line thins out as it lengthens, per cent: at 0 every line is
    /// the same Width, at 100 a line at exactly Max distance has no width left.
    /// Nearest runs Taper and Fade from its shortest line to its longest.
    #[slider(
        label = "Taper",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub taper: f32,

    /// How much a line fades as it lengthens, per cent: at 100 a line at
    /// exactly Max distance is invisible, so the web comes and goes instead of
    /// switching on. The default, because a plexus that pops is the one thing
    /// everybody has to go and fix.
    #[slider(
        label = "Fade",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub fade: f32,

    /// The thickness of a line, px@comp.
    #[slider(
        label = "Width",
        min = 0.0,
        max = 100.0,
        default = 2.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub width: f32,

    /// How soft a line's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// Multiplies the colour a line inherits — which is the **mean of its two
    /// ends' own colours**, so a producer's Colour over life still reads along
    /// the web. White leaves that alone, which is why it is the default.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// **The budget dial**, the family's row: the most **points** that
    /// may be considered. A stream longer than this is trimmed to its newest by
    /// birth index — the producer's own cap rule applied a second time — which
    /// is what bounds the pairing as well as the drawing. Not animatable: it is
    /// a capacity declaration.
    #[counter(
        label = "Max points",
        min = 1,
        max = 200_000,
        default = 2_000,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_points: i32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl ConnectPoints {
    /// The raster factor, for the one input the declaration cannot scale: a
    /// stream read off a wire is in px@comp and has to be rearranged into the
    /// pixels the frame is drawn at.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The web, as a stream of segments: one entry per line, its position the
    /// nearer end and its tail the further one.
    ///
    /// **Every decision is here**, in one expression both render paths read, so
    /// the CPU oracle and the instanced draw cannot come to join different
    /// pairs. `in_stream` is in the units the caller wants out — px@comp for a
    /// reader, raster pixels for a draw ([`PointsStream::rescaled`]) — and so
    /// are Max distance and Width, which travel through the bag's own rescale.
    ///
    /// The pairs are found through [`buckets`](Self::buckets); what is left
    /// here is the rule about which of them survive, which is deliberately one
    /// walk in one order:
    ///
    /// - points in `id` order — a fact of the evaluation, never of scheduling;
    /// - each point's candidates by distance, `id` breaking every tie;
    /// - a pair joined only while **both** ends are below Max connections.
    #[must_use]
    pub fn links(self, in_stream: &PointsStream) -> (PointsStream, Vec<[f32; 3]>) {
        let mut points = in_stream.clone();
        let between = Between::from_code(self.between);
        // Picked alone is the same web drawn over the picked points only.
        let (from, to) = (self.between_from.as_str(), self.between_to.as_str());
        if between == Between::Picked {
            points.retain(|i| in_stream.in_group(from, 0.5, i));
        }
        // The newest by birth index, which is the cap rule the whole family
        // applies — and here it is the ceiling on the pairing as much as on
        // the drawing.
        points.keep_newest(self.max_points.clamp(0, points::CAP_HARD as i32) as usize);
        let mut out = PointsStream {
            projection: points.projection,
            ..PointsStream::default()
        };
        let mut tails: Vec<[f32; 3]> = Vec::new();
        let reach = self.max_distance.max(0.0);
        let mode = ConnectMode::from_code(self.mode);
        let links = self.max_links.clamp(0, 64) as u32;
        let n = points.len();
        let nearest = mode == ConnectMode::Nearest;
        let counted = nearest || mode == ConnectMode::Nearby;
        if n < 2 || (reach <= 0.0 && !nearest) || (counted && links == 0) {
            return (out, tails);
        }
        let across = between == Between::PickedAndRest;
        // One end in the first group and the other in the second, either
        // way round. With no names those are the picked points and the rest.
        let first = |i: usize| points.in_group(from, 0.5, i);
        let second = |i: usize| match to {
            "" => !points.picked(i),
            _ => points.in_group(to, 0.5, i),
        };
        let allowed =
            |i: usize, j: usize| !across || (first(i) && second(j)) || (first(j) && second(i));
        // Where each point is *seen*, which is where "near enough" is judged.
        // On a 2D layer this is the pair the stream already holds.
        let seen: Vec<[f32; 2]> = (0..n).map(|i| points.projected(i)).collect();
        // With Depth on, nearness is between the points themselves. Two points
        // within reach in space are within reach across the layer's plane too,
        // so that plane is what the squares are cut from.
        let flat: Vec<[f32; 2]> = if self.depth {
            points.position.iter().map(|p| [p[0], p[1]]).collect()
        } else {
            seen.clone()
        };
        let apart = |i: usize, j: usize| {
            let (dx, dy) = (flat[j][0] - flat[i][0], flat[j][1] - flat[i][1]);
            let dz = if self.depth {
                points.position[j][2] - points.position[i][2]
            } else {
                0.0
            };
            (dx * dx + dy * dy + dz * dz).sqrt()
        };

        let taper = (self.taper / 100.0).clamp(0.0, 1.0);
        let fade = (self.fade / 100.0).clamp(0.0, 1.0);
        let width = self.width.max(0.0);
        let a = self.colour[3];
        let tint = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        // One line from point `i` to point `j`, `d` apart, where `span` is
        // the length that reads as fully faded.
        let mut join = |i: usize, j: usize, d: f32, span: f32| {
            // How far along its own reach this line is: 0 for two points on
            // top of each other, 1 at exactly Max distance.
            let u = if span > 0.0 {
                (d / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dim = 1.0 - fade * u;
            let mut colour = [0.0f32; 4];
            for (c, k) in colour.iter_mut().zip(0..4) {
                let mean = 0.5 * (points.colour[i][k] + points.colour[j][k]);
                *c = mean * tint[k] * dim;
            }
            out.position.push(points.position[i]);
            tails.push(points.position[j]);
            out.speed.push(points.speed[i]);
            out.age.push(points.age[i]);
            out.life.push(points.life[i]);
            out.size.push(width * (1.0 - taper * u));
            out.rotation.push(0.0);
            out.colour.push(colour);
            // The segment's own index, ascending, which is the order the
            // dabs go down in and so the order they cover each other in.
            out.id.push(out.id.len() as u64);
        };

        if mode != ConnectMode::Nearby {
            // The other modes name their pairs outright. A pair further apart
            // than Max distance is still not joined.
            let mut pairs = match mode {
                ConnectMode::InOrder => {
                    // By the number each point carries. The sort keeps the
                    // stream's own order where two carry the same.
                    let mut order: Vec<usize> = (0..n).collect();
                    let by = |i: usize| points.value_of(self.order_name.as_str(), i);
                    order.sort_by(|a, b| by(*a).total_cmp(&by(*b)));
                    let mut pairs: Vec<(usize, usize)> =
                        order.windows(2).map(|w| (w[0], w[1])).collect();
                    // Two points are already joined, so only three or more
                    // have a gap left to close.
                    let ends = order.first().zip(order.last());
                    let ends = ends.filter(|_| self.closed && n > 2);
                    pairs.extend(ends.map(|(first, last)| (*last, *first)));
                    pairs
                }
                ConnectMode::Triangles => Self::mesh(&seen, false),
                ConnectMode::Nearest => Self::nearest(&flat, links as usize, &apart, &allowed),
                _ => Self::mesh(&seen, true),
            };
            pairs.retain(|(i, j)| allowed(*i, *j));
            let lengths: Vec<f32> = pairs.iter().map(|(i, j)| apart(*i, *j)).collect();
            // Nearest has no Max distance, so Taper and Fade run from its
            // shortest line to its longest. Lines all one length, as on an
            // even grid, are then all drawn in full rather than all faded out.
            let longest = lengths.iter().copied().fold(0.0, f32::max);
            let shortest = lengths.iter().copied().fold(longest, f32::min);
            for ((i, j), d) in pairs.into_iter().zip(lengths) {
                if nearest {
                    join(i, j, d - shortest, longest - shortest);
                } else if d <= reach {
                    join(i, j, d, reach);
                }
            }
            return (out, tails);
        }

        let cells = Self::buckets(&flat, reach);
        let mut degree = vec![0u32; n];
        // Bounded by the pairing rule itself: every segment spends one of the
        // two ends' allowance, so there can never be more than `n · links / 2`
        // of them (14-ENGINEERING-RULES §6).
        let budget = (n as u64 * u64::from(links) / 2).min(u32::MAX as u64) as usize;
        let mut made = 0usize;
        let mut near: Vec<(f32, usize)> = Vec::new();

        for i in 0..n {
            if degree[i] >= links {
                continue;
            }
            near.clear();
            let (cx, cy) = Self::cell_of(flat[i], reach);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let Some(bucket) = cells.get(&(cx.saturating_add(dx), cy.saturating_add(dy)))
                    else {
                        continue;
                    };
                    for &j in bucket {
                        // Each pair once, and never a point with itself: the
                        // walk is ascending, so the later index owns the pair.
                        let j = j as usize;
                        if j <= i || !allowed(i, j) {
                            continue;
                        }
                        let d = apart(i, j);
                        if d <= reach {
                            near.push((d, j));
                        }
                    }
                }
            }
            // Nearest first, and the lower `id` first at equal distance —
            // `total_cmp` rather than `partial_cmp`, so a NaN orders rather
            // than making the sort's answer depend on the comparison order.
            near.sort_unstable_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            for &(d, j) in &near {
                if degree[i] >= links {
                    break;
                }
                if degree[j] >= links || made >= budget {
                    continue;
                }
                degree[i] += 1;
                degree[j] += 1;
                made += 1;
                join(i, j, d, reach);
            }
        }
        (out, tails)
    }

    /// Each point's `k` nearest of the points it may join, as pairs, each
    /// once and in a fixed order. `flat` is where the points sit across and
    /// down, and `apart` how far two of them are from each other.
    ///
    /// The points are sorted across, and each one looks left and right only
    /// until the gap across is already more than its `k`-th best.
    fn nearest(
        flat: &[[f32; 2]],
        k: usize,
        apart: &dyn Fn(usize, usize) -> f32,
        allowed: &dyn Fn(usize, usize) -> bool,
    ) -> Vec<(usize, usize)> {
        // ponytail: a sorted sweep, not a tree. The ceiling is a stream that
        // is tall and thin, where every point is close across to every other
        // and the sweep asks all of them, so n points cost n² distances. A
        // k-d tree is the upgrade if a column of points has to be fast.
        let x = |i: usize| flat.get(i).map_or(0.0, |p| p[0]);
        let mut by_x: Vec<usize> = (0..flat.len()).collect();
        by_x.sort_by(|a, b| x(*a).total_cmp(&x(*b)).then(a.cmp(b)));
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        // The nearest so far, closest first, and the lower index first at
        // equal distance.
        let mut best: Vec<(f32, usize)> = Vec::with_capacity(k + 1);
        for (at, &i) in by_x.iter().enumerate() {
            if pairs.len() >= points::CAP_HARD as usize {
                break;
            }
            best.clear();
            // False once `j` is too far across for anything beyond it to be
            // nearer than what is already held.
            let mut consider = |j: usize| {
                let kth = best.get(k.saturating_sub(1)).map_or(f32::INFINITY, |b| b.0);
                if (x(j) - x(i)).abs() > kth {
                    return false;
                }
                if allowed(i, j) {
                    let d = apart(i, j);
                    let slot =
                        best.partition_point(|b| b.0.total_cmp(&d).then(b.1.cmp(&j)).is_lt());
                    if slot < k {
                        best.insert(slot, (d, j));
                        best.truncate(k);
                    }
                }
                true
            };
            let (left, right) = by_x.split_at(at);
            for &j in right.iter().skip(1) {
                if !consider(j) {
                    break;
                }
            }
            for &j in left.iter().rev() {
                if !consider(j) {
                    break;
                }
            }
            pairs.extend(best.iter().map(|b| (i.min(b.1), i.max(b.1))));
        }
        // Two points that chose each other are one line.
        pairs.sort_unstable();
        pairs.dedup();
        pairs
    }

    /// The pairs a triangulation of `seen` joins, each once and in a fixed
    /// order: every edge of it, or only the ones round the outside.
    ///
    /// Two points in the same place count as the first of them, and a point
    /// that is not a number is left out.
    fn mesh(seen: &[[f32; 2]], outline: bool) -> Vec<(usize, usize)> {
        use spade::{DelaunayTriangulation, Point2, Triangulation};
        let mut mesh: DelaunayTriangulation<Point2<f64>> = DelaunayTriangulation::new();
        // Which point each vertex of the mesh stands for.
        let mut owner: Vec<usize> = Vec::new();
        for (i, p) in seen.iter().enumerate() {
            let Ok(v) = mesh.insert(Point2::new(f64::from(p[0]), f64::from(p[1]))) else {
                continue;
            };
            if v.index() == owner.len() {
                owner.push(i);
            }
        }
        let pair = |a: usize, b: usize| {
            let (a, b) = (owner.get(a).copied()?, owner.get(b).copied()?);
            Some((a.min(b), a.max(b)))
        };
        let mut pairs: Vec<(usize, usize)> = if outline {
            mesh.convex_hull()
                .filter_map(|e| pair(e.from().index(), e.to().index()))
                .collect()
        } else {
            mesh.undirected_edges()
                .filter_map(|e| {
                    let [a, b] = e.vertices();
                    pair(a.index(), b.index())
                })
                .collect()
        };
        pairs.sort_unstable();
        pairs
    }

    /// Which square of the projected plane a point falls in, at a grid pitch of
    /// one `reach`.
    ///
    /// A point whose coordinates are not finite — a producer handed a nonsense
    /// number — lands in the origin cell and is simply too far from everything
    /// to be joined, which is a degrade rather than a fault
    /// (14-ENGINEERING-RULES §4).
    fn cell_of(p: [f32; 2], reach: f32) -> (i32, i32) {
        let axis = |v: f32| {
            let c = (v / reach).floor();
            if c.is_finite() {
                c.clamp(i32::MIN as f32, i32::MAX as f32) as i32
            } else {
                0
            }
        };
        (axis(p[0]), axis(p[1]))
    }

    /// The projected plane cut into squares of one Max distance, each holding
    /// the indices that fall in it, in ascending order.
    ///
    /// **This is what keeps the pairing off the `n²` path.** Two points further
    /// apart than one square cannot be within reach of each other, so the nine
    /// squares around a point are the whole of what it has to ask — which makes
    /// the walk `O(n · k)` for `k` the crowd in a neighbourhood rather than
    /// `O(n²)` for the whole field.
    ///
    /// (ponytail: uniform buckets, no rebalancing. The ceiling is a *clump*:
    /// `m` points inside one square is `O(m²)` distance tests again, and the
    /// only things that bound it are Max points — 200 000 on the slider, a
    /// million at `points::CAP_HARD` — and Max connections, at most 64, ending
    /// each point's inner walk early. A hundredth of a default field in one
    /// square is 2000 points and four million tests for that square alone. The
    /// trigger is the shape that produces it rather than a profile: a
    /// Particulate stream emitted from a tight nozzle, or any bag whose points
    /// pile into far less than the frame, missing docs/13 §2's B12–B14 while
    /// the same point count spread evenly holds them. That comp wants a k-d
    /// tree or a sorted sweep here.)
    #[must_use]
    fn buckets(seen: &[[f32; 2]], reach: f32) -> HashMap<(i32, i32), Vec<u32>> {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::with_capacity(seen.len());
        for (i, p) in seen.iter().enumerate() {
            let Ok(i) = u32::try_from(i) else { break };
            cells.entry(Self::cell_of(*p, reach)).or_default().push(i);
        }
        cells
    }

    /// How the web is drawn — capsules through the shared kernel, and the
    /// host Mix.
    #[must_use]
    pub fn draw_style(self) -> points::DrawStyle {
        points::DrawStyle {
            // A capsule is a disc whose tail is somewhere else, so the mode is
            // the disc's and the geometry is in the tails.
            mode: points::RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            // Not Particulate's Streak: that one asks the *evaluation* for a
            // tail at an age offset. These tails are other points.
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Connect points' behaviour.
///
/// **No CPU reference through the trait**, the shape every points effect has:
/// what it draws is a stream and a camera, neither of which is a number in the
/// bag [`apply_cpu`](EffectDef::apply_cpu) is handed. Both ride the carriage
/// beside the op. The §1.6 oracle is [`ConnectPoints::links`] with
/// [`points::draw_stream`], exercised directly from the test suite.
pub struct ConnectPointsDef;

impl EffectDef for ConnectPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ConnectPoints as EffectMetadata>::SCHEMA
    }

    /// A picture in, a picture out, and a **stream in** beside it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// The raster factor, so a px@comp stream reaches the pixels this frame is
    /// drawn at.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(ConnectPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

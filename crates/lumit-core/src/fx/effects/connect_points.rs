//! Connect points: lines drawn between the points of a stream that are near
//! enough to each other — the plexus look.
//!
//! **In plain terms.** Wire a producer's teal Points socket into this effect and
//! every pair of points closer together than **Max distance** is joined by a
//! line. Particles drifting past each other web up and let go again; a Grid
//! becomes a mesh; a Scatter inside a silhouette becomes the constellation
//! everybody makes by hand out of a plugin they had to go and buy.
//!
//! **A line is a capsule, and a capsule is a disc that has been stretched.**
//! Nothing new is drawn here: the shared points draw already runs a dab from a
//! head to a tail (K-601), so a segment is one entry in an ordinary stream whose
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
//! wears the "no stream" mark K-509 gave the family.

use std::collections::HashMap;

use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port, PortType,
    ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The wire-only data input (points-stream.md §4.1).
pub const POINTS_PORT: &str = "points";

/// What this effect consumes. Not `three_d`: a web is drawn on the layer's own
/// flat picture, and "near enough to join" is a nearness *in that picture* —
/// the same reading, and for the same reason, that makes Points sample's
/// Nearest distance a distance on the frame (K-561).
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

/// Two kickers, as the generators have: which pairs are joined, and what the
/// line between them looks like.
pub const CONNECT_GROUPS: &[ParamGroup] = &[
    group(
        "Connections",
        &["max_distance", "max_links", "taper", "fade"],
    ),
    group("Line", &["width", "feather", "colour", "max_points"]),
];

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
)]
pub struct ConnectPoints {
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
    /// every point rather than only at the one being walked.
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

    /// How much a line thins out as it lengthens, per cent: at 0 every line is
    /// the same Width, at 100 a line at exactly Max distance has no width left.
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

    /// **The budget dial** (K-475), the family's row: the most **points** that
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
    /// The raster factor, for the one input the declaration cannot scale
    /// (K-385): a stream read off a wire is in px@comp and has to be rearranged
    /// into the pixels the frame is drawn at.
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
        // The newest by birth index, which is the cap rule the whole family
        // applies (K-475) — and here it is the ceiling on the pairing as much
        // as on the drawing.
        points.keep_newest(self.max_points.clamp(0, points::CAP_HARD as i32) as usize);
        let mut out = PointsStream {
            projection: points.projection,
            ..PointsStream::default()
        };
        let mut tails: Vec<[f32; 3]> = Vec::new();
        let reach = self.max_distance.max(0.0);
        let links = self.max_links.clamp(0, 64) as u32;
        let n = points.len();
        if reach <= 0.0 || links == 0 || n < 2 {
            return (out, tails);
        }
        // Where each point is *seen*, which is where "near enough" is judged
        // (K-561). On a 2D layer this is the pair the stream already holds.
        let seen: Vec<[f32; 2]> = (0..n).map(|i| points.projected(i)).collect();
        let cells = Self::buckets(&seen, reach);

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
        let mut degree = vec![0u32; n];
        // Bounded by the pairing rule itself: every segment spends one of the
        // two ends' allowance, so there can never be more than `n · links / 2`
        // of them (14-ENGINEERING-RULES §6).
        let budget = (n as u64 * u64::from(links) / 2).min(u32::MAX as u64) as usize;
        let mut near: Vec<(f32, usize)> = Vec::new();

        for i in 0..n {
            if degree[i] >= links {
                continue;
            }
            near.clear();
            let (cx, cy) = Self::cell_of(seen[i], reach);
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
                        if j <= i {
                            continue;
                        }
                        let d = (seen[j][0] - seen[i][0]).hypot(seen[j][1] - seen[i][1]);
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
                if degree[j] >= links || out.len() >= budget {
                    continue;
                }
                degree[i] += 1;
                degree[j] += 1;
                // How far along its own reach this line is: 0 for two points on
                // top of each other, 1 at exactly Max distance.
                let u = (d / reach).clamp(0.0, 1.0);
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
            }
        }
        (out, tails)
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

    /// How the web is drawn — capsules through the shared kernel, and the host
    /// Mix (K-425).
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

    /// A picture in, a picture out, and a **stream in** beside it (K-492).
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// The raster factor, so a px@comp stream reaches the pixels this frame is
    /// drawn at (K-385).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(ConnectPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

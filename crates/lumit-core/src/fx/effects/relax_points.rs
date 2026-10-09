//! Relax points pushes a stream's points apart until they stop overlapping.
//!
//! Each point wants a round space of its own: Radius, or its own size. Where
//! two spaces overlap, both points are pushed apart by a share of the
//! overlap, and that is done Iterations times. It remembers nothing: every
//! frame starts again from the places the wire brings. The moved stream goes
//! out on the Points socket, and is drawn as discs unless Mix is 0. Nothing
//! wired draws nothing.

use std::collections::HashMap;

use crate::fx::effects::vary_points::{POINTS_IN, POINTS_OUT};
use crate::fx::points::{self, PointsStream};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The most passes it will make, whatever is typed.
const MAX_ITERATIONS: i64 = 100;

/// The most neighbours one point is tested against in one pass.
const MAX_NEIGHBOURS: usize = 256;

/// No new pass starts once this many pairs have been tested.
const MAX_TESTS: u64 = 50_000_000;

/// The dice that say which way two points in the same place part.
const PART_ATTR: u32 = 48;

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The budget and the disc, under the family's own heading.
pub const RELAX_GROUPS: &[ParamGroup] = &[group("Point", &["max_points", "feather"])];

/// Radius is read while Use point size is off, and Spacing while it is on.
pub const RELAX_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "radius",
        on: "use_size",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "spacing",
        on: "use_size",
        cond: EnabledCond::BoolIs(true),
    },
];

/// Relax points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "relax_points",
    label = "Relax points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be pushed anywhere.
    roi = FullFrame,
    premultiplied = true,
    groups = RELAX_GROUPS,
    enabled_when = RELAX_ENABLED_WHEN,
)]
pub struct RelaxPoints {
    /// The space each point wants round itself, px@comp. Two points end up
    /// at least twice this apart.
    #[slider(min = 0.0, max = 200.0, default = 30.0, hard_min = 0.0, unit = Px)]
    pub radius: f32,

    /// Each point's space is its own size instead, so big points get more
    /// room and small ones fill the gaps.
    #[toggle(label = "Use point size", default = false)]
    pub use_size: bool,

    /// How much of its own size a point wants, per cent. Over 100 leaves
    /// gaps between points, under it lets them overlap.
    #[slider(min = 0.0, max = 300.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub spacing: f32,

    /// How many times the points are pushed apart. More settles further.
    #[counter(
        min = 0,
        max = 50,
        default = 8,
        hard_min = 0,
        hard_max = MAX_ITERATIONS,
        unit = Raw
    )]
    pub iterations: i32,

    /// How much of an overlap one pass removes, per cent. 100 can make the
    /// points jitter from frame to frame.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub strength: f32,

    /// Free lets the points drift anywhere. Keep inside holds them in the
    /// box they started in.
    #[choice(label = "Bounds", options = ["Free", "Keep inside"], default = 0)]
    pub bounds: u32,

    /// Which points move. The rest stay where they are and still push the
    /// ones that move.
    #[choice(
        label = "Apply to",
        options = ["All points", "Picked", "Not picked"],
        default = 0
    )]
    pub apply_to: u32,

    /// The most points it works on. A longer stream is trimmed to its
    /// newest, as the rest of the family does.
    #[counter(
        label = "Max points",
        min = 1,
        max = 200_000,
        default = points::CAP_DEFAULT,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_points: i32,

    /// How soft the disc a point is drawn as is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// handed on and nothing is drawn.
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

impl RelaxPoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// The wired stream with its points pushed apart, across and down the
    /// layer's own plane. `in_stream` and the bag must be in the same units,
    /// and the answer is in those units too.
    ///
    /// Every pass reads where all the points were after the last one and
    /// writes new places, so the order they are visited in changes nothing.
    #[must_use]
    pub fn apply(self, in_stream: &PointsStream) -> PointsStream {
        let mut out = in_stream.clone();
        out.keep_newest(self.max_points.clamp(0, points::CAP_HARD as i32) as usize);
        let n = out.len();
        let strength = (self.strength / 100.0).clamp(0.0, 1.0);
        let spacing = (self.spacing / 100.0).max(0.0);
        // The round space each point wants, as a radius. A stretched point
        // takes its longer side, so it never overlaps along it.
        let space: Vec<f32> = (0..n)
            .map(|i| {
                let own = if self.use_size {
                    let stretch = out.stretch_of(i);
                    let size = out.size.get(i).copied().unwrap_or(0.0);
                    0.5 * size * stretch[0].abs().max(stretch[1].abs()) * spacing
                } else {
                    self.radius
                };
                if own.is_finite() {
                    own.max(0.0)
                } else {
                    0.0
                }
            })
            .collect();
        // No two points further apart than this can overlap, so it is the
        // side of the squares neighbours are looked for in.
        let reach = 2.0 * space.iter().copied().fold(0.0, f32::max);
        if n < 2 || strength <= 0.0 || reach <= 0.0 {
            return out;
        }
        let moves: Vec<bool> = (0..n)
            .map(|i| {
                let picked = out.picked(i);
                !((self.apply_to == 1 && !picked) || (self.apply_to == 2 && picked))
            })
            .collect();
        let mut at: Vec<[f32; 2]> = (0..n)
            .map(|i| out.position.get(i).map_or([0.0; 2], |p| [p[0], p[1]]))
            .collect();
        // The box the points start in, for Keep inside.
        let (mut low, mut high) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for p in &at {
            low = [low[0].min(p[0]), low[1].min(p[1])];
            high = [high[0].max(p[0]), high[1].max(p[1])];
        }
        let inside = self.bounds == 1 && low[0] <= high[0] && low[1] <= high[1];

        // ponytail: uniform squares and a count on each point's neighbours,
        // not a tree. A pass costs at most points times MAX_NEIGHBOURS tests,
        // and no pass starts after MAX_TESTS, so the worst case is a million
        // points in one pile: one pass of 256 million tests, and each point
        // pushed only from the first 256 it shares a square with. A stream
        // spread evenly costs about ten tests a point a pass. One huge point
        // among small ones makes every square huge and is the same pile. A
        // k-d tree, or squares sized to the usual point, is the upgrade.
        let mut next = at.clone();
        let mut tests = 0u64;
        for _ in 0..self.iterations.clamp(0, MAX_ITERATIONS as i32) {
            if tests >= MAX_TESTS {
                break;
            }
            let cells = buckets(&at, reach);
            for i in 0..n {
                if !moves[i] {
                    continue;
                }
                let (cx, cy) = cell_of(at[i], reach);
                let mut push = [0.0f32; 2];
                let mut seen = 0usize;
                'near: for down in -1..=1 {
                    for across in -1..=1 {
                        let square = (cx.saturating_add(across), cy.saturating_add(down));
                        let Some(cell) = cells.get(&square) else {
                            continue;
                        };
                        for &j in cell {
                            let j = j as usize;
                            if j == i {
                                continue;
                            }
                            if seen >= MAX_NEIGHBOURS {
                                break 'near;
                            }
                            seen += 1;
                            let (dx, dy) = (at[i][0] - at[j][0], at[i][1] - at[j][1]);
                            let apart = dx.hypot(dy);
                            let overlap = space[i] + space[j] - apart;
                            // A NaN is no overlap, so it never reaches a
                            // position.
                            if overlap.is_nan() || overlap <= 0.0 {
                                continue;
                            }
                            let away = if apart > 1e-6 {
                                [dx / apart, dy / apart]
                            } else {
                                part(&out.id, i, j)
                            };
                            // Two points that both move share the overlap.
                            // One beside a point that stays takes all of it.
                            let share = if moves[j] { 0.5 } else { 1.0 };
                            push[0] += away[0] * overlap * share * strength;
                            push[1] += away[1] * overlap * share * strength;
                        }
                    }
                }
                tests += seen as u64;
                let mut to = [at[i][0] + push[0], at[i][1] + push[1]];
                if inside {
                    to = [to[0].clamp(low[0], high[0]), to[1].clamp(low[1], high[1])];
                }
                next[i] = to;
            }
            std::mem::swap(&mut at, &mut next);
        }
        for (p, to) in out.position.iter_mut().zip(&at) {
            (p[0], p[1]) = (to[0], to[1]);
        }
        out
    }
}

/// Which way point `i` leaves point `j` when both are in the same place. The
/// way comes from the two ids, so it is the same every frame, and `j` leaves
/// `i` the opposite way.
fn part(ids: &[u64], i: usize, j: usize) -> [f32; 2] {
    let id = |k: usize| ids.get(k).copied().unwrap_or(0);
    let (low, high) = (id(i).min(id(j)), id(i).max(id(j)));
    let angle = points::draw(0, low ^ high.rotate_left(32), PART_ATTR) * std::f32::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    if (id(i), i) < (id(j), j) {
        [cos, sin]
    } else {
        [-cos, -sin]
    }
}

/// Which square of side `reach` a place is in. A place that is not a number
/// goes in the first square.
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

/// The plane cut into squares of side `reach`, each holding the points in it
/// in stream order. Only ever looked up by square, so the map's own order
/// never reaches a result.
fn buckets(at: &[[f32; 2]], reach: f32) -> HashMap<(i32, i32), Vec<u32>> {
    let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::with_capacity(at.len());
    for (i, p) in at.iter().enumerate() {
        let Ok(i) = u32::try_from(i) else { break };
        cells.entry(cell_of(*p, reach)).or_default().push(i);
    }
    cells
}

/// Relax points' behaviour, the same shape as Vary points'.
pub struct RelaxPointsDef;

impl EffectDef for RelaxPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<RelaxPoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(RelaxPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    fn modify_points(&self, p: Params<'_>, cx: &crate::fx::ModifyCx<'_>) -> Option<PointsStream> {
        Some(RelaxPoints::read(p).apply(cx.input()?))
    }
}

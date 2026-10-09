//! Points along path puts points along one of the layer's masks, or along a
//! Shape layer's own paths.
//!
//! Count points are spread by distance along the path between Start and End,
//! and Offset slides them all along it, so animating Offset marches them
//! round. Placement can throw them at random places along it instead, or put
//! one on each of the mask's own corners. Each point can turn to face the way
//! the path runs, which is what a Clone to points below wants, and carries
//! how far along the path it is for the effects below to read. There is no
//! time in it and nothing is remembered. A layer with no mask makes no points.
//!
//! Follow set to Shape takes the outlines the Shape layer the effect is on
//! draws instead, after any trim, offset or combine, every one of them in the
//! layer's own order as one long run, so Count is shared between them by
//! length. On any other layer it makes none.

use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    Port, PortType, ResolveCx, Signature, Value,
};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

/// The same Points output every producer declares.
const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// Where the points go, the dice only Random rolls, and what a point looks
/// like.
pub const ALONG_GROUPS: &[ParamGroup] = &[
    group(
        "Path",
        &[
            "path_from",
            "mask_path",
            "placement",
            "count",
            "start",
            "end",
            "offset",
            "wrap",
            "align",
        ],
        None,
    ),
    group("", &["seed"], Some(("placement", &[1]))),
    group("Point", &["size", "feather", "colour"], None),
];

const fn grey(param: &'static str) -> EnabledWhen {
    EnabledWhen {
        param,
        on: "placement",
        cond: EnabledCond::ChoiceIsNot(2),
    }
}

/// Corners takes its points from the path, so it has no Count and nothing to
/// slide. Following the shape leaves no mask to choose.
pub const ALONG_ENABLED_WHEN: &[EnabledWhen] = &[
    grey("count"),
    grey("offset"),
    grey("wrap"),
    EnabledWhen {
        param: "mask_path",
        on: "path_from",
        cond: EnabledCond::ChoiceIsNot(1),
    },
];

/// Which dice a Random placement rolls for where a point sits.
const ALONG_ATTR: u32 = 0;

/// Where along the path the points are put.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// Evenly spaced by distance.
    #[default]
    Even,
    /// At random places, from the seed, the same on every frame.
    Random,
    /// One on each of the mask's own vertices.
    Corners,
}

impl Placement {
    /// The Choice option labels, in code order. A Choice is stored as its
    /// index, so a new placement goes on the end.
    pub const OPTIONS: &'static [&'static str] = &["Even", "Random", "Corners"];

    /// The placement for a stored Choice index. Anything unknown is Even.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => Placement::Random,
            2 => Placement::Corners,
            _ => Placement::Even,
        }
    }
}

/// Points along path's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "points_along_path",
    label = "Points along path",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A mask may be drawn anywhere, and the discs are drawn over the whole
    // picture.
    roi = FullFrame,
    premultiplied = true,
    // Not seeded, as Grid is not: there is no clock in it.
    seeded = false,
    groups = ALONG_GROUPS,
    enabled_when = ALONG_ENABLED_WHEN,
)]
pub struct PointsAlongPath {
    /// What the points sit along: one of the layer's masks, or the paths of
    /// the Shape layer the effect is on. `mask::effect_path_at` reads it. A
    /// Choice is stored as its index, so a new one goes on the end.
    #[choice(label = "Follow", options = ["Mask", "Shape"], default = 0)]
    pub path_from: u32,

    /// Which of the layer's masks the points sit along.
    #[mask_path(label = "Mask path")]
    pub mask_path: bool,

    /// Where along the path the points are put. Corners ignores Count and
    /// keeps the vertices between Start and End.
    #[choice(label = "Placement", options = *Placement::OPTIONS, default = 0)]
    pub placement: u32,

    /// How many points.
    #[counter(
        label = "Count",
        min = 1,
        max = 200,
        default = 20,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub count: i32,

    /// Where along the path the first point sits, per cent of its length.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub start: f32,

    /// Where along the path the last point sits, per cent of its length.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub end: f32,

    /// Slides every point along the path, per cent of its length. A closed
    /// path wraps round, and an open one stops at its ends unless Wrap is on.
    #[slider(min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub offset: f32,

    /// On an open path, a point pushed past one end comes back in at the
    /// other. A closed path always does.
    #[toggle(label = "Wrap", default = false)]
    pub wrap: bool,

    /// Turn each point to face the way the path runs.
    #[toggle(label = "Align to path", default = true)]
    pub align: bool,

    /// Which random places. The reseed button rolls it.
    #[seed]
    pub seed: u32,

    /// The diameter of the disc a point is drawn as, px@comp.
    #[slider(min = 0.0, max = 200.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// How soft that disc's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The colour a point is drawn in. Scene-linear, and values above 1 are
    /// useful under a glow.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// emitted and nothing is drawn.
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

impl PointsAlongPath {
    /// The raster factor, since the mask path and the camera arrive in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The points along `path`, in the path's own units. An empty path makes
    /// none. `id` is the point's place in the row, so a consumer can follow
    /// one while Offset moves it, and `index` is how far along the path it
    /// sits, 0 to 1.
    #[must_use]
    pub fn stream(self, path: &MaskPolyline, projection: Projection) -> PointsStream {
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        if path.is_empty() {
            return out;
        }
        let n = self.count.clamp(0, points::CAP_HARD as i32);
        let length = path.length();
        let placement = Placement::from_code(self.placement);
        let wraps = path.closed || self.wrap;
        let (from, to) = (
            (self.start / 100.0).clamp(0.0, 1.0),
            (self.end / 100.0).clamp(0.0, 1.0),
        );
        // Round a whole closed path the last point would land on the first,
        // so the gaps are counted rather than the points.
        let gaps = if wraps && to - from >= 1.0 { n } else { n - 1 };
        let size = self.size.max(0.0);
        // Premultiplied, as every colour in the working space is.
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        // One point at `at`, `far` px along the path.
        let mut put = |at: [f32; 2], facing: [f32; 2], far: f32, id: u64| {
            out.position.push([at[0], at[1], 0.0]);
            out.speed.push([0.0; 3]);
            out.age.push(0.0);
            // 1 rather than 0, so a consumer dividing by it reads a young point.
            out.life.push(1.0);
            out.size.push(size);
            out.rotation.push(if self.align {
                facing[1].atan2(facing[0])
            } else {
                0.0
            });
            out.colour.push(colour);
            out.id.push(id);
            out.index
                .push(if length > 0.0 { far / length } else { 0.0 });
        };
        if placement == Placement::Corners {
            let corners = path.corners.iter().take(points::CAP_HARD as usize);
            for (id, &corner) in corners.enumerate() {
                let far = path.arc.get(corner).copied().unwrap_or(0.0);
                // By its place in the list, not its distance: where one path
                // stops and the next starts, both vertices are equally far.
                let Some(&at) = path.points.get(corner) else {
                    continue;
                };
                if (from * length..=to * length).contains(&far) {
                    // A vertex its path ends on faces the way it arrived.
                    let ends = path.arc.get(corner + 1).is_none_or(|next| *next <= far);
                    let edge = if ends {
                        corner.saturating_sub(1)
                    } else {
                        corner
                    };
                    put(at, path.edge_tangent(edge), far, id as u64);
                }
            }
            return out;
        }
        for i in 0..n {
            let id = u64::from(i.unsigned_abs());
            let spread = if placement == Placement::Random {
                (to - from) * points::draw(self.seed, id, ALONG_ATTR)
            } else {
                (to - from) * i as f32 / gaps.max(1) as f32
            };
            let along = from + spread + self.offset / 100.0;
            // An open path's own end is not past it, so it stays put.
            let along = if path.closed || (self.wrap && !(0.0..=1.0).contains(&along)) {
                along.rem_euclid(1.0)
            } else {
                along.clamp(0.0, 1.0)
            };
            let far = along * length;
            put(path.point_at(far), path.tangent_at(far), far, id);
        }
        out
    }

    /// How the stream is drawn: a feathered disc per point, and the Mix.
    #[must_use]
    pub fn draw_style(self) -> DrawStyle {
        DrawStyle {
            mode: RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Points along path's behaviour. No CPU reference through the trait, as with
/// the rest of the family: the path and the camera aren't in the bag.
pub struct PointsAlongPathDef;

impl EffectDef for PointsAlongPathDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PointsAlongPath as EffectMetadata>::SCHEMA
    }

    /// The picture and the data, as every producer declares it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(PointsAlongPath::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

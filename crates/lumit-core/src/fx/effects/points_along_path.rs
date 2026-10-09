//! Points along path puts evenly spaced points along one of the layer's masks.
//!
//! Count points are spread by distance along the path between Start and End,
//! and Offset slides them all along it, so animating Offset marches them
//! round. Each point can turn to face the way the path runs, which is what a
//! Clone to points below wants. There is no time in it and nothing is
//! remembered. A layer with no mask makes no points.

use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, Port, PortType,
    ResolveCx, Signature, Value,
};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

/// The same Points output every producer declares.
const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// Where the points go, and what one looks like.
pub const ALONG_GROUPS: &[ParamGroup] = &[
    group(
        "Path",
        &["mask_path", "count", "start", "end", "offset", "align"],
    ),
    group("Point", &["size", "feather", "colour"]),
];

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
)]
pub struct PointsAlongPath {
    /// Which of the layer's masks the points sit along.
    #[mask_path(label = "Mask path")]
    pub mask_path: bool,

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
    /// path wraps round, and an open one stops at its ends.
    #[slider(min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub offset: f32,

    /// Turn each point to face the way the path runs.
    #[toggle(label = "Align to path", default = true)]
    pub align: bool,

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
    /// one while Offset moves it.
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
        let (from, to) = (
            (self.start / 100.0).clamp(0.0, 1.0),
            (self.end / 100.0).clamp(0.0, 1.0),
        );
        // Round a whole closed path the last point would land on the first,
        // so the gaps are counted rather than the points.
        let gaps = if path.closed && to - from >= 1.0 {
            n
        } else {
            n - 1
        };
        let size = self.size.max(0.0);
        // Premultiplied, as every colour in the working space is.
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        for i in 0..n {
            let along = from + (to - from) * i as f32 / gaps.max(1) as f32 + self.offset / 100.0;
            let along = if path.closed {
                along.rem_euclid(1.0)
            } else {
                along.clamp(0.0, 1.0)
            };
            let at = path.point_at(along * length);
            let facing = path.tangent_at(along * length);
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
            out.id.push(u64::from(i.unsigned_abs()));
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

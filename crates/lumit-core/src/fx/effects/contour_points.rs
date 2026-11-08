//! Contour points puts one point in the middle of each closed piece of a
//! path: Cavalry's sub-meshes.
//!
//! Follow says which path, as on Points along path: one of the layer's
//! masks, or every path of the Shape layer the effect is on. A Shape layer's
//! art is read after its combines, so two shapes cut one from the other give
//! a point on each piece the cut leaves, and the items of one layer give a
//! point each. A point sits on its piece's centre of area, carries the
//! piece's place in the path as its number, and is as big as Size, or as
//! the piece itself under Size from area. Clone to points' Points input puts
//! a copy on each piece. The stream goes out on the Points socket, and is
//! drawn as discs unless Mix is 0.

use crate::fx::effects::vary_points::POINTS_OUT;
use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode};
use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, registry::Signature, schema::EffectSchema, schema::EnabledCond,
    schema::EnabledWhen,
};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

/// Mask path greys while Follow is Shape.
pub const CONTOUR_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "mask_path",
    on: "path_from",
    cond: EnabledCond::ChoiceIsNot(1),
}];

/// The separate pieces a flattened path is made of, each as its own run of
/// points. A path with several pieces is one run with a step of no length
/// from each piece to the next, which is where it is cut.
#[must_use]
pub fn pieces(path: &MaskPolyline) -> Vec<&[[f32; 2]]> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..path.points.len() {
        let jumps = path.arc.get(i) == path.arc.get(i - 1) && path.points[i] != path.points[i - 1];
        if jumps {
            out.push(&path.points[start..i]);
            start = i;
        }
    }
    if start < path.points.len() {
        out.push(&path.points[start..]);
    }
    out.retain(|piece| !piece.is_empty());
    out
}

/// A piece's centre of area and its area, px². A piece with no area, a line,
/// has its points' middle and nought.
#[must_use]
pub fn centre_of(piece: &[[f32; 2]]) -> ([f32; 2], f32) {
    let n = piece.len();
    let (mut area, mut cx, mut cy) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let [x0, y0] = piece[i].map(f64::from);
        let [x1, y1] = piece[(i + 1) % n].map(f64::from);
        let cross = x0 * y1 - x1 * y0;
        area += cross;
        cx += (x0 + x1) * cross;
        cy += (y0 + y1) * cross;
    }
    area *= 0.5;
    if area.abs() < 1e-6 {
        let k = n.max(1) as f64;
        let (sx, sy) = piece.iter().fold((0.0, 0.0), |(sx, sy), p| {
            (sx + f64::from(p[0]), sy + f64::from(p[1]))
        });
        return ([(sx / k) as f32, (sy / k) as f32], 0.0);
    }
    (
        [(cx / (6.0 * area)) as f32, (cy / (6.0 * area)) as f32],
        area.abs() as f32,
    )
}

/// Contour points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "contour_points",
    label = "Contour points",
    version = 1,
    category = Points,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    seeded = false,
    enabled_when = CONTOUR_ENABLED_WHEN,
)]
pub struct ContourPoints {
    /// Whose pieces: one of the layer's masks, or the paths of the Shape
    /// layer the effect is on. A new option goes on the end.
    #[choice(label = "Follow", options = ["Mask", "Shape"], default = 1)]
    pub path_from: u32,

    /// Which of the layer's masks.
    #[mask_path(label = "Mask path")]
    pub mask_path: bool,

    /// Each point as big as a circle of its piece's area, rather than Size.
    #[toggle(label = "Size from area", default = false)]
    pub size_from_area: bool,

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

    /// The colour a point is drawn in.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

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

impl ContourPoints {
    /// The raster factor, since the path arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// A point on each piece of `path`, in the path's units.
    #[must_use]
    pub fn stream(self, path: &MaskPolyline, projection: Projection) -> PointsStream {
        let mut out = PointsStream {
            projection,
            ..PointsStream::default()
        };
        // Premultiplied, as every colour in the working space is.
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        for (i, piece) in pieces(path).into_iter().enumerate() {
            if out.len() >= points::CAP_HARD as usize {
                break;
            }
            let ([x, y], area) = centre_of(piece);
            let size = if self.size_from_area {
                2.0 * (area / std::f32::consts::PI).sqrt()
            } else {
                self.size.max(0.0)
            };
            out.position.push([x, y, 0.0]);
            out.speed.push([0.0; 3]);
            out.age.push(0.0);
            out.life.push(1.0);
            out.size.push(size);
            out.rotation.push(0.0);
            out.colour.push(colour);
            out.id.push(i as u64);
        }
        out
    }

    /// How the stream is drawn — a feathered disc per point, and the host Mix.
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

/// Contour points' behaviour, Points along path's shape.
pub struct ContourPointsDef;

impl EffectDef for ContourPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ContourPoints as EffectMetadata>::SCHEMA
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(ContourPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

//! Intersection points puts a point wherever a path crosses itself or
//! another.
//!
//! Follow says which paths, as on Points along path: one of the layer's
//! masks, which can only cross itself, or every path of the Shape layer the
//! effect is on, which cross one another as well. The points come in the
//! order the path meets them, each turned to the way the path runs there. A
//! crossing is found once, however many pieces meet at it within a hundredth
//! of a pixel. The stream goes out on the Points socket, and is drawn as
//! discs unless Mix is 0.

use crate::fx::effects::contour_points::pieces;
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
pub const INTERSECTION_ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "mask_path",
    on: "path_from",
    cond: EnabledCond::ChoiceIsNot(1),
}];

/// An edge: its two ends, the piece it is on, its place there, and how many
/// edges that piece has if it is closed, 0 if it is open.
type Edge = ([f32; 2], [f32; 2], usize, usize, usize);

/// Where the edges `a0`–`a1` and `b0`–`b1` cross, as how far along the first
/// one, or `None` where they do not. Parallel edges never cross.
fn crossing(a0: [f32; 2], a1: [f32; 2], b0: [f32; 2], b1: [f32; 2]) -> Option<f32> {
    let r = [a1[0] - a0[0], a1[1] - a0[1]];
    let s = [b1[0] - b0[0], b1[1] - b0[1]];
    let d = r[0] * s[1] - r[1] * s[0];
    if d.abs() < 1e-9 {
        return None;
    }
    let q = [b0[0] - a0[0], b0[1] - a0[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / d;
    let u = (q[0] * r[1] - q[1] * r[0]) / d;
    ((0.0..1.0).contains(&t) && (0.0..1.0).contains(&u)).then_some(t)
}

/// Every crossing of `path`, in the order the path meets them: the place
/// and the way the first edge runs there.
#[must_use]
pub fn crossings(path: &MaskPolyline) -> Vec<([f32; 2], f32)> {
    // Every edge, with which piece it is on and its place there, so that
    // neighbours, which only touch, are not counted.
    let mut edges: Vec<Edge> = Vec::new();
    for (k, piece) in pieces(path).into_iter().enumerate() {
        // An open piece's ends do not meet, so its first and last edges can
        // cross.
        let closed = piece.len() > 2 && piece.first() == piece.last();
        let n = if closed { piece.len() - 1 } else { 0 };
        for i in 0..piece.len().saturating_sub(1) {
            if piece[i] != piece[i + 1] {
                edges.push((piece[i], piece[i + 1], k, i, n));
            }
        }
    }
    let boxes: Vec<[f32; 4]> = edges
        .iter()
        .map(|(a, b, ..)| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ]
        })
        .collect();
    let mut out: Vec<(usize, f32, [f32; 2], f32)> = Vec::new();
    for (i, &(a0, a1, ka, ia, na)) in edges.iter().enumerate() {
        for (j, &(b0, b1, kb, ib, nb)) in edges.iter().enumerate().skip(i + 1) {
            let (p, q) = (boxes[i], boxes[j]);
            if p[2] < q[0] || q[2] < p[0] || p[3] < q[1] || q[3] < p[1] {
                continue;
            }
            // Edges next to each other on one piece share an end, which is
            // not a crossing; nor are a closed piece's last and first.
            if ka == kb {
                let gap = ib.abs_diff(ia);
                if gap <= 1 || gap + 1 == na.max(nb) {
                    continue;
                }
            }
            if let Some(t) = crossing(a0, a1, b0, b1) {
                let at = [a0[0] + (a1[0] - a0[0]) * t, a0[1] + (a1[1] - a0[1]) * t];
                let way = (a1[1] - a0[1]).atan2(a1[0] - a0[0]);
                out.push((i, t, at, way));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut kept: Vec<([f32; 2], f32)> = Vec::new();
    for (_, _, at, way) in out {
        let near = |p: &([f32; 2], f32)| (p.0[0] - at[0]).hypot(p.0[1] - at[1]) < 0.01;
        if !kept.iter().any(near) {
            kept.push((at, way));
        }
    }
    kept
}

/// Intersection points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "intersection_points",
    label = "Intersection points",
    version = 1,
    category = Points,
    cost = Moderate,
    roi = FullFrame,
    premultiplied = true,
    seeded = false,
    enabled_when = INTERSECTION_ENABLED_WHEN,
)]
pub struct IntersectionPoints {
    /// Whose crossings: one of the layer's masks, or the paths of the Shape
    /// layer the effect is on. A new option goes on the end.
    #[choice(label = "Follow", options = ["Mask", "Shape"], default = 1)]
    pub path_from: u32,

    /// Which of the layer's masks.
    #[mask_path(label = "Mask path")]
    pub mask_path: bool,

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

impl IntersectionPoints {
    /// The raster factor, since the path arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// A point on each crossing of `path`, in the path's units.
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
        for (i, ([x, y], way)) in crossings(path)
            .into_iter()
            .take(points::CAP_HARD as usize)
            .enumerate()
        {
            out.position.push([x, y, 0.0]);
            out.speed.push([0.0; 3]);
            out.age.push(0.0);
            out.life.push(1.0);
            out.size.push(self.size.max(0.0));
            out.rotation.push(way);
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

/// Intersection points' behaviour, Points along path's shape.
pub struct IntersectionPointsDef;

impl EffectDef for IntersectionPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<IntersectionPoints as EffectMetadata>::SCHEMA
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(
            IntersectionPoints::DERIVED_PX_SCALE,
            Value::Float(cx.px_scale),
        );
    }
}

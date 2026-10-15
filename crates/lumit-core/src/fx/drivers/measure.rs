//! Measure: how big a layer's content is, as numbers.
//!
//! **In plain terms.** Name a layer and this hands out the box round what it
//! shows: its width and height, its centre, and its four edges. Wire Width into
//! a plate's size and the plate grows and shrinks with the name on a lower
//! third, frame by frame, as the words change.
//!
//! **What counts as the content depends on the kind.** A Text layer is
//! measured round its letters as they are set, not round the paragraph box
//! they wrap in. A Shape layer is measured round its art. A solid and a precomp
//! are their frame. Footage is the comp's frame, because the size of a file is
//! not kept in the document; it is the same stand-in an expression's `width`
//! reads. A Points stream wired into the socket is measured instead of the
//! layer, round every live point.
//!
//! **Two spaces.** *Layer* is the layer's own pixels, which is what
//! `sourceRectAtTime` answers in. *Comp* runs the box through the layer's
//! transform and its parents', so a scaled or moved layer measures where it
//! is drawn. A turned layer measures the box round its turned corners.
//!
//! **Nothing to measure reads nought**: a Null, a camera, a light, an empty
//! text or a shape with no art.

use std::sync::Arc;

use crate::expression::ExpressionContext;
use crate::fx::points::PointsStream;
use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use crate::model::{Composition, Document, Layer, LayerKind};
use crate::shape::Affine;
use lumit_fx_macros::Effect;

/// The layer-reference row, by parameter id.
pub const LAYER_PARAM: &str = "layer";
/// The wire-only Points input. Wired, it is measured instead of the layer.
pub const POINTS_PORT: &str = "points";

/// What the Space row offers.
pub const SPACE_OPTIONS: [&str; 2] = ["Layer", "Comp"];
/// Space: the box through the layer's transform and its parents'.
pub const SPACE_COMP: u32 = 1;

const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];
const OUTPUTS: &[Port] = &[
    Port::new("width", "Width", PortType::Number),
    Port::new("height", "Height", PortType::Number),
    Port::new("centre_x", "Centre X", PortType::Number),
    Port::new("centre_y", "Centre Y", PortType::Number),
    Port::new("left", "Left", PortType::Number),
    Port::new("top", "Top", PortType::Number),
    Port::new("right", "Right", PortType::Number),
    Port::new("bottom", "Bottom", PortType::Number),
];

/// Measure's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "measure",
    label = "Measure",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Measure {
    /// The layer measured. A fresh node points at the layer it lands on, and
    /// an unset row measures that layer too.
    #[layer(label = "Layer", self_default = true)]
    pub layer: bool,

    /// Room added either side of the box, px@comp.
    #[slider(label = "Padding x", min = 0.0, max = 200.0, default = 0.0, unit = Px)]
    pub padding_x: f32,

    /// Room added above and below the box, px@comp.
    #[slider(label = "Padding y", min = 0.0, max = 200.0, default = 0.0, unit = Px)]
    pub padding_y: f32,

    /// The layer's own pixels, or the comp's.
    #[choice(label = "Space", options = SPACE_OPTIONS, default = 0)]
    pub space: u32,
}

/// Measure's behaviour.
pub struct MeasureDef;

impl EffectDef for MeasureDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Measure as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: POINTS_IN,
            outputs: OUTPUTS,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Measure::read(cx.params);
        let [left, top, right, bottom] =
            measured(cx, p.space == SPACE_COMP).map_or([0.0; 4], |r| {
                let (px, py) = (f64::from(p.padding_x), f64::from(p.padding_y));
                [r[0] - px, r[1] - py, r[2] + px, r[3] + py]
            });
        let out = [
            right - left,
            bottom - top,
            (left + right) * 0.5,
            (top + bottom) * 0.5,
            left,
            top,
            right,
            bottom,
        ];
        for (port, v) in OUTPUTS.iter().zip(out) {
            push(port.id, Value::Float(v as f32));
        }
    }
}

/// The box this node measures, before padding.
fn measured(cx: &DriverCx<'_>, comp_space: bool) -> Option<[f64; 4]> {
    let ctx = cx.context;
    let doc = &ctx.document;
    let comp = ctx.comp.and_then(|id| doc.comp(id));
    let own = comp
        .zip(ctx.layer)
        .and_then(|(c, id)| c.layers.iter().find(|l| l.id == id));
    // The comp time of this frame, from this node's own layer clock: a node
    // can be asked at another time than the frame's (a Smooth reads around it).
    let t = cx.lt + own.map_or(0.0, |l| l.start_offset.0.to_f64());
    if let Some(stream) = (cx.points_input)(POINTS_PORT) {
        let rect = stream_box(&stream)?;
        return match (comp, own) {
            (Some(c), Some(l)) if comp_space => Some(to_comp(c, l, t, ctx, rect)),
            _ => Some(rect),
        };
    }
    let comp = comp?;
    let id = cx.inst.layer_ref(LAYER_PARAM).or(ctx.layer)?;
    let layer = comp.layers.iter().find(|l| l.id == id)?;
    let rect = layer_rect(doc, comp, layer, t, ctx)?;
    Some(if comp_space {
        to_comp(comp, layer, t, ctx, rect)
    } else {
        rect
    })
}

/// The box round some points, or `None` when there are none.
fn bounds(points: impl Iterator<Item = (f64, f64)>) -> Option<[f64; 4]> {
    points.fold(None, |b, (x, y)| {
        Some(b.map_or([x, y, x, y], |b: [f64; 4]| {
            [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
        }))
    })
}

/// The box round every live point of a stream, where it is drawn.
fn stream_box(stream: &PointsStream) -> Option<[f64; 4]> {
    bounds((0..stream.len()).map(|i| {
        let [x, y] = stream.projected(i);
        (f64::from(x), f64::from(y))
    }))
}

/// What `layer` shows at comp time `t`, as left, top, right and bottom in its
/// own pixels, or `None` where it shows nothing. The one measuring both this
/// driver and an expression's `sourceRectAtTime` read.
#[must_use]
pub fn layer_rect(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    context: &ExpressionContext,
) -> Option<[f64; 4]> {
    let lt = crate::time::layer_time(t, layer.start_offset.0);
    let frame = |w: u32, h: u32| Some([0.0, 0.0, f64::from(w), f64::from(h)]);
    match &layer.kind {
        LayerKind::Text { document } => {
            // A line on a path is drawn round the path, so the path's box,
            // with a text size of room either side for the letters on it.
            if let Some(id) = document.path {
                let path = crate::mask::mask_path_at(&layer.masks, Some(id), false, lt);
                let size = document.size;
                let along = bounds(
                    path.points
                        .iter()
                        .map(|[x, y]| (f64::from(*x), f64::from(*y))),
                );
                if let Some(b) = along {
                    return Some([b[0] - size, b[1] - size, b[2] + size, b[3] + size]);
                }
            }
            // The words this layer shows at `t`, which an expression may write.
            // One level deeper, so a text that measures itself stops.
            let words = document.resolved_text(Arc::new(ExpressionContext {
                layer: Some(layer.id),
                comp_time: t,
                ..context.increase_depth()
            }));
            crate::text::words_box(document, &words)
        }
        // The picture's corner is the corner of the art's box.
        LayerKind::Shape { contents } => crate::shape::contents_bounds(contents, lt)
            .map(|(x0, y0, x1, y1)| [0.0, 0.0, x1 - x0, y1 - y0]),
        LayerKind::Solid { def } => doc.solid(*def).and_then(|s| frame(s.width, s.height)),
        LayerKind::Precomp { comp: inner } => {
            doc.comp(*inner).and_then(|c| frame(c.width, c.height))
        }
        LayerKind::Footage { .. } | LayerKind::Sequence { .. } | LayerKind::Adjustment => {
            frame(comp.width, comp.height)
        }
        LayerKind::Null | LayerKind::Camera { .. } | LayerKind::Light { .. } => None,
    }
}

/// `rect` in `layer`'s pixels, run through its transform and its parents' at
/// comp time `t`, as the box round the four placed corners.
///
/// ponytail: flat placement only (position, anchor, scale and Z rotation), the
/// way `occlusion` reads it. A layer turned in X or Y, or pushed in Z, measures
/// as if it lay flat; go through the compositor's own matrices if a 3D layer
/// ever needs measuring through a camera.
fn to_comp(
    comp: &Composition,
    layer: &Layer,
    t: f64,
    context: &ExpressionContext,
    rect: [f64; 4],
) -> [f64; 4] {
    let mut world = placement(layer, t, context);
    for id in crate::model::layer_parent_chain(comp, layer.id) {
        if let Some(parent) = comp.layers.iter().find(|l| l.id == id) {
            world = world.then(&placement(parent, t, context));
        }
    }
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[0], rect[3]),
        (rect[2], rect[3]),
    ];
    bounds(corners.into_iter().map(|c| world.apply(c))).unwrap_or(rect)
}

/// `layer`'s own flat placement at comp time `t`, in the renderer's order:
/// back by the anchor, scale, turn, then out to the position.
fn placement(layer: &Layer, t: f64, context: &ExpressionContext) -> Affine {
    let lt = crate::time::layer_time(t, layer.start_offset.0);
    let ctx = Arc::new(ExpressionContext {
        layer: Some(layer.id),
        comp_time: t,
        ..context.clone()
    });
    let tr = &layer.transform;
    let v = |p: &crate::anim::Property| p.value_at_with_context(lt, ctx.clone());
    let (sx, sy) = (v(&tr.scale_x) / 100.0, v(&tr.scale_y) / 100.0);
    let (sin, cos) = v(&tr.rotation).to_radians().sin_cos();
    Affine::translation(-v(&tr.anchor_x), -v(&tr.anchor_y))
        .then(&Affine([sx, 0.0, 0.0, sy, 0.0, 0.0]))
        .then(&Affine([cos, sin, -sin, cos, 0.0, 0.0]))
        .then(&Affine::translation(v(&tr.position_x), v(&tr.position_y)))
}

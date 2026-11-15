//! **Time effects that follow their layer's motion** (`lumit_core::fx::follow`).
//! Echo, Motion blur, Timewarp and Posterize time read their layer at other
//! moments. Read from the layer's own picture, those moments held nothing its
//! Position, Scale or Rotation keys move, nor what the effects above were
//! doing then, so a moving Text layer had to be precomposed first.
//!
//! Here the layer is drawn into a comp-sized picture as the effects above the
//! time effect leave it, placed by its own transform and its parents', the
//! way a group header wraps its members. That picture is built again at each
//! moment the effect reads: as neighbour frames for Echo and Motion blur, as
//! the pictures Timewarp shows, or held whole for Posterize time. The effect
//! and every effect after it, with the layer's styles, run on it in comp
//! space. The layer's blend mode and Matte apply to the finished picture.

use std::collections::HashMap;
use std::sync::Arc;

use lumit_core::fx::follow::{self, Follow};
use lumit_core::model::{BlendMode, Composition, Document, Layer, LayerKind};
use uuid::Uuid;

use super::{blend_of, comp_walk, moments::picture_of, strip_temporal_inputs, Walk};
use crate::decode::CompLayerPixels;
use crate::draw::{CompLayerDraw, DrawSource, GraphDraw, LayerInputDraw, PlaneDraw, RotoMatteDraw};

pub(super) use follow::split;

/// The layer and the moment it is wrapped at.
pub(super) struct Frame<'a> {
    pub doc: &'a Arc<Document>,
    pub comp: &'a Composition,
    pub layer: &'a Layer,
    /// Where its time effect sits in its stack, and how it reads.
    pub split: (usize, Follow),
    /// The comp time and the frame's own.
    pub t: (f64, f64),
    /// The layer time, the time its effects resolve at, and the frame's own.
    pub lt: (f64, f64, f64),
}

/// What the wrapped stack reads besides its ops, made by the walk's own
/// helpers.
pub(super) struct Extras {
    pub colour_tables: Vec<Option<crate::colour::TableRequest>>,
    pub dof_inputs: Vec<LayerInputDraw>,
    pub mattes: Vec<LayerInputDraw>,
    pub mask_paths: Vec<lumit_core::mask::MaskPolyline>,
    pub roto_mattes: Vec<Option<RotoMatteDraw>>,
    pub planes: Vec<Option<PlaneDraw>>,
    pub points_schedules: Vec<lumit_core::fx::points::PointsSchedule>,
    pub flare_lens_files: Vec<Option<lumit_core::fx::lens::LensSource>>,
    pub graph_fx: Vec<GraphDraw>,
}

/// A copy of `comp` in which only `layer` draws, as [`follow::inner`] leaves
/// it, with none of its own blend mode or Matte. The other layers stay,
/// hidden, so a parent, an expression or a camera still finds them; cameras
/// and lights stay switched on, since they light and see the layer.
fn inner(comp: &Composition, layer: &Layer, (at, how): (usize, Follow)) -> Composition {
    let mut inner = comp.clone();
    // A group header's effects run on the finished picture, outside.
    inner.groups.clear();
    for l in &mut inner.layers {
        l.switches.solo = false;
        if l.id == layer.id {
            *l = follow::inner(layer, at, how);
            l.matte = None;
            l.blend = BlendMode::Normal;
            l.switches.solo = false;
            l.switches.visible = true;
        } else if !matches!(l.kind, LayerKind::Camera { .. } | LayerKind::Light { .. }) {
            l.switches.visible = false;
        }
    }
    inner
}

/// The comp times the picture of `layer` is built at around comp time `t`,
/// for the planner, which fetches its footage there.
pub(crate) fn moments(comp: &Composition, layer: &Layer, t: f64) -> Vec<f64> {
    follow::times(comp, layer, t)
}

/// The layer's draw as a comp-sized picture of the layer as the effects above
/// its time effect leave it, with that picture at each moment the effect
/// reads, and the rest of the stack to run on them. `None` when the layer
/// draws nothing, so the ordinary path has its say.
#[allow(clippy::too_many_arguments)]
pub(super) fn wrap(
    f: Frame<'_>,
    pixels_by_layer: &HashMap<Uuid, &CompLayerPixels>,
    visited: &mut Vec<Uuid>,
    with: &Walk<'_>,
    resolve: impl Fn(&Layer, f64) -> (Vec<Uuid>, lumit_core::fx::params::ResolvedStack),
    extras: impl Fn(&Layer, f64) -> Extras,
) -> Option<CompLayerDraw> {
    let Frame {
        doc,
        comp,
        layer,
        split: (at, how),
        t: (t, frame_t),
        lt: (lt, effect_lt, _),
    } = f;
    let inner = inner(comp, layer, (at, how));
    let mut walk = |tau: f64, rebuild: bool| {
        comp_walk(
            doc,
            &inner,
            tau,
            frame_t,
            pixels_by_layer,
            visited,
            None,
            false,
            Walk {
                clone: with.clone,
                values: with.values,
                rebuild,
                seen_through: with.seen_through,
            },
            None,
        )
    };
    // A hold draws the whole layer at the time it holds, transform and all.
    let now = match how {
        Follow::Hold => follow::held_time(layer, t),
        Follow::Neighbours | Follow::Moments => t,
    };
    let members = walk(now, false);
    if members.is_empty() {
        return None;
    }
    let tail = follow::tail(layer, at, how);
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    // The same picture at another moment. A neighbour carries no neighbours
    // of its own.
    let mut then = |tau: f64| {
        let mut draws = walk(tau, true);
        strip_temporal_inputs(&mut draws);
        draws
    };
    let flow_below: Vec<_> = if how == Follow::Neighbours {
        follow::neighbour_times(&tail, t, lt, dt)
            .into_iter()
            .map(|(offset, tau, measure)| {
                (
                    offset,
                    then(tau),
                    Some(crate::track::camera_pose(doc, comp, tau)),
                    measure,
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    // Timewarp's pictures: the layer, placed, at each moment it shows. They
    // stand in for the layer's own at those moments, which hold none of its
    // motion. Timewarp is the first effect of the tail, so its slot is first.
    // On a frame where it shows the frame itself it reads no moments, and its
    // slot stays as it was.
    let moments = if how == Follow::Moments {
        follow::times(comp, layer, t)
    } else {
        Vec::new()
    };
    let shown: Option<LayerInputDraw> = (!moments.is_empty()).then(|| {
        LayerInputDraw::Several(
            moments
                .into_iter()
                .map(|tau| {
                    let camera = Some(crate::track::camera_pose(doc, comp, tau));
                    LayerInputDraw::Layer(picture_of(comp, then(tau), camera, tau))
                })
                .collect(),
        )
    });
    let (fx_ids, fx) = resolve(&tail, effect_lt);
    let mut ex = extras(&tail, effect_lt);
    if let (Some(shown), Some(slot)) = (shown, ex.dof_inputs.first_mut()) {
        *slot = shown;
    }
    let (w, h) = (comp.width as f32, comp.height as f32);
    Some(CompLayerDraw {
        layer: layer.id,
        source: DrawSource::Nested {
            width: comp.width,
            height: comp.height,
            // Transparent where the layer is not, as a Precomp's picture is.
            background: [0.0, 0.0, 0.0, 0.0],
            draws: members,
            camera: Some(crate::track::camera_pose(doc, comp, t)),
            key: None,
            paint: Vec::new(),
            paint_time: t,
            flow: None,
        },
        natural_size: (w, h),
        // Comp-sized and drawn where the layer already landed.
        position: (w * 0.5, h * 0.5),
        anchor: (w * 0.5, h * 0.5),
        scale: (100.0, 100.0),
        rotation_deg: 0.0,
        // The layer's opacity is in the picture, ghosts and all.
        opacity: 100.0,
        z: 0.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: 0.0,
        three_d: false,
        ends_run: false,
        matte: None,
        blend: blend_of(layer.blend),
        mask_cov: None,
        pre: None,
        fx,
        fx_ids,
        neighbours: Vec::new(),
        flow_fields: Vec::new(),
        colour_tables: ex.colour_tables,
        dof_inputs: ex.dof_inputs,
        mattes: ex.mattes,
        mask_paths: ex.mask_paths,
        roto_mattes: ex.roto_mattes,
        planes: ex.planes,
        points_schedules: ex.points_schedules,
        flare_lens_files: ex.flare_lens_files,
        graph_fx: ex.graph_fx,
        // Resolved at comp scale; the realiser rescales to what it renders at.
        fx_ref_width: Some(w),
        fx_input_key: None,
        mb: Vec::new(),
        lights: Vec::new(),
        material: Default::default(),
        temporal_below: None,
        accumulation_below: None,
        flow_below,
        in_scene: None,
    })
}

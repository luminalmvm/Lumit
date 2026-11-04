//! Depth points: a shot as points in the comp's 3D space, each one pushed along
//! its ray from a camera by how far away the shot's depth says it is.
//!
//! From that camera every point lands on its own pixel whatever its depth, so
//! the points line up with the footage, and from anywhere else they are the
//! shape the depth describes. Nothing is remembered between frames.
//!
//! The depth is this effect's own analysis, the same job and sidecar as the
//! Depth effect's, or a picture on another layer. With neither, the points sit
//! flat on the layer's plane and the status card says why.

use crate::fx::{
    params::ParamId, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, schema::EffectSchema, schema::EnabledCond, schema::EnabledWhen,
};
use crate::model::{Composition, Document, EffectInstance, Layer, LayerKind};
use crate::ops::Op;
use crate::track::{camera_pose_of, solve_store, Unsolved};
use lumit_fx_macros::Effect;
use std::sync::Arc;

/// The Depth layer takes over from the analysis, so what asks for one stands
/// aside. Cancel stays live, so a running job can still stop.
const ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "analyse",
        on: "depth_layer",
        cond: EnabledCond::LayerUnset,
    },
    EnabledWhen {
        param: "frames",
        on: "depth_layer",
        cond: EnabledCond::LayerUnset,
    },
];

/// Depth points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "depth_points",
    label = "Depth points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point can land anywhere in the frame.
    roi = FullFrame,
    premultiplied = true,
    // The depth is the only other picture it reads, and it has its own row.
    matte = false,
    enabled_when = ENABLED_WHEN,
)]
pub struct DepthPoints {
    /// Read the shot and file its depth. A button, not a value.
    #[action(label = "Analyse")]
    pub analyse: (),
    /// Stop a running analysis. The frames already read are kept.
    #[action(label = "Cancel")]
    pub cancel: (),
    /// Which frames Analyse reads - see [`crate::planes::FRAMES_OPTIONS`].
    #[choice(options = *crate::planes::FRAMES_OPTIONS, default = 0, label = "Frames")]
    pub frames: u32,
    /// A picture to read depth from instead, nearer being brighter. Unset, the
    /// points read this effect's own analysis.
    #[layer(label = "Depth layer", self_default = false)]
    pub depth_layer: bool,
    /// Read the depth the other way up, far being bright.
    #[toggle(default = false, label = "Invert")]
    pub invert: bool,
    /// The Camera layer the points line up from. Setting it carries the layer
    /// on that camera, so a shot follows the camera that filmed it. Unset, the
    /// comp's own camera where it was at the layer's first frame, so a camera
    /// moved after that sees round the points.
    #[layer(label = "Camera", self_default = false)]
    pub camera: bool,
    /// How far the points spread along their rays. Nought is flat on the
    /// layer's plane; 100% takes the furthest to twice its distance from the
    /// camera and the nearest to two thirds of it.
    #[slider(
        label = "Depth",
        min = 0.0,
        max = 190.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 190.0,
        unit = Percent
    )]
    pub amount: f32,
    /// How wide a point looks from that camera, px@comp.
    #[slider(
        label = "Point size",
        min = 1.0,
        max = 20.0,
        default = 2.0,
        hard_min = 1.0,
        hard_max = 100.0,
        unit = Px
    )]
    pub point_size: f32,
    /// How many of the picture's pixels get a point. Points stay where they are
    /// as this moves, more of them joining in.
    #[slider(
        label = "Density",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub density: f32,
}

impl DepthPoints {
    /// Raster pixels per comp pixel, read back out of a resolved bag.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");
}

/// Depth points' behaviour. What it draws needs the camera and the layer's
/// place in the comp, so it has no CPU reference through the trait.
pub struct DepthPointsDef;

impl EffectDef for DepthPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<DepthPoints as EffectMetadata>::SCHEMA
    }

    /// The Camera row is read for where the camera is, never for a picture.
    fn shape_inputs(&self) -> &'static [&'static str] {
        &["camera"]
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(DepthPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    /// A precomp's picture comes out at the render scale, and the raster factor
    /// has to follow it there.
    fn derived_spatial(&self) -> &'static [ParamId] {
        &[DepthPoints::DERIVED_PX_SCALE]
    }
}

/// Where a layer's points come from at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Source {
    /// The source camera's eye, comp pixels.
    pub eye: (f64, f64, f64),
    /// How far a layer carried on that camera slides along its view, comp
    /// pixels, so a zoom keeps the picture filling the frame.
    pub slide: (f64, f64, f64),
}

/// A Camera layer's pose at comp time `t`, however it gets there.
fn pose_of(
    doc: &Arc<Document>,
    comp: &Composition,
    camera: &Layer,
    t: f64,
) -> Option<crate::model::CameraPose> {
    let store = solve_store().unwrap_or(&Unsolved);
    camera_pose_of(doc, comp, camera, t, store).map(|linked| linked.pose)
}

/// Where `layer`'s points come from at comp time `t`. `named` is the Camera
/// row's layer. Without one, the comp's own camera as it was at the layer's first
/// frame, so moving it later shows the depth. A 3D view's camera is never the
/// comp's own, or the points would line up from wherever they are looked at.
#[must_use]
pub fn source(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    named: Option<uuid::Uuid>,
    t: f64,
) -> Source {
    let camera = |l: &&Layer| matches!(l.kind, LayerKind::Camera { .. });
    let named = named
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
        .filter(camera);
    let first = layer.in_point.0.to_f64();
    let own = comp.layers.iter().filter(camera).find(|l| {
        l.id != crate::view_camera::ID
            && l.switches.visible
            && first >= l.in_point.0.to_f64()
            && first < l.out_point.0.to_f64()
    });
    let pose = named.map_or_else(
        || own.and_then(|l| pose_of(doc, comp, l, first)),
        |l| pose_of(doc, comp, l, t),
    );
    let Some(pose) = pose else {
        // The view's camera was made for the comp before a viewport widened it,
        // so its point of interest is still the true middle.
        let middle = comp.layers.iter().find_map(|l| match &l.kind {
            LayerKind::Camera { options, .. } if l.id == crate::view_camera::ID => Some((
                options.point_of_interest[0].value_at(0.0),
                options.point_of_interest[1].value_at(0.0),
            )),
            _ => None,
        });
        let (w, h) = middle.map_or((f64::from(comp.width), f64::from(comp.height)), |(x, y)| {
            (x * 2.0, y * 2.0)
        });
        return Source {
            eye: crate::camera::default_pose(w, h).position,
            slide: (0.0, 0.0, 0.0),
        };
    };
    // Only a layer carried on the camera slides, by what its zoom has done since
    // the layer's first frame.
    let slide = named
        .filter(|l| layer.parent == Some(l.id))
        .and_then(|l| pose_of(doc, comp, l, first))
        .map_or((0.0, 0.0, 0.0), |at_first| {
            let lt = crate::time::layer_time(t, layer.start_offset.0);
            let by = layer.transform.position_z.value_at(lt)
                * (pose.zoom / at_first.zoom.max(1e-9) - 1.0);
            let f = crate::camera::forward(pose.rotation_deg);
            (f.0 * by, f.1 * by, f.2 * by)
        });
    Source {
        eye: pose.position,
        slide,
    }
}

/// What a frame's name owes Depth points' [`Source`], for the row the frame key
/// is walking. Its eye is not the active camera's, and a hidden parent or a solve
/// can move it with nothing else in the name changing.
#[must_use]
pub fn stamp(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: Option<&Layer>,
    e: &EffectInstance,
    id: &str,
    t: f64,
) -> Option<u64> {
    let layer =
        layer.filter(|_| id == "camera" && e.effect.match_name == crate::planes::DEPTH_POINTS)?;
    let s = source(doc, comp, layer, e.layer_ref("camera"), t);
    let mut h = blake3::Hasher::new();
    for v in [s.eye.0, s.eye.1, s.eye.2, s.slide.0, s.slide.1, s.slide.2] {
        h.update(&v.to_le_bytes());
    }
    let mut out = [0; 8];
    out.copy_from_slice(&h.finalize().as_bytes()[..8]);
    Some(u64::from_le_bytes(out))
}

/// `animation` with `by` added to every value. An expression is left as it is.
fn moved_by(animation: &crate::anim::Animation, by: f64) -> crate::anim::Animation {
    use crate::anim::Animation;
    match animation {
        Animation::Static(v) => Animation::Static(v + by),
        Animation::Keyframed(keys) => {
            let mut keys = keys.clone();
            for key in &mut keys {
                key.value += by;
            }
            Animation::Keyframed(keys)
        }
        Animation::Expression(text) => Animation::Expression(text.clone()),
    }
}

/// The op that gives `layer` the effect stack `effects`, as
/// [`super::particulate::set_effects`] makes it. Setting the Camera row also
/// carries the layer on that camera, where it sat in the frame, and clearing it
/// puts the layer back, all in one undo step. Switching the effect off leaves
/// the layer riding the camera, which is how you compare it with the plate.
#[must_use]
pub fn set_effects(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    effects: Vec<EffectInstance>,
) -> Op {
    let camera = |list: &[EffectInstance]| {
        list.iter()
            .find(|e| e.effect.match_name == crate::planes::DEPTH_POINTS)
            .and_then(|e| e.layer_ref("camera"))
    };
    let (was, now) = (camera(&layer.effects), camera(&effects));
    let set = super::particulate::set_effects(comp.id, layer, effects);
    if was == now {
        return set;
    }
    let first = layer.in_point.0.to_f64();
    // The zoom it is drawn with, as a solve can hold one far from the stored row.
    let zoom = |id: Option<uuid::Uuid>| {
        let camera = comp.layers.iter().find(|l| Some(l.id) == id)?;
        pose_of(doc, comp, camera, first).map(|pose| pose.zoom)
    };
    // From the comp's frame into the camera's, where the origin is the eye and
    // the plane it sees 1:1 is its zoom ahead.
    let (w, h) = (f64::from(comp.width) / 2.0, f64::from(comp.height) / 2.0);
    let (dx, dy) = match (was, now) {
        (None, Some(_)) => (-w, -h),
        (Some(_), None) => (w, h),
        _ => (0.0, 0.0),
    };
    let dz = zoom(now).unwrap_or(0.0) - zoom(was).unwrap_or(0.0);
    let tr = &layer.transform;
    let mut ops = vec![
        set,
        Op::SetLayerParent {
            comp: comp.id,
            layer: layer.id,
            parent: now,
        },
    ];
    for (prop, property, by) in [
        (crate::model::TransformProp::PositionX, &tr.position_x, dx),
        (crate::model::TransformProp::PositionY, &tr.position_y, dy),
        (crate::model::TransformProp::PositionZ, &tr.position_z, dz),
    ] {
        ops.push(Op::SetTransformProperty {
            comp: comp.id,
            layer: layer.id,
            prop,
            animation: moved_by(&property.animation, by),
        });
    }
    Op::Batch { ops }
}

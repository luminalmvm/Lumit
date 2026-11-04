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
use lumit_fx_macros::Effect;

/// The id the Viewer's view camera is inserted under: fixed, so every render
/// through the same view builds the same document and names the same frames.
pub const VIEW_CAMERA_ID: uuid::Uuid =
    uuid::Uuid::from_u128(0x6c75_6d69_7420_7669_6577_2063_616d_6572);

/// The Depth layer takes over from the analysis, so Frames stands aside.
const ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "frames",
    on: "depth_layer",
    cond: EnabledCond::LayerUnset,
}];

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
    /// The Camera layer the points line up from. Unset, the comp's own camera.
    #[layer(label = "Camera", self_default = false)]
    pub camera: bool,
    /// How far the points spread along their rays. Nought is flat on the
    /// layer's plane; 100% takes the furthest to twice its distance from the
    /// camera and the nearest to two thirds of it.
    #[slider(
        label = "Depth",
        min = 0.0,
        max = 100.0,
        default = 50.0,
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

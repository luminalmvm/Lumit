//! Projecting a picture on a Model layer in one step, and a card to project it
//! on with no file behind it.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::layer::{InstanceHome, LayerReference};
use crate::api::BridgeError;
use lumit_core::anim::Property;
use lumit_core::model::{Document, LayerKind, TransformGroup};
use std::sync::Arc;

impl LayerReference {
    /// The camera and picture Project from camera would use on this Model
    /// layer at comp frame `frame`, `None` while it can't.
    #[frb(ignore)]
    fn projection_source(&self, selected: &[Uuid], frame: i64) -> Option<(Uuid, Uuid)> {
        let (doc, layer) = (self.document().ok()?, self.item().ok()?);
        lumit_render::model_material::rows(&doc, &layer)?;
        let comp = doc.comp(self.comp_id)?;
        let t = comp.frame_rate.time_of_frame(frame).ok()?.0.to_f64();
        lumit_core::model_material::projection_source(comp, self.layer_id, selected, t)
    }

    /// Whether Project from camera has a camera and a picture to use.
    #[frb(sync)]
    pub fn can_project_from_camera(&self, selected: Vec<Uuid>, frame: i64) -> bool {
        self.projection_source(&selected, frame).is_some()
    }

    /// Project a picture on every material of this Model layer, as one undo
    /// step.
    #[frb(sync)]
    pub fn project_from_camera(&self, selected: Vec<Uuid>, frame: i64) -> Result<(), BridgeError> {
        let (camera, picture) = self
            .projection_source(&selected, frame)
            .ok_or(BridgeError::InvalidLayer)?;
        self.with_instances(InstanceHome::Materials, move |list| {
            for rows in list {
                lumit_core::model_material::set_projection(rows, picture, camera);
            }
            Ok(())
        })
    }
}

impl CompositionReference {
    /// Add a card filling the frame of the comp's camera at comp frame `frame`,
    /// square on to it.
    #[frb(sync)]
    pub fn add_card_layer(
        &self,
        row: Option<u32>,
        frame: i64,
    ) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let t = comp
            .frame_rate
            .time_of_frame(frame)
            .map_err(|_| BridgeError::InvalidTime)?;
        let doc = {
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let pose = lumit_render::track::camera_pose(&doc, &comp, t.0.to_f64());
        // A camera sees its frame one for one on the plane its zoom away.
        let turn = lumit_core::camera::rotation(pose.rotation_deg);
        let (x, y, z) = pose.position;
        let at = |k: usize, from: f64| Property::fixed(from + turn[k][2] * pose.zoom);
        let transform = TransformGroup {
            position_x: at(0, x),
            position_y: at(1, y),
            position_z: at(2, z),
            rotation_x: Property::fixed(pose.rotation_deg.0),
            rotation_y: Property::fixed(pose.rotation_deg.1),
            rotation: Property::fixed(pose.rotation_deg.2),
            ..TransformGroup::default()
        };
        let wide = f64::from(comp.width);
        self.add_at(
            lumit_core::model_layer::card_layer(&comp, wide, transform),
            row,
        )
    }
}

/// Where a layer made at solved points `tracks` of `tracked` goes at comp frame
/// `frame`, turned to face the camera, and the document it was worked out on.
pub(crate) fn placed_at_points(
    tracked: &LayerReference,
    tracks: &[u32],
    frame: i64,
) -> Result<(TransformGroup, Arc<Document>), BridgeError> {
    let comp = tracked.composition()?;
    let media = crate::api::track::tracked_media(tracked).ok_or(BridgeError::NotFootage)?;
    let at = lumit_render::track::point_centroid(media, tracks).ok_or(BridgeError::NoSolve)?;

    // The camera the layer is turned to face: the active one at this frame,
    // with its link followed, so a null made under a derived camera faces the
    // motion that is actually being drawn.
    let doc = {
        let proj = tracked.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        state.store.snapshot()
    };
    let t = comp
        .frame_rate
        .time_of_frame(frame)
        .map_err(|_| BridgeError::InvalidTime)?;
    let facing = doc.comp(tracked.comp_id).map_or((0.0, 0.0, 0.0), |c| {
        lumit_render::track::camera_pose(&doc, c, t.0.to_f64()).rotation_deg
    });
    // The solve's points sit in the space of the camera it drives, so a
    // parented camera takes them with it.
    let at = doc
        .comp(tracked.comp_id)
        .and_then(|c| {
            let camera = c.active_camera(t.0.to_f64()).filter(|l| {
                matches!(l.kind, lumit_core::model::LayerKind::Camera { solve_link: Some(id), .. } if id == tracked.layer_id)
            })?;
            let at = t.0.to_f64();
            let context = lumit_core::model::comp_context(&doc, c, at);
            let read = lumit_core::model::context_reader(&context, at);
            lumit_core::model::parent_space(c, camera, at, &read, lumit_render::track::solves_for(&doc))
        })
        .map_or(at, |space| {
            let p = space.point((at[0], at[1], at[2]));
            [p.0, p.1, p.2]
        });

    let transform = TransformGroup {
        position_x: Property::fixed(at[0]),
        position_y: Property::fixed(at[1]),
        position_z: Property::fixed(at[2]),
        rotation_x: Property::fixed(facing.0),
        rotation_y: Property::fixed(facing.1),
        rotation: Property::fixed(facing.2),
        ..TransformGroup::default()
    };
    Ok((transform, doc))
}

/// Add a card at solved points `tracks` of `tracked`, filling the frame of the
/// camera solved from it, with `tracked`'s picture projected from that camera.
/// One undo step, the camera too when there isn't one yet.
#[frb(sync)]
pub fn add_card_at_points(
    tracked: LayerReference,
    tracks: Vec<u32>,
    frame: i64,
) -> Result<LayerReference, BridgeError> {
    let comp = tracked.composition()?;
    let (transform, doc) = placed_at_points(&tracked, &tracks, frame)?;
    let on_it = |id: Option<Uuid>| id == Some(tracked.layer_id);
    let camera = comp
        .layers
        .iter()
        .find(|l| matches!(&l.kind, LayerKind::Camera { solve_link, .. } if on_it(*solve_link)));
    let mut ops = Vec::new();
    let camera_id = match camera {
        Some(camera) => camera.id,
        None => {
            let made = crate::api::track::solved_camera_layer(&comp, tracked.layer_id);
            let id = made.id;
            ops.push(lumit_core::ops::Op::AddLayer {
                comp: tracked.comp_id,
                index: 0,
                layer: Box::new(made),
            });
            id
        }
    };
    let media = crate::api::track::tracked_media(&tracked).ok_or(BridgeError::NotFootage)?;
    let solved = lumit_render::track::solved(media).ok_or(BridgeError::NoSolve)?;
    let pose = solved.pose(frame).ok_or(BridgeError::NoSolve)?;
    let wide = frame_width_at(&comp, pose, &transform);
    let mut layer = lumit_core::model_layer::card_layer(&comp, wide, transform);
    let id = lumit_core::model_material::material_id(layer.id, 0);
    let mut rows = lumit_render::model_material::file_rows(&doc, &layer, id)
        .ok_or(BridgeError::InvalidEffect)?;
    lumit_core::model_material::set_projection(&mut rows, tracked.layer_id, camera_id);
    if let LayerKind::Model { settings, .. } = &mut layer.kind {
        settings.materials = vec![Some(rows)];
    }
    crate::edits::solo_on_arrival(&mut layer, comp.layers.iter());
    let id = layer.id;
    ops.push(lumit_core::ops::Op::AddLayer {
        comp: tracked.comp_id,
        index: ops.len(),
        layer: Box::new(layer),
    });
    tracked.commit(lumit_core::ops::Op::Batch { ops })?;
    Ok(LayerReference::new(tracked.project_id, tracked.comp_id, id))
}

/// How wide `comp`'s frame is, seen from `pose`, at the depth `transform` sits.
fn frame_width_at(
    comp: &lumit_core::model::Composition,
    pose: lumit_core::model::CameraPose,
    transform: &TransformGroup,
) -> f64 {
    let at = |p: &Property| p.value_at(0.0);
    let (x, y, z) = pose.position;
    let away = [
        at(&transform.position_x) - x,
        at(&transform.position_y) - y,
        at(&transform.position_z) - z,
    ];
    let turn = lumit_core::camera::rotation(pose.rotation_deg);
    let depth: f64 = (0..3).map(|k| away[k] * turn[k][2]).sum();
    f64::from(comp.width) * depth.max(1.0) / pose.zoom.max(1.0)
}

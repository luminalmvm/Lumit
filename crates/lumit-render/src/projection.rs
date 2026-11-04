//! A material's base colour layer projected from a Camera layer: which camera
//! at which moment, read fresh for every frame, and which pictures and models
//! that takes.

use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, EffectInstance, Layer, LayerKind};
use lumit_core::model_layer::ModelSettings;
use lumit_core::model_material::{camera_at, projection, Projection};
use lumit_gpu::mesh::projector::view_of;
use lumit_scene::ModelScene;

use crate::draw::{DofInputDraw, LayerInputDraw};
use crate::model_material::{kept_for, materials_of, Look};

/// A material's camera as the mesh pass wants it.
pub(crate) struct Projector {
    /// Comp pixels to where the camera sees them ([`view_of`]).
    pub view: Mat4,
    /// The camera's frame to the picture's uvs.
    pub plate: Mat3,
    /// The models on this camera, each with its metres to comp pixels. What
    /// the nearest of them hides from the camera it never reaches what's
    /// behind, so a picture doesn't land twice.
    pub stand_ins: Vec<(Arc<ModelScene>, Mat4)>,
}

/// The moment a Model layer is being drawn at.
pub(crate) struct Moment<'a> {
    pub t_comp: f64,
    pub lt: f64,
    pub context: &'a Arc<ExpressionContext>,
}

/// Fill in `looks` for the materials `settings` project: each one's camera,
/// and the picture a held one holds. Nothing is asked of a layer with no rows
/// kept, so an ordinary model pays nothing.
pub(crate) fn aim(
    doc: &Arc<Document>,
    comp: &Composition,
    settings: &ModelSettings,
    scene: &ModelScene,
    at: Moment<'_>,
    looks: &mut [Look],
    held: &dyn Fn(&EffectInstance, &str, f64) -> Option<DofInputDraw>,
) {
    if settings.materials.is_empty() {
        return;
    }
    let materials = materials_of(scene);
    for look in looks {
        let Some(rows) = kept_for(settings, &materials, look.index) else {
            continue;
        };
        let Some(p) = projection(rows, comp, at.lt) else {
            continue;
        };
        if let (Some(t), Some(slot)) = (p.held, look.pictures.first_mut()) {
            *slot = held(rows, "base_colour_layer", t)
                .map_or(LayerInputDraw::Absent, LayerInputDraw::Layer);
        }
        // No picture at this moment leaves the file's own colour as it is.
        if let Some(natural) = look.pictures.first().and_then(LayerInputDraw::size) {
            look.projector = projector(doc, comp, &p, natural, &at);
        }
    }
}

fn projector(
    doc: &Arc<Document>,
    comp: &Composition,
    p: &Projection,
    natural: [f32; 2],
    at: &Moment<'_>,
) -> Option<Projector> {
    let time = p.time_at(at.t_comp);
    let find = |id| comp.layers.iter().find(|l| l.id == id);
    let pose = camera_at(doc, comp, find(p.camera)?, time)?;
    let camera = crate::export::camera_mat(comp.width, comp.height, pose);
    // The picture's own expressions, about it and at that moment.
    let context = Arc::new(ExpressionContext {
        comp_time: time,
        layer: Some(p.picture),
        ..(**at.context).clone()
    });
    Some(Projector {
        view: view_of(camera, (comp.width, comp.height), pose.zoom as f32),
        plate: plate(comp, find(p.picture)?, natural, time, context)?,
        stand_ins: stand_ins(doc, comp, p.camera, at),
    })
}

/// The camera's frame to the uvs of picture layer `src`, `natural` pixels in
/// size, so it lands as it sits in the comp and a plate scaled to fit still
/// fits. A 3D plate has no one place in the frame, so it fills it. `None` for
/// one scaled to nothing.
fn plate(
    comp: &Composition,
    src: &Layer,
    natural: [f32; 2],
    time: f64,
    context: Arc<ExpressionContext>,
) -> Option<Mat3> {
    let stretch = |x: f32, y: f32| Mat3::from_scale(glam::vec2(x, y));
    let [nw, nh] = natural.map(|v| v.max(1.0));
    let frame_to_comp = stretch(comp.width as f32, comp.height as f32);
    let comp_to_layer = match src.switches.three_d {
        true => stretch(nw / comp.width as f32, nh / comp.height as f32),
        false => {
            let m = Mat4::from_cols_array_2d(&crate::placement::world_placement(
                comp, src, time, context,
            ));
            let flat = |c: glam::Vec4| Vec3::new(c.x, c.y, c.w);
            let place = Mat3::from_cols(flat(m.x_axis), flat(m.y_axis), flat(m.w_axis));
            if !place.determinant().is_normal() {
                return None;
            }
            place.inverse()
        }
    };
    Some(stretch(1.0 / nw, 1.0 / nh) * comp_to_layer * frame_to_comp)
}

/// The Model layers showing at this moment with a material on `camera`, each
/// where it is now, so a held camera's picture stays on what it saw.
fn stand_ins(
    doc: &Arc<Document>,
    comp: &Composition,
    camera: uuid::Uuid,
    at: &Moment<'_>,
) -> Vec<(Arc<ModelScene>, Mat4)> {
    let shows = |l: &&Layer| {
        l.switches.visible
            && at.t_comp >= l.in_point.0.to_f64()
            && at.t_comp < l.out_point.0.to_f64()
    };
    comp.layers
        .iter()
        .filter(shows)
        .filter_map(|l| {
            let LayerKind::Model { item, settings } = &l.kind else {
                return None;
            };
            let lt = lumit_core::time::layer_time(at.t_comp, l.start_offset.0);
            let mut kept = settings.materials.iter().flatten();
            kept.find(|rows| projection(rows, comp, lt).is_some_and(|p| p.camera == camera))?;
            let scene = crate::model::scene(doc, *item)?;
            let size = (comp.width as f32, comp.height as f32);
            let model = crate::model::model_to_comp(settings, &scene.bounds, size);
            let context = Arc::new(ExpressionContext {
                layer: Some(l.id),
                ..(**at.context).clone()
            });
            let placed = crate::placement::world_placement(comp, l, at.t_comp, context);
            let to_comp = Mat4::from_cols_array_2d(&placed) * Mat4::from_cols_array_2d(&model);
            Some((scene, to_comp))
        })
        .collect()
}

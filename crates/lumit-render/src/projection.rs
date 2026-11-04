//! A material's base colour layer projected from a Camera layer: which camera
//! at which moment, read fresh for every frame, and which pictures and models
//! that takes.

use std::cell::RefCell;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, EffectInstance, Layer, LayerKind};
use lumit_core::model_layer::ModelSettings;
use lumit_core::model_material::{camera_at, projection, Projection};
use lumit_gpu::mesh::projector::view_of;
use lumit_scene::ModelScene;

use crate::draw::{DofInputDraw, LayerInputDraw};
use crate::model::Pictures;
use crate::model_material::{kept_for, materials_of, Look};

/// The row a projection reads its picture from.
const PICTURE_ROW: &str = "base_colour_layer";

/// A material's camera as the mesh pass wants it.
pub(crate) struct Projector {
    /// Comp pixels to where the camera sees them ([`view_of`]).
    pub view: Mat4,
    /// The camera's frame to the picture's uvs.
    pub plate: Mat3,
    /// Where the camera is, in comp pixels.
    pub eye: Vec3,
    /// The model's metres to comp pixels at the moment the camera is read.
    pub surface: Mat4,
    /// Whether that moment is a held one, so the picture stays on the model.
    pub held: bool,
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
    /// The layer, and where it is at `t_comp`.
    pub layer: &'a Layer,
    pub placement: [[f32; 4]; 4],
}

/// A projected picture by its layer and the moment it holds, if it holds.
type PlateKey = (uuid::Uuid, Option<u64>);

fn plate_key(p: &Projection) -> PlateKey {
    (p.picture, p.held.map(f64::to_bits))
}

/// What a Model layer's material rows read as pictures, this frame. A held
/// row reads its held frame and nothing at this one, and materials projecting
/// one picture share the first's.
pub(crate) struct Plates<'a> {
    comp: &'a Composition,
    lt: f64,
    pictures: Pictures<'a>,
    fetched: RefCell<Vec<PlateKey>>,
}

impl<'a> Plates<'a> {
    pub(crate) fn new(comp: &'a Composition, lt: f64, pictures: Pictures<'a>) -> Self {
        Self {
            comp,
            lt,
            pictures,
            fetched: RefCell::new(Vec::new()),
        }
    }

    /// Row `row` of `rows` as a picture.
    pub(crate) fn fetch(&self, rows: &EffectInstance, row: &str) -> Option<DofInputDraw> {
        let (now, held) = self.pictures;
        let found = projection(rows, self.comp, self.lt).filter(|_| row == PICTURE_ROW);
        let Some(p) = found else {
            return now(rows, row);
        };
        let key = plate_key(&p);
        let mut fetched = self.fetched.borrow_mut();
        if fetched.contains(&key) {
            return None;
        }
        fetched.push(key);
        drop(fetched);
        p.held
            .map_or_else(|| now(rows, row), |t| held(rows, row, t))
    }
}

/// Fill in `looks` for the materials `settings` project: each one's camera,
/// and which earlier material made its picture. Nothing is asked of a layer
/// with no rows kept, so an ordinary model pays nothing.
pub(crate) fn aim(
    doc: &Arc<Document>,
    comp: &Composition,
    settings: &ModelSettings,
    scene: &ModelScene,
    at: &Moment<'_>,
    looks: &mut [Look],
) {
    if settings.materials.is_empty() {
        return;
    }
    let materials = materials_of(scene);
    let mut made: Vec<(PlateKey, usize, [f32; 2])> = Vec::new();
    for (k, look) in looks.iter_mut().enumerate() {
        let rows = kept_for(settings, &materials, look.index);
        let Some(p) = rows.and_then(|rows| projection(rows, comp, at.lt)) else {
            continue;
        };
        let key = plate_key(&p);
        let natural = match look.pictures.first().and_then(LayerInputDraw::size) {
            Some(natural) => {
                made.push((key, k, natural));
                Some(natural)
            }
            None => made.iter().find(|m| m.0 == key).map(|m| {
                look.plate_from = Some(m.1);
                m.2
            }),
        };
        // No picture at this moment leaves the file's own colour as it is.
        look.projector = natural.and_then(|n| projector(doc, comp, settings, scene, &p, n, at));
    }
}

fn projector(
    doc: &Arc<Document>,
    comp: &Composition,
    settings: &ModelSettings,
    scene: &ModelScene,
    p: &Projection,
    natural: [f32; 2],
    at: &Moment<'_>,
) -> Option<Projector> {
    let time = p.time_at(at.t_comp);
    let find = |id| comp.layers.iter().find(|l| l.id == id);
    let pose = camera_at(doc, comp, find(p.camera)?, time)?;
    let camera = crate::export::camera_mat(comp.width, comp.height, pose);
    let size = (comp.width as f32, comp.height as f32);
    let model =
        Mat4::from_cols_array_2d(&crate::model::model_to_comp(settings, &scene.bounds, size));
    let placed = match p.held {
        Some(_) => placed_at(comp, at.layer, time, at.context),
        None => at.placement,
    };
    let (x, y, z) = pose.position;
    Some(Projector {
        view: view_of(camera, (comp.width, comp.height), pose.zoom as f32),
        plate: plate(comp, find(p.picture)?, natural, time, at.context)?,
        eye: Vec3::new(x as f32, y as f32, z as f32),
        surface: Mat4::from_cols_array_2d(&placed) * model,
        held: p.held.is_some(),
        stand_ins: stand_ins(doc, comp, p.camera, time, at),
    })
}

/// Layer `l`'s own pixels to comp pixels at comp time `time`, its expressions
/// about it and at that moment.
fn placed_at(
    comp: &Composition,
    l: &Layer,
    time: f64,
    context: &Arc<ExpressionContext>,
) -> [[f32; 4]; 4] {
    let context = Arc::new(ExpressionContext {
        comp_time: time,
        layer: Some(l.id),
        ..(**context).clone()
    });
    crate::placement::world_placement(comp, l, time, context)
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
    context: &Arc<ExpressionContext>,
) -> Option<Mat3> {
    let stretch = |x: f32, y: f32| Mat3::from_scale(glam::vec2(x, y));
    let [nw, nh] = natural.map(|v| v.max(1.0));
    let frame_to_comp = stretch(comp.width as f32, comp.height as f32);
    let comp_to_layer = match src.switches.three_d {
        true => stretch(nw / comp.width as f32, nh / comp.height as f32),
        false => {
            let m = Mat4::from_cols_array_2d(&placed_at(comp, src, time, context));
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

/// The Model layers on `camera` that show at comp time `time`, each where it
/// is at that moment, as that is what the camera saw.
fn stand_ins(
    doc: &Arc<Document>,
    comp: &Composition,
    camera: uuid::Uuid,
    time: f64,
    at: &Moment<'_>,
) -> Vec<(Arc<ModelScene>, Mat4)> {
    let shows = |l: &&Layer| {
        l.switches.visible && time >= l.in_point.0.to_f64() && time < l.out_point.0.to_f64()
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
            let placed = placed_at(comp, l, time, at.context);
            let to_comp = Mat4::from_cols_array_2d(&placed) * Mat4::from_cols_array_2d(&model);
            Some((scene, to_comp))
        })
        .collect()
}

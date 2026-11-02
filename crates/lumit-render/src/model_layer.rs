//! A Model layer in the realiser: the mesh pass's picture and depth for one
//! Model draw, and the order of a run of 3D layers that holds one. The
//! compositor joins the two with
//! [`lumit_gpu::composite::Compositor::composite_joined`].

use crate::draw::CompLayerDraw;
use crate::realise::Realiser;
use glam::{Mat4, Vec3};
use lumit_gpu::composite::{order::cut_by_depth, Cut};
use lumit_gpu::mesh::{ModelPicture, ModelView};
use lumit_scene::ModelScene;
use std::sync::Arc;

impl Realiser<'_> {
    /// One Model draw seen through the comp's camera, or the default one when
    /// it has none, and lit by `l`'s lights as `l`'s Material options take
    /// them, at this render's scale and anti-aliasing. `model_to_comp` takes
    /// the model's metres to comp pixels, column-major, the half turn about X
    /// in it. `None` when the card can't hold the model, which leaves it out
    /// of the frame rather than failing it.
    pub fn model_picture(
        &self,
        l: &CompLayerDraw,
        scene: &Arc<ModelScene>,
        model_to_comp: [[f32; 4]; 4],
        camera: Option<lumit_core::model::CameraPose>,
        (width, height): (u32, u32),
    ) -> Option<ModelPicture> {
        let pose = camera.unwrap_or_else(|| {
            lumit_core::camera::default_pose(f64::from(width), f64::from(height))
        });
        let eye = [pose.position.0, pose.position.1, pose.position.2].map(|v| v as f32);
        let view = ModelView {
            comp: (width, height),
            render_scale: self.render_scale,
            samples: self.samples,
            camera: crate::export::camera_mat(width, height, pose),
            model_to_comp: Mat4::from_cols_array_2d(&model_to_comp),
            lights: &l.lights,
            material: lumit_gpu::fx::lighting::LightingMaterial { eye, ..l.material },
        };
        self.fx.model(&self.ctx, scene, &view).ok()
    }
}

/// How a run of 3D layers that holds a model draws: cut by depth throughout,
/// each layer by the distance to its middle and each model by the distance to
/// its own. `worlds` is each draw's placed quad
/// ([`lumit_gpu::composite::CompositeLayer::world`]), `middles` a model's
/// middle in comp pixels where the draw is one, `flat` the motion-blurred
/// ones, and `view_proj` the comp's camera.
pub fn model_run_order(
    worlds: &[Mat4],
    middles: &[Option<Vec3>],
    flat: &[bool],
    view_proj: Mat4,
) -> Vec<(usize, Option<Cut>)> {
    let depths: Vec<f32> = worlds
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let middle = middles
                .get(i)
                .copied()
                .flatten()
                .unwrap_or_else(|| m.transform_point3(Vec3::new(0.5, 0.5, 0.0)));
            (view_proj * middle.extend(1.0)).w
        })
        .collect();
    cut_by_depth(&depths, flat)
}

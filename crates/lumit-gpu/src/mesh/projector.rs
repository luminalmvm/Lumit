//! A material's base colour read from a camera rather than off the model's
//! uvs. The shader works out where the camera saw each point and reads the
//! picture there (`projected` in `mesh.wgsl`), and a depth like a shadow map's says
//! whether it saw that point at all, so a picture never lands on what is
//! behind the surface it was meant for.
//!
//! Nothing here runs for a material on its uvs, which only binds an empty depth.

use super::shadow::{box_corners, MapCaster};
use super::MeshFx;
use crate::fx::FxEngine;
use crate::GpuContext;
use glam::{Mat3, Mat4, Vec3};

/// A camera as the shader reads it.
#[derive(Clone)]
pub struct Projector {
    /// Comp pixels to where the camera sees them ([`view_of`]).
    pub view: Mat4,
    /// The camera's frame, 0 to 1 across and down, to the picture's uvs.
    pub plate: Mat3,
    /// What the camera sees first, when there is anything.
    pub depth: Option<ProjectorDepth>,
}

/// How far the nearest surface is from a camera at every texel.
#[derive(Clone)]
pub struct ProjectorDepth {
    pub(super) texture: wgpu::Texture,
    /// The nearest depth kept and how far past that the farthest is, in comp
    /// pixels.
    pub(super) range: [f32; 2],
}

/// [`crate::composite::camera_matrix`] made into what a shadow map's view is:
/// across and down the camera's frame times w, the depth in comp pixels, and w.
#[must_use]
pub fn view_of(camera: Mat4, (w, h): (u32, u32), zoom: f32) -> Mat4 {
    let rows = camera.transpose();
    Mat4::from_cols(
        rows.x_axis / w.max(1) as f32,
        rows.y_axis / h.max(1) as f32,
        rows.w_axis * zoom.max(1.0),
        rows.w_axis,
    )
    .transpose()
}

impl Projector {
    /// The numbers the material's uniform carries: `view`, then `plate`, then
    /// x = 1 to project, y and z the depth's range with z nought for no depth.
    pub(super) fn uniform(p: Option<&Self>) -> ([[f32; 4]; 4], [[f32; 4]; 3], [f32; 4]) {
        let Some(p) = p else {
            return Default::default();
        };
        let plate = p.plate.to_cols_array_2d().map(|c| [c[0], c[1], c[2], 0.0]);
        let [near, range] = p.depth.as_ref().map_or([0.0; 2], |d| d.range);
        (p.view.to_cols_array_2d(), plate, [1.0, near, range, 0.0])
    }
}

impl FxEngine {
    /// How far `stand_ins` are from the camera in `view`, for a picture `size`
    /// pixels in size. `None` when none of them is in front of it.
    ///
    /// Two thirds of the picture's size, so the half a texel the shader reads
    /// past a point is three quarters of a pixel, further than the corner of a
    /// pixel is from its middle.
    pub fn projector_depth(
        &self,
        ctx: &GpuContext,
        stand_ins: &[MapCaster<'_>],
        view: Mat4,
        size: (u32, u32),
    ) -> Option<ProjectorDepth> {
        let fx = self.mesh.get_or_init(|| MeshFx::new(ctx));
        let models: Vec<_> = stand_ins
            .iter()
            .filter_map(|c| Some((fx.model_for(ctx, c.scene).ok()?, c)))
            .collect();
        let depths: Vec<f32> = stand_ins
            .iter()
            .flat_map(box_corners)
            .map(|p: Vec3| (view * p.extend(1.0)).z)
            .filter(|z| z.is_finite())
            .collect();
        let far = depths.iter().copied().reduce(f32::max)? + 1.0;
        let near = (depths.iter().copied().fold(far, f32::min) - 1.0).max(0.5);
        if far <= near {
            return None;
        }
        let range = (near, far - near);
        let map = (size.0.div_ceil(3) * 2, size.1.div_ceil(3) * 2);
        Some(ProjectorDepth {
            texture: fx.draw_one_view(ctx, &models, view, range, map),
            range: [range.0, range.1],
        })
    }
}

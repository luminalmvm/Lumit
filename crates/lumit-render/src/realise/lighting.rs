//! A finished layer lit by the comp's lights and blurred by the camera's
//! depth of field.

use super::{dof, Realiser};
use crate::draw::{CompLayerDraw, DrawSource};

impl Realiser<'_> {
    /// A finished layer lit by the comp's lights (docs/06) and then blurred by
    /// the camera's depth of field (docs/impl/camera.md §5), each only where
    /// it applies, so a comp with neither renders byte-for-byte as before.
    /// `casters` are the segment's shadows, `own` this layer's square
    /// among them, `maps` the models' shadows and `environment` the comp's,
    /// which lights a 3D layer that takes light; `grow` is how much its
    /// effects grew it and `size` the comp's, which the depth of field reads.
    /// Hands back the picture, how much it has grown now, and the lit picture
    /// before the blur with its own grow when it was blurred. Particles in
    /// the scene come with their `depth`.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub(super) fn lit(
        &self,
        l: &CompLayerDraw,
        mut tex: wgpu::Texture,
        eye: [f32; 3],
        camera: Option<lumit_core::model::CameraPose>,
        (casters, own, maps, environment): (
            Option<&lumit_gpu::fx::lighting::ShadowCasters>,
            Option<usize>,
            Option<&lumit_gpu::mesh::shadow::ShadowMaps>,
            Option<&lumit_gpu::fx::environment::EnvironmentLight>,
        ),
        grow: (f32, f32),
        size: (u32, u32),
        depth: Option<&wgpu::Texture>,
    ) -> (
        wgpu::Texture,
        (f32, f32),
        Option<(wgpu::Texture, (f32, f32))>,
    ) {
        // The mesh pass lit a model already, and its depth of field comes from
        // the comp's depth.
        if matches!(l.source, DrawSource::Model(_)) {
            return (tex, grow, None);
        }
        if let Some(depth) = depth {
            return (
                self.scene_lit(l, tex, depth, camera, size, casters),
                grow,
                None,
            );
        }
        // After its effects and before it is placed. `l.lights` is empty
        // unless the comp holds lights and this layer accepts them, and empty
        // means the pass never runs. An unlit layer only takes shadows, so
        // with no casters there is nothing to do, unless it shows only them
        // and so has to be cleared.
        let unshaded = l.material.unlit
            && casters.is_none()
            && maps.is_none()
            && l.material.accepts_shadows != lumit_gpu::fx::lighting::LightingShadows::Only;
        let environment = environment.filter(|_| l.three_d && !l.material.unlit);
        let (w, h) = (tex.width(), tex.height());
        let op = lighting_op(l, w, h, eye, environment.is_some()).filter(|_| !unshaded);
        if let Some(mut op) = op {
            op.own_caster = own;
            tex = self.fx.lighting(
                &self.ctx,
                &tex,
                tex.width(),
                tex.height(),
                &op,
                casters,
                maps,
                environment,
            );
        }
        // Depth of field (docs/impl/camera.md §5): each part of a 3D layer
        // off the camera's focus plane is blurred by its own circle of
        // confusion. None unless the camera carries one.
        let Some((reach, field, step, room)) =
            dof::dof_blur(camera, l, &tex, grow, size, self.render_scale)
        else {
            return (tex, grow, None);
        };
        // Room round the layer for its outline to blur into, or a solid would
        // come out as sharp as it went in.
        let (w, h) = (tex.width() + room.0 * 2, tex.height() + room.1 * 2);
        let padded = lumit_gpu::fx::fit_centred(&self.ctx, tex.clone(), w, h);
        let blurred = self
            .fx
            .blur_field(&self.ctx, &padded, w, h, reach, field, step);
        let wider = (
            grow.0 * w as f32 / tex.width() as f32,
            grow.1 * h as f32 / tex.height() as f32,
        );
        (blurred, wider, Some((tex, grow)))
    }
}

/// The lighting pass's parameters for one draw (docs/06), or `None`
/// when nothing lights it — which is the common case and the reason a comp
/// without Light layers pays nothing. `surround` is an environment lighting
/// it, which needs the pass with no lights at all.
///
/// The layer's plane is described to the kernel as an affine frame rather than
/// a matrix: where texel (0, 0) sits in comp pixels, and how far one texel of
/// movement carries you in each direction. That is all a placement is (the
/// camera's perspective comes later, when the lit layer is projected), and it
/// works out the same whether the layer is being rendered at full resolution
/// or a quarter of it, because the steps are measured against the raster this
/// pass is actually running on.
pub(super) fn lighting_op(
    l: &crate::draw::CompLayerDraw,
    w: u32,
    h: u32,
    eye: [f32; 3],
    surround: bool,
) -> Option<lumit_gpu::fx::lighting::LightingOp> {
    use crate::build::shading::{place_dir, place_point, unit};

    if (l.lights.is_empty() && !surround) || w == 0 || h == 0 {
        return None;
    }
    // A 2D layer is drawn flat at z = 0 whatever its transform says, because
    // no camera reads those fields. Shading it at its stored depth would light
    // a layer that is not where the light was told it is.
    let (z, rx, ry) = if l.three_d {
        (l.z, l.rotation_x_deg, l.rotation_y_deg)
    } else {
        (0.0, 0.0, 0.0)
    };
    let mut place = lumit_gpu::composite::place_matrix(
        l.position,
        l.anchor,
        l.scale,
        l.rotation_deg,
        z,
        rx,
        ry,
    );
    if let Some(pre) = &l.pre {
        place = lumit_gpu::composite::concat_place(*pre, place);
    }
    let (nw, nh) = l.natural_size;
    let origin = place_point(&place, 0.0, 0.0, 0.0);
    let du = place_dir(&place, nw / w as f32, 0.0, 0.0);
    let dv = place_dir(&place, 0.0, nh / h as f32, 0.0);
    Some(lumit_gpu::fx::lighting::LightingOp {
        origin,
        du,
        dv,
        // Wound so an untransformed layer faces the viewer: this model puts
        // increasing z away from the camera, as After Effects does, so the
        // side a light has to be on is the negative one.
        normal: unit(cross(dv, du)),
        material: lumit_gpu::fx::lighting::LightingMaterial { eye, ..l.material },
        lights: l.lights.clone(),
        own_caster: None,
    })
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

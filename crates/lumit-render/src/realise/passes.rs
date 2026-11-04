//! A 3D precomp's scene passes for the effects that read them: its distance
//! from the camera, which way it faces, where it is, and its parts.

use super::shadows::shadow_only;
use super::{placed_in_3d, Realiser};
use crate::draw::{CompLayerDraw, DrawSource, NestedInputDraw};
use glam::{Mat4, Vec3};
use lumit_core::fx::effects::scene_pass::{PassKind, SceneRead};
use lumit_gpu::mesh::passes::{PassLook, PassModel, PassPlane, PassView};

impl Realiser<'_> {
    /// **Scene depth**: a Precomp's 3D layers as how far each pixel is from its
    /// camera, metres, which is what a lens effect's Depth row reads from a
    /// Precomp. Each layer's picture is made as for colour, so what its effects
    /// cut away is not there. Adjustment and 2D layers draw nothing, so a comp
    /// with no 3D layer is all far away.
    pub fn realise_scene_depth(
        &self,
        n: &crate::draw::NestedInputDraw,
        capture: Option<&mut Option<lumit_gpu::fx::lens_blur::SceneLayers>>,
    ) -> wgpu::Texture {
        // ponytail: 2D layers are still made here and only dropped by the depth
        // pass. Skip them before making them if a heavy 2D layer ever shows.
        // With no 3D layer nothing is made, unless rays want the 2D ones as
        // their backdrop.
        let made = capture.is_some() || n.draws.iter().any(|d| d.three_d);
        let draws: &[CompLayerDraw] = if n.camera.is_some() && made {
            crate::environment::peel(&n.draws).1
        } else {
            &[]
        };
        let metres_per_w = n
            .camera
            .map_or(1.0, |c| (c.zoom / n.px_per_metre.max(1e-6)) as f32);
        let (tw, th) = lumit_gpu::composite::scaled_size(n.width, n.height, self.render_scale);
        self.open_frame(tw, th, draws);
        let out = self.realise_segment(
            n.camera,
            n.width,
            n.height,
            [0.0; 4],
            draws,
            &None,
            None,
            Some(metres_per_w),
            capture,
            None,
            None,
            None,
        );
        self.ctx.end_frame();
        out
    }

    /// Normal, Position, Part id or a Part matte of the comp's 3D layers. Each
    /// 3D layer's picture is made as for colour, so what its effects cut away
    /// is not there. A model draws from its mesh, so its own effects have no
    /// say here, and 2D layers draw nothing, as for depth. A group of 3D layers
    /// covers the groups below it, as it does in the picture. `None` for a
    /// matte of parts none of which is there.
    pub(super) fn realise_scene_pass(
        &self,
        n: &NestedInputDraw,
        read: &SceneRead,
    ) -> Option<wgpu::Texture> {
        let (width, height) = (n.width, n.height);
        let draws = crate::environment::peel(&n.draws).1;
        let (_, draws) = crate::haze::peel(draws);
        let (tw, th) = lumit_gpu::composite::scaled_size(width, height, self.render_scale);
        self.open_frame(tw, th, draws);
        let pose = n.camera.unwrap_or_else(|| {
            lumit_core::camera::default_pose(f64::from(width), f64::from(height))
        });
        let shown = |l: &&CompLayerDraw| {
            l.three_d && l.opacity > 0.0 && !shadow_only(l) && !super::scene::draws_in_scene(l)
        };
        // Each draw's group, the way the picture cuts its runs of 3D layers.
        let mut next = 0;
        let groups: Vec<u32> = draws
            .iter()
            .map(|l| {
                let alone = !l.three_d || l.ends_run;
                next += u32::from(alone);
                let group = next;
                next += u32::from(alone);
                group
            })
            .collect();
        let in_groups = || draws.iter().zip(groups.iter().copied());
        let pictures: Vec<_> = in_groups()
            .filter(|(l, _)| shown(l) && !matches!(l.source, DrawSource::Model(_)))
            .map(|(l, group)| {
                let (tex, grow, _) =
                    self.layer_picture(l, n.camera, (width, height), (None, None, None), &mut None);
                (l, tex, grow, group)
            })
            .collect();
        let planes: Vec<PassPlane<'_>> = pictures
            .iter()
            .map(|(l, tex, grow, group)| PassPlane {
                texture: tex,
                world: placed_in_3d(tex, l, *grow).world(),
                opacity: (l.opacity / 100.0).clamp(0.0, 1.0),
                id: lumit_scene::parts::layer_number(l.layer.as_bytes()),
                group: *group,
            })
            .collect();
        let looks: Vec<(&CompLayerDraw, Vec<_>, u32)> = in_groups()
            .filter(|(l, _)| shown(l))
            .filter_map(|(l, group)| match &l.source {
                DrawSource::Model(m) => {
                    Some((l, m.looks.iter().map(|k| self.look(k)).collect(), group))
                }
                _ => None,
            })
            .collect();
        let models: Vec<PassModel<'_>> = looks
            .iter()
            .filter_map(|(l, looks, group)| match &l.source {
                DrawSource::Model(m) => Some(PassModel {
                    scene: &m.scene,
                    model_to_comp: crate::model_layer::to_comp(m),
                    layer: *l.layer.as_bytes(),
                    looks,
                    poses: m
                        .poses
                        .get(m.poses.len() / 2)
                        .map_or(&[][..], std::slice::from_ref),
                    group: *group,
                }),
                _ => None,
            })
            .collect();
        let view = PassView {
            comp: (width, height),
            render_scale: self.render_scale,
            camera: crate::export::camera_mat(width, height, pose),
            eye: Vec3::new(
                pose.position.0 as f32,
                pose.position.1 as f32,
                pose.position.2 as f32,
            ),
            to_space: to_metres(n, read.from_camera, pose),
            look: match read.pass {
                PassKind::Position => PassLook::Position,
                PassKind::PartId => PassLook::PartId,
                PassKind::Matte => PassLook::Matte,
                PassKind::Normal | PassKind::Depth => PassLook::Normal,
            },
            parts: &read.parts,
            planes: &planes,
            models: &models,
        };
        let out = self.fx.scene_pass(&self.ctx, &view);
        self.ctx.end_frame();
        out
    }
}

/// Comp pixels to metres, x right, y up and z towards the viewer: from the
/// comp's middle, or from the camera.
fn to_metres(n: &NestedInputDraw, from_camera: bool, pose: lumit_core::model::CameraPose) -> Mat4 {
    let (w, h) = (n.width as f32, n.height as f32);
    let per_metre = (n.px_per_metre as f32).max(1e-6);
    let scale = Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0) / per_metre);
    if !from_camera {
        return scale * Mat4::from_translation(Vec3::new(-w / 2.0, -h / 2.0, 0.0));
    }
    let frame = lumit_gpu::composite::camera_frame(
        w,
        h,
        pose.zoom as f32,
        (
            pose.position.0 as f32,
            pose.position.1 as f32,
            pose.position.2 as f32,
        ),
        (
            pose.rotation_deg.0 as f32,
            pose.rotation_deg.1 as f32,
            pose.rotation_deg.2 as f32,
        ),
    );
    // The camera's frame has its eye behind the comp's middle.
    let zoom = pose.zoom.max(1.0) as f32;
    scale * Mat4::from_translation(Vec3::new(-w / 2.0, -h / 2.0, zoom)) * frame
}

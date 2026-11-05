//! Volumetric light and smoke in the draw list (`lumit_core::haze`).
//!
//! When a comp's lights show in the air or it has a box of smoke, the walk puts
//! one haze draw just above its top 3D layer, or under everything when it has
//! none. The realiser ends a segment there, as it does at an adjustment layer,
//! and lays the haze over everything below before going on with the 2D layers
//! above. A comp with neither has no haze draw and runs none of this.

use std::sync::Arc;

use lumit_gpu::fx::lighting::{CasterPlane, LightingLight, LightingShadows};
use lumit_gpu::fx::volumetric::Haze;

use crate::build::shading::{caster_plane, lighting_light};
use crate::draw::{CompLayerDraw, DrawSource};
use crate::realise::Realiser;

/// What a haze draw shows: the comp's lights, each with its haze, and its boxes
/// of smoke. A light that only lights the smoke has a Volumetric of nought.
#[derive(Clone)]
pub struct HazeLights {
    pub(crate) lights: Vec<(LightingLight, Haze)>,
    pub(crate) volumes: Vec<crate::sim::smoke::VolumeDraw>,
}

/// Put the comp's haze draw just above its top 3D layer, or under every layer
/// when it has none, if any of its lights show in the air at `t`.
pub(crate) fn add(
    draws: &mut Vec<CompLayerDraw>,
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    t: f64,
) {
    let volumes = crate::sim::smoke::volumes(doc, comp, t);
    // A smoke layer's own picture never shows, only its box does.
    if !volumes.is_empty() {
        draws.retain(|d| {
            matches!(d.source, DrawSource::Model(_)) || !volumes.iter().any(|v| v.layer == d.layer)
        });
    }
    let lit = comp
        .layers
        .iter()
        .any(|l| matches!(l.kind, lumit_core::model::LayerKind::Light { .. }));
    if !lit && volumes.is_empty() {
        return;
    }
    let context = lumit_core::model::comp_context(doc, comp, t);
    let read = lumit_core::model::context_reader(&context, t);
    let every = comp.lights_at(t, &read, crate::track::solves_for(doc));
    use lumit_core::model::LightKind;
    let in_air =
        |l: &&lumit_core::model::ResolvedLight| l.haze.amount > 0.0 && l.kind != LightKind::Ambient;
    // Those that show in the air first, so a full list drops the others.
    let also = every.iter().filter(|l| !volumes.is_empty() && !in_air(l));
    let lights: Vec<(LightingLight, Haze)> = every
        .iter()
        .filter(in_air)
        .chain(also)
        .take(lumit_gpu::fx::lighting::MAX_LIT_LIGHTS)
        .map(|l| {
            let h = if in_air(&l) {
                l.haze
            } else {
                lumit_core::haze::HazeLook {
                    colour: [1.0; 3],
                    ..Default::default()
                }
            };
            let mut light = lighting_light(l, false);
            // The haze's colour tints only what it scatters.
            light.colour = [0, 1, 2].map(|c| light.colour[c] * h.colour[c]);
            let haze = Haze {
                amount: h.amount,
                density: h.density,
                falloff: h.falloff,
                scatter: h.scatter,
                noise: h.noise,
                noise_size: h.noise_size,
                drift: h.drift,
            };
            (light, haze)
        })
        .collect();
    if lights.is_empty() && volumes.is_empty() {
        return;
    }
    // Above the top 3D layer, and above the layers under the top box of smoke,
    // whose layer may be the only 3D one and has no draw of its own.
    let at_index = |id: uuid::Uuid| comp.layers.iter().position(|l| l.id == id);
    let top_box = volumes.iter().filter_map(|v| at_index(v.layer)).min();
    let at = draws
        .iter()
        .rposition(|d| {
            d.three_d || top_box.is_some_and(|k| at_index(d.layer).is_some_and(|i| i > k))
        })
        .map_or(0, |i| i + 1);
    let haze = HazeLights { lights, volumes };
    draws.insert(at, CompLayerDraw::over_comp(comp, DrawSource::Haze(haze)));
}

/// The haze draw a segment ends with, if it has one, and the layers under it.
pub(crate) fn peel(layers: &[CompLayerDraw]) -> (Option<&HazeLights>, &[CompLayerDraw]) {
    match layers.split_last() {
        Some((
            CompLayerDraw {
                source: DrawSource::Haze(lights),
                ..
            },
            below,
        )) => (Some(lights), below),
        _ => (None, layers),
    }
}

impl Realiser<'_> {
    /// How far the 3D in `layers` is from the camera `cam` at each pixel, as
    /// clip w counted from the returned near. The near sits just short of the
    /// nearest corner, so a view from far off (an orthographic one is a
    /// hundred comps away) keeps a pixel's worth of precision where the
    /// layers are. `wide` keeps a depth past one at any bit depth, for boxes of
    /// smoke, which the haze alone has never needed.
    pub(crate) fn view_depth(
        &self,
        layers: &[lumit_gpu::composite::CompositeLayer<'_>],
        models: &[lumit_gpu::mesh::ModelPicture],
        cam: glam::Mat4,
        (width, height): (u32, u32),
        wide: bool,
    ) -> (wgpu::Texture, f32) {
        let mut near = f32::INFINITY;
        // A model's quad is the whole frame, not where the model is.
        let model = |l: &&lumit_gpu::composite::CompositeLayer<'_>| {
            models.iter().any(|m| &m.colour == l.texture)
        };
        for l in layers.iter().filter(|l| l.three_d && !model(l)) {
            let m = cam * l.world();
            for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                near = near.min(m.col(0).w * u + m.col(1).w * v + m.col(3).w);
            }
        }
        let near = if near.is_finite() && near > 0.0 {
            near * 0.999
        } else {
            0.0
        };
        let depth = if wide {
            self.compositor.composite_depth_wide(
                &self.ctx,
                width,
                height,
                &[layers],
                cam,
                near,
                self.render_scale,
                models,
            )
        } else {
            self.compositor.composite_depth_joined(
                &self.ctx,
                width,
                height,
                &[layers],
                cam,
                1.0,
                near,
                self.render_scale,
                None,
                models,
            )
        };
        (depth, near)
    }

    /// `frame` with the haze of `lights` laid over it, if there is any, seen
    /// through `camera`. `drawn` is the segment's draws with each one's
    /// finished picture and grow, `placed` the same as the compositor draws
    /// them, `models` the models among them, which the haze stops at, and
    /// `maps` their shadows, which cut it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn haze_over(
        &self,
        frame: wgpu::Texture,
        lights: Option<&HazeLights>,
        (layers, textures, grown): (&[CompLayerDraw], &[wgpu::Texture], &[(f32, f32)]),
        placed: &[lumit_gpu::composite::CompositeLayer<'_>],
        (models, maps): (
            &[lumit_gpu::mesh::ModelPicture],
            Option<&lumit_gpu::mesh::shadow::ShadowMaps>,
        ),
        camera: Option<lumit_core::model::CameraPose>,
        (width, height): (u32, u32),
    ) -> wgpu::Texture {
        use lumit_gpu::fx::lighting::{DIFFUSION_SPREAD, MAX_SHADOW_CASTERS, MAX_STRETCH};
        let Some(HazeLights { lights, volumes }) = lights else {
            return frame;
        };
        let pose = camera.unwrap_or_else(|| {
            lumit_core::camera::default_pose(f64::from(width), f64::from(height))
        });
        let cam = crate::export::camera_mat(width, height, pose);
        // Walked here so the frame's cancelling reaches it.
        let mut boxes = crate::sim::smoke::walked_volumes(self.fx, &self.ctx, volumes);
        let (depth, near) =
            self.view_depth(placed, models, cam, (width, height), !boxes.is_empty());
        // The shafts: every casting 3D layer cuts every light that casts.
        let shadowed = lights.iter().any(|(l, _)| l.shadow_darkness > 0.0);
        let casting: Vec<(&wgpu::Texture, CasterPlane)> = layers
            .iter()
            .zip(textures)
            .zip(grown)
            .filter(|((l, t), _)| {
                // A model's shafts come from its shadow maps, and particles in
                // the scene cast none.
                shadowed
                    && l.three_d
                    && !models.iter().any(|m| &m.colour == *t)
                    && l.material.casts_shadows != LightingShadows::Off
                    && l.opacity > 0.0
            })
            .take(MAX_SHADOW_CASTERS)
            .map(|((l, t), g)| (t, caster_plane(t, l, *g)))
            .collect();
        let longest = casting
            .iter()
            .map(|(t, _)| t.width().max(t.height()))
            .max()
            .unwrap_or(0);
        let spread = lights
            .iter()
            .map(|(l, _)| l.shadow_diffusion * DIFFUSION_SPREAD * MAX_STRETCH)
            .fold(0.0, f32::max);
        // ponytail: made again here when the layers' own shadows made them
        // too. Share the squares if a profile ever shows it.
        let casters = self.fx.shadow_casters(
            &self.ctx,
            &casting,
            (longest.saturating_mul(4) / 3)
                .next_power_of_two()
                .clamp(64, 1024),
            false,
            spread,
        );
        // The lit haze is kept round the lights and the 3D layers, so a view
        // from far off still spends its steps where they are.
        let mut points: Vec<[f32; 3]> = lights
            .iter()
            .filter(|(l, _)| l.kind != lumit_gpu::fx::lighting::LightingKind::Parallel)
            .map(|(l, _)| [0, 1, 2].map(|k| l.corners.iter().map(|c| c[k]).sum::<f32>() * 0.25))
            .collect();
        for l in placed.iter().filter(|l| l.three_d) {
            let m = l.world();
            for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                points.push(
                    (m.col(0) * u + m.col(1) * v + m.col(3))
                        .truncate()
                        .to_array(),
                );
            }
        }
        let (w, h) = (width as f32, height as f32);
        if points.is_empty() {
            points.push([w * 0.5, h * 0.5, 0.0]);
        }
        let lo = [0, 1, 2].map(|k| points.iter().map(|p| p[k]).fold(f32::MAX, f32::min));
        let hi = [0, 1, 2].map(|k| points.iter().map(|p| p[k]).fold(f32::MIN, f32::max));
        let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
        let half = [0, 1, 2].map(|k| (hi[k] - lo[k]) * 0.5);
        let radius = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt() + w.max(h);
        let [right, up, forward] = lumit_core::camera::axes(pose.rotation_deg);
        let v = |p: (f64, f64, f64)| [p.0 as f32, p.1 as f32, p.2 as f32];
        // More boxes than one walk takes go in lots, furthest first, so the
        // nearer ones cover the further.
        let eye_at = glam::Vec3::from(v(pose.position));
        let distance = |b: &lumit_gpu::fx::volumetric::VolumeOp| {
            glam::Vec3::from(b.look.from_box[3]).distance_squared(eye_at)
        };
        boxes.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
        // A surface that takes no light or no shadow keeps out of smoke's shadow.
        let no_shadow = if boxes.is_empty() {
            None
        } else {
            let refused: Vec<&wgpu::Texture> = layers
                .iter()
                .zip(textures)
                .filter(|(l, _)| {
                    l.three_d
                        && (l.material.unlit || l.material.accepts_shadows == LightingShadows::Off)
                })
                .map(|(_, t)| t)
                .collect();
            let them: Vec<_> = placed
                .iter()
                .filter(|p| p.three_d && refused.contains(&p.texture))
                .cloned()
                .collect();
            (!them.is_empty()).then(|| {
                self.compositor.composite_depth_wide(
                    &self.ctx,
                    width,
                    height,
                    &[&them],
                    cam,
                    near,
                    self.render_scale,
                    models,
                )
            })
        };
        let lots = lumit_gpu::fx::volumetric::MAX_VOLUMES;
        let extra = boxes.len().saturating_sub(1) / lots;
        let base = lumit_gpu::fx::volumetric::HazeOp {
            eye: v(pose.position),
            right: v(right),
            up: v(up),
            forward: v(forward),
            zoom: pose.zoom.max(1.0) as f32,
            comp: (w, h),
            scale: frame.width() as f32 / width.max(1) as f32,
            near,
            centre,
            radius,
            lights: lights.clone(),
            volumes: Vec::new(),
            no_shadow,
        };
        let mut picture = frame;
        for k in 0..=extra {
            let lot = boxes.iter().skip(k * lots).take(lots).cloned().collect();
            let mut op = base.clone();
            op.volumes = lot;
            // The haze only needs walking once.
            if k < extra {
                for (_, h) in &mut op.lights {
                    h.amount = 0.0;
                }
            }
            picture = self
                .fx
                .haze(&self.ctx, &picture, &depth, &op, casters.as_ref(), maps);
        }
        picture
    }
}

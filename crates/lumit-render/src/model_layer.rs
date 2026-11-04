//! A Model layer in the realiser: the mesh pass's picture and depth for one
//! Model draw, and the order of a run of 3D layers that holds one. The
//! compositor joins the two with
//! [`lumit_gpu::composite::Compositor::composite_joined`].

use crate::draw::{CompLayerDraw, DrawSource};
use crate::model::{Behind, ModelDraw};
use crate::realise::Realiser;
use glam::{Mat4, Vec3};
use lumit_gpu::composite::{order::cut_by_depth, Cut};
use lumit_gpu::fx::lighting::{LightingShadows, ShadowCasters};
use lumit_gpu::mesh::display::Shading;
use lumit_gpu::mesh::shadow::{MapCaster, ShadowMaps};
use lumit_gpu::mesh::{ModelPicture, ModelView};

impl Realiser<'_> {
    /// Model draw `m` seen through the comp's camera, or the default one when
    /// it has none, and lit by `l`'s lights as `l`'s Material options take
    /// them and by the comp's environment, with the 3D layers' and models'
    /// shadows, at this render's scale and anti-aliasing. `None` when the
    /// card can't hold the model, which leaves it out of the frame rather
    /// than failing it, or when it is only there to cast. A holdout draws
    /// only the shadows it takes, if any, to lay over what is under its run.
    pub fn model_picture(
        &self,
        l: &CompLayerDraw,
        m: &ModelDraw,
        camera: Option<lumit_core::model::CameraPose>,
        (width, height): (u32, u32),
        (casters, maps, environment): (
            Option<&ShadowCasters>,
            Option<&ShadowMaps>,
            Option<&lumit_gpu::fx::environment::EnvironmentLight>,
        ),
    ) -> Option<ModelPicture> {
        // Only there to cast, unless it is a holdout, which is never seen, or
        // a view is showing the models themselves.
        let display = crate::model::display();
        let only = l.material.casts_shadows == LightingShadows::Only;
        if only && !m.holdout && display.takes_shadows() {
            return None;
        }
        let pose = camera.unwrap_or_else(|| {
            lumit_core::camera::default_pose(f64::from(width), f64::from(height))
        });
        let eye = [pose.position.0, pose.position.1, pose.position.2].map(|v| v as f32);
        let shutter: Vec<Mat4> = m
            .shutter
            .iter()
            .map(|p| Mat4::from_cols_array_2d(p) * Mat4::from_cols_array_2d(&m.model_to_comp))
            .collect();
        let takes = !m.holdout || l.material.accepts_shadows != LightingShadows::Off;
        // The glass is left out of the picture it bends.
        let behind = match m.behind.lock().as_deref() {
            Ok(Behind::LeftOut) => return None,
            Ok(Behind::Bends(picture)) => Some(picture.clone()),
            _ => None,
        };
        // Lines show no material, so they make no layer's picture.
        let wire = display.shading == Shading::Wireframe;
        let looks: Vec<_> = m
            .looks
            .iter()
            .filter(|_| !wire)
            .map(|k| self.look(k))
            .collect();
        let view = ModelView {
            comp: (width, height),
            render_scale: self.render_scale,
            samples: self.samples,
            camera: crate::export::camera_mat(width, height, pose),
            model_to_comp: to_comp(m),
            shutter: &shutter,
            shutter_scenes: &m.shutter_scenes,
            lights: &l.lights,
            material: lumit_gpu::fx::lighting::LightingMaterial {
                eye,
                accepts_shadows: match m.holdout {
                    true => LightingShadows::Only,
                    false => l.material.accepts_shadows,
                },
                ..l.material
            },
            instances: &[],
            casters: casters.filter(|_| takes),
            // ponytail: a holdout takes no model's shadow, as the maps hold its
            // own and the footage has those already. A map without it if a
            // model's shadow on a stand-in is missed.
            maps: maps.filter(|_| takes && !m.holdout),
            display,
            environment,
            looks: &looks,
            behind: behind.as_ref(),
            poses: &m.poses,
        };
        let picture = self.fx.model(&self.ctx, &m.scene, &view).ok()?;
        // A holdout stands in for something in the footage, sharp or not.
        let lens = camera.filter(|_| !m.holds_out()).and_then(|cam| {
            let (bounds, placed) = (&m.scene.bounds, to_comp(m));
            crate::realise::dof::model_lens(cam, bounds, placed, (width, height), self.render_scale)
        });
        let picture = match lens {
            Some(lens) => self.fx.model_lens(&self.ctx, &picture, &lens),
            None => picture,
        };
        Some(ModelPicture {
            holdout: m.holds_out(),
            ..picture
        })
    }

    /// The shadow maps of the models in `layers` that cast, for the lights
    /// that cast on what takes shadows or show in `haze`. `None`, and nothing
    /// made, when no model casts or a view shows models without shadows.
    ///
    /// ponytail: made again every frame. Keep them while the models and
    /// lights hold still if they show in a profile.
    pub(crate) fn shadow_maps(
        &self,
        layers: &[CompLayerDraw],
        haze: Option<&crate::haze::HazeLights>,
    ) -> Option<ShadowMaps> {
        if !crate::model::display().takes_shadows() {
            return None;
        }
        // ponytail: a faded model casts its whole shadow, as a depth can't
        // carry how see-through it is. Fade it into the maps if it's missed.
        let casters: Vec<MapCaster> = layers
            .iter()
            .filter(|l| l.material.casts_shadows != LightingShadows::Off && l.opacity > 0.0)
            .filter_map(|l| match &l.source {
                DrawSource::Model(m) => Some(MapCaster {
                    scene: &m.scene,
                    model_to_comp: to_comp(m),
                    pose: m.poses.get(m.poses.len() / 2),
                }),
                _ => None,
            })
            .collect();
        if casters.is_empty() {
            return None;
        }
        let lights: Vec<_> = layers
            .iter()
            .filter(|l| l.material.accepts_shadows != LightingShadows::Off)
            .flat_map(|l| l.lights.iter().copied())
            .chain(haze.into_iter().flatten().map(|(l, _)| *l))
            .collect();
        // About a texel a pixel at 1080p, fewer at a smaller preview.
        let side = ((2048.0 * self.render_scale) as u32)
            .next_power_of_two()
            .clamp(512, 2048);
        self.fx.shadow_maps(&self.ctx, &casters, &lights, side)
    }

    /// A material's rows for the mesh pass, the layers they name made into
    /// pictures at their own size.
    fn look(&self, look: &crate::model_material::Look) -> lumit_gpu::mesh::look::Look {
        let picture = |slot: &crate::draw::LayerInputDraw| {
            let [w, h] = slot.size()?;
            let px = |v: f32| ((v * self.render_scale).ceil() as u32).clamp(1, 8192);
            let made =
                self.render_layer_inputs(std::slice::from_ref(slot), px(w), px(h), (w, h), None);
            match made.into_iter().next()? {
                crate::fxops::LayerInput::Texture(t) => Some(t),
                _ => None,
            }
        };
        lumit_gpu::mesh::look::Look {
            index: look.index,
            material: look.material.clone(),
            pictures: std::array::from_fn(|k| look.pictures.get(k).and_then(picture)),
            textures: look.textures,
        }
    }
}

/// How many layers from `i` on make a run of 3D layers.
fn run_from(layers: &[CompLayerDraw], i: usize) -> usize {
    let in_run = |l: &CompLayerDraw| l.three_d && !l.ends_run;
    let starts = layers.get(i).is_some_and(in_run) && (i == 0 || !in_run(&layers[i - 1]));
    match starts {
        true => layers[i..].iter().take_while(|l| in_run(l)).count(),
        false => 0,
    }
}

/// Whether a run of 3D layers starts at `i` with a model of glass in it,
/// which bends the comp's picture under the run, so that is made first.
pub(crate) fn glass_starts(layers: &[CompLayerDraw], i: usize) -> bool {
    (layers.iter().skip(i).take(run_from(layers, i))).any(|l| glass(l).is_some())
}

impl Realiser<'_> {
    /// Lay the picture behind the glass in the run of 3D layers that begins
    /// at `i`, if it has any: `seed`, the comp under the run, with the run
    /// drawn over it without its glass. Cards in front of the glass are in it
    /// too, but they cover the glass by depth.
    // ponytail: the run is drawn twice, so glass costs about an adjustment
    // layer plus the run again. Drawing the run's pictures once and
    // compositing them twice is the upgrade.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lay_behind(
        &self,
        layers: &[CompLayerDraw],
        i: usize,
        seed: &Option<wgpu::Texture>,
        camera: Option<lumit_core::model::CameraPose>,
        (width, height): (u32, u32),
        background: [f64; 4],
        region: Option<lumit_gpu::composite::Region>,
        environment: Option<&lumit_gpu::fx::environment::EnvironmentLight>,
    ) {
        let run = &layers[i..i + run_from(layers, i)];
        if !run.iter().any(|l| glass(l).is_some()) {
            return;
        }
        let lay = |behind: &dyn Fn() -> Behind| {
            for m in run.iter().filter_map(glass) {
                if let Ok(mut b) = m.behind.lock() {
                    *b = behind();
                }
            }
        };
        lay(&|| Behind::LeftOut);
        let picture = self.realise_segment(
            camera,
            width,
            height,
            background,
            run,
            seed,
            region,
            None,
            None,
            None,
            None,
            environment,
        );
        lay(&|| Behind::Bends(picture.clone()));
    }
}

/// `l`'s model if it has glass that bends what is behind it: only as it
/// exports, and never one that only shows or casts shadows or holds out.
pub(crate) fn glass(l: &CompLayerDraw) -> Option<&ModelDraw> {
    let DrawSource::Model(m) = &l.source else {
        return None;
    };
    let surface = !m.holdout
        && l.material.accepts_shadows != LightingShadows::Only
        && l.material.casts_shadows != LightingShadows::Only;
    let bends = surface && crate::model::display().takes_shadows();
    (bends && m.looks.iter().any(|k| k.material.transmission > 0.0)).then_some(m)
}

/// A Model draw's metres to comp pixels, parents and all.
fn to_comp(m: &ModelDraw) -> Mat4 {
    Mat4::from_cols_array_2d(&m.placement) * Mat4::from_cols_array_2d(&m.model_to_comp)
}

/// The middle of a Model draw's box in comp pixels, `None` for any other draw.
pub(crate) fn middle(l: &CompLayerDraw) -> Option<Vec3> {
    let DrawSource::Model(m) = &l.source else {
        return None;
    };
    let bounds = m.poses.first().map_or(m.scene.bounds, |p| p.bounds);
    let (lo, hi) = (Vec3::from(bounds.min), Vec3::from(bounds.max));
    Some(to_comp(m).transform_point3((lo + hi) / 2.0))
}

/// Each model's picture after its effects, with the depth the mesh pass
/// drew beside it. `depths` names each model by its place in `pictures`,
/// and whether it is a holdout. An effect that grew the picture is cropped
/// back to the frame, so each pixel still lines up with its depth.
pub(crate) fn joined(
    ctx: &lumit_gpu::GpuContext,
    pictures: &mut [wgpu::Texture],
    depths: Vec<(usize, wgpu::Texture, bool)>,
) -> Vec<ModelPicture> {
    depths
        .into_iter()
        .filter_map(|(i, depth, holdout)| {
            let colour = pictures.get_mut(i)?;
            if (colour.width(), colour.height()) != (depth.width(), depth.height()) {
                *colour =
                    lumit_gpu::fx::fit_centred(ctx, colour.clone(), depth.width(), depth.height());
            }
            Some(ModelPicture {
                colour: colour.clone(),
                depth,
                holdout,
            })
        })
        .collect()
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

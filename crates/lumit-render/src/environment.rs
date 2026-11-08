//! An Environment light in the draw list (`lumit_core::environment`).
//!
//! When a comp has one, the walk takes its source layer's draw out of the
//! list, so the picture lights the scene without being drawn in it, sets it
//! flat at its own size and puts it at the head of the list. The realiser
//! takes it off there before anything else, makes the cubes the 3D layers and
//! models are lit by, and with Show in background draws them behind
//! everything. A comp with no Environment light has no such draw.

use glam::Mat3;
use lumit_gpu::fx::environment::EnvironmentLight;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::draw::{CompLayerDraw, DrawSource};
use crate::realise::Realiser;

/// An Environment light's picture and how it lights.
pub struct EnvironmentDraw {
    /// The source layer's own draw, flat at its own size.
    pub source: CompLayerDraw,
    /// The content name of its picture, `None` to make the cubes every frame.
    pub key: Option<u128>,
    pub colour: [f32; 3],
    pub rotation_deg: (f64, f64, f64),
    pub background: bool,
    pub blur: f32,
}

/// Take every layer an Environment light names out of `draws`, lit or not,
/// and put the comp's environment at their head if it has one.
pub(crate) fn add(
    draws: &mut Vec<CompLayerDraw>,
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    t: f64,
    pixels_by_layer: &HashMap<Uuid, &crate::decode::CompLayerPixels>,
) {
    use lumit_core::model::{LayerKind, LightKind};
    let any = comp.layers.iter().any(
        |l| matches!(&l.kind, LayerKind::Light { light } if light.kind == LightKind::Environment),
    );
    if !any {
        return;
    }
    let pictured = |d: &CompLayerDraw| {
        matches!(
            d.source,
            DrawSource::Pixels { .. } | DrawSource::Nested { .. } | DrawSource::Graph(_)
        )
    };
    let (named, rest): (Vec<_>, Vec<_>) = std::mem::take(draws)
        .into_iter()
        .partition(|d| pictured(d) && comp.feeds_environment(d.layer));
    *draws = rest;
    let context = lumit_core::model::comp_context(doc, comp, t);
    let read = lumit_core::model::context_reader(&context, t);
    let Some(e) = comp.environment_at(t, &read, crate::track::solves_for(doc)) else {
        return;
    };
    // One out of its span was never drawn, and so lights nothing.
    let Some(mut source) = named.into_iter().find(|d| d.layer == e.source) else {
        return;
    };
    // A plain solid that reflects it is drawn at its own size, or its
    // reflection would be eight texels across.
    for d in draws
        .iter_mut()
        .filter(|d| d.three_d && d.material.reflection > 0.0)
    {
        crate::build::shading::at_own_size(d);
    }
    let (w, h) = source.natural_size;
    source.position = (w * 0.5, h * 0.5);
    source.anchor = (w * 0.5, h * 0.5);
    source.scale = (100.0, 100.0);
    (
        source.rotation_deg,
        source.rotation_x_deg,
        source.rotation_y_deg,
    ) = (0.0, 0.0, 0.0);
    (source.z, source.three_d, source.ends_run, source.opacity) = (0.0, false, false, 100.0);
    source.blend = lumit_gpu::composite::Blend::Normal;
    (source.matte, source.pre) = (None, None);
    source.mb.clear();
    source.lights.clear();
    source.material = Default::default();
    // A still's picture is the same at every moment, so its cubes are too.
    let input = match (
        &source.source,
        comp.layers.iter().find(|l| l.id == e.source),
    ) {
        (DrawSource::Pixels { tex_w, tex_h, .. }, Some(layer)) => {
            let raster = (*tex_w, *tex_h);
            crate::build::fx_key::still_input_key(
                doc,
                layer,
                pixels_by_layer,
                t,
                raster,
                &source.fx,
            )
        }
        _ => source.fx_input_key,
    };
    let key = picture_key(&source, input);
    let look = EnvironmentDraw {
        source,
        key,
        colour: e.colour,
        rotation_deg: e.rotation_deg,
        background: e.background,
        blur: e.blur,
    };
    draws.insert(
        0,
        CompLayerDraw::over_comp(comp, DrawSource::Environment(Box::new(look))),
    );
}

/// The content name of the source's picture: its own and its effects', when
/// they read nothing but it. `None` makes the cubes again every frame.
fn picture_key(d: &CompLayerDraw, input: Option<u128>) -> Option<u128> {
    let alone = d.dof_inputs.is_empty()
        && d.mattes.is_empty()
        && d.colour_tables.is_empty()
        && d.flare_lens_files.is_empty()
        && d.roto_mattes.is_empty()
        && d.planes.is_empty()
        && d.graph_fx.is_empty()
        && d.neighbours.is_empty()
        && d.flow_fields.is_empty()
        && d.mask_paths.is_empty()
        && d.points_schedules.is_empty();
    let key = input.filter(|_| alone)?;
    let mut hash = blake3::Hasher::new();
    hash.update(&key.to_le_bytes());
    for op in d.fx.iter() {
        op.feed_hash(&mut |b| {
            hash.update(b);
        });
    }
    Some(u128::from_le_bytes(
        hash.finalize().as_bytes()[..16].try_into().ok()?,
    ))
}

/// The environment a draw list starts with, if it has one, and the rest.
pub(crate) fn peel(layers: &[CompLayerDraw]) -> (Option<&EnvironmentDraw>, &[CompLayerDraw]) {
    match layers.split_first() {
        Some((
            CompLayerDraw {
                source: DrawSource::Environment(e),
                ..
            },
            rest,
        )) => (Some(e), rest),
        _ => (None, layers),
    }
}

/// A turn in the compositor's `Ry · Rx · Rz` order, as it places a layer.
fn turn((x, y, z): (f64, f64, f64)) -> Mat3 {
    Mat3::from_rotation_y((y as f32).to_radians())
        * Mat3::from_rotation_x((x as f32).to_radians())
        * Mat3::from_rotation_z((z as f32).to_radians())
}

impl Realiser<'_> {
    /// What `d` lights with: its picture's cubes, made now or kept from a
    /// frame of the same picture at the same size.
    pub(crate) fn environment_light(&self, d: &EnvironmentDraw) -> EnvironmentLight {
        let (w, h) = (
            d.source.natural_size.0.max(1.0) as u32,
            d.source.natural_size.1.max(1.0) as u32,
        );
        let (tw, th) = lumit_gpu::composite::scaled_size(w, h, self.render_scale);
        let key = d.key.map(|k| {
            let mut hash = blake3::Hasher::new();
            hash.update(b"environment/");
            hash.update(&k.to_le_bytes());
            hash.update(&tw.to_le_bytes());
            hash.update(&th.to_le_bytes());
            let mut name = [0u8; 16];
            name.copy_from_slice(&hash.finalize().as_bytes()[..16]);
            u128::from_le_bytes(name)
        });
        let maps = match key.and_then(|k| self.fx.kept_environment(k)) {
            Some(maps) => maps,
            None => {
                let picture = self.realise(None, w, h, [0.0; 4], std::slice::from_ref(&d.source));
                self.fx.environment(&self.ctx, key, &picture)
            }
        };
        // The light's turn takes the picture round with it, so a comp
        // direction is turned back by it to find the picture's.
        let back = turn(d.rotation_deg).transpose();
        EnvironmentLight {
            maps,
            colour: d.colour,
            turn: [back.x_axis, back.y_axis, back.z_axis].map(|c| c.to_array()),
        }
    }

    /// The environment behind everything, through `camera` or the default
    /// one, at the comp's size. `None` from a view so far off it is flat,
    /// where every ray would read the one colour, so its own backdrop stays.
    pub(crate) fn environment_background(
        &self,
        light: &EnvironmentLight,
        d: &EnvironmentDraw,
        camera: Option<lumit_core::model::CameraPose>,
        (width, height): (u32, u32),
    ) -> Option<wgpu::Texture> {
        let pose = camera.unwrap_or_else(|| {
            lumit_core::camera::default_pose(f64::from(width), f64::from(height))
        });
        // The fixed views stand a hundred comps away.
        if pose.zoom > 50.0 * f64::from(width) {
            return None;
        }
        let ray = turn(pose.rotation_deg);
        Some(self.fx.environment_background(
            &self.ctx,
            light,
            (
                [ray.x_axis, ray.y_axis, ray.z_axis].map(|c| c.to_array()),
                pose.zoom.max(1.0) as f32,
            ),
            (width, height),
            self.render_scale,
            d.blur,
        ))
    }
}

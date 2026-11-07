//! Camera and Light layers from what a 3D model's file holds, keyed a frame at
//! a time so they follow it through its take, its Retime and its layer's moves.

use std::sync::Arc;

use glam::{EulerRot, Mat3, Mat4, Quat};
use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LightDef, LightFalloff, LightKind,
};
use lumit_core::model_layer::ModelSettings;
use lumit_core::time::Rational;
use lumit_scene::pose::sampled;
use lumit_scene::{ModelScene, Path, Projection};

/// One node's place at each frame: where it is and how it's turned, in comp
/// pixels and degrees, each turn unrolled from the frame before so a key never
/// spins the long way round.
#[derive(Default)]
struct Track {
    times: Vec<Rational>,
    position: [Vec<f64>; 3],
    rotation: [Vec<f64>; 3],
    /// A camera's Zoom at each frame, when its lens moves.
    zoom: Vec<f64>,
}

impl Track {
    fn push(&mut self, time: Rational, at: Mat4) {
        // The model's camera looks down its -Z with Y up, which the half turn
        // into the comp makes the comp's own: at rest it comes to no turn.
        let flip = Mat3::from_rotation_x(std::f32::consts::PI);
        let m = Mat3::from_mat4(at);
        let axes = [m.x_axis, m.y_axis, m.z_axis].map(|a| a.normalize_or_zero());
        let turn =
            Quat::from_mat3(&(Mat3::from_cols(axes[0], axes[1], axes[2]) * flip.transpose()));
        let (y, x, z) = turn.to_euler(EulerRot::YXZ);
        let origin = at.w_axis.truncate();
        for (i, v) in origin.to_array().into_iter().enumerate() {
            self.position[i].push(f64::from(v));
        }
        for (row, angle) in self.rotation.iter_mut().zip([x, y, z]) {
            let mut deg = f64::from(angle).to_degrees();
            if let Some(&last) = row.last() {
                deg += ((last - deg) / 360.0).round() * 360.0;
            }
            row.push(deg);
        }
        self.times.push(time);
    }

    /// `values` as a row: one number when they never change, a key a frame when
    /// they do.
    fn row(&self, values: &[f64]) -> Property {
        let first = values.first().copied().unwrap_or(0.0);
        if values.iter().all(|&v| (v - first).abs() < 1e-3) {
            return Property::fixed(first);
        }
        // A key the line between the kept key before it and the next one
        // already gives adds nothing but a mark on the timeline.
        let time_of = |i: usize| self.times[i].to_f64();
        let mut kept = vec![0];
        for i in 1..values.len().saturating_sub(1) {
            let a = kept[kept.len() - 1];
            let share = (time_of(i) - time_of(a)) / (time_of(i + 1) - time_of(a));
            let line = values[a] + (values[i + 1] - values[a]) * share;
            let span_holds = (a + 1..=i).all(|j| {
                let share = (time_of(j) - time_of(a)) / (time_of(i + 1) - time_of(a));
                (values[a] + (values[i + 1] - values[a]) * share - values[j]).abs() < 1e-3
            });
            if (line - values[i]).abs() >= 1e-3 || !span_holds {
                kept.push(i);
            }
        }
        kept.push(values.len() - 1);
        let keys = kept.into_iter().map(|i| Keyframe {
            time: self.times[i],
            value: values[i],
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        });
        let mut row = Property::zero();
        row.set_animation(Animation::Keyframed(keys.collect()));
        row
    }

    /// The rows on `layer`.
    fn lay_on(&self, layer: &mut Layer) {
        let tr = &mut layer.transform;
        tr.position_x = self.row(&self.position[0]);
        tr.position_y = self.row(&self.position[1]);
        tr.position_z = self.row(&self.position[2]);
        tr.rotation_x = self.row(&self.rotation[0]);
        tr.rotation_y = self.row(&self.rotation[1]);
        tr.rotation = self.row(&self.rotation[2]);
    }
}

/// How many frames a take is followed for at most.
const MOST_FRAMES: i64 = 100_000;

/// A Camera or Light layer for each camera and light in the file of Model layer
/// `layer`, over the part of the comp it spans, the comp's first camera first.
/// `camera_name` and `light_name` name one the file doesn't. Empty until the
/// file has been read and on any other kind of layer.
#[must_use]
pub fn layers_from_model(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    (camera_name, light_name): (&str, &str),
) -> Vec<Layer> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return Vec::new();
    };
    let Some(Some(scene)) = crate::model::scene_if_read(doc, *item) else {
        return Vec::new();
    };
    let nodes: Vec<usize> = (0..scene.nodes.len())
        .filter(|&k| scene.nodes[k].camera.is_some() || scene.nodes[k].light.is_some())
        .collect();
    if nodes.is_empty() {
        return Vec::new();
    }
    let tracks = follow(doc, comp, layer, settings, &scene, &nodes);
    let size = (comp.width as f32, comp.height as f32);
    let to_layer = crate::model::model_to_comp(settings, &scene.bounds, size);
    // The comp's pixels to a metre of the model at the layer's first frame.
    let scale = Mat4::from_cols_array_2d(&to_layer)
        .x_axis
        .truncate()
        .length();
    nodes
        .iter()
        .zip(&tracks)
        .filter_map(|(&k, track)| {
            let node = &scene.nodes[k];
            let named = |own: &str, fallback: &str| {
                [node.name.as_str(), own, fallback]
                    .into_iter()
                    .find(|n| !n.is_empty())
                    .unwrap_or_default()
                    .to_owned()
            };
            let mut made = if let Some(camera) = node.camera.and_then(|c| scene.cameras.get(c)) {
                camera_layer(comp, camera, named(&camera.name, camera_name))
            } else {
                let light = node.light.and_then(|l| scene.lights.get(l))?;
                light_layer(comp, light, named(&light.name, light_name), scale)
            };
            track.lay_on(&mut made);
            if let (LayerKind::Camera { zoom, .. }, false) = (&mut made.kind, track.zoom.is_empty())
            {
                *zoom = track.row(&track.zoom);
            }
            lumit_core::edits::solo_on_arrival(&mut made, comp.layers.iter());
            Some(made)
        })
        .collect()
}

/// Where each of `nodes` is at every frame the layer spans, in comp pixels.
fn follow(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    settings: &ModelSettings,
    scene: &ModelScene,
    nodes: &[usize],
) -> Vec<Track> {
    let size = (comp.width as f32, comp.height as f32);
    let to_layer =
        Mat4::from_cols_array_2d(&crate::model::model_to_comp(settings, &scene.bounds, size));
    let rate = comp.frame_rate;
    let first = rate.frame_at(
        layer
            .in_point
            .max(lumit_core::time::CompTime(Rational::ZERO)),
    );
    let last = rate.frame_at(
        layer
            .out_point
            .min(lumit_core::time::CompTime(comp.duration.0)),
    );
    let mut tracks: Vec<Track> = nodes.iter().map(|_| Track::default()).collect();
    for frame in first..last.max(first + 1).min(first + MOST_FRAMES) {
        let Ok(time) = rate.time_of_frame(frame) else {
            continue;
        };
        let t = time.0.to_f64();
        let context = Arc::new(ExpressionContext {
            layer: Some(layer.id),
            ..(*lumit_core::model::comp_context(doc, comp, t)).clone()
        });
        let place = crate::placement::world_placement(comp, layer, t, Arc::clone(&context));
        let place = Mat4::from_cols_array_2d(&place) * to_layer;
        let pose = crate::model_motion::pose_at(comp, layer, settings, scene, t, &context);
        let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
        let clip = crate::model_motion::clip_of(settings, scene);
        let moment =
            clip.and_then(|c| crate::model_motion::take_moment(layer, settings, c, lt, &context));
        for (track, &k) in tracks.iter_mut().zip(nodes) {
            let lens = clip.and_then(|c| {
                let moving = c
                    .channels
                    .iter()
                    .find(|ch| ch.node == k && ch.path == Path::YFov)?;
                Some(zoom_of(comp, sampled(moving, moment? as f32)?))
            });
            track.zoom.extend(lens);
            let world = match &pose {
                Some(pose) => pose.worlds.get(k).copied().unwrap_or(Mat4::IDENTITY),
                None => scene.node_world(k),
            };
            track.push(time.0, place * world);
        }
    }
    tracks
}

/// A vertical field of view as Zoom: the comp pixels from the eye at which it
/// fills the comp's height.
fn zoom_of(comp: &Composition, yfov: f32) -> f64 {
    f64::from(comp.height) * 0.5 / f64::from((yfov * 0.5).tan())
}

fn camera_layer(comp: &Composition, camera: &lumit_scene::Camera, name: String) -> Layer {
    let mut made = lumit_core::edits::fresh_camera(comp, name, None);
    let zoom = match camera.projection {
        Projection::Perspective { yfov } if yfov > 0.0 => zoom_of(comp, yfov),
        // No orthographic camera in a comp: the default lens stands in.
        _ => lumit_core::camera::default_zoom(f64::from(comp.width)),
    };
    if let LayerKind::Camera {
        zoom: z, options, ..
    } = &mut made.kind
    {
        *z = Property::fixed(zoom);
        options.two_node = false;
    }
    made
}

fn light_layer(
    comp: &Composition,
    light: &lumit_scene::Light,
    name: String,
    px_per_metre: f32,
) -> Layer {
    let c = light.colour.map(|c| Property::fixed(f64::from(c)));
    let mut def = LightDef {
        colour: c,
        two_node: false,
        casts_shadows: true,
        shadow_diffusion: Property::fixed(f64::from(comp.width) * 0.01),
        ..LightDef::default()
    };
    // The file's light is physical and a comp's percent isn't: a white surface
    // facing it shows as bright as its light's share of pi lux, which is the
    // diffuse term with the pi left out. Exporters that start from watts turn
    // them into lux or candela at 683 lumens a watt, which is far past what a
    // light typed by hand comes to, and comes back out here.
    let raw = f64::from(light.intensity);
    let share = if raw > 50.0 { raw / 683.0 } else { raw } / std::f64::consts::PI;
    match light.kind {
        lumit_scene::LightKind::Directional => {
            def.kind = LightKind::Parallel;
            def.intensity = Property::fixed((share * 100.0).clamp(0.0, 1000.0));
        }
        kind => {
            def.kind = match kind {
                lumit_scene::LightKind::Spot => LightKind::Spot,
                _ => LightKind::Point,
            };
            // Full strength out to where the inverse square comes to 100%.
            def.falloff = LightFalloff::InverseSquareClamped;
            def.intensity = Property::fixed(100.0);
            def.radius_px = Property::fixed((share.sqrt() * f64::from(px_per_metre)).max(1.0));
        }
    }
    if light.kind == lumit_scene::LightKind::Spot {
        def.cone_angle = Property::fixed(f64::from(light.outer_cone).to_degrees() * 2.0);
        let inner = (light.inner_cone / light.outer_cone.max(1e-3)).clamp(0.0, 1.0);
        def.cone_feather = Property::fixed(f64::from(1.0 - inner) * 100.0);
    }
    lumit_core::edits::base_layer(
        name,
        LayerKind::Light {
            light: Box::new(def),
        },
        comp.duration.0,
        lumit_core::model::TransformGroup::default(),
    )
}

/// Whether `scene` holds a camera or a light to make a layer from.
#[must_use]
pub fn holds_any(scene: &ModelScene) -> bool {
    scene
        .nodes
        .iter()
        .any(|n| n.camera.is_some() || n.light.is_some())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// A key the straight line already gives is left out, and the row still
    /// reads the same at every key it had.
    #[test]
    fn a_row_keeps_only_the_keys_a_line_cant_give() {
        let mut track = Track::default();
        let values: Vec<f64> = (0..=20)
            .map(|i| if i <= 10 { f64::from(i) * 2.0 } else { 20.0 })
            .collect();
        for i in 0..=20 {
            track.times.push(Rational::new(i, 10).unwrap());
        }
        let row = track.row(&values);
        let Animation::Keyframed(keys) = &row.animation else {
            panic!("keyed");
        };
        assert_eq!(keys.len(), 3, "the two ends and the corner");
        for (i, v) in values.iter().enumerate() {
            assert!((row.value_at(i as f64 / 10.0) - v).abs() < 1e-3);
        }
    }

    /// At rest a model's camera is the comp's: no turn, whatever its place.
    /// Looking back along itself turns it half way round Y, and the same turn
    /// again next frame doesn't unwind.
    #[test]
    fn a_track_reads_turns_as_the_comp_has_them() {
        let flip = Mat4::from_rotation_x(std::f32::consts::PI);
        let mut track = Track::default();
        let time = Rational::new(0, 1).unwrap();
        track.push(
            time,
            flip * Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        );
        assert!(
            (track.position[1][0] + 2.0).abs() < 1e-3,
            "y goes down the comp"
        );
        for row in &track.rotation {
            assert!(row[0].abs() < 1e-3, "{row:?}");
        }
        for deg in [170.0f32, -170.0] {
            track.push(time, flip * Mat4::from_rotation_y(deg.to_radians()));
        }
        // 170 then 190, the short way, not back through nought.
        let y = &track.rotation[1];
        assert!((y[2].abs() - y[1].abs() - 20.0).abs() < 1e-2, "{y:?}");
    }
}

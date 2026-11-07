//! Per-layer motion blur's sub-frame copies (docs/06 §4).

use std::sync::Arc;

use glam::{Mat4, Vec4};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{CameraPose, Composition, Layer};
use lumit_gpu::composite::{camera_frame, place_matrix, MbSample};

/// Where `layer` is at each moment of the open shutter around comp time
/// `t_comp`, as the copies its blur is drawn from. Empty, so the layer draws
/// once as it is, unless the comp master and the layer's own switch are on,
/// `samples` is at least 2 and the picture moves at all.
///
/// The count follows the motion, as After Effects' adaptive samples do. The
/// comp's `samples` copies come first. When the layer's corners travel further
/// than that many pixels across the shutter, it is drawn at enough moments to
/// put the copies about a pixel apart, up to the comp's `limit`. Its parents
/// and, for a 3D layer, the camera move the copies too. Shared by every caller
/// of the one comp walk, so the Viewer and export smear alike.
pub fn samples(
    comp: &Composition,
    layer: &Layer,
    t_comp: f64,
    natural: (f32, f32),
    context: Arc<ExpressionContext>,
) -> Vec<MbSample> {
    // A model blurs in the mesh pass, from its own placements (model::draw),
    // and particles in the scene along their own paths.
    let model = lumit_core::extrude::draws_mesh(layer) || crate::realise::scene::in_scene(layer);
    if !layer.switches.motion_blur || model {
        return Vec::new();
    }
    let blur = comp.motion_blur;
    let first = blur.sample_offsets();
    if first.is_empty() {
        return Vec::new();
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let (w, h) = (comp.width, comp.height);
    let camera = |t| crate::track::camera_pose(&context.document, comp, t);
    let frame_camera = layer.switches.three_d.then(|| camera(t_comp));
    let copies_at = |offsets: &[f64]| -> Vec<(MbSample, Mat4)> {
        offsets
            .iter()
            .map(|off| {
                let t = t_comp + off * dt;
                // An expression reading `time` has to run at this moment, or
                // an expression-driven layer would not smear.
                let context = Arc::new(ExpressionContext {
                    comp_time: t,
                    ..(*context).clone()
                });
                let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
                let mut copy = super::drawn(layer, lt, &context);
                copy.pre = super::parent_world_placement(comp, layer, t, context);
                let mut seen = Mat4::IDENTITY;
                if let Some(frame) = frame_camera {
                    let now = camera(t);
                    if moved(&frame, &now) {
                        // The camera's own matrix flattens depth and has no
                        // inverse, so the move goes in as the change of view,
                        // seen through the frame's lens.
                        let view = |p: &CameraPose| {
                            camera_frame(
                                w as f32,
                                h as f32,
                                frame.zoom as f32,
                                vec3(p.position),
                                vec3(p.rotation_deg),
                            )
                        };
                        let pre = copy
                            .pre
                            .map_or(Mat4::IDENTITY, |m| Mat4::from_cols_array_2d(&m));
                        copy.pre =
                            Some((view(&frame).inverse() * view(&now) * pre).to_cols_array_2d());
                    }
                    seen = crate::export::camera_mat(w, h, frame);
                }
                if let Some(pre) = copy.pre {
                    seen *= Mat4::from_cols_array_2d(&pre);
                }
                let place = place_matrix(
                    copy.position,
                    copy.anchor,
                    copy.scale,
                    copy.rotation_deg,
                    copy.z,
                    copy.rotation_x_deg,
                    copy.rotation_y_deg,
                );
                (copy, seen * Mat4::from_cols_array_2d(&place))
            })
            .collect()
    };
    let copies = copies_at(&first);
    // The midpoint copies span all but one slice of the shutter.
    let n = first.len() as f64;
    let travel = travel(&copies, natural) * n / (n - 1.0);
    if travel == 0.0 {
        return Vec::new();
    }
    let wanted = blur.samples_for(travel);
    let copies = if wanted as usize == first.len() {
        copies
    } else {
        copies_at(&blur.offsets(wanted))
    };
    copies.into_iter().map(|(copy, _)| copy).collect()
}

fn vec3(v: (f64, f64, f64)) -> (f32, f32, f32) {
    (v.0 as f32, v.1 as f32, v.2 as f32)
}

fn moved(a: &CameraPose, b: &CameraPose) -> bool {
    (a.zoom, a.position, a.rotation_deg) != (b.zoom, b.position, b.rotation_deg)
}

/// How far the layer's corners travel on the picture from copy to copy, in
/// comp pixels, the furthest corner each step.
fn travel(copies: &[(MbSample, Mat4)], natural: (f32, f32)) -> f64 {
    let corners = [
        (0.0, 0.0),
        (natural.0, 0.0),
        (0.0, natural.1),
        (natural.0, natural.1),
    ];
    let seen = |m: &Mat4, (x, y): (f32, f32)| {
        let p = *m * Vec4::new(x, y, 0.0, 1.0);
        // A corner behind the eye has no place on the picture.
        (p.w > 1e-3).then(|| (p.x / p.w, p.y / p.w))
    };
    copies
        .windows(2)
        .map(|pair| {
            corners
                .iter()
                .filter_map(|&c| {
                    let (a, b) = (seen(&pair[0].1, c)?, seen(&pair[1].1, c)?);
                    Some(f64::from((b.0 - a.0).hypot(b.1 - a.1)))
                })
                .fold(0.0, f64::max)
        })
        .sum()
}

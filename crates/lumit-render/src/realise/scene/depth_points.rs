//! Depth points in the comp's 3D scene. Each pixel of the layer's picture is a
//! point on the ray from the source camera's eye through where that pixel sits
//! on the layer's plane, so from that camera it lands on its own pixel at any
//! depth. The frame it draws into and how it joins the 3D layers are
//! Particulate's in the scene.

use super::{Joined, FRAMES, PENDING};
use crate::fxops::LayerInput;
use glam::{DMat4, DVec3};
use lumit_core::fx::effects::depth_points::{DepthPoints, VIEW_CAMERA_ID};
use lumit_core::fx::params::ResolvedFx;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::{Composition, Document, EffectInstance, Layer, LayerKind};
use lumit_core::planes::{PlaneTask, DEPTH_POINTS};
use std::sync::Arc;

/// Where a layer's Depth points' rays start, and the plane they pass through.
#[derive(Clone)]
pub struct Rays {
    eye: DVec3,
    /// The layer's own pixels to comp pixels.
    place: DMat4,
}

/// The eye of the Camera row's layer where it names a camera, or of the comp's
/// own camera. A 3D view is drawn through a Camera layer it puts on top of a
/// copy of the comp, which isn't the comp's own, or the points would line up
/// from wherever they are looked at.
fn eye(doc: &Arc<Document>, comp: &Composition, e: &EffectInstance, t: f64) -> DVec3 {
    let camera = |l: &&Layer| matches!(l.kind, LayerKind::Camera { .. });
    let named = e
        .layer_ref("camera")
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
        .filter(camera);
    let own = comp.layers.iter().filter(camera).find(|l| {
        l.id != VIEW_CAMERA_ID
            && l.switches.visible
            && t >= l.in_point.0.to_f64()
            && t < l.out_point.0.to_f64()
    });
    let pose = named.or(own).and_then(|l| {
        lumit_core::track::camera_pose_of(doc, comp, l, t, &crate::track::Store)
            .map(|linked| linked.pose)
    });
    let at = pose.map_or_else(
        || {
            // The view's camera was made for the comp before a viewport widened
            // it, so its point of interest is still the true middle.
            let middle = comp.layers.iter().find_map(|l| match &l.kind {
                LayerKind::Camera { options, .. } if l.id == VIEW_CAMERA_ID => Some((
                    options.point_of_interest[0].value_at(0.0),
                    options.point_of_interest[1].value_at(0.0),
                )),
                _ => None,
            });
            let (w, h) = middle
                .map_or((f64::from(comp.width), f64::from(comp.height)), |(x, y)| {
                    (x * 2.0, y * 2.0)
                });
            lumit_core::camera::default_pose(w, h).position
        },
        |pose| pose.position,
    );
    DVec3::new(at.0, at.1, at.2)
}

/// `layer`'s rays at comp time `t`, `None` with no Depth points on it.
pub(super) fn rays(doc: &Arc<Document>, comp: &Composition, layer: &Layer, t: f64) -> Option<Rays> {
    let e = layer
        .effects
        .iter()
        .find(|e| e.enabled && e.effect.match_name == DEPTH_POINTS)?;
    let context = Arc::new(lumit_core::expression::ExpressionContext {
        document: doc.clone(),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: t,
        current_depth: 0,
        inputs: None,
    });
    let place = crate::placement::world_placement(comp, layer, t, context);
    Some(Rays {
        eye: eye(doc, comp, e, t),
        place: glam::Mat4::from_cols_array_2d(&place).as_dmat4(),
    })
}

/// The picture Depth points read for depth: its Depth layer's, else its own
/// analysis's.
fn depth_of<'a>(
    input: Option<&'a LayerInput>,
    own: &'a wgpu::Texture,
    plane: Option<&'a wgpu::Texture>,
) -> Option<&'a wgpu::Texture> {
    let picked = match input {
        Some(LayerInput::Several(list)) => list.first().and_then(|(l, _)| l.texture(own)),
        _ => None,
    };
    picked.or(plane)
}

/// Draw `op` if it is Depth points in a frame that can see it, leaving the
/// picture in `tex` and its distances for its layer to claim. Off a 3D layer it
/// passes the picture through. Any other op gets its plane back untouched.
pub(crate) fn draw<'a>(
    fx: &lumit_gpu::fx::FxEngine,
    ctx: &lumit_gpu::GpuContext,
    (tex, out_w, out_h): (&mut wgpu::Texture, &mut u32, &mut u32),
    op: &ResolvedFx<'_>,
    input: Option<&LayerInput>,
    plane: Option<(PlaneTask, Option<&'a wgpu::Texture>)>,
) -> Option<(PlaneTask, Option<&'a wgpu::Texture>)> {
    if op.def.schema().match_name != DEPTH_POINTS {
        return plane;
    }
    let frame = FRAMES.with(|f| f.borrow().last().cloned())?;
    let rays = frame.carried.rays.as_ref()?;
    let p = DepthPoints::read(op.params);
    let s = f64::from(
        op.params
            .float(DepthPoints::DERIVED_PX_SCALE, 1.0)
            .max(1e-6),
    );
    let (w, h) = (*out_w, *out_h);
    let (wf, hf) = (f64::from(w), f64::from(h));
    // From the raster to the layer's own pixels and on to the comp, by the
    // picture's centre, which stays put however far an effect above grew it.
    let across = rays.place.x_axis.truncate() / s;
    let down = rays.place.y_axis.truncate() / s;
    let corner = rays.place.transform_point3(DVec3::new(
        (f64::from(frame.natural.0) - wf / s) / 2.0,
        (f64::from(frame.natural.1) - hf / s) / 2.0,
        0.0,
    ));
    // A point is as big as the layer's scale makes it, and never smaller than
    // the gap between its neighbours. Its size arrives in raster pixels.
    let scale = across.cross(down).length().sqrt();
    let half = 0.5 * f64::from(p.point_size).max(1.0) * scale;
    let (cw, ch) = frame.comp;
    let rows = lumit_core::camera::projection(&frame.camera, f64::from(cw), f64::from(ch));
    let seen =
        |v: DVec3, w: f64| rows.map(|r| (r[0] * v.x + r[1] * v.y + r[2] * v.z + r[3] * w) as f32);
    let middle = corner + across * wf / 2.0 + down * hf / 2.0;
    let w_middle = seen(middle, 1.0)[2];
    let depth = depth_of(input, tex, plane.and_then(|(_, slot)| slot))
        .map(|d| lumit_gpu::fx::fit_centred(ctx, d.clone(), w, h));
    let (colour, inv_w) = fx.depth_points(
        ctx,
        tex,
        depth.as_ref(),
        &lumit_gpu::fx::depth_points::DepthPointsOp {
            origin: seen(rays.eye, 1.0),
            along: seen(corner - rays.eye, 0.0),
            across: seen(across, 0.0),
            down: seen(down, 0.0),
            target: lumit_gpu::composite::scaled_size(cw, ch, frame.render_scale.max(1e-6)),
            render_scale: frame.render_scale.max(1e-6),
            half: half as f32,
            amount: p.amount / 100.0,
            density: p.density / 100.0,
            invert: p.invert,
            fallback: if w_middle > 0.0 { 1.0 / w_middle } else { 0.0 },
        },
    );
    (*out_w, *out_h) = (colour.width(), colour.height());
    *tex = colour;
    PENDING.with(|pending| {
        *pending.borrow_mut() = Some(Joined {
            depth: inv_w,
            middle: middle.as_vec3(),
        });
    });
    None
}

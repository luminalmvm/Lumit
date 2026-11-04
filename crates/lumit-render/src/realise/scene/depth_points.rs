//! Depth points in the comp's 3D scene. Each pixel of the layer's picture is a
//! point on the ray from the source camera's eye through where that pixel sits
//! on the layer's plane, so from that camera it lands on its own pixel at any
//! depth. The frame it draws into and how it joins the 3D layers are
//! Particulate's in the scene.

use super::{Joined, FRAMES, PENDING};
use crate::fxops::LayerInput;
use glam::{DMat4, DVec3};
use lumit_core::fx::effects::depth_points::{self, DepthPoints};
use lumit_core::fx::params::ResolvedFx;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::{Composition, Document, Layer};
use lumit_core::planes::{PlaneTask, DEPTH_POINTS};
use std::sync::Arc;

/// Where a layer's Depth points' rays start, and the plane they pass through.
#[derive(Clone)]
pub struct Rays {
    eye: DVec3,
    /// The layer's own pixels to comp pixels.
    place: DMat4,
    /// Whether its Camera row names a camera.
    named: bool,
    /// What steadies its analysis at this frame.
    steady: [f32; 2],
}

/// `layer`'s rays at comp time `t`, `None` with no Depth points on it.
pub(super) fn rays(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    source_frame: i64,
) -> Option<Rays> {
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
    let mut place = glam::Mat4::from_cols_array_2d(&place).as_dmat4();
    let named = e.layer_ref("camera");
    let source = depth_points::source(doc, comp, layer, named, t);
    let slide = source.slide;
    place.w_axis += glam::DVec4::new(slide.0, slide.1, slide.2, 0.0);
    Some(Rays {
        eye: DVec3::new(source.eye.0, source.eye.1, source.eye.2),
        place,
        named: named.is_some(),
        steady: crate::planes::steady::of(e.id, source_frame),
    })
}

impl Rays {
    /// Spliced out of a collapsed precomp by `outer`, the eye goes with it if a
    /// camera inside the precomp is named, and is `comp`'s camera's if not.
    pub(super) fn spliced(
        &mut self,
        doc: &Arc<Document>,
        comp: &Composition,
        layer: &Layer,
        outer: DMat4,
        t: f64,
    ) {
        self.eye = if self.named {
            outer.transform_point3(self.eye)
        } else {
            let at = depth_points::source(doc, comp, layer, None, t).eye;
            DVec3::new(at.0, at.1, at.2)
        };
    }
}

/// The picture its Depth layer row gives Depth points, if any.
fn picked<'a>(input: Option<&'a LayerInput>, own: &'a wgpu::Texture) -> Option<&'a wgpu::Texture> {
    match input {
        Some(LayerInput::Several(list)) => list.first().and_then(|(l, _)| l.texture(own)),
        _ => None,
    }
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
    let place = frame
        .carried
        .outer
        .map_or(rays.place, |o| o.as_dmat4() * rays.place);
    let across = place.x_axis.truncate() / s;
    let down = place.y_axis.truncate() / s;
    let corner = place.transform_point3(DVec3::new(
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
    // Only its own analysis wants steadying: a picture is what it is.
    let map = picked(input, tex);
    let steady = if map.is_some() {
        [1.0, 0.0]
    } else {
        rays.steady
    };
    let depth = map
        .or(plane.and_then(|(_, slot)| slot))
        .map(|d| lumit_gpu::fx::fit_centred(ctx, d.clone(), w, h));
    let r = frame.render_scale.max(1e-6);
    let zoom = frame.camera.zoom.max(1.0);
    // The camera's blur, as the layers take it.
    let lens = frame.camera.dof.map_or([0.0; 3], |dof| {
        let k =
            lumit_core::camera::coc_per_zoom(&dof, zoom) / 2.0 * crate::realise::dof::DOF_SOFTNESS;
        [k as f32 * r, zoom as f32, dof.focus_distance as f32]
    });
    let (colour, inv_w) = fx.depth_points(
        ctx,
        tex,
        depth.as_ref(),
        &lumit_gpu::fx::depth_points::DepthPointsOp {
            origin: seen(rays.eye, 1.0),
            along: seen(corner - rays.eye, 0.0),
            across: seen(across, 0.0),
            down: seen(down, 0.0),
            target: lumit_gpu::composite::scaled_size(cw, ch, r),
            render_scale: r,
            half: half as f32,
            amount: p.amount / 100.0,
            density: p.density / 100.0,
            invert: p.invert,
            steady,
            lens,
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

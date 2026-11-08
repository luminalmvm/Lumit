//! Where a layer sits: its own placement and its parents', as the compositor
//! places it.

use std::sync::Arc;

use lumit_core::expression::ExpressionContext;

/// The world placement matrix of `layer`'s parent chain within `comp` at comp
/// time `t_comp` (layer parenting): `P_top × … × P_grandparent ×
/// P_parent`, each ancestor's `place_matrix` sampled at its own local time
/// (`t_comp − start_offset`). `None` when the layer has no parent. Used as a
/// draw's `pre`, which the GPU applies as `pre × own_placement` — so the child
/// ends up placed inside its parent's coordinate space (After Effects
/// parenting). Cycle- and missing-parent-safe via `model::layer_parent_chain`.
/// Shared by the preview (here) and the export path so the two stay identical.
/// v1 composes the full `place_matrix` (2D plus the 2.5D axes it already
/// carries); no behaviour changes for an unparented layer (`None`).
pub fn parent_world_placement(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> Option<[[f32; 4]; 4]> {
    let mut world: Option<[[f32; 4]; 4]> = None;
    // Outermost first, so concat_place(outer, inner) = outer × inner.
    let read = lumit_core::model::context_reader(&context, t_comp);
    let solves = crate::track::solves_for(&context.document);
    for (_, p) in lumit_core::model::parent_placements(comp, layer, t_comp, &read, solves) {
        let p = place_of(&p);
        world = Some(match world {
            Some(w) => lumit_gpu::composite::concat_place(w, p),
            None => p,
        });
    }
    world
}

/// Where `layer` sits in `comp` at comp time `t_comp`, parents and all: its
/// own pixels to comp pixels, as the compositor places it.
pub(crate) fn world_placement(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> [[f32; 4]; 4] {
    let own = own_placement(layer, t_comp, &context);
    match parent_world_placement(comp, layer, t_comp, context) {
        Some(pre) => lumit_gpu::composite::concat_place(pre, own),
        None => own,
    }
}

/// `layer`'s own placement at comp time `t_comp`, before any parent's.
pub(crate) fn own_placement(
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: &Arc<ExpressionContext>,
) -> [[f32; 4]; 4] {
    let read = lumit_core::model::context_reader(context, t_comp);
    let solves = crate::track::solves_for(&context.document);
    let comp = context.comp.and_then(|id| context.document.comp(id));
    place_of(&match comp {
        Some(comp) => lumit_core::orient::placed_in(comp, layer, t_comp, &read, solves),
        None => lumit_core::model::placement(layer, t_comp, &read),
    })
}

/// What the compositor draws `layer` with at layer time `lt`, each property
/// read under `context`, with its Orient in it.
pub(crate) fn drawn_placement(
    layer: &lumit_core::model::Layer,
    lt: f64,
    context: &Arc<ExpressionContext>,
) -> lumit_core::model::Placement {
    let read = |p: &lumit_core::anim::Property| p.value_at_with_context(lt, context.clone());
    let flat = !layer.switches.three_d;
    let mut p = lumit_core::orient::placed_at(layer, lt, flat, &read);
    if layer.transform.orient == lumit_core::orient::Orient::TowardsCamera {
        let t = lt + layer.start_offset.0.to_f64();
        if let Some(comp) = context.comp.and_then(|id| context.document.comp(id)) {
            let read = lumit_core::model::context_reader(context, t);
            let solves = crate::track::solves_for(&context.document);
            lumit_core::orient::face_camera(comp, layer, t, &read, solves, &mut p);
        }
    }
    p
}

fn place_of(p: &lumit_core::model::Placement) -> [[f32; 4]; 4] {
    let f = |v: (f64, f64, f64)| (v.0 as f32, v.1 as f32, v.2 as f32);
    lumit_gpu::composite::place_matrix_3d(f(p.position), f(p.anchor), f(p.scale), f(p.rotation_deg))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::edits::{base_layer, new_comp_ops, CompSettings};
    use lumit_core::model::{Document, Layer, LayerKind};
    use lumit_core::ops::{apply, Op};
    use lumit_core::orient::Orient;
    use lumit_core::time::Rational;

    fn ramp(to: f64) -> Property {
        let key = |sec: i64, value: f64| Keyframe {
            time: Rational::new(sec, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let mut p = Property::fixed(0.0);
        p.animation = Animation::Keyframed(vec![key(0, 0.0), key(2, to)]);
        p
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-4, "{a} is not {b}");
    }

    fn placed(edit: impl Fn(&mut Layer), t: f64) -> lumit_core::model::Placement {
        let mut doc = Document::new();
        let (id, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let duration = doc.comp(id).unwrap().duration.0;
        let mut layer = base_layer("car".into(), LayerKind::Null, duration, Default::default());
        edit(&mut layer);
        let (layer_id, index) = (layer.id, 0);
        let add = Op::AddLayer {
            comp: id,
            index,
            layer: Box::new(layer),
        };
        apply(&mut doc, &add).unwrap();
        let doc = Arc::new(doc);
        let comp = doc.comp(id).unwrap();
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        let context = Arc::new(ExpressionContext {
            document: doc.clone(),
            comp: Some(id),
            layer: Some(layer_id),
            comp_time: t,
            current_depth: 0,
            inputs: None,
        });
        let drawn = drawn_placement(layer, t, &context);
        let own = lumit_core::orient::placed_in(
            comp,
            layer,
            t,
            &lumit_core::model::context_reader(&context, t),
            None,
        );
        close(drawn.rotation_deg.2, own.rotation_deg.2);
        drawn
    }

    /// Along path is the tangent plus the layer's own Rotation, and a layer
    /// with Orient off draws exactly what it draws without any of this.
    #[test]
    fn a_layer_is_drawn_along_its_path() {
        let moving = |l: &mut Layer| {
            l.transform.position_x = ramp(100.0);
            l.transform.position_y = ramp(100.0);
            l.transform.rotation = Property::fixed(10.0);
        };
        assert_eq!(placed(moving, 1.0).rotation_deg, (0.0, 0.0, 10.0));
        let along = placed(
            |l| {
                moving(l);
                l.transform.orient = Orient::AlongPath;
            },
            1.0,
        );
        close(along.rotation_deg.2, 55.0);
    }

    /// A 3D layer off to one side of the camera turns to face it, with its z
    /// axis pointing away from the eye.
    #[test]
    fn a_layer_is_drawn_towards_the_camera() {
        let off_side = |l: &mut Layer| {
            l.switches.three_d = true;
            l.transform.position_x = Property::fixed(5000.0);
            l.transform.orient = Orient::TowardsCamera;
        };
        let p = placed(off_side, 1.0);
        let z = lumit_core::camera::forward(p.rotation_deg);
        assert!(z.0 > 0.5, "turned away from the camera: {z:?}");
        close(z.1.abs().min(1.0), z.1.abs());
    }
}

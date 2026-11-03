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
    place_of(&lumit_core::model::placement(
        layer,
        t_comp,
        &lumit_core::model::context_reader(context, t_comp),
    ))
}

fn place_of(p: &lumit_core::model::Placement) -> [[f32; 4]; 4] {
    let f = |v: (f64, f64, f64)| (v.0 as f32, v.1 as f32, v.2 as f32);
    lumit_gpu::composite::place_matrix_3d(f(p.position), f(p.anchor), f(p.scale), f(p.rotation_deg))
}

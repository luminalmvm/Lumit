//! The Viewer's "at" chip on the worker: the point it is asked under, and the
//! copy of the document cut short at that point.

use flutter_rust_bridge::frb;

use super::WorkerState;

/// Latch the "at effect" prefix the Viewer is asking under.
///
/// **A prefix is a way of looking, and a way of looking renames every frame.**
/// The name memo is keyed by `(comp, frame, quality)` against a document
/// *revision* — it cannot see a change that does not move the revision, and
/// turning the chip on does not. Left standing, the memo would go on serving
/// the full stack's names for the cut picture and the Viewer would show the
/// frame it already had: the chip would look broken in the one way nothing
/// else would catch. So this clears it exactly as the viewer-look message
/// does, and starts both readers of those names over with it.
#[frb(ignore)]
pub(super) fn set_prefix(
    state: &mut WorkerState,
    prefix: Option<crate::api::state::prefix::BridgePrefixPoint>,
) {
    if state.prefix == prefix {
        return;
    }
    state.prefix = prefix;
    state.names.clear();
    state.published_bar = None;
    state.fill_exhausted = false;
}

/// `document` as the Viewer is showing it: the committed snapshot, with the
/// prefix layer's stack cut short after the picked effect while the chip is on.
///
/// **In plain terms.** The image chain a layer has *is* its effect stack, so
/// "the picture at the third effect" is the picture the composition makes with
/// that layer's stack stopping after the third effect. There is no second
/// render path: a patched **copy** of the snapshot goes down the ordinary one,
/// exactly as every drag preview already does. The frame key hashes each
/// layer's effects, so the cut names its own frame and the cache cannot hand
/// the Viewer the full picture by mistake.
///
/// Returns the snapshot untouched where the cut would change nothing — another
/// composition's layer, a layer that has gone, an effect the layer no longer
/// carries, or a point that keeps the whole stack. A stale chip is then
/// harmless rather than wrong.
#[frb(ignore)]
pub(super) fn viewed(
    prefix: Option<crate::api::state::prefix::BridgePrefixPoint>,
    document: std::sync::Arc<lumit_core::model::Document>,
) -> std::sync::Arc<lumit_core::model::Document> {
    // Taken by value rather than off the state, so a caller that already holds
    // a mutable borrow of another of its fields (playback does) can still ask.
    let Some(prefix) = prefix else {
        return document;
    };
    cut_to_prefix(&prefix, &document).unwrap_or(document)
}

/// The cut itself, apart from the worker's state so a test can make one.
#[frb(ignore)]
fn cut_to_prefix(
    prefix: &crate::api::state::prefix::BridgePrefixPoint,
    document: &std::sync::Arc<lumit_core::model::Document>,
) -> Option<std::sync::Arc<lumit_core::model::Document>> {
    let cut = cut_stack(prefix, document);
    let Some(node) = prefix.shader_node else {
        return cut;
    };
    // A box inside a Custom shader: the stack or the graph is cut at the
    // shader as for the shader's own chip, and then the shader's Result reads
    // that box instead. Its graph is part of the frame key, so the copy names
    // its own frame.
    let (comp, layer, effect) = match (prefix.graph, prefix.layer) {
        (Some(point), _) => (point.comp, None, point.node),
        (None, Some(layer)) => (layer.comp_id, Some(layer.layer_id), prefix.effect?),
        (None, None) => return cut,
    };
    let mut copy = cut.unwrap_or_else(|| document.clone());
    let inst = shader_mut(std::sync::Arc::make_mut(&mut copy), comp, layer, effect)?;
    let graph = lumit_core::fx::effects::custom_shader::graph_of(inst)?;
    let viewed = lumit_core::fx::shader::graph::viewed_at(graph, node)?;
    inst.extra
        .get_mut(lumit_core::fx::effects::custom_shader::EXTRA_KEY)?
        .as_object_mut()?
        .insert("graph".to_owned(), viewed);
    Some(copy)
}

/// The Custom shader `effect`: on `layer`'s stack, or a box of the comp's
/// node graph where there is no layer.
fn shader_mut(
    document: &mut lumit_core::model::Document,
    comp: uuid::Uuid,
    layer: Option<uuid::Uuid>,
    effect: uuid::Uuid,
) -> Option<&mut lumit_core::model::EffectInstance> {
    let comp = document.comp_mut(comp)?;
    match layer {
        Some(layer) => comp
            .layers
            .iter_mut()
            .find(|l| l.id == layer)?
            .effects
            .iter_mut()
            .find(|e| e.id == effect),
        None => comp.graph.as_mut()?.nodes.iter_mut().find_map(|n| match n {
            lumit_core::comp_graph::GraphNode::Fx(inst) if inst.id == effect => Some(inst),
            _ => None,
        }),
    }
}

/// A layer's stack cut short at the point, or a node graph's Output fed from
/// the point's box.
fn cut_stack(
    prefix: &crate::api::state::prefix::BridgePrefixPoint,
    document: &std::sync::Arc<lumit_core::model::Document>,
) -> Option<std::sync::Arc<lumit_core::model::Document>> {
    // A node graph's point is the other arm: the box is shown by wiring it
    // into the Output on a patched copy, and the copy names its own frame by
    // construction, the key feeding the Output's wire. `None` where the
    // reading would be the one the Viewer already has.
    if let Some(point) = prefix.graph {
        let graph = document
            .comp(point.comp)?
            .graph
            .as_ref()?
            .viewed_at(point.node)?;
        let mut copy = (**document).clone();
        copy.comp_mut(point.comp)?.graph = Some(graph);
        return Some(std::sync::Arc::new(copy));
    }
    let layer_ref = prefix.layer?;
    let comp = layer_ref.comp_id;
    let layer = document
        .comp(comp)?
        .layers
        .iter()
        .find(|l| l.id == layer_ref.layer_id)?;
    let keep = lumit_core::graph::prefix_len(
        &layer.effects,
        prefix
            .effect
            .map_or(lumit_core::graph::NodeRef::Source, |id| {
                lumit_core::graph::NodeRef::Effect(id)
            }),
    )?;
    // ponytail: one document clone per cut frame, which is what every drag
    // preview already costs. The ceiling is that the clone scales with the
    // *document* — layer and keyframe count — not with the picture, so it is
    // free on a small comp and grows without the frame getting any harder. The
    // trigger is docs/13 §2.1's mandate breaking here first: an effect-list
    // drag on a comp of hundreds of layers missing B1's 8 ms UI frame while the
    // same drag on a handful of layers is smooth. The fix is to memoise the cut
    // document by `(prefix, revision)` on the worker state — a drag holds both
    // still, so one clone would serve the whole gesture.
    lumit_core::graph::truncated_effects(document, comp, layer_ref.layer_id, keep)
}

#[cfg(test)]
mod tests;

//! Layer groups across the seam.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::api::layer::LayerReference;
use crate::api::project::ProjectReference;
use crate::api::state::LumitBridgeState;

/// A project with one comp holding four Nulls, top of the stack first.
fn comp_with_four() -> (ProjectReference, CompositionReference, Vec<LayerReference>) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let comp = project
        .new_composition("Scene".into(), None)
        .expect("a composition");
    // Each `add_null_layer` goes in at the top, so the stack reads newest
    // first; reversing gives the four in the order the outline shows them.
    let mut layers: Vec<LayerReference> = (0..4)
        .map(|_| comp.add_null_layer(None).expect("a null layer"))
        .collect();
    layers.reverse();
    (project, comp, layers)
}

fn ids(layers: &[LayerReference]) -> Vec<Uuid> {
    layers.iter().map(|l| l.layer_id).collect()
}

fn span(model: &BridgeCompModel, i: usize) -> (i64, i64) {
    (
        model.layers[i].info.in_frame,
        model.layers[i].info.out_frame,
    )
}

/// A new layer lands on the row it was asked for, so the menu can put one
/// directly above the layer you had selected instead of at the top.
#[test]
fn a_new_layer_lands_on_the_row_it_was_given() {
    let (_project, comp, layers) = comp_with_four();
    let all = ids(&layers);

    // Row 2 is the third layer down: the new one takes that place and the
    // rest move down under it.
    let landed = comp.add_null_layer(Some(2)).expect("a null layer");
    let stack: Vec<Uuid> = comp
        .get_layers()
        .expect("the stack")
        .iter()
        .map(|l| l.layer_id)
        .collect();
    assert_eq!(stack[2], landed.layer_id);
    assert_eq!(stack, [all[0], all[1], landed.layer_id, all[2], all[3]]);

    // No row still means the top, and a row past the end means the bottom
    // rather than an error, which is what a drop under the last layer is.
    let top = comp.add_null_layer(None).expect("a null layer");
    let bottom = comp.add_null_layer(Some(99)).expect("a null layer");
    let stack: Vec<Uuid> = comp
        .get_layers()
        .expect("the stack")
        .iter()
        .map(|l| l.layer_id)
        .collect();
    assert_eq!(stack.first(), Some(&top.layer_id));
    assert_eq!(stack.last(), Some(&bottom.layer_id));
}

#[test]
fn the_read_model_resolves_the_run_the_header_spans() {
    let (_project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    let group = comp
        .group_layers(all[1..3].to_vec(), "Titles".into())
        .expect("a run of two groups");

    let model = comp.get_model().expect("the read model");
    assert_eq!(model.groups.len(), 1);
    let g = &model.groups[0];
    assert_eq!(g.id, group);
    assert_eq!(g.name, "Titles");
    assert_eq!(g.members, all[1..3].to_vec(), "in stack order");
    // Four fresh Nulls each span the whole comp, so the combined bar does.
    assert_eq!((g.in_frame, g.out_frame), span(&model, 1));
    // A fresh layer is visible and unlocked, so the header's faces agree.
    assert!(g.visible && !g.locked);
    // The stack itself is untouched: grouping moves nothing.
    assert_eq!(
        model
            .layers
            .iter()
            .map(|e| e.layer.layer_id)
            .collect::<Vec<_>>(),
        all
    );
}

/// The group-effects crossing (docs/impl/group-effects.md §6): the header's
/// stack is listed on the read model, added to through the group road, and
/// every existing param command reaches it through the shared instance
/// lookup — the same pattern grown its third arm, so a member's own stack
/// and the header's cannot be confused.
#[test]
fn a_headers_effects_cross_and_edit_through_the_shared_lookup() {
    let (_project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    let group = comp
        .group_layers(all[1..3].to_vec(), "Titles".into())
        .expect("a group");

    assert!(
        comp.get_model().expect("model").groups[0]
            .effects
            .is_empty(),
        "an undressed header lists nothing"
    );
    comp.add_group_effect(group, "blur".into())
        .expect("the group road adds");
    // A driver has no graph to land on, so the group road refuses it.
    assert!(comp.add_group_effect(group, "wiggle".into()).is_err());

    let model = comp.get_model().expect("model");
    assert_eq!(model.groups[0].effects.len(), 1);
    assert_eq!(model.groups[0].effects[0].name, "blur");
    assert!(
        model.layers.iter().all(|e| e.info.effects.is_empty()),
        "no member's own stack grew"
    );

    // The staged handles, and the shared lookup routing an ordinary
    // layer command to the header's list: bypass through a member's own
    // LayerReference lands on the group, not the member.
    let handles = comp.get_group_effects(group).expect("handles");
    assert_eq!(handles.len(), 1);
    layers[1]
        .set_effect_enabled(&handles[0], false)
        .expect("the shared lookup routes to the group");
    let model = comp.get_model().expect("model");
    assert!(!model.groups[0].effects[0].enabled);
    assert!(model.layers.iter().all(|e| e.info.effects.is_empty()));

    // And remove, the same way; an unknown group refuses calmly.
    layers[1]
        .remove_effect(&handles[0])
        .expect("removed from the header");
    assert!(comp.get_model().expect("model").groups[0]
        .effects
        .is_empty());
    assert!(comp.get_group_effects(Uuid::now_v7()).is_err());
}

/// A link that would cross the new comp's edge is kept by a hidden copy on
/// the near side, both ways, and the whole thing is one undo.
#[test]
fn precompose_keeps_split_links_with_hidden_copies() {
    use crate::api::layer::BridgeMatte;
    let (project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    layers[1].set_parent(Some(all[0]), 0).expect("parented");
    layers[3]
        .set_matte(Some(BridgeMatte {
            layer: all[2],
            luma: false,
            inverted: false,
        }))
        .expect("matted");
    assert!(comp
        .precompose_splits_links(all[1..3].to_vec(), false)
        .expect("asked"));
    assert!(!comp
        .precompose_splits_links(vec![all[0]], true)
        .expect("asked"));

    let pre = comp
        .precompose(all[1..3].to_vec(), "Packed".into(), false, false, None)
        .expect("precomposed");
    let Some(crate::api::project_item::ItemReference::Composition(inner)) =
        pre.get_source_item().expect("source")
    else {
        panic!("a Precomp layer's source is a composition");
    };
    let inside = inner.get_model().expect("model").layers;
    assert_eq!(inside.len(), 3, "the parent was copied in");
    let parent = layers[1].layer_id;
    let moved = inside
        .iter()
        .find(|e| e.layer.layer_id == parent)
        .expect("moved");
    let copy = inside
        .iter()
        .find(|e| !e.info.switches.visible)
        .expect("copy");
    assert_eq!(
        moved.layer.get_parent().expect("parent"),
        Some(copy.layer.layer_id)
    );

    let outside = comp.get_model().expect("model").layers;
    assert_eq!(outside.len(), 4, "the matte was left behind as a copy");
    let left = outside
        .iter()
        .find(|e| !e.info.switches.visible)
        .expect("copy");
    let matte = layers[3].get_matte().expect("matte").expect("still matted");
    assert_eq!(matte.layer, left.layer.layer_id);

    project.undo().expect("undo");
    assert_eq!(comp.get_model().expect("model").layers.len(), 4);
    assert_eq!(
        layers[3].get_matte().expect("matte").expect("matted").layer,
        all[2]
    );
}

/// A duplicate is a comp of its own: fresh layer and effect ids, parent
/// and matte on its own layers, the original untouched, one undo.
#[test]
fn duplicating_a_comp_gives_its_layers_fresh_ids() {
    use crate::api::layer::BridgeMatte;
    let (project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    layers[1].set_parent(Some(all[0]), 0).expect("parented");
    layers[3]
        .set_matte(Some(BridgeMatte {
            layer: all[2],
            luma: false,
            inverted: false,
        }))
        .expect("matted");
    layers[2].add_effect("blur".into()).expect("an effect");
    let before = comp.composition().expect("original");

    let copy = comp
        .duplicate(false)
        .expect("duplicated")
        .composition()
        .expect("copy");
    let every_id = |c: &lumit_core::model::Composition| -> Vec<Uuid> {
        c.layers
            .iter()
            .flat_map(|l| std::iter::once(l.id).chain(l.effects.iter().map(|e| e.id)))
            .collect()
    };
    let (old, new) = (every_id(&before), every_id(&copy));
    assert_eq!(new.len(), 5);
    assert!(new.iter().all(|id| !old.contains(id)), "no id is kept");
    assert_eq!(copy.layers[1].parent, Some(copy.layers[0].id));
    assert_eq!(
        copy.layers[3].matte.map(|m| m.layer),
        Some(copy.layers[2].id)
    );
    assert_eq!(comp.composition().expect("original"), before);

    project.undo().expect("undo");
    assert!(comp.document().expect("document").comp(copy.id).is_none());
}

/// *Pre-compose group* carries the header's stack onto the new Precomp
/// layer — where it means exactly what it meant — and ungroups the emptied
/// band in the same batch, so one undo restores band, wardrobe and layers
/// together.
#[test]
fn precomposing_a_group_moves_the_headers_stack_onto_the_precomp_layer() {
    let (project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    let group = comp
        .group_layers(all[1..3].to_vec(), "Titles".into())
        .expect("a group");
    comp.add_group_effect(group, "blur".into())
        .expect("dressed");

    let pre = comp
        .precompose(
            all[1..3].to_vec(),
            "Packed".into(),
            false,
            false,
            Some(group),
        )
        .expect("precomposed");
    let model = comp.get_model().expect("model");
    assert!(model.groups.is_empty(), "the emptied band went with them");
    let packed = model
        .layers
        .iter()
        .find(|e| e.layer.layer_id == pre.layer_id)
        .expect("the Precomp layer");
    assert_eq!(packed.info.effects.len(), 1);
    assert_eq!(
        packed.info.effects[0].name, "blur",
        "the header's stack rode across"
    );

    // One undo puts the band back, wardrobe included, and the layers too.
    project.undo().expect("undo");
    let back = comp.get_model().expect("model");
    assert_eq!(back.groups.len(), 1);
    assert_eq!(back.groups[0].effects.len(), 1);
    assert_eq!(back.groups[0].members, all[1..3].to_vec());

    // A dangling group id refuses before anything commits.
    assert!(comp
        .precompose(
            all[1..3].to_vec(),
            "Packed".into(),
            false,
            false,
            Some(Uuid::now_v7()),
        )
        .is_err());
}

/// One switch click on a multi-selection is **one** undo step (the owner's
/// 53-layer Ctrl+A recorded 53), and a locked sibling sits its share out
/// while the clicked row's own refusal is the whole call's.
#[test]
fn a_selection_switch_is_one_step_and_a_locked_sibling_sits_out() {
    let (project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    layers[2]
        .set_switch(crate::api::layer::BridgeLayerSwitch::Locked, true)
        .expect("lock the sibling");

    comp.set_switch_on_layers(
        all[0],
        all[0..3].to_vec(),
        BridgeLayerSwitch::Visible,
        false,
    )
    .expect("the batch lands");
    let model = comp.get_model().expect("model");
    let visible = |m: &BridgeCompModel, i: usize| m.layers[i].info.switches.visible;
    assert!(!visible(&model, 0) && !visible(&model, 1));
    assert!(
        visible(&model, 2),
        "the locked sibling silently kept its state"
    );
    assert!(visible(&model, 3), "the unpicked layer was never targeted");

    // One undo restores every layer at once — the complaint this fixes.
    project.undo().expect("undo");
    let back = comp.get_model().expect("model");
    assert!(back.layers.iter().all(|e| e.info.switches.visible));

    // The clicked row is not exempted from its own refusal rules: a click
    // that lands *on* the locked row refuses whole, committing nothing.
    assert!(comp
        .set_switch_on_layers(
            all[2],
            all[0..3].to_vec(),
            BridgeLayerSwitch::Visible,
            false
        )
        .is_err());
    let held = comp.get_model().expect("model");
    assert!(
        held.layers.iter().all(|e| e.info.switches.visible),
        "a refused call commits nothing at all"
    );
}

/// Dragging one selected bar slides the whole selection as one step, and
/// the earliest bar hitting comp zero stops the set with its shape intact
/// — the same wall the group bar's drag has.
#[test]
fn a_selection_slide_moves_every_layer_as_one_step_and_clamps_as_a_unit() {
    let (project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    let before = comp.get_model().expect("model");

    comp.slide_layers(vec![all[0], all[1], all[2]], 12, 0)
        .expect("the selection slides");
    let after = comp.get_model().expect("model");
    for i in 0..3 {
        assert_eq!(
            span(&after, i),
            (span(&before, i).0 + 12, span(&before, i).1 + 12),
            "layer {i} moved whole"
        );
    }
    assert_eq!(span(&after, 3), span(&before, 3), "the unpicked one held");

    project.undo().expect("undo");
    let back = comp.get_model().expect("model");
    for i in 0..4 {
        assert_eq!(span(&back, i), span(&before, i), "one undo took it all");
    }

    // Stagger two layers, then drive the pair at the wall: the earlier one
    // stops at zero and the later one keeps its distance from it.
    comp.slide_layers(vec![all[0]], 5, 0).expect("stagger one");
    comp.slide_layers(vec![all[2]], 12, 0)
        .expect("stagger the other");
    comp.slide_layers(vec![all[0], all[2]], -1000, 0)
        .expect("clamped, not refused");
    let walled = comp.get_model().expect("model");
    assert_eq!(span(&walled, 0).0, 0, "the earliest bar sits on the wall");
    assert_eq!(
        span(&walled, 2).0,
        7,
        "and the set kept its shape rather than folding against it"
    );
}

/// Staggering places each layer a step after the one before it, from the
/// earliest in point, in one undo step; a step of 0 lines them up; and a
/// fan holds each layer at comp zero on its own.
#[test]
fn a_stagger_places_layers_in_order_and_a_fan_stops_each_at_the_wall() {
    let (project, comp, layers) = comp_with_four();
    let all = ids(&layers);
    comp.slide_layers(vec![all[1]], 10, 0)
        .expect("move one off zero");
    let before = comp.get_model().expect("model");

    comp.stagger_layers(vec![all[2], all[1], all[0]], 3)
        .expect("staggered");
    let after = comp.get_model().expect("model");
    assert_eq!(span(&after, 2).0, 0, "the first given starts earliest");
    assert_eq!(span(&after, 1).0, 3);
    assert_eq!(span(&after, 0).0, 6);
    assert_eq!(
        span(&after, 1).1 - span(&after, 1).0,
        span(&before, 1).1 - span(&before, 1).0,
        "a layer keeps its length"
    );
    project.undo().expect("undo");
    let back = comp.get_model().expect("model");
    for i in 0..4 {
        assert_eq!(span(&back, i), span(&before, i), "one undo took it all");
    }

    comp.stagger_layers(vec![all[1], all[3]], 0)
        .expect("aligned");
    let lined = comp.get_model().expect("model");
    assert_eq!(span(&lined, 1).0, span(&lined, 3).0, "both start together");

    comp.fan_layers(vec![all[0], all[1]], vec![-5, 4])
        .expect("clamped, not refused");
    let fanned = comp.get_model().expect("model");
    assert_eq!(span(&fanned, 0).0, 0, "already on the wall, it stays");
    assert_eq!(span(&fanned, 1).0, span(&lined, 1).0 + 4);
}

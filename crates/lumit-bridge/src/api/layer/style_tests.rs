//! Layer styles across the seam (K-706, docs/impl/layer-styles.md §5).
//!
//! **In plain terms.** A style is an effect instance in a second, order-locked
//! list on the layer. The point of the bridge half is that saying so is enough:
//! adding one sorts it into Photoshop's fixed order, a second copy of the same
//! style is refused, and every command a parameter row already had — stage a
//! value, commit the list, bypass, remove — works on a style because **one**
//! lookup answers "which of the layer's two lists is this instance on".

use crate::api::effect::{list_styles, BridgeEffectValue, BridgeScalar};
use crate::api::state::LumitBridgeState;

/// The whole of what makes a style row work: it is added, it lands in §2's
/// pinned painting order whatever order it was asked for, it is refused a
/// second time, and a parameter edit staged exactly the way an *effect*
/// parameter's drag is staged commits onto the style rather than onto the
/// effect stack.
#[test]
fn a_style_is_added_ordered_refused_twice_and_edited_through_the_shared_lookup() {
    let project = LumitBridgeState::new_project(None).expect("a project");
    let comp = project.new_composition("Scene".into(), None).expect("comp");
    let layer = comp.add_solid_layer().expect("a solid");

    // Asked for out of order on purpose: the op sorts, so no caller has to.
    layer.add_style("style_stroke".into()).expect("stroke");
    layer.add_style("style_drop_shadow".into()).expect("shadow");
    assert_eq!(
        layer
            .get_info()
            .expect("info")
            .styles
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>(),
        vec!["style_drop_shadow".to_owned(), "style_stroke".to_owned()],
        "the read model carries §2's painting order, not the order they arrived"
    );
    assert!(
        layer.get_info().expect("info").effects.is_empty(),
        "a style is not on the effect stack"
    );

    // Nine named slots, not a stack: the same style twice is refused, and an
    // effect's match name is not a style's.
    assert!(layer.add_style("style_stroke".into()).is_err());
    assert!(layer.add_style("gaussian_blur".into()).is_err());

    // A parameter edit, staged exactly as an effect parameter's drag is.
    let mut staged = layer.get_styles().expect("styles");
    staged[0]
        .set_value(
            "distance".into(),
            BridgeEffectValue::Float(BridgeScalar::Static(41.0)),
        )
        .expect("staged");
    layer.set_effects(staged).expect("committed");

    let info = layer.get_info().expect("info");
    assert_eq!(info.styles.len(), 2, "the list is still the list");
    assert!(
        info.effects.is_empty(),
        "committing a staged style list must not land on the effect stack"
    );
    assert!(
        matches!(
            info.styles[0]
                .values
                .iter()
                .find(|v| v.id == "distance")
                .map(|v| &v.value),
            Some(BridgeEffectValue::Float(BridgeScalar::Static(v))) if (*v - 41.0).abs() < 1e-9
        ),
        "the shared lookup routed the edit onto the style"
    );

    // Bypass and remove are the *effect* commands, unchanged — §5's whole point.
    let handles = layer.get_styles().expect("styles");
    layer
        .set_effect_enabled(&handles[1], false)
        .expect("bypassed");
    assert!(!layer.get_info().expect("info").styles[1].enabled);
    layer.remove_effect(&handles[1]).expect("removed");
    assert_eq!(layer.get_info().expect("info").styles.len(), 1);

    // One op each, so one undo step each.
    project.undo().expect("undone");
    assert_eq!(layer.get_info().expect("info").styles.len(), 2);

    project.close().expect("closed");
}

/// The seven the Add-style menu offers are the seven that render (§8), in §2's
/// order — Satin and Bevel and emboss are modelled and imported, never offered.
#[test]
fn the_add_style_listing_offers_the_seven_that_render() {
    let names: Vec<String> = list_styles().into_iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        vec![
            "style_drop_shadow",
            "style_outer_glow",
            "style_gradient_overlay",
            "style_colour_overlay",
            "style_inner_glow",
            "style_inner_shadow",
            "style_stroke",
        ]
    );
}

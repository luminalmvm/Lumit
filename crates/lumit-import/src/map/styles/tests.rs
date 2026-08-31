//! The layer-styles map stage, checked against hand-built capture nodes
//! (docs/impl/layer-styles.md §9).
//!
//! The golden bundle proves the *negative* half — twenty-two layers carry the
//! group with every slot switched off, and none of them gets a style — because
//! nobody in the fixture project actually added one. The positive half is here:
//! one small AE group per assertion, so the angle formula, the opacity, the
//! order and every report row can be pinned exactly rather than hunted for
//! among a megabyte of JSON.

use std::collections::BTreeMap;

use lumit_core::anim::Animation;
use lumit_core::model::{EffectValue, TransformGroup};

use super::*;
use crate::map::time::TimeBase;
use crate::report::ImportReport;

// ---------------------------------------------------------------------------
// Building an After Effects style group by hand
// ---------------------------------------------------------------------------

/// A leaf: a match name and a still number.
fn leaf(match_name: &str, value: f64) -> Property {
    Property {
        match_name: Some(match_name.to_string()),
        value_type: Some("float".to_string()),
        value: Some(serde_json::json!(value)),
        ..Property::default()
    }
}

/// A colour leaf, in After Effects' display-referred 0..1.
fn colour(match_name: &str, rgba: [f64; 4]) -> Property {
    Property {
        match_name: Some(match_name.to_string()),
        value_type: Some("colour".to_string()),
        value: Some(serde_json::json!(rgba)),
        ..Property::default()
    }
}

/// One style slot, switched on or off.
fn slot(match_name: &str, enabled: bool, children: Vec<Property>) -> Property {
    Property {
        match_name: Some(match_name.to_string()),
        name: Some(match_name.to_string()),
        enabled: Some(enabled),
        group: Some(children),
        ..Property::default()
    }
}

/// The `ADBE Layer Styles` group, as a layer's property list holds it.
fn group_of(slots: Vec<Property>) -> Vec<Property> {
    let mut members = vec![slot(
        "ADBE Blend Options Group",
        true,
        vec![leaf("ADBE Global Angle2", 120.0)],
    )];
    members.extend(slots);
    vec![slot("ADBE Layer Styles", true, members)]
}

/// A layer nobody has turned or squashed, so §3's deviation row stays away.
fn square() -> TransformGroup {
    TransformGroup {
        rotation: lumit_core::anim::Property::fixed(0.0),
        scale_x: lumit_core::anim::Property::fixed(100.0),
        scale_y: lumit_core::anim::Property::fixed(100.0),
        ..TransformGroup::default()
    }
}

/// Map `slots` and hand back the styles and the report they raised.
fn mapped(slots: Vec<Property>, transform: &TransformGroup) -> (Vec<EffectInstance>, ImportReport) {
    let mut report = ImportReport::default();
    let mut conv = Conv {
        report: &mut report,
        tb: TimeBase::new(lumit_core::time::FrameRate::new(24, 1).expect("24 fps")),
        offset: lumit_core::time::Rational::ZERO,
        size: (1920.0, 1080.0),
        span: (
            lumit_core::time::Rational::ZERO,
            lumit_core::time::Rational::ZERO,
        ),
        layer_ids: BTreeMap::new(),
        masks: Vec::new(),
        self_index: 0,
    };
    let path = crate::report::ItemPath::item("Scene").layer("Title");
    let out = styles(&mut conv, &path, &group_of(slots), transform);
    (out, report)
}

/// One style's still parameter, as the number it holds.
fn value_of(style: &EffectInstance, id: &str) -> f64 {
    match &style.params.iter().find(|p| p.id == id).expect(id).value {
        EffectValue::Float(p) => match &p.animation {
            Animation::Static(v) => *v,
            other => panic!("{id} is not still: {other:?}"),
        },
        EffectValue::Choice(v) | EffectValue::Seed(v) => f64::from(*v),
        EffectValue::Bool(b) => f64::from(u8::from(*b)),
        other => panic!("{id} is not a number: {other:?}"),
    }
}

/// After Effects' Drop Shadow at its own defaults.
fn ae_drop_shadow(enabled: bool) -> Property {
    slot(
        "dropShadow/enabled",
        enabled,
        vec![
            leaf("dropShadow/mode2", 5.0),
            colour("dropShadow/color", [0.0, 0.0, 0.0, 1.0]),
            leaf("dropShadow/opacity", 75.0),
            leaf("dropShadow/useGlobalAngle", 0.0),
            leaf("dropShadow/localLightingAngle", 120.0),
            leaf("dropShadow/distance", 5.0),
            leaf("dropShadow/chokeMatte", 0.0),
            leaf("dropShadow/blur", 5.0),
            leaf("dropShadow/noise", 0.0),
            leaf("dropShadow/layerConceals", 1.0),
        ],
    )
}

// ---------------------------------------------------------------------------
// The assertions
// ---------------------------------------------------------------------------

/// **A switched-off slot is a style nobody added.** After Effects lists all ten
/// on any layer that has ever carried the group, so importing the off ones
/// would dress every such layer in styles the user never applied.
#[test]
fn only_the_styles_after_effects_reports_as_on_are_imported() {
    let (styles, report) = mapped(vec![ae_drop_shadow(false)], &square());
    assert!(styles.is_empty());
    assert!(
        report.rows.is_empty(),
        "a layer that wears nothing has nothing to report: {:?}",
        report.rows
    );
}

/// **The angle formula, and the rest of the shadow's dials.** After Effects'
/// 120° default becomes 150° — down and to the right, which is where the
/// default shadow is.
#[test]
fn a_drop_shadow_maps_its_angle_opacity_and_distances() {
    let (styles, _) = mapped(vec![ae_drop_shadow(true)], &square());
    let shadow = styles.first().expect("one style");
    assert_eq!(shadow.effect.match_name, "style_drop_shadow");
    assert!(shadow.enabled);

    assert!((value_of(shadow, "direction") - 150.0).abs() < 1e-9);
    // Per cent one for one: the DOM hands per cent, not the format's byte.
    assert!((value_of(shadow, "opacity") - 75.0).abs() < 1e-9);
    assert!((value_of(shadow, "distance") - 5.0).abs() < 1e-9);
    assert!((value_of(shadow, "softness") - 5.0).abs() < 1e-9);
    assert!((value_of(shadow, "spread")).abs() < 1e-9);
    assert!(
        (value_of(shadow, "knockout") - 1.0).abs() < 1e-9,
        "Layer Knocks Out Drop Shadow is on by default in both"
    );
}

/// **Photoshop's order, whatever order the capture listed them in**, and one
/// instance of each.
#[test]
fn the_styles_come_out_in_the_pinned_painting_order() {
    let stroke = slot(
        "frameFX/enabled",
        true,
        vec![
            leaf("frameFX/mode2", 1.0),
            colour("frameFX/color", [1.0, 0.0, 0.0, 1.0]),
            leaf("frameFX/size", 3.0),
            leaf("frameFX/opacity", 100.0),
            leaf("frameFX/style", 1.0),
        ],
    );
    let fill = slot(
        "solidFill/enabled",
        true,
        vec![
            leaf("solidFill/mode2", 1.0),
            colour("solidFill/color", [1.0, 0.0, 0.0, 1.0]),
            leaf("solidFill/opacity", 100.0),
        ],
    );
    // Deliberately backwards from §2's order.
    let (styles, _) = mapped(vec![stroke, fill, ae_drop_shadow(true)], &square());
    assert_eq!(
        styles
            .iter()
            .map(|s| s.effect.match_name.as_str())
            .collect::<Vec<_>>(),
        vec!["style_drop_shadow", "style_colour_overlay", "style_stroke",]
    );
    // Stroke's Position: After Effects' Outside is Lumit's first entry.
    let stroke = styles.last().expect("the stroke");
    assert!(value_of(stroke, "position").abs() < 1e-9);
}

/// **The link to the composition's light is baked and named** — Lumit has no
/// comp-wide light for a style to follow (§1).
#[test]
fn following_the_global_light_bakes_the_angle_and_says_so() {
    let mut shadow = ae_drop_shadow(true);
    for leaf in shadow.group.as_mut().expect("dials") {
        if leaf.match_name.as_deref() == Some("dropShadow/useGlobalAngle") {
            leaf.value = Some(serde_json::json!(1.0));
        }
    }
    let (styles, report) = mapped(vec![shadow], &square());
    assert!((value_of(&styles[0], "direction") - 150.0).abs() < 1e-9);
    assert!(
        report.rows.iter().any(|r| matches!(
            &r.reason,
            Reason::EffectParamApproximated { effect, param, imported_as }
                if effect == "Drop shadow" && param == "Angle" && imported_as.contains("120")
        )),
        "the link is named: {:?}",
        report.rows
    );
}

/// **A glow's Screen cannot be honoured underneath the layer**, and the report
/// is where that is said rather than in a wrongly blended picture.
#[test]
fn an_outer_styles_blend_mode_is_reported_rather_than_applied() {
    let glow = slot(
        "outerGlow/enabled",
        true,
        vec![
            leaf("outerGlow/mode2", 11.0),
            leaf("outerGlow/opacity", 75.0),
            leaf("outerGlow/noise", 0.0),
            leaf("outerGlow/AEColorChoice", 1.0),
            colour("outerGlow/color", [1.0, 1.0, 0.745, 1.0]),
            leaf("outerGlow/glowTechnique", 1.0),
            leaf("outerGlow/chokeMatte", 0.0),
            leaf("outerGlow/blur", 5.0),
            leaf("outerGlow/inputRange", 50.0),
            leaf("outerGlow/shadingNoise", 0.0),
        ],
    );
    let (styles, report) = mapped(vec![glow], &square());
    assert_eq!(styles[0].effect.match_name, "style_outer_glow");
    assert!(
        value_of(&styles[0], lumit_core::fx::BLEND_PARAM).abs() < 1e-9,
        "the Blend row stays Normal on an outer style"
    );
    assert!(report.rows.iter().any(|r| matches!(
        &r.reason,
        Reason::EffectParamApproximated { effect, param, .. }
            if effect == "Outer glow" && param == "Blend Mode"
    )));
    // At their defaults, the glow's unmodelled dials raise nothing.
    assert!(!report.rows.iter().any(|r| matches!(
        &r.reason,
        Reason::EffectParamNotCarried { param, .. } if param == "Technique" || param == "Range"
    )));
}

/// **An interior style's mode is honoured**, because that is exactly what the
/// injected Blend row does.
#[test]
fn an_interior_styles_blend_mode_lands_on_the_blend_row() {
    let fill = slot(
        "solidFill/enabled",
        true,
        vec![
            leaf("solidFill/mode2", 11.0),
            colour("solidFill/color", [1.0, 0.0, 0.0, 1.0]),
            leaf("solidFill/opacity", 60.0),
        ],
    );
    let (styles, _) = mapped(vec![fill], &square());
    let want = lumit_core::model::BlendMode::ALL
        .iter()
        .position(|b| *b == lumit_core::model::BlendMode::Screen)
        .expect("Screen is a blend mode");
    assert!((value_of(&styles[0], lumit_core::fx::BLEND_PARAM) - want as f64).abs() < 1e-9);
    // An overlay's Opacity **is** its Mix row (§1).
    assert!((value_of(&styles[0], "mix") - 60.0).abs() < 1e-9);
}

/// **Satin and Bevel are kept and reported, not drawn** (§8).
#[test]
fn satin_is_kept_whole_and_named_as_undrawn() {
    let satin = slot(
        "chromeFX/enabled",
        true,
        vec![
            leaf("chromeFX/mode2", 5.0),
            colour("chromeFX/color", [0.0, 0.0, 0.0, 1.0]),
            leaf("chromeFX/opacity", 50.0),
            leaf("chromeFX/localLightingAngle", 19.0),
            leaf("chromeFX/distance", 11.0),
            leaf("chromeFX/blur", 14.0),
            leaf("chromeFX/invert", 1.0),
        ],
    );
    let (styles, report) = mapped(vec![satin], &square());
    assert_eq!(styles[0].effect.match_name, "style_satin");
    assert!((value_of(&styles[0], "distance") - 11.0).abs() < 1e-9);
    // A direction rather than a light: 90 − 19.
    assert!((value_of(&styles[0], "direction") - 71.0).abs() < 1e-9);
    assert!(report.rows.iter().any(|r| matches!(
        &r.reason,
        Reason::EffectDiffers { effect, detail } if effect == "Satin" && detail.contains("not drawn")
    )));
}

/// **Pattern Overlay is not one of the nine**, and is named rather than
/// dropped.
#[test]
fn a_pattern_overlay_is_reported_and_left_behind() {
    let pattern = slot(
        "patternFill/enabled",
        true,
        vec![leaf("patternFill/opacity", 100.0)],
    );
    let (styles, report) = mapped(vec![pattern, ae_drop_shadow(true)], &square());
    assert_eq!(styles.len(), 1, "only the shadow");
    assert!(report.rows.iter().any(|r| matches!(
        &r.reason,
        Reason::EffectParamNotCarried { effect, param }
            if effect == "Layer styles" && param == "Pattern Overlay"
    )));
}

/// **A turned layer is told that its styles turn with it** (§3's deviation).
#[test]
fn a_rotated_styled_layer_gets_the_pre_transform_row() {
    let mut turned = square();
    turned.rotation = lumit_core::anim::Property::fixed(12.0);
    let (_, report) = mapped(vec![ae_drop_shadow(true)], &turned);
    assert!(report.rows.iter().any(|r| matches!(
        &r.reason,
        Reason::EffectDiffers { effect, .. } if effect == "Layer styles"
    )));

    // And an upright one is not.
    let (_, quiet) = mapped(vec![ae_drop_shadow(true)], &square());
    assert!(!quiet.rows.iter().any(|r| matches!(
        &r.reason,
        Reason::EffectDiffers { effect, .. } if effect == "Layer styles"
    )));
}

/// **The Blending Options subgroup is reported once**, and only on a layer that
/// actually wears a style.
#[test]
fn blending_options_are_reported_once_on_a_styled_layer() {
    let (_, report) = mapped(vec![ae_drop_shadow(true)], &square());
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| matches!(
                &r.reason,
                Reason::EffectParamNotCarried { param, .. } if param == "Blending Options"
            ))
            .count(),
        1
    );
}

//! A text layer's animators through the bridge.

use super::*;

/// **Adding the first animator moves the anchor with it, in one undo step**.
/// An animated line is drawn into a box one text size larger a side with the
/// words that far in, so without the compensating shift the words would jump
/// the moment the first animator arrived — and the shift has to
/// ride in the same `Op` as the document, or `Ctrl+Z` puts the pivot back
/// before it takes the animator away.
#[test]
fn adding_the_first_animator_moves_the_anchor_and_undoes_in_one_step() {
    use crate::api::assets::{
        BridgeColourRgba, BridgeRangeSelector, BridgeSelectorBasis, BridgeSelectorShape,
        BridgeTextAnimator, BridgeTextDocument,
    };
    use crate::api::effect::BridgeScalar;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");
    let zero = || BridgeScalar::Static(0.0);
    let plain = BridgeTextDocument {
        text: "Lumit".into(),
        expression: None,
        size: 72.0,
        fill: BridgeColourRgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        path: None,
        path_offset: zero(),
        animators: Vec::new(),
        style: crate::api::assets::default_text_style(),
        paragraph: crate::api::assets::default_paragraph_style(),
    };
    text.set_text(plain.clone()).expect("set");
    let anchor = |t: &crate::api::layer::LayerReference| match t
        .get_transform()
        .expect("transform")
        .anchor_x
    {
        BridgeScalar::Static(v) => v,
        other => panic!("the anchor is not still: {other:?}"),
    };
    let before = anchor(&text);

    let animator = BridgeTextAnimator {
        name: "Cascade".into(),
        selector: BridgeRangeSelector {
            start: zero(),
            end: BridgeScalar::Static(25.0),
            offset: zero(),
            basis: BridgeSelectorBasis::Characters,
            shape: BridgeSelectorShape::Square,
            kind: crate::api::assets::BridgeSelectorKind::Range,
            target: None,
            radius: BridgeScalar::Static(200.0),
            invert: false,
            min_amount: zero(),
            max_amount: BridgeScalar::Static(100.0),
            wiggles: BridgeScalar::Static(2.0),
            correlation: BridgeScalar::Static(50.0),
            seed: 0,
        },
        axes: Vec::new(),
        properties: None,
        position_x: zero(),
        position_y: BridgeScalar::Static(-40.0),
        rotation: zero(),
        scale_x: BridgeScalar::Static(100.0),
        scale_y: BridgeScalar::Static(100.0),
        opacity: BridgeScalar::Static(100.0),
        position_z: zero(),
        rotation_x: zero(),
        rotation_y: zero(),
        fill_r: zero(),
        fill_g: zero(),
        fill_b: zero(),
    };
    let animated = BridgeTextDocument {
        animators: vec![animator.clone()],
        ..plain.clone()
    };
    text.set_text(animated).expect("set");
    assert!(
        (anchor(&text) - before - 72.0).abs() < 1e-6,
        "the anchor did not follow the margin ({} → {})",
        before,
        anchor(&text)
    );
    let back = text.get_text().expect("text").expect("still text");
    assert_eq!(
        back.animators,
        vec![animator],
        "the animator did not survive"
    );

    // One undo step takes the animator and the pivot away together.
    project.undo().expect("undone");
    assert!(
        text.get_text()
            .expect("text")
            .expect("still text")
            .animators
            .is_empty(),
        "the animator outlived its undo"
    );
    assert!(
        (anchor(&text) - before).abs() < 1e-6,
        "the pivot was left where the animator put it"
    );

    // And editing an animator that is already there moves nothing.
    let with = BridgeTextDocument {
        animators: vec![BridgeTextAnimator {
            name: "Cascade".into(),
            ..back.animators[0].clone()
        }],
        ..plain
    };
    text.set_text(with.clone()).expect("set");
    let settled = anchor(&text);
    text.set_text(BridgeTextDocument {
        animators: vec![BridgeTextAnimator {
            rotation: BridgeScalar::Static(30.0),
            ..with.animators[0].clone()
        }],
        ..with
    })
    .expect("set");
    assert!(
        (anchor(&text) - settled).abs() < 1e-6,
        "editing an animator moved the layer"
    );
}

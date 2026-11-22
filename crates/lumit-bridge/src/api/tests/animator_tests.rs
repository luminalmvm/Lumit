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
        BridgeColourRgba, BridgeRangeSelector, BridgeSelectorBasis, BridgeSelectorMode,
        BridgeSelectorShape, BridgeTextAnimator, BridgeTextDocument,
    };
    use crate::api::effect::BridgeScalar;

    let (project, layer) = project_with_layer();
    let comp = CompositionReference::new(project.id, layer.comp_id());
    let text = comp.add_text_layer(None).expect("a text layer");
    let zero = || BridgeScalar::Static(0.0);
    let plain = BridgeTextDocument {
        text: "Lumit".into(),
        expression: None,
        keys: None,
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
            mode: BridgeSelectorMode::Range,
            ease: 0.0,
            kind: crate::api::assets::BridgeSelectorKind::Range,
            target: None,
            radius: BridgeScalar::Static(200.0),
            invert: false,
            ease_high: zero(),
            ease_low: zero(),
            amount: BridgeScalar::Static(100.0),
            delay: BridgeScalar::Static(0.1),
            delay_ease: zero(),
            order: crate::api::assets::BridgeStaggerOrder::Forward,
            bounce_amplitude: zero(),
            bounce_frequency: BridgeScalar::Static(2.0),
            bounce_decay: BridgeScalar::Static(3.0),
            min_amount: zero(),
            max_amount: BridgeScalar::Static(100.0),
            wiggles: BridgeScalar::Static(2.0),
            correlation: BridgeScalar::Static(50.0),
            seed: 0,
            combine: crate::api::assets::BridgeSelectorCombine::Add,
            units: crate::api::assets::BridgeSelectorUnits::Percentage,
            randomize: false,
            delay_frames: false,
            total: false,
            repeat: crate::api::assets::BridgeStaggerRepeat::Once,
            bounce_last: false,
        },
        selectors: Vec::new(),
        enabled: true,
        solo: false,
        amount: BridgeScalar::Static(100.0),
        follow: None,
        follow_offset: 0.0,
        skew: zero(),
        skew_axis: zero(),
        tracking: zero(),
        blur: zero(),
        character_offset: zero(),
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
        box_opacity: zero(),
        box_r: BridgeScalar::Static(1.0),
        box_g: BridgeScalar::Static(0.75),
        box_b: BridgeScalar::Static(0.15),
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

/// **The new rows cross and come back**: a second selector with its own
/// mode, the eye, solo, Amount, Follow and the per-letter rows, through the
/// document and back, and a fresh animator writes none of them.
#[test]
fn more_selectors_and_the_new_rows_survive_the_bridge() {
    use crate::api::assets::{
        animators, BridgeSelectorBasis, BridgeSelectorCombine, BridgeSelectorUnits,
        BridgeStaggerRepeat,
    };
    use crate::api::effect::BridgeScalar;
    let zero = lumit_core::time::Rational::new(0, 1).expect("time");
    let fresh = lumit_core::text::TextAnimator::new("A");
    let mut a = animators::read_animator(&fresh, zero);
    let mut second = a.selector.clone();
    second.combine = BridgeSelectorCombine::Intersect;
    second.units = BridgeSelectorUnits::Indexed;
    second.basis = BridgeSelectorBasis::Lines;
    second.randomize = true;
    second.total = true;
    second.repeat = BridgeStaggerRepeat::PingPong;
    second.bounce_last = true;
    second.delay_frames = true;
    a.selectors = vec![second];
    a.enabled = false;
    a.solo = true;
    a.amount = BridgeScalar::Static(50.0);
    a.follow = Some("B".into());
    a.follow_offset = 0.25;
    a.skew = BridgeScalar::Static(20.0);
    a.skew_axis = BridgeScalar::Static(45.0);
    a.tracking = BridgeScalar::Static(100.0);
    a.blur = BridgeScalar::Static(4.0);
    a.character_offset = BridgeScalar::Static(3.0);
    let core = animators::animator_from(a.clone(), zero).expect("written");
    assert_eq!(animators::read_animator(&core, zero), a);
    // A fresh animator written back is the fresh animator: no new keys in the file.
    let back = animators::animator_from(animators::read_animator(&fresh, zero), zero)
        .expect("written");
    assert_eq!(
        serde_json::to_value(&back).expect("json"),
        serde_json::json!({ "name": "A" })
    );
}

/// **A user preset keeps its keys relative**: saved from two seconds in and
/// put at five, a key at three lands at six.
#[test]
fn a_user_preset_moves_its_keys_to_where_it_is_put() {
    use crate::api::assets::{text_animators_from_json, text_animators_to_json};
    use crate::api::effect::{BridgeRational, BridgeScalar};
    let zero = lumit_core::time::Rational::new(0, 1).expect("time");
    let mut a = crate::api::assets::animators::read_animator(
        &lumit_core::text::TextAnimator::new("Rise"),
        zero,
    );
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    let key = |s: i64, value: f64| Keyframe {
        time: lumit_core::time::Rational::new(s, 1).expect("time"),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let keyed = Property {
        animation: Animation::Keyframed(vec![key(3, 40.0), key(4, 0.0)]),
        extra: serde_json::Map::new(),
    };
    a.position_y = BridgeScalar::read_at(&keyed, zero);
    let json = text_animators_to_json(vec![a], BridgeRational { num: 2, den: 1 }).expect("json");
    let back = text_animators_from_json(json, BridgeRational { num: 5, den: 1 }).expect("back");
    let BridgeScalar::Keyframed(keys) = &back[0].position_y else {
        panic!("not keyed: {:?}", back[0].position_y);
    };
    assert_eq!((keys[0].time.num, keys[0].time.den), (6, 1));
    assert_eq!((keys[1].time.num, keys[1].time.den), (7, 1));
}

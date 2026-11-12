//! Tests of expressions.

use super::*;
use crate::model::{LinearColour, TextDocument};

fn document(expression: Option<&str>) -> TextDocument {
    TextDocument {
        text: "typed".into(),
        expression: expression.map(str::to_owned),
        size: 48.0,
        fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
        path: None,
        path_offset: crate::anim::Property::zero(),
        animators: Vec::new(),
        style: Default::default(),
        paragraph: Default::default(),
        captions: None,
        extra: serde_json::Map::new(),
    }
}

fn at(time: f64) -> Arc<ExpressionContext> {
    let mut context = ExpressionContext::detached();
    context.comp_time = time;
    Arc::new(context)
}

/// The point of the feature: a number reaches the screen as words.
#[test]
fn a_number_prints_as_words() {
    assert_eq!(evaluate_text("1 + 1", Some(at(0.0))), "2");
    assert_eq!(evaluate_text("time", Some(at(3.0))), "3.0");
    assert_eq!(evaluate_text("\"frame \" + 7", Some(at(0.0))), "frame 7");
}

/// A document written before expressions existed loads with none, and a
/// document with one round-trips.
#[test]
fn the_field_is_optional_on_disk() {
    let old = r#"{"text":"hi","size":12.0,"fill":[1.0,1.0,1.0,1.0]}"#;
    let d: TextDocument = serde_json::from_str(old).unwrap();
    assert_eq!(d.expression, None);
    // Absent rather than null, so an untouched project file does not grow.
    assert!(!serde_json::to_string(&d).unwrap().contains("expression"));

    let driven = document(Some("time"));
    let json = serde_json::to_string(&driven).unwrap();
    assert_eq!(serde_json::from_str::<TextDocument>(&json).unwrap(), driven);
}

/// An engine handed back to the pool must not carry the last evaluation's
/// context into the next one. Reusing engines is only safe because the tag
/// is cleared on the way back.
#[test]
fn a_pooled_engine_does_not_leak_its_context() {
    assert_eq!(evaluate("time", Some(at(7.0))), 7.0);
    // No context at all: `time` must be unknown again, not still 7.
    assert_eq!(evaluate_text("time", None), "");
}

/// A comp holding `layers`, filed in a document — the least scaffolding an
/// expression that refers to another layer can be evaluated against.
fn doc_with(layers: Vec<crate::model::Layer>) -> (Arc<Document>, Uuid) {
    use crate::model::{Composition, ProjectItem};
    use crate::time::{Duration, FrameRate, Rational};

    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "c".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: crate::model::LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let id = comp.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

/// A solid layer named `name` whose x position is driven by `expression`.
fn driven_layer(name: &str, expression: &str) -> crate::model::Layer {
    use crate::anim::Animation;
    use crate::model::{LayerKind, Switches, TransformGroup};
    use crate::time::{CompTime, Rational};

    let at = |s: i64| CompTime(Rational::new(s, 1).unwrap());
    let mut transform = TransformGroup::default();
    transform.position_x.animation = Animation::Expression(expression.into());

    crate::model::Layer {
        graph: Default::default(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind: LayerKind::Solid {
            def: Uuid::now_v7(),
        },
        in_point: at(0),
        out_point: at(10),
        start_offset: at(0),
        transform,
        matte: None,
        parent: None,
        label: 0,
        volume_db: crate::anim::Property::zero(),
        pan: crate::anim::Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
        retime: None,
        blend: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        markers: Vec::new(),
        paint: Default::default(),
        puppet: None,
        extra: serde_json::Map::new(),
    }
}

/// A node graph's Inputs are readable by name inside the graph
/// (docs/impl/node-graph-comp.md §5.5): the host's own value where it
/// handed one over, the Input's declared default otherwise, a colour's
/// first channel, and the miss value for a picture or a name the graph
/// does not have.
#[test]
fn an_expression_reads_a_graphs_input_by_name() {
    use crate::comp_graph::{CompGraph, GraphInput, GraphNode, InputKind};
    use crate::model::{EffectValue, ProjectItem};

    let declared = |id: &str, kind: InputKind, first: f64| GraphNode::Input {
        id: Uuid::now_v7(),
        input: GraphInput {
            id: id.to_owned(),
            label: format!("The {id}"),
            kind,
            default: [first, 0.25, 0.5, 1.0],
            min: 0.0,
            max: 100.0,
            unit: crate::fx::params::Unit::Raw,
            preview: None,
        },
    };
    let graph = CompGraph {
        widths: Vec::new(),
        notes: Vec::new(),
        ties: Vec::new(),
        looks: Default::default(),
        nodes: vec![
            declared("gain", InputKind::Number, 7.0),
            declared("tint", InputKind::Colour, 0.75),
            declared("plate", InputKind::Picture, 0.0),
            GraphNode::Output { id: Uuid::now_v7() },
        ],
        edges: Vec::new(),
        layout: Vec::new(),
        exposed: Vec::new(),
        groups: Vec::new(),
    };
    let (document, comp) = doc_with(Vec::new());
    let mut doc = (*document).clone();
    if let Some(ProjectItem::Composition(c)) = doc.item_mut(comp) {
        c.graph = Some(graph);
    }
    let context = |values: Option<Vec<(String, EffectValue)>>| {
        Arc::new(ExpressionContext {
            document: Arc::new(doc.clone()),
            comp: Some(comp),
            layer: None,
            comp_time: 2.0,
            current_depth: 0,
            inputs: values.map(Into::into),
        })
    };

    // Nobody handed a value over: the Input's own default answers, by id
    // or by the word on its row.
    assert_eq!(evaluate("input(\"gain\")", Some(context(None))), 7.0);
    assert_eq!(evaluate("input(\"The gain\")", Some(context(None))), 7.0);
    // A colour answers its first channel, which is the number an
    // expression on a number row can use.
    assert_eq!(evaluate("input(\"tint\")", Some(context(None))), 0.75);
    // A picture carries a texture, and an unknown name is a miss: both
    // read as the module's miss value.
    assert_eq!(evaluate("input(\"plate\")", Some(context(None))), -1.0);
    assert_eq!(evaluate("input(\"nothing\")", Some(context(None))), -1.0);

    // A host's own value comes first, and it is read at the graph's time,
    // so a keyed row moves the answer.
    let mut keyed = crate::anim::Property::fixed(0.0);
    keyed.animation = crate::anim::Animation::Expression("time * 5.0".into());
    let over = vec![("gain".to_owned(), EffectValue::Float(keyed))];
    assert_eq!(
        evaluate("input(\"gain\") + 1.0", Some(context(Some(over)))),
        11.0,
        "the host's value at the graph's own time"
    );

    // The depth guard holds: a value that names the Input it is standing
    // in for gives up rather than spinning.
    let mut itself = crate::anim::Property::fixed(0.0);
    itself.animation = crate::anim::Animation::Expression("input(\"gain\")".into());
    let loop_over = vec![("gain".to_owned(), EffectValue::Float(itself))];
    let _ = evaluate("input(\"gain\")", Some(context(Some(loop_over))));
}

/// Expressions nest — a property may read another property that is itself
/// an expression — so evaluation has to survive being re-entered while an
/// engine is already checked out of the pool. This is the test that fails
/// if engines are shared rather than pooled.
#[test]
fn evaluation_can_re_enter_itself() {
    let driven = driven_layer("Driven", "time * 3.0");
    let driven_id = driven.id;
    // Trimmed a second in, its keys still on its own clock from its start.
    let mut trimmed = driven_layer("Trimmed", "0");
    trimmed.in_point.0 = crate::time::Rational::new(1, 1).unwrap();
    trimmed.transform.position_x = crate::anim::Property {
        animation: crate::anim::Animation::Keyframed(
            [0, 4]
                .map(|s| crate::anim::Keyframe {
                    time: crate::time::Rational::new(s, 1).unwrap(),
                    value: s as f64 * 10.0,
                    interp_in: crate::anim::SideInterp::Linear,
                    interp_out: crate::anim::SideInterp::Linear,
                })
                .to_vec(),
        ),
        extra: serde_json::Map::new(),
    };
    let (document, comp) = doc_with(vec![driven, trimmed]);

    let context = Arc::new(ExpressionContext {
        document,
        comp: Some(comp),
        layer: Some(driven_id),
        comp_time: 2.0,
        current_depth: 0,
        inputs: None,
    });
    assert_eq!(
        evaluate("layer(\"Driven\").x + 1.0", Some(context.clone())),
        7.0
    );
    assert_eq!(evaluate("layer(\"Trimmed\").x", Some(context)), 20.0);
}

/// Two properties that read each other must give up rather than recurse
/// until the stack runs out.
#[test]
fn a_cycle_stops_instead_of_overflowing() {
    let a = driven_layer("A", "layer(\"B\").x");
    let a_id = a.id;
    let (document, comp) = doc_with(vec![a, driven_layer("B", "layer(\"A\").x")]);

    let context = Arc::new(ExpressionContext {
        document,
        comp: Some(comp),
        layer: Some(a_id),
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    });
    // The value is meaningless; returning at all is the point.
    let _ = evaluate("layer(\"A\").x", Some(context));
}

/// **One expression cannot build something enormous.**
///
/// Loops do not parse in an expression, so the danger is never a long run.
/// It is one call that allocates: `blob(50_000_000)` and padding an array to
/// five million items both went through untouched before the ceilings, on
/// every property read. A refusal comes back as `-1` like any other.
#[test]
fn an_expression_cannot_build_something_enormous() {
    assert_eq!(evaluate("1 + 2 * 3", None), 7.0);
    assert_eq!(evaluate("if 2 > 1 { 9 } else { 0 }", None), 9.0);
    assert_eq!(evaluate("[1, 2].len()", None), 2.0);
    assert_eq!(evaluate_text("\"frame \" + 7", None), "frame 7");

    assert_eq!(evaluate("blob(50_000_000).len()", None), -1.0);
    assert_eq!(evaluate("[0].pad(5_000_000, 0).len()", None), -1.0);
    assert_eq!(evaluate("\"x\".pad(5_000_000, 'y').len()", None), -1.0);

    let string = |len| format!("\"{}\".len()", "x".repeat(len));
    assert_eq!(
        evaluate(&string(MAX_EXPRESSION_STRING_BYTES), None),
        MAX_EXPRESSION_STRING_BYTES as f64
    );
    assert_eq!(
        evaluate(&string(MAX_EXPRESSION_STRING_BYTES + 1), None),
        -1.0
    );

    // Rhai's array literal refuses *at* the ceiling rather than above it,
    // one item stricter than the same array built by `pad`.
    let array = |len| format!("[{}].len()", vec!["0"; len].join(","));
    assert_eq!(
        evaluate(&array(MAX_EXPRESSION_ARRAY_ITEMS - 1), None),
        (MAX_EXPRESSION_ARRAY_ITEMS - 1) as f64
    );
    assert_eq!(evaluate(&array(MAX_EXPRESSION_ARRAY_ITEMS), None), -1.0);

    let map = |len| {
        format!(
            "#{{{}}}.len()",
            (0..len)
                .map(|n| format!("key_{n}: 0"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    assert_eq!(
        evaluate(&map(MAX_EXPRESSION_MAP_ITEMS), None),
        MAX_EXPRESSION_MAP_ITEMS as f64
    );
    assert_eq!(evaluate(&map(MAX_EXPRESSION_MAP_ITEMS + 1), None), -1.0);

    // An engine that refused one expression still answers the next, and the
    // operation count resets per evaluation. Thirty of these cost more than
    // one budget between them, so a count that carried over would refuse.
    let heavy = array(5_000);
    for _ in 0..30 {
        assert_eq!(evaluate(&heavy, None), 5_000.0);
    }
}

/// A layer carrying what an After Effects shake preset arrives as: a set
/// of controls with a keyed Amount, for expressions on other rows to read.
/// Its rotation holds 30 under an expression of its own.
fn rigged() -> (Arc<Document>, Uuid, Uuid) {
    use crate::anim::{Animation, Keyframe, Property, SideInterp};
    use crate::model::{EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue};
    use crate::time::{CompTime, Rational};

    let key = |seconds: i64, value: f64| Keyframe {
        time: Rational::new(seconds, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let named = |id: &str, name: &str, property: Property| EffectParam {
        id: id.into(),
        value: EffectValue::Float(property),
        extra: [("name".to_owned(), serde_json::json!(name))]
            .into_iter()
            .collect(),
    };
    let mut amount = Property::fixed(0.0);
    amount.animation = Animation::Keyframed(vec![key(1, 0.0), key(2, 40.0), key(4, 0.0)]);

    let mut layer = driven_layer("Rig", "0");
    layer.transform.position_x = Property::fixed(960.0);
    layer.transform.rotation.animation = Animation::Expression("value + 5".into());
    Slot::new(&[30.0], 0, 1)
        .in_language(Language::JavaScript)
        .write(&mut layer.transform.rotation.extra);
    layer.in_point = CompTime(Rational::new(1, 2).unwrap());
    layer.effects.push(EffectInstance {
        id: Uuid::now_v7(),
        effect: EffectKey {
            namespace: EffectNamespace::Placeholder,
            match_name: "Pseudo/1".into(),
            version: 0,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: vec![
            named("p1", "Amount", amount),
            named("p2", "Seed", Property::fixed(3.0)),
        ],
        sample_temporally: true,
        roto: None,
        custom_name: Some("Shake".into()),
        linked_pairs: Vec::new(),
        plugin_state: None,
        extra: serde_json::Map::new(),
    });
    let id = layer.id;
    let (document, comp) = doc_with(vec![layer]);
    (document, comp, id)
}

/// The context an expression on the rigged layer runs under at `time`.
fn on_rig(rig: &(Arc<Document>, Uuid, Uuid), time: f64) -> Arc<ExpressionContext> {
    Arc::new(ExpressionContext {
        document: rig.0.clone(),
        comp: Some(rig.1),
        layer: Some(rig.2),
        comp_time: time,
        current_depth: 0,
        inputs: None,
    })
}

/// **An After Effects expression reads what After Effects lets it read.**
///
/// The effect and its rows by name, the keyframes under a row, the comp
/// and the layer. This is the whole reason the second language exists: a
/// rig built there has to find its controls here.
#[test]
fn an_after_effects_expression_reads_its_layer() {
    let rig = rigged();
    let number = |source: &str, time: f64| {
        evaluate_in(Language::JavaScript, source, Some(on_rig(&rig, time)))
    };

    assert_eq!(number("effect(\"Shake\")(\"Amount\") / 2", 1.5), 10.0);
    assert_eq!(
        number(
            "var c = effect('Shake')\n('Seed'); Math.floor(c) + effect(1)(2)",
            0.0
        ),
        6.0
    );
    assert_eq!(
        number("thisLayer.effect(1).param('amount').valueAtTime(2)", 0.0),
        40.0
    );
    assert_eq!(
        number(
            "effect('Shake').name.length + (effect('Shake').active ? 1 : 0)",
            0.0
        ),
        6.0
    );

    // Keyframes are counted from one and timed on the comp's clock.
    let keys = "var p = effect('Shake')('Amount'); var last = p.key(p.numKeys); \
                    p.numKeys * 100 + last.time * 10 + p.nearestKey(1.9).index";
    assert_eq!(number(keys, 0.0), 342.0);
    let rate = number("effect('Shake')('Amount').velocityAtTime(1.5)", 0.0);
    assert!((rate - 40.0).abs() < 1e-6, "{rate}");

    // A row's keys can be made to repeat past their end, or before their start.
    let looped =
        |kind: &str, time: f64| number(&format!("effect('Shake')('Amount').{kind}"), time).round();
    assert_eq!(looped("loopOut()", 3.0), 20.0);
    assert_eq!(looped("loopOut('cycle')", 5.0), 40.0);
    assert_eq!(looped("loopOut('pingpong')", 5.0), 20.0);
    assert_eq!(looped("loopOut('offset')", 5.0), 40.0);
    assert_eq!(looped("loopOut('continue')", 5.0), -20.0);
    assert_eq!(looped("loopOut('cycle', 1)", 5.0), 20.0);
    assert_eq!(looped("loopOutDuration('cycle', 1)", 4.5), 10.0);
    assert_eq!(looped("loopIn('cycle')", 0.0), 20.0);

    assert_eq!(
        number("thisComp.width / 2 + thisComp.height + index", 0.0),
        2041.0
    );
    assert_eq!(
        number("Math.round((inPoint + thisComp.frameDuration) * 60)", 0.0),
        31.0
    );
    assert_eq!(
        number(
            "thisComp.layer('Rig').transform.position[0] + thisLayer.name.length",
            0.0
        ),
        963.0
    );
    assert_eq!(
        number(
            "comp('c').layer(1).position.length + thisComp.numLayers",
            0.0
        ),
        3.0
    );
    assert_eq!(number("timeToFrames(1.5) + framesToTime(30)", 0.0), 90.5);
    assert_eq!(number("posterizeTime(2); time", 1.7), 1.5);
    assert_eq!(number("hasParent ? 1 : (parent == null ? 2 : 3)", 0.0), 2.0);
}

/// **`value` is what the property held before its expression.**
///
/// A position is two properties here and one in After Effects, so both
/// carry the same text and each takes its own number of the answer. The
/// same slot is what `wiggle` wanders around, and what a broken expression
/// falls back to.
#[test]
fn value_is_what_the_property_held_before_its_expression() {
    use crate::anim::{Animation, Property};

    let rig = rigged();
    let driven = |source: &str, slot: Slot, time: f64| {
        let mut property = Property::fixed(0.0);
        property.animation = Animation::Expression(source.into());
        slot.in_language(Language::JavaScript)
            .write(&mut property.extra);
        property.value_at_with_context(0.0, on_rig(&rig, time))
    };

    let shaken = "value + [effect('Shake')('Amount'), 1]";
    assert_eq!(
        driven(shaken, Slot::new(&[960.0, 540.0], 0, 5), 2.0),
        1000.0
    );
    assert_eq!(driven(shaken, Slot::new(&[960.0, 540.0], 1, 5), 2.0), 541.0);
    // One put on a point's y row in the app reads the whole point, as an
    // import gives it, so the row neither jumps nor takes the x.
    let mut layer = driven_layer("Pin", "0");
    layer.transform.position_x = Property::fixed(960.0);
    layer.transform.position_y = Property::fixed(540.0);
    let mut y = layer.transform.position_y.clone();
    y.set_animation(Animation::Expression("value + [0, 10]".into()));
    y.set_expression_language(Language::JavaScript);
    note_transform_point(&layer, crate::model::TransformProp::PositionY, &mut y);
    assert_eq!(y.value_at_with_context(0.0, on_rig(&rig, 0.0)), 550.0);
    // A separated y row is a number of its own, as in After Effects, and
    // still takes its own number of a list.
    layer.transform.axis_modes.position = crate::model::AxisMode::Separated;
    for (text, want) in [("value + 10", 550.0), ("[1, 2]", 2.0)] {
        let mut y = layer.transform.position_y.clone();
        y.set_animation(Animation::Expression(text.into()));
        y.set_expression_language(Language::JavaScript);
        note_transform_point(&layer, crate::model::TransformProp::PositionY, &mut y);
        assert_eq!(y.value_at_with_context(0.0, on_rig(&rig, 0.0)), want);
    }
    assert_eq!(
        driven(
            "value.length > 1 ? value[1] : value",
            Slot::new(&[7.0], 0, 0),
            0.0
        ),
        7.0
    );

    // A wiggle stays within its amount, answers the same every time it
    // is asked, moves, and is another property's on another seed.
    let wiggle = |seed: u32, time: f64| driven("wiggle(2, 30)", Slot::new(&[100.0], 0, seed), time);
    let a = wiggle(1, 0.3);
    assert_eq!(a, wiggle(1, 0.3));
    assert!((a - 100.0).abs() <= 30.0, "{a}");
    assert_ne!(a, wiggle(1, 0.8));
    assert_ne!(a, wiggle(2, 0.3));
    // Seeding it by hand moves it too, and both numbers of a point wander
    // on their own.
    assert_ne!(
        driven(
            "seedRandom(8, true); wiggle(2, 30)",
            Slot::new(&[100.0], 0, 1),
            0.3
        ),
        a
    );
    let pair = "var w = wiggle(2, 30, 2) - value; w[0] - w[1]";
    assert_ne!(driven(pair, Slot::new(&[5.0, 5.0], 0, 1), 0.3), 0.0);
    assert_eq!(
        driven("wiggle(0, 0)", Slot::new(&[100.0], 0, 1), 0.3),
        100.0
    );

    // Its own property read by name is that property before the
    // expression, not the expression run again: the rig's rotation holds
    // 30 under an expression of its own.
    assert_eq!(driven("rotation + 1", Slot::new(&[30.0], 0, 1), 0.0), 31.0);

    // A broken expression leaves the property where it was.
    assert_eq!(
        driven(
            "value + effect('Gone')('Amount')",
            Slot::new(&[12.0], 0, 0),
            0.0
        ),
        12.0
    );
    // With nothing recorded there is nothing to fall back to.
    assert_eq!(
        driven("effect('Gone')('Amount')", Slot::default(), 0.0),
        -1.0
    );

    // Putting an expression on a property by hand writes down what it
    // held, an edit of the text keeps that, and taking it off forgets it.
    let mut typed = Property::fixed(64.0);
    typed.set_animation(Animation::Expression("64".into()));
    assert_eq!(typed.expression_language(), Language::Rhai);
    typed.set_expression_language(Language::JavaScript);
    typed.set_animation(Animation::Expression("value / 2 + wiggle(0, 0)".into()));
    assert_eq!(typed.expression_language(), Language::JavaScript);
    assert_eq!(typed.value_at_with_context(0.0, on_rig(&rig, 0.0)), 96.0);
    typed.set_animation(Animation::Static(1.0));
    assert!(typed.extra.is_empty());

    // A panel holds the text and not the property, and still reads the
    // slot: the rig's rotation is 30 under `value + 5`.
    let js = Language::JavaScript;
    assert_eq!(evaluate_in(js, "value + 5", Some(on_rig(&rig, 0.0))), 35.0);
    assert_eq!(
        evaluate_range(
            js,
            "value + 5",
            Some(&*on_rig(&rig, 0.0)),
            0.0,
            1.0,
            3,
            None
        ),
        [35.0, 35.0, 35.0]
    );
    // A line still being typed has no curve to draw, in either language.
    assert!(evaluate_range(js, "value +", None, 0.0, 1.0, 3, None).is_empty());
    assert!(evaluate_range(Language::Rhai, "time +", None, 0.0, 1.0, 3, None).is_empty());
}

/// **An expression is run by the language it was given, never by a guess.**
///
/// The same five characters mean two things: Rhai divides whole numbers as
/// whole numbers and JavaScript does not. So the language is a choice
/// stored beside the text, and a property with nothing stored is Rhai,
/// which is every expression a project already holds.
#[test]
fn an_expression_is_run_by_the_language_it_was_given() {
    use crate::anim::{Animation, Property};

    let js = Language::JavaScript;
    assert_eq!(evaluate("7 / 2", None), 3.0);
    assert_eq!(evaluate_in(Language::Rhai, "7 / 2", None), 3.0);
    assert_eq!(evaluate_in(js, "7 / 2", None), 3.5);

    let mut property = Property::fixed(0.0);
    property.set_animation(Animation::Expression("7 / 2".into()));
    assert_eq!(property.value_at(0.0), 3.0, "nothing stored is Rhai");
    property.set_expression_language(js);
    assert_eq!(property.value_at(0.0), 3.5);
    property.set_expression_language(Language::Rhai);
    assert_eq!(property.value_at(0.0), 3.0);
    assert_eq!(Language::from_id("something newer"), Language::Rhai);
    assert_eq!(Language::from_id(js.id()), js);

    // What the importer asks of an After Effects expression: does it read
    // as JavaScript, with every name one Lumit has.
    for known in [
        "Math.sin(time)",
        "wiggle(5, 20)",
        "x = 7 / 2; x",
        "[thisComp.width / 2, thisComp.height / 2]",
        "effect(\"Shake\")(\"Seed\")",
        "input_1 * 2",
    ] {
        assert!(is_javascript(known), "{known}");
    }
    for refused in ["nonesuch(time)", "if 2 > 1 { 9 } else { 0 }", "1 +"] {
        assert!(!is_javascript(refused), "{refused}");
    }
    assert!(is_runnable("time * 90") && !is_runnable("Math.sin(time)"));

    // The typed entry points answer in the language named.
    let value = |source: &str, vars: &[(&str, f64)]| evaluate_value_in(js, source, None, vars);
    assert_eq!(
        value("[Math.abs(-3), 4]", &[]),
        Ok(ExprValue::Point(3.0, 4.0))
    );
    assert_eq!(
        value("input_1 + Math.PI * 0", &[("input_1", 2.0)]),
        Ok(ExprValue::Number(2.0))
    );
    assert_eq!(
        value("[Math.abs(1), 0, 0]", &[]),
        Ok(ExprValue::Colour([1.0, 0.0, 0.0, 1.0]))
    );
    assert!(value("'words' + Math.PI", &[]).is_err());
    assert!(value("Math.nothing(1)", &[]).is_err());
    assert!(value("1 +", &[]).is_err_and(|why| why.contains("line 1")));
    assert!(
        evaluate_value("Math.abs(-3)", None, &[]).is_err(),
        "Rhai has no Math"
    );
}

/// **A value expression says what it answered with, or why it did not.**
///
/// [`evaluate`] turns every refusal into `-1`, which is the right bargain
/// for a property that must produce a number for every frame and the wrong
/// one for anything that has to report. Each arm here was a plausible wrong
/// answer under the old road.
#[test]
fn a_value_expression_answers_with_its_type_or_with_a_reason() {
    let value = |src: &str| evaluate_value(src, None, &[]);

    // Rhai keeps whole numbers and fractions apart, and a boolean is a
    // number here as it is everywhere else in this file.
    assert_eq!(value("2.5"), Ok(ExprValue::Number(2.5)));
    assert_eq!(value("2"), Ok(ExprValue::Number(2.0)));
    assert_eq!(value("true"), Ok(ExprValue::Number(1.0)));

    assert_eq!(value("[100, 200.5]"), Ok(ExprValue::Point(100.0, 200.5)));

    // Three numbers is a colour at full alpha; four states its own.
    assert_eq!(
        value("[1.0, 0.5, 0.0]"),
        Ok(ExprValue::Colour([1.0, 0.5, 0.0, 1.0]))
    );
    assert_eq!(
        value("[1.0, 0.5, 0.0, 0.25]"),
        Ok(ExprValue::Colour([1.0, 0.5, 0.0, 0.25]))
    );

    // The refusals, each of which `evaluate` would have called -1.
    assert!(value("this is not (").is_err(), "it does not parse");
    assert!(value("nonesuch()").is_err(), "the name does not exist");
    assert!(value("\"words\"").is_err(), "a string is not a value");
    assert!(
        value("[1, 2, 3, 4, 5]").is_err(),
        "five of anything is nothing"
    );
    assert!(value("[1, \"two\"]").is_err(), "a pair holding a word");
}

/// A camera turned a little, a 3D null turned more, a 3D card parented to
/// it, and a 2D layer and a 3D one with the same flat transform.
fn staged() -> (Arc<Document>, Uuid, Vec<(String, Uuid)>) {
    use crate::anim::Property;
    use crate::model::{CameraOptions, LayerKind};

    let mut flat = driven_layer("Flat", "0");
    let tr = &mut flat.transform;
    (tr.position_x, tr.position_y) = (Property::fixed(100.0), Property::fixed(50.0));
    tr.anchor_x = Property::fixed(10.0);
    tr.scale_x = Property::fixed(200.0);
    let mut plate = flat.clone();
    (plate.id, plate.name) = (Uuid::now_v7(), "Plate".into());
    plate.switches.three_d = true;

    let zoom = crate::camera::default_zoom(1920.0);
    let mut camera = driven_layer("Camera", "0");
    camera.kind = LayerKind::Camera {
        zoom: Property::fixed(zoom),
        solve_link: None,
        correction_base: None,
        options: Box::new(CameraOptions::fresh(zoom, (960.0, 540.0, 0.0))),
    };
    let tr = &mut camera.transform;
    (tr.position_x, tr.position_y) = (Property::fixed(900.0), Property::fixed(500.0));
    tr.position_z = Property::fixed(-zoom);
    (tr.rotation_x, tr.rotation_y) = (Property::fixed(-5.0), Property::fixed(10.0));

    let mut null = driven_layer("Null", "0");
    null.switches.three_d = true;
    let tr = &mut null.transform;
    (tr.position_x, tr.position_y) = (Property::fixed(960.0), Property::fixed(540.0));
    tr.position_z = Property::fixed(100.0);
    tr.rotation_y = Property::fixed(30.0);
    let mut card = driven_layer("Card", "0");
    card.switches.three_d = true;
    card.parent = Some(null.id);
    let tr = &mut card.transform;
    (tr.position_x, tr.position_z) = (Property::fixed(100.0), Property::fixed(200.0));
    (tr.anchor_x, tr.anchor_y) = (Property::fixed(50.0), Property::fixed(50.0));
    (tr.scale_x, tr.scale_y) = (Property::fixed(50.0), Property::fixed(50.0));
    tr.rotation = Property::fixed(15.0);

    let layers = vec![camera, null, card, flat, plate];
    let names = layers.iter().map(|l| (l.name.clone(), l.id)).collect();
    let (document, comp) = doc_with(layers);
    (document, comp, names)
}

/// **A point crosses between a layer's spaces and back, in both
/// languages.** On the picture a 2D layer is its flat placement, a 3D one
/// is that placement seen through the camera, and every way there has a
/// way back that lands on the point it started from.
#[test]
fn a_point_crosses_a_layers_spaces_and_back_in_both_languages() {
    let (document, comp, names) = staged();
    let on = |name: &str| {
        let layer = names.iter().find(|(n, _)| n == name).map(|(_, id)| *id);
        Arc::new(ExpressionContext {
            document: document.clone(),
            comp: Some(comp),
            layer,
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        })
    };
    let js = |source: &str| evaluate_in(Language::JavaScript, source, Some(on("Flat")));
    let rhai = |source: &str| evaluate_in(Language::Rhai, source, Some(on("Flat")));

    // The 2D layer is never seen through the camera, and keeps as many
    // numbers as it was given.
    assert_eq!(js("toComp([5, 5])[0] * 1000 + toComp([5, 5])[1]"), 90_055.0);
    assert_eq!(
        js("toComp([5, 5, 7]).length * 10 + toComp([5, 5, 7])[2]"),
        30.0
    );
    assert_eq!(js("fromComp([90, 55])[0] + fromComp([90, 55])[1]"), 10.0);
    assert_eq!(rhai("layer().to_comp([5.0, 5.0])[0]"), 90.0);
    // The 3D one with the same transform is in the same place in the
    // comp's space, and on the picture where the camera puts that.
    assert_eq!(js("thisComp.layer('Plate').toWorld([5, 5])[0]"), 90.0);
    let pose = document.comp(comp).and_then(|c| c.camera_pose(0.0));
    let (x, y, depth) =
        crate::camera::project_depth(&pose.expect("a camera"), 1920.0, 1080.0, (90.0, 55.0, 0.0));
    for (axis, want) in [x, y, depth].into_iter().enumerate() {
        let got = js(&format!(
            "thisComp.layer('Plate').toComp([5, 5, 0])[{axis}]"
        ));
        assert!((got - want).abs() < 1e-9, "{axis}: {got} against {want}");
    }

    // Every way and its way back, at a time given and at the time now.
    // Not toCompVec through a camera: After Effects' fromCompVec isn't
    // quite its inverse, and the render tests hold both to its numbers.
    let ways = [
        ("toComp", "fromComp", "to_comp", "from_comp"),
        ("toWorld", "fromWorld", "to_world", "from_world"),
        (
            "toWorldVec",
            "fromWorldVec",
            "to_world_vec",
            "from_world_vec",
        ),
    ];
    for (to, from, to_r, from_r) in ways {
        let off = js(&format!(
            "var L = thisComp.layer('Card'); var q = L.{from}(L.{to}([3, 4, 5], 1)); \
                 1000 + Math.abs(q[0] - 3) + Math.abs(q[1] - 4) + Math.abs(q[2] - 5)"
        ));
        assert!(
            (off - 1000.0).abs() < 1e-6,
            "{to} and {from} in JavaScript: {off}"
        );
        // Rhai takes an expression rather than statements, so the
        // card is named each time.
        let card = "layer(\"Card\")";
        for back in [
            format!("{card}.{from_r}({card}.{to_r}([3.0, 4.0, 5.0]), 1.0)"),
            format!("{card}.{from_r}({card}.{to_r}([3.0, 4.0, 5.0], 1.0))"),
        ] {
            let off = rhai(&format!(
                "1000.0 + abs({back}[0] - 3.0) + abs({back}[1] - 4.0) + abs({back}[2] - 5.0)"
            ));
            assert!((off - 1000.0).abs() < 1e-6, "{back} in Rhai: {off}");
        }
    }
    // A point on the picture meets the card's own plane where it showed.
    let off = js(
        "var L = thisComp.layer('Card'); var q = L.fromCompToSurface(L.toComp([7, 8, 0])); \
             q.length * 100 + Math.abs(q[0] - 7) + Math.abs(q[1] - 8)",
    );
    assert!((off - 200.0).abs() < 1e-6, "{off}");
    let back = "layer(\"Card\").from_comp_to_surface(layer(\"Card\").to_comp([7.0, 8.0, 0.0]))";
    let off = rhai(&format!(
        "1000.0 + abs({back}[0] - 7.0) + abs({back}[1] - 8.0)"
    ));
    assert!((off - 1000.0).abs() < 1e-6, "{off}");
    let at = rhai("layer(\"Card\").from_comp_to_surface([960.0, 540.0], 1.0).len()");
    assert_eq!(at, 2.0);

    // A 3D layer's points have a z, its turn is in the three rotations,
    // and the camera answers for its own rows.
    let zoom = crate::camera::default_zoom(1920.0);
    assert_eq!(js("thisComp.layer('Card').position.length"), 3.0);
    assert_eq!(js("thisComp.layer('Card').transform.position[2]"), 200.0);
    assert_eq!(
        js("thisComp.layer('Card').anchorPoint[2] + thisComp.layer('Card').scale[2]"),
        100.0
    );
    assert_eq!(
        js("thisComp.layer('Null').yRotation + thisComp.layer('Card').zRotation"),
        45.0
    );
    assert_eq!(
        js("thisComp.layer('Camera').xRotation + thisComp.layer('Card').orientation[1]"),
        -5.0
    );
    assert_eq!(js("thisComp.activeCamera.zoom"), zoom);
    assert_eq!(
        js("thisComp.activeCamera.cameraOption.pointOfInterest[0]"),
        960.0
    );
    assert_eq!(
        rhai("layer(\"Card\").z + layer(\"Null\").rotation_y + layer(\"Card\").rotation_z"),
        245.0
    );
    assert_eq!(
        rhai("layer(\"Camera\").rotation_x + layer(\"Card\").orientation[2]"),
        -5.0
    );
    assert_eq!(rhai("comp().active_camera.zoom"), zoom);
    assert_eq!(rhai("comp().active_camera.point_of_interest[1]"), 540.0);
    assert_eq!(rhai("layer(\"Card\").zoom"), -1.0, "a card has no zoom");
}

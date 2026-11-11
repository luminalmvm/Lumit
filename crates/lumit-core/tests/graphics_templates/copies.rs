//! A placed copy: the values it keeps as its own, fed or following the
//! template, the rows it shows, the document it plays, its clock, and what an
//! expression reads from it by a control's name.

use std::sync::Arc;

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::expression::script::Language;
use lumit_core::expression::{evaluate_value_in, ExprValue};
use lumit_core::fx::builtins;
use lumit_core::fx::effects::template::{self as values, Held};
use lumit_core::fx::schema::ParamKind;
use lumit_core::model::{EffectInstance, EffectValue, LayerKind, TransformProp};
use lumit_core::ops::{self, Op};
use lumit_core::template::time::{copy_map, Ends, Timing};
use lumit_core::template::{
    author, placed, rows, Fit, Picture, Setting, Target, TextRow, Value, PLAY_INTRO, PLAY_OUTRO,
};
use uuid::Uuid;

use crate::kit::{self, control, secs, Kit};

fn opacity(kit: &Kit) -> Target {
    Target::Transform {
        layer: kit.plate,
        props: vec![TransformProp::Opacity],
    }
}

fn row(inst: &EffectInstance, id: &str) -> Option<EffectValue> {
    inst.params
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.value.clone())
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn fixed(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

#[test]
fn a_copy_keeps_its_own_values_and_its_data_rows_apart() {
    let (kit, template) = kit::kit_with_template();
    let c = |id: &str| control(&template, id);
    let def = builtins::def(values::MATCH_NAME).unwrap();
    let mut inst = values::instance_for(kit.tpl, &template);
    assert_eq!(values::comp_of(&inst), Some(kit.tpl));
    let held = values::held(&inst);
    assert_eq!(held.controls.len(), 10);
    assert!(held.intro && held.outro);
    assert!(values::is_bare(&inst));
    assert_eq!(values::ends(&inst), Ends::BOTH);

    let derived: Vec<&str> = def.derived(&inst).iter().map(|r| r.id).collect();
    assert_eq!(
        derived,
        [
            "title",
            "title_font",
            "title_size",
            "title_colour",
            "opacity",
            "position_x",
            "position_y",
            "tint",
            "glow_on",
            "style",
            "logo",
            "logo_fit",
            "spin",
            "font",
            "radius",
            PLAY_INTRO,
            PLAY_OUTRO
        ]
    );
    let words = vec!["NTSC".to_owned(), "PAL".to_owned()];
    assert_eq!(def.row_state(&inst).options, vec![("style", words)]);

    let picture = Value::Picture(Picture {
        item: kit.badge,
        size: (200, 200),
        fit: Fit::Stretch,
    });
    let set = vec![
        ("title", Value::Text("Mine".into())),
        ("opacity", Value::Number(25.0)),
        ("position", Value::Point(vec![1.0, 2.0])),
        ("tint", Value::Colour([0.5, 0.5, 0.25, 1.0])),
        ("glow_on", Value::Tick(false)),
        ("style", Value::Choice(1)),
        ("logo", picture),
        ("spin", Value::Number(90.0)),
        (
            "font",
            Value::Font {
                family: "Mono".into(),
                face: "Bold".into(),
            },
        ),
    ];
    for (id, value) in &set {
        assert!(values::set_value(&mut inst, c(id), value), "{id}");
        assert_eq!(
            values::value_of(&inst, c(id), 0.0, None).as_ref(),
            Some(value)
        );
    }
    assert!(!values::set_value(
        &mut inst,
        c("opacity"),
        &Value::Text("no".into())
    ));
    assert_eq!(values::value_of(&inst, c("radius"), 0.0, None), None);
    assert!(!values::is_bare(&inst));

    // A text control's further rows reach the same layer.
    values::set_row(&mut inst, "title_size", fixed(90.0));
    values::set_row(
        &mut inst,
        "title_font",
        EffectValue::Text("Serif\nItalic".into()),
    );
    let red = [1.0, 0.0, 0.0, 1.0].map(Property::fixed);
    values::set_row(&mut inst, "title_colour", EffectValue::Colour(red));
    let settings = values::settings_at(&inst, &template, 0.0, None, None);
    let wants = [
        Setting {
            target: Target::Font { layer: kit.title },
            value: Value::Font {
                family: "Serif".into(),
                face: "Italic".into(),
            },
        },
        Setting {
            target: Target::TextRow {
                layer: kit.title,
                value: TextRow::Size,
            },
            value: Value::Number(90.0),
        },
        Setting {
            target: Target::TextRow {
                layer: kit.title,
                value: TextRow::Fill,
            },
            value: Value::Colour([1.0, 0.0, 0.0, 1.0]),
        },
    ];
    for want in &wants {
        assert!(settings.contains(want), "{want:?}");
    }
    assert_eq!(settings.len(), set.len() + wants.len());

    values::set_row(&mut inst, PLAY_OUTRO, EffectValue::Bool(false));
    assert_eq!(
        values::ends(&inst),
        Ends {
            intro: true,
            outro: false
        }
    );

    // A data row feeds what the copy has not set itself.
    assert!(values::feed_value(
        &mut inst,
        c("opacity"),
        &Value::Number(70.0)
    ));
    assert_eq!(
        values::value_of(&inst, c("opacity"), 0.0, None),
        Some(Value::Number(25.0))
    );
    assert!(values::feed_value(
        &mut inst,
        c("radius"),
        &Value::Number(12.0)
    ));
    assert!(!values::feed_value(
        &mut inst,
        c("radius"),
        &Value::Tick(true)
    ));
    values::feed_row(&mut inst, "title_size", &Value::Number(40.0));
    values::feed_row(&mut inst, "radius_x", &Value::Tick(true));
    let mut fresh = values::instance_for(kit.tpl, &template);
    values::feed_row(&mut fresh, "title_size", &Value::Number(40.0));
    values::feed_row(
        &mut fresh,
        "title_colour",
        &Value::Colour([0.0, 1.0, 0.0, 1.0]),
    );
    values::feed_row(
        &mut fresh,
        "title_font",
        &Value::Font {
            family: "Inter".into(),
            face: "Bold".into(),
        },
    );
    assert_eq!(values::held(&fresh).fed.len(), 3);
    assert_eq!(
        row(&inst, "title_size"),
        Some(fixed(90.0)),
        "set by hand, kept"
    );
    assert_eq!(values::held(&inst).fed, ["radius"]);
    values::follow(&mut inst, Some(2));
    assert_eq!(values::held(&inst).row, Some(2));
    values::unfeed(&mut inst);
    assert_eq!(values::value_of(&inst, c("radius"), 0.0, None), None);
    assert_eq!(
        values::value_of(&inst, c("opacity"), 0.0, None),
        Some(Value::Number(25.0))
    );
    values::clear_rows(&mut inst, &["opacity".to_owned()]);
    assert_eq!(values::value_of(&inst, c("opacity"), 0.0, None), None);
    assert!(!values::held(&inst).has("opacity"));
}

#[test]
fn a_copy_shown_offers_the_template_and_stores_only_what_moved() {
    let (kit, template) = kit::kit_with_template();
    let c = |id: &str| control(&template, id);
    let def = builtins::def(values::MATCH_NAME).unwrap();
    let mut stored = values::instance_for(kit.tpl, &template);
    values::set_value(&mut stored, c("title"), &Value::Text("Mine".into()));
    values::feed_value(&mut stored, c("radius"), &Value::Number(12.0));

    let mut shown = stored.clone();
    values::offer(&mut shown, &kit.doc);
    assert_eq!(row(&shown, "opacity"), Some(fixed(100.0)));
    assert_eq!(row(&shown, "title"), Some(EffectValue::Text("Mine".into())));
    assert_eq!(
        row(&shown, "title_font"),
        Some(EffectValue::Text("Inter\nRegular".into()))
    );
    assert_eq!(row(&shown, "title_size"), Some(fixed(72.0)));
    assert_eq!(row(&shown, "position_y"), Some(fixed(540.0)));
    assert_eq!(row(&shown, PLAY_INTRO), Some(EffectValue::Bool(true)));
    assert_eq!(
        row(&shown, "radius"),
        Some(fixed(12.0)),
        "a fed row shows its feed"
    );

    // Each row's default is the template's own, so a reset follows it.
    let derived = def.derived(&shown);
    let default_of = |id: &str| derived.iter().find(|r| r.id == id).unwrap().kind;
    assert!(matches!(default_of("opacity"), ParamKind::Float { default, .. } if default == 100.0));
    assert!(matches!(default_of("title"), ParamKind::Text { default, .. } if default == "Hello"));
    assert!(
        matches!(default_of("title_size"), ParamKind::Float { default, .. } if default == 72.0)
    );
    assert!(matches!(default_of("tint"), ParamKind::Colour { default, .. } if default == [1.0; 4]));
    assert!(matches!(
        default_of("glow_on"),
        ParamKind::Bool { default: true, .. }
    ));
    assert!(matches!(
        default_of("style"),
        ParamKind::Choice { default: 0, .. }
    ));
    assert!(matches!(default_of("spin"), ParamKind::Angle { default, .. } if default == 0.0));
    assert!(
        matches!(default_of("position_x"), ParamKind::Float { default, .. } if default == 960.0)
    );
    assert!(
        matches!(default_of("logo_fit"), ParamKind::Choice { options, .. } if options.len() == 3)
    );

    // Stored as it was shown: only the copy's own and its fed rows stay.
    let mut same = shown.clone();
    values::settle(&mut same, Some(&stored));
    let kept = values::held(&same);
    assert_eq!(
        (kept.own.clone(), kept.fed.clone()),
        (vec!["title".into()], vec!["radius".into()])
    );
    assert!(kept.live.is_empty());
    let ids: Vec<String> = same.params.iter().map(|p| p.id.clone()).collect();
    assert_eq!(sorted(ids), ["radius", "title"]);

    // One axis moved makes the whole point the copy's own, a fed row moved
    // is now the copy's own, and a row put back at the template's value
    // follows the template again.
    let mut edited = shown.clone();
    for p in &mut edited.params {
        match p.id.as_str() {
            "position_x" => p.value = fixed(10.0),
            "radius" => p.value = fixed(13.0),
            "title" => p.value = EffectValue::Text("Hello".into()),
            _ => {}
        }
    }
    values::settle(&mut edited, Some(&stored));
    let kept = values::held(&edited);
    assert_eq!(
        sorted(kept.own.clone()),
        ["position_x", "position_y", "radius"]
    );
    assert!(kept.fed.is_empty());
    assert_eq!(values::value_of(&edited, c("title"), 0.0, None), None);
    assert_eq!(
        values::value_of(&edited, c("position"), 0.0, None),
        Some(Value::Point(vec![10.0, 540.0]))
    );

    // Never handed out, an instance keeps the marks it has.
    let mut plain = stored.clone();
    values::settle(&mut plain, None);
    assert_eq!(values::held(&plain).own, ["title"]);
    assert_eq!(values::held(&plain).fed, ["radius"]);

    // A copy of something that is not a template is offered nothing.
    let mut stray = values::instance_for(kit.logo_comp, &template);
    values::offer(&mut stray, &kit.doc);
    assert!(stray.params.is_empty());
}

#[test]
fn a_copy_follows_its_template_through_edits_and_new_ids() {
    let (mut kit, template) = kit::kit_with_template();
    let mut inst = values::instance_for(kit.tpl, &template);
    assert!(!values::refresh(&mut inst, &kit.doc), "already up to date");

    let mut grown = template.clone();
    let mut shown = author::control_for(
        &kit.doc,
        kit.comp(),
        Target::Visible { layer: kit.bar },
        "Bar shown",
    )
    .unwrap();
    shown.id = grown.free_id("Bar shown");
    grown.controls.push(shown);
    grown.outro = None;
    kit::install(&mut kit, &grown);
    assert!(values::refresh(&mut inst, &kit.doc));
    let held = values::held(&inst);
    assert_eq!(held.controls.len(), 11);
    assert!(held.intro && !held.outro);
    assert!(!values::refresh(&mut inst, &kit.doc));
    let def = builtins::def(values::MATCH_NAME).unwrap();
    assert!(def.derived(&inst).iter().any(|r| r.id == "bar_shown"));
    assert!(!def.derived(&inst).iter().any(|r| r.id == PLAY_OUTRO));

    values::rebind(&mut inst, kit.logo_comp);
    assert_eq!(values::comp_of(&inst), Some(kit.logo_comp));
    assert!(
        !values::refresh(&mut inst, &kit.doc),
        "the logo has no controls"
    );
    let mut fill = builtins::instantiate("fill").unwrap();
    values::rebind(&mut fill, kit.tpl);
    assert_eq!(values::comp_of(&fill), None);
    assert_eq!(values::held(&fill), Held::default());
    assert!(def.derived(&fill).is_empty());

    let host = Uuid::now_v7();
    assert_eq!(values::offered_id(host), values::offered_id(host));
    assert_ne!(values::offered_id(host), values::offered_id(Uuid::now_v7()));
}

#[test]
fn a_menu_look_comes_first_and_a_copys_own_value_wins() {
    let (mut kit, template) = kit::kit_with_template();
    let title = Target::Text { layer: kit.title };
    let mut looked = template.clone();
    let style = looked
        .controls
        .iter_mut()
        .find(|c| c.id == "style")
        .unwrap();
    style.looks = vec![
        Vec::new(),
        vec![
            Setting {
                target: opacity(&kit),
                value: Value::Number(30.0),
            },
            Setting {
                target: title.clone(),
                value: Value::Text("Night".into()),
            },
        ],
    ];
    kit::install(&mut kit, &looked);
    let mut inst = values::instance_for(kit.tpl, &looked);
    assert!(values::held(&inst)
        .controls
        .iter()
        .all(|c| c.looks.is_empty()));
    values::set_value(&mut inst, control(&looked, "style"), &Value::Choice(1));
    values::set_value(
        &mut inst,
        control(&looked, "title"),
        &Value::Text("Own".into()),
    );

    let settings = values::settings_at(&inst, &looked, 0.0, None, None);
    let order: Vec<&Value> = settings.iter().map(|s| &s.value).collect();
    assert_eq!(
        order,
        [
            &Value::Number(30.0),
            &Value::Text("Night".into()),
            &Value::Text("Own".into()),
            &Value::Choice(1)
        ]
    );
    let doc = Arc::new(kit.doc.clone());
    let made = placed::with(&doc, kit.tpl, &settings).unwrap();
    let c = made.comp(kit.tpl).unwrap();
    assert_eq!(
        rows::read(&made, c, &title),
        Some(Value::Text("Own".into()))
    );
    assert_eq!(
        rows::read(&made, c, &opacity(&kit)),
        Some(Value::Number(30.0))
    );
}

#[test]
fn a_copy_plays_its_own_document_and_the_project_is_left_alone() {
    let (mut kit, template) = kit::kit_with_template();
    let c = |id: &str| control(&template, id);
    let mut on_layer = values::instance_for(kit.tpl, &template);
    values::set_value(&mut on_layer, c("opacity"), &Value::Number(25.0));
    values::set_value(&mut on_layer, c("title"), &Value::Text("Copy one".into()));
    // The clip's opacity runs from nothing to full over its first second.
    let mut on_clip = values::instance_for(kit.tpl, &template);
    values::set_value(&mut on_clip, c("opacity"), &Value::Number(0.0));
    let key = |s: i64, value: f64| Keyframe {
        time: secs(s),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    on_clip.params[0].value = EffectValue::Float(Property {
        animation: Animation::Keyframed(vec![key(0, 0.0), key(1, 100.0)]),
        extra: serde_json::Map::new(),
    });
    let show = kit::show(&mut kit, Some(on_layer), Some(on_clip));
    let doc = Arc::new(kit.doc.clone());
    let owner = doc.comp(show).unwrap();
    let title = Target::Text { layer: kit.title };

    let copy = kit::layer_named(owner, "Copy");
    let made = placed::for_layer(&doc, owner, copy, 0.0, 0.0, None).unwrap();
    let inside = made.comp(kit.tpl).unwrap();
    assert_eq!(
        rows::read(&made, inside, &opacity(&kit)),
        Some(Value::Number(25.0))
    );
    assert_eq!(
        rows::read(&made, inside, &title),
        Some(Value::Text("Copy one".into()))
    );
    let project = doc.comp(kit.tpl).unwrap();
    assert_eq!(
        rows::read(&doc, project, &opacity(&kit)),
        Some(Value::Number(100.0))
    );
    assert_eq!(
        rows::read(&doc, project, &title),
        Some(Value::Text("Hello".into()))
    );
    let again = placed::for_layer(&doc, owner, copy, 0.0, 0.0, None).unwrap();
    assert!(
        Arc::ptr_eq(&made, &again),
        "a still copy makes one document"
    );

    // A clip's values run on the clip's own clock, from where it is placed.
    let cut = kit::layer_named(owner, "Cut");
    let LayerKind::Sequence { clips } = &cut.kind else {
        panic!("the cut is a Sequence layer");
    };
    let made = placed::for_clip(&doc, owner, cut, &clips[0], 2.5, 2.5).unwrap();
    let Some(Value::Number(half)) = rows::read(&made, made.comp(kit.tpl).unwrap(), &opacity(&kit))
    else {
        panic!("the clip's copy has an opacity");
    };
    assert!((half - 50.0).abs() < 1e-9, "{half}");

    // Nothing to write is the project itself.
    let other = kit::layer_named(owner, "Other");
    assert!(placed::for_layer(&doc, owner, other, 0.0, 0.0, None).is_none());
    assert!(placed::document(&doc, kit.tpl, None, 0.0, None, None).is_none());
    let bare = values::instance_for(kit.tpl, &template);
    assert!(placed::document(&doc, kit.tpl, Some(&bare), 0.0, None, None).is_none());
    let elsewhere = values::instance_for(kit.logo_comp, &template);
    assert!(placed::document(&doc, kit.tpl, Some(&elsewhere), 0.0, None, None).is_none());
    assert!(placed::document(&doc, kit.logo_comp, Some(&elsewhere), 0.0, None, None).is_none());
    assert!(placed::with(&doc, kit.tpl, &[]).is_none());

    // A copy's own picture is fitted into the frame of the one it replaces.
    let mut pictured = values::instance_for(kit.tpl, &template);
    let badge = Picture {
        item: kit.badge,
        size: (200, 200),
        fit: Fit::Fit,
    };
    values::set_value(&mut pictured, c("logo"), &Value::Picture(badge));
    let made = placed::document(&doc, kit.tpl, Some(&pictured), 0.0, None, None).unwrap();
    let slot = kit::layer_named(made.comp(kit.tpl).unwrap(), "Logo");
    let LayerKind::Precomp { comp: fitted } = slot.kind else {
        panic!("the logo shows a composition");
    };
    assert_ne!(fitted, kit.logo_comp);
    let inner = &made.comp(fitted).unwrap().layers[0];
    assert_eq!(inner.kind, LayerKind::Precomp { comp: kit.badge });
    assert_eq!(inner.transform.scale_x.value_at(0.0), 25.0);
    let project = kit::layer_named(doc.comp(kit.tpl).unwrap(), "Logo");
    assert_eq!(
        project.kind,
        LayerKind::Precomp {
            comp: kit.logo_comp
        }
    );
    assert!(doc.comp(fitted).is_none());
}

#[test]
fn a_copys_clock_keeps_the_intro_and_outro_at_their_speed() {
    let timing = Timing {
        intro: 1.0,
        outro: 4.0,
        duration: 5.0,
    };
    let keys = |map: Property| match map.animation {
        Animation::Keyframed(keys) => keys
            .iter()
            .map(|k| (k.time.to_f64(), k.value))
            .collect::<Vec<_>>(),
        other => panic!("a map is keyframed, not {other:?}"),
    };
    // Ten seconds from two: the intro's second, a stretched middle, the
    // outro's second.
    let map = copy_map(timing, Ends::BOTH, 2.0, 10.0).unwrap();
    assert_eq!(
        keys(map),
        [(2.0, 0.0), (3.0, 1.0), (11.0, 4.0), (12.0, 5.0)]
    );
    // Too short for both ends: heard straight through.
    let map = copy_map(timing, Ends::BOTH, 0.0, 1.5).unwrap();
    assert_eq!(keys(map), [(0.0, 0.0), (1.5, 5.0)]);
    // Neither end played: the middle alone, over the whole copy.
    let neither = Ends {
        intro: false,
        outro: false,
    };
    let map = copy_map(timing, neither, 0.0, 10.0).unwrap();
    assert_eq!(keys(map), [(0.0, 1.0), (10.0, 4.0)]);
    assert!(copy_map(timing, Ends::BOTH, 0.0, 0.0).is_none());
    assert!(copy_map(timing, Ends::BOTH, 0.0, f64::NAN).is_none());
}

#[test]
fn an_expression_reads_a_copys_controls_by_their_names() {
    let (mut kit, template) = kit::kit_with_template();
    let mut own = values::instance_for(kit.tpl, &template);
    values::set_value(
        &mut own,
        control(&template, "opacity"),
        &Value::Number(25.0),
    );
    let show = kit::show(&mut kit, Some(own), None);
    let doc = Arc::new(kit.doc.clone());
    let copy = kit::layer_named(doc.comp(show).unwrap(), "Copy").id;
    let context = kit::context(&doc, show, copy, 0.0);
    let run =
        |source: &str| evaluate_value_in(Language::JavaScript, source, Some(context.clone()), &[]);
    let number = |source: &str| {
        run(source).map(|v| match v {
            ExprValue::Number(n) => n,
            other => panic!("{source} answered {other:?}"),
        })
    };
    assert_eq!(number("thisLayer.essentialProperty('Opacity')"), Ok(25.0));
    assert_eq!(
        number("thisComp.layer('Copy').essentialProperty('spin')"),
        Ok(0.0)
    );
    assert_eq!(
        number("thisLayer.masterProperty('Style')"),
        Ok(1.0),
        "a menu counts from one"
    );
    assert_eq!(
        number("thisLayer.essentialProperty('Title').length"),
        Ok(5.0)
    );
    assert_eq!(
        number("thisLayer.essentialProperty(' Position ')[1]"),
        Ok(540.0)
    );
    assert_eq!(
        number("thisLayer.essentialProperty('Glow on') ? 7 : 3"),
        Ok(7.0)
    );
    assert_eq!(number("thisLayer.essentialProperty('Tint')[0]"), Ok(1.0));
    assert_eq!(
        number("thisLayer.essentialProperty('Font').length"),
        Ok(5.0)
    );
    let fails = |source: &str, words: &str| {
        let said = run(source).unwrap_err();
        assert!(said.contains(words), "{source}: {said}");
    };
    fails("thisLayer.essentialProperty('Logo')", "holds nothing");
    fails("thisLayer.essentialProperty('Nope')", "no control called");
    fails(
        "thisComp.layer('Other').essentialProperty('Title')",
        "not a copy",
    );
}

#[test]
fn template_data_is_linked_and_unlinked_through_its_op() {
    let (mut kit, _) = kit::kit_with_template();
    let data = lumit_core::template::feed::Data {
        file: "rows.csv".into(),
        ..Default::default()
    };
    assert_eq!(kit.comp().template_data(), None);
    let link = Op::SetCompTemplateData {
        comp: kit.tpl,
        data: Some(Box::new(data.clone())),
    };
    let undo = ops::apply(&mut kit.doc, &link).unwrap();
    assert_eq!(kit.comp().template_data(), Some(data));
    assert!(matches!(undo, Op::SetCompTemplateData { data: None, .. }));
    // The data lives beside the template, which reads as it did.
    assert_eq!(kit.template().controls.len(), 10);
    ops::apply(&mut kit.doc, &undo).unwrap();
    assert_eq!(kit.comp().template_data(), None);
    let nowhere = Op::SetCompTemplateData {
        comp: Uuid::now_v7(),
        data: None,
    };
    assert!(ops::apply(&mut kit.doc, &nowhere).is_err());
}

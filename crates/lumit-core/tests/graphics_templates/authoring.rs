//! Making a template: a control from each kind of row, the template set and
//! undone through its op, a write to the template's rows as the ops a hand
//! would make, a menu's looks recorded, and every kind of row read and
//! written.

use lumit_core::edits::base_layer;
use lumit_core::fx::params::Unit;
use lumit_core::model::{Folder, LayerKind, ProjectItem, TransformGroup, TransformProp};
use lumit_core::ops::{self, Op};
use lumit_core::template::time::Timing;
use lumit_core::template::{
    self, author, rows, ControlKind, Fit, MaskRow, Picture, Setting, ShapeRow, Target, TextRow,
    Value,
};
use uuid::Uuid;

use crate::kit::{self, secs, Kit};

fn opacity(kit: &Kit) -> Target {
    Target::Transform {
        layer: kit.plate,
        props: vec![TransformProp::Opacity],
    }
}

fn param(kit: &Kit, effect: Uuid, param: &str) -> Target {
    Target::Param {
        layer: kit.plate,
        effect,
        param: param.into(),
    }
}

fn text_row(kit: &Kit, value: TextRow) -> Target {
    Target::TextRow {
        layer: kit.title,
        value,
    }
}

fn shape_row(kit: &Kit, value: ShapeRow) -> Target {
    Target::Shape {
        layer: kit.bar,
        item: kit.item,
        value,
    }
}

fn mask_row(kit: &Kit, value: MaskRow) -> Target {
    Target::Mask {
        layer: kit.bar,
        mask: kit.mask,
        value,
    }
}

#[test]
fn a_control_takes_its_kind_range_and_options_from_the_row_it_names() {
    use ControlKind as K;
    let kit = kit::kit();
    let made = |target: Target| author::control_for(&kit.doc, kit.comp(), target, "Row");
    let position = Target::Transform {
        layer: kit.plate,
        props: vec![TransformProp::PositionX, TransformProp::PositionY],
    };
    let cases = vec![
        (opacity(&kit), Some(K::Number)),
        (
            Target::Transform {
                layer: kit.plate,
                props: vec![TransformProp::Rotation],
            },
            Some(K::Angle),
        ),
        (position.clone(), Some(K::Point)),
        (param(&kit, kit.fill, "colour"), Some(K::Colour)),
        (param(&kit, kit.blur, "radius"), Some(K::Number)),
        (
            param(&kit, kit.blur, "repeat_edge_pixels"),
            Some(K::Checkbox),
        ),
        (param(&kit, kit.safe, "standard"), Some(K::Menu)),
        (param(&kit, kit.fill, "no_such_row"), None),
        (
            Target::ParamPoint {
                layer: kit.plate,
                effect: kit.point,
                x: "point_x".into(),
                y: "point_y".into(),
            },
            Some(K::Point),
        ),
        (
            Target::EffectOn {
                layer: kit.plate,
                effect: kit.fill,
            },
            Some(K::Checkbox),
        ),
        (Target::Text { layer: kit.title }, Some(K::Text)),
        (Target::Font { layer: kit.title }, Some(K::Font)),
        (text_row(&kit, TextRow::Size), Some(K::Number)),
        (text_row(&kit, TextRow::Fill), Some(K::Colour)),
        (text_row(&kit, TextRow::Stroke), Some(K::Colour)),
        (text_row(&kit, TextRow::StrokeWidth), Some(K::Number)),
        (text_row(&kit, TextRow::Tracking), Some(K::Number)),
        (text_row(&kit, TextRow::Background), Some(K::Colour)),
        (shape_row(&kit, ShapeRow::Fill), Some(K::Colour)),
        // The rectangle has no outline, so there is no row to set.
        (shape_row(&kit, ShapeRow::Stroke), None),
        (shape_row(&kit, ShapeRow::StrokeWidth), Some(K::Number)),
        (shape_row(&kit, ShapeRow::Opacity), Some(K::Number)),
        (shape_row(&kit, ShapeRow::Width), Some(K::Number)),
        (shape_row(&kit, ShapeRow::Height), Some(K::Number)),
        (shape_row(&kit, ShapeRow::Roundness), Some(K::Number)),
        (shape_row(&kit, ShapeRow::Rotation), Some(K::Angle)),
        (mask_row(&kit, MaskRow::Opacity), Some(K::Number)),
        (mask_row(&kit, MaskRow::Feather), Some(K::Number)),
        (mask_row(&kit, MaskRow::Expansion), Some(K::Number)),
        (Target::Visible { layer: kit.plate }, Some(K::Checkbox)),
        (Target::Volume { layer: kit.plate }, Some(K::Number)),
        (
            Target::Source {
                layer: kit.logo,
                frame: (100, 50),
            },
            Some(K::Footage),
        ),
        // Rows a layer of that kind does not have, or a layer that has gone.
        (Target::Text { layer: kit.plate }, None),
        (
            Target::Copy {
                layer: kit.plate,
                control: "title".into(),
            },
            None,
        ),
        (
            Target::Source {
                layer: kit.title,
                frame: (1, 1),
            },
            None,
        ),
        (
            Target::Visible {
                layer: Uuid::now_v7(),
            },
            None,
        ),
    ];
    for (target, kind) in cases {
        assert_eq!(made(target.clone()).map(|c| c.kind), kind, "{target:?}");
    }

    let opacity = made(opacity(&kit)).unwrap();
    assert_eq!(
        (opacity.unit, opacity.slider),
        (Unit::Percent, Some((0.0, 100.0)))
    );
    let point = made(position).unwrap();
    assert_eq!((point.axes, point.unit), (2, Unit::Px));
    assert_eq!(point.slider, Some((0.0, 1920.0)));
    let radius = made(param(&kit, kit.blur, "radius")).unwrap();
    assert_eq!(radius.slider, Some((0.0, 500.0)));
    assert_eq!(radius.range, Some((0.0, 2000.0)), "a hard limit is a wall");
    let menu = made(param(&kit, kit.safe, "standard")).unwrap();
    assert_eq!(menu.options, ["NTSC", "PAL"]);
    assert_eq!(
        made(text_row(&kit, TextRow::Size)).unwrap().range,
        Some((1.0, 2000.0))
    );
    let width = made(shape_row(&kit, ShapeRow::Width)).unwrap();
    assert_eq!((width.unit, width.slider), (Unit::Px, Some((0.0, 1920.0))));
    let volume = made(Target::Volume { layer: kit.plate }).unwrap();
    assert_eq!(volume.slider, Some((-48.0, 12.0)));
    let expansion = made(mask_row(&kit, MaskRow::Expansion)).unwrap();
    assert_eq!(expansion.slider, Some((-200.0, 200.0)));
    assert_eq!(
        made(Target::Text { layer: kit.title }).unwrap().label,
        "Row"
    );
}

#[test]
fn a_template_is_set_and_undone_through_its_op() {
    let mut kit = kit::kit();
    let template = kit::authored(&kit);
    let ids: Vec<&str> = template.controls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "title", "opacity", "position", "tint", "glow_on", "style", "logo", "spin", "font",
            "radius"
        ]
    );
    // An id is never one a control or a copy's further rows already use.
    assert_eq!(template.free_id("Title"), "title_2");
    assert_eq!(template.free_id("Title size"), "title_size_2");
    assert_eq!(template.free_id("Play intro"), "play_intro_2");
    assert_eq!(template.free_id(" -- "), "control");
    assert_eq!(template.free_id("Big -- Number"), "big_number");
    assert_eq!(template.groups(), ["Words", "Look"]);

    assert!(!kit.comp().is_template());
    let undo = kit::install(&mut kit, &template);
    assert!(matches!(undo, Op::SetCompTemplate { template: None, .. }));
    assert!(kit.comp().is_template());
    assert_eq!(kit.template(), template);
    assert_eq!(kit::control(&template, "style").options, ["NTSC", "PAL"]);
    assert!(template.control("nope").is_none());
    assert_eq!(
        kit.comp().template_timing(),
        Some(Timing {
            intro: 1.0,
            outro: 4.0,
            duration: 5.0
        })
    );

    // The library's own marks are not part of the design; what it places is.
    let stamp = template::design_stamp(&kit.doc, kit.tpl).unwrap();
    let mut saved = template.clone();
    saved.library = Some(Uuid::now_v7());
    saved.version = 3;
    saved.saved = Some(stamp.clone());
    let mut marked = kit.doc.clone();
    let op = Op::SetCompTemplate {
        comp: kit.tpl,
        template: Some(Box::new(saved)),
    };
    ops::apply(&mut marked, &op).unwrap();
    assert_eq!(
        template::design_stamp(&marked, kit.tpl),
        Some(stamp.clone())
    );
    marked.comp_mut(kit.logo_comp).unwrap().name = "logo, larger".into();
    assert_ne!(template::design_stamp(&marked, kit.tpl), Some(stamp));

    let redo = ops::apply(&mut kit.doc, &undo).unwrap();
    assert!(!kit.comp().is_template());
    assert!(kit.comp().template().is_none());
    assert!(matches!(
        redo,
        Op::SetCompTemplate {
            template: Some(_),
            ..
        }
    ));
    let nowhere = Op::SetCompTemplate {
        comp: Uuid::now_v7(),
        template: None,
    };
    assert!(ops::apply(&mut kit.doc, &nowhere).is_err());

    // Only an intro: the outro is the template's end.
    let mut intro_only = template.clone();
    intro_only.outro = None;
    kit::install(&mut kit, &intro_only);
    let timing = kit.comp().template_timing().unwrap();
    assert_eq!((timing.intro, timing.outro), (1.0, 5.0));
    intro_only.intro = None;
    kit::install(&mut kit, &intro_only);
    assert_eq!(kit.comp().template_timing(), None);

    for fit in [Fit::Fill, Fit::Fit, Fit::Stretch] {
        assert_eq!(Fit::from_choice(fit.to_choice()), fit);
    }
    assert_eq!(Fit::from_choice(9), Fit::Fill);

    // The library's folder, found by name, and a name nothing has yet.
    assert_eq!(author::library_folder(&kit.doc, "Templates"), None);
    let folder = Uuid::now_v7();
    kit.doc.items.push(ProjectItem::Folder(Folder {
        id: folder,
        name: "Templates".into(),
        children: Vec::new(),
        extra: serde_json::Map::new(),
    }));
    assert_eq!(author::library_folder(&kit.doc, "Templates"), Some(folder));
    assert_eq!(author::free_name("Card", &["Other"]), "Card");
    assert_eq!(
        author::free_name("Card", &["Card", "Card 2", "Card 4"]),
        "Card 3"
    );
}

#[test]
fn a_write_to_the_template_is_the_ops_a_hand_would_make() {
    let (mut kit, _) = kit::kit_with_template();
    let position = Target::Transform {
        layer: kit.plate,
        props: vec![TransformProp::PositionX, TransformProp::PositionY],
    };
    let settings = vec![
        Setting {
            target: opacity(&kit),
            value: Value::Number(40.0),
        },
        Setting {
            target: position,
            value: Value::Point(vec![100.0, 200.0]),
        },
        Setting {
            target: Target::Text { layer: kit.title },
            value: Value::Text("Goodbye".into()),
        },
        Setting {
            target: shape_row(&kit, ShapeRow::Width),
            value: Value::Number(250.0),
        },
        Setting {
            target: mask_row(&kit, MaskRow::Feather),
            value: Value::Number(12.0),
        },
        Setting {
            target: Target::Visible { layer: kit.bar },
            value: Value::Tick(false),
        },
        Setting {
            target: Target::Volume { layer: kit.plate },
            value: Value::Number(-6.0),
        },
        Setting {
            target: param(&kit, kit.fill, "colour"),
            value: Value::Colour([0.5, 0.25, 0.0, 1.0]),
        },
    ];
    let logo = Setting {
        target: Target::Source {
            layer: kit.logo,
            frame: (100, 50),
        },
        value: Value::Picture(Picture {
            item: kit.badge,
            size: (200, 200),
            fit: Fit::Fill,
        }),
    };
    let all: Vec<Setting> = settings.iter().cloned().chain([logo.clone()]).collect();
    let made = author::ops(&kit.doc, kit.tpl, &all);
    let names: Vec<&str> = made
        .iter()
        .map(|op| match op {
            Op::AddItem { .. } => "item",
            Op::SetTextDocument { .. } => "text",
            Op::SetLayerMasks { .. } => "masks",
            Op::SetShapeContents { .. } => "shape",
            Op::SetLayerVisible { .. } => "visible",
            Op::SetTransformProperty { .. } => "transform",
            Op::SetLayerEffects { .. } => "effects",
            Op::SetLayerVolume { .. } => "volume",
            Op::SetLayerKind { .. } => "kind",
            _ => "other",
        })
        .collect();
    assert_eq!(
        names,
        [
            "item",
            "text",
            "masks",
            "shape",
            "visible",
            "transform",
            "transform",
            "transform",
            "effects",
            "volume",
            "kind"
        ],
        "one op per part of a layer that changed, and the picture's composition first"
    );
    assert_eq!(
        rows::read(&kit.doc, kit.comp(), &opacity(&kit)),
        Some(Value::Number(100.0)),
        "nothing commits until the ops are applied"
    );
    for op in &made {
        ops::apply(&mut kit.doc, op).unwrap();
    }
    for setting in &settings {
        let read = rows::read(&kit.doc, kit.comp(), &setting.target);
        assert_eq!(read, Some(setting.value.clone()), "{:?}", setting.target);
    }
    // The logo now shows a composition the size of the old picture, holding
    // the badge scaled to cover it.
    let Some(Value::Picture(shown)) = rows::read(&kit.doc, kit.comp(), &logo.target) else {
        panic!("the logo still shows a picture");
    };
    assert_ne!(shown.item, kit.logo_comp);
    let fitted = kit.doc.comp(shown.item).unwrap();
    assert_eq!((fitted.width, fitted.height), (100, 50));
    assert_eq!(
        fitted.layers[0].kind,
        LayerKind::Precomp { comp: kit.badge }
    );
    assert_eq!(fitted.layers[0].transform.scale_x.value_at(0.0), 50.0);

    assert!(author::ops(&kit.doc, kit.tpl, &[]).is_empty());
    assert!(author::ops(&kit.doc, Uuid::now_v7(), &settings).is_empty());
}

#[test]
fn a_picture_is_filled_fitted_or_stretched_into_the_frame_it_replaces() {
    let (kit, _) = kit::kit_with_template();
    let source = Target::Source {
        layer: kit.logo,
        frame: (100, 50),
    };
    let picture =
        |item: Uuid, size: (u32, u32), fit: Fit| Value::Picture(Picture { item, size, fit });
    for (fit, sx, sy) in [
        (Fit::Fill, 50.0, 50.0),
        (Fit::Fit, 25.0, 25.0),
        (Fit::Stretch, 50.0, 25.0),
    ] {
        let mut doc = kit.doc.clone();
        assert!(rows::write(
            &mut doc,
            kit.tpl,
            &source,
            &picture(kit.badge, (200, 200), fit)
        ));
        let slot = kit::layer_named(doc.comp(kit.tpl).unwrap(), "Logo");
        let LayerKind::Precomp { comp: made } = slot.kind else {
            panic!("the slot shows a composition");
        };
        let inner = &doc.comp(made).unwrap().layers[0];
        let scale = (
            inner.transform.scale_x.value_at(0.0),
            inner.transform.scale_y.value_at(0.0),
        );
        assert_eq!(scale, (sx, sy), "{fit:?}");
    }
    let mut doc = kit.doc.clone();
    let refused = [
        (source.clone(), picture(kit.tpl, (200, 200), Fit::Fill)),
        (source.clone(), picture(kit.badge, (0, 200), Fit::Fill)),
        (
            source.clone(),
            picture(Uuid::now_v7(), (200, 200), Fit::Fill),
        ),
        (source.clone(), Value::Text("logo.png".into())),
        (
            Target::Source {
                layer: kit.title,
                frame: (100, 50),
            },
            picture(kit.badge, (200, 200), Fit::Fill),
        ),
    ];
    for (target, value) in refused {
        assert!(
            !rows::write(&mut doc, kit.tpl, &target, &value),
            "{value:?}"
        );
    }
    assert_eq!(doc, kit.doc, "a refused picture changes nothing");
}

#[test]
fn a_menu_option_records_the_rows_a_hand_changed() {
    let (kit, template) = kit::kit_with_template();
    let title = Target::Text { layer: kit.title };
    let style = param(&kit, kit.safe, "standard");
    let spin = Target::Transform {
        layer: kit.plate,
        props: vec![TransformProp::Rotation],
    };
    let was = kit.comp().clone();
    let mut doc = kit.doc.clone();
    rows::write(&mut doc, kit.tpl, &opacity(&kit), &Value::Number(30.0));
    rows::write(&mut doc, kit.tpl, &title, &Value::Text("Night".into()));
    rows::write(&mut doc, kit.tpl, &style, &Value::Choice(1));
    let now = doc.comp(kit.tpl).unwrap().clone();

    let changed: Vec<Target> = rows::changed(&kit.doc, &was, &now)
        .into_iter()
        .map(|s| s.target)
        .collect();
    assert_eq!(changed.len(), 3, "{changed:?}");
    assert!(changed.contains(&opacity(&kit)) && changed.contains(&title));

    let recorded = author::recorded(&kit.doc, &template, "style", 1, &was, &now).unwrap();
    let looks = &kit::control(&recorded, "style").looks;
    assert_eq!(looks.len(), 2);
    let held = |looks: &[Vec<Setting>], n: usize, target: &Target| {
        looks[n]
            .iter()
            .find(|s| s.target == *target)
            .map(|s| s.value.clone())
    };
    assert_eq!(held(looks, 1, &opacity(&kit)), Some(Value::Number(30.0)));
    assert_eq!(held(looks, 1, &title), Some(Value::Text("Night".into())));
    // The other option puts back what the rows held before.
    assert_eq!(held(looks, 0, &opacity(&kit)), Some(Value::Number(100.0)));
    assert_eq!(held(looks, 0, &title), Some(Value::Text("Hello".into())));
    assert!(
        looks.iter().flatten().all(|s| s.target != style),
        "the menu's own row is choosing, not a look"
    );

    // Recording the first option over it keeps the second's own rows.
    let mut doc = kit.doc.clone();
    rows::write(&mut doc, kit.tpl, &spin, &Value::Number(45.0));
    let turned = doc.comp(kit.tpl).unwrap().clone();
    let again = author::recorded(&kit.doc, &recorded, "style", 0, &was, &turned).unwrap();
    let looks = &kit::control(&again, "style").looks;
    assert_eq!(held(looks, 0, &spin), Some(Value::Number(45.0)));
    assert_eq!(held(looks, 1, &spin), Some(Value::Number(0.0)));
    assert_eq!(held(looks, 1, &opacity(&kit)), Some(Value::Number(30.0)));

    assert!(author::recorded(&kit.doc, &template, "opacity", 0, &was, &now).is_none());
    assert!(author::recorded(&kit.doc, &template, "nope", 0, &was, &now).is_none());
}

#[test]
fn every_kind_of_row_reads_back_what_was_written() {
    let (mut kit, _) = kit::kit_with_template();
    let colour = |r: f64| Value::Colour([r, 0.25, 0.125, 1.0]);
    let written = vec![
        (
            Target::Transform {
                layer: kit.plate,
                props: vec![TransformProp::Rotation],
            },
            Value::Number(45.0),
        ),
        (
            Target::Transform {
                layer: kit.plate,
                props: vec![TransformProp::AnchorX, TransformProp::AnchorY],
            },
            Value::Point(vec![5.0, 6.0]),
        ),
        (param(&kit, kit.blur, "radius"), Value::Number(12.0)),
        (
            param(&kit, kit.blur, "repeat_edge_pixels"),
            Value::Tick(false),
        ),
        (param(&kit, kit.safe, "standard"), Value::Choice(1)),
        (param(&kit, kit.fill, "colour"), colour(0.5)),
        (
            Target::ParamPoint {
                layer: kit.plate,
                effect: kit.point,
                x: "point_x".into(),
                y: "point_y".into(),
            },
            Value::Point(vec![10.0, 20.0]),
        ),
        (
            Target::EffectOn {
                layer: kit.plate,
                effect: kit.safe,
            },
            Value::Tick(false),
        ),
        (
            Target::Text { layer: kit.title },
            Value::Text("Words".into()),
        ),
        (
            Target::Font { layer: kit.title },
            Value::Font {
                family: "Mono".into(),
                face: "Bold".into(),
            },
        ),
        (text_row(&kit, TextRow::Size), Value::Number(96.0)),
        (text_row(&kit, TextRow::Fill), colour(0.5)),
        (text_row(&kit, TextRow::Stroke), colour(0.75)),
        (text_row(&kit, TextRow::StrokeWidth), Value::Number(3.0)),
        (text_row(&kit, TextRow::Tracking), Value::Number(20.0)),
        (text_row(&kit, TextRow::Background), colour(1.0)),
        (shape_row(&kit, ShapeRow::Fill), colour(0.5)),
        (shape_row(&kit, ShapeRow::Stroke), colour(0.25)),
        (shape_row(&kit, ShapeRow::StrokeWidth), Value::Number(4.0)),
        (shape_row(&kit, ShapeRow::Opacity), Value::Number(50.0)),
        (shape_row(&kit, ShapeRow::Width), Value::Number(300.0)),
        (shape_row(&kit, ShapeRow::Height), Value::Number(60.0)),
        (shape_row(&kit, ShapeRow::Roundness), Value::Number(8.0)),
        (shape_row(&kit, ShapeRow::Rotation), Value::Number(15.0)),
        (mask_row(&kit, MaskRow::Opacity), Value::Number(70.0)),
        (mask_row(&kit, MaskRow::Feather), Value::Number(5.0)),
        (mask_row(&kit, MaskRow::Expansion), Value::Number(-3.0)),
        (Target::Visible { layer: kit.plate }, Value::Tick(false)),
        (Target::Volume { layer: kit.plate }, Value::Number(-12.0)),
    ];
    let before = kit.comp().clone();
    for (target, value) in &written {
        assert!(
            rows::write(&mut kit.doc, kit.tpl, target, value),
            "{target:?}"
        );
        assert_eq!(
            rows::read(&kit.doc, kit.comp(), target).as_ref(),
            Some(value)
        );
    }
    // Every row written is a change found. An effect's point is found as its
    // two rows, as the Timeline shows them.
    let changed = rows::changed(&kit.doc, &before, kit.comp());
    for (target, value) in &written {
        if matches!(target, Target::ParamPoint { .. }) {
            continue;
        }
        let found = changed.iter().find(|s| s.target == *target);
        assert_eq!(found.map(|s| &s.value), Some(value), "{target:?}");
    }

    let wrong = vec![
        (opacity(&kit), Value::Text("no".into())),
        (
            Target::Transform {
                layer: kit.plate,
                props: vec![TransformProp::ScaleX, TransformProp::ScaleY],
            },
            Value::Colour([1.0; 4]),
        ),
        (param(&kit, kit.blur, "radius"), Value::Tick(true)),
        (param(&kit, kit.blur, "no_such_row"), Value::Number(1.0)),
        (Target::Text { layer: kit.plate }, Value::Text("no".into())),
        (
            Target::Font { layer: kit.plate },
            Value::Font {
                family: "Mono".into(),
                face: String::new(),
            },
        ),
        (text_row(&kit, TextRow::Size), Value::Colour([1.0; 4])),
        (
            Target::Shape {
                layer: kit.bar,
                item: Uuid::now_v7(),
                value: ShapeRow::Width,
            },
            Value::Number(1.0),
        ),
        (shape_row(&kit, ShapeRow::Width), Value::Tick(true)),
        (
            Target::Mask {
                layer: kit.bar,
                mask: Uuid::now_v7(),
                value: MaskRow::Opacity,
            },
            Value::Number(1.0),
        ),
        (Target::Visible { layer: kit.plate }, Value::Number(1.0)),
        (
            Target::Volume {
                layer: Uuid::now_v7(),
            },
            Value::Number(1.0),
        ),
        (
            Target::Copy {
                layer: kit.plate,
                control: "title".into(),
            },
            Value::Text("no".into()),
        ),
    ];
    let kept = kit.doc.clone();
    for (target, value) in &wrong {
        assert!(
            !rows::write(&mut kit.doc, kit.tpl, target, value),
            "{target:?}"
        );
    }
    let target = opacity(&kit);
    assert!(!rows::write(
        &mut kit.doc,
        Uuid::now_v7(),
        &target,
        &Value::Number(1.0)
    ));
    assert_eq!(kit.doc, kept, "a write that does not fit changes nothing");

    // What recording compares: every row a layer has, by kind.
    let c = kit.comp();
    let rows_of = |layer| rows::of_layer(&kit.doc, c, kit::layer_named(c, layer));
    let title = rows_of("Title");
    let count = |found: &[(Target, Value)], pick: fn(&Target) -> bool| {
        found.iter().filter(|(t, _)| pick(t)).count()
    };
    assert_eq!(count(&title, |t| matches!(t, Target::TextRow { .. })), 6);
    assert_eq!(
        count(&title, |t| matches!(
            t,
            Target::Text { .. } | Target::Font { .. }
        )),
        2
    );
    let bar = rows_of("Bar");
    assert_eq!(count(&bar, |t| matches!(t, Target::Shape { .. })), 8);
    assert_eq!(count(&bar, |t| matches!(t, Target::Mask { .. })), 3);
    let plate = rows_of("Plate");
    assert_eq!(count(&plate, |t| matches!(t, Target::EffectOn { .. })), 4);
    assert!(plate.contains(&(param(&kit, kit.blur, "radius"), Value::Number(12.0))));
    assert!(plate.contains(&(
        Target::Transform {
            layer: kit.plate,
            props: vec![TransformProp::PositionX, TransformProp::PositionY]
        },
        Value::Point(vec![960.0, 540.0])
    )));
    assert!(plate.contains(&(Target::Volume { layer: kit.plate }, Value::Number(-12.0))));
}

#[test]
fn a_control_of_a_template_placed_inside_another_reaches_that_copy() {
    let (mut kit, _) = kit::kit_with_template();
    let inner = base_layer(
        "Inner".into(),
        LayerKind::Precomp { comp: kit.tpl },
        secs(5),
        TransformGroup::default(),
    );
    let inner_id = inner.id;
    let card = kit::comp("Card", 1920, 1080, 5, vec![inner]);
    let card_id = card.id;
    kit.doc.items.push(ProjectItem::Composition(card));
    let target = Target::Copy {
        layer: inner_id,
        control: "title".into(),
    };

    let c = kit.doc.comp(card_id).unwrap();
    let control = author::control_for(&kit.doc, c, target.clone(), "Headline").unwrap();
    assert_eq!(control.kind, ControlKind::Text);
    assert!(control.font && control.size && control.colour);
    assert_eq!(
        (control.label.as_str(), &control.target),
        ("Headline", &target)
    );
    assert!(
        control.group.is_none(),
        "the inner template's heading stays there"
    );
    let unknown = Target::Copy {
        layer: inner_id,
        control: "nope".into(),
    };
    assert!(author::control_for(&kit.doc, c, unknown.clone(), "No").is_none());
    assert_eq!(
        rows::read(&kit.doc, c, &target),
        Some(Value::Text("Hello".into())),
        "a copy that set nothing reads the inner template"
    );

    let via = author::ops(
        &kit.doc,
        card_id,
        &[Setting {
            target: target.clone(),
            value: Value::Text("Via an op".into()),
        }],
    );
    assert!(matches!(
        via.as_slice(),
        [Op::SetLayerGraphInputs {
            inputs: Some(_),
            ..
        }]
    ));

    assert!(rows::write(
        &mut kit.doc,
        card_id,
        &target,
        &Value::Text("Nested".into())
    ));
    let c = kit.doc.comp(card_id).unwrap();
    assert_eq!(
        rows::read(&kit.doc, c, &target),
        Some(Value::Text("Nested".into()))
    );
    let values = c.layers[0].graph_inputs.as_ref().unwrap();
    assert_eq!(
        lumit_core::fx::effects::template::comp_of(values),
        Some(kit.tpl)
    );
    assert!(!rows::write(
        &mut kit.doc,
        card_id,
        &unknown,
        &Value::Text("x".into())
    ));
    assert!(!rows::write(
        &mut kit.doc,
        card_id,
        &target,
        &Value::Number(1.0)
    ));
}

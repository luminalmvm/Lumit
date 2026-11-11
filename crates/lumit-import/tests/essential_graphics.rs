//! A project saved by After Effects 2026 with Essential Graphics in it, read
//! both ways in: straight from the `.aep`, and from the bundle the script
//! wrote of the same project in the same sitting.
//!
//! "Lower third" has a controller of each kind. "Main" places it twice, and
//! each layer sets some of them, one with keyframes.
//! `tools/ae-bridge/make-essential-fixture.jsx` builds both files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::fx::effects::template::value_of;
use lumit_core::model::{Composition, Document, Layer, LayerKind, ProjectItem};
use lumit_core::template::{ControlKind, Target, Template, Value};
use lumit_import::report::{ImportReport, Reason};

/// The project by each way in that carries Essential Graphics.
fn imported() -> Vec<(&'static str, Document, ImportReport)> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    ["essential-graphics.aep", "essential-graphics.lum-bundle"]
        .into_iter()
        .map(|name| {
            let bundle = lumit_import::open_ae(&fixtures.join(name)).expect("the project opens");
            let (doc, report) = lumit_import::map::map_capture(&bundle.capture);
            (name, doc, report)
        })
        .collect()
}

fn comp<'a>(doc: &'a Document, name: &str) -> &'a Composition {
    doc.items
        .iter()
        .find_map(|item| match item {
            ProjectItem::Composition(c) if c.name == name => Some(c),
            _ => None,
        })
        .expect("the composition is there")
}

fn layer<'a>(comp: &'a Composition, name: &str) -> &'a Layer {
    comp.layers
        .iter()
        .find(|l| l.name == name)
        .expect("the layer is there")
}

/// What a copy holds for the control labelled `label` at its own time `t`.
fn held(template: &Template, copy: &Layer, label: &str, t: f64) -> Option<Value> {
    let control = template
        .controls
        .iter()
        .find(|c| c.label == label)
        .expect("the control is there");
    value_of(copy.graph_inputs.as_ref()?, control, t, None)
}

#[test]
fn controllers_become_the_templates_controls() {
    for (route, doc, report) in imported() {
        let lower = comp(&doc, "Lower third");
        let template = lower.template().expect("a template");
        let kinds: Vec<(&str, ControlKind)> = template
            .controls
            .iter()
            .map(|c| (c.label.as_str(), c.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("Title", ControlKind::Text),
                ("Bar colour", ControlKind::Colour),
                ("Bar opacity", ControlKind::Number),
                ("Blur", ControlKind::Number),
                ("Show plate", ControlKind::Checkbox),
                ("Bar position", ControlKind::Point),
                ("Bar angle", ControlKind::Angle),
                ("Side", ControlKind::Menu),
                ("Title scale", ControlKind::Point),
                ("Spot", ControlKind::Point),
                ("Tint", ControlKind::Colour),
                ("Turn", ControlKind::Angle),
                ("Amount", ControlKind::Number),
                ("Plate colour", ControlKind::Colour),
                ("Logo", ControlKind::Footage),
            ],
            "{route}"
        );

        // Every control names a row that is there to read.
        for control in &template.controls {
            let row = lumit_core::template::rows::read(&doc, lower, &control.target);
            assert!(row.is_some(), "{route}: {} reads nothing", control.label);
        }
        let side = template
            .controls
            .iter()
            .find(|c| c.label == "Side")
            .unwrap();
        assert_eq!(side.options, ["Left", "Centre", "Right"], "{route}");
        // The menu itself arrived as a set of Custom controls, on its second
        // option, counting from 0.
        let Target::Param { effect, param, .. } = &side.target else {
            panic!("{route}: a menu is an effect's row");
        };
        let menu = layer(lower, "Plate")
            .effects
            .iter()
            .find(|e| e.id == *effect)
            .unwrap();
        assert_eq!(menu.effect.match_name, "custom_controls", "{route}");
        assert_eq!(
            menu.param(param),
            Some(&lumit_core::model::EffectValue::Choice(1)),
            "{route}"
        );

        // A rectangle's size is one row there and two here, and says so.
        let lost: Vec<&str> = report
            .rows
            .iter()
            .filter_map(|row| match &row.reason {
                Reason::ControlNotCarried { control, .. } => Some(control.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lost, ["Bar size"], "{route}");
    }
}

#[test]
fn each_copy_keeps_what_its_layer_set() {
    for (route, doc, report) in imported() {
        let template = comp(&doc, "Lower third").template().expect("a template");
        let main = comp(&doc, "Main");
        let (a, b) = (layer(main, "Copy A"), layer(main, "Copy B"));
        let text = |words: &str| Some(Value::Text(words.to_string()));

        assert_eq!(
            held(&template, a, "Title", 0.0),
            text("Ada Lovelace"),
            "{route}"
        );
        assert_eq!(
            held(&template, a, "Side", 0.0),
            Some(Value::Choice(2)),
            "{route}"
        );
        assert!(matches!(
            held(&template, a, "Bar colour", 0.0),
            Some(Value::Colour(_))
        ));
        // Keyed a second apart, on the copy's own clock.
        for (t, opacity) in [(0.0, 0.0), (0.5, 50.0), (1.0, 100.0)] {
            assert_eq!(
                held(&template, a, "Bar opacity", t),
                Some(Value::Number(opacity)),
                "{route} at {t}"
            );
        }
        // What copy A left alone follows the template.
        assert_eq!(held(&template, a, "Bar angle", 0.0), None, "{route}");

        assert_eq!(
            held(&template, b, "Title", 0.0),
            text("Grace Hopper"),
            "{route}"
        );
        assert_eq!(
            held(&template, b, "Show plate", 0.0),
            Some(Value::Tick(false))
        );
        assert_eq!(
            held(&template, b, "Bar angle", 0.0),
            Some(Value::Number(45.0))
        );
        assert_eq!(held(&template, b, "Amount", 0.0), Some(Value::Number(60.0)));
        assert_eq!(
            held(&template, b, "Bar position", 0.0),
            Some(Value::Point(vec![100.0, 300.0])),
            "{route}"
        );
        assert_eq!(
            held(&template, b, "Title scale", 0.0),
            Some(Value::Point(vec![50.0, 50.0])),
            "{route}"
        );
        // An effect's point, which the project file keeps as a share of the
        // layer it is on.
        assert_eq!(
            held(&template, b, "Spot", 0.0),
            Some(Value::Point(vec![10.0, 20.0])),
            "{route}"
        );
        assert_eq!(held(&template, b, "Blur", 0.0), Some(Value::Number(12.0)));
        for label in ["Tint", "Plate colour"] {
            assert!(
                matches!(held(&template, b, label, 0.0), Some(Value::Colour(_))),
                "{route}: {label}"
            );
        }
        // The picture copy B put in is the composition After Effects made
        // for it.
        let Some(Value::Picture(picture)) = held(&template, b, "Logo", 0.0) else {
            panic!("{route}: copy B's picture");
        };
        assert!(doc.comp(picture.item).is_some(), "{route}");
        assert_eq!(held(&template, b, "Side", 0.0), None, "{route}");

        assert!(
            !report
                .rows
                .iter()
                .any(|row| matches!(row.reason, Reason::ControlValueNotCarried { .. })),
            "{route}"
        );
    }
}

#[test]
fn an_expression_reads_a_copys_control() {
    for (route, doc, _) in imported() {
        let main = comp(&doc, "Main");
        let a = layer(main, "Copy A");
        assert!(matches!(a.kind, LayerKind::Precomp { .. }));
        let context = Arc::new(ExpressionContext {
            document: Arc::new(doc.clone()),
            comp: Some(main.id),
            layer: Some(a.id),
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        });
        // `thisLayer.essentialProperty("Amount") * 2`, and copy A follows the
        // template's 25.
        let turned = a.transform.rotation.value_at_with_context(0.0, context);
        assert_eq!(turned, 50.0, "{route}");
    }
}

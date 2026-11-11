//! Data feeding a template: a table read from a file, its columns matched to
//! controls, each row read as a copy's values or its faults, a copy fed from
//! a row, each row's version of the project and its file's name. And the
//! Custom controls effect, whose menu a template's own menus follow.

use std::path::{Path, PathBuf};

use lumit_core::fx::builtins;
use lumit_core::fx::effects::custom_controls::{
    self as custom, Control as Rig, ControlKind as RigKind,
};
use lumit_core::fx::effects::template as values;
use lumit_core::fx::schema::ParamKind;
use lumit_core::model::{EffectValue, LayerKind};
use lumit_core::ops::{self, Op};
use lumit_core::template::data::{self, Format, Table, TableError};
use lumit_core::template::feed::{self, Data, Fault, Feeds};
use lumit_core::template::version::{self, Host};
use lumit_core::template::{rows, Fit, Picture, Target, Template, Value};
use uuid::Uuid;

use crate::kit::{self, control, Kit};

const COLUMNS: [&str; 14] = [
    "Title",
    "Opacity",
    "Tint",
    "Glow on",
    "Style",
    "Position",
    "Logo",
    "Title size",
    "Title font",
    "Title colour",
    "Length",
    "File name",
    "Radius",
    "Notes",
];

/// Two rows: one every cell of which reads, and one most of whose cells do
/// not. Written out as CSV and read back, as a linked file is.
fn data_for(template: &Template) -> Data {
    let cells = |row: [&str; 14]| row.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
    let written = Table {
        columns: cells(COLUMNS),
        rows: vec![
            cells([
                "Ada",
                "50%",
                "#F00",
                "yes",
                "pal",
                "(10, 20)",
                "logo.png",
                "96",
                "Inter, Bold",
                "#0F0",
                "8",
                "ada/out:1",
                "5000",
                "x",
            ]),
            cells([
                "Grace", "lots", "", "maybe", "3", "1 2 3 4", "", "-5", "", "nope", "0", "", "", "",
            ]),
        ],
    };
    let table = data::parse(&data::to_csv(&written), Format::Csv).unwrap();
    assert_eq!(table, written, "what was written reads back the same");
    Data {
        file: "jobs/rows.csv".into(),
        feeds: feed::matched(template, &table.columns),
        table,
        extra: serde_json::Map::new(),
    }
}

/// The footage the project holds: the badge, for the one file it reads.
fn pictures_of(kit: &Kit) -> impl Fn(&Path) -> Option<(Uuid, (u32, u32))> {
    let badge = kit.badge;
    let file = version::cell_file("jobs/rows.csv", "logo.png");
    move |path: &Path| (path == file).then_some((badge, (200, 200)))
}

fn title_of(inst: &lumit_core::model::EffectInstance, template: &Template) -> Option<Value> {
    values::value_of(inst, control(template, "title"), 0.0, None)
}

#[test]
fn columns_feed_controls_and_each_row_reads_as_values_or_faults() {
    let (_, template) = kit::kit_with_template();
    let data = data_for(&template);
    let fed = |id: &str| Feeds::Control(id.into());
    assert_eq!(
        data.feeds,
        [
            fed("title"),
            fed("opacity"),
            fed("tint"),
            fed("glow_on"),
            fed("style"),
            fed("position"),
            fed("logo"),
            Feeds::Size("title".into()),
            Feeds::Font("title".into()),
            Feeds::Colour("title".into()),
            Feeds::Length,
            Feeds::FileName,
            fed("radius"),
            Feeds::Nothing,
        ]
    );

    let first = feed::row_values(&template, &data, 0).unwrap();
    let id = |s: &str| s.to_owned();
    assert_eq!(
        first.values,
        [
            (id("title"), Value::Text("Ada".into())),
            (id("opacity"), Value::Number(50.0)),
            (id("tint"), Value::Colour([1.0, 0.0, 0.0, 1.0])),
            (id("glow_on"), Value::Tick(true)),
            (id("style"), Value::Choice(1)),
            (id("position"), Value::Point(vec![10.0, 20.0])),
            // The radius's hard limit is a wall here as on a copy.
            (id("radius"), Value::Number(2000.0)),
        ]
    );
    assert_eq!(
        first.extras,
        [
            (id("title_size"), Value::Number(96.0)),
            (
                id("title_font"),
                Value::Font {
                    family: "Inter".into(),
                    face: "Bold".into()
                }
            ),
            (id("title_colour"), Value::Colour([0.0, 1.0, 0.0, 1.0])),
        ]
    );
    assert_eq!(first.files, [(id("logo"), id("logo.png"))]);
    assert_eq!(first.length, Some(8.0));
    assert_eq!(first.file_name.as_deref(), Some("ada/out:1"));
    assert!(first.faults.is_empty());

    let second = feed::row_values(&template, &data, 1).unwrap();
    assert_eq!(second.values, [(id("title"), Value::Text("Grace".into()))]);
    let faults: Vec<usize> = second.faults.iter().map(|f: &Fault| f.column).collect();
    assert_eq!(faults, [1, 3, 4, 5, 7, 9, 10]);
    assert!(second.files.is_empty() && second.length.is_none());
    assert!(feed::row_values(&template, &data, 2).is_none());

    // A display colour is kept scene-linear.
    let grey = feed::row_colour("#888").unwrap();
    assert!((grey[0] - 0.246_201).abs() < 1e-5, "{grey:?}");
    assert_eq!(grey[3], 1.0);
    assert_eq!(feed::row_colour("grey"), None);
    // A column named after a text row that the copy may not change feeds
    // nothing.
    let mut plain = template.clone();
    plain.controls[0].size = false;
    assert_eq!(feed::matched(&plain, &[id("Title size")]), [Feeds::Nothing]);
    assert_eq!(
        feed::matched(
            &template,
            &[id("duration"), id("output"), id("Title color")]
        ),
        [
            Feeds::Length,
            Feeds::FileName,
            Feeds::Colour("title".into())
        ]
    );
}

#[test]
fn a_starting_table_holds_the_templates_own_values_and_reads_back() {
    let (kit, template) = kit::kit_with_template();
    let table = feed::starting_table(&kit.doc, kit.comp(), &template);
    assert_eq!(
        table.columns,
        [
            "Title",
            "Title font",
            "Title size",
            "Title colour",
            "Opacity",
            "Position",
            "Tint",
            "Glow on",
            "Style",
            "Logo",
            "Spin",
            "Font",
            "Radius"
        ]
    );
    // Colour cells are built here: a literal would read as a colour of the
    // interface to the design-token lint.
    let white = format!("#{}", "F".repeat(6));
    assert_eq!(
        table.rows,
        [[
            "Hello",
            "Inter, Regular",
            "72",
            white.as_str(),
            "100",
            "960, 540",
            white.as_str(),
            "yes",
            "NTSC",
            "",
            "0",
            "Inter, Regular",
            "30"
        ]]
    );
    let back = data::parse(&data::to_csv(&table), Format::Csv).unwrap();
    assert_eq!(back, table);
    let data = Data {
        file: String::new(),
        feeds: feed::matched(&template, &back.columns),
        table: back,
        extra: serde_json::Map::new(),
    };
    assert!(!data.feeds.contains(&Feeds::Nothing));
    let read = feed::row_values(&template, &data, 0).unwrap();
    assert!(read.faults.is_empty());
    // Every cell reads back as the row it came from holds.
    for (id, value) in &read.values {
        let held = rows::read(&kit.doc, kit.comp(), &control(&template, id).target);
        assert_eq!(held.as_ref(), Some(value), "{id}");
    }
    assert_eq!(read.values.len(), 9, "every control but the logo");
    assert_eq!(read.extras.len(), 3, "the title's font, size and colour");

    // A colour with alpha, a fraction and a font with no face.
    let mut doc = kit.doc.clone();
    let tint = &control(&template, "tint").target;
    rows::write(
        &mut doc,
        kit.tpl,
        tint,
        &Value::Colour([0.5, 0.5, 0.5, 0.5]),
    );
    let opacity = &control(&template, "opacity").target;
    rows::write(&mut doc, kit.tpl, opacity, &Value::Number(37.5));
    let font = Value::Font {
        family: "Inter".into(),
        face: String::new(),
    };
    rows::write(&mut doc, kit.tpl, &Target::Font { layer: kit.title }, &font);
    let table = feed::starting_table(&doc, doc.comp(kit.tpl).unwrap(), &template);
    let cell = |column: &str| {
        let at = table.columns.iter().position(|c| c == column).unwrap();
        table.rows[0][at].clone()
    };
    assert_eq!(cell("Tint"), format!("#{}80", "BC".repeat(3)));
    assert_eq!(cell("Opacity"), "37.5");
    assert_eq!(
        (cell("Font"), cell("Title font")),
        ("Inter".into(), "Inter".into())
    );
}

#[test]
fn a_table_is_read_from_csv_tsv_and_json_or_refused() {
    let table = |columns: &[&str], rows: &[&[&str]]| Table {
        columns: columns.iter().map(|c| (*c).to_owned()).collect(),
        rows: rows
            .iter()
            .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
            .collect(),
    };
    let one = table(&["A", "B"], &[&["1", "2"]]);
    assert_eq!(data::parse("A\tB\n1\t2\n", Format::Tsv), Ok(one.clone()));
    assert_eq!(
        data::parse("A\tB\n1\t2", Format::Csv),
        Ok(one.clone()),
        "a tab file named .csv"
    );
    assert_eq!(data::parse("A,B\r1,2\r", Format::Csv), Ok(one.clone()));
    assert_eq!(
        data::parse("A\n1,2\n", Format::Csv),
        Ok(table(&["A", "2"], &[&["1", "2"]])),
        "a long row grows the table"
    );
    assert_eq!(
        data::parse(",B\n1,2", Format::Csv),
        Ok(table(&["1", "B"], &[&["1", "2"]]))
    );
    assert_eq!(
        data::parse("A,B\n\"x\"\"y\",a\"b\n", Format::Csv),
        Ok(table(&["A", "B"], &[&["x\"y", "a\"b"]]))
    );
    assert_eq!(
        data::parse(r#"[["A","B"],[1,null],[true,[1,2]]]"#, Format::Json),
        Ok(table(&["A", "B"], &[&["1", ""], &["true", "1, 2"]]))
    );
    assert_eq!(
        data::parse(r#"[{"A":{"x":1}}]"#, Format::Json),
        Ok(table(&["A"], &[&[r#"{"x":1}"#]]))
    );
    assert_eq!(data::parse(" \n ", Format::Csv), Err(TableError::Empty));
    assert_eq!(
        data::parse("\u{feff}", Format::Json),
        Err(TableError::Empty)
    );
    assert_eq!(data::parse("{}", Format::Json), Err(TableError::NoRows));
    assert_eq!(data::parse("5", Format::Json), Err(TableError::NoRows));
    assert_eq!(
        data::parse(r#"{"rows": []}"#, Format::Json),
        Err(TableError::NoRows)
    );
    assert_eq!(
        data::parse(r#"[{"A":1},[1]]"#, Format::Json),
        Err(TableError::NoRows)
    );
    assert!(matches!(
        data::parse("[1,", Format::Json),
        Err(TableError::Json(_))
    ));
    assert_eq!(
        data::parse(&"a,".repeat(data::MAX_COLUMNS + 1), Format::Csv),
        Err(TableError::TooManyColumns)
    );

    assert_eq!(Format::of_name("rows.TSV"), Some(Format::Tsv));
    assert_eq!(Format::of_name("rows.tab"), Some(Format::Tsv));
    assert_eq!(Format::of_name("rows.json"), Some(Format::Json));
    assert_eq!(Format::of_name("rows.csv"), Some(Format::Csv));
    assert_eq!(Format::of_name("rows.txt"), None);

    let options = ["Light".to_owned(), "Dark".to_owned()];
    assert_eq!(data::option("2", &options), Some(1));
    assert_eq!(data::option("0", &options), None);
    assert_eq!(data::option("dim", &options), None);
    assert_eq!(data::tick("Off"), Some(false));
    assert_eq!(data::tick("perhaps"), None);
    assert_eq!(data::colour("ff00ff80").map(|c| c[3]), Some(128.0 / 255.0));
    assert_eq!(data::colour("#12"), None);
    assert_eq!(data::colour("#ggg"), None);
    assert_eq!(data::colour("#é00"), None);
    assert_eq!(data::point("[1;2;3]"), Some([1.0, 2.0, 3.0]));
    assert_eq!(data::point("1"), None);
    assert_eq!(data::number("12 px"), Some(12.0));
    assert_eq!(data::number("1,000,000"), Some(1_000_000.0));
    assert_eq!(data::number("inf"), None);
    assert_eq!(data::matched_name(" First_Name "), "firstname");
}

#[test]
fn each_row_feeds_a_copy_and_makes_its_own_version() {
    let (mut kit, template) = kit::kit_with_template();
    let data = data_for(&template);
    let pictures = pictures_of(&kit);
    let c = |id: &str| control(&template, id);

    // A copy fed from a row keeps what it set itself.
    let mut inst = values::instance_for(kit.tpl, &template);
    values::set_value(&mut inst, c("opacity"), &Value::Number(25.0));
    version::feed_copy(&template, &data, 0, &mut inst, &pictures);
    assert_eq!(title_of(&inst, &template), Some(Value::Text("Ada".into())));
    assert_eq!(
        values::value_of(&inst, c("opacity"), 0.0, None),
        Some(Value::Number(25.0))
    );
    let badge = Picture {
        item: kit.badge,
        size: (200, 200),
        fit: Fit::Fill,
    };
    assert_eq!(
        values::value_of(&inst, c("logo"), 0.0, None),
        Some(Value::Picture(badge))
    );
    let held = values::held(&inst);
    assert_eq!(held.row, Some(0));
    assert!(held.fed.iter().any(|r| r == "title_size") && held.own == ["opacity"]);
    // The next row takes away what the first fed.
    version::feed_copy(&template, &data, 1, &mut inst, &pictures);
    assert_eq!(
        title_of(&inst, &template),
        Some(Value::Text("Grace".into()))
    );
    assert_eq!(values::value_of(&inst, c("tint"), 0.0, None), None);
    assert_eq!(values::value_of(&inst, c("logo"), 0.0, None), None);
    version::feed_copy(&template, &data, 9, &mut inst, &pictures);
    assert_eq!(
        (title_of(&inst, &template), values::held(&inst).row),
        (None, Some(9))
    );

    let fresh = version::fed(kit.tpl, &template, &data, 0, None, &pictures).unwrap();
    assert_eq!(values::comp_of(&fresh), Some(kit.tpl));
    let stranger = values::instance_for(kit.logo_comp, &template);
    let ignored = version::fed(kit.tpl, &template, &data, 1, Some(&stranger), &pictures).unwrap();
    assert_eq!(values::comp_of(&ignored), Some(kit.tpl));

    // Changing the data feeds every copy that follows a row again.
    let on_layer = version::fed(kit.tpl, &template, &data, 0, None, &pictures);
    let on_clip = version::fed(kit.tpl, &template, &data, 1, None, &pictures);
    let show = kit::show(&mut kit, on_layer, on_clip);
    let mut changed = data.clone();
    changed.table.rows[0][0] = "Ada L.".into();
    let refed = version::refeed_ops(&kit.doc, kit.tpl, &template, Some(&changed), &pictures);
    let [Op::SetLayerGraphInputs {
        comp,
        inputs: Some(inst),
        ..
    }] = refed.as_slice()
    else {
        panic!("one copy's row changed: {refed:?}");
    };
    assert_eq!(*comp, show);
    assert_eq!(
        title_of(inst, &template),
        Some(Value::Text("Ada L.".into()))
    );
    // Unlinking the data leaves each copy bare.
    let gone = version::refeed_ops(&kit.doc, kit.tpl, &template, None, &pictures);
    assert!(matches!(
        &gone[0],
        Op::SetLayerGraphInputs { inputs: None, .. }
    ));
    let Op::SetSequenceClips { clips, .. } = &gone[1] else {
        panic!("the clip is fed again too: {gone:?}");
    };
    assert!(clips[0].inputs.is_none());

    // One row's version: the copy in its video, or the template on its own.
    let owner = kit.doc.comp(show).unwrap();
    let copy = kit::layer_named(owner, "Copy").id;
    let cut = kit::layer_named(owner, "Cut");
    let LayerKind::Sequence { clips } = &cut.kind else {
        panic!("the cut is a Sequence layer");
    };
    let (cut, clip) = (cut.id, clips[0].id);
    let host = |layer: Uuid, clip: Option<Uuid>| Host {
        comp: show,
        layer,
        clip,
    };
    let rv = version::row_version(
        &kit.doc,
        kit.tpl,
        &template,
        &data,
        1,
        Some(host(copy, None)),
        &pictures,
    )
    .unwrap();
    assert_eq!(rv.comp, show);
    let [Op::SetLayerGraphInputs {
        inputs: Some(inst), ..
    }] = rv.ops.as_slice()
    else {
        panic!("one op for the copy: {:?}", rv.ops);
    };
    assert_eq!(title_of(inst, &template), Some(Value::Text("Grace".into())));
    let rv = version::row_version(
        &kit.doc,
        kit.tpl,
        &template,
        &data,
        0,
        Some(host(cut, Some(clip))),
        &pictures,
    )
    .unwrap();
    let [Op::SetSequenceClips { clips, .. }] = rv.ops.as_slice() else {
        panic!("one op for the clip: {:?}", rv.ops);
    };
    assert_eq!(
        title_of(clips[0].inputs.as_ref().unwrap(), &template),
        Some(Value::Text("Ada".into()))
    );
    let wrong = [host(copy, Some(clip)), host(Uuid::now_v7(), None)];
    for host in wrong {
        let rv = version::row_version(
            &kit.doc,
            kit.tpl,
            &template,
            &data,
            0,
            Some(host),
            &pictures,
        );
        assert!(rv.is_none());
    }

    let alone =
        version::row_version(&kit.doc, kit.tpl, &template, &data, 0, None, &pictures).unwrap();
    assert_eq!(alone.read.length, Some(8.0));
    let mut made = kit.doc.clone();
    for op in &alone.ops {
        ops::apply(&mut made, op).unwrap();
    }
    let outer = made.comp(alone.comp).unwrap();
    assert_eq!(outer.duration.0.to_f64(), 8.0, "as long as the row says");
    let [placed] = outer.layers.as_slice() else {
        panic!("one copy of the template");
    };
    assert_eq!(placed.kind, LayerKind::Precomp { comp: kit.tpl });
    assert_eq!(placed.out_point.0.to_f64(), 8.0);
    assert_eq!(
        values::held(placed.graph_inputs.as_ref().unwrap()).row,
        Some(0)
    );
    let unread =
        version::row_version(&kit.doc, kit.tpl, &template, &data, 1, None, &pictures).unwrap();
    let mut made = kit.doc.clone();
    for op in &unread.ops {
        ops::apply(&mut made, op).unwrap();
    }
    assert_eq!(made.comp(unread.comp).unwrap().duration.0.to_f64(), 5.0);
    assert!(
        version::row_version(&kit.doc, kit.tpl, &template, &data, 2, None, &pictures).is_none()
    );

    // A row is faulty for a cell that will not read or a file not held.
    let first = feed::row_values(&template, &data, 0).unwrap();
    let second = feed::row_values(&template, &data, 1).unwrap();
    let nothing = |_: &Path| -> Option<(Uuid, (u32, u32))> { None };
    assert!(!version::faulty(&data, &first, &pictures));
    assert!(version::faulty(&data, &first, &nothing));
    assert!(version::faulty(&data, &second, &pictures));
}

#[test]
fn a_rows_file_name_comes_from_its_cells() {
    let (_, template) = kit::kit_with_template();
    let data = data_for(&template);
    let name = |pattern: &str, row: usize, named: Option<&str>| {
        version::row_name(pattern, "Lower third", &data, row, named, "2026-11-11")
    };
    assert_eq!(
        name("{comp}/{Title} {row} on {date}", 0, None),
        "Lower third/Ada 1 on 2026-11-11"
    );
    assert_eq!(name("{comp}", 0, Some("ada/out:1")), "ada/out_1");
    assert_eq!(name("{Title}", 1, Some("  ")), "Grace");
    assert_eq!(name("../../{Title}?", 0, None), "Ada_");
    assert_eq!(name("{Unknown} x", 0, None), "{Unknown} x");
    assert_eq!(name("a {Title", 0, None), "a {Title");
    assert_eq!(name("...", 0, None), "row");
    assert_eq!(name("{Notes}", 1, None), "");
    assert_eq!(name("{Title}{row}", 5, None), "{Title}6");

    assert_eq!(
        version::cell_file("jobs/rows.csv", "logo.png"),
        Path::new("jobs").join("logo.png")
    );
    let whole = std::env::temp_dir().join("logo.png");
    let whole_text = whole.to_string_lossy().into_owned();
    assert_eq!(version::cell_file("jobs/rows.csv", &whole_text), whole);
    assert_eq!(
        version::cell_file("rows.csv", "logo.png"),
        PathBuf::from("logo.png")
    );
}

#[test]
fn custom_controls_hold_a_menu_and_its_own_words() {
    let rig = |id: &str, label: &str, kind: RigKind, first: f64, options: &[&str]| Rig {
        id: id.into(),
        label: label.into(),
        kind,
        default: [first, 0.25, 0.0, 1.0],
        min: 0.0,
        max: 50.0,
        options: options.iter().map(|o| (*o).to_owned()).collect(),
    };
    let list = vec![
        rig("amp", "Amplitude", RigKind::Number, 5.0, &[]),
        rig("turn", "Turn", RigKind::Angle, 45.0, &[]),
        rig("on", "On", RigKind::Checkbox, 1.0, &[]),
        rig("tint", "Tint", RigKind::Colour, 0.5, &[]),
        rig("mode", "Mode", RigKind::Menu, 1.0, &["Calm", "Wild"]),
        // A second control under one id is one value, shown once.
        rig("amp", "Again", RigKind::Number, 9.0, &[]),
    ];
    let mut inst = builtins::instantiate("custom_controls").unwrap();
    custom::set_controls(&mut inst, &list);
    assert_eq!(custom::controls_of(&inst), list);
    let value = |id: &str| {
        inst.params
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.value.clone())
    };
    assert_eq!(inst.params.len(), 5);
    assert_eq!(value("mode"), Some(EffectValue::Choice(1)));
    assert_eq!(value("on"), Some(EffectValue::Bool(true)));
    assert!(matches!(value("amp"), Some(EffectValue::Float(p)) if p.value_at(0.0) == 5.0));
    assert!(matches!(value("tint"), Some(EffectValue::Colour(_))));
    assert_eq!(custom::id_of(&inst, "MODE"), Some("mode"));
    assert_eq!(custom::id_of(&inst, "nope"), None);

    let def = builtins::def("custom_controls").unwrap();
    let derived = def.derived(&inst);
    let ids: Vec<&str> = derived.iter().map(|r| r.id).collect();
    assert_eq!(ids, ["amp", "turn", "on", "tint", "mode"]);
    assert!(
        matches!(derived[0].kind, ParamKind::Float { default, slider, .. } if default == 5.0 && slider == (0.0, 50.0))
    );
    assert!(matches!(derived[1].kind, ParamKind::Angle { default, .. } if default == 45.0));
    assert!(matches!(
        derived[2].kind,
        ParamKind::Bool { default: true, .. }
    ));
    assert!(
        matches!(derived[3].kind, ParamKind::Colour { default, .. } if default == [0.5, 0.25, 0.0, 1.0])
    );
    assert!(matches!(
        derived[4].kind,
        ParamKind::Choice { default: 1, .. }
    ));
    let words = vec!["Calm".to_owned(), "Wild".to_owned()];
    assert_eq!(def.row_state(&inst).options, vec![("mode", words)]);

    // A control taken away takes its value with it.
    custom::set_controls(&mut inst, &list[4..5]);
    let ids: Vec<&str> = inst.params.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["mode"]);
    custom::set_controls(&mut inst, &[]);
    assert!(custom::controls_of(&inst).is_empty());
    assert!(!inst.extra.contains_key(custom::EXTRA_KEY));
    assert!(def.derived(&inst).is_empty());
}

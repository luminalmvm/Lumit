use std::io::Read;
use std::sync::Arc;

use lumit_core::anim::{Animation, CubicSpan, Keyframe, Property, SideInterp};
use lumit_core::edits::base_layer;
use lumit_core::mask::{BezierPath, Mask, PathKeyframe, Vertex};
use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, MatteChannel, MatteRef, ProjectItem,
    TextDocument, TransformGroup,
};
use lumit_core::shape::modifier::{ModifierKind, ShapeEntry, ShapeModifier, Trim};
use lumit_core::shape::primitive::ShapePrimitive;
use lumit_core::shape::ShapeItem;
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use serde_json::Value;
use uuid::Uuid;

use crate::settings::{Format, Settings};
use crate::{Host, Picture, Still};

/// A host whose every footage item is a small still.
struct Stills;

impl Host for Stills {
    fn picture(&self, _item: Uuid) -> Picture {
        let (w, h) = (8u32, 6u32);
        let rgba = (0..w * h)
            .flat_map(|i| [(i * 5) as u8, 128, (255 - i * 5) as u8, 255])
            .collect();
        Picture::Still(Still {
            width: w,
            height: h,
            rgba,
        })
    }
}

fn r(v: f64) -> Rational {
    Rational::new((v * 1000.0).round() as i64, 1000).unwrap()
}

fn key(t: f64, v: f64, inn: SideInterp, out: SideInterp) -> Keyframe {
    Keyframe {
        time: r(t),
        value: v,
        interp_in: inn,
        interp_out: out,
    }
}

fn ease(speed: f64, influence: f64) -> SideInterp {
    SideInterp::Bezier { speed, influence }
}

fn keyed(keys: Vec<Keyframe>) -> Property {
    Property {
        animation: Animation::Keyframed(keys),
        extra: serde_json::Map::new(),
    }
}

fn comp(name: &str, w: u32, h: u32, seconds: i64, layers: Vec<Layer>) -> Composition {
    Composition {
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
        name: name.into(),
        width: w,
        height: h,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(seconds, 1).unwrap()),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn at(x: f64, y: f64) -> TransformGroup {
    TransformGroup {
        position_x: Property::fixed(x),
        position_y: Property::fixed(y),
        ..TransformGroup::default()
    }
}

fn vertex(x: f64, y: f64) -> Vertex {
    Vertex {
        pos: (x, y),
        tan_in: (0.0, 0.0),
        tan_out: (0.0, 0.0),
    }
}

/// A comp with something of everything the emitter maps: eased keys, a
/// morphing path, a trim, a precomp with Retime, a mask, a matte, text as
/// shapes with an animator, an image, a solid and an expression.
fn fixture() -> (Arc<Document>, Uuid) {
    let seconds = 4;
    // The precomp: a ball crossing it.
    let mut ball = ShapeItem::filled(
        "Ball",
        BezierPath {
            vertices: Vec::new(),
            closed: true,
        },
        LinearColour([0.9, 0.2, 0.1, 1.0]),
    );
    ball.primitive = Some(ShapePrimitive {
        kind: 1,
        position_x: Property::fixed(0.0),
        position_y: Property::fixed(0.0),
        width: Property::fixed(80.0),
        height: Property::fixed(80.0),
        roundness: Property::zero(),
        points: Property::fixed(5.0),
        rotation: Property::zero(),
        outer_radius: Property::zero(),
        inner_radius: Property::zero(),
        outer_roundness: Property::zero(),
        inner_roundness: Property::zero(),
        extra: serde_json::Map::new(),
    });
    let mut ball_layer = base_layer(
        "Ball".into(),
        LayerKind::Shape {
            contents: vec![ShapeEntry::Item(ball)],
        },
        Rational::new(seconds, 1).unwrap(),
        at(0.0, 100.0),
    );
    ball_layer.transform.position_x = keyed(vec![
        key(0.0, 40.0, SideInterp::Linear, ease(0.0, 0.5)),
        key(4.0, 260.0, ease(0.0, 0.8), SideInterp::Linear),
    ]);
    let inner = comp("Inner", 300, 200, seconds, vec![ball_layer]);
    let inner_id = inner.id;

    // The card: a rounded rectangle and a shape that morphs, trimmed.
    let mut card = ShapeItem::filled(
        "Card",
        BezierPath {
            vertices: Vec::new(),
            closed: true,
        },
        LinearColour([0.1, 0.3, 0.8, 1.0]),
    );
    card.primitive = Some(ShapePrimitive {
        kind: 0,
        width: Property::fixed(160.0),
        height: Property::fixed(100.0),
        roundness: Property::fixed(16.0),
        ..ball_layer_primitive()
    });
    let mut morph = ShapeItem::filled(
        "Morph",
        BezierPath {
            vertices: Vec::new(),
            closed: true,
        },
        LinearColour([1.0, 0.8, 0.1, 1.0]),
    );
    morph.stroke = Some(lumit_core::shape::colour::still(LinearColour([
        1.0, 1.0, 1.0, 1.0,
    ])));
    morph.stroke_width = Property::fixed(4.0);
    let tri = BezierPath {
        vertices: vec![vertex(0.0, -40.0), vertex(40.0, 30.0), vertex(-40.0, 30.0)],
        closed: true,
    };
    let square = BezierPath {
        vertices: vec![
            vertex(-35.0, -35.0),
            vertex(35.0, -35.0),
            vertex(35.0, 35.0),
            vertex(-35.0, 35.0),
        ],
        closed: true,
    };
    morph.path = tri.clone();
    morph.path_keys = vec![
        PathKeyframe {
            time: r(0.5),
            path: tri,
            interp_in: SideInterp::Linear,
            interp_out: ease(0.0, 0.66),
        },
        PathKeyframe {
            time: r(2.5),
            path: square,
            interp_in: ease(0.0, 0.66),
            interp_out: SideInterp::Linear,
        },
    ];
    let trim = ShapeModifier {
        id: Uuid::now_v7(),
        name: "Trim".into(),
        hidden: false,
        own: true,
        kind: ModifierKind::Trim(Trim {
            start: Property::zero(),
            end: keyed(vec![
                key(0.0, 0.0, SideInterp::Linear, ease(0.0, 0.4)),
                key(1.5, 100.0, ease(0.0, 0.9), SideInterp::Linear),
            ]),
            offset: Property::zero(),
            extra: serde_json::Map::new(),
        }),
    };
    let mut card_layer = base_layer(
        "Card".into(),
        LayerKind::Shape {
            contents: vec![
                ShapeEntry::Item(card),
                ShapeEntry::Item(morph),
                ShapeEntry::Modifier(trim),
            ],
        },
        Rational::new(seconds, 1).unwrap(),
        at(320.0, 180.0),
    );
    card_layer.start_offset = CompTime(r(0.5));
    card_layer.transform.position_y = keyed(vec![
        key(0.0, 120.0, SideInterp::Linear, ease(0.0, 0.33)),
        key(1.0, 200.0, ease(-80.0, 0.5), ease(30.0, 0.2)),
        key(2.0, 180.0, ease(0.0, 0.7), SideInterp::Linear),
    ]);
    card_layer.transform.rotation = keyed(vec![
        key(0.0, -10.0, SideInterp::Linear, SideInterp::Hold),
        key(1.2, 10.0, SideInterp::Hold, SideInterp::Linear),
    ]);
    if let Some(shadow) = lumit_core::fx::builtins::instantiate("drop_shadow") {
        card_layer.effects.push(shadow);
    }
    card_layer.effects.push(effect(
        "transform",
        &[("position_x", 12.0), ("rotation", 4.0)],
    ));

    // The precomp layer, retimed and masked, its opacity an expression.
    let mut badge = base_layer(
        "Badge".into(),
        LayerKind::Precomp { comp: inner_id },
        Rational::new(seconds, 1).unwrap(),
        at(170.0, 120.0),
    );
    badge.transform.anchor_x = Property::fixed(150.0);
    badge.transform.anchor_y = Property::fixed(100.0);
    badge.retime = Some(keyed(vec![
        key(0.0, 0.0, SideInterp::Linear, ease(0.0, 0.6)),
        key(4.0, 4.0, ease(3.0, 0.3), SideInterp::Linear),
    ]));
    badge.masks.push(Mask::ellipse(150.0, 100.0, 140.0, 80.0));
    badge.effects.push(effect(
        "stroke",
        &[("brush_size", 6.0), ("hardness", 100.0)],
    ));
    badge.transform.opacity = Property {
        animation: Animation::Expression("40.0 + time * 15.0".into()),
        extra: serde_json::Map::new(),
    };

    // A title as shapes, a letter at a time, through a wipe.
    let mut animator = lumit_core::text::TextAnimator::new("Rise");
    animator.position_y = Property::fixed(30.0);
    animator.opacity = Property::fixed(0.0);
    animator.selector.start = keyed(vec![
        key(0.5, 0.0, SideInterp::Linear, SideInterp::Linear),
        key(2.5, 100.0, SideInterp::Linear, SideInterp::Linear),
    ]);
    let title_doc = TextDocument {
        text: "Lumit".into(),
        expression: None,
        size: 64.0,
        fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
        path: None,
        path_offset: Property::zero(),
        animators: vec![animator],
        captions: None,
        style: Box::default(),
        paragraph: Default::default(),
        extra: serde_json::Map::new(),
    };
    let mut title = base_layer(
        "Title".into(),
        LayerKind::Text {
            document: title_doc,
        },
        Rational::new(seconds, 1).unwrap(),
        at(380.0, 40.0),
    );
    let mut wipe_rect = ShapeItem::filled(
        "Wipe",
        Mask::rectangle(0.0, 0.0, 240.0, 160.0).path,
        LinearColour([1.0; 4]),
    );
    wipe_rect.opacity = Property::fixed(100.0);
    let mut wipe = base_layer(
        "Wipe".into(),
        LayerKind::Shape {
            contents: vec![ShapeEntry::Item(wipe_rect)],
        },
        Rational::new(seconds, 1).unwrap(),
        at(360.0, 20.0),
    );
    wipe.transform.position_x = keyed(vec![
        key(0.0, 120.0, SideInterp::Linear, SideInterp::Linear),
        key(2.0, 380.0, SideInterp::Linear, SideInterp::Linear),
    ]);
    wipe.switches.visible = false;
    title.matte = Some(MatteRef {
        layer: wipe.id,
        channel: MatteChannel::Alpha,
        inverted: false,
        source: Default::default(),
    });

    // A still, scaling up, and a solid behind everything.
    let mut photo = base_layer(
        "Photo".into(),
        LayerKind::Footage {
            item: Uuid::now_v7(),
        },
        Rational::new(seconds, 1).unwrap(),
        at(560.0, 300.0),
    );
    photo.transform.scale_x = keyed(vec![
        key(0.0, 600.0, SideInterp::Linear, SideInterp::Linear),
        key(4.0, 900.0, SideInterp::Linear, SideInterp::Linear),
    ]);
    photo.transform.scale_y = photo.transform.scale_x.clone();
    photo.effects.push(effect(
        "levels",
        &[("master_in_white", 0.5), ("master_gamma", 1.4)],
    ));
    let solid_def = lumit_core::model::SolidDef {
        id: Uuid::now_v7(),
        name: "Bg".into(),
        colour: LinearColour([0.05, 0.05, 0.08, 1.0]),
        width: 640,
        height: 360,
        extra: serde_json::Map::new(),
    };
    let bg = base_layer(
        "Bg".into(),
        LayerKind::Solid { def: solid_def.id },
        Rational::new(seconds, 1).unwrap(),
        TransformGroup::default(),
    );

    let main = comp(
        "Main",
        640,
        360,
        seconds,
        vec![title, wipe, card_layer, badge, photo, bg],
    );
    let main_id = main.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(solid_def));
    doc.items.push(ProjectItem::Composition(inner));
    doc.items.push(ProjectItem::Composition(main));
    (Arc::new(doc), main_id)
}

/// A built-in effect with some of its numbers set.
fn effect(name: &str, set: &[(&str, f64)]) -> lumit_core::model::EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate(name).unwrap();
    for (id, v) in set {
        let p = e.params.iter_mut().find(|p| p.id == *id).unwrap();
        p.value = lumit_core::model::EffectValue::Float(Property::fixed(*v));
    }
    e
}

fn ball_layer_primitive() -> ShapePrimitive {
    ShapePrimitive {
        kind: 0,
        position_x: Property::zero(),
        position_y: Property::zero(),
        width: Property::zero(),
        height: Property::zero(),
        roundness: Property::zero(),
        points: Property::fixed(5.0),
        rotation: Property::zero(),
        outer_radius: Property::zero(),
        inner_radius: Property::zero(),
        outer_roundness: Property::zero(),
        inner_roundness: Property::zero(),
        extra: serde_json::Map::new(),
    }
}

/// A Lottie property's value at frame `f`, evaluated as a player does.
fn play(p: &Value, f: f64) -> Vec<f64> {
    let nums = |v: &Value| -> Vec<f64> {
        match v {
            Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
            v => v.as_f64().into_iter().collect(),
        }
    };
    if p["a"] == 0 {
        return nums(&p["k"]);
    }
    let keys = p["k"].as_array().unwrap();
    let t = |k: &Value| k["t"].as_f64().unwrap();
    if f <= t(&keys[0]) {
        return nums(&keys[0]["s"]);
    }
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if f >= t(b) {
            continue;
        }
        let (v0, v1) = (nums(&a["s"]), nums(&b["s"]));
        if a["h"] == 1 {
            return v0;
        }
        let u = (f - t(a)) / (t(b) - t(a));
        return (0..v0.len())
            .map(|d| {
                let pick = |side: &str, axis: &str| {
                    let v = &a[side][axis];
                    let list = nums(v);
                    list.get(d).or(list.first()).copied().unwrap()
                };
                let e = CubicSpan::from_points(
                    [0.0, pick("o", "x"), pick("i", "x"), 1.0],
                    [0.0, pick("o", "y"), pick("i", "y"), 1.0],
                )
                .value_at(u);
                v0[d] + (v1[d] - v0[d]) * e
            })
            .collect();
    }
    nums(&keys[keys.len() - 1]["s"])
}

fn layer<'a>(layers: &'a Value, name: &str) -> &'a Value {
    layers
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["nm"] == name && l.get("td").is_none())
        .unwrap_or_else(|| panic!("no layer {name}"))
}

/// The promise of the emitter: the file a player reads gives, frame by
/// frame, the numbers Lumit draws, and everything in the comp arrives as
/// what it is (keys, a precomp with its Retime, a mask, a matte, letters,
/// an image).
#[test]
fn an_export_plays_back_what_lumit_draws() {
    let (doc, main) = fixture();
    let out = crate::export(&doc, main, &Settings::default(), &Stills).unwrap();
    let json: Value = serde_json::from_slice(&out.bytes).unwrap();
    // `ty` leads every object: the Flutter and Android players skip what
    // comes before it, and draw nothing.
    assert!(!String::from_utf8_lossy(&out.bytes).contains(",\"ty\":"));
    assert_eq!(json["v"], "5.12.2");
    assert_eq!(json["fr"], 30);
    assert_eq!(json["op"], 120);
    let comp = doc.comp(main).unwrap();
    let layers = &json["layers"];
    let lumit = |name: &str| comp.layers.iter().find(|l| l.name == name).unwrap();

    for f in 0..=120 {
        let f = f64::from(f);
        let t = f / 30.0;
        // Eased and held keys, on a layer that starts late.
        let card = lumit("Card");
        let lt = t - 0.5;
        let ks = &layer(layers, "Card")["ks"];
        let y = play(&ks["p"]["y"], f)[0];
        assert!(
            (y - card.transform.position_y.value_at(lt)).abs() < 2e-3,
            "card y at {f}: {y}"
        );
        let rot = play(&ks["r"], f)[0];
        assert!(
            (rot - card.transform.rotation.value_at(lt)).abs() < 2e-3,
            "card r at {f}"
        );

        // The precomp's Retime, and its expression baked to the frame.
        let badge = lumit("Badge");
        let l = layer(layers, "Badge");
        let tm = play(&l["tm"], f)[0];
        let LayerKind::Precomp { comp: inner } = badge.kind else {
            panic!()
        };
        let shown = lumit_core::model::nested_source_time(badge, doc.comp(inner).unwrap(), t);
        assert!((tm - shown).abs() < 2e-3, "tm at {f}: {tm} vs {shown}");
        let o = play(&l["ks"]["o"], f)[0];
        assert!(
            (o - (40.0 + t * 15.0)).abs() < 0.02,
            "badge opacity at {f}: {o}"
        );
    }

    // The morph eases between its two shapes as Lumit's does.
    let card = lumit("Card");
    let LayerKind::Shape { contents } = &card.kind else {
        panic!()
    };
    let ShapeEntry::Item(morph) = &contents[1] else {
        panic!()
    };
    let card_json = layer(layers, "Card");
    let group = card_json["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["nm"] == "Morph")
        .unwrap();
    let ks = &group["it"][0]["ks"];
    for f in [15.0, 30.0, 47.0, 60.0, 75.0, 90.0] {
        let lt = f / 30.0 - 0.5;
        let want = lumit_core::mask::resample(&morph.path_at(lt), 4);
        let keys = ks["k"].as_array().unwrap();
        let k0 = &keys[0];
        let k1 = &keys[1];
        let (t0, t1) = (k0["t"].as_f64().unwrap(), k1["t"].as_f64().unwrap());
        let u = ((f - t0) / (t1 - t0)).clamp(0.0, 1.0);
        let e = CubicSpan::from_points(
            [
                0.0,
                k0["o"]["x"].as_f64().unwrap(),
                k0["i"]["x"].as_f64().unwrap(),
                1.0,
            ],
            [
                0.0,
                k0["o"]["y"].as_f64().unwrap(),
                k0["i"]["y"].as_f64().unwrap(),
                1.0,
            ],
        )
        .value_at(u);
        for (n, v) in want.vertices.iter().enumerate() {
            let a = &k0["s"][0]["v"][n];
            let b = &k1["s"][0]["v"][n];
            let x = a[0].as_f64().unwrap() + (b[0].as_f64().unwrap() - a[0].as_f64().unwrap()) * e;
            assert!(
                (x - v.pos.0).abs() < 0.01,
                "morph vertex {n} at {f}: {x} vs {}",
                v.pos.0
            );
        }
    }

    // A matte copy sits directly above the title and points at it.
    let list = layers.as_array().unwrap();
    let title = list.iter().position(|l| l["nm"] == "Title").unwrap();
    assert_eq!(list[title - 1]["td"], 1);
    assert_eq!(list[title]["tt"], 1);
    assert_eq!(list[title]["tp"], list[title - 1]["ind"]);
    // Letters, one group each.
    assert_eq!(list[title]["shapes"].as_array().unwrap().len(), 5);
    // The mask, the precomp's asset and the embedded picture.
    let badge = layer(layers, "Badge");
    assert_eq!(badge["hasMask"], true);
    assert_eq!(badge["masksProperties"][0]["mode"], "a");
    let assets = json["assets"].as_array().unwrap();
    assert!(assets
        .iter()
        .any(|a| a["id"] == badge["refId"] && a["layers"].is_array()));
    assert!(assets.iter().any(|a| a["p"]
        .as_str()
        .is_some_and(|p| p.starts_with("data:image/png;base64,"))));
    // The expression was baked, and the check says so.
    assert!(out.issues.iter().any(|i| i.reason == "expression_baked"));

    // The effects lottie-web draws, in its units: a Transform's points in the
    // art's coordinates, a Stroke's brush as a radius on the first mask, and
    // Levels' white point in the display encoding.
    let ef = |l: &Value, ty: u64| {
        l["ef"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["ty"] == ty)
            .unwrap()
            .clone()
    };
    let card_ef = ef(layer(layers, "Card"), 35);
    let LayerKind::Shape { contents } = &card.kind else {
        panic!()
    };
    let (x0, ..) = lumit_core::shape::contents_bounds(contents, 0.0).unwrap();
    assert!((card_ef["ef"][1]["v"]["k"][0].as_f64().unwrap() - (12.0 + x0)).abs() < 1e-3);
    let stroke = ef(layer(layers, "Badge"), 22);
    assert_eq!(stroke["ef"][0]["v"]["k"], 1);
    assert_eq!(stroke["ef"][4]["v"]["k"], 3);
    let levels = ef(layer(layers, "Photo"), 24);
    let white = levels["ef"][4]["v"]["k"].as_f64().unwrap();
    assert!((white - crate::colour::srgb_encode(0.5)).abs() < 1e-3);
    assert_eq!(levels["ef"].as_array().unwrap().len(), 36);
}

/// A .lottie opens as a v2 dotLottie: its manifest, its animation and its
/// picture inside, an opaque one as a JPEG when a quality is asked for.
#[test]
fn a_dotlottie_holds_its_manifest_animation_and_pictures() {
    let (doc, main) = fixture();
    let settings = Settings {
        format: Format::DotLottie,
        file_stem: "opening titles".into(),
        image_quality: Some(80),
        ..Settings::default()
    };
    let out = crate::export(&doc, main, &settings, &Stills).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(out.bytes)).unwrap();
    let mut read = |name: &str| {
        let mut s = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut s).unwrap();
        s
    };
    let manifest: Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["version"], "2");
    assert_eq!(manifest["animations"][0]["id"], "opening_titles");
    let animation: Value = serde_json::from_slice(&read("a/opening_titles.json")).unwrap();
    let image = animation["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["u"] == "/i/")
        .unwrap();
    assert_eq!(image["e"], 0);
    // An opaque picture with a quality asked for goes in as a JPEG.
    assert_eq!(image["p"], "image_1.jpg");
    let jpeg = read("i/image_1.jpg");
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
}

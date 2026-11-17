#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use lumit_core::anim::{evaluate, Animation, Keyframe, SideInterp};
use lumit_core::model::{Composition, LayerKind, MatteChannel, ProjectItem};
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::time::Rational;
use serde_json::{json, Value};

use super::import;

/// A small deterministic generator, so the cases are the same every run.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

fn main_comp(items: &[ProjectItem]) -> &Composition {
    match items.first() {
        Some(ProjectItem::Composition(c)) => c,
        _ => panic!("the main comp comes first"),
    }
}

/// The whole promise of the key maths, both ways: Lumit keys written as a
/// Lottie's keyframes, read back in, evaluate as the keys did, for eased,
/// straight, held, automatic and bumped spans alike. Two layers in three
/// start late, and their keys are written in the comp's frames, as a Lottie
/// has them, so they must land on the layer's own clock.
#[test]
fn eased_keys_come_back_as_they_went_out() {
    let fps = 30.0;
    let mut rng = Lcg(11);
    let mut cases: Vec<Vec<Keyframe>> = Vec::new();
    let mut layers = Vec::new();
    for case in 0..120 {
        let n = 2 + (rng.next() * 5.0) as usize;
        let mut t = 0.0;
        let keys: Vec<Keyframe> = (0..n)
            .map(|i| {
                t += 0.1 + (rng.next() * 2.0 * 1000.0).round() / 1000.0;
                let v = if case % 4 == 0 && i % 2 == 0 {
                    10.0
                } else {
                    (rng.next() - 0.5) * 400.0
                };
                let mut side = || {
                    let r = rng.next();
                    match (r * 6.0) as u32 {
                        0 => SideInterp::Linear,
                        1 => SideInterp::Hold,
                        2 => SideInterp::Auto {
                            clamped: r > 0.4,
                            speed: 0.0,
                            influence: 0.2 + r * 0.5,
                        },
                        _ => SideInterp::Bezier {
                            speed: (rng.next() - 0.5) * 300.0,
                            influence: 0.01 + rng.next() * 0.99,
                        },
                    }
                };
                let (interp_in, interp_out) = (side(), side());
                Keyframe {
                    time: Rational::new((t * 1000.0_f64).round() as i64, 1000).unwrap(),
                    value: v,
                    interp_in,
                    interp_out,
                }
            })
            .collect();
        let track = crate::keys::track(&[crate::keys::curve(&keys)]).unwrap();
        let st = f64::from(case % 3) * 7.5;
        let k: Vec<Value> = track
            .keys
            .iter()
            .map(|key| {
                let mut out = json!({"t": key.seconds * fps + st, "s": key.value});
                if key.hold {
                    out["h"] = json!(1);
                } else if let (Some(o), Some(i)) = (key.out.first(), key.inn.first()) {
                    out["o"] = json!({"x": [o[0]], "y": [o[1]]});
                    out["i"] = json!({"x": [i[0]], "y": [i[1]]});
                }
                out
            })
            .collect();
        layers.push(json!({
            "ty": 3, "ind": case + 1, "ip": 0, "op": 600, "st": st,
            "ks": {"r": {"a": 1, "k": k}}
        }));
        cases.push(keys);
    }
    let file =
        json!({"v": "5.12.2", "fr": fps, "ip": 0, "op": 600, "w": 100, "h": 100, "layers": layers});
    let imported = import(file.to_string().as_bytes(), Path::new("keys.json")).unwrap();
    let comp = main_comp(&imported.items);
    for (n, (keys, layer)) in cases.iter().zip(&comp.layers).enumerate() {
        let st = (n % 3) as f64 * 7.5 / fps;
        assert!((layer.start_offset.0.to_f64() - st).abs() < 1e-12);
        let (t0, t1) = (keys[0].time.to_f64(), keys[keys.len() - 1].time.to_f64());
        for s in 0..=300 {
            let at = t0 - 0.5 + (t1 - t0 + 1.0) * f64::from(s) / 300.0;
            let want = evaluate(keys, at).unwrap();
            let got = layer.transform.rotation.value_at(at);
            assert!(
                (want - got).abs() <= 1e-6 * (1.0 + want.abs()),
                "{}: at {at} Lumit {want} imported {got}",
                layer.name
            );
        }
    }
}

/// One small file with each kind that has to land somewhere particular: a
/// precomp with a Retime, a mask, a matte, an embedded picture, a parent and
/// a moved group of shapes, with a key in the old form.
#[test]
fn a_small_lottie_lands_where_it_should() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let rect = json!({"ty": "gr", "it": [
        {"ty": "rc", "p": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [20, 10]}, "r": {"a": 0, "k": 0}},
        {"ty": "fl", "c": {"a": 0, "k": [1, 0, 0, 1]}, "o": {"a": 0, "k": 50}, "r": 1},
        {"ty": "tr", "p": {"a": 0, "k": [10, 5]}, "a": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [100, 100]},
         "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}}
    ]});
    // A hole listed before its outline, wound the other way.
    let square = |half: f64, turn: f64| {
        let v: Vec<[f64; 2]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
            .iter()
            .map(|[x, y]| [x * half, y * half * turn])
            .collect();
        let flat = vec![[0.0f64; 2]; 4];
        json!({"ty": "sh", "ks": {"a": 0, "k": {"c": true, "v": v, "i": flat, "o": flat}}})
    };
    let matte = json!({"ty": 4, "ind": 1, "nm": "Matte", "td": 1, "ip": 0, "op": 60, "st": 0, "ks": {},
        "shapes": [square(10.0, -1.0), square(25.0, 1.0),
                   {"ty": "fl", "c": {"a": 0, "k": [1, 1, 1, 1]}, "o": {"a": 0, "k": 100}, "r": 1}]});
    let position = json!({"a": 1, "k": [
        {"t": 0, "s": [0, 0], "e": [100, 50], "o": {"x": 0.3, "y": 0}, "i": {"x": 0.7, "y": 1}},
        {"t": 30}
    ]});
    let retime = json!({"a": 1, "k": [
        {"t": 0, "s": [0], "o": {"x": [0], "y": [0]}, "i": {"x": [1], "y": [1]}},
        {"t": 30, "s": [2]}
    ]});
    let mask = json!({"mode": "s", "inv": false, "o": {"a": 0, "k": 50}, "x": {"a": 0, "k": 0},
        "pt": {"a": 0, "k": {"c": true, "v": [[0, 0], [10, 0], [10, 10]],
                             "i": [[0, 0], [0, 0], [0, 0]], "o": [[0, 0], [0, 0], [0, 0]]}}});
    let pre = json!({"ty": 0, "ind": 2, "nm": "Pre", "refId": "pre", "tt": 1, "tp": 1, "w": 200, "h": 100,
        "ip": 0, "op": 60, "st": 0, "ks": {"p": position}, "tm": retime,
        "hasMask": true, "masksProperties": [mask]});
    let picture = json!({"ty": 2, "ind": 3, "nm": "Picture", "refId": "img", "parent": 2,
        "ip": 0, "op": 60, "st": 0, "ks": {}});
    let inner = json!({"ty": 4, "ind": 1, "ip": 0, "op": 90, "st": 0, "ks": {}, "shapes": [rect]});
    let assets = json!([
        {"id": "img", "w": 1, "h": 1, "u": "", "p": format!("data:image/png;base64,{png}"), "e": 1},
        {"id": "pre", "nm": "Inner", "layers": [inner]}
    ]);
    let file = json!({
        "v": "5.7.4", "fr": 30, "ip": 0, "op": 60, "w": 200, "h": 100, "nm": "Main",
        "assets": assets, "layers": [matte, pre, picture]
    });
    let source = Path::new("C:/somewhere/logo.json");
    let imported = import(file.to_string().as_bytes(), source).unwrap();
    let comp = main_comp(&imported.items);
    assert_eq!(imported.main, comp.id);
    assert_eq!(imported.layers, 4);
    let [matte, pre, picture] = &comp.layers[..] else {
        panic!("three layers, got {}", comp.layers.len());
    };

    // The matte is drawn only through the layer it mattes.
    assert!(!matte.switches.visible);
    // Its hole is cut from its outline, not the outline from the hole.
    let LayerKind::Shape { contents } = &matte.kind else {
        panic!("a shape layer");
    };
    let runs: Vec<(usize, u32)> = lumit_core::shape::modifier::items(contents)
        .iter()
        .map(|i| (i.path.vertices.len(), i.combine))
        .collect();
    assert_eq!(runs.len(), 2);
    let outline = lumit_core::shape::modifier::items(contents)[0]
        .path
        .vertices[0]
        .pos;
    assert_eq!((outline, runs[0].1, runs[1].1), ((-25.0, -25.0), 0, 2));
    let m = pre.matte.unwrap();
    assert_eq!(
        (m.layer, m.channel, m.inverted),
        (matte.id, MatteChannel::Alpha, false)
    );

    // The old key form, eased in and out: slow at first, half way at half time.
    let x = pre.transform.position_x.value_at(0.25);
    assert!(x > 0.0 && x < 20.0, "position x {x}");
    assert!((pre.transform.position_x.value_at(0.5) - 50.0).abs() < 1e-6);
    assert!((pre.transform.position_x.value_at(1.0) - 100.0).abs() < 1e-9);

    // A straight Retime from 0 to 2 seconds of the precomp over its first second.
    let retime = pre.retime.as_ref().unwrap();
    assert!((retime.value_at(0.5) - 1.0).abs() < 1e-9);

    let mask = &pre.masks[0];
    assert_eq!(mask.mode, lumit_core::mask::MaskMode::Subtract);
    assert!((mask.opacity.value_at(0.0) - 50.0).abs() < 1e-9);
    assert_eq!(mask.path.vertices.len(), 3);

    assert_eq!(picture.parent, Some(pre.id));
    let LayerKind::Footage { item } = picture.kind else {
        panic!("a footage layer");
    };
    let (file, bytes) = &imported.files[0];
    assert_eq!(
        file,
        &Path::new("C:/somewhere")
            .join("logo images")
            .join("img.png")
    );
    assert!(bytes.starts_with(b"\x89PNG"));
    let footage = imported
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Footage(f) if f.id == item => Some(f),
            _ => None,
        })
        .unwrap();
    assert_eq!(footage.media.absolute_path, file.to_string_lossy());

    // The precomp: its own comp, its rectangle moved by its group and painted
    // half see-through red.
    let LayerKind::Precomp { comp: inner } = pre.kind else {
        panic!("a precomp layer");
    };
    let inner = imported
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.id == inner => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (inner.name.as_str(), inner.width, inner.height),
        ("Inner", 200, 100)
    );
    let LayerKind::Shape { contents } = &inner.layers[0].kind else {
        panic!("a shape layer");
    };
    let Some(ShapeEntry::Item(item)) = contents.first() else {
        panic!("one item");
    };
    let prim = item.primitive.as_ref().unwrap();
    assert_eq!(prim.position_x.animation, Animation::Static(10.0));
    assert_eq!(prim.width.animation, Animation::Static(20.0));
    assert!((item.opacity.value_at(0.0) - 50.0).abs() < 1e-9);
    let fill = item.fill.as_ref().unwrap();
    assert!((fill[0].value_at(0.0) - 1.0).abs() < 1e-6 && fill[1].value_at(0.0).abs() < 1e-6);

    // The folder named after the file holds the precomp and the picture.
    let folder = imported
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Folder(f) => Some(f),
            _ => None,
        })
        .unwrap();
    assert_eq!(folder.name, "logo");
    assert!(folder.children.contains(&inner.id) && folder.children.contains(&footage.id));
    assert!(imported.report.is_empty(), "{:?}", imported.report);
}

/// A few thousand paths under a few thousand paints in one group would make
/// millions of items, so the import stops at its budget instead.
#[test]
fn paths_times_paints_stop_at_the_budget() {
    let path = json!({"ty": "sh", "ks": {"a": 0, "k": {"c": true, "v": [[0, 0], [1, 1]],
        "i": [[0, 0], [0, 0]], "o": [[0, 0], [0, 0]]}}});
    let fill = json!({"ty": "fl", "c": {"a": 0, "k": [1, 1, 1, 1]}, "o": {"a": 0, "k": 100}});
    let mut it = vec![path; 3000];
    it.extend(vec![fill; 3000]);
    let layer = json!({"ty": 4, "ind": 1, "ip": 0, "op": 30, "st": 0, "ks": {},
        "shapes": [{"ty": "gr", "it": it}]});
    let file = json!({"v": "5.7.4", "fr": 30, "ip": 0, "op": 30, "w": 100, "h": 100,
        "layers": [layer]});
    let result = import(file.to_string().as_bytes(), Path::new("big.json"));
    assert!(
        matches!(result, Err(crate::LottieError::TooLarge(_))),
        "{result:?}"
    );
}

//! A Model layer's rows through the walk: only what changed is kept and it
//! finds its part again, a pose comes from the moment alone, a file that holds
//! no take keeps one name for every frame, and a keyed camera comes out of a file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::edits::{new_comp_ops, CompSettings};
use lumit_core::model::{Document, EffectValue, FootageItem, LayerKind, MediaRef, ProjectItem};
use lumit_core::model_layer::Take;
use lumit_core::ops::{apply, Op};
use lumit_render::{model, model_extract, model_motion};
use std::sync::Arc;
use uuid::Uuid;

/// A triangle under a parent that a take slides a metre along x in a second,
/// and a camera a metre back of it that slides with it.
fn write_rig(dir: &std::path::Path) -> std::path::PathBuf {
    let mut bytes: Vec<u8> = Vec::new();
    let floats: [f32; 9] = [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0];
    for f in floats {
        bytes.extend(f.to_le_bytes());
    }
    for f in [0.0f32, 1.0] {
        bytes.extend(f.to_le_bytes());
    }
    for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0] {
        bytes.extend(f.to_le_bytes());
    }
    std::fs::write(dir.join("rig.bin"), &bytes).unwrap();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"name":"Base","children":[1,2]}},{{"name":"Tri","mesh":0}},
            {{"name":"Lens","camera":0,"translation":[0,0,1]}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],
        "cameras":[{{"type":"perspective","perspective":{{"yfov":0.8,"znear":0.1}}}}],
        "animations":[{{"name":"Slide","samplers":[{{"input":1,"output":2}}],
            "channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}}],
        "accessors":[
            {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
              "min":[-1,-1,0],"max":[1,1,0]}},
            {{"bufferView":1,"componentType":5126,"count":2,"type":"SCALAR",
              "min":[0],"max":[1]}},
            {{"bufferView":2,"componentType":5126,"count":2,"type":"VEC3"}}],
        "bufferViews":[{{"buffer":0,"byteLength":36}},
            {{"buffer":0,"byteOffset":36,"byteLength":8}},
            {{"buffer":0,"byteOffset":44,"byteLength":24}}],
        "buffers":[{{"byteLength":{},"uri":"rig.bin"}}]}}"#,
        bytes.len()
    );
    let path = dir.join("rig.gltf");
    std::fs::write(&path, json).unwrap();
    path
}

fn project(path: &std::path::Path) -> (Arc<Document>, Uuid, Uuid, Uuid) {
    let mut doc = Document::new();
    let settings = CompSettings {
        width: 200,
        height: 100,
        ..CompSettings::default()
    };
    let (comp, ops) = new_comp_ops(&doc, settings, None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    let item = FootageItem {
        id: Uuid::now_v7(),
        name: "rig.gltf".into(),
        media: MediaRef {
            relative_path: String::new(),
            absolute_path: path.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: None,
        sequence: None,
        extra: serde_json::Map::new(),
        source_layer: None,
        interpretation: Default::default(),
    };
    doc.items.push(ProjectItem::Footage(item.clone()));
    let layer = lumit_core::model_layer::model_layer(doc.comp(comp).unwrap(), &item);
    let id = layer.id;
    apply(
        &mut doc,
        &Op::AddLayer {
            comp,
            index: 0,
            layer: Box::new(layer),
        },
    )
    .unwrap();
    (Arc::new(doc), comp, id, item.id)
}

/// The scene, read beside the frame as a panel would start it.
fn read_scene(doc: &Document, item: Uuid) -> Arc<lumit_scene::ModelScene> {
    for _ in 0..500 {
        if let Some(Some(scene)) = model::scene_if_read(doc, item) {
            return scene;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the file was never read");
}

#[test]
fn rows_keep_only_what_changed_and_the_take_plays() {
    let dir = std::env::temp_dir().join(format!("lumit-motion-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let (doc, comp, layer_id, item) = project(&write_rig(&dir));
    let scene = read_scene(&doc, item);
    let c = doc.comp(comp).unwrap();
    let layer = c.layers.iter().find(|l| l.id == layer_id).unwrap();
    let LayerKind::Model { settings, .. } = &layer.kind else {
        panic!("a Model layer");
    };
    assert_eq!(
        settings.take,
        Take::First,
        "a new layer plays its first take"
    );

    // Saved as nothing changed: no rows at all.
    let mut list = model_motion::all_rows(layer, settings, &scene);
    assert_eq!(
        list.len(),
        1 + scene.nodes.len(),
        "playback, then a part a node"
    );
    let kept = model_motion::with_rows(settings, &scene, &list);
    assert_eq!((kept.playback.as_ref(), kept.parts.len()), (None, 0));
    assert_eq!(kept.take, Take::First);

    // One part moved is one part saved, named by its path, and survives a file
    // that has lost the part and one that has it back.
    let tri = scene.nodes.iter().position(|n| n.name == "Tri").unwrap();
    let rows = &mut list[1 + tri];
    let id = rows.id;
    for p in &mut rows.params {
        if p.id == "position_y" {
            p.value = EffectValue::Float(Property::fixed(25.0));
        }
    }
    let kept = model_motion::with_rows(settings, &scene, &list);
    assert_eq!(kept.parts.len(), 1);
    assert_eq!(kept.parts[0].custom_name.as_deref(), Some("Base/Tri"));
    let json = serde_json::to_string(&kept).unwrap();
    let opened: lumit_core::model_layer::ModelSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(opened, kept);
    let mut lost = (*scene).clone();
    lost.nodes[tri].path = "Elsewhere".into();
    let again = model_motion::with_rows(&kept, &lost, &model_motion::all_rows(layer, &kept, &lost));
    assert_eq!(again.parts.len(), 1, "a part the file lost keeps its rows");
    assert_eq!(model_motion::all_rows(layer, &kept, &scene)[1 + tri].id, id);

    // Picking a take by the Animation row keeps it by name.
    let mut list = model_motion::all_rows(layer, settings, &scene);
    for p in &mut list[0].params {
        if p.id == "take" {
            p.value = EffectValue::Choice(0);
        }
    }
    assert_eq!(
        model_motion::with_rows(settings, &scene, &list).take,
        Take::Rest
    );

    // A take the file has lost stays named, and reads past the file's own.
    let lost = lumit_core::model_layer::ModelSettings {
        take: Take::Named("Gone".into()),
        ..(**settings).clone()
    };
    let rows = model_motion::all_rows(layer, &lost, &scene);
    assert!(matches!(
        rows[0].param("take"),
        Some(EffectValue::Choice(2))
    ));
    assert_eq!(
        model_motion::with_rows(&lost, &scene, &rows).take,
        lost.take
    );

    // The same moment comes out the same whatever came before, and a moment
    // past the take goes round.
    let context = lumit_core::model::comp_context(&doc, c, 0.25);
    let at = |t: f64| model_motion::pose_at(c, layer, settings, &scene, t, &context).unwrap();
    let first = at(0.25);
    let _ = at(0.75);
    assert_eq!(first, at(0.25));
    assert_eq!(first.worlds[tri], at(1.25).worlds[tri]);
    assert!((first.worlds[tri].w_axis.x - 0.25).abs() < 1e-4);

    // The rows time it: Offset moves it along, Mirror comes back past the end,
    // and Retime reads the take as it reads footage.
    let timed = |offset: f64, looping: u32, retime: Option<Property>, t: f64| {
        let mut rows =
            lumit_core::fx::builtins::instantiate(lumit_core::model_part::PLAYBACK_NAME).unwrap();
        for p in &mut rows.params {
            match p.id.as_str() {
                "offset" => p.value = EffectValue::Float(Property::fixed(offset)),
                "looping" => p.value = EffectValue::Choice(looping),
                _ => {}
            }
        }
        let s = lumit_core::model_layer::ModelSettings {
            playback: Some(rows),
            ..(**settings).clone()
        };
        let mut l = layer.clone();
        l.retime = retime;
        let pose = model_motion::pose_at(c, &l, &s, &scene, t, &context).unwrap();
        pose.worlds[tri].w_axis.x
    };
    assert!((timed(0.5, 1, None, 0.25) - 0.75).abs() < 1e-4, "Offset");
    assert!((timed(0.0, 2, None, 1.25) - 0.75).abs() < 1e-4, "Mirror");
    assert!((timed(0.0, 3, None, 5.0) - 1.0).abs() < 1e-4, "Freeze");
    // Half speed: the second second of the layer shows the first of the take.
    let mut half = Property::zero();
    let key = |secs: i64, value: f64| lumit_core::anim::Keyframe {
        time: lumit_core::time::Rational::new(secs, 1).unwrap(),
        value,
        interp_in: lumit_core::anim::SideInterp::Linear,
        interp_out: lumit_core::anim::SideInterp::Linear,
    };
    half.set_animation(lumit_core::anim::Animation::Keyframed(vec![
        key(0, 0.0),
        key(2, 1.0),
    ]));
    assert!(
        (timed(0.0, 1, Some(half), 0.5) - 0.25).abs() < 1e-4,
        "Retime"
    );

    // A part turned on three axes turns in the order the layer's rows do.
    let mut rows = model_motion::all_rows(layer, settings, &scene);
    for p in &mut rows[1 + tri].params {
        let v = match p.id.as_str() {
            "rotation_x" => 20.0,
            "rotation_y" => 35.0,
            "rotation_z" => 50.0,
            _ => continue,
        };
        p.value = EffectValue::Float(Property::fixed(v));
    }
    let s = model_motion::with_rows(settings, &scene, &rows);
    let s = lumit_core::model_layer::ModelSettings {
        take: Take::Rest,
        ..s
    };
    let turned = model_motion::pose_at(c, layer, &s, &scene, 0.0, &context).unwrap();
    let to_layer = glam::Mat4::from_cols_array_2d(&model::model_to_comp(
        &s,
        &scene.bounds,
        (c.width as f32, c.height as f32),
    ));
    let in_layer = glam::Mat3::from_mat4(to_layer * turned.worlds[tri] * to_layer.inverse());
    let want = lumit_core::camera::rotation((20.0, 35.0, 50.0));
    for (r, row) in want.iter().enumerate() {
        for (k, &v) in row.iter().enumerate() {
            let got = in_layer.col(k)[r];
            assert!(
                (f64::from(got) - v).abs() < 1e-4,
                "row {r} column {k}: {got} against {v}"
            );
        }
    }

    // The camera comes out keyed and following the layer, with nothing else.
    let made = model_extract::layers_from_model(&doc, c, layer, ("Camera", "Light"));
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].name, "Lens");
    assert!(made[0].transform.position_x.is_animated());
    assert!(!made[0].transform.position_y.is_animated());
    let LayerKind::Camera { zoom, options, .. } = &made[0].kind else {
        panic!("a Camera layer");
    };
    assert!(!options.two_node);
    assert!((zoom.value_at(0.0) - 50.0 / (0.4f64).tan()).abs() < 1e-3);
    std::fs::remove_dir_all(&dir).ok();
}

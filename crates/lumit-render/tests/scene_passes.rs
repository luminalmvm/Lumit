//! A 3D precomp's Part id pass reads back as each part's own number, whatever
//! the frame, and a Part matte keeps the parts it names with soft edges. A
//! saved matte is only that number, so a number that moved would cut it loose.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::edits::{new_comp_ops, CompSettings};
use lumit_core::model::{
    ColourDepth, Document, EffectInstance, EffectValue, FootageItem, Layer, LayerKind,
    LinearColour, MediaRef, ProjectItem, SolidDef,
};
use lumit_core::ops::{apply, Op};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 128;

/// One white triangle drawn twice, by two named nodes under a third.
fn model(dir: &std::path::Path) -> std::path::PathBuf {
    let floats: [f32; 9] = [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0];
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    std::fs::write(dir.join("two.bin"), bytes).unwrap();
    let json = r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],
        "nodes":[{"name":"Body","children":[1,2]},
            {"name":"Left","mesh":0,"translation":[-1.1,0,0]},
            {"name":"Right","mesh":0,"translation":[1.1,0,0]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[-1,-1,0],"max":[1,1,0]}],
        "bufferViews":[{"buffer":0,"byteLength":36}],
        "buffers":[{"byteLength":36,"uri":"two.bin"}]}"#;
    let path = dir.join("two.gltf");
    std::fs::write(&path, json).unwrap();
    path
}

/// A comp holding the Model layer, and the Precomp layer of it in a parent.
fn project(path: &std::path::Path) -> (Document, Uuid, Layer) {
    let mut doc = Document::new();
    let settings = CompSettings {
        width: SIZE,
        height: SIZE,
        ..CompSettings::default()
    };
    let (inner, ops) = new_comp_ops(&doc, settings.clone(), None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    let item = FootageItem {
        id: Uuid::now_v7(),
        name: "two.gltf".into(),
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
    let model = lumit_core::model_layer::model_layer(doc.comp(inner).unwrap(), &item);
    doc.items.push(ProjectItem::Footage(item));
    apply(
        &mut doc,
        &Op::AddLayer {
            comp: inner,
            index: 0,
            layer: Box::new(model),
        },
    )
    .unwrap();
    let (parent, ops) = new_comp_ops(&doc, settings, None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    let placed = lumit_core::edits::base_layer(
        "placed".into(),
        LayerKind::Precomp { comp: inner },
        doc.comp(inner).unwrap().duration.0,
        lumit_core::edits::centred_transform(f64::from(SIZE), f64::from(SIZE), SIZE, SIZE),
    );
    (doc, parent, placed)
}

fn effect(name: &str, source: Uuid, set: &[(&str, EffectValue)]) -> EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate(name).unwrap();
    for p in &mut e.params {
        if p.id == "source" {
            p.value = EffectValue::Layer(Some(source));
        }
        if let Some((_, value)) = set.iter().find(|(id, _)| *id == p.id) {
            p.value = value.clone();
        }
    }
    e
}

/// The parent as `effects` leave the Precomp layer, frames `frame`.
fn drawn(
    r: &mut HeadlessRenderer,
    (doc, parent, placed): &(Document, Uuid, Layer),
    effects: Vec<EffectInstance>,
    frame: u64,
) -> Vec<u8> {
    let mut doc = doc.clone();
    let mut layer = placed.clone();
    layer.effects = effects;
    apply(
        &mut doc,
        &Op::AddLayer {
            comp: *parent,
            index: 0,
            layer: Box::new(layer),
        },
    )
    .unwrap();
    r.render_rgba(&Arc::new(doc), *parent, frame, 1.0)
        .expect("the render")
        .0
}

fn number(px: &[u8]) -> u32 {
    u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2])
}

#[test]
fn part_ids_read_back_and_a_matte_keeps_its_part() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let project = project(&model(dir.path()));
    let own = project.2.id;
    let parts = lumit_render::passes::scene_parts(&project.0, &project.2).unwrap();
    let id_of = |name: &str| parts.iter().find(|p| p.1 == name).expect("named").0;
    let (left, right) = (id_of("Left"), id_of("Right"));

    let ids = effect("scene_pass", own, &[("pass", EffectValue::Choice(2))]);
    let picture = drawn(&mut r, &project, vec![ids.clone()], 0);
    let at = |id: u32| {
        picture
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, px)| number(px) == id)
            .map(|(k, _)| k)
            .collect::<Vec<_>>()
    };
    let (on_left, on_right) = (at(left), at(right));
    assert!(on_left.len() > 100, "{} pixels name Left", on_left.len());
    assert!(on_right.len() > 100, "{} pixels name Right", on_right.len());
    let later = drawn(&mut r, &project, vec![ids], 7);
    assert!(later == picture, "the same parts, the same pixels");

    let plain = drawn(&mut r, &project, Vec::new(), 0);
    let matte = |r: &mut HeadlessRenderer, parts: &str| {
        let text = EffectValue::Text(parts.to_owned());
        let e = effect("part_matte", own, &[("parts", text)]);
        drawn(r, &project, vec![e], 0)
    };
    assert!(matte(&mut r, "") == plain, "no parts leaves the picture be");
    assert!(
        matte(&mut r, "1") == plain,
        "lost parts leave the picture be"
    );
    let kept = matte(&mut r, &left.to_string());
    let whole = on_left.iter().filter(|&&k| kept[k * 4] == plain[k * 4]);
    assert!(
        whole.count() * 10 > on_left.len() * 8,
        "Left stays, all but its edge"
    );
    assert!(on_right.iter().all(|&k| kept[k * 4] == 0));
    let soft = kept.chunks_exact(4).filter(|px| (32..200).contains(&px[0]));
    assert!(soft.count() > 4, "its edges take the anti-aliasing");

    // At the other colour depths too.
    for depth in [ColourDepth::Eight, ColourDepth::ThirtyTwo] {
        let mut deep = (project.0.clone(), project.1, project.2.clone());
        deep.0.colour_depth = depth;
        let text = EffectValue::Text(left.to_string());
        let e = effect("part_matte", own, &[("parts", text)]);
        let kept = drawn(&mut r, &deep, vec![e], 0);
        let shows = on_left.iter().filter(|&&k| kept[k * 4] > 16);
        assert!(
            shows.count() * 10 > on_left.len() * 8,
            "{depth:?} keeps Left"
        );
        assert!(on_right.iter().all(|&k| kept[k * 4] == 0), "{depth:?}");
    }
}

/// A designer tries an effect on the Model layer first, so a Part matte has to
/// work there, with no precomp, and a duplicate comp keeps what it picked.
#[test]
fn a_model_layer_hands_over_its_own_parts() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (mut doc, _, placed) = project(&model(dir.path()));
    let LayerKind::Precomp { comp: scene } = placed.kind else {
        panic!("a Precomp layer");
    };
    let mut layer = doc.comp(scene).unwrap().layers[0].clone();
    let parts = lumit_render::passes::scene_parts(&doc, &layer).unwrap();
    let left = parts.iter().find(|p| p.1 == "Left").expect("named").0;
    let right = parts.iter().find(|p| p.1 == "Right").expect("named").0;
    let (own, picked_left) = (layer.id, left);

    let ids = effect("scene_pass", own, &[("pass", EffectValue::Choice(2))]);
    layer.effects = vec![ids];
    let mut seen = doc.clone();
    seen.comp_mut(scene).unwrap().layers[0] = layer.clone();
    let px = r.render_rgba(&Arc::new(seen), scene, 0, 1.0).unwrap().0;
    let count = |id: u32| px.chunks_exact(4).filter(|p| number(p) == id).count();
    assert!(
        count(left) > 100 && count(right) > 100,
        "both parts are named"
    );

    let text = EffectValue::Text(left.to_string());
    layer.effects = vec![effect("part_matte", own, &[("parts", text)])];
    doc.comp_mut(scene).unwrap().layers[0] = layer;
    let on_right: Vec<usize> = px
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| number(p) == right)
        .map(|(k, _)| k)
        .collect();
    let kept = r
        .render_rgba(&Arc::new(doc.clone()), scene, 0, 1.0)
        .unwrap()
        .0;
    assert!(kept.chunks_exact(4).any(|p| p[0] > 16), "Left shows");
    assert!(on_right.iter().all(|&k| kept[k * 4] == 0), "Right doesn't");

    let mut copy = vec![doc.comp(scene).unwrap().clone()];
    lumit_render::passes::fresh_ids_keeping_parts(&doc, &mut copy).unwrap();
    let new = &copy[0].layers[0];
    assert_ne!(new.id, own);
    let parts = lumit_render::passes::scene_parts(&doc, new).unwrap();
    let left = parts.iter().find(|p| p.1 == "Left").expect("named").0;
    let picked = new.effects[0].text("parts").unwrap();
    assert_eq!(picked, left.to_string(), "the copy's matte keeps its part");

    // Split: past the cut the first half has ended, so the second half's row
    // has to read itself, or its matte goes and its grade floods the model.
    let whole = doc.comp(scene).unwrap().layers[0].clone();
    let frames = doc.comp(scene).unwrap().frame_rate;
    let cut = frames.time_of_frame(10).unwrap();
    let (mut head, mut tail) = (whole.clone(), whole.clone());
    head.out_point = cut;
    tail.id = Uuid::now_v7();
    tail.in_point = cut;
    lumit_render::passes::follow_copy(&doc, &whole, &mut tail);
    assert_eq!(tail.effects[0].layer_ref("source"), Some(tail.id));
    let mut split = doc.clone();
    split.comp_mut(scene).unwrap().layers = vec![tail, head];
    let later = r.render_rgba(&Arc::new(split), scene, 20, 1.0).unwrap().0;
    assert!(
        later.chunks_exact(4).any(|p| p[0] > 16),
        "Left shows past the cut"
    );
    assert!(on_right.iter().all(|&k| later[k * 4] == 0), "Right doesn't");

    // A hidden Model layer is read all the same, as a Solid's matte here.
    let def = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Solid".into(),
        colour: LinearColour([1.0; 4]),
        width: SIZE,
        height: SIZE,
        extra: serde_json::Map::new(),
    }));
    let mut hidden = doc.comp(scene).unwrap().layers[0].clone();
    hidden.effects.clear();
    hidden.switches.visible = false;
    let mut solid = lumit_core::edits::base_layer(
        "Solid".into(),
        LayerKind::Solid { def },
        doc.comp(scene).unwrap().duration.0,
        lumit_core::edits::centred_transform(f64::from(SIZE), f64::from(SIZE), SIZE, SIZE),
    );
    let text = EffectValue::Text(picked_left.to_string());
    solid.effects = vec![effect("part_matte", own, &[("parts", text)])];
    hidden.id = own;
    doc.comp_mut(scene).unwrap().layers = vec![solid, hidden];
    let cut_out = r.render_rgba(&Arc::new(doc), scene, 0, 1.0).unwrap().0;
    assert!(cut_out.chunks_exact(4).any(|p| p[0] > 16), "Left is kept");
    assert!(
        on_right.iter().all(|&k| cut_out[k * 4] == 0),
        "Right is cut"
    );
}

/// A 2D layer cuts the 3D layers into groups, and the higher one covers the
/// lower in the picture, so it has to in a pass.
#[test]
fn a_group_covers_the_groups_below_it_in_a_pass() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (mut doc, parent, placed) = project(&model(dir.path()));
    let LayerKind::Precomp { comp: scene } = placed.kind else {
        panic!("a Precomp layer");
    };
    let def = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Solid".into(),
        colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
        width: SIZE,
        height: SIZE,
        extra: serde_json::Map::new(),
    }));
    let solid = |name: &str, three_d: bool, z: f64| {
        let mut l = lumit_core::edits::base_layer(
            name.into(),
            LayerKind::Solid { def },
            doc.comp(scene).unwrap().duration.0,
            lumit_core::edits::centred_transform(f64::from(SIZE), f64::from(SIZE), SIZE, SIZE),
        );
        l.switches.three_d = three_d;
        l.transform.position_z = lumit_core::anim::Property::fixed(z);
        l
    };
    // Far behind the model, and above it in the stack past a 2D layer.
    let (behind, between) = (solid("Behind", true, 2000.0), solid("Between", false, 0.0));
    let number_of_behind = lumit_scene::parts::layer_number(behind.id.as_bytes());
    let comp = doc.comp_mut(scene).unwrap();
    comp.layers.insert(0, between);
    comp.layers.insert(0, behind);
    let ids = effect("scene_pass", placed.id, &[("pass", EffectValue::Choice(2))]);
    let px = drawn(&mut r, &(doc, parent, placed), vec![ids], 0);
    let middle = &px[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize..][..4];
    assert_eq!(number(middle), number_of_behind);
}

/// A triangle on a joint a take lifts, and another on a node of its own.
fn skinned(dir: &std::path::Path) -> std::path::PathBuf {
    let mut bytes: Vec<u8> = Vec::new();
    for f in [-1.0f32, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0] {
        bytes.extend(f.to_le_bytes());
    }
    bytes.extend([0u8; 12]);
    for _ in 0..3 {
        for f in [1.0f32, 0.0, 0.0, 0.0] {
            bytes.extend(f.to_le_bytes());
        }
    }
    for f in [0.0f32, 1.0, -1.2, 0.0, 0.0, -1.2, 1.0, 0.0] {
        bytes.extend(f.to_le_bytes());
    }
    let length = bytes.len();
    std::fs::write(dir.join("skinned.bin"), bytes).unwrap();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"name":"Rig","children":[1,2,3]}},
            {{"name":"Skin","mesh":0,"skin":0}},
            {{"name":"Joint","translation":[-1.2,0,0]}},
            {{"name":"Prop","mesh":1,"translation":[1.2,0,0]}}],
        "skins":[{{"joints":[2]}}],
        "meshes":[
            {{"primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}}}}]}},
            {{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],
        "animations":[{{"name":"Lift","samplers":[{{"input":3,"output":4}}],
            "channels":[{{"sampler":0,"target":{{"node":2,"path":"translation"}}}}]}}],
        "accessors":[
            {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
                "min":[-1,-1,0],"max":[1,1,0]}},
            {{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"}},
            {{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}},
            {{"bufferView":3,"componentType":5126,"count":2,"type":"SCALAR",
                "min":[0],"max":[1]}},
            {{"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"}}],
        "bufferViews":[
            {{"buffer":0,"byteOffset":0,"byteLength":36}},
            {{"buffer":0,"byteOffset":36,"byteLength":12}},
            {{"buffer":0,"byteOffset":48,"byteLength":48}},
            {{"buffer":0,"byteOffset":96,"byteLength":8}},
            {{"buffer":0,"byteOffset":104,"byteLength":24}}],
        "buffers":[{{"byteLength":{length},"uri":"skinned.bin"}}]}}"#
    );
    let path = dir.join("skinned.gltf");
    std::fs::write(&path, json).unwrap();
    path
}

/// The pixels naming `id`.
fn named(px: &[u8], id: u32) -> Vec<usize> {
    px.chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| number(p) == id)
        .map(|(k, _)| k)
        .collect()
}

/// How far down the picture they are, on average.
fn height(pixels: &[usize]) -> f64 {
    let rows = pixels.iter().map(|&k| (k / SIZE as usize) as f64);
    rows.sum::<f64>() / pixels.len().max(1) as f64
}

/// A pass and a matte stay on a part while its take moves it and while its
/// rows do, and a hidden part draws nothing. Without this a playing model's
/// matte is wrong and nothing says so.
#[test]
fn a_pass_follows_a_take_and_a_parts_rows() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut project = project(&skinned(dir.path()));
    let own = project.2.id;
    let parts = lumit_render::passes::scene_parts(&project.0, &project.2).unwrap();
    let id_of = |name: &str| parts.iter().find(|p| p.1 == name).expect("named").0;
    let (skin, prop) = (id_of("Skin"), id_of("Prop"));
    let ids = || effect("scene_pass", own, &[("pass", EffectValue::Choice(2))]);
    let matte = |id: u32| {
        let text = EffectValue::Text(id.to_string());
        effect("part_matte", own, &[("parts", text)])
    };

    // The take lifts the skin and leaves the other part where it was.
    let first = drawn(&mut r, &project, vec![ids()], 0);
    let later = drawn(&mut r, &project, vec![ids()], 12);
    let (skin_first, skin_later) = (named(&first, skin), named(&later, skin));
    assert!(skin_first.len() > 100 && skin_later.len() > 100);
    assert!(
        height(&skin_first) - height(&skin_later) > 3.0,
        "the skin rises, from {} to {}",
        height(&skin_first),
        height(&skin_later)
    );
    assert_eq!(named(&first, prop), named(&later, prop));

    // Where the picture shows the skin at that frame, the pass says so.
    let plain = drawn(&mut r, &project, Vec::new(), 12);
    let lit = |k: &&usize| plain[**k * 4] > 64;
    let on = skin_later.iter().filter(lit).count();
    assert!(
        on * 10 >= skin_later.len() * 9,
        "{on} of {}",
        skin_later.len()
    );
    let drawn_px = plain.chunks_exact(4).filter(|p| p[0] > 200).count();
    let prop_later = named(&later, prop);
    assert!(
        (drawn_px as i64 - (skin_later.len() + prop_later.len()) as i64).abs() * 10
            < drawn_px as i64,
        "the pass covers what the picture does: {drawn_px} against {} and {}",
        skin_later.len(),
        prop_later.len()
    );

    // The matte goes with it.
    let kept = drawn(&mut r, &project, vec![matte(skin)], 12);
    assert!(prop_later.iter().all(|&k| kept[k * 4] == 0));
    let shown = skin_later.iter().filter(|&&k| kept[k * 4] > 64).count();
    assert!(shown * 10 >= skin_later.len() * 8, "{shown}");

    // A part's rows move it in the pass, and hide it.
    let set_rows = |doc: &mut Document, row: &str, value: EffectValue| {
        let LayerKind::Precomp { comp: inner } = project.2.kind else {
            panic!("a Precomp layer");
        };
        let layer = doc.comp(inner).unwrap().layers[0].clone();
        let LayerKind::Model { item, settings } = &layer.kind else {
            panic!("a Model layer");
        };
        let scene = lumit_render::model::scene_if_read(doc, *item)
            .flatten()
            .expect("the file was read");
        let k = scene.nodes.iter().position(|n| n.name == "Prop").unwrap();
        let mut rows = lumit_render::model_motion::all_rows(&layer, settings, &scene);
        for p in rows[1 + k].params.iter_mut().filter(|p| p.id == row) {
            p.value = value.clone();
        }
        let kept = lumit_render::model_motion::with_rows(settings, &scene, &rows);
        let inner = doc.comp_mut(inner).unwrap();
        if let LayerKind::Model { settings, .. } = &mut inner.layers[0].kind {
            **settings = kept;
        }
    };
    let rest = height(&prop_later);
    let distance = EffectValue::Float(lumit_core::anim::Property::fixed(40.0));
    set_rows(&mut project.0, "position_y", distance);
    let moved = drawn(&mut r, &project, vec![ids()], 12);
    let moved = named(&moved, prop);
    assert!(moved.len() > 100);
    assert!(
        (height(&moved) - rest).abs() > 5.0,
        "the part moves, from {rest} to {}",
        height(&moved)
    );

    let hide = EffectValue::Float(lumit_core::anim::Property::fixed(0.0));
    set_rows(&mut project.0, "visible", hide);
    let hidden = drawn(&mut r, &project, vec![ids()], 12);
    assert!(
        named(&hidden, prop).is_empty(),
        "a hidden part draws nothing"
    );
    assert_eq!(named(&hidden, skin).len(), skin_later.len());
    // Kept while hidden is an empty matte, not lost picks.
    let gone = drawn(&mut r, &project, vec![matte(prop)], 12);
    assert!(gone.chunks_exact(4).all(|p| p[0] == 0), "nothing is left");
}

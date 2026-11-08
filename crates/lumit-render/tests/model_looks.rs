//! A 3D view's look for models never reaches a picture named for the look
//! models export with. A precomp holding a model is drawn shaded in a Top
//! view, then as it exports, on one renderer, and the second must be the
//! picture a clean render makes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::edits::{new_comp_ops, CompSettings};
use lumit_core::model::{Document, FootageItem, LayerKind, MediaRef, ProjectItem};
use lumit_core::ops::{apply, Op};
use lumit_gpu::mesh::display::{ModelDisplay, Shading};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

/// A white triangle facing the camera, as a glTF beside its buffer.
fn triangle(dir: &std::path::Path) -> std::path::PathBuf {
    let floats: [f32; 9] = [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0];
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    std::fs::write(dir.join("triangle.bin"), bytes).unwrap();
    let json = r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[-1,-1,0],"max":[1,1,0]}],
        "bufferViews":[{"buffer":0,"byteLength":36}],
        "buffers":[{"byteLength":36,"uri":"triangle.bin"}]}"#;
    let path = dir.join("triangle.gltf");
    std::fs::write(&path, json).unwrap();
    path
}

/// A comp holding the triangle's Model layer, placed in a parent comp as a
/// Precomp layer. Answers the document and the parent.
fn project(path: &std::path::Path) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let settings = CompSettings {
        width: 64,
        height: 64,
        ..CompSettings::default()
    };
    let (inner, ops) = new_comp_ops(&doc, settings.clone(), None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    let item = FootageItem {
        id: Uuid::now_v7(),
        name: "triangle.gltf".into(),
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
    let add = |doc: &mut Document, comp, layer| {
        apply(
            doc,
            &Op::AddLayer {
                comp,
                index: 0,
                layer: Box::new(layer),
            },
        )
        .unwrap();
    };
    add(&mut doc, inner, model);
    let (parent, ops) = new_comp_ops(&doc, settings, None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    let duration = doc.comp(inner).unwrap().duration.0;
    let placed = lumit_core::edits::base_layer(
        "placed".into(),
        LayerKind::Precomp { comp: inner },
        duration,
        lumit_core::edits::centred_transform(64.0, 64.0, 64, 64),
    );
    add(&mut doc, parent, placed);
    (Arc::new(doc), parent)
}

#[test]
fn a_shaded_precomp_never_reaches_a_final_frame() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (doc, parent) = project(&triangle(dir.path()));
    let mut shown = |shading| {
        lumit_render::model::show_models(ModelDisplay {
            shading,
            faces: false,
        });
        r.render_rgba(&doc, parent, 0, 1.0).expect("the render").0
    };
    let clean = shown(Shading::Final);
    let shaded = shown(Shading::Shaded);
    let after = shown(Shading::Final);
    lumit_render::model::show_models(ModelDisplay::default());
    assert!(clean.iter().any(|&v| v > 8), "the triangle draws");
    assert!(shaded != clean, "a Shaded view draws its own picture");
    assert!(after == clean, "Final after Shaded is a clean Final");
}

/// A model in a collapsed precomp is lit by the parent's lights, exactly as if
/// it sat in the parent.
#[test]
fn a_collapsed_model_takes_its_parents_light() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (doc, parent) = project(&triangle(dir.path()));
    let mut doc = (*doc).clone();
    let LayerKind::Precomp { comp } = doc.comp(parent).unwrap().layers[0].kind else {
        panic!("project() places a precomp first");
    };
    let inner = doc.comp(comp).unwrap().clone();
    let mut tf = lumit_core::edits::centred_transform(64.0, 64.0, 64, 64);
    tf.position_z = lumit_core::anim::Property::fixed(-40.0);
    let light = lumit_core::edits::base_layer(
        "light".into(),
        LayerKind::Light {
            light: Box::new(lumit_core::model::LightDef::default()),
        },
        inner.duration.0,
        tf,
    );
    let place = doc.comp_mut(parent).unwrap();
    place.layers[0].switches.three_d = true;
    place.layers[0].switches.collapse = true;
    place.layers.insert(0, light);
    let collapsed = r
        .render_rgba(&Arc::new(doc.clone()), parent, 0, 1.0)
        .unwrap()
        .0;
    let place = doc.comp_mut(parent).unwrap();
    place.layers[1] = inner.layers[0].clone();
    let direct = r.render_rgba(&Arc::new(doc), parent, 0, 1.0).unwrap().0;
    assert!(direct.iter().any(|&v| v > 8), "the light reaches the model");
    assert!(collapsed == direct);
}

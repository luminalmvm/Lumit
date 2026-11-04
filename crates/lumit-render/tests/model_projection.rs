//! A picture projected on a model from a camera. From that camera it has to be
//! the picture itself, whatever the model's shape, and a camera moved off it
//! has to find it still on the surfaces it landed on, with Base colour on the
//! ones the first camera couldn't see.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::edits::{new_comp_ops, CompSettings};
use lumit_core::model::*;
use lumit_core::ops::{apply, Op};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 128;

/// A large card behind a small one, as a glTF beside its buffer. Each is two
/// triangles facing the comp's default camera.
fn cards(dir: &std::path::Path) -> std::path::PathBuf {
    let quad = |half: f32, z: f32| {
        [
            [-half, -half, z],
            [half, -half, z],
            [half, half, z],
            [-half, -half, z],
            [half, half, z],
            [-half, half, z],
        ]
    };
    let floats: Vec<f32> = [quad(9.0, -1.0), quad(0.2, 1.0)]
        .iter()
        .flatten()
        .flatten()
        .copied()
        .collect();
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    std::fs::write(dir.join("cards.bin"), &bytes).unwrap();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],
        "nodes":[{{"mesh":0}}],
        "materials":[{{"name":"Card"}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"material":0}}]}}],
        "accessors":[{{"bufferView":0,"componentType":5126,"count":12,"type":"VEC3",
            "min":[-9,-9,-1],"max":[9,9,1]}}],
        "bufferViews":[{{"buffer":0,"byteLength":{n}}}],
        "buffers":[{{"byteLength":{n},"uri":"cards.bin"}}]}}"#,
        n = bytes.len()
    );
    let path = dir.join("cards.gltf");
    std::fs::write(&path, json).unwrap();
    path
}

/// A picture that differs at every pixel, so a slip of one shows.
fn picture(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("plate.png");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(file, SIZE, SIZE);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    let pixels: Vec<u8> = (0..SIZE * SIZE)
        .flat_map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            [40 + x as u8, 40 + y as u8, (60 + (x + y) / 2) as u8]
        })
        .collect();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    path
}

fn footage(doc: &mut Document, path: &std::path::Path) -> FootageItem {
    let item = FootageItem {
        id: Uuid::now_v7(),
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
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
    item
}

fn add(doc: &mut Document, comp: Uuid, layer: Layer) {
    let (index, layer) = (doc.comp(comp).unwrap().layers.len(), Box::new(layer));
    apply(doc, &Op::AddLayer { comp, index, layer }).unwrap();
}

/// A comp holding the plate, on its own and hidden when `shows` is off.
fn with_plate(doc: &mut Document, plate: &FootageItem, shows: bool) -> (Uuid, Uuid) {
    let settings = CompSettings {
        width: SIZE,
        height: SIZE,
        ..CompSettings::default()
    };
    let (comp, ops) = new_comp_ops(doc, settings, None);
    for op in &ops {
        apply(doc, op).unwrap();
    }
    let size = f64::from(SIZE);
    let duration = doc.comp(comp).unwrap().duration.0;
    let t = lumit_core::edits::centred_transform(size, size, SIZE, SIZE);
    let mut layer = lumit_core::edits::base_layer(
        "plate".into(),
        LayerKind::Footage { item: plate.id },
        duration,
        t,
    );
    layer.switches.visible = shows;
    let id = layer.id;
    add(doc, comp, layer);
    (comp, id)
}

/// `plate` on two cards, from a camera that starts straight on and has slid
/// sideways by frame 10, its time held at frame 0 or following the playhead.
fn projected(doc: &mut Document, model: &FootageItem, plate: &FootageItem, held: bool) -> Uuid {
    let (comp, shown) = with_plate(doc, plate, false);
    let c = doc.comp(comp).unwrap().clone();
    let mut camera = lumit_core::edits::fresh_camera(&c, "Camera".into(), None);
    let x = camera.transform.position_x.value_at(0.0);
    let key = |t: i64, value: f64| Keyframe {
        time: c.frame_rate.time_of_frame(t).unwrap().0,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    camera.transform.position_x = Property {
        animation: Animation::Keyframed(vec![key(0, x), key(10, x + 90.0)]),
        ..Property::fixed(x)
    };
    let mut rows = lumit_core::fx::builtins::instantiate(lumit_core::model_material::MATCH_NAME)
        .expect("the material's rows");
    let mut set = |id: &str, v: EffectValue| {
        rows.params.iter_mut().find(|p| p.id == id).unwrap().value = v;
    };
    set("base_colour_layer", EffectValue::Layer(Some(shown)));
    set("camera", EffectValue::Layer(Some(camera.id)));
    set(
        "base_colour",
        EffectValue::Colour([1.0, 0.0, 0.0, 1.0].map(Property::fixed)),
    );
    if held {
        set("projection_time", EffectValue::Choice(1));
    }
    let mut layer = lumit_core::model_layer::model_layer(&c, model);
    if let LayerKind::Model { settings, .. } = &mut layer.kind {
        settings.fit = false;
        settings.materials = vec![Some(rows)];
    }
    add(doc, comp, camera);
    add(doc, comp, layer);
    comp
}

/// The most any colour channel differs where `a` covers, out of 255, leaving
/// out the single pixels where three surfaces meet.
fn off_by(a: &[u8], b: &[u8]) -> u8 {
    let mut each: Vec<u8> = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, _)| p[3] == 255)
        .map(|(p, q)| (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap())
        .collect();
    each.sort_unstable();
    each.get(each.len().saturating_sub(3)).copied().unwrap_or(0)
}

fn reds(px: &[u8]) -> usize {
    px.chunks_exact(4)
        .filter(|p| p[0] > 200 && p[1] < 40 && p[2] < 40)
        .count()
}

#[test]
fn a_projection_is_the_picture_from_its_camera_and_stays_on_the_model_from_others() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::new();
    let plate = footage(&mut doc, &picture(dir.path()));
    let model = footage(&mut doc, &cards(dir.path()));
    let (alone, _) = with_plate(&mut doc, &plate, true);
    let held = projected(&mut doc, &model, &plate, true);
    let follows = projected(&mut doc, &model, &plate, false);
    let doc = Arc::new(doc);
    let mut frame = |comp, f| r.render_rgba(&doc, comp, f, 1.0).expect("the render").0;
    let picture = frame(alone, 0);

    let from_its_camera = frame(held, 0);
    assert!(off_by(&from_its_camera, &picture) <= 2);
    assert_eq!(reds(&from_its_camera), 0, "the camera saw all of this");

    // The camera has moved on, and the picture stayed where it landed: the
    // large card behind the small one was never seen, so it shows Base colour.
    let moved = frame(held, 10);
    assert!(off_by(&moved, &picture) > 8, "the cards have slid apart");
    assert!(
        reds(&moved) > 20,
        "what the camera couldn't see shows Base colour"
    );

    // Following the playhead, the picture is seen from where it is projected.
    let followed = frame(follows, 10);
    assert!(off_by(&followed, &picture) <= 2);
    assert_eq!(reds(&followed), 0);
}

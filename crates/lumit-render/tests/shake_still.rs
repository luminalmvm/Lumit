//! A Shake at Amplitude 0 draws the layer exactly as it draws with no Shake.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::edits::{base_layer, centred_transform, new_comp_ops, CompSettings};
use lumit_core::model::{
    Document, EffectInstance, EffectValue, FootageItem, LayerKind, MediaRef, ProjectItem,
};
use lumit_core::ops::{apply, Op};
use lumit_core::time::{FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

const W: u32 = 64;
const H: u32 = 36;

fn picture(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    let px: Vec<u8> = (0..W * H)
        .flat_map(|i| {
            let (x, y) = (i % W, i / W);
            let check = if (x / 3 + y / 3) % 2 == 0 { 220 } else { 40 };
            [check, (x * 4) as u8, (y * 7) as u8, 128 + (x * 2) as u8]
        })
        .collect();
    let mut png = png.write_header().unwrap();
    png.write_image_data(&px).unwrap();
}

fn scene(file: &Path, shake: Option<EffectInstance>) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let settings = CompSettings {
        width: W,
        height: H,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        background: [0.0; 4],
        ..CompSettings::default()
    };
    let (comp, ops) = new_comp_ops(&doc, settings, None);
    for op in &ops {
        apply(&mut doc, op).unwrap();
    }
    let item = Uuid::now_v7();
    let name = "picture.png".to_owned();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id: item,
        name: name.clone(),
        media: MediaRef {
            relative_path: name,
            absolute_path: file.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
        source_layer: None,
        interpretation: Default::default(),
    }));
    let mut layer = base_layer(
        "Layer".into(),
        LayerKind::Footage { item },
        doc.comp(comp).unwrap().duration.0,
        centred_transform(f64::from(W), f64::from(H), W, H),
    );
    layer.transform.rotation = Property::fixed(17.0);
    layer.effects = shake.into_iter().collect();
    let layer = Box::new(layer);
    apply(
        &mut doc,
        &Op::AddLayer {
            comp,
            index: 0,
            layer,
        },
    )
    .unwrap();
    (Arc::new(doc), comp)
}

fn shake(version: u32, amplitude: Property, rows: &[(&str, f64)]) -> EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate("shake").unwrap();
    e.effect.version = version;
    let rotated = [("rotation", 12.0), ("z_amp", 15.0)];
    for p in &mut e.params {
        if let Some((_, v)) = rotated.iter().chain(rows).find(|(id, _)| p.id == *id) {
            p.value = EffectValue::Float(Property::fixed(*v));
        }
        match p.id.as_str() {
            "amplitude" => p.value = EffectValue::Float(amplitude.clone()),
            "motion_blur" => p.value = EffectValue::Bool(true),
            _ => {}
        }
    }
    e
}

#[test]
fn a_shake_at_amplitude_0_leaves_the_picture_alone() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("picture.png");
    picture(&file);
    let key = |t, value| Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let faded = Property {
        animation: Animation::Keyframed(vec![key(0, 40.0), key(1, 0.0)]),
        extra: serde_json::Map::new(),
    };
    let channels = [
        ("red_amount", 1.6),
        ("green_phase", -0.3),
        ("blue_phase", 0.5),
        ("rgb_randomness", 0.7),
        ("rgb_freq", 2.3),
    ];
    let mut render = |d: (Arc<Document>, Uuid)| {
        r.render_preview(&d.0, d.1, 45, Default::default(), 1.0)
            .unwrap()
            .0
    };
    let none = render(scene(&file, None));
    for (what, version, amplitude) in [
        ("still", 3, Property::fixed(0.0)),
        ("keyed", 3, faded.clone()),
        // Saved before Amplitude scaled the twist and the zoom.
        ("old and keyed", 2, faded.clone()),
    ] {
        for rows in [&[][..], &channels[..]] {
            let fx = shake(version, amplitude.clone(), rows);
            let got = render(scene(&file, Some(fx)));
            assert!(got == none, "{what} with {} rows", rows.len());
        }
    }
    // The fade itself moves the picture, so this isn't passing by doing nothing.
    let moving = shake(3, Property::fixed(40.0), &[]);
    assert!(render(scene(&file, Some(moving))) != none);
}

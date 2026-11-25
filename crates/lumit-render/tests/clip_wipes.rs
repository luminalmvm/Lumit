#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, ProjectItem,
    SolidDef, Switches, TransformGroup,
};
use lumit_core::sequence::{transition, Clip, ClipSource, FadeShape};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 64;
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn second(n: i64) -> Rational {
    Rational::new(n, 1).unwrap()
}

fn sequence(clips: Vec<Clip>) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "row".into(),
        kind: LayerKind::Sequence { clips },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(second(3)),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup {
            position_x: lumit_core::anim::Property::fixed(f64::from(SIZE) / 2.0),
            position_y: lumit_core::anim::Property::fixed(f64::from(SIZE) / 2.0),
            anchor_x: lumit_core::anim::Property::fixed(f64::from(SIZE) / 2.0),
            anchor_y: lumit_core::anim::Property::fixed(f64::from(SIZE) / 2.0),
            ..TransformGroup::default()
        },
        matte: None,
        parent: None,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        grade_versions: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

fn png(dir: &std::path::Path, name: &str, colour: [u8; 3]) -> std::path::PathBuf {
    let path = dir.join(name);
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(file, SIZE, SIZE);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    let pixels: Vec<u8> = (0..SIZE * SIZE).flat_map(|_| colour).collect();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    path
}

fn footage(doc: &mut Document, path: &std::path::Path) -> ClipSource {
    let id = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        id,
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
    }));
    ClipSource::Footage(id)
}

fn solid(doc: &mut Document, colour: [f32; 4]) -> ClipSource {
    let id = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id,
        name: "solid".into(),
        colour: LinearColour(colour),
        width: SIZE,
        height: SIZE,
        extra: serde_json::Map::new(),
    }));
    ClipSource::Solid(id)
}

fn nested(doc: &mut Document, colour: [f32; 4]) -> ClipSource {
    let ClipSource::Solid(def) = solid(doc, colour) else {
        panic!("a solid");
    };
    let mut layer = sequence(Vec::new());
    layer.kind = LayerKind::Solid { def };
    let inner = composition(vec![layer]);
    let id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));
    ClipSource::Comp(id)
}

fn composition(layers: Vec<Layer>) -> Composition {
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
        name: "cut".into(),
        width: SIZE,
        height: SIZE,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(second(3)),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn overlap(
    mut doc: Document,
    going: ClipSource,
    coming: ClipSource,
    kind: Option<&str>,
) -> (Arc<Document>, Uuid) {
    let clip = |source, at| Clip::new(source, Rational::ZERO, second(2), second(at), second(2));
    let mut incoming = clip(coming, 1);
    incoming.fade_in.shape = FadeShape::Linear;
    incoming.transition = kind.map(|k| transition::fresh(k, None, 64.0, 64.0).unwrap());
    let comp = composition(vec![sequence(vec![clip(going, 0), incoming])]);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

fn near(got: [u8; 3], want: [u8; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) < 40)
}

fn at(px: &[u8], x: u32, y: u32) -> [u8; 3] {
    let d = ((y * SIZE + x) * 4) as usize;
    [px[d], px[d + 1], px[d + 2]]
}

#[test]
fn every_kind_sweeps_from_the_outgoing_clip_to_the_incoming_one() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (red_png, blue_png) = (
        png(dir.path(), "red.png", RED),
        png(dir.path(), "blue.png", BLUE),
    );
    for (route, how) in [("solids", 0), ("footage", 1), ("compositions", 2)] {
        let build = |kind| {
            let mut doc = Document::new();
            let (a, b) = match how {
                1 => (footage(&mut doc, &red_png), footage(&mut doc, &blue_png)),
                2 => (
                    nested(&mut doc, [1.0, 0.0, 0.0, 1.0]),
                    nested(&mut doc, [0.0, 0.0, 1.0, 1.0]),
                ),
                _ => (
                    solid(&mut doc, [1.0, 0.0, 0.0, 1.0]),
                    solid(&mut doc, [0.0, 0.0, 1.0, 1.0]),
                ),
            };
            overlap(doc, a, b, kind)
        };
        for kind in transition::TABLE.iter().map(|k| k.name) {
            let (doc, comp) = build(Some(kind));
            let (px, _, _) = r.render_rgba(&doc, comp, 60, 1.0).unwrap();
            for (x, y) in [(2, 2), (32, 32), (61, 61), (2, 61), (61, 2)] {
                assert!(
                    near(at(&px, x, y), RED),
                    "{route} {kind} starts whole at {x},{y}: {:?}",
                    at(&px, x, y)
                );
            }
            let (px, _, _) = r.render_rgba(&doc, comp, 120, 1.0).unwrap();
            assert!(
                near(at(&px, 32, 32), BLUE),
                "{route} {kind} ends on the incoming clip"
            );
        }
        let (doc, comp) = build(Some("linear_wipe"));
        let (px, _, _) = r.render_rgba(&doc, comp, 90, 1.0).unwrap();
        assert!(
            near(at(&px, 8, 32), BLUE),
            "{route} left: {:?}",
            at(&px, 8, 32)
        );
        assert!(
            near(at(&px, 56, 32), RED),
            "{route} right: {:?}",
            at(&px, 56, 32)
        );

        let (doc, comp) = build(Some("iris_wipe"));
        let half_res = lumit_render::plan::Quality {
            auto_res: true,
            display_scale: 0.5,
            ..lumit_render::plan::Quality::default()
        };
        let (px, w, _) = r.render_preview(&doc, comp, 90, half_res, 0.5).unwrap();
        let half = |x: u32, y: u32| {
            let d = ((y * w + x) * 4) as usize;
            [px[d], px[d + 1], px[d + 2]]
        };
        let last = w - 1;
        assert!(near(half(w / 2, w / 2), BLUE), "{route} half iris centre");
        for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
            assert!(
                near(half(x, y), RED),
                "{route} half iris corner {x},{y}: {:?}",
                half(x, y)
            );
        }

        for kind in transition::TABLE.iter().map(|k| k.name) {
            let (doc, comp) = build(Some(kind));
            let (px, _, _) = r.render_rgba(&doc, comp, 119, 1.0).unwrap();
            let blue = (0..SIZE * SIZE)
                .filter(|i| near(at(&px, i % SIZE, i / SIZE), BLUE))
                .count();
            assert!(
                blue as u32 > SIZE * SIZE / 2,
                "{route} {kind}: {blue} of the picture is through"
            );
        }

        let (doc, comp) = build(Some("flash"));
        let (px, _, _) = r.render_rgba(&doc, comp, 90, 1.0).unwrap();
        assert!(
            near(at(&px, 32, 32), [255, 255, 255]),
            "{route} flash: {:?}",
            at(&px, 32, 32)
        );

        let (doc, comp) = build(Some("move"));
        let (px, _, _) = r.render_rgba(&doc, comp, 90, 1.0).unwrap();
        let (left, right) = (at(&px, 8, 32), at(&px, 56, 32));
        assert!(
            near(left, RED) && near(right, BLUE),
            "{route} push: {left:?} {right:?}"
        );

        // Half way through a Drift, each clip is half there, in linear light:
        // a picture the effect made part clear is not dimmed a second time.
        let (doc, comp) = build(Some("move"));
        let mut drift = (*doc).clone();
        if let Some(ProjectItem::Composition(c)) = drift.item_mut(comp) {
            if let LayerKind::Sequence { clips } = &mut c.layers[0].kind {
                let looks = transition::kind("move").unwrap().looks;
                let look = looks.iter().position(|l| l.name == "Drift");
                clips[1].transition = transition::fresh("move", look, 64.0, 64.0);
            }
        }
        let (px, _, _) = r.render_rgba(&Arc::new(drift), comp, 90, 1.0).unwrap();
        let mid = at(&px, 32, 32);
        assert!(
            (168..=208).contains(&mid[0]) && (168..=208).contains(&mid[2]),
            "{route} drift: {mid:?}"
        );

        let (doc, comp) = build(None);
        let (px, _, _) = r.render_rgba(&doc, comp, 90, 1.0).unwrap();
        let (left, right) = (at(&px, 8, 32), at(&px, 56, 32));
        assert_eq!(left, right, "{route} a dissolve has no edge");
        assert!(
            left[0] > 60 && left[2] > 60,
            "{route} a dissolve shows both: {left:?}"
        );

        let (doc, comp) = build(Some("linear_wipe"));
        let mut dressed = (*doc).clone();
        if let Some(ProjectItem::Composition(c)) = dressed.item_mut(comp) {
            if let LayerKind::Sequence { clips } = &mut c.layers[0].kind {
                let invert = lumit_core::fx::builtins::instantiate("invert").unwrap();
                clips[0].effects.push(invert);
            }
        }
        let (px, _, _) = r.render_rgba(&Arc::new(dressed), comp, 90, 1.0).unwrap();
        assert!(
            near(at(&px, 8, 32), BLUE),
            "{route} incoming as it was: {:?}",
            at(&px, 8, 32)
        );
        assert!(
            near(at(&px, 56, 32), [0, 255, 255]),
            "{route} outgoing inverted: {:?}",
            at(&px, 56, 32)
        );

        let (doc, comp) = build(Some("linear_wipe"));
        let mut bypassed = (*doc).clone();
        if let Some(ProjectItem::Composition(c)) = bypassed.item_mut(comp) {
            if let LayerKind::Sequence { clips } = &mut c.layers[0].kind {
                clips[1].transition.as_mut().unwrap().enabled = false;
            }
        }
        let (px, _, _) = r.render_rgba(&Arc::new(bypassed), comp, 90, 1.0).unwrap();
        assert_eq!(
            at(&px, 8, 32),
            at(&px, 56, 32),
            "{route} bypassed has no edge"
        );
    }
}

#[test]
fn an_overlap_with_no_kind_is_left_out_of_the_file() {
    let clip = Clip::new(
        ClipSource::Solid(Uuid::now_v7()),
        Rational::ZERO,
        second(1),
        Rational::ZERO,
        second(1),
    );
    let text = serde_json::to_string(&clip).unwrap();
    assert!(!text.contains("transition"), "{text}");
    let back: Clip = serde_json::from_str(&text).unwrap();
    assert_eq!(back.transition, None);
}

//! An overlap on a picture row drawn as a Transition effect: the effect runs
//! over the outgoing clip, the incoming clip beneath it, from nothing taken away
//! to nothing left. One with no effect named is the dissolve it always was.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, ProjectItem,
    SolidDef, Switches, TransformGroup,
};
use lumit_core::sequence::{transition::KINDS, Clip, ClipSource, FadeShape};
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

/// A one-colour picture on disk, which comes in through the decode worker.
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

/// A composition holding nothing but one colour.
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

/// Red for two seconds, then blue for two, the blue laid over the red's last
/// second, with `kind` on the overlap. Sixty frames a second, so frame 60 is
/// where the overlap starts and frame 120 where it has ended.
fn overlap(
    mut doc: Document,
    going: ClipSource,
    coming: ClipSource,
    kind: Option<&str>,
) -> (Arc<Document>, Uuid) {
    let clip = |source, at| Clip::new(source, Rational::ZERO, second(2), second(at), second(2));
    let mut incoming = clip(coming, 1);
    incoming.fade_in.shape = FadeShape::Linear;
    incoming.transition =
        kind.map(|k| lumit_core::fx::builtins::instantiate_for_raster(k, 64.0, 64.0).unwrap());
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

/// Every kind takes nothing away at its first frame and has taken everything at
/// the first frame past the overlap, and a Linear wipe half way is one edge, the
/// incoming clip on the side it went first. On solids, mixed where they are drawn,
/// and on footage, whose two pictures come through the decode worker together.
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
        for kind in KINDS {
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
        // Half way through a Linear wipe at its usual angle, the left has gone.
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

        // Near the end of each kind, what's left is less than half of it.
        for kind in KINDS {
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

        // No kind named is a dissolve: both, at once, everywhere.
        let (doc, comp) = build(None);
        let (px, _, _) = r.render_rgba(&doc, comp, 90, 1.0).unwrap();
        let (left, right) = (at(&px, 8, 32), at(&px, 56, 32));
        assert_eq!(left, right, "{route} a dissolve has no edge");
        assert!(
            left[0] > 60 && left[2] > 60,
            "{route} a dissolve shows both: {left:?}"
        );

        // A clip's own effects stay on it through the wipe: the outgoing clip
        // inverted on its side, and the incoming clip as it was on the other.
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

        // Bypassed, the effect is a dissolve again.
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

/// A row with nothing set writes no new field, and an old file's clip reads as
/// the dissolve, so nothing about it can change.
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

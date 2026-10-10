//! Layers of a Photoshop document, drawn in a real render.
//!
//! # In plain terms
//!
//! The reader's own tests prove a layer's pixels come out of the file right,
//! and the plan test proves the layer pick reaches the decoder's request.
//! Neither draws anything. This one writes a small document, builds the comp
//! an import would, and renders it through the entry the Viewer and the
//! exporter share, so it crosses the probe, the decode worker and the
//! compositor together.
//!
//! The document is a red background, a grey square on a layer of its own and
//! a blue fill layer. Each layer is its own footage item pointing at the same
//! file, so a render that read the flattened picture, or the wrong layer,
//! shows the wrong colour. The grey is what catches a 16 bit layer decoded
//! with the wrong curve.
//!
//! An Illustrator document goes through the same entry, and nothing but its
//! own reader opens it, so the second test here is what proves the probe and
//! the frame index answer for a file ffmpeg can't read.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, ProjectItem,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_media::psd::fixture::{document, Layer as PsdLayer};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const SIZE: u32 = 32;
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREY: [u8; 4] = [128, 128, 128, 255];

/// A comp the document's size holding one Footage layer per record named, top
/// first, each reading its own layer of `path`.
fn comp_of(path: &str, records: &[u32]) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let mut layers = Vec::new();
    for record in records {
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: format!("{record}/art.psd"),
            media: MediaRef {
                relative_path: "art.psd".into(),
                absolute_path: path.into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
            source_layer: Some(*record),
        }));
        layers.push(Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: format!("{record}"),
            kind: LayerKind::Footage { item },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(1, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            // A layer is read at the document's size, so the default
            // transform lays it over the comp exactly.
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            pan: Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        });
    }
    let comp = Uuid::now_v7();
    doc.items.push(ProjectItem::Composition(Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: comp,
        name: "art".into(),
        width: SIZE,
        height: SIZE,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(1, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    }));
    (Arc::new(doc), comp)
}

fn px(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}

fn close(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1)
}

#[test]
fn each_layer_of_a_document_draws_as_itself() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        eprintln!("no adapter here");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Bottom first: the background, then a grey square at rows and columns 8
    // to 16, then a blue fill layer masked to the columns left of 4.
    let mut fill = PsdLayer::fill("Fill", [0.0, 0.0, 255.0]);
    fill.mask = Some(([0, 0, 32, 4], 0, vec![255; 32 * 4]));
    let records = [
        PsdLayer::solid("Background", [0, 0, 32, 32], RED),
        PsdLayer::solid("Square", [8, 8, 16, 16], GREY),
        fill,
    ];
    // A 16 bit document is read as floats, and has to draw the same colours.
    for depth in [8, 16] {
        let path = dir.path().join(format!("art{depth}.psd"));
        std::fs::write(&path, document(SIZE, SIZE, depth, &records)).unwrap();
        let path = path.to_string_lossy().into_owned();

        // Every layer, the fill on top: the document as it looked.
        let (doc, comp) = comp_of(&path, &[2, 1, 0]);
        let (rgba, w, h) = r.render_rgba(&doc, comp, 0, 1.0).expect("render");
        assert_eq!((w, h), (SIZE, SIZE));
        assert!(
            close(px(&rgba, w, 12, 12), GREY),
            "the square, over the red, at {depth} bit"
        );
        assert!(close(px(&rgba, w, 6, 2), RED), "the background around it");
        assert!(close(px(&rgba, w, 20, 20), RED));
        assert!(close(px(&rgba, w, 2, 20), BLUE), "the fill, where it shows");

        // The square alone: where it is not, the comp's own black shows, so
        // the layer was read as itself and not as the flattened picture.
        let (doc, comp) = comp_of(&path, &[1]);
        let (rgba, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("render");
        assert!(close(px(&rgba, w, 12, 12), GREY));
        assert!(
            close(px(&rgba, w, 2, 2), [0, 0, 0, 255]),
            "empty off the layer"
        );
    }
}

#[test]
fn each_layer_of_an_illustrator_document_draws_as_itself() {
    use lumit_media::ai::fixture::{document as ai_document, Layer as AiLayer};

    let Ok(mut r) = HeadlessRenderer::shared() else {
        eprintln!("no adapter here");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Bottom first: a red background, then a blue square at rows and columns
    // 8 to 16.
    let layers = [
        AiLayer::solid("Background", [0, 0, 32, 32], [255, 0, 0]),
        AiLayer::solid("Square", [8, 8, 16, 16], [0, 0, 255]),
    ];
    let path = dir.path().join("art.ai");
    std::fs::write(&path, ai_document(SIZE, SIZE, &layers)).unwrap();
    let path = path.to_string_lossy().into_owned();

    let (doc, comp) = comp_of(&path, &[1, 0]);
    let (rgba, w, h) = r.render_rgba(&doc, comp, 0, 1.0).expect("render");
    assert_eq!((w, h), (SIZE, SIZE));
    assert!(
        close(px(&rgba, w, 12, 12), BLUE),
        "the square, over the red"
    );
    assert!(close(px(&rgba, w, 20, 20), RED), "the background around it");

    // The square alone, with the comp's own black where it is not.
    let (doc, comp) = comp_of(&path, &[1]);
    let (rgba, w, _) = r.render_rgba(&doc, comp, 0, 1.0).expect("render");
    assert!(close(px(&rgba, w, 12, 12), BLUE));
    assert!(
        close(px(&rgba, w, 2, 2), [0, 0, 0, 255]),
        "empty off the layer"
    );
}

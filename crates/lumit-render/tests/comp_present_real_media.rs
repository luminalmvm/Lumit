//! Rendering a comp built around a real clip, then presenting it — the whole
//! path the app walks when a comp is created from footage.
//!
//! Chasing a crash that only appears in the live app: "Binding D3D surface
//! failed", then the process gone, shortly after a comp is created from an
//! ordinary 1080p clip. The pieces have each been cleared on their own — the
//! file probes and decodes all its frames, and a shared texture can be made at
//! every size a preview asks for — so what is left is the two of them together,
//! plus the size change that creating a comp in an existing project causes.
//! That last part is the discriminator the report gives: an *empty* project was
//! fine, and an empty project is precisely the case with no previous comp and
//! therefore no shared-texture handover.
//!
//! Ignored by default: it wants a media file, named by `LUMIT_TEST_MEDIA`.

#![cfg(all(windows, feature = "shared-texture"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, FootageItem, Layer, LayerKind, MediaRef, ProjectItem, Switches,
    TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use uuid::Uuid;

/// A document holding one comp of `w × h` at `fps`, with the clip on it.
fn doc_with_clip(path: &str, w: u32, h: u32, fps: (u32, u32)) -> (std::sync::Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let item_id = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id: item_id,
        name: "clip.mp4".into(),
        media: MediaRef {
            relative_path: path.into(),
            absolute_path: path.into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
    }));

    let comp_id = Uuid::now_v7();
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "clip.mp4".into(),
        kind: LayerKind::Footage { item: item_id },
        in_point: CompTime(Rational::new(0, 1).unwrap()),
        out_point: CompTime(Rational::new(3, 1).unwrap()),
        start_offset: CompTime(Rational::new(0, 1).unwrap()),
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
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    doc.items.push(ProjectItem::Composition(Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: comp_id,
        name: "Repro".into(),
        width: w,
        height: h,
        frame_rate: FrameRate::new(fps.0, fps.1).unwrap(),
        duration: Duration(Rational::new(3, 1).unwrap()),
        background: lumit_core::model::LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    }));
    (std::sync::Arc::new(doc), comp_id)
}

/// Render and present a comp of the clip, then a *second* comp of a different
/// shape — the size change an existing project makes when a new comp is
/// created, and the one an empty project avoids.
#[test]
#[ignore = "harness: set LUMIT_TEST_MEDIA to a clip"]
fn a_comp_of_real_media_renders_and_presents_across_a_size_change() {
    let Ok(path) = std::env::var("LUMIT_TEST_MEDIA") else {
        eprintln!("set LUMIT_TEST_MEDIA to the clip to render");
        return;
    };
    let Ok(mut r) = HeadlessRenderer::new() else {
        eprintln!("no adapter here");
        return;
    };

    // First comp: some other shape, standing in for whatever was already open.
    let (doc_a, comp_a) = doc_with_clip(&path, 1280, 720, (30, 1));
    // Then the comp the clip's own size and rate would make.
    let (doc_b, comp_b) = doc_with_clip(&path, 1920, 1080, (24_000, 1_001));

    for (label, doc, comp) in [
        ("first comp 1280x720@30", &doc_a, comp_a),
        ("new comp 1920x1080@23.976", &doc_b, comp_b),
        ("back to the first", &doc_a, comp_a),
    ] {
        for frame in 0..6u64 {
            let prepared = r
                .render_prepared(doc, comp, frame, Quality::default(), true, true)
                .unwrap_or_else(|e| panic!("{label}: render frame {frame}: {e}"));
            let info = r
                .present_prepared(&prepared)
                .unwrap_or_else(|e| panic!("{label}: present frame {frame}: {e}"));
            assert_ne!(
                info.handle, 0,
                "{label}: frame {frame} handed out no handle"
            );
        }
        eprintln!("{label}: rendered and presented 6 frames");
    }
}

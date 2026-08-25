//! Temporary by-hand probe: where does a played frame's time go at full scale
//! vs quarter scale? Run with:
//!   cargo run --release -p lumit-render --features shared-texture --example perf_probe
//! Needs C:/tmp/test1080p60.mp4 (the playback bench's file). A by-hand probe,
//! like the playback bench: panicking on a missing file is its job.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, FootageItem, LayerKind, LinearColour, MediaRef, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::{HeadlessRenderer, Quality};
use std::time::Instant;
use uuid::Uuid;

const W: u32 = 1920;
const H: u32 = 1080;
const N: u64 = 90;

fn layer(kind: LayerKind, name: &str) -> lumit_core::model::Layer {
    lumit_core::model::Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::new(0, 1).unwrap()),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::new(0, 1).unwrap()),
        transform: TransformGroup {
            anchor_x: Property::fixed(f64::from(W) * 0.5),
            anchor_y: Property::fixed(f64::from(H) * 0.5),
            position_x: Property::fixed(f64::from(W) * 0.5),
            position_y: Property::fixed(f64::from(H) * 0.5),
            ..Default::default()
        },
        matte: None,
        parent: None,
        label: 0,
        volume_db: Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

fn doc_with(kind: LayerKind, extra_item: Option<ProjectItem>) -> (std::sync::Arc<Document>, Uuid) {
    let mut doc = Document::new();
    if let Some(item) = extra_item {
        doc.items.push(item);
    }
    let comp_id = Uuid::now_v7();
    doc.items.push(ProjectItem::Composition(Composition {
        id: comp_id,
        name: "Probe".into(),
        width: W,
        height: H,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer(kind, "probe")],
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    }));
    (std::sync::Arc::new(doc), comp_id)
}

fn quality(scale: f32) -> Quality {
    Quality {
        draft: false,
        auto_res: scale < 1.0,
        display_scale: scale,
        divisor: 1,
    }
}

fn run(
    label: &str,
    r: &mut HeadlessRenderer,
    doc: &std::sync::Arc<Document>,
    comp: Uuid,
    scale: f32,
) {
    let start = Instant::now();
    for f in 0..N {
        r.render_prepared(doc, comp, f, quality(scale), true, false)
            .expect("render");
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / N as f64;
    println!("{label:44} {ms:7.2} ms/frame  ({:.1} fps)", 1000.0 / ms);
}

#[cfg(all(windows, feature = "shared-texture"))]
fn run_present(
    label: &str,
    r: &mut HeadlessRenderer,
    doc: &std::sync::Arc<Document>,
    comp: Uuid,
    scale: f32,
) {
    let start = Instant::now();
    for f in 0..N {
        let p = r
            .render_prepared(doc, comp, f, quality(scale), true, false)
            .expect("render");
        r.present_prepared(&p).expect("present");
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / N as f64;
    println!("{label:44} {ms:7.2} ms/frame  ({:.1} fps)", 1000.0 / ms);
}

fn main() {
    let path = std::path::Path::new("C:/tmp/test1080p60.mp4");

    // --- 1. Raw sequential decode, no GPU at all. ---
    for (label, tw) in [
        ("decode sequential, native width", None),
        ("decode sequential, 480 target", Some(480u32)),
    ] {
        let index = lumit_media::index::build_frame_index(path).expect("index");
        let mut dec = lumit_media::VideoDecoder::open(path, index).expect("open");
        let start = Instant::now();
        for f in 0..N as usize {
            dec.frame_rgba(f, tw).expect("decode");
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / N as f64;
        println!("{label:44} {ms:7.2} ms/frame  ({:.1} fps)", 1000.0 / ms);
    }

    // --- 2. Composite-only: a full-frame solid, no decode anywhere. ---
    let mut r = HeadlessRenderer::new().expect("gpu");
    let solid_id = Uuid::now_v7();
    let solid = ProjectItem::Solid(SolidDef {
        id: solid_id,
        name: "Solid".into(),
        colour: LinearColour([1.0, 0.2, 0.1, 1.0]),
        width: W,
        height: H,
        extra: serde_json::Map::new(),
    });
    let (doc, comp) = doc_with(LayerKind::Solid { def: solid_id }, Some(solid));
    run(
        "solid comp, render only, scale 1.0",
        &mut r,
        &doc,
        comp,
        1.0,
    );
    run(
        "solid comp, render only, scale 0.25",
        &mut r,
        &doc,
        comp,
        0.25,
    );

    // --- 3. Footage comp: first pass decodes, second pass hits the cache. ---
    let item_id = Uuid::now_v7();
    let footage = ProjectItem::Footage(FootageItem {
        sequence: None,
        id: item_id,
        name: "test1080p60.mp4".into(),
        media: MediaRef {
            relative_path: "test1080p60.mp4".into(),
            absolute_path: "C:/tmp/test1080p60.mp4".into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
    });
    let (doc, comp) = doc_with(LayerKind::Footage { item: item_id }, Some(footage));
    run(
        "footage comp, cold decode, scale 1.0",
        &mut r,
        &doc,
        comp,
        1.0,
    );
    run(
        "footage comp, warm decode cache, scale 1.0",
        &mut r,
        &doc,
        comp,
        1.0,
    );
    run(
        "footage comp, cold-ish decode, scale 0.25",
        &mut r,
        &doc,
        comp,
        0.25,
    );

    // --- 4. Render + present (the whole per-frame pipeline). ---
    #[cfg(all(windows, feature = "shared-texture"))]
    {
        run_present(
            "footage comp, warm, render+present, 1.0",
            &mut r,
            &doc,
            comp,
            1.0,
        );
        run_present(
            "footage comp, warm, render+present, 0.25",
            &mut r,
            &doc,
            comp,
            0.25,
        );
    }
}

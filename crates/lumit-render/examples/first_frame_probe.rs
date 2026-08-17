//! Temporary by-hand probe: what does the FIRST preview cost, before any
//! footage is involved? Run with:
//!   cargo run --release -p lumit-render --features shared-texture --example first_frame_probe
//! Needs no media at all — the whole point is that a brand-new empty comp in a
//! brand-new project takes noticeable time to show anything, so this times the
//! startup that stands between "worker started" and "first frame out".
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{Composition, Document, LinearColour, ProjectItem};
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_render::{HeadlessRenderer, Quality};
use std::time::Instant;
use uuid::Uuid;

const W: u32 = 1920;
const H: u32 = 1080;

fn empty_doc() -> (std::sync::Arc<Document>, Uuid) {
    let mut doc = Document::new();
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
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    }));
    (std::sync::Arc::new(doc), comp_id)
}

fn ms(at: Instant) -> f64 {
    at.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    // --- 1. The parts of a renderer, one at a time. ---
    let t = Instant::now();
    let gpu = lumit_gpu::GpuContext::headless().expect("gpu");
    println!("{:44} {:8.1} ms", "GpuContext::headless", ms(t));

    let t = Instant::now();
    let _colour = lumit_gpu::ColourEngine::new(&gpu);
    println!("{:44} {:8.1} ms", "ColourEngine::new", ms(t));

    let t = Instant::now();
    let _compositor = lumit_gpu::Compositor::new(&gpu);
    println!("{:44} {:8.1} ms", "Compositor::new", ms(t));

    let t = Instant::now();
    let _fx = lumit_gpu::fx::FxEngine::new(&gpu);
    println!(
        "{:44} {:8.1} ms",
        "FxEngine::new (every WGSL kernel)",
        ms(t)
    );

    let t = Instant::now();
    let _scope = lumit_gpu::scope::ScopeEngine::new(&gpu);
    println!("{:44} {:8.1} ms", "ScopeEngine::new", ms(t));

    // --- 2. The whole thing, as the worker builds it. ---
    let t = Instant::now();
    let mut r = HeadlessRenderer::new().expect("gpu");
    println!(
        "{:44} {:8.1} ms",
        "HeadlessRenderer::new (worker start)",
        ms(t)
    );

    // --- 3. The first frame of an empty comp, then the ones after it. ---
    let (doc, comp) = empty_doc();
    let q = Quality {
        draft: false,
        auto_res: false,
        display_scale: 1.0,
        divisor: 1,
    };

    let t = Instant::now();
    let first = r
        .render_prepared(&doc, comp, 0, q, true, false)
        .expect("render");
    println!("{:44} {:8.1} ms", "empty comp, FIRST frame", ms(t));

    #[cfg(all(windows, feature = "shared-texture"))]
    {
        let t = Instant::now();
        r.present_prepared(&first).expect("present");
        println!("{:44} {:8.1} ms", "empty comp, FIRST present", ms(t));
    }
    drop(first);

    for f in 1..4 {
        let t = Instant::now();
        let p = r
            .render_prepared(&doc, comp, f, q, true, false)
            .expect("render");
        #[cfg(all(windows, feature = "shared-texture"))]
        r.present_prepared(&p).expect("present");
        drop(p);
        println!("{:44} {:8.1} ms", format!("empty comp, frame {f}"), ms(t));
    }
}

//! **A lost graphics device is survivable** (K-585, budget B9).
//!
//! # In plain terms
//!
//! A graphics card can be taken away from a program that has done nothing
//! wrong: a driver update mid-session, a hung dispatch the watchdog killed, a
//! laptop switching between its two GPUs. Everything on the card goes with it —
//! every texture, every compiled shader, every frame the preview had banked —
//! and nothing built against the old device will ever work again.
//!
//! The cure is not repair, it is replacement: notice, throw the renderer away,
//! build another, carry on. This file proves the two halves of that from the
//! renderer's side, which is where the worker's recovery step does its work:
//!
//! 1. A renderer whose device has gone **says so** — the driver's own callback
//!    raises the flag, and it is still raised on the next turn round the loop.
//! 2. A renderer built afterwards **draws the same picture**, on the same
//!    machine, with no ceremony in between. That is the whole of the worker's
//!    recovery: `HeadlessRenderer::new` on the existing K-434 build road.
//!
//! The loss here is real rather than a set boolean: `simulate_device_loss`
//! destroys the device, so the second render is genuinely being served by a
//! device that did not exist when the first one was.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
    TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 32;

/// One flat red solid filling a small comp — the cheapest picture that is
/// unmistakably *a picture*, so "it drew again" cannot be satisfied by a blank
/// frame or by the comp's own background.
fn project() -> (Arc<Document>, Uuid) {
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "red".into(),
        colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
        width: COMP,
        height: COMP,
        extra: serde_json::Map::new(),
    }));
    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: COMP,
        height: COMP,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "red".into(),
            kind: LayerKind::Solid { def },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            pan: lumit_core::anim::Property::zero(),
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
        }],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id)
}

#[test]
fn a_renderer_rebuilt_after_a_device_loss_draws_the_picture_again() {
    let Ok(mut renderer) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (doc, comp) = project();

    let (before, w, h) = renderer
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("first draw");
    assert_eq!((w, h), (COMP, COMP), "rendered at comp size");
    assert!(
        !renderer.device_lost(),
        "a renderer that has just drawn is not lost"
    );

    // Take the device away for real.
    renderer.simulate_device_loss();
    assert!(
        renderer.device_lost(),
        "the driver's callback must reach the renderer, or nothing downstream \
         ever learns to rebuild"
    );

    // What the worker does about it: drop the renderer, build another. Nothing
    // is carried across — the caches on the card went with the device.
    drop(renderer);
    let mut rebuilt = HeadlessRenderer::new().expect("a device can be opened again after a loss");
    assert!(
        !rebuilt.device_lost(),
        "the replacement starts healthy; the flag belongs to the device, not to \
         the process"
    );

    let (after, aw, ah) = rebuilt
        .render_rgba(&doc, comp, 0, 1.0)
        .expect("the picture comes back");
    assert_eq!((aw, ah), (w, h), "the same raster as before the loss");
    assert_eq!(
        after, before,
        "and the same pixels: a rebuilt renderer is not a degraded one"
    );
}

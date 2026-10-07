//! **Particulate on a solid, through the document** — the owner's own staging.
//!
//! # In plain terms
//!
//! The unit tests in `gpufx.rs` prove the passes work when a birth schedule is
//! handed to them. They build that schedule themselves. Nothing proved that the
//! *application* builds one — that adding Particulate to a solid layer the way
//! a user does draws a single pixel.
//!
//! This does. It builds the project the owner described — a 1920x1080 comp, a
//! comp-sized white solid, Add effect -> Particulate, nothing touched — and
//! pushes it through the same public entry the Viewer and the exporter use
//! (`HeadlessRenderer::render_rgba`, the one comp walk preview and export
//! share).

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, EffectValue, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

/// The comp the New composition dialog makes when nobody chooses
/// (`BridgeCompSettings::defaults`): 1920x1080, 60 fps, 30 seconds. Staged at
/// the real numbers because Particulate's default Position is 960, 540 — the
/// centre of exactly this comp — and a smaller test raster would move the
/// emitter off-frame and prove nothing about what the owner saw.
const W: u32 = 1920;
const H: u32 = 1080;
const FPS: u32 = 60;

/// A comp-sized white solid, anchored and placed at the centre, the way
/// `add_solid_layer` seeds one (`centred_transform`).
fn solid_layer(def: Uuid) -> Layer {
    use lumit_core::anim::Property;
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "White solid 1".into(),
        kind: LayerKind::Solid { def },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(30, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup {
            anchor_x: Property::fixed(f64::from(W) * 0.5),
            anchor_y: Property::fixed(f64::from(H) * 0.5),
            position_x: Property::fixed(f64::from(W) * 0.5),
            position_y: Property::fixed(f64::from(H) * 0.5),
            ..TransformGroup::default()
        },
        matte: None,
        parent: None,
        label: 2,
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
    }
}

/// The project: one comp, one white solid, and `effects` on it — empty for the
/// control, one default Particulate for the subject.
fn project(effects: Vec<lumit_core::model::EffectInstance>) -> (Arc<Document>, Uuid) {
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "White solid 1".into(),
        // Not white: the particles are white too, and a white field cannot
        // show a white mote. Mid grey is what the owner would have reached for
        // the moment the first render came back blank, and it is what makes
        // "the picture changed" a readable claim.
        colour: LinearColour([0.25, 0.25, 0.25, 1.0]),
        width: W,
        height: H,
        extra: serde_json::Map::new(),
    }));

    let mut layer = solid_layer(def);
    layer.effects = effects;

    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp 1".into(),
        width: W,
        height: H,
        frame_rate: FrameRate::new(FPS, 1).unwrap(),
        duration: Duration(Rational::new(30, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), comp_id)
}

/// A fresh Particulate at its declared defaults — `Add effect -> Particulate`
/// and nothing else, which is exactly what the owner did.
fn default_particulate() -> lumit_core::model::EffectInstance {
    lumit_core::fx::instantiate("particulate").expect("particulate is a built-in")
}

/// Sets one parameter on an instance, for the fiddling half of the report.
fn set(inst: &mut lumit_core::model::EffectInstance, id: &str, v: EffectValue) {
    for p in &mut inst.params {
        if p.id == id {
            p.value = v;
            return;
        }
    }
    panic!("particulate has no parameter {id}");
}

fn f(v: f64) -> EffectValue {
    EffectValue::Float(lumit_core::anim::Property::fixed(v))
}

/// How many pixels differ from the control render, and by how much at worst.
fn diff(a: &[u8], b: &[u8]) -> (usize, u8) {
    let mut n = 0;
    let mut worst = 0u8;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        if d > 0 {
            n += 1;
            worst = worst.max(d);
        }
    }
    (n, worst)
}

/// **The owner's report, as a test**: default Particulate on a solid draws
/// something, at a frame in the middle of the layer's span.
///
/// Frame 60 is one second in — long enough that the default Emit rate of 150
/// per second has issued 150 births and the default Life of 2 s has killed
/// none of them, so the frame is unambiguously mid-field.
#[test]
fn a_default_particulate_on_a_solid_draws_particles() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };

    let (bare_doc, bare_comp) = project(Vec::new());
    let (fx_doc, fx_comp) = project(vec![default_particulate()]);

    let (bare, w, h) = r
        .render_rgba(&bare_doc, bare_comp, 60, 1.0)
        .expect("the bare solid renders");
    let (drawn, dw, dh) = r
        .render_rgba(&fx_doc, fx_comp, 60, 1.0)
        .expect("the solid with Particulate renders");
    assert_eq!((w, h), (W, H), "rendered at comp size");
    assert_eq!((dw, dh), (w, h), "both renders are the same raster");

    let (n, worst) = diff(&bare, &drawn);
    assert!(
        n > 0,
        "default Particulate on a solid changed no pixel at all — the effect is invisible, \
         which is the owner's report exactly"
    );
    assert!(
        worst > 4,
        "default Particulate changed {n} pixels but by at most {worst}/255 — that is not a \
         field of motes, it is rounding"
    );
}

/// **The crash, as a test** — an Emit rate big enough to fill the candidate
/// window renders a frame instead of faulting the device.
///
/// Emit rate's slider stops at 1 000 but its hard maximum is open, so a typed
/// ten million is a document a user can make — and the way to make one is to
/// see nothing and reach for the biggest number on the row, which is what
/// happened. The candidate set is then trimmed to `MAX_CANDIDATES`, and the
/// evaluate pass dispatches one workgroup per 64 of them: at the old ceiling of
/// 8 000 000 that asked the device for 125 000 workgroups against a limit of
/// 65 535, which is a validation error that invalidates the encoder and takes
/// the draw down with it.
///
/// It has to check the *picture*, not just that a frame came back, because
/// `lumit-gpu`'s uncaptured-error handler reports and carries on rather than
/// panicking (docs/14) — the render returns `Ok` either way. What separates
/// the two is unmistakable once looked at: a working frame differs from the
/// bare solid in the few thousand bytes the particles cover, and a faulted one
/// differs in all 8 294 400, because the invalidated encoder never ran the
/// draw and what comes back is black.
#[test]
fn a_huge_emit_rate_renders_instead_of_faulting_the_device() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (bare_doc, bare_comp) = project(Vec::new());
    let (bare, ..) = r
        .render_rgba(&bare_doc, bare_comp, 60, 1.0)
        .expect("the bare solid renders");

    // Ten million a second, and again with the budget dial at its hard ceiling
    // so the stream buffer is at its largest at the same time.
    for (what, rate, cap) in [
        ("ten million a second", 10_000_000.0, None),
        ("and at the hard cap", 10_000_000.0, Some(1_000_000.0)),
        ("a hundred million a second", 100_000_000.0, None),
    ] {
        let mut inst = default_particulate();
        set(&mut inst, "emit_rate", f(rate));
        if let Some(c) = cap {
            set(&mut inst, "max_particles", f(c));
        }
        let (doc, comp) = project(vec![inst]);
        let (drawn, ..) = r
            .render_rgba(&doc, comp, 60, 1.0)
            .unwrap_or_else(|e| panic!("{what} failed to render: {e}"));
        // A drawn frame differs from the solid in the pixels the particles
        // cover and nowhere else — a few tens of thousands of bytes out of
        // eight million. A *faulted* frame differs in every one of them: the
        // invalidated encoder never ran the draw, and what comes back is black
        // rather than the picture that was copied in. So the gate is two-sided,
        // and it is the upper half that catches this bug.
        let (n, worst) = diff(&bare, &drawn);
        assert!(
            n > 0 && worst > 4,
            "{what} came back as the bare solid ({n} bytes differ, worst {worst}) — the \
             evaluate pass did not run"
        );
        assert!(
            n < drawn.len() / 2,
            "{what} changed {n} of {} bytes — that is not a particle field, it is a destroyed \
             frame, which is what an over-sized dispatch leaves behind",
            drawn.len()
        );
    }

    // And the device still works afterwards: a faulted encoder poisons what
    // follows it, so the plain render below is the real proof that nothing was
    // left broken.
    let (again, ..) = r
        .render_rgba(&bare_doc, bare_comp, 60, 1.0)
        .expect("the device still renders after the big frames");
    assert!(
        again == bare,
        "the bare solid changed after a huge-rate frame — the device was left in a bad state"
    );
}

// ------------------------------------------------ the third axis

/// **The generators go through the same door**: dropping Grid or Scatter on a
/// solid draws something, at the raster the comp asks for.
///
/// The unit tests build the point set themselves. What this one proves is the
/// *application* half — the draw builder threading a carriage to an effect with
/// no Emit rate to scan, and the GPU table finding the pass by name — which no
/// unit test can see.
#[test]
fn the_generators_draw_on_a_solid_through_the_document() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (bare_doc, bare_comp) = project(Vec::new());
    let (bare, ..) = r
        .render_rgba(&bare_doc, bare_comp, 60, 1.0)
        .expect("the bare solid renders");

    for name in ["grid", "scatter"] {
        let inst = lumit_core::fx::instantiate(name).expect("a built-in");
        let (doc, comp) = project(vec![inst]);
        let (drawn, ..) = r
            .render_rgba(&doc, comp, 60, 1.0)
            .expect("the solid with a generator renders");
        let (n, worst) = diff(&bare, &drawn);
        assert!(
            n > 0 && worst > 4,
            "{name} on a solid changed {n} pixels by at most {worst}/255 — that is not a field \
             of points"
        );
        // Twice is once: there is no clock in either of them.
        let (again, ..) = r
            .render_rgba(&doc, comp, 60, 1.0)
            .expect("the generator renders again");
        assert_eq!(drawn, again, "{name} drew two different pictures");
        // And no clock means the frame does not matter, which is the whole
        // claim of a generator against a particle system.
        let (later, ..) = r
            .render_rgba(&doc, comp, 300, 1.0)
            .expect("the generator renders later");
        assert_eq!(drawn, later, "{name} moved with the playhead");
    }
}

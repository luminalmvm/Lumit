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
//! (`HeadlessRenderer::render_rgba`, the one comp walk of K-031).

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
/// `add_solid_layer` seeds one (`centred_transform`, K-150).
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
    let Ok(mut r) = HeadlessRenderer::new() else {
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

/// The other half of the same claim: **before the layer's in point there is
/// nothing to draw, and after it there is**. A schedule scanned from the wrong
/// clock draws the same thing at every time, which this notices.
#[test]
fn particulate_draws_nothing_at_frame_zero_and_something_later() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (bare_doc, bare_comp) = project(Vec::new());
    let (fx_doc, fx_comp) = project(vec![default_particulate()]);

    let (bare0, ..) = r
        .render_rgba(&bare_doc, bare_comp, 0, 1.0)
        .expect("bare f0");
    let (fx0, ..) = r.render_rgba(&fx_doc, fx_comp, 0, 1.0).expect("fx f0");
    let (bare120, ..) = r
        .render_rgba(&bare_doc, bare_comp, 120, 1.0)
        .expect("bare f120");
    let (fx120, ..) = r.render_rgba(&fx_doc, fx_comp, 120, 1.0).expect("fx f120");

    // Frame 0 is the layer's in point: the schedule has run for no time at all,
    // so no birth has been issued and the picture is the solid, untouched.
    assert_eq!(
        diff(&bare0, &fx0).0,
        0,
        "at the layer's in point Particulate has issued no births, so it must pass its picture \
         through unchanged"
    );
    assert!(
        diff(&bare120, &fx120).0 > 0,
        "two seconds in, Particulate must be drawing"
    );
}

/// **The fiddling**, in one render each: the parameter moves the owner made
/// after the first blank frame. None of these may panic, and none may produce a
/// GPU validation fault — docs/14 §4's no-panic rule reaches every path a
/// slider can steer into.
#[test]
fn particulate_survives_the_parameter_fiddling() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };

    let cases: Vec<(&str, Vec<(&str, EffectValue)>)> = vec![
        // "Drive Emit rate" — up to the top of the slider and back to nothing.
        ("emit rate at the maximum", vec![("emit_rate", f(1000.0))]),
        ("emit rate at zero", vec![("emit_rate", f(0.0))]),
        // Rate zero is the division suspect: no births, so nothing to spread
        // inside a frame and nothing to compact.
        (
            "emit rate zero with a huge cap",
            vec![("emit_rate", f(0.0)), ("max_particles", f(200_000.0))],
        ),
        // "Change render mode to sprite with no layer" — the documented
        // fallback to discs.
        (
            "sprite mode, no layer",
            vec![("mode", EffectValue::Choice(1))],
        ),
        (
            "sprite mode, no layer, no births",
            vec![("mode", EffectValue::Choice(1)), ("emit_rate", f(0.0))],
        ),
        ("streak mode", vec![("mode", EffectValue::Choice(2))]),
        (
            "streak mode with no tail",
            vec![("mode", EffectValue::Choice(2)), ("streak_length", f(0.0))],
        ),
        // "Crank Max particles" — the budget dial at its hard ceiling and at
        // its floor. The cases share one `HeadlessRenderer`, so the cap also
        // *shrinks* from a million to one between two renders on the same
        // engine, which is the pooled-buffer case worth having.
        (
            "max particles at the hard cap",
            vec![("max_particles", f(1_000_000.0))],
        ),
        ("max particles at one", vec![("max_particles", f(1.0))]),
        (
            "a cap far below the live count",
            vec![("emit_rate", f(1000.0)), ("max_particles", f(1.0))],
        ),
        // Life at nothing: every candidate is already dead, so the live count
        // is zero with a full candidate set behind it.
        ("no life at all", vec![("life", f(0.0))]),
        (
            "no life, no jitter",
            vec![("life", f(0.0)), ("life_jitter", f(0.0))],
        ),
        // Size zero: live particles that cover no pixel.
        ("no size", vec![("size", f(0.0)), ("size_jitter", f(0.0))]),
        // The mask-path emitter with no mask on the layer — the documented
        // empty-polyline no-op.
        (
            "mask path emitter with no mask",
            vec![("shape", EffectValue::Choice(4))],
        ),
    ];

    for (what, params) in cases {
        let mut inst = default_particulate();
        for (id, v) in params {
            set(&mut inst, id, v);
        }
        let (doc, comp) = project(vec![inst]);
        // Two frames apiece: one mid-field, and one the scrub lands on.
        for frame in [60u64, 3] {
            r.render_rgba(&doc, comp, frame, 1.0)
                .unwrap_or_else(|e| panic!("{what} at frame {frame} failed to render: {e}"));
        }
    }
}

/// **The scrub**, which is the other thing the owner did: the same document at
/// many times, in the order a playhead drag visits them, including backwards
/// over the in point. Random access is the whole premise of K-474, so this is
/// both a crash test and the property test for it.
#[test]
fn particulate_survives_a_scrub() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };
    let (doc, comp) = project(vec![default_particulate()]);

    // Forwards, backwards, and a jump: the same frame must come back the same
    // picture whichever way the playhead reached it (K-474 random access).
    let mut first: Option<Vec<u8>> = None;
    for frame in [90u64, 0, 300, 3, 90, 1, 600, 90] {
        let (rgba, ..) = r
            .render_rgba(&doc, comp, frame, 1.0)
            .unwrap_or_else(|e| panic!("the scrub failed at frame {frame}: {e}"));
        if frame == 90 {
            match &first {
                None => first = Some(rgba),
                Some(want) => assert!(
                    *want == rgba,
                    "frame 90 came back different after scrubbing away and back — a particle \
                     system with no state cannot do that (K-474)"
                ),
            }
        }
    }
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
    let Ok(mut r) = HeadlessRenderer::new() else {
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

// ------------------------------------------------ the third axis (K-561)

/// The same project, with the layer's 3D switch set and a camera in the comp.
///
/// The camera is placed a little off centre and turned, because a camera that
/// sits exactly where the default one is looks at the layer's plane head-on and
/// foreshortens a *centred* emitter almost symmetrically — which is a picture
/// that could be mistaken for the flat one.
fn project_3d(
    effects: Vec<lumit_core::model::EffectInstance>,
    three_d: bool,
    with_camera: bool,
) -> (Arc<Document>, Uuid) {
    use lumit_core::anim::Property;
    let (doc, comp_id) = project(effects);
    let mut doc = (*doc).clone();
    let comp = doc.comp_mut(comp_id).expect("the comp is there");
    comp.layers[0].switches.three_d = three_d;
    if with_camera {
        let mut camera = solid_layer(Uuid::now_v7());
        camera.name = "Camera 1".into();
        camera.kind = LayerKind::Camera {
            zoom: Property::fixed(f64::from(H) * 1.5),
            solve_link: None,
            correction_base: None,
        };
        camera.transform.position_x = Property::fixed(f64::from(W) * 0.35);
        camera.transform.position_y = Property::fixed(f64::from(H) * 0.65);
        camera.transform.rotation_y = Property::fixed(-12.0);
        comp.layers.insert(0, camera);
    }
    (Arc::new(doc), comp_id)
}

/// A Particulate with real depth in it: an emitter reaching a long way through
/// the layer's plane, and particles launched out of it.
fn deep_particulate() -> lumit_core::model::EffectInstance {
    let mut inst = default_particulate();
    set(&mut inst, "depth", f(1200.0));
    set(&mut inst, "direction_z", f(20.0));
    set(&mut inst, "spread_z", f(120.0));
    set(&mut inst, "size", f(20.0));
    inst
}

/// **The whole claim, through the application** (K-561): on a 3D layer the
/// particles are seen through the composition's camera, and on a 2D layer they
/// are not — the camera might as well not be there.
///
/// The second half is the K-258 gate at its widest: a project that never asked
/// for depth renders byte for byte whatever the comp is looking with.
#[test]
fn a_three_d_particulate_is_seen_through_the_comps_camera() {
    let Ok(mut r) = HeadlessRenderer::new() else {
        lumit_gpu::no_adapter();
        return;
    };

    // ONE instance, cloned: `instantiate` rolls a fresh seed for every new
    // effect (docs/08 §2.4), so two "identical" fixtures would be two different
    // fields of particles and every comparison below would be meaningless.
    let deep = deep_particulate();
    macro_rules! render {
        ($three_d:expr, $camera:expr, $fx:expr) => {{
            let (doc, comp) = project_3d($fx, $three_d, $camera);
            let (px, ..) = r
                .render_rgba(&doc, comp, 60, 1.0)
                .expect("the comp renders");
            px
        }};
    }
    // A 2D layer: the camera changes nothing at all, byte for byte.
    let flat = render!(false, false, vec![deep.clone()]);
    let flat_with_camera = render!(false, true, vec![deep.clone()]);
    assert_eq!(
        flat, flat_with_camera,
        "a camera moved a 2D layer's particles — the K-258 guarantee is broken"
    );

    // A 3D layer without a camera: still nothing to see depth with.
    assert_eq!(
        render!(true, false, vec![deep.clone()]),
        flat,
        "the 3D switch alone moved the particles — there is no camera to see them with"
    );

    // And with both: a different picture, and still a picture.
    let seen = render!(true, true, vec![deep.clone()]);
    let (n, worst) = diff(&flat, &seen);
    assert!(
        n > 0 && worst > 4,
        "the camera drew the same field as the flat layer ({n} bytes differ, worst {worst})"
    );
    assert!(
        n < seen.len() / 2,
        "the camera destroyed the frame rather than projecting it ({n} bytes)"
    );

    // Twice is once (docs/08 §2.4): the projection is arithmetic, not state.
    assert_eq!(
        seen,
        render!(true, true, vec![deep.clone()]),
        "one frame, two pictures"
    );
}

/// **The generators go through the same door** (K-598, K-599): dropping Grid or
/// Scatter on a solid draws something, at the raster the comp asks for.
///
/// The unit tests build the point set themselves. What this one proves is the
/// *application* half — the draw builder threading a carriage to an effect with
/// no Emit rate to scan, and the GPU table finding the pass by name — which no
/// unit test can see.
#[test]
fn the_generators_draw_on_a_solid_through_the_document() {
    let Ok(mut r) = HeadlessRenderer::new() else {
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

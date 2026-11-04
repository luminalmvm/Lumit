//! Depth points line up with the footage from the camera they came from: every
//! point is on its pixel's ray, however deep the depth says it is, and from
//! anywhere else they are not flat.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectParam, EffectValue, LinearColour, ProjectItem,
    SolidDef,
};
use lumit_core::planes::DEPTH_POINTS;
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use lumit_render::planes::{publish, run_from_planes, PlaneKind, PlaneOut};
use std::sync::Arc;
use uuid::Uuid;

const W: u32 = 640;
const H: u32 = 360;

fn set(fx: &mut EffectInstance, id: &str, value: EffectValue) {
    match fx.params.iter_mut().find(|p| p.id == id) {
        Some(p) => p.value = value,
        None => fx.params.push(EffectParam {
            id: id.to_owned(),
            value,
            extra: serde_json::Map::new(),
        }),
    }
}

fn float(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

/// A depth that varies a lot across the frame, with a hard step in it.
fn publish_depth(effect: Uuid) {
    let (w, h) = (W as usize / 2, H as usize / 2);
    let mut data = Vec::with_capacity(w * h * 2);
    for y in 0..h {
        for x in 0..w {
            let wave = 0.5 + 0.5 * (x as f64 * 0.07).sin() * (y as f64 * 0.11).cos();
            let d = if x > w / 2 && y > h / 2 { 1.0 } else { wave };
            data.extend_from_slice(&((d * 65535.0) as u16).to_le_bytes());
        }
    }
    let plane = PlaneOut {
        width: w as u32,
        height: h as u32,
        kind: PlaneKind::Depth,
        data,
    };
    publish(
        effect,
        run_from_planes(60.0, 1, "test", &[(0, plane)]).expect("a run"),
    );
}

struct Scene {
    doc: Arc<Document>,
    comp: Uuid,
}

/// A 3D solid wearing busy noise so a pixel out of place shows, with Depth
/// points of `size` on it when `points` is true, tilted by `tilt` degrees. A
/// camera off to one side is named by Depth points' Camera row and is the
/// active one only when `from_named` is true.
fn scene(
    noise: &EffectInstance,
    points: bool,
    size: f64,
    analysed: bool,
    from_named: bool,
    tilt: f64,
) -> Scene {
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Solid".into(),
        colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
        width: W,
        height: H,
        extra: serde_json::Map::new(),
    }));
    let mut comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: W,
        height: H,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(1, 1).unwrap()),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };

    let named = {
        let mut c = lumit_core::edits::fresh_camera(&comp, "Named".into(), None);
        c.transform.position_x = Property::fixed(f64::from(W) * 0.5 + 160.0);
        c.transform.position_y = Property::fixed(f64::from(H) * 0.5 - 60.0);
        c
    };
    let mut other = lumit_core::edits::fresh_camera(&comp, "Other".into(), None);
    other.transform.position_x = Property::fixed(f64::from(W) * 0.5 - 300.0);
    // The topmost visible camera is the one the comp is seen through.
    other.switches.visible = !from_named;

    let mut layer = lumit_core::edits::base_layer(
        "Solid".into(),
        lumit_core::model::LayerKind::Solid { def },
        comp.duration.0,
        lumit_core::model::TransformGroup {
            anchor_x: Property::fixed(f64::from(W) * 0.5),
            anchor_y: Property::fixed(f64::from(H) * 0.5),
            position_x: Property::fixed(f64::from(W) * 0.5),
            position_y: Property::fixed(f64::from(H) * 0.5),
            rotation_y: Property::fixed(tilt),
            ..Default::default()
        },
    );
    layer.switches.three_d = true;
    layer.effects.push(noise.clone());
    if points {
        let mut fx = lumit_core::fx::builtins::instantiate(DEPTH_POINTS).expect("a built-in");
        set(&mut fx, "point_size", float(size));
        set(&mut fx, "camera", EffectValue::Layer(Some(named.id)));
        if analysed {
            publish_depth(fx.id);
        }
        layer.effects.push(fx);
    }
    comp.layers = vec![other, named, layer];
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    Scene {
        doc: Arc::new(doc),
        comp: id,
    }
}

fn render(r: &mut HeadlessRenderer, s: &Scene) -> Vec<u8> {
    r.render_rgba(&s.doc, s.comp, 0, 1.0).expect("a frame").0
}

/// The worst difference between `a` and `b` over the frame's green channel.
fn worst(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .skip(1)
        .step_by(4)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// How far `a` sits from `b` in the tile with its corner at `(x0, y0)`, as the
/// whole-pixel shift of `b` that matches it best.
fn best_shift(a: &[u8], b: &[u8], (x0, y0): (i32, i32)) -> (i32, i32) {
    const TILE: i32 = 40;
    let at = |p: &[u8], x: i32, y: i32| i32::from(p[((y * W as i32 + x) * 4 + 1) as usize]);
    let mut best = (i32::MAX, (0, 0));
    for dy in -3..=3 {
        for dx in -3..=3 {
            let mut sum = 0;
            for y in y0..y0 + TILE {
                for x in x0..x0 + TILE {
                    sum += (at(a, x, y) - at(b, x + dx, y + dy)).abs();
                }
            }
            // The smaller shift wins a tie.
            let key = sum * 8 + dx.abs() + dy.abs();
            if key < best.0 {
                best = (key, (dx, dy));
            }
        }
    }
    best.1
}

/// How many tiles across the middle of the frame have moved from `flat`.
fn moved(points: &[u8], flat: &[u8]) -> i32 {
    (20..W as i32 - 200)
        .step_by(40)
        .map(|x0| {
            let (dx, dy) = best_shift(points, flat, (x0, H as i32 / 2));
            i32::from(dx != 0 || dy != 0)
        })
        .sum()
}

#[test]
fn points_line_up_with_the_footage_from_their_camera_and_not_from_elsewhere() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    // One noise for them all, as a fresh one picks its own seed.
    let mut noise = lumit_core::fx::builtins::instantiate("fractal_noise").expect("a built-in");
    set(&mut noise, "scale", float(12.0));
    set(&mut noise, "contrast", float(150.0));
    set(&mut noise, "seed", EffectValue::Seed(7));
    let mut draw = |points, size, analysed, from_named, tilt| {
        render(
            &mut r,
            &scene(&noise, points, size, analysed, from_named, tilt),
        )
    };

    // A layer square on to the camera falls on whole pixels, so at Point size 1
    // the points are the footage to the last pixel, however deep each one is.
    let square = draw(false, 1.0, false, true, 0.0);
    assert!(worst(&draw(true, 1.0, true, true, 0.0), &square) <= 1);
    // With no depth yet they lie on the layer.
    assert!(worst(&draw(true, 1.0, false, true, 0.0), &square) <= 1);

    // Tilted, the compositor filters and the points read the nearest pixel, so
    // tile by tile each is within a pixel at the sizes that close the gaps.
    let flat = draw(false, 1.0, false, true, 12.0);
    for size in [1.0, 2.0] {
        let points = draw(true, size, true, true, 12.0);
        for y0 in (20..H as i32 - 60).step_by(40) {
            for x0 in (20..W as i32 - 200).step_by(40) {
                let (dx, dy) = best_shift(&points, &flat, (x0, y0));
                assert!(
                    dx.abs() <= 1 && dy.abs() <= 1,
                    "at Point size {size} the tile at {x0}, {y0} is {dx}, {dy} out"
                );
            }
        }
    }

    // Seen from the other camera the depth shows.
    let flat = draw(false, 1.0, false, false, 12.0);
    let points = draw(true, 1.0, true, false, 12.0);
    assert!(moved(&points, &flat) > 3, "flat from the other camera");

    // A 3D view is a Camera layer put on top of a copy of the comp. Its points
    // still come from the comp's camera, not from the view.
    let s = scene(&noise, true, 1.0, true, false, 12.0);
    let mut doc = Arc::unwrap_or_clone(s.doc);
    let comp = doc.comp_mut(s.comp).expect("the comp");
    comp.layers[0].id = lumit_core::view_camera::ID;
    let fx = comp.layers[2].effects.last_mut().expect("Depth points");
    set(fx, "camera", EffectValue::Layer(None));
    let viewed = Scene {
        doc: Arc::new(doc),
        comp: s.comp,
    };
    let points = render(&mut r, &viewed);
    assert!(moved(&points, &flat) > 3, "flat in a 3D view");

    // A Depth layer stands in for an analysis, its effects and all.
    let s = scene(&noise, true, 1.0, false, false, 12.0);
    let mut doc = Arc::unwrap_or_clone(s.doc);
    let comp = doc.comp_mut(s.comp).expect("the comp");
    let mut map = comp.layers[2].clone();
    map.id = Uuid::now_v7();
    map.effects = vec![noise.clone()];
    map.switches.visible = false;
    let fx = comp.layers[2].effects.last_mut().expect("Depth points");
    set(fx, "depth_layer", EffectValue::Layer(Some(map.id)));
    comp.layers.push(map);
    let mapped = Scene {
        doc: Arc::new(doc),
        comp: s.comp,
    };
    assert!(
        moved(&render(&mut r, &mapped), &flat) > 3,
        "the Depth layer went unread"
    );

    // The Viewer's present and an export's are the one picture.
    let s = scene(&noise, true, 2.0, true, false, 12.0);
    let q = Quality::default();
    let viewer = r
        .render_preview(&s.doc, s.comp, 0, q, 1.0)
        .expect("a frame")
        .0;
    let export = r.render_preview16(&s.doc, s.comp, 0, q).expect("a frame").0;
    for (eight, sixteen) in viewer.iter().zip(&export) {
        let deep = (f32::from(*sixteen) / 65_535.0 * 255.0).round() as i32;
        assert!((i32::from(*eight) - deep).abs() <= 1);
    }
}

/// Two keys on a property, across the comp's one second.
fn keyed(from: f64, to: f64) -> Property {
    let side = lumit_core::anim::SideInterp::Linear;
    let key = |s: i64, value: f64| lumit_core::anim::Keyframe {
        time: Rational::new(s, 1).unwrap(),
        value,
        interp_in: side,
        interp_out: side,
    };
    Property {
        animation: lumit_core::anim::Animation::Keyframed(vec![key(0, from), key(1, to)]),
        extra: serde_json::Map::new(),
    }
}

/// `s` with its comp's top camera out of the way and its second changed by `set`.
fn staged(s: Scene, set: impl FnOnce(&mut lumit_core::model::Layer)) -> Scene {
    let mut doc = Arc::unwrap_or_clone(s.doc);
    let comp = doc.comp_mut(s.comp).expect("the comp");
    comp.layers[0].switches.visible = false;
    set(&mut comp.layers[1]);
    Scene {
        doc: Arc::new(doc),
        comp: s.comp,
    }
}

/// Whether every tile is within a pixel of `flat` at frame `frame`.
fn lined_up(points: &[u8], flat: &[u8], frame: u32) {
    for y0 in (60..H as i32 - 60).step_by(40) {
        for x0 in (60..W as i32 - 60).step_by(40) {
            let (dx, dy) = best_shift(points, flat, (x0, y0));
            assert!(
                dx.abs() <= 1 && dy.abs() <= 1,
                "at frame {frame} the tile at {x0}, {y0} is {dx}, {dy} out"
            );
        }
    }
}

struct Stub;
impl lumit_eval::SourceStamper for Stub {
    fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
        Some((format!("stub:{item}"), (lt * 60.0).round().max(0.0) as u64))
    }
}

#[test]
fn a_moving_camera_shows_the_depth_or_carries_the_shot() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut noise = lumit_core::fx::builtins::instantiate("fractal_noise").expect("a built-in");
    set(&mut noise, "scale", float(12.0));
    set(&mut noise, "contrast", float(150.0));
    set(&mut noise, "seed", EffectValue::Seed(7));
    let mid = f64::from(W) * 0.5;
    let frame = |r: &mut HeadlessRenderer, s: &Scene, at: u64| {
        r.render_rgba(&s.doc, s.comp, at, 1.0).expect("a frame").0
    };
    let moving =
        |c: &mut lumit_core::model::Layer| c.transform.position_x = keyed(mid, mid + 300.0);

    // The comp's own camera, keyed, with the row unset: lined up on the layer's
    // first frame, and the depth shows once it has moved.
    let unset = |analysed| {
        let s = scene(&noise, true, 1.0, analysed, true, 0.0);
        let mut doc = Arc::unwrap_or_clone(s.doc);
        let comp = doc.comp_mut(s.comp).expect("the comp");
        let fx = comp.layers[2].effects.last_mut().expect("Depth points");
        set(fx, "camera", EffectValue::Layer(None));
        staged(
            Scene {
                doc: Arc::new(doc),
                comp: s.comp,
            },
            moving,
        )
    };
    let (points, flat) = (unset(true), unset(false));
    lined_up(&frame(&mut r, &points, 0), &frame(&mut r, &flat, 0), 0);
    assert!(
        moved(&frame(&mut r, &points, 30), &frame(&mut r, &flat, 30)) > 3,
        "no parallax from a moved comp camera"
    );

    // Named, keyed and zooming, by the row's own route: through it the points
    // are the plate on every frame.
    let named = |enabled| {
        let s = scene(&noise, true, 1.0, true, true, 0.0);
        let s = staged(s, moving);
        let mut doc = Arc::unwrap_or_clone(s.doc);
        let comp = doc.comp(s.comp).expect("the comp").clone();
        let camera = comp.layers[1].id;
        let mut effects = comp.layers[2].effects.clone();
        let fx = effects.last_mut().expect("Depth points");
        set(fx, "camera", EffectValue::Layer(None));
        let mut before = comp.layers[2].clone();
        before.effects = effects.clone();
        let fx = effects.last_mut().expect("Depth points");
        set(fx, "camera", EffectValue::Layer(Some(camera)));
        let shared = Arc::new(doc.clone());
        let op =
            lumit_core::fx::effects::depth_points::set_effects(&shared, &comp, &before, effects);
        let c = doc.comp_mut(s.comp).expect("the comp");
        c.layers[2] = before;
        lumit_core::ops::apply(&mut doc, &op).expect("the ops apply");
        let c = doc.comp_mut(s.comp).expect("the comp");
        if let lumit_core::model::LayerKind::Camera { zoom, .. } = &mut c.layers[1].kind {
            let first = zoom.value_at(0.0);
            *zoom = keyed(first, first * 1.5);
        }
        c.layers[2]
            .effects
            .last_mut()
            .expect("Depth points")
            .enabled = enabled;
        Scene {
            doc: Arc::new(doc),
            comp: s.comp,
        }
    };
    let (points, flat) = (named(true), named(false));
    let plate = frame(&mut r, &flat, 0);
    for at in [0, 30, 59] {
        lined_up(&frame(&mut r, &points, at), &plate, at as u32);
    }

    // A solved camera whose lens is nothing like the stored zoom, moving and
    // turning. The row places the plate by the solve's zoom, so through it the
    // points are the plate on every frame. Switching them off keeps it riding.
    let solved = |enabled| {
        let s = scene(&noise, true, 1.0, true, true, 0.0);
        let mut doc = Arc::unwrap_or_clone(s.doc);
        let media = Uuid::now_v7();
        let c = doc.comp_mut(s.comp).expect("the comp");
        let stored = lumit_core::camera::default_zoom(f64::from(W));
        let focal = stored * 1.4;
        let mut shot = lumit_core::edits::base_layer(
            "Shot".into(),
            lumit_core::model::LayerKind::Footage { item: media },
            c.duration.0,
            Default::default(),
        );
        shot.switches.visible = false;
        if let lumit_core::model::LayerKind::Camera { solve_link, .. } = &mut c.layers[1].kind {
            *solve_link = Some(shot.id);
        }
        c.layers[0].switches.visible = false;
        let fx = c.layers[2].effects.last_mut().expect("Depth points");
        set(fx, "camera", EffectValue::Layer(None));
        c.layers.push(shot);
        let poses = (0..60_i64)
            .map(|n| {
                let turn = (n as f64 * 0.2_f64).to_radians();
                let (s, co) = turn.sin_cos();
                lumit_track::solve::SolvedPose {
                    frame: n,
                    rotation: [[co, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, co]],
                    position: [n as f64 * 3.0, -20.0, -focal],
                    segment: 0,
                    focal_px: focal,
                    mean_reprojection_px: 0.0,
                    source: lumit_track::solve::PoseSource::Keyframe,
                }
            })
            .collect();
        lumit_render::track::publish(
            media,
            60.0,
            60,
            lumit_track::solve::CameraSolve {
                poses,
                segments: vec![lumit_track::solve::SolveSegment {
                    first_frame: 0,
                    last_frame: 59,
                    focal_px: focal,
                    ramp: false,
                }],
                points: Vec::new(),
                keyframes: vec![0, 59],
                mean_reprojection_px: 0.0,
                notes: Vec::new(),
            },
        );
        // The row, by the route the panel takes, and then the effect's switch.
        let shared = Arc::new(doc.clone());
        let comp = shared.comp(s.comp).expect("the comp").clone();
        let camera = comp.layers[1].id;
        let before = comp.layers[2].clone();
        let mut effects = before.effects.clone();
        let fx = effects.last_mut().expect("Depth points");
        set(fx, "camera", EffectValue::Layer(Some(camera)));
        let op =
            lumit_core::fx::effects::depth_points::set_effects(&shared, &comp, &before, effects);
        lumit_core::ops::apply(&mut doc, &op).expect("the ops apply");
        let shared = Arc::new(doc.clone());
        let comp = shared.comp(s.comp).expect("the comp").clone();
        assert_eq!(comp.layers[2].parent, Some(camera), "the row parents");
        let mut effects = comp.layers[2].effects.clone();
        effects.last_mut().expect("Depth points").enabled = enabled;
        let op = lumit_core::fx::effects::depth_points::set_effects(
            &shared,
            &comp,
            &comp.layers[2],
            effects,
        );
        lumit_core::ops::apply(&mut doc, &op).expect("the ops apply");
        let c = doc.comp(s.comp).expect("the comp");
        assert_eq!(
            c.layers[2].parent,
            Some(camera),
            "the switch leaves it riding"
        );
        Scene {
            doc: Arc::new(doc),
            comp: s.comp,
        }
    };
    let (points, flat) = (solved(true), solved(false));
    let plate = frame(&mut r, &flat, 0);
    for at in [0, 30, 59] {
        lined_up(&frame(&mut r, &points, at), &plate, at as u32);
    }

    // A collapsed precomp moved and turned takes its points with it.
    let collapsed = |enabled| {
        let s = scene(&noise, true, 1.0, true, true, 0.0);
        let mut doc = Arc::unwrap_or_clone(s.doc);
        let inner = doc.comp_mut(s.comp).expect("the comp");
        let fx = inner.layers[2].effects.last_mut().expect("Depth points");
        set(fx, "camera", EffectValue::Layer(None));
        fx.enabled = enabled;
        let mut outer = doc.comp(s.comp).expect("the comp").clone();
        outer.id = Uuid::now_v7();
        let mut layer = lumit_core::edits::base_layer(
            "Inside".into(),
            lumit_core::model::LayerKind::Precomp { comp: s.comp },
            outer.duration.0,
            lumit_core::model::TransformGroup {
                anchor_x: Property::fixed(f64::from(W) * 0.5),
                anchor_y: Property::fixed(f64::from(H) * 0.5),
                position_x: Property::fixed(f64::from(W) * 0.5 + 60.0),
                position_y: Property::fixed(f64::from(H) * 0.5),
                rotation_y: Property::fixed(14.0),
                ..Default::default()
            },
        );
        layer.switches.three_d = true;
        layer.switches.collapse = true;
        outer.layers = vec![layer];
        let id = outer.id;
        doc.items.push(ProjectItem::Composition(outer));
        Scene {
            doc: Arc::new(doc),
            comp: id,
        }
    };
    let (points, flat) = (collapsed(true), collapsed(false));
    let (points, flat) = (frame(&mut r, &points, 0), frame(&mut r, &flat, 0));
    for y0 in (80..H as i32 - 80).step_by(40) {
        for x0 in (120..W as i32 - 120).step_by(40) {
            let (dx, dy) = best_shift(&points, &flat, (x0, y0));
            assert!(
                dx.abs() <= 1 && dy.abs() <= 1,
                "{x0}, {y0} is {dx}, {dy} out"
            );
        }
    }

    // A hidden parent moving the named camera renames the frame.
    let key = |x: f64| {
        let s = scene(&noise, true, 1.0, true, false, 0.0);
        let mut doc = Arc::unwrap_or_clone(s.doc);
        let comp = doc.comp_mut(s.comp).expect("the comp");
        let mut null = lumit_core::edits::null_layer(comp);
        null.transform.position_x = Property::fixed(x);
        null.switches.visible = false;
        comp.layers[1].parent = Some(null.id);
        comp.layers[1].switches.visible = false;
        comp.layers.push(null);
        let doc = Arc::new(doc);
        let comp = doc.comp(s.comp).expect("the comp");
        lumit_eval::comp_frame_key(&doc, comp, 0.0, lumit_eval::Quality::default(), &Stub)
            .expect("keyable")
    };
    assert_ne!(
        key(10.0),
        key(500.0),
        "the named camera's parent is in the name"
    );
}

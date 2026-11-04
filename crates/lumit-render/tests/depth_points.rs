//! Depth points line up with the footage from the camera they came from: every
//! point is on its pixel's ray, however deep the depth says it is, and from
//! anywhere else they are not flat.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::fx::effects::depth_points::VIEW_CAMERA_ID;
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
    comp.layers[0].id = VIEW_CAMERA_ID;
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

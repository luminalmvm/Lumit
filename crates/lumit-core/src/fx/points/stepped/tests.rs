//! Particulate's walk from the in point, on the host.

use crate::expression::ExpressionContext;
use crate::fx::effects::particulate::Particulate;
use crate::fx::markers::MarkerContext;
use crate::fx::params::Params;
use crate::fx::registry::EffectMetadata;
use crate::fx::tests::{fixed, particulate, resolve_bag};
use crate::mask::MaskPolyline;
use crate::model::{EffectInstance, EffectValue};
use std::sync::Arc;

/// **A collider's walk lands in one place however it is reached**: a frame
/// stepped to from a kept state is the frame walked from the in point, which
/// is what lets an export match the Viewer, and so is one told to stop part
/// way and asked again. And the floor holds. A flock and sparks land in one
/// place too.
#[test]
fn a_stepped_frame_is_the_walk_from_the_start() {
    use super::{stop_when, stream_at, Collider, Inputs, SAME, SPARK_ID};
    use std::sync::atomic::{AtomicU32, Ordering};
    let e = particulate(&[
        ("gravity", fixed(900.0)),
        ("emit_rate", fixed(400.0)),
        ("turbulence_amount", fixed(30.0)),
    ]);
    let floor = 700.0f32;
    let inputs = |e: &EffectInstance, lt: f64, floor: Option<f32>| {
        let bag = resolve_bag(
            std::slice::from_ref(e),
            lt,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        let collider = floor.map(|y| Collider::new(vec![[[0.0, y], [1920.0, y]]], SAME, false));
        Some(Inputs::new(
            Particulate::read(Params::new(&bag)),
            [1920.0, 1080.0],
            MaskPolyline::default(),
            collider,
        ))
    };
    let at = |lt: f64| inputs(&e, lt, Some(floor));
    let dt = 1.0 / 30.0;
    let key = |n: u8| [n; 32];
    // Walked whole, under a name nothing else uses.
    let whole = stream_at(key(201), 43.0 * dt, dt, &at).expect("a walk");
    // Walked past it first, so the frame is three steps on from a kept one.
    stream_at(key(202), 45.0 * dt, dt, &at).expect("a walk");
    let resumed = stream_at(key(202), 43.0 * dt, dt, &at).expect("a walk");
    assert!(!whole.is_empty(), "nothing was born");
    assert_eq!(whole, resumed, "a kept walk landed somewhere else");
    // Stopped after ten steps, then asked again.
    static ASKED: AtomicU32 = AtomicU32::new(0);
    fn tenth() -> bool {
        ASKED.fetch_add(1, Ordering::Relaxed) >= 10
    }
    let before = stop_when(Some(tenth));
    let stopped = stream_at(key(203), 43.0 * dt, dt, &at);
    stop_when(before);
    assert!(stopped.is_none(), "a walk told to stop carried on");
    let again = stream_at(key(203), 43.0 * dt, dt, &at).expect("a walk");
    assert_eq!(whole, again, "a stopped walk went on somewhere else");
    assert!(
        whole.position.iter().all(|p| p[1] <= floor + 0.01),
        "a particle went through the floor"
    );

    // A flock with nothing to hit, and sparks off a floor that kills.
    let rows = [("emit_rate", fixed(400.0)), ("initial_speed", fixed(150.0))];
    let flock = particulate(
        &[
            &rows[..],
            &[
                ("separation", fixed(60.0)),
                ("alignment", fixed(50.0)),
                ("cohesion", fixed(40.0)),
            ],
        ]
        .concat(),
    );
    let sparks = particulate(&[
        ("gravity", fixed(900.0)),
        ("emit_rate", fixed(400.0)),
        ("on_contact", EffectValue::Choice(3)),
        ("sparks", fixed(8.0)),
    ]);
    let plain = particulate(&rows);
    let walk = |e: &EffectInstance, floor: Option<f32>, n: u8| {
        let at = |lt: f64| inputs(e, lt, floor);
        let whole = stream_at(key(n), 43.0 * dt, dt, &at).expect("a walk");
        stream_at(key(n + 1), 45.0 * dt, dt, &at).expect("a walk");
        let resumed = stream_at(key(n + 1), 43.0 * dt, dt, &at).expect("a walk");
        assert_eq!(whole, resumed, "a kept walk landed somewhere else");
        whole
    };
    let flocked = walk(&flock, None, 204);
    assert_ne!(
        flocked,
        walk(&plain, None, 206),
        "the flock steered nothing"
    );
    let sparked = walk(&sparks, Some(floor), 208);
    assert!(
        sparked.id.iter().any(|&id| id & SPARK_ID != 0),
        "nothing threw sparks"
    );
}

/// **A collider turned against the particles in 3D is a surface**: a floor
/// tilted flat under them holds them up, and one facing the way they do is
/// walls.
#[test]
fn a_collider_turned_in_3d_is_a_surface() {
    use super::{stream_at, Collider, Inputs};
    use crate::anim::Property;
    use crate::model::*;
    use crate::time::{Duration, FrameRate, Rational};

    let solid = SolidDef {
        id: uuid::Uuid::now_v7(),
        name: "Solid".into(),
        colour: LinearColour::BLACK,
        width: 1920,
        height: 1080,
        extra: serde_json::Map::new(),
    };
    let item = ProjectItem::Solid(solid.clone());
    let mut comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: uuid::Uuid::now_v7(),
        name: "Scene".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(5, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let layer = |comp: &Composition| {
        let mut l = crate::comp_graph::read_layer(uuid::Uuid::now_v7(), &item, comp).unwrap();
        l.switches.three_d = true;
        l
    };
    let owner = layer(&comp);
    let facing = layer(&comp);
    // Laid flat, a little below the emitter.
    let mut floor = layer(&comp);
    floor.transform.rotation_x = Property::fixed(-90.0);
    floor.transform.position_y = Property::fixed(800.0);
    comp.layers = vec![owner.clone(), facing.clone(), floor.clone()];
    let mut doc = Document::new();
    doc.items.push(item.clone());
    doc.items.push(ProjectItem::Composition(comp.clone()));
    let context = ExpressionContext {
        document: Arc::new(doc.clone()),
        comp: Some(comp.id),
        layer: Some(owner.id),
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    };
    let walls = Collider::of(&doc, &comp, &owner, &facing, 0.0, &context);
    let surface = Collider::of(&doc, &comp, &owner, &floor, 0.0, &context);
    assert!(format!("{walls:?}").contains("surface: false"));
    assert!(format!("{surface:?}").contains("surface: true"));

    let e = particulate(&[
        ("gravity", fixed(900.0)),
        ("emit_rate", fixed(300.0)),
        ("position_y", fixed(500.0)),
        ("turbulence_amount", fixed(0.0)),
    ]);
    // Trails off, and the walk is the collider's alone.
    assert!(!crate::fx::points::stepped::trails(&comp, &e));
    let at = |lt: f64| {
        let bag = resolve_bag(
            std::slice::from_ref(&e),
            lt,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        Some(Inputs::new(
            Particulate::read(Params::new(&bag)),
            [1920.0, 1080.0],
            MaskPolyline::default(),
            Some(surface.clone()),
        ))
    };
    let dt = 1.0 / 30.0;
    let held = stream_at([231; 32], 40.0 * dt, dt, &at).expect("a walk");
    assert!(!held.is_empty(), "nothing was born");
    assert!(
        held.position.iter().all(|p| p[1] <= 800.01),
        "a particle went through the floor"
    );
}

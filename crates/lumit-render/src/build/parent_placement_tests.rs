//! Tests of parented placement.

use super::*;
use lumit_core::anim::Property;
use lumit_core::model::*;
use lumit_core::{time::CompTime, time::Duration, time::FrameRate, time::Rational};

fn layer(px: f64, py: f64, parent: Option<uuid::Uuid>) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: uuid::Uuid::now_v7(),
        name: "l".into(),
        kind: LayerKind::Solid {
            def: uuid::Uuid::now_v7(),
        },
        in_point: CompTime(Rational::new(0, 1).unwrap()),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::new(0, 1).unwrap()),
        transform: TransformGroup {
            position_x: Property::fixed(px),
            position_y: Property::fixed(py),
            ..TransformGroup::default()
        },
        matte: None,
        parent,
        label: 0,
        volume_db: lumit_core::anim::Property::zero(),
        pan: lumit_core::anim::Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        graph_inputs: None,
        blend: BlendMode::Normal,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

fn place_of(l: &Layer) -> [[f32; 4]; 4] {
    let tr = &l.transform;
    lumit_gpu::composite::place_matrix(
        (
            tr.position_x.value_at(0.0) as f32,
            tr.position_y.value_at(0.0) as f32,
        ),
        (0.0, 0.0),
        (100.0, 100.0),
        0.0,
        0.0,
        0.0,
        0.0,
    )
}

fn comp(layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: uuid::Uuid::now_v7(),
        name: "c".into(),
        width: 100,
        height: 100,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

#[test]
fn unparented_is_none_and_a_chain_composes_top_outermost() {
    let gp = layer(10.0, 20.0, None);
    let parent = layer(100.0, 0.0, Some(gp.id));
    let child = layer(5.0, 5.0, Some(parent.id));
    let c = comp(vec![gp.clone(), parent.clone(), child.clone()]);
    // No parent → no placement.
    assert!(
        parent_world_placement(&c, &gp, 0.0, Arc::new(ExpressionContext::detached())).is_none()
    );
    // The child's world placement is grandparent × parent (top outermost),
    // exactly the manual concat — proving the walk and fold order.
    let world =
        parent_world_placement(&c, &child, 0.0, Arc::new(ExpressionContext::detached())).unwrap();
    let expected = lumit_gpu::composite::concat_place(place_of(&gp), place_of(&parent));
    assert_eq!(world, expected);
    // A 2D grandparent's hidden z and tilt leave the chain as it was.
    let mut tilted = gp.clone();
    tilted.transform.position_z = Property::fixed(-50.0);
    tilted.transform.rotation_x = Property::fixed(60.0);
    let c = comp(vec![tilted, parent, child.clone()]);
    let flat = parent_world_placement(&c, &child, 0.0, Arc::new(ExpressionContext::detached()));
    assert_eq!(flat, Some(expected));

    // A layer on a two-node camera turns with its aim, and an imported
    // camera's anchor (After Effects' point of interest) moves nothing.
    let mut camera = layer(100.0, 50.0, None);
    camera.transform.anchor_x = Property::fixed(400.0);
    camera.transform.position_z = Property::fixed(-300.0);
    camera.kind = LayerKind::Camera {
        zoom: Property::fixed(800.0),
        solve_link: None,
        correction_base: None,
        options: Box::new(CameraOptions {
            two_node: true,
            ..CameraOptions::fresh(800.0, (400.0, 0.0, 0.0))
        }),
    };
    let mut hud = layer(0.0, 0.0, Some(camera.id));
    hud.switches.three_d = true;
    let c = comp(vec![camera, hud.clone()]);
    let aim =
        lumit_core::camera::look_at((100.0, 50.0, -300.0), (400.0, 0.0, 0.0), (0.0, 0.0, 0.0))
            .unwrap();
    let expected = lumit_gpu::composite::place_matrix(
        (100.0, 50.0),
        (0.0, 0.0),
        (100.0, 100.0),
        aim.2 as f32,
        -300.0,
        aim.0 as f32,
        aim.1 as f32,
    );
    let on_camera = parent_world_placement(&c, &hud, 0.0, Arc::new(ExpressionContext::detached()));
    assert_eq!(on_camera, Some(expected));
}

#[test]
fn taking_and_losing_a_parent_leaves_the_layer_where_it_was() {
    let ctx = || Arc::new(ExpressionContext::detached());
    let world = |c: &Composition, l: &Layer| {
        let own = own_placement(l, 0.0, &ctx());
        match parent_world_placement(c, l, 0.0, ctx()) {
            Some(pre) => lumit_gpu::composite::concat_place(pre, own),
            None => own,
        }
    };
    // A scaled parent and a tilted one are separate rigs: scale has no z,
    // so a tilt under a scaled parent has no exact answer.
    for tilted in [false, true] {
        let (tilt, scale) = if tilted { (15.0, 100.0) } else { (0.0, 50.0) };
        let mut gp = layer(10.0, 20.0, None);
        gp.transform.rotation_y = Property::fixed(tilt);
        let mut parent = layer(100.0, 40.0, Some(gp.id));
        parent.transform.anchor_x = Property::fixed(7.0);
        parent.transform.rotation = Property::fixed(30.0);
        parent.transform.scale_x = Property::fixed(scale);
        parent.transform.scale_y = Property::fixed(scale);
        // Flipped, more than a turn round and tilted: the awkward ones to
        // read back out of a matrix.
        let mut child = layer(5.0, 60.0, None);
        child.transform.anchor_y = Property::fixed(4.0);
        child.transform.rotation = Property::fixed(400.0);
        child.transform.rotation_x = Property::fixed(tilt);
        child.transform.scale_x = Property::fixed(80.0);
        child.transform.scale_y = Property::fixed(-80.0);
        // A tilt only counts on a 3D layer.
        gp.switches.three_d = tilted;
        child.switches.three_d = tilted;

        let before = world(&comp(vec![child.clone()]), &child);
        for new_parent in [Some(parent.id), None] {
            let c = comp(vec![gp.clone(), parent.clone(), child.clone()]);
            let edits = reparented(&c, &child, new_parent, 0.0, ctx());
            assert!(!edits.is_empty());
            for (prop, animation) in edits {
                child.transform.get_mut(prop).unwrap().animation = animation;
            }
            child.parent = new_parent;
            let c = comp(vec![gp.clone(), parent.clone(), child.clone()]);
            let after = world(&c, &child);
            for (b, a) in before.iter().flatten().zip(after.iter().flatten()) {
                assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
            }
        }
        // Back where it started, in the numbers too.
        assert!((child.transform.rotation.value_at(0.0) - 400.0).abs() < 1e-2);
        assert!((child.transform.scale_y.value_at(0.0) + 80.0).abs() < 1e-2);

        // A camera keeps its eye, its turn and what it is aimed at, under
        // the same parents.
        for two_node in [false, true] {
            let mut camera = layer(30.0, 70.0, None);
            camera.kind = LayerKind::Camera {
                zoom: Property::fixed(800.0),
                solve_link: None,
                correction_base: None,
                options: Box::new(CameraOptions {
                    two_node,
                    ..CameraOptions::fresh(800.0, (90.0, 20.0, 300.0))
                }),
            };
            camera.transform.position_z = Property::fixed(-500.0);
            camera.transform.rotation_x = Property::fixed(12.0);
            camera.transform.rotation = Property::fixed(-25.0);
            let seen = |c: &Composition, l: &Layer| {
                let pose = c.camera_pose(0.0).unwrap();
                let own = |p: TransformProp| l.prop(p).unwrap().value_at(0.0);
                let poi = (
                    own(TransformProp::PoiX),
                    own(TransformProp::PoiY),
                    own(TransformProp::PoiZ),
                );
                // A one-node camera's point of interest is inert.
                let context = ctx();
                let read = lumit_core::model::context_reader(&context, 0.0);
                let poi = match parent_space(c, l, 0.0, &read, None) {
                    _ if !two_node => (0.0, 0.0, 0.0),
                    Some(s) => s.point(poi),
                    None => poi,
                };
                let axes = lumit_core::camera::axes(pose.rotation_deg);
                [pose.position, poi, axes[0], axes[1], axes[2]]
            };
            let before = seen(&comp(vec![camera.clone()]), &camera);
            for new_parent in [Some(parent.id), None] {
                let c = comp(vec![camera.clone(), gp.clone(), parent.clone()]);
                let edits = reparented(&c, &camera, new_parent, 0.0, ctx());
                assert!(!edits.is_empty());
                for (prop, animation) in edits {
                    camera.prop_mut(prop).unwrap().animation = animation;
                }
                camera.parent = new_parent;
                let c = comp(vec![camera.clone(), gp.clone(), parent.clone()]);
                let after = seen(&c, &camera);
                for (b, a) in before.iter().zip(&after) {
                    let gap = (b.0 - a.0).abs() + (b.1 - a.1).abs() + (b.2 - a.2).abs();
                    assert!(gap < 1e-2, "{two_node}: {before:?} became {after:?}");
                }
            }
        }
    }

    // A keyed camera keeps its whole move under a null turned a quarter
    // round, where x and z trade places: every key is carried, not just
    // the one under the playhead, and an eased move keeps its ease.
    let mut null = layer(200.0, 100.0, None);
    null.switches.three_d = true;
    null.transform.rotation_y = Property::fixed(90.0);
    let mut camera = layer(0.0, 50.0, None);
    camera.kind = LayerKind::Camera {
        zoom: Property::fixed(800.0),
        solve_link: None,
        correction_base: None,
        options: Box::new(CameraOptions::fresh(800.0, (0.0, 0.0, 0.0))),
    };
    let key = |s: i64, v: f64| lumit_core::anim::Keyframe {
        time: Rational::new(s, 1).unwrap(),
        value: v,
        interp_in: lumit_core::anim::SideInterp::Linear,
        interp_out: lumit_core::anim::SideInterp::Linear,
    };
    let keyed = |a: f64, b: f64| Property {
        animation: lumit_core::anim::Animation::Keyframed(vec![key(0, a), key(2, b)]),
        ..Property::fixed(0.0)
    };
    let eased = |a: f64, b: f64, speed: f64| {
        let ease = lumit_core::anim::SideInterp::Bezier {
            speed,
            influence: 0.4,
        };
        let mut keys = [key(0, a), key(2, b)];
        keys[0].interp_out = ease;
        keys[1].interp_in = ease;
        Property {
            animation: lumit_core::anim::Animation::Keyframed(keys.to_vec()),
            ..Property::fixed(0.0)
        }
    };
    camera.transform.position_x = eased(0.0, 300.0, 400.0);
    camera.transform.position_y = eased(50.0, 50.0, 0.0);
    camera.transform.position_z = eased(-500.0, -200.0, -100.0);
    camera.transform.rotation_y = keyed(0.0, 40.0);
    camera.transform.orientation[0] = Property::fixed(10.0);
    camera.transform.orientation[2] = Property {
        animation: lumit_core::anim::Animation::Keyframed(vec![key(1, 0.0), key(3, 30.0)]),
        ..Property::fixed(0.0)
    };
    let poses = |c: &Composition| [0.0, 0.5, 1.0, 2.0].map(|t| c.camera_pose(t).unwrap());
    let before = poses(&comp(vec![camera.clone(), null.clone()]));
    let c = comp(vec![camera.clone(), null.clone()]);
    for (prop, animation) in reparented(&c, &camera, Some(null.id), 1.0, ctx()) {
        camera.prop_mut(prop).unwrap().animation = animation;
    }
    camera.parent = Some(null.id);
    let after = poses(&comp(vec![camera.clone(), null.clone()]));
    for (b, a) in before.iter().zip(&after) {
        let (pb, pa) = (b.position, a.position);
        let gap = (pb.0 - pa.0).abs() + (pb.1 - pa.1).abs() + (pb.2 - pa.2).abs();
        // Every axis, so a roll shows as well as where it looks.
        let (ab, aa) = (
            lumit_core::camera::axes(b.rotation_deg),
            lumit_core::camera::axes(a.rotation_deg),
        );
        let turn: f64 = ab
            .iter()
            .zip(&aa)
            .map(|(u, v)| (u.0 - v.0).abs() + (u.1 - v.1).abs() + (u.2 - v.2).abs())
            .sum();
        assert!(gap < 1e-2 && turn < 1e-6, "{b:?} became {a:?}");
    }
    // The rows keep their keys, and the Orientation keeps its own two
    // rather than gaining one at every position key.
    assert_eq!(camera.transform.rotation_y, keyed(0.0, 40.0));
    for p in &camera.transform.orientation {
        let times: Vec<_> = match &p.animation {
            lumit_core::anim::Animation::Keyframed(keys) => {
                keys.iter().map(|k| k.time.to_f64()).collect()
            }
            _ => Vec::new(),
        };
        assert_eq!(times, [1.0, 3.0], "{p:?}");
    }

    // After Effects' own numbers: a 3D layer put under a turned null
    // keeps its X, Y and Z Rotation, and Orientation takes the turn.
    let mut rider = layer(1000.0, 600.0, None);
    rider.switches.three_d = true;
    rider.transform.after_effects_order = true;
    rider.transform.anchor_x = Property::fixed(400.0);
    rider.transform.anchor_y = Property::fixed(200.0);
    rider.transform.rotation_x = Property::fixed(30.0);
    rider.transform.rotation_y = Property::fixed(40.0);
    let mut turner = layer(800.0, 500.0, None);
    turner.switches.three_d = true;
    turner.transform.position_z = Property::fixed(100.0);
    turner.transform.orientation[0] = Property::fixed(20.0);
    turner.transform.rotation_y = Property::fixed(25.0);
    let before = world(&comp(vec![rider.clone()]), &rider);
    let c = comp(vec![rider.clone(), turner.clone()]);
    for (prop, animation) in reparented(&c, &rider, Some(turner.id), 0.0, ctx()) {
        rider.transform.get_mut(prop).unwrap().animation = animation;
    }
    rider.parent = Some(turner.id);
    let after = world(&comp(vec![rider.clone(), turner.clone()]), &rider);
    for (b, a) in before.iter().flatten().zip(after.iter().flatten()) {
        assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
    }
    let tr = &rider.transform;
    let read = |p: &Property| p.value_at(0.0);
    let turn = |a: f64, b: f64| ((a - b) % 360.0 + 360.0) % 360.0;
    for (got, want) in [
        (read(&tr.rotation_x), 30.0),
        (read(&tr.rotation_y), 40.0),
        (read(&tr.rotation), 0.0),
        (read(&tr.orientation[0]), 338.1198),
        (read(&tr.orientation[1]), 336.6010),
        (read(&tr.orientation[2]), 350.9384),
    ] {
        let off = turn(got, want);
        assert!(off.min(360.0 - off) < 1e-2, "{got} for {want}");
    }
    let at = (
        read(&tr.position_x),
        read(&tr.position_y),
        read(&tr.position_z),
    );
    let want = (235.4291, 59.7672, -31.6390);
    let gap = (at.0 - want.0).abs() + (at.1 - want.1).abs() + (at.2 - want.2).abs();
    assert!(gap < 1e-2, "{at:?}");

    // Under a mirrored parent with an Orientation of its own, there and
    // back again.
    let mut mirror = turner.clone();
    mirror.id = uuid::Uuid::now_v7();
    mirror.transform.scale_x = Property::fixed(-100.0);
    mirror.transform.orientation = [15.0, -30.0, 10.0].map(Property::fixed);
    let mut card = layer(1000.0, 600.0, None);
    card.switches.three_d = true;
    card.transform.anchor_x = Property::fixed(400.0);
    card.transform.anchor_y = Property::fixed(200.0);
    card.transform.rotation_x = Property::fixed(30.0);
    card.transform.rotation_y = Property::fixed(40.0);
    card.transform.orientation = [5.0, 0.0, 20.0].map(Property::fixed);
    let before = world(&comp(vec![card.clone()]), &card);
    for new_parent in [Some(mirror.id), None] {
        let c = comp(vec![card.clone(), mirror.clone()]);
        for (prop, animation) in reparented(&c, &card, new_parent, 0.0, ctx()) {
            card.transform.get_mut(prop).unwrap().animation = animation;
        }
        card.parent = new_parent;
        let after = world(&comp(vec![card.clone(), mirror.clone()]), &card);
        for (b, a) in before.iter().flatten().zip(after.iter().flatten()) {
            assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
        }
    }

    // A keyed Orientation has every key turned, so the whole move stays.
    let mut spun = layer(1000.0, 600.0, None);
    spun.switches.three_d = true;
    spun.transform.rotation_x = keyed(30.0, -10.0);
    spun.transform.orientation = [keyed(0.0, 60.0), keyed(10.0, 80.0), keyed(0.0, 20.0)];
    let worlds = |c: &Composition, l: &Layer| {
        [0.0, 0.5, 1.0, 2.0].map(|t| {
            let own = own_placement(l, t, &ctx());
            match parent_world_placement(c, l, t, ctx()) {
                Some(pre) => lumit_gpu::composite::concat_place(pre, own),
                None => own,
            }
        })
    };
    let before = worlds(&comp(vec![spun.clone()]), &spun);
    let c = comp(vec![spun.clone(), turner.clone()]);
    for (prop, animation) in reparented(&c, &spun, Some(turner.id), 1.0, ctx()) {
        assert!(!matches!(prop, TransformProp::RotationX));
        spun.transform.get_mut(prop).unwrap().animation = animation;
    }
    spun.parent = Some(turner.id);
    let after = worlds(&comp(vec![spun.clone(), turner.clone()]), &spun);
    for (b, a) in before
        .iter()
        .flatten()
        .flatten()
        .zip(after.iter().flatten().flatten())
    {
        assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
    }
}

/// A 2D layer pinned to a 3D null with `toComp` sits where the compositor
/// draws the null, through parents and a turned camera, and `fromComp`
/// takes a picture point back to the null's own pixels. A 2D parent's
/// leftover z and tilt count for nothing, as they do in the picture.
/// Orientation, Anchor Z and Scale Z are on the way, and the picture's own
/// placement, with Anchor Z in its position, lands where the walk does.
#[test]
#[allow(clippy::panic)]
fn to_comp_lands_where_the_compositor_draws_and_from_comp_takes_it_back() {
    use lumit_core::expression::{evaluate_value_in, script::Language, ExprValue};

    let mut flat = layer(40.0, -30.0, None);
    flat.transform.position_z = Property::fixed(-50.0);
    flat.transform.rotation_x = Property::fixed(60.0);
    flat.transform.rotation = Property::fixed(10.0);
    let mut card = layer(15.0, 25.0, Some(flat.id));
    card.name = "Card".into();
    card.transform.rotation_y = Property::fixed(40.0);
    let mut rig = layer(900.0, 500.0, Some(flat.id));
    rig.switches.three_d = true;
    rig.transform.position_z = Property::fixed(300.0);
    rig.transform.anchor_x = Property::fixed(20.0);
    rig.transform.rotation = Property::fixed(-12.0);
    rig.transform.rotation_y = Property::fixed(35.0);
    rig.transform.scale_x = Property::fixed(150.0);
    rig.transform.scale_y = Property::fixed(80.0);
    rig.transform.orientation = [10.0, -25.0, 5.0].map(Property::fixed);
    rig.transform.anchor_z = Property::fixed(30.0);
    rig.transform.scale_z = Property::fixed(60.0);
    let mut marker = layer(120.0, -60.0, Some(rig.id));
    marker.name = "Marker".into();
    marker.switches.three_d = true;
    marker.transform.position_z = Property::fixed(-80.0);
    marker.transform.rotation_x = Property::fixed(20.0);
    marker.transform.orientation[1] = Property::fixed(20.0);
    marker.transform.anchor_z = Property::fixed(-15.0);
    let mut camera = layer(700.0, 450.0, None);
    camera.kind = LayerKind::Camera {
        zoom: Property::fixed(1400.0),
        options: Box::default(),
        solve_link: None,
        correction_base: None,
    };
    camera.transform.position_z = Property::fixed(-1500.0);
    camera.transform.rotation_x = Property::fixed(-10.0);
    camera.transform.rotation_y = Property::fixed(15.0);
    camera.transform.rotation = Property::fixed(5.0);
    // Below the active camera, a two-node one with a layer on it, and a
    // null moved by an expression on time with a layer on that.
    let mut aimer = layer(300.0, 200.0, None);
    aimer.kind = LayerKind::Camera {
        zoom: Property::fixed(900.0),
        options: Box::new(CameraOptions {
            two_node: true,
            ..CameraOptions::fresh(900.0, (800.0, 600.0, 400.0))
        }),
        solve_link: None,
        correction_base: None,
    };
    aimer.transform.position_z = Property::fixed(-700.0);
    aimer.transform.rotation = Property::fixed(20.0);
    let mut hud = layer(50.0, -40.0, Some(aimer.id));
    hud.name = "Hud".into();
    hud.switches.three_d = true;
    hud.transform.position_z = Property::fixed(600.0);
    let mut drift = layer(0.0, 300.0, None);
    drift.switches.three_d = true;
    drift.transform.position_x = Property {
        animation: lumit_core::anim::Animation::Expression("100.0 + time * 40.0".into()),
        ..Property::fixed(0.0)
    };
    drift.transform.rotation_y = Property::fixed(25.0);
    let mut rider = layer(60.0, 10.0, Some(drift.id));
    rider.name = "Rider".into();
    rider.switches.three_d = true;
    let mut c = comp(vec![
        camera,
        flat,
        card.clone(),
        rig,
        marker.clone(),
        aimer,
        hud,
        drift,
        rider,
    ]);
    (c.width, c.height) = (1920, 1080);
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(c.clone()));
    let context = Arc::new(ExpressionContext {
        document: Arc::new(doc),
        comp: Some(c.id),
        layer: None,
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    });
    let run = |language, text: &str| match evaluate_value_in(
        language,
        text,
        Some(context.clone()),
        &[],
    ) {
        Ok(ExprValue::Number(n)) => n,
        other => panic!("{text}: {other:?}"),
    };

    let world = world_placement(&c, &marker, 0.0, context.clone());
    let d = drawn(&marker, 0.0, &context);
    let own = lumit_gpu::composite::place_matrix(
        d.position,
        d.anchor,
        d.scale,
        d.rotation_deg,
        d.z,
        d.rotation_x_deg,
        d.rotation_y_deg,
    );
    let pre = parent_world_placement(&c, &marker, 0.0, context.clone()).unwrap();
    let picture = lumit_gpu::composite::concat_place(pre, own);
    for (x, y) in [(0.0, 0.0), (64.0, 0.0), (0.0, 48.0)] {
        let at = |m: &[[f32; 4]; 4], r: usize| m[0][r] * x + m[1][r] * y + m[3][r];
        for r in 0..3 {
            assert!((at(&world, r) - at(&picture, r)).abs() < 1e-2, "{x}, {y}");
        }
    }
    let pose = c.camera_pose(0.0).unwrap();
    let cam = crate::export::camera_mat(c.width, c.height, pose);
    let a = (cam * glam::Mat4::from_cols_array_2d(&world)).to_cols_array_2d();
    // After Effects' third number, zoom · (1 − 1/z), from the same divide:
    // the compositor's w is z over the zoom.
    let depth = pose.zoom - 1.0 / f64::from(a[3][3]);
    let drawn = [
        f64::from(a[3][0] / a[3][3]),
        f64::from(a[3][1] / a[3][3]),
        depth,
    ];
    for (axis, want) in drawn.into_iter().enumerate() {
        let js = run(
            Language::JavaScript,
            &format!("thisComp.layer('Marker').toComp([0, 0, 0])[{axis}]"),
        );
        let rhai = run(
            Language::Rhai,
            &format!("layer(\"Marker\").to_comp([0, 0, 0])[{axis}]"),
        );
        for got in [js, rhai] {
            assert!((got - want).abs() < 1e-2, "{got} is not {want}");
        }
    }
    // A 2D layer is drawn flat, with no camera.
    let flat_world = world_placement(&c, &card, 0.0, context.clone());
    let on_card = |r: usize| flat_world[0][r] * 5.0 + flat_world[1][r] * 7.0 + flat_world[3][r];
    for (axis, want) in [on_card(0), on_card(1)].into_iter().enumerate() {
        let got = run(
            Language::JavaScript,
            &format!("thisComp.layer('Card').toComp([5, 7])[{axis}]"),
        );
        assert!((got - f64::from(want)).abs() < 1e-2, "{got} is not {want}");
    }
    for (axis, want) in [30.0, -40.0, 25.0].into_iter().enumerate() {
        let back = run(
            Language::JavaScript,
            &format!(
                "var m = thisComp.layer('Marker'); m.fromComp(m.toComp([30, -40, 25]))[{axis}]"
            ),
        );
        assert!((back - want).abs() < 1e-6, "{back} is not {want}");
    }
    for (axis, want) in [30.0, -40.0].into_iter().enumerate() {
        let back = run(
                Language::JavaScript,
                &format!(
                    "var m = thisComp.layer('Marker'); m.fromCompToSurface(m.toComp([30, -40]))[{axis}]"
                ),
            );
        assert!((back - want).abs() < 1e-6, "{back} is not {want}");
    }

    // A layer on a two-node camera turns with the camera's aim, and one
    // on a null moved by `time` follows it at that time.
    let later = Arc::new(ExpressionContext {
        comp_time: 0.5,
        ..(*context).clone()
    });
    for (name, t, at) in [("Hud", 0.0, &context), ("Rider", 0.5, &later)] {
        let l = c.layers.iter().find(|l| l.name == name).unwrap();
        let world = world_placement(&c, l, t, at.clone());
        let a = (cam * glam::Mat4::from_cols_array_2d(&world)).to_cols_array_2d();
        for axis in 0..2 {
            let want = a[3][axis] / a[3][3];
            let text = format!("thisComp.layer('{name}').toComp([0, 0, 0])[{axis}]");
            let got = match evaluate_value_in(Language::JavaScript, &text, Some(at.clone()), &[]) {
                Ok(ExprValue::Number(n)) => n,
                other => panic!("{text}: {other:?}"),
            };
            assert!(
                (got - f64::from(want)).abs() < 1e-2,
                "{name}: {got} is not {want}"
            );
        }
    }
}

/// The rig of aecheck's tocomp-check.jsx, and After Effects' own numbers
/// for it: toComp's third number, fromComp off it, the Vec forms, a point
/// behind the eye, and the facing sign of toCompVec.
#[test]
#[allow(clippy::panic)]
fn to_comp_reads_what_after_effects_reads() {
    use lumit_core::expression::{evaluate_value_in, script::Language, ExprValue};

    let mut rig = layer(900.0, 500.0, None);
    rig.switches.three_d = true;
    rig.transform.position_z = Property::fixed(300.0);
    rig.transform.anchor_x = Property::fixed(20.0);
    rig.transform.rotation = Property::fixed(-12.0);
    rig.transform.rotation_y = Property::fixed(35.0);
    rig.transform.scale_x = Property::fixed(150.0);
    rig.transform.scale_y = Property::fixed(80.0);
    let mut marker = layer(120.0, -60.0, Some(rig.id));
    marker.name = "Marker".into();
    marker.switches.three_d = true;
    marker.transform.position_z = Property::fixed(-80.0);
    marker.transform.rotation_x = Property::fixed(20.0);
    let mut face = layer(960.0, 540.0, None);
    face.name = "Face".into();
    face.switches.three_d = true;
    let mut back = face.clone();
    back.id = uuid::Uuid::now_v7();
    back.name = "Back".into();
    back.transform.rotation_y = Property::fixed(180.0);
    let mut camera = layer(700.0, 450.0, None);
    camera.kind = LayerKind::Camera {
        zoom: Property::fixed(1400.0),
        options: Box::default(),
        solve_link: None,
        correction_base: None,
    };
    camera.transform.position_z = Property::fixed(-1500.0);
    camera.transform.rotation_y = Property::fixed(15.0);
    camera.transform.rotation = Property::fixed(5.0);
    let mut c = comp(vec![camera, rig, marker, face, back]);
    (c.width, c.height) = (1920, 1080);
    let pose = c.camera_pose(0.0).unwrap();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(c.clone()));
    let context = Arc::new(ExpressionContext {
        document: Arc::new(doc),
        comp: Some(c.id),
        layer: None,
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    });
    let run = |language, text: &str| match evaluate_value_in(
        language,
        text,
        Some(context.clone()),
        &[],
    ) {
        Ok(ExprValue::Number(n)) => n,
        other => panic!("{text}: {other:?}"),
    };

    // 100 px behind the eye, as the camera sees it.
    let m = lumit_core::camera::rotation(pose.rotation_deg);
    let a = (30.0, 20.0, -100.0);
    let eye = [pose.position.0, pose.position.1, pose.position.2];
    let at = |r: usize| eye[r] + m[r][0] * a.0 + m[r][1] * a.1 + m[r][2] * a.2;
    let behind = format!("[{}, {}, {}]", at(0), at(1), at(2));
    let behind_js = format!("toComp(thisComp.layer('Marker').fromWorld({behind}))");
    let behind_rhai = format!("to_comp(layer(\"Marker\").from_world({behind}))");
    for (in_js, in_rhai, want) in [
        (
            "toComp([0, 0, 0])",
            "to_comp([0, 0, 0])",
            [814.5410, 529.0258, 1399.1609],
        ),
        (
            "toComp([30, -40, 25])",
            "to_comp([30, -40, 25])",
            [842.3415, 488.1836, 1399.1593],
        ),
        (
            "fromComp([960, 540, 0])",
            "from_comp([960, 540, 0])",
            [473.1247, -304.8729, -1494.0807],
        ),
        (
            "fromComp([960, 540])",
            "from_comp([960, 540])",
            [473.1247, -304.8729, -1494.0807],
        ),
        (
            "fromComp([960, 540, -1400])",
            "from_comp([960, 540, -1400])",
            [473.2362, -304.9918, -1494.5374],
        ),
        (
            "fromComp([100, 900, 1399])",
            "from_comp([100, 900, 1399])",
            [-422.4201, 46.1342, -546.0801],
        ),
        (
            "toCompVec([10, 20, 30])",
            "to_comp_vec([10, 20, 30])",
            [20.9047, 7.2530, 46218.5615],
        ),
        (
            "fromCompVec([10, 20, 30])",
            "from_comp_vec([10, 20, 30])",
            [-17.5206, 29.4023, 36.8613],
        ),
        (
            "fromCompVec([10, 20])",
            "from_comp_vec([10, 20])",
            [-2.4660, 21.3642, 9.7869],
        ),
        (
            behind_js.as_str(),
            behind_rhai.as_str(),
            [540.0, 260.0, 1414.0],
        ),
    ] {
        for (axis, want) in want.into_iter().enumerate() {
            let js = run(
                Language::JavaScript,
                &format!("thisComp.layer('Marker').{in_js}[{axis}]"),
            );
            let rhai = run(
                Language::Rhai,
                &format!("layer(\"Marker\").{in_rhai}[{axis}]"),
            );
            for got in [js, rhai] {
                assert!((got - want).abs() < 2e-3, "{in_js}: {got} is not {want}");
            }
        }
    }
    // The manual's promise: above nought while the front faces the camera.
    let facing = |name: &str| {
        run(
            Language::JavaScript,
            &format!("thisComp.layer('{name}').toCompVec([0, 0, 1])[2]"),
        )
    };
    assert!(facing("Face") > 0.0);
    assert!(facing("Back") < 0.0);
}

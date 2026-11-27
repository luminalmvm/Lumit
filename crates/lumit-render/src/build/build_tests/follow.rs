use super::*;

#[test]
fn an_echo_trails_the_layers_own_motion() {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    let solid_id = Uuid::now_v7();
    let key = |t: i64, value: f64| Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let mut blur = lumit_core::fx::builtins::instantiate("blur").unwrap();
    blur.enabled = true;
    let echo = lumit_core::fx::builtins::instantiate("echo").unwrap();
    let transform = TransformGroup {
        position_x: Property {
            animation: Animation::Keyframed(vec![key(0, 20.0), key(1, 80.0)]),
            ..Property::zero()
        },
        ..TransformGroup::default()
    };
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "solid".into(),
        kind: LayerKind::Solid { def: solid_id },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform,
        matte: None,
        parent: None,
        label: 0,
        volume_db: Property::zero(),
        pan: Property::zero(),
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
        grade_versions: None,
        graph_inputs: None,
        blend: lumit_core::model::BlendMode::Screen,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: vec![blur, echo],
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        blending: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 100,
        height: 40,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_id,
            name: "White".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 10,
            height: 10,
            extra: serde_json::Map::new(),
        },
    ));
    doc.items
        .push(lumit_core::model::ProjectItem::Composition(comp.clone()));
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc),
        &comp,
        0.5,
        &map,
        &mut vec![comp.id],
    );
    assert_eq!(draws.len(), 1);
    let trail = &draws[0];
    assert_eq!(trail.layer, layer.id);
    assert_eq!(
        trail.blend,
        crate::build::blend_of(lumit_core::model::BlendMode::Screen),
        "the layer's blend mode applies to the whole trail"
    );
    assert_eq!(trail.position, (50.0, 20.0), "comp-sized and unmoved");
    let DrawSource::Nested { draws: now, .. } = &trail.source else {
        panic!("the trail is a comp-sized picture");
    };
    assert_eq!(now.len(), 1);
    assert!(
        (now[0].position.0 - 50.0).abs() < 1e-3,
        "{:?}",
        now[0].position
    );
    assert_eq!(now[0].fx_ids.len(), 1, "only the Blur runs inside");
    assert_eq!(
        trail.fx_ids,
        vec![layer.effects[1].id],
        "the Echo runs on the picture"
    );
    let offsets: Vec<i32> = trail.flow_below.iter().map(|n| n.0).collect();
    assert_eq!(offsets, vec![-4, -3, -2, -1], "one picture per echo");
    for (offset, then, _, _) in &trail.flow_below {
        let x = 20.0 + 60.0 * (0.5 + f64::from(*offset) / 30.0);
        assert!(
            (f64::from(then[0].position.0) - x).abs() < 0.05,
            "echo {offset} sits where the solid was: {:?} vs {x}",
            then[0].position
        );
        assert_eq!(then[0].fx_ids.len(), 1, "and is blurred as it was then");
    }

    struct Stub;
    impl lumit_eval::SourceStamper for Stub {
        fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((format!("stub:{item}"), (lt * 60.0).round() as u64))
        }
    }
    let name_of = |doc: &Document, comp: &Composition| {
        lumit_eval::comp_frame_key(
            &std::sync::Arc::new(doc.clone()),
            comp,
            0.5,
            lumit_eval::Quality::default(),
            &Stub,
        )
        .unwrap()
    };
    let early = |comp: &Composition| {
        let mut comp = comp.clone();
        comp.layers[0].transform.position_x = Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: Rational::new(14, 30).unwrap(),
                    value: 30.0,
                    interp_in: SideInterp::Hold,
                    interp_out: SideInterp::Hold,
                },
                Keyframe {
                    time: Rational::new(15, 30).unwrap(),
                    value: 50.0,
                    interp_in: SideInterp::Hold,
                    interp_out: SideInterp::Hold,
                },
            ]),
            ..Property::zero()
        };
        comp
    };
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_id,
            name: "White".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 10,
            height: 10,
            extra: serde_json::Map::new(),
        },
    ));
    let held = early(&comp);
    let mut changed = held.clone();
    if let Animation::Keyframed(keys) = &mut changed.layers[0].transform.position_x.animation {
        keys[0].value = 10.0;
    }
    assert_ne!(
        name_of(&doc, &held),
        name_of(&doc, &changed),
        "the solid a frame back is in the frame's name"
    );
    let mut off = (held.clone(), changed.clone());
    off.0.layers[0].effects[1].enabled = false;
    off.1.layers[0].effects[1].enabled = false;
    assert_eq!(
        name_of(&doc, &off.0),
        name_of(&doc, &off.1),
        "without the Echo it is not"
    );
}

fn moving_solid(effects: Vec<lumit_core::model::EffectInstance>) -> (Document, Composition) {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    let solid_id = Uuid::now_v7();
    let key = |t: i64, value: f64| Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "solid".into(),
        kind: LayerKind::Solid { def: solid_id },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup {
            position_x: Property {
                animation: Animation::Keyframed(vec![key(0, 20.0), key(1, 80.0)]),
                ..Property::zero()
            },
            ..TransformGroup::default()
        },
        matte: None,
        parent: None,
        label: 0,
        volume_db: Property::zero(),
        pan: Property::zero(),
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
        grade_versions: None,
        graph_inputs: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects,
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        blending: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 100,
        height: 40,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_id,
            name: "White".into(),
            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
            width: 10,
            height: 10,
            extra: serde_json::Map::new(),
        },
    ));
    doc.items
        .push(lumit_core::model::ProjectItem::Composition(comp.clone()));
    (doc, comp)
}

fn effect_with(name: &str, rows: &[(&str, f64)]) -> lumit_core::model::EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate(name).unwrap();
    e.enabled = true;
    for (id, v) in rows {
        let p = e.params.iter_mut().find(|p| p.id == *id).unwrap();
        p.value = lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(*v));
    }
    e
}

#[test]
fn time_effects_follow_the_layers_own_motion() {
    let x_at = |t: f64| 20.0 + 60.0 * t;
    let draw = |effects| {
        let (doc, comp) = moving_solid(effects);
        let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut draws = build_comp_draws(
            &std::sync::Arc::new(doc),
            &comp,
            0.55,
            &map,
            &mut vec![comp.id],
        );
        assert_eq!(draws.len(), 1);
        draws.remove(0)
    };
    let members = |d: &crate::draw::CompLayerDraw| match &d.source {
        DrawSource::Nested { draws, .. } => draws[0].position.0,
        _ => panic!("drawn as a comp-sized picture"),
    };

    let blurred = draw(vec![effect_with("motion_blur", &[])]);
    assert!((f64::from(members(&blurred)) - x_at(0.55)).abs() < 1e-3);
    assert_eq!(blurred.flow_below.len(), 1);
    let (offset, then, _, measure) = &blurred.flow_below[0];
    let full = lumit_core::fx::temporal::FlowWant::Full;
    assert_eq!((*offset, *measure), (1, Some((1, full))));
    let next = 0.55 + 1.0 / 30.0;
    assert!((f64::from(then[0].position.0) - x_at(next)).abs() < 1e-3);

    let held = draw(vec![effect_with("posterize_time", &[("rate", 2.0)])]);
    assert!(
        (f64::from(members(&held)) - x_at(0.5)).abs() < 1e-3,
        "held at half a second: {}",
        members(&held)
    );
    assert!(held.fx_ids.is_empty(), "nothing runs on the held picture");

    let slowed = draw(vec![effect_with("timewarp", &[("speed", 50.0)])]);
    let crate::draw::LayerInputDraw::Several(pictures) = &slowed.dof_inputs[0] else {
        panic!("Timewarp reads pictures of the placed layer");
    };
    assert!(!pictures.is_empty());
    for p in pictures {
        let crate::draw::LayerInputDraw::Layer(p) = p else {
            panic!("each a picture");
        };
        let x = p.nested.as_ref().unwrap().draws[0].position.0;
        assert!(
            (f64::from(x) - x_at(0.275)).abs() < 2.0 + 1e-3,
            "a moment near 0.275 s: x {x}"
        );
    }
}

#[test]
fn echo_time_spaces_the_echoes() {
    for spacing in [-0.1, 0.1] {
        let (doc, comp) = moving_solid(vec![effect_with(
            "echo",
            &[("echo_time", spacing), ("echoes", 3.0)],
        )]);
        let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc),
            &comp,
            0.5,
            &map,
            &mut vec![comp.id],
        );
        let trail = &draws[0];
        assert_eq!(trail.flow_below.len(), 3);
        for (offset, then, _, _) in &trail.flow_below {
            let at = 0.5 + f64::from(-offset) * spacing;
            let x = 20.0 + 60.0 * at;
            assert!(
                (f64::from(then[0].position.0) - x).abs() < 0.05,
                "echo {offset} at {at} s: {:?} vs {x}",
                then[0].position
            );
        }
    }
}

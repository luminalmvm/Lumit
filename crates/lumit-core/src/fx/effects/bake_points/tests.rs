//! Tests of Bake points.

use super::*;
use crate::anim::Property;
use crate::expression::ExpressionContext;
use crate::fx::MarkerContext;
use crate::graph::{Edge, InputRef, NodeRef, OutputRef};
use crate::model::*;
use crate::time::{CompTime, Duration, FrameRate, Rational};

/// A stream of the points `ids` at time `t`: every column carried, each
/// point somewhere of its own, and one colour for all of them.
fn points(ids: &[u64], t: f32) -> PointsStream {
    let each =
        |f: &dyn Fn(f32) -> f32| -> Vec<f32> { ids.iter().map(|&id| f(id as f32)).collect() };
    PointsStream {
        position: ids
            .iter()
            .map(|&id| {
                [
                    100.0 + id as f32 * 7.5 + t * 30.0,
                    50.0 - t * id as f32,
                    0.0,
                ]
            })
            .collect(),
        speed: ids.iter().map(|&id| [30.0, -(id as f32), 0.0]).collect(),
        age: each(&|id| t + id * 0.01),
        life: each(&|_| 2.0),
        size: each(&|id| 4.0 + id),
        rotation: each(&|id| id * 0.3 + t),
        colour: ids.iter().map(|_| [0.5, 0.25, 1.0, 1.0]).collect(),
        id: ids.to_vec(),
        projection: Projection::FLAT,
        stretch: ids.iter().map(|&id| [1.0, 1.0 + id as f32 * 0.1]).collect(),
        pick: ids.iter().map(|&id| (id % 2) as f32).collect(),
        index: Vec::new(),
        named: vec![
            points::Named {
                name: "tint".into(),
                width: 4,
                values: ids
                    .iter()
                    .map(|&id| [id as f32 * 0.1, t, 0.5, 1.0])
                    .collect(),
            },
            points::Named {
                name: "heat".into(),
                width: 1,
                values: ids
                    .iter()
                    .map(|&id| [id as f32 + t, 0.0, 0.0, 0.0])
                    .collect(),
            },
        ],
        px_scale: 0.5,
    }
}

fn bits(s: &PointsStream) -> Vec<u32> {
    let rows = s.position.iter().chain(&s.speed).flatten();
    let ones = s.age.iter().chain(&s.rotation).chain(&s.size);
    rows.chain(ones).map(|v| v.to_bits()).collect()
}

/// The frames of a bake: still for a frame, then point 1 dies, 40 is
/// born and the rest come in another order, then nothing at all, then
/// points with no stretch or pick and a nought with its sign.
fn frames() -> Vec<PointsStream> {
    let mut last = points(&[0, 1, 2, 3, 4, 5], 2.0 / 24.0);
    last.stretch.clear();
    last.pick.clear();
    last.rotation[0] = -0.0;
    vec![
        points(&[0, 1, 2, 3, 4, 5], 0.0),
        points(&[0, 1, 2, 3, 4, 5], 0.0),
        points(&[5, 3, 0, 2, 4, 40], 1.0 / 24.0),
        PointsStream {
            px_scale: 0.5,
            ..PointsStream::default()
        },
        last,
    ]
}

fn bake(fingerprint: [u8; 32]) -> Baked {
    let mut baked = Baked::new(fingerprint, 24.0, 0, 5);
    for stream in frames() {
        assert!(baked.push(&stream, MAX_BYTES).is_some());
    }
    baked.seal();
    baked
}

/// What is read back from a bake's file is the stream that was baked, to
/// the bit, however its points were born, died or shuffled between
/// frames. Each number is written as its change from the same point on
/// the frame before, so a slip in matching points up would read back as
/// points in the wrong place.
#[test]
fn a_bake_reads_back_from_its_file_to_the_bit() {
    let streams = frames();
    let mut baked = Baked::new([3; 32], 24.0, 0, 5);
    let mut added = Vec::new();
    for stream in &streams {
        added.push(baked.push(stream, MAX_BYTES).expect("there is room"));
    }
    assert_eq!(added[1], 0, "a frame that holds still costs nothing");
    assert_eq!(baked.frames, [0, 0, 1, 2, 3]);
    assert_eq!(baked.unique.len(), 4);
    // A frame with no room is left out, and the bake is as it was.
    assert_eq!(baked.push(&streams[2], 10), None);
    assert_eq!(baked.frames.len(), 5);

    baked.seal();
    assert_ne!(baked.content, [0; 32]);
    let mut other = bake([3; 32]);
    assert_eq!(
        other.content, baked.content,
        "the same frames, the same hash"
    );
    other.unique[2].px_scale = 1.0;
    other.seal();
    assert_ne!(other.content, baked.content, "and other frames another");

    let stored = baked.stored();
    let back = Baked::restored(&stored).expect("the bake reads back");
    assert_eq!(back, baked);
    let back = Arc::new(back);
    for (k, stream) in streams.iter().enumerate() {
        let read = Reading::Baked(Arc::clone(&back), back.frames[k]);
        let read = read.stream(Projection::FLAT).expect("a baked frame");
        assert!(read == *stream, "frame {k} reads back as it was baked");
        assert_eq!(bits(&read), bits(stream), "frame {k}, to the bit");
    }

    // A file that does not add up is no bake, rather than a wrong one.
    let broken = |f: &dyn Fn(&mut StoredFrame)| {
        let mut s = stored.clone();
        f(&mut s.unique[1]);
        Baked::restored(&s)
    };
    assert!(broken(&|f| {
        f.ids.pop();
    })
    .is_none());
    assert!(broken(&|f| f.count = MAX_POINTS as u32 + 1).is_none());
    assert!(broken(&|f| {
        f.columns.pop();
    })
    .is_none());
    assert!(broken(&|f| f.columns[0].truncate(3)).is_none());
}

fn secs(n: i64, d: i64) -> CompTime {
    CompTime(Rational::new(n, d).unwrap())
}

fn set(inst: &mut EffectInstance, id: &str, value: f64) {
    let row = inst.params.iter_mut().find(|p| p.id == id).unwrap();
    row.value = EffectValue::Float(Property::fixed(value));
}

fn mode(inst: &mut EffectInstance, mode: u32) {
    let row = inst.params.iter_mut().find(|p| p.id == "mode").unwrap();
    row.value = EffectValue::Choice(mode);
}

/// A board with Pick points, Bake points and Connect points on it, and a
/// Math driver wired into each of the other two.
fn board() -> (Document, Uuid) {
    let pick = crate::fx::instantiate("pick_points").unwrap();
    let bake = crate::fx::instantiate(MATCH_NAME).unwrap();
    let connect = crate::fx::instantiate("connect_points").unwrap();
    let above = crate::fx::instantiate("math").unwrap();
    let below = crate::fx::instantiate("math").unwrap();
    let stream = |from: Uuid, to: Uuid| Edge {
        from: OutputRef::EffectData {
            effect: from,
            port: "points".into(),
        },
        to: InputRef::Param {
            node: NodeRef::Effect(to),
            port: "points".into(),
        },
    };
    let drive = |from: Uuid, to: Uuid, port: &str| Edge {
        from: OutputRef::Driver {
            node: from,
            port: "value".into(),
        },
        to: InputRef::Param {
            node: NodeRef::Effect(to),
            port: port.into(),
        },
    };
    let def = Uuid::now_v7();
    let mut layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "Board".into(),
        kind: LayerKind::Solid { def },
        in_point: secs(0, 1),
        out_point: secs(5, 24),
        start_offset: secs(0, 1),
        transform: TransformGroup::default(),
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
        blend: BlendMode::Normal,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    layer.graph.edges = vec![
        stream(pick.id, bake.id),
        stream(bake.id, connect.id),
        drive(above.id, pick.id, "to"),
        drive(below.id, connect.id, "max_distance"),
    ];
    layer.graph.nodes = vec![above, below];
    layer.effects = vec![pick, bake, connect];
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Bake".into(),
        width: 640,
        height: 360,
        frame_rate: FrameRate::new(24, 1).unwrap(),
        duration: Duration(Rational::new(5, 24).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Board".into(),
        colour: LinearColour([0.1, 0.2, 0.9, 1.0]),
        width: 640,
        height: 360,
        extra: serde_json::Map::new(),
    }));
    doc.items.push(ProjectItem::Composition(comp));
    (doc, comp_id)
}

/// Auto hands on the bake only while what is above the effect is what it
/// was baked from. Moving the layer, or changing what is below the
/// effect, keeps it. Changing an effect above or a driver feeding one
/// does not. Read hands on the bake whatever, and nothing past its end.
#[test]
fn auto_reads_a_bake_only_while_what_is_above_it_is_unchanged() {
    let (plain, comp_id) = board();
    let edited = |f: &dyn Fn(&mut Layer)| {
        let mut doc = plain.clone();
        f(&mut doc.comp_mut(comp_id).unwrap().layers[0]);
        Arc::new(doc)
    };
    let base = edited(&|_| {});
    let at = |doc: &Arc<Document>, t: f64| {
        let comp = doc.comp(comp_id).unwrap();
        let layer = &comp.layers[0];
        reading(doc, comp, layer, &layer.effects[1], t)
    };
    let frame = |doc: &Arc<Document>, t: f64| match at(doc, t) {
        Reading::Baked(_, unique) => Some(unique),
        _ => None,
    };
    let live = |doc: &Arc<Document>, t: f64| matches!(at(doc, t), Reading::Live);
    let comp = base.comp(comp_id).unwrap();
    let layer = &comp.layers[0];
    let effect = layer.effects[1].id;
    assert_eq!(fingerprint(&base, comp, layer, Uuid::now_v7()), None);
    assert!(live(&base, 0.0), "nothing is baked yet");

    let print = fingerprint(&base, comp, layer, effect).unwrap();
    publish(effect, bake(print));
    assert_eq!(frame(&base, 2.0 / 24.0), Some(1));
    assert_eq!(frame(&base, 4.0 / 24.0), Some(3));
    let read = at(&base, 2.0 / 24.0).stream(Projection::FLAT).unwrap();
    assert!(read == frames()[2], "Auto hands on the baked frame");

    let moved = edited(&|l| {
        l.transform.position_x = Property::fixed(400.0);
        l.name = "Moved".into();
    });
    assert_eq!(frame(&moved, 2.0 / 24.0), Some(1), "moving keeps a bake");
    let below = edited(&|l| {
        set(&mut l.effects[2], "max_distance", 3.0);
        set(&mut l.graph.nodes[1], "a", 9.0);
    });
    assert_eq!(frame(&below, 2.0 / 24.0), Some(1), "so does what is below");

    let above = edited(&|l| set(&mut l.effects[0], "to", 20.0));
    assert!(live(&above, 2.0 / 24.0), "an effect above counts");
    let driven = edited(&|l| set(&mut l.graph.nodes[0], "a", 9.0));
    assert!(live(&driven, 2.0 / 24.0), "and a driver feeding one");
    assert!(live(&base, 9.0), "past the bake is live");

    let read = edited(&|l| {
        set(&mut l.effects[0], "to", 20.0);
        mode(&mut l.effects[1], MODE_READ);
    });
    assert_eq!(frame(&read, 2.0 / 24.0), Some(1), "Read takes a stale bake");
    let past = at(&read, 9.0).stream(Projection::FLAT).unwrap();
    assert!(past.is_empty(), "and nothing past its end");
    let bypass = edited(&|l| mode(&mut l.effects[1], MODE_BYPASS));
    assert!(live(&bypass, 2.0 / 24.0));

    // What the effect resolves to says which frame of which bake is read,
    // so a picture drawn from one is never served for another.
    let derived = |doc: &Arc<Document>, t: f64| {
        let layer = &doc.comp(comp_id).unwrap().layers[0];
        let context = ExpressionContext {
            document: Arc::clone(doc),
            comp: Some(comp_id),
            layer: Some(layer.id),
            comp_time: t,
            current_depth: 0,
            inputs: None,
        };
        let stack = crate::fx::resolved::resolve_stack(
            &layer.effects,
            t,
            1000.0,
            1.0,
            &MarkerContext::NONE,
            Arc::new(context),
        );
        let params = stack.get(1).expect("the bake resolves").params;
        (
            params.choice(BakePoints::DERIVED_BAKE, u32::MAX),
            params.int(BakePoints::DERIVED_FRAME, i32::MAX),
        )
    };
    let held = baked(effect).unwrap();
    let [a, b, c, d, ..] = held.content;
    let head = u32::from_le_bytes([a, b, c, d]);
    assert_eq!(derived(&base, 2.0 / 24.0), (head, 1));
    assert_eq!(derived(&bypass, 2.0 / 24.0), (0, -1));
    assert_eq!(derived(&read, 9.0), (0, -2));

    retain(|id| *id != effect);
    assert!(baked(effect).is_none());
    assert!(matches!(at(&read, 2.0 / 24.0), Reading::Nothing));
}

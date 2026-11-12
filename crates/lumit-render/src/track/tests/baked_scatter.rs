//! Tests for a baked Scatter read by a driver.

use super::*;

/// A Scatter's points are made while a frame is drawn and exist nowhere
/// else, so a driver wired to them counts none. Baked, they are plain
/// data: a driver wired to the Bake points counts them, and the frame
/// drawn from the bake is the frame drawn from the live stream.
#[test]
fn a_baked_scatter_is_read_by_a_driver_and_draws_as_it_did_live() {
    use lumit_core::fx::drivers::points_sample::{COUNT_PORT, POINTS_PORT};
    use lumit_core::graph::{Edge, InputRef, NodeRef, OutputRef};
    use lumit_core::model::SolidDef;

    let _serial = serially();
    let dir = tempfile::tempdir().unwrap();
    with_cache(dir.path());
    let Ok(mut renderer) = crate::headless::HeadlessRenderer::new() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };

    let set = |inst: &mut EffectInstance, id: &str, value: EffectValue| {
        inst.params.iter_mut().find(|p| p.id == id).unwrap().value = value;
    };
    let number = |v: f64| EffectValue::Float(Property::fixed(v));
    let wire = |from: OutputRef, node: NodeRef, port: &str| Edge {
        from,
        to: InputRef::Param {
            node,
            port: port.into(),
        },
    };
    let stream_of = |effect: Uuid| OutputRef::EffectData {
        effect,
        port: "points".into(),
    };

    // A board with a Scatter on it, baked, and a web drawn from the bake.
    // Neither of the first two draws, so the web is the whole picture.
    let mut scatter = lumit_core::fx::builtins::instantiate("scatter").unwrap();
    set(&mut scatter, "density", number(3000.0));
    set(&mut scatter, "mix", number(0.0));
    // A Pick points between them marks half the points under a name, so
    // the stream that is baked carries a named column.
    let mut pick = lumit_core::fx::builtins::instantiate("pick_points").unwrap();
    set(&mut pick, "result", EffectValue::Choice(1));
    set(&mut pick, "group_name", EffectValue::Text("kept".into()));
    set(&mut pick, "to", number(50.0));
    set(&mut pick, "mix", number(0.0));
    // And a Vary points stores each point's colour under another, so it
    // carries a column four numbers wide as well.
    let mut vary = lumit_core::fx::builtins::instantiate("vary_points").unwrap();
    set(&mut vary, "opacity", number(40.0));
    set(&mut vary, "set_number", EffectValue::Bool(true));
    set(&mut vary, "number_name", EffectValue::Text("tint".into()));
    set(&mut vary, "number_store", EffectValue::Choice(2));
    set(&mut vary, "mix", number(0.0));
    let mut bake = lumit_core::fx::builtins::instantiate(bake_points::MATCH_NAME).unwrap();
    set(&mut bake, "mix", number(0.0));
    let mut connect = lumit_core::fx::builtins::instantiate("connect_points").unwrap();
    set(&mut connect, "max_distance", number(8.0));
    let (scatter_id, bake_id, connect_id) = (scatter.id, bake.id, connect.id);
    let (pick_id, vary_id) = (pick.id, vary.id);

    let (cw, ch) = (64u32, 36u32);
    let def = Uuid::now_v7();
    let mut board = layer("board", LayerKind::Solid { def }, secs(4, 24));
    board.effects = vec![scatter, pick, vary, bake, connect];
    board.graph.edges = vec![
        wire(stream_of(scatter_id), NodeRef::Effect(pick_id), "points"),
        wire(stream_of(pick_id), NodeRef::Effect(vary_id), "points"),
        wire(stream_of(vary_id), NodeRef::Effect(bake_id), "points"),
        wire(stream_of(bake_id), NodeRef::Effect(connect_id), "points"),
    ];
    let board_id = board.id;
    let comp_id = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Board".into(),
        colour: LinearColour([0.1, 0.2, 0.9, 1.0]),
        width: cw,
        height: ch,
        extra: serde_json::Map::new(),
    }));
    doc.items.push(ProjectItem::Composition(Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: comp_id,
        name: "bake".into(),
        width: cw,
        height: ch,
        frame_rate: FrameRate::new(24, 1).unwrap(),
        duration: Duration(Rational::new(4, 24).unwrap()),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers: vec![board],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }));

    // The same document with the Bake points on another Mode.
    let with_mode = |mode: u32| {
        let mut doc = doc.clone();
        let board = &mut doc.comp_mut(comp_id).unwrap().layers[0];
        set(&mut board.effects[3], "mode", EffectValue::Choice(mode));
        Arc::new(doc)
    };
    let (auto, bypass) = (with_mode(0), with_mode(bake_points::MODE_BYPASS));
    let mut frame_of = |doc: &Arc<Document>| renderer.render_rgba(doc, comp_id, 2, 1.0).unwrap().0;
    let live = frame_of(&bypass);
    assert_eq!(frame_of(&auto), live, "nothing is baked, so Auto is live");

    // And with two Points sample drivers: one counts the bake's stream
    // and one Scatter's own, each onto a Slider control for the reading.
    let counts = |doc: &Arc<Document>| {
        let mut doc = Document::clone(doc);
        let board = &mut doc.comp_mut(comp_id).unwrap().layers[0];
        let mut read = Vec::new();
        for source in [bake_id, scatter_id] {
            let sample = lumit_core::fx::builtins::instantiate("points_sample").unwrap();
            let slider = lumit_core::fx::builtins::instantiate("slider_control").unwrap();
            board.graph.edges.extend([
                wire(stream_of(source), NodeRef::Driver(sample.id), POINTS_PORT),
                wire(
                    OutputRef::Driver {
                        node: sample.id,
                        port: COUNT_PORT.into(),
                    },
                    NodeRef::Effect(slider.id),
                    "slider",
                ),
            ]);
            read.push(NodeRef::Effect(slider.id));
            board.graph.nodes.push(sample);
            board.effects.push(slider);
        }
        let doc = Arc::new(doc);
        let board = &doc.comp(comp_id).unwrap().layers[0];
        let driven = lumit_core::fx::drivers::resolve_drivers(
            &board.graph,
            2.0 / 24.0,
            Arc::new(lumit_core::expression::ExpressionContext {
                document: doc.clone(),
                comp: Some(comp_id),
                layer: Some(board_id),
                comp_time: 2.0 / 24.0,
                current_depth: 0,
                inputs: None,
            }),
            None,
        );
        read.iter()
            .map(
                |node| match driven.param(*node, lumit_core::fx::params::ParamId::new("slider")) {
                    Some(lumit_core::fx::params::Value::Float(v)) => v,
                    _ => 0.0,
                },
            )
            .collect::<Vec<f32>>()
    };
    assert_eq!(counts(&auto), [0.0, 0.0], "nothing to count before a bake");

    assert_eq!(
        request_bake(Arc::clone(&auto), comp_id, board_id, bake_id),
        Requested::Started
    );
    loop {
        match progress(bake_id) {
            Some(Progress::Done) => break,
            Some(Progress::Failed(why)) => panic!("the bake failed: {why}"),
            Some(Progress::Cancelled) => panic!("the bake was stopped"),
            _ => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    let baked = bake_points::baked(bake_id).expect("the bake is in the table");
    assert_eq!(
        baked.frames.len(),
        4,
        "one stream for every frame of the span"
    );

    let read = counts(&auto);
    assert!(read[0] > 0.0, "the driver counts the baked points");
    assert_eq!(read[1], 0.0, "and still none of Scatter's own");
    assert_eq!(
        frame_of(&auto),
        live,
        "the bake draws what the live stream drew"
    );
    assert_eq!(counts(&bypass)[0], 0.0, "Bypass reads no bake");

    // And the baked stream is the live one, named column and all: the
    // stream a live walk hands the effect, against the bake read back.
    crate::fxops::watch_points(Some(bake_id));
    frame_of(&bypass);
    let handed = crate::fxops::take_watched_points().expect("the walk reached the effect");
    crate::fxops::watch_points(None);
    let column = |name: &str| {
        let found = handed.named.iter().find(|c| c.name == name);
        found.expect("the live stream carries the named column")
    };
    let kept = &column("kept").values;
    let (yes, no) = ([1.0, 0.0, 0.0, 0.0], [0.0; 4]);
    assert!(
        kept.contains(&yes) && kept.contains(&no),
        "some kept, some not"
    );
    let tint = column("tint");
    assert_eq!(tint.width, 4, "a colour is four numbers");
    assert!(
        tint.values.iter().all(|c| c[3] > 0.0) && tint.values.iter().any(|c| c[3] < 0.9),
        "and each point has its own"
    );
    let board = &auto.comp(comp_id).unwrap().layers[0];
    let read = bake_points::reading(
        &auto,
        auto.comp(comp_id).unwrap(),
        board,
        &board.effects[3],
        2.0 / 24.0,
    )
    .stream(handed.projection)
    .expect("Auto reads the bake");
    assert!(read == handed, "the bake is the stream that was live");

    // The file holds the same: forgotten and read back the way opening
    // the project reads it, the stream is still that one.
    let read_back = || {
        bake_points::retain(|_| false);
        for job in warm_jobs(&auto) {
            run(job, &AtomicBool::new(false));
        }
        let board = &auto.comp(comp_id).unwrap().layers[0];
        let inst = &board.effects[3];
        bake_points::reading(&auto, auto.comp(comp_id).unwrap(), board, inst, 2.0 / 24.0)
            .stream(handed.projection)
    };
    assert!(read_back() == Some(handed.clone()), "the file round-trips");

    // And a file that is not a bake reads as no bake at all.
    let junk = Answer::Bake(vec![7; 64]);
    write_sidecar(dir.path(), AnalysisKey::bake(bake_id), 24.0, 4, &junk);
    assert_eq!(read_back(), None, "Auto is live again");
    assert_eq!(progress(bake_id), None, "and nothing claims to be done");
    clear();
}

use super::*;
use lumit_core::model::ProjectItem;

#[test]
fn oversize_footage_decodes_within_the_texture_limit() {
    assert_eq!(fit_texture(None, 7680, 4320), None, "8K fits");
    assert_eq!(fit_texture(Some(960), 7680, 4320), Some(960));
    assert_eq!(fit_texture(None, 9000, 5000), Some(8192), "wide: capped");
    assert_eq!(
        fit_texture(None, 5000, 9000),
        Some(4551),
        "tall: the height caps"
    );
    assert_eq!(
        fit_texture(Some(4000), 9000, 5000),
        Some(4000),
        "smaller ask kept"
    );
    assert_eq!(
        fit_texture(Some(8500), 9000, 5000),
        Some(8192),
        "larger ask capped"
    );
}

#[test]
fn a_layer_shown_smaller_decodes_no_larger_than_it_is_shown() {
    let on = Quality {
        shown_size: true,
        ..Quality::default()
    };
    assert_eq!(on.target_width_shown(3840, Some(0.5)), Some(1920));
    assert_eq!(on.target_width_shown(3840, None), None, "not a candidate");
    let half = Quality { divisor: 2, ..on };
    assert_eq!(
        half.target_width_shown(3840, Some(0.5)),
        Some(960),
        "Half of the shown size"
    );
    let off = Quality::default();
    assert_eq!(
        off.target_width_shown(3840, Some(0.5)),
        None,
        "the setting off, or export, decodes as before"
    );
}

#[test]
fn target_width_follows_the_quality_policy() {
    let full = Quality::default();
    assert_eq!(full.target_width(1920), None, "Full decodes at native");

    let half = Quality {
        divisor: 2,
        ..Quality::default()
    };
    assert_eq!(half.target_width(1920), Some(960));

    let auto = Quality {
        auto_res: true,
        display_scale: 0.25,
        ..Quality::default()
    };
    assert_eq!(auto.target_width(1920), Some(480));

    let zoomed = Quality {
        auto_res: true,
        display_scale: 4.0,
        ..Quality::default()
    };
    assert_eq!(zoomed.target_width(1920), None);

    let half_of_small = Quality {
        auto_res: true,
        display_scale: 0.5,
        ..Quality::default()
    };
    assert_eq!(half_of_small.target_width(1000), Some(500));

    let draft = Quality {
        draft: true,
        ..Quality::default()
    };
    assert_eq!(draft.target_width(3840), Some(DRAFT_MAX_WIDTH));
    assert_eq!(draft.target_width(1920), Some(DRAFT_MAX_WIDTH));
    let draft_half = Quality {
        draft: true,
        divisor: 2,
        ..Quality::default()
    };
    assert_eq!(draft_half.target_width(1920), Some(DRAFT_MAX_WIDTH));
    let draft_quarter = Quality {
        draft: true,
        divisor: 4,
        ..Quality::default()
    };
    assert_eq!(draft_quarter.target_width(1920), Some(480));
    assert_eq!(draft_quarter.target_width(1280), Some(320));
    let draft_auto = Quality {
        draft: true,
        auto_res: true,
        display_scale: 0.1,
        ..Quality::default()
    };
    assert_eq!(draft_auto.target_width(1920), Some(192));
    assert_eq!(draft.target_width(320), None);
}

/// **A hidden matte source decodes for whatever layer names it, an
/// adjustment layer included** (docs/06 §1.6).
///
/// The regression, and the reason it read as "hiding the matte layer breaks
/// the matte": a layer acting as an adjustment draws the composite beneath
/// it rather than a picture of its own, so this walk skipped it outright —
/// and took its matte reference with it. The matte source was then wanted by
/// nothing but itself, so it decoded only while its own visibility switch
/// was on. Switch off the eye — which is what everyone does to a matte
/// source, since its picture is not meant to be in the frame — and its
/// footage never decoded, `pixels_for` gave up, and the matte silently
/// vanished. On an ordinary consumer the same matte worked, which is what
/// made it look like a visibility bug rather than an adjustment one.
///
/// Both consumers are checked, hidden matte source either way: the ordinary
/// one is the case that always worked and must keep working, the adjustment
/// one is the fix.
#[test]
fn a_hidden_matte_source_decodes_for_an_adjustment_layer_too() {
    use lumit_core::model::{
        Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MatteChannel, MatteRef,
        MediaRef, Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;

    let layer = |kind: LayerKind| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "l".into(),
        kind,
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
        retime: None,
        interpolation: lumit_core::retime::Interpolation::default(),
        parked_flow: None,
        grade_versions: None,
        graph_inputs: None,
        blend: lumit_core::model::BlendMode::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = |layers: Vec<Layer>| Composition {
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
        name: "c".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };

    for adjustment in [false, true] {
        let mut doc = Document::new();
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: "f".into(),
            media: MediaRef {
                relative_path: "f.mp4".into(),
                absolute_path: "/f.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
            source_layer: None,
            interpretation: Default::default(),
        }));
        let mut matte_layer = layer(LayerKind::Footage { item });
        matte_layer.switches.visible = false;
        let matte_id = matte_layer.id;
        let mut consumer = layer(LayerKind::Solid {
            def: Uuid::now_v7(),
        });
        consumer.adjustment = adjustment;
        consumer.matte = Some(MatteRef {
            layer: matte_layer.id,
            channel: MatteChannel::Alpha,
            inverted: false,
            source: lumit_core::model::LayerInputSource::default(),
        });
        let outer = comp(vec![matte_layer, consumer]);
        let outer_id = outer.id;
        doc.items.push(ProjectItem::Composition(outer));
        let probes: HashMap<Uuid, crate::source::SourceProbe> = [(
            item,
            crate::source::SourceProbe::Video {
                fps: 60.0,
                width: 64,
                height: 64,
                frames: 600,
                audio: false,
            },
        )]
        .into_iter()
        .collect();
        let outer = doc.comp(outer_id).unwrap();
        let jobs = plan_comp_frame(&doc, outer, 0.0, Quality::default(), &probes);
        let who = if adjustment {
            "a layer acting as an adjustment"
        } else {
            "an ordinary layer"
        };
        assert_eq!(
            jobs.len(),
            1,
            "{who} matted by a hidden footage layer must plan that layer's decode"
        );
        assert_eq!(jobs[0].item, item);
        assert_eq!(
            jobs[0].layer, matte_id,
            "and the job is the matte source's own, not the consumer's"
        );
    }
}

/// **A retimed Precomp is planned and held by the moment it shows**
/// (docs/impl/node-graph-comp.md §5.6). The builder evaluates the nested
/// comp at `source_time_at`, so the planner has to decode there and ask
/// the held question there: planning at layer time would fetch the frame
/// after the one on screen, and asking at layer time would hold a texture
/// the realiser never looks for.
#[test]
fn a_retimed_precomp_plans_and_holds_by_its_mapped_time() {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::model::{
        Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, Switches,
        TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;

    let layer = |kind: LayerKind| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "l".into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
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
        interpolation: lumit_core::retime::Interpolation::default(),
        parked_flow: None,
        grade_versions: None,
        graph_inputs: None,
        blend: lumit_core::model::BlendMode::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = |layers: Vec<Layer>| Composition {
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
        name: "c".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    let item = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id: item,
        name: "f".into(),
        media: MediaRef {
            relative_path: "f.mp4".into(),
            absolute_path: "/f.mp4".into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
        source_layer: None,
        interpretation: Default::default(),
    }));
    let inner = comp(vec![layer(LayerKind::Footage { item })]);
    let inner_id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));
    let mut placed = layer(LayerKind::Precomp { comp: inner_id });
    placed.retime = Some(Property {
        animation: Animation::Keyframed(vec![
            Keyframe {
                time: Rational::ZERO,
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            Keyframe {
                time: Rational::new(10, 1).unwrap(),
                value: 5.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
        ]),
        extra: serde_json::Map::new(),
    });
    let outer = comp(vec![placed]);
    let outer_id = outer.id;
    doc.items.push(ProjectItem::Composition(outer));
    let probes: HashMap<Uuid, crate::source::SourceProbe> = [(
        item,
        crate::source::SourceProbe::Video {
            fps: 60.0,
            width: 64,
            height: 64,
            frames: 600,
            audio: false,
        },
    )]
    .into_iter()
    .collect();

    let asked = std::cell::Cell::new(f64::NAN);
    let held = |_: &std::sync::Arc<Document>,
                _: &Composition,
                lt: f64,
                _: Option<&lumit_core::model::EffectInstance>| {
        asked.set(lt);
        false
    };
    let held: HeldNested<'_> = &held;
    let doc = std::sync::Arc::new(doc);
    let jobs = plan_comp_frame_held(
        &doc,
        doc.comp(outer_id).unwrap(),
        4.0,
        Quality::default(),
        &probes,
        Some(held),
    );
    assert_eq!(asked.get(), 2.0, "the held question is asked at the map");
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].source_frame, 120,
        "and the decode is the frame the map points at, not frame 240"
    );
}

#[test]
fn a_clip_under_accumulation_mb_is_planned_at_every_shutter_moment() {
    use lumit_core::model::{
        Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef, Switches,
        TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;

    let layer = |kind: LayerKind| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "l".into(),
        kind,
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
        retime: None,
        interpolation: lumit_core::retime::Interpolation::default(),
        parked_flow: None,
        grade_versions: None,
        graph_inputs: None,
        blend: lumit_core::model::BlendMode::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    let footage = |doc: &mut Document| {
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: "f".into(),
            media: MediaRef {
                relative_path: "f.mp4".into(),
                absolute_path: "/f.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
            source_layer: None,
            interpretation: Default::default(),
        }));
        item
    };
    let (above_item, below_item) = (footage(&mut doc), footage(&mut doc));
    let mut adjust = layer(LayerKind::Adjustment);
    let mut mb = lumit_core::fx::builtins::instantiate("accumulation_mb").unwrap();
    for p in &mut mb.params {
        let v = match p.id.as_str() {
            "samples" => 4.0,
            "shutter_angle" => 360.0,
            "shutter_phase" => -180.0,
            _ => continue,
        };
        p.value = lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(v));
    }
    adjust.effects = vec![mb];
    let above = layer(LayerKind::Footage { item: above_item });
    let below = layer(LayerKind::Footage { item: below_item });
    let (above_id, below_id) = (above.id, below.id);
    let mut comp = Composition {
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
        name: "c".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![above, adjust, below],
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let probe = crate::source::SourceProbe::Video {
        fps: 60.0,
        width: 64,
        height: 64,
        frames: 600,
        audio: false,
    };
    let probes: HashMap<Uuid, crate::source::SourceProbe> =
        [(above_item, probe), (below_item, probe)]
            .into_iter()
            .collect();
    let jobs = plan_comp_frame(&doc, &comp, 1.0, Quality::default(), &probes);
    let job_for = |id: Uuid| jobs.iter().find(|j| j.layer == id).expect("planned");

    let covered = job_for(below_id);
    assert_eq!(
        covered.source_frame, 60,
        "frame 30 of the comp is frame 60 of the clip"
    );
    let moment =
        |offset: f64, source_frame: usize, ceil: usize, weight: f32| crate::decode::ShutterSample {
            offset,
            source_frame,
            blend: Some((ceil, weight)),
        };
    assert_eq!(
        covered.shutter,
        vec![
            moment(-0.375, 59, 60, 0.25),
            moment(-0.125, 59, 60, 0.75),
            moment(0.125, 60, 61, 0.25),
            moment(0.375, 60, 61, 0.75),
        ],
        "each shutter moment is the pair of source frames it falls between"
    );
    assert_eq!(
        covered.shutter_flow, None,
        "a Nearest clip crossfades its in-betweens"
    );

    let clear = job_for(above_id);
    assert!(
        clear.shutter.is_empty(),
        "a layer above the adjustment is not covered"
    );
    assert_eq!(clear.shutter_flow, None);
    // The moments are content: the covered job cannot share a name with an
    // uncovered decode of the same frame, or a cached decode would serve a
    // shutterless picture to the samples.
    assert_ne!(covered.source_key(), clear.source_key());
    assert!(!same_decode(
        std::slice::from_ref(covered),
        std::slice::from_ref(clear)
    ));

    let flow = lumit_core::retime::FlowParams {
        smoothness: 0.25,
        ..lumit_core::retime::FlowParams::default()
    };
    comp.layers[2].interpolation = lumit_core::retime::Interpolation::Flow(flow.clone());
    let jobs = plan_comp_frame(&doc, &comp, 1.0, Quality::default(), &probes);
    let covered = jobs.iter().find(|j| j.layer == below_id).expect("planned");
    assert_eq!(
        covered.shutter_flow,
        Some(flow),
        "a Flow clip makes its in-betweens with its own flow settings"
    );
}

#[test]
fn a_node_graph_plans_the_footage_its_boxes_read() {
    use lumit_core::anim::Property;
    use lumit_core::comp_graph::{CompGraph, GraphEdge, GraphNode};
    use lumit_core::model::{
        Composition, Document, EffectParam, EffectValue, FootageItem, Layer, LayerKind,
        LinearColour, MediaRef, Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;

    let layer = |kind: LayerKind| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "l".into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(10, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
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
        interpolation: lumit_core::retime::Interpolation::default(),
        parked_flow: None,
        grade_versions: None,
        graph_inputs: None,
        blend: lumit_core::model::BlendMode::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp_of = |layers: Vec<Layer>, graph: Option<CompGraph>| Composition {
        graph,
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
        name: "c".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let wire = |from: Uuid, from_port: &str, to: Uuid, to_port: &str| GraphEdge {
        from,
        from_port: from_port.to_owned(),
        to,
        to_port: to_port.to_owned(),
    };

    let mut doc = Document::new();
    let item = Uuid::now_v7();
    doc.items.push(ProjectItem::Footage(FootageItem {
        sequence: None,
        id: item,
        name: "f".into(),
        media: MediaRef {
            relative_path: "f.mp4".into(),
            absolute_path: "/f.mp4".into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        extra: serde_json::Map::new(),
        colour_space: None,
        source_layer: None,
        interpretation: Default::default(),
    }));
    let probes: HashMap<Uuid, crate::source::SourceProbe> = [(
        item,
        crate::source::SourceProbe::Video {
            fps: 60.0,
            width: 64,
            height: 64,
            frames: 600,
            audio: false,
        },
    )]
    .into_iter()
    .collect();

    let (read, out) = (Uuid::now_v7(), Uuid::now_v7());
    let graph = comp_of(
        Vec::new(),
        Some(CompGraph {
            widths: Vec::new(),
            notes: Vec::new(),
            ties: Vec::new(),
            looks: Default::default(),
            nodes: vec![
                GraphNode::Read {
                    id: read,
                    item,
                    custom_name: None,
                },
                GraphNode::Output { id: out },
            ],
            edges: vec![wire(read, "output", out, "input")],
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        }),
    );
    let graph_id = graph.id;
    doc.items.push(ProjectItem::Composition(graph.clone()));
    let jobs = plan_comp_frame(&doc, &graph, 0.0, Quality::default(), &probes);
    assert_eq!(jobs.len(), 1, "a Read of footage plans one job");
    assert_eq!(
        jobs[0].layer, read,
        "and it is keyed by the box's own id, which is what the builder looks up by"
    );
    assert_eq!(jobs[0].item, item);

    let inner_layer = layer(LayerKind::Footage { item });
    let inner_id = inner_layer.id;
    let inner = comp_of(vec![inner_layer], None);
    let inner_comp_id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));
    let (read_comp, out2) = (Uuid::now_v7(), Uuid::now_v7());
    let reader = comp_of(
        Vec::new(),
        Some(CompGraph {
            widths: Vec::new(),
            notes: Vec::new(),
            ties: Vec::new(),
            looks: Default::default(),
            nodes: vec![
                GraphNode::Read {
                    id: read_comp,
                    item: inner_comp_id,
                    custom_name: None,
                },
                GraphNode::Output { id: out2 },
            ],
            edges: vec![wire(read_comp, "output", out2, "input")],
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        }),
    );
    let jobs = plan_comp_frame(&doc, &reader, 0.0, Quality::default(), &probes);
    assert_eq!(jobs.len(), 1, "a Read of a comp plans that comp's decodes");
    assert_eq!(
        jobs[0].layer, inner_id,
        "keyed by the nested comp's own layer, not by the box"
    );

    let mut inst = lumit_core::fx::builtins::instantiate("node_graph").expect("the effect exists");
    lumit_core::fx::effects::node_graph::bind(
        &mut inst,
        graph_id,
        graph.graph.as_ref().expect("a node graph"),
    );
    let mut hidden = layer(LayerKind::Footage { item });
    hidden.switches.visible = false;
    inst.params.push(EffectParam::new(
        "plate",
        EffectValue::Layer(Some(hidden.id)),
    ));
    let mut host = layer(LayerKind::Null);
    let host_id = host.id;
    host.effects = vec![inst];
    let parent = comp_of(vec![host, hidden], None);
    let jobs = plan_comp_frame(&doc, &parent, 0.0, Quality::default(), &probes);
    assert!(
        jobs.iter().any(|j| j.layer == read),
        "the graph the effect applies must plan its own Read"
    );
    assert!(
        jobs.iter().any(|j| j.layer != read && j.layer != host_id),
        "and the layer a further picture row names is planned as any layer input is"
    );
    assert_eq!(jobs.len(), 2, "those two, and nothing else");

    let mut nested_box =
        lumit_core::fx::builtins::instantiate("node_graph").expect("the effect exists");
    lumit_core::fx::effects::node_graph::bind(
        &mut nested_box,
        graph_id,
        graph.graph.as_ref().expect("a node graph"),
    );
    let out3 = Uuid::now_v7();
    let box_id = nested_box.id;
    let outer = comp_of(
        Vec::new(),
        Some(CompGraph {
            widths: Vec::new(),
            notes: Vec::new(),
            ties: Vec::new(),
            looks: Default::default(),
            nodes: vec![GraphNode::Fx(nested_box), GraphNode::Output { id: out3 }],
            edges: vec![wire(box_id, "output", out3, "input")],
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        }),
    );
    let jobs = plan_comp_frame(&doc, &outer, 0.0, Quality::default(), &probes);
    assert_eq!(jobs.len(), 1, "a nested graph plans the inner Read");
    assert_eq!(
        jobs[0].layer, read,
        "keyed by the inner Read's own id, which is what the lowering looks up by"
    );

    let mut header =
        lumit_core::fx::builtins::instantiate("node_graph").expect("the effect exists");
    lumit_core::fx::effects::node_graph::bind(
        &mut header,
        graph_id,
        graph.graph.as_ref().expect("a node graph"),
    );
    let member = layer(LayerKind::Null);
    let member_id = member.id;
    let mut grouped = comp_of(vec![member], None);
    grouped.groups = vec![lumit_core::group::LayerGroup {
        id: Uuid::now_v7(),
        name: "band".into(),
        label: 0,
        members: vec![member_id],
        effects: vec![header],
    }];
    let jobs = plan_comp_frame(&doc, &grouped, 0.0, Quality::default(), &probes);
    assert_eq!(jobs.len(), 1, "a graph on a header plans its own Read");
    assert_eq!(jobs[0].layer, read);

    let switch_jobs = |driven: bool| {
        let (a, b, out) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut switch =
            lumit_core::fx::builtins::instantiate("switch").expect("the effect exists");
        if let Some(p) = switch.params.iter_mut().find(|p| p.id == "index") {
            p.value = EffectValue::Float(lumit_core::anim::Property::fixed(1.0));
        }
        let switch_id = switch.id;
        let reader = |id| GraphNode::Read {
            id,
            item,
            custom_name: None,
        };
        let mut nodes = vec![
            reader(a),
            reader(b),
            GraphNode::Fx(switch),
            GraphNode::Output { id: out },
        ];
        let mut edges = vec![
            wire(a, "output", switch_id, "in0"),
            wire(b, "output", switch_id, "in1"),
            wire(switch_id, "output", out, "input"),
        ];
        if driven {
            let wiggle =
                lumit_core::fx::builtins::instantiate("wiggle").expect("the driver exists");
            edges.push(wire(wiggle.id, "value", switch_id, "index"));
            nodes.push(GraphNode::Fx(wiggle));
        }
        let comp = comp_of(
            Vec::new(),
            Some(CompGraph {
                widths: Vec::new(),
                notes: Vec::new(),
                ties: Vec::new(),
                looks: Default::default(),
                nodes,
                edges,
                layout: Vec::new(),
                exposed: Vec::new(),
                groups: Vec::new(),
            }),
        );
        let jobs = plan_comp_frame(&doc, &comp, 0.0, Quality::default(), &probes);
        (a, b, jobs)
    };
    let (_, b, jobs) = switch_jobs(false);
    assert_eq!(jobs.len(), 1, "only the socket the Switch shows is fetched");
    assert_eq!(jobs[0].layer, b);
    let (_, _, jobs) = switch_jobs(true);
    assert_eq!(jobs.len(), 2, "a wired Index fetches every socket");
}

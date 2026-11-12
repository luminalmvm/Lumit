//! Tests of the layer and asset defaults.

use super::*;
use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::expression::script::Language;
use lumit_core::model::{Composition, Document, TransformProp};
use lumit_core::ops::apply;
use lumit_core::time::FrameRate;

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

fn run(doc: &mut Document, ops: &[Op]) {
    for op in ops {
        apply(doc, op).unwrap();
    }
}

fn add_comp(doc: &mut Document, settings: CompSettings) -> Uuid {
    let (id, ops) = new_comp_ops(doc, settings, None);
    run(doc, &ops);
    id
}

fn comp(doc: &Document, id: Uuid) -> &Composition {
    doc.comp(id).unwrap()
}

fn value(p: &Property) -> f64 {
    p.value_at(0.0)
}

fn key(n: i64) -> Keyframe {
    Keyframe {
        time: rat(n, 1),
        value: n as f64,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

#[test]
fn each_kind_starts_on_its_own_label() {
    let text = serde_json::from_value(serde_json::json!({
        "text": "Hi", "size": 72.0, "fill": [1.0, 1.0, 1.0, 1.0]
    }))
    .unwrap();
    let id = Uuid::now_v7();
    let kinds = [
        (LayerKind::Footage { item: id }, 1),
        (LayerKind::Solid { def: id }, 2),
        (LayerKind::Precomp { comp: id }, 3),
        (LayerKind::Text { document: text }, 4),
        (LayerKind::Sequence { clips: Vec::new() }, 6),
        (LayerKind::Adjustment, 7),
        (LayerKind::Null, 8),
        (
            LayerKind::Shape {
                contents: Vec::new(),
            },
            9,
        ),
        (
            LayerKind::Light {
                light: Default::default(),
            },
            10,
        ),
    ];
    for (kind, label) in kinds {
        let layer = base_layer("L".into(), kind, rat(4, 1), TransformGroup::default());
        assert_eq!(layer.label, label, "{:?}", layer.kind);
        assert_eq!(
            (layer.in_point.0, layer.out_point.0),
            (Rational::ZERO, rat(4, 1))
        );
    }

    // The camera sits a 50 mm zoom back from the comp centre, and wears label 5.
    let mut doc = Document::new();
    let c = add_comp(&mut doc, CompSettings::default());
    let camera = fresh_camera(comp(&doc, c), "Camera".into(), None);
    let zoom = lumit_core::camera::default_zoom(1920.0);
    assert_eq!(camera.label, 5);
    let t = &camera.transform;
    assert_eq!(
        [
            value(&t.position_x),
            value(&t.position_y),
            value(&t.position_z)
        ],
        [960.0, 540.0, -zoom]
    );
    assert_eq!(value(camera.prop(TransformProp::Zoom).unwrap()), zoom);
}

#[test]
fn a_centred_transform_pivots_about_the_content_middle() {
    let t = centred_transform(400.0, 200.0, 1920, 1080);
    assert_eq!(
        [&t.anchor_x, &t.anchor_y, &t.position_x, &t.position_y].map(value),
        [200.0, 100.0, 960.0, 540.0]
    );
}

#[test]
fn a_newcomer_joins_a_solo_only_when_it_would_show() {
    let layer = |kind: LayerKind, audio_only: bool, solo: bool| {
        let mut l = base_layer("L".into(), kind, rat(1, 1), TransformGroup::default());
        l.audio_only = audio_only;
        l.switches.solo = solo;
        l
    };
    let item = Uuid::now_v7();
    let picture = [layer(LayerKind::Footage { item }, false, true)];
    let sound = [layer(LayerKind::Footage { item }, true, true)];
    let cases = [
        // newcomer kind, audio only, existing, soloed after
        (LayerKind::Solid { def: item }, false, &picture, true),
        (LayerKind::Null, false, &picture, false),
        (LayerKind::Solid { def: item }, false, &sound, false),
        (LayerKind::Footage { item }, true, &sound, true),
        (LayerKind::Footage { item }, true, &picture, false),
    ];
    for (kind, audio_only, existing, soloed) in cases {
        let mut new = layer(kind, audio_only, false);
        solo_on_arrival(&mut new, existing.iter());
        assert_eq!(new.switches.solo, soloed, "{:?}", new.kind);
    }
}

#[test]
fn new_solids_and_nulls_are_comp_sized_and_filed() {
    let mut doc = Document::new();
    let c = add_comp(&mut doc, CompSettings::default());

    let null = null_layer(comp(&doc, c));
    assert_eq!((null.label, null.out_point.0), (8, rat(30, 1)));
    let t = &null.transform;
    assert_eq!(
        [&t.anchor_x, &t.position_x, &t.position_y].map(value),
        [0.0, 960.0, 540.0]
    );

    let (first, ops) = white_solid_layer(&doc, comp(&doc, c));
    assert_eq!(
        ops.len(),
        4,
        "the Solids folder is made with the first solid"
    );
    run(&mut doc, &ops);
    let (second, ops) = white_solid_layer(&doc, comp(&doc, c));
    assert_eq!(ops.len(), 2, "and found again for the second");
    run(&mut doc, &ops);

    assert_eq!(
        (first.name.as_str(), second.name.as_str()),
        ("White solid 1", "White solid 2")
    );
    assert_eq!(first.label, 2);
    assert_eq!(value(&first.transform.anchor_x), 960.0);
    let defs: Vec<Uuid> = [&first, &second]
        .map(|l| match l.kind {
            LayerKind::Solid { def } => def,
            _ => panic!("not a solid"),
        })
        .to_vec();
    for def in &defs {
        let solid = doc.items.iter().find_map(|i| match i {
            ProjectItem::Solid(s) if s.id == *def => Some(s),
            _ => None,
        });
        let solid = solid.unwrap();
        assert_eq!(
            (solid.width, solid.height, solid.colour.0),
            (1920, 1080, [1.0; 4])
        );
    }
    let folder = doc.folder(doc.auto_folders.solids.unwrap()).unwrap();
    assert_eq!((folder.name.as_str(), &folder.children), ("Solids", &defs));

    // A remembered folder that has since been deleted is made again.
    doc.auto_folders.solids = Some(Uuid::now_v7());
    let (id, ops) = ensure_auto_folder_ops(&doc, AutoFolderKind::Solids);
    run(&mut doc, &ops);
    assert_eq!(doc.auto_folders.solids, Some(id));
    assert!(doc.folder(id).unwrap().children.is_empty());
    let op = file_into_folder_op(&doc, id, defs[0]);
    run(&mut doc, &[op]);
    assert_eq!(doc.folder(id).unwrap().children, vec![defs[0]]);
}

#[test]
fn new_comps_are_named_clamped_and_filed() {
    let mut doc = Document::new();
    let first = add_comp(
        &mut doc,
        CompSettings {
            width: 4,
            height: 99_999,
            motion_blur_samples: 0,
            ..CompSettings::default()
        },
    );
    let named = add_comp(
        &mut doc,
        CompSettings {
            name: "Titles".into(),
            ..CompSettings::default()
        },
    );
    let third = add_comp(&mut doc, CompSettings::default());
    let (graph, ops) = new_comp_ops(
        &doc,
        CompSettings::default(),
        Some(lumit_core::comp_graph::CompGraph::new_with_output()),
    );
    run(&mut doc, &ops);

    let c = comp(&doc, first);
    assert_eq!((c.name.as_str(), c.width, c.height), ("Comp 1", 16, 16384));
    assert_eq!(c.motion_blur.samples, 1);
    assert_eq!(comp(&doc, named).name, "Titles");
    assert_eq!(comp(&doc, third).name, "Comp 3");
    assert_eq!(comp(&doc, graph).name, "Node graph 1");
    assert_eq!(next_comp_name(&doc, false), "Comp 4");

    let folder = doc.folder(doc.auto_folders.compositions.unwrap()).unwrap();
    assert_eq!(folder.children, vec![first, named, third, graph]);

    let defaults = CompSettings::default();
    assert_eq!((defaults.width, defaults.height), (1920, 1080));
    assert_eq!(defaults.frame_rate, FrameRate::new(60, 1).unwrap());
    assert_eq!(defaults.duration.0, rat(30, 1));
}

#[test]
fn a_comp_is_at_least_one_frame_long() {
    let rate = FrameRate::new(25, 1).unwrap();
    assert_eq!(comp_duration(rate, Rational::ZERO).unwrap().0, rat(1, 25));
    assert_eq!(comp_duration(rate, rat(2, 1)).unwrap().0, rat(2, 1));
}

#[test]
fn new_settings_crop_about_the_centre() {
    let mut doc = Document::new();
    let c = add_comp(&mut doc, CompSettings::default());
    let null = null_layer(comp(&doc, c));
    let null_id = null.id;
    doc.comp_mut(c).unwrap().layers.push(null);
    doc.comp_mut(c).unwrap().motion_blur.enabled = true;
    let settings = |width, height| CompSettings {
        name: "Comp 1".into(),
        width,
        height,
        ..CompSettings::default()
    };

    // Nothing but the settings when the size and shutter stand.
    assert_eq!(
        comp_settings_ops(comp(&doc, c), settings(1920, 1080)).len(),
        1
    );

    // Sizes are clamped, and the crop is centred on the old frame.
    let ops = comp_settings_ops(comp(&doc, c), settings(4, 99_999));
    assert!(matches!(
        ops[1],
        Op::CropCompToRegion { x, y, width: 16, height: 16384, .. }
            if x == 952.0 && y == -7652.0
    ));

    let ops = comp_settings_ops(
        comp(&doc, c),
        CompSettings {
            name: "Smaller".into(),
            shutter_angle: 90.0,
            motion_blur_samples: 0,
            ..settings(1280, 720)
        },
    );
    // The settings go first at the old size, so the undo step is named for them.
    assert!(matches!(
        ops[0],
        Op::SetCompSettings {
            width: 1920,
            height: 1080,
            ..
        }
    ));
    assert_eq!(ops.len(), 3);
    run(&mut doc, &ops);

    let c = comp(&doc, c);
    assert_eq!((c.name.as_str(), c.width, c.height), ("Smaller", 1280, 720));
    let t = &c.layers.iter().find(|l| l.id == null_id).unwrap().transform;
    assert_eq!([&t.position_x, &t.position_y].map(value), [640.0, 360.0]);
    let blur = c.motion_blur;
    assert_eq!(
        (blur.enabled, blur.shutter_angle, blur.samples),
        (true, 90.0, 1)
    );
}

#[test]
fn keys_move_onto_the_layer_clock_in_order() {
    let moved = key_on_layer_clock(key(3), rat(1, 1)).unwrap();
    assert_eq!((moved.time, moved.value), (rat(2, 1), 3.0));
    // A time that would overflow is refused rather than wrapped.
    let far = Keyframe {
        time: rat(-i64::MAX, 1),
        ..key(0)
    };
    assert!(key_on_layer_clock(far, rat(2, 1)).is_none());

    assert!(keyframed(Vec::new()).is_none());
    assert!(keyframed(vec![key(1), key(1)]).is_none());
    assert!(keyframed(vec![key(2), key(1)]).is_none());
    assert!(matches!(
        keyframed(vec![key(0), key(1)]),
        Some(Animation::Keyframed(keys)) if keys.len() == 2
    ));
}

#[test]
fn transform_ops_carry_the_expression_note() {
    let mut doc = Document::new();
    let c = add_comp(&mut doc, CompSettings::default());
    let null = null_layer(comp(&doc, c));
    let id = null.id;
    doc.comp_mut(c).unwrap().layers.push(null);
    let current = |doc: &Document| {
        comp(doc, c)
            .layers
            .iter()
            .find(|l| l.id == id)
            .unwrap()
            .clone()
    };
    let px = TransformProp::PositionX;

    // A plain value writes no note.
    let ops = transform_ops(c, &current(&doc), px, Animation::Static(5.0), None);
    assert_eq!(ops.len(), 1);
    run(&mut doc, &ops);

    // An expression arriving writes one, in the language asked for.
    let expr = |s: &str| Animation::Expression(s.into());
    let ops = transform_ops(
        c,
        &current(&doc),
        px,
        expr("value + 1"),
        Some(Language::JavaScript),
    );
    assert!(matches!(
        &ops[1],
        Op::SetTransformExpressionSlot { slot: Some(_), .. }
    ));
    run(&mut doc, &ops);
    let held = current(&doc).prop(px).unwrap().clone();
    assert_eq!(held.expression_language(), Language::JavaScript);
    assert_eq!(held.animation, expr("value + 1"));

    // Editing the text keeps the note; taking the expression off clears it.
    assert_eq!(
        transform_ops(c, &current(&doc), px, expr("value + 2"), None).len(),
        1
    );
    let ops = transform_ops(c, &current(&doc), px, Animation::Static(3.0), None);
    assert!(matches!(
        &ops[1],
        Op::SetTransformExpressionSlot { slot: None, .. }
    ));

    // A camera channel on a layer that has none is only the property op.
    assert_eq!(
        transform_ops(c, &current(&doc), TransformProp::Zoom, expr("1"), None).len(),
        1
    );
}

#[test]
fn category_keys_are_distinct_snake_case() {
    use lumit_core::fx::schema::FxCategory::*;
    let all = [
        BlurSharpen,
        Colour,
        Distortion,
        Generate,
        Stylise,
        Temporal,
        Transition,
        Utility,
        Audio,
        Compositing,
        Controls,
        Drivers,
    ];
    let mut keys: Vec<&str> = all.into_iter().map(fx_category_key).collect();
    assert!(keys
        .iter()
        .all(|k| k.chars().all(|c| c.is_ascii_lowercase() || c == '_')));
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), all.len());
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    base_layer(name.into(), kind, rat(1, 1), TransformGroup::default())
}

fn named(name: &str) -> Layer {
    layer(name, LayerKind::Null)
}

#[test]
fn link_copies_keep_parents_and_mattes_across_the_cut() {
    let rig = named("rig");
    let mut arm = named("arm");
    arm.parent = Some(rig.id);
    let mut hand = named("hand");
    hand.parent = Some(arm.id);
    let mut shadow = named("shadow");
    shadow.matte = Some(lumit_core::model::MatteRef {
        layer: hand.id,
        channel: lumit_core::model::MatteChannel::Alpha,
        inverted: false,
        source: Default::default(),
    });
    let layers = [rig.clone(), arm.clone(), hand.clone(), shadow];

    assert!(link_copies(&layers, &[]).is_empty());
    let copies = link_copies(&layers, &[hand.id]);

    // Inside: the hand leans on the arm, so the arm comes along, and the
    // arm's rig with it, hidden and silent.
    assert_eq!(copies.inside.len(), 2);
    let arm_copy = copies.inside.iter().find(|l| l.name == "arm").unwrap();
    assert_eq!(copies.inside_ids[&arm.id], arm_copy.id);
    assert_eq!(arm_copy.parent, Some(copies.inside_ids[&rig.id]));
    assert!(copies
        .inside
        .iter()
        .all(|l| !l.switches.visible && !l.switches.audible));

    // Outside: the shadow's matte is the hand, so a copy of the hand stays,
    // still parented to the arm that stays outside too.
    assert_eq!(copies.outside.len(), 1);
    assert_eq!(copies.outside[0].parent, Some(arm.id));
    assert_eq!(copies.outside_ids[&hand.id], copies.outside[0].id);
}

#[test]
fn nested_comps_are_found_everywhere_and_can_be_repointed() {
    use lumit_core::comp_graph::{CompGraph, GraphNode, NODE_GRAPH};
    use lumit_core::fx::effects::node_graph;

    let mut doc = Document::new();
    let outer = add_comp(&mut doc, CompSettings::default());
    let [a, b, c, d, e] = [(); 5].map(|_| add_comp(&mut doc, CompSettings::default()));
    let graph = CompGraph::new_with_output();
    let bound = |to| {
        let mut inst = lumit_core::fx::builtins::instantiate(NODE_GRAPH).unwrap();
        node_graph::bind(&mut inst, to, &graph);
        inst
    };

    let mut nest = comp(&doc, outer).clone();
    let precomp = |comp| layer("p", LayerKind::Precomp { comp });
    let clip = lumit_core::sequence::Clip::new(
        lumit_core::sequence::ClipSource::Comp(b),
        Rational::ZERO,
        rat(1, 1),
        Rational::ZERO,
        rat(1, 1),
    );
    let mut fx_layer = named("fx");
    fx_layer.effects.push(bound(c));
    nest.layers = vec![
        precomp(a),
        precomp(a),
        precomp(Uuid::now_v7()),
        layer("s", LayerKind::Sequence { clips: vec![clip] }),
        fx_layer,
    ];
    nest.groups.push(lumit_core::group::LayerGroup {
        id: Uuid::now_v7(),
        name: "G".into(),
        label: 0,
        members: Vec::new(),
        effects: vec![bound(d)],
    });
    let mut g = CompGraph::new_with_output();
    g.nodes.push(GraphNode::Read {
        id: Uuid::now_v7(),
        item: e,
        custom_name: None,
    });
    g.nodes.push(GraphNode::Fx(bound(a)));
    nest.graph = Some(g);

    assert_eq!(nested_comps(&doc, &nest), vec![a, b, c, d, e]);

    // What the visitor writes back is what the comp then names.
    let swap = Uuid::now_v7();
    visit_comp_refs(&mut nest, &mut |id| {
        if *id == c || *id == a {
            *id = swap;
        }
    });
    assert!(matches!(nest.layers[0].kind, LayerKind::Precomp { comp } if comp == swap));
    assert_eq!(node_graph::comp_of(&nest.layers[4].effects[0]), Some(swap));
    assert_eq!(node_graph::comp_of(&nest.groups[0].effects[0]), Some(d));
    assert!(matches!(
        nest.graph.as_ref().unwrap().nodes.last(),
        Some(GraphNode::Fx(inst)) if node_graph::comp_of(inst) == Some(swap)
    ));
}

#[test]
fn fresh_ids_follow_links_inside_and_keep_those_outside() {
    let mut doc = Document::new();
    let elsewhere = add_comp(&mut doc, CompSettings::default());
    let c = add_comp(&mut doc, CompSettings::default());
    let mut nest = comp(&doc, c).clone();
    let parent = named("parent");
    let mut child = layer("child", LayerKind::Precomp { comp: elsewhere });
    child.parent = Some(parent.id);
    nest.layers = vec![parent.clone(), child.clone()];

    let mut comps = [nest];
    fresh_ids(&mut comps).unwrap();
    let [nest] = comps;
    assert_eq!(nest.id, c);
    let (p, ch) = (&nest.layers[0], &nest.layers[1]);
    assert_ne!(p.id, parent.id);
    assert_ne!(ch.id, child.id);
    assert_eq!(ch.parent, Some(p.id));
    assert!(matches!(ch.kind, LayerKind::Precomp { comp } if comp == elsewhere));
}

/// A span of layers on the comp's clock, for the precompose tests.
fn spanned(name: &str, from: i64, to: i64) -> Layer {
    let mut l = named(name);
    l.in_point = CompTime(rat(from, 1));
    l.out_point = CompTime(rat(to, 1));
    l
}

fn no_length(_: &Layer) -> Option<lumit_core::time::Duration> {
    None
}

#[test]
fn precompose_packs_the_layers_trims_to_their_span_and_keeps_a_parent() {
    let mut doc = Document::new();
    let c = add_comp(&mut doc, CompSettings::default());
    let a = spanned("a", 2, 6);
    let mut b = spanned("b", 3, 8);
    let rig = named("rig");
    b.parent = Some(rig.id);
    doc.comp_mut(c).unwrap().layers = vec![a.clone(), b.clone(), rig.clone()];

    let (ops, id) = precompose_ops(
        &doc,
        comp(&doc, c),
        Precompose {
            layers: &[b.id, a.id],
            name: String::new(),
            leave_attributes: false,
            adjust_duration: true,
            group: None,
            source_length: &no_length,
        },
    )
    .unwrap();
    run(&mut doc, &ops);

    // The Precomp layer takes the top layer's place and spans the pair.
    let outer = comp(&doc, c);
    assert_eq!(outer.layers.len(), 2);
    let pre = &outer.layers[0];
    assert_eq!(pre.id, id);
    assert_eq!((pre.in_point.0, pre.out_point.0), (rat(2, 1), rat(8, 1)));
    assert_eq!(pre.start_offset.0, rat(2, 1));
    let LayerKind::Precomp { comp: inner } = pre.kind else {
        panic!("a Precomp layer");
    };

    // Inside, time starts at the first in point, and the parent that
    // stayed outside is kept by a hidden copy.
    let inner = comp(&doc, inner);
    assert_eq!(inner.name, "Pre-comp 1");
    assert_eq!(inner.duration.0, rat(6, 1));
    let inside_a = inner.layers.iter().find(|l| l.name == "a").unwrap();
    let inside_b = inner.layers.iter().find(|l| l.name == "b").unwrap();
    assert_eq!(
        (inside_a.in_point.0, inside_a.out_point.0),
        (rat(0, 1), rat(4, 1))
    );
    assert_eq!(
        (inside_b.in_point.0, inside_b.out_point.0),
        (rat(1, 1), rat(6, 1))
    );
    let copy = inside_b.parent.unwrap();
    assert_ne!(copy, rig.id);
    assert!(inner.layers.iter().any(|l| l.id == copy && l.name == "rig"));
}

#[test]
fn precompose_leaving_attributes_keeps_them_on_the_precomp_layer() {
    let mut doc = Document::new();
    let c = add_comp(&mut doc, CompSettings::default());
    let mut a = spanned("a", 0, 4);
    a.transform.rotation = Property::fixed(30.0);
    let b = spanned("b", 0, 4);
    doc.comp_mut(c).unwrap().layers = vec![a.clone(), b.clone()];

    // Leaving attributes behind means one layer, so a pair is refused, and
    // so is a layer the comp does not have.
    let asked = |layers: &[Uuid], leave: bool| {
        precompose_ops(
            &doc,
            comp(&doc, c),
            Precompose {
                layers,
                name: "Packed".into(),
                leave_attributes: leave,
                adjust_duration: false,
                group: None,
                source_length: &no_length,
            },
        )
        .map(|(ops, id)| (ops.len(), id))
    };
    assert!(matches!(
        asked(&[a.id, b.id], true),
        Err(lumit_core::ops::OpError::UnknownLayer)
    ));
    assert!(matches!(
        asked(&[Uuid::now_v7()], false),
        Err(lumit_core::ops::OpError::UnknownLayer)
    ));

    let (ops, id) = precompose_ops(
        &doc,
        comp(&doc, c),
        Precompose {
            layers: &[a.id],
            name: "Packed".into(),
            leave_attributes: true,
            adjust_duration: false,
            group: None,
            source_length: &no_length,
        },
    )
    .unwrap();
    run(&mut doc, &ops);

    // The Precomp layer stands where the layer stood, under its id and
    // with its turn, and the layer inside is back to its source.
    assert_eq!(id, a.id);
    let pre = comp(&doc, c).layers.iter().find(|l| l.id == id).unwrap();
    assert_eq!(value(&pre.transform.rotation), 30.0);
    let LayerKind::Precomp { comp: inner } = pre.kind else {
        panic!("a Precomp layer");
    };
    let inner = comp(&doc, inner);
    assert_eq!(inner.name, "Packed");
    assert_eq!(inner.duration, comp(&doc, c).duration);
    assert_eq!(inner.layers.len(), 1);
    assert_ne!(inner.layers[0].id, a.id);
    assert_eq!(value(&inner.layers[0].transform.rotation), 0.0);
}

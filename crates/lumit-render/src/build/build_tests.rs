//! Draw-building tests: layer geometry under reduced-resolution decode,
//! collapsed Precomps, the live value patch, and adjustment staging.
//!
//! These moved out of the egui shell with the pixel pass — they always
//! tested the builder, not the interface, and they now guard it for both
//! frontends at once.

use crate::build::{build_comp_draws, patch_layer_prop};
use crate::decode::CompLayerPixels;
use crate::draw::DrawSource;
use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::collections::HashMap;
use uuid::Uuid;

// Regression: under auto res a footage layer decodes at a reduced size that
// changes with viewport zoom. Its comp-space geometry must use the *native*
// source size, not the decoded size — otherwise a small layer balloons as
// you zoom in (the auto-res bug Mack reported, 2026-07-13).
#[test]
fn footage_geometry_uses_native_size_not_decoded_size() {
    let item = Uuid::now_v7();
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "clip".into(),
        kind: LayerKind::Footage { item },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    // Native 1920x1080, decoded 480x270 (zoomed out, quarter res).
    let lp = CompLayerPixels {
        layer: layer.id,
        width: 480,
        height: 270,
        rgba: vec![0u8; 480 * 270 * 4],
        natural_w: 1920,
        natural_h: 1080,
        temporal: Vec::new(),
        flow_fields: Vec::new(),
        shutter: Vec::new(),
        source_key: 0,
        source_frame: 0,
    };
    let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    map.insert(layer.id, &lp);
    let doc = Document::new();
    let mut visited = vec![comp.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &comp,
        0.0,
        &map,
        &mut visited,
    );

    assert_eq!(draws.len(), 1);
    // Geometry uses native size (zoom-independent), not the 480x270 decode.
    assert_eq!(draws[0].natural_size, (1920.0, 1080.0));
    // The texture still carries the decoded dimensions.
    match &draws[0].source {
        DrawSource::Pixels { tex_w, tex_h, .. } => assert_eq!((*tex_w, *tex_h), (480, 270)),
        _ => panic!("expected a pixel source for a footage layer"),
    }
}

// Collapse (docs/06 §1.4): a collapsed Precomp splices its inner draws
// into the parent list with the parent's placement multiplied in front —
// no Nested intermediate. Off (or forced by a mask) renders Nested.
#[test]
fn collapsed_precomp_splices_inner_draws_with_parent_placement() {
    use lumit_core::model::{ProjectItem, TextDocument};
    let text_layer = || Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "inner".into(),
        kind: LayerKind::Text {
            document: TextDocument {
                text: "hi".into(),
                expression: None,
                size: 24.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                path: None,
                path_offset: lumit_core::anim::Property::zero(),
                animators: Vec::new(),
                extra: serde_json::Map::new(),
            },
        },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let nested = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Nested".into(),
        width: 640,
        height: 360,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![text_layer()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let nested_id = nested.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(nested));

    let mut pre_layer = text_layer();
    pre_layer.kind = LayerKind::Precomp { comp: nested_id };
    pre_layer.switches.collapse = true;
    pre_layer.transform.position_x = lumit_core::anim::Property::fixed(100.0);
    pre_layer.transform.scale_x = lumit_core::anim::Property::fixed(200.0);
    let parent = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Parent".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![pre_layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let mut visited = vec![parent.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &parent,
        0.0,
        &map,
        &mut visited,
    );
    // Spliced: one draw, pixel source (the inner text), pre = the parent
    // Precomp layer's placement matrix — exactly the compositor's maths.
    assert_eq!(draws.len(), 1);
    assert!(matches!(draws[0].source, DrawSource::Pixels { .. }));
    let tr = &pre_layer.transform;
    let expect = lumit_gpu::place_matrix(
        (
            tr.position_x.value_at(0.0) as f32,
            tr.position_y.value_at(0.0) as f32,
        ),
        (
            tr.anchor_x.value_at(0.0) as f32,
            tr.anchor_y.value_at(0.0) as f32,
        ),
        (
            tr.scale_x.value_at(0.0) as f32,
            tr.scale_y.value_at(0.0) as f32,
        ),
        0.0,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(draws[0].pre, Some(expect));

    // Switch off → the Nested intermediate as before, no pre. The
    // intermediate clears to nothing, never to the nested comp's own
    // background colour: the nested comp here is opaque black, and a
    // Precomp that painted that black over the parent's stack would be the
    // "precomps go black where they should be see-through" bug.
    let mut off = parent.clone();
    off.layers[0].switches.collapse = false;
    let mut visited = vec![off.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &off,
        0.0,
        &map,
        &mut visited,
    );
    assert_eq!(draws.len(), 1);
    let DrawSource::Nested { background, .. } = &draws[0].source else {
        panic!("an uncollapsed Precomp renders to an intermediate");
    };
    assert_eq!(*background, [0.0, 0.0, 0.0, 0.0]);
    assert!(draws[0].pre.is_none());

    // A mask on the Precomp layer forces the intermediate (§1.4) even
    // with the switch set.
    let mut forced = parent.clone();
    forced.layers[0]
        .masks
        .push(lumit_core::mask::Mask::rectangle(0.0, 0.0, 10.0, 10.0));
    let mut visited = vec![forced.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &forced,
        0.0,
        &map,
        &mut visited,
    );
    assert_eq!(draws.len(), 1);
    assert!(matches!(draws[0].source, DrawSource::Nested { .. }));

    // Paint does the same, and the strokes ride the Nested draw so the
    // realiser can stamp them into the picture it makes. Before
    // this they were built, carried nowhere, and dropped: the brush left a
    // Timeline row and no pixels.
    let mut painted = parent.clone();
    painted.layers[0]
        .paint
        .push(lumit_core::paint::PaintStroke::new(
            "Brush 1",
            vec![(5.0, 5.0)],
        ));
    let mut visited = vec![painted.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &painted,
        0.0,
        &map,
        &mut visited,
    );
    assert_eq!(draws.len(), 1);
    let DrawSource::Nested { paint, .. } = &draws[0].source else {
        panic!("paint forces the intermediate a stroke needs to land in");
    };
    assert_eq!(paint.len(), 1, "the stroke travels with the draw");
}

// The live value-drag preview renders a comp patched with the provisional
// value. Patching a layer's Position X to 500 must show through as the
// draw's position, without touching the committed document.
#[test]
fn patch_layer_prop_overrides_the_previewed_value() {
    use lumit_core::model::TransformProp;
    let item = Uuid::now_v7();
    let layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "clip".into(),
        kind: LayerKind::Footage { item },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };

    let patched = patch_layer_prop(&comp, layer.id, TransformProp::PositionX, 500.0);
    // The committed comp is untouched (default position 0).
    assert_eq!(comp.layers[0].transform.position_x.value_at(0.0), 0.0);

    let lp = CompLayerPixels {
        layer: layer.id,
        width: 1920,
        height: 1080,
        rgba: vec![0u8; 16],
        natural_w: 1920,
        natural_h: 1080,
        temporal: Vec::new(),
        flow_fields: Vec::new(),
        shutter: Vec::new(),
        source_key: 0,
        source_frame: 0,
    };
    let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    map.insert(layer.id, &lp);
    let doc = Document::new();
    let mut visited = vec![patched.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &patched,
        0.0,
        &map,
        &mut visited,
    );
    assert_eq!(draws.len(), 1);
    assert_eq!(draws[0].position.0, 500.0);
}

/// An adjustment layer with a live stack emits an Adjust staging draw
/// above the content beneath it (docs/06 §1.5), carrying its resolved
/// effects, comp-sized geometry, and a comp-sized mask coverage; a dead
/// stack (fx switch off, everything disabled, or no effects) emits
/// nothing at all.
#[test]
fn a_live_adjustment_layer_emits_a_staging_draw() {
    let solid_def = Uuid::now_v7();
    let base = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "under".into(),
        kind: LayerKind::Solid { def: solid_def },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let mut adj = base.clone();
    adj.id = Uuid::now_v7();
    adj.name = "adjust".into();
    adj.kind = LayerKind::Adjustment;
    adj.effects
        .push(lumit_core::fx::instantiate("saturation").unwrap());
    adj.masks
        .push(lumit_core::mask::Mask::rectangle(0.0, 0.0, 960.0, 1080.0));
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_def,
            name: "red".into(),
            colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
            width: 1920,
            height: 1080,
            extra: serde_json::Map::new(),
        },
    ));
    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        // Index 0 = top: the adjustment sits above the solid.
        layers: vec![adj.clone(), base.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let mut visited = vec![comp.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &comp,
        0.0,
        &map,
        &mut visited,
    );
    // Bottom-up: the solid first, then the staging point above it.
    assert_eq!(draws.len(), 2);
    assert!(matches!(draws[0].source, DrawSource::Pixels { .. }));
    assert!(matches!(draws[1].source, DrawSource::Adjust));
    assert_eq!(draws[1].natural_size, (1920.0, 1080.0));
    assert_eq!(draws[1].fx.len(), 1);
    let (_, cov_w, cov_h) = draws[1].mask_cov.as_ref().unwrap();
    assert_eq!((*cov_w, *cov_h), (1920, 1080));

    // Dead stacks emit nothing: fx switch off, all effects disabled,
    // or an empty stack.
    for edit in [
        &(|l: &mut Layer| l.switches.fx = false) as &dyn Fn(&mut Layer),
        &|l: &mut Layer| l.effects[0].enabled = false,
        &|l: &mut Layer| l.effects.clear(),
    ] {
        let mut dead = adj.clone();
        edit(&mut dead);
        let mut comp = comp.clone();
        comp.layers[0] = dead;
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.0,
            &map,
            &mut visited,
        );
        assert_eq!(draws.len(), 1, "a dead adjustment stack must not stage");
        assert!(matches!(draws[0].source, DrawSource::Pixels { .. }));
    }
}

/// **The adjustment switch and the Adjustment kind build the same draw**, and
/// switching it off gives the layer its own picture back exactly.
///
/// The whole point of the flag is that it round-trips a layer with a
/// source, which a kind flip could not: so the layer under test here is a
/// **solid** carrying the flag, checked against a layer that is an
/// adjustment by kind, and then checked again with the flag off.
#[test]
fn the_adjustment_flag_builds_the_same_draw_as_the_adjustment_kind() {
    let solid_def = Uuid::now_v7();
    let base = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "under".into(),
        kind: LayerKind::Solid { def: solid_def },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_def,
            name: "red".into(),
            colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
            width: 1920,
            height: 1080,
            extra: serde_json::Map::new(),
        },
    ));
    let doc = std::sync::Arc::new(doc);

    // The layer on top, once as a kind and once as a flagged solid. Same id,
    // same effect, same everything else, so any difference in the draw is the
    // one thing under test.
    let top_id = Uuid::now_v7();
    let mut by_kind = base.clone();
    by_kind.id = top_id;
    by_kind.name = "adjust".into();
    by_kind.kind = LayerKind::Adjustment;
    by_kind
        .effects
        .push(lumit_core::fx::instantiate("saturation").unwrap());
    let mut by_flag = by_kind.clone();
    by_flag.kind = LayerKind::Solid { def: solid_def };
    by_flag.adjustment = true;

    let comp_of = |top: Layer| Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![top, base.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let draws_of = |comp: &Composition| {
        let mut visited = vec![comp.id];
        build_comp_draws(&doc, comp, 0.0, &map, &mut visited)
    };

    let kind_draws = draws_of(&comp_of(by_kind));
    let flag_draws = draws_of(&comp_of(by_flag.clone()));
    assert_eq!(kind_draws.len(), 2);
    assert_eq!(flag_draws.len(), 2, "the flagged solid stages, not draws");
    assert!(matches!(flag_draws[1].source, DrawSource::Adjust));
    assert_eq!(flag_draws[1].natural_size, (1920.0, 1080.0));
    assert_eq!(
        flag_draws[1].fx.len(),
        kind_draws[1].fx.len(),
        "the same stack resolves either way"
    );
    assert_eq!(flag_draws[1].natural_size, kind_draws[1].natural_size);
    assert_eq!(flag_draws[1].opacity, kind_draws[1].opacity);

    // Off again: the solid is a solid, with its own colour back — the
    // round-trip a kind flip could never do for a layer with a source.
    let mut off = by_flag;
    off.adjustment = false;
    let off_draws = draws_of(&comp_of(off));
    assert_eq!(off_draws.len(), 2);
    assert!(
        matches!(off_draws[1].source, DrawSource::Pixels { .. }),
        "with the switch off the layer draws its own picture again"
    );
}

/// **A Lens flare on an adjustment layer flares the picture below it**. The
/// regression: the flare's Matte source could only name *another* layer, and
/// an adjustment layer has no picture of its own, so putting the effect on one
/// meant hunting for some other layer to point at — and whichever you picked
/// was the wrong picture, since an adjustment layer is supposed to act on
/// everything beneath it.
///
/// The fix is a reference to the layer the effect is ON, which resolves to
/// that effect's own input rather than a second render. This test checks the
/// draw builder's half: the matte slot comes back as
/// [`LayerInputDraw::ThisLayer`] (nothing to render — `run_ops` binds the
/// texture it is already carrying), on an adjustment layer and on an
/// ordinary one alike, and stays `Absent` while the Source type is not
/// Matte or the reference is unset.
#[test]
fn a_flare_matte_pointed_at_its_own_layer_reads_this_layers_input() {
    use crate::draw::LayerInputDraw;
    use lumit_core::model::{EffectValue, LayerKind};

    let solid_def = Uuid::now_v7();
    let base = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "under".into(),
        kind: LayerKind::Solid { def: solid_def },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_def,
            name: "red".into(),
            colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        },
    ));

    // A flare in Matte mode whose Matte layer is `owner` — exactly what
    // `add_effect` now writes for a fresh instance.
    let flare_on = |owner: Uuid, source_type: u32, matte: Option<Uuid>| {
        let mut fx = lumit_core::fx::instantiate("lens_flare").unwrap();
        let _ = owner;
        for p in &mut fx.params {
            match p.id.as_str() {
                "source_type" => p.value = EffectValue::Choice(source_type),
                "matte" => p.value = EffectValue::Layer(matte),
                _ => {}
            }
        }
        fx
    };

    let comp_of = |layers: Vec<Layer>| Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let slots = |comp: &Composition| -> Vec<Vec<LayerInputDraw>> {
        let mut visited = vec![comp.id];
        build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            comp,
            0.0,
            &map,
            &mut visited,
        )
        .into_iter()
        .map(|d| d.mattes)
        .collect()
    };

    // 1. An adjustment layer, the case that did not work at all before.
    let mut adj = base.clone();
    adj.id = Uuid::now_v7();
    adj.name = "adjust".into();
    adj.kind = LayerKind::Adjustment;
    adj.effects.push(flare_on(adj.id, 1, Some(adj.id)));
    let comp = comp_of(vec![adj.clone(), base.clone()]);
    let drawn = slots(&comp);
    // Bottom-up: the solid, then the adjustment's staging draw.
    assert_eq!(drawn.len(), 2);
    assert!(
        matches!(drawn[1].as_slice(), [LayerInputDraw::ThisLayer]),
        "an adjustment layer's flare must read the composite below it"
    );

    // 2. An ordinary layer pointed at itself reads its own input too — no
    //    second render of the same picture.
    let mut own = base.clone();
    own.id = Uuid::now_v7();
    own.effects.push(flare_on(own.id, 1, Some(own.id)));
    let comp = comp_of(vec![own.clone()]);
    assert!(
        matches!(slots(&comp)[0].as_slice(), [LayerInputDraw::ThisLayer]),
        "a layer's own flare must read its own input"
    );

    // 3. Absent while the Source type is Manual, and while the reference is
    //    unset — both still the labelled no-flare they always were.
    //
    //    The Manual case points at a layer that really is in the comp, on
    //    purpose: the flare's matte comes off the same carriage as every
    //    other effect's, and nothing in that carriage knows what a Source
    //    type is. What keeps it absent is the general rule that a row the panel
    //    does not show fills no slot (`param_visible`) — and a test that let the
    //    reference dangle would pass without that rule ever running, while a
    //    real project paid for a layer render per frame it never looked at.
    let mut target = base.clone();
    target.id = Uuid::now_v7();
    target.name = "matte source".into();
    for (source_type, matte) in [(0u32, Some(target.id)), (1, None)] {
        let mut quiet = base.clone();
        quiet.id = Uuid::now_v7();
        quiet.effects.push(flare_on(quiet.id, source_type, matte));
        let comp = comp_of(vec![quiet, target.clone()]);
        // Bottom-up, so the flare's own draw is last.
        let drawn = slots(&comp);
        assert!(
            matches!(drawn[drawn.len() - 1].as_slice(), [LayerInputDraw::Absent]),
            "source {source_type} / matte {matte:?} must stay absent"
        );
    }
}

// --- Settings → Export filename template -------------------------------

/// A paint stroke is stamped into the layer's own pixels before its masks gate
/// them — the render side of the feature, checked where the pixels are
/// actually made rather than through a GPU nobody has on CI.
#[test]
fn a_paint_stroke_reaches_the_layers_pixels() {
    let solid_id = Uuid::now_v7();
    let mut layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "solid".into(),
        kind: LayerKind::Solid { def: solid_id },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let mut stroke = lumit_core::paint::PaintStroke::new("Brush 1", vec![(20.0, 20.0)]);
    stroke.width = 10.0;
    stroke.colour = LinearColour([1.0, 0.0, 0.0, 1.0]);
    layer.paint.push(stroke);

    let painted = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 40,
        height: 40,
        frame_rate: FrameRate::new(60, 1).unwrap(),
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
            width: 40,
            height: 40,
            extra: serde_json::Map::new(),
        },
    ));
    doc.items
        .push(lumit_core::model::ProjectItem::Composition(painted.clone()));

    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let mut visited = vec![painted.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(doc.clone()),
        &painted,
        0.0,
        &map,
        &mut visited,
    );
    assert_eq!(draws.len(), 1);
    let DrawSource::Pixels { rgba, tex_w, .. } = &draws[0].source else {
        panic!("a solid draws pixels");
    };
    assert_eq!(
        *tex_w, 40,
        "a painted solid is rasterised at its real size, not as an 8x8 tile"
    );
    let px = |x: u32, y: u32| {
        let i = ((y * tex_w + x) as usize) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    };
    assert_eq!(px(20, 20), [255, 0, 0], "the stroke is in the picture");
    assert_eq!(px(2, 2), [255, 255, 255], "and the solid elsewhere");
}

/// A puppet pin carries the layer's pixels with it, at the same seam paint and
/// masks act on (docs/impl/puppet.md §3, PU2) — checked where the pixels are
/// made, as the paint test above is, rather than through a GPU.
///
/// One pin, so the solve short-circuits to a pure translation (§2.3) and the
/// mark it drags is exactly eight pixels lower — an end-to-end assertion with
/// no tolerance in it.
#[test]
fn a_puppet_pin_carries_the_layers_pixels() {
    let solid_id = Uuid::now_v7();
    let mut layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "solid".into(),
        kind: LayerKind::Solid { def: solid_id },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    // A red mark to follow, in a white square that is otherwise featureless.
    let mut stroke = lumit_core::paint::PaintStroke::new("Brush 1", vec![(20.0, 20.0)]);
    stroke.width = 6.0;
    stroke.colour = LinearColour([1.0, 0.0, 0.0, 1.0]);
    layer.paint.push(stroke);

    let key = |t: i64, value: f64| lumit_core::anim::Keyframe {
        time: Rational::new(t, 1).unwrap(),
        value,
        interp_in: lumit_core::anim::SideInterp::Linear,
        interp_out: lumit_core::anim::SideInterp::Linear,
    };
    let mut block = lumit_core::puppet::PuppetBlock::new(Rational::ZERO);
    block.density = 8.0;
    let mut pin = lumit_core::puppet::PuppetPin::new(
        lumit_core::puppet::PuppetPinKind::Position,
        "Pin 1",
        20.0,
        20.0,
    );
    // Placed at the reference time and dragged eight pixels down by one second.
    pin.y.animation = lumit_core::anim::Animation::Keyframed(vec![key(0, 20.0), key(1, 28.0)]);
    block.pins.push(pin);
    layer.puppet = Some(block);

    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 40,
        height: 40,
        frame_rate: FrameRate::new(60, 1).unwrap(),
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
            width: 40,
            height: 40,
            extra: serde_json::Map::new(),
        },
    ));
    doc.items
        .push(lumit_core::model::ProjectItem::Composition(comp.clone()));

    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let one_second = |t: f64| {
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            t,
            &map,
            &mut visited,
        );
        let DrawSource::Pixels { rgba, tex_w, .. } = &draws[0].source else {
            panic!("a solid draws pixels");
        };
        let w = *tex_w;
        let rgba = rgba.clone();
        move |x: u32, y: u32| {
            let i = ((y * w + x) as usize) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        }
    };

    // At the reference time every pin is where it was placed, so the puppet is
    // an identity and the picture is the one paint left (§2.3's early-out).
    let at_rest = one_second(0.0);
    assert_eq!(at_rest(20, 20), [255, 0, 0, 255], "the mark, unmoved");

    let dragged = one_second(1.0);
    assert_eq!(
        dragged(20, 28),
        [255, 0, 0, 255],
        "the pin dragged the mark eight pixels down with it"
    );
    assert_eq!(
        dragged(20, 20),
        [255, 255, 255, 255],
        "and left white where it used to be"
    );
}

/// **The matte list is 1:1 with the ops that will consume it** (the
/// one-predicate/one-order rule with its second predicate).
///
/// Two ways the build side can drift from `run_ops` and neither shows as an
/// error — both show as a matte driving the wrong effect:
///
/// - a **bypassed** effect resolves to no op, so it must fill no slot;
/// - an **orchestration-only** effect (Posterize time) carries a Matte row like
///   everything else, but resolves to no op either — it changes what *time* the
///   layers below render at, and there is no per-pixel pass to dissolve.
///
/// So the assertion is the invariant itself: as many matte slots as the resolve
/// produces ops *that carry the pair* — which is the very rule `run_ops`
/// advances its counter by, and the opted-out Depth of field below is here to
/// hold both sides to it. A bound row lands as its own slot, an unset one as
/// `Absent`, and "this layer" as `ThisLayer`.
#[test]
fn the_matte_list_is_one_slot_per_resolved_op() {
    let solid_def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_def,
            name: "red".into(),
            colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        },
    ));
    let base = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "under".into(),
        kind: LayerKind::Solid { def: solid_def },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };

    let mut layer = base.clone();
    let mut bypassed = lumit_core::fx::instantiate("saturation").unwrap();
    bypassed.enabled = false;
    let mut pointed = lumit_core::fx::instantiate("glow").unwrap();
    for p in &mut pointed.params {
        if p.id == lumit_core::fx::MATTE_PARAM {
            p.value = lumit_core::model::EffectValue::Layer(Some(layer.id));
        }
    }
    layer.effects = vec![
        lumit_core::fx::instantiate("blur").unwrap(),
        bypassed,
        // Orchestration-only: a Matte row, but no op to hang it on.
        lumit_core::fx::instantiate("posterize_time").unwrap(),
        pointed,
        // Claims the matte under its own older id: still one slot on
        // the one carriage, filled from `depth` rather than `matte`.
        lumit_core::fx::instantiate("dof").unwrap(),
    ];

    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let mut visited = vec![comp.id];
    let draws = build_comp_draws(&std::sync::Arc::new(doc), &comp, 0.0, &map, &mut visited);
    let drawn = draws.first().expect("one layer, one draw");
    // The consumption side's own rule, spelled here: `run_ops` advances its
    // matte counter for exactly the ops whose role names a parameter.
    let want = drawn
        .fx
        .iter()
        .filter(|op| op.def.schema().matte.param().is_some())
        .count();
    assert_eq!(
        drawn.mattes.len(),
        want,
        "one matte slot per resolved op that declares a matte — no more, no \
         fewer ({} ops in all)",
        drawn.fx.len()
    );
    // Blur (unset), Glow (this layer), Depth of field (unset `depth`): three
    // ops, three slots, and the DoF's comes off the SAME list even though its
    // parameter is called something else — that is the one carriage, and a
    // DoF that fell out of this list would shift the glow's slot onto it.
    assert!(
        matches!(
            drawn.mattes.as_slice(),
            [
                crate::draw::LayerInputDraw::Absent,
                crate::draw::LayerInputDraw::ThisLayer,
                crate::draw::LayerInputDraw::Absent,
            ]
        ),
        "the bypassed and orchestration-only effects fill no slot, and the \
         effects that claim the matte themselves are on the same list: {:?} \
         slots for {} ops",
        drawn.mattes.len(),
        drawn.fx.len()
    );
}

/// **One predicate, one order, for the mask paths too**.
///
/// The list a layer's draw carries must be 1:1 and in stack order with the
/// resolved ops whose effect declares a
/// [`ParamKind::MaskPath`](lumit_core::fx::ParamKind::MaskPath) row, because
/// that is exactly what `fxops::run_ops`'s own counter walks. A build side that
/// filled a slot per *instance* rather than per *op* — forgetting the bypassed
/// effect, or the orchestration-only one that resolves to nothing — would hand
/// one effect's shape to another and there would be no error to see, only a
/// brush walking the wrong curve.
///
/// Stated over the catalogue rather than over one effect on purpose. **No
/// built-in declares a path row yet** (the seam landed ahead of its
/// consumers), so both sides come to zero today and the assertion is that they
/// agree; the day Scribble, Stroke or Vegas's Mask/Path source lands, the same
/// test counts its slot without being touched.
#[test]
fn the_mask_path_list_is_one_to_one_with_the_ops_that_declare_a_path() {
    let solid_def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(lumit_core::model::ProjectItem::Solid(
        lumit_core::model::SolidDef {
            id: solid_def,
            name: "red".into(),
            colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
            width: 64,
            height: 64,
            extra: serde_json::Map::new(),
        },
    ));
    let mut layer = Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "masked".into(),
        kind: LayerKind::Solid { def: solid_def },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        // Two masks, so "First mask" has something to be first of and a
        // second shape exists to be picked wrongly.
        masks: vec![
            lumit_core::mask::Mask::rectangle(0.0, 0.0, 32.0, 32.0),
            lumit_core::mask::Mask::ellipse(48.0, 48.0, 8.0, 8.0),
        ],
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    // Every built-in, on one layer: a bypassed effect (no op), an
    // orchestration-only effect (a row, no op) and every image op there is.
    let mut bypassed = lumit_core::fx::instantiate("saturation").unwrap();
    bypassed.enabled = false;
    layer.effects.push(bypassed);
    layer
        .effects
        .push(lumit_core::fx::instantiate("posterize_time").unwrap());
    for schema in lumit_core::fx::BUILTINS {
        if let Some(inst) = lumit_core::fx::instantiate(schema.match_name) {
            layer.effects.push(inst);
        }
    }

    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let mut visited = vec![comp.id];
    let doc = std::sync::Arc::new(doc);
    let draws = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);
    let drawn = draws.first().expect("one layer, one draw");
    // The consumption side's own rule, spelled out: `run_ops` advances its
    // path counter for exactly the ops whose schema declares a path row.
    let want: usize = drawn
        .fx
        .iter()
        .map(|op| op.def.schema().mask_path_count())
        .sum();
    // The catalogue really does declare more than one row somewhere, or the
    // count below would agree for the trivial reason.
    assert!(
        want > drawn
            .fx
            .iter()
            .filter(|op| op.def.schema().mask_path().is_some())
            .count(),
        "no effect declares a second path row - the per-row rule is untested"
    );
    assert_eq!(
        drawn.mask_paths.len(),
        want,
        "one polyline per mask-path row of every resolved op — no more, no \
         fewer ({} ops in all)",
        drawn.fx.len()
    );
    // And the draw is deterministic: the same document at the same frame
    // builds the same geometry, which is what lets a frame key name it.
    let mut visited = vec![comp.id];
    let again = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);
    assert_eq!(
        drawn.mask_paths,
        again.first().expect("one draw").mask_paths,
        "two builds of one document flattened differently"
    );
}

/// **Text on a path**: the layer's picture is drawn into the box the
/// curve asks for, and a row naming nothing — or a mask since deleted — lays
/// the line straight rather than emptying the layer (docs/14 §4).
#[test]
fn a_text_layer_on_a_path_draws_into_the_paths_own_box() {
    use lumit_core::model::TextDocument;
    let mask = lumit_core::mask::Mask::rectangle(20.0, 30.0, 300.0, 120.0);
    let layer = |path: Option<Uuid>| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "type".into(),
        kind: LayerKind::Text {
            document: TextDocument {
                text: "Lumit".into(),
                expression: None,
                size: 48.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
                path,
                path_offset: lumit_core::anim::Property::zero(),
                animators: Vec::new(),
                extra: serde_json::Map::new(),
            },
        },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        // The mask is geometry only, so it gates nothing and only lends its
        // curve — which is exactly the mask somebody draws to run type along.
        masks: vec![lumit_core::mask::Mask {
            mode: lumit_core::mask::MaskMode::None,
            ..mask.clone()
        }],
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let sizes = |path: Option<Uuid>| {
        let comp = Composition {
            master_volume_db: 0.0,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "C".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer(path)],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = std::sync::Arc::new(Document::new());
        let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);
        assert_eq!(draws.len(), 1);
        draws[0].natural_size
    };

    let straight = sizes(None);
    // The rectangle reaches (320, 150), plus one text size of room for what
    // sits above and below the baseline: 368 × 198, corner at the layer's own
    // origin so the mask still means what it meant.
    assert_eq!(sizes(Some(mask.id)), (368.0, 198.0));
    // A mask that is not there is not a path: the line lays straight, exactly
    // as it does with the row unset.
    assert_eq!(sizes(Some(Uuid::now_v7())), straight);
    assert!(straight.0 > 0.0 && straight.0 < 368.0, "{straight:?}");
}

/// **A matte is image content** (docs/impl/ocio.md §5.2): a track matte
/// drawn from a footage layer carries that item's colour space to the realiser,
/// exactly as the layer's own pixels do, so log footage used as a matte is
/// interpreted rather than assumed. The layer being gated keeps its own space,
/// whatever the matte's is.
#[test]
fn a_matte_from_tagged_footage_carries_its_own_colour_space() {
    use lumit_core::model::{
        FootageItem, LayerInputSource, MatteChannel, MatteRef, MediaRef, ProjectItem,
    };

    let footage = |name: &str, space: Option<&str>| FootageItem {
        sequence: None,
        id: Uuid::now_v7(),
        name: name.into(),
        media: MediaRef {
            relative_path: name.into(),
            absolute_path: name.into(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: space.map(str::to_owned),
        extra: serde_json::Map::new(),
    };
    let plate = footage("plate.mov", Some("ACEScct"));
    let holdout = footage("holdout.mov", Some("srgb_texture"));

    let layer = |item: Uuid, name: &str| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind: LayerKind::Footage { item },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let matte_layer = layer(holdout.id, "holdout");
    let mut gated = layer(plate.id, "plate");
    gated.matte = Some(MatteRef {
        layer: matte_layer.id,
        channel: MatteChannel::Alpha,
        inverted: false,
        source: LayerInputSource::None,
    });

    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 640,
        height: 360,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![gated.clone(), matte_layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };

    let pixels = |l: &Layer| CompLayerPixels {
        layer: l.id,
        width: 640,
        height: 360,
        rgba: vec![0u8; 640 * 360 * 4],
        natural_w: 640,
        natural_h: 360,
        temporal: Vec::new(),
        flow_fields: Vec::new(),
        shutter: Vec::new(),
        source_key: 0,
        source_frame: 0,
    };
    let (gp, mp) = (pixels(&gated), pixels(&matte_layer));
    let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    map.insert(gated.id, &gp);
    map.insert(matte_layer.id, &mp);

    let mut doc = Document::new();
    doc.items.push(ProjectItem::Footage(plate));
    doc.items.push(ProjectItem::Footage(holdout));
    let doc = std::sync::Arc::new(doc);
    let mut visited = vec![comp.id];
    let draws = build_comp_draws(&doc, &comp, 0.0, &map, &mut visited);

    let gated_draw = draws
        .iter()
        .find(|d| d.matte.is_some())
        .expect("the gated layer draws with its matte");
    assert_eq!(
        gated_draw.matte.as_ref().unwrap().colour_space.as_deref(),
        Some("srgb_texture"),
        "the matte reads through the space its own footage was tagged with"
    );
    match &gated_draw.source {
        DrawSource::Pixels { colour_space, .. } => assert_eq!(
            colour_space.as_deref(),
            Some("ACEScct"),
            "and the layer it gates keeps its own"
        ),
        other => panic!(
            "footage draws pixels, not {:?}",
            std::mem::discriminant(other)
        ),
    }
}

// ---------------------------------------------------------------------------
// Effects on a layer group (docs/impl/group-effects.md §2): the wrap,
// asserted on the draw list itself so it runs on CI machines with no GPU.
// ---------------------------------------------------------------------------

/// A solid layer for the group tests below — solids draw with no decoded
/// pixels, which keeps these tests device-free.
fn grouped_scene(
    effects: Vec<lumit_core::model::EffectInstance>,
    member_visible: bool,
) -> (std::sync::Arc<Document>, Composition) {
    use lumit_core::model::{ProjectItem, SolidDef};
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "s".into(),
        colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
        width: 16,
        height: 16,
        extra: serde_json::Map::new(),
    }));
    let mk = |name: &str| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind: LayerKind::Solid { def },
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
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let above = mk("above");
    let mut member = mk("member");
    member.switches.visible = member_visible;
    let below = mk("below");
    let group = lumit_core::group::LayerGroup {
        id: Uuid::now_v7(),
        name: "band".into(),
        label: 0,
        members: vec![member.id],
        effects,
    };
    let comp = Composition {
        master_volume_db: 0.0,
        groups: vec![group],
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 64,
        height: 64,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![above, member, below],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    (std::sync::Arc::new(doc), comp)
}

fn draws_of(doc: &std::sync::Arc<Document>, comp: &Composition) -> Vec<crate::draw::CompLayerDraw> {
    let map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    let mut visited = vec![comp.id];
    build_comp_draws(doc, comp, 0.0, &map, &mut visited)
}

// While the header carries no live effect the walk stays group-blind — the
// same three plain draws, in the same order, no Nested unit.
#[test]
fn a_group_with_no_live_header_builds_the_ungrouped_draws() {
    let (doc, comp) = grouped_scene(Vec::new(), true);
    let mut bypassed = lumit_core::fx::instantiate("blur").expect("a builtin");
    bypassed.enabled = false;
    let (doc_b, comp_b) = grouped_scene(vec![bypassed], true);

    for (doc, comp) in [(&doc, &comp), (&doc_b, &comp_b)] {
        let draws = draws_of(doc, comp);
        assert_eq!(draws.len(), 3, "one plain draw per layer, no unit");
        assert!(
            draws
                .iter()
                .all(|d| matches!(d.source, DrawSource::Pixels { .. })),
            "and none of them is a Nested wrap"
        );
        // Bottom-up build order, unchanged.
        let ids: Vec<Uuid> = draws.iter().map(|d| d.layer).collect();
        let mut stack: Vec<Uuid> = comp.layers.iter().map(|l| l.id).collect();
        stack.reverse();
        assert_eq!(ids, stack);
    }
}

// A live header wraps exactly the member run in one comp-sized Nested draw
// carrying the resolved stack, between the unwrapped neighbours.
#[test]
fn a_live_header_wraps_the_run_in_one_nested_draw() {
    let blur = lumit_core::fx::instantiate("blur").expect("a builtin");
    let (doc, comp) = grouped_scene(vec![blur], true);
    let draws = draws_of(&doc, &comp);
    assert_eq!(draws.len(), 3, "below, the unit, above");
    // Bottom-up: below first, then the unit, then above.
    assert_eq!(draws[0].layer, comp.layers[2].id);
    assert_eq!(draws[2].layer, comp.layers[0].id);
    let unit = &draws[1];
    assert_eq!(
        unit.layer, comp.groups[0].id,
        "the profiler's row is the group"
    );
    let DrawSource::Nested {
        width,
        height,
        draws: inner,
        key,
        ..
    } = &unit.source
    else {
        panic!("a live header builds a Nested unit");
    };
    assert_eq!((*width, *height), (comp.width, comp.height), "comp-sized");
    assert!(key.is_none(), "uncached in v1 (§4)");
    assert_eq!(inner.len(), 1, "the member's own draw, unchanged, inside");
    assert_eq!(inner[0].layer, comp.layers[1].id);
    assert!(!unit.fx.is_empty(), "the header's stack rides the unit");
    assert_eq!(unit.fx_ids.len(), unit.fx.len());
    assert_eq!(unit.opacity, 100.0);
    assert!(!unit.three_d);
    assert_eq!(
        unit.fx_ref_width,
        Some(comp.width as f32),
        "resolved at comp scale, rescaled by realise"
    );
}

// An empty run runs nothing: every member gated out means no unit draw at
// all, however loud the header's stack.
#[test]
fn an_empty_run_contributes_no_unit_draw() {
    let blur = lumit_core::fx::instantiate("blur").expect("a builtin");
    let (doc, comp) = grouped_scene(vec![blur], false);
    let draws = draws_of(&doc, &comp);
    assert_eq!(draws.len(), 2, "the two neighbours and nothing else");
    assert!(
        draws.iter().all(|d| d.layer != comp.groups[0].id),
        "no unit for an empty run"
    );
}

/// The colour-table list (docs/impl/effect-registry.md §2.5a): one slot per
/// `lut` or OCIO op, in stack order, on the predicate `run_ops` counts by; an
/// OCIO effect with nothing to do fills its slot with `None`.
#[test]
fn colour_tables_fill_one_slot_per_table_effect_in_stack_order() {
    use crate::colour::{Edge, OcioRequest, TableRequest};
    use lumit_core::model::{EffectValue, FileParam};
    let set = |e: &mut lumit_core::model::EffectInstance, id: &str, v: EffectValue| {
        if let Some(p) = e.params.iter_mut().find(|p| p.id == id) {
            p.value = v;
        }
    };
    let text = |s: &str| EffectValue::Text(s.into());

    let sat = lumit_core::fx::instantiate("saturation").unwrap();
    let mut display = lumit_core::fx::instantiate("ocio_display").unwrap();
    set(&mut display, "display", text("sRGB"));
    set(&mut display, "view", text("Standard"));
    set(&mut display, "inverse", EffectValue::Bool(true));
    let unset_display = lumit_core::fx::instantiate("ocio_display").unwrap();
    let mut lut = lumit_core::fx::instantiate("lut").unwrap();
    set(
        &mut lut,
        "file",
        EffectValue::File(FileParam::single("grade.cube")),
    );
    let mut off = lumit_core::fx::instantiate("ocio_look").unwrap();
    set(&mut off, "look", text("warm"));
    off.enabled = false;
    let mut convert = lumit_core::fx::instantiate("ocio_colour_space").unwrap();
    set(&mut convert, "output_colour_space", text("ACEScg"));
    let mut file = lumit_core::fx::instantiate("ocio_file").unwrap();
    set(
        &mut file,
        "file",
        EffectValue::File(FileParam::single("grade.cc")),
    );

    let effects = vec![sat, display, unset_display, lut, off, convert, file];
    assert_eq!(
        crate::build::colour_tables(&effects, 0.0),
        vec![
            Some(TableRequest::Ocio(OcioRequest::Config(Edge::Display {
                input: String::new(),
                display: "sRGB".into(),
                view: "Standard".into(),
                inverse: true,
            }))),
            None,
            Some(TableRequest::Cube("grade.cube".into())),
            Some(TableRequest::Ocio(OcioRequest::Config(Edge::Convert {
                from: String::new(),
                to: "ACEScg".into(),
            }))),
            Some(TableRequest::Ocio(OcioRequest::File {
                path: "grade.cc".into(),
                inverse: false,
            })),
        ]
    );
}

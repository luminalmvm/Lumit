//! Draw-building tests: layer geometry under reduced-resolution decode,
//! collapsed Precomps, the live value patch, and adjustment staging.
//!
//! These moved out of the egui shell with the pixel pass (K-178) — they always
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
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
        source_key: 0,
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
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "inner".into(),
        kind: LayerKind::Text {
            document: TextDocument {
                text: "hi".into(),
                expression: None,
                size: 24.0,
                fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let nested = Composition {
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
    // background colour (K-241): the nested comp here is opaque black, and a
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
}

// The live value-drag preview renders a comp patched with the provisional
// value. Patching a layer's Position X to 500 must show through as the
// draw's position, without touching the committed document.
#[test]
fn patch_layer_prop_overrides_the_previewed_value() {
    use lumit_core::model::TransformProp;
    let item = Uuid::now_v7();
    let layer = Layer {
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let comp = Composition {
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
        source_key: 0,
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
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

/// **A Lens flare on an adjustment layer flares the picture below it**
/// (K-288). The regression: the flare's Matte source could only name
/// *another* layer, and an adjustment layer has no picture of its own, so
/// putting the effect on one meant hunting for some other layer to point at
/// — and whichever you picked was the wrong picture, since an adjustment
/// layer is supposed to act on everything beneath it.
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
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
    //    purpose: since K-395 the flare's matte comes off the same carriage as
    //    every other effect's, and nothing in that carriage knows what a Source
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

// --- K-119: Settings → Export filename template ------------------------

/// A paint stroke is stamped into the layer's own pixels before its masks gate
/// them (K-227) — the render side of the feature, checked where the pixels are
/// actually made rather than through a GPU nobody has on CI.
#[test]
fn a_paint_stroke_reaches_the_layers_pixels() {
    let solid_id = Uuid::now_v7();
    let mut layer = Layer {
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    let mut stroke = lumit_core::paint::PaintStroke::new("Brush 1", vec![(20.0, 20.0)]);
    stroke.width = 10.0;
    stroke.colour = LinearColour([1.0, 0.0, 0.0, 1.0]);
    layer.paint.push(stroke);

    let painted = Composition {
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

/// **The matte list is 1:1 with the ops that will consume it** (K-395, the
/// K-387 one-predicate/one-order rule with its second predicate).
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
/// `Absent`, and "this layer" as `ThisLayer` (K-288).
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
        audio_only: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        effects: Vec::new(),
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
        // Claims the matte under its own older id (K-395): still one slot on
        // the one carriage, filled from `depth` rather than `matte`.
        lumit_core::fx::instantiate("dof").unwrap(),
    ];

    let comp = Composition {
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
    // parameter is called something else — that is the K-395 consolidation, and
    // a DoF that fell out of this list would shift the glow's slot onto it.
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

/// **One predicate, one order, for the mask paths too** (K-387, K-408).
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
/// built-in declares a path row yet** (K-408 landed the seam ahead of its
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
        audio_only: false,
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
        effects: Vec::new(),
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
    let want = drawn
        .fx
        .iter()
        .filter(|op| op.def.schema().mask_path().is_some())
        .count();
    assert_eq!(
        drawn.mask_paths.len(),
        want,
        "one polyline per resolved op that declares a mask path — no more, no \
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

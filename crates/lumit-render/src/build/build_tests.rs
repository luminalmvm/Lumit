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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        rgba: vec![0u8; 480 * 270 * 4].into(),
        format: lumit_media::PixelFormat::Srgb8,
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
                style: Default::default(),
                paragraph: Default::default(),
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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        rgba: vec![0u8; 16].into(),
        format: lumit_media::PixelFormat::Srgb8,
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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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
        graph_inputs: None,
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
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
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

// ---------------------------------------------------------------------------
// Effects on a layer group (docs/impl/group-effects.md §2): the wrap,
// asserted on the draw list itself so it runs on CI machines with no GPU.
// ---------------------------------------------------------------------------

// The float import's end of the bargain (docs/impl/media-io.md §5): a plate
// that decoded as floats has to reach the draw list still floats,
// because the draw list is the last place anything can say so before the
// upload picks a texture format. Reading those bytes as sRGB would not look
// slightly wrong, it would look like static.
//
// Built from a value above white, since that is the whole thing eight bits
// could not carry.
#[test]
fn a_float_plate_reaches_the_draw_list_still_float() {
    let (comp, layer_id, lp) = float_layer_comp(Vec::new(), Vec::new());
    let mut map: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
    map.insert(layer_id, &lp);
    let mut visited = vec![comp.id];
    let draws = build_comp_draws(
        &std::sync::Arc::new(Document::new()),
        &comp,
        0.0,
        &map,
        &mut visited,
    );

    assert_eq!(draws.len(), 1);
    match &draws[0].source {
        DrawSource::Pixels { format, rgba, .. } => {
            assert_eq!(*format, lumit_media::PixelFormat::LinearF32);
            // Eight bytes a pixel, and the value untouched on the way through.
            assert_eq!(rgba.len(), 4 * 4 * 16);
            assert_eq!(float_px(rgba, 0)[0], 4.0);
        }
        _ => panic!("expected a pixel source for a footage layer"),
    }
}

/// Read pixel `n`'s four channels out of a float buffer.
fn float_px(rgba: &[u8], n: usize) -> [f32; 4] {
    lumit_core::pixels::f32_px(rgba, n)
}

/// A one-layer comp over a 4×4 float plate carrying 4.0 in every colour
/// channel and an opaque alpha, with whatever masks and paint the caller wants
/// on the layer.
fn float_layer_comp(
    masks: Vec<lumit_core::mask::Mask>,
    paint: Vec<lumit_core::paint::PaintStroke>,
) -> (Composition, Uuid, CompLayerPixels) {
    let item = Uuid::now_v7();
    let mut layer = Layer {
        graph: Default::default(),
        graph_inputs: None,
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "plate".into(),
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
        masks,
        paint,
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    };
    layer.name = "plate".into();
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 4,
        height: 4,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer.clone()],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let mut rgba = Vec::new();
    for _ in 0..4 * 4 {
        for v in [4.0f32, 4.0, 4.0, 1.0] {
            rgba.extend_from_slice(&v.to_le_bytes());
        }
    }
    let lp = CompLayerPixels {
        layer: layer.id,
        width: 4,
        height: 4,
        rgba: rgba.into(),
        format: lumit_media::PixelFormat::LinearF32,
        natural_w: 4,
        natural_h: 4,
        temporal: Vec::new(),
        flow_fields: Vec::new(),
        shutter: Vec::new(),
        source_key: 0,
        source_frame: 0,
    };
    (comp, layer.id, lp)
}

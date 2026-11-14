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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        lighting: Default::default(),
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
        at: 0.0,
        elsewhere: Vec::new(),
        width: 480,
        height: 270,
        rgba: vec![0u8; 480 * 270 * 4].into(),
        format: lumit_media::decode::PixelFormat::Srgb8,
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        lighting: Default::default(),
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
        lighting: Default::default(),
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
    let expect = lumit_gpu::composite::place_matrix(
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        lighting: Default::default(),
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
        at: 0.0,
        elsewhere: Vec::new(),
        width: 1920,
        height: 1080,
        rgba: vec![0u8; 16].into(),
        format: lumit_media::decode::PixelFormat::Srgb8,
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        .push(lumit_core::fx::builtins::instantiate("saturation").unwrap());
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
        lighting: Default::default(),
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        lighting: Default::default(),
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        lighting: Default::default(),
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
    let mut bypassed = lumit_core::fx::builtins::instantiate("saturation").unwrap();
    bypassed.enabled = false;
    let mut pointed = lumit_core::fx::builtins::instantiate("glow").unwrap();
    for p in &mut pointed.params {
        if p.id == lumit_core::fx::schema::MATTE_PARAM {
            p.value = lumit_core::model::EffectValue::Layer(Some(layer.id));
        }
    }
    layer.effects = vec![
        lumit_core::fx::builtins::instantiate("blur").unwrap(),
        bypassed,
        // Orchestration-only: a Matte row, but no op to hang it on.
        lumit_core::fx::builtins::instantiate("posterize_time").unwrap(),
        pointed,
        // Claims the matte under its own older id: still one slot on
        // the one carriage, filled from `depth` rather than `matte`.
        lumit_core::fx::builtins::instantiate("dof").unwrap(),
    ];

    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
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
    let held = draws.first().expect("one layer, one draw");
    // The Posterize time holds the whole layer, so the layer is drawn,
    // effects and all, inside the held picture.
    let DrawSource::Nested { draws: inside, .. } = &held.source else {
        panic!("a held layer is drawn as a picture");
    };
    let drawn = inside.first().expect("the layer, inside");
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
            assert_eq!(*format, lumit_media::decode::PixelFormat::LinearF32);
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
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
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
        lighting: Default::default(),
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
        at: 0.0,
        elsewhere: Vec::new(),
        width: 4,
        height: 4,
        rgba: rgba.into(),
        format: lumit_media::decode::PixelFormat::LinearF32,
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

/// **A Proximity selector follows its null**: the letters nearest the null
/// are moved most, and dragging the null to the other end of the word moves
/// the other letters instead.
#[test]
fn a_proximity_selector_follows_its_null() {
    use lumit_core::anim::Property;
    use lumit_core::model::{ProjectItem, TextDocument};
    use lumit_core::text::selector::{RangeSelector, SelectorKind, SelectorShape};
    use lumit_core::text::{AnimatorAxis, TextAnimator};
    let layer = |kind: LayerKind| Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "layer".into(),
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
    let null = layer(LayerKind::Null);
    let mut animator = TextAnimator::new("Near");
    animator.selector = RangeSelector {
        kind: SelectorKind::Proximity,
        target: Some(null.id),
        radius: Property::fixed(150.0),
        shape: SelectorShape::Ramp,
        ..RangeSelector::default()
    };
    animator.position_y = Property::fixed(-20.0);
    animator.axes.push(AnimatorAxis {
        tag: "wght".into(),
        value: Property::fixed(900.0),
    });
    let document = TextDocument {
        text: "WWWWWW".into(),
        expression: None,
        size: 72.0,
        fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
        path: None,
        path_offset: Property::zero(),
        animators: vec![animator],
        style: Default::default(),
        paragraph: Default::default(),
        extra: serde_json::Map::new(),
    };
    let text = layer(LayerKind::Text {
        document: document.clone(),
    });
    let mut comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Comp".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![null, text],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    lumit_core::text::install_glyph_middles(lumit_text::glyph_middles);
    let middles = lumit_text::glyph_middles(&comp.layers[1], &document, "WWWWWW", 0.0);
    let (first, last) = (middles[0].unwrap(), middles[5].unwrap());
    let pushes = |comp: &Composition, at: [f32; 2]| {
        let mut comp = comp.clone();
        comp.layers[0].transform.position_x = Property::fixed(f64::from(at[0]));
        comp.layers[0].transform.position_y = Property::fixed(f64::from(at[1]));
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp.clone()));
        let doc = std::sync::Arc::new(doc);
        let x = lumit_core::text::layer_glyph_xforms(
            &doc,
            &comp,
            &comp.layers[1],
            &document,
            "WWWWWW",
            0.0,
        );
        (
            x.iter().map(|x| x.position[1]).collect::<Vec<f32>>(),
            x[0].axes.len,
        )
    };
    let (on_first, axes) = pushes(&comp, first);
    assert!((on_first[0] + 20.0).abs() < 1e-3, "{on_first:?}");
    assert_eq!(on_first[5], 0.0, "{on_first:?}");
    assert_eq!(axes, 1, "the reached letter is set on the weight axis");
    comp.layers[1].transform = TransformGroup::default();
    let (on_last, _) = pushes(&comp, last);
    assert_eq!(on_last[0], 0.0, "{on_last:?}");
    assert!((on_last[5] + 20.0).abs() < 1e-3, "{on_last:?}");
}

/// **An Echo follows its layer's motion.** A solid moved by Position keys,
/// with a Blur above its Echo: the layer is drawn into a comp-sized picture
/// that the Echo runs on, and each picture the Echo reads is the solid where
/// it was at that earlier frame, blurred as it was then.
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
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
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
        lighting: Default::default(),
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

    // The frame's name holds those earlier moments: a key a frame back moved
    // changes it, and with the Echo off it does not.
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

/// A 10 px white solid moved from x 20 to x 80 over the first second of a
/// 100 x 40 comp at 30 fps, with `effects`.
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
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
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
        lighting: Default::default(),
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

/// One effect by match name with some of its number rows set.
fn effect_with(name: &str, rows: &[(&str, f64)]) -> lumit_core::model::EffectInstance {
    let mut e = lumit_core::fx::builtins::instantiate(name).unwrap();
    e.enabled = true;
    for (id, v) in rows {
        let p = e.params.iter_mut().find(|p| p.id == *id).unwrap();
        p.value = lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(*v));
    }
    e
}

/// **Motion blur, Timewarp and Posterize time follow the layer's motion**,
/// as Echo does: each reads the solid where it was at the moments it reads.
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

    // Motion blur measures the solid's own motion against the next frame.
    let blurred = draw(vec![effect_with("motion_blur", &[])]);
    assert!((f64::from(members(&blurred)) - x_at(0.55)).abs() < 1e-3);
    assert_eq!(blurred.flow_below.len(), 1);
    let (offset, then, _, measure) = &blurred.flow_below[0];
    assert_eq!((*offset, *measure), (1, Some(1)));
    let next = 0.55 + 1.0 / 30.0;
    assert!((f64::from(then[0].position.0) - x_at(next)).abs() < 1e-3);

    // Posterize time holds the whole layer, its position too.
    let held = draw(vec![effect_with("posterize_time", &[("rate", 2.0)])]);
    assert!(
        (f64::from(members(&held)) - x_at(0.5)).abs() < 1e-3,
        "held at half a second: {}",
        members(&held)
    );
    assert!(held.fx_ids.is_empty(), "nothing runs on the held picture");

    // Timewarp at half speed shows the solid where it was a quarter second
    // in, around the frame's moment.
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

/// **Echo time spaces the echoes**, as in After Effects: a tenth of a second
/// back, each echo is the solid where it was that much earlier; ahead, where
/// it will be.
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

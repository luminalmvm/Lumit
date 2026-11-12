//! A float plate reaching the draw list still float.

use super::*;

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
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
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
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
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

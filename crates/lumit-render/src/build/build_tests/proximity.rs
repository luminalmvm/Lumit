//! A Proximity selector following its null.

use super::*;

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
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
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
        grade_versions: None,
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
        captions: None,
        extra: serde_json::Map::new(),
    };
    let text = layer(LayerKind::Text {
        document: document.clone(),
    });
    let mut comp = Composition {
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
        let x = lumit_core::text::ports::layer_glyph_xforms(
            &doc,
            &comp,
            &comp.layers[1],
            &document,
            "WWWWWW",
            0.0,
            &[],
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

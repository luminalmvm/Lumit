//! **A copy of a graphics template draws its own values.**
//!
//! A template is a composition with controls, and each Precomp layer or clip
//! placing it can set them for itself. Nothing in the template's composition
//! changes when a copy sets a value, so the picture, the nested frame's name
//! and the frame's own name all have to follow the copy and not the
//! composition. This file is that claim made through the entry the Viewer and
//! the exporter both use.
//!
//! The template is a white card with one control, its opacity. Two copies sit
//! side by side in one frame, the left one set to nothing and the right one
//! left alone. A nested frame named by its composition alone would be one
//! picture for both: here the left half must be dark and the right half lit.

// A test binary: a failed setup step should stop this test, loudly, and the
// no-panic rule of docs/14 is about the engine's own paths.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::fx::effects::template as values;
use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
    TransformGroup, TransformProp,
};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::template::{Control, ControlKind, Target, Template, Value};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

/// The card is this wide and the frame twice it, so two copies fill a frame.
const CARD: u32 = 32;
const HEIGHT: u32 = 64;

fn layer(name: &str, kind: LayerKind) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(1, 1).unwrap()),
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
    }
}

fn comp_of(name: &str, width: u32, layers: Vec<Layer>) -> Composition {
    Composition {
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
        name: name.into(),
        width,
        height: HEIGHT,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(1, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

/// What a copy does with the template's one control.
#[derive(Clone, Copy, PartialEq)]
enum Copy {
    /// Sets nothing, so it shows the template as it is.
    Follows,
    /// Sets the card's opacity to nothing.
    Hidden,
}

/// The project: the template, and a composition placing it twice as Precomp
/// layers, or once as a clip.
///
/// Answers the document and the placing composition's id.
fn project(left: Copy, right: Option<Copy>, as_clip: bool) -> (Arc<Document>, Uuid) {
    let card = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: card,
        name: "card".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: CARD,
        height: HEIGHT,
        extra: serde_json::Map::new(),
    }));
    let plate = layer("card", LayerKind::Solid { def: card });
    let control = Control::new(
        "opacity".into(),
        "Opacity".into(),
        ControlKind::Number,
        Target::Transform {
            layer: plate.id,
            props: vec![TransformProp::Opacity],
        },
    );
    let template = Template {
        controls: vec![control.clone()],
        ..Template::default()
    };
    let mut inner = comp_of("template", CARD, vec![plate]);
    inner.set_template(Some(&template));
    let inner_id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));

    let values_of = |copy: Copy| {
        (copy == Copy::Hidden).then(|| {
            let mut inst = values::instance_for(inner_id, &template);
            assert!(values::set_value(&mut inst, &control, &Value::Number(0.0)));
            inst
        })
    };
    let layers = if as_clip {
        let second = Rational::new(1, 1).unwrap();
        let mut clip = Clip::new(
            ClipSource::Comp(inner_id),
            Rational::ZERO,
            second,
            Rational::ZERO,
            second,
        );
        clip.inputs = values_of(left);
        vec![layer("cut", LayerKind::Sequence { clips: vec![clip] })]
    } else {
        let mut first = layer("left", LayerKind::Precomp { comp: inner_id });
        first.graph_inputs = values_of(left);
        let mut layers = vec![first];
        if let Some(right) = right {
            let mut second = layer("right", LayerKind::Precomp { comp: inner_id });
            second.transform.position_x = Property::fixed(f64::from(CARD));
            second.graph_inputs = values_of(right);
            layers.push(second);
        }
        layers
    };
    let parent = comp_of("parent", CARD * 2, layers);
    let parent_id = parent.id;
    doc.items.push(ProjectItem::Composition(parent));
    (Arc::new(doc), parent_id)
}

/// Whether the left card's place and the right card's place are lit.
fn halves(rgba: &[u8]) -> (bool, bool) {
    let width = (CARD * 2) as usize;
    let at = |x: usize| rgba[((HEIGHT as usize / 2) * width + x) * 4] > 128;
    (at(CARD as usize / 2), at(CARD as usize + CARD as usize / 2))
}

#[test]
fn each_copy_of_a_template_draws_the_values_it_set() {
    let Ok(mut r) = HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut render = |left: Copy, right: Option<Copy>, as_clip: bool| {
        let (doc, comp) = project(left, right, as_clip);
        halves(&r.render_rgba(&doc, comp, 3, 1.0).expect("the render").0)
    };

    // Two copies in one frame, one set and one not: two pictures.
    assert_eq!(
        render(Copy::Hidden, Some(Copy::Follows), false),
        (false, true),
        "the copy set to nothing is dark and the copy left alone is lit"
    );
    assert_eq!(
        render(Copy::Follows, Some(Copy::Hidden), false),
        (true, false),
        "and the other way round, with the first frame's textures still kept"
    );
    assert_eq!(
        render(Copy::Follows, Some(Copy::Follows), false),
        (true, true),
        "a copy that sets nothing shows the template"
    );

    // A clip is a copy too, and its values are its own.
    assert_eq!(
        render(Copy::Follows, None, true),
        (true, false),
        "a clip that sets nothing shows the template"
    );
    assert_eq!(
        render(Copy::Hidden, None, true),
        (false, false),
        "a clip's value reaches its picture"
    );
}

//! Particulate's simulation through the whole renderer: a frame comes out the
//! same however it was reached, and particles hit what they should.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::Property;
use lumit_core::fx::builtins::instantiate;
use lumit_core::model::{
    Composition, Document, EffectInstance, EffectValue, Layer, LayerKind, LinearColour,
    ProjectItem, SolidDef, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::headless::HeadlessRenderer;
use std::sync::Arc;
use uuid::Uuid;

const W: u32 = 480;
const H: u32 = 270;

fn set(e: &mut EffectInstance, id: &str, value: EffectValue) {
    e.params
        .iter_mut()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no row {id}"))
        .value = value;
}

fn float(e: &mut EffectInstance, id: &str, v: f64) {
    set(e, id, EffectValue::Float(Property::fixed(v)));
}

fn solid_layer(def: Uuid) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: "Solid".into(),
        kind: LayerKind::Solid { def },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(30, 1).unwrap()),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup {
            anchor_x: Property::fixed(f64::from(W) * 0.5),
            anchor_y: Property::fixed(f64::from(H) * 0.5),
            position_x: Property::fixed(f64::from(W) * 0.5),
            position_y: Property::fixed(f64::from(H) * 0.5),
            ..TransformGroup::default()
        },
        matte: None,
        parent: None,
        label: 2,
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
    }
}

/// Particles falling from the top of a black solid, turned by the flow.
fn project(rows: impl FnOnce(&mut EffectInstance)) -> (Arc<Document>, Uuid) {
    let def = Uuid::now_v7();
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "Solid".into(),
        colour: LinearColour([0.0, 0.0, 0.0, 1.0]),
        width: W,
        height: H,
        extra: serde_json::Map::new(),
    }));
    let mut e = instantiate("particulate").unwrap();
    set(&mut e, "simulate", EffectValue::Bool(true));
    float(&mut e, "emit_rate", 3000.0);
    float(&mut e, "life", 3.0);
    float(&mut e, "gravity", 400.0);
    float(&mut e, "size", 3.0);
    float(&mut e, "position_x", 240.0);
    float(&mut e, "position_y", 30.0);
    float(&mut e, "turbulence_amount", 60.0);
    float(&mut e, "turbulence_scale", 80.0);
    rows(&mut e);
    let mut layer = solid_layer(def);
    layer.effects = vec![e];
    let comp = Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Sim".into(),
        width: W,
        height: H,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(Rational::new(30, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers: vec![layer],
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

fn pixels(doc: &Arc<Document>, comp: Uuid, frame: u64) -> Option<Vec<u8>> {
    let mut renderer = HeadlessRenderer::shared().ok()?;
    renderer
        .render_rgba(doc, comp, frame, 1.0)
        .ok()
        .map(|f| f.0)
}

/// A frame reached by scrubbing back to it, from a kept state, is the frame
/// walked from the in point. What export reads is what the Viewer showed.
#[test]
fn a_frame_is_the_same_however_it_is_reached() {
    // The same project twice over, so the second has nothing kept for it.
    let (doc, id) = project(|_| {});
    let Some(walked) = pixels(&doc, id, 40) else {
        return;
    };
    assert!(walked.iter().step_by(4).any(|v| *v > 40), "particles drew");
    let _ = pixels(&doc, id, 57);
    assert!(pixels(&doc, id, 40).unwrap() == walked, "scrubbed back");
    assert!(pixels(&doc, id, 33).unwrap() != walked, "another frame");
    assert!(pixels(&doc, id, 40).unwrap() == walked, "scrubbed on");
}

/// The floor stops what falls on it: below it nothing lands.
#[test]
fn the_ground_stops_what_falls() {
    let row =
        |pixels: &[u8], y: usize| (0..W as usize).any(|x| pixels[(y * W as usize + x) * 4] > 40);
    let (free, id) = project(|e| float(e, "turbulence_amount", 0.0));
    let Some(falling) = pixels(&free, id, 45) else {
        return;
    };
    assert!(row(&falling, 250), "with no floor they fall past it");
    let (floored, id) = project(|e| {
        set(e, "ground", EffectValue::Bool(true));
        float(e, "ground_height", -60.0);
        float(e, "turbulence_amount", 0.0);
    });
    let landed = pixels(&floored, id, 45).unwrap();
    assert!(!row(&landed, 250), "the floor at 195 holds them");
    assert!(row(&landed, 190), "and they land on it");
}

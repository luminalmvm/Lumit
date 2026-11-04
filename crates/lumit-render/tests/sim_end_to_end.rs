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

/// Walked from the in point on a renderer of its own, with nothing kept and no
/// picture cached.
fn walked_alone(doc: &Arc<Document>, comp: Uuid, frame: u64) -> Option<Vec<u8>> {
    let mut renderer = HeadlessRenderer::new().ok()?;
    renderer
        .render_rgba(doc, comp, frame, 1.0)
        .ok()
        .map(|f| f.0)
}

/// A frame reached from a kept state, after playing past it and coming back, is
/// the frame walked from the in point. Neither is a cached picture: this
/// renderer has never drawn it. What export reads is what the Viewer showed.
#[test]
fn a_frame_is_the_same_however_it_is_reached() {
    let (doc, id) = project(|_| {});
    let Some(walked) = walked_alone(&doc, id, 44) else {
        return;
    };
    assert!(walked.iter().step_by(4).any(|v| *v > 40), "particles drew");
    // On past it, keeping a state at 40 on the way, and back to it.
    let _ = pixels(&doc, id, 57);
    assert!(pixels(&doc, id, 44).unwrap() == walked, "scrubbed back");
    assert!(pixels(&doc, id, 33).unwrap() != walked, "another frame");
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

/// A 3D card lying flat stops what falls on it, and what misses it falls on.
#[test]
fn a_3d_card_stops_what_falls() {
    let row = |pixels: &[u8], y: usize, from: usize, to: usize| {
        (from..to).any(|x| pixels[(y * W as usize + x) * 4] > 40)
    };
    let carded = |card: bool| {
        project(|e| {
            float(e, "turbulence_amount", 0.0);
            float(e, "life", 2.0);
            let id = Uuid::now_v7();
            set(e, "collide_with", EffectValue::Layer(card.then_some(id)));
            CARD.with(|c| c.set(id));
        })
    };
    let with_card = |(doc, comp): (Arc<Document>, Uuid)| {
        let mut doc = Document::clone(&doc);
        let def = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: "Card".into(),
            colour: LinearColour([0.0, 0.0, 0.0, 0.0]),
            width: 400,
            height: 100,
            extra: serde_json::Map::new(),
        }));
        let mut card = solid_layer(def);
        card.id = CARD.with(std::cell::Cell::get);
        card.switches.three_d = true;
        card.transform.anchor_x = Property::fixed(200.0);
        card.transform.anchor_y = Property::fixed(50.0);
        card.transform.position_y = Property::fixed(180.0);
        card.transform.rotation_x = Property::fixed(90.0);
        let c = doc.comp_mut(comp).unwrap();
        c.layers[0].switches.three_d = true;
        c.layers.push(card);
        (Arc::new(doc), comp)
    };
    let (open, id) = with_card(carded(false));
    let Some(falling) = pixels(&open, id, 45) else {
        return;
    };
    assert!(
        row(&falling, 250, 200, 280),
        "with no card they fall past it"
    );
    let (shut, id) = with_card(carded(true));
    let landed = pixels(&shut, id, 45).unwrap();
    assert!(!row(&landed, 250, 200, 280), "the card at 180 holds them");
    assert!(row(&landed, 170, 200, 280), "and they land on it");
}

/// A frame read from a bake is the frame walked from the in point.
#[test]
fn a_baked_frame_is_the_walked_frame() {
    use lumit_render::track::{progress, request_bake, Progress};
    let (doc, id) = project(|_| {});
    let mut shorter = Document::clone(&doc);
    let layer = &mut shorter.comp_mut(id).unwrap().layers[0];
    layer.out_point = CompTime(Rational::new(2, 1).unwrap());
    let (layer, effect) = (layer.id, layer.effects[0].id);
    let doc = Arc::new(shorter);
    if HeadlessRenderer::shared().is_err() {
        return;
    }
    let asked = request_bake(Arc::clone(&doc), id, layer, effect);
    assert!(matches!(asked, lumit_render::track::Requested::Started));
    loop {
        match progress(effect) {
            Some(Progress::Done) => break,
            Some(Progress::Failed(why)) => panic!("the bake failed: {why:?}"),
            _ => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    assert_eq!(
        lumit_render::sim::bake::status(&doc, id, layer, effect),
        Some((57, 60))
    );
    let baked = walked_alone(&doc, id, 44).unwrap();
    assert!(baked.iter().step_by(4).any(|v| *v > 40), "particles drew");
    let dir = lumit_project::cache_dir().unwrap().join("sim");
    std::fs::remove_dir_all(dir.join(effect.to_string())).unwrap();
    assert!(lumit_render::sim::bake::status(&doc, id, layer, effect).is_none());
    assert!(walked_alone(&doc, id, 44).unwrap() == baked, "walked again");
}

/// Sparks are thrown from what dies on a floor and from what bounces off it, out
/// to the sides where nothing else lands, and each frame they are the same.
#[test]
fn sparks_fly_from_where_particles_hit() {
    let sides = |pixels: &[u8]| {
        (150..195).any(|y: usize| {
            (0..W as usize)
                .filter(|x| x.abs_diff(240) > 60)
                .any(|x| pixels[(y * W as usize + x) * 4] > 30)
        })
    };
    let run = |from: Option<(u32, u32)>| {
        project(|e| {
            set(e, "ground", EffectValue::Bool(true));
            float(e, "ground_height", -60.0);
            float(e, "turbulence_amount", 0.0);
            float(e, "initial_speed", 0.0);
            if let Some((from, contact)) = from {
                float(e, "sparks", 4.0);
                set(e, "sparks_from", EffectValue::Choice(from));
                set(e, "on_contact", EffectValue::Choice(contact));
                float(e, "spark_speed", 300.0);
                float(e, "spark_size", 100.0);
            }
        })
    };
    let (none, id) = run(None);
    let Some(plain) = pixels(&none, id, 45) else {
        return;
    };
    assert!(!sides(&plain), "nothing out there without sparks");
    for (from, contact) in [(0, 3), (1, 0)] {
        let (doc, id) = run(Some((from, contact)));
        assert!(sides(&pixels(&doc, id, 45).unwrap()), "from {from}");
    }
}

/// A frame nobody wants any more stops walking, and the walk carries on from
/// where it stopped.
#[test]
fn a_stop_reaches_a_long_walk() {
    use lumit_eval::epoch::Epoch;
    use lumit_render::cancel::{drawn_short, Scope};
    let (doc, id) = project(|e| float(e, "max_particles", 200_000.0));
    if pixels(&doc, id, 1).is_none() {
        return;
    }
    let epoch = Epoch::new();
    let scope = Scope::new(epoch.token());
    let stopper = {
        let epoch = epoch.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            epoch.bump();
        })
    };
    let _ = drawn_short();
    let got = pixels(&doc, id, 890);
    assert!(got.is_none() || drawn_short(), "the walk heard the stop");
    stopper.join().unwrap();
    drop(scope);
    let whole = pixels(&doc, id, 890).unwrap();
    assert!(!drawn_short());
    assert!(whole.iter().step_by(4).any(|v| *v > 40), "particles drew");
}

thread_local! {
    static CARD: std::cell::Cell<Uuid> = const { std::cell::Cell::new(Uuid::nil()) };
}

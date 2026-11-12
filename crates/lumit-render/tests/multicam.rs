//! A multicam clip draws its own angle of its comp and no other, and two
//! angles of one comp at the same moment are two pictures with two names.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
    TransformGroup,
};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;

fn s(n: i64) -> Rational {
    Rational::new(n, 1).unwrap()
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(s(10)),
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
        grade_versions: None,
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

fn comp_of(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
        width: COMP,
        height: COMP,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(s(10)),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

/// A multicam comp of two angles, red and blue, each one solid clip across
/// the whole comp. Its id and the two angles' ids.
fn multicam(doc: &mut Document) -> (Uuid, Uuid, Uuid) {
    let angle = |doc: &mut Document, name: &str, colour: [f32; 4]| {
        let def = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: name.into(),
            colour: LinearColour(colour),
            width: COMP,
            height: COMP,
            extra: serde_json::Map::new(),
        }));
        let clip = Clip::new(ClipSource::Solid(def), s(0), s(10), s(0), s(10));
        layer(name, LayerKind::Sequence { clips: vec![clip] })
    };
    let red = angle(doc, "Red", [1.0, 0.0, 0.0, 1.0]);
    let blue = angle(doc, "Blue", [0.0, 0.0, 1.0, 1.0]);
    let (red_id, blue_id) = (red.id, blue.id);
    let mut mc = comp_of("Multicam", vec![red, blue]);
    mc.set_multicam(Some(&lumit_core::multicam::Multicam::default()));
    let id = mc.id;
    doc.items.push(ProjectItem::Composition(mc));
    (id, red_id, blue_id)
}

/// A comp of one Sequence row per angle given, each with one clip of the
/// multicam comp from 0 to 2 s showing that angle, at that opacity.
fn cut(doc: &mut Document, mc: Uuid, rows: &[(Uuid, f64)]) -> Uuid {
    let layers = rows
        .iter()
        .map(|(angle, opacity)| {
            let mut clip = Clip::new(ClipSource::Comp(mc), s(0), s(2), s(0), s(2));
            clip.angle = Some(*angle);
            clip.opacity = *opacity;
            layer("row", LayerKind::Sequence { clips: vec![clip] })
        })
        .collect();
    let comp = comp_of("Cut", layers);
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    id
}

#[test]
fn a_multicam_clip_draws_its_own_angle() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut doc = Document::new();
    let (mc, red, blue) = multicam(&mut doc);
    let reds = cut(&mut doc, mc, &[(red, 1.0)]);
    let blues = cut(&mut doc, mc, &[(blue, 1.0)]);
    // Red at half over blue, the same moment of the same comp in one frame.
    let both = cut(&mut doc, mc, &[(red, 0.5), (blue, 1.0)]);
    let doc = Arc::new(doc);
    let mut centre = |comp: Uuid| {
        let (px, w, _) = r.render_rgba(&doc, comp, 30, 1.0).unwrap();
        let d = (((COMP / 2) * w + COMP / 2) * 4) as usize;
        [px[d], px[d + 1], px[d + 2]]
    };
    let red_px = centre(reds);
    assert!(
        red_px[0] > 200 && red_px[2] < 30,
        "angle one is red: {red_px:?}"
    );
    let blue_px = centre(blues);
    assert!(
        blue_px[2] > 200 && blue_px[0] < 30,
        "angle two is blue: {blue_px:?}"
    );
    // Had the two angles shared one picture, this would be all one colour.
    let mixed = centre(both);
    assert!(
        mixed[0] > 100 && mixed[2] > 100,
        "both angles show at one moment: {mixed:?}"
    );
    // And an angle picked again draws its own colour, not the last one drawn.
    assert_eq!(centre(reds), red_px);
}

//! A clip's own effects and blend mode on a Sequence layer that draws a
//! picture: they reach that clip's part of the row and no other, and they
//! name the frame.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::model::{
    BlendMode, Composition, Document, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

const COMP: u32 = 64;

fn solid(def: Uuid, name: &str, colour: [f32; 4], w: u32, h: u32) -> ProjectItem {
    ProjectItem::Solid(SolidDef {
        id: def,
        name: name.into(),
        colour: LinearColour(colour),
        width: w,
        height: h,
        extra: serde_json::Map::new(),
    })
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
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

fn comp_of(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
        width: COMP,
        height: COMP,
        frame_rate: FrameRate::new(60, 1).unwrap(),
        duration: Duration(Rational::new(10, 1).unwrap()),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn rgb(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let d = ((y * w + x) * 4) as usize;
    [rgba[d], rgba[d + 1], rgba[d + 2]]
}

struct Stub;
impl lumit_eval::SourceStamper for Stub {
    fn stamp(&self, item: Uuid, lt: f64, _native: bool) -> Option<(String, u64)> {
        Some((format!("stub:{item}"), (lt * 60.0).round().max(0.0) as u64))
    }
}

fn key_of(doc: &Arc<Document>, comp: Uuid, t: f64) -> lumit_eval::FrameKey {
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.id == comp => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    lumit_eval::comp_frame_key(doc, &comp, t, lumit_eval::Quality::default(), &Stub)
        .expect("a solid comp is always keyable")
}

/// A red comp cut into two one-second clips on a Sequence layer, over a grey
/// plate. `dress` sets up the first clip.
fn two_clips(dress: impl Fn(&mut Clip)) -> (Arc<Document>, Uuid) {
    let mut doc = Document::new();
    let (red, grey) = (Uuid::now_v7(), Uuid::now_v7());
    doc.items
        .push(solid(red, "red", [1.0, 0.0, 0.0, 1.0], COMP, COMP));
    doc.items
        .push(solid(grey, "grey", [0.5, 0.5, 0.5, 1.0], COMP, COMP));
    let inner = comp_of("Inner", vec![layer("red", LayerKind::Solid { def: red })]);
    let inner_id = inner.id;
    doc.items.push(ProjectItem::Composition(inner));
    let second = |n: i64| Rational::new(n, 1).unwrap();
    let clip = |at: i64| {
        Clip::new(
            ClipSource::Comp(inner_id),
            Rational::ZERO,
            second(1),
            second(at),
            second(1),
        )
    };
    let mut first = clip(0);
    dress(&mut first);
    let row = layer(
        "row",
        LayerKind::Sequence {
            clips: vec![first, clip(1)],
        },
    );
    let comp = comp_of(
        "Comp",
        vec![row, layer("plate", LayerKind::Solid { def: grey })],
    );
    let id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    (Arc::new(doc), id)
}

#[test]
fn a_clips_effects_and_blend_reach_that_clip_alone() {
    let Ok(mut r) = lumit_render::headless::HeadlessRenderer::shared() else {
        lumit_gpu::no_adapter();
        return;
    };
    let mut centre = |doc: &Arc<Document>, comp: Uuid, frame: u64| {
        let (px, w, _) = r.render_rgba(doc, comp, frame, 1.0).unwrap();
        rgb(&px, w, COMP / 2, COMP / 2)
    };

    let (plain, plain_comp) = two_clips(|_| {});
    let red = centre(&plain, plain_comp, 30);
    assert!(red[0] > 200 && red[1] < 30, "the clip is red: {red:?}");

    // Invert on the first clip: cyan while it shows, red once the second does.
    let (inverted, comp) = two_clips(|c| {
        c.effects
            .push(lumit_core::fx::instantiate("invert").unwrap());
    });
    let cyan = centre(&inverted, comp, 30);
    assert!(
        cyan[0] < 30 && cyan[1] > 200,
        "the first clip is inverted: {cyan:?}"
    );
    assert_eq!(centre(&inverted, comp, 90), red, "and the second is not");
    assert_ne!(
        key_of(&inverted, comp, 0.5),
        key_of(&plain, plain_comp, 0.5),
        "a clip's effect names the frame it is on"
    );
    assert_eq!(
        key_of(&inverted, comp, 1.5),
        key_of(&plain, plain_comp, 1.5),
        "and leaves the frames of the next clip alone"
    );

    // The stack's bypass takes it off again.
    let (bypassed, comp) = two_clips(|c| {
        c.effects
            .push(lumit_core::fx::instantiate("invert").unwrap());
        c.fx = false;
    });
    assert_eq!(centre(&bypassed, comp, 30), red);

    // Multiply on the first clip darkens it against the grey plate.
    let (multiplied, comp) = two_clips(|c| c.blend = Some(BlendMode::Multiply));
    let dark = centre(&multiplied, comp, 30);
    assert!(dark[0] < red[0] - 20, "red times grey is darker: {dark:?}");
    assert_eq!(centre(&multiplied, comp, 90), red);
}

/// In a dissolve a clip's effects leave with the clip: its stack is worn all
/// the way through, at less and less of its Mix, so nothing changes at once.
#[test]
fn a_clips_effects_fade_with_it_through_a_dissolve() {
    let second = |n: i64| Rational::new(n, 1).unwrap();
    let clip = |at: i64, length: i64| {
        Clip::new(
            ClipSource::Comp(Uuid::now_v7()),
            Rational::ZERO,
            second(length),
            second(at),
            second(length),
        )
    };
    // Two seconds each, the second laid over the last second of the first.
    let mut first = clip(0, 2);
    first
        .effects
        .push(lumit_core::fx::instantiate("invert").unwrap());
    let row = layer(
        "row",
        LayerKind::Sequence {
            clips: vec![first, clip(1, 2)],
        },
    );
    let mix = |lt: f64| {
        lumit_core::sequence::worn_at(&row, lt)
            .and_then(|worn| worn.effects.first()?.float_at("mix", lt))
    };
    assert_eq!(mix(0.5), Some(100.0), "whole before the dissolve");
    let (early, late) = (mix(1.25).unwrap(), mix(1.75).unwrap());
    assert!(early < 100.0 && late > 0.0, "{early} then {late}");
    assert!(early > late, "less of it as the clip leaves");
    assert_eq!(mix(2.5), None, "and gone with the clip");
}

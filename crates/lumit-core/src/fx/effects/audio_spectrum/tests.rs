#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use super::draw::{laid_out, render};
use super::sound::reading;
use super::*;
use crate::anim::Property;
use crate::expression::ExpressionContext;
use crate::fx::markers::MarkerContext;
use crate::fx::registry::AudioTap;
use crate::model::EffectValue;

/// A steady tone at one pitch, full scale, with a chosen sign of life.
struct Tone(f64);

impl AudioTap for Tone {
    fn samples(&self, _: uuid::Uuid, _: f64, _: f64, _: &mut Vec<f32>) -> Option<f64> {
        None
    }

    fn strip(
        &self,
        _: Option<uuid::Uuid>,
        _: Option<uuid::Uuid>,
        half: f64,
        out: &mut Vec<f32>,
    ) -> Option<f64> {
        let rate = 48_000.0;
        let n = (half * 2.0 * rate) as usize;
        out.extend((0..n).map(|i| (std::f64::consts::TAU * self.0 * i as f64 / rate).sin() as f32));
        Some(rate)
    }
}

fn instance() -> crate::model::EffectInstance {
    crate::fx::builtins::instantiate(MATCH_NAME).unwrap()
}

fn set(e: &mut crate::model::EffectInstance, id: &str, v: f64) {
    for p in &mut e.params {
        if p.id == id {
            p.value = EffectValue::Float(Property::fixed(v));
        }
    }
}

fn context() -> Arc<ExpressionContext> {
    Arc::new(ExpressionContext::detached())
}

/// A 1 kHz tone has to light the band that holds 1 kHz and leave the far ends
/// dark, or the bars draw the wrong picture.
#[test]
fn a_tone_lights_the_band_it_is_in() {
    let mut e = instance();
    set(&mut e, "bands", 32.0);
    let levels = reading(&e, 0.0, 30.0, &context(), Some(&Tone(1000.0)));
    assert_eq!(levels.len(), 32);
    let (loudest, peak) =
        levels.iter().enumerate().fold(
            (0, 0.0f32),
            |held, (i, l)| if *l > held.1 { (i, *l) } else { held },
        );
    // 40 Hz to 16 kHz is about 8.6 octaves; 1 kHz sits 4.6 of them up.
    let expected = (32.0 * (1000.0f64 / 40.0).log2() / (16000.0f64 / 40.0).log2()) as usize;
    assert!(
        loudest.abs_diff(expected) <= 1,
        "band {loudest}, wanted {expected}"
    );
    assert!(
        peak > 0.9,
        "a full-scale tone reads near the top, got {peak}"
    );
    assert!(levels[0] < 0.2 && levels[31] < 0.2, "{levels:?}");
}

/// The Waveform shape reads the wave, signed. No tap is silence, never a fault.
#[test]
fn a_waveform_reads_signed_samples_and_no_sound_is_silence() {
    let mut e = instance();
    for p in &mut e.params {
        if p.id == "shape" {
            p.value = EffectValue::Choice(SHAPE_WAVEFORM);
        }
    }
    let wave = reading(&e, 0.0, 30.0, &context(), Some(&Tone(90.0)));
    assert!(
        wave.iter().any(|s| *s > 0.5) && wave.iter().any(|s| *s < -0.5),
        "{wave:?}"
    );
    let quiet = reading(&e, 0.0, 30.0, &context(), None);
    assert!(quiet.iter().all(|s| *s == 0.0));
}

/// A bar reaches Height at full level and stays put at silence, in the colours
/// the rows give it, which is the picture the rows promise.
#[test]
fn a_bar_at_full_level_reaches_height_in_the_outside_colour() {
    let e = instance();
    let ops = crate::fx::resolved::resolve_stack(
        std::slice::from_ref(&e),
        0.0,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        context(),
    );
    let mut s = AudioSpectrum::read(ops.get(0).unwrap().params);
    s.bands = 2;
    s.thickness = 8.0;
    s.height = 100.0;
    s.from_x = 0.0;
    s.to_x = 200.0;
    s.from_y = 150.0;
    s.to_y = 150.0;
    s.inside_colour = [0.0, 0.0, 1.0, 1.0];
    s.outside_colour = [1.0, 0.0, 0.0, 1.0];
    let (w, h) = (200u32, 200u32);
    let laid = laid_out(&s, 1.0, &[1.0, 0.0], &MaskPolyline::default(), w, h);
    let mut rgba = vec![0.0; (w * h * 4) as usize];
    render(&mut rgba, w, h, &laid);
    let at = |x: u32, y: u32| {
        let i = (y * w + x) as usize * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    // The first bar, at x 50: blue at its base, red at its tip, nothing past it.
    assert!(at(50, 148)[2] > 0.9, "{:?}", at(50, 148));
    assert!(at(50, 53)[0] > 0.9, "{:?}", at(50, 53));
    assert_eq!(at(50, 40)[3], 0.0);
    // The second, at silence, is a dot on the line and no more.
    assert!(at(150, 150)[3] > 0.9);
    assert_eq!(at(150, 120)[3], 0.0);
}

/// Particulate emits from the socket: the stream has one point per band and each
/// sits at its bar's tip, or it has nothing to emit from.
#[test]
fn the_points_socket_hands_on_the_tips() {
    let mut e = instance();
    set(&mut e, "bands", 16.0);
    let id = e.id;
    let graph = crate::graph::LayerGraph::default();
    let mut doc = crate::model::Document::new();
    let comp = crate::model::Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: uuid::Uuid::now_v7(),
        name: "c".into(),
        width: 1920,
        height: 1080,
        frame_rate: crate::time::FrameRate::new(60, 1).unwrap(),
        duration: crate::time::Duration(crate::time::Rational::new(10, 1).unwrap()),
        background: crate::model::LinearColour::BLACK,
        work_area: None,
        layers: Vec::new(),
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(crate::model::ProjectItem::Composition(comp));
    let context = Arc::new(ExpressionContext {
        document: Arc::new(doc),
        comp: Some(comp_id),
        layer: None,
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    });
    let stream = crate::fx::drivers::effect_stream_in(
        &graph,
        id,
        0.0,
        context,
        Some(&Tone(1000.0)),
        crate::fx::points::Projection::FLAT,
        Some(std::slice::from_ref(&e)),
        crate::fx::drivers::NO_CLONE,
    )
    .unwrap();
    assert_eq!(stream.position.len(), 16);
    let highest = stream
        .position
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min);
    assert!(
        highest < 540.0 - 200.0,
        "the tone's bar reaches up, got {highest}"
    );
}

//! The Preset row's looks. Picking one writes its values into the look rows;
//! the row then reads Custom again as soon as one of them is edited, so it
//! never names a look the light no longer has.

use crate::fx::params::Value;
use crate::fx::registry::Pressed;
use crate::model::{EffectInstance, EffectValue};

/// The Preset row's options. Custom is first and does nothing.
pub const NAMES: &[&str] = &[
    "Custom",
    "Blue blade",
    "Red blade",
    "Neon",
    "Ice",
    "Electric",
    "Fire",
    "Energy",
    "Plasma",
    "Ghost",
    "Toxic",
    "Solar",
    "Laser",
];

/// The rows a look sets. Where the line runs, how much of it is lit, the
/// seed and the composite are the user's and are left alone.
const LOOK_ROWS: &[&str] = &[
    "core_size",
    "core_softness",
    "core_colour",
    "halo",
    "start_size",
    "end_size",
    "glow_colour",
    "glow_intensity",
    "glow_size",
    "glow_falloff",
    "flicker",
    "flicker_speed",
    "distortion_type",
    "core_distortion",
    "glow_distortion",
    "distortion_size",
    "distortion_detail",
    "distortion_speed",
    "wind_direction",
    "wind_speed",
];

/// One look: every row in [`LOOK_ROWS`], so picking a look after another
/// leaves nothing of the first behind.
struct Spec {
    core: f32,
    softness: f32,
    core_colour: [f32; 3],
    halo: f32,
    sizes: [f32; 2],
    glow_colour: [f32; 3],
    intensity: f32,
    glow: f32,
    falloff: f32,
    flicker: [f32; 2],
    kind: u32,
    distortion: [f32; 2],
    noise: f32,
    detail: i32,
    speed: f32,
    wind: [f32; 2],
}

const BASE: Spec = Spec {
    core: 10.0,
    softness: 40.0,
    core_colour: [1.0, 1.0, 1.0],
    halo: 60.0,
    sizes: [100.0, 100.0],
    glow_colour: [0.06, 0.28, 1.0],
    intensity: 100.0,
    glow: 60.0,
    falloff: 50.0,
    flicker: [0.0, 12.0],
    kind: 0,
    distortion: [0.0, 0.0],
    noise: 60.0,
    detail: 3,
    speed: 1.0,
    wind: [0.0, 0.0],
};

fn spec(index: u32) -> Option<Spec> {
    Some(match index {
        1 => Spec {
            halo: 90.0,
            core: 9.0,
            sizes: [100.0, 85.0],
            glow_colour: [0.04, 0.22, 1.0],
            intensity: 130.0,
            flicker: [6.0, 24.0],
            ..BASE
        },
        2 => Spec {
            halo: 90.0,
            core: 9.0,
            sizes: [100.0, 85.0],
            core_colour: [1.0, 0.92, 0.9],
            glow_colour: [1.0, 0.02, 0.01],
            intensity: 150.0,
            glow: 70.0,
            flicker: [12.0, 20.0],
            kind: 2,
            distortion: [1.5, 4.0],
            noise: 30.0,
            speed: 6.0,
            ..BASE
        },
        3 => Spec {
            halo: 30.0,
            core: 6.0,
            softness: 70.0,
            core_colour: [1.0, 0.85, 0.95],
            glow_colour: [1.0, 0.04, 0.42],
            intensity: 170.0,
            glow: 36.0,
            falloff: 30.0,
            ..BASE
        },
        4 => Spec {
            halo: 50.0,
            core: 7.0,
            softness: 60.0,
            core_colour: [0.9, 1.0, 1.0],
            glow_colour: [0.05, 0.75, 1.0],
            intensity: 140.0,
            glow: 50.0,
            falloff: 40.0,
            kind: 0,
            distortion: [0.0, 6.0],
            noise: 90.0,
            speed: 0.5,
            ..BASE
        },
        5 => Spec {
            halo: 80.0,
            core: 4.0,
            softness: 30.0,
            glow_colour: [0.3, 0.35, 1.0],
            intensity: 160.0,
            glow: 45.0,
            falloff: 60.0,
            flicker: [40.0, 25.0],
            kind: 2,
            distortion: [6.0, 10.0],
            noise: 25.0,
            detail: 4,
            speed: 10.0,
            ..BASE
        },
        6 => Spec {
            halo: 40.0,
            core: 14.0,
            softness: 90.0,
            core_colour: [1.0, 0.85, 0.5],
            glow_colour: [1.0, 0.22, 0.02],
            intensity: 150.0,
            glow: 80.0,
            falloff: 70.0,
            flicker: [15.0, 10.0],
            kind: 1,
            distortion: [6.0, 40.0],
            noise: 70.0,
            detail: 4,
            speed: 1.5,
            wind: [0.0, 140.0],
            ..BASE
        },
        7 => Spec {
            halo: 60.0,
            core: 8.0,
            core_colour: [0.9, 1.0, 0.95],
            glow_colour: [0.0, 0.85, 0.55],
            intensity: 120.0,
            glow: 90.0,
            falloff: 60.0,
            flicker: [10.0, 8.0],
            distortion: [2.0, 20.0],
            noise: 80.0,
            speed: 2.0,
            ..BASE
        },
        8 => Spec {
            halo: 50.0,
            core: 10.0,
            softness: 70.0,
            core_colour: [1.0, 0.9, 1.0],
            glow_colour: [0.55, 0.08, 1.0],
            intensity: 140.0,
            glow: 80.0,
            falloff: 65.0,
            flicker: [10.0, 14.0],
            kind: 1,
            distortion: [3.0, 25.0],
            noise: 50.0,
            speed: 3.0,
            ..BASE
        },
        9 => Spec {
            halo: 20.0,
            core: 12.0,
            softness: 100.0,
            core_colour: [0.75, 0.88, 1.0],
            glow_colour: [0.45, 0.65, 1.0],
            intensity: 70.0,
            glow: 120.0,
            falloff: 90.0,
            kind: 1,
            distortion: [4.0, 50.0],
            noise: 120.0,
            speed: 0.6,
            wind: [0.0, 40.0],
            ..BASE
        },
        10 => Spec {
            halo: 60.0,
            core: 7.0,
            core_colour: [0.95, 1.0, 0.8],
            glow_colour: [0.3, 1.0, 0.02],
            intensity: 140.0,
            glow: 60.0,
            flicker: [20.0, 18.0],
            kind: 2,
            distortion: [3.0, 12.0],
            noise: 35.0,
            speed: 5.0,
            ..BASE
        },
        11 => Spec {
            halo: 50.0,
            core: 12.0,
            softness: 60.0,
            core_colour: [1.0, 0.95, 0.8],
            glow_colour: [1.0, 0.5, 0.04],
            intensity: 200.0,
            glow: 150.0,
            falloff: 80.0,
            distortion: [0.0, 15.0],
            noise: 150.0,
            speed: 0.4,
            ..BASE
        },
        12 => Spec {
            halo: 100.0,
            core: 3.0,
            softness: 20.0,
            core_colour: [1.0, 0.9, 0.9],
            glow_colour: [1.0, 0.02, 0.05],
            intensity: 220.0,
            glow: 18.0,
            falloff: 15.0,
            ..BASE
        },
        _ => return None,
    })
}

fn rows(s: &Spec) -> [Value; 20] {
    let c = |c: [f32; 3]| Value::Colour([c[0], c[1], c[2], 1.0]);
    [
        Value::Float(s.core),
        Value::Float(s.softness),
        c(s.core_colour),
        Value::Float(s.halo),
        Value::Float(s.sizes[0]),
        Value::Float(s.sizes[1]),
        c(s.glow_colour),
        Value::Float(s.intensity),
        Value::Float(s.glow),
        Value::Float(s.falloff),
        Value::Float(s.flicker[0]),
        Value::Float(s.flicker[1]),
        Value::Choice(s.kind),
        Value::Float(s.distortion[0]),
        Value::Float(s.distortion[1]),
        Value::Float(s.noise),
        Value::Int(s.detail),
        Value::Float(s.speed),
        Value::Float(s.wind[0]),
        Value::Float(s.wind[1]),
    ]
}

fn preset_of(e: &EffectInstance) -> u32 {
    match e.param("preset") {
        Some(EffectValue::Choice(v)) => *v,
        _ => 0,
    }
}

/// A keyed row keeps its keys: a look is a starting point, not an edit of
/// somebody's animation.
fn keyed(e: &EffectInstance, id: &str) -> bool {
    match e.param(id) {
        Some(EffectValue::Float(p)) => p.is_animated(),
        Some(EffectValue::Colour(c)) => c.iter().any(|p| p.is_animated()),
        _ => false,
    }
}

/// What picking a look, or editing one, writes back.
pub fn settle(before: Option<&EffectInstance>, after: &EffectInstance) -> Option<Pressed> {
    let chosen = preset_of(after);
    if before.map(preset_of) != Some(chosen) {
        let s = spec(chosen)?;
        let rows = LOOK_ROWS
            .iter()
            .zip(rows(&s))
            .filter(|(id, _)| !keyed(after, id))
            .map(|(id, v)| (*id, v))
            .collect();
        return Some(Pressed { rows, memory: None });
    }
    let before = before?;
    let edited = LOOK_ROWS
        .iter()
        .any(|id| before.param(id) != after.param(id));
    (chosen != 0 && edited).then(|| Pressed {
        rows: vec![("preset", Value::Choice(0))],
        memory: None,
    })
}

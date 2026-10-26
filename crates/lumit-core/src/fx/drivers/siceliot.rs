//! SICELIOT's drivers: Stagger, Random, Oscillator and Value noise, the value
//! behaviours of Cavalry made into drivers (docs/siceliot/).
//!
//! **In plain terms.** In Cavalry one Random or Stagger wired into a Duplicator
//! gives every copy its own number. Here the same happens through Clone to
//! points: each of these reads which copy the layer is being stamped as, the
//! number Clone index hands out, so a single driver gives every copy its own
//! value. Outside Clone to points a layer is the first copy of one, and the
//! drivers still work, as plain values over time.
//!
//! All four are pure functions of their rows, the copy and the layer's time:
//! the same frame always reads the same number, on every machine.

use crate::fx::{
    cpu, noise, params::CurvePoints, params::Value, points, registry::DriverCx,
    registry::EffectDef, registry::EffectMetadata, registry::Signature, schema::EffectSchema,
    schema::Port, schema::PortType,
};
use lumit_fx_macros::Effect;

/// The match names, which Clone to points also asks for: a layer whose graph
/// holds one of these is rendered once per copy, as with Clone index.
pub const MATCH_NAMES: [&str; 4] = [
    "siceliot_stagger",
    "siceliot_random",
    "siceliot_oscillator",
    "siceliot_noise",
];

/// The port every one of them hands its number out by.
pub const VALUE_PORT: &str = "value";

const VALUE_OUT: &[Port] = &[Port::new(VALUE_PORT, "Value", PortType::Number)];

/// The dice Random rolls, kept off the numbers Clone index and the points
/// effects use.
const RANDOM_ATTR: u32 = 70;

/// Which copy of how many, and the copy as 0 at the first and 1 at the last.
fn copy(cx: &DriverCx<'_>) -> (u32, f32) {
    let (index, count) = cx.clone;
    let last = count.saturating_sub(1).max(1);
    (index, index as f32 / last as f32)
}

fn between(min: f32, max: f32, t: f32) -> f32 {
    min + (max - min) * t
}

// ---------------------------------------------------------------- Stagger

/// Stagger's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "siceliot_stagger",
    label = "Stagger",
    version = 1,
    category = SiceliotDrivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Stagger {
    /// The value the first copy gets.
    #[slider(label = "Minimum", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub minimum: f32,

    /// The value the last copy gets.
    #[slider(label = "Maximum", min = -100.0, max = 100.0, default = 100.0, unit = Raw)]
    pub maximum: f32,

    /// How the values are spread between the first copy and the last.
    #[curve(label = "Curve", default = [[0.0, 0.0], [1.0, 1.0]])]
    pub curve: CurvePoints,

    /// Moves the run along the copies, as a share of them. Keyframe it to
    /// send a wave through the copies.
    #[slider(label = "Offset", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub offset: f32,

    /// Past the last copy, start again from the first instead of holding.
    #[toggle(label = "Loop", default = false)]
    pub looped: bool,

    /// The last copy first.
    #[toggle(label = "Reverse", default = false)]
    pub reverse: bool,
}

/// Stagger's behaviour.
pub struct StaggerDef;

impl EffectDef for StaggerDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Stagger as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: VALUE_OUT,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Stagger::read(cx.params);
        let (_, norm) = copy(cx);
        push(VALUE_PORT, Value::Float(stagger(&p, norm)));
    }
}

/// The value for the copy `norm` of the way from the first to the last.
#[must_use]
pub fn stagger(p: &Stagger, norm: f32) -> f32 {
    let mut t = if p.reverse { 1.0 - norm } else { norm };
    t += p.offset / 100.0;
    t = if p.looped {
        t - t.floor()
    } else {
        t.clamp(0.0, 1.0)
    };
    let shaped = cpu::curve_at(t, &cpu::curve_table(&p.curve));
    between(p.minimum, p.maximum, shaped)
}

// ----------------------------------------------------------------- Random

/// Random's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "siceliot_random",
    label = "Random",
    version = 1,
    category = SiceliotDrivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Random {
    /// The lowest value it can give.
    #[slider(label = "Minimum", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub minimum: f32,

    /// The highest value it can give.
    #[slider(label = "Maximum", min = -100.0, max = 100.0, default = 100.0, unit = Raw)]
    pub maximum: f32,

    /// Leans the values towards Minimum (below 0) or Maximum (above 0).
    #[slider(label = "Bias", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub bias: f32,

    /// New values a second. At 0 each copy keeps one value for good.
    #[slider(
        label = "Rate",
        min = 0.0,
        max = 30.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub rate: f32,

    /// Which values come out.
    #[seed]
    pub seed: u32,
}

/// Random's behaviour.
pub struct RandomDef;

impl EffectDef for RandomDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Random as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: VALUE_OUT,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Random::read(cx.params);
        let (index, _) = copy(cx);
        push(VALUE_PORT, Value::Float(random(&p, index, cx.lt)));
    }
}

/// The value for copy `index` at layer time `lt`.
#[must_use]
pub fn random(p: &Random, index: u32, lt: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let step = if p.rate > 0.0 {
        (lt * f64::from(p.rate)).floor().max(0.0) as u32
    } else {
        0
    };
    let u = points::draw(p.seed, u64::from(index), RANDOM_ATTR.wrapping_add(step));
    // Bias as a power: 0 is even, +100 crowds towards Maximum, -100 towards
    // Minimum, each by a factor of four.
    let shaped = u.powf(2f32.powf(-p.bias / 50.0));
    between(p.minimum, p.maximum, shaped)
}

// ------------------------------------------------------------- Oscillator

/// The wave shapes Oscillator offers.
pub const OSCILLATOR_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Sawtooth"];

/// Oscillator's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "siceliot_oscillator",
    label = "Oscillator",
    version = 1,
    category = SiceliotDrivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Oscillator {
    /// The wave's shape.
    #[choice(label = "Shape", options = OSCILLATOR_SHAPES, default = 0)]
    pub shape: u32,

    /// The bottom of the wave.
    #[slider(label = "Minimum", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub minimum: f32,

    /// The top of the wave.
    #[slider(label = "Maximum", min = -100.0, max = 100.0, default = 100.0, unit = Raw)]
    pub maximum: f32,

    /// Waves a second.
    #[slider(
        label = "Frequency",
        min = 0.0,
        max = 20.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub frequency: f32,

    /// Where in the wave it starts. Keyframe it from 0 to 360 for an exact
    /// loop.
    #[dial(label = "Phase", default = 0.0)]
    pub phase: f32,

    /// How much further along the wave each copy is than the one before, so
    /// the copies ripple. Read when Phase from is Copy.
    #[dial(label = "Phase per copy", default = 0.0)]
    pub phase_per_copy: f32,

    /// What sets each copy's place on the wave: its number (Copy), or where
    /// it stands (Position), so the wave crosses the points as a texture
    /// whatever order they come in.
    #[choice(label = "Phase from", options = PHASE_FROM, default = 0)]
    pub phase_from: u32,

    /// The direction the wave travels across the points, when Phase from is
    /// Position. 0° runs left to right.
    #[dial(label = "Wave angle", default = 0.0)]
    pub wave_angle: f32,

    /// The distance from one crest to the next, when Phase from is
    /// Position.
    #[slider(
        label = "Wavelength",
        min = 10.0,
        max = 4000.0,
        default = 400.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub wavelength: f32,
}

/// What sets an Oscillator copy's place on the wave.
pub const PHASE_FROM: &[&str] = &["Copy", "Position"];

/// Oscillator's behaviour.
pub struct OscillatorDef;

impl EffectDef for OscillatorDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Oscillator as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: VALUE_OUT,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Oscillator::read(cx.params);
        let (index, _) = copy(cx);
        push(
            VALUE_PORT,
            Value::Float(oscillator(&p, index, cx.point, cx.lt)),
        );
    }
}

/// The wave in -1..=1 at `x` waves along.
fn wave(shape: u32, x: f64) -> f32 {
    let f = (x - x.floor()) as f32;
    match shape {
        1 => 4.0 * (f - 0.25 - (f - 0.25).floor() - 0.5).abs() - 1.0,
        2 => {
            if f < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        3 => 2.0 * f - 1.0,
        _ => (std::f32::consts::TAU * f).sin(),
    }
}

/// The value for copy `index`, standing at `point` (px@comp, when known), at
/// layer time `lt`.
#[must_use]
pub fn oscillator(p: &Oscillator, index: u32, point: Option<[f32; 2]>, lt: f64) -> f32 {
    let offset = if p.phase_from == 1 {
        // How many wavelengths along the wave's direction the point stands,
        // taken away so the wave travels forward along its angle: a point
        // further along reads what a point behind it read a moment ago.
        let [x, y] = point.unwrap_or([0.0; 2]);
        let a = p.wave_angle.to_radians();
        -f64::from((x * a.cos() + y * a.sin()) / p.wavelength.max(1.0))
    } else {
        f64::from(index) * f64::from(p.phase_per_copy) / 360.0
    };
    let turns = lt * f64::from(p.frequency) + f64::from(p.phase) / 360.0 + offset;
    between(p.minimum, p.maximum, 0.5 * (wave(p.shape, turns) + 1.0))
}

// ------------------------------------------------------------ Value noise

/// Value noise's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "siceliot_noise",
    label = "Value noise",
    version = 1,
    category = SiceliotDrivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct ValueNoise {
    /// The lowest it wanders to.
    #[slider(label = "Minimum", min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub minimum: f32,

    /// The highest it wanders to.
    #[slider(label = "Maximum", min = -100.0, max = 100.0, default = 100.0, unit = Raw)]
    pub maximum: f32,

    /// How quickly it wanders: turns a second.
    #[slider(
        label = "Frequency",
        min = 0.0,
        max = 20.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub frequency: f32,

    /// Layers of finer wander on top, each half as strong.
    #[counter(
        label = "Detail",
        min = 1,
        max = 4,
        default = 1,
        hard_min = 1,
        hard_max = 4,
        unit = Raw
    )]
    pub detail: i32,

    /// What makes the copies wander differently: nothing (they move
    /// together), their number (each on its own path), or where they stand
    /// (a texture of noise that drifts across the points).
    #[choice(label = "Vary by", options = VARY_BY, default = 1)]
    pub vary_by: u32,

    /// How big the noise's features are across the points, when Vary by is
    /// Position.
    #[slider(
        label = "Noise scale",
        min = 10.0,
        max = 4000.0,
        default = 300.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub noise_scale: f32,

    /// Which path it wanders.
    #[seed]
    pub seed: u32,
}

/// What makes Value noise differ from copy to copy.
pub const VARY_BY: &[&str] = &["Nothing", "Copy", "Position"];

/// Value noise's behaviour.
pub struct ValueNoiseDef;

impl EffectDef for ValueNoiseDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ValueNoise as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: VALUE_OUT,
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = ValueNoise::read(cx.params);
        let (index, _) = copy(cx);
        push(
            VALUE_PORT,
            Value::Float(value_noise(&p, index, cx.point, cx.lt)),
        );
    }
}

/// The value for copy `index`, standing at `point` (px@comp, when known), at
/// layer time `lt`.
#[must_use]
pub fn value_noise(p: &ValueNoise, index: u32, point: Option<[f32; 2]>, lt: f64) -> f32 {
    let time = (lt * f64::from(p.frequency)) as f32;
    // Where in the noise field this copy reads: along time, on a row of its
    // own (Copy), or at its place in a field that drifts with time
    // (Position).
    #[allow(clippy::cast_precision_loss)]
    let at = match p.vary_by {
        1 => [time, index as f32 * 7.31, 0.0],
        2 => {
            let [x, y] = point.unwrap_or([0.0; 2]);
            let s = p.noise_scale.max(1.0);
            [x / s, y / s, time]
        }
        _ => [time, 0.0, 0.0],
    };
    let (mut sum, mut weight, mut scale) = (0.0f32, 0.0f32, 1.0f32);
    for octave in 0..p.detail.clamp(1, 4) {
        #[allow(clippy::cast_sign_loss)]
        let channel = octave as u32;
        sum += scale * noise::value3(p.seed, channel, at[0] / scale, at[1] / scale, at[2], 0);
        weight += scale;
        scale *= 0.5;
    }
    between(p.minimum, p.maximum, 0.5 * (sum / weight + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stagger_with(curve: [[f32; 2]; 2]) -> Stagger {
        Stagger {
            minimum: 10.0,
            maximum: 50.0,
            curve: CurvePoints::sanitised(&curve),
            offset: 0.0,
            looped: false,
            reverse: false,
        }
    }

    /// The four values are what the docs promise: Stagger runs Minimum to
    /// Maximum across the copies, Random stays inside its range and keeps a
    /// copy's value until Rate moves it, Oscillator ripples by Phase per copy,
    /// Value noise stays in range and differs per copy. Guards wrong values
    /// in every project that wires one.
    #[test]
    fn siceliot_drivers_give_the_documented_values() {
        let s = stagger_with([[0.0, 0.0], [1.0, 1.0]]);
        assert!((stagger(&s, 0.0) - 10.0).abs() < 1e-3);
        assert!((stagger(&s, 0.5) - 30.0).abs() < 0.5);
        assert!((stagger(&s, 1.0) - 50.0).abs() < 1e-3);
        let mut r = s;
        r.reverse = true;
        assert!((stagger(&r, 0.0) - 50.0).abs() < 1e-3);
        let mut l = s;
        l.looped = true;
        l.offset = 50.0;
        assert!(
            (stagger(&l, 0.75) - 20.0).abs() < 0.5,
            "0.75 + 0.5 loops to 0.25"
        );

        let rnd = Random {
            minimum: -5.0,
            maximum: 5.0,
            bias: 0.0,
            rate: 0.0,
            seed: 7,
        };
        let values: Vec<f32> = (0..64).map(|i| random(&rnd, i, 0.0)).collect();
        assert!(values.iter().all(|v| (-5.0..=5.0).contains(v)));
        assert!(values.windows(2).any(|w| w[0] != w[1]), "copies differ");
        assert_eq!(random(&rnd, 3, 0.0), random(&rnd, 3, 9.0), "Rate 0 holds");
        let moving = Random { rate: 2.0, ..rnd };
        assert_eq!(random(&moving, 3, 0.1), random(&moving, 3, 0.4));
        assert_ne!(random(&moving, 3, 0.1), random(&moving, 3, 0.6));

        let osc = Oscillator {
            shape: 0,
            minimum: 0.0,
            maximum: 10.0,
            frequency: 1.0,
            phase: 0.0,
            phase_per_copy: 90.0,
            phase_from: 0,
            wave_angle: 0.0,
            wavelength: 400.0,
        };
        assert!(
            (oscillator(&osc, 0, None, 0.0) - 5.0).abs() < 1e-3,
            "sine starts mid-way"
        );
        assert!(
            (oscillator(&osc, 1, None, 0.0) - 10.0).abs() < 1e-3,
            "a quarter turn on"
        );
        assert!((oscillator(&osc, 0, None, 0.25) - 10.0).abs() < 1e-3);
        let tri = Oscillator {
            shape: 1,
            phase_per_copy: 0.0,
            ..osc
        };
        assert!((oscillator(&tri, 0, None, 0.0) - 5.0).abs() < 1e-3);
        assert!((oscillator(&tri, 0, None, 0.25) - 10.0).abs() < 1e-3);
        assert!((oscillator(&tri, 0, None, 0.75) - 0.0).abs() < 1e-3);

        let n = ValueNoise {
            minimum: 0.0,
            maximum: 1.0,
            frequency: 3.0,
            detail: 3,
            vary_by: 1,
            noise_scale: 300.0,
            seed: 11,
        };
        for i in 0..8 {
            for f in 0..48 {
                let v = value_noise(&n, i, None, f64::from(f) / 24.0);
                assert!((0.0..=1.0).contains(&v), "{v} out of range");
            }
        }
        assert_ne!(
            value_noise(&n, 0, None, 0.37),
            value_noise(&n, 1, None, 0.37),
            "copies wander apart"
        );
        let together = ValueNoise { vary_by: 0, ..n };
        assert_eq!(
            value_noise(&together, 0, None, 0.37),
            value_noise(&together, 5, None, 0.37)
        );

        // Position: a point a quarter wavelength along the wave's angle is a
        // quarter turn behind, whatever its number; at 90° the wave runs
        // down, so moving across changes nothing.
        let spatial = Oscillator {
            phase_from: 1,
            phase_per_copy: 0.0,
            wavelength: 400.0,
            ..osc
        };
        let at =
            |x: f32, y: f32, index: u32, lt: f64| oscillator(&spatial, index, Some([x, y]), lt);
        assert!((at(0.0, 0.0, 7, 0.0) - 5.0).abs() < 1e-3);
        assert!(
            (at(100.0, 0.0, 0, 0.0) - 0.0).abs() < 1e-3,
            "a quarter behind"
        );
        assert!(
            (at(100.0, 0.0, 0, 0.25) - 5.0).abs() < 1e-3,
            "the crest arrives"
        );
        assert_eq!(
            at(100.0, 0.0, 0, 0.0),
            at(100.0, 0.0, 9, 0.0),
            "the number is ignored"
        );
        let down = Oscillator {
            wave_angle: 90.0,
            ..spatial
        };
        assert!(
            (oscillator(&down, 0, Some([300.0, 0.0]), 0.0) - 5.0).abs() < 1e-3,
            "across a downward wave nothing changes"
        );

        // Position noise: neighbours a fraction of Noise scale apart read
        // nearly the same, points far apart read differently.
        let field = ValueNoise {
            vary_by: 2,
            detail: 1,
            ..n
        };
        let near = (value_noise(&field, 0, Some([0.0, 0.0]), 0.0)
            - value_noise(&field, 1, Some([3.0, 0.0]), 0.0))
        .abs();
        let far = (0..8)
            .map(|k| {
                (value_noise(&field, 0, Some([0.0, 0.0]), 0.0)
                    - value_noise(&field, 0, Some([k as f32 * 450.0 + 450.0, 0.0]), 0.0))
                .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(near < 0.05, "neighbours differ by {near}");
        assert!(far > 0.1, "far points differ by at most {far}");
    }
}

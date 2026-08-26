//! Remap (K-471 §1.3): one range of numbers into another.
//!
//! **In plain terms.** A loudness that arrives as 0 to 1 and a blur radius that
//! wants 0 to 40 do not speak the same language. Remap is the translator: say
//! what range comes in and what range should come out, and it maps one onto the
//! other in a straight line. It is the piece that makes every other driver
//! usable on every parameter.
//!
//! An input range of zero width has no line through it, so it answers the
//! output's low end rather than dividing by nought.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Remap's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "remap",
    label = "Remap",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Remap {
    /// The number coming in — usually wired from another driver.
    #[slider(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub value: f32,

    /// The bottom of the range it arrives in.
    #[slider(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub in_low: f32,

    /// The top of the range it arrives in.
    #[slider(min = 0.0, max = 1.0, default = 1.0, unit = Raw)]
    pub in_high: f32,

    /// The bottom of the range it leaves in.
    #[slider(min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub out_low: f32,

    /// The top of the range it leaves in.
    #[slider(min = 0.0, max = 100.0, default = 100.0, unit = Raw)]
    pub out_high: f32,

    /// Hold the result inside the output range instead of letting it run past
    /// the ends. On by default: a driver is usually feeding something with a
    /// range of its own.
    #[toggle(label = "Clamp", default = true)]
    pub clamp: bool,
}

/// The port the mapped number leaves by.
pub const VALUE_PORT: &str = "value";

/// Remap's behaviour.
pub struct RemapDef;

impl EffectDef for RemapDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Remap as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: &[Port {
                id: VALUE_PORT,
                label: "Value",
                ty: PortType::Number,
                three_d: false,
            }],
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Remap::read(cx.params);
        push(
            VALUE_PORT,
            Value::Float(map(
                p.value, p.in_low, p.in_high, p.out_low, p.out_high, p.clamp,
            )),
        );
    }
}

/// The straight line itself. A zero-width input range answers `out_low`.
#[must_use]
pub fn map(v: f32, in_low: f32, in_high: f32, out_low: f32, out_high: f32, clamp: bool) -> f32 {
    let span = in_high - in_low;
    let t = if span == 0.0 {
        0.0
    } else {
        (v - in_low) / span
    };
    let out = out_low + t * (out_high - out_low);
    if !out.is_finite() {
        return out_low;
    }
    if clamp {
        // The ends in either order: an inverting map (100 down to 0) is an
        // ordinary thing to ask for, and clamping it to `out_low..out_high`
        // would pin every value to one end.
        out.clamp(out_low.min(out_high), out_low.max(out_high))
    } else {
        out
    }
}

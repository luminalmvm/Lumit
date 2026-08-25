//! Smooth (K-471 §1.3): takes the jitter off whatever feeds it.
//!
//! **In plain terms.** An audio level jumps about frame to frame, and a scale
//! driven straight from one stutters. Smooth is the answer: it averages its
//! input over a short stretch of time centred on the frame, so the number still
//! follows the music but stops twitching. Time is how long that stretch is —
//! longer is calmer and lazier to react.
//!
//! **It is a temporal driver**, and says so ([`EffectDef::driver_window`]).
//! Everything else in the Drivers family answers from the frame it is on;
//! this one reads its input at a spread of nearby times, so the frame key has
//! to fold that range in or a cached frame could outlive the values it was
//! averaged from (K-471 §2.3).
//!
//! Centred, not trailing: a smoothed ramp comes out as the same ramp rather
//! than as the ramp running late, which is what makes Smooth safe to drop into
//! a chain that was already timed.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Smooth's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "smooth",
    label = "Smooth",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Smooth {
    /// The number coming in. Read at a spread of times around the frame when a
    /// wire feeds it; a value typed here has nothing to smooth and comes
    /// straight out.
    #[slider(min = 0.0, max = 100.0, default = 0.0, unit = Raw)]
    pub value: f32,

    /// How long a stretch to average over, in seconds. Nought is no smoothing
    /// at all.
    #[slider(
        min = 0.0,
        max = 2.0,
        default = 0.2,
        hard_min = 0.0,
        hard_max = 10.0,
        unit = Seconds
    )]
    pub time: f32,
}

/// The port the smoothed number leaves by, and the input port it reads.
pub const VALUE_PORT: &str = "value";

/// How many times the input is read across the window.
///
/// ponytail: a fixed nine-tap box average, and a box is a blunt filter. Nine
/// taps is smooth enough for a driver and cheap enough to run per frame; if a
/// visible stair ever shows up, weight the taps rather than adding more.
const TAPS: u32 = 9;

/// Smooth's behaviour.
pub struct SmoothDef;

impl EffectDef for SmoothDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Smooth as EffectMetadata>::SCHEMA
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
            }],
        }
    }

    fn driver_window(&self, p: crate::fx::Params<'_>) -> f64 {
        // Centred, so it reaches half the window either side of the frame.
        f64::from(Smooth::read(p).time.max(0.0)) / 2.0
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Smooth::read(cx.params);
        let half = f64::from(p.time.max(0.0)) / 2.0;
        // Nothing wired, or no window: the value is already the answer. The
        // unwired case matters — the taps would all read the same typed number
        // and cost eight evaluations to prove it.
        if half <= 0.0 {
            push(VALUE_PORT, Value::Float(p.value));
            return;
        }
        let mut sum = 0.0f64;
        let mut n = 0u32;
        for i in 0..TAPS {
            let f = f64::from(i) / f64::from(TAPS - 1); // 0..=1
            let t = cx.lt - half + f * 2.0 * half;
            match (cx.sample_input)(VALUE_PORT, t) {
                Some(v) => {
                    sum += f64::from(v.as_f32());
                    n += 1;
                }
                // The port is unwired, or the evaluation budget is spent. Either
                // way there is nothing to average and the typed value stands.
                None => {
                    push(VALUE_PORT, Value::Float(p.value));
                    return;
                }
            }
        }
        push(VALUE_PORT, Value::Float((sum / f64::from(n)) as f32));
    }
}

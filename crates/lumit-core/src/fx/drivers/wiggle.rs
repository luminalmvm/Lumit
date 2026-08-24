//! Wiggle (K-471 §1.3): a number that wobbles.
//!
//! **In plain terms.** The oldest trick in motion graphics: make something
//! drift about instead of sitting perfectly still, so it reads as alive rather
//! than as a computer's idea of a shot. Amount is how far it strays; Frequency
//! is how often. Wire its Value into a position, a rotation, a blur radius —
//! anything that takes a number.
//!
//! **The wobble is the same every time.** The path is seeded from the node's own
//! id and read at the layer's time, so it never depends on the wall clock, on
//! which frame rendered first, or on which machine is rendering. Two Wiggles on
//! one layer wobble differently (different ids); the same Wiggle wobbles
//! identically in the preview and in the export, which is K-031's promise.

use crate::fx::{
    noise, DriverCx, EffectDef, EffectMetadata, EffectSchema, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Wiggle's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "wiggle",
    label = "Wiggle",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    // A driver makes a value, not a picture, so there is nothing for a matte to
    // gate (K-395's `None`, as the Controls family declares).
    matte = false,
)]
pub struct Wiggle {
    /// How far the value strays either side of nought. Unbounded on purpose:
    /// whatever this drives decides what its numbers mean, exactly as a Slider
    /// control's do.
    #[slider(min = 0.0, max = 100.0, default = 10.0, unit = Raw)]
    pub amount: f32,

    /// Wobbles per second.
    #[slider(min = 0.0, max = 20.0, default = 2.0, hard_min = 0.0, unit = Raw)]
    pub frequency: f32,
}

/// The port Wiggle's number leaves by.
pub const VALUE_PORT: &str = "value";

/// Wiggle's behaviour.
pub struct WiggleDef;

impl EffectDef for WiggleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Wiggle as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            outputs: &[(VALUE_PORT, PortType::Number)],
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Wiggle::read(cx.params);
        push(VALUE_PORT, Value::Float(p.amount * wobble(cx, p.frequency)));
    }
}

/// The wobble itself, in −1..=1: one octave of the shared seeded value noise
/// (docs/08 §3.37), walked along its x axis at `frequency` cells per second.
///
/// One octave rather than a fractal sum, because a driver's job is a smooth
/// drift and the extra octaves only add a jitter the user cannot dial out. The
/// recipe is pinned here and by
/// `wiggle_is_the_same_wobble_every_time` — nothing else may change it without
/// changing every project that uses it.
fn wobble(cx: &DriverCx<'_>, frequency: f32) -> f32 {
    noise::value3(
        seed_of(cx.node),
        0,
        (cx.lt * frequency as f64) as f32,
        0.0,
        0.0,
        0,
    )
}

/// A node's id folded to a noise seed. Any stable fold would do; this one is
/// the two halves of the uuid exclusive-ored together, which is cheap and
/// spreads two ids that differ in one byte.
fn seed_of(node: uuid::Uuid) -> u32 {
    let n = node.as_u128();
    (n as u32) ^ ((n >> 32) as u32) ^ ((n >> 64) as u32) ^ ((n >> 96) as u32)
}

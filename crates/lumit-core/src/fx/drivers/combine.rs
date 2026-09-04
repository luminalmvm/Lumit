//! Combine (node-graph.md §1.3): four numbers put back together as a colour.
//!
//! **In plain terms.** The other half of [`split`](super::split). Wire a number
//! into Red, another into Green, another into Blue — an audio level, a wiggle,
//! a remapped distance — and out comes the colour they make. Alpha sits at one
//! unless something is wired into it, so three wires are the usual shape and
//! the fourth is there when it is wanted.
//!
//! **The numbers are scene-linear channels, not per cent**, exactly as a
//! Colour control's swatch holds them, and nothing is clamped on the way
//! through: a channel driven past one stays past one, which is what makes a
//! Split into a Combine give the colour back bit for bit.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Combine's controls.
///
/// Sliders rather than one colour swatch, because each row is a **socket**: the
/// point of the node is that a number arriving by wire becomes a channel, and a
/// swatch has nowhere for four separate wires to land. The 0..1 slider is the
/// range a colour is usually written in; a wire may carry any number, since a
/// driver's own sockets are never held to a range.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "combine",
    label = "Combine",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Combine {
    /// How much red.
    #[slider(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub red: f32,

    /// How much green.
    #[slider(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub green: f32,

    /// How much blue.
    #[slider(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub blue: f32,

    /// How opaque. One unless something is wired into it, so a Combine with
    /// three wires makes an opaque colour rather than an invisible one.
    #[slider(min = 0.0, max = 1.0, default = 1.0, unit = Raw)]
    pub alpha: f32,
}

/// The port the colour leaves by.
pub const COLOUR_PORT: &str = "colour";

/// Combine's behaviour.
pub struct CombineDef;

impl EffectDef for CombineDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Combine as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: &[Port {
                id: COLOUR_PORT,
                label: "Colour",
                ty: PortType::Colour,
                three_d: false,
            }],
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = Combine::read(cx.params);
        push(
            COLOUR_PORT,
            Value::Colour([p.red, p.green, p.blue, p.alpha]),
        );
    }
}

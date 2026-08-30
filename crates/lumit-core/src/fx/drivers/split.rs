//! Split (K-471 §1.3): a colour taken apart into its four numbers.
//!
//! **In plain terms.** Wire a colour into it — a Colour cycle, a Colour
//! control, or just the swatch on its own row — and out come four numbers: how
//! much red, green, blue and alpha that colour holds. Wire the red out into a
//! scale and the picture grows with the red of a track's tint.
//!
//! **Nothing is converted on the way through.** The channels leave exactly as
//! the colour holds them, scene-linear and unclamped, so a value above one
//! survives the trip: pairing this with [`combine`](super::combine) gives back
//! the colour that went in, bit for bit.

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Split's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "split",
    label = "Split",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Split {
    /// The colour to take apart. A wire from anything that makes a colour lands
    /// here; unwired, it is the swatch on the row, which makes the node a
    /// constant four numbers.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],
}

/// The port the red channel leaves by.
pub const RED_PORT: &str = "red";
/// The port the green channel leaves by.
pub const GREEN_PORT: &str = "green";
/// The port the blue channel leaves by.
pub const BLUE_PORT: &str = "blue";
/// The port the alpha channel leaves by.
pub const ALPHA_PORT: &str = "alpha";

/// Split's behaviour.
pub struct SplitDef;

impl EffectDef for SplitDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Split as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: &[
                Port {
                    id: RED_PORT,
                    label: "Red",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: GREEN_PORT,
                    label: "Green",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: BLUE_PORT,
                    label: "Blue",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: ALPHA_PORT,
                    label: "Alpha",
                    ty: PortType::Number,
                    three_d: false,
                },
            ],
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let c = Split::read(cx.params).colour;
        push(RED_PORT, Value::Float(c[0]));
        push(GREEN_PORT, Value::Float(c[1]));
        push(BLUE_PORT, Value::Float(c[2]));
        push(ALPHA_PORT, Value::Float(c[3]));
    }
}

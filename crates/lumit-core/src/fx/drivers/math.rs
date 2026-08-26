//! Math (K-471 §1.3): an expression you can see.
//!
//! **In plain terms.** Two numbers and one arithmetic sign. Wire something into
//! A, something into B, pick Multiply, and out comes the product — the same
//! thing an expression would do, drawn as a box with two wires going in so you
//! can tell at a glance what feeds what.
//!
//! Dividing by nought and taking the remainder by nought both answer nought
//! rather than infinity or a fault: a picture has to render, and a number that
//! became infinite would travel into a uniform and take the frame with it
//! (14-ENGINEERING-RULES §4).

use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
    CHOICE_UNGROUPED,
};
use lumit_fx_macros::Effect;

/// The operations Math offers, in dropdown order.
pub const OPERATIONS: &[&str] = &[
    "Add",
    "Subtract",
    "Multiply",
    "Divide",
    "Minimum",
    "Maximum",
    "Remainder",
    "Power",
];

/// Math's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "math",
    label = "Math",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct Math {
    /// The left-hand number.
    #[slider(min = -100.0, max = 100.0, default = 0.0, unit = Raw)]
    pub a: f32,

    /// The right-hand number.
    #[slider(min = -100.0, max = 100.0, default = 1.0, unit = Raw)]
    pub b: f32,

    /// What to do with them.
    #[choice(options = *OPERATIONS, dividers_after = CHOICE_UNGROUPED, default = 2)]
    pub operation: u32,
}

/// The port the result leaves by.
pub const VALUE_PORT: &str = "value";

/// Math's behaviour.
pub struct MathDef;

impl EffectDef for MathDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Math as EffectMetadata>::SCHEMA
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
        let p = Math::read(cx.params);
        push(VALUE_PORT, Value::Float(apply(p.operation, p.a, p.b)));
    }
}

/// The arithmetic itself, by option index. A result that is not a finite
/// number becomes nought — see the module note.
#[must_use]
pub fn apply(operation: u32, a: f32, b: f32) -> f32 {
    let v = match operation {
        0 => a + b,
        1 => a - b,
        3 => {
            if b == 0.0 {
                0.0
            } else {
                a / b
            }
        }
        4 => a.min(b),
        5 => a.max(b),
        6 => {
            if b == 0.0 {
                0.0
            } else {
                a % b
            }
        }
        7 => a.powf(b),
        // Multiply is the default, and is what an option index this build does
        // not know falls back to (K-065: an unknown choice renders).
        _ => a * b,
    };
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

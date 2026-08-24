//! Colour cycle (K-471 §1.3): a colour that turns through the hue wheel.
//!
//! **In plain terms.** A colour that keeps changing — red, orange, yellow, and
//! round again — at whatever rate you set. Wire it into a Fill, a Glow's tint
//! or a Gradient's stop and the picture cycles without a single keyframe.
//!
//! Phase is where on the wheel it starts, in whole turns; Rate is how many
//! turns a second. Leave Rate at nought and the colour holds still at Phase,
//! which is what you want when Phase is itself being driven by something else.

use crate::fx::{
    cpu, DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use lumit_fx_macros::Effect;

/// Colour cycle's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "colour_cycle",
    label = "Colour cycle",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct ColourCycle {
    /// Where on the wheel, in whole turns: 0 red, 1/3 green, 2/3 blue, 1 red
    /// again. Turns rather than degrees so that driving it from a Wiggle of
    /// amount 1 sweeps exactly one revolution.
    #[slider(min = 0.0, max = 1.0, default = 0.0, unit = Raw)]
    pub phase: f32,

    /// Turns per second. Nought holds the colour still at Phase.
    #[slider(min = -2.0, max = 2.0, default = 0.2, unit = Raw)]
    pub rate: f32,

    /// How coloured, per cent: 0 is grey, 100 is the full hue.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub saturation: f32,

    /// How bright, per cent of white. Unbounded above so a driven colour can
    /// reach scene-linear headroom, exactly as a Colour control's channels can.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub brightness: f32,
}

/// The port the colour leaves by.
pub const COLOUR_PORT: &str = "colour";

/// Colour cycle's behaviour.
pub struct ColourCycleDef;

impl EffectDef for ColourCycleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ColourCycle as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            outputs: &[Port {
                id: COLOUR_PORT,
                label: "Colour",
                ty: PortType::Colour,
            }],
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let p = ColourCycle::read(cx.params);
        let turns = f64::from(p.phase) + f64::from(p.rate) * cx.lt;
        // Folded into one turn before it becomes degrees: `hsv_to_rgb` wraps its
        // sector anyway, but folding here keeps the number small after an hour
        // of comp time, where an f32 of degrees would have lost precision.
        let degrees = (turns.rem_euclid(1.0) * 360.0) as f32;
        let rgb = cpu::hsv_to_rgb(
            degrees,
            (p.saturation / 100.0).clamp(0.0, 1.0),
            (p.brightness / 100.0).max(0.0),
        );
        push(COLOUR_PORT, Value::Colour([rgb[0], rgb[1], rgb[2], 1.0]));
    }
}

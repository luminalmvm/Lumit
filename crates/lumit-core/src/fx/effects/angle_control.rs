//! Angle control (docs/08 §3.81): one angle, held for something else to read.
//!
//! **In plain terms.** The [Slider control](super::slider_control) with a dial
//! instead of a track. It draws nothing; it exists so that an expression can
//! read a direction somebody set by turning it, and so that direction can be
//! keyframed once and drive several properties at a time.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The Angle control's one control.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "angle_control",
    label = "Angle control",
    version = 1,
    category = Controls,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct AngleControl {
    /// Degrees, unbounded — an angle animates through full turns rather than
    /// stopping at 360 (the [`Angle`](crate::fx::ParamKind::Angle) kind's own
    /// rule), which is exactly what a rig spinning something wants.
    #[dial(default = 0.0, step = 15.0)]
    pub angle: f32,
}

/// The Angle control's behaviour: none, by design.
pub struct AngleControlDef;

impl EffectDef for AngleControlDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AngleControl as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}

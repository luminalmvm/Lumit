//! Still effect parameters, one helper per kind.

use super::*;

/// A still float parameter.
pub(super) fn f(v: f64) -> EffectValue {
    EffectValue::Float(Property::fixed(v))
}

/// A still point parameter, in pixels at composition size.
pub(super) fn pt(x: f64, y: f64) -> EffectValue {
    EffectValue::Point(Property::fixed(x), Property::fixed(y))
}

/// A still colour parameter, scene-linear RGBA.
pub(super) fn colour(rgba: [f64; 4]) -> EffectValue {
    EffectValue::Colour(rgba.map(Property::fixed))
}

/// A dropdown, by option index.
pub(super) fn choice(i: u32) -> EffectValue {
    EffectValue::Choice(i)
}

/// A checkbox.
pub(super) fn on(b: bool) -> EffectValue {
    EffectValue::Bool(b)
}

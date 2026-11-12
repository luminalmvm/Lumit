//! The first and last source positions a clip's Retime map reaches.

use super::*;

/// The last source position a map reaches — what the clip asks of its source.
#[frb(ignore)]
pub(super) fn map_start_value(
    map: &lumit_core::anim::Property,
    now: lumit_core::time::Rational,
) -> Option<lumit_core::time::Rational> {
    let lumit_core::anim::Animation::Keyframed(keys) = &map.animation else {
        return None;
    };
    let first = keys.first()?.value;
    if (first - now.to_f64()).abs() < 1e-9 {
        return None;
    }
    lumit_core::time::Rational::from_f64_on_grid(first, lumit_core::time::Rational::FLICK_DEN).ok()
}

pub(super) fn map_end_value(
    map: &lumit_core::anim::Property,
) -> Option<lumit_core::time::Rational> {
    let lumit_core::anim::Animation::Keyframed(keys) = &map.animation else {
        return None;
    };
    let last = keys.last()?;
    lumit_core::time::Rational::from_f64_on_grid(last.value, lumit_core::time::Rational::FLICK_DEN)
        .ok()
}

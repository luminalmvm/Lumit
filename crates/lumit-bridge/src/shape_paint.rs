//! A shape item's fill and outline colours across the bridge: four scalars,
//! the same shape an effect's colour crosses as, so a channel keys on the
//! same stopwatch. `None` is no fill or no outline, on both sides.

use lumit_core::anim::Property;
use lumit_core::time::Rational;

use crate::api::effect::{BridgeColour, BridgeScalar};
use crate::api::BridgeError;

pub(crate) fn read(channels: Option<&[Property; 4]>, offset: Rational) -> Option<BridgeColour> {
    let [r, g, b, a] = channels?
        .each_ref()
        .map(|p| BridgeScalar::read_at(p, offset));
    Some(BridgeColour { r, g, b, a })
}

pub(crate) fn write(
    colour: Option<&BridgeColour>,
    offset: Rational,
) -> Result<Option<[Property; 4]>, BridgeError> {
    let Some(colour) = colour else {
        return Ok(None);
    };
    let property = |s: &BridgeScalar| -> Result<Property, BridgeError> {
        Ok(Property {
            animation: s.animation_at(offset)?,
            extra: serde_json::Map::new(),
        })
    };
    Ok(Some([
        property(&colour.r)?,
        property(&colour.g)?,
        property(&colour.b)?,
        property(&colour.a)?,
    ]))
}

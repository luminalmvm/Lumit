//! A shape item's colours across the bridge: the fill, the outline and the
//! gradient's. Four scalars, the same shape an effect's colour crosses as, so
//! a channel keys on the same stopwatch.

use lumit_core::anim::Property;
use lumit_core::time::Rational;

use crate::api::effect::{BridgeColour, BridgeScalar};
use crate::api::BridgeError;

pub(crate) fn read(channels: &[Property; 4], offset: Rational) -> BridgeColour {
    let [r, g, b, a] = channels
        .each_ref()
        .map(|p| BridgeScalar::read_at(p, offset));
    BridgeColour { r, g, b, a }
}

pub(crate) fn write(colour: &BridgeColour, offset: Rational) -> Result<[Property; 4], BridgeError> {
    let property = |s: &BridgeScalar| -> Result<Property, BridgeError> {
        Ok(Property {
            animation: s.animation_at(offset)?,
            extra: serde_json::Map::new(),
        })
    };
    Ok([
        property(&colour.r)?,
        property(&colour.g)?,
        property(&colour.b)?,
        property(&colour.a)?,
    ])
}

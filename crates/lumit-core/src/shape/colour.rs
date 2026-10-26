//! A shape's fill and outline colours: four channels, each a property of its
//! own, so a colour keys the way every other colour in the model does. A
//! colour nobody has keyed is written as the four plain numbers it always was,
//! so a file from before stays the same file.

use crate::anim::Property;
use crate::model::LinearColour;

/// The four channels of a still colour.
pub fn still(colour: LinearColour) -> [Property; 4] {
    colour.0.map(|v| Property::fixed(f64::from(v)))
}

/// The colour at `t`, on the layer's clock.
pub fn at(channels: &[Property; 4], t: f64) -> LinearColour {
    LinearColour(channels.each_ref().map(|p| p.value_at(t) as f32))
}

/// Whether any channel moves.
pub fn is_animated(channels: &[Property; 4]) -> bool {
    channels.iter().any(Property::is_animated)
}

/// serde for an `Option<[Property; 4]>`: `null`, or a list of four numbers or
/// keyed properties.
pub(crate) mod keyed {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    use crate::anim::Property;

    pub fn serialize<S: Serializer>(c: &Option<[Property; 4]>, s: S) -> Result<S::Ok, S::Error> {
        match c {
            Some(channels) => crate::mask::still_or_keyed_vec::serialize(channels, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[Property; 4]>, D::Error> {
        #[derive(Deserialize)]
        struct Channels(#[serde(with = "crate::mask::still_or_keyed_vec")] Vec<Property>);
        let Some(Channels(list)) = Option::<Channels>::deserialize(d)? else {
            return Ok(None);
        };
        list.try_into()
            .map(Some)
            .map_err(|_| D::Error::custom("a colour has four channels"))
    }
}

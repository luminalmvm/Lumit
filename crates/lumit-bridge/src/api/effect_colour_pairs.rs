use flutter_rust_bridge::frb;

use crate::api::{effect::BridgeEffectInstance, BridgeError};

#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeColourPair {
    pub first: String,
    pub second: String,
}

#[frb(sync)]
pub fn list_colour_pairs(effect: String) -> Vec<BridgeColourPair> {
    lumit_core::fx::colour_pairs::of(&effect)
        .map(|(first, second)| BridgeColourPair {
            first: first.to_owned(),
            second: second.to_owned(),
        })
        .collect()
}

impl BridgeEffectInstance {
    #[frb(sync)]
    pub fn swap_colours(&mut self, id: String) -> Result<(), BridgeError> {
        if lumit_core::fx::colour_pairs::swap(&mut self.effect, &id) {
            Ok(())
        } else {
            Err(BridgeError::InvalidParam)
        }
    }
}

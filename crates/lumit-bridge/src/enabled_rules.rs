//! A greying rule's condition, as the bridge carries it.

use crate::api::effect::BridgeEnabledCond;
use lumit_core::fx::schema::EnabledCond;

pub(crate) fn cond(cond: EnabledCond) -> BridgeEnabledCond {
    match cond {
        EnabledCond::BoolIs(v) => BridgeEnabledCond::BoolIs(v),
        EnabledCond::ChoiceIs(i) => BridgeEnabledCond::ChoiceIs(i),
        EnabledCond::ChoiceIsNot(i) => BridgeEnabledCond::ChoiceIsNot(i),
        EnabledCond::LayerSet => BridgeEnabledCond::LayerSet,
        EnabledCond::LayerUnset => BridgeEnabledCond::LayerUnset,
        EnabledCond::Nought => BridgeEnabledCond::Nought,
        EnabledCond::Or(other, then) => {
            BridgeEnabledCond::Or(other.to_owned(), Box::new(self::cond(*then)))
        }
    }
}

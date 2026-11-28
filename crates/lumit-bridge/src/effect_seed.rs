use lumit_core::model::EffectParam;
use lumit_core::time::Rational;

use crate::api::{
    effect::{BridgeEffectValue, BridgeScalar},
    BridgeError,
};

pub(crate) fn read(param: &EffectParam, seed: u32, offset: Rational) -> BridgeEffectValue {
    BridgeEffectValue::Seed(
        param
            .keyed_seed()
            .map_or(BridgeScalar::Static(f64::from(seed)), |keys| {
                BridgeScalar::read_at(keys, offset)
            }),
    )
}

pub(crate) fn write(
    param: &mut EffectParam,
    seed: BridgeScalar,
    offset: Rational,
) -> Result<(), BridgeError> {
    param
        .set_seed(seed.animation_at(offset)?)
        .then_some(())
        .ok_or(BridgeError::ParamKindMismatch)
}

use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::expression::{ExpressionContext, Row};
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::api::{
    effect::{
        document_for, sample_scalar_with_context, BridgeColour, BridgeRational, BridgeScalar,
    },
    layer::LayerReference,
};

#[frb(sync)]
pub fn sample_colour_with_context(
    colour: BridgeColour,
    time: BridgeRational,
    layer: LayerReference,
    effect: Uuid,
    param: String,
) -> Vec<f64> {
    let context = document_for(&layer).map(|document| {
        Arc::new(ExpressionContext {
            document,
            comp: Some(layer.comp_id),
            layer: Some(layer.layer_id),
            comp_time: Rational::new(time.num, time.den)
                .unwrap_or(Rational::ZERO)
                .to_f64(),
            current_depth: 0,
            inputs: None,
        })
    });
    [colour.r, colour.g, colour.b, colour.a]
        .into_iter()
        .enumerate()
        .map(|(channel, scalar)| match (scalar, &context) {
            (BridgeScalar::Expression(text, language, _), Some(context)) => {
                lumit_core::expression::within_steps(
                    lumit_core::expression::INTERFACE_STEPS,
                    || {
                        lumit_core::expression::channel::evaluate_row(
                            language.read(),
                            &text,
                            context.clone(),
                            Row::Channel {
                                effect,
                                param: &param,
                                channel,
                            },
                        )
                    },
                )
            }
            (scalar, _) => sample_scalar_with_context(scalar, time, layer, None, None),
        })
        .collect()
}

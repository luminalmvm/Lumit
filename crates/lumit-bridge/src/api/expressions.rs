use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::{expression::ExpressionContext, time::Rational};

use crate::api::{
    effect::{document_for, BridgeRational},
    layer::LayerReference,
};

#[frb(opaque)]
pub struct Expressions {}

impl Expressions {
    pub fn get_expressions_metadata() -> String {
        lumit_core::expression::get_api_metadata()
    }
}

#[frb(sync)]
pub fn text_expression_fault(
    expression: String,
    time: BridgeRational,
    layer: LayerReference,
) -> Option<String> {
    let context = Arc::new(ExpressionContext {
        document: document_for(&layer)?,
        comp: Some(layer.comp_id),
        layer: Some(layer.layer_id),
        comp_time: Rational::new(time.num, time.den)
            .unwrap_or(Rational::ZERO)
            .to_f64(),
        current_depth: 0,
        inputs: None,
    });
    lumit_core::expression::within_steps(lumit_core::expression::INTERFACE_STEPS, || {
        lumit_core::expression::evaluate_text(&expression, Some(context)).err()
    })
}

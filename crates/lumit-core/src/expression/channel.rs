use std::sync::Arc;

use super::script::Language;
use super::{ExpressionContext, Row};
use crate::model::{EffectInstance, EffectValue};

pub(super) fn note_colour(inst: &mut EffectInstance, param: &str) -> bool {
    let Some(EffectValue::Colour(channels)) = inst
        .params
        .iter_mut()
        .find(|p| p.id == param)
        .map(|p| &mut p.value)
    else {
        return false;
    };
    let colour = channels.each_ref().map(super::underlying_value);
    for (axis, channel) in channels.iter_mut().enumerate() {
        super::note_point(channel, &colour, axis);
    }
    true
}

pub fn evaluate_row(
    language: Language,
    expression: &str,
    context: Arc<ExpressionContext>,
    row: Row<'_>,
) -> f64 {
    let slot = super::slot_of_row(&context, row, None);
    match language {
        Language::Rhai => {
            super::convert_result(super::eval_dynamic(expression, Some(context), &[]), slot)
        }
        Language::JavaScript => {
            let own = super::own_of_row(&context, row);
            super::javascript_owned(expression, Some(&context), slot, own)
        }
    }
}

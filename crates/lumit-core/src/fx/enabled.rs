//! Whether a row is live, by its effect's greying rules.

use super::builtins::schema;
use super::schema::EnabledCond;
use crate::model::{EffectInstance, EffectValue};

pub fn param_enabled(inst: &EffectInstance, id: &str) -> bool {
    let Some(s) = schema(&inst.effect.match_name) else {
        // No built-in schema (an OFX or placeholder instance) means no rules,
        // so nothing is greyed.
        return true;
    };
    s.enabled_when
        .iter()
        .filter(|rule| rule.param == id)
        .all(|rule| {
            // A rule naming a parameter the instance does not carry cannot be
            // judged, so it does not grey anything: an older instance that
            // predates the deciding parameter stays fully editable rather than
            // locking a row it can never unlock (the `fill_missing_params`
            // trap, from the other side).
            let Some(value) = inst.param(rule.on) else {
                return true;
            };
            holds(inst, rule.cond, value)
        })
}

fn holds(inst: &EffectInstance, cond: EnabledCond, value: &EffectValue) -> bool {
    match (cond, value) {
        (EnabledCond::Or(other, then), _) => {
            matches!(inst.param(other), Some(EffectValue::Bool(true))) || holds(inst, *then, value)
        }
        (EnabledCond::BoolIs(want), EffectValue::Bool(got)) => *got == want,
        (EnabledCond::ChoiceIs(want), EffectValue::Choice(got)) => *got == want,
        (EnabledCond::ChoiceIsNot(no), EffectValue::Choice(got)) => *got != no,
        (EnabledCond::LayerSet, EffectValue::Layer(layer)) => layer.is_some(),
        (EnabledCond::LayerUnset, EffectValue::Layer(layer)) => layer.is_none(),
        (EnabledCond::Nought, EffectValue::Float(p)) => {
            matches!(p.animation, crate::anim::Animation::Static(v) if v == 0.0)
        }
        // A rule pointed at the wrong kind of parameter is a schema
        // mistake, not a reason to grey a row the owner can then never
        // reach. `every_enablement_rule_names_a_parameter_of_its_kind`
        // fails the build for it instead.
        _ => true,
    }
}

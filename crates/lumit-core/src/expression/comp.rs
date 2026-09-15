use std::sync::Arc;

use rhai::plugin::*;
use uuid::Uuid; // a "prelude" import for macros

use crate::comp_graph::InputKind;
use crate::expression::ExpressionContext;
use crate::model::EffectValue;

/// What an Input named `name` is worth inside the graph this expression is
/// running in (docs/impl/node-graph-comp.md §5.5): the host's own value where
/// it handed one over, and the Input's declared default otherwise.
///
/// `None` for a picture Input, which carries a texture rather than a number,
/// and for a name the graph does not have - both of which the caller answers
/// with the module's miss value, as a layer reference that names nothing does.
fn input_value(context: &Arc<ExpressionContext>, name: &str) -> Option<f64> {
    if let Some(values) = &context.inputs {
        if let Some((_, value)) = values.iter().find(|(id, _)| id == name) {
            // Evaluated a level deeper, so a value that is itself an
            // expression naming this Input stops rather than spinning.
            let deeper = Arc::new(context.increase_depth());
            return match value {
                EffectValue::Float(p) => Some(p.value_at_with_context(context.comp_time, deeper)),
                EffectValue::Colour(c) => {
                    Some(c[0].value_at_with_context(context.comp_time, deeper))
                }
                _ => None,
            };
        }
    }
    let graph = context.document.comp(context.comp?)?.graph.as_ref()?;
    let input = graph
        .inputs()
        .find(|input| input.id == name || input.label == name)?;
    match input.kind {
        InputKind::Picture => None,
        InputKind::Number | InputKind::Angle | InputKind::Colour => Some(input.default[0]),
    }
}

// Rhai's `#[export_module]` expands to argument-unwrapping code of its own,
// which trips `clippy::unwrap_used` on the generated `&mut` receivers. The
// lint is about *our* unwraps, and there is no way to spell these differently
// short of dropping the macro, so it is switched off for the generated module
// only — not for the module's callers, and not for the helpers above.
#[allow(clippy::unwrap_used)]
#[export_module]
pub mod comp {

    use crate::expression::ExpressionContext;

    #[derive(Clone, CustomType)]
    pub struct Comp {
        id: Option<Uuid>,
    }

    /// get the current composition
    pub fn comp(context: NativeCallContext) -> Comp {
        Comp {
            id: ExpressionContext::from_call(&context).comp,
        }
    }

    /// get the value of a node graph's Input by its id or its label
    pub fn input(context: NativeCallContext, name: &str) -> f64 {
        // The miss value the layer module answers an invalid reference with:
        // a picture Input and a name the graph does not have are both misses,
        // and neither is worth stopping an expression for.
        super::input_value(&ExpressionContext::from_call(&context), name).unwrap_or(-1.0)
    }

    /// get the name of a composition
    #[rhai_fn(get = "name")]
    pub fn name(context: NativeCallContext, this: &mut Comp) -> String {
        let context = ExpressionContext::from_call(&context);
        this.id
            .and_then(|id| context.document.comp(id))
            .map_or_else(|| "Invalid Comp Reference".into(), |c| c.name.clone())
    }
}

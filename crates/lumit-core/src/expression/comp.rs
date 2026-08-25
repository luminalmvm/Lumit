use rhai::plugin::*;
use uuid::Uuid; // a "prelude" import for macros

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

    /// get the name of a composition
    #[rhai_fn(get = "name")]
    pub fn name(context: NativeCallContext, this: &mut Comp) -> String {
        let context = ExpressionContext::from_call(&context);
        this.id
            .and_then(|id| context.document.comp(id))
            .map_or_else(|| "Invalid Comp Reference".into(), |c| c.name.clone())
    }
}

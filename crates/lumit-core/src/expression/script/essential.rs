//! `essentialProperty`: what a copy of a graphics template holds for one of
//! the template's controls.
//!
//! After Effects reads a layer's Essential Property by the controller's name,
//! `thisLayer.essentialProperty("Title")`, and older projects spell it
//! `masterProperty`. Here the layer is a copy and the name is a control's
//! label. The answer is the copy's own value where it set one, and the
//! template's row where it did not.

use std::rc::Rc;
use std::sync::Arc;

use uuid::Uuid;

use super::interp::{fail, Interp, Res, Value};
use crate::expression::ExpressionContext;
use crate::model::LayerKind;
use crate::template::Value as Held;

impl Interp<'_> {
    /// The value `layer` of `comp` holds for the control called `name`: a
    /// number, a list for a point or a colour, words for text, true or false
    /// for a checkbox, and a menu's option counting from 1.
    pub(super) fn essential_property(&mut self, comp: Uuid, layer: Uuid, name: &str) -> Res<Value> {
        let context = self.ae.context;
        let doc = &context.document;
        let placed = doc
            .comp(comp)
            .and_then(|c| c.layers.iter().find(|l| l.id == layer));
        let Some(placed) = placed else {
            return fail("a layer that is no longer there");
        };
        let template = match &placed.kind {
            LayerKind::Precomp { comp: nested } => {
                doc.comp(*nested).and_then(|c| Some((c, c.template()?)))
            }
            _ => None,
        };
        let Some((inside, template)) = template else {
            return fail(format!(
                "'{}' is not a copy of a graphics template",
                placed.name
            ));
        };
        let name = name.trim();
        let controls = &template.controls;
        let control = controls
            .iter()
            .find(|c| c.label == name)
            .or_else(|| controls.iter().find(|c| c.label.eq_ignore_ascii_case(name)));
        let Some(control) = control else {
            return fail(format!("no control called '{name}'"));
        };

        let time = self.ae.time;
        let own = placed.graph_inputs.as_ref().and_then(|values| {
            // A row that is itself an expression runs a level deeper, so two
            // that read each other stop.
            let deeper = Arc::new(ExpressionContext {
                document: doc.clone(),
                comp: Some(comp),
                layer: Some(layer),
                comp_time: time,
                current_depth: context.current_depth + 1,
                inputs: context.inputs.clone(),
            });
            let local = time - placed.start_offset.0.to_f64();
            crate::fx::effects::template::value_of(values, control, local, Some(&deeper))
        });
        let held = own.or_else(|| crate::template::rows::read(doc, inside, &control.target));
        match held {
            Some(Held::Number(n)) => Ok(Value::Num(n)),
            Some(Held::Point(axes)) => self.numbers(&axes),
            Some(Held::Colour(channels)) => self.numbers(&channels),
            Some(Held::Tick(on)) => Ok(Value::Bool(on)),
            // After Effects counts a menu from one, and an expression that
            // reads one compares against that.
            Some(Held::Choice(option)) => Ok(Value::Num(f64::from(option) + 1.0)),
            Some(Held::Text(words)) => Ok(Value::Str(Rc::from(words.as_str()))),
            Some(Held::Font { family, .. }) => Ok(Value::Str(Rc::from(family.as_str()))),
            Some(Held::Picture(_)) | None => fail(format!(
                "the control '{name}' holds nothing an expression can read"
            )),
        }
    }
}

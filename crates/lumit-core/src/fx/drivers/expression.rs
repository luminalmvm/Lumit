//! Expression (node-graph.md §1.3): a box whose outputs are what an
//! expression returns.
//!
//! **In plain terms.** Type an expression — `time * 2`, `sin(time) * 50`,
//! `[comp_width / 2, comp_height / 2]`, `[1, 0.5, 0]` — and the box hands out
//! its answer on a wire, so a parameter can follow a line of arithmetic
//! instead of its keyframes. It is the same language a property's own
//! expression is written in, with the same names in scope (`time`, the comp's
//! size and rate, `layer("…")`, `comp("…")`), and one evaluation per frame.
//!
//! **Four sockets, and the answer decides which carry a value.** A port is a
//! fact about the catalogue entry — one shared `ExpressionDef` serves every
//! Expression box — and nothing revalidates a wire when the text changes, so
//! an output whose *type* followed the result would strand a wire the moment
//! the user rewrote the text. Instead the box always declares Value, Colour,
//! Point x and Point y, exactly the shape [`split`](super::split) has, and each
//! frame the result fills only the sockets of its own kind: a number fills
//! Value, a pair fills Point x and Point y, three or four numbers fill Colour.
//! The others carry nothing, and a parameter wired to one reads its own
//! keyframes for that frame — the same calm an unwired socket has.
//!
//! **A refused expression carries nothing.** A syntax error, a name that is
//! not in scope, a division by nought, a result that is not a number, a point
//! or a colour: every one of these pushes no value at all, so what the box
//! drives falls back to its keyframes rather than snapping to a number that
//! looks like an answer. The sentence about *why* is
//! [`crate::expression::evaluate_value`]'s to give and an editor's to show.
//!
//! **The answer is the same every time.** It depends on the source text and
//! on what the frame's expression context carries — the document, the comp,
//! the layer and the time — and on nothing else: no wall clock, no render
//! order, no shared state. The same box at the same time hands out the same
//! value in the preview and in the export, on every machine.
//!
//! **The text is not a parameter.** It lives on the instance in
//! `EffectInstance.extra`, under `extra["expression"]["source"]`, exactly where
//! the Custom shader keeps its WGSL: a string is not `Copy`, two expressions
//! cannot be interpolated, and it is the thing the outputs are derived *from*
//! rather than one of them. [`source_of`] and [`set_source`] are the two ways
//! in, so nothing outside this file needs to know the key.

use crate::expression::{evaluate_value, ExprValue};
use crate::fx::{
    DriverCx, EffectDef, EffectMetadata, EffectSchema, Port, PortType, Signature, Value,
};
use crate::model::EffectInstance;
use lumit_fx_macros::Effect;

/// The `extra` key the expression block lives under.
pub const EXTRA_KEY: &str = "expression";

/// Expression's controls.
///
/// One row, and it is a button rather than a value: the text the box runs is
/// not a parameter (see the module doc), so there is nothing here to keyframe.
/// The row is the Custom shader's `edit` row again — the place the panel puts
/// a way into the text — and it is what keeps the manual's page for the box
/// from describing no control at all.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "expression",
    label = "Expression",
    version = 1,
    category = Drivers,
    cost = Trivial,
    roi = Exact,
    // A driver makes a value, not a picture, so there is nothing for a matte to
    // gate (the matte carriage's `None`, as the Controls family declares).
    matte = false,
)]
pub struct Expression {
    /// Open the editor surface. A button, not a value — the source is not a
    /// parameter, so this row carries none.
    #[action(label = "Edit expression…")]
    pub edit: (),
}

/// The port a number leaves by.
pub const VALUE_PORT: &str = "value";
/// The port a colour leaves by.
pub const COLOUR_PORT: &str = "colour";
/// The port a point's x leaves by.
pub const POINT_X_PORT: &str = "point_x";
/// The port a point's y leaves by.
pub const POINT_Y_PORT: &str = "point_y";

/// The expression text this instance holds, or the empty string for a fresh
/// one — which evaluates to nothing and so drives nothing.
#[must_use]
pub fn source_of(inst: &EffectInstance) -> &str {
    inst.extra
        .get(EXTRA_KEY)
        .and_then(|block| block.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

/// Write the expression text onto `inst` — what an editor's commit does. Any
/// other key under the block is kept, so a later field beside the source
/// survives an edit of the text.
pub fn set_source(inst: &mut EffectInstance, source: &str) {
    let block = inst
        .extra
        .entry(EXTRA_KEY.to_owned())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !block.is_object() {
        *block = serde_json::Value::Object(serde_json::Map::new());
    }
    if let Some(map) = block.as_object_mut() {
        map.insert(
            "source".to_owned(),
            serde_json::Value::String(source.to_owned()),
        );
    }
}

/// Expression's behaviour.
pub struct ExpressionDef;

impl EffectDef for ExpressionDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Expression as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn signature(&self) -> Signature {
        Signature::Data {
            inputs: &[],
            outputs: &[
                Port {
                    id: VALUE_PORT,
                    label: "Value",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: COLOUR_PORT,
                    label: "Colour",
                    ty: PortType::Colour,
                    three_d: false,
                },
                Port {
                    id: POINT_X_PORT,
                    label: "Point x",
                    ty: PortType::Number,
                    three_d: false,
                },
                Port {
                    id: POINT_Y_PORT,
                    label: "Point y",
                    ty: PortType::Number,
                    three_d: false,
                },
            ],
        }
    }

    fn eval_driver(&self, cx: &DriverCx<'_>, push: &mut dyn FnMut(&'static str, Value)) {
        let source = source_of(cx.inst);
        if source.trim().is_empty() {
            return;
        }
        // A refusal is `Err`, and it pushes nothing: the sentence is the
        // editor's to show, and the parameter's keyframes are the calm degrade.
        let Ok(result) = evaluate_value(source, Some(cx.context.clone())) else {
            return;
        };
        match result {
            // `1.0 / 0.0` is not a refusal in Rhai, it is an infinity — and an
            // infinity or a NaN on a wire is a frame key that never matches
            // and a kernel handed a number that is not one. It is treated as
            // the refusal it is.
            ExprValue::Number(n) if n.is_finite() => push(VALUE_PORT, Value::Float(n as f32)),
            ExprValue::Number(_) => {}
            ExprValue::Point(x, y) if x.is_finite() && y.is_finite() => {
                push(POINT_X_PORT, Value::Float(x as f32));
                push(POINT_Y_PORT, Value::Float(y as f32));
            }
            ExprValue::Point(..) => {}
            ExprValue::Colour(c) if c.iter().all(|v| v.is_finite()) => {
                push(COLOUR_PORT, Value::Colour(c));
            }
            ExprValue::Colour(_) => {}
        }
    }
}

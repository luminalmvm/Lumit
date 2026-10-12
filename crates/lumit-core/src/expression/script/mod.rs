//! JavaScript expressions, with what After Effects adds to them.
//!
//! **In plain terms.** People arrive with years of expressions written for
//! After Effects, and those are JavaScript: `var`, functions, loops,
//! `effect("Shake")("Seed")`, `wiggle(3, 20)`, `value + [x, y]`. This module
//! runs them as they are, so a preset or a project brought across keeps
//! moving the way it did. It is the second of the two languages an expression
//! can be written in ([`Language`]), and the one its author picks by name.
//!
//! It is a small interpreter rather than a JavaScript engine off the shelf,
//! for three reasons. After Effects' arithmetic is not JavaScript's:
//! `[10, 20] + [1, 2]` is `[11, 22]` there, and every expression on a position
//! relies on it. An expression runs for every frame on a render thread, so it
//! must be stoppable and must answer the same on every machine. And the whole
//! of the language an expression uses fits in a few files.
//!
//! [`lex`] and [`parse`] turn the text into a tree, [`interp`] walks it, and
//! [`ae`] is what After Effects adds: the comp, the layers, their effects, and
//! the helpers.

mod ae;
mod interp;
mod lex;
mod parse;

use crate::expression::ExpressionContext;
pub(crate) use interp::sparingly;
use interp::{to_number, Interp, Value};
pub(crate) use parse::Program;

/// The language an expression is written in.
///
/// It is chosen with the expression and stored beside it, never worked out
/// from the text: `7 / 2` is 3 in Rhai and 3.5 in JavaScript, and nothing in
/// those five characters says which was meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// Lumit's first language: one line of [Rhai](https://rhai.rs). What an
    /// expression with nothing stored beside it is, which is every expression
    /// in a project saved before there was a choice.
    #[default]
    Rhai,
    /// JavaScript as After Effects has it.
    JavaScript,
}

impl Language {
    /// The word a document stores a language as.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Language::Rhai => "rhai",
            Language::JavaScript => "javascript",
        }
    }

    /// The language a stored word names. A word no build knows is Rhai, as a
    /// missing one is.
    #[must_use]
    pub fn from_id(id: &str) -> Language {
        if id == Language::JavaScript.id() {
            Language::JavaScript
        } else {
            Language::Rhai
        }
    }

    /// The language of the expression on the property `extra` belongs to.
    /// One lookup, since this is asked before every evaluation.
    #[must_use]
    pub fn of(extra: &serde_json::Map<String, serde_json::Value>) -> Language {
        extra
            .get(SLOT_KEY)
            .and_then(|kept| kept.get("language"))
            .and_then(serde_json::Value::as_str)
            .map_or(Language::Rhai, Language::from_id)
    }
}

/// What an expression is told about the property it is written on.
///
/// **In plain terms.** `value` in an expression means "what this property
/// held before the expression took over", and `wiggle` wanders around it. A
/// position is two numbers in After Effects and two separate properties in
/// Lumit, so each of the two carries the same expression, the whole pair as
/// its `value`, and which of the pair it is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Slot {
    value: [f64; 4],
    len: u8,
    /// Which number of the expression's answer this property takes.
    pub axis: u8,
    /// What tells this property's random numbers from another's.
    pub seed: u32,
    /// The language the expression is written in.
    pub language: Language,
}

/// The key a [`Slot`] is kept under in a property's `extra`.
pub const SLOT_KEY: &str = "expr";

impl Slot {
    /// A slot for the property that is number `axis` of `value`.
    #[must_use]
    pub fn new(value: &[f64], axis: usize, seed: u32) -> Slot {
        let mut slot = Slot {
            len: value.len().min(4) as u8,
            axis: axis.min(3) as u8,
            seed,
            ..Slot::default()
        };
        for (to, from) in slot.value.iter_mut().zip(value) {
            *to = *from;
        }
        slot
    }

    /// The same slot, for an expression written in `language`.
    #[must_use]
    pub fn in_language(mut self, language: Language) -> Slot {
        self.language = language;
        self
    }

    /// The property's own value: empty when nobody recorded one.
    #[must_use]
    pub fn values(&self) -> &[f64] {
        self.value.get(..usize::from(self.len)).unwrap_or_default()
    }

    /// This property's own number out of [`Self::values`].
    #[must_use]
    pub fn own(&self) -> Option<f64> {
        self.values().get(usize::from(self.axis)).copied()
    }

    /// Read a slot back out of a property's `extra`.
    #[must_use]
    pub fn read(extra: &serde_json::Map<String, serde_json::Value>) -> Slot {
        let Some(kept) = extra.get(SLOT_KEY).and_then(serde_json::Value::as_object) else {
            return Slot::default();
        };
        let number = |key: &str| kept.get(key).and_then(serde_json::Value::as_u64);
        let mut value = [0.0; 4];
        let mut len = 0u8;
        match kept.get("value") {
            Some(serde_json::Value::Array(items)) => {
                for (to, from) in value.iter_mut().zip(items) {
                    *to = from.as_f64().unwrap_or(0.0);
                    len += 1;
                }
            }
            Some(one) => {
                if let (Some(one), Some(to)) = (one.as_f64(), value.first_mut()) {
                    *to = one;
                    len = 1;
                }
            }
            None => {}
        }
        Slot {
            value,
            len,
            axis: number("axis").map_or(0, |n| n.min(3) as u8),
            seed: number("seed").map_or(0, |n| n as u32),
            language: Language::of(extra),
        }
    }

    /// Write this slot into a property's `extra`.
    pub fn write(&self, extra: &mut serde_json::Map<String, serde_json::Value>) {
        let mut kept = serde_json::Map::new();
        match self.values() {
            [] => {}
            [one] => {
                kept.insert("value".into(), serde_json::json!(one));
            }
            many => {
                kept.insert("value".into(), serde_json::json!(many));
            }
        }
        if self.axis != 0 {
            kept.insert("axis".into(), serde_json::json!(self.axis));
        }
        if self.seed != 0 {
            kept.insert("seed".into(), serde_json::json!(self.seed));
        }
        if self.language != Language::Rhai {
            kept.insert("language".into(), serde_json::json!(self.language.id()));
        }
        if kept.is_empty() {
            extra.remove(SLOT_KEY);
        } else {
            extra.insert(SLOT_KEY.into(), serde_json::Value::Object(kept));
        }
    }
}

/// What an expression came to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    Number(f64),
    List(Vec<f64>),
    Text(String),
}

/// Read `source` as an expression in this language.
pub(crate) fn compile(source: &str) -> Result<Program, String> {
    parse::parse(source)
}

/// Whether every name `program` reads without declaring is one this language
/// provides.
pub(crate) fn names_known(program: &Program) -> bool {
    program
        .free_names()
        .all(|name| ae::knows(name) || is_input(name))
}

/// Whether `name` is one of an Expression box's inputs, `input_1` upwards,
/// which both languages read by name.
pub(crate) fn is_input(name: &str) -> bool {
    name.strip_prefix("input_")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Run `program` and say what it came to, or why it did not.
pub(crate) fn run(
    program: &Program,
    context: &ExpressionContext,
    slot: Slot,
    vars: &[(&str, f64)],
) -> Result<Answer, String> {
    let _running = interp::Running::begin();
    let mut interp = Interp::new(program, ae::Ae::new(context, slot, vars));
    let value = interp.run()?;
    let value = interp.plain(value).map_err(|abort| match abort {
        interp::Abort::Error(why) => why,
        interp::Abort::Throw(_) => "the expression threw".to_owned(),
    })?;
    Ok(match &value {
        Value::Array(_) => Answer::List(
            interp
                .vector(&value)
                .map_err(|_| "a list holding something that is not a number".to_owned())?,
        ),
        Value::Str(text) => Answer::Text(text.to_string()),
        Value::Num(_) | Value::Bool(_) => Answer::Number(to_number(&value)),
        Value::Undefined | Value::Null => return Err("the expression came to nothing".into()),
        _ => return Err("the expression did not come to a value".into()),
    })
}

#[cfg(test)]
mod tests;

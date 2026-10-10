//! Expressions: a small script on a property, evaluated every time that
//! property is read.
//!
//! In plain terms: instead of a number or a row of keyframes, a property can
//! hold a line of code — `time * 90`, `layer("Sun").x` — and the answer is
//! worked out afresh at each frame.
//!
//! There are two languages ([`Language`]), and whoever writes an expression
//! picks which it is in. [Rhai] is one line long; the values it can see (the
//! comp, the layers, `time`) are assembled in [`apply_context_to_scope`] and
//! [`ExpressionContext`]. JavaScript is the language After Effects
//! expressions are written in, and [`script`] runs it, so one brought across
//! from there keeps working as written. The choice is stored beside the text
//! and never guessed from it.
//!
//! The thing worth knowing before editing this file is that **evaluation is on
//! the hot path**. Every driven property is re-evaluated for every frame, in
//! both the renderer and the frame-cache key, so anything done per evaluation
//! is done tens of thousands of times a second. That is why engines are pooled
//! rather than built (see [`with_engine`]).
//!
//! [Rhai]: https://rhai.rs

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use crate::Document;
use rhai::{exported_module, Dynamic, Engine, Scope};
use uuid::Uuid;

mod comp;
mod layer;
mod math;
mod script;

pub use script::{Language, Slot, SLOT_KEY};

// What one expression is allowed to build. A property wants a number, a point,
// a colour or a line of text, so these are far above anything a real expression
// asks for. Without them a saved project can call `blob(50_000_000)` or pad an
// array to five million items on every property read, and nothing stops it.
const MAX_EXPRESSION_OPERATIONS: u64 = 100_000;
const MAX_EXPRESSION_STRING_BYTES: usize = 1 << 20;
const MAX_EXPRESSION_ARRAY_ITEMS: usize = 16_384;
const MAX_EXPRESSION_MAP_ITEMS: usize = 16_384;

#[derive(Clone, Debug)]
pub struct ExpressionContext {
    pub document: Arc<Document>,
    pub comp: Option<Uuid>,
    pub layer: Option<Uuid>,
    pub comp_time: f64,
    pub current_depth: u32,
    /// The values a host hands a node graph's Inputs, by Input id
    /// (docs/impl/node-graph-comp.md §5.5) - what `input("gain")` reads inside
    /// a graph. `None` is every evaluation that is not a graph's, and an Input
    /// nobody overrode falls back to its own default in the document.
    ///
    /// Shared rather than owned: one list serves every property in the graph,
    /// and a context is cloned for each of them.
    pub inputs: Option<Arc<[(String, crate::model::EffectValue)]>>,
}

impl ExpressionContext {
    /// A context that offers nothing but `time` — for evaluations with no comp
    /// or layer behind them: a standalone preview of an expression, and the
    /// unit tests. The comp and layer constants are simply absent from the
    /// scope, so an expression that reads one fails visibly rather than quietly
    /// reading an invented number.
    pub fn detached() -> ExpressionContext {
        // The empty document is shared, not copied. This is called once per
        // context-less evaluation, and `Document` is the whole project.
        static EMPTY: OnceLock<Arc<Document>> = OnceLock::new();
        ExpressionContext {
            document: EMPTY.get_or_init(|| Arc::new(Document::new())).clone(),
            comp: None,
            layer: None,
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        }
    }

    /// The context an expression is running under, read back off the engine
    /// inside a `layer(…)` or `comp(…)` helper.
    ///
    /// Rhai's `clone_cast` **panics** when the tag is absent or of another
    /// type, and absent is an ordinary case: any evaluation that did not set
    /// one — a standalone preview, a half-typed expression in the graph
    /// editor — would take the whole engine down with it. Engine crates do not
    /// panic (14-ENGINEERING-RULES §4), so a missing context becomes the
    /// detached one and the helpers report an invalid reference as they
    /// already do for a name that matches no layer.
    pub(crate) fn from_call(context: &rhai::NativeCallContext) -> Arc<ExpressionContext> {
        context
            .engine()
            .default_tag()
            .clone()
            .try_cast::<Arc<ExpressionContext>>()
            .unwrap_or_else(|| Arc::new(ExpressionContext::detached()))
    }

    pub fn increase_depth(&self) -> ExpressionContext {
        ExpressionContext {
            document: self.document.clone(),
            comp: self.comp,
            layer: self.layer,
            comp_time: self.comp_time,
            current_depth: self.current_depth + 1,
            inputs: self.inputs.clone(),
        }
    }
}

fn make_engine() -> Engine {
    let mut engine = Engine::new();

    engine.set_max_operations(MAX_EXPRESSION_OPERATIONS);
    engine.set_max_string_size(MAX_EXPRESSION_STRING_BYTES);
    engine.set_max_array_size(MAX_EXPRESSION_ARRAY_ITEMS);
    engine.set_max_map_size(MAX_EXPRESSION_MAP_ITEMS);

    let math = exported_module!(math::math);
    let comp = exported_module!(comp::comp);
    let layer = exported_module!(layer::layers);

    engine.register_global_module(math.into());
    engine.register_global_module(comp.into());
    engine.register_global_module(layer.into());

    engine
}

thread_local! {
    /// Engines that are built but not currently in use, on this thread.
    ///
    /// Building an engine means registering three modules' worth of functions,
    /// which measures at roughly 370µs — about forty times the cost of running
    /// a typical expression, and enough that a few dozen driven properties
    /// would eat a whole 60fps frame on engine construction alone. So engines
    /// are kept and handed out again.
    ///
    /// A stack rather than a single shared engine because **expressions nest**:
    /// `layer("Sun").x` evaluates another property from inside an evaluation
    /// already in progress, and the inner one needs an engine of its own — the
    /// context it reads lives on the engine (`set_default_tag`), so the two
    /// cannot share one. The borrow is held only across the pop and the push,
    /// never across evaluation, so re-entry finds the cell free.
    ///
    /// Thread-local because the pool needs no locking that way, and evaluation
    /// is synchronous within whichever thread is drawing or keying a frame.
    static ENGINE_POOL: RefCell<Vec<Engine>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with an engine, borrowed from this thread's pool or built if the
/// pool is empty, and return the engine afterwards for the next evaluation.
///
/// An engine that panics its way out is simply not returned; the next call
/// builds a fresh one.
fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
    let mut engine = ENGINE_POOL
        .with(|pool| pool.borrow_mut().pop())
        .unwrap_or_else(make_engine);

    let result = f(&mut engine);

    // Drop the context this evaluation put on the engine, so a pooled engine
    // never carries one comp's document into another comp's evaluation.
    engine.set_default_tag(Dynamic::UNIT);
    ENGINE_POOL.with(|pool| pool.borrow_mut().push(engine));

    result
}

/// Run `expression` at `time` and hand back whatever it produced, untouched.
/// The typed wrappers below decide what to make of it.
///
/// `vars` are extra numbers the expression can read by name, which is how an
/// Expression box's inputs arrive. They go in first, so a name the context
/// also gives (`time`) keeps the context's meaning.
fn eval_dynamic(
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    vars: &[(&str, f64)],
) -> Result<Dynamic, Box<rhai::EvalAltResult>> {
    let mut scope = Scope::new();
    for (name, value) in vars {
        scope.push_constant(*name, *value);
    }

    if let Some(context) = context.as_ref() {
        if context.current_depth >= MAXIMUM_DEPTH {
            return Err(Box::new(rhai::EvalAltResult::ErrorSystem(
                "expression".into(),
                "expressions nest more than a hundred deep — most likely two \
                 properties refer to each other"
                    .into(),
            )));
        }

        apply_context_to_scope(&mut scope, context);
    }

    with_engine(|engine| {
        if let Some(context) = context {
            engine.set_default_tag(Dynamic::from(context));
        }

        engine.eval_expression_with_scope::<Dynamic>(&mut scope, expression)
    })
}

const MAXIMUM_DEPTH: u32 = 100;

thread_local! {
    /// JavaScript programs already read, by their text. `None` is a text that
    /// does not read, remembered so it is not tried again on every frame.
    ///
    /// Reading a text is most of the cost of running a short one, and the same
    /// few are run for every frame, so each is read once per thread.
    static PROGRAMS: RefCell<HashMap<String, Option<Rc<script::Program>>>> =
        RefCell::new(HashMap::new());
}

/// More texts than this in one thread's cache and it is emptied: somebody is
/// typing, and every keystroke is a new text.
const MAX_CACHED_PROGRAMS: usize = 512;

/// The JavaScript program for `expression`, read once per thread, or `None`
/// when the text does not read as JavaScript.
fn program_for(expression: &str) -> Option<Rc<script::Program>> {
    if let Some(found) = PROGRAMS.with(|programs| programs.borrow().get(expression).cloned()) {
        return found;
    }
    let program = script::compile(expression).ok().map(Rc::new);
    PROGRAMS.with(|programs| {
        let mut programs = programs.borrow_mut();
        if programs.len() >= MAX_CACHED_PROGRAMS {
            programs.clear();
        }
        programs.insert(expression.to_owned(), program.clone());
    });
    program
}

/// Run a JavaScript expression, or say why it did not run.
fn run_javascript(
    expression: &str,
    context: Option<&ExpressionContext>,
    slot: Slot,
    vars: &[(&str, f64)],
) -> Result<script::Answer, String> {
    let Some(program) = program_for(expression) else {
        // Read again for the sentence: the cache keeps the program, not why
        // there is none, and this is the road an editor takes, not a frame.
        return Err(script::compile(expression)
            .err()
            .unwrap_or_else(|| "the expression does not read".into()));
    };
    let detached;
    let context = match context {
        Some(context) => context,
        None => {
            detached = ExpressionContext::detached();
            &detached
        }
    };
    if context.current_depth >= MAXIMUM_DEPTH {
        return Err(
            "expressions nest more than a hundred deep — most likely two properties refer to \
             each other"
                .into(),
        );
    }
    script::run(&program, context, slot, vars)
}

/// A JavaScript expression's answer as the one number a property wants.
///
/// A list answers with the number this property is of it. An expression that
/// fails leaves the property at the value it had before the expression, where
/// that is known, which is what After Effects shows for a broken one.
fn javascript_number(expression: &str, context: Option<&ExpressionContext>, slot: Slot) -> f64 {
    let answer = match run_javascript(expression, context, slot, &[]) {
        Ok(script::Answer::Number(n)) => Some(n),
        Ok(script::Answer::List(list)) => list.get(usize::from(slot.axis)).copied(),
        _ => None,
    };
    answer
        .filter(|n| n.is_finite())
        .or(slot.own())
        .unwrap_or(-1.0)
}

/// The slot of the property on the context's layer that carries `expression`.
///
/// For the callers that hold an expression's text and not the property it
/// came off: the panels, which sample a row to show its number and its curve.
/// A point's two rows carry the same text, and the first is the one found.
fn slot_on_layer(context: &ExpressionContext, expression: &str) -> Slot {
    use crate::anim::{Animation, Property};
    use crate::model::EffectValue;

    let carries = |property: &Property| {
        matches!(&property.animation, Animation::Expression(e) if e == expression)
            && property.extra.contains_key(SLOT_KEY)
    };
    let layer = context.comp.zip(context.layer).and_then(|(comp, layer)| {
        let comp = context.document.comp(comp)?;
        comp.layers.iter().find(|l| l.id == layer)
    });
    let Some(layer) = layer else {
        return Slot::default();
    };
    let tr = &layer.transform;
    let transform = [
        &tr.anchor_x,
        &tr.anchor_y,
        &tr.position_x,
        &tr.position_y,
        &tr.scale_x,
        &tr.scale_y,
        &tr.rotation,
        &tr.opacity,
    ];
    let params = layer.effects.iter().flat_map(|effect| {
        effect.params.iter().flat_map(|param| {
            let parts: Vec<&Property> = match &param.value {
                EffectValue::Float(p) => vec![p],
                EffectValue::Point(x, y) => vec![x, y],
                EffectValue::Colour(c) => c.iter().collect(),
                _ => Vec::new(),
            };
            parts
        })
    });
    transform
        .into_iter()
        .chain(params)
        .find(|property| carries(property))
        .map_or_else(Slot::default, |property| Slot::read(&property.extra))
}

/// Whether this text is a Rhai expression Lumit can actually run — it parses,
/// and every name in it exists.
///
/// **In plain terms.** An expression imported from another application is
/// written in *that* application's language, and pasting it here would not
/// error in the user's face: it would quietly answer the same wrong number on
/// every frame. So the importer asks this first, and files away anything the
/// engine cannot run instead of letting it drive a property (docs/11 §3).
///
/// It is a compile *and* a trial run, because a name the language has never
/// heard of parses perfectly well and only fails when it is reached. The trial
/// runs against [`ExpressionContext::detached`], so `time` and the maths are
/// there but no comp and no layer are: an expression that reaches for a
/// neighbouring layer answers "no" here. That is the safe way round for an
/// import, where the keyframes underneath are still there to drive the
/// property either way.
pub fn is_runnable(expression: &str) -> bool {
    eval_dynamic(
        expression,
        Some(Arc::new(ExpressionContext::detached())),
        &[],
    )
    .is_ok()
}

/// Whether this text is a JavaScript expression Lumit can run: it reads as
/// JavaScript, and every name it uses is one Lumit provides.
///
/// The importer's question for an After Effects expression. There is no trial
/// run as [`is_runnable`] has, because nearly every one reads its layer and
/// there is none yet.
pub fn is_javascript(expression: &str) -> bool {
    program_for(expression).is_some_and(|program| script::names_known(&program))
}

/// Run a Rhai expression for the one number a property wants. `-1` when it
/// fails.
pub fn evaluate(expression: &str, context: Option<Arc<ExpressionContext>>) -> f64 {
    convert_result(eval_dynamic(expression, context, &[]))
}

/// [`evaluate`] in the language named, for the callers that hold an
/// expression's text and its language but not the property it came off: the
/// panels, sampling a row to show its number.
pub fn evaluate_in(
    language: Language,
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
) -> f64 {
    match language {
        Language::Rhai => evaluate(expression, context),
        Language::JavaScript => {
            let slot = context
                .as_deref()
                .map_or_else(Slot::default, |context| slot_on_layer(context, expression));
            javascript_number(expression, context.as_deref(), slot)
        }
    }
}

/// [`evaluate`] for a property that is in hand, whose `extra` says which
/// language its expression is in and what the expression may know about the
/// property: its value before the expression, and which number of a point it
/// is ([`Slot`]).
pub fn evaluate_property(
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    extra: &serde_json::Map<String, serde_json::Value>,
) -> f64 {
    match Language::of(extra) {
        Language::Rhai => evaluate(expression, context),
        Language::JavaScript => {
            javascript_number(expression, context.as_deref(), Slot::read(extra))
        }
    }
}

/// Evaluate an expression for its **words** rather than its number — what a
/// text layer whose content is expression-driven shows at `time`.
///
/// Every result type is welcome: a number prints as a number, a string as
/// itself. The point of the feature is putting a value on screen, and refusing
/// a type would only mean the user has to wrap it in a conversion.
///
/// A broken expression prints nothing rather than failing the frame — these are
/// typed against a live preview, where a half-written expression is invalid for
/// most of the time it takes to write it.
pub fn evaluate_text(expression: &str, context: Option<Arc<ExpressionContext>>) -> String {
    match eval_dynamic(expression, context, &[]) {
        Ok(val) => val.to_string(),
        Err(_) => String::new(),
    }
}

/// Sample an expression across a span of time — what the graph editor draws as
/// a curve.
///
/// The expression is compiled once and then run per sample, which is the whole
/// reason this exists separately from calling [`evaluate`] in a loop.
///
/// An expression that does not compile yields **no samples at all**, not a row
/// of zeroes: the graph has nothing truthful to draw for a line that is still
/// being typed, and a flat curve at zero would read as a real answer.
pub fn evaluate_range(
    language: Language,
    expression: &str,
    context: Option<&ExpressionContext>,
    start: f64,
    end: f64,
    samples: i64,
) -> Vec<f64> {
    if language == Language::JavaScript {
        if program_for(expression).is_none() {
            return Vec::new();
        }
        let slot = context.map_or_else(Slot::default, |context| slot_on_layer(context, expression));
        let delta = (end - start) / (samples as f64);
        return (0..samples)
            .map(|i| {
                let mut at = context.cloned().unwrap_or_else(ExpressionContext::detached);
                at.comp_time = start + (delta * (i as f64));
                javascript_number(expression, Some(&at), slot)
            })
            .collect();
    }
    with_engine(|engine| {
        let Ok(ast) = engine.compile_expression(expression) else {
            return Vec::new();
        };

        let delta = (end - start) / (samples as f64);
        (0..samples)
            .map(|i| {
                let mut scope = Scope::new();
                let mut ctx = context.cloned();

                if let Some(ctx) = ctx.as_mut() {
                    ctx.comp_time = start + (delta * (i as f64));
                    apply_context_to_scope(&mut scope, ctx);
                }

                // The layer and comp helpers read the context off the engine,
                // so a sampled expression that calls one needs it there too.
                engine.set_default_tag(match ctx {
                    Some(ctx) => Dynamic::from(Arc::new(ctx)),
                    None => Dynamic::UNIT,
                });

                convert_result(engine.eval_ast_with_scope::<Dynamic>(&mut scope, &ast))
            })
            .collect()
    })
}

/// What an expression answered with, when it answered with something a wire
/// can carry.
///
/// **In plain terms.** A property expression is asked for one number and gets
/// one number - anything else, including the error, becomes `-1`. A driver
/// wants more than that: the same expression that returns `50` could return a
/// pair for a position or four numbers for a colour, and a driver that turned
/// a mistake into `-1` would drive whatever it is wired to with a plausible
/// wrong answer rather than saying it could not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExprValue {
    /// One number: a float, a whole number, or a boolean read as 1 and 0.
    Number(f64),
    /// Two numbers - a position, a scale, an anchor.
    Point(f64, f64),
    /// Three or four - a colour, alpha defaulting to opaque.
    Colour([f32; 4]),
}

/// Run a Rhai `expression` and read what it answered as a value, or say why
/// not.
///
/// The failing sibling of [`evaluate`]: every refusal comes back as a sentence
/// rather than as a number that looks like an answer. Rhai's own message is
/// kept whole - it names the line and the thing it could not find, which is
/// what an editor needs to show.
///
/// `vars` are extra numbers in scope by name: an Expression box's inputs.
pub fn evaluate_value(
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    vars: &[(&str, f64)],
) -> Result<ExprValue, String> {
    evaluate_value_in(Language::Rhai, expression, context, vars)
}

/// [`evaluate_value`] in the language named.
pub fn evaluate_value_in(
    language: Language,
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    vars: &[(&str, f64)],
) -> Result<ExprValue, String> {
    if language == Language::JavaScript {
        return match run_javascript(expression, context.as_deref(), Slot::default(), vars)? {
            script::Answer::Number(n) => Ok(ExprValue::Number(n)),
            script::Answer::List(list) => list_value(&list),
            script::Answer::Text(_) => Err("a string - not a number, a point or a colour".into()),
        };
    }
    let value = eval_dynamic(expression, context, vars).map_err(|e| e.to_string())?;
    if let Some(n) = as_f64(value.clone()) {
        return Ok(ExprValue::Number(n));
    }
    // An array is how both of the other two arrive: `[x, y]` for a point,
    // `[r, g, b]` or `[r, g, b, a]` for a colour. Read element by element so a
    // list of the right length holding something that is not a number is a
    // refusal rather than a zero.
    if let Some(items) = value.clone().try_cast::<rhai::Array>() {
        let mut numbers = Vec::with_capacity(items.len());
        for item in items {
            let Some(n) = as_f64(item) else {
                return Err("an array holding something that is not a number".into());
            };
            numbers.push(n);
        }
        return list_value(&numbers);
    }
    Err(format!(
        "a {} - not a number, a point or a colour",
        value.type_name()
    ))
}

/// A list of numbers as the value its length makes it.
fn list_value(numbers: &[f64]) -> Result<ExprValue, String> {
    match *numbers {
        [x, y] => Ok(ExprValue::Point(x, y)),
        [r, g, b] => Ok(ExprValue::Colour([r as f32, g as f32, b as f32, 1.0])),
        [r, g, b, a] => Ok(ExprValue::Colour([r as f32, g as f32, b as f32, a as f32])),
        _ => Err(format!(
            "an array of {} numbers: two make a point, three or four a colour",
            numbers.len()
        )),
    }
}

fn convert_result(result: Result<Dynamic, Box<rhai::EvalAltResult>>) -> f64 {
    let Ok(val) = result else { return -1.0 };
    as_f64(val).unwrap_or(-1.0)
}

/// A Rhai value read as a number, if it is one. Rhai keeps whole numbers and
/// fractions as separate types, so `2` and `2.0` arrive differently and both
/// have to be accepted.
fn as_f64(val: Dynamic) -> Option<f64> {
    if val.is_float() {
        return val.as_float().ok();
    }
    if val.is_int() {
        return val.as_int().ok().map(|v| v as f64);
    }
    if val.is_bool() {
        return val.as_bool().ok().map(|v| if v { 1.0 } else { 0.0 });
    }
    None
}

pub fn get_api_metadata() -> String {
    with_engine(|engine| engine.gen_fn_metadata_to_json(false).unwrap_or_default())
}

fn apply_context_to_scope(scope: &mut Scope<'_>, context: &ExpressionContext) {
    // `time` comes off the context itself and is pushed unconditionally. It is
    // the one constant that does not depend on finding a comp in the document,
    // and every expression that animates reads it — scoping it to a successful
    // comp lookup silently turns `time * 2` into an error, which resolves to
    // nothing, which keys every frame the same and freezes the picture.
    scope.push_constant("time", context.comp_time);

    let doc = &context.document;

    if let Some(comp_id) = context.comp {
        if let Some(comp) = doc.comp(comp_id) {
            scope.push_constant("comp_height", comp.height as i64);
            scope.push_constant("comp_width", comp.width as i64);
            scope.push_constant("comp_fps", comp.frame_rate.fps() as i64);
            scope.push_constant("num_markers", comp.markers.len() as i64);
            scope.push_constant("num_layers", comp.layers.len() as i64);

            if let Some(layer_id) = context.layer {
                if let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) {
                    scope.push_constant("cut_in", layer.in_point.0.to_f64());
                    scope.push_constant("cut_out", layer.out_point.0.to_f64());
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::{LinearColour, TextDocument};

    fn document(expression: Option<&str>) -> TextDocument {
        TextDocument {
            text: "typed".into(),
            expression: expression.map(str::to_owned),
            size: 48.0,
            fill: LinearColour([1.0, 1.0, 1.0, 1.0]),
            path: None,
            path_offset: crate::anim::Property::zero(),
            animators: Vec::new(),
            style: Default::default(),
            paragraph: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn at(time: f64) -> Arc<ExpressionContext> {
        let mut context = ExpressionContext::detached();
        context.comp_time = time;
        Arc::new(context)
    }

    /// The point of the feature: a number reaches the screen as words.
    #[test]
    fn a_number_prints_as_words() {
        assert_eq!(evaluate_text("1 + 1", Some(at(0.0))), "2");
        assert_eq!(evaluate_text("time", Some(at(3.0))), "3.0");
        assert_eq!(evaluate_text("\"frame \" + 7", Some(at(0.0))), "frame 7");
    }

    /// A document written before expressions existed loads with none, and a
    /// document with one round-trips.
    #[test]
    fn the_field_is_optional_on_disk() {
        let old = r#"{"text":"hi","size":12.0,"fill":[1.0,1.0,1.0,1.0]}"#;
        let d: TextDocument = serde_json::from_str(old).unwrap();
        assert_eq!(d.expression, None);
        // Absent rather than null, so an untouched project file does not grow.
        assert!(!serde_json::to_string(&d).unwrap().contains("expression"));

        let driven = document(Some("time"));
        let json = serde_json::to_string(&driven).unwrap();
        assert_eq!(serde_json::from_str::<TextDocument>(&json).unwrap(), driven);
    }

    /// An engine handed back to the pool must not carry the last evaluation's
    /// context into the next one. Reusing engines is only safe because the tag
    /// is cleared on the way back.
    #[test]
    fn a_pooled_engine_does_not_leak_its_context() {
        assert_eq!(evaluate("time", Some(at(7.0))), 7.0);
        // No context at all: `time` must be unknown again, not still 7.
        assert_eq!(evaluate_text("time", None), "");
    }

    /// A comp holding `layers`, filed in a document — the least scaffolding an
    /// expression that refers to another layer can be evaluated against.
    fn doc_with(layers: Vec<crate::model::Layer>) -> (Arc<Document>, Uuid) {
        use crate::model::{Composition, ProjectItem};
        use crate::time::{Duration, FrameRate, Rational};

        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: crate::model::LinearColour::BLACK,
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let id = comp.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp));
        (Arc::new(doc), id)
    }

    /// A solid layer named `name` whose x position is driven by `expression`.
    fn driven_layer(name: &str, expression: &str) -> crate::model::Layer {
        use crate::anim::Animation;
        use crate::model::{LayerKind, Switches, TransformGroup};
        use crate::time::{CompTime, Rational};

        let at = |s: i64| CompTime(Rational::new(s, 1).unwrap());
        let mut transform = TransformGroup::default();
        transform.position_x.animation = Animation::Expression(expression.into());

        crate::model::Layer {
            graph: Default::default(),
            id: Uuid::now_v7(),
            name: name.into(),
            kind: LayerKind::Solid {
                def: Uuid::now_v7(),
            },
            in_point: at(0),
            out_point: at(10),
            start_offset: at(0),
            transform,
            matte: None,
            parent: None,
            label: 0,
            volume_db: crate::anim::Property::zero(),
            pan: crate::anim::Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            markers: Vec::new(),
            paint: Default::default(),
            puppet: None,
            extra: serde_json::Map::new(),
        }
    }

    /// A node graph's Inputs are readable by name inside the graph
    /// (docs/impl/node-graph-comp.md §5.5): the host's own value where it
    /// handed one over, the Input's declared default otherwise, a colour's
    /// first channel, and the miss value for a picture or a name the graph
    /// does not have.
    #[test]
    fn an_expression_reads_a_graphs_input_by_name() {
        use crate::comp_graph::{CompGraph, GraphInput, GraphNode, InputKind};
        use crate::model::{EffectValue, ProjectItem};

        let declared = |id: &str, kind: InputKind, first: f64| GraphNode::Input {
            id: Uuid::now_v7(),
            input: GraphInput {
                id: id.to_owned(),
                label: format!("The {id}"),
                kind,
                default: [first, 0.25, 0.5, 1.0],
                min: 0.0,
                max: 100.0,
                unit: crate::fx::Unit::Raw,
                preview: None,
            },
        };
        let graph = CompGraph {
            nodes: vec![
                declared("gain", InputKind::Number, 7.0),
                declared("tint", InputKind::Colour, 0.75),
                declared("plate", InputKind::Picture, 0.0),
                GraphNode::Output { id: Uuid::now_v7() },
            ],
            edges: Vec::new(),
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        };
        let (document, comp) = doc_with(Vec::new());
        let mut doc = (*document).clone();
        if let Some(ProjectItem::Composition(c)) = doc.item_mut(comp) {
            c.graph = Some(graph);
        }
        let context = |values: Option<Vec<(String, EffectValue)>>| {
            Arc::new(ExpressionContext {
                document: Arc::new(doc.clone()),
                comp: Some(comp),
                layer: None,
                comp_time: 2.0,
                current_depth: 0,
                inputs: values.map(Into::into),
            })
        };

        // Nobody handed a value over: the Input's own default answers, by id
        // or by the word on its row.
        assert_eq!(evaluate("input(\"gain\")", Some(context(None))), 7.0);
        assert_eq!(evaluate("input(\"The gain\")", Some(context(None))), 7.0);
        // A colour answers its first channel, which is the number an
        // expression on a number row can use.
        assert_eq!(evaluate("input(\"tint\")", Some(context(None))), 0.75);
        // A picture carries a texture, and an unknown name is a miss: both
        // read as the module's miss value.
        assert_eq!(evaluate("input(\"plate\")", Some(context(None))), -1.0);
        assert_eq!(evaluate("input(\"nothing\")", Some(context(None))), -1.0);

        // A host's own value comes first, and it is read at the graph's time,
        // so a keyed row moves the answer.
        let mut keyed = crate::anim::Property::fixed(0.0);
        keyed.animation = crate::anim::Animation::Expression("time * 5.0".into());
        let over = vec![("gain".to_owned(), EffectValue::Float(keyed))];
        assert_eq!(
            evaluate("input(\"gain\") + 1.0", Some(context(Some(over)))),
            11.0,
            "the host's value at the graph's own time"
        );

        // The depth guard holds: a value that names the Input it is standing
        // in for gives up rather than spinning.
        let mut itself = crate::anim::Property::fixed(0.0);
        itself.animation = crate::anim::Animation::Expression("input(\"gain\")".into());
        let loop_over = vec![("gain".to_owned(), EffectValue::Float(itself))];
        let _ = evaluate("input(\"gain\")", Some(context(Some(loop_over))));
    }

    /// Expressions nest — a property may read another property that is itself
    /// an expression — so evaluation has to survive being re-entered while an
    /// engine is already checked out of the pool. This is the test that fails
    /// if engines are shared rather than pooled.
    #[test]
    fn evaluation_can_re_enter_itself() {
        let driven = driven_layer("Driven", "time * 3.0");
        let driven_id = driven.id;
        let (document, comp) = doc_with(vec![driven]);

        let context = Arc::new(ExpressionContext {
            document,
            comp: Some(comp),
            layer: Some(driven_id),
            comp_time: 2.0,
            current_depth: 0,
            inputs: None,
        });
        assert_eq!(evaluate("layer(\"Driven\").x + 1.0", Some(context)), 7.0);
    }

    /// Two properties that read each other must give up rather than recurse
    /// until the stack runs out.
    #[test]
    fn a_cycle_stops_instead_of_overflowing() {
        let a = driven_layer("A", "layer(\"B\").x");
        let a_id = a.id;
        let (document, comp) = doc_with(vec![a, driven_layer("B", "layer(\"A\").x")]);

        let context = Arc::new(ExpressionContext {
            document,
            comp: Some(comp),
            layer: Some(a_id),
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        });
        // The value is meaningless; returning at all is the point.
        let _ = evaluate("layer(\"A\").x", Some(context));
    }

    /// **One expression cannot build something enormous.**
    ///
    /// Loops do not parse in an expression, so the danger is never a long run.
    /// It is one call that allocates: `blob(50_000_000)` and padding an array to
    /// five million items both went through untouched before the ceilings, on
    /// every property read. A refusal comes back as `-1` like any other.
    #[test]
    fn an_expression_cannot_build_something_enormous() {
        assert_eq!(evaluate("1 + 2 * 3", None), 7.0);
        assert_eq!(evaluate("if 2 > 1 { 9 } else { 0 }", None), 9.0);
        assert_eq!(evaluate("[1, 2].len()", None), 2.0);
        assert_eq!(evaluate_text("\"frame \" + 7", None), "frame 7");

        assert_eq!(evaluate("blob(50_000_000).len()", None), -1.0);
        assert_eq!(evaluate("[0].pad(5_000_000, 0).len()", None), -1.0);
        assert_eq!(evaluate("\"x\".pad(5_000_000, 'y').len()", None), -1.0);

        let string = |len| format!("\"{}\".len()", "x".repeat(len));
        assert_eq!(
            evaluate(&string(MAX_EXPRESSION_STRING_BYTES), None),
            MAX_EXPRESSION_STRING_BYTES as f64
        );
        assert_eq!(
            evaluate(&string(MAX_EXPRESSION_STRING_BYTES + 1), None),
            -1.0
        );

        // Rhai's array literal refuses *at* the ceiling rather than above it,
        // one item stricter than the same array built by `pad`.
        let array = |len| format!("[{}].len()", vec!["0"; len].join(","));
        assert_eq!(
            evaluate(&array(MAX_EXPRESSION_ARRAY_ITEMS - 1), None),
            (MAX_EXPRESSION_ARRAY_ITEMS - 1) as f64
        );
        assert_eq!(evaluate(&array(MAX_EXPRESSION_ARRAY_ITEMS), None), -1.0);

        let map = |len| {
            format!(
                "#{{{}}}.len()",
                (0..len)
                    .map(|n| format!("key_{n}: 0"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        assert_eq!(
            evaluate(&map(MAX_EXPRESSION_MAP_ITEMS), None),
            MAX_EXPRESSION_MAP_ITEMS as f64
        );
        assert_eq!(evaluate(&map(MAX_EXPRESSION_MAP_ITEMS + 1), None), -1.0);

        // An engine that refused one expression still answers the next, and the
        // operation count resets per evaluation. Thirty of these cost more than
        // one budget between them, so a count that carried over would refuse.
        let heavy = array(5_000);
        for _ in 0..30 {
            assert_eq!(evaluate(&heavy, None), 5_000.0);
        }
    }

    /// A layer carrying what an After Effects shake preset arrives as: a set
    /// of controls with a keyed Amount, for expressions on other rows to read.
    /// Its rotation holds 30 under an expression of its own.
    fn rigged() -> (Arc<Document>, Uuid, Uuid) {
        use crate::anim::{Animation, Keyframe, Property, SideInterp};
        use crate::model::{EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue};
        use crate::time::{CompTime, Rational};

        let key = |seconds: i64, value: f64| Keyframe {
            time: Rational::new(seconds, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let named = |id: &str, name: &str, property: Property| EffectParam {
            id: id.into(),
            value: EffectValue::Float(property),
            extra: [("name".to_owned(), serde_json::json!(name))]
                .into_iter()
                .collect(),
        };
        let mut amount = Property::fixed(0.0);
        amount.animation = Animation::Keyframed(vec![key(1, 0.0), key(2, 40.0), key(4, 0.0)]);

        let mut layer = driven_layer("Rig", "0");
        layer.transform.position_x = Property::fixed(960.0);
        layer.transform.rotation.animation = Animation::Expression("value + 5".into());
        Slot::new(&[30.0], 0, 1)
            .in_language(Language::JavaScript)
            .write(&mut layer.transform.rotation.extra);
        layer.in_point = CompTime(Rational::new(1, 2).unwrap());
        layer.effects.push(EffectInstance {
            id: Uuid::now_v7(),
            effect: EffectKey {
                namespace: EffectNamespace::Placeholder,
                match_name: "Pseudo/1".into(),
                version: 0,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: vec![
                named("p1", "Amount", amount),
                named("p2", "Seed", Property::fixed(3.0)),
            ],
            sample_temporally: true,
            roto: None,
            custom_name: Some("Shake".into()),
            linked_pairs: Vec::new(),
            plugin_state: None,
            extra: serde_json::Map::new(),
        });
        let id = layer.id;
        let (document, comp) = doc_with(vec![layer]);
        (document, comp, id)
    }

    /// The context an expression on the rigged layer runs under at `time`.
    fn on_rig(rig: &(Arc<Document>, Uuid, Uuid), time: f64) -> Arc<ExpressionContext> {
        Arc::new(ExpressionContext {
            document: rig.0.clone(),
            comp: Some(rig.1),
            layer: Some(rig.2),
            comp_time: time,
            current_depth: 0,
            inputs: None,
        })
    }

    /// **An After Effects expression reads what After Effects lets it read.**
    ///
    /// The effect and its rows by name, the keyframes under a row, the comp
    /// and the layer. This is the whole reason the second language exists: a
    /// rig built there has to find its controls here.
    #[test]
    fn an_after_effects_expression_reads_its_layer() {
        let rig = rigged();
        let number = |source: &str, time: f64| {
            evaluate_in(Language::JavaScript, source, Some(on_rig(&rig, time)))
        };

        assert_eq!(number("effect(\"Shake\")(\"Amount\") / 2", 1.5), 10.0);
        assert_eq!(
            number(
                "var c = effect('Shake')\n('Seed'); Math.floor(c) + effect(1)(2)",
                0.0
            ),
            6.0
        );
        assert_eq!(
            number("thisLayer.effect(1).param('amount').valueAtTime(2)", 0.0),
            40.0
        );
        assert_eq!(
            number(
                "effect('Shake').name.length + (effect('Shake').active ? 1 : 0)",
                0.0
            ),
            6.0
        );

        // Keyframes are counted from one and timed on the comp's clock.
        let keys = "var p = effect('Shake')('Amount'); var last = p.key(p.numKeys); \
                    p.numKeys * 100 + last.time * 10 + p.nearestKey(1.9).index";
        assert_eq!(number(keys, 0.0), 342.0);
        let rate = number("effect('Shake')('Amount').velocityAtTime(1.5)", 0.0);
        assert!((rate - 40.0).abs() < 1e-6, "{rate}");

        // A row's keys can be made to repeat past their end, or before their start.
        let looped = |kind: &str, time: f64| {
            number(&format!("effect('Shake')('Amount').{kind}"), time).round()
        };
        assert_eq!(looped("loopOut()", 3.0), 20.0);
        assert_eq!(looped("loopOut('cycle')", 5.0), 40.0);
        assert_eq!(looped("loopOut('pingpong')", 5.0), 20.0);
        assert_eq!(looped("loopOut('offset')", 5.0), 40.0);
        assert_eq!(looped("loopOut('continue')", 5.0), -20.0);
        assert_eq!(looped("loopOut('cycle', 1)", 5.0), 20.0);
        assert_eq!(looped("loopOutDuration('cycle', 1)", 4.5), 10.0);
        assert_eq!(looped("loopIn('cycle')", 0.0), 20.0);

        assert_eq!(
            number("thisComp.width / 2 + thisComp.height + index", 0.0),
            2041.0
        );
        assert_eq!(
            number("Math.round((inPoint + thisComp.frameDuration) * 60)", 0.0),
            31.0
        );
        assert_eq!(
            number(
                "thisComp.layer('Rig').transform.position[0] + thisLayer.name.length",
                0.0
            ),
            963.0
        );
        assert_eq!(
            number(
                "comp('c').layer(1).position.length + thisComp.numLayers",
                0.0
            ),
            3.0
        );
        assert_eq!(number("timeToFrames(1.5) + framesToTime(30)", 0.0), 90.5);
        assert_eq!(number("posterizeTime(2); time", 1.7), 1.5);
        assert_eq!(number("hasParent ? 1 : (parent == null ? 2 : 3)", 0.0), 2.0);
    }

    /// **`value` is what the property held before its expression.**
    ///
    /// A position is two properties here and one in After Effects, so both
    /// carry the same text and each takes its own number of the answer. The
    /// same slot is what `wiggle` wanders around, and what a broken expression
    /// falls back to.
    #[test]
    fn value_is_what_the_property_held_before_its_expression() {
        use crate::anim::{Animation, Property};

        let rig = rigged();
        let driven = |source: &str, slot: Slot, time: f64| {
            let mut property = Property::fixed(0.0);
            property.animation = Animation::Expression(source.into());
            slot.in_language(Language::JavaScript)
                .write(&mut property.extra);
            property.value_at_with_context(0.0, on_rig(&rig, time))
        };

        let shaken = "value + [effect('Shake')('Amount'), 1]";
        assert_eq!(
            driven(shaken, Slot::new(&[960.0, 540.0], 0, 5), 2.0),
            1000.0
        );
        assert_eq!(driven(shaken, Slot::new(&[960.0, 540.0], 1, 5), 2.0), 541.0);
        assert_eq!(
            driven(
                "value.length > 1 ? value[1] : value",
                Slot::new(&[7.0], 0, 0),
                0.0
            ),
            7.0
        );

        // A wiggle stays within its amount, answers the same every time it
        // is asked, moves, and is another property's on another seed.
        let wiggle =
            |seed: u32, time: f64| driven("wiggle(2, 30)", Slot::new(&[100.0], 0, seed), time);
        let a = wiggle(1, 0.3);
        assert_eq!(a, wiggle(1, 0.3));
        assert!((a - 100.0).abs() <= 30.0, "{a}");
        assert_ne!(a, wiggle(1, 0.8));
        assert_ne!(a, wiggle(2, 0.3));
        // Seeding it by hand moves it too, and both numbers of a point wander
        // on their own.
        assert_ne!(
            driven(
                "seedRandom(8, true); wiggle(2, 30)",
                Slot::new(&[100.0], 0, 1),
                0.3
            ),
            a
        );
        let pair = "var w = wiggle(2, 30, 2) - value; w[0] - w[1]";
        assert_ne!(driven(pair, Slot::new(&[5.0, 5.0], 0, 1), 0.3), 0.0);
        assert_eq!(
            driven("wiggle(0, 0)", Slot::new(&[100.0], 0, 1), 0.3),
            100.0
        );

        // A broken expression leaves the property where it was.
        assert_eq!(
            driven(
                "value + effect('Gone')('Amount')",
                Slot::new(&[12.0], 0, 0),
                0.0
            ),
            12.0
        );
        // With nothing recorded there is nothing to fall back to.
        assert_eq!(
            driven("effect('Gone')('Amount')", Slot::default(), 0.0),
            -1.0
        );

        // Putting an expression on a property by hand writes down what it
        // held, an edit of the text keeps that, and taking it off forgets it.
        let mut typed = Property::fixed(64.0);
        typed.set_animation(Animation::Expression("64".into()));
        assert_eq!(typed.expression_language(), Language::Rhai);
        typed.set_expression_language(Language::JavaScript);
        typed.set_animation(Animation::Expression("value / 2 + wiggle(0, 0)".into()));
        assert_eq!(typed.expression_language(), Language::JavaScript);
        assert_eq!(typed.value_at_with_context(0.0, on_rig(&rig, 0.0)), 96.0);
        typed.set_animation(Animation::Static(1.0));
        assert!(typed.extra.is_empty());

        // A panel holds the text and not the property, and still reads the
        // slot: the rig's rotation is 30 under `value + 5`.
        let js = Language::JavaScript;
        assert_eq!(evaluate_in(js, "value + 5", Some(on_rig(&rig, 0.0))), 35.0);
        assert_eq!(
            evaluate_range(js, "value + 5", Some(&*on_rig(&rig, 0.0)), 0.0, 1.0, 3),
            [35.0, 35.0, 35.0]
        );
        // A line still being typed has no curve to draw, in either language.
        assert!(evaluate_range(js, "value +", None, 0.0, 1.0, 3).is_empty());
        assert!(evaluate_range(Language::Rhai, "time +", None, 0.0, 1.0, 3).is_empty());
    }

    /// **An expression is run by the language it was given, never by a guess.**
    ///
    /// The same five characters mean two things: Rhai divides whole numbers as
    /// whole numbers and JavaScript does not. So the language is a choice
    /// stored beside the text, and a property with nothing stored is Rhai,
    /// which is every expression a project already holds.
    #[test]
    fn an_expression_is_run_by_the_language_it_was_given() {
        use crate::anim::{Animation, Property};

        let js = Language::JavaScript;
        assert_eq!(evaluate("7 / 2", None), 3.0);
        assert_eq!(evaluate_in(Language::Rhai, "7 / 2", None), 3.0);
        assert_eq!(evaluate_in(js, "7 / 2", None), 3.5);

        let mut property = Property::fixed(0.0);
        property.set_animation(Animation::Expression("7 / 2".into()));
        assert_eq!(property.value_at(0.0), 3.0, "nothing stored is Rhai");
        property.set_expression_language(js);
        assert_eq!(property.value_at(0.0), 3.5);
        property.set_expression_language(Language::Rhai);
        assert_eq!(property.value_at(0.0), 3.0);
        assert_eq!(Language::from_id("something newer"), Language::Rhai);
        assert_eq!(Language::from_id(js.id()), js);

        // What the importer asks of an After Effects expression: does it read
        // as JavaScript, with every name one Lumit has.
        for known in [
            "Math.sin(time)",
            "wiggle(5, 20)",
            "x = 7 / 2; x",
            "[thisComp.width / 2, thisComp.height / 2]",
            "effect(\"Shake\")(\"Seed\")",
            "input_1 * 2",
        ] {
            assert!(is_javascript(known), "{known}");
        }
        for refused in ["nonesuch(time)", "if 2 > 1 { 9 } else { 0 }", "1 +"] {
            assert!(!is_javascript(refused), "{refused}");
        }
        assert!(is_runnable("time * 90") && !is_runnable("Math.sin(time)"));

        // The typed entry points answer in the language named.
        let value = |source: &str, vars: &[(&str, f64)]| evaluate_value_in(js, source, None, vars);
        assert_eq!(
            value("[Math.abs(-3), 4]", &[]),
            Ok(ExprValue::Point(3.0, 4.0))
        );
        assert_eq!(
            value("input_1 + Math.PI * 0", &[("input_1", 2.0)]),
            Ok(ExprValue::Number(2.0))
        );
        assert_eq!(
            value("[Math.abs(1), 0, 0]", &[]),
            Ok(ExprValue::Colour([1.0, 0.0, 0.0, 1.0]))
        );
        assert!(value("'words' + Math.PI", &[]).is_err());
        assert!(value("Math.nothing(1)", &[]).is_err());
        assert!(value("1 +", &[]).is_err_and(|why| why.contains("line 1")));
        assert!(
            evaluate_value("Math.abs(-3)", None, &[]).is_err(),
            "Rhai has no Math"
        );
    }

    /// **A value expression says what it answered with, or why it did not.**
    ///
    /// [`evaluate`] turns every refusal into `-1`, which is the right bargain
    /// for a property that must produce a number for every frame and the wrong
    /// one for anything that has to report. Each arm here was a plausible wrong
    /// answer under the old road.
    #[test]
    fn a_value_expression_answers_with_its_type_or_with_a_reason() {
        let value = |src: &str| evaluate_value(src, None, &[]);

        // Rhai keeps whole numbers and fractions apart, and a boolean is a
        // number here as it is everywhere else in this file.
        assert_eq!(value("2.5"), Ok(ExprValue::Number(2.5)));
        assert_eq!(value("2"), Ok(ExprValue::Number(2.0)));
        assert_eq!(value("true"), Ok(ExprValue::Number(1.0)));

        assert_eq!(value("[100, 200.5]"), Ok(ExprValue::Point(100.0, 200.5)));

        // Three numbers is a colour at full alpha; four states its own.
        assert_eq!(
            value("[1.0, 0.5, 0.0]"),
            Ok(ExprValue::Colour([1.0, 0.5, 0.0, 1.0]))
        );
        assert_eq!(
            value("[1.0, 0.5, 0.0, 0.25]"),
            Ok(ExprValue::Colour([1.0, 0.5, 0.0, 0.25]))
        );

        // The refusals, each of which `evaluate` would have called -1.
        assert!(value("this is not (").is_err(), "it does not parse");
        assert!(value("nonesuch()").is_err(), "the name does not exist");
        assert!(value("\"words\"").is_err(), "a string is not a value");
        assert!(
            value("[1, 2, 3, 4, 5]").is_err(),
            "five of anything is nothing"
        );
        assert!(value("[1, \"two\"]").is_err(), "a pair holding a word");
    }
}

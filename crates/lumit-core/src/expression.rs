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

use crate::model::Document;
use rhai::{exported_module, Dynamic, Engine, Scope};
use uuid::Uuid;

mod comp;
mod layer;
mod math;
mod point_row;
pub mod script;
pub(crate) mod space;

use script::{Language, Slot, SLOT_KEY};

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

    // An expression is a sum, not a program. Out of the box Rhai lets `eval`
    // run any text as a whole script, and a script can `import` a file by its
    // path, a network path included, and print to the console. An expression
    // comes in with a project or an import, so none of that is left on.
    engine.set_max_modules(0);
    engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
    engine.disable_symbol("eval");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});

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

    // A Rhai expression shares the JavaScript budget, so what it asks of the
    // engine (`to_comp` and the like) is counted as well.
    let _running = script::interp::Running::begin();
    with_engine(|engine| {
        if let Some(context) = context {
            engine.set_default_tag(Dynamic::from(context));
        }

        engine.eval_expression_with_scope::<Dynamic>(&mut scope, expression)
    })
}

const MAXIMUM_DEPTH: u32 = 100;

/// A JavaScript text read into a program, or why it does not read.
type Read = Result<Rc<script::parse::Program>, Rc<str>>;

thread_local! {
    /// JavaScript programs already read, by their text. A text that does not
    /// read is remembered with why, so it is not tried again on every frame.
    ///
    /// Reading a text is most of the cost of running a short one, and the same
    /// few are run for every frame, so each is read once per thread.
    static PROGRAMS: RefCell<HashMap<String, Read>> = RefCell::new(HashMap::new());
}

/// More texts than this in one thread's cache and it is emptied: somebody is
/// typing, and every keystroke is a new text.
const MAX_CACHED_PROGRAMS: usize = 512;

/// The JavaScript program for `expression`, read once per thread, or why the
/// text does not read as JavaScript.
fn program_for(expression: &str) -> Read {
    if let Some(found) = PROGRAMS.with(|programs| programs.borrow().get(expression).cloned()) {
        return found;
    }
    let program = script::compile(expression)
        .map(Rc::new)
        .map_err(|why| Rc::from(why.as_str()));
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
    let program = program_for(expression).map_err(|why| why.to_string())?;
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
pub fn slot_on_layer(context: &ExpressionContext, expression: &str) -> Slot {
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
    let camera = match &layer.kind {
        crate::model::LayerKind::Camera { zoom, options, .. } => {
            let [x, y, z] = &options.point_of_interest;
            vec![x, y, z, zoom]
        }
        _ => Vec::new(),
    };
    let transform = [
        &tr.anchor_x,
        &tr.anchor_y,
        &tr.position_x,
        &tr.position_y,
        &tr.position_z,
        &tr.scale_x,
        &tr.scale_y,
        &tr.rotation,
        &tr.rotation_x,
        &tr.rotation_y,
        &tr.opacity,
    ]
    .into_iter()
    .chain(camera);
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
        .chain(params)
        .find(|property| carries(property))
        .map_or_else(Slot::default, |property| Slot::read(&property.extra))
}

/// A row an expression can sit on: a transform property, or an effect
/// parameter by the effect's id and its own. An effect's point is the pair
/// of rows its schema names `<stem>_x` and `<stem>_y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    Transform(crate::model::TransformProp),
    Effect { effect: Uuid, param: &'a str },
}

/// What `value` reads on a property: the number under its expression, or
/// what it holds at nought.
#[must_use]
pub fn underlying_value(property: &crate::anim::Property) -> f64 {
    match &property.animation {
        crate::anim::Animation::Expression(_) => Slot::read(&property.extra).own().unwrap_or(0.0),
        _ => property.value_at(0.0),
    }
}

/// Note on `property`, whose expression was only just put on, the point it
/// is number `axis` of, as an import does. `value` then reads the point, and
/// a list the expression answers with gives this row its own number. A row
/// shown on its own passes only its own number, and `value` is that.
pub fn note_point(property: &mut crate::anim::Property, point: &[f64], axis: usize) {
    if !matches!(property.animation, crate::anim::Animation::Expression(_))
        || axis == 0 && point.len() < 2
    {
        return;
    }
    let kept = Slot::read(&property.extra);
    Slot::new(point, axis, kept.seed)
        .in_language(kept.language)
        .write(&mut property.extra);
}

/// [`note_point`] for the row `prop` of `layer`, given that row's new state.
pub fn note_transform_point(
    layer: &crate::model::Layer,
    prop: crate::model::TransformProp,
    next: &mut crate::anim::Property,
) {
    let (rows, axis) = layer.point_rows(prop);
    if layer.separated(prop) {
        note_point(next, &[underlying_value(next)], axis);
        return;
    }
    let point: Vec<f64> = rows
        .iter()
        .map(|r| match r {
            r if *r == prop => underlying_value(next),
            r => layer.prop(*r).map_or(0.0, underlying_value),
        })
        .collect();
    note_point(next, &point, axis);
}

/// [`note_point`] for the parameter `param` of `inst`, when it is half of a
/// pair and its expression was only just put on.
pub fn note_effect_point(inst: &mut crate::model::EffectInstance, param: &str) {
    use crate::model::EffectValue;
    let Some(pair) = crate::fx::builtins::schema(&inst.effect.match_name)
        .and_then(|schema| schema.pairs().find(|p| p.x == param || p.y == param))
    else {
        return;
    };
    let held = |id: &str| match inst.params.iter().find(|p| p.id == id).map(|p| &p.value) {
        Some(EffectValue::Float(p)) => Some(underlying_value(p)),
        _ => None,
    };
    let (Some(x), Some(y)) = (held(pair.x), held(pair.y)) else {
        return;
    };
    let axis = usize::from(pair.y == param);
    if let Some(EffectValue::Float(p)) = inst
        .params
        .iter_mut()
        .find(|p| p.id == param)
        .map(|p| &mut p.value)
    {
        note_point(p, &[x, y], axis);
    }
}

/// What an expression on `row` of the context's layer is told about its
/// property. `own` is the number the row shows while it is not an expression
/// yet, and the slot is then the one putting one on would write.
#[must_use]
pub fn slot_of_row(context: &ExpressionContext, row: Row<'_>, own: Option<f64>) -> Slot {
    let layer = context.comp.zip(context.layer).and_then(|(comp, layer)| {
        let comp = context.document.comp(comp)?;
        comp.layers.iter().find(|l| l.id == layer)
    });
    let Some(layer) = layer else {
        return Slot::default();
    };
    let Some((props, axis)) = point_row::point_of_row(layer, row) else {
        return Slot::default();
    };
    let Some(this) = props.get(axis) else {
        return Slot::default();
    };
    if own.is_none() && this.extra.contains_key(SLOT_KEY) {
        return Slot::read(&this.extra);
    }
    let own = own.unwrap_or_else(|| underlying_value(this));
    if matches!(row, Row::Transform(prop) if layer.separated(prop)) {
        return Slot::new(&[own], axis, 0);
    }
    let mut point: Vec<f64> = props.iter().map(|p| underlying_value(p)).collect();
    if let Some(at) = point.get_mut(axis) {
        *at = own;
    }
    Slot::new(&point, axis, 0)
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
    program_for(expression).is_ok_and(|program| script::names_known(&program))
}

/// What a JavaScript expression may spend when it is run from the interface's
/// own thread, which is what a Rhai one may spend anywhere. A render gives it
/// twenty times this, on a thread nobody is waiting on.
pub const INTERFACE_STEPS: u32 = MAX_EXPRESSION_OPERATIONS as u32;

/// Run `work` with every JavaScript expression inside it sharing one budget of
/// `steps`, however many it runs and however many properties they read.
///
/// **In plain terms.** The panels sample an expression to show its number and
/// its curve, on the thread that draws the window. A loop that never ends
/// would otherwise freeze it for as long as a render lets the same loop run.
pub fn within_steps<T>(steps: u32, work: impl FnOnce() -> T) -> T {
    script::interp::sparingly(steps, work)
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

/// [`evaluate_in`] with what the expression knows about its property given,
/// not looked up: for text still being typed, which no property holds yet.
pub fn evaluate_with(
    language: Language,
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    slot: Slot,
) -> f64 {
    match language {
        Language::Rhai => evaluate(expression, context),
        Language::JavaScript => javascript_number(expression, context.as_deref(), slot),
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
///
/// `slot` is what the row the curve is drawn for knows about its property
/// ([`slot_of_row`]), and the one carrying the text is found when it is left out.
pub fn evaluate_range(
    language: Language,
    expression: &str,
    context: Option<&ExpressionContext>,
    start: f64,
    end: f64,
    samples: i64,
    slot: Option<Slot>,
) -> Vec<f64> {
    // A curve is a few hundred samples. More than a screen is wide draws
    // nothing new, and the number comes from outside.
    let samples = samples.clamp(0, 8192);
    if language == Language::JavaScript {
        if program_for(expression).is_err() {
            return Vec::new();
        }
        let slot = slot.unwrap_or_else(|| {
            context.map_or_else(Slot::default, |context| slot_on_layer(context, expression))
        });
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
    value_with(language, expression, context, Slot::default(), vars)
}

fn value_with(
    language: Language,
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    slot: Slot,
    vars: &[(&str, f64)],
) -> Result<ExprValue, String> {
    if language == Language::JavaScript {
        return match run_javascript(expression, context.as_deref(), slot, vars)? {
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

/// Why `expression` gives a property no number, or `None` when it does.
///
/// [`evaluate_in`] cannot say: it hands the property of a failed expression a
/// number all the same, so that a render has something to draw. This is for
/// the editor, which shows the sentence. `slot` is what the expression knows
/// about its property, as a render gives it, so `value` and `wiggle` run.
pub fn fault_in(
    language: Language,
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
    slot: Slot,
) -> Option<String> {
    match value_with(language, expression, context, slot, &[]) {
        Err(why) => Some(why),
        Ok(ExprValue::Number(n)) if !n.is_finite() => Some("not a finite number".into()),
        Ok(ExprValue::Number(_)) => None,
        // JavaScript gives a property the number that is its own of a list.
        Ok(_) if language == Language::JavaScript => None,
        Ok(_) => Some("an array - a property takes one number".into()),
    }
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
mod tests;

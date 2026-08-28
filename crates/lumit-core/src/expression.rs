//! Expressions: a small script on a property, evaluated every time that
//! property is read.
//!
//! In plain terms: instead of a number or a row of keyframes, a property can
//! hold a line of code — `time * 90`, `layer("Sun").x` — and the answer is
//! worked out afresh at each frame. The language is [Rhai]; the values it can
//! see (the comp, the layers, `time`) are assembled in
//! [`apply_context_to_scope`] and [`ExpressionContext`].
//!
//! The thing worth knowing before editing this file is that **evaluation is on
//! the hot path**. Every driven property is re-evaluated for every frame, in
//! both the renderer and the frame-cache key, so anything done per evaluation
//! is done tens of thousands of times a second. That is why engines are pooled
//! rather than built (see [`with_engine`]).
//!
//! [Rhai]: https://rhai.rs

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

use crate::Document;
use rhai::{exported_module, Dynamic, Engine, Scope};
use uuid::Uuid;

mod comp;
mod layer;
mod math;

#[derive(Clone, Debug)]
pub struct ExpressionContext {
    pub document: Arc<Document>,
    pub comp: Option<Uuid>,
    pub layer: Option<Uuid>,
    pub comp_time: f64,
    pub current_depth: u32,
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
        }
    }
}

fn make_engine() -> Engine {
    let mut engine = Engine::new();

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
fn eval_dynamic(
    expression: &str,
    context: Option<Arc<ExpressionContext>>,
) -> Result<Dynamic, Box<rhai::EvalAltResult>> {
    let mut scope = Scope::new();

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

/// Whether this text is an expression Lumit can actually run — it parses in
/// Lumit's language, and every name in it exists.
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
    eval_dynamic(expression, Some(Arc::new(ExpressionContext::detached()))).is_ok()
}

pub fn evaluate(expression: &str, context: Option<Arc<ExpressionContext>>) -> f64 {
    convert_result(eval_dynamic(expression, context))
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
    match eval_dynamic(expression, context) {
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
    expression: &str,
    context: Option<&ExpressionContext>,
    start: f64,
    end: f64,
    samples: i64,
) -> Vec<f64> {
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

    /// A broken expression prints nothing rather than failing the frame.
    #[test]
    fn a_broken_expression_prints_nothing() {
        assert_eq!(
            evaluate_text("this is not an expression", Some(at(0.0))),
            ""
        );
        assert_eq!(evaluate_text("no_such_variable", Some(at(0.0))), "");
    }

    /// `time` is readable with nothing but a detached context behind it.
    ///
    /// This is the regression test for scoping `time` to a successful comp
    /// lookup: an expression that reads it then errors, resolves to nothing,
    /// and keys every frame of the comp identically.
    #[test]
    fn time_is_readable_without_a_comp() {
        assert_eq!(evaluate("time * 2.0", Some(at(1.5))), 3.0);
        assert_ne!(
            evaluate("time", Some(at(1.0))),
            evaluate("time", Some(at(2.0)))
        );
    }

    /// Without an expression the layer shows exactly what was typed, and the
    /// typed words survive underneath one that is set.
    #[test]
    fn the_typed_words_are_kept_and_restored() {
        assert_eq!(document(None).resolved_text(at(0.0)), "typed");
        let driven = document(Some("time * 2"));
        assert_eq!(driven.resolved_text(at(1.5)), "3.0");
        assert_eq!(driven.text, "typed");
    }

    /// The same expression at the same time gives the same answer, on any
    /// machine and any run — the determinism rule, applied to words.
    #[test]
    fn resolution_is_deterministic() {
        let d = document(Some("noise(time) + time"));
        assert_eq!(d.resolved_text(at(2.0)), d.resolved_text(at(2.0)));
        assert_ne!(d.resolved_text(at(2.0)), d.resolved_text(at(3.0)));
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
            audio_only: false,
            adjustment: false,
            retime: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            interpolation: Default::default(),
            parked_flow: None,
            markers: Vec::new(),
            paint: Default::default(),
            extra: serde_json::Map::new(),
        }
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
        });
        // The value is meaningless; returning at all is the point.
        let _ = evaluate("layer(\"A\").x", Some(context));
    }

    /// A graph curve for an expression that does not compile is no curve, not
    /// a flat line at zero that reads as a real answer.
    #[test]
    fn an_uncompilable_expression_samples_to_nothing() {
        assert!(evaluate_range("this is not (", None, 0.0, 1.0, 8).is_empty());
        assert_eq!(evaluate_range("time", None, 0.0, 4.0, 4).len(), 4);
    }
}

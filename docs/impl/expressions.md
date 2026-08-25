# Expressions on Rhai

**In plain terms.** A property that would normally hold a number, or a row of keyframes,
can instead hold a line of code — `time * 90`, `layer("Sun").x + 20`. The answer is worked
out afresh every time the property is read, which is at least twice per frame per driven
property. This note is the *how*: what the language is, what it can see, and the two
things about the implementation that look odd until you know why.

It describes **what ships**, not what was once planned. The engine is
[Rhai](https://rhai.rs), settled in **K-305**, which supersedes K-063's choice of
JavaScript on QuickJS-ng. If this note and the code ever disagree again, the code wins and
this note is wrong — say so in the same commit that fixes it.

## 1. What is actually there

`crates/lumit-core/src/expression.rs` and its three submodules:

| file | what it holds |
|---|---|
| `expression.rs` | the engine pool, `ExpressionContext`, the scope constants, the entry points |
| `expression/math.rs` | the maths functions registered into every engine |
| `expression/layer.rs` | `layer()` and the layer property getters |
| `expression/comp.rs` | `comp()` and its one getter |

Entry points, all in `expression.rs`:

- `evaluate(expr, context) -> f64` — a numeric property. A failure resolves to `-1.0`.
- `evaluate_text(expr, context) -> String` — a text layer's line (K-306). Any result type
  prints; a failure prints nothing rather than failing the frame.
- `evaluate_range(expr, context, start, end, samples) -> Vec<f64>` — the graph editor's
  curve. Compiled once, run per sample. An expression that does not compile yields **no**
  samples, deliberately: a flat line at zero would read as a real answer.
- `get_api_metadata() -> String` — Rhai's own function metadata as JSON, which is what
  feeds the editor's completion list.

## 2. Determinism: what is promised and what is not

**K-305 is the binding statement; this is the engineering consequence.**

Lumit promises **reproducibility**: the same project, on the same machine, gives the same
frames on every run. That is what the frame cache key relies on and what the tests assert
(`resolution_is_deterministic`). Nothing in the evaluator may break it — so no wall-clock,
no unseeded randomness, no iteration over a `HashMap` whose order is not fixed.

Lumit does **not** promise bit-identical results across operating systems or GPU vendors.
Rhai's `sin`/`cos` reach the platform's libm, which may differ in the last bit. This is a
deliberate, recorded position rather than an oversight: the same is already true of much of
the engine, and emphatically true of the GPU, where most of Lumit's arithmetic happens.
Making the expression evaluator exact would not make the picture exact. The aim is to be as
close as the hardware allows, and to be honest that the floor is not exact.

Practical rules that follow:

- **No wall-clock, ever.** `time` comes from the render's context, never from the system.
  There is no `Date`-equivalent registered and none should be added.
- **`noise(t)` is a pure function of `t`**, built on an integer-lattice hash with a
  smoothstep blend (`math.rs`). It takes no seed and holds no state, so it is the same on
  every run by construction. If a seeded, AE-style `wiggle` is added later it must key off
  the property id and the time, never off a counter.
- **No host-side f32 shortcuts.** Everything crossing the boundary is `f64`.

## 3. The engine pool — the part that looks odd

Building an engine means `Engine::new()` plus registering three modules of functions. That
measures at **~370µs**, which is roughly forty times the cost of *running* a typical
expression. Evaluation happens per driven property, per frame, in both the renderer and the
frame-cache key, so building an engine per evaluation put a ceiling of about forty driven
properties on a 60fps frame — spent entirely on setup.

The obvious fix, one shared engine, does not work here: the evaluation context is parked on
the engine itself (`Engine::set_default_tag`), and **expressions nest**. `layer("Sun").x`
starts a second evaluation inside the first, and the inner one needs a different context
from the outer. One engine would have the inner trample the outer.

So `expression.rs` keeps a **thread-local pool** of built engines. An evaluation pops one,
uses it, and pushes it back; a nested evaluation pops a second. Measured after: **~1.08µs**
per evaluation, so the same frame budget holds roughly 15,000.

Three properties of the pool are load-bearing, and each has a test:

- The `RefCell` borrow is held only across the pop and the push, **never across
  evaluation** — otherwise re-entry would panic on a double borrow
  (`evaluation_can_re_enter_itself`).
- The context tag is **cleared when the engine goes back**, so a pooled engine cannot carry
  one comp's document into another comp's evaluation
  (`a_pooled_engine_does_not_leak_its_context`).
- An engine that panics its way out is simply not returned; the next call builds a fresh
  one.

Compiled ASTs are **not** cached yet. That was the obvious first optimisation and is no
longer the expensive part; it is worth doing when parsing shows up in a profile, not before.

## 4. What an expression can see

Assembled in `apply_context_to_scope` from an `ExpressionContext { document, comp, layer,
comp_time, current_depth }`.

**Constants.** `time` is pushed unconditionally, because it is the one value that does not
depend on resolving a comp out of the document — scoping it to a successful lookup once
turned every `time`-reading expression into an error, which resolved to nothing, which
keyed every frame of the comp identically. `comp_width`, `comp_height`, `comp_fps`,
`num_layers`, `num_markers` need the comp; `cut_in` and `cut_out` need the layer too, and
are simply absent otherwise, so an expression that reads one fails visibly rather than
quietly reading an invented number.

**`time` is comp time**, matching After Effects. A layer's own clock is `layer().time`,
which subtracts the layer's in point.

**Functions.** `math.rs`: `sin`, `cos`, `sinh`, `cosh`, `floor`, `ceil`, `round`, `abs`,
`clamp`, `noise`, `smoothstep`, `fit`, `fit_clamped`, `fit01`. Every one takes `Dynamic`
and goes through `to_f64`, because Rhai keeps whole numbers and fractions as separate types
and `2` must work wherever `2.0` does.

**Objects.** `comp()` (`.name`) and `layer()` / `layer("Name")` (`.name`, `.time`, `.x`,
`.y`, `.rotation`, `.scale_x`, `.scale_y`, `.anchor_x`, `.anchor_y`, `.opacity`). A
reference that does not resolve returns `"Invalid Layer Reference"` or `-1.0` rather than
erroring — a name is typed one character at a time and is wrong for most of that.

The layer getters share one helper (`transform_property`) which re-points the context at
the layer being read and steps `current_depth`.

## 5. Never panic; bound the recursion

`lumit-core` is an engine crate, so `docs/14-ENGINEERING-RULES.md` §4 applies: no panics.
Two traps specific to this code:

- **`Dynamic::clone_cast` panics** when the engine's tag is absent or of another type, and
  absent is ordinary — a standalone preview sets no context. Read the context through
  `ExpressionContext::from_call`, which falls back to the detached context. Never
  `clone_cast` the tag directly.
- **Rhai's `#[export_module]` expands to `unwrap()`s** of its own on `&mut` receivers,
  which trips `clippy::unwrap_used`. The generated modules carry a scoped `allow` naming
  the reason; do not widen it to hand-written code.

Recursion is bounded by `MAXIMUM_DEPTH = 100` on `ExpressionContext::current_depth`, which
is what stops two properties that read each other from running until the stack goes
(`a_cycle_stops_instead_of_overflowing`). It is a depth limit, not cycle detection: a
visited-set would catch a cycle on the first hop instead of the hundredth, and is worth
doing if deep rigs ever make the difference visible.

There is **no evaluation time budget**. A long-running expression is not currently
interrupted, so a pathological one can stall a render thread. Rhai supports this through
`Engine::on_progress`; wiring it to the epoch token
([playback-scheduler.md](playback-scheduler.md)) is the known gap.

## 6. Where an expression is read from

- **Numeric properties** — `Property::value_at_with_context`, in `anim.rs`. The plain
  `value_at` passes no context, so an expression read through it sees no `time`; every
  render and cache-key path uses the context form.
- **Text layers** — `TextDocument::resolved_text` (K-306). The rasteriser and the frame
  cache key both go through it, so they cannot disagree about what the layer says. Hashing
  the *stored* text for a driven layer would key every frame identically and freeze the
  number on screen.
- **The frame cache key** — `lumit-eval` builds one `ExpressionContext` per key and shares
  the document rather than copying it. Copying the project per layer is quadratic: 151µs
  per copy at two hundred layers, which per layer per frame is 30ms — twice the whole 60fps
  budget, before anything is drawn.

## 7. Test plan

Implemented in `expression.rs`'s test module unless noted.

1. **Reproducibility** — the same expression at the same time gives the same answer, and a
   different one at a different time (`resolution_is_deterministic`). Cross-platform
   byte-identity is explicitly *not* asserted, per K-305.
2. **`time` without a comp** — the regression test for scoping it to a comp lookup
   (`time_is_readable_without_a_comp`), plus `expression_driven_text_keys_per_frame` in
   `lumit-eval`, which fails if a driven caption keys the same every frame.
3. **Re-entrancy** — an expression reading a property that is itself an expression
   (`evaluation_can_re_enter_itself`). This is the test that fails if the pool is replaced
   by a shared engine.
4. **Pool hygiene** — a pooled engine does not carry the previous context
   (`a_pooled_engine_does_not_leak_its_context`).
5. **Cycles** — two properties reading each other return rather than overflowing
   (`a_cycle_stops_instead_of_overflowing`).
6. **Graph sampling** — an uncompilable expression yields no samples, a valid one yields
   the number asked for (`an_uncompilable_expression_samples_to_nothing`).
7. **Text** — printing, clearing, the typed words surviving underneath, and the field being
   optional on disk.

## 8. Known gaps

Named so they are not rediscovered as bugs:

- **No evaluation budget** (§5) — the one with real teeth.
- **No AST cache** (§3) — cheap to add, no longer the bottleneck.
- **`-1.0` is the failure value** for numeric expressions, so a broken expression becomes a
  plausible-looking coordinate. Text got this right by printing nothing. Surfacing the error
  in the UI is the real answer.
- **Depth limit, not cycle detection** (§5).
- **Scalars only.** Point and colour properties cannot be driven yet.
- **No AE-compatible library** — no `wiggle`, `loopOut`, `valueAtTime`, `linear`, `ease`,
  marker access. [12-PLUGINS.md](../12-PLUGINS.md) §scripting still describes the
  JavaScript-shaped API that K-063 assumed; it has not been rewritten for Rhai, and the
  names above are the gap between what it promises and what exists.

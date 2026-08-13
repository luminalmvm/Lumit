# The document: lumit-core and lumit-project

`lumit-core` defines what a project *is* and how edits change it. Pure data plus
command application: no GPU, no threads, no async, no IO (one exception:
`preset.rs` reads `.lumfx` preset files). `lumit-project` writes and reads that
document to disk and keeps the crash-recovery journal.

Specs: [03-DATA-MODEL.md](../03-DATA-MODEL.md), [04-RETIMING.md](../04-RETIMING.md),
[10-FILE-FORMAT.md](../10-FILE-FORMAT.md). Impl notes: `rational-time.md`,
`keyframe-eval.md`, `expressions.md`.

## Module map

| File | Owns |
|---|---|
| `src/time.rs` | `Rational`, `Duration`, the four timebase newtypes, `FrameRate` |
| `src/model.rs` | `Document`, `ProjectItem`, `Composition`, `Layer`, blend modes |
| `src/anim.rs` | `Keyframe`, bezier evaluation, `Property`/`Animation` |
| `src/ops.rs` | The `Op` command enum; `apply()` returns the exact inverse |
| `src/store.rs` | `DocumentStore`: snapshots, journal, undo/redo, observer |
| `src/retime.rs` | Retime maths; `Interpolation` (Nearest/Blend/Flow), `FlowParams` |
| `src/sequence.rs` | Clips inside Sequence layers, playhead resolution |
| `src/markers.rs`, `src/mask.rs`, `src/shape.rs`, `src/paint.rs` | Markers, masks, shape layers, paint strokes (each with its CPU rasteriser) |
| `src/pixels.rs`, `src/lut.rs` | sRGB conversion, CPU blending helpers; `.cube` LUT parsing |
| `src/expression.rs` (+ `expression/{math,comp,layer}.rs`) | Rhai expressions |
| `src/fx/` | Effect schemas, parameter resolution, CPU reference implementations |
| `src/preset.rs` | `.lumfx` presets |

## Rational time

Time is never a float in the document. `Rational` is `num: i64, den: i64`, always
normalised; every operation is `checked_*` and returns `Result<_, TimeError>`;
intermediate multiplies widen to `i128` (see `impl Ord`, `time.rs:153`). 29.97 fps
crosses the codebase as `30000/1001`, never `29.97`.

The four timebases from the glossary are four distinct newtypes made by one macro
(`timebase!`, `time.rs:179`): `SourceTime`, `ClipTime`, `LayerTime`, `CompTime`.
They deliberately overload **no operators**. The only arithmetic is
`add_dur(Duration)`, `sub_dur(Duration)` and `delta(Self) -> Duration`. Adding a
`CompTime` to a `SourceTime` does not compile — that is the feature. Floats exist
at two doors only: `to_f64` on the way out (evaluation), `from_f64_on_grid` on the
way in (quantise to an explicit grid).

## The document model

`Document` holds a flat `Vec<ProjectItem>` (Footage | Folder | Composition | Solid);
folders reference children by id. A `Composition` holds `Vec<Layer>` with index 0 on
top. A `Layer` is: id, name, `LayerKind`, a span in `CompTime`, a `TransformGroup`
of 11 `Property` scalars, matte, parent, masks, paint, effects, switches, optional
retime, blend mode.

Three durable rules:

- Every entity id is a `Uuid::now_v7()`. Nothing identifies anything by index.
- Every model struct carries `#[serde(flatten)] extra: serde_json::Map` — unknown
  fields from a newer Lumit round-trip instead of being dropped.
- Dangling references (matte, parent, precomp target) degrade to "not there".
  They never error.

## Animation

`Property` wraps an `Animation` enum (`anim.rs:353`): `Static(f64)`,
`Keyframed(Vec<Keyframe>)`, or `Expression(String)`. A `Keyframe` is rational time,
f64 value, and per-side interpolation (`Hold | Linear | Bezier { speed, influence }`)
— the After Effects model, so imports are lossless. Between two bezier keys the
curve is a cubic; "what value at time t" solves the cubic with bracketed Newton
(`solve_u`). `insert_key_preserving_shape` splits the cubic (de Casteljau) so adding
a key never changes the curve (K-221).

## Edits, undo, snapshots

Every edit is one `Op` — a big serde-tagged enum of small, invertible commands.
`ops::apply(&mut doc, &op)` mutates the document and returns the **exact inverse
op** (mostly via `std::mem::replace`; see `ops.rs:451`). List-valued edits replace
the whole list, which makes the inverse trivial. `Batch` rolls back applied members
if a later member fails. Layer lock is enforced centrally inside `apply` (K-291,
the `lock_guards` table).

`DocumentStore` (`store.rs`) is where threads meet the document:

1. `commit` locks the journal mutex, clones the whole `Document`, applies the op.
2. It pushes `JournalEntry { op, inverse }`, clears redo, publishes the new
   `Arc<Document>` through `ArcSwap`, bumps an atomic revision.
3. It notifies the observer **after** dropping the lock — the callback crosses FFI
   and may re-enter the store; a test hangs if you regress this.

Undo applies the stored inverse; history caps at 500 entries. Readers hold their
`Arc` snapshot; no edit ever changes data under them.

## Retime

The live representation is `retime: Option<Property>` on layers and clips: a
keyframable map from layer-local seconds to source seconds. `None` means "not
retimed" — different from an identity map, and a `Static` map is a freeze.
`Interpolation` (Nearest/Blend/Flow) and `FlowParams` are render policy, separate
from the map. The older segment store in `retime.rs` survives for the 0.1.0→0.2.0
file migration (K-249).

## Expressions

Rhai scripts per property (`expression.rs`). Engines cost ~370µs to build, so they
pool in a `thread_local!` stack — a stack because expressions re-enter (an
expression reading `layer("Sun").x` evaluates that property too). Hermetic by
construction: a script sees only pushed constants and the registered math/comp/layer
modules over the immutable snapshot; recursion caps at depth 100; a failed script
evaluates to −1.0 (or "" for text), never a failed frame.

## Effects

Today: a static `BUILTINS: &[EffectSchema]` catalogue (`fx/builtins.rs`);
`instantiate` copies defaults into an `EffectInstance`; `resolve_stack` evaluates
every animatable parameter at layer time into flat `Resolved` ops — plain numbers
consumed by both `fx/cpu.rs` (the CPU reference, and the GPU's test oracle, K-019)
and `lumit-gpu`'s WGSL kernels. Preview equals export because both read the same
resolution (K-031).

**Landing soon (PR #98, K-373).** Effects move to single declaration: a
`#[derive(Effect)]` proc macro (new crate `crates/lumit-fx-macros`) turns one struct
per effect (`fx/effects/*.rs`) into schema + typed parameter reader; `fx/registry.rs`
adds the `EffectDef` trait and `Catalogue`; `fx/catalogue.rs` lists effects in
Add-effect menu order; `fx/params.rs` adds the resolved `(ParamId, Value)` arena and
a `Unit` on every numeric parameter. Nine colour effects are migrated; the render
path still resolves through the old enum until later batches. Read
`docs/impl/effect-registry.md` first.

**Landing soon (PR #97).** The model gains Light layers:
`LayerKind::Light { light: Box<LightDef> }` with `LightKind` Point/Spot/Area,
animatable colour and intensity, and `accepts_lights: bool` on every layer. A new
`src/lighting.rs` is the CPU oracle for the GPU shading pass. Two effects join the
catalogue: `light_wrap` and `sprite_flare`.

## Saving: lumit-project

*(Section completed from the project-crate survey below; the crate is small.)*

`lumit-project` serialises the `Document` to the `.lum` container, appends every
committed op to the on-disk operation journal, and rebuilds `last snapshot +
journal replay` on crash recovery. It owns the 0.1.0→0.2.0 migration (segment
Retime → property Retime, K-249). Details: [10-FILE-FORMAT.md](../10-FILE-FORMAT.md).

## Traps

- The workspace denies `unwrap`, `expect`, `panic`, `todo`, `unimplemented`,
  `unsafe`. Engine failure degrades to a picture (identity, hold, empty frame) —
  never an error the render surfaces.
- Keyframe vectors must stay sorted with unique times. The editing ops enforce it;
  `evaluate` assumes it.
- A new `Op` that touches a layer must be added to `lock_guards`, or it silently
  bypasses layer lock.
- New serialised fields need `#[serde(default)]` + `skip_serializing_if` so
  untouched projects serialise byte-identically — the frame cache keys on those
  bytes.
- `retime: None` skips the map entirely; an identity map does not. A clip's
  identity map starts at `source_in`, not zero.

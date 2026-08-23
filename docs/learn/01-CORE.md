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
| `src/ops.rs` | The `Op` command enum. `apply()` returns the exact inverse |
| `src/store.rs` | `DocumentStore`: snapshots, journal, undo/redo, observer |
| `src/retime.rs` | Retime maths, `Interpolation` (Nearest/Blend/Flow), `FlowParams` |
| `src/sequence.rs` | Clips inside Sequence layers, playhead resolution |
| `src/markers.rs`, `src/mask.rs`, `src/shape.rs`, `src/paint.rs` | Markers, masks, shape layers, paint strokes (each with its CPU rasteriser) |
| `src/pixels.rs`, `src/lut.rs` | sRGB conversion, CPU blending helpers, `.cube` LUT parsing |
| `src/expression.rs` (+ `expression/{math,comp,layer}.rs`) | Rhai expressions |
| `src/lighting.rs` | The CPU oracle for shading a layer with the comp's Light layers |
| `src/fx/` | Effect schemas, the effect registry, parameter resolution, CPU reference implementations |
| `src/preset.rs` | `.lumfx` presets |

## Rational time

Time is never a float in the document. `Rational` is `num: i64, den: i64`, always
normalised. Every operation is `checked_*` and returns `Result<_, TimeError>`.
Intermediate multiplies widen to `i128` (see `impl Ord for Rational` in `time.rs`). 29.97 fps
crosses the codebase as `30000/1001`, never `29.97`.

The four timebases from the glossary are four distinct newtypes made by one macro
(the `timebase!` macro in `time.rs`): `SourceTime`, `ClipTime`, `LayerTime`, `CompTime`.
They deliberately overload **no operators**. The only arithmetic is
`add_dur(Duration)`, `sub_dur(Duration)` and `delta(Self) -> Duration`. Adding a
`CompTime` to a `SourceTime` does not compile — that is the feature. Floats exist
at two doors only: `to_f64` on the way out (evaluation), `from_f64_on_grid` on the
way in (quantise to an explicit grid).

## The document model

`Document` holds a flat `Vec<ProjectItem>` (Footage | Folder | Composition | Solid).
Folders reference children by id. A `Composition` holds `Vec<Layer>` with index 0 on
top. A `Layer` is: id, name, `LayerKind`, a span in `CompTime`, and a `TransformGroup`
of 11 `Property` scalars. A `Layer` also holds matte, parent, masks, paint, effects,
switches, optional retime, and blend mode.

One `LayerKind` is worth naming on its own. `LayerKind::Light { light: Box<LightDef> }`
(K-360) is a layer that emits rather than one that draws: `LightKind` is Point, Spot or
Area, and `LightDef` carries animatable colour, intensity, falloff distance, spot cone
angle and — for an Area light — a half-width and half-height in comp pixels (half, so it
measures from the centre outward like the flare's own source dials). The *placement* is
deliberately not in
`LightDef`; a light sits at its layer's ordinary transform, so it animates and parents
like everything else. `LightDef` is boxed because eight extra `Property` channels would
otherwise be paid for by every layer in every comp. On the receiving side, every layer
carries an `accepts_lights: bool` switch, on by default.

`lighting.rs` is the CPU half of what those lights do, and it is the reference the WGSL
twin is tested against. Two things about it are choices rather than physics. Light
**adds**: the shaded result is the picture times `1 + light`, so a layer no light reaches
is untouched and a comp with no lights renders byte-for-byte as it did before lights
existed. And a layer has **one** normal — the direction its plane faces — because a 2.5D
compositor has no per-pixel normals and inventing them from luminance is a quality cliff.
For an Area light the answer is closed-form (the diffuse form factor: one term per edge of
the rectangle, exact, no sampling); Points and Spots fall back to the ordinary cosine law.
`MAX_LIT_LIGHTS` caps one pass at eight, nearest kept.

Three durable rules:

- Every entity id is a `Uuid::now_v7()`. Nothing identifies anything by index.
- Every model struct carries `#[serde(flatten)] extra: serde_json::Map`. Unknown
  fields from a newer Lumit round-trip. Lumit does not drop them.
- Dangling references (matte, parent, precomp target) degrade to "not there".
  They never error.

## Animation

`Property` wraps an `Animation` enum (`Animation` in `anim.rs`): `Static(f64)`,
`Keyframed(Vec<Keyframe>)`, or `Expression(String)`. A `Keyframe` is rational time,
f64 value, and per-side interpolation (`Hold | Linear | Bezier { speed, influence }`).
This is the After Effects model, so imports are lossless. Between two bezier keys the
curve is a cubic. "What value at time t" solves that cubic with bracketed Newton
(`solve_u`). `insert_key_preserving_shape` splits the cubic (de Casteljau), so adding
a key never changes the curve (K-221).

## Edits, undo, snapshots

Every edit is one `Op`, a big serde-tagged enum of small, invertible commands.
`ops::apply(&mut doc, &op)` mutates the document and returns the **exact inverse
op** (mostly via `std::mem::replace`, see the `Op::SetMediaRef` arm in `ops.rs`). List-valued edits replace
the whole list, which makes the inverse trivial. `Batch` reverts applied members
if a later member fails. `apply` enforces layer lock centrally (K-291,
the `lock_guards` table).

`DocumentStore` (`store.rs`) is where threads meet the document:

1. `commit` locks the journal mutex, clones the whole `Document`, applies the op.
2. It pushes `JournalEntry { op, inverse }`, clears redo, publishes the new
   `Arc<Document>` through `ArcSwap`, bumps an atomic revision.
3. It notifies the observer **after** dropping the lock. The callback crosses FFI
   and may re-enter the store. A test hangs if you regress this.

Undo applies the stored inverse. History caps at 500 entries. Readers hold their
`Arc` snapshot. No edit ever changes data under them.

## Retime

The live representation is `retime: Option<Property>` on layers and clips: a
keyframable map from layer-local seconds to source seconds. `None` means "not
retimed", which is different from an identity map. A `Static` map is a freeze.
`Interpolation` (Nearest/Blend/Flow) and `FlowParams` are render policy, separate
from the map. The older segment store in `retime.rs` survives for the 0.1.0→0.2.0
file migration (K-249).

## Expressions

Rhai scripts per property (`expression.rs`). Engines cost ~370µs to build, so they
pool in a `thread_local!` stack. It is a stack because expressions re-enter: an
expression that reads `layer("Sun").x` evaluates that property too.

Scripts are hermetic by construction. A script sees only pushed constants and the
registered math/comp/layer modules over the immutable snapshot. Recursion caps at
depth 100. A failed script evaluates to −1.0 (or "" for text), never a failed frame.

## Effects

Every effect is declared once, in its own file under `fx/effects/` — 35 of them,
registered in `fx/catalogue.rs` in Add-effect menu order (K-137), which the macro expands
into `BUILTINS: &[EffectSchema]`. `instantiate` copies a schema's defaults into an
`EffectInstance`. `resolve_stack` evaluates every declared parameter at layer time into
the parameter arena (`ResolvedStack`), and dispatch is a lookup: `lumit-render`'s GPU
table and each effect's own `apply_cpu` (the CPU reference, and the GPU's test oracle,
K-019) read the same bag. Preview equals export because both read the same resolution
(K-031). Read `docs/impl/effect-registry.md` before touching any of it.

### An effect declared once (K-381)

The complaint the new machinery answers: an effect was written down five or six times — a
schema literal in `builtins.rs`, a variant of `Resolved`, an arm of `resolve_one`, an arm
of `cpu::apply`, an arm of `run_ops`, and `rescale_px` if it held a length. Three of those
matches were exhaustive, which made `Resolved` a compile-time chokepoint: every effect that
would ever exist had to be known when `lumit-core` was built. That is an awkward property
for a program that intends to host OFX plugins, and the sets had already quietly drifted
apart.

Four pieces replace it:

| Where | What it does |
|---|---|
| `crates/lumit-fx-macros/` | A proc-macro crate. `#[derive(Effect)]` reads one plain struct — the `#[effect(...)]` header, then one attribute per field (`#[slider]`, `#[toggle]`, `#[choice]`, `#[colour]`, `#[dial]`, `#[counter]`, `#[seed]`, `#[file]`, `#[layer]`) — and writes both the `EffectSchema` and a typed reader for it. A field with no attribute is a compile error, so a parameter cannot be half-declared. Labels default to the field name in sentence case |
| `fx/registry.rs` | `EffectDef`, the trait an effect implements (its schema, its CPU reference, whether it is an image op at all), plus `Catalogue`, which looks one up by `match_name` |
| `fx/catalogue.rs` | Registration, as a written list: `catalogue![SaturationDef, VibrancyDef, …]`. Nine lines today |
| `fx/params.rs` | The resolved form: a bag of `(ParamId, Value)` pairs instead of a closed enum |

An effect's own file is then short enough to read whole — `fx/effects/contrast.rs` is a
struct with two annotated fields and one `apply_cpu` that calls the existing `cpu::contrast`.
Nothing about any effect's *maths* changed.

Two details in `params.rs` carry the weight, and both exist to keep properties the old
`Resolved` enum had for free:

- **A key is a number, not a string.** `ParamId` is the FNV-1a 64 hash of the parameter's
  stable snake_case id, computed in a `const fn`, so a built-in's ids are compile-time
  constants and a lookup compares two `u64`s. Every parameter of one layer's whole stack
  lives contiguously in a single `ResolvedStack` arena; a `ResolvedFx` borrows the run that
  is its own and stays `Copy`. That is one allocation per stack — one *fewer* than the
  `Vec<Resolved>` it replaces.
- **The frame key is fed field by field.** `Resolved` was hashed byte-wise (K-143), which
  `Value` cannot be: it has padding, and a padding byte in a cache key is a wrong picture
  that only reproduces on one machine. `ResolvedStack::feed_hash` writes the effect name,
  its version, then each parameter's id, a tag byte and its live bytes.

Every numeric parameter also declares a `Unit` (`Raw`, `Px`, `Degrees`, `Seconds`;
`PctDiag` exists but no parameter may use it, K-419). That is what turns the old per-variant `rescale_px` into one generic pass
(`rescale_spatial`): an effect can no longer be *forgotten* there, which is how a preview
raster and a full-size export come to disagree.

**State of the migration: complete.** All 35 effects are declared once; `Resolved`,
`resolve_one`, `cpu::apply`'s match and the hand-written `BUILTINS` literal are gone, and
the parity test went with them — its job was making the port mechanical, and the port is
done. What remains of the programme is dynamic and spare parameters (docs/impl §6 step 5).
Adding a *new* effect during the
transition still needs the old sites; migrating one is a commit that changes no pixel.

## Saving: lumit-project

`lumit-project` serialises the `Document` to the `.lum` container. It appends every
committed op to the on-disk operation journal. On crash recovery it rebuilds
`last snapshot + journal replay`. It owns the 0.1.0→0.2.0 migration (segment
Retime → property Retime, K-249). Details: [10-FILE-FORMAT.md](../10-FILE-FORMAT.md).

## Traps

- The workspace denies `unwrap`, `expect`, `panic`, `todo`, `unimplemented`,
  `unsafe`. Engine failure degrades to a picture (identity, hold, empty frame). It
  never becomes an error the render surfaces.
- Keyframe vectors must stay sorted with unique times. The editing ops enforce it.
  `evaluate` assumes it.
- Add every new `Op` that touches a layer to `lock_guards`. An `Op` that is missing
  there silently bypasses layer lock.
- New serialised fields need `#[serde(default)]` + `skip_serializing_if` so
  untouched projects serialise byte-identically. The frame cache keys on those
  bytes.
- `retime: None` skips the map entirely. An identity map does not. A clip's
  identity map starts at `source_in`, not zero.
- **An effect's schema lives in `fx/effects/<name>.rs`** and nowhere else — the old
  hand-written catalogue is gone. A new or renamed label also wants the fixture refresh
  (`cargo test -p lumit-core regenerate_fx_label_fixture -- --ignored`) and its
  translation entry, or the l10n gate says so.
- A new numeric parameter needs its `Unit`. `Raw` on something spatial compiles, renders
  correctly at full size, and is wrong at every preview resolution.

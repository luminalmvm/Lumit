# The effect registry: one declaration per effect

**Covers:** how a built-in effect is declared, registered, resolved at a frame and
dispatched to the GPU, once the `fx::Resolved` enum and the hand-written half of
`BUILTINS` are gone. **Feeds:** [08-EFFECTS.md](../08-EFFECTS.md),
[05-ARCHITECTURE.md](../05-ARCHITECTURE.md), [06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md).

## In plain terms

Adding one effect to Lumit currently means writing the same effect down five times, in
five files, and keeping the five copies in step by hand: the catalogue entry that says
what its controls are, a variant in a giant enum that says what those controls look like
once they are numbers, the arm that fills that variant in, the arm that turns it into a
GPU call, and the CPU version that the tests check the GPU against. Nothing but a test
notices when the five drift apart, and adding a control to an existing effect means
touching all five again.

This note describes the arrangement that replaces it. An effect is declared **once**, as a
struct whose fields *are* its parameters, with the slider ranges and defaults written as
attributes on those fields. A macro turns that single declaration into the catalogue entry.
One small file lists the effects that exist. At render time the parameters are evaluated
into a **bag of key/value pairs** rather than into a variant of a giant enum, which is what
lets an effect grow parameters that nobody wrote down at compile time — a shader the user
typed, a node network they built, or a spare slider they added to drive something else with
an expression.

The rule of thumb it buys: **adding an effect touches its own file and one line of a list.**

## 1. What was wrong

The inventory, at the point this note was written (35 built-ins):

| Surface | Where | Size |
|---|---|---|
| Schema literal | `lumit-core/src/fx/builtins.rs` | 3 948 lines |
| `Resolved` variant | `lumit-core/src/fx/resolved.rs` | 34 variants, ~558 lines |
| `resolve_one` arm | same file | 33 arms, ~3 000 lines |
| CPU oracle arm | `lumit-core/src/fx/cpu.rs` | one exhaustive match |
| GPU dispatch arm | `lumit-render/src/fxops.rs` | one exhaustive match, 36 sites |
| Bridge kind mapping | `lumit-bridge/src/api/effect.rs` | hand-written mirror of `ParamKind` |

Three of those matches are **exhaustive**, so the enum is a compile-time chokepoint: every
effect must be known to `lumit-core` at compile time, and a third-party or user-authored
effect cannot exist at all. `rescale_px` is a fourth exhaustive match whose only job is to
know which fields of which variant are in pixels — a fact the schema could have carried.

The three sets are not even in bijection: `spectral_split` has a variant and no schema
entry, `posterize_time` and `accumulation_mb` have schema entries and no variant.

## 2. The shape

Five pieces, in dependency order.

### 2.1 The declaration (one struct, in `lumit-core/src/fx/effects/<name>.rs`)

```rust
/// Depth of field (docs/08 §3.20).
#[derive(Effect)]
#[effect(
    match_name = "dof",
    label = "Depth of field",
    version = 1,
    category = Distortion,
    cost = Heavy,
    roi = FullFrame,
)]
pub struct Dof {
    #[layer]
    pub depth: LayerRef,

    #[slider(0.0..=1.0, default = 0.5, unit = PctDiag)]
    pub focus: f32,

    #[toggle(default = false)]
    pub use_focus_point: bool,

    #[choice(["Rendered", "Depth map", "Focus map"], default = "Rendered")]
    pub display: Display,
    …
}
```

The macro generates, from that one declaration:

- `impl EffectMetadata for Dof`, carrying `const SCHEMA: EffectSchema` — the same
  `EffectSchema` value that used to be hand-written in `builtins.rs`, so every existing
  consumer (the Add-effect menu, the bridge, `param_enabled`, the backfill) is unchanged.
- `impl Dof { fn read(params: Params<'_>) -> Self }` — the typed reader that pulls each
  field back out of the resolved bag by its generated `ParamId`, applying the declared
  default when the bag has no entry for it. This is what makes an old project that predates
  a parameter simply work, and it is the only place a parameter's default is written.
- `const IDS: &[ParamId]` and one `pub const <FIELD>: ParamId` per field, so a lookup in a
  hot loop is a comparison of two `u64`s that were computed at compile time.

Attributes map to `ParamKind` one-for-one: `#[slider]` → `Float`, `#[counter]` → `Int`,
`#[dial]` → `Angle`, `#[toggle]` → `Bool`, `#[choice]` → `Choice`, `#[colour]` → `Colour`,
`#[seed]` → `Seed`, `#[file]` → `File`, `#[layer]` → `Layer`. Group and greying metadata
(`ParamGroup`, `EnabledWhen`, K-145 and K-313) stay declared on the effect attribute,
because they name *runs* of parameters rather than living inside one.

### 2.2 Units are declared, not remembered

Every numeric parameter declares a `unit`, which is what removes the fourth exhaustive
match:

| Unit | Meaning | Resolve does |
|---|---|---|
| `Raw` | a plain number (a mix, a gamma) | nothing |
| `PctDiag` | % of the comp diagonal (docs/08 §2.3) | × `diag_px / 100` |
| `Px` | already pixels of the target raster | × `px_scale` |
| `Degrees` | an angle | nothing |
| `Seconds` | a duration | nothing (rational time is resolved upstream) |

`diag_px` reaches the resolve step **already scaled by the preview factor**, which is why
`PctDiag` does not multiply by `px_scale` a second time — the hand-written arms did exactly
this, and doing both would shrink a preview radius twice.

`rescale_px` becomes one generic pass over the bag: rescale every value whose declared unit
is `PctDiag` or `Px`. An effect cannot forget to be rescaled, which was possible before and
is how a preview raster and a full-size export could disagree. **Radial blur's Amount is a
case in point**: the old `rescale_px` skipped it on the mistaken grounds that the whole op
was frame-relative, so an adjustment stack under reduced-resolution preview blurred too far.
Declaring the unit fixed it without anyone deciding to — which is the point of declaring it.

**docs/08 §2.3 is unchanged by this** — a raw "pixels of whatever buffer I was handed"
parameter is still forbidden. `Px` means px@comp on the way in, and the resolve step is the
only thing that converts to the raster in play.

### 2.3 The bag (`lumit-core/src/fx/params.rs`)

```rust
/// A parameter's identity: the FNV-1a 64 hash of its stable id, computed in a
/// `const fn` so a built-in's ids are compile-time constants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ParamId(u64);

/// One parameter, resolved to plain numbers at a frame.
#[derive(Clone, Copy, PartialEq)]
pub enum Value {
    Float(f32),
    Int(i32),
    Bool(bool),
    Choice(u32),
    Colour([f32; 4]),
    /// A layer reference resolves to *whether* it is bound; the texture itself
    /// rides beside the op, as it does today (docs/impl/layer-input.md).
    Layer(bool),
    /// A file reference resolves to its slot in the stack's file table, for the
    /// same reason.
    File(u32),
}
```

The storage is an **arena per resolved stack**, not a fixed-size array per effect:

```rust
/// Everything one layer's stack resolved to at one frame. One allocation,
/// which is one fewer than the `Vec<Resolved>` it replaces.
pub struct ResolvedStack {
    ops: Vec<Op>,                    // { def: &'static dyn EffectDef, instance: Uuid, span: Range<u32> }
    entries: Vec<(ParamId, Value)>,  // every op's parameters, contiguous
}

/// One resolved effect: `Copy`, because it borrows the arena rather than owning
/// anything. This is what the draw structs and `run_ops` pass around.
#[derive(Clone, Copy)]
pub struct ResolvedFx<'a> {
    pub def: &'static dyn EffectDef,
    pub instance: Uuid,
    pub params: Params<'a>,          // &'a [(ParamId, Value)]
}
```

**Why an arena and not the fixed stack array the issue thread suggested.** The two aims
were "no per-parameter heap allocation" and "no cap on how many parameters an effect has".
A fixed `[(ParamId, Value); N]` meets the first and fails the second — and N would have to
be ≥ 50 for the Lens flare, which is 1.2 kB copied for a one-parameter Blur. The arena
meets both: parameters are contiguous in one allocation that the stack already made, a
lookup is a short linear scan of adjacent memory (an effect's parameters are ≤ 50 and
almost always ≤ 10, so this is faster than hashing), and `ResolvedFx` stays `Copy`.

**Determinism.** `Resolved` was `Copy` plain-old-data so a stack could be hashed
byte-for-byte (K-143). `Value` has padding, so byte-hashing it would be feeding
uninitialised bytes into a frame key — the arena is hashed **field by field** through an
explicit `feed_hash`, in stack order, which is stronger than the byte-wise version it
replaces (it cannot silently change when a variant grows). See §5.

### 2.4 The effect's own behaviour (`EffectDef`, in `lumit-core`)

```rust
pub trait EffectDef: Sync + 'static {
    fn schema(&self) -> &'static EffectSchema;

    /// Host-side maths the kernels must not repeat: the clamps, the `exp2`s, the
    /// blade normals. Turns the resolved bag into the effect's own parameter
    /// struct — the *same* struct the CPU reference takes, which is what removes
    /// the second of the parallel field lists.
    fn pack(&self, p: Params<'_>) -> Packed;

    /// The CPU reference (docs/08 §1.6), dispatched by the registry rather than
    /// by a match.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>);

    /// Parameters this instance has beyond its schema's (§4). The default
    /// implementation returns none, which is every built-in but two.
    fn derived(&self, _inst: &EffectInstance) -> Vec<ParamSchema> { Vec::new() }
}
```

`pack` is deliberately not generic: an effect that needs to fold an aperture into eight
blade normals does it here, once, for both the GPU and the CPU path. As built it is an
inherent `packed()` on the parameter struct rather than a trait method, so each effect
returns its own shape and no `Packed` union has to exist.

**An effect with a mode fork returns an enum, not a tuple.** A *quality tier* (K-090) —
RGB split's and chromatic aberration's Wavelength toggle — runs a different kernel with a
different uniform, which is why those effects had two `Resolved` variants each before they
moved. `packed()` answering with a small enum keeps the fork in one place: the CPU
reference and the GPU wrapper both match on it, so neither can decide the mode for itself.

### 2.4a Resolve-time derivation (the time and marker seam, K-385)

A few effects derive values at resolve time from things that are not parameters: Flash
builds a beat envelope from the marker context and a whole keyframed trigger track,
Scanlines rolls by layer time, Block glitch ticks by it, and the temporal family samples
around it. The bag cannot carry "layer time" as a declared parameter - it is not one -
so `EffectDef` grows one optional hook:

```rust
/// What resolve-time derivation sees: the instance, layer time, the raster
/// diagonal, the §2.3 preview factor, the marker context and the expression
/// context - exactly what the old `resolve_one` arms read, no more.
pub struct ResolveCx<'a> { ... }

trait EffectDef {
    ...
    /// Values derived at resolve time from non-parameters. Pushed into the bag
    /// under `ParamId`s the effect declares beside its schema consts (their ids
    /// namespaced `derived.`), after the declared parameters, in declaration
    /// order. The default pushes nothing, which is every effect but a few.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {}
}
```

Three rules keep it honest:

- **The hook reads, never writes.** It sees the instance and the contexts; the only
  output is the pushed values. Host maths that needs no time stays in `pack`.
- **Derived ids are constants, not schema rows.** They never appear in the panel, are
  not keyframeable, and are not serialised - they are the *result* of parameters and
  time, recomputed every resolve. The collision test covers them like any other id.
- **The frame key needs nothing new.** Layer time is part of the frame's identity and
  the marker context is document state the key already covers; the hook only moves
  where the derivation runs, exactly as the old arms did it.

**The trap: a derived value does not rescale.** `ResolvedStack::rescale_spatial` finds a
value's unit by matching its id against the schema, and a derived id matches nothing — so
a derived value in raster pixels is left behind when a stack resolved against one raster is
reused at another. Scanlines is the live case: `derived.roll_px` is a product of the *raster*
line period, and under rescale the period moves while the offset does not, shifting the
pattern's phase with the size. Prefer deriving a quantity that is already unit-free (a tick,
a strength, a count of periods) over one in pixels; where pixels are unavoidable the rescale
pass has to be told, and that is a decision, not a batch (see §6).

### 2.5 The GPU half (`lumit-render/src/gpufx.rs`)

`lumit-gpu` only dev-depends on `lumit-core` (docs/05), so the GPU dispatch table cannot
live beside the schema. It lives in `lumit-render`, which depends on both — one module for
all of them, not a file per effect, because each wrapper is a few lines — and is keyed by
the same `match_name`:

```rust
pub trait GpuEffect: Sync + 'static {
    fn match_name(&self) -> &'static str;
    fn run(&self, fx: &FxEngine, ctx: &GpuContext, tex: &Tex, w: u32, h: u32, p: Params<'_>)
        -> Tex;
}
```

`run_ops`' exhaustive match becomes a lookup. `every_migrated_effect_has_a_gpu_entry`
asserts the two registries agree: every schema that resolves has exactly one `GpuEffect`,
and every `GpuEffect` names a schema.

### 2.6 Registration is a list, not a `ctor`

`lumit-core/src/fx/catalogue.rs`, the whole of it:

```rust
macro_rules! catalogue {
    ($($m:ident => $t:ty),* $(,)?) => { … };
}

catalogue! {
    blur => Blur,
    directional_blur => DirectionalBlur,
    …
    lens_flare => LensFlare,
}
```

which expands to `pub const BUILTINS: &[EffectSchema]` (unchanged in type, so nothing
downstream moves) and `static DEFS: &[&dyn EffectDef]`. The order is the source order, so
the Add-effect menu is stable; there is no start-up cost and nothing runs before `main`.

`ctor` was the issue's original proposal and was withdrawn in the thread. The reason to
leave it withdrawn is not start-up time — it is that a `ctor` makes catalogue order depend
on link order, and the menu, the command palette and the preset browser are all
`BUILTINS`-driven (K-137), so an unstable order is a visible defect. Third-party effects
(OFX, docs/12) register at run time through the *same* `EffectDef` trait object, into a
registry that starts as the built-in list — which is the seam this refactor is really for.

## 3. What a frame does

1. `resolve_stack` walks the instances, and for each enabled built-in looks the def up by
   `match_name` and evaluates **every declared parameter** — including derived and spare
   ones (§4) — through the expression context, converting by declared unit. There is no
   per-effect resolve code: the loop is the same for all 35.
2. Host maths that used to sit in the `resolve_one` arm now sits in `pack`, called once at
   dispatch, on whichever side needs it.
3. `run_ops` looks up the `GpuEffect` and calls it with the bag.
4. The CPU ladder rung (K-019) and the parity tests call `apply_cpu` with the same bag.

Two effects (`posterize_time`, `accumulation_mb`) are orchestration-only and have no image
op; they declare no `GpuEffect`, which is exactly what the old `resolve_one` returning
`None` meant, said in the type system instead.

## 4. Parameters that are not in the schema

Two kinds, and they are stored the same way.

**Derived parameters** belong to an effect whose parameter *set* is a function of its own
state — a custom-shader effect whose uniforms come from the shader source, a node-graph
effect whose exposed inputs come from the graph. `EffectDef::derived(&inst)` returns them.

**Spare parameters** are the user's own: a slider they added to an effect (Houdini's spare
parameters; After Effects' Expression Controls) purely so other properties can read it
through an expression.

Both live on the instance, in `EffectInstance.extra` under a `dynamic_params` key, as a list
of `ParamSchema`-shaped records; both are ordinary `EffectParam` values otherwise, so they
keyframe, they serialise, they are visible to expressions by id, and the panel draws them
with the same row widgets. Four rules, which are the answers the issue thread arrived at:

1. **Nothing is removed automatically.** A shader edit that stops mentioning `u_wobble`
   leaves `u_wobble` on the instance, so the expression that reads it keeps working. A
   "Remove unused parameters" action removes them deliberately, and says what it will break.
2. **Nothing is added automatically either.** The derived set is offered, and adding it is
   an action. This keeps the document the source of truth: nothing re-derives a parameter
   list while a frame is being rendered.
3. **Keyframes outlive their parameter.** A parameter with no schema behind it is still a
   stored property; K-065's "keep what you do not understand" already covers it.
4. **The cache key covers them**, and this is the part that must not be got wrong: the key
   already hashes every `EffectParam` id and value (`lumit-eval`), so a dynamic parameter's
   *value* is covered for free. What is not is the *shape* — a shader source that changes
   which uniforms exist. So the key additionally feeds the derived set's ids and kinds, in
   order. The effect's `version` stays what it means today: the maths generation.

A spare parameter needs no shader at all, which is why the same mechanism gives us the
"slider effect" the thread wanted: an effect whose whole purpose is to hold values for
other properties to read. It renders as identity and declares `roi: Exact, cost: Trivial`.

## 5. Traps

- **Padding is not a value.** Never `bytemuck::bytes_of` a `Value`; feed the tag and the
  live fields. The frame key is the one place where a byte of uninitialised memory turns
  into a wrong picture that only reproduces on one machine.
- **`ParamId` collisions.** FNV-1a 64 over ids that are snake_case ASCII: a collision inside
  one effect's ~50 parameters is not a practical risk, but it is a *silent* one, so the
  catalogue test checks every built-in's ids for pairwise distinct hashes, and the dynamic
  path refuses to add a parameter whose hash collides with an existing one on that instance.
- **Order is a promise.** The bag is in schema order and `params` iterates in that order;
  the panel, the bridge and the cache key all rely on it. Sorting the bag by `ParamId`
  would be faster to search and would silently reorder the UI.
- **`pack` runs on both paths.** Host maths in `pack` must be identical for the GPU and CPU
  routes — that is the point of it — so it must not read anything the CPU path lacks
  (a device limit, an adapter feature).
- **A missing parameter is a default, not a fault.** `read` fills from the declared default,
  which is what makes K-258's backfill and old projects work. It must never panic
  (14-ENGINEERING-RULES §4) and never log per frame.
- **Two registries, one truth.** The `lumit-render` GPU table is keyed by `match_name`
  strings; a typo there is a missing effect at run time, not a compile error. The agreement
  test is therefore not optional.

## 6. Migration order

The old and new paths coexist for exactly as long as the migration takes, and no longer.

1. `params.rs`, the derive macro, `EffectDef`, the catalogue macro — with tests, no effect
   migrated. `BUILTINS` is still the hand-written list.
2. Migrate the catalogue in batches, simplest first (the colour family: `saturation`,
   `exposure`, `contrast`, `gamma`, `tint`, `invert`, `hue_shift`, `temperature`,
   `vibrancy`, `colour_balance`), each batch deleting its `Resolved` variants, its
   `resolve_one` arm, its `cpu::apply` arm and its `run_ops` arm.
3. The awkward ones last, and each on its own. `flash`, `scanlines` and `block_glitch` were
   on this list for their seam rather than their size — each derives a number from the
   *layer time* at resolve, and Flash also reads the §1.4 marker context and its Trigger
   property's whole keyframe track — and came off it when K-385 widened `EffectDef` with
   §2.4a's hook. What is left is blocked by something the bag cannot carry:
   - a **side table** threaded beside the ops and consumed by a counter that must stay 1:1
     and in order with them: `lut` (`luts[lut_i]`), `dof` (23 parameters, folded aperture,
     and a layer-input slot), `light_wrap` (a layer-input slot off the same `dof_i`
     counter), `lens_flare` (50 parameters, bakes, `flare_mattes[flare_i]`);
   - a **neighbour frame or flow field** off the layer's decode: `echo`, `motion_blur`,
     `datamosh`;
   - a **variable-shape payload**: `shake`'s nine sub-frame samples, which also fork the
     dispatch to a different kernel. `Value` has no array kind, so this is a decision.

   `matte_key` is on none of those counts — a flat bundle of scalars, colours and two
   normalised Choice codes — and is simply the next one to move.
4. Delete `Resolved`, `resolve_one`, `rescale_px` and the hand-written `BUILTINS` body.
5. Dynamic parameters, then spare parameters, then the panel affordances for both.

Each batch is a commit that leaves CI green, and the parity oracles (`lumit-gpu`'s
`*_matches_the_cpu_oracle`) are what prove a migrated effect still renders the same picture.
**No effect's maths changes in this refactor.** A commit that migrates an effect and changes
its output is two commits, and the second one needs a decision entry.

## 7. Test plan

Catalogue sweeps (these are the guards that the old arrangement lacked):

1. `every_builtin_declares_a_unique_match_name` — and a unique `ParamId` per parameter.
2. `every_schema_has_a_def_and_every_def_has_a_schema` — the core registry is a bijection.
3. `every_migrated_effect_has_a_gpu_entry` — the `lumit-render` table agrees with the
   catalogue. While the migration runs it is scoped to the migrated effects; when the last
   batch lands it covers `BUILTINS`, with the two orchestration-only effects named
   explicitly as the exceptions.
4. `every_parameter_declares_a_unit`, `only_spatial_values_rescale`,
   `a_migrated_spatial_parameter_rescales_as_the_old_op_did` and
   `the_stylise_family_rescales_once_in_each_unit` (all in `lumit-core/src/fx/tests.rs`) —
   the generic `rescale_px` replacement moves exactly the values the old match moved.
   Golden-tested against a table of the old behaviour: the first names every spatial
   parameter in the catalogue *and* its unit, so a new one has to be written down; the third
   drives the real `ResolvedOps::rescale_px` on a resolved blur and pins the radius the old
   `Resolved::Blur` arm would have carried; the fourth does the same for both units at once
   and pins that each is scaled **once** on the way in — a half-resolution resolve followed
   by a further halving is a quarter of the authored width, not an eighth.
5. `the_generated_schema_matches_the_hand_written_one` — during the migration only, and
   deleted with the last batch: for each migrated effect, `Effect::SCHEMA` equals the
   `BUILTINS` literal it replaces, field for field. This is what makes the port mechanical
   rather than a rewrite.
6. `a_missing_parameter_reads_its_default` and `an_unknown_parameter_is_ignored`.
7. `resolve_is_deterministic` — the same instance at the same time resolves to a
   byte-identical hash across two runs and two thread counts.

Dynamic parameters:

8. `a_derived_parameter_animates_and_serialises_like_a_declared_one`.
9. `removing_a_shader_uniform_leaves_its_parameter_and_its_expression_alive`.
10. `the_cache_key_changes_when_the_derived_shape_changes_and_not_when_it_does_not`.
11. `a_spare_parameter_is_readable_from_an_expression_by_id`.
12. `a_colliding_parameter_id_is_refused_at_the_edit`, not at render time.

Parity, throughout:

13. Every existing `*_matches_the_cpu_oracle` test in `lumit-gpu` passes unchanged. They are
    the actual acceptance criterion for the migration; they take the effect's own parameter
    struct, which `pack` now produces, so they need no rewriting.

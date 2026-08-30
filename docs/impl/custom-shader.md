# The Custom shader: a shader the user writes, and the graph that writes it

**Decision:** K-642 (commissioned by the owner, 2026-08-30). **Builds on:** K-381 and
[effect-registry.md](effect-registry.md) §4 (dynamic parameters — the settled rules this
note stands on; the panel affordances are the half that was owed), K-471 and
[node-graph.md](node-graph.md) (the driver graph the outer world keeps), K-263 (naga
validates every shader without a graphics card), K-624 (entering a precomp — the navigation
precedent for entering a shader), K-593 (a definition that can fail wears a calm badge),
K-129/K-065 (`.lumfx` preset files), K-031 (preview equals export).
**Feeds:** [08-EFFECTS.md](../08-EFFECTS.md), [12-PLUGINS.md](../12-PLUGINS.md),
[06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md).
**Status: CS1's engine half is built** (K-650, 2026-08-30) — the catalogue entry, the §1.4
grammar and its derived rows, the §1.3 assembler, naga validation and the §2.2 refusals, the
§2.3 NaN epilogue, the source-hash pipeline cache and the §3.2 last-good rule, and the §2.4
frame-key term. CS2 (the bridge), CS3 (the editor surface), CS4 (the inner graph) and CS5
(entry) are not. Four things K-650 settled where this note left a choice open, and which the
rest of the note should be read against:

- **The derived rows are `&'static [ParamSchema]`** from a session-lived parse cache keyed by
  the source hash, not owned records. §1.5's four rules are unchanged; what changed is that
  the rows go through the *existing* resolve loop rather than a second one.
- **The frame key folds the whole `extra.shader` block minus `origin`**, which covers §2.4's
  two terms with one and no parse. The source hash additionally rides in the resolved bag
  (pushed by `resolve_derived`), which is also how the assembled text reaches the GPU pass —
  and which puts the source in the K-421 per-effect cache key for free.
- **The prologue, the epilogue and the assembler live in `lumit-core/src/fx/shader/`**, not in
  `lumit-gpu`: the generated `Params` struct is a product of the parse, and `lumit-gpu` only
  dev-depends on `lumit-core`. An empty `Params` gets one placeholder member, WGSL having no
  empty struct.
- **The compile is synchronous** and §3.2's coalesced worker job is not built; there is no
  editor to feed it yet. The half that could not be added later is: the last-good fallback is
  off unless an interactive renderer turns it on, so export and headless never fall back.

## In plain terms

Every effect in Lumit is a little program that runs on the graphics card, and every one of
them was written by us. This note is about letting somebody else write one — a person with
an idea and twenty lines of shader code, who does not want to build a plugin, learn Rust,
or ask us first.

The **Custom shader** effect is one entry in the effects menu like any other. Drop it on a
layer and it does nothing, because it has no program yet. Open it and type one, or load a
file somebody sent you, and the layer starts doing whatever the program says. The clever
part is the controls: the program says which numbers it wants — a radius, an angle, a
colour — and those turn into ordinary rows in the Effect controls panel, with sliders,
keyframes, expressions and everything else a built-in effect's rows have. Nothing about
them is second class.

Phase two is the part the owner asked for by name. Typing shader code is a skill; wiring
boxes together is not. So the effect also holds a **graph** — the same kind of canvas the
Graph panel already draws, but inside the effect rather than beside it: boxes for the
picture coming in, boxes for adding and multiplying and mixing, a box for the picture going
out. Double-clicking the effect opens that inner graph, exactly the way double-clicking a
Precomp layer opens the composition inside it, with a breadcrumb to come back by. The graph
*compiles into* shader code, so the text view and the box view are two views of one thing.

Everything else in this note exists to make that safe: a program somebody sent you must not
be able to read your disc, must not be able to draw a different picture on your machine than
it drew on theirs, and must not be able to break the composition it sits in when it has a
typo in it.

---

## 1. The effect

### 1.1 One catalogue entry

```rust
/// Custom shader (docs/08 §3.x when it lands).
#[derive(Effect)]
#[effect(
    match_name = "custom_shader",
    label = "Custom shader",
    version = 1,
    category = Utility,
    cost = Heavy,
    roi = FullFrame,
)]
pub struct CustomShader {
    #[layer]
    pub input2: LayerRef,

    #[action]
    pub edit: Action,

    #[action]
    pub load_from_file: Action,

    #[slider(0.0..=1.0, default = 1.0)]
    pub mix: f32,
}
```

Four declared rows, and every other row this effect ever shows is **derived** (§1.5).

- **`roi = FullFrame`, `cost = Heavy`.** Both are statements about a program nobody has read.
  A shader may sample anywhere in its input, so no padding is honest and the whole frame is
  the only correct region; and its cost is unknowable, so it declares the class that makes
  the governor cautious rather than the one that makes it optimistic. This is not
  pessimism for its own sake: an ROI declaration that is *wrong* is a wrong picture, and a
  cost declaration that is wrong is a dropped frame. Only one of those is recoverable.
- **`mix` earns the injected Blend** (K-425), so a custom shader gets a Mix row and a blend
  mode for free, on the same seam every other effect uses. Nothing about that is
  shader-specific and none of it reaches the user's code.
- **The matte is `Strength`** — the default role (registry §2.5b). A shader could claim its
  matte and use it per pixel, but the claim is a fact declared in the schema, and this
  effect's schema cannot know what an arbitrary program means by "amount". The generic
  dissolve beside the dispatch is the honest answer, and the matte texture is *also* bound
  (§1.3) so a shader that wants to read it can — it just does not get to switch the dissolve
  off. A shader that reads the matte and is also dissolved by it applies it twice; that is
  the user's arithmetic, visible in their own code, and not a seam the host can get wrong.
- **`input2` is the one extra picture**, an ordinary layer reference riding the existing
  auxiliary-layer carriage (registry §2.5a, K-429 — `EffectSchema::layer_input()` finds the
  first `Layer` row that is not the matte). One, not many: the carriage is one slot per op
  walked by a shared counter, and a second would need a second counter and a second
  predicate on both sides of `build.rs`. If a shader genuinely needs three inputs, the
  answer is three shaders and a stack, which is what the stack is for.
- **Two `Action` rows** (K-417), because the source is not a parameter (§1.2). `Edit shader…`
  opens the editor surface; `Load from file…` opens a native file dialog and copies the text
  in. Neither carries a value and neither animates, which is exactly what an Action row is.

### 1.2 The source is instance state, not a parameter

The WGSL text lives on the instance, in `EffectInstance.extra`, under a `shader` key:

```jsonc
"shader": {
  "language": "wgsl",          // §5 reserves "glsl"; absent means wgsl
  "source": "…",               // the user's text, verbatim
  "graph": { … },              // §4, absent until the inner graph exists
  "origin": "C:/…/warp.wgsl"   // where it was loaded from, remembered for reload; never read at render
}
```

**Why not a parameter kind.** Three separate reasons, any one of which is enough:

1. **The parameter bag cannot hold it.** `Value` is `Copy`, borrows nothing, and is hashed
   field by field (registry §2.3). A kilobyte of text is none of those things, and widening
   the arena's slot to carry a pointer would give every parameter of every effect a lifetime.
2. **It is the thing the parameter set is derived *from*.** A parameter whose edit changes
   which other parameters exist is a different kind of object, and §4 of the registry note
   already has a home for that class of per-instance state.
3. **It does not animate.** Every parameter MUST be animatable (docs/08 §1.1); two shader
   sources cannot be interpolated and a hold-keyed source would be a second file-reference
   mechanism for something that is not a file.

`extra` is `#[serde(flatten)]`, so this rides through save, load, undo, copy/paste, the
`.lumfx` preset (§6) and an older reader (K-065) with no format work at all. It costs one
thing, and §2.4 is where that debt is paid: **the frame key does not hash `extra` today.**

### 1.3 The binding contract

The host owns every binding. The user's text declares none, and one that tries is refused
before it reaches naga (§2.2). Group 0, and the first five entries are the fx layout every
kernel already uses (`fx_gamma.wgsl` is the shortest example of it in the tree):

| Binding | Declaration | What it is |
|---|---|---|
| 0 | `var src: texture_2d<f32>` | the picture arriving at this effect, premultiplied scene-linear fp16 |
| 1 | `var orig: texture_2d<f32>` | the layer's picture before the stack — the Mix reference |
| 2 | `var dst: texture_storage_2d<rgba16float, write>` | the picture leaving |
| 3 | `var<uniform> lumit: LumitHeader` | the host's own header, fixed layout (below) |
| 4 | `var matte: texture_2d<f32>` | the K-395 matte, prepared at the seam; bound to `src` when there is none |
| 5 | `var input2: texture_2d<f32>` | the extra layer input; bound to `src` when unbound |
| 6 | `var<uniform> p: Params` | the user's own parameters, laid out by the host (§1.4) |

```wgsl
struct LumitHeader {
    roi_offset: vec2<u32>,   // gpu-foundation §4's common header, unchanged
    roi_size: vec2<u32>,
    comp_scale: f32,         // raster px per px@comp: 1.0 at full, 0.5 at Half
    time: f32,               // layer time in seconds — HANDED IN (§2.3)
    seed: u32,               // this instance's seed — HANDED IN (§2.3)
    mix_amt: f32,
    matte_on: f32,
    input2_on: f32,          // 0 when binding 5 is `src` standing in for nothing
}
```

**Two uniform buffers, not one.** The header's layout is fixed and the user's is generated,
and a single block would mean the host's own declaration could not be written until the
user's struct had been parsed. Splitting them lets the whole prologue be a constant string
with one generated struct spliced into it, which is the difference between a text assembler
and a text *compiler*. The cost is one binding and one 32-byte buffer per dispatch, against
an arena the engine already allocates per frame.

**The contract the user writes to is one function:**

```wgsl
fn shade(uv: vec2<f32>) -> vec4<f32>
```

Fragment-shaped, deliberately, because that is the model every person who has written a
shader before already has: one pixel in, one colour out, no neighbours, no order. It
compiles to a **compute** entry point, because that is what the rest of the engine is
(K-011, and the whole `FxEngine`); the fragment stage is the authoring model, not the
carriage. Nobody has to know that, and the note says it so the next reader does not
"discover" a fragment pipeline is missing.

`uv` is `(xy + 0.5) / raster_size`, spanning **the raster in play** — half the pixels at Half
preview. That is a trap with its own entry in §7; `lumit.comp_scale` is handed in for the
shaders that must not care.

**The assembled module**, in order:

1. **Prologue** (constant host text): `LumitHeader`; the lifted `Params` struct (§1.4); all
   seven bindings; the helpers.
2. **The user's text**, minus the `Params` struct that was lifted out of it, verbatim and
   unrewritten.
3. **Epilogue** (constant host text): the `@compute @workgroup_size(8, 8)` entry point that
   bounds-checks, computes `uv`, calls `shade`, sanitises (§2.3), applies `mix_amt` and the
   matte dissolve, and stores.

Lifting the user's `Params` struct to the top is what makes every helper, every binding and
every parameter visible to every line the user writes, in a language where a declaration must
precede its use. It is one rule with no exceptions: the host moves exactly one struct, the
one named `Params`, and when the user declares none it emits an empty one there instead.

**Helpers in the prologue**, so that nobody writes a wrong one:

| Helper | Does |
|---|---|
| `lumit_load(xy: vec2<i32>) -> vec4<f32>` | `textureLoad(src, clamped xy, 0)` — edge-clamped, so a sample off the frame is the edge rather than black |
| `lumit_sample(uv: vec2<f32>) -> vec4<f32>` | bilinear from `src`; there is no sampler in this layout, so the four loads and the two lerps are written once, here |
| `lumit_sample2(uv)`, `lumit_orig(uv)` | the same for bindings 5 and 1 |
| `lumit_matte(uv) -> f32` | the K-395 strength at that point (the Rec. 709 luma the seam already prepared) |
| `lumit_size() -> vec2<f32>` | the raster size in pixels |
| `lumit_px(uv) -> vec2<f32>` | uv → px@comp, i.e. `uv * lumit_size() / lumit.comp_scale` |
| `lumit_unpremult(c)`, `lumit_premult(c)` | the pair every colour operation needs (docs/08 §2.2) |

### 1.4 The declaration grammar

One struct, doc comments over its fields. This is the whole grammar, and it is pinned:

```wgsl
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Radius
    radius: f32,
    /// @bounded(0, 1) @default(0.5) Blend point
    blend_point: f32,
    /// @dial @default(0) Angle
    angle: f32,
    /// @counter(1, 16) @default(4) Steps
    steps: i32,
    /// @toggle @default(true) Invert
    invert: u32,
    /// @choice("Soft", "Hard", "Wrapped") @default("Soft") Edge
    edge: u32,
    /// @colour @default(1, 0.5, 0.2, 1) Tint
    tint: vec4<f32>,
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @seed Seed
    seed_v: u32,
}
```

**The WGSL type chooses the family; the annotation refines it.** This ordering matters,
because it means a field with no annotation at all is still a working parameter:

| Type | No annotation | With annotation |
|---|---|---|
| `f32` | `Float`, slider 0..1, no hard bound (docs/08 §1.2 — a slider may be typed past) | `@slider(lo,hi)`, `@bounded(lo,hi)` → the K-414 closed Slider, `@dial` → Angle |
| `i32` | `Int`, 0..100 | `@counter(lo,hi)` |
| `u32` | `Int` | `@toggle` → Bool, `@choice(…)` → Choice, `@seed` → Seed |
| `vec4<f32>` | `Colour` | `@colour` (the same thing, written out) |
| `vec2<f32>` | `Point`, **px@comp** (K-419) | `@point` |
| `vec3<f32>` | **refused** | — |

`vec3` is refused rather than padded silently, because a `vec3<f32>` in a uniform block
occupies sixteen bytes and reads back wrong the moment somebody assumes it occupies twelve
(§7). The message names `vec4` and says why.

**Units** are the registry's four and no others (§2.2 there): `@unit(px)` (px@comp, and the
one that rescales), `@unit(deg)` (implied by `@dial`), `@unit(s)`, and raw, which is the
default. There is no per-cent-of-the-diagonal unit and there never will be — `no_parameter_
is_a_per_cent_of_the_diagonal` is the standing gate and the derived path is held to it too.

**The label** is whatever is left on the line once the annotations are taken off, trimmed;
it is written in sentence case and shown as-is. A line with nothing left humanises the field
name (`blend_point` → "Blend point"). **The id** is the field name, unchanged: snake_case
ASCII, hashed to a `ParamId` by the same const FNV-1a every declared parameter uses, and
refused at the edit if it collides with an id already on that instance (registry §5, test 12
there).

**Defaults**, when `@default` is absent: `0`, `false`, the first choice, opaque white for a
colour, `(0, 0)` for a point, `0` for a seed. Every one of them is a legal value of its kind,
which is what keeps `read()`'s "a missing parameter is a default, not a fault" rule true.

**The grammar is parsed by a line reader in `lumit-core`, not by naga.** This is deliberate
and it is what keeps the panel working on a machine with no GPU and on a shader that does not
compile: the parameter list is derived from the annotated block, the picture is refused by
the validator, and the two failures are independent and separately reported. The reader finds
`struct Params {` at module scope, walks to the matching `}`, and reads `/// …` lines above
each field. It is a hundred and fifty lines of nothing clever, and it must never panic
(docs/14 §4) — a malformed annotation is a **skipped parameter with a message**, not an error
that costs the user the other eight rows.

### 1.5 How an instance's parameters serialise

Exactly as [effect-registry.md](effect-registry.md) §4 settled, with nothing added. The
derived set is offered; the stored set is the document's:

- `EffectDef::derived(&inst)` reads `extra.shader.source`, runs the §1.4 reader, and returns
  the `ParamSchema` list. It is called at edit time and at panel build, **not per frame** —
  the values are ordinary `EffectParam`s in `inst.params` by then, and resolve reads them
  through the same generic loop every declared parameter goes through.
- The four §4 rules apply verbatim and are worth restating because this effect is the reason
  they were written: **nothing is removed automatically** (an edit that drops `radius` leaves
  the row and the expression reading it alive); **nothing is added automatically** (the
  derived set is *offered*, and adopting it is an action, so no parameter list changes while
  a frame is rendering); **keyframes outlive their parameter**; **the cache key covers the
  shape as well as the values** (§2.4).
- The panel affordance those rules were waiting for is one row above the derived block:
  **Sync parameters** — live when the derived set differs from the stored set, naming the
  count each way ("3 new, 1 no longer used"), and adding the new ones. **Remove unused
  parameters** is the separate, deliberate action that takes the others away, and it says
  what it will break before it does it.

---

## 2. Validation and determinism

### 2.1 The validator is the one wgpu uses

`naga` 24 with `wgsl-in` is already a direct dependency of `lumit-gpu`, for K-263's
`every_wgsl_kernel_parses_and_validates`. The custom shader takes the identical road, on the
identical settings, because anything else would mean a shader that passes here and fails at
pipeline creation on a stranger's adapter:

```rust
let module = naga::front::wgsl::parse_str(&assembled)
    .map_err(|e| e.emit_to_string(&assembled))?;
let mut v = naga::valid::Validator::new(
    naga::valid::ValidationFlags::all(),
    naga::valid::Capabilities::empty(),   // what a stock device gives a kernel
);
v.validate(&module).map_err(|e| e.emit_to_string(&assembled))?;
```

`Capabilities::empty()` is the load-bearing argument: it is what the shipped kernels are held
to, and a custom shader that asked for more would compile on the author's machine and be a
black frame on somebody else's.

**The message the user sees is naga's own**, verbatim, with the line numbers **remapped** to
the user's text — the prologue's lines are subtracted, and an error that lands inside the
prologue or the epilogue is reported as "in the host's own wrapper" with the raw message
underneath, because that is a bug in Lumit and should read like one.

### 2.2 The refusal taxonomy

The node graph's dividing line (node-graph.md §1.5) holds here and is worth restating,
because a shader is the first thing in Lumit that can be *syntactically* wrong: **an edit
this application made is refused; a state some other entity's edit produced is degraded.**
A shader source is the user's own edit, so most of this table refuses — and the one that
degrades is the one that has to.

| What | Answer | Why |
|---|---|---|
| The user text declares `@group` or `@binding` | **refused at the edit**, before assembly | The host owns the bind group; a shader that binds its own would either collide or silently read a buffer it was never handed. A text-level check on the user block, so the message says "the host declares the bindings" rather than a naga duplicate-declaration error. |
| The user text declares a module-scope name in the reserved set — `src`, `orig`, `dst`, `matte`, `input2`, `lumit`, `p`, `Params` (except as the parameter struct), or anything starting `lumit_` | **refused at the edit** | Shadowing `p` would silently override the parameters with nothing; naga would not complain. Short list, pinned here, and quoted in the message. |
| No `fn shade(uv: vec2<f32>) -> vec4<f32>` | **refused at the edit** | The one thing the contract asks for. |
| A `vec3<f32>` parameter field | **refused at the edit** | §1.4; the padding trap, named rather than papered over. |
| Parse or validation error | **degrades**: calm badge with the message, and the last pipeline that compiled keeps running (§3) | This is the state a person is in for most of the time they are typing. Going black on every keystroke is punishment UI (docs/15). |
| A malformed annotation on one field | that parameter is skipped, with a message; the rest stand | A typo in a doc comment must not cost the other eight rows. |
| No source at all (a fresh instance) | identity passthrough, no badge | The K-111 rule for an unset file: a thing the user must supply cannot have a tasteful default, and an empty effect is not a failed one. |

**The badge already exists.** `effectBadgeRow` (K-593's `EffectDef::last_error`) draws a
reason key plus a verbatim detail, in the accent colour, never red, never modal — written for
a plugin that failed and shaped exactly right for a shader that will not compile. Two new
reason keys: *this shader did not compile* and *this shader is still compiling*. The
compiler's own sentence goes in the `detail` slot, untranslated, for the reason that slot
exists: it is somebody else's sentence about somebody else's code.

### 2.3 Nothing varies that the host did not hand in

WGSL has no clock and no random-number generator, so this is enforced by the language rather
than by us — which is a stronger guarantee than any check we could write, and worth saying
out loud because it is the reason a shader from a stranger is safe in a way a plugin binary
is not.

What the host hands in is therefore the *entire* source of variation, and both entries are
chosen to keep determinism the host's:

- **`lumit.time`** is the layer time in seconds at this frame — the same `f64` the resolve
  walk uses, narrowed once. It is already part of the frame's identity, so a shader that
  moves with it caches correctly with no new terms.
- **`lumit.seed`** is derived from the **instance id**, and is constant for the life of that
  instance. It is deliberately *not* a frame counter: a value that changed per frame would
  make two renders of one frame disagree and would put a different picture behind a cache
  key that had not moved. A shader that wants per-frame noise combines `seed` and `time`
  itself, in its own arithmetic, and stays a pure function of (document, frame).
- **Nothing else.** No adapter name, no wall clock, no render index, no dispatch order.

**NaN discipline is the host's job, in the epilogue.** A shader returning a NaN or an
infinity writes it into an `Rgba16Float` texture that the compositor, every effect above it,
the scopes and the exporter then read; one poisoned pixel becomes a black composition three
effects later and an export nobody can debug. So the generated entry point replaces every
non-finite component with zero before storing — two instructions, in host code the user
cannot remove:

```wgsl
var c = shade(uv);
c = select(c, vec4<f32>(0.0), c != c);              // NaN
c = clamp(c, vec4<f32>(-3.4e38), vec4<f32>(3.4e38)); // ±Inf, without clamping the picture
```

This is a trust boundary, not a nicety: the shader is untrusted input in exactly the sense
docs/12 §5 means, including when the user wrote it themselves.

### 2.4 Frame-key folding

`lumit-eval`'s per-effect loop hashes the namespace, the match name, the version, the
temporal opt-out and every `EffectParam`'s id and value at the frame's time. **It does not
hash `extra`**, so nothing about the source or the graph reaches the key today. Two terms
are added, inside that loop, under the `custom_shader` match name:

- **The source hash** — a 64-bit hash of `extra.shader.source` **as the document holds it**,
  not of the assembled module. The prologue and the epilogue are the host's, and when they
  change the *effect's `version`* changes, which is exactly what `version` has always meant
  (registry §4 rule 4: "the maths generation"). One term, one meaning each.
- **The derived shape** — the derived set's ids and kinds, in order. The *values* ride the
  ordinary `params` loop for free; what does not is a source edit that changes which
  parameters exist while every stored value stays put.

`layout`-style presentation state (`origin`, and §4's node positions) is **not** fed, for the
same reason `LayerGraph::layout` is not: moving a box changes no pixel.

---

## 3. Pipeline caching, and the last good pipeline

### 3.1 Keyed by source hash, not by instance

One compiled pipeline per distinct source, in a bounded LRU beside the `FxEngine`'s own
pipelines — 32 entries, through the governor's ledger like every other GPU allocation, and
an eviction is a recompile rather than a wrong picture. Two layers running the same shader
share one pipeline; two instances of one shader with different parameters share the pipeline
and not the uniform buffer, which is the split to get right and the subject of a test (§7).

The bind group layout is the custom shader's own — seven entries, not the shared fx five —
exactly as `fx-lut-pl` is its own today. Building it once at `FxEngine::new` costs nothing
and removes a per-compile branch.

### 3.2 A broken edit keeps the last good pipeline — interactively, and only there

This is the decision in §3, and it needs the argument spelled out because the obvious version
of it is wrong.

**What the user needs.** Editing a shader means being syntactically broken most of the time.
An editor that black-frames the composition on every keystroke is unusable, and it is
precisely the punishment UI docs/15 forbids. So while the source does not compile, the
Viewer keeps drawing the **last pipeline that did**, with the badge up and the compiler's
message under it.

**Why the naive version breaks determinism.** Compiling a shader is milliseconds to tens of
milliseconds — a frame's whole budget — so it cannot happen on the render path, which means
the pipeline in play depends on *when* a background compile finished. Two renders of the same
document at the same frame could then disagree, and worse, a frame drawn with the *old*
pipeline would be filed under the *new* source's key. That is a stale cache entry that
survives the edit, and it is the exact failure K-031 exists to prevent.

**The split that fixes it, in one line each:**

- The frame key always names the source **in the document**. It never knows or cares which
  pipeline was in play.
- A frame drawn with a stale pipeline is **rendered, shown, and discarded** — never entered
  into any cache tier. It is a picture on the screen with a badge over it, not a fact about
  the project.
- The **export and headless paths do not fall back at all.** They compile synchronously; a
  shader that does not compile renders as identity, the error goes in the export log, and the
  file is written. An export that silently used yesterday's shader would be worse than one
  that says the shader is broken.

So the stale picture is an interactive affordance with a label on it, and every cached and
every written frame is what the document says. One mechanism — compile off the render path,
keep the last good pipeline for the screen — answers both the typing problem and the
budget problem, which is why it is worth the paragraph.

**Compilation is one worker job per source hash**, coalesced: ten keystrokes queue one
compile of the tenth, not ten compiles. It carries the epoch token like every other job
(playback-scheduler.md), so a compile whose document has moved on is dropped at its next
checkpoint.

---

## 4. The shader node graph (phase two)

**Specified here so the effect stores it from day one.** Not built in the first wave; the
`graph` key exists in §1.2's shape from the first commit so that adding it later moves no
file format.

### 4.1 The graph is master when it is there

Two views of one thing, and exactly one of them is authoritative at a time:

| The instance holds | Master | The other |
|---|---|---|
| `source` only | the text | — (this is hand-written WGSL, and it is just text) |
| `source` + `graph` | the **graph** | `source` is the compiler's output, cached in the document so a build that cannot compile the graph can still render it |

**Why the graph and not the text.** The alternative — round-tripping edits both ways — is the
problem AE has with expressions and keyframes, and it does not have a good answer there
either. Compiling a graph to text is a total function; parsing arbitrary text back into a
graph is not, so a bidirectional model would either refuse most edits or lose the graph
silently. One direction, and the other direction is a deliberate act:

**Detach.** Editing the text of an instance that has a graph is refused, with one offer:
*Detach the graph* — which keeps the compiled text, drops the `graph` key, and leaves an
ordinary hand-written shader behind. It is one undo step and it is not reversible by another
button, which is the honest shape: the graph is gone because the user said so.

**The cached text is not trusted.** It is a convenience for reading and for `.lumfx` sharing;
the render compiles from the graph whenever a graph is present, and a mismatch between the two
is a stale cache to overwrite, never a conflict to resolve.

### 4.2 Entry: opening the box like a precomp

The owner's framing, and the navigation road is already built. **Double-clicking** the Custom
shader — the box on the Graph panel's canvas, or its heading in the Effect controls stack,
which are one selection (K-300) — opens the **inner graph in the Graph panel**, with a
breadcrumb back: `Wall › Custom shader`. Escape, or the breadcrumb's first crumb, returns.

K-624 is the precedent and the parts that transfer are named rather than assumed:

- **A composition remembers where you were, and so does a shader.** The inner graph's zoom
  and scroll come back when you re-enter it, in the session blob beside `compViews`, never in
  the document — standing somewhere in a graph is a way of working on it, not an edit to it.
- **Node positions are document data**, exactly as `LayerGraph::layout` is: the positions are
  the drawing, they travel with the file, and they are absent from the frame key.
- **The mapping that K-624 sends to the engine has no analogue here.** A precomp's entry
  needed `Layer::entry_time` because two comps keep different clocks; a shader graph has no
  clock of its own. Nothing crosses the bridge on the double-click but the instance id.

**The inner graph is a different kind of graph from the outer one, and reuses the canvas
rather than the model.** The layer's graph has an image chain that *is* the effect stack
(node-graph.md §1.1) and drivers hanging off it; the inner graph has no stack, no picture
chain and no drivers — it is a pure-function DAG whose wires carry numbers and vectors. What
is shared is the drawing: the dot grid, the node card with its shared header (a tick, a twirl and a
name — the same one an Effect controls heading wears), type-coloured wires and sockets, Tab
search, frame-all, the selected border.
Sharing the widget and not the document type is what keeps §1.1's honesty guarantee from
being quietly weakened by a second meaning for `Edge`.

### 4.3 Node vocabulary v1

Small on purpose. Every node is a pure function of its inputs and compiles to one WGSL `let`.

| Family | Nodes |
|---|---|
| **Input** | Picture (`uv` → rgba, from `src`), Second picture (from `input2`), Matte, UV, Time, Seed, **Parameter** |
| **Maths** | add, subtract, multiply, divide, modulo, mix, clamp, saturate, pow, sqrt, abs, sign, min, max, floor, ceil, fract, step, smoothstep, sin, cos, atan2, length, distance, dot, normalize |
| **Vector** | split (2/3/4 → scalars), combine (scalars → 2/3/4), swizzle |
| **Texture** | sample (a picture input + a uv → rgba, bilinear through `lumit_sample`) |
| **Colour** | luminance, premultiply, unpremultiply, tint, blend (the docs/08 §2.6 modes, one node) |
| **Output** | Result (exactly one per graph, rgba) |

**The Parameter node is how the inner graph declares a dynamic parameter.** One node, one row
in Effect controls; its own inspector carries the kind, range, default, unit and label — the
same five facts §1.4's annotations carry, because it compiles to exactly those annotations.
That closes the loop: the graph's parameters and a hand-written shader's parameters are one
mechanism with two front doors.

**Types** are `f32 | vec2 | vec3 | vec4` on every port, coloured by the node-graph's own
`PortType::Number` and `Colour` tokens where they map and one new token where they do not.
A scalar broadcasts to any width; two vectors must match. A mismatch is refused at the drop,
visually and op-free, exactly as the outer canvas refuses one.

### 4.4 Compilation

Topological order, ties broken by node id — the driver graph's rule (node-graph.md §2.2), and
for the same reason: a `HashMap` iteration order would make the emitted text vary between runs,
which would make the source hash vary, which would miss the pipeline cache and rename every
frame. **The emitted text must be byte-identical for a given graph, on every machine, for
ever.**

- One `let` per node, named `n<index>` in topological order. That is already common
  subexpression elimination by construction — a node feeding three others is evaluated once —
  so no optimiser is written and none is wanted.
- **No loops and no branches in v1.** `mix`, `step`, `smoothstep` and `clamp` cover what
  people actually reach a branch for, and every one of them is uniform-cost, which is what
  keeps the `cost = Heavy` declaration from being a lie by two orders of magnitude. Loops are
  growth, and they arrive with a bounded iteration count declared on the node, or not at all.
- A **cycle** is refused at the edit, in the same commit as the wire, with the same message
  shape the outer graph uses.
- The output is the text a competent person would have written: a `struct Params` with the
  annotations the Parameter nodes carry, and a `fn shade` of straight-line `let`s. That is
  what makes the text view of a graph worth looking at, and it is a test (§7).

---

## 5. GLSL, later

`naga` also ships a `glsl-in` front end, and turning it on is one Cargo feature. A `.frag`
somebody found on Shadertoy is the single most likely thing a user will try to paste in, so
this is recorded as **growth, not v1**:

- `extra.shader.language` exists from the first commit and holds `"wgsl"`. Nothing else reads
  it yet.
- When it lands, the road is identical: assemble, front-end, validate, badge. Only the
  front-end call and the prologue's text differ, and the §1.4 annotation reader works
  unchanged over a GLSL `uniform` block's doc comments, because it reads comments and field
  declarations rather than WGSL.

**Why not now.** naga's GLSL front end is partial by design and its failures are reported in
the vocabulary of a dialect Lumit does not otherwise speak — so the common outcome would be a
confusing message about somebody else's language, on a road we cannot fix. Shipping one road
that works completely is worth more than two that half do, and the second is cheap to add once
the first has proven the seam.

---

## 6. Sharing, and what this is not

**A saved custom shader is a `.lumfx` preset** (K-129, K-065) and needs no new file format.
`EffectPreset` serialises whole `EffectInstance`s, and `extra` is `#[serde(flatten)]`, so the
source, the graph and the dynamic parameters ride along today. It lands in the preset library
like any other, appears in the preset browser, and applies as one `SetLayerEffects`. A shader
sent as a bare `.wgsl` file is loaded by the Action row instead, and the **text is copied into
the instance** — the `origin` path is a memory of where it came from, never a thing the render
reads. A project must be one file that opens on another machine.

**Its relationship to LFX** (docs/12 §3), stated so nobody builds the wrong one:

| | Custom shader | LFX plugin |
|---|---|---|
| What it is | WGSL text in the document | native code in a bundle |
| Where it runs | in process, in the host's own pipeline, on the host's bindings | out of process, always, in its own device context |
| What it can reach | its seven bindings | whatever the sandbox and the extensions allow |
| Distributed as | a `.lumfx`, or a project | an installer |
| Failure | a calm badge and identity | a killed process, a calm badge and identity |

They are not competitors. A custom shader is what somebody writes in an afternoon for one
project; LFX is what somebody ships to strangers with a UI, a licence and a version number.
And the security difference is the one that matters: **a `.lumfx` from a stranger is data,
never code.** It carries text that naga validates and that can address nothing but its own
bind group — no filesystem, no network, no ambient authority of any kind — so docs/12 §5's
"opening a project executes nothing" survives shader sharing intact, which it would not
survive a native plugin arriving in a project file.

---

## 7. Traps

- **Uniform layout is the host's arithmetic, and it is easy to get silently wrong.** WGSL's
  uniform address space aligns `vec2<f32>` to 8 bytes and `vec3`/`vec4` to 16, and rounds a
  struct's size up to its largest member's alignment. The host generates the `Params` struct
  and uploads the matching bytes, so the two must agree exactly — one wrong offset and every
  field after it reads a neighbour's value, with no error anywhere. Two rules keep it honest:
  **declaration order is never changed** (registry §5 — the panel, the bridge and the key all
  rely on schema order), and padding is inserted as **explicit named fields** in the generated
  struct, visible in the text view, so a person reading the compiled shader sees the layout
  the host uploaded rather than one they have to infer.
- **`vec3` is sixteen bytes.** The most common form of the trap above, which is why §1.4
  refuses the type rather than handling it.
- **`dst` is a storage texture, not a render target.** `textureStore` only; no read of `dst`,
  no filtering, no blending. A shader that wants its own previous output wants Echo, or a
  second layer.
- **`src` is premultiplied, scene-linear fp16.** Not sRGB and not 0..1: a bright highlight is
  legitimately 8.0. A shader written against a Shadertoy assumption of 0..1 sRGB will look
  wrong, and that is not a bug in Lumit. The helpers are the mitigation; the note is the
  documentation.
- **`uv` spans the raster, not the composition.** At Half preview there are half as many
  pixels, so a shader whose look is a function of pixel *count* — a dither, a fixed-step
  march — will differ between preview and export, which is a K-031 failure the user authored.
  `lumit.comp_scale` and `lumit_px()` are handed in precisely so that a distance can be
  written in px@comp and be right at every resolution.
- **A `@unit(px)` parameter rescales; anything the shader derives does not.** The registry's
  §2.4a trap, inherited whole: values in the bag with a declared spatial unit are moved by
  `ResolvedStack::rescale_spatial`; a number the shader computes from `lumit_size()` is not.
  Prefer the parameter.
- **NaN** — §2.3. The one thing a shader can do that hurts something other than itself.
- **`extra` is invisible to the frame key** until §2.4's two terms are added. This is the
  single most dangerous line in the note: without them, editing a shader changes the picture
  and not its name, and the Viewer shows a cached frame from the previous source with no
  indication anything is wrong.
- **One pipeline, many uniforms.** Two instances sharing a source share the compiled pipeline
  and must not share the uniform buffer. Getting this backwards in either direction is a
  plausible bug with a very confusing symptom: either two identical-looking effects that
  ignore their own controls, or a recompile storm on a stack of eight.
- **The compile is not on the render path.** Anything that makes it synchronous on the
  interactive path — a "just this once" fallback, a cache miss handled eagerly — reintroduces
  a tens-of-milliseconds stall into a 16 ms budget, and it will be reported as a stutter with
  no obvious cause.

---

## 8. Test plan

**§1 — the effect and the grammar** (`lumit-core`):

1. `a_custom_shader_with_no_source_is_a_passthrough` — byte-identical to the input, no badge,
   no pipeline compiled.
2. `the_annotation_reader_derives_every_kind` — one struct with all nine forms of §1.4, over
   a golden `ParamSchema` list: ids, kinds, ranges, units, defaults and labels.
3. `an_unannotated_field_is_still_a_parameter` — the type-chooses-the-family table, one case
   per row.
4. `a_malformed_annotation_skips_one_parameter_and_keeps_the_rest`.
5. `a_vec3_parameter_is_refused_with_the_padding_reason`.
6. `the_reader_never_panics` — a fuzz over truncated, unbalanced and non-ASCII sources
   (docs/14 §4; this reads user text, so it is a parser at a trust boundary).
7. `a_derived_parameter_animates_and_serialises_like_a_declared_one` — registry §7 test 8,
   instantiated on this effect, which is the effect it was written for.
8. `removing_a_shader_uniform_leaves_its_parameter_and_its_expression_alive` — registry test 9,
   likewise.

**§2 — validation and determinism**:

9. `a_shader_that_declares_its_own_binding_is_refused_at_the_edit`, and the same for each
   reserved name and for a missing `shade`.
10. `a_compile_error_reports_the_users_own_line_number` — an error on line 3 of a source with
    a 40-line prologue reports 3.
11. `the_assembled_module_validates` (`lumit-gpu`, no GPU needed) — the prologue and epilogue
    round every fixture through the K-263 road, so a change to the host's own wrapper cannot
    ship broken. It is the same test `wgsl_validates.rs` already is, pointed at assembled
    sources.
12. `a_nan_returned_by_a_shader_never_leaves_the_effect` (`lumit-gpu`, GPU) — a shader whose
    `shade` returns `0.0/0.0`, asserted finite on readback.
13. `the_frame_key_changes_with_the_source_and_not_with_its_position` — edit one character of
    the source, key moves; move the node's canvas position, key holds.
14. `the_frame_key_changes_when_the_derived_shape_changes` — registry test 10 on this effect.
15. `a_shader_is_a_pure_function_of_frame_and_document` — the same frame rendered twice, and
    an export against a preview (K-031's matrix gains a custom-shader row), on a fixture whose
    shader reads `time`, `seed`, the matte and both pictures.

**§3 — caching**:

16. `one_pipeline_per_source_hash` — two instances, one source: one compile. Two sources: two.
17. `two_instances_of_one_source_keep_their_own_uniforms` — different parameter values, two
    different pictures, one pipeline.
18. `a_broken_edit_keeps_the_last_good_picture_and_raises_the_badge`.
19. `a_frame_drawn_with_a_stale_pipeline_is_never_cached` — the one that protects K-031;
    assert the cache is untouched across a broken-then-fixed edit, and that the frame after the
    fix is the new shader's.
20. `an_export_refuses_the_stale_pipeline` — a document whose shader does not compile exports
    as identity with the error in the log, never as the previous shader.
21. `ten_keystrokes_queue_one_compile`.

**§4 — the inner graph**:

22. `a_graph_compiles_to_byte_identical_wgsl` — the same graph, twice, on two thread counts,
    and against a golden string. This is the determinism gate for the whole of §4.
23. `every_node_in_the_v1_vocabulary_compiles_and_matches_a_cpu_evaluation` — one case per
    node, the graph's own value against the same arithmetic in Rust.
24. `a_cycle_is_refused_at_the_edit`, `a_type_mismatch_is_refused_at_the_drop`.
25. `a_parameter_node_becomes_a_row` — round trip: node → annotation → `ParamSchema` → panel
    row → stored `EffectParam`.
26. `the_graph_is_master` — with a graph present the render compiles from the graph even when
    the cached `source` has been tampered with in the file.
27. `detaching_a_graph_keeps_the_text_and_is_one_undo_step`.
28. Flutter: `entering_a_shader_shows_a_breadcrumb_and_escape_returns`; the inner graph's zoom
    and scroll come back on re-entry and are absent from the document.

**§6 — sharing**:

29. `a_custom_shader_round_trips_through_a_lumfx_preset` — source, graph and dynamic parameter
    values all survive save → load → apply, with fresh instance ids.
30. `an_older_reader_preserves_a_shader_it_cannot_run` — K-065's unknown-field rule, on the
    `shader` key.

---

## 9. Work packages

Ordered; each sized for one agent; each lands with its tests and its GUIDE paragraph (K-007).
CS1 → CS2 → CS3; CS4 needs CS1 and CS3; CS5 needs CS4.

### CS1 — The effect, the grammar and the pipeline

The catalogue entry (§1.1), the `extra.shader` shape (§1.2), the annotation reader and
`EffectDef::derived` (§1.4, §1.5), the prologue/epilogue assembler and the bind group layout
(§1.3), naga validation and the refusal taxonomy (§2.1, §2.2), the NaN epilogue (§2.3), the
frame-key terms (§2.4), the source-hash pipeline cache and the last-good rule (§3).
**Files**: `crates/lumit-core/src/fx/effects/custom_shader.rs`, a new
`crates/lumit-core/src/fx/shader/` for the reader, `catalogue.rs` (one line),
`crates/lumit-eval/src/lib.rs` (the two key terms),
`crates/lumit-gpu/src/fx/custom_shader.rs` + the assembler's prologue/epilogue,
`crates/lumit-render/src/gpufx.rs` (the wrapper).
**Tests**: §8 items 1–21.
**Not in this package**: any panel work. The effect is complete, headless, and testable
before a single widget exists — which is the point of doing it first.

### CS2 — The bridge: parameters per instance

The **owed call** from docs/TODO's registry section: `list_parameters` is per *effect*, and a
dynamic parameter is per *instance*. The seam is an instance-scoped read on the effect
handle — `EffectReference::list_parameters()`, answering the declared rows followed by the
instance's derived ones in order, with each row carrying whether it is declared or derived —
plus the writes the affordances need: `set_shader_source`, `sync_parameters`,
`remove_unused_parameters`, and `shader_status` (the badge's reason key and detail).
**Files**: `crates/lumit-bridge/src/api/effect.rs` (then codegen; generated files are never
edited), `docs/17-BRIDGE-CONTRACT.md`, `flutter_ui/lib/l10n/engine_labels.dart` +
`app_en.arb` (new keys listed in the commit message and the PR for translation, K-303).
**Tests**: an frb test driving edit → derive → sync → keyframe → undo; `engine_labels_test`
green; `bridge_call_budget_test` unchanged at 0 for rebuild paths — the derived list is
fetched on selection and on document change and cached Dart-side, exactly as `get_effects` is.

### CS3 — The editor surface

The text editor the `Edit shader…` action opens: a monospaced, line-numbered code surface
with the compiler's message anchored to its line, the badge over the effect, `Load from
file…`, and the Sync/Remove parameter affordances (§1.5). No syntax highlighting in v1 — it
is a want, not a need, and the error ribbon is the thing that makes the surface usable.
**Files**: `flutter_ui/lib/panels/shader_editor.dart`, the Effect controls Action rows, arb
keys.
**Tests**: widget tests for the error anchor, the sync affordance's counts, and the badge;
one gesture, one undo step per commit.

### CS4 — The inner graph and its compiler

The v1 vocabulary (§4.3), the DAG-to-WGSL compiler (§4.4), the graph-is-master rule and
detach (§4.1), stored under `extra.shader.graph`.
**Files**: `crates/lumit-core/src/fx/shader/graph.rs` and `compile.rs`, the bridge reads and
writes, `flutter_ui/lib/panels/shader_graph.dart` reusing the Graph panel's canvas widgets.
**Tests**: §8 items 22–27.

### CS5 — Entry

Double-click entry from both selection surfaces, the breadcrumb, Escape/back, and the
session-held view (§4.2), on K-624's road.
**Files**: `flutter_ui/lib/panels/graph_panel.dart`, the shell's breadcrumb, `SavedSession`,
arb keys.
**Tests**: §8 item 28.

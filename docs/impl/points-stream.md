# The points stream — implementation note

**Decision:** K-491 (the programme is commissioned; K-474 and K-475 confirmed), K-492 (a
points connection is a graph wire), K-494 (the v1 consumers), K-495 (points are 2D).
**Related:** K-446 (Particulate emits a points stream), K-471 (the wiring model), K-472
(the `Points` port type), K-419 (px@comp), K-031 (preview equals export), K-123 (layer
references). This note is the *how* for the points-stream programme's **infrastructure**:
how a stream is consumed, the first consumers, the evaluation contract, the seam, and the
ordered work packages. [particulate.md](particulate.md) remains the effect's own design —
parameters, closed forms, budgets, its test plan — and nothing here changes it beyond the
open questions it deferred to this step. [node-graph.md](node-graph.md) §6 holds the type
system this note builds on.

## In plain terms

Particulate makes particles, and hands them out twice: once as a **picture** (it draws
them over its input, like any effect), and once as **data** — the points stream, a list
of where every particle is this frame, how fast it is moving, how old it is, what colour
it wears. The picture is for looking at. The data is for *other things to use*: a wire
from Particulate's Points socket into another box makes that box follow the particles.

The question this design step settles is what such a wire *is*. The answer: exactly what
a driver's wire already is — a graph edge, drawn on the same canvas, coloured teal like
every geometry socket, refused calmly when it makes no sense. The picture's path is
still the effect list and nothing else; a points wire carries data alongside that path,
never the picture itself, so the plain list view never has to lie.

The first thing that consumes the stream, beyond Particulate's own drawing, is a new
driver: **Points sample**. It reads the stream and makes numbers from it — how many
particles are alive, and how far the nearest one is from a point you choose. Those
numbers drive parameters through the machinery drivers already have, so "the glow
brightens when a particle passes the lamp" is two wires and a Remap, with no new idea in
it. The rest of the family — Connect points, Clone to points, Trail, Scatter — waits,
each as its own named package, all plugging into sockets this programme leaves ready.

## 1. How a stream is consumed — the wire (K-492)

The Tier C2 question — graph wire versus reference parameter — is decided for the
**graph wire**, consistent with drivers and with the owner's ruling that the graph is
"both a second view of the effect stack *and* a way to wire effects into each other"
(K-445's resolution, K-471). A reference parameter would have been a second way of
saying the same thing with a second storage; the matte row's two-feed arrangement
(parameter *and* `SourceMatte` edge, one overriding the other) is precisely the
awkwardness not to repeat. A points connection has **one storage: the edge**.

### 1.1 The new edge arm

`OutputRef` gains its third arm — the first wire whose source is a *stack* effect:

```rust
enum OutputRef {
    Driver { node: Uuid, port: String },
    SourceMatte,
    /// A stack effect's declared data output — the first stack-sourced wire.
    /// The effect keeps making its picture for the chain; this taps the data
    /// it declares beside it (K-492).
    EffectData { effect: Uuid, port: String },
}
```

The destination is the existing `InputRef::Param` — for v1 always a *driver's* declared
data input (§2.2); when the family lands, also a stack effect's declared Points input.
No new `InputRef` arm.

### 1.2 The rules, and why the chain invariant holds

**A points wire is a data edge, never an image edge.** It does not reorder, branch, or
skip the image chain; `Layer::effects` remains the only authority for the picture's
path, and every image gesture still lowers to `SetLayerEffects`. The K-471 §1.1
invariant — every graph state has an honest stack rendering — survives because a points
wire renders in the stack view the same way a driven parameter does: the consumer's row
says what feeds it, by name. No carve-out is needed; the carve-out that *is* recorded is
the ordering rule below.

Validation (`LayerGraph::validate`, same refusals, same calm messages):

- **Type match**: the source port's declared type must equal the destination's —
  `Points` into `Points`, through the existing `PortTypeMismatch` refusal.
- **Same layer only**: edges never cross layers (K-471). A cross-layer points tap, if
  ever wanted, is a layer-reference parameter drawn as a derived source node — exactly
  Audio level's shape — and is deferred with the family.
- **One wire per input**: the existing rule, unchanged.
- **Downstream only, for stack consumers**: when the family's stack effects gain Points
  inputs, a points wire between two stack effects must flow *down* the stack — the
  producer strictly earlier in the list than the consumer. This is the recorded
  carve-out: the constraint exists so that when emit-from-image makes a stream depend on
  the producer's *input picture*, the stream at the consumer's position is already
  well-defined. A `SetLayerEffects` reorder that would invert a points wire **heals**
  (drops the edge) inside `prune_to`, the same rule as deleting the producer — the stack
  edit cannot be refused on the wiring's behalf (node-graph.md §3). Both halves landed
  with PS3 rather than waiting for the first stack consumer: the rule is positional, so it
  is answered from the two boxes' places in the list before any port is looked up, and
  landing it early means the healing path exists before anything can reach the state it
  prevents. The refusal is the existing `Cycle` sentence — a wire drawn back up the stack
  is asking for the consumer's own output as part of its input.
- **The cycle check walks through effect data sources.** v1 makes a genuine cycle
  constructible: Points sample reads Particulate's stream, and its Count output is wired
  into Particulate's Emit rate — the stream depends on the parameters, the parameters on
  the stream. `check_acyclic` therefore grows effect nodes into its link set: an
  `EffectData` edge contributes (Effect → destination driver), and a driver-into-effect-
  parameter edge contributes (driver → Effect). Kahn's walk over drivers *and* effects;
  anything left is a `Cycle`, refused at commit like every other loop.

`prune_to` gains the source arm: its current comment — "a wire's *source* is a driver or
the layer's own alpha, neither of which the stack can remove" — becomes false the moment
`EffectData` exists, so removing an effect drops the edges that **source from** it as
well as the ones that point at it.

### 1.3 Resolve-time meaning

Because commit-time validation refuses cycles, the demand-driven driver walk
(node-graph.md §2.1, `fx/drivers/mod.rs`) stays well-founded when an `EffectData` wire
appears in it: evaluating the wire evaluates the producer's stream (§3.3), which
resolves the producer's parameters *with their own driver substitutions applied* — the
sampled stream must be the same stream the picture draws, or the driver would report a
particle field the viewer cannot see. Acyclicity is what makes that recursion terminate.

## 2. The v1 consumers (K-494)

The owner's commission — "the point stream type stuff that we can use to **drive things
and render** etc" — names both halves.

### 2.1 Render: Particulate's own modes

The render half is Particulate's three instanced modes — disc, sprite, streak — exactly
as [particulate.md](particulate.md) §2 (Render group) and §6 specify. Nothing to add
here; PS1 and PS2 build them.

### 2.2 Drive: the Points sample driver

One new driver in the Drivers category, the first consumer of a points wire and the
proof of the seam:

| | |
|---|---|
| **Data input** | `points` — "Points", `PortType::Points`. Wire-only: declared in the signature, not the schema — there is no stored value, nothing to keyframe, no panel row. Unwired reads as an empty stream, the documented no-op. |
| **Parameter** | Position — 2D point, px@comp, default comp centre, animatable and drivable, with the pick-on-Viewer dropper. The query point. |
| **Outputs** | Count — "Count", number: live particles this frame. Nearest distance — "Nearest distance", number: px@comp from Position to the nearest live particle. |
| **Empty stream** | Count is 0; Nearest distance is 1e9 — "nothing is anywhere near", which is the honest direction for the wire's typical use (Remap → nearness drives a value). Documented here, pinned by test. |
| **Temporal window** | 0 — pointwise. The stream at the frame is all it reads. |

The nearest-particle search is a linear scan over the live set — bounded by the
producer's cap (K-475), a few hundred microseconds at the default 20 000 even on the
CPU, and deterministic. (ponytail: O(n) scan; a grid or kd-tree only if a profile ever
shows a real graph spending it.)

**Why a driver consumer is legal at all**: driver evaluation is CPU work at resolve
time, before any pixel exists — and a v1 stream is a pure function of the document and
the time (K-474), never of the input picture. **Emit-from-image breaks that**: a
luminance-weighted birth schedule makes the stream depend on the producer's input frame,
which does not exist at resolve time. The emit-from-image package therefore owns a
recorded constraint: when it lands, a stream from an image-dependent emitter either
refuses driver-side sampling or the emit weighting is declared image-independent for
sampling purposes — that package decides, with its own appended entry. Named, not faked.

### 2.3 Deferred, as named packages

Each of these is a future work package with its own design step, all consuming the
contract in §3 unchanged: **Connect points** (lines between near particles — plexus),
**Clone to points** (a layer instanced per particle, generalising Sprite mode),
**Trail** (history drawn from closed-form back-evaluation, like Streak but longer),
**Scatter** (an image broken to points and displaced), **Emit-from-image** (with §2.2's
recorded constraint), **cross-layer points taps** (§1.2). None blocks, none is built.

## 3. The evaluation contract

### 3.1 The layout, confirmed — and 2D (K-495)

particulate.md §4's `PointsStream` layout is **confirmed as finalised**, including the
`life` buffer and id-is-birth-index. Positions and speeds are **`Vec2`**: the effect is
2D, the whole v1 family is 2D, and 2.5D points remain the recorded growth path
(node-graph.md §6.2) — `position`/`speed` grow to `Vec3` if and when 2.5D is decided,
which changes buffer strides and nothing in this note's contracts. Deciding `Vec3` now
would mean a dead z lane in every buffer, every kernel and every test for a feature with
no decision behind it.

The stream exists in two forms with one meaning:

- **GPU form**: the SoA buffers of particulate.md §4, written by the evaluate/compaction
  passes, read by the instanced draw and (later) by stack consumers.
- **CPU form**: the same attributes in plain `Vec`s, produced by the shared closed-form
  module — the K-019 reference oracle, **and** what the Points sample driver reads.
  One module (`fx/points.rs`), two callers; a second implementation of the closed forms
  would be a drift waiting to be found.

### 3.2 Equality and determinism

- CPU and GPU stream attributes agree to **10⁻⁵ of each attribute's own range**
  (particulate.md §5 — data has no perceptual tolerance; K-508 corrected the measure from
  the ≤ 2 ULP this note first asked for, which no GPU can meet against libm); pixels agree
  to the `moderate` perceptual epsilon.
- Compaction is a prefix sum in birth-index order, never atomics (particulate.md §5), so
  `id` order is a fact, not a scheduling artefact.
- The Points sample driver reads the CPU form, so its numbers are bit-identical across
  machines and renders by construction, and export equals preview (K-031) with no new
  argument.

### 3.3 Where the stream lives, per frame

**v1**: the GPU stream never leaves Particulate — the evaluate, compaction and draw
passes share buffers inside the effect's own op, from the governor's pools, sized by the
cap (K-475). The only cross-effect consumer is the driver, which evaluates the CPU form
inside the driver walk: the walk gains the layer's stack and timing context (in point,
comp rate) so an `EffectData` wire can resolve its producer, and the evaluated stream is
**memoised per producer within one frame's walk** so two wires from one Particulate cost
one evaluation. The birth-schedule scan is cached exactly as particulate.md §3.1 says,
shared by both forms.

**The family's carriage, designed now, built with its first consumer**: when a stack
effect declares a Points input, the frame's arena keys the producer's compacted SoA set
by `(layer, effect instance id)` beside the intermediate textures; the draw builder
threads a consumer's points input as a `PointsInputDraw { producer }` resolved at build
time — the same shape `LayerInputDraw` threads a matte texture (K-288,
layer-input.md). Lifetime is the layer's render scope, released with the frame arena.
Budget accounting: the buffers are the **producer's** declared peak scratch (the cap,
docs/13 §6); a consumer declares nothing for reading them.

### 3.4 Cache keys

**No new terms.** Particulate's key is the standard formula — parameters hash as usual,
`seeded` folds the layer's local time (K-474). A points-wired layer is already covered
by node-graph.md §2.3's corrected folding: the wires fold (an `EffectData` edge is an
edge like any other), the layer time folds, the driver declarations fold, and the
producer's stored parameters were always hashed with the stack. The frame-key
sensitivity tests in PS4 pin it: cutting the points wire changes the key; editing the
producer's Emit rate changes the key; an unrelated layer's edit does not.

## 4. The seam and the UI

### 4.1 The registry

`Signature` grows both ways, defaults keeping every existing declaration untouched:

```rust
enum Signature {
    /// A picture operation; `extra` is its declared data outputs — empty for
    /// every effect but Particulate (particulate.md §4's registry shape).
    Image { extra: &'static [Port] },
    /// A driver: declared data inputs (wire-only, no stored value) and outputs.
    Data { inputs: &'static [Port], outputs: &'static [Port] },
}
```

The default `signature()` returns `Image { extra: &[] }`; `Signature::outputs()` returns
`extra` for `Image`, so the bridge and the validator read one method whichever kind an
entry is. `LayerGraph::validate`'s `output_type` gains the `EffectData` arm (look the
effect up in the stack, ask its signature); `input_type` on a driver checks the
signature's data inputs beside the schema params, so `InputRef::Param` needs no new arm.

### 4.2 What crosses the bridge

- `BridgeOutputRef` gains the matching `EffectData { effect, port }` arm.
- `read_layer_graph` appends `def.signature().outputs()` to an effect box's outputs
  (today hardcoded to the one image `OUTPUT_PORT`), so Particulate's box grows its teal
  socket with no Particulate-specific code at the seam; `catalogue_ports` already reads
  the signature and needs only the driver-inputs half.
- Driver boxes append their signature data inputs to their parameter sockets.
- Refusals cross as the existing calm sentences; the cycle message already exists.
- **K-005/K-303**: the port label "Points" is a new engine-sendable word — its
  `engine_labels.dart` entry and `app_en.arb` key land with the seam package, listed in
  the commit and PR for Crowdin. Particulate's own labels land with PS1, Points sample's
  with PS4, the moment each enters the catalogue (the label walk fails CI otherwise).
- **No stream introspection in v1.** A live particle-count readout on the node or in
  the Node preview would be a per-frame value, which must ride a render response —
  never a bridge call from a rebuild path — and its natural home is the Details
  inspector (the PLAN's B4) when that exists. The Node preview shows Particulate's
  *picture* already, since it is a stack effect; a count readout is named future work.

### 4.3 The panels

- **Graph panel**: the teal socket goes live. Drag-to-wire, the Tab search's type
  filter, and mismatch refusal all come free from the existing type machinery once the
  bridge reports the ports; the only new panel knowledge is nothing at all — teal was
  drawn from `PortColours` since WP1.
- **Effect controls**: Particulate's parameter surface is particulate.md §2 through the
  ordinary schema — groups as kickers, the two over-life curves (K-412), the seed row
  with reseed (docs/08 §2.4), the mask-path reference (K-408), the sprite layer
  reference with the standard source combobox (K-142), the Mix row with Blend (K-425).
  No new row kinds.
- **Node panel**: Points sample renders like any driver — its Position row, its wired
  Points input shown as a socket, its outputs as sockets.

## 5. Work packages

Ordered; each sized for one agent; each lands with its tests (K-007) and its GUIDE.md
plain-English addition where a new mechanism appears. PS1 → PS2 and PS1 → PS3 → PS4 →
PS5 → PS6 → PS7; PS2 may run in parallel with PS3–PS4 once PS1 lands. Binding documents
per package are listed; fresh-read the K numbers cited before appending anything.

### PS1 — Stream core and the closed-form Particulate (engine, CPU)

The shared points module `crates/lumit-core/src/fx/points.rs`: the CPU `PointsStream`
(SoA `Vec`s per §3.1), the birth schedule and its cache (particulate.md §3.1), the
closed forms and series guard (§3.2), turbulence through the **shared** noise
(`fx/noise.rs` — do not invent a second lattice), per-particle draws by
`hash(seed, b, attribute_id)`, the cap rule (newest `cap`). The `Signature` split of
§4.1 (defaults untouched for every existing entry). The Particulate effect
`crates/lumit-core/src/fx/effects/particulate.rs`: schema per particulate.md §2, traits
per §2's Traits block, the declared Points output, and the **CPU render path** — discs
and streaks as software dabs (the paint rasteriser's precedent), sprite via the
layer-input machinery (layer-input.md), unset-sprite-draws-discs.
**l10n**: Particulate's label, group kickers and every parameter label —
`app_en.arb` + `engine_labels.dart` in the same commit, keys listed for Crowdin (K-303).
**Tests**: particulate.md §9 items 1–7 and 9–11 on the CPU path (determinism, random
access, schedule against closed-form counts, force closed forms across the guard,
turbulence golden values, id stability, cap rule, mask-path no-op, sprite fallback,
frame-key sensitivity); registry-agreement suite green with a `Signature::Image` that
now carries a field; the ninety existing declarations untouched.
**Binds**: particulate.md §2–§5, this note §3–§4.1.

### PS2 — GPU evaluate, compaction, instanced draw

The WGSL twin of PS1's closed forms (one thread per candidate), prefix-sum compaction in
birth-index order, the instanced quad raster for disc/sprite/streak, sprite texture
threading as a matte's is, the host Blend/Mix seam (K-425), buffers from the governor's
pools sized by the cap, cancellation between evaluate and draw, the halving degradation
rung (interaction-only, status readout, never on export).
**Files**: `crates/lumit-gpu` (WGSL + pipeline), `crates/lumit-render` (draw building).
**Tests**: particulate.md §9 item 8 (CPU/GPU twins — pixels within `moderate` epsilon,
stream attributes within 10⁻⁵ of their range, K-508), item 1's export-equals-preview fixture joining the K-031
matrix, item 7's degradation-never-on-export render test. **docs/08 gains its §3.x
Particulate entry in this package** — the effect is whole here — derived from
particulate.md, and docs/13 §7.3 gains the four budget rows (gated in PS7).
**Binds**: particulate.md §5–§7, docs/06 §1.2, docs/13 §6.

### PS3 — The points edge

`OutputRef::EffectData`, `Signature::Data`'s `inputs` half of §4.1 (PS1 landed the
outputs), `LayerGraph::validate`'s new arms (§1.2: type via signature, the downstream-only
rule, cycle check through effect data sources), `prune_to`'s source arm and its
inverting-reorder heal (and its corrected comment), serialisation round-trip. The bridge
half of the edge: `BridgeOutputRef::EffectData`, effect boxes reporting signature extra
outputs, driver boxes reporting signature data inputs, `catalogue_ports`, docs/17 §"The
layer graph" gaining the arm.
**Tests**: type-mismatch and cycle refusals including the Particulate ← Points sample ←
Particulate loop of §1.2; prune on producer delete; JSON round-trip; the read model
showing Particulate's teal output socket; old-file load → byte-identical re-save
(standing invariant).
**Binds**: this note §1, §4.1–§4.2; node-graph.md §1.5, §3.

### PS4 — The Points sample driver

`crates/lumit-core/src/fx/drivers/points_sample.rs` per §2.2 (the `Signature::Data` inputs
it declares into landed with PS3);
the driver walk gaining the layer's stack and timing context, the `EffectData` arm in
`Eval`, and the per-producer per-frame stream memo (§3.3); `driver_window` 0; the
empty-stream values pinned.
**l10n**: Points sample's label, Position, Count, Nearest distance — arb +
`engine_labels.dart`, same commit, Crowdin-listed.
**Tests**: the sampled stream equals PS1's drawn stream **under driven producer
parameters** (a Wiggle on Emit rate, then sample — the §1.3 property); unwired and
empty-stream values; nearest-distance against a hand-placed fixture; frame-key
sensitivity (§3.4's three cases); a points-driven row joining the K-031 matrix (driven
picture differs from the wire-cut picture, and preview equals export).
**Binds**: this note §2.2, §3.3–§3.4; node-graph.md §2.

### PS5 — Seam and codegen

Whatever PS3–PS4 added to `crates/lumit-bridge/src/api/**` finishes crossing: codegen
rerun (generated files never edited), the dylib rebuilt for the frb tests, the port
label "Points" through `engine_labels.dart` + `app_en.arb` (Crowdin-listed), docs/17
checked against the shipped surface.
**Tests**: `engine_labels_test.dart` green; an frb test wiring Particulate's Points
output into a Points sample and reading the graph back; `bridge_call_budget_test.dart`
unchanged at 0.
**Binds**: this note §4.2; docs/17.

### PS6 — UI

Graph panel: the live teal wire end to end (drag from the Points socket, Tab filtered to
Points-accepting entries, mismatch refusal, one gesture one op); Effect controls:
Particulate's surface verified against particulate.md §2 (curves, reseed, mask path,
sprite combobox, Mix/Blend) — expected mostly free from the schema, the package is the
verification and any missing row kind; Node panel rows for Points sample.
**Tests**: widget tests — the wire gesture commits one `SetLayerGraph`; the Tab filter;
Particulate's rows render with the curve and seed rows present; 0 bridge calls in
rebuild paths. Run only the affected test files.
**Binds**: this note §4.3; particulate.md §2; the NodeGraph drawing (K-458).

### PS7 — Conformance, goldens, budget gates ✅ landed

Golden frames for the three render modes at pinned seeds; the four K-475 numbers as perf
-harness gates on the reference-desktop runner (docs/13 §7.3): default look ≲ 0.2 ms,
20 000 discs ≤ 1 ms desktop / ≤ 4 ms laptop, the 1 000 000 hard cap ≤ 16 ms with the
cancellation check, degradation determinism (newest `cap/2`).
Also **the clamp question K-509 left open**: whether a driven value should be clamped
to its parameter's hard range engine-side, as a typed value is. It is not today, which is
what makes an unwired Points sample show up as a parameter pinned *past* its limits
rather than at them; PS6 marks the cause in the panel and deliberately does not treat the
symptom. It touches every driver rather than this one, and it changes rendered output, so
it is answered here — with its own appended entry — or recorded as deliberate.
**Tests**: particulate.md §9 item 12; the golden suite.
**Binds**: particulate.md §7, §9; docs/13 §7.3; docs/16's verification-beats-assertion
standing rule; K-509.

**What landed.** Four things, and one of them changed a number in docs/13 rather than
meeting it:

- **The goldens** are `crates/lumit-render/particulate-golden.txt`, regenerated on purpose
  like `fx-reference.json`: the pinned fixture's ids (exact), its eight stream attributes,
  and the CPU reference's drawn frame in each of disc, sprite and streak. Everything is
  held to K-508's 10⁻⁵-of-range bound. Two tests read it — the CPU one, which needs no
  graphics adapter and so gates on every runner there is, and the card's, which puts the
  `particulate_stream` read-back against the same numbers. The existing twin test says the
  two paths agree with each other; the goldens say what they agree *on*, which is the one
  failure a twin test cannot see.
- **The budget gates** are three scenarios in `lumit-bench` (`scenarios::particulate`),
  which is how every other B-number gates: emit a `Measurement`, ride the checked-in
  per-OS baseline and its ratio factor, assert the absolute budget only under
  `LUMIT_REFERENCE_HW=1`. They time one pass at 1080p rather than a comp, so they need no
  ffmpeg and no reference comp.
- **B12's resolution is recorded in docs/13 §2's row, not hidden**: the three numbers are
  measured **above the pass floor** — the same call with nothing to emit, which is one
  full-frame copy and one round trip to the queue. Measured whole, B12 was 0.266 ms against
  ≲ 0.2 ms with 0.062 ms of that being the copy; the floor is real work but it is the
  frame's, not the effect's. §7.3's other convention then applies unchanged: B12 and B13
  are under a millisecond, so the ratio gate stays quiet on them.
- **The clamp is answered, K-510**: a driven value is held to its parameter's hard range at
  the substitution in `resolve_into_arena` — which is the walk both the preview and the
  export take, so the two clamp identically by construction. On an **effect's** sockets
  only: a driver's own row exists to be handed numbers from outside its range, and Remap is
  the proof. K-509's "no stream" mark stays exactly as PS6 built it; this is the backstop
  beneath it, not a replacement for it.
- **The degradation rung** is pinned as the guarantee it is:
  `particulate_exports_its_whole_declared_field` holds the export walk's picture identical
  to the interactive one and different from a comp declaring half the cap.

## 6. Test plan — the core invariants

Beyond the per-package tests, the properties a regression would betray:

1. **One stream, two readers**: the driver's sampled stream and the drawn stream agree
   under any parameter/driver configuration (PS4's central test).
2. **Determinism**: a Particulate comp renders bit-identically twice; export equals
   preview with the points wire live (K-031 rows from PS2 and PS4).
3. **The chain is still the list**: node-graph.md §9's property test extended over
   graphs containing `EffectData` edges — the effect list order still equals the image-
   chain order the read model reports.
4. **Old files are untouched**: pre-points fixtures load, save, byte-compare (standing).
5. **Refusal, not corruption**: the cycle and mismatch fixtures of PS3 never reach the
   document.

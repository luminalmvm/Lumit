# The node graph — implementation note

**Decision:** K-471 (the stack stays the spine; a layer gains an additive driver graph),
K-472 (port types, wire colours, the points stream), K-473 (the selected node border).
**Related:** K-445 (the graph is a second view that can also wire), K-446 (Particulate
emits a points stream), K-448 (the Node preview is its own panel), K-458 (the drawing is
authoritative), K-381 (the effect registry), K-395 (the uniform matte row), K-142 (matte
sources), K-305 (expressions). This note is the *how* for the whole of redesign phase 3:
model, ops, bridge surface, migration, the points-stream type, and the ordered work
packages. The approved **NodeGraph** and **Nodes-workspace** drawings bind the two new
surfaces; where this note and a drawing disagree, the drawing wins (K-458).

## In plain terms

Today a layer's effects are a list: the picture goes in at the top, each effect changes
it, and the result comes out at the bottom. That list is simple, and it stays. The Graph
panel draws **the same list** as boxes joined left to right by wires — the picture's path
made visible. What the graph adds is a second kind of box: a **driver**. A driver makes
no picture; it makes a *value* — a wobbling number (Wiggle), the loudness of the music
(Audio level), a slowly turning colour (Colour cycle) — and a wire from a driver into an
effect's socket makes that effect's parameter follow the value instead of its keyframes.
"The glow pulses with the music" becomes one wire you can see, instead of an expression
you have to write.

The important design decision is what the *saved project* holds. It still holds the list,
exactly as before — old projects open unchanged, and everything that edits the list keeps
working. Alongside the list, each layer can now also hold its drivers and their wires. A
project that never opens the Graph panel never has any. And because the picture's path is
always the list, everything the graph shows can always be shown honestly in the ordinary
Effect controls too — a driven parameter's row simply says *driven* and names the driver.
There is no graph you can build that the stack view has to lie about.

## 1. The model

### 1.1 The one rule

**`Layer::effects` remains the only authority for the image chain.** The Graph panel
derives its image-path nodes — the Source node, one node per `EffectInstance` in stack
order, the Layer out node — from the list, and every image-wire gesture lowers to the
existing whole-stack `SetLayerEffects` commit:

- dropping a node onto a wire = insert at that index (auto-wire is this, automated);
- deleting a node with Heal on = remove at that index — the list heals by construction;
- rewiring the chain = reorder.

An effect's main **Input** port accepts exactly one wire and it is, by construction, the
previous stack entry's Output. The panel never offers a gesture that would branch or skip
the image chain (dropping an Output on an occupied Input re-routes; it never fans out).
This is the honesty guarantee: every graph state has a stack rendering, because the image
chain *is* the stack. Merging image chains is what layers and blend modes already do; a
compositing merge node would be a new decision, deliberately not taken here.

### 1.2 What the document gains

```rust
struct Layer {
    // ...
    graph: LayerGraph,          // K-471; empty by default, absent from the file when empty
}

/// The additive wiring a layer carries beside its effect stack.
struct LayerGraph {
    nodes: Vec<EffectInstance>,        // drivers — same struct, Drivers registry (§1.3)
    edges: Vec<Edge>,                  // §1.4
    layout: Vec<(NodeRef, [f64; 2])>,  // canvas positions; missing entries auto-place
}

/// Names anything the canvas draws. Stack-derived nodes get stable synthetic refs.
enum NodeRef {
    Source,                    // the layer's own source (image + matte outputs)
    Effect(Uuid),              // an EffectInstance in `effects`
    Driver(Uuid),              // an EffectInstance in `graph.nodes`
    Out,                       // the layer's output
}

struct Edge {
    from: OutputRef,           // Driver(Uuid, PortId) | SourceMatte
    to: InputRef,              // Param(NodeRef, PortId) | Matte(Uuid /* effect */)
}
```

Positions are document data (they persist and travel), edited through the same commit as
everything else in the graph; a drag is staged and commits once, the K-344 pattern.

### 1.3 Drivers reuse the effect registry

A driver is an `EffectInstance` whose `EffectDef` declares a **data signature** instead of
an image kernel — no WGSL, no CPU pixel path, a scalar/colour/analysis function evaluated
at resolve time. The K-381 registry machinery carries them for free: one file each under
`lumit-core/src/fx/effects/` (or a sibling `drivers/` module), schema-declared parameters,
catalogue generation, `list_parameters`, the Effect-controls row rendering. The registry
gains one field:

```rust
enum Signature {
    Image,                                     // today's effects (implicit until now)
    Data { outputs: &'static [(&'static str, PortType)] },
}
```

The v1 driver set is the six the drawings show, in a **Drivers** catalogue category:

| Driver | Inputs | Outputs | Notes |
|---|---|---|---|
| **Wiggle** | Amount, Frequency (number) | Value (number) | Deterministic value noise seeded by the node's id, sampled at layer time. Same recipe as the expressions' wiggle if one exists when built; otherwise pin the noise in this note's test plan. |
| **Audio level** | Audio (layer reference) | Amplitude, Low (number) | Windowed RMS of the referenced layer's decoded audio at layer time; Low is the same over a low band. Deterministic per (audio fingerprint, time, window). |
| **Colour cycle** | Phase (number) | Colour (colour) | Hue rotation over time. |
| **Math** | A, B (number), operation (choice) | Value (number) | An expression you can see. |
| **Remap** | Value, in/out ranges (number) | Value (number) | Linear range map with clamp choice. |
| **Smooth** | Value (number), Time (number) | Value (number) | Temporal smoothing of its input — a temporal dependency, declared as such (§2.3). |

A driver's cross-layer input (Audio level's Audio) is a **layer-reference parameter**
(docs/03 §8) — the existing machinery, with the existing degrade-to-no-op on a dangling
id. **Edges never cross layers**; the canvas draws a referenced layer as a derived source
node (the drawing's Music node) and the wire from it renders the parameter, exactly as
the image chain's wires render the list.

### 1.4 Edges, ports, and what a wire means

- `Param(node, port)` — the destination parameter follows the source output. At most one
  edge per input. Types must match (§6.1); number accepts number, colour accepts colour.
- `Matte(effect)` — the effect's matte input. Two feeds exist: the effect's **matte
  parameter** (a layer reference with channel/invert/source, K-142/K-395 — set from the
  Effect-controls Matte row or by wiring from a derived source node), and the one new
  capability the NodeGraph drawing shows, `SourceMatte` — the layer's **own** masked
  source alpha, a texture the pipeline has already computed at that point in the chain.
  An in-graph `SourceMatte` edge overrides the parameter while it exists; the Matte row
  displays whichever is in force, by name.
- **Exposure** (the `E` badge) grows a node to show one hollow, type-coloured socket per
  parameter. It is presentation state per node (a bool on the instance), not wiring;
  a wired socket is shown regardless.
- **Bypass** (`B`) is the existing `enabled` flag; a bypassed node draws its border dashed
  (both drawings).

### 1.5 Validation

`SetLayerGraph` is **refused** (an op error, surfaced as a calm message) when an edge
references a missing node/port, mistypes, doubles up an input, or closes a cycle among
driver nodes. Refusal, not degradation, because unlike a dangling matte this state can
only be reached by an edit we control, never by deleting some *other* entity — a deleted
driver takes its edges with it inside the same commit. A dangling **layer reference** on
a driver's parameter degrades exactly as a matte does.

## 2. Evaluation and determinism

### 2.1 Where drivers run

Driver evaluation is parameter evaluation. One property's order (docs/03 §6.3) becomes:

```
keyframe/static evaluation → [expression] → driver edge (wins if present) → clamp/validate
```

At resolve time, before an effect's parameters pack into uniforms, the resolver evaluates
the layer's driver subgraph at that frame — topological order over the (acyclic) driver
nodes, pure CPU scalar work — and substitutes driven values. Nothing downstream changes:
the kernels see numbers, exactly as they see keyframed numbers today. The compiled
evaluation graph (K-015) is untouched in shape; drivers never become pixel nodes.

### 2.2 Determinism

Same project, same time, same value — no wall clock, no render-order dependence:

- **Wiggle** is seeded by its node id and sampled at layer time; two renders agree bit
  for bit, and export equals preview (K-031).
- **Audio level** computes from decoded samples through a fixed window; its value folds
  the audio fingerprint, the time and the window into the frame key.
- Driver evaluation order is the topological order with ties broken by node id — never
  a HashMap iteration.

### 2.3 Cache correctness

A driven parameter's evaluated value reaches the effect's content hash through the same
door a keyframed value does — substitution happens before packing, so the existing
formula (docs/06 §5.2) needs no new terms for pointwise drivers. Two drivers are not
pointwise and declare **temporal dependencies** in the metadata pass, as temporal effects
already must: Smooth (reads its input over a window) and Audio level (reads audio around
the frame). The declared window folds the sampled range into the hash.

## 3. Ops and undo

One new op, shaped like the one it mirrors:

- **`SetLayerGraph { layer, graph }`** — the whole-graph commit, exactly as
  `SetLayerEffects` is the whole-stack commit for add/remove/reorder. Add a driver,
  remove one, connect, disconnect, move, toggle exposure: each gesture is one
  `SetLayerGraph`, one undo step. Auto-wire folds the edge into the same commit as the
  add. Heal on an *effect* delete is `SetLayerEffects` (the list heals by construction);
  heal on a *driver* delete is dropping its edges in the same `SetLayerGraph`.
- Driver **parameters** ride the property path — `<layer>/graph/<node>/<param>` beside
  the existing `<layer>/effects/<effect>/<param>` — so keyframing, expressions, the
  stopwatch and every existing property op work on a driver row unchanged.

No per-edge ops until a measured drag shows the whole-graph clone on the edit path;
docs/05 §3 already names structural sharing as the upgrade if cloning ever bites.

## 4. Serialisation, old files, versioning

- `graph` is **additive with a serde default** (empty), skipped on save when empty. Every
  pre-K-471 file loads to an empty graph; a file that never wires never changes on disk.
- Unknown-field preservation (K-065) carries a graph through an older reader unharmed,
  under the pre-1.0 no-migration policy (docs/03 §12). No `min_reader` bump: an older
  Lumit renders such a project without its drivers — the same silent-degrade class as a
  missing plugin — which pre-1.0 accepts and 1.0's registry will revisit.
- AE import never produces a graph (AE has no equivalent); round-trip untouched.

## 5. The bridge surface

- **Read**: `graph_of_layer(layer) -> BridgeLayerGraph` — nodes (id, ref kind, name,
  ports with id/name/type/wired, position, enabled, exposed), edges, and the stack order.
  Fetched on selection and on document change, cached Dart-side; **never called in a
  rebuild path** (the budget test expects 0).
- **Write**: `set_layer_graph(...)`; stack gestures reuse `set_effects`; driver params
  reuse the property calls.
- **Catalogue**: the Drivers category rides the existing effect-catalogue listing.
- **Port types cross as an enum**; Dart maps type → theme token. No colour crosses the
  bridge.
- **K-005 gate**: every driver name, port name and parameter label the engine can send
  gets its `engine_labels.dart` entry and matching `app_en.arb` key in the same commit —
  `engine_labels_test.dart` walks the tables.

## 6. Port types and the points stream

### 6.1 The types

```rust
enum PortType { Image, Matte, Number, Colour, Shape, Points, Audio }
```

Wire and socket colour is the type — colour as the legend (K-445), five colours for
seven types, grouped as the NodeGraph drawing's legend groups them:

| Token (theme, viz-family) | Types | Drawing's colour |
|---|---|---|
| `port.image` | image · matte | blue |
| `port.number` | number | amber |
| `port.colour` | colour | magenta |
| `port.geometry` | shape · points | teal |
| `port.audio` | audio | green |

The tokens live in the theme struct (`PortColours`, 15-DESIGN §4.1) under the no-hex
rule; the drawing's hexes are the dark theme's values.

### 6.2 The points stream (K-446's seam)

`PortType::Points` lands with WP1 so the type system is complete from the first commit.
A **points stream** is evaluated data, like an image — never stored in the document:

```rust
/// One frame's particles/instances, structure-of-arrays, GPU-resident.
struct PointsStream {
    count: u32,                 // budgeted cap, declared like any allocation
    position: Buffer<Vec2>,     // px@comp (K-419); grows to Vec3 with 2.5D points
    speed: Buffer<Vec2>,        // px per second, direction and magnitude (the glossary
                                // reserves "velocity" for a Retime lens label — §9 ban)
    age: Buffer<f32>,           // seconds since birth
    size: Buffer<f32>,          // px
    rotation: Buffer<f32>,      // radians
    colour: Buffer<[f16; 4]>,   // premultiplied, working space
    id: Buffer<u64>,            // stable per particle across frames — what makes trails possible
}
```

**Particulate** (K-446) is a stack effect — image in, image out (it draws its points over
its input), *plus* a declared `Points` output port, so it sits honestly in the linear
stack today and feeds the later grid/scatter/clone-to-points/connect-points family
without redesign. Its feature set is WP6's design document, not this note.

## 7. What the graph never shows

- The compiled **evaluation graph** (K-015) — the Graph panel draws the document (stack +
  wiring), never `lumit-eval`'s nodes; constant-folding, deduplication and pass-through
  elision remain invisible.
- **Branched image chains** — §1.1's rule; the gesture does not exist.
- **Other layers' internals** — a derived source node is a name and its output ports,
  never the other layer's own graph.
- The Layer out node draws an **Audio** input port (the Nodes-workspace drawing) that
  represents the layer's own audio and accepts no wire in this phase — audio comes only
  from a footage layer's own stream (K-435). Drawn, unfilled, honest; wiring audio into
  a layer's output is future work, listed not faked.

## 8. Work packages

Ordered; each sized for one agent; each lands with its tests (K-007). K-458's standing
rules bind every UI package: the named drawing is authoritative, cited by name and never
by any working-folder path. WP1 → WP2 → WP3 → WP4 → WP5; WP6 needs
only WP1's type enum and may run any time after it.

### WP1 — Engine model and evaluation

`LayerGraph` on `Layer`, `NodeRef`/`Edge`, the `Signature` split in the registry, the six
v1 drivers, `SetLayerGraph` with validation (§1.5), driver evaluation in the resolve path
(§2.1), the `SourceMatte` feed, `PortType` including `Points`, hash/temporal declarations
(§2.3).
**Files**: `crates/lumit-core/src/` (layer, ops, `fx/` registry and resolve,
new `fx/drivers/` one file per driver), render threading of `SourceMatte` in
`crates/lumit-render`.
**Tests** (lumit-core unless noted): old-file load → empty graph and byte-identical
re-save; graph round-trip; cycle/mistype/double-input refusal; driven-overrides-keyframes
evaluation; per-driver value tests (Wiggle determinism across two evaluations, Audio
level against a synthesised tone, Math/Remap/Smooth against closed forms); frame-key
sensitivity (driven value changes ⇒ key changes; unrelated edit ⇒ key stable); undo
inverse restores nodes *and* edges; export-equals-preview on a driven comp (render test).

### WP2 — Bridge

`graph_of_layer`, `set_layer_graph`, the property-path extension for driver params, the
Drivers category in the catalogue listing, `BridgePortType`.
**Files**: `crates/lumit-bridge/src/api/**` (then codegen; generated files never edited),
`flutter_ui/lib/l10n/engine_labels.dart` + `app_en.arb` (new keys listed in the commit
message and PR for Crowdin, K-303).
**Tests**: `engine_labels_test.dart` green over the new tables; an frb test driving
add-driver/connect/undo through the bridge; `bridge_call_budget_test.dart` unchanged at 0
for rebuild paths.

### WP3 — Graph panel: view and wiring

The panel to the approved **NodeGraph** drawing: dot-grid canvas, node anatomy (header
kick, port rows, sockets), type-coloured wires and the legend, Auto-wire and Heal
toggles (`HouseToggle`, on in `animated` per K-465), frame-all and zoom readout, the Tab
search popover with type-filtered results when a wire is dragged onto empty canvas,
drag-to-wire and disconnect, `E` exposure, dashed bypass, the selected border in
`animated` (K-473). Image-chain gestures lower to `set_effects`; everything else to
`set_layer_graph`; one gesture, one undo step. The Effect-controls rows gain the *driven*
state (hollow type-coloured ring, driver's name in the well — the Nodes-workspace
drawing's Node panel rows are the reference).
**Files**: `flutter_ui/lib/panels/graph_panel.dart` (+ parts), `theme.dart`'s
`PortColours`, Effect-controls row driven state, arb keys.
**Tests**: widget tests per gesture asserting the single op committed and the wire drawn;
type-mismatch drop refused visually and op-free; legend/colour from tokens (no-hex lint
is the gate); driven row rendering.

### WP4 — Nodes workspace

The workspace to the approved **Nodes-workspace** drawing: the preset (graph as the main
surface, small viewer keeping the whole viewer bar, short timeline beneath the graph,
the **Node panel** — the selected node's rows — lower right), the workspace tab, the
toolbar's layer picker and Search nodes hint. Adds the preset to docs/07 §1.6 in the
same commit (it is the second preset after Retiming whose inventory differs).
**Files**: workspace preset tables in `flutter_ui/lib/shell/`, `panels/node_panel.dart`,
docs/07 §1.6, arb keys.
**Tests**: preset layout test (panel inventory and shares); Node panel follows graph
selection; timeline in the workspace is the ordinary Timeline at reduced height (shared
widget, no fork).

### WP5 — Node preview panel

K-448: its own panel, openable in a sidebar of the Effects workspace — a locked,
read-only second viewport showing one node's output without soloing. Engine side: a
render request for the compiled node matching a stack prefix (the request tuple already
addresses nodes), presented to a second shared texture alongside the Viewer's.
**Files**: `crates/lumit-render` (second present target), bridge call, 
`flutter_ui/lib/panels/node_preview_panel.dart`, Effects-workspace sidebar wiring.
**Tests**: preview of a two-effect layer's first node differs from the Viewer exactly by
the second effect (frame test); closing the panel stops the second render (no idle work);
skip-on-no-GPU markers as the Viewer tests use.

### WP6 — Points stream and the Particulate design document

Writes `docs/impl/particulate.md`: Particulate's parameter surface against K-446's
constraints (separate effect, points out, the owner's 2-second-per-frame class budget for
physical simulation recorded where relevant), the `PointsStream` attribute layout
finalised (§6.2 is the starting shape), emission/simulation determinism rules (seeded,
frame-stepped, cache-keyed), and its own test plan. **Design only — no implementation in
phase 3.**
**Files**: `docs/impl/particulate.md`, a row in `docs/impl/README.md`, glossary entry if
new terms appear.
**Tests**: none to run; the document carries the plan.

## 9. Test plan — the core invariants

Beyond the per-package tests, four properties are the ones a regression would betray:

1. **Old files are untouched**: load any pre-K-471 fixture, save, byte-compare.
2. **The stack view never lies**: for every graph fixture, the effect list order equals
   the image-chain order the graph read model reports (a property test over generated
   graphs, not one example).
3. **Determinism**: a driven comp renders bit-identically twice, and export equals
   preview (K-031's standing gate extended with one driven fixture).
4. **Undo symmetry**: any `SetLayerGraph`/`SetLayerEffects` sequence walked back restores
   the original document exactly (journal round-trip over the same generated fixtures).

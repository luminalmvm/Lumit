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
    exposed: Vec<NodeRef>,             // the `E` badges (WP2; §1.4)
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
`lumit-core/src/fx/drivers/`, schema-declared parameters, catalogue generation,
`list_parameters`, the Effect-controls row rendering. The registry gains one declaration:

```rust
enum Signature {
    Image,                                     // today's effects (implicit until now)
    Data { outputs: &'static [(&'static str, PortType)] },
}
```

**Built, 2026-08-24.** It is a **method on `EffectDef`** with `Signature::Image` as its
default, not a field on `EffectSchema`: a field would have meant a `#[derive(Effect)]`
change and a line added to ninety declarations that all say the same thing. Two more
default methods ride beside it — `eval_driver`, which computes the outputs, and
`driver_window`, §2.3's temporal declaration in seconds either side of the frame. Every
image effect takes all three defaults and is untouched.

The v1 driver set is the six the drawings show, in a **Drivers** catalogue category:

| Driver | Inputs | Outputs | Notes |
|---|---|---|---|
| **Wiggle** | Amount, Frequency (number) | Value (number) | Deterministic value noise seeded by the node's id, sampled at layer time. Same recipe as the expressions' wiggle if one exists when built; otherwise pin the noise in this note's test plan. |
| **Audio level** | Audio (layer reference), Window (seconds) | Amplitude, Low (number) | Windowed RMS of the referenced layer's decoded audio at layer time; Low is the same over a low band (one pole at 200 Hz). Deterministic per (audio fingerprint, time, window). |
| **Colour cycle** | Phase (turns), Rate (turns/second), Saturation, Brightness | Colour (colour) | Hue rotation over time. |
| **Math** | A, B (number), operation (choice) | Value (number) | An expression you can see. |
| **Remap** | Value, in/out ranges (number) | Value (number) | Linear range map with clamp choice. |
| **Smooth** | Value (number), Time (number) | Value (number) | Temporal smoothing of its input — a temporal dependency, declared as such (§2.3). |

**Two corrections from the build (2026-08-24).** *Colour cycle* was drawn here with Phase
alone, and a colour cycle that cannot cycle without a keyframe is not one — it gained
**Rate**, so hue is `phase + rate x layer time`, and **Saturation** and **Brightness**,
because a hue on its own is not a colour. *Audio level*'s samples arrive through an
`AudioTap` trait the host implements: `lumit-core` knows nothing of media, so the driver
owns the windowed RMS (testable against a synthesised tone in the same crate) and the
decoding stays where decoding lives. **`lumit-render` passes `None` today** — the seam is
built and tested, the wiring to decoded audio is not, and it is in no work package's file
list. It is recorded in docs/TODO.md rather than pretended about: until it lands, Audio
level reads silence, which is the same labelled no-op a dangling reference gives.

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
  **Built, 2026-08-24**: it needed no new carriage. `SourceMatte` lowers to K-288's
  `LayerInputDraw::ThisLayer`, which the draw builder has always meant by "a matte pointed
  at the layer the effect is on" — the effect's own input at its point in the chain. One
  branch in `mattes_for`, and nothing downstream learns a new shape.
- **Exposure** (the `E` badge) grows a node to show one hollow, type-coloured socket per
  parameter. It is presentation state per node, not wiring; a wired socket is shown
  regardless. **Built, 2026-08-24** as `LayerGraph::exposed`, a `Vec<NodeRef>` beside
  `layout` rather than the bool on the instance this line first named — see WP2.
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

Two drivers are not pointwise and declare **temporal dependencies** in the metadata pass,
as temporal effects already must: Smooth (reads its input over a window) and Audio level
(reads audio around the frame). The declared window folds the sampled range into the hash.

**Corrected by the build, 2026-08-24.** This section said the existing formula (docs/06
§5.2) needed no new terms, on the reasoning that substitution happens before packing. It
does need them, because **the frame key is not built from the packed values** — it is
built from the document, hashing each stored property at the frame's time. A driven
parameter's stored value is exactly the thing the picture no longer uses, so left alone
the key would have missed every driver. A wired layer therefore folds three things beside
its stack:

- **its driver nodes, hashed exactly as its effects are** — identity, version and stored
  parameters at the frame's time. That is a hash of the *declaration* rather than of the
  evaluated value, which discriminates identically (a driver is a pure function of its
  parameters, its node id and the time) and costs no graph walk inside the key. It also
  folds Audio level's referenced layer, and with it the audio fingerprint, through the
  layer-reference arm that was already there.
- **the layer time**, because a driver's output moves with it while every stored number
  holds still. Without this, two frames of a Wiggle-driven stack would share a name.
- **the wires**, since moving one changes which parameter follows what without changing
  any stored value.

The declared window is folded beside them, so widening Smooth's window retires the frames
smoothed with the narrow one. The stored value of a driven parameter is still hashed by
the ordinary loop, which costs a needless *miss* when somebody edits a keyframe under a
live wire and can never cause a stale *hit* — the safe direction, and far cheaper than
running the graph inside the key walk.

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

- **Read**: `LayerReference::get_graph() -> BridgeLayerGraph` (this note first called it
  `graph_of_layer`) — the derived boxes (ref kind, match name, label, custom name,
  enabled, ports with id/label/type/wired) and the stored `BridgeGraphWiring` (edges,
  positions, exposure). Fetched on selection and on document change, cached Dart-side;
  **never called in a rebuild path** (the budget test expects 0).
- **Write**: `LayerReference::set_graph(drivers, wiring)`; stack gestures reuse
  `set_effects`; driver params reuse the property calls, from the staged instances
  `get_graph_drivers()` hands out.
- **Catalogue**: the Drivers category rides the existing effect-catalogue listing.
- **Port types cross as an enum**; Dart maps type → theme token. No colour crosses the
  bridge.
- **K-005 gate**: every label the engine can send gets its `engine_labels.dart` entry and
  matching `app_en.arb` key in the same commit — `engine_labels_test.dart` walks the
  tables. The driver names, their controls and the Drivers heading landed with WP1 (they
  could not wait, see there); what is left for WP2 is the **port** names it introduces.

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
**Landed 2026-08-24** as described, with the corrections recorded in §1.3, §1.4 and §2.3
and three notes on the seam:
- **The l10n gate moved here from WP2.** The moment a driver enters the catalogue the
  engine can send its name, `fx-labels.txt` regenerates and `engine_labels_test.dart`
  fails — so WP1 lands the `app_en.arb` keys and `engine_labels.dart` entries for the
  driver names, their controls and the Drivers category. WP2 still owns the entries for
  the port names it adds.
- **The Drivers family is filtered out of `list_effects`** for now: it is in the catalogue
  (the lookup needs it) but is not an Add-effect entry, because dropping a driver on a
  stack would add a node that changes no pixel. WP2 gives the family its own listing and
  removes the filter.
- **`resolve_stack_temporal_named` gained a `&ResolvedDrivers` argument**; the two thinner
  wrappers keep their signatures and pass `ResolvedDrivers::NONE`, so the fifty-odd call
  sites that do not care are untouched. Substitution happens inside the one walk that
  evaluates a parameter, so a driven number goes through the identical `Unit` arm a typed
  one does and cannot land in the wrong units.
- **Wiggle's noise is pinned** (this note asked for it, there being no expressions'
  wiggle to match): one octave of the shared seeded 3-D value noise (docs/08 §3.37),
  walked along its x axis at Frequency cells per second, seeded by the node id's four
  32-bit halves exclusive-ored together, scaled by Amount. **Smooth** is a nine-tap box
  average over a **centred** window, so a smoothed ramp comes back as the ramp rather
  than as the ramp running late.
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

**Landed 2026-08-24.** The seam as shipped is `LayerReference::get_graph` (the whole
structure in one call), `get_graph_drivers` (staged instances, as `get_effects` is for the
stack), `new_driver` (uncommitted) and `set_graph(drivers, wiring)` (the one commit), plus
`list_drivers()` and `BridgePortType`; docs/17 §"The layer graph" is the contract. Four
things this note asked for came out differently, and each is the smaller change:

- **`graph_of_layer` is `LayerReference::get_graph`**, a method on the handle, because
  every other read is (docs/17 "references up"). The read model splits in two — derived
  boxes that are never written back, and the stored `BridgeGraphWiring` that is — so the
  read hands back exactly the object the write takes.
- **Exposure lives in `LayerGraph::exposed`, not on the instance.** §1.4 said a bool on
  `EffectInstance`, which would have meant a field on every effect in the document and a
  literal touched in five crates, for state no pixel reads — and it still could not carry
  a *derived* node. A `Vec<NodeRef>` beside `layout` is presentation state filed with the
  other presentation state, absent from an untouched file, and absent from the frame key
  for the same reason `layout` is.
- **The Drivers family got its own listing rather than the `list_effects` filter simply
  going.** Removing the filter would have put drivers in the Add-effect menu and the
  effects browser, where dropping one adds a node that changes no pixel. `list_drivers()`
  answers the canvas's question and `list_effects()` the stack's; one shared catalogue walk
  builds both.
- **A port declares its own English label** (`fx::Port { id, label, ty }`, shared by
  `Signature::Data`'s outputs and the derived nodes' constants), so the K-303 walk finds
  port words the same way it finds an effect's. Four were new — Image, Input, Output,
  Layer out; the drivers' output words were already in the table as parameter labels.
  The **property-path spelling is `<layer>/graph/<node>/<param>`**, as §3 proposed: the
  fold paths name the layer's group in the second segment, and `graph` is what the field
  is called.

The one thing WP2 does **not** carry is a `values` list on a driver box: a driver's
parameters come from `get_graph_drivers()`, cached at the same two moments the graph is
(selection, document change), exactly as the Effects panel caches `get_effects` — so WP4's
Node panel still costs no call in a rebuild.

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
addresses nodes).

**Landed 2026-08-24**, with the transport settled the smaller way (**K-486**, which
supersedes this note's "presented to a second shared texture alongside the Viewer's").
The preview is a **bounded thumbnail on the worker's existing response stream**, not a
second zero-copy target: it is a still that changes when the pick, the playhead, the
layer or the document does, so a second present target would have been a whole Viewer's
plumbing — three platform variants, a second Dart texture registration, a present pool
keyed by role rather than size — for a picture the size of a scope trace. K-486 carries
the reasoning; the seam is `preview_node` / `WorkerResponse::NodePreview` (docs/17).

Three further things came out of building it:

- **The prefix is a length, not a new render path.** `graph::prefix_len` turns a node
  into "how many effects are upstream of it" and `graph::truncated_effects` hands back a
  patched *copy* of the snapshot with the layer's stack cut there — the same shape the
  dropper's solo read and every drag preview already use. The ordinary interactive path
  renders it, so export-equals-preview and the drag fast path come for free.
- **The frame key needed no field.** It already hashes each layer's effects, so a shorter
  stack is a different name by construction. The Layer out node cuts nothing, and so
  rides the frame the Viewer has already banked.
- **A driver is answered with silence.** It makes a number, not a picture; the panel
  draws its own empty face rather than being told a picture is coming.

**Files**: `crates/lumit-core/src/graph.rs` (`prefix_len`, `truncated_effects`),
`crates/lumit-bridge/src/api/{composition,state,worker_thread}.rs` (then codegen),
`flutter_ui/lib/panels/node_preview_panel.dart`, `state/dock.dart` (the panel and its
place in the Effects preset), arb keys.
**Tests**: preview of a two-effect layer's first node differs from the Viewer exactly by
the second effect, and equals a project authored with only that effect
(`lumit-render/tests/node_prefix_preview.rs`, skip-on-no-GPU as the Viewer tests are);
each prefix names its own frame; the preview's own drain lane; the panel's face, its
following of the pick, and 0 bridge calls on a hover. Closing the panel stops the second
render by construction — the reply subscription and every listener go with it, so
nothing is left asking.

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
   graphs, not one example). **This one belongs to WP2**, which is where the read model
   (`graph_of_layer`) is built — WP1 has no image-chain reporting to hold the list
   against, because the model deliberately stores none.
3. **Determinism**: a driven comp renders bit-identically twice, and export equals
   preview (K-031's standing gate extended with one driven fixture).
4. **Undo symmetry**: any `SetLayerGraph`/`SetLayerEffects` sequence walked back restores
   the original document exactly (journal round-trip over the same generated fixtures).

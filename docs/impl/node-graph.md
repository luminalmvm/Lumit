# The node graph — implementation note

**Decision:** K-471 (the stack stays the spine; a layer gains an additive driver graph),
K-472 (port types, wire colours, the points stream), K-473 (the selected node border).
**Related:** K-445 (the graph is a second view that can also wire), K-446 (Particulate
emits a points stream), K-448/K-486/K-528 (the picture at a node — its own panel, then a
bounded thumbnail, now the Viewer's own chip), K-458 (the drawing is
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

**Built 2026-08-30 (K-674).** A chain input's wire is picked up by its far end exactly as
a stored wire is: dropped on another chain input it re-routes (the fed box moves to sit
right after the wire's source — one `reorder` op; dropped on the Layer out, the source
moves to the end), and dropped on empty canvas the connection goes the only honest way a
derived wire can — the box it fed leaves the list (`remove`), neighbours joining by
construction. Each answer is one op and one undo step, and a press that never travelled
does nothing: a chain discard costs an effect, so a slip must never be one.

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
    exposed: Vec<NodeRef>,             // the boxes twirled open (WP2; §1.4)
    groups: Vec<NodeGroup>,            // named regions of the canvas (K-651)
}

/// A named set of boxes drawn on one tinted wash (K-651). No geometry: the
/// rectangle is worked out from where the members are sitting, so it follows a
/// dragged box, and `colour` is an index into the frontend's label palette.
struct NodeGroup {
    name: String,
    colour: u32,
    members: Vec<NodeRef>,
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
| **Audio level** | Audio (layer reference, **unset = this comp's mix**), Window (seconds) | Amplitude, Low (number) | Windowed RMS of the chosen sound; Low is the same over a low band (one pole at 200 Hz). Deterministic per (audio fingerprint, time, window). |
| **Colour cycle** | Phase (turns), Rate (turns/second), Saturation, Brightness | Colour (colour) | Hue rotation over time. |
| **Math** | A, B (number), operation (choice) | Value (number) | An expression you can see. |
| **Remap** | Value, in/out ranges (number) | Value (number) | Linear range map with clamp choice. |
| **Smooth** | Value (number), Time (number) | Value (number) | Temporal smoothing of its input — a temporal dependency, declared as such (§2.3). |

The points-stream programme adds a seventh (K-492, K-494, points-stream.md §2.2), the
first with a **data** input rather than only numbers:

| Driver | Inputs | Outputs | Notes |
|---|---|---|---|
| **Points sample** | Points (stream, **wire-only** — no stored value, nothing to keyframe, no panel row), Position (px@comp) | Count, Nearest distance (number) | Reads a points stream and makes numbers of it: how many particles are alive, and how far the nearest is from Position. Unwired reads as an empty stream — Count 0, Nearest distance 1e9, "nothing anywhere near". Pointwise, so its `driver_window` is nought. |

**Reading a stream makes the walk re-entrant.** Answering the Points sample's wire
evaluates the producer's stream, and the producer's own parameters may themselves be
driven — so `Eval::output` calls back into itself through `Eval::stream`. That terminates
because the loop it could otherwise make (the stream feeding a driver that feeds the
producer) is refused at commit by the cycle check, and it is bounded anyway by the same
evaluation budget and depth every other wire spends. One stream is evaluated per producer
per frame's walk, however many wires read it.

The programme adds an **eighth** (K-604, points-stream.md §1.2, §2.3), and it is the first
driver whose *output* is a stream rather than a number:

| Driver | Inputs | Outputs | Notes |
|---|---|---|---|
| **Layer points** | Points layer (layer reference) | Points (stream) | Another layer's points, brought into this layer's graph. The family's cross-layer tap. It has no wire inputs at all: what it reads is *named*, because edges never cross layers. The stream is the first enabled effect on the named layer that makes points, evaluated with **that layer's own graph applied** — so what a tap reads is what that layer draws. `eval_driver` pushes nothing (a stream is not a `Value`); the walk fetches it through `Eval::points_input` instead, and the draw builder through `fx::driver_stream`. |

And a **ninth and tenth** (K-656), the two halves of one idea — the join between the number
wires and the colour ones, which the graph had no way to cross:

| Driver | Inputs | Outputs | Notes |
|---|---|---|---|
| **Split** | Colour (colour) | Red, Green, Blue, Alpha (number) | A colour taken apart. Nothing is converted or clamped on the way through: the channels leave scene-linear exactly as the colour holds them, so a value above one survives. Unwired, the row's own swatch makes the node a constant four numbers. |
| **Combine** | Red, Green, Blue, Alpha (number; Alpha defaults to one) | Colour (colour) | Four numbers put back together. Sliders rather than one swatch, because each row is a **socket** and a swatch has nowhere for four wires to land; the 0..1 range is where a colour is usually written, and a wire may carry any number (K-510). |

Both are closed-form and pointwise — no window, no state, `driver_window` nought — and
`a_colour_survives_split_and_combine_unchanged` is the pair's own test: a colour through
Split and back through Combine is the colour that went in, **bit for bit**, including a
channel above one. That round trip is the whole specification of "nothing is converted".

**A tap reaches one layer, never two.** The far side is evaluated by a fresh walk over that
layer's graph, built with the crossing flag cleared, so a tap over there answers the empty
stream. Two layers naming each other therefore stop at the second hop — no visited set, no
cycle to detect, and a bound that does not depend on the budget noticing. The far walk shares
the near one's remaining budget, so a fan of taps cannot buy itself more work than one
frame's allowance.

**Two corrections from the build (2026-08-24).** *Colour cycle* was drawn here with Phase
alone, and a colour cycle that cannot cycle without a keyframe is not one — it gained
**Rate**, so hue is `phase + rate x layer time`, and **Saturation** and **Brightness**,
because a hue on its own is not a colour. *Audio level*'s samples arrive through an
`AudioTap` trait the host implements: `lumit-core` knows nothing of media, so the driver
owns the windowed RMS (testable against a synthesised tone in the same crate) and the
decoding stays where decoding lives.

**The tap is wired, 2026-08-24.** `lumit_render::audio_tap::DocumentAudio` is the host
half: it is made inside `build_comp_draws_at` from the document that walk already holds,
and it answers a layer id and a layer-time range by decoding that layer's own footage
item. Three things make it a *deterministic* answer rather than a plausible one. It is
built where **both** renders build their draws — the Viewer's and the exporter's — so
there is no second implementation to drift (K-031). It decodes at a fixed
`audio_tap::TAP_RATE` (48 kHz) rather than at the sound device's rate, so the level is a
fact about the project and not about the machine; the playback mixer's device rate never
reaches a pixel. And layer time *is* source time for sound, which is exactly the mapping
`lumit_audio::mix::place_on_timeline` uses to place a clip, so the driver and the mixer
cannot disagree about which moment of the track a frame sits on. Decoded tracks are
shared per file across the process under a byte budget. A layer that is not footage, a
missing file, a failed decode or a reference naming no layer all read as silence — the
same labelled no-op a dangling reference gives. The K-031 matrix gained an audio-driven
row (`headless::tests::the_preview_and_export_paths_agree_on_an_audio_driven_comp`),
which asserts both that the two renders agree **and** that the driven picture differs
from the same comp with the wire cut — equal pixels there would mean silence again.

**The source is a choice of two, and one of them is the comp (K-657).** Audio level's Audio
row left **unset** reads the composition's own mix rather than silence: the picker's empty
entry says *This comp*, and the row is the source dropdown the feature was asked for
without a second control beside it. The mix is not a second summing — `DocumentAudio::mix`
asks `AudioJobsBuilder::audio_jobs` for the same job list export, playback and beat
detection all mix from (every audible layer at its own Volume, precomps' carriers
multiplied through, a solo honoured) and sums it with `place_on_timeline`, `volume_bake`
and `mix_stereo`, the master ceiling included. It is `export::mix_decoded` restricted to a
window: each clip is clipped to the window *before* its Volume is baked, so a five-minute
track costs a window's arithmetic per frame rather than a track's, and preview and export
reach the same number because both run this same function at the same comp time
(`headless::tests::a_comp_mix_driven_parameter_renders_the_same_picture_twice`).

**The window is centred by the host, not named by the driver.** `AudioTap` gained
`mix(half, out)` beside `samples(layer, from, to, out)` because a driver knows only its
*own* layer's time, and a layer that starts late would otherwise read the comp's mix at
the wrong moment of the track. The comp's clock belongs to whoever built the frame, so the
tap holds it. The default implementation answers `None`, which is the documented silence
for a host with no mix to offer.

**A probe that claimed to happen once now does.** `AudioJobsBuilder`'s has-audio memo moved
from a field to a process-wide map keyed by the footage item: a builder is made afresh on
every ask — and the comp-mix tap asks on every frame it draws — so per builder, "probed at
most once a session" was a comment rather than a fact, and an FFmpeg open per item per
frame is not a thing a playing timeline can afford.

A driver's cross-layer input (Audio level's Audio, Layer points' Points layer) is a
**layer-reference parameter** (docs/03 §8) — the existing machinery, with the existing
degrade-to-no-op on a dangling id. **Edges never cross layers**; the canvas draws a
referenced layer as a derived source node (the drawing's Music node) and the wire from it
renders the parameter, exactly as the image chain's wires render the list. K-604 settles
that this holds for a *stream* as well as for a number, which is what makes the family's
cross-layer tap a node rather than a new kind of edge.

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
- **Exposure** (the header twirl; the `E` badge until K-637) grows a node to show one
  hollow, type-coloured socket per parameter. It is presentation state per node, not
  wiring; a wired socket is shown regardless. **Built, 2026-08-24** as
  `LayerGraph::exposed`, a `Vec<NodeRef>` beside `layout` rather than the bool on the
  instance this line first named — see WP2.
- **Bypass** is the existing `enabled` flag, answered by the enable tick left of the
  node's name (the `B` badge until K-637); a bypassed node draws its border dashed (both
  drawings).

### 1.5 Validation

`SetLayerGraph` is **refused** (an op error, surfaced as a calm message) when an edge
references a missing node/port, mistypes, doubles up an input, or closes a cycle among
driver nodes. Refusal, not degradation, because unlike a dangling matte this state can
only be reached by an edit we control, never by deleting some *other* entity — a deleted
driver takes its edges with it inside the same commit. A dangling **layer reference** on
a driver's parameter degrades exactly as a matte does.

**The taxonomy, extended over the cross-layer points tap (K-604).** The dividing line is
unchanged and worth restating, because a tap is the first node that can be *right* about
its own graph and still answer nothing: **an edit this application made is refused; a state
some other entity's edit produced is degraded.** A tap's wiring is the first kind — it type-
checks, one-wires and cycle-checks through the existing arms with no new case, because it is
an ordinary driver output into an ordinary Points socket. What it *names* is the second kind,
and every way of naming nothing reads as the **empty stream** — the consumer draws the
picture it was handed, and the box wears the "no stream" mark K-509 gave the family:

| The tap | Answer | Why not a refusal |
|---|---|---|
| Names no layer (an unset row) | empty stream | A fresh node has an unset row; refusing it would refuse adding one. |
| Names a layer somebody deleted | empty stream | The deletion was the *timeline's* edit, not the graph's — a matte dangles the same way. |
| Names a layer with no producer on it | empty stream | The far layer's stack is its own to edit; this graph cannot be refused on its behalf. |
| Names a layer whose producer is bypassed, or whose fx switch is off | empty stream | A producer that draws nothing hands out nothing; the stream and the picture agree about an off switch. |
| Names a layer whose producer needs a picture (Scatter, Emit from image) | empty stream | K-599's recorded constraint, unchanged: at resolve time no picture exists. |
| Is itself reached across a layer boundary — a **second hop** | empty stream | The one-hop rule (§1.3). It is a bound, not a judgement, and it is what makes two layers naming each other terminate. |

Nothing here is a new refusal, and that is the finding: the tap needed the taxonomy widened
on the **degrade** side only. Its wiring rules needed no change at all, which is the whole
return on settling the design as a layer-reference parameter rather than as an edge that
crosses layers.

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
- **`SetLayerEffects` prunes the graph** (built 2026-08-24). The image chain heals by
  construction, but the *wires* do not: an edge, a canvas position or an exposure naming
  a removed effect would be left dangling, and the next `SetLayerGraph` of any kind — a
  box dragged, a wire drawn — would be refused for it. `LayerGraph::prune_to` drops them
  inside the same apply, and the op's inverse becomes an `Op::Batch` of
  `[SetLayerEffects(previous), SetLayerGraph(previous)]` so one undo restores the stack
  **and** its wiring together. A layer with an empty graph — the overwhelming case —
  neither clones nor prunes anything. This is the only place the graph heals rather than
  refusing, and it is because the edit is the *stack's*, which cannot be refused on the
  wiring's behalf.
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
- **Catalogue**: the Drivers category rides the existing effect-catalogue listing,
  through `list_drivers()` of its own. **Amended 2026-08-30 (K-645)**: `list_drivers()`
  still answers the canvas's narrower question — what may be *dropped on the graph* — but
  the family is no longer filtered out of `list_effects()`, and every entry now carries the
  `controls` grouping key and heading, because `FxCategory::grouping()` files Drivers under
  Controls. The variant is unchanged, and so are the docs' own Drivers pages: what merged
  is the application's browse grouping, nothing else. Applying a driver from any of those
  listings lands it on the layer's **graph** rather than its stack, and
  `LayerReference::add_effect` is where that fork lives so no caller has to know.
  **Built 2026-08-24**: an entry carries the ports it
  declares (`BridgeEffectInfo::inputs` / `outputs`, `wired` always false), which is what
  lets the panel fold the auto-wire into the add's own commit and filter the console to
  the entries a dragged wire could land on. Without it the auto-wire had to be a second op,
  because a driver's outputs only existed once `get_graph` could derive them.
- **Live drag on a driver**: `CompositionReference::render_frame_with_driver_preview(
  frame, scale, layer, drivers)` — **built 2026-08-24**, the twin of
  `render_frame_with_preview`. It substitutes `Layer::graph.nodes` on the worker's
  throwaway clone exactly as the stack call substitutes `Layer::effects`, so a driven
  parameter moves under the pointer instead of only on release. The nodes only: a drag on
  a number changes no wire, position or exposure, and staging them would invent a state
  the document cannot be in.
- **Node groups** (K-651): `BridgeGraphWiring::groups` rides the stored half, and
  `save_node_group` / `insert_node_group` / `list_node_groups()` are the library seam —
  `.lumgrp` files beside the `.lumfx` presets, in the same per-user folder. A group names
  boxes and a palette index; the wash's rectangle is derived from its members' positions,
  so nothing about geometry is stored and dragging a member is still one `layout` write.
  Insert is one `SetLayerGraph`: fresh ids, the wires that were inside the set re-pointed,
  the wires that left it dropped.
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
message and PR for translation, K-303).
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
toggles (`HouseToggle`, on in `animated` per K-465), frame-all and zoom readout, the
console (Ctrl+Space over the canvas, or a wire dragged onto empty ground — K-673) with
type-filtered results while a wire is in hand,
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

### WP5 — The picture at a node

K-448 asked for its own panel, openable in a sidebar of the Effects workspace: a locked,
read-only second viewport showing one node's output without soloing. **K-486** landed it
that way on 2026-08-24 as a bounded 256px thumbnail rather than a second zero-copy
target.

**K-528 folded it into the Viewer** (owner ruling: "node preview is just the viewer") and
supersedes both. The panel is gone. Selecting an effect — a box on this canvas *or* a
heading in the Effect controls stack, which are one selection (K-300) — offers an **"at
&lt;effect&gt;" chip** over the Viewer's own picture, and turning it on renders the
composition with that layer's stack truncated there, down the ordinary frame transport at
the Viewer's full quality. A second viewport for a still was the thing worth removing: at
full size, in the Viewer you are already looking at, it is the same picture answered
properly. K-528 carries the reasoning; the seam is `render_frame`'s optional
`BridgePrefixPoint` (docs/17).

The three things WP5 established all survive the fold, and two of them are why it was
cheap:

- **The prefix is a length, not a new render path.** `graph::prefix_len` turns a node
  into "how many effects are upstream of it" and `graph::truncated_effects` hands back a
  patched *copy* of the snapshot with the layer's stack cut there — the same shape the
  dropper's solo read and every drag preview already use. The ordinary interactive path
  renders it, so export-equals-preview and the drag fast path come for free. Both
  functions are unchanged; only their caller moved.
- **The frame key needed no field.** It already hashes each layer's effects, so a shorter
  stack is a different name by construction. Selecting the last effect cuts nothing, and
  so rides the frame the Viewer has already banked. What the interactive path *did* need
  was the name **memo** emptied when the point moves: a prefix renames every frame
  without moving the document revision, which is the one case the memo cannot see (the
  viewer look taught this first).
- **A driver offers no chip.** It makes a number, not a picture.

**Files**: `crates/lumit-core/src/graph.rs` (`prefix_len`, `truncated_effects`,
unchanged), `crates/lumit-bridge/src/api/{composition,state,worker_thread}.rs` (then
codegen), `flutter_ui/lib/panels/viewer_prefix_chip.dart`, `viewer_panel_frb.dart` (one
hookup), `main.dart` (the chip's point, beside the selection it follows),
`state/dock.dart` (the folded panel, and a saved layout that still names it), arb keys.
**Tests**: the cut document renders exactly what a project authored with those effects
renders, and differs from the full frame (`lumit-render/tests/node_prefix_preview.rs`,
skip-on-no-GPU); each prefix names its own frame, on the thumbnail path and on the
interactive one; latching a point empties the name memo; the chip from **both** selection
surfaces, its clearing, and its bounded cost per toggle.

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

# The node graph composition, an implementation note

**Decision:** a node graph is a composition whose picture is made by nodes and wires
instead of a layer stack. Image wires branch and merge. Merge and Switch are registry
effects that only a node graph can hold. A node graph is applied to a layer as the Node
graph effect, whose parameters are the graph's Input nodes, and the same effect is how one
node graph nests inside another. **Related:** [node-graph.md](node-graph.md) (the layer's
driver graph, which this note leaves exactly as it is), [custom-shader.md](custom-shader.md)
§4 (the third graph kind that shares the canvas), [layer-input.md](layer-input.md) (the
second-picture carriage), [group-effects.md](group-effects.md) (the precedent for a comp
that renders through the Precomp path), [effect-registry.md](effect-registry.md) §4
(derived parameters). Issue #106 is the proposal this note answers.

## In plain terms

Lumit has two ways of making a picture and each has its people. The Timeline stacks layers,
which is the right tool for timing and animation. A node graph joins pictures with wires,
which is the right tool for compositing: fork one picture into three, treat each differently,
merge them back over each other, and see the whole plan at a glance. Until now the node
graph was only a second view of one layer's effect list, with drivers hanging off it, and it
said so: no branches and no merge, because that view had to stay honest to the list.

This note adds the other kind. A **node graph** is a composition. It has a size, a frame rate
and a duration like any comp, it sits in the project panel like any comp, and everything that
already works on a comp works on it without new code: it can be placed in another comp as a
Precomp layer, it can be read by another node graph, it can be exported, its frames are named
and cached the same way. What is different is inside. Instead of layers it holds nodes:
**Read** nodes that bring footage, solids and comps in, **effect** and **driver** nodes from the
catalogue, a **Merge** that lays one picture over another with a blend mode, a **Switch** that
picks one of several, **Input** nodes that stand for values or pictures handed in from outside,
and one **Output** node whose picture the comp shows.

The two systems meet by nesting, not by mirroring. A comp is a Read node in a graph. A graph
is a layer in a comp. A graph is an effect on a layer, with the graph's Inputs as the effect's
rows, so a graph with no Read nodes is a reusable effect preset with a picture in and a
picture out. That is how Nuke, Fusion and Blender do it, and it is what issue #106 asked for.
The request to make the Timeline and a graph two views of one document was considered and
not taken: a Switch, a fork, or a picture merged over itself has no honest place in a layer
stack, and forcing one would slow both surfaces down. The layer's own graph keeps doing what it
does, so a Timeline user who never opens a node graph loses nothing.

## 0. Words

Glossary entries land with the first package, because the identifiers follow the words.

| Word | Meaning |
|---|---|
| **Node graph** (composition) | A composition whose picture is made by nodes and wires instead of a layer stack. Sized, timed and filed like any comp, so it is placed as a Precomp layer, read by another node graph, or applied to a layer as the Node graph effect. |
| **Read node** | A box that brings a project item into a node graph: footage, a solid or a composition. Drawn under the item's own name, with the item's kind as its kicker. |
| **Input node** | A box standing for a value or picture handed in from outside the graph. Applied as an effect or nested, an Input is a parameter row or a socket on the outer box. Viewed on its own, it is its default (a value) or transparent (a picture). |
| **Output node** | The one box whose picture the node graph shows. Every node graph has exactly one and it cannot be deleted. |
| **Merge** | The node that lays picture A over picture B with a blend mode and an opacity. A node graph's own; a layer stack joins pictures with layers and blend modes. |
| **Switch** (node) | The node that shows one of its pictures, chosen by an index. Not a layer's switches, which stay the per-layer toggles they were. |
| **Node graph effect** | The effect that applies a node graph to a layer. The layer's picture is the graph's first picture Input, the graph's other Inputs are the effect's rows, and the Output is what the effect hands on. |

The panel keeps its name. It is the Graph panel in the specs and "Node graph" on its tab, and
it now draws three things: a layer's graph, a Custom shader's inner graph, and a node graph
composition.

## 1. The model

### 1.1 A composition, with a graph in it

```rust
struct Composition {
    // ...
    layers: Vec<Layer>,                 // empty on a node graph
    graph: Option<CompGraph>,           // Some on a node graph; absent from the file otherwise
}
```

`graph` sits beside `beat_grid` with the same serde attributes (`default`,
`skip_serializing_if = "Option::is_none"`), so every project written before it existed opens
and re-saves byte for byte, and no schema version moves (docs/10 §1.1, the `anti_aliasing`
precedent). There is no new `ProjectItem` variant. That is the whole return on the decision:
the four exhaustive matches over `ProjectItem`, the Precomp layer arm, the matte and
layer-input arms, `add_precomp_layer`, comp tabs, export and the frame key all reach a node
graph already, because it is a `Composition`. Docs/03 §2 records the same reasoning for image
sequences being a flag on footage rather than an item of their own.

**A node graph has no layers, and the engine says so.** Every op that adds, removes or
reorders a layer, or edits one, is refused on a comp whose `graph` is `Some` with a new
`OpError::CompIsNodeGraph` ("a node graph has no layers"). The check is one guard at the top of
`Op::apply`, beside the lock guard, so every present and future layer op is covered and a
`Batch` is covered through its members. The other way round holds too: `SetCompGraph` on a
comp that has layers is refused with the same error, so a comp is one thing or the other and
never both. A new node graph is made empty with its graph seeded, and a layer comp never
becomes one.

**Built 2026-09-06.** `SetCompGraph` is refused on a comp whose `graph` is `None` as well
as on one with layers: the inverse carries the graph that was there, and a first write onto
nothing would have had to invent an Output with a fresh id, which is neither exact nor
deterministic. A node graph is made by being born one, which is what New node graph does.

### 1.2 The graph

```rust
/// What a node graph composition holds instead of layers.
struct CompGraph {
    nodes: Vec<GraphNode>,             // document order; the Output is always among them
    edges: Vec<GraphEdge>,             // §1.4
    layout: Vec<(Uuid, [f64; 2])>,     // canvas positions; a node with none is auto-placed
    exposed: Vec<Uuid>,                // boxes twirled open to show every socket
    groups: Vec<GraphGroup>,           // named washes; { name, colour: u32, members: Vec<Uuid> }
}

enum GraphNode {
    /// A project item brought in: footage, a solid or a composition (a node graph included).
    Read { id: Uuid, item: Uuid, custom_name: Option<String> },
    /// A value or picture handed in from outside (§1.5).
    Input { id: Uuid, input: GraphInput },
    /// Any catalogue entry: an image effect, a driver, a Merge, a Switch, a Node graph effect.
    Fx(EffectInstance),
    /// The one box whose picture the comp shows.
    Output { id: Uuid },
}

struct GraphEdge {
    from: Uuid, from_port: String,     // a node and one of its output sockets
    to: Uuid,   to_port: String,       // a node and one of its input sockets
}
```

Every node is stored and every node id is a `Uuid`, so the graph needs none of the layer
graph's derived sentinels (`NodeRef::Source`, `NodeRef::Out`) and reuses none of its types.
`custom-shader.md` §4.2 gives the reason and it holds here: sharing `Edge` between a graph
that may branch and one that must not would weaken the layer graph's honesty guarantee by
giving one type two meanings. What *is* shared is everything below the type: `PortType`, the
port colours, the effect registry, `EffectInstance`, the driver walk, the resolve path, the
canvas, the console, the Node panel rows.

An `Fx` node is an ordinary `EffectInstance`. That one choice is most of the feature: its
parameters take keyframes, expressions and driver wires; its `enabled` is the bypass tick;
its `custom_name` is the box's name; the Node panel draws its rows with the widgets it already
has; presets and copy carry it. A driver in a node graph is the same `EffectInstance` a driver
on a layer is. Merge, Switch and the Node graph effect are `Fx` nodes too (§1.3).

### 1.3 Three new catalogue entries

| Entry | Category | Rows | Sockets in a node graph | In a layer stack |
|---|---|---|---|---|
| **Merge** (`merge`) | Compositing | Mode (the `BlendMode::NAMES` choice, id `mode`), Opacity (0 to 100, %) | in: A (`input`, image), B (`background`, image), Opacity (number). out: `output` | Not offered. Adding one to a stack is refused. |
| **Switch** (`switch`) | Compositing | Index (whole number, 0 and up) | in: `in0`, `in1`, ... one more than are wired, Index (number). out: `output` | Not offered, as Merge. |
| **Node graph** (`node_graph`) | Utility | the graph's Inputs (derived, §1.5), Mix, the injected Blend | in: `input` (the first picture Input), one image socket per further picture Input, one socket per value Input, Matte. out: `output` | Applies the graph to the layer (§2.4). |

**Compositing is a new `FxCategory`.** Its two members are `is_image_op() == false`, exactly
as the Controls family and the drivers are, so a resolve of a layer stack never makes an op
of them. The category is the filter key: `list_effects` leaves Compositing out, as it once
left Drivers out, and the node graph's console asks `list_graph_nodes()`, which is Drivers and
Compositing together. `LayerReference::add_effect` refuses a Compositing name with
`BridgeError::NotAStackEffect`. A Merge and a Switch are realised by the graph walk itself
(§2.3), not by a kernel, which is why they have no CPU oracle and no GPU entry.

Merge's Mode row is spelled `mode`, never `blend`: `EffectSchema::blend()` treats a row called
`blend` over the blend-mode names as the Mix seam's own, and would blend Merge's result a
second time against its input (schema.rs's own note on the Lens flare records the bug). Merge
declares `matte = false`. Switch declares `matte = false`. The Node graph effect keeps the
universal Matte row and the Mix seam, so a graph dissolves back over the layer like any
effect.

### 1.4 Sockets and wires

One function, `comp_graph::ports_of(node) -> (inputs, outputs)`, is the table every reader
uses: the validator, the bridge's read model, the label walk and the walk that lowers the
graph to draws. They cannot disagree because there is one of it.

| Node | Inputs | Outputs |
|---|---|---|
| Read | none | `output` (image) |
| Input, picture | none | `output` (image) |
| Input, value | none | `value` (number or colour, by kind) |
| Fx, image effect | `input` (image); `matte` (matte) where the schema declares a matte row; one image socket per `ParamKind::Layer` row that is not the matte, named by the row's id; one socket per parameter whose `ParamKind::port_type()` answers; the signature's data inputs | `output` (image); the signature's data outputs |
| Fx, driver | the parameter sockets and the signature's data inputs, as on a layer | the signature's outputs |
| Fx, `merge` | `input`, `background` (image); `opacity` (number) | `output` |
| Fx, `switch` | `in0` .. `inN` (image), N being one more than the last wired index; `index` (number) | `output` |
| Fx, `node_graph` | `input` (image); one image socket per further picture Input of the graph it names, by that Input's id; one socket per value Input; `matte` | `output` |
| Output | `input` (image) | none |

Two things differ from a layer's graph. First, in a node graph **a layer-reference row is an
image socket**. On a layer that row is a dropdown of the comp's layers; a node graph has no
layers, so the wire is the reference. Light wrap's Background, Texturize's Texture, Set matte's
source and Merge's B all arrive this way, through the one carriage `layer-input.md` §1
describes, with nothing per effect at the seam. Second, **a matte socket accepts an image
wire as well as a matte wire**: what the row means by matte is the picture's own channel,
chosen by the row's Channel setting, and refusing a picture there would refuse the thing
people reach for first. Everywhere else a wire's type must equal its socket's type.

The rules `SetCompGraph` enforces, refused as `OpError::InvalidGraph` with `GraphError`'s own
sentence, plus two new variants (`NoOutput`, `SecondOutput`):

- every edge names a node the graph has and a socket `ports_of` lists;
- types match, with the matte exception above;
- one wire per input socket; an output may feed any number;
- no loop anywhere in the graph, image edges and value edges alike, because the walk pulls
  each node's inputs before the node and a loop would never end. Kahn's walk in document
  order with ids as the tie-break, as `LayerGraph::check_acyclic` does it;
- exactly one Output node.

**Built 2026-09-06.** The one function is `ports_of(graph, node, doc)`: it takes the graph
because a Switch's spare socket is one beyond the highest wired, and an optional document
because a Node graph box's sockets are the Inputs of the comp it names (with no document it
shows `input` and `matte` alone, and the validator always passes one). A Switch's sockets are
capped at 64, since the index comes out of the document and a hand-edited wire could ask for
four billion. Merge's two pictures are labelled A and B with the ids `input` and
`background`, so no new word was minted. The points, shape and audio skip below applies to
every socket list, a driver's included. A nested Node graph box draws no matte socket: the
walk could not honour a wire into one, and a socket the walk ignores is worse than none. The
Node graph effect on a layer keeps its Matte row, which the stack's own dissolve honours.

**Points, shape and audio sockets were not drawn in a node graph in v1.** Round two lifts
the skip whole (§5.1): every socket a signature declares is drawn, and a points wire is
carried through the projection into the driver walk and onto the step it feeds.

**Refusal, not degradation, for edits; degradation for the world moving.** A wire that breaks
a rule is an edit the application made and is refused before anything is swapped. A Read node
whose item somebody deleted is a state the project panel's edit produced: it draws transparent
and wears the missing mark, exactly as a Precomp layer of a deleted comp draws nothing today,
because `RemoveItem` cascades to nothing and must not (its inverse is a single `AddItem`).
Deleting a node takes its wires with it inside the same commit.

### 1.5 Inputs

```rust
struct GraphInput {
    id: String,        // snake_case, unique in the graph; the parameter id outside
    label: String,     // the row's word
    kind: InputKind,   // Picture | Number | Angle | Colour
    default: [f64; 4], // one number, or four for a colour
    min: f64, max: f64,
    unit: Unit,        // Raw, Px, Degrees or Percent
}
```

These are the five facts the Custom shader's Parameter node carries (custom-shader.md §4.3),
with the same names, so the Node panel edits both with one form. Order is document order,
which is the row order outside. A Point is two Number inputs. There is no Checkbox in v1: a
switch has no socket to feed, so nothing inside the graph could read it.

**Built 2026-09-06.** `unit` is the registry's own `Unit`, which gained serde derives, rather
than a lowercase word with a conversion pair beside it. A value Input's row id that collides
with the effect's own rows (`open`, `mix`, `blend`, the matte trio) derives no row, and the
lowering skips those ids when it reads a host instance's overrides, so such an Input bakes
its own default rather than the effect's Mix.

What an Input is worth depends on how the graph is being looked at:

| The graph is | A picture Input reads | A value Input reads |
|---|---|---|
| viewed on its own, or placed as a Precomp layer | transparent | its default |
| applied as the Node graph effect | the first: the layer's picture at that point in the stack. The others: the effect's own layer-reference rows, rendered alone at the effect's raster (the depth-pass carriage) | the effect's own rows |
| nested as a Node graph node in another graph | the first: the box's `input` socket. The others: their own image sockets | their own sockets, or the row's value when nothing is wired |

**The rows are derived, never adopted.** The Node graph effect implements
`EffectDef::derived(&inst)` the way the Custom shader does: from a declaration the instance
carries, through a cache kept for as long as the application runs, keyed by the declaration hash, into a leaked
`&'static [ParamSchema]`. The declaration is `extra.node_graph.inputs`, a copy of the graph's
`GraphInput` list written when the instance is made, and `extra.node_graph.comp` names the
graph. A copy rather than a document lookup because `derived` has no document in its hand and
the shader's road is the one every reader already walks. The copy is refreshed on the clones
the bridge hands out (`get_effects` refreshes it from the live graph before it fills the
derived rows), so a row added inside the graph appears the next time the stack is read and
lands in the document with the user's next edit, which is docs/08 §3.95's rule: offered, never
adopted behind anybody's back. A row for an Input the graph no longer has stays as an ordinary
parameter the graph ignores. The renderer never reads the copy; it lowers the graph from the
document and reads the instance's parameters by the Inputs' ids.

A further picture Input derives a `ParamKind::Layer { self_default: false }` row. Those rows
are on the derived list, not the schema, so `EffectSchema::layer_input()` does not see them;
the graph's own lowering carries them (§2.4).

### 1.6 Time and raster

A node graph's clock is comp time. A Read node reads footage at the graph's own time, a Read
of a comp evaluates that comp at the graph's own time, and an Fx node's keyframes are on the
graph's clock. v1 had no per-node time. Round two adds the Time offset box and the rule
that a box's input at another time is its input cone at that time (§5.2), and makes a
Precomp layer's Retime map reach the comp inside it (§5.6), so a graph is retimed the way
any comp is: place it and retime the layer.

Every node's picture is the graph's frame: `width` by `height` logical pixels, allocated at the
render scale like any comp. A Read node places its item as a fresh layer would, centred at its
natural size with no transform, which is what makes a Read node a *synthetic layer* (§2.1). A
Transform effect node moves it from there. Applied as an effect, the graph runs at the host
layer's working raster and its own frame size is not consulted, exactly as any effect runs at
the raster it is handed; px@comp values rescale through the same `px_scale` the host stack's
do.

## 2. Evaluation

### 2.1 A Read node is a layer at default placement

`comp_graph::read_layer(node, comp) -> Layer` builds a `Layer` for a Read node: the node's id
as the layer's, the item as its kind (`Footage`, `Solid` or `Precomp`), the comp's whole span,
zero offset, a centred transform, everything else default. Nothing keeps it; it is built where
it is needed and dropped. This is what lets the decode planner, the pixel fetch, the colour
space tag, the frame key's source arm and the "in use" walk treat a Read node as the layer it
behaves like, with no second code path:

- `collect_comp_jobs` gains a node graph arm that plans one job per Read node of footage
  through the synthetic layer, keyed by the node's id, and recurses into a Read node of a comp
  under the same `visited` guard the Precomp arm uses;
- the draw builder fetches a Read node's pixels with `pixels_for(&read_layer)` and its
  colour space with `footage_colour_space`, and lowers a Read of a comp through
  `nested_comp_draw`, cached under that comp's own frame key;
- `Document::item_is_used` and `comp_footage_items` walk a node graph's Read nodes beside
  every comp's layers, so the project panel's "in use" badge and the export's footage list
  cannot under-report a graph. `layer_names_item` gains a `read_names_item` twin.

**Built 2026-09-06.** The document holds no size for footage, so `read_layer` can only
centre a clip as if it were comp-sized. The lowering knows the size the pixels actually
arrived at and takes the draw's anchor from that, so a clip smaller or larger than the comp
lands on the comp's centre. The frame key still hashes the synthetic layer's transform, which
is fine: the key needs to move when the picture does, and a different clip is a different
source stamp. Two more things the build found. A Read box has no in and out points, so the
lowering opens the synthetic layer's span before the fetch and a Read draws whenever the
graph is asked, past the graph comp's own duration included (a Read of a comp still draws
what a Precomp layer would past that comp's duration). And the footage walk follows a Node
graph effect on a layer and a nested Node graph box inside a graph, as the planner does, so
the footage a graph reads is probed before the frame that needs it; a Node graph effect on a
group header is planned and probed the same way.

### 2.2 The projection: drivers and Inputs through the existing walks

Before anything is drawn, `comp_graph::project(graph, overrides) -> Projection` turns the
value half of the graph into shapes the engine already evaluates:

- **driver wires become a `LayerGraph`.** Every driver node goes into `nodes`, every wire out
  of a driver into `edges` as `OutputRef::Driver` into `InputRef::Param { node:
  NodeRef::Effect(fx) | NodeRef::Driver(driver), port }`. `resolve_drivers` then answers a
  `ResolvedDrivers` whose substitutions name `NodeRef::Effect(fx node id)`, which is the id
  `resolve_stack_temporal_named` resolves each Fx node under. Not one line of the driver walk
  changes, and a Wiggle into a Switch's Index works on the first day;
- **an Input's value is baked.** A wire from a value Input into any parameter, on an effect or
  on a driver, means "this parameter is that value", which is what overriding keyframes means.
  The projection clones the target instance and writes the value as `Static` into that
  parameter, then drops the wire. `overrides` is the list of `(input id, EffectValue)` a host
  instance supplies; an Input nobody overrides bakes its default. Colours bake four
  properties, numbers one;
- the result is the list of Fx instances as they are to be resolved, the driver graph, and
  the image wiring untouched.

**Built 2026-09-06.** Merge and Switch report `is_image_op() == false`, so the stack
resolver skips them; their Mode, Opacity and Index rows resolve through a small public
`fx::resolve_instance`, one instance at a time, so they keyframe and take a driver wire like
any row. A host layer's own driver wire into a Node graph effect's value row substitutes into
the graph's Input as an override, because a driven parameter never reads its keyframes.

### 2.3 The graph walk: one new draw source, one walker

The draw builder gains one arm at the top of `build_comp_draws_at`: a comp with a graph
returns a single `CompLayerDraw` at identity placement whose source is `DrawSource::Graph(plan)`.
Because that is the whole of a node graph's draw list, the Precomp layer arm, the matte arm and
the layer-input arm all reach it through the calls they already make, and a node graph placed,
matted with or fed to an effect needs nothing further. `nested_texture_key` and the frame-key
cache apply at the Precomp level as they do to any comp.

```rust
struct GraphDraw {
    width: u32, height: u32,
    steps: Vec<GraphStep>,          // topological order; a step's inputs index earlier steps
    output: Option<usize>,          // the step wired into Output; None draws nothing
}
enum GraphStep {
    Provided,                       // the first picture Input: the effect's own picture, or transparent
    Picture(LayerInputDraw),        // a further picture Input: a layer rendered alone, or Absent
    Read(Box<CompLayerDraw>),       // a project item at default placement, realised alone
    Fx { input: Option<usize>, ops: ResolvedStack, fx_ids: Vec<Uuid>,
         matte: Option<usize>, picture: Option<usize>,
         luts: Vec<Option<String>>, flare_lens: Vec<Option<String>> },
    Merge { a: Option<usize>, b: Option<usize>, mode: u32, opacity: f32 },
    Switch { inputs: Vec<Option<usize>>, index: i64 },
}
```

`Realiser::realise_graph(plan, provided: Option<&Tex>, w, h) -> Tex` walks the steps in order
and keeps one texture per step:

| Step | Made by |
|---|---|
| Provided | the texture handed in, else a transparent one |
| Picture | `render_layer_inputs`, the depth-pass road |
| Read | `realise` over the one draw, which is the matte path's "render alone at comp size" |
| Fx | `run_ops` over the step's one op, with `mattes` and `layer_inputs` bound to the upstream steps' textures as `LayerInput::Texture`, every other carriage empty, and no per-effect cache |
| Merge | the compositor: B at Normal and full opacity, then A over it with the mode and the opacity, both full-frame at identity, on a transparent ground. An unwired A gives B; an unwired B gives A over nothing |
| Switch | the texture of `inputs[index]`, transparent when that socket is unwired or the index is out of range |

An unwired image input anywhere reads transparent. A node whose output nothing consumes is not
in `steps` at all; the builder lowers only what the Output reaches, so an idle branch costs
nothing. `run_ops` is called once per Fx step rather than once per chain; fusing runs of
single-consumer nodes into one call is the upgrade if a profile ever asks for it, and it
changes no pixel.

**What the walk did not carry in v1**, each the same boundary a group header has
(group-effects.md §3): neighbour frames, flow fields, the below-stack of Posterize time and
accumulation motion blur, points schedules, paint, lighting, mask paths and roto mattes.
Round two carries the time-shaped ones and the points schedule (§5.1, §5.2). Paint, lighting,
mask paths and roto mattes stay out for a reason rather than a boundary: a graph has no
masks, no paint strokes, no lights and no roto layer, so there is nothing to carry.

**Built 2026-09-06.** `realise_graph(plan, provided, w, h, scale)` takes the raster and one
number, raster pixels per graph-frame pixel: the render scale when the graph is a comp, and
the host's own px factor when it is an effect, because a Read's placement and a box's px
parameters ask the same question. Transparent is no step at all rather than a step of its
own; a missing item, a folder, a cyclic comp and an unwired socket all read the same way. A
Read inside a graph applied as an effect is placed in the graph comp's own pixel space
scaled by that factor, which is exact when the graph's frame and the host's raster share a
coordinate frame and approximate otherwise, and the code says so where it happens.

### 2.4 The Node graph effect on a layer

`run_ops` cannot reach the realiser, and `realise_segment` runs a stack in one call. Rather
than split the stack at an op, which nothing does today, the realiser hands `run_ops` one more
side list: `graphs: &[&dyn Fn(Tex, u32, u32) -> Tex]`, one closure per enabled `node_graph`
op in stack order, bound by the k-th rule every other carriage follows. Inside the op loop a
`node_graph` op calls its closure in place of a kernel and carries on; the Mix seam and the
matte dissolve run after it as they run after any kernel. The closure is made in
`realise_segment` and is `|tex, w, h| self.realise_graph(plan, Some(&tex), w, h)`.

The builder makes the plan: `graph_fx_for(layer.effects)` lowers, for each enabled Node graph
op, the comp its `extra.node_graph.comp` names, with the instance's parameters as the
`overrides` and the instance's further picture rows lowered through `layer_slot` into
`GraphStep::Picture`. A comp that is not there, is not a node graph, or is already on the
`visited` path lowers to a passthrough closure, which is the degrade a dangling reference
gets everywhere else. A graph nested as a node inside another graph is the same op inside the
inner walk, under the same guard.

**Built 2026-09-06.** `run_ops_with_roto` is gone: one `run_ops` carries the roto mattes and
the graph closures, as its own note said it should the day a second side list arrived. The
`node_graph` op applies its Mix and the injected Blend itself through `blend_mix`, since
there is no kernel to apply Mix inside, and Mix 100 at Normal runs no pass. The per-effect
cache chain breaks at a `node_graph` op, whose picture is not in its bag. The adjustment
split hands `run_ops` real closures too, so a Node graph effect on an adjustment layer applies
to the composite below rather than passing through. The effect's graph is lowered at the
host layer's own time, the clock its decodes and its frame name were already read at, so a
layer with a start offset shows the graph at the moment its name says. A bypassed Merge,
Switch or nested Node graph box hands on the picture it was given (`in0` for a Switch,
`input` otherwise), which is what its tick means on every other box.

### 2.5 The frame key

`feed_comp` forks after the camera block, so every byte a layer comp feeds today is unchanged.
A comp with a graph feeds `b"graph-comp/"` and then, in document order: for each node its
kind tag; a Read node's source through `feed_source` on its synthetic layer, or `b"noitem"`;
an Input's five facts and default; an Fx node through `feed_effect_stack` with
`marker_layer: None` (the group-header call, which is why a `ParamKind::Layer` row's own
dangling marker is harmless: the wire is fed as an edge); the Output's id. Then every edge as
its four parts, length-prefixed. Layout, exposure and groups feed nothing. The Output's wire
being fed is what makes the Viewer's "at node" picture (§4.5) a different name with no field
added.

A layer whose stack holds a Node graph effect feeds, at that instance, `b"node-graph/"` and the
named comp's own key at the layer time under the `visited` guard (`b"cycle"` or `b"nocomp"`
otherwise), beside the instance's parameters, which the ordinary loop already feeds. The
further picture rows are `EffectValue::Layer` parameters, so the Layer arm feeds them already.

`identical_content_hashes_identically_across_instances` must keep holding: two node graphs
with the same content and different ids name the same frame, so the key feeds node ids only
where the layer graph does (the edge list), and content everywhere else.

**Built 2026-09-06.** The fork sits after the lights block rather than the camera block; a
node graph has neither, so the bytes are the same. Two things were fed that this section did
not list: each Fx box's `enabled` byte, since `feed_effect_stack` says nothing about a
bypassed instance and a bypassed box would otherwise keep the frame's name, and the graph's
own clock whenever a wire leaves a driver, for the reason the layer graph's fold gives (two
frames of a Wiggle-driven graph must not share a name).

## 3. Ops and undo

One new op, one guard, one error:

- **`SetCompGraph { comp, graph: Box<CompGraph> }`**, the whole-graph commit, shaped like
  `SetLayerGraph`: validated before the swap (§1.4), refused as `InvalidGraph`, inverse the
  previous graph. Add a node, remove one, wire, unwire, move, expose, name a group, rename a
  box, bypass one: each gesture is one op and one undo step. Parameter edits ride the staged
  instance and land through this same op, as a driver's do through `SetLayerGraph`.
- **`OpError::CompIsNodeGraph`** from the guard at the top of `apply` (§1.1), in both
  directions.
- `Op::name` answers "Edit node graph"; the phrase gets its `engine_labels.dart` entry and
  `app_en.arb` key with the op.
- `op_scope` files it as a comp-wide change with no layer, beside `SetGroupEffects`, so the
  comp model refreshes and no layer builder does.
- `TrimCompToWorkArea` and `CropCompToRegion` work on a node graph as they do on an empty
  comp: the duration and the frame move, and there are no layers to slide.
- There is no command that duplicates a comp, node graph or otherwise. The one road that
  copies boxes between graphs is a saved group (§5.8), which mints fresh node ids and
  re-points its wires on `preset::group_instantiated`'s model, because a node graph's ids
  are resolved across the comp and a copy that kept them would alias its original's boxes.
- `SetLayerGraphInputs { layer, inputs }` (§5.3), the whole-instance commit for a placed
  graph's Inputs, named "Edit node graph inputs".

## 4. The surface

### 4.1 Bridge

Mirrors of the layer graph's calls, on `CompositionReference`, all sync, whole-graph, one op:

- `get_node_graph() -> BridgeCompGraph { nodes: Vec<BridgeCompNode>, wiring: BridgeCompWiring }`.
  `nodes` is derived (id, kind, item reference for a Read, match name, label, custom name,
  enabled, missing, `inputs` and `outputs` as `BridgePort`), read whole and never written
  back. `wiring` is stored: the Read and Input and Output nodes as data, edges, positions,
  exposure and groups, keyed by `Uuid`, and is handed straight back to `set_node_graph`.
- `get_node_graph_instances() -> Vec<BridgeEffectInstance>`, the staged Fx nodes at offset
  zero, since a graph's clock is the comp's, as a group header's is.
- `new_graph_instance(name) -> BridgeEffectInstance`, uncommitted, as `new_driver`.
- `set_node_graph(instances, wiring)`, the commit.
- `ProjectReference::new_node_graph(name, settings) -> CompositionReference`, which is
  `new_composition` with the graph seeded: one Output node, auto-placed. Named "Node graph N".
- `BridgeCompModel.is_node_graph: bool`, the fact the Timeline, the Graph panel and the
  project row read for nothing (docs/17's rule).
- `LayerReference::add_node_graph_effect(graph: &CompositionReference)`, which instantiates
  `node_graph` bound to that comp and writes the Inputs copy. `BridgeEffectInstance::node_graph_comp()`
  answers the bound comp for the panel's header.
- `list_graph_nodes()`, the node graph console's catalogue: Drivers and Compositing.
- `BridgePrefixPoint` grows a second shape naming a comp and a node (§4.5).
- `RenderCompRequestWithPreview` grows `graph_instances: Option<Vec<EffectInstance>>`, patched
  onto the throwaway clone's graph before any layer is looked up, so dragging a node's number
  moves the picture live as a driver's does.

Refusals cross as `BridgeError::OpError` and the panel says its own calm sentence, because a
sync `BridgeError` reaches Dart opaque. No colour crosses; port types cross as the enum they
are. docs/17 gains a section beside "The layer graph".

**Built 2026-09-06.** As landed: `new_graph_instance(name, graph)` binds a nested Node graph
box when `name` is the effect and `graph` names a node graph, and refuses otherwise;
`render_frame_with_graph_preview(frame, scale, instances)` is the live drag, on a preview
request whose `layer` became optional (nine constructors gained a line, none a shape);
`BridgePrefixPoint` is `{ layer: Option, effect: Option, graph: Option<BridgeGraphPoint {
comp, node }> }`, which left the one Dart call site untouched; an Input's unit crosses as the
`BridgeUnit` that already crossed; `list_effects` leaves the bare `node_graph` out as well as
the Compositing family, since it is added through its own door; a Read whose item is gone
crosses with an empty label and `missing` set; the default name is "Node graph N", counted
apart from "Comp N"; and one `with_live_inputs` helper refreshes a Node graph instance's
Inputs copy on every clone the bridge hands out.

**Built 2026-09-08, round two.** Six more, and no new idiom with them:
`BridgeLayerInfo.graph_inputs` and `LayerReference::get_graph_inputs()` for a placed graph's
Inputs, committed through `set_effects` under `InstanceHome::GraphInputs` and previewed
through the request's own `effects` field; `BridgeCompModel.graph_boxes` for a box's rows in
the Timeline; `CompositionReference::save_graph_group(name, colour, nodes)`,
`insert_graph_group(text, x, y)` and the free `list_graph_groups()` for saved groups;
`BridgeLayerInfo.collapse_forced` for the dimmed cell; `BridgeGraphInput.preview` for a
picture Input's stand-in; and `list_graph_nodes` filtered through the engine's own
`offered_in_graph`, which is what leaves Layer points out.

### 4.2 The Graph panel's third subject

The panel's subject today is the selected layer, or a Custom shader entered from it. It gains a
third: **when the selected comp is a node graph, the canvas draws the comp's graph**. Entering
a node graph is opening it, from the project panel, a comp tab, a Read node of it, or a Node
graph effect's Open action, so there is no new entry road and no breadcrumb of its own; the comp
tabs are the way back. Selecting a layer is impossible in a node graph, so the two subjects
never compete.

The canvas is shared, not copied. Before the third subject is drawn, the layer canvas's box,
socket, layout, drag and card types are moved off `BridgeGraphNode` and `BridgeNodeRef` onto
the string key they already index by plus a small record (key, title, custom name, enabled,
derived, inputs, outputs), which changes nothing the layer panel does and is what
`shader_graph.dart` had to duplicate by hand. The node graph's file then imports the card and
the layout as well as the painters and the metrics. What it does not import is the layer
canvas's chain machinery (the derived chain painter, `_chainDrop`, the reorder gestures,
`_isChainType`), which exists only because a layer's image chain is its effect list. A node
graph stores its image edges, so they draw through the ordinary stored-edge loop, an image
socket takes a drag like any other, and branching falls out of the rule the layer canvas
already has: only the destination is exclusive.

The gestures are the layer canvas's: drag to wire, pick up a wire by its far end, drop a wire
on empty ground for the console filtered to its type, drop a loose box on a wire to splice it,
Auto-wire (a box added while one is picked is wired after it, and takes over feeding the
Output if the picked box did), Heal (deleting a box joins what fed its `input` to everything
its `output` fed), marquee and modifiers, groups and their washes, frame-all, snap, rename
by double-click, the tick and the twirl, Delete claimed while the panel is focused. Save group
to a file was not offered in v1 for a node graph, since the `.lumgrp` preset carries
layer-graph edges; round two adds the second preset kind (§5.8).

The console over a node graph lists, in this order: the project's items as Read nodes (name,
kind as kicker), Input (four kinds), effects, Merge and Switch under Compositing, drivers.
Dragging footage or a comp from the project panel onto the canvas makes a Read node where it
lands; the payloads are the ones the Timeline already accepts. A Read node of a comp opens that
comp on double-click; a Node graph node likewise.

The type of a wire decides its colour and the legend is unchanged: an image wire is the
image colour whether it feeds an `input`, a `background` or a `matte`.

**Built 2026-09-06.** The shared card record carries three marks (tick, twirl, rename) and a
kicker word rather than one "derived" flag, since a Read box takes a name but no bypass. A
Read whose item is gone wears the dashed border and the word "missing" as its kicker, so no
key on the layer canvas moved. The canvas's console drops any effect whose name
`list_graph_nodes` already offers, because `list_effects` carries the drivers too, and it
finds the project's items by walking folders, since `get_items` answers one level. A
catalogue entry's own listing declares no picture ports, so the canvas adds `input` and
`output` for auto-wire and the console's type filter. The layer canvas gained one gesture:
double-clicking a Node graph box opens the comp it is bound to.

### 4.3 The Node panel

The panel follows the picked box, as it does for a layer's graph. For an Fx node it draws the
instance's rows with `EffectParamRowFrb`, writing through a comp-graph editor of the same
twenty-five lines the driver arm is, and previewing through the new request field. The rows
are handed an empty layer list, and the Layer and MaskPath arms draw their dash for an owner
with no layers rather than a picker, since in a node graph those rows are sockets. For an
Input node it draws the five facts as a form: kind, label, default, min, max, unit, the one
the shader's Parameter node uses. For a Read node it draws the item's name, kind and an Open
action for a comp. The Output node has no rows.

**Built 2026-09-06.** The pick the canvas publishes is `compGraphNode`, a record of the box's
id, its drawn name and whether it makes a picture, so the Viewer's chip can name a box and
know whether to offer itself without learning what a graph is. `viewerGraphPoint` sits beside
the layer point rather than widening it, and `viewerPrefix` reads the graph point first. The
comp model exposes `isNodeGraph` as it exposes `compGone`, off the held read, so the Timeline
and the Graph panel branch for nothing. `currentLayer` in the row widget became nullable and
its four callers guard it, which is what lets a graph box's rows draw with no layers to pick
from.

### 4.4 Project panel, Timeline, Effect controls

- The project row draws a node graph with the `nodes` glyph and the type word "node graph".
  **New node graph** is offered on the Composition menu, in the command palette and on the
  project panel's context menu beside New composition; it opens the composition settings
  dialogue with the name prefilled. The bottom bar keeps its three words.
- The Timeline, on a node graph, shows the ruler and one line of hint text in place of the
  layer rows: "This composition is a node graph. Its boxes are in the Node graph panel."
  The empty-comp hint does not fire, because the comp is not empty.
- The Effect controls draw a Node graph effect's card with the graph's name in its header and
  an **Open** action row that fronts the graph, then the derived rows, then Mix and the Matte
  row. The Custom shader's card is the shape.

**Built 2026-09-06.** New node graph is one funnel in the app state, reached from the
Composition menu, the palette and the project row's menu (the panel has no empty-area menu,
and the footer keeps its three words); it opens the composition settings dialogue with the
node graph door set and lets the engine name the comp. The project panel keeps the per-item
node graph fact beside the names it already caches, one read per comp per document change.
The Timeline keeps its comp tabs and its bar over a node graph, since the tabs are the way
back out, and draws the hint where the layer table would go; the ruler lives inside that
table and goes with it, as it does for an empty comp. The Input form's wells commit once per
drag, a nested box's Input rows draw as derived rows, and a layer row with no layers to pick
from draws its dash. The layer's console offers each node graph once, under Node graphs,
applied rather than fronted; the palette still fronts every comp. The card's Open row fronts
the bound comp and does nothing when the comp has gone.

### 4.5 The picture at a node

Selecting a box in a node graph offers the Viewer's "at" chip, as selecting an effect does.
On, the worker renders a patched copy of the document whose Output is wired to the picked
box, through the ordinary frame path at full quality, named differently by construction
(§2.5). `cut_to_prefix` grows one arm; `viewed`, `set_prefix` and the six places that call
them need nothing. A driver, a value Input and the Output itself offer no chip: the first two
make no picture and the last is the picture the Viewer already shows.

## 5. Round two: what v1 left out, and how each is built

v1 named eleven boundaries. Round two lifts every one of them. Each subsection is the
design its package builds from, and a Built paragraph joins it as it lands. The rule that
held in round one holds here: nothing is faked, and a degrade is a still or a passthrough,
never a fault.

### 5.1 Points wires in a graph

`ports_of` draws every socket a signature declares; the type skip goes. Nothing in the
catalogue declares a Shape port, and the one Audio port is the Layer out's, so those two
kinds appear the day an effect declares one and need no rule of their own. A points socket
is drawn teal, one per declared port, and the canvas's no-stream mark and the Node panel's
row word follow from the read model with no Dart on that path; the Node panel's comp arm
fills `noStream` from the box's inputs.

The rules are the ones §1.4 has. The layer graph's positional rule (`flows_down_the_stack`)
has no meaning over a graph's boxes and is not applied; `CompGraph::check_acyclic` already
walks every wire, image, value and points alike.

**The projection carries the wire.** A points wire becomes `OutputRef::EffectData { effect,
port }` into `InputRef::Param` in `Projection::drivers`, the shape a layer stores it in, so
`Eval::points_input` finds it as it does on a layer. What the walk lacked was the producer:
`Eval::stream` looked it up through `context.layer` in `comp.layers`, and a graph has no
layer. `Eval` gains `stack: Option<&[EffectInstance]>`, the instances an `EffectData` wire
names when the graph is not a layer's; with it set, the producer is read from the slice
(no masks, effects on) and never from the document. `resolve_drivers`, `effect_stream` and
`driver_stream` gain a twin taking the slice, and the old names call it with `None`. The
memo, the budget, the one-hop bound, Trail's back samples and the "picture-fed producers
hand out no stream in the driver walk" refusal (points-stream.md §3.3) transfer unchanged,
because none of them read the layer.

**Layer points has no home in a graph.** `list_graph_nodes` leaves it out: a graph has no
layers to tap, and the wire is the tap. A hand-edited one reads the empty stream.

**The carriage.** The per-op body of `points_schedules_for` becomes a free function the
layer closure and the graph lowering both call, so the scan of Emit rate, the back-sample
loop and the time rule (`sample_temporally` picks `t` or `frame_t`) exist once.
`GraphStep::Fx` gains `schedule: Option<PointsSchedule>`, and `realise_graph` hands
`run_ops` a one-element slice or an empty one, which satisfies the k-th rule trivially
since a step holds one op. The projection is `FLAT`: a graph has no camera and no layer
transform, and the raster factor `realise_graph` already rescales ops by rescales the
stream. `input_from` is `None`; the frame key covers the producer by feeding its box and
the wire, which a test pins for a Particulate feeding a Trail.

### 5.2 Time inside a graph

**One rule.** A box's input at another time is its input cone evaluated at that time. On a
layer a temporal effect reads the source frames and not the ops before it (docs/08
§3.13), which stays. In a graph a wire means "the picture this box makes", so Echo reads
its input box at the previous frames, Motion blur and Datamosh measure their input box
against itself a frame away, Posterize time holds its input at the grid time,
accumulation motion blur averages its input over the shutter, and a Time offset box shows
its input at another time. A Read is a box like any other, so a Read at another time is
that footage or that comp at that time.

**Which times.** `fx::temporal::input_times(inst, t, dt) -> Vec<f64>` answers the times a
box asks its input at when it is asked at `t`, the box's own time first: Echo, Motion blur
and Datamosh give `t` then `t + o·dt` over their window (`frames_needed`, else the traits);
Posterize time gives the held time alone; accumulation motion blur gives `t` then each
shutter moment from `sample_offsets()`; Time offset gives `t + offset` alone; every other
box gives `t`. **The bound** is the layer path's `strip_temporal_inputs` rule, stated once:
at any time other than the graph's own `t`, only the first entry is taken. A shift always
applies, so a Time offset before an Echo offsets every neighbour, and a window never
nests, so an Echo inside an Echo's neighbour holds a still. `CompGraph::time_demands(root,
t, dt, input_times) -> Vec<(Uuid, f64)>` is the worklist over the root's cone under that
rule, deduplicated on exact bits, and it has three readers that cannot disagree: the
lowering, the planner and the frame key.

**The plan.** Steps are keyed by node and time bits in one plan, and `wired(node, port,
τ)` resolves a socket at a time. A Read at `τ` is its own step with its own decode; two
demands of one Read at one time share one step and one job. Echo, Motion blur and
Datamosh: `GraphStep::Fx` gains `neighbours: Vec<(i32, usize)>` (offset to step) and
`flow_neighbours: Vec<i32>`, the realiser gathers the textures, measures the flow on the
card through `measure_flow` (the below-stack road, `None` degrading to none) and hands both
lists to `run_ops`. Posterize time and Time offset lower to no step: the box's output is
its input at the shifted time. Accumulation motion blur lowers to `GraphStep::Accumulate {
input, samples: Vec<usize>, mix, matte, channel, invert, anchor }`, realised by
`Compositor::accumulate` at equal weights or `accumulate_with_shutter` when a matte is
bound, then the Mix. `realise_graph` drops a step's texture after its last consumer, so a
neighbour set costs what a layer's neighbour set costs and no more. `strip_temporal_inputs`
recurses into a graph draw and clears these lists, so a graph inside a held or sampled
below-stack holds a still as a layer does.

**The planner** plans one job per Read per demanded time, keyed by the node id at `t` and
by `Uuid::new_v5(node, τ bits)` otherwise, so every id round one filed is unchanged. A Read
of a comp at `τ` recurses into that comp at `τ`. **The key** feeds `b"shifted/"` after the
node loop, then each demanded `(node, τ)` with `τ ≠ t` as the node's content at `τ`, a
Read through `feed_source` and an Fx box through `feed_effect_stack`; a graph with no
temporal box feeds nothing new and keeps every name it had.

**`dt`** is the graph comp's frame when the graph is a comp and the host comp's frame when
it is an effect, as the raster is the host's (§1.6).

**Provided at another time.** Applied as an effect, the first picture Input at `τ ≠ lt` is
the host layer's own neighbour at that whole-frame offset, and the frame-time picture when
the host has none (a fractional shift reads the frame-time picture too). The host plans
those neighbours through `fx::temporal::layer_temporal_window(doc, layer, lt)`, which
unions `stack_temporal_window` with each enabled Node graph effect's integer demands on
Provided; the planner and the key's `b"temporal/"` block both call it, so they agree. The
graph closure on `run_ops` takes the host's neighbour list beside the picture. A nested
graph is lowered into the outer plan, so its boxes take node-and-time steps as the outer
ones do and its Provided is the outer box's input at the shifted time.

**Posterize on the host reaches the graph.** The planner, the builder and the key lower a
Node graph effect at `this_layer_effect_time` rather than at `lt`, the time the layer's
own ops already resolve at.

**Time offset** is a new Compositing entry, `time_offset`: one row, Offset in seconds
(`Unit::Seconds`, keyframeable, default 0), sockets `input`, `offset` and `output`,
`is_image_op() == false`, realised by the walk and refused on a stack as Merge is. It is
the per-node time v1 had none of; Precomp retime (§5.6) is the other half.

**Built 2026-09-08.** `time_demands(root, t, input_times)` takes no `dt`: the closure a
caller hands in already holds the frame it measures windows in, and a second copy on the
call could only disagree with it. `input_times` and `layer_temporal_window` are public as
`fx::input_times` and `fx::layer_temporal_window`, since `fx::temporal` is a private
module; `layer_temporal_window(doc, layer, lt, dt)` takes the host comp's frame as its
last argument.

### 5.3 A placed graph's Inputs

`Layer` gains `graph_inputs: Option<EffectInstance>` with the `retime` field's serde
attributes: a `node_graph` instance bound to the layer's own comp, meaningful only on a
Precomp layer of a node graph. `add_precomp_layer` binds one when the comp is a graph.
`Op::SetLayerGraphInputs { layer, inputs: Option<Box<EffectInstance>> }` is the commit,
shaped like `SetLayerStyles`, named "Edit node graph inputs". The layer's own driver wires
reach its rows as they reach a Node graph effect's, since the instance resolves under its
own id.

**Reading it.** One helper, `graph_comp_draw(doc, comp, t, frame_t, view, ...)`, lowers a
graph comp to its one draw for every road that reaches a comp: the node-graph arm of
`build_comp_draws_at`, the Precomp arm, `nested_comp_draw` and the Read step. `view` is
`GraphView { values: &[(String, EffectValue)], pictures_fed: bool }`, and the Precomp arm
and `nested_comp_draw` fill `values` from the layer's instance through
`node_graph::overrides_of(inst, graph, drivers)`, which moves from build.rs into core so
the key can call it too. **Caching.** A placed graph with its own values is a different
picture from the same graph with defaults, so the nested name folds the instance:
`NestedKeyer::nested_key(nested, lt, inputs: Option<&EffectInstance>)`, and the planner's
`HeldNested` takes the same instance, so what the builder names and what the planner
skips agree. The `precomp/` arm of the key feeds the instance through `feed_effect_stack`
beside the nested comp's key.

**Bridge.** `BridgeLayerInfo.graph_inputs: Option<BridgeEffectInstanceInfo>`, with the
Inputs copy refreshed by `with_live_inputs` as every clone's is, and
`LayerReference::get_graph_inputs() -> Option<BridgeEffectInstance>`, which hands a fresh
bound instance when the layer has none and its comp is a graph (a layer from an older file
or an import), offered and never adopted. `InstanceHome` gains `GraphInputs`, found by id
and, failing that, by being a `node_graph` instance on a Precomp layer of a graph, so
`set_effects` commits it and the preview request patches it by the same rule with nothing
new on the Dart side of the seam.

**Dart.** The Effect controls draw a section after Styles headed by the graph's name with
the Open action, one card, the rows. The Timeline folds it under `graphPath(id) =
'$id/graph'` beside Styles; `foldRowPath`, `moveLaneKeys`, `graphChannels` and
`EffectStackEditor.stackWith` each gain the arm the Styles arm is.

**Built 2026-09-08.** `Op::SetLayerGraphInputs` carries `comp` beside `layer`, as every
other layer op does: the apply finds the layer through it, and the lock guard, the node
graph guard and `op_scope` all read it.

**Built 2026-09-08, the bridge.** The rule `InstanceHome::GraphInputs` and the preview both
ask is one function, `api::layer::is_graph_inputs(doc, layer, instance)`: the stored id when
the layer has Inputs, and a `node_graph` instance bound to the comp the layer places when it
has none. `instance_home` takes the instance rather than its id, since the fallback is a
question about what the instance is bound to and not about which list holds it; the three
commands that called it with an id pass their own staged copy. `set_effects` takes one more
line for the adoption: a staged list of one against a document that holds none is the first
commit rather than a stale stack. `get_graph_inputs` seeds the offered instance at the
graph's own frame size, which is where a positional default belongs.

### 5.4 Per-box cache

`CompGraph::cone_of(node) -> Vec<Uuid>` is the lowering's own cone-and-Kahn walk moved into
core, in the order the lowering steps in. lumit-eval gains
`graph_box_input_key(doc, comp, node, t, view, quality, stamper) -> Option<u128>`:
`b"graph-box/"`, the view's values, then the box's cone without the box at the times
`time_demands` asks with the box as root, each node's content and the wires among them,
the very feeds `feed_comp_graph` writes. `NestedKeyer` gains `graph_box_key`, answering
`None` in draft as `nested_key` does; `GraphStep::Fx` gains `input_key: Option<u128>`;
`realise_graph` hands `run_ops` the cache with it. `op_keys` breaks its chain on a bound
matte or picture texture because nothing names them; the graph's call says they are named
(they are inside `input_key`), so the break is lifted there and nowhere else. The raster
and the flare state are already inside `op_keys`, and one `(w, h)` never sees two scales in
one `realise_graph` call, which the code says where it matters.

### 5.5 Expressions read an Input

`ExpressionContext` gains `inputs: Option<Arc<[(String, EffectValue)]>>`, carried by
`increase_depth`; the other literals gain `inputs: None`. The comp module gains
`input(name) -> f64`: the value named by an Input's id or label, from the context's
overrides evaluated at `comp_time` under `increase_depth`, else the graph's own default read
from the document through `context.comp`; a Colour answers its first channel; a picture
Input or an unknown name answers the module's miss value. The lowering sets the field from
the view's values, and `feed_comp_graph` takes the view and builds the same context, so the
key evaluates what the render evaluates and a host's values are part of the graph's name
whichever cache asks.

### 5.6 Precomp retime

The model, the ops, the bridge, the Retime row, the graph channel and the un-retime re-hang
already work on a Precomp layer, and two bridge tests prove it. What was missing were the
sites that evaluate a nested comp, every one of which passed layer time straight through.
Each now maps it: `let st = layer.source_time_at(lt)` at the top of the Precomp arm, and
`st` (and `frame_st`) into the collapsed splice, the nested build, the flow neighbours'
rebuilds, `camera_pose` and `nested_key`; the same in `nested_comp_draw` for the matte and
layer-input roads; the same in the planner's Precomp arm and its held check; the same in
the key's `precomp/` arm. `collapse_state` and `paint_time` keep `lt`: the layer's own
opacity and paint are not remapped (docs/04 §11.6). **Overrun** under a Retime map clamps
`st` to the nested comp's span, holding the boundary frame as docs/04 §11.3 and
`entry_time` do; an un-retimed Precomp is unchanged and still draws nothing past the
nested duration. **Interpolation** stays unread on a Precomp: a comp evaluated at an exact
time has no in-between frame to invent, which is why the dropdown and the Flow cell are
footage-only. Comp motion blur still smears a Precomp's transform and not the nested
picture's own motion, retimed or not, which docs/04 §11.4 now says plainly.

**Audio.** One guard above the kind match in the audio walk: a layer with a Retime map
contributes nothing, footage and Precomp alike, which is the rule docs/04 §11.5 and docs/09
§7 state for a layer and the mix enforced only for a clip. `kind_has_audio` stays blind to
it, so the row keeps its mute cell, drawn dimmed with the tip "Retimed layers are silent".

**Dart.** The Retime row's clock face reads the nested comp's rate for a Precomp layer, not
the parent's.

### 5.7 A box's keys in the Timeline

On a node graph the Timeline draws the ruler and one row per Fx box in document order:
a heading row with the box's name and twirl, then, open, one parameter row per row the box
has, with the diamonds a layer's effect rows have. A box has no span, so no bar is drawn. A
graph with no Fx box keeps the hint.

`BridgeCompModel` gains `graph_boxes: Vec<BridgeEffectInstanceInfo>`, the boxes with their
live Inputs, empty on a layer comp, so the rows read the held model and ask the engine
nothing. The rows are `FoldGroupRow` and `FoldEffectParamRow` with a third discriminator,
`graphBox`, under the prefix `n:<box>` and the path `n:<box>/<param>`: two segments, so
`effectIdOfPath` and `styleIdOfPath` stay silent and `isUnderPath` still contains.
`LayerRow.entry` and `SelectedKey.entry` become nullable, keeping one row list and one
height so the lazy blocks, the drag arithmetic and the alignment tests hold.
`KeyLane.entry` goes, it was never read. A drag commits through a callback
`commitKeyGesture` tries before `moveLaneKeys`; delete, ease, plant and stagger commit
through an optional `write` closure on `GraphChannel` that `commitChannelEdits` tries first;
both commit `setNodeGraph` with the staged instances and the stored wiring, and both close
the gap a group header's keys still had. `graphChannels` takes the boxes beside the layers
and gains the `n:` arm, so the Graph editor plots a box's channels. `_keysIn` and `_rowAt`
walk header and box rows as `_selectedKeyPlaces` already does.

**Built 2026-09-08, the bridge half.** `graph_boxes` is filled with the same pair every
other read model uses, `with_live_inputs` then `read_instance_info`, so a nested Node graph
box's own rows are as live on a Timeline row as they are on a card.

### 5.8 Saved groups for a graph

A second preset kind beside `GroupPreset`: `CompGroupPreset { format, name, colour, nodes:
Vec<GraphNode>, edges: Vec<GraphEdge>, layout }`, the Output never among the nodes, layout
relative to the set's top-left, extension `.lumngrp` so neither listing can offer the
other's file. `comp_group_from_graph` and `comp_group_instantiated` are the layer pair's
twins: fresh ids, edges re-pointed and dropped when an end is missing, layout offset. A Read
keeps its item id and lands wearing the missing mark in a project that lacks it, which is
how a deleted item already reads. `CompositionReference` gains `save_graph_group(name,
colour, nodes)`, `insert_graph_group(text, x, y)` (one `SetCompGraph`, the Output left
alone) and `list_graph_groups()`. The comp canvas gains the Save group button, live for any
picked box but the Output, and a saved-groups block in its console guarded as the layer
canvas's is; the file dialogue gains the type.

**Built 2026-09-08, the bridge half.** A group text carrying an Output is **refused whole**
rather than having the Output dropped on the way in: nothing Lumit writes carries one, so a
file that does was written by something else, and refusing it entire is what a bad wire and
a mistyped drop already get. The refusal is `SetCompGraph`'s own `SecondOutput`, so the
document is left exactly as it was and the panel says the engine's sentence.
`list_graph_groups` is a free function beside `list_node_groups`, both over `presets_in`,
whose probe already accepted a `nodes` array.

### 5.9 Collapse on a placed graph

`collapse_state` answers `Forced` when the nested comp is a node graph, one clause beside
`inner_forces`, so the render path is exact: a graph draw needs its intermediate for the
clipping collapse would skip. Nothing reported a forced collapse to the panel before;
`BridgeLayerInfo.collapse_forced` does now, read from `collapse_state` at the model's
time, and the Timeline's collapse cell draws dimmed with the tip "Collapse is forced off"
whenever it is set, node graph or any other reason.

**Built 2026-09-08, the bridge half.** The comp read model carries no playhead, so the
answer is read at the layer's own in point. Every term of the rule but one is time-free, so
the reading only ever drifts on a layer whose opacity is keyframed across 100%, and it says
so where it is built.

### 5.10 Audio

The audio walk's per-layer body becomes `walk_layer`, and the walk gains a graph arm: for
each Read box in the Output's cone, `read_layer` with its out point reopened as the picture
path reopens it, handed to `walk_layer` at unity with the strip being the node when the
graph is the comp being mixed. A Read of footage contributes like a footage layer, a Read of
a comp like a Precomp layer, and the graph's own master volume applies as any comp's does.
`kind_has_audio` consults a nested graph's Reads, so a Precomp row of a graph wears its mute
switch. Beats and the Audio level driver's "this comp" mix build from jobs, so both start
answering with no further change.

### 5.11 A picture Input's preview item

`GraphInput` gains `preview: Option<Uuid>`, serde-skipped, an item to stand in for the
picture when nothing feeds the Input: viewed on its own or placed as a Precomp layer, never
applied or nested, which is `GraphView.pictures_fed`. The lowering lowers such an Input as
a Read of the item; the key feeds the item's source in that view. The Node panel's Input
form gains a picker of the project's footage and comps for a picture Input.

**Built 2026-09-08.** The field is on the model, and `BridgeGraphInput` carries it both
ways, so a graph read out and written back keeps the one it has. The Node panel's picker is
what still owes it a way in.

## 6. Traps

- **Do not reuse `NodeRef::Driver` for a node graph's boxes.** `prune_to`, `preset::driver_id`,
  `prefix_len` and `BridgeNodeRef::core` all read that variant with layer meaning.
- **`ParamKind::Layer` answers no `port_type`.** The socket for a layer-reference row is the
  node graph's own rule in `ports_of`, not the parameter's.
- **`layer_input()` sees the schema, not the derived rows.** A Node graph effect's further
  picture rows are carried by the graph's lowering, or they are not carried at all.
- **The frame key hashes `comp.layers` for a `Layer` reference.** A wire into a matte or
  layer-input socket is fed as an edge plus the upstream node's content, never through that
  arm.
- **Every `Composition {` literal grows a line.** There are about a hundred across the
  crates, all tests but two, and `matrix_layer` in headless.rs and `add_comp` in the bridge
  tests are the two that bite first.
- **Name Merge's row `mode`.** See §1.3.
- **`derived` has no document.** The Inputs copy on the instance is the declaration; refresh
  it on clones, never in the document behind the user's back.
- **A refused `SetCompGraph` reaches Dart as an opaque error.** The panel declines a bad drop
  itself, from the types in the read model, and the engine is the backstop.
- **Write files with LF endings.** A CRLF file reads as a whole-file rewrite in review.

## 7. Test plan

Engine, `lumit-core` unless said:

1. An untouched project gains no `graph` key and re-saves byte for byte; a node graph
   round-trips whole, nodes, edges, positions, groups.
2. `SetCompGraph` accepts a well-formed graph and refuses each of: an unknown node, an
   unknown socket, a mistyped wire, a second wire on one input, a loop through image edges, a
   loop through a driver and an Input, no Output, two Outputs. Refusal leaves the document
   untouched. Undo restores the previous graph.
3. `AddLayer` on a node graph, and `SetCompGraph` on a comp with layers, are refused with
   `CompIsNodeGraph`, including inside a `Batch`.
4. `ports_of` for each node kind: an image effect with a matte row and a layer row shows
   `input`, `matte`, the row's socket and its number sockets; Merge shows A, B and Opacity;
   Switch shows one spare socket beyond the last wired; a driver shows what it shows on a
   layer; points sockets are absent.
5. `project`: a driver into an Fx parameter substitutes as it does on a layer; an Input wired
   into a parameter bakes its default, and bakes the override when one is given; an Input
   wired into a driver's socket bakes into the driver.
6. `read_layer` names the item, spans the comp and sits centred; `item_is_used` and
   `comp_footage_items` see a Read node.
7. The Node graph effect's derived rows follow the Inputs copy: a Number Input is a Float row,
   a Colour a Colour row, a second picture Input a Layer row, the first picture Input no row;
   `get_effects` refreshes the copy from the live graph on the staged clone only (bridge).
8. Frame key (`lumit-eval`): a node graph names a frame; a parameter edit, a rewired edge, a
   changed Input default and a Read swapped to another item each rename it; moving a box,
   twirling one, or naming a group keeps it; two node graphs of identical content and
   different ids share a name; a comp with no graph keeps the name it had; a host layer's
   frame renames when the graph it applies changes, and holds when the graph's layout does;
   an unprobed Read makes the frame unkeyable.
9. Render (`lumit-render/tests/node_graph_end_to_end.rs`, skip on no GPU):
   - a Read of a solid wired to Output renders the solid, and a swapped Output wire renders
     the other Read;
   - one Read forked into two effects and merged renders what the two-layer comp with the
     same effects and blend mode renders, at full and at half preview resolution;
   - Merge with an unwired A gives B, with an unwired B gives A, and Opacity 50 halves A;
   - Switch shows the picture its Index names, transparent out of range, and a Wiggle into
     Index changes the picture between frames;
   - a node graph placed as a Precomp layer renders inside the parent as a layer comp of the
     same picture does; the same for a node graph used as a matte source and as a Light wrap
     background;
   - the Node graph effect on a solid equals the same effects applied inline; a second
     picture Input fed from a layer row equals the depth-pass carriage; a value Input keyed on
     the host moves the picture; a graph nested in a graph equals the flattened graph;
   - a cycle through a Node graph effect degrades to a passthrough and renders;
   - preview equals export on a node-graph row of the matrix and on a host-effect row.
10. Decode plan: a Read of footage plans one job under the node's id; a Read of a comp plans
    that comp's; a Node graph effect on a layer plans the graph's footage.
11. The picture at a node: the patched copy renders the picked box's output and names its own
    frame; the Output offers no cut.
12. Undo symmetry over a random sequence of `SetCompGraph` and parameter edits.

Bridge (`api/tests.rs`): the graph crosses in one call; a box added, wired and undone is one
op; a refused wire is an `OpError`; `new_node_graph` seeds an Output and files the comp;
`add_node_graph_effect` binds the comp and writes the Inputs copy; the model carries
`is_node_graph`; a live drag substitutes the instance and touches no document.

Round two, engine (`lumit-core` unless said):

13. `ports_of` draws a Particulate's points output and a Clone to points' points input
    (the rewrite of `a_points_socket_is_not_drawn`); a points wire validates, a mistyped one
    is refused, a loop through a points wire is refused.
14. `project` turns a points wire into an `EffectData` edge; `Eval` with a stack finds the
    producer with no layer in the context, memoises it once a frame, refuses a Scatter as
    the layer walk does; `list_graph_nodes` leaves Layer points out (bridge).
15. `input_times` for Echo, Motion blur, Datamosh, Posterize time, accumulation motion
    blur, Time offset and a plain effect; `time_demands` over a fork, over a Time offset
    before an Echo (every neighbour shifted), over an Echo inside an Echo (a still), with
    exact-bit dedup; `layer_temporal_window` unions a Node graph effect's demands.
16. `Layer.graph_inputs` round-trips and is absent from an older file; `SetLayerGraphInputs`
    undoes; `overrides_of` prefers a driver's value, skips the declared ids.
17. `cone_of` orders as the lowering does; `input("x")` reads an override, a default, a
    colour's first channel, and misses on a picture and an unknown name; the same
    expression evaluates the same under the key's context (`lumit-eval`).
18. `CompGroupPreset` round-trips, never carries the Output, instantiates with fresh ids and
    drops a wire to a node the file lacks; a Read keeps its item.
19. `collapse_state` answers `Forced` for a node graph; `GraphInput.preview` round-trips and
    is absent when `None`.
20. Frame key (`lumit-eval`): a Particulate feeding a Trail renames per frame; an Echo box
    renames when its input's previous frame changes; a placed graph's Input value renames
    the parent and the nested name; a box's input key moves with its cone and not with a
    sibling branch; a retimed Precomp's key follows `source_time_at`, and two layer times
    that map to one source time name one frame; a preview item renames a graph viewed on
    its own and not the same graph applied.
21. Render (`node_graph_end_to_end.rs`, skip on no GPU): a Grid wired to a Clone to points
    equals the same on a layer; a Particulate box draws nothing at frame zero and
    something later; an Echo box on a Read of footage equals Echo on a layer of it; a Time
    offset of one frame equals the Read a frame later; Posterize time in a graph holds; an
    accumulation box smears a moving Read and is identity on a still one; a Node graph
    effect's Echo reads the host's neighbours; a placed graph with a keyed Input moves the
    picture and equals the Node graph effect on a solid with the same values; a box's
    second render at one frame is a cache hit and the picture is bit-identical; a preview
    item shows when the graph is viewed and not when it is applied; a retimed Precomp at
    frame 2N equals the un-retimed one at frame N, a freeze holds, the boundary frame holds
    past the end, and the same precomp as a matte source and as a Light wrap background
    follows its map; preview equals export on a temporal graph row, a retimed Precomp row
    and a placed-graph-with-values row of the matrix.
22. Decode plan: a Read under an Echo plans one job per demanded time under the derived id
    and the node id at `t`; a retimed Precomp plans and holds by `st`; `same_decode`
    separates two times of one Read.
23. Audio: a graph's Read of footage mixes at unity as a footage layer does, and a Precomp
    layer of it carries it with the layer's Volume; an idle Read is silent; a retimed
    footage layer and a retimed Precomp contribute no jobs, and the row keeps its mute
    switch; beats are found on a graph.
24. `effect_examples` skips Merge, Switch, Node graph and Time offset as unillustrable,
    and Split and Combine with the drivers.

Bridge, round two: `graph_inputs` on the layer info and a fresh one offered for a layer with
none; `set_effects` on that instance commits `SetLayerGraphInputs` and the preview patches
it; `graph_boxes` on the model, empty on a layer comp; the three graph group calls;
`collapse_forced`.

Flutter (single files): the existing graph panel tests stay green after the canvas refactor;
the node graph canvas draws Read, Input, Fx, Merge, Switch and Output boxes with their
sockets, refuses a mistyped drop visually and op-free, wires an image fork and a merge, splices
a dropped box, heals a deletion, and commits one op per gesture; the console lists items and
the four vocabularies in order; a dropped `CompDragData` makes a Read; the Node panel follows
a picked Fx node, an Input node's form and a Read node; the Timeline shows the hint; the
project row shows the type word; the chip names a node; the budget tests stay at their
numbers and the new canvas asks the engine nothing on hover; `engine_labels_test` and
`arb_test` pass.

Flutter, round two: a points socket on the comp canvas is drawn and wired; the Timeline on
a node graph draws a row per box with its lanes, a drag on a box's key commits one
`setNodeGraph`, Delete on one removes it, the Graph editor plots it, and the width sweep
gains a `Timeline (node graph)` case; the Effect controls draw the placed graph's section
and the Timeline its fold; the comp canvas saves and inserts a group; the collapse cell
dims when forced; the Retime clock face counts at the nested rate; the audio cell dims on a
retimed layer; the Input form offers a preview item; the budgets hold.

## 8. Work packages

Ordered, each with its tests, each one pull request's worth. NG2 and NG3 both stand on NG1
and may land together; NG4 stands on NG3; NG5 on NG4; NG6 on nothing but this note.

- **NG1, the model.** `comp_graph.rs` (`CompGraph`, `GraphNode`, `GraphEdge`, `GraphInput`,
  `ports_of`, `validate`, `project`, `read_layer`, `viewed_at`), the field on `Composition`
  and every literal, `SetCompGraph` and the guard, `Op::name`, the `merge`, `switch` and
  `node_graph` entries with `FxCategory::Compositing`, the Node graph effect's `derived` and
  the Inputs copy, the "in use" walks, the label fixtures, the glossary entries (§0), the
  `engine_labels.dart` entries and `app_en.arb` keys the fixtures demand. Tests 1 to 7, 12.
- **NG2, evaluation.** `DrawSource::Graph`, the lowering in the draw builder, `realise_graph`,
  the `graphs` side list on `run_ops`, the planner arm, the frame-key arm and the host-effect
  fold, the matrix rows. Tests 8 to 11.
- **NG3, the bridge.** `api/comp_graph.rs`, the calls of §4.1, the prefix shape, the preview
  field, codegen, the docs/17 section. The bridge tests.
- **NG4, the canvas.** The keyed-record refactor of the layer canvas, the node graph canvas
  and its console, the drop target, the Graph panel's third subject, the Node panel's arm and
  the Input form. Its Flutter tests.
- **NG5, the shell.** Project row and menus, the Timeline hint, the Effect controls header
  and Open action, the Viewer chip for a node, the palette entry. Its Flutter tests.
- **NG6, the docs.** The amendments the design owes: the scoping sentences in docs/03 §8.1,
  docs/06 §1.1 and §1.4, docs/07 §3.1, §12.2, §13.2 and §2.2.1, docs/08 §1.1 and §2.6, docs/15
  §12A.7, docs/17, node-graph.md §1.1 and §7, custom-shader.md §4.2, points-stream.md §1;
  the `docs/impl/README.md` row; the GUIDE line; the TODO entries; the manual's note on
  `use/nodes.mdx`. A manual page of its own follows when the feature ships.

Round two, in order; P2 and P3 stand on P1, P4 and P5 on P3, P6 on everything:

- **P1, core.** `Eval::stack` and the `_in` twins, the lifted socket skip, `input_times`,
  `time_demands`, `layer_temporal_window`, the `time_offset` entry, `Layer.graph_inputs`
  and `SetLayerGraphInputs`, `overrides_of` and `GraphView` in core, `cone_of`,
  `ExpressionContext.inputs` and `input()`, `CompGroupPreset` and its pair, the collapse
  clause, `GraphInput.preview`; the label fixtures regenerated. Tests 13 to 19.
- **P2, render and eval.** The shared schedule function and the step's carriage, the
  node-and-time lowering with its three step shapes, the planner's demanded jobs, the
  `b"shifted/"` and `precomp/` folds, `graph_comp_draw` and the folded nested name, the box
  key and `op_keys`' named sides, the key's context, the nine Precomp retime sites and the
  audio guard, the audio walk's graph arm, the preview item's lowering, the matrix rows,
  the unillustrable arms. Tests 20 to 24.
- **P3, bridge.** `graph_inputs` on the info and the reference, `InstanceHome::GraphInputs`,
  `graph_boxes` on the model, the graph group calls, `collapse_forced`, codegen, the
  docs/17 lines. The bridge tests.
- **P4, canvas and Node panel.** The no-stream word on a box's rows, Save group and the
  console block, the preview picker. Its Flutter tests.
- **P5, Timeline and Effect controls.** Box rows and their lanes, the placed graph's
  section and fold, the dimmed collapse and audio cells, the Retime clock face. Its
  Flutter tests, and every `app_en.arb` key of the round, appended after P4 has landed.
- **P6, docs and the manual.** The Built paragraphs here; docs/03 §5.2, docs/04 §11.3 to
  §11.5, docs/06, docs/08 (Time offset, and the temporal rule inside a graph), docs/09 §7,
  docs/17, the glossary, TODO, ae-import.md, points-stream.md; the manual page
  `use/node-graphs.mdx` in the manual's voice with its sidebar entry, the generated effect
  pages for Merge, Switch, Node graph and Time offset, the Compositing block of the effects
  index placed between Utility and Controls, and the pages that owe a link.

## The decision, as one entry

> ## A node graph is a composition, and joins pictures by nesting, not by mirroring
>
> **Status: proposed (2026-09-06).** The ask, from the Discord thread of 2026-09-03 and issue
> #106: branching and merging in the node graph, a composition-level graph in the Nuke manner
> that can be placed and retimed like any comp, and a graph that can be applied to a layer as
> an effect with its inputs as parameters. Design in docs/impl/node-graph-comp.md.
>
> `Composition` gains `graph: Option<CompGraph>`, serde-skipped when absent, and a comp is a
> layer stack or a node graph, never both, enforced by one guard in `Op::apply`. A graph holds
> Read, Input, Fx and Output nodes and typed wires; image wires branch and merge; Merge and
> Switch are Compositing-category registry entries that only a graph can hold and the graph
> walk realises; any effect or driver is an Fx node, so its parameters keyframe, express and
> take wires unchanged. A Read node is a synthetic layer at default placement, which is what
> lets the planner, the fetch, the key and the "in use" walk treat it as a layer. The value
> half projects onto the existing driver walk; an Input's value bakes into its target's
> parameter. The render adds one `DrawSource` and one walker, and the Node graph effect
> reaches that walker through a closure list on `run_ops`, so a stack is still one walk. The
> Timeline is not a second view of a graph: the two meet by nesting, on the advice in issue
> #106, and the layer's own graph is untouched. v1 named its boundaries, each with its lift;
> round two (2026-09-08) lifted every one: points wires, time inside a graph under one rule
> with a Time offset box, a placed graph's Inputs on the layer, a per-box cache, an
> expression reading an Input, Precomp retime end to end, a box's keys in the Timeline,
> saved groups for a graph, a forced collapse, audio through a graph's Reads, and a picture
> Input's preview item.

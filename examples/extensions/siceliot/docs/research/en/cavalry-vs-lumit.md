# Cavalry vs Lumit — the non-destructive model

*Italiano: [../it/cavalry-vs-lumit.md](../it/cavalry-vs-lumit.md)*

> **Research notes, 26 October 2026**, written before any SICELIOT code. They explain the reasoning
> behind the project; parts are now out of date (for example, Lumit 0.9.1 does support LFX, and
> many of the "missing" effects below now exist in SICELIOT). For what was actually built see
> [../../en/effects.md](../../en/effects.md) and [../../en/engine-changes.md](../../en/engine-changes.md).

Sources: the Cavalry documentation ([cavalry.studio/docs](https://cavalry.studio/docs/)) and the
Lumit documentation and source code ([github.com/luminalmvm/Lumit](https://github.com/luminalmvm/Lumit),
`main` branch on 26/10/2026). The full list of Cavalry nodes is in
[cavalry-nodes-by-role.md](cavalry-nodes-by-role.md).

---

## 1. How Cavalry works

### 1.1 The core idea: everything stays a live parameter
In Cavalry nothing is "applied" and frozen. Every shape, deformer, shader and filter is a **layer
with attributes**, and the result is recalculated every frame from the source. To change anything
you change an attribute, unplug a connection or reorder a list. There is no "bake" unless you ask
for one.

Under the layer interface sits a **node graph**: connections pass data from one layer's attribute
to another's, without expressions, so quickly.

### 1.2 The four mechanisms that make it modular

**A. Connections between attributes** ([Connections](https://cavalry.studio/docs/getting-started/key-concepts/connections/))
- Any attribute can drive any other compatible attribute (same data type), **across layers and
  inside other behaviours too**. For example a Random can drive a Stagger's `maximum`, which in
  turn drives a Duplicator's `shapeScale`.
- Every attribute has **one input and any number of outputs**. Keyframes count as an input, so a
  connection replaces the animation.
- Every layer has an `id` output. Connecting the `id` passes *the whole node*, not a number: that
  is how a deformer joins the Deformers list or a shader joins a material.

**B. Index context** ([Context](https://cavalry.studio/docs/getting-started/key-concepts/context/), [Index Context](https://cavalry.studio/docs/nodes/utilities/index-context/))
- The Duplicator asks every connected value **once per copy**, passing the copy's index. Everything
  upstream (Random, Stagger, Noise, Color Array, String Generator, even a polygon's number of sides
  or a blur's radius) is evaluated again for each copy.
- The index flows through **nested Duplicators**, and **Index Context → Depth** picks which level
  to read. `Use Index Context` turns it off.
- Other "contexts" work the same way: path length (Length Context), the copy's velocity (Velocity
  Context), a seed per connection (Layer Seed).
- **Arrays** (Value, Color, Shape, Shader, Asset, String Array) hand out one element per index, so
  one network of nodes produces copies that all differ.

**C. Ordered stacks with blend modes**: every stage is a reorderable list, and each entry chooses
how it combines with the ones below.

| Stack | Where | How they combine |
|---|---|---|
| **Falloff** | on every Behaviour / Field | Normal (Add), Min, Max, Minus, Multiply, Screen, Overlay, bottom up |
| **Behaviour Mixer** | combines value behaviours | the same modes, plus Strength and Enabled per index (does not take deformers) |
| **Deformers** | list on the shape | in order, each with its own Strength and Falloffs |
| **Shaders** | in the Fill / Stroke materials | blend mode + opacity; the material's Alpha as master |
| **Filters** | list on the shape | blend mode + opacity + **Mattes** (luma/alpha, feather, *Filter Input* or *Clip Output*) |

The **Falloff** ([doc](https://cavalry.studio/docs/nodes/utilities/falloff/)) is the "where" of
every effect: a Circle / Rectangle / Linear / Sweep / Shape (filled or edges), a graph that sets
the decay curve, a Probability to make it binary, and layer modes to combine several.

**D. Encapsulation and reuse**
- **Pre-comps** and **Pre-Comp Overrides** ([doc](https://cavalry.studio/docs/nodes/shapes/composition/pre-comp-overrides/)):
  one source composition, many instances, and on each instance only the attributes marked as
  overrides change. This is the template system.
- **Component**: gathers layers and "promotes" some attributes, making a custom node with a few
  controls.
- **JavaScript Shape / Deformer / Utility / Modifier / Emitter, Custom Distribution, SkSL Shader /
  Filter**: write a new node in code and use it like the built-in ones.

### 1.3 Effects plug into each other
This is what makes Cavalry so modular: the categories are not sealed boxes.

- **Shape → Shader**: *Shape to Shader* uses one shape as the texture of another.
- **Shader → Filter**: *Distortion* and *Distort Edges* warp pixels by sampling a shader (for
  example a Noise Shader).
- **Shader → Shader**: *Blend Shader* merges several shaders into one reusable shader.
- **Shape → Falloff → Behaviour**: any shape becomes the area a deformer or a value acts in.
- **Distribution → Distribution**: Mask, Sort and Shuffle take another distribution and filter or
  reorder it.
- **Duplicator → Duplicator**: nesting, with the index passed on.
- **Behaviour → a Behaviour's attribute**: a modifier's own parameters can be driven in turn.
- **Filter with a matte**: an animated shape limits where a filter acts, with feather.

### 1.4 Evaluation order (for one shape)
```
Generator (Basic Shape, Duplicator + Distribution, Text…)
  → Deformers (in a list, each with Strength and Falloffs)
  → Materials: Fill/Stroke with a stack of Shaders (blend + opacity)
  → Filters (in a list, blend + opacity + mattes)
  → The shape's blend mode onto the composition
Values (value Behaviours, Utilities) come in at any level through connections,
re-evaluated per copy thanks to the context.
```

---

## 2. How Lumit works

Lumit is closer to **After Effects** than to Cavalry: a **pixel**, layer-based compositor with one
effect stack per layer. It has, however, added three things that bring it towards procedural work:
**drivers**, **points streams** and **node graph compositions**.

### 2.1 The effect stack (raster)
- Every layer has an **effect stack** applied top to bottom. Each effect works on the **picture** it
  receives.
- Order per layer: source → retime → masks → **effects** → transform → blend mode.
- **Every effect has Mix, Blend (26 modes) and Matte.** The Matte is any layer, read as luminance,
  alpha or a channel, invertible, and taken raw, with its masks or with its effects. In some
  effects the matte modulates the parameter itself, not only the strength (Gaussian blur narrows
  its radius, for example).
- **Adjustment layers**: effects act on everything below.

### 2.2 The layer's Node graph
- It is the **same stack** drawn as boxes and wires: Source → effects → Layer out. An effect is
  bypassed by unplugging its wire.
- **Drivers** make values (number, colour, point) and wire into parameters: Wiggle, Math, Remap,
  Smooth, Split, Combine, Expression, Colour cycle, Audio level, Beat trigger, Measure, Points
  sample, Layer points, **Clone index**.
- **Limits:** a layer's graph has no branches or merges, and **wires do not cross layers** (drivers
  that read another layer pick it from a menu).
- **Node groups** are saved as `.lumgrp` and re-inserted from the search or a shelf.

### 2.3 Node graph compositions (the real "node-based" part)
- A composition made of boxes: **Read** (footage, solid, comp), **Input** (picture, number, angle,
  colour), **Effect** (any effect or driver), **Merge** (A over B with blend and opacity),
  **Switch** (choose by index), **Time offset**, nested **Node graph** and **Output**.
- Branches and merges are allowed, a picture can be used several times and graphs nest.
- **Reuse as an effect:** the *Node graph* effect applies a graph to any layer. Its **Inputs**
  become the effect's controls. This is the closest thing to Cavalry's *Component* and *Pre-Comp
  Overrides*.
- A graph can also be placed as a precomp layer, and its Inputs become the layer's controls.

### 2.4 Points streams (the "Cavalry-like" part)
A family of effects produces, modifies and consumes **points streams**, joined by a wire type of
their own:

- **Producers:** Grid (Rows, Offset rows, Rings, Spiral, Hexagons, Point), Scatter, Points along
  path, Text to points, Emit from image, Track points, Particulate.
- **Modifiers:** Vary points, Pick points, Transform points, Relax points, Flow points, Merge
  points, Bake points.
- **Consumers:** Clone to points, Connect points, Trail, Label points, Points field.
- **Per-point attributes ("Names"):** each point can carry up to 64 named values, plus the built-in
  `@index`, `@indexnorm`, `@n`, `@id`, `@x/@y/@z`, `@size`, `@age`, `@rand01`, `@picked`. This is
  Cavalry's index/context, but *per point* rather than per copy of any node.
- **Vary points:** a **Pattern** (Index, Random, Noise, Distance, Age, Across, Down, Image, Stripes,
  Checker, Waves, Number…) through a **Curve**, driving size, opacity, rotation, offset and colour.
  Cavalry's Stagger and Falloff in one.
- **Clone to points:** stamps up to 4 layers on the points, choosing the layer in order, at random
  or by number, with a time offset per copy. With a **Clone index** driver inside the cloned layer
  each copy renders differently, up to **32 unique renders**.

### 2.5 Other tools
- **Shape layers:** vector groups with fill and stroke, Linear/Radial gradients with up to 16 stops,
  **Combine** (Union, Subtract, Intersect, Exclude) and operations in a **fixed order**: smooth →
  twist → deform (noise) → offset → trim → repeater (at most 100 copies). Then stroke taper and
  dashes.
- **Masks** with modes (Add, Subtract, Intersect, Difference, Lighten, Darken) and **mattes** from
  any layer.
- **Expressions** in JavaScript (After Effects-compatible) or Rhai on properties and parameters;
  **Custom controls** to expose values.
- **Presets** `.lumfx` (effect stacks with keyframes) and **scripts** `.jsx` / `.js`.
- **Custom shader** (WGSL by hand or built from a graph of boxes) and **LFX extensions**.

---

## 3. Concept by concept

| Cavalry concept | In Lumit | Status |
|---|---|---|
| Connect any attribute to any other | Driver → parameter (same layer); expressions (across layers too) | 🟡 partial: wires only inside a layer, across layers only by expression or picker |
| A behaviour driving another behaviour's parameters | Driver → driver (e.g. Wiggle → Remap → parameter) | ✅ |
| Index context (per copy) | `@index` and Names on points; Clone index in the cloned layer | 🟡 only inside the points family; 32 unique renders |
| Nested Duplicators with Depth | — | ❌ not documented |
| Arrays by index (colours, shapes, assets) | Clone to points with 4 layers + *Choose by*; Vary points' colour ramp; arrays in expressions | 🟡 |
| Falloff (shape, curve, layer modes) | Matte on every effect; Pattern + Curve in Vary points; Pick points | 🟡 the matte is a picture, not a stack of falloffs with modes |
| Behaviour Mixer | A chain of Math / Expression | 🟡 by hand |
| Deformers list on the shape | Shape operations in a fixed order | ❌ no free stack of geometric deformers |
| Shader stack in materials | The fill's gradient (2 kinds) + Generate effects + Merge in the node graph | 🟡 |
| Filter stack with blend + matte | Effect stack with Mix, Blend, Matte | ✅ (even more flexible) |
| Shape to Shader / Shader → Distortion | Displacement map, Texturize, Custom shader's *Second input*; any branch in the node graph | ✅ in the node graph |
| Blend Shader | **Merge** box in the node graph | ✅ |
| Pre-Comp Overrides / Component | Node graph with **Inputs** + the *Node graph* effect; Custom controls | ✅ the concept is there, for effects and pictures only |
| Distributions | Grid / Scatter / Points along path / Text to points / Emit from image… | 🟡 missing Fibonacci, Rose, Math, Intersections, Voxelize, Sort/Shuffle, Shape edges as such |
| Duplicator | Clone to points | 🟡 |
| Particles | Particulate + Flow points | 🟡 fewer modifiers |
| Forge Dynamics (physics) | — | ❌ |
| JavaScript Shape / Deformer / Distribution | — (Expression for values, Custom shader for pixels) | ❌ no geometry from code |
| SkSL Shader / Filter | Custom shader WGSL / LFX | ✅ |
| Value Solver, Spring, Trails | Smooth driver, Echo, Trail (on points) | 🟡 |
| Spreadsheet, String Generator / Manipulator | Rhai expressions on text | 🟡 / ❌ |

### The most important missing effects
- **Shaders:** **conical, sweep and multi-point** gradients, **Voronoi**, **Checkerboard**, SLA
  noise (only Fractal noise exists).
- **Filters:** an N-stop **Gradient Map** (Tritone has 3), **Halftone**, **Dithering**, **Inner
  Shadow**, **Erosion** (choke/dilate), **Light Sweep**, **Lighting** (long shadows), **Bilateral
  blur**, **Scrape**, **Stripes**, **Slit Scan**.
- **Geometric deformers:** Bend, Lattice, Four Point Warp, Wave on a path, Bevel, Subdivide, Path
  Offset in a stack, live Boolean between layers, Morph between different shapes, Pathfinder,
  Squash & Stretch, Motion Stretch.

---

## 4. What LFX can do

### 4.1 What LFX offers (from the code, `api: 1`)
- A folder or an `.lfx` package with `extension.json` and a **WGSL** shader, optionally with an HTML
  interface page.
- Up to **8 passes** per effect, also at ½ or ¼ resolution; each pass can read the earlier ones.
- It reads: the effect's **input**, the **original layer** (before effects), **a second input**
  (another layer or a depth map), the sizes, `time` and `seed`.
- Parameters from `struct Params`: slider, bounded, angle, integer, toggle, menu (choice), seed,
  colour, point. **They are ordinary parameters**: the Custom shader's documentation, whose
  contract LFX shares, says they take keyframes and expressions like built-in ones, so they should
  also take **driver** wires in the node graph (to be checked in Lumit).
- Output: **a picture**. Plus Mix, Blend and Matte, which Lumit adds itself.

### 4.2 The trick to get Cavalry's modularity inside Lumit
Since LFX parameters take drivers, an LFX effect becomes "Cavalry-like" when combined with the
other pieces:
- **Per copy:** put the LFX effect in the cloned layer and wire **Clone index → Normalised /
  Random** into one of its parameters. Each copy of Clone to points then has a different gradient
  or colour, as with Stagger + Duplicator.
- **Falloff:** an LFX that **draws a matte** (circle, box, linear, sweep, with a curve and
  repetitions) to use as the **Matte** of any other effect. Cavalry's Falloff, in pixels.
- **Blend Shader / Shape to Shader:** combine several LFX boxes with **Merge** in the node graph,
  then save the group (`.lumngrp`) or apply it all as a *Node graph* effect with its Inputs
  exposed.

### 4.3 Feasibility, effect by effect

| Cavalry effect | In LFX? | Notes |
|---|---|---|
| Conical / sweep / multi-point gradient | ✅ easy | 1 pass; a fixed number of stops (e.g. 4–6 colours as parameters) |
| Voronoi, Checkerboard, SLA / other noise | ✅ easy | 1 pass, animated with `time` |
| **Falloff** as a matte generator | ✅ easy | Circle, Box, Linear, Sweep, Probability; the curve approximated with eases (there is no "curve" parameter) |
| N-stop Gradient Map | ✅ | stops as colour parameters |
| Halftone, Dithering, Stripes, Scrape | ✅ | 1–2 passes |
| Inner Shadow, Erosion / Dilate | ✅ | 2–3 passes (separable blur + threshold) |
| Light Sweep, Lighting (long shadows) | ✅ / 🟡 | long shadows by marching towards the light |
| Bilateral blur | ✅ | 2 passes |
| Bend, Wave, Four Point Warp, simple Lattice | 🟡 | **only as image warps**, not geometry |
| Distortion sampling a shader | ✅ | a noise shader as *second input*, or noise inside |
| Slit Scan, Trails, Value Solver | ❌ | LFX sees no other frames (only `time`); Echo or Time offset in the node graph instead |
| Real geometric deformers, Boolean, Bevel, Subdivide | ❌ | LFX makes pixels, not paths |
| New distributions (Fibonacci, Rose, Math…) | ❌ | LFX makes no points streams |
| Value behaviours (Spring, Random, Stagger as a node) | ❌ | LFX makes no numbers; use drivers and expressions |
| Physics, procedural text, spreadsheets | ❌ | outside the LFX model |

### 4.4 What LFX cannot do can still be done
Lumit is open source (Rust), so the parts marked ❌ can be added **to the engine**:
- new **points distributions** (Fibonacci, Rose, Math): effects of the points family in Rust;
- new **drivers** (Spring, Stagger as a node, arrays by index);
- **geometric deformers** for shape layers, i.e. a free stack of operations instead of the fixed
  order.

These are contributions to Lumit's code (pull requests), not extensions.

---

## 5. Notes and open checks
- Lumit's *Effects* documentation page still says "LFX — Not built yet", while the *Writing LFX
  effects* page and the code (`crates/lumit-core/src/fx/lfx.rs`) implement it. *(Checked later:
  Lumit 0.9.1 supports LFX.)*
- Splitting Cavalry's Behaviours into "deformers" and "values" is my own classification.
- Clone to points' limit of 32 unique renders and the repeater's 100 copies are stated in Lumit's
  documentation.

# Cavalry — effects and modifiers

*Italiano: [../it/cavalry-effetti-e-modificatori.md](../it/cavalry-effetti-e-modificatori.md)*

> **Research notes, 26 October 2026.** For which of these SICELIOT now covers, see
> [../../SICELIOT.md](../../SICELIOT.md).

The full list of nodes documented on [cavalry.studio/docs](https://cavalry.studio/docs/) (sitemap
read on 26/10/2026), grouped by **role in the non-destructive chain**. Descriptions are summarised
from the official documentation; the raw list is [../cavalry-nodes.tsv](../cavalry-nodes.tsv).

> Splitting Behaviours into *Deformer* and *Value* is my own reading of what each node produces;
> the documentation lists them all together as "Behaviours".

## How they fit together (in short)

```
 Utilities (math, arrays, logic, context, falloff)
        │ numbers / colours / strings
        ▼
 "Value" behaviour ──connection──▶ any attribute (other behaviours' too)
                                         │
 Distribution ──▶ Duplicator / Connect / Points to Path …   (points → geometry)
                                         │
 Shape (generator) ──▶ Deformers list ──▶ Materials (Fill/Stroke + Shader stack) ──▶ Filter stack ──▶ the shape's blend mode
                          ▲ each with Falloffs                 ▲ blend + opacity       ▲ blend + opacity + matte
```

- **Connections**: an attribute can drive many (outputs) but has a single input; keyframes count as
  an input.
- **Context / index**: the Duplicator asks for the value *once per copy*, passing the index, so the
  same Random/Stagger/Noise gives each copy a different value; the index flows through nested
  Duplicators.
- **Ordered stacks with blend**: Falloff (Add/Min/Max/Minus/Multiply/Screen/Overlay), Behaviour
  Mixer (same modes), Shaders (blend mode + opacity), Filters (blend mode + opacity + matte).
- **Fixed order**: Shaders apply to the materials *before* Filters.

---

## 1. Shapes — geometry generators

### Primitives
| Node | What it does |
|---|---|
| Basic Shape | A primitive: Rectangle, Ellipse, Polygon, Star, Arc, Arrow, Capsule, Cogwheel, Ring, Super Ellipse, Super Shape |
| Basic Line | A line: Line, Bézier, Spiral |
| Background Shape | A shape that fits the composition's resolution |
| Editable Shape | A shape with hand-editable points |
| Text Shape | Text |
| SVG | Shapes from .svg files |
| Footage Shape | A shape the size of its footage |
| Cel Animation Shape | Frame-by-frame animation |
| Spacer | An empty mesh for layouts |

### Procedural (take other shapes or points as input)
| Node | What it does |
|---|---|
| **Duplicator** | Duplicates shapes on a Distribution's points |
| Custom Shape | An instance inheriting another shape |
| Connect Shape | Joins distribution points with lines |
| Points to Path | Makes a path from a distribution's points |
| Outline | A geometric outline from another shape's path |
| Extrude | Extrusion for depth |
| Merge | Joins two open paths into a closed one |
| Convex Hull | A shape enclosing other shapes |
| Metaballs Shape | Merges shapes into an organic blob |
| Trails | Trails from other shapes' movement |
| Ray | An infinite ray that stops at collisions |
| Shortest Path | The shortest path through a distribution's points |
| Quad Tree Shape | Subdivides a square by a distribution |
| Segment Path | Splits a path into segments (sub-meshes) |
| Extract Sub-Meshes | Extracts given sub-meshes from another shape |
| Sub-Mesh Bounding Box | Rectangles from sub-meshes' bounding boxes |
| Image to Shapes | Shapes by sampling an image |
| Isolines Shape | Contours (isolines) from images or shapes |
| Mesh Shape | A mesh of vertices to warp images or shaders |
| Corner Pin | Fake 4-corner perspective |
| Rectangle Pattern | Bar and pie charts |
| Particle Shape | A particle system (smoke, rain, dust) |
| Forge Dynamics Shape | Native physics (with Fields, Constraints, Collision Events) |
| JavaScript Shape | A shape written in JavaScript |

### Containers and composition
| Node | What it does |
|---|---|
| Group | Groups layers under one transform |
| Component | A container with "promoted" attributes, like a preset |
| Composition | The stage; usable as a pre-comp |
| Pre-Comp Overrides | Overrides attributes inside a pre-comp (templates) |
| Layout Group / Grid Layout Group / Grid Layout Row | Stacks shapes in rows, columns and grids |
| Layout Shape | (Deprecated) the old layout |

---

## 2. Distributions — sources of points

They feed the Duplicator and other point-based nodes, and can be re-distributed.

| Node | What it does |
|---|---|
| Point | Every point at 0,0 |
| Linear | In a horizontal or vertical line |
| Grid | A grid |
| Circle | A circle |
| Fibonacci | A Fibonacci spiral |
| Rose | A rose curve |
| Random | Random positions |
| Path | Along one or more paths |
| Shape Edges | On another shape's edges |
| Shape Points | On another shape's vertices |
| Sub-Mesh | On sub-meshes' positions |
| Transform | On a layer's position |
| Intersections | Where shapes intersect |
| Mask | Removes points outside a shape |
| Voxelize | By voxelising another shape |
| Particle | On a Particle Shape's particles |
| Array | One position per index |
| Math | Mathematical expressions |
| Custom | JavaScript |
| Shuffle | Shuffles the ids |
| Sort | Reorders another distribution's ids |

---

## 3. Behaviours — DEFORMERS (change geometry, go in the *Deformers* list)

### Deformation
| Node | What it does |
|---|---|
| Noise | Deforms with noise (or makes values) |
| Oscillator | Trigonometric waves (deforms or makes values) |
| Wave | Looping waves along open and closed paths |
| Bend Deformer | Bends round a circle |
| Lattice Deformer | A cage of control points |
| Four Point Warp | A 4-corner warp with bézier handles |
| Pinch | Pinches/pulls inside a falloff by moving a Null |
| Push Along Vector | Pushes along a vector |
| Flare | Tapers along an axis |
| Skew | Skew |
| Squash and Stretch | Squash and stretch (bulge, area kept) |
| Motion Stretch | Smear from movement |
| 3D Matrix | Fake 3D transforms on shapes and sub-meshes |
| Manipulator | Applies transforms to shapes |
| Look At | Turns towards a target |
| Align | Live pivots on the bounding box |
| Pathfinder | Moves or deforms along a path |
| Morph | Morphs between shapes with different point counts |
| Blend Shape | Blends between shapes with the same point count |
| Blend Sub-Mesh Positions | Blends between two shapes' sub-mesh positions |
| JavaScript Deformer | A deformer written in JavaScript |
| Mesh Solver | Chains deformers iteratively (differential growth, drawing machines) |

### Path operations (topology)
| Node | What it does |
|---|---|
| Boolean | Procedural union/subtraction |
| Path Offset | Grows/shrinks the path |
| Bevel | Rounded or chamfered corners |
| Subdivide | Subdivides and smooths |
| Add Divisions | Adds points for smoother deformation |
| Resample Path | Simplifies / removes overlaps |
| Clean Up | Removes needless points |
| Curves to Lines | Béziers → segments |
| Path Average | Smooths by averaging points |
| Path Relax | Pushes points apart |
| Reverse Path | Reverses direction |
| Travel Deformer | Moves contours' start points |
| Extend Open Paths | Extends open paths' ends |
| Chop Path | Cuts into slices |
| Knot | Gaps where a contour crosses itself |
| Fill Rule | How holes and self-crossings affect the fill |
| Stitches | Fills with a stitch effect |
| Voxelize | Converts to voxels |
| Flatten Shape Layers | Flattens the inner hierarchy into one path |
| Auto-Crop | Crops to the bounding box |

### Sub-meshes (work on the "pieces" of Duplicators, text, SVG…)
| Node | What it does |
|---|---|
| Sub-Mesh | Reaches sub-meshes for transforms, materials, etc. |
| Contours to Sub-Meshes | Contours → sub-meshes |
| Apply Distribution | Puts sub-meshes on a distribution |
| Apply Layout (Grid / Horizontal / Vertical) | Stacks sub-meshes |
| Visibility Sequence | Sub-meshes' visibility in sequence |
| Color Material Override | Overrides sub-meshes' colours |
| HSV Material Override | Overrides sub-meshes' H/S/V |
| Alpha Material Override | Overrides sub-meshes' alpha |
| Swap Color Override | Finds and swaps given colours |

### Automatic animation
| Node | What it does |
|---|---|
| Auto-Animate | Transform/opacity animation from a few keyframes |
| Spring | Springy secondary motion on position keyframes |
| Rubber Hose Limb | 2-bone IK for limbs |

---

## 4. Behaviours — VALUES (make numbers or colours to wire into any attribute)

| Node | What it does |
|---|---|
| **Stagger** | Sequential values between min and max, by index (with a graph) |
| Random | Random numbers with bias |
| Noise / Oscillator / Sound | Also as sources of values |
| Value / Value2 / Value3 | Store and manipulate 1D/2D/3D values |
| Value Blend / Value2 Blend / Value3 Blend | Interpolate (lerp) between two values |
| Position Blend | Blends between two positions |
| Color Blend | Blends between two colours through a gradient |
| Number Range | Remaps one range into another |
| Number Range to Color | Remaps values onto a colour gradient |
| Area Range | Reads values as an area and remaps them to dimensions |
| Round | Rounds / to multiples |
| Modulate | Repeating sequences (modulo) |
| Frame | Uses time to drive attributes |
| Distance | Drives an attribute by the distance from another layer |
| Get Vector | The vector between two points |
| Is Within | True when one shape is inside another |
| Material Sampler | Samples a shape's colour |
| Value Solver / Value2 Solver | Accumulate or fade values over time (memory of earlier frames) |
| **Behaviour Mixer** | Combines several value behaviours in a stack with blend modes (no deformers) |

---

## 5. Shaders — procedural fills (a stack in the Fill/Stroke materials)

| Node | What it does |
|---|---|
| Color Shader | A single colour |
| Gradient Shader | Gradients; the kind is an inner generator (Linear, Radial, Conical and others) |
| Multi-Point Gradient Shader | A gradient of several points with influence |
| Noise Shader | Perlin, Value or Cellular noise |
| SLA Shader | "New" noises |
| Voronoi Shader | A cellular pattern |
| Checkerboard Shader | A checkerboard |
| Image Shader | Images, sequences, video |
| **Shape to Shader** | Uses a shape as another shape's shader |
| **Blend Shader** | Combines several shaders into one |
| SkSL Shader | A shader written in SkSL (code) |

Common attributes: **Blend Mode** (how it combines with the shaders below in the stack) and
**Opacity**.

---

## 6. Filters — raster effects (a stack on the shape)

### Blurs
Gaussian Blur · Fast Blur · Box Blur · Bilateral Blur · Directional Blur · Zoom Blur · Luminance
Blur (blurs by another layer's luminance) · Background Blur (blurs what is inside another shape)

### Colour and tone
Brightness and Contrast · Levels · Curves · Gamma Correction · HSV Adjustment · Black and White ·
Invert · Threshold · Posterize · TriTone · **Gradient Map** · Fill Color · Shift Channels ·
Dithering · Chroma Key

### Distortion
**Distortion** (warps by sampling a *shader*) · **Distort Edges** (edges warped by a shader) ·
Ripple · Twist · Bulge · Spherise · Polar Coordinates · Mirror · Scrape · Stripes · Slit Scan ·
Pixelate · Pixel Sorting · RGB Split · Chromatic Aberration

### Style and light
Glow · Drop Shadow · Inner Shadow · Lighting (point lights, long shadows) · Light Sweep · Halftone
· Grain · Scan Lines · Vignette · Edge Detection · Erosion · Sharpen

### Transitions
Linear Wipe · Radial Wipe · Venetian Blinds

### Code
SkSL Filter

Common attributes: **Blend Mode**, **Opacity**, **Mattes** (a list of shapes limiting the filter,
each with luminance/alpha mode, blend, opacity, feather, and Apply Mode *Filter Input* or *Clip
Output*).

Motion Blur is a general setting, not a filter in the list.

---

## 7. Utilities — building blocks for logic

### Controlling effects
| Node | What it does |
|---|---|
| **Falloff** | Attenuates behaviours and deformers in space (a stack with blend modes) |
| Range Falloff | Attenuates by a range of values |
| **Index Context** | Hands the index to other layers |
| Length Context | Path length as a per-copy value |
| Velocity Context / Velocity Magnitude Context | Each copy's direction/speed in the Duplicator |
| Layer Seed | Unique random seeds per connection |
| Accumulator | Stacks shapes or builds layouts inside a Duplicator |

### Maths and logic
Math · Math2 · Math3 · Jsmath · Comparison · Logic · If Else · Sequence (non-repeating sequences) ·
Seconds to Frames · Local Time · Timeline Counter (accumulates or triggers values with markers) ·
Animation Control (remaps an animation curve as a percentage)

### Arrays (assignment by index)
Value Array · Value2 Array · Value3 Array · Color Array · Shader Array · Shape Array · Asset Array
· String Array · Typeface Array · Array Manipulator (experimental)

### Colour
HSV Color · Color Info · Contrasting Color · Index to Color

### Measures and constraints
Measure · Measure Text · Bounding Box · Path Length · Radius · Count Sub-Meshes · Get Sub-Mesh
Transform · Get Name · Transform Constraint · Bounding Box Constraint · Composition Constraint ·
Component Constraint

### Shared materials
Fill · Stroke · Stroke Duplicator (several strokes combined)

### Sampling and data
Image Sampler · Spreadsheet · Spreadsheet Lookup · Asset from Smart Folder · String From Asset

### Controls and rigs
Null · Rig Control · Custom Dropdown · Camera · Camera Guide · Lattice Controller · Scheduling
Group (places child layers in time)

### JavaScript
JavaScript Utility · JavaScript Modifier · JavaScript Emitter · (plus JavaScript Shape, JavaScript
Deformer and Custom Distribution in the other categories)

### Particles — emitters and modifiers
Particle Emitter · Distribution Emitter · Force Modifier · Turbulence Modifier · Flow Field Modifier
· Vortex Modifier · Magnetic Modifier · Goal Modifier · Path Modifier · Speed Modifier · Data
Modifier · Visual Modifier · Image Modifier

### Forge Dynamics — fields, constraints, events
Attractor Field · Direction Field · Drag Field · Vortex Field · Buoyancy Field · Path Field ·
Distance Constraint · Pin Constraint · Bridge Constraint · Color / Visibility / Impulse / Sticky /
Body Settings Collision Event

### Text
String · String Generator (Value, Random Number, Random Date, Formatted String, Formatted
Date/Time, Hash, Hexadecimal, Timecode, Block) · String Manipulator (Change Case, Join, Regex,
Replace, Resize, Shuffle, Sub-String, Transition, Unicode Offset) · String Length · Typeface ·
Apply Typeface / Font Size / Font Style / OpenType / Character Spacing / Text Material

---

## Useful references in the docs
- Key concepts: [Connections](https://cavalry.studio/docs/getting-started/key-concepts/connections/), [Context](https://cavalry.studio/docs/getting-started/key-concepts/context/), [Layers](https://cavalry.studio/docs/getting-started/key-concepts/layers/)
- Stacks and blending: [Common Attributes (Behaviours)](https://cavalry.studio/docs/nodes/behaviours/common-attributes-behaviours/) (falloff), [Behaviour Mixer](https://cavalry.studio/docs/nodes/behaviours/behaviour-mixer/), [Common Attributes (Shaders)](https://cavalry.studio/docs/nodes/effects/shaders/common-attributes-shaders/), [Common Attributes (Filters)](https://cavalry.studio/docs/nodes/effects/filters/common-attributes-filters/), [Blend Modes](https://cavalry.studio/docs/nodes/general/blend-modes/)

# 06 · Render pipeline

**Status: canonical.** This document specifies how Lumit turns a project into pixels:
evaluation semantics, compositing, colour, caching, preview, export, and scopes. The
process/thread architecture that hosts all of this is [05-ARCHITECTURE.md](05-ARCHITECTURE.md);
the budgets it must meet are [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md). Terminology is
[01-GLOSSARY.md](01-GLOSSARY.md), binding. Key words MUST, SHOULD, MAY follow RFC 2119.

Decisions implemented here: K-014, K-015, K-016, K-017, K-024, K-026.

---

## 1. Evaluation model

### 1.1 Layers in front, DAG underneath (K-015)

The document model (layers, keyframes, clips — [03-DATA-MODEL.md](03-DATA-MODEL.md)) is never
evaluated directly. A compiler lowers each comp into an immutable **evaluation graph**: a DAG of
typed nodes (source, retime, mask, effect, transform, blend, matte-apply, comp-output).
Recompilation is incremental per comp, runs on every edit, and publishes a new immutable graph
snapshot; renders in flight keep the old snapshot. Users never see this compiled graph — the
Graph panel (K-471) draws the document's own stack and wiring, never these nodes.

Evaluation is demand-driven pull in two strictly separated phases:

1. **Metadata pass** — cheap, synchronous, main-thread-adjacent. Establishes, per node: output
   format, defined region (DoD), duration, temporal dependencies, and identity status (a node
   MUST be able to declare itself a pass-through for its current parameters, e.g. opacity 1.0,
   blur radius 0, disabled effect — the compiler constant-folds these away).
2. **Pixel pass** — expensive, worker/GPU threads, cancellable at node boundaries and between
   macro-tiles (epoch tokens, [05-ARCHITECTURE.md](05-ARCHITECTURE.md)).

A request is the tuple `(node, local time, quality, roi)`. Quality bundles preview resolution
tier, bit depth, and draft flags. Local time, not comp time: retimed and nested content
evaluates at whatever time the retime/nesting maths resolves to.

Identical subgraphs (same footage, same leading effects) compile to a single shared node by
content-hash deduplication; two layers sharing a source and grade evaluate it once.

**Occlusion (K-423).** A layer that provably paints every pixel of the frame hides the layers
beneath it, and those are not decoded, uploaded, effected or composited. The draw builder and
the decode planner ask one predicate (`lumit_core::occlusion::occluder_index`) so they skip
exactly the same layers, and the cull must be invisible in the picture, so the predicate is
deliberately narrow. v1 accepts only a Solid layer whose colour has alpha 1, visible and in
span (soloed if anything is), 2D with zero rotation, Normal blend at 100% opacity, with no
masks, paint, enabled effects or motion blur, whose axis-aligned placement — its own transform
and its parent chain, none rotated, 3D, or driven by an expression — covers the comp rectangle.
It refuses whenever the comp has an active camera, a visible Adjustment layer sits above the
candidate, or any visible layer above it names a layer below as a matte or an effect's layer
input; and it is off inside a collapsed Precomp's splice (§1.4), whose layers are not clipped
to their own comp. Footage that the probe reports as alpha-free is a later extension: the
predicate lives in `lumit-core`, which knows no probe. The frame key keeps hashing culled
layers (over-keying is safe); preview and export stay byte-identical (K-031).

### 1.2 Render order for one layer

For a visual layer at comp time `t`, the compiled subgraph is, in order:

1. **Source** — fetch or rasterise the layer source at the resolved source time. For footage:
   decode, colour-interpret, linearise, premultiply (§3). For text/shape/solid: rasterise
   vectors at the working raster size. A **shape layer** (K-237,
   [03-DATA-MODEL.md](03-DATA-MODEL.md) §7.2) has no asset at all: its contents are rasterised
   into their own bounding box, which is also the layer's natural size — the one kind whose size
   moves when it is edited. Each item is filled through the mask rasteriser and then outlined
   through the paint rasteriser, in list order.
2. **Retime** — for a Footage layer, the retime map converts layer time to source time and the
   layer's frame-interpolation policy (nearest / blend / flow) synthesises non-integer source
   frames ([04-RETIMING.md](04-RETIMING.md)). Overrun holds the boundary frame. Retime affects
   only source fetch; keyframes on masks, effects, and transform remain in layer/comp time.
2.5. **Paint** — the layer's paint strokes (K-227, [03-DATA-MODEL.md](03-DATA-MODEL.md) §7.1)
   are stamped into its raster in the order they were made: brush strokes lay colour down,
   eraser strokes take alpha away, clone strokes copy from the raster **as it was before any
   stroke in the pass** was stamped. Paint happens before masks, so a mask gates the painted
   picture and effects see it. A layer with paint on it is rasterised at its real size (a flat
   solid is otherwise an 8×8 tile), and paint on a Precomp layer forces the nested intermediate
   exactly as a mask does. Stamping is on the CPU today; a GPU path changes nothing above it.
3. **Masks** — bezier paths combined top-to-bottom by mode (none, add, subtract, intersect,
   difference; lighten and darken are not built yet), each with feather, expansion, opacity,
   inversion — applied to a mask in that order, before it folds into the stack
   ([03-DATA-MODEL.md](03-DATA-MODEL.md) §7). Masks gate the layer's alpha before any effect
   runs, so effects see the masked image.
4. **Effect stack** — top-to-bottom ([08-EFFECTS.md](08-EFFECTS.md)). Each effect sees the
   output of the one above it, in working space, premultiplied unless it declares otherwise
   (§3.4).
4.5. **Lighting** — if the layer's **Accepts lights** switch is on and the composition holds
   Light layers, the layer's picture is shaded by them (§1.8). After effects, because a light
   should fall on the finished layer rather than on an intermediate; before the transform,
   because the shading is computed on the layer's own plane.
5. **Transform** — anchor point, position, scale, rotation, opacity as one 4×4 matrix (K-023),
   concatenated with the parent chain. Filtering is bilinear (draft) or bicubic (full quality),
   always on premultiplied pixels.
6. **Motion blur** — shutter-window multi-sampling wraps steps 1–5 where enabled (§4).
7. **Composite** — blend mode, matte, and opacity combine the layer's output `over` the
   accumulated composite of the layers below (§3.3, §2.4).

Comp evaluation runs bottom layer first; each layer composites onto the result. 3D layer sets,
cameras, and lights follow [03-DATA-MODEL.md](03-DATA-MODEL.md) (contiguous 3D runs are
z-sorted and rendered through the active camera; a 2D layer breaks the run).

### 1.3 Sequence layer evaluation

A Sequence layer resolves in two stages, and layer-level treatment always follows clip
resolution:

1. **Clip resolution.** Comp time → layer time → the single active clip (clips never overlap;
   a gap between clips is transparent). Layer time → clip time → the clip's Retime → source
   time. The clip's source is fetched and its frame-interpolation policy applied. The result is
   the Sequence layer's raw output for that frame: one image, as if the layer were footage.
2. **Layer treatment.** Masks, the effect stack, transform, motion blur, blend mode, and matte
   then apply to that output exactly as §1.2 steps 3–7. Effects on a Sequence layer therefore
   span edit points seamlessly — a glow does not pop at a cut.

Per-clip state is limited to source, trim, Retime, and frame-interpolation policy. Anything
needing per-clip effects is expressed by precomposing the clip's source.

### 1.4 Precomp layers, nesting, and collapse

Default nesting: the nested comp renders to an intermediate at its own raster size (scaled by
the active quality), clipped to its own bounds, then behaves as footage in the parent —
masked, effected, transformed like any raster layer. The nested comp is sampled at the parent's
frame times; its own frame rate governs only its internal keyframe display.

That intermediate is **transparent** where the nested comp's own layers do not cover it
(K-241). A comp's background colour is the backdrop for looking at *that* comp, not a layer
inside it, so it is painted only by the comp being viewed or exported — a Precomp layer carries
the alpha its content has, and the parent's stack shows through the gaps.

**Collapse** (the collapse switch on a Precomp layer) removes the intermediate:

- Inner layers' transforms concatenate with the Precomp layer's transform into single matrices;
  content is resampled once, never twice.
- No clipping at the nested comp's bounds: inner content outside them becomes visible in the
  parent. DoD propagation (§2.2) carries the true extents through.
- Inner layers' blend modes composite directly against the parent's stack, in stack order at
  the Precomp layer's position.
- 3D passes through: inner 3D layers join the parent's 3D set and are viewed through the
  parent's camera.

**What forces an intermediate anyway** (collapse remains set but Lumit renders the nested comp
to a buffer at that point, at concatenated-transform resolution where possible): any effect on
the Precomp layer; any mask on it; a blend mode other than Normal or opacity below 100% on the
Precomp layer itself; the Precomp layer being consumed as a matte; preserve-underlying-
transparency; an inner layer consuming a matte (splicing a comp-space matte across comps is a
later refinement); a live adjustment layer inside the nested comp (K-091 — its stack applies
within its own comp, which splicing cannot honour; After Effects instead lets it bleed into
the parent's stack, and Lumit deliberately does not). The Viewer MUST indicate when a
collapsed layer has been forced to an intermediate (a dimmed collapse switch). Text and shape layers behave as permanently collapsed
vector sources: rasterisation happens after the full transform chain every frame.

### 1.5 Adjustment layers

A layer with the adjustment switch renders no content of its own. Its effect stack is applied
to the composite of everything below it in the same comp. Its masks and opacity build a
coverage map: the effected composite is mixed back over the uneffected composite by that
coverage. Its transform moves the coverage map, not the picture. The adjustment node's input
ROI is the effect stack's expanded ROI intersected with the coverage DoD — an adjustment layer
masked to a small region costs a small region.

### 1.6 Mattes

Any layer may name any other layer in the comp as its **matte** (dropdown/pick-whip, matching
the AE 2023 model). Four combinations: alpha or luma, normal or inverted.

- The matte layer is evaluated through its own full pipeline (§1.2 steps 1–6) — its rendered,
  transformed comp-space output is the matte signal. It is one node in the graph: when one
  matte serves many consumers it is evaluated once per `(time, quality)` and shared by hash;
  no per-consumer re-render.
- Matte application happens at the consumer's composite step: the consumer's post-transform
  premultiplied image is multiplied by the matte's coverage (alpha channel, or luma per §3.5)
  before blending.
- A matte layer keeps its own visibility switch; being a matte does not disable it. A layer MAY
  matte a layer that is itself matted; cycles are rejected at compile time.
- A **Precomp** matte source has no pixels of its own: its nested comp is rendered (the same
  recursion §1.4 performs for a Precomp layer's picture, under the same cycle guard) and that
  render is the matte signal (K-268). The matte **source mode** (§none/masks/effects, K-142)
  does not apply to a comp reference — a comp already carries its own layers' masks and
  effects. Footage inside such a comp decodes with the rest of the frame: the decode plan
  follows matte and layer-input references whether or not the referenced layer is visible.

### 1.7 Anti-aliasing the composite (K-274)

A layer is drawn as a rectangle placed by its transform. Where the transform turns that
rectangle off-axis its edge crosses pixels diagonally, and a pixel is either drawn or not —
so the edge is a staircase, and on a slow rotation the steps crawl. **Multisampling** fixes
it: the card keeps N coverage samples per pixel, shades once, and averages by how many samples
the shape actually covered.

- **The count is a project property** (`Document::anti_aliasing`,
  [03-DATA-MODEL.md](03-DATA-MODEL.md) §2), default 4, and **one value serves preview and
  export**. Both drive the same realise walk with the same count, which is what keeps the
  K-031 identity true with anti-aliasing on.
- **It is orthogonal to preview resolution.** A reduced-resolution preview is a smaller
  picture with the same edge treatment; the count does not change with the scale.
- **The composite target is multisampled, the working texture is not.** One multisample
  colour texture lives beside the single-sample comp frame for the whole composite; every
  pass attaches the former and resolves into the latter. Every reader downstream — the
  snapshot copy for shader-computed blends, read-backs, the Scopes trace, the shared-texture
  hand-off and the display blit — takes the resolved texture, because a multisample texture
  cannot be sampled or copied to a buffer.
- **Per-layer motion blur takes the same count**, because its sub-frame placements are the
  same geometry the composite draws; an aliased smear under an anti-aliased composite would
  show the seam on every blurring layer.
- **The count is asked of the adapter, never assumed.** A card that will not multisample the
  working format at the count asked for falls back to the highest it will, down to 1, and the
  interface reports which is in use. That is a machine's limit, never a render error.
- **It is part of a frame's content hash** (§5.2), so a frame banked at one count is never
  served at another.

What multisampling does *not* fix is worth stating: the inside of a layer's picture is a
texture lookup and its quality is the sampler's business. A shape's own curves, a mask's edge
and a glyph's outline are already anti-aliased where they are rasterised. What stair-steps is
the layer's quad edge, and that is what this addresses.

The *how* — the traps in the composite loop as it stands, and the test plan — is
[impl/anti-aliasing.md](impl/anti-aliasing.md).

### 1.8 Lighting (K-361)

A composition's **Light layers** ([03-DATA-MODEL.md](03-DATA-MODEL.md) §5.5, K-360) shade
every layer whose **Accepts lights** switch is on. The pass is not an effect — it has no
entry in [08-EFFECTS.md](08-EFFECTS.md), no `Resolved` variant and no place in a stack,
because it is not something added to a layer but something the composition does to it.
`lumit_core::lighting` is the reference implementation and `fx_lighting.wgsl` its twin,
compared by test as every kernel is (docs/08 §1.6).

**The surface.** Every pixel of a layer shares one normal: the direction its own plane faces.
A 2.5D compositor has no per-pixel normals, and deriving them from luminance is a
content-dependent quality cliff. For a softbox raking across footage the flat-plane answer is
not an approximation of the right one — it *is* the right one. A layer without the 3D switch
is shaded at z = 0 with no out-of-plane rotation, matching where the compositor actually
draws it rather than what its transform happens to hold.

**Area lights.** How brightly a flat surface is lit by a flat glowing rectangle is exact and
closed-form: the cosine-weighted fraction of the surface's sky that the rectangle covers,
summed one term per edge. No sampling, no noise, four `acos` calls. The rectangle is clipped
to the surface's horizon first — the half of a light that has sunk behind a surface cannot
light it — which is why the sum runs over a polygon of up to five corners rather than a flat
four. This integral is the identity-matrix case of Linearly Transformed Cosines; K-361
records why the fitted matrix tables that would buy specular are deliberately not here.

**Point and spot lights** take the cosine law and nothing else, attenuated by the light's
`falloff_px` and, for a spot, softened across the outer tenth of its cone. There is no inverse
square: measured in comp pixels it is a number with no meaning, and `falloff_px` already says
where the light stops.

**Light adds, it does not replace** — the picture is multiplied by `1 + light`, so an unlit
pixel is untouched and nothing is ever driven to black by the arrival of a light elsewhere.
A composition with no Light layers produces an empty light list, the pass never runs, and the
frame is byte-for-byte what it was before lighting existed. Eight lights shade one layer; the
nearest win, chosen by a total order so two runs agree.

## 2. ROI and DoD

### 2.1 Request propagation

Every node participates in the two-way region protocol:

- **DoD (defined region)** flows upstream→downstream in the metadata pass: the bounding box of
  pixels a node can produce. Sources report media bounds; transforms transform the box; blurs
  and glows pad it by their reach; blends union their inputs; comp output clamps to comp bounds
  (except inside collapsed precomps, where true extents propagate).
- **ROI** flows downstream→upstream in the request phase: the region the consumer actually
  needs. Every effect MUST declare its input expansion `roi_in = f(roi_out)` (blur radius,
  maximum displacement). Nodes evaluate only `ROI ∩ DoD`.

`ROI ∩ DoD` bounds both texture allocation and dispatch grids. A 200 px title in a 4K comp
allocates and computes a title-sized region plus effect padding, not 4K. Temporal dependencies
are declared in the same pass: an effect sampling other frames (echo, flow retime, temporal
blur) declares which input times it needs, and those become ordinary upstream requests.

### 2.2 Execution: full-frame per node, macro-tiles under pressure

The execution model is **full-region-per-node on the GPU**: each node's output is one texture
(sized to its `ROI ∩ DoD`), pool-allocated, lifetime managed by refcount from the compiled
graph, freed the moment its last reader completes. Simple per-pixel node runs (colour ops,
LUTs, transfer curves) are fused into single WGSL passes at compile time.

When a requested allocation exceeds the VRAM budget (resource governor,
[13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)), the scheduler splits the request into 2–4
**macro-tiles** and runs the subgraph per tile, using each effect's declared expansion for
correct overlap. Macro-tiling is a fallback, not the model; it also caps single-dispatch
duration as TDR insurance. Per-node CPU fallback (every effect ships a CPU reference
implementation, K-019) bridges via readback/upload nodes inserted by the scheduler, batching
adjacent CPU nodes to avoid ping-ponging.

## 3. Colour

### 3.1 Working space (K-026)

The working space is **scene-linear, premultiplied alpha, fp16 RGBA** per pixel by
default. All compositing, filtering, resampling, and motion-blur accumulation happen
here.

**Depth is one project-wide switch (K-069, supersedes K-026's per-comp clause).** The
project's working depth — 8 bpc integer, 16 bpc float (default), or 32 bpc float —
applies to every comp, every effect buffer, and every inter-node texture in the
project. **v1 status:** the engine currently renders **fp16 only**; the 8/32 bpc options
and the depth control below are the intended design, not yet built ([TODO.md](TODO.md)). There is no per-comp override: switching the project switches everything,
exactly like AE's project bit depth. The control lives as a small depth button at the
foot of the Project panel (AE's spot; click to cycle, dialogue for the long list
later), and Application Settings holds only the *default for newly created projects*.
Kernels MAY use wider internal accumulators where the algorithm needs them (large
iterative blurs, scopes), but everything a node reads or writes is project depth.

Why fp16 stays the default (K-069): fp16 here is floating point, not AE's integer
16bpc — it already carries values above 1.0 (superwhites, glow overshoot, up to 65504)
and negatives, in linear light. fp32 buys extra mantissa (deep shadow gradients under
extreme pushes, very long chains) at 16 bytes/px: double the bandwidth on a
bandwidth-bound compositor and half the frames per cache byte. The depth is part of
every cache key's quality field, so switching depth simply re-keys the project and the
caches rebuild.

### 3.2 Input: decode and linearise

Every footage item carries a **colour-space tag** in its interpretation settings. Defaults:
video streams are assumed Rec.709 (BT.1886 transfer), stills and screen/game captures sRGB,
unless container metadata says otherwise; the user can override per item. Game captures — the
primary v1 audience's material — therefore linearise through the sRGB/Rec.709 assumptions
without configuration.

Decode path: hardware decode lands NV12/P010 in GPU memory ([05-ARCHITECTURE.md](05-ARCHITECTURE.md));
one compute pass performs colour-matrix conversion, chroma upsampling, transfer-function
linearisation, and premultiplication straight into a working-space texture. No CPU round trip.
Alpha interpretation (straight vs premultiplied) is per footage item; straight sources are
premultiplied after linearisation.

### 3.3 Display transform and the OCIO slot

The Viewer applies a **display transform** as the final blit: working linear → the display's
space (sRGB by default; the exposure control and channel isolation are viewer-only and sit
inside this stage). Nothing upstream of this blit is display-referred.

Both the per-footage input transform and the display transform are implementations of one
internal `ColourTransform` interface (shader source + optional LUT textures). v1 ships built-in
transforms (sRGB, Rec.709/BT.1886, linear). **OCIO v2 integration is post-v1 but slots here**:
an OCIO-backed `ColourTransform` generated from a config via OCIO's GPU shader API, transpiled
to WGSL. Nothing else in the pipeline may assume the transform set is fixed.

**Perceptual operations (K-034).** Linear RGB is correct for combining *light*; it is the
wrong space for combining *colours as perceived*: a linear (or worse, gamma-space) lerp
between saturated colours passes through muddy grey, and rotating hue in RGB changes
brightness. Operations whose meaning is perceptual — gradient interpolation, keyframed
colour properties, hue rotation, saturation — MUST convert linear RGB → Oklab (or its
polar form OkLCh), operate, and convert back. The conversion pair lives in one module
(`lumit-gpu::oklab`, CPU + WGSL with byte-identical constants) and costs two 3×3 matrix
multiplies and three cube roots per direction — cheap enough to inline per pixel in effect
kernels. Hue rotation in OkLCh preserves the L axis by construction; the tests assert it.
Compositing, blend modes' linear subset, and everything in §render-order stay in linear
RGB — K-034 changes where *interpolation* happens, never where light is added.

**The parity guarantee (K-031).** Preview and export MUST share one colour code path: the
same input transforms, working space, and output transform implementations, in the same
precision. At Full resolution and full quality, the frame presented in the Viewer is
bit-identical to the frame handed to the encoder; export-only stages (encoder subsampling,
8/10-bit quantisation, container tagging) sit strictly downstream of that point. There is
no "render colour engine" distinct from the preview's — having two is how other tools end
up with previews that lie. CI enforces parity with a golden test comparing Viewer readback
against export output for a reference comp in every shipped colour configuration.

### 3.4 Premultiplication rules

Premultiplied everywhere, with exactly these boundaries:

- **Decode/rasterise** → premultiply immediately after linearisation. Vector rasterisation
  (text, shape, masks) produces premultiplied coverage directly.
- **Effects** receive premultiplied input by default. An effect MAY declare
  `wants_straight_alpha` (colour-correction ops that must not tint transparent regions); the
  host unpremultiplies before it and re-premultiplies after, fused into adjacent passes where
  possible. Effect authors never hand-roll this.
- **Transforms and all filtering** operate on premultiplied pixels, always.
- **Blend modes** that need straight colour (the perceptual set, §3.5) unpremultiply
  transiently inside the blend pass.
- **Export** re-encodes to straight or premultiplied per the output settings (§7); the display
  blit outputs opaque display-referred pixels.

### 3.5 Blend modes

v1 blend-mode list, grouped by the domain the maths runs in. "Linear" = scene-linear working
space; "perceptual" = the blend runs on sRGB-encoded (display-referred) values — operands are
unpremultiplied, encoded, blended, decoded, re-premultiplied, fused in one pass. The perceptual
set exists because those formulas were designed on gamma-encoded 8-bit pixels and editors
expect that look; running them in linear is mathematically tidy and visually wrong to the
target audience. Out-of-range values pass through the extended (unclamped) transfer function.

The full After Effects colour-blend set ships in v1 (K-162, T24): every mode below is
implemented, verified against a Rust reference of its formula
(`composite::tests::perceptual_blend_modes_match_the_reference_formula`). The layer dropdown and
the effect Mode param both list them from one source (`BlendMode::ALL`), in AE's grouped order.

| Mode | Domain | Notes |
|---|---|---|
| Normal | linear | Premultiplied `over`: `A + B·(1−a_A)`. |
| Add | linear | Physically additive; the montage staple for glows/flashes. |
| Subtract | linear | `dst − src` per channel, clamped at black — Add's darkening twin (GEN-1, K-151). |
| Multiply | linear | Physical filter/shadow behaviour (fixed-function `Dst` blend). |
| Darken, Lighten | either (invariant) | Per-channel min/max; monotonic transfer makes the domain irrelevant. Computed in linear. |
| Screen | perceptual | |
| Overlay, Soft light, Hard light | perceptual | |
| Linear light, Vivid light, Pin light, Hard mix | perceptual | Contrast group; Hard mix thresholds Vivid light at 0.5. |
| Colour dodge, Colour burn, Linear burn | perceptual | |
| Darker colour, Lighter colour | perceptual (non-separable) | Whole-pixel min/max by perceptual luma. |
| Difference, Exclusion, Divide | perceptual | |
| Hue, Saturation, Colour, Luminosity | perceptual | HSL decomposition on encoded values (W3C non-separable). |

**(a) Luma extraction** — everywhere luma is needed (luma mattes, stencil/silhouette luma):
luma = Rec.709 Y of the sRGB-encoded signal (perceptual luma), so a 50% grey solid yields
approximately 50% coverage, matching editor expectation. This is a single normative definition;
no per-feature variation.

The remaining AE modes — Dissolve / Dancing dissolve (need a dither seed), the legacy
"Classic" variants, and the alpha operators (Stencil / Silhouette / Alpha add / Luminescent
premul, which change alpha compositing rather than colour) — are post-v1. The enum is
open-ended and serialised by name ([10-FILE-FORMAT.md](10-FILE-FORMAT.md)) so adding modes never
breaks projects.

## 4. Motion blur

- **Comp-level settings**: shutter angle 0–720° (180° default; blur window =
  angle/360 × frame duration), shutter phase −360°–360° (default −90°, centring the window on
  the frame), and an adaptive sample limit (default 64, maximum 256).
- **Per-layer switch** enables blur for that layer. Adaptive degradation MAY skip motion blur
  during interaction ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)); export never skips it.
- **Transform motion blur** is multi-sampling: the layer's steps 1–5 output is sampled at N
  times across the shutter window and accumulated (fp32 accumulator) with equal weights. N is
  adaptive: `N = clamp(ceil(max screen-space displacement in px / 2), 2, adaptive limit)`,
  computed in the metadata pass from the transform curves — deterministic, so preview and
  export agree. Where only the transform animates (source static under the shutter), the
  sampled source is rendered once and only the transform re-evaluated per sample.
- **Effect-internal blur**: an effect MAY declare `motion_blurred_internally` (e.g. the
  RSMB-class flow blur, directional blurs driven by motion vectors). The host then excludes it
  from multi-sampling and passes it the shutter interval, so blur is neither doubled nor
  skipped.
- **Interaction with Retime**: shutter sample times are comp times, each mapped through the
  layer's (or clip's) Retime to a source time — shutter samples live in retimed source time.
  Consequences, all required: a freeze (speed 0) produces no source-motion blur but transform
  blur still applies; a speed ramp stretches or compresses the source-time shutter window in
  proportion to speed; fractional sample times use the clip's frame-interpolation policy.
  Overrun regions hold the boundary frame for all samples.

Status (shipped v1, K-120): the transform-multi-sampling core is live in the shape above with
these v1 trims, each a recorded follow-up rather than a reversal. N is a **fixed comp setting**
(`samples`, default 16, control range 2–64, hard cap 256 — the same maximum the adaptive rule
above will respect), not yet the adaptive displacement-derived count; the frame-time source is
rendered once and only the transform re-evaluated per sample (the source-static case above —
sub-frame *source* re-render, and therefore the Retime interaction bullet, awaits the
accumulation path); the accumulator is the working-format average (`motion_blur_average`'s
additive-on-both-channels mean) rather than a dedicated fp32 target; parent motion within the
shutter and blur on a collapsed Precomp's inner layers are deferred (K-120). Preview and export
share one sample-time derivation and one averaging helper, so K-031 holds.

## 5. Caching (K-016)

### 5.1 Three tiers

| Tier | Contents | Survives |
|---|---|---|
| **VRAM cache** | Textures of recently used node outputs and final frames | Nothing (device loss drops it; recovery is by design from lower tiers) |
| **RAM cache** | fp16 planes of node outputs and final comp frames | GPU device loss; cleared on quit |
| **Disk cache** | Final frames and expensive intermediates, persistent | Sessions; deletable at any time |

Playback reads VRAM first, promotes RAM→VRAM, and promotes disk→RAM→VRAM ahead of the playhead
(never plays directly from disk). Writes are write-behind on background IO threads; a disk
write never blocks a render.

**A write-behind queue MUST be bounded and de-duplicated (K-277).** Its entries are whole
frames, so its depth is a memory budget: at most eight frames may be waiting to be written,
and a frame already on its way down is never handed over a second time. A frame counts as
parked only when its write has *finished*, so anything deciding what to copy down MUST ask
"is it on its way?" as well as "is it there?" — asking only the second is how the idle
backup re-queued the same frames every few milliseconds until the application held tens of
gigabytes. A refused park costs that frame its place on disk and nothing else: it is still
on the card and in memory, and it is offered again later.

**Shipped (K-214).** All three tiers run. The VRAM tier holds finished display textures
(K-187), the RAM tier holds their bytes, and the disk tier parks them in a folder that
outlives the session. The rungs between them are built both ways: a frame evicted from VRAM
is read back off the card and lands in RAM and on disk, and a frame held below is uploaded
straight back into a texture rather than composited again. What the tiers hold is
**final comp frames only**, plus one further store that is not a tier of the ladder: the
**per-effect intermediate cache** (K-421). It lives in the realiser, VRAM only, and holds
every effect's output under a content name (§5.2). It never demotes and nothing is promoted
into it; its purpose is the seconds between two edits of one stack, so that editing the last
effect of a layer re-runs that effect and no other. It is bounded (256 MB by default, a
setter beside the frame budget), empties with Clear cache, and is written only by committed,
non-playback renders — a drag and a playback run read it and leave it alone. The same store
holds **nested frames** (K-422): a non-collapsed Precomp's finished linear texture, filed
under the nested comp's own frame key (§5.2) mixed with the exact render scale and sample
count the texture was made at, and served wherever that comp is realised — as a layer, as a
matte, as an effect's layer input. The decode planner asks the store before it plans a
nested comp's decodes, and plans none for a held frame; what it says is held it pins until
the frame is realised, so its answer cannot be evicted out from under the realiser. A
collapsed Precomp is never cached: its inner draws are spliced into the parent's list and
composite against the parent's stack, so there is no one picture to keep. The general
node-output cache the K-178 evaluator will own is still not built; these are the
effect-stack and precomp slices of it, ahead of the evaluator, built where the work runs.

**"Ahead of the playhead" applies to BOTH lower rungs, and neither used to.** The ring renders
ahead of the clock, so a frame is composited before it is shown — but the trip *up* the ladder
was made at the moment the frame was wanted, inside the turn that had to produce it. From memory
that is an upload: quick, but paid out of the frame's own budget rather than out of the slack the
ring exists to bank. From disk it is worse — see below. Both rungs are now climbed over the same
look-ahead window whose source decodes are already posted, so by the time the ring reaches the
frame it is a hit on the card and no composite happens at all.

**And the disk rung is what makes the tier count during playback.** A read off disk
goes to the IO thread, and the bytes come back one or two turns of the worker loop later. A
frame asked for at the moment it must be shown thus always arrives too late, and playback
composites it again — a span parked on disk was then worth nothing to playback, which is most
of what the disk tier holds after a project is re-opened. Playback asks for the frames of its
look-ahead window instead, at the same time as it posts their source decodes, so the frame is
on the card before playback reaches it. Three refinements close the gaps that lead alone left
open. Pressing play asks the disk for the first stretch of the run before the first render
turn — at the start of a run the ring fills by rendering back-to-back, so a lead measured
from the render head is no lead at all there. In **every-frame mode only**, a frame whose
copy has been asked for and not yet arrived is given a bounded grace (tens of milliseconds)
before being composited anyway: the mode promises every frame, not any particular arrival
time, and the copy is far cheaper than the render. Adaptive playback never waits — it keeps
chasing its clock, and a frame that has not arrived is composited as before. And a frame
read back off disk is banked in the RAM tier as well as uploaded, so the next pass over the
same span climbs from memory instead of reading the same files again — without that, a comp
larger than the VRAM budget re-read its files on every pass and the IO thread's rate became
the playback rate.

**Two costs the ladder used to pay for each frame, and no longer pays.** The bytes of a frame
are held in one allocation that the memory tier and the disk tier share, in place of a copy for
each (8 MB for each 1080p frame, twice, on the worker thread). And a promotion writes into a
display texture that the VRAM cache has finished with, in place of making one, whenever nothing
shows that texture any more. The share count of the texture is what says so, which is the only
safe test: a write into a texture that a present still shows would put the wrong picture on the
screen.

### 5.2 Cache key

Every cache entry is keyed by a 128-bit content hash (BLAKE3-short or xxHash3-128; collisions
treated as impossible — no structural comparison at lookup):

```
key(node) = H(
    node type id ‖ algorithm version,
    evaluated parameter values at the node's local time (post-expression),
    local time,
    quality (preview resolution tier, bit depth, draft flags, proxy state),
    key(input₁) ‖ key(input₂) ‖ …,
    keys of all temporally sampled inputs (declared in the metadata pass)
)
```

Normative details:

- **No instance identity and no timeline position appear in any key.** "Node id" in K-016 means
  the node's type identifier plus algorithm version — never which layer or comp instance it
  came from, and never where the playhead or layer sits on the timeline. This is the After
  Effects Global Performance Cache lesson taken whole: because keys are pure content, an undo
  instantly revalidates every frame it restores, a duplicated comp shares its original's cache
  entirely, a layer moved in time re-uses every frame whose content hash is unchanged, and the
  same nested comp used in five places renders once. It also makes compile-time deduplication
  free — identical subgraphs collide to identical keys.
- **Evaluated values, not keyframe data**: a parameter animated elsewhere but constant over a
  span hashes identically across that span.
- **Algorithm version** is bumped whenever an effect's output changes, invalidating stale
  entries by construction.
- Seeded randomness (wiggle, noise) hashes its seed and time inputs; expressions are
  deterministic (K-305), so their outputs are hashable values like any other.

**Invalidation is pure hash mismatch.** There is no invalidation machinery, no dirty flags, no
dependency walker: an edit changes evaluated values, values change hashes, old entries simply
stop being addressed and age out via eviction.

**As built (K-214).** Every tier is content-keyed, and the invalidation machinery is gone. It
briefly existed: while the tiers were keyed by `(comp, frame, scale)` an edit did not rename any
frame, so the only safe answer was for a committed change to drop every held frame of every
composition — and the cost was paid on the edits that cannot change a pixel. A rename, a
work-area nudge, a solo toggle, sound added to a layer, an opacity keyframe on a hidden layer:
each one emptied the cache and the bar went blank with it. None of them does now, and an undo
finds its frames still filed under the names the restored document asks for.

**A derived camera is in the name too** (K-417). A Camera layer carrying a *solve link* is
placed each frame from a camera solve that the document does not contain, so a key made from
its stored transform would name two different pictures the same and hand back the frames
banked before the solve landed. The frame key therefore asks for the camera the picture is
actually drawn with — `SourceStamper::camera`, defaulting to the document's own answer and
overridden by the renderer to follow the link — which is the same reading `build.rs` and
`headless.rs` composite through. No algorithm-version bump was needed: an unlinked camera
hashes exactly what it always did.

The one place the key was not honest has been fixed with it: a layer's inherited **parent-chain**
placement now feeds its contribution (`ALGO_VERSION` 2). A hidden layer contributes nothing —
correctly, since it draws nothing — but its children still follow it, so moving a hidden parent
moved the picture while leaving every name alone, and the children served frames from before the
move. K-206 makes that the common case rather than a corner: a Null is the layer a user hides
most readily, having nothing to look at.

**The quality axis is one number, and everything that reads it must round the same way.**
Auto resolution keys at 1% steps, thus two scales inside one step are one quality. Footage also
folds in the width it decodes at, and that width came from the raw scale — so 0.4235 and 0.4240
were one quality by the tag and two names by the width. The cache bar asks by a scale rounded to
a thousandth, which is nearly never the float the render used, thus it named every frame
differently from the way it was banked and drew nothing over a composition that was full and
playing. A composition of solids was correct throughout, because a solid folds in the tag alone.
The decode width now comes from the same rounded scale the tag does (`Quality::keyed_scale`),
thus the width in the name is the width the pixels were decoded at.

A frame is only nameable once its footage is probed. Until then it renders live and is banked
nowhere, so an entry can never be a promise the renderer did not keep. A **file parameter**
(a `.cube` LUT, a `.lens` prescription) joins the key by path, size and last-modified time
(K-431) — the loaders identify a file by more than its path, so a name that mentioned only
the path could outlive an edit to the file itself.

**A nested comp is named on its own (K-422).** The key used to fold a Precomp layer's comp
into the parent's hasher inline, so the nested frame had no name of its own. It is now made
with a fresh hasher — `comp_frame_key` of the nested comp at the layer's time on the flick
grid, at the same quality tier — and the parent folds in that 16-byte name. The parent's
semantics are unchanged (an inner edit still renames every parent frame that shows the
comp), and the nested name is the same whichever parent asks, at whatever comp time, which
is what "the same nested comp used in five places renders once" above rests on. The
builder carries the name on the nested draw, the realiser files and serves the texture
under it, and the decode planner skips a held comp's decodes by it. `ALGO_VERSION` 4.

**The per-effect names (K-421).** An intermediate is named exactly as the formula above says
a node is, with the chain made explicit: `key_k = H(input, raster, flare substitutions,
op_0 … op_k)`, where each op contributes its effect name and algorithm version, every
resolved parameter value (post-expression, post-rescale — the numbers the kernel is handed),
and the identity of whatever rides beside it: a LUT by path and mtime, a custom lens by its
content hash, a mask path by its vertices. The flare term counts the frames a Lens flare
drew other optics than its parameters name (K-431); it was the bake *generation*, which
moves the moment any bake is queued, so a keyframed aperture renamed every op in the
project on every frame. The input is the layer's source *by identity*,
not by its bytes — the decode job's fields for footage, the colour and size for a solid —
plus the masks and paint baked into it and the raster size; a nested comp's input is its own
frame key (K-422). A text or shape layer and an adjustment layer's composite have no name
yet and run their stacks uncached. And an op that binds a picture nobody named — another layer's
texture as a plate or a matte, the neighbour frames, the flow field — **breaks the chain**:
it and everything after it have no name, because a name that omitted an input would be a
wrong picture filed under a true-looking label. `ThisLayer` and an unset reference are
functions of the chain itself and do not break it.

**A render probes what its composition can show, and nothing else** — the footage its layers
name, the footage its Sequence layers' clips name, and the same again through every composition
it nests (`lumit_core::model::comp_footage_items`, walked whatever the layers' switches and spans
say, so the answer does not move with the playhead). A probe opens a file and loads or builds its
frame index, so probing the whole Project panel made the first frame of *any* composition wait
for every file in the project, and a freshly made empty composition wait for all of them to show
nothing. The probe cache is per item and survives across compositions, so a source shared by two
comps is opened once a session and each comp's first frame pays only for what it adds. Nameability
is unchanged by this: everything a comp shows is probed before its frames are named.

### 5.3 Eviction

Cost-aware LRU (GreedyDual-style), managed by the resource governor's budgets: each entry
records size in bytes and measured recompute cost in ms; eviction preference is stale ×
cheap-to-recompute × large. Additional rules: the displayed frame and a window around the
playhead are pinned; final comp frames outlive intermediates at equal staleness (playback needs
finals; intermediates rebuild from cached inputs); VRAM eviction demotes to RAM only when
recompute cost exceeds a readback-cost threshold, otherwise drops.

**As built (K-214), with one deviation, and why.** Every eviction demotes; there is no cost
threshold on the decision. The threshold is the right idea and the number to compare against
is not available: a composite is *submitted* to the graphics card and the call returns, so the
wall-clock the renderer can measure around it is the submit rather than the work — a frame that
costs the card 8 ms can measure under one. Gating the ladder on that would gate it on noise.
What bounds the traffic instead is a hard ceiling on how many read-backs may be in flight at
once (four); a burst of evictions past it drops the extra frames, which costs a re-render and
nothing else. The read-back is *encoded* at eviction time and collected a worker turn or two
later, so an eviction never makes the preview wait for the bus. The measured cost is still
recorded and still used — for eviction **ordering**, which is comparative and where a noisy
number is good enough.

Two rules fall out of the ladder being two-way. A frame that arrived by being promoted UP is
not read back when it goes: it is already held below, and demoting it again would be pure
traffic — this is what stops a scrub across a span larger than the cache from reading the same
frames off the card over and over. And a frame goes to disk on the way *down*, not when RAM
later forgets it, so an editor that stops unexpectedly has still banked what it rendered.

### 5.4 Disk cache format and location

The disk cache lives in the project's sidecar folder (`<project>.lum-cache/`,
[10-FILE-FORMAT.md](10-FILE-FORMAT.md)), deletable at any time with no correctness effect:

- `frames/<first two hex chars>/<hash>.kfr` — one file per entry: a small header (format
  version, dimensions, pixel format, colourspace marker) plus LZ4-compressed fp16 planes.
- `index.db` — SQLite: hash → file, size, recompute cost, last-use, quality. Rebuilt by scan if
  missing or corrupt; a corrupt entry is discarded silently and re-rendered.
- Default size cap 50 GB, user-set; evicted by the same cost-aware policy using the index.

**As built (K-214).**

- **Where.** Three choices, in Settings → Performance (docs/07 §15): the application's own
  cache folder keyed by the document's id (the default), a `<project>.lum-cache/` folder beside
  the project file, or a folder the user picks. The choice is application-wide by default and
  can be made **per project** instead (K-215), in which case it is stored in the `.lum` through
  an ordinary op — so it is undoable, and it travels with a copy of the project rather than
  staying behind in one machine's settings. A project's own answer overrides the application's. The sidecar cannot be the default because it
  only works once the project *has* a file, and a project should cache from the moment it is
  created; the document id is written into the `.lum` and survives every save, so an app-data
  cache still finds its frames tomorrow. An unsaved project set to "beside the project" falls
  back to the app-data folder until it is saved. Changing the choice moves nothing: the old
  folder is simply no longer addressed, and may be deleted by hand at any time.
- **Format.** `KFR1`: magic, pixel format, colourspace, width, height, then LZ4-compressed
  **RGBA8** — the display-encoded bytes the preview compositor actually produces, which are the
  same pixels an export writes (K-031). fp16 planes join as a new format tag when the working
  format reaches the processor; the header carries a format field for exactly that. One
  canonical channel order on disk, so a cache is not silently unreadable on the next platform:
  the Windows and macOS zero-copy paths composite in BGRA, and the swizzle is paid on the IO
  thread in both directions, never on a render.
- **The index** (K-215). `index.bin` — every entry's hash, size, recompute cost, last use and
  quality — plus `index.log`, one fixed-size record appended per change since that snapshot.
  Opening reads the snapshot and replays the log; a record torn by a crash is a partial
  trailing record and is discarded by length. Either file missing or unreadable means the
  folder is walked once and the index rebuilt from it, which is this section's "rebuilt by scan
  if missing or corrupt". So presence, the byte total and the eviction order all cost nothing at
  run time, and **eviction is the same stale × large ÷ cheap-to-remake policy as the tiers
  above** rather than the modification-time order a filesystem is limited to.

  A **deviation, recorded rather than silent**: the spec says `index.db`, SQLite. This is a flat
  map of fixed-size rows, read once at start-up and otherwise held in memory; SQLite would add a
  C dependency to an engine crate to store it, and the media frame index (docs/10 §3) already
  sets the house precedent of a plain binary sidecar.
- Anything unreadable — bad magic, unknown format, truncation, a failed decompression, the
  wrong pixel count — is deleted and re-rendered. The cache can never make a frame wrong, only
  faster.

### 5.5 Idle-time background cache fill

After ~200 ms without user input, an idle-priority scheduler renders final frames outward from
the playhead across the work area, at the current preview quality, into RAM (write-behind to
disk). It yields to any interactive request via epoch cancellation and is the first thing the
degradation ladder pauses. Concurrency adapts to measured per-frame cost and memory headroom
(the MFR lesson) — never a fixed thread count.

**As built (K-424).** The fill renders one frame per idle turn, forward-biased two frames
ahead for each one behind, after a ~200 ms lull — **into VRAM first**, the same tier a
scrub renders into. Both walks **wrap at the ends of the work area**: playback loops it, so
the frame after the last is the first, and the forward walk carries on there rather than
stopping; the walk ends when every frame has been visited once. Once the card is full the
fill **keeps going into RAM**: each further render pushes the card's stalest frame out, and
an eviction is a read-back into memory (and on to disk), so what the walk leaves behind is
the card full and the rest of the work area held below it — a loop that fits in VRAM plus
RAM plays warm from end to end. The reach is the two budgets together divided by one
frame's bytes at the current preview scale, so the walk never cycles frames through disk.
The LRU stays the eviction authority; the fill never chooses a victim. A frame already
held in memory is climbed back onto the card only while the card has room — promoting one
into a full card would push another down, and the next turn would promote that one, for
ever — otherwise it counts as warm where it is and goes up when playback asks for it.

**And a second job runs on the same lull: the idle backup.** A frame reached the disk tier by
one route only — pushed out of the VRAM cache, read back on the way down, parked. That route
needs the cache to be *full*. Give it a budget larger than a session ever fills (10 GB on a
roomy card) and it is never full, thus nothing is ever pushed out, thus **nothing is ever
written to disk**: the more memory the user gives the cache, the more certainly the tier that
exists to make tomorrow start warm stays empty. The failure is silent — the bar is green all
session, and blank again after a restart.

So the ladder has a second way down. On each idle lull, one held frame that is not yet parked
is copied down: the frame stays on the card and keeps serving the Viewer, and a copy goes to
memory and to disk. The copy is the same non-blocking read-back an eviction uses and is bounded
by the same in-flight ceiling, thus it can never compete with the picture. A frame that has been
copied down is marked as held below, so the day it *is* pushed out it goes without being read a
second time.

The idle fill obeys the same rule, and it matters most immediately after the backup has done its
job: **the fill never composites a frame that is already held below.** It has no deadline — that
is what makes it the fill — so a frame that exists in memory or in a file is climbed rather than
made again. Without this, re-opening a project would walk a full disk cache and re-render every
frame of it, which is the exact opposite of what the cache is for. An upload counts as that
turn's work, so a request arriving mid-fill still waits at most one frame; asking the disk for a
copy costs this thread nothing, so the walk queues those as it passes and the copies land while
it is idle.

The backup runs **alongside** the fill rather than after it. On a long composition the fill has
frames to make for as long as the budget lasts, so "when the fill is finished" would mean
"never" — which is how long the disk tier stayed empty.

### 5.6 Cache bars

The timeline shows, per comp, a per-frame strip: **green** — final frame in RAM or VRAM at
current quality, plays in real time now; **blue** — on disk only, promotable; **dimmed
green/blue** — cached at a lower preview resolution than currently displayed. Redrawn from a
lock-free bitmap snapshot; the UI thread never queries the cache itself (K-017).

**As built (K-214, K-441).** The strip is one byte per frame, in two nibbles: *where* the
picture is kept and *how big* it is.

The **low nibble is the storage state** — `0` nothing, `1` held coarser, `2` held at this
resolution, `3` on disk coarser, `4` on disk at this resolution — and playable outranks
promotable, so a frame both held and parked reads as held.

The **high nibble is the resolution tier**: the preview *divisor* the picture found was
actually made at, relative to the scale the bar asked about — `1` full, `2` half, `3` third,
`4` quarter, the same ladder the realtime controller drops along (§6.2). It is `0` exactly
when the storage state is `0`, since a frame nobody holds has no size. This is what lets the
bar say not just "cached" but "cached at what size" ([15-DESIGN.md](15-DESIGN.md) §6.3); the
tiers are probed finest first, so the reported divisor is the best picture there is of that
frame and does not depend on the order the tiers filled.

Two limits on the tier, stated rather than guessed. A frame held at some *other* scale — one
no adaptive tier renders at — is not found at all and reads as nothing held, exactly as it did
before the tier existed. And on a sampled composition (below), a frame the refinement sweep
has not reached yet wears its sample's tier along with its sample's storage state.

The snapshot is not an optimisation here, it is the only way the question can be answered.
Under content keying, "is frame 12 held?" means *naming* frame 12 — hashing the whole
composition at that time — which needs the renderer's probe results and is not work for the
thread that paints. So the bar leaves a note saying which composition, how many frames and what
preview scale it is drawing, and the render worker publishes the strip it asks for: all zeros
until the worker has answered, which is honest rather than another composition's frames.

Three bounds on that work, all stated rather than hidden. It is recomputed only when something
has actually moved (the bar's request, the document revision, or one of the three tiers'
contents), at most every 150 ms — and at most every 500 ms while playback is running, since the
walk shares the thread that renders and that thread has a deadline. And a composition longer than about a thousand frames is
**sampled** — one frame per stride stands for its neighbours — because the stripe is a
thousand-odd pixels wide at most and hashing forty thousand frames to draw it would be work
nobody can see. The dimmed state probes the adaptive tiers' scales (Half, Third, Quarter),
which are the scales frames genuinely get cached at, rather than every scale a Viewer could
be resized to.

## 6. Preview

### 6.1 Preview resolution

Full / Half / Third / Quarter / Auto, per comp, chosen in the Viewer. This is true raster
downsampling — Half renders every node at half raster in each axis (¼ the pixels, roughly 4×
the speed), not a display-side rescale. Auto picks the tier that supplies at least one rendered
pixel per displayed pixel at the current Viewer zoom. The tier is part of the cache key's
quality field, so each tier's caches are first-class and independent.

### 6.2 Adaptive degradation

During interaction only — scrubbing, dragging a property, moving a layer — the engine MAY
degrade below the user's chosen tier along the ladder in
[13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) (resolution tier, skipped motion blur,
blend-instead-of-flow interpolation, macro-tiling). On idle the current frame re-renders at
full chosen quality. Degradation MUST be visible in the status readout, MUST never apply to
export, and MUST never change the document.

### 6.3 Scrubbing

Latest-wins with epoch cancellation: every playhead move bumps the epoch; in-flight work for
stale epochs aborts at its next checkpoint; there is no queue of stale positions, only a
mailbox holding the newest completed frame. Completed-but-stale frames still enter the cache —
the work is kept. First (possibly degraded) frame within the scrub budget
([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)), refined on idle.

### 6.4 Playback

- **Render-ahead ring buffer**: playback renders ahead of the playhead into a bounded ring
  (target 8–16 frames, elastic with measured frame cost), fed by the same cache — a green-bar
  region costs a VRAM promotion only.
- **Pre-roll**: on play, Lumit fills a short ring segment before the first frame is presented
  (bounded at ~150 ms) so playback starts clean instead of stuttering into speed.
- **Sustained playback**: decode(N+k) ∥ evaluate(N+1…) ∥ present(N), bounded queues providing
  back-pressure. If the ring underruns, the degradation ladder engages before frames drop.
- **A/V sync**: the audio clock is the master ([09-AUDIO.md](09-AUDIO.md)). Video frame
  selection is a function of the audio clock; when video falls behind, frames are held/dropped
  and audio never glitches. Positions are tracked in samples, never frames.

### 6.5 Preview modes (K-030)

Two independent controls, never merged: the **preview resolution picker** (§6.1 —
Full/Half/Third/Quarter/Auto, in the Viewer bar, the default way to move through a
project) and the **preview mode toggle** (Cached/Realtime, in the transport and Settings →
Preview). Realtime is NOT an entry in the resolution dropdown; when the mode is Cached,
the picked resolution is always honoured (§6.2's interaction-only degradation aside).

Playback runs in one of two user-selected modes (per comp):

- **Cached** (default): as above — full chosen quality, render-ahead ring plus the three-tier
  cache; if the ring underruns, the degradation ladder engages, and background cache fill
  makes the next pass better.
- **Realtime (adaptive)**: never waits for cache. Every frame is rendered live at whatever
  resolution tier sustains the comp frame rate, adjusted continuously from measured frame
  cost (drop a tier when the last frames overran, climb when there is headroom; hysteresis
  so the tier does not flap). Frames rendered this way still enter the cache at their tier.
  This is the "just play it now" mode for heavy comps: motion and timing are judged in real
  time at reduced resolution rather than full quality after a wait. The active tier MUST be
  visible in the Viewer's degradation indicator, and the mode MUST never affect export.

## 7. Export

### 7.1 The export queue

Export runs through a queue ([07-UI-SPEC.md](07-UI-SPEC.md)): each item is a comp + range (work
area by default) + preset + output path. Queue items snapshot the compiled evaluation graph at
queue time; subsequent edits do not alter a queued item.

**Editing during export is supported in v1.** Because the export renders from an immutable
snapshot, the user keeps editing while the queue runs; export work executes at background
priority and interactive work pre-empts it (the governor arbitrates). A queue toggle
"prioritise export" reverses that preference.

### 7.2 Baking (K-024)

Baking — flattening retimes to explicit frame mappings, pre-compositing static subtrees,
rasterising vectors at output resolution, sampling expressions to curves — exists **only inside
the export compiler**, operates on the snapshot, and is discarded when the item completes.
Nothing baked ever appears in the project document or is observable in the file format.

### 7.3 Determinism

Same project, same Lumit version, same machine, same preset → identical output pixels, every
run. Therefore, normatively: adaptive degradation never applies to export; motion-blur sample
counts come from the deterministic formula (§4); expressions are deterministic (K-305); every
frame renders at full chosen quality regardless of load — under resource pressure export gets
slower, never different. Bit-exactness across different GPUs/driver versions is not promised
(floating-point variance); cross-machine consistency is visually lossless, same-machine
consistency is exact.

### 7.4 Encoders

All encoding goes through ffmpeg as the single abstraction: hardware encode via `h264_nvenc` /
`hevc_nvenc`, `*_amf`, `*_qsv` (probed and picked automatically), with **x264/x265 software
fallback** always available and used for quality-first masters. (ProRes/DNxHR intermediate
codecs for interchange are planned but **not in v1** - v1 encodes H.264/HEVC only, see
[TODO.md](TODO.md).) Audio: AAC via ffmpeg. Colour: working
space -> the preset's output space (Rec.709/sRGB in v1) as the final export transform; alpha
export straight or premultiplied per output settings.

**Audio-only output.** An export can carry sound and no picture: an **`.m4a`** (AAC, the same
codec and the same mixdown a video export uses) or a **`.wav`** (uncompressed 16-bit PCM,
where a bitrate means nothing and is not offered). The container opens with whichever streams
it was given — video, audio, or both — and asking for neither is a typed error rather than an
empty file.

**Container metadata.** An export writes an **ordered** key/value set into the container:
title, author, copyright, comment and creation time by default, and whatever else the
Metadata page grows. Ordered rather than a map because the order lands in the file's bytes and
export is deterministic (§7.3). The keys are FFmpeg's own (`title`, `artist`, `copyright`,
`comment`, `creation_time`), so what is written is what a player reads; an emptied field is
removed rather than written blank.

**Image sequences (K-201).** Beside the video formats, an export can write one still per
frame: **PNG** or **TIFF**, lossless RGBA, through the same ffmpeg seam (the image2 muxer) and
the same frame walk — choose `shot.png` and the frames land beside it as `shot.00001.png`,
`shot.00002.png`, … A sequence carries no audio (a folder of stills has nowhere to put it) and
no bitrate (it is lossless); a cancelled or failed sequence removes the frames it wrote rather
than leaving a folder that looks like a finished export. Both still formats can also carry
**16 bits per channel** (`rgba64`), which the video codecs cannot; the pack stage hands the
encoder little-endian samples either way and each format's own byte order is the encoder's
business, not the caller's.

**The export dialogue's own fields (K-201).** Beyond the preset stamp, the dialogue carries a
**frame rate** (defaulting to the comp's own; a different rate resamples by nearest comp frame
over the same wall-clock span, and the file is stamped with the chosen rate as an exact
rational — 29.97 never quietly becomes 30) and an explicit **range** in comp frames
(defaulting to the work area exactly as §7.1 says, else the whole comp), plus the AAC bitrate
when audio joins.

### 7.5 Preset set (v1)

| Preset | Frame | Codec | Bitrate |
|---|---|---|---|
| YouTube 1080p60 | 1920×1080 @ 60 | H.264 high, 4:2:0 | VBR target 16 Mbps, peak 24 |
| YouTube 1440p60 | 2560×1440 @ 60 | HEVC (H.264 fallback) | VBR target 25 Mbps, peak 35 |
| YouTube 4K60 | 3840×2160 @ 60 | HEVC (H.264 fallback) | VBR target 45 Mbps, peak 60 |
| Vertical 1080×1920 | 1080×1920 @ 60 | H.264 high | VBR target 16 Mbps, peak 24 |
| Master (intermediate) - *planned, not in v1* | comp size/rate | DNxHR HQX or ProRes 422 HQ | codec-defined |

Every landscape preset offers a **one-click vertical variant** (1080×1920): centre-crop with a
draggable reframe, or pillar-fit. Audio on all delivery presets: AAC 320 kbps, 48 kHz. Presets
are data, not code; user presets serialise next to built-ins
([10-FILE-FORMAT.md](10-FILE-FORMAT.md)).

## 8. Scopes

Waveform, vectorscope, and histogram are GPU compute passes over the **displayed frame**
(post-display-transform by default; a scopes option selects the working-space signal instead):
one scatter/accumulate pass with atomic adds into small histogram buffers, one normalise/draw
pass.
They run at most once per displayed frame, only while a Scopes panel is open, on the same queue
as the Viewer blit; budget < 0.5 ms at 4K. Never computed on the CPU.

## Open questions

- **Per-comp compatibility toggle for blend domain** — should a comp be able to opt its
  perceptual-set modes into linear maths (the inverse of AE's "blend colours using 1.0 gamma")
  for users who want physical compositing throughout? Leaning yes, post-v1, as a comp setting
  hashed into the quality field.
- **Preserve underlying transparency** — carried in the data model but not yet specified here;
  confirm v1 or defer.
- **Matte luma in HDR** — perceptual luma (§3.5a) is defined via the extended sRGB transfer;
  behaviour for >1.0 values needs a worked example before freeze.
- **Auto preview resolution and DPI scaling** — whether Auto accounts for OS display scaling or
  raw pixels only.
- **Disk cache of intermediates** — v1 persists final frames and "expensive" intermediates;
  the cost threshold for persisting an intermediate needs tuning against real montage projects.
- **Vertical reframe keyframing** — is the one-click vertical variant's reframe animatable in
  v1, or a static offset?
- **OCIO config surface** — when OCIO lands, whether the working space becomes configurable
  (ACEScg) or stays linear-Rec.709 with OCIO only at the ends. Nothing in this document may
  assume either answer.

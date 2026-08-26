# Built-in effects

**Status: implementation-ready.** Specifies the effect model and the built-in effect suite
(K-064, K-019). Terminology per [01-GLOSSARY.md](01-GLOSSARY.md); render semantics per
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md); plugin-hosted effects (OFX, LFX) per
[12-PLUGINS.md](12-PLUGINS.md). The goal of Tier 1 is blunt: a new montage editor MUST need
zero third-party plugins to achieve the core genre look.

---

## 1. The effect model

### 1.1 Anatomy of an effect

An **effect** is one image operation instance in a layer's effect stack. Every built-in
effect consists of exactly four parts, and an effect is not mergeable until all four exist:

1. **A typed parameter set.** Every parameter MUST be animatable (keyframes and expressions,
   per [01-GLOSSARY.md](01-GLOSSARY.md) §3) and MUST be visible to the expression system by
   its stable identifier (`effect("Glow")("Radius")` style access). Parameter types: float,
   integer, boolean, enum, angle (degrees), colour (scene-linear RGBA), 2D point (comp
   space), **curve** (K-412 — an ordered list of 2..16 control points in the unit square,
   with a clamped cubic through them), seed (integer), file reference, layer reference, **mask-path
   reference** (K-408 — one of the owning layer's masks, whose *geometry* the effect walks),
   marker-trigger (§1.4), and **action** (K-417 — a button, the one row that is not
   animatable, because it carries no value to animate; see §1.2).
2. **A WGSL compute implementation** — the production path, running on wgpu (K-011).
   Implementations MUST be pure functions of (inputs, parameters, time): no global state,
   no reading outside declared inputs.
3. **A CPU reference implementation** (K-019) — a plain Rust implementation of identical
   semantics. It is the test oracle (§1.6) and the CPU fallback rung of the degradation
   ladder ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)).
4. **A trait declaration** (§1.3) the evaluation graph compiler reads to plan scheduling,
   caching, and cancellation.

Effects are versioned. The version participates in the cache key (K-016), so changing an
effect's maths in a release invalidates stale cached frames rather than mixing generations.

The four parts are **declared once**, in the effect's own file (K-381): the parameter set is
a struct whose fields are the parameters, the schema below is generated from it, and the
effect registers itself by one line of a written list. An effect is therefore *not* a variant
of a closed enum, and one that this build has never heard of — an OFX plugin, in time a
user's own — is an ordinary member of the catalogue rather than an impossibility. See
[impl/effect-registry.md](impl/effect-registry.md) for the shape, the parameter bag a frame
resolves to, and the rules for parameters that are not in the schema at all (§4 there:
derived parameters read out of a shader or a node graph, and the user's own spare
parameters).

### 1.2 Parameter conventions

- **Names** are sentence case in the UI, stable snake_case identifiers in the schema.
- **Ranges** declare a slider range and a hard range; sliders MAY be exceeded by typing,
  hard ranges MUST NOT be. Hard ranges MAY be one-sided (K-090): a threshold clamps at
  zero below and is unbounded above where that is the honest shape of the parameter.
- **Closed ranges are their own kind** (K-414). Where a parameter's whole meaning lives
  inside one range — a wipe is between not begun and complete, and there is no picture
  either side of that — the schema declares a **Slider**: one range that is both the
  slider's travel and the hard bound, drawn as a track and thumb with the value beside it.
  The stored value is an ordinary float, exactly as an Int's and an Angle's are (the kind
  is the control, not the storage), so adopting it on an existing parameter moves no stored
  value, no keyframe and no pixel — and the parameter keeps every float affordance,
  including the graph editor. It is deliberately *not* the default for a parameter that
  merely has bounds: Temperature's ±150 slider runs to a ±200 hard range precisely because
  there is a picture beyond the slider's end, so it stays a plain float.
- **Defaults** MUST produce a visible, tasteful result on typical 1080p60 game footage —
  the "drop it on and it already looks right" rule. An effect whose default state is a
  no-op is a bug unless the effect is inherently trigger-driven (Flash, Shake in
  beat-triggered mode).
- **Reset** restores defaults per parameter and per effect.
- **File-reference** parameters (K-111) hold a path chosen from a native file dialog, filtered
  by the effect's declared extensions (e.g. `.cube` for a LUT). They animate only by
  *stepping*: the stored value is a set of referenced paths plus a hold-keyframed index that
  selects which one is live at a given time — two file paths cannot be blended, so only Hold
  keyframes (§6.2 of [03-DATA-MODEL.md](03-DATA-MODEL.md)) apply; the common case is a single
  path with a static index. An **unset** file resolves to identity: the effect is a no-op
  until a file is chosen, the one sanctioned exception to the "no no-op default" rule above,
  since a file the user must supply cannot have a tasteful default.
- **Layer-reference** parameters (K-123, [impl/layer-input.md](impl/layer-input.md)) name
  a layer in the same composition as an auxiliary picture an effect samples — a
  depth pass for Depth of field (§3.22), a bright-source matte for the Lens flare (§3.27).
  The stored value is an optional layer id (the shape
  a matte reference uses, §5.1 of [03-DATA-MODEL.md](03-DATA-MODEL.md)), static in v1. The
  host renders that layer alone and threads its texture to the effect, exactly as a matte
  layer is rendered alone. An **unset** or **dangling** reference resolves to identity — the
  same sanctioned exception to the "no no-op default" rule, since a layer the user must
  supply cannot have a tasteful default.
  **This layer** (K-288): a reference may name the layer the effect is *on*, and then it is
  not a second render at all — it is the effect's own input at its point in the stack. On an
  ordinary layer that is the picture the effect is about to process; on an **adjustment
  layer** it is the composite of everything below, which is the only picture an adjustment
  layer has, and which is what makes a matte-sourced effect usable there. A schema may
  declare this the *default* for one of its layer references (the Lens flare's Matte
  does), in which case a fresh instance added to a layer starts pointed at that layer; the
  source combobox below does not apply to a this-layer reference, since nothing is
  re-rendered. Beside the picker sits a **source** combobox
  (K-142, revising K-125's before/after bool) choosing *what of* the referenced layer is
  read: **None** (its raw footage/solid — no masks, no effects), **Masks** (its source plus
  its masks) or **Effects and masks** (its finished picture — a graded or blurred input).
  The same three-way source applies to a track matte (§5.1 of
  [03-DATA-MODEL.md](03-DATA-MODEL.md)). Temporal effects on the referenced layer (echo,
  flow motion blur) are still not sub-sampled through the input in v1 — the spatial and
  colour stack applies, an echo/flow degrades to a still (the K-125 boundary).
- **Mask-path** parameters (K-408) name one of the **owning layer's** masks, and hand the
  effect that mask's *geometry* — where the curve goes — rather than the coverage it produces.
  Coverage is a picture, and a picture cannot say which way is *along* a curve; a brush that
  walks a path from one per cent to another, or segments marching round one, need the
  vertices. The stored value is an optional mask id, static in v1 exactly as a layer reference
  is: the *shape* animates, on the mask's own path keyframes, but which mask is named does not.
  The panel draws the layer's masks by name with **First mask** as the unset entry — the
  self-default a schema declares, and the reason it resolves at render time rather than being
  written into the instance: an effect is usually added before the mask is drawn. A mask in
  mode **None** is offered like any other, since that mode is geometry-only and is precisely
  the mask somebody draws for an effect to walk.
  The render flattens the named mask's `path_at(t)` to an **arc-length-parameterised
  polyline** — vertices plus the distance along the path at each of them — within a fixed
  tolerance of **0.5 px at composition size**, and threads it beside the op the way the Matte
  (§2.6) and the auxiliary lists are threaded. The tolerance is a constant on purpose: the
  polyline is part of what the effect draws, so a tolerance that could vary with the preview
  raster would let one frame key name two pictures. A row naming nothing, a mask since
  deleted, or a layer with no masks all arrive as an **empty polyline**, which is the effect's
  documented no-op — the same sanctioned exception an unset file or layer takes.
- **Curve** parameters (K-412) hold a **tone curve as its own control points**: an ordered
  list of 2..16 `[x, y]` pairs in the unit square, the identity diagonal by default. This is
  the one parameter whose value is a *shape*, and it is what an editor edits — points move
  sideways as well as up and down, which is exactly what a row of fixed sliders cannot say.
  A **clamped cubic** through the points (Photoshop's family) is fitted **host-side** and
  baked into a 257-entry table per channel, so both render paths are handed identical numbers
  and neither fits a spline per pixel; §1.6 is then checking the *lookup*, which is the point
  of baking it. Curve values are **static in v1**, joining File, Layer and mask-path on that
  side: a list that grows and shrinks has no interpolation between two keyframes, which is
  precisely why After Effects' own curve blob only ever steps. A list that arrives out of
  order, outside the square, with a repeated x, or with fewer than two points is
  **straightened on read** — sorted, clamped, deduplicated, and replaced by the diagonal when
  nothing usable survives — quietly, because it comes off a document rather than a caller.

- **Action** parameters (K-417) are **buttons, not values**: a row the panel draws as a push
  button, which asks the engine to *do* something rather than describing what a picture
  should look like. The Camera track's Analyse and Cancel (§3.85) are the first two, and the
  kind is generic because they will not be the last — beat detection is already waiting.
  An Action is the one exception to the "every parameter is animatable" rule of §1.1, and it
  is an exception because there is nothing to animate: it carries **no value**, so no
  `EffectParam` is written for it, nothing is stored, nothing keyframes, no expression can
  read it, and the resolve step puts nothing in the arena — so pressing one renames no cached
  frame. It crosses the bridge as an **event** naming the effect instance and the row, never
  as a parameter value. Written as a Bool the effect watched for a rising edge, a button
  would keyframe, save, and fire again the next time the project was opened; that is what
  this kind exists to prevent.

### 1.3 Traits

Every effect declares, statically:

| Trait | Values | Consumed by |
|---|---|---|
| **Cost class** | `trivial` (pointwise), `cheap` (small fixed kernel), `moderate` (large-radius / multi-pass), `heavy` (iterative or flow-based) | Adaptive degradation ordering, background render budgeting |
| **ROI support** | `exact` (output pixel needs only the same input pixel), `padded(r)` (needs input dilated by radius r, in **px@comp** — scaled to the raster in play exactly as a px@comp parameter is, K-433), `full-frame` (needs the whole input) | Region-of-interest rendering, tiling |
| **Temporal window** | Set of source-relative frame offsets required, e.g. `{0}`, `{-1, 0, +1}`, `{-n..0}` for echoes | Cache prefetcher and decode planner (§2.5) |
| **Alpha mode** | `premultiplied` (default) or `unpremultiplied` (§2.2) | Host unpremultiply/re-premultiply wrapping |
| **Cancellation points** | `per-pass` and/or `per-tile` | Epoch-based cancellation on scrub (K-017): every pass boundary and tile boundary MUST check the epoch and abandon work |
| **Randomness** | `none` or `seeded` | Determinism audit (§2.4); frame keys — a seeded effect's pixels are a function of time under constant parameters, so the layer's local time joins its cache key |
| **Marker input** | `none` or `beat` | Marker-trigger plumbing (§1.4); frame keys — a marker-driven instance's pixels follow the beat times, so its local time and §1.4 window join its cache key |

### 1.4 Marker-trigger parameters

Montage effects fire on beats. A **marker-trigger** parameter binds an effect to markers on
the comp or a named layer, filtered by label (default: beat markers, see
[09-AUDIO.md](09-AUDIO.md) §5). At evaluation the host supplies the effect with the ordered
marker times inside its temporal window plus the nearest markers either side of the current
frame. Markers are project data, so marker-driven effects remain pure functions of the
project and time — determinism is preserved. Effects with `marker input: beat` MUST also
work with no markers present (falling back to their continuous behaviour or to manual
keyframed triggers).

**Status (v1 plumbing, shipped):** resolution receives a marker context — the comp's
beat-marker times translated into the layer's local time (one subtraction with the
layer's start offset, the same subtraction that produces the layer time itself, so the
envelope maths lives in a single time base) plus the comp frame rate, since
duration-class parameters are authored in comp frames. It is built by one shared
constructor that preview and export both call (K-031), and a caller without markers
passes an obvious empty context on which every marker-driven effect falls back
gracefully. v1 binds to **comp beat markers only**: binding to a named layer's markers,
and label filtering beyond the beat kind, follow later with no change to the context's
shape.

### 1.5 The effect stack and adjustment layers

- Each layer owns one ordered **effect stack**, applied top-to-bottom after masks, before
  transform (per-layer render order in [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
- **Adjustment layers** render no content of their own; their effect stack is applied to
  the composite of all layers below, and the adjustment layer's masks and opacity attenuate
  the result. This is the standard vehicle for the montage "style pass" (motion blur + glow
  + grade over everything).
- Effects MAY be individually bypassed; the layer's fx switch bypasses the whole stack.
  Bypass state is not animatable (use the effect's own Mix/Amount parameter for that).
- Every effect SHOULD expose a final **Mix** parameter (0–100%, default 100%) blending
  processed over unprocessed input, host-provided so it is uniform.
- **Every Mix row carries a Blend choice** (K-425): how the effect's result combines with
  its input, offered as the layer blend modes verbatim (`BlendMode::ALL`, the same words
  the layer's Mode dropdown uses), default Normal. It is injected beside `mix` by the
  derive on every effect that has a Mix slider and does not declare a `blend` of its own
  (the Lens flare keeps its older one), and implemented **once at the dispatch seam**, not
  per kernel:

  ```
  unmixed = kernel(input)  at Mix 100          # the seam forces the kernel's Mix
  out     = input·(1 − mix) + blend(input, unmixed)·mix
  ```

  The Mix lives inside every kernel, and blending an already-mixed output would apply the
  Mix twice; so when Blend is anything but Normal the kernel runs with Mix forced to 100
  and the seam lerps once, after the blend (`cpu::blend_seam`, `cpu::blend_mix` and its
  op-for-op WGSL twin `fx_blend_mix.wgsl`). The domains follow the compositor's layer
  modes ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §blend domains): Add, Multiply,
  Lighten, Darken and Subtract per channel in linear light, the rest encoded to sRGB for
  the W3C formula and decoded; alpha is the effect's own. **Normal runs no pass** — the
  kernel's own Mix does the whole job and the picture is byte for byte what it was
  (K-258). Where an effect also has a generic strength matte (§2.6), the blend runs first
  and the matte dissolve after it, so the matte still holds the whole result off.

### 1.6 CPU reference as oracle

For every effect, the test suite renders a fixed corpus (synthetic gradients, alpha edges,
HDR values > 1.0, real game-capture frames) through both implementations and asserts
agreement within a declared tolerance (default: ≤ 2 ULP fp16 for `trivial`/`cheap`, small
perceptual epsilon for `moderate`/`heavy` where floating-point reduction order differs).
Flow-based effects compare against the reference flow fields, not bit-exact pixels (§3.1).
A WGSL change without a matching reference change MUST fail CI.

---

## 2. Quality rules (all effects)

### 2.1 Working space

All effects operate in the working space defined by
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.1: scene-linear, premultiplied alpha, at
the project-wide depth (K-069). Effects MUST NOT assume display-referred input: values
above 1.0 are legal and meaningful (glow depends on them). Effects MUST NOT clip highlights
except where clipping is the documented behaviour of a parameter.

### 2.2 Unpremultiplied exceptions

Colour-manipulation effects operate on unpremultiplied colour, because grading
premultiplied values shifts matte edges. Effects declaring `alpha mode: unpremultiplied`
are wrapped by the host: unpremultiply → effect → re-premultiply, fused into the effect's
first/last passes where possible. The Tier 1 effects requiring this: **the colour effects
(Colour balance, Saturation, Contrast, Gamma), LUT, Sharpen, Matte key** (edge haloes
otherwise). Contrast and Gamma join the list because Contrast's `− pivot` offset makes it
*affine* and Gamma's power curve is *non-linear* — neither is a pure scale, so unlike Exposure
and Hue shift they do not commute with premultiplied alpha (§3.18, §3.19). **Curves, Levels,
Brightness and Hue and saturation** join it for the same two reasons between them: a tone
curve and a levels power are non-linear, Brightness is affine, and an HSV round trip is
neither (§3.30–§3.33). **Noise** (§3.36) joins it because adding a signed amount to a
channel is affine for the same reason Brightness is — grain sprinkled onto premultiplied
values would be scaled by coverage and so would fade out across a soft edge instead of
lying evenly over it. **Set matte** (§3.44) joins it for a third reason again: its whole
job is to change a pixel's *coverage* and leave its colour alone, and a premultiplied value
multiplied by a new alpha would have been scaled twice. The three *generators* do not:
**Fill** (§3.34) never reads the
source colour at all, and **Gradient** (§3.35) and **Fractal noise** (§3.37) replace the
frame outright, so there is nothing to unpremultiply. Matte key joins it
because its colour-difference matte and despill read straight colour: keying the premultiplied
values would judge (and fringe) the edge pixels by their coverage rather than their true colour
(§3.21). All others consume premultiplied input directly (Block glitch, Scanlines and Datamosh
among them — §3.12).

### 2.3 Resolution-independent units

Parameters MUST be expressed in units that survive comp resizing and preview resolution:

- **px@comp** — pixels at composition size, for **every** distance, radius and
  displacement (K-419). A value is authored against the composition's own raster; the
  engine scales it by the preview resolution factor to the raster actually in play, and
  again to a different export size, so Half or Quarter preview frames exactly like the
  export. No distance in Lumit is a percentage of the composition diagonal.
- **degrees** — all angles.
- **seconds** or **frames** — durations; frames are comp-frame-rate frames.
- **per cent** — a share of something the parameter names: the host-uniform Mix, a
  channel's share of a grain, a position given as a fraction of the frame's own width
  and height. 100 is the whole of it. A per cent survives resizing because it is not a
  distance; a *distance* is px@comp, never a percentage of the diagonal (K-419).
- **none** — a number with no unit at all: a gamma, a count, a stop, a rate in Hz.

Every parameter declares which of these it is (`Unit` in `lumit_core::fx`), and a test
fails the build on one that declares nothing — the declaration is where the resolve
step learns what to scale for a preview raster, and where the panel learns what to
write beside the value (K-443).

A raw "pixels of whatever buffer I was handed" parameter is forbidden; previews at Quarter
resolution MUST look like the export, only softer.

### 2.4 Determinism

Randomness MUST be seeded and stateless: `hash(seed, frame_index, pixel/element id)` style
generators only. Two exports of the same project MUST be bit-identical per
[14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md); wall-clock time, thread scheduling, and
GPU vendor MUST NOT influence output (within §1.6 tolerances). Every seeded effect exposes
its **Seed** parameter and a "reseed" button in Effect Controls.

### 2.5 Temporal effects and the prefetcher

Effects that read other frames (`temporal window ≠ {0}`) declare the window so the cache
and decode planner can schedule input frames before the effect runs, instead of stalling
the pixel pass on demand-decode. Temporal windows are expressed in **source-relative**
frames; the host resolves them through Retime so a slowed clip requests the correct source
frames. Temporal effects MUST define behaviour at layer/clip boundaries (typical: clamp to
the boundary frame, matching Overrun semantics in [04-RETIMING.md](04-RETIMING.md)).

### 2.6 Every effect can be driven by a matte (K-395)

Every built-in effect has a **Matte** input: a layer whose brightness says *how much of the
effect* each pixel gets. It is one row, in the same place, on all of them — the layer
picker with an **Invert** checkbox beside it and a **Channel** choice (K-425), labelled
"Matte" / "Invert" / "Channel". **Two effects carry none.** The Matte key (K-425): a
keyer's subject is the picture it keys, and a strength matte over a key is a garbage
matte, which is a mask's job. And **Set matte** (K-429): every Matte row answers "how much
of me happens here", and Set matte has no answer to give — what it takes from another
layer is the coverage itself, so the row it shows is its own source picker, riding the
ordinary auxiliary-layer carriage beside Light wrap's Background rather than the universal
one. Both keep their stored ids, so a project saved before either drop loads exactly as it
did (K-065, K-258 — the forward-migration walk only appends what a schema has *grown*, and
carries a row nobody declares any more along untouched).

**The Channel choice says which channel of the matte layer drives the effect**: Luminance
(the default — the premultiplied Rec. 709 luma every kernel has always read), Red, Green,
Blue or Alpha, the same `CHANNEL_OPTIONS` list Set matte and Depth of field offer. The
pick and the Invert are applied **once, at the dispatch seam, before anyone reads the
matte**: `cpu::matte_prepare` and its op-for-op twin `fx_matte_prepare.wgsl` rewrite the
RGBA matte into a grey picture whose R = G = B = the chosen channel, clamped to 0..1 and
inverted if asked, alpha 1. Every kernel then reads luma of that grey — which is the
channel — and no kernel learns about the row, nor inverts on its own any more (Gaussian
blur, Glow and Turbulent displace used to; they read the matte as it arrives now, so
Invert cannot be applied twice). Luminance with Invert off runs **no pass**: the kernels
read exactly that already, and a pass through an fp16 texture would requantise what they
read, so the default is byte for byte the matte of K-395 (K-258). Effects that own a
channel choice for their matte carry no Channel row and get the raw RGBA: Depth of field
(`depth_channel`), Displacement map (its two channel choices), Set matte (Channel) and the
Lens flare (source detection).

This is **not** masking the effect's result. A mask hides what an effect did; a matte feeds
the effect itself, and what "feeds" means may differ per effect. The two are worth having
separately: a Fractal noise driving a Turbulent displace's *vectors* is not the same
picture as a Turbulent displace hidden behind a shape.

**The default meaning is strength.** Unless an effect declares better, the matte scales the
effect's per-pixel mix: matte white applies the effect in full, black leaves the source
untouched, grey is part way. Formally, and once for all of them —

```
k    = clamp(Rec.709 luma of the matte, premultiplied, 0..1)     # inverted if Invert
out  = input·(1 − k) + effected·k
```

— applied **after** the effect's own Mix, which lives inside its kernel. It is implemented
once beside the registry dispatch (`lumit_core::fx::cpu::matte_mix` and its op-for-op WGSL
twin `fx_matte_mix.wgsl`), not thirty-odd times, which is what makes the row meaningful on
every effect from the day it landed rather than on the handful someone remembered.

**Effects may override with a deeper meaning** where the matte belongs inside the maths.
An override takes the same row and the same labels, keeps its stored parameter id (K-065 —
a save is a save), and documents what its matte means in its own schema prose, which is
what the manual's parameter tables generate from. Six effects do for a reason of their
own, and each is a picture the dissolve above cannot produce (Set matte was a seventh
until K-429 took its Matte row away altogether):

| Effect | What its matte means |
|---|---|
| Gaussian blur (§3.8) | scales the **radius** per pixel — the blur is genuinely narrower where the matte is grey, not a full-width blur faded back |
| Turbulent displace (§3.38) | scales the **displacement vector** per pixel — the picture is warped *less* where the matte is grey, where a dissolve would blend a fully-warped picture over an unwarped one and show both edges |
| Glow (§3.3) | gates which pixels may **seed** the halo, before the bright pass — only the lit part of the matte blooms, but its halo still spills outward across the dark part |
| Depth of field (§3.22) | a **depth** pass: the luma is how far away each pixel is, and the blur widens with the distance from focus, so a mid-grey matte can be perfectly sharp |
| Lens flare (§3.27) | where the flare **detects its light sources**, in Matte source mode |
| Displacement map (§3.49) | **is the map**: its chosen channels say which way and how far each pixel is pushed, mid-grey meaning no push — the effect's subject rather than a strength applied to one (K-402) |

**And the matte scales the amount of every blur, sharpen and colour effect (K-426, the
owner's rule for mattes).** The matte is not a mask: where an effect has an amount — a
Length, a Gamma, a Density — the matte multiplies *that control* per pixel, toward its
neutral value, before the maths runs: white keeps the control where it was set, black puts
it at the value that does nothing, grey lands between. Seventeen more effects claim their
matte that way, each naming the control in its declaration (and so in the manual's Matte
row): Directional blur's Length, Radial blur's Amount, Unsharp mask's and Sharpen's Amount,
Channel blur's four radii, Exposure's Stops toward 0, Saturation toward 100, Gamma toward 1,
Temperature toward 0, Vibrancy's Amount, Hue shift's Angle toward 0, Brightness's two
controls toward neutral, Colour balance's Lift toward 0 and Gamma and Gain toward 1, every
range of Hue and saturation toward 0, Photo filter's Density, Shadow highlight's two amounts,
and Posterize's Levels toward 256 (black matte: a step too fine to see). Where scaling the
amount is *mathematically* the dissolve — the output is a straight lerp of the input —
nothing changes and the effect keeps the strength semantic: Contrast and Vignette are
exactly that, as are Tritone, Black and white, Tint, Curves, Levels, Invert, LUT and
Broadcast safe at Mix. **Threshold is the exception the owner named** (K-559): its matte
scales the **Level** instead — `level·k` at each pixel, before the cut — so the threshold
moves across the frame, which is behaviour and not a fade. The formula is one
helper on each path (`cpu::matte_toward` and its WGSL twin): `neutral·(1 − k) + value·k`,
spelled so that k = 1 is the value to the bit, which is what keeps an empty matte
byte-identical to the effect before the claim (K-258).

**And the matte scales the displacement of every distortion (K-427, the same rule).** A
distortion's amount is a distance: how far a pixel is moved. The matte multiplies *that*,
per pixel, so a grey matte is a smaller move rather than a full move faded back — which is
the difference between a warp and a double exposure of a warped picture over a still one.
Fourteen more effects claim their matte that way, each naming what it scales in its
declaration (and so in the manual's Matte row): RGB split's and Chromatic aberration's
Amount (both tiers, Wavelength included), the Shake's Amplitude and Rotation amount,
Block glitch's Intensity, Offset's shift, Lens distort's distortion (toward the identity),
Corner pin's and Bezier warp's displacement from the frame's own corners, Twirl's Angle,
Spherize's Bulge, Ripple's and Wave warp's Wave height, and Warp's Bend with both
distortions. Scanlines is the exception in form and not in rule: scaling its Intensity
would be the dissolve to the bit, so the matte **divides its Line period** instead — the
lines spread apart as the matte darkens and are too far apart to see at black (the divide
floors at `cpu::SCANLINES_MIN_K`) — and Intensity is left alone.

**k is read at the destination pixel** — where the pixel lands, not where it was fetched
from — for every one of the fourteen, which is the only choice that makes the matte's own
picture line up with the picture the viewer sees: the frame the matte draws is the frame
that comes out. It is what turns a whole-frame move into a warp, and it is why a black
matte on Corner pin leaves a pixel exactly where it was rather than leaving it transparent.

Where scaling the amount is *mathematically* the dissolve, nothing changes and the effect
keeps the strength semantic (the rule's own test): **Datamosh** is exactly that — its
output is `current·(1 − Intensity) + melted·Intensity`, so Intensity·k and the dissolve are
the same arithmetic — and **Tile, Mirror and Polar coordinates** have no amount to scale at
all, being a repeat, a reflection and a change of coordinates. Turbulent displace (K-395)
and Displacement map (K-402) had already claimed theirs, on this rule before it was written
down.

**And the matte scales the amount of every generator and stylise effect (K-428, the same
rule again).** Where an effect *draws* something over the picture, its amount is the drawn
thing's **opacity**, and the matte multiplies that — so the drawing fades along its own
length and what lies underneath is untouched. Where an effect has a **size**, the matte
scales the size. Eleven more effects claim their matte that way, each naming the control in
its declaration (and so in the manual's Matte row): Add grain's Intensity, Lightning's
bolt opacity (its core's coverage and its Glow opacity together), Radio waves' Opacity,
Vegas' Opacity, Scribble's and Stroke's Opacity, Drop shadow's shadow Opacity, Roughen
edges' Border, Median's Radius, and Emboss's and Texturize's Relief. Where scaling the
amount is *mathematically* the dissolve the effect keeps the strength semantic, and in this
pair of families that is four: **Noise, Flash, Sprite flare and Light wrap** each add a
linear amount of something to the picture — grain, a colour, light, a screened spill — so
`amount·k` and the dissolve are the same arithmetic. **Fill, Gradient, Fractal noise, Beam,
Mosaic and Find edges** have no amount of their own to scale, replacing the picture rather
than adding to it, and Glow (K-395) keeps its seed gate and the Lens flare its source
detection.

Two of the eleven are the rule at its most visible. **Median** at a half matte draws
*exactly* the Radius 1 picture when set to Radius 2 — a genuinely smaller window, which is
the difference between despeckling a sky and veiling a face. **Emboss** at a black matte is
the flat mid-grey sheet, because Relief 0 is that sheet and not the identity (§3.67) — the
one place in the batch where the matte's black is emphatically *not* "leave the picture
alone", and honestly so, since the matte turns Relief down rather than turning Emboss off.

`k` is read at the **destination** pixel for these too, which for Drop shadow means where
the shadow *falls* rather than where the shape stands: paint the matte over the wall and it
is the shadow on the wall that goes.

**And the matte scales the shutter, the decay and the completion (K-429, the same rule a
fourth time).** In the Temporal family the amount is a *duration*: how long the shutter was
open, how long a trail lasts. **Echo**'s matte scales its **Decay**, so the ghosts are
genuinely fewer and shorter where the matte is dark — and because `(decay·k)^(i+1)`
factorises as `decay^(i+1) · k^(i+1)`, a half matte draws *exactly* the half-decay trail. A
tap the matte has taken to nothing is **skipped** rather than folded in at zero, because a
zero-weight tap is not a no-op under every combine mode (Multiply by nothing is black).
Both motion blurs scale their **Shutter angle**: **Fast motion blur** (§3.2) reads `k` at
the destination pixel and spends it everywhere the shutter is spent — this pixel's own
vector, the neighbourhood's dominant sweep, and each tap's reach — so a half matte is
exactly the half-shutter streak, a genuinely shorter smear and not a long one faded back
over a sharp picture. **Motion blur** (accumulation, §3.26) has no kernel to claim it
inside, so it claims it in the **combine**: the same N sub-frame re-renders are averaged
over a shorter slice of the open shutter, shrunk toward the frame's own moment, so black is
the unblurred frame and grey is a genuinely shorter exposure. **Posterize time** keeps the
strength dissolve: it holds a time rather than drawing an amount, and its own output is
what a dissolve blends.

In the Transition family the amount is **Completion**, and scaling it per pixel is what
turns a wipe into a **gradient wipe**: paint a ramp and the edge follows the ramp instead of
the schema's straight line. **Linear wipe**, **Radial wipe**, **Venetian blinds** and **Card
wipe** each scale Completion toward 0, so black holds the frame back and white lets the wipe
finish; the Card wipe asks it per *pixel* rather than per card, so one half of a card can be
standing while the other half has flipped away. The **Iris wipe** has no Completion at all —
**the radius is the transition** (§3.71) — so it scales that instead, which is the same
sentence about the same thing: the polygon opens wide where the matte is white and shuts to
nothing where it is black, and a black matte is the same exact identity Outer radius 0
already is. Outside those families **Transform** and **Broadcast safe** keep the strength
dissolve, because scaling their amount *is* that dissolve, and the **Camera track** carries
no row at all (K-417): it draws nothing for a matte to gate.

The difference is worth stating once, because it is the whole reason the hook exists. The
dissolve can only change *how much of a finished effect* survives; it cannot change what
the effect did. A blur dissolved to a half still gathered from the full radius — a sharp
picture with a wide veil over it — where a blur at half the radius is genuinely less soft.
A glow dissolved by a matte has its halo clipped to the matte's outline, so a glow "on the
sign" does not light the wall beside it, where a gated seed does.

**One carriage, not four.** The override is a property of the declaration, not a private
arrangement per effect: the schema names the parameter and says the effect reads it
itself, and one place then decides who gets the texture — the kernel, or the dissolve,
never both. Depth of field and the Lens flare had their own lists before this and no
longer do (docs/impl/effect-registry.md §2.5b).

**The matte is a layer, rendered exactly as a depth pass is** (K-387,
[impl/layer-input.md](impl/layer-input.md)): alone, at the effect's own raster, through the
same helper preview and export share, with the referenced layer's masks and effects applied
per its Layer source mode. A matte pointed at the layer the effect is *on* is the effect's
own input, not a second render (K-288) — on an adjustment layer, that is the composite of
everything below.

**Unset is the default, and unset costs nothing.** A fresh effect starts with no matte and
Invert off; a project saved before this existed carries neither parameter and reads exactly
that (K-258). No matte bound means no dissolve pass runs at all — not a lerp by one — so
such an effect renders **byte-for-byte** what it rendered before the row existed. That is
the invariant the campaign is held to, and it has its own regression test at each stage.

---

## 3. Tier 1 — the montage staples (v1)

The in-box replacements for the scene's paid stack. Two shape rules (K-090): an effect
does **one thing** (multi-purpose designs split; an all-in-one grading suite may exist
later as a deliberate exception), and every schema declares a **category** — Blur &
sharpen, Colour, Distortion, Generate, Stylise, Temporal, Transition, Utility, Controls —
which is how
the Add-effect menu groups (**Generate** added by K-398, for the effects that *make* pixels
rather than change them; **Transition** by K-400, for the effects that *remove* the picture
progressively so a cut can be made out of them; **Controls** by K-414, for the effects that
change no pixel at all — see §3.80). The flow engine is **not** in this list: it is a per-layer option (K-088),
specified in §3.1's original text but surfaced as layer UI, not an effect. Summary:

| # | Effect | Replaces | Cost | Temporal window |
|---|---|---|---|---|
| 3.2 | Fast motion blur (flow) | RSMB | heavy | `{-1, 0, +1}` |
| 3.3 | Glow | Deep Glow | moderate | `{0}` |
| 3.4 | Shake | Sapphire S_Shake | cheap | `{0}` |
| 3.5 | Transform | AE's Transform effect | trivial | `{0}` |
| 3.6 | RGB split | stock CC pack fillers | cheap | `{0}` |
| 3.7 | Flash | strobe presets | trivial | `{0}` |
| 3.8 | Gaussian blur / Directional blur / Radial blur | stock AE trio | moderate | `{0}` |
| 3.9 | Unsharp mask, Sharpen | stock | cheap | `{0}` |
| 3.10 | Colour balance, Saturation + preset browser | Magic Bullet Looks | cheap | `{0}` |
| 3.11 | LUT | stock + Looks | trivial | `{0}` |
| 3.12 | Block glitch | Universe / glitch packs | cheap | `{0}` |
| 3.12 | Scanlines | Universe / glitch packs | cheap | `{0}` |
| 3.12 | Datamosh | Universe / glitch packs | moderate | `{-1, 0}` |
| 3.13 | Echo | stock Echo / speed-lines packs | moderate | `{-n..0}` |
| 3.14 | Vignette | stock CC pack vignette | cheap | `{0}` |
| 3.15 | Chromatic aberration | stock CC pack fillers | cheap | `{0}` |
| 3.16 | Exposure | stock CC pack exposure/levels | cheap | `{0}` |
| 3.17 | Hue shift | stock CC pack hue/saturation | cheap | `{0}` |
| 3.18 | Contrast | stock CC pack contrast/levels | cheap | `{0}` |
| 3.19 | Gamma | stock CC pack gamma/levels | cheap | `{0}` |
| 3.20 | Temperature | stock CC pack white-balance | cheap | `{0}` |
| 3.21 | Matte key | Keylight-style colour-difference keyer | cheap | `{0}` |
| 3.22 | Depth of field | Frischluft / Camera Lens Blur, with an iris | moderate | `{0}` |
| 3.23 | Invert | stock CC pack invert | cheap | `{0}` |
| 3.24 | Tint | AE Tint / duotone | cheap | `{0}` |
| 3.25 | Posterize time | AE Posterize Time | cheap | `{0}` |
| 3.26 | Motion blur (accumulation) | RSMB / ReelSmart (accumulation) | heavy | `{0}` |
| 3.27 | Lens flare | Optical Flares / Knoll Light Factory | heavy | `{0}` |
| 3.30 | Curves | AE Curves (`ADBE CurvesCustom`) | cheap | `{0}` |
| 3.31 | Levels | AE Levels | cheap | `{0}` |
| 3.32 | Brightness | AE Brightness & Contrast | cheap | `{0}` |
| 3.33 | Hue and saturation | AE Hue/Saturation | cheap | `{0}` |
| 3.34 | Fill | AE Fill (`ADBE Fill`) | trivial | `{0}` |
| 3.35 | Gradient | AE Gradient Ramp (`ADBE Ramp`) | cheap | `{0}` |
| 3.36 | Noise | AE Noise / grain plug-ins | cheap | `{0}` |
| 3.37 | Fractal noise | AE Fractal Noise | moderate | `{0}` |
| 3.43 | Drop shadow | AE Drop Shadow (`ADBE Drop Shadow`) | moderate | `{0}` |
| 3.44 | Set matte | AE Set Matte (`ADBE Set Matte3`) | trivial | `{0}` |
| 3.45 | Channel blur | AE Channel Blur | moderate | `{0}` |
| 3.46 | Linear wipe | AE Linear Wipe | trivial | `{0}` |
| 3.47 | Radial wipe | AE Radial Wipe | cheap | `{0}` |

### 3.1 Flow engine — optical-flow retime interpolation (Twixtor-class)

**K-088: not an effect.** Everything below stands as the engine specification, but flow is
surfaced as a **layer option**: a toggle in the footage layer's switch cluster, a **Flow**
group beside Transform and Effects carrying these parameters, engaging only when the
footage's rate (through any retime) undershoots the composition's — when a source frame
would otherwise hold across two or more comp frames.

**Input rate (conform, K-095; keyframeable, K-160).** The Flow group carries an **Input
rate** control: the fps the clip is *interpreted* at for flow. It is a keyframeable value the
user types any rate into (a numeric field with the usual stopwatch and keyframe navigator,
not a preset dropdown), so the conform rate can ramp over the clip. `0` reads as Native (the
default) and interpolates between adjacent source frames; a positive rate below native
conforms the clip to that rate, so flow brackets the source frames spaced `1/rate` apart and
interpolates between those — the standard way to get real slow-motion out of high-framerate
footage (whose adjacent frames barely move). **Animation drawn on 2s or 3s is the mirror
case and wants the same control**: a 24 fps cut on 2s holds each drawing twice, so at the
native rate half the pairs bracket a drawing and its own duplicate (no motion) while the
rest carry the whole step, which judders. Conforming to the drawn rate — 12 for 2s, 8 for
3s — makes every bracket span two different drawings. Keyframeable because a cut's cadence
is not always constant. The rate is read at frame time (`FlowParams::
input_fps_at`) and keys the frame cache — the value it reads at each local time is hashed, so
the same source time synthesises from different frames under it — and applies identically in
preview and export.

Not a stack effect: the flow engine is the shared module behind the **flow** frame
interpolation mode of Retime ([04-RETIMING.md](04-RETIMING.md)) and the Motion blur effect
(§3.2). It is specified here because it is one engine with one quality bar.

**What it does.** Estimates **dense per-pixel motion vectors** (forward and backward)
between adjacent decoded source frames, then synthesises any intermediate time by
bidirectional warping with occlusion-aware blending. This is what makes extreme slow motion
(5–20% speed) look continuous instead of a slideshow.

**Algorithm sketch.**
1. Build image pyramids of frames A and B (luminance + gradient channels), typically 5–7
   levels down to ~1/64 area.
2. Coarse-to-fine **dense inverse search** (DIS, Kroeger et al. ECCV 2016): initialise each
   level from the upsampled coarser level, refine by inverse-search patch matching (8×8
   patches on a stride-4 grid, a few Newton steps each) then densify. Compute A→B and B→A
   fields. The exact structure — patch size, grid, occlusion and confidence — is pinned in
   [docs/impl/optical-flow.md](impl/optical-flow.md); DIS is the shipped v1 engine (K-169).
3. **Occlusion detection** by forward-backward consistency: where `flow_AB` followed by
   `flow_BA` fails to return within a threshold, the pixel is occluded in one frame.
4. Synthesis at fraction t: splat/warp A forward by `t·flow_AB` and B backward by
   `(1−t)·flow_BA`; blend `(1−t)/t` weighted; in occluded regions take only the frame in
   which the pixel is visible; inpaint the (rare) both-occluded holes from neighbours.
5. HUD/overlay guard: static-region detection (near-zero flow with high texture) biases
   those pixels toward pure blending, reducing the classic Twixtor HUD smearing.
   **Shipped (K-331)** as `lumit_flow::hud_weights`: per pixel, `stillness × texture`, where
   stillness tapers over 0.25–1.0 px of measured motion and texture over 0.02–0.08 of Sobel
   magnitude in encoded luma. The texture term takes a **3×3 max** of the gradient before the
   taper, not the gradient itself — a gradient is zero inside every stroke of a glyph and
   spikes only at its rim, so a per-pixel test guards a HUD's outlines and leaves its insides
   to smear, which is the artefact rather than the fix. The result is 3×3 box-blurred (as
   FX-19's confidence is, and for the same reason: a hard boundary between warped and unwarped
   is itself visible), and synthesis mixes each pixel back toward the plain blend by it.

**Parameters** (surfaced per clip / per layer as render-policy options, not a stack entry).
All of these ship as `FlowParams` fields (K-331); each is content in the frame-cache key,
because each changes the synthesised picture.

| Parameter | Range / type | Default | Notes |
|---|---|---|---|
| Flow resolution | Native / Half / Quarter | Native | The size flow is *measured* at. Independent of the preview quality tier (K-331) — see below |
| Vector detail | Low / Medium / High / Ultra | Medium | Pyramid depth + refinement iterations |
| Smoothness | 0–100 | 50 | Regularisation weight; high = fewer tears, gloopier. Scales the smoothing pass's flow-range sigma, 50 being the tuned default the analytic tests were fitted against |
| Occlusion handling | Blend / Visible-only | Visible-only | Blend trades ghosting for fewer holes |
| Fallback | enum | Blend | Behaviour where confidence is low: **blend** (crossfade) or **nearest** |
| HUD guard | bool | on | Step 5's static-region bias; off for footage with no overlay |
| Always | bool | off | Force flow past the engagement gate below |
| Input rate | fps, keyframeable | 0 (Auto) | The conform rate above (K-095, K-160). Shipped with cadence presets beside the field — Auto, On 2s (12), On 3s (8), On 4s (6), 24, 25, 30 — named for the cadence rather than the number, since an editor knows a cut is "on 2s" without doing 24 ÷ 2 |

**Flow resolution is not the preview resolution (K-331).** Flow used to be measured on
whatever the preview scale had shrunk the decode to, which made a draft scrub and an export
two different *measurements* rather than one measurement at two sizes. The working resolution
is now this parameter alone. Consequence, accepted deliberately: **a layer whose flow is live
decodes at native width even in draft preview**, since full-resolution flow cannot be measured
on a shrunk decode — draft stops being cheap on flow layers, and the resolution knob is how a
user buys the speed back. The rule is "whoever asks": a layer with a live flow-consuming
effect (§3.2, §3.12) decodes natively on the same grounds, which is also what lets the two
consumers share one measured field.

**Engagement gate (K-088, built in K-331).** Flow engages only where a source frame would
otherwise hold across two or more comp frames — i.e. where `|speed| · source_rate / comp_rate`
is under 1, the source rate being the conform rate when one is set. Outside that (100% or
faster, or a freeze) the policy degrades to Nearest and costs nothing. **Always** overrides.
The gate is evaluated in the render plan *and* in the frame-cache key, so a gated-off flow
layer keys like the Nearest it renders as rather than hashing a sub-frame position that
changes no pixels.

**Artefact behaviour.** Flow failure MUST degrade to blending, never to garbage: every
synthesised pixel carries a confidence value, and low-confidence pixels fall back per the
Fallback parameter. The Viewer offers a diagnostic channel view (motion vectors, occlusion
matte, confidence) so editors can see *why* a region tears and mask or retrim rather than
guess. Flow fields are cached per source-frame-pair (content-hashed, K-016) so scrubbing a
retimed clip does not recompute flow. CUDA MAY accelerate this node where present (K-014);
the WGSL path is the portable baseline and the CPU reference is the oracle for the flow
field itself (vector-field tolerance, then bit-tolerant synthesis).

### 3.2 Fast motion blur (flow) — synthesised motion blur (RSMB-class)

Labelled **Fast motion blur** in the UI (a single-pass per-pixel smear, distinct from the
whole-scene re-rendering **Motion blur** of §3.26). Game capture has zero natural motion blur;
this effect synthesises it from motion vectors.
Applied per layer or, most commonly, on an adjustment layer over the whole montage.

**Algorithm sketch.** Obtain per-pixel motion vectors for the current frame: from the flow
engine (§3.1, frames −1/+1, averaged and scaled), or — when the input is a transformed
layer with no source motion — analytically from the transform derivative (cheap, exact,
automatically used when the host detects the layer is a static source under animation).
Blur each pixel along its vector with a line integral: N samples along
`±vector · shutter/360 · 0.5`, weighted by a box or triangle shutter profile. Sample count
adapts to vector length (4–64), clamped by quality.

**Parameters.**

| Parameter | Range / type | Default |
|---|---|---|
| Shutter angle | 0–720° | 180° |
| Amount | 0–200% | 100% (scales vectors after shutter) |
| Vector source | Auto / Flow / Transform-only | Auto |
| Quality | Normal / High | Normal |

(K-390 dropped the *Draft* tier this table used to list: tap counts became adaptive,
so the cheap tier is what a short streak already costs and a third name would have
been a knob with nothing behind it.)

Interaction rule: layers already blurred by the engine's own transform motion blur
([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)) MUST NOT be double-blurred — Auto mode
detects engine motion blur upstream and contributes only source-motion blur on top.

**Status (v1 core, shipped).** The second temporal effect (after Echo), and the first
consumer of the §3.1 flow field. Its temporal window is `{0, 1}`: the flow engine measures
the per-pixel motion between the current source frame and the next, and the smear runs each
pixel along that vector. The field is computed **in the decode worker**, where both frames
already live as decoded RGBA (mirroring how the Flow retiming policy computes flow there),
and handed to the kernel as an `rgba32float` texture threaded exactly as Echo's
neighbour frames are (decode → draw → realise/export → the pass): `.xy` the flow vectors,
`.z` a per-pixel **confidence** in 0..1. Preview and export compute
it the same way — the same `to_gray` → `lumit_flow` forward/backward-flow call on the same
source frames — so they match (K-031); the exact f32 flow texture keeps the CPU/GPU oracle at
the cheap-class ≤ 2 fp16 ULP bound, the only rounding being the colour taps.

**Confidence, not a hard cut (FX-19).** A patch-based flow field is unreliable at occlusions and
motion boundaries; gating the blur on/off there leaves hard un-blurred cut regions. Instead
confidence — `lumit_flow::confidence`, a 0..1 forward–backward-consistency measure (1 where the
two flows agree, tapering to 0 where they disagree, an invalid patch fully suspect), 3×3
box-blurred so the falloff has no seam — is a **smooth** quantity throughout, never a switch.

**What confidence does with it changed in v2 (K-392).** v1 *shortened* the streak by
confidence, so an uncertain pixel came out sharp in the middle of a blurred frame and
confidence 0 was a bit-exact passthrough. v2 **steers** instead: an uncertain pixel borrows
its neighbourhood's motion (sampled bilinearly between 16 px tile centres, so the borrowed
direction is continuous and disagreeing neighbours cancel toward not blurring) rather than
collapsing to none. Zero blur therefore survives in exactly one place — where the
neighbourhood itself is still — which is the owner's stated rule; zero motion and zero
shutter remain bit-exact passthroughs (pinned by test), confidence 0 alone no longer is.
[docs/impl/optical-flow.md](impl/optical-flow.md) §4.7 carries the reconstruction.

The shipped parameter set
is **Shutter angle** (0–720°, default 180 — streak length is shutter ÷ 360 of the
inter-frame motion, so 180° is half of it, the film-standard look), **Samples** (the tap
**cap**, slider 8–32, hard 2–64: the kernel spends `ceil(‖v‖/2)` taps and no more than this,
so a short streak is cheap and both backends still integrate identically), **Quality**
(*Normal* | *High*, default Normal — High halves the tap spacing and bends each streak along
the field; it is the **only** method choice, one reconstruction adapting internally), a
**View** enum (*Rendered* | *Motion vectors* | *Confidence* | *Dominant motion*, default
Rendered — the diagnostic views output the flow colour-coded, the confidence as greyscale, or
the tile field an uncertain pixel borrows from, so a user can see what the smear follows, where
it is unsure, and what it does about it) and the host **Mix**. Blur length in pixels = motion
vector × (shutter ÷ 360); the gather alternates between the pixel's own direction and the
neighbourhood's dominant one, McGuire-weighted (cone + cone + cylinder), edges clamped so a
full-frame smear never darkens the border. Pinned simplifications, each stable when the rest of
§3.2 lands: **Vector source is Flow only** (Auto's transform-derivative path and the
engine-motion-blur double-blur guard follow) and **Amount** (the post-shutter vector scale) is
deferred. Without a depth buffer the reconstruction cannot tell a fast
foreground from a fast background, so a *small* static object entirely surrounded by fast
motion receives its neighbours' smear (large static regions do not — the reach is a tile or
two plus the streak); the fix, if ever wanted, is a depth input rather than a constant.

**On an adjustment layer and on a Precomp, the picture measures itself** (K-565). The
sentence above — *most commonly, on an adjustment layer over the whole montage* — was for a
long time the one placement where the effect did nothing at all, and a Precomp layer was the
same: the decode worker measures between decoded source frames, and neither of those kinds
has any. Both do have a picture that can be **built again at another moment**, which is what
they now do. An adjustment's below-stack is rebuilt at `t + offset·dt` through the same
`below_draws_at` Posterize Time (§3.25) and accumulation motion blur (§3.26) drive; a
Precomp's nested comp is rebuilt at the neighbour layer time. The realiser renders the
neighbour beside the frame-time picture and measures between the two **on the card** — the
flow engine takes an RGBA texture directly, converting it to the luma its pyramid starts
from in one compute pass ([impl/optical-flow.md](impl/optical-flow.md) §1), because reading
two composites back to make CPU greys costs several times what the measurement does. The
offset steps by the **comp's own frame**, which is the shutter the effect smears over, and
the working resolution is the half-res the decode worker measures an effect's motion at, so
a composite is measured the way footage is rather than by a second rule. Datamosh (§3.12)
goes through the identical machinery for its own `-1` measurement; K-544's contract holds
unchanged, because the builder emits **one neighbour render per offset the stack asked for**
and neither consumer can take the other's field.

Two boundaries, both inherited and both deliberate. **Cost**: each measurement is a second
render of the below-stack (or of the nested comp) plus one flow pair, taken on demand — a
layer with neither effect builds nothing and never reaches the flow engine — and the whole
of it degrades to the old passthrough when the device has no GPU flow. **Footage beneath is
held**: a re-render re-decodes nothing (the temporal-re-render trap,
[impl/temporal-rerender.md](impl/temporal-rerender.md)), so the neighbour composite carries
the *same* decoded frames as this one. What is measured is therefore comp-driven motion —
transforms, effects, cameras, nested animation — and not the sub-frame motion inside footage
playing back beneath the adjustment, which is measured by putting the effect on the footage
layer itself. Sequence-clip temporal effects remain deferred exactly as they are for Echo.

**Its matte scales Shutter angle per pixel** (K-429, §2.6), read at the destination pixel
and spent everywhere the shutter is spent — this pixel's own vector, the neighbourhood's
dominant sweep, and each tap's reach — so a grey matte is a genuinely shorter streak
rather than a long one faded back over a sharp picture.

**A Motion vectors layer may stand in for the measured flow** (K-429). A **Motion vectors**
Layer row names a layer whose **red and green channels are the per-pixel motion**, in the
encoding every engine's velocity pass and every renderer's vector pass already uses: red is
sideways, green is up-and-down, and **mid-grey (0.5) is standing still**, so the motion in
pixels is `(r − ½) · Vector scale` across and `(g − ½) · Vector scale` down. Blue and alpha
are not read, and confidence comes out at 1 everywhere — a supplied vector is not a
measurement that can have failed to match. **Vector scale** (px@comp, default 32, greyed
until a layer is picked) is what makes one engine's normalisation agree with the frame it
came from; different passes normalise differently, and this is the dial that reconciles
them. The layer is an ordinary auxiliary layer input on the K-387 carriage (§1.2, K-123),
so a matte may be given as well; unset is the labelled no-op, and the measured flow is
used. Bound, it is also the one way this effect works on a layer that has no measured
flow at all — a solid, a shape, a comp — because the field no longer has to be measured
from two decoded frames. The conversion is one pass, `cpu::motion_vectors_field` and its
WGSL twin, and everything downstream reads one kind of field and knows nothing about where
it came from.

### 3.3 Glow — exposure-aware bloom (Deep Glow-class)

**Why scene-linear matters.** Stock-AE-style glow looks grey because it thresholds and
blurs display-referred pixels. Lumit's glow operates on scene-linear energy: bright pixels
bloom proportionally to how far above threshold they are, and additive recombination cannot
band or clip prematurely.

**Algorithm sketch.**
1. Threshold pass: `max(0, colour − threshold)` with a soft knee (smoothstep over
   `knee` width), in linear light, premultiplied input taken directly.
2. Progressive downsample chain (13-tap Karis-average filter to kill fireflies), typically
   7–9 mips.
3. Progressive upsample with per-level weights following the **Falloff** exponent —
   physically-plausible inverse-power falloff rather than one gaussian radius.
4. Optional **chromatic aberration**: per-level RGB scale offsets during upsampling,
   spreading long-radius bloom slightly by wavelength.
5. Recombine: `input + intensity · bloom` (Add), or Screen for an SDR-safe variant.

**Parameters.**

| Parameter | Range / type | Default |
|---|---|---|
| Threshold | 0–4 (linear value), hard min 0, unbounded above | 0.8 |
| Softness (id `knee`) | 0–1 | 0.5 |
| Radius | px@comp, hard min 0, unbounded above | 24 px |
| Falloff | 0.5–4.0 | 1.0 |
| Intensity | 0–10 | 1.0 |
| Chromatic aberration | 0–100% | 0 |
| Tint | colour | white |
| Recombine | Add / Screen | Add |

Cost class `moderate`; ROI `full-frame` (Radius is unbounded px@comp, so a %-diag padding
cannot bound it statically, K-135 — mirroring Chromatic aberration's own px@comp choice).
The mip chain makes large radii near-constant cost — the "radius 200 makes AE cry" failure
mode does not exist here.

**The Matte gates the seed (K-395, §2.6).** Glow is one of the four effects that claim the
matte inside their own maths: the input is multiplied by the matte's luma **before** step 1,
so only what the matte lights is allowed to bloom. The halo then spreads from those pixels
as light does — outward across the dark part of the matte, past its edge, over the parts of
the picture the matte excluded. Dissolving a finished glow instead clips the halo to the
matte's outline, which is why this is an override and not the generic semantic: a glow "on
the sign only" should still light the wall beside it.

**Status (v1 core, shipped; ranges revised K-135/FX-16):** the bright-pass → separable
gaussian → additive recombine spine, with Threshold (hard range clamped at zero below and
unbounded above — the K-090 one-sided shape; HDR values glow harder; **default 0.8** so a
fresh instance blooms highlights just shy of white), **Softness** (the soft-knee width — its
UI label was renamed from Knee for plainer language; the stable parameter id stays `knee`, so
saved projects and expressions are unaffected), **Radius** (now **px@comp** rather than %
diag, K-135: a real-pixel half-width scaled by the preview factor, clamped at zero below and
unbounded above so a wide bloom is a matter of typing a larger number, not hitting a cap —
default 24 px), Intensity, Tint and the host Mix. The knee is pinned as
`max(0, c − threshold) · smoothstep(threshold − knee, threshold + knee, c)` per channel. The
bright pass thresholds all four premultiplied channels alike, so the halo carries alpha and
glow spreads over transparency like light; output alpha saturates at 1. The internal gaussian
uses Repeat edges (fixed), so the halo holds its strength along frame borders. Intensity 0 is
the neutral point — a bit-exact passthrough, pinned by test. The progressive mip chain, and
with it Falloff, Chromatic aberration and the Screen recombine, replace the single gaussian
later; every shipped parameter is stable when they do.

### 3.4 Shake — parameterised camera shake (S_Shake-class)

Seeded-noise transform wobble, the beatshake workhorse. Implemented as a transform-domain
effect: it perturbs a virtual camera (translation, rotation, and a per-axis x/y/z wobble
where z is a depth/scale shake) and resamples the layer once — not a pixel-noise effect.

**Algorithm sketch.** Three independent 1D fractal noise generators (fBm over seeded value
noise, 2–4 octaves) drive x, y (as px@comp) and rotation (degrees), sampled at
`time · frequency`. A style preset sets octave count, lacunarity, and a per-axis frequency
multiplier. **Trigger mode** gates the noise with an envelope: on each trigger (beat marker
via §1.4, or manual keyframe on the Trigger parameter) the envelope jumps to 1 and decays
exponentially over Decay seconds, so shakes hit on the beat and settle.

**Parameters.**

| Parameter | Range / type | Default |
|---|---|---|
| Style | Subtle / Normal / Twitchy / Jumpy | Normal |
| Amplitude | 0–400 px@comp | 30 px@comp |
| Frequency | 0.1–30 Hz | 8 Hz |
| Rotation amount | 0–45° | 1° |
| Rotation frequency | ×0–4 | ×1 |
| *Per-axis wobble* (twirl) | | |
| — X amount / X frequency | ×0–2 / ×0–4 | ×1 / ×1 |
| — Y amount / Y frequency | ×0–2 / ×0–4 | ×1 / ×1 |
| — Z amount / Z frequency | 0–20% / ×0–4 | 0 / ×1 |
| *Motion blur* (twirl) | | |
| — Motion blur | boolean | off |
| — Shutter | 0–1 | 0.5 |
| Edges | Transparent / Repeat / Mirror | Mirror |
| Mode | Continuous / Triggered | Continuous |
| Trigger source | marker-trigger | comp beat markers |
| Decay | 0.05–2 s | 0.35 s |
| Seed | seed | per-instance |

The master Amplitude and Frequency drive the overall translational sway; the **Per-axis
wobble** twirl (K-146) biases each axis and adds depth. **Rotation frequency** (K-541) is the
twist's own rate multiplier, beside its amount, so a slow sway can carry a fast shudder;
×1 is the master rate the twist had before the row existed. X and Y amount/frequency are
dimensionless multipliers on the master values (×1 reproduces the plain uniform shake); Z
is the depth/scale shake — Z amount is a scale-pump per cent (the old Zoom pump, same
range), Z frequency a rate multiplier. **Edges** (K-145, the reusable control) governs the
border the resample reveals: Transparent leaves it clear, Repeat holds the edge pixel,
Mirror reflects. The **Motion blur** twirl (T18/K-165) adds the shake's own motion blur:
the wobble is a pure function of time, so with the toggle on it is sampled at several
sub-frame placements across the shutter and the resamples are averaged — translation,
rotation and zoom smear together, along the shake's own inter-frame movement, and only this
effect's output is affected (not the layer or comp motion blur). The Shutter (0–1) sets how
far across the shutter window the samples spread; off, or Shutter 0, is the plain single
resample. This is the streak the S_Shake feature wiggle expressions never had.

**Status (v1, continuous form, shipped):** Amplitude, Frequency, Rotation amount and
Rotation frequency, the
Per-axis wobble twirl (X/Y/Z amount and frequency), the Motion blur twirl (T18/K-165), an
Edges control (Transparent / Repeat / Mirror, default Mirror — pass 5 owner feedback: the
reflected border reads most natural under a shake) and Seed (per-instance
default, with reseed). The generator is pinned as two octaves of seeded value noise
(lacunarity 2, gain 0.5, smoothstep-interpolated, one independent channel per axis) sampled
at local time × frequency — deterministic and hop-free per §2.4. Resolved host-side into an
affine and dispatched through the §3.5 Transform kernel (which now carries the Edges
policy): no kernel of its own, and the zero-wobble state is a bit-exact passthrough (pinned
by test). **Motion blur (T18/K-165):** with the twirl on, the resolver samples the wobble at
a fixed, odd number of sub-frame placements (host-side — the noise lattice needs 64-bit
integers the GPU has not got) and a dedicated averaging kernel resamples the input through
each and takes the premultiplied-linear mean; off, or Shutter 0, is the exact single
resample. The shutter window is measured in the shake's own phase (local time × frequency),
not seconds, so the effect resolver stays frame-rate-agnostic and the smear is frame-rate
independent (see K-165). **Migration (FX-11/K-146):** this reshape replaced the old Zoom
pump and Auto-scale bool — a project saved before it maps its Zoom pump to the Z amount, and
its Auto-scale to the Edges control (on → Repeat, which hides the border as the cover scale
once did; off → Transparent). The Auto-scale cover (which zoomed in to keep every corner
covered) is gone; the Edges control handles the revealed border instead. Style presets and
Triggered mode (§1.4) follow; shipped parameters are stable when they do.

**The Matte scales the displacement (K-427, §2.6):** the shove, the twist and the zoom the
wobble gives a pixel are all pulled toward none by the matte at the pixel they land on, so
a half-grey matte on a 6 px shove is the 3 px shove and a soft matte turns a frame-wide
shove into a **warp** — one part of the picture moving while another stays put, which no
dissolve of a shaken frame can be. The motion-blurred tier scales every sub-frame tap the
same way. The Transform effect (§3.5) shares this kernel and does **not** claim it: it
keeps the strength dissolve.

### 3.5 Transform — the transform properties as an effect (K-090)

Position, Anchor, Scale, Rotation, Opacity — the layer transform group, as a stack entry.
Its point is adjustment layers: applied there, it transforms the composite of everything
below, which is the montage punch-in/whip gesture without touching per-layer transforms.
Parameters mirror the transform group exactly (same names, units, animatability); an
additional Skew pair arrives post-v1. Cost `trivial`, ROI `exact` under pure translation
and `full-frame` otherwise, `{0}` temporal.

### 3.6 RGB split

**Quality (K-090; picker-driven A1/K-163):** a `Wavelength` Bool (default off) switches from
the three-tap split to a smooth dispersion (more samples across the offset span, recombined in
linear) for the higher-quality look. The three-colour picker drives it: each spectral tap is
tinted by the picker sampled as a gradient (Colour 1 → Colour 2 → Colour 3), so the default
red / green / blue gives a red→green→blue fringe and any other colours re-tint it. Parameters
are shared between modes.

**Parameters:** Amount (0–200 px@comp, hard 0–500, default 8), Angle (degrees), Red / Green / Blue per-tap
amounts (%), Colour 1 / 2 / 3 (the three tap tints), Wavelength (Bool), Samples (Wavelength
mode), Mix. **Linear only (K-161, T17):** RGB split has no Radial mode — the always-radial
fringe is §3.15 Chromatic aberration's job.

**Algorithm sketch.** Three tinted taps along the Angle offset: taps 1 and 2 sample behind the
offset, tap 3 ahead, each read in **full colour**, multiplied by its tint, and summed. With the
default red / green / blue tints each tap keeps only its own channel, giving the classic
R-behind / B-ahead / G-anchored split bit-for-bit; any other colours cross-tint the fringe.
Operates premultiplied; alpha stays put to avoid fringed mattes. Trivially animatable Amount is
the scene's impact-frame staple.

**Per-tap amounts (FX-9, K-143):** three per-cent scales — **Red**, **Green**, **Blue**
(defaults 100 / 0 / 100, open both sides, K-135) — multiply the overall Amount per tap, so the
taps can fringe by different amounts and the middle tap be nudged off its anchor. Taps 1 and 2
displace along −offset, tap 3 along +offset, so the 100 / 0 / 100 defaults (with the default
tints) reproduce the classic split bit-for-bit. They apply to the classic (non-Wavelength) mode
only.

**Tap tints (K-161, T17; K-163):** the same reusable three-colour picker §3.15 chromatic
aberration carries (`channel_colour_1/2/3`, default red / green / blue) tints the taps. In the
classic mode the primaries reduce to the historical channel-separated split; other colours
produce coloured fringes (a cyan/magenta split, a warm/cool split, and so on). The picker drives
the **Wavelength** mode too (A1/K-163): it defines the dispersion gradient there.
**Normalisation (K-167, T17):** the classic mode normalises the three tints per output channel
(each channel's column of tap weights is rescaled to sum to 1, host-side in
`lumit_core::fx::normalise_tint_columns`, shared by CPU and GPU) — the same rule the
Wavelength gradient already applied. So custom tints recolour only where the taps *disagree*
(the misaligned fringe); uniform or aligned regions pass through at their original exposure,
and the default primaries are untouched bit-for-bit (columns already sum to 1).

**Wavelength samples (FX-9, K-144):** the Wavelength mode carries a **Samples** control (the
tap count, `3..=64`, default 16). More taps fill the same `±offset` span more densely, so a
large offset disperses as a smooth fringe instead of a few discrete stacked copies. Each tap's
colour is the three-colour picker sampled as a gradient at its offset fraction (A1/K-163:
Colour 1 at the −offset end, Colour 2 at centre, Colour 3 at the +offset end); the colour
columns are normalised across the taps host-side and shared by the CPU reference and the WGSL
kernel, so a uniform image still passes through unchanged (the fringe is tinted, not the
exposure) and preview equals export (K-031).

**§3.15 Chromatic aberration** is the always-radial sibling shipped alongside this effect: the
same three-tinted-tap idea, but growing radially from the frame centre rather than along an
angle. It carries the same three-colour channel picker and the same Wavelength/Samples
dispersion (K-144) — see §3.15.

**The Matte scales Amount on both (K-427, §2.6):** the offset vector the three taps spread
across is multiplied by the matte at the pixel it is read for, in both the classic tier and
the Wavelength one, so the fringe is genuinely narrower where the matte is grey rather than
a wide fringe faded back. A black matte returns the pixel untouched.

### 3.7 Flash — beat-aware strobe

**Parameters:** Mode (Trigger / Strobe), Trigger source (marker-trigger), Duration
(frames, default 2), Attack/decay shape (Hard / Fade), Colour (default white), Intensity
(0–4, additive in linear), Blend (Add / Screen / Solid), Every Nth beat (strobe mode,
integer), Phase offset (frames).

**Algorithm sketch.** Computes a scalar envelope from trigger times (§1.4) or the strobe
grid, then composites the flash colour over/into the input by the envelope. `trivial`
cost, `exact` ROI. Ships with the "white flash on every kick" preset that is half the
genre.

**Status (v1, shipped):** Mode (Manual / Trigger / Strobe) — Manual is the pre-marker
manual form (keyframed hits on Trigger decaying exponentially over Decay) and the
default, so existing instances and old projects render byte-identically — plus Duration
(frames, default 2; hard floor 0, unbounded above per K-090), Attack/decay shape
(Hard / Fade), Every Nth beat (Strobe; the spec's integer ≥ 1, carried as a rounded
float row for now) and Phase offset (frames). The envelope is pinned host-side in one
shared function: from the nearest trigger at/before the frame — every Nth beat of the
§1.4 context, phase-shifted — Hard holds full strength while elapsed < Duration and
Fade ramps `1 − elapsed/Duration` over the same span; with no markers it is zero, the
§1.4 graceful fallback. It reaches the unchanged kernel as the resolved strength.
Trigger source is implicitly the comp's beat markers (the §1.4 v1 scope); the
marker-trigger parameter type surfaces when named-layer binding lands. The Blend
sub-param (Add / Screen / Solid) is deferred — the kernel keeps its current
blend-toward-colour compositing — and Intensity stays the shipped percentage scale on
the envelope. Shipped parameters are stable when these follow.

### 3.8 Blur — Gaussian, Directional, Radial (three effects)

**Three single-purpose effects (K-137).** This began as one mode-driven "Blur" effect;
K-137 split it into **Gaussian blur**, **Directional blur** and **Radial blur** — one job per
effect (K-090), each in the **Blur & sharpen** category. The maths, kernels and CPU oracles
are unchanged by the split; only the schema and the resolve arms that read it changed. All
three are premultiplied (blurring unpremultiplied colour bleeds haloes) and declare `per-tile`
cancellation.

- **Gaussian blur** (match_name `blur`): Radius (px@comp, default 30, slider 0–500, hard
  0–2000). Separable two-pass; large radii switch to mip-assisted sampling. ROI
  `padded(radius)`. **Keeps match_name `blur`, so a project saved with the old combined effect
  loads here as Gaussian at its stored Radius, byte-identically** — whatever mode it had saved,
  its now-unread mode/length/centre parameters are simply ignored.
- **Directional blur** (match_name `directional_blur`): Length (px@comp, default 200, slider
  0–2000, **hard-unbounded above** per K-090) and Angle. Line-integral sampling along the
  angle. Length may exceed the frame, since it is its own effect rather than
  sharing the family's reach; the tap count still clamps (`cpu::dir_blur_taps`), so a long
  streak stays bounded in cost. ROI `full-frame` (an unbounded Length cannot be padded
  statically).
- **Radial blur** (match_name `radial_blur`): Centre X / Centre Y (**px@comp**, K-558, default
  960/540 and centred on the actual comp by `instantiate_for_raster` — the schema has no
  Point-shaped `ParamKind`, so this follows Transform's own `anchor_x`/`anchor_y` split),
  Amount (px@comp, default 150, slider 0–2000, hard-unbounded above),
  Type (Spin / Zoom, default Spin) and **Edges** (Transparent / Repeat / Mirror). Amount is the
  peak per-pixel tap spread, reached at the frame's farthest corner from Centre, and may exceed
  100 % now it is its own effect (the tap count clamps in `cpu::radial_blur_taps`). Both types
  reduce to one linear scale of the pixel's own (position − centre) vector — Zoom along that
  vector (an exact ray sample), Spin along its perpendicular (the first-order/tangent
  approximation to the true arc about Centre) — so neither needs a division or a runtime trig
  call: the one scale factor (Amount ÷ half the raster diagonal) is a plain host-side division,
  not a per-pixel or per-tap one, and every tap collapses to exactly the pixel itself at Centre
  with no epsilon guard. The tangent approximation is exact for Zoom and close for Spin across
  the useful Amount range; the oracle holds to ≤ 2 fp16 ULP (measured worst 1 ULP). Amount 0 is
  a bit-exact passthrough (pinned by test, mirroring Directional's zero-length case).

**The Matte scales Gaussian blur's radius (K-395, §2.6).** Gaussian blur is one of the four
effects that claim the matte inside their own maths: each pixel's matte luma multiplies
Radius before the kernel is built, so white blurs at the full Radius, mid-grey at half of
it, and black not at all. Both separable passes read the *destination* pixel's matte, so the
two halves agree on that pixel's kernel width. This is a different picture from dissolving a
full-width blur, and the difference is the point: a dissolve leaves every pixel gathered
from the full radius away and merely fades that back, so it reads as a veil over a sharp
image, where a radius ramp reads as a lens racking focus. **Directional and Radial claim it
the same way (K-426):** the matte scales Directional blur's Length and Radial blur's Amount
per pixel — the same host-computed taps, packed closer — so the streak or the sweep is
genuinely shorter where the matte is grey, not a long one faded back.

**Edges (K-137).** The old effect carried one shared Transparent / Repeat / Mirror control
across every mode. The split keeps that control **only on Radial** (the sweep most often wants
Mirror or Transparent); **Gaussian and Directional resolve at the old default, Repeat**
(full-frame game footage never darkens along the border), so their look is unchanged. Radial's
taps run through the same edge-policy bilinear sampler the others use, so it clamps, mirrors or
clears exactly like them.

### 3.9 Sharpen — Unsharp mask and plain Sharpen (two effects)

**Two effects (K-138).** The original §3.9 effect was really an unsharp mask; K-138 renamed
its **label** to **Unsharp mask** (match_name stays `sharpen`, so saved projects are
unchanged) and added a separate plain **Sharpen**. Both are in the **Blur & sharpen** category
and run in linear light on unpremultiplied colour (§2.2).

- **Unsharp mask** (match_name `sharpen`): Amount (0–300 %), Radius (1–50 px@comp, hard 0–100, default 8), Threshold
  (0–1, suppresses noise amplification), and a luminance-only option (avoids chroma fringing on
  compressed game capture). Algorithm: `input + amount · (input − gaussian(input, radius))`
  gated by threshold — a radius-controlled detail lift.
- **Sharpen** (match_name `sharpen_simple`, K-138): the plain sibling — a high-pass
  convolution scaled by **Amount** (default 1 = the classic 5/−1 kernel, slider 0–5,
  hard-clamped ≥ 0) with an adjustable **Radius** (T15; the neighbour distance in pixels,
  default 1 = a 3×3 kernel, slider 1–8, host-rounded to a whole pixel). `out = u + amount ·
  (4·u − up − down − left − right)` per RGB channel, the four axis neighbours taken `radius`
  pixels out and clamp-addressed (so a border never invents dark detail); the result clamps
  ≥ 0, re-premultiplies by the centre alpha, and keeps alpha. Amount 0 (whatever the Mix) and
  Mix 0 are the bit-exact passthrough. Cheap; the honest "just sharpen it" control beside the
  Unsharp mask's knobs.

**The Matte scales Amount on both (K-426, §2.6):** less detail is added back where the matte
is grey, which differs from fading a finished sharpen wherever the full Amount undershot
past zero and was clipped.

### 3.10 The colour effects — Colour balance, Saturation, and the preset browser (Magic Bullet-class)

The "CC" engine, as single-purpose effects (K-090: the v1 all-in-one Grade split; an
all-in-one grading suite MAY return later as the deliberate exception). Each is `cheap`,
pointwise, unpremultiplied (§2.2), all parameters animatable, neutral by default (a
grade's tasteful default is a preset choice — see the browser below):

- **Colour balance** — **lift / gamma / gain** per channel (per-master and per-channel
  trackballs, UI: [07-UI-SPEC.md](07-UI-SPEC.md) colour workspace). Applied in linear
  (gain), with gamma on a display-referred intermediate for familiar feel, documented
  precisely in the implementation notes.
- **Saturation** (per cent about Rec. 709 luma in linear light; 0 = greyscale, 100 = neutral,
  200 = doubled) — the hard ceiling is **open** (K-135): the luma/colour mix keeps
  extrapolating past 200, so the slider reaches a heavy 400 and typing higher pushes further.
- **Vibrancy** (v1, shipped, K-152) — a saturation boost *weighted by each pixel's current
  colourfulness*: the per-pixel factor is `1 + amount·(1 − sat)`, where `sat = (max − min)/max`
  is the scale-invariant HSV saturation (clamped 0..1), so less-saturated pixels lift more and
  already-vivid ones little — skin tones and near-neutrals gain while saturated areas are
  protected from clipping, unlike Saturation's uniform scale. One **Amount** dial (per cent):
  0 is the neutral, bit-exact identity; the slider reaches a heavy 200 and typing higher pushes
  further (open ceiling, K-135, floored at 0). Same domain as Saturation — linear light,
  unpremultiplied (§2.2), re-premultiplied, colour scaled about Rec. 709 luma and clamped at
  zero. `cheap` cost, `Exact` ROI; the §1.6 CPU/GPU oracle holds to ≤ 2 fp16 ULP, and the
  neutral is the bit-exact identity on both paths.

**Vignette** (§3.14, shipped) is one of these single-purpose colour effects, because every CC
pack has one. The remaining "CC" stages arrive the same way: **exposure / white balance**
(stops; Temperature via Bradford-adapted CCT shift; Tint). **Curves** has landed as §3.30 —
master + R/G/B + alpha, each a real point list with a clamped cubic through it, which is the
point list sketched here (K-412; K-396's five fixed knots a channel were the floor it grew
from).

**Preset browser.** Colour presets get a dedicated browser (per
[07-UI-SPEC.md](07-UI-SPEC.md)): a panel of live thumbnails, each preset applied to the
frame under the playhead, Magic Bullet Looks-style. Thumbnails are rendered by the normal
engine at thumbnail resolution through the real effect — never approximations. Ships with
≥ 40 presets across the genre families (clean/bright, teal-orange, moody desat, anime
vibrance, VHS warm). Selecting a preset sets parameters; it never locks editing.

**The Matte scales the amount of each (K-426, §2.6):** Colour balance's Lift is pulled
toward 0 and its Gamma and Gain toward 1 per pixel before the grade runs; Saturation is
pulled toward 100; Vibrancy's Amount is scaled. A grey matte is a gentler grade, not a full
grade faded back — which differs wherever the full grade clipped, or (Colour balance) wherever
Gamma is not 1.

### 3.11 LUT — .cube loader

**Parameters:** File (file reference, `.cube` 1D and 3D, sizes to 65³), Input space
(Linear / sRGB / Rec. 709 — what the LUT expects), Interpolation (Trilinear /
Tetrahedral, default Tetrahedral), Mix.

**Algorithm sketch.** Host parses and uploads the LUT as a 3D texture at load, converts
working-space linear into the LUT's expected space, applies, converts back. Unpremultiplied.
Missing file behaviour: effect becomes a labelled no-op with a warning badge — never a
render failure ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) never-crash rule). The
file's content hash joins the cache key; project save embeds small LUTs (K-040) so shared
projects survive relinking.

**Status (v1, shipped, K-114; Input space K-543):** **File + Input space + Mix**. The File
parameter picks a `.cube`
cube (animatable by stepping between paths with hold keys — two files cannot be blended,
K-111) and Mix blends the graded result over the input. **3D trilinear** only (the manual
eight-corner interpolation of [docs/impl/lut.md](impl/lut.md) §2–3, matching the CPU oracle
`lut::Lut3d::sample_in` to ≤ 2 fp16 ULP; Tetrahedral is deferred). **Input space** (K-543) is
a three-option dropdown — **Linear** (default), **sRGB**, **Rec. 709** — naming the transfer
function the cube was authored against: the straight colour converts into it, the table
applies, and the result converts back to scene-linear, so a `.cube` baked in a
display-referred grading application lands in the cells its author was looking at. Linear is
the identity in both directions and is byte-for-byte the picture this effect rendered before
the row existed (K-258). There is no Log option: the pipeline defines no log transfer
function and §1's rules forbid inventing one (a follow-up, with OCIO).
Unpremultiplied (§2.2), and the transfer sits **inside** the unpremultiply/re-premultiply
pair — a transfer function is a statement about colour, not about coverage. An **unset,
missing, 1D or unreadable** file is a labelled no-op,
never a fault. GPU-only: the parsed cube is threaded beside the resolved op (like Echo's
neighbour frames and Motion blur's flow field), so the CPU-degradation rung renders a LUT as
identity — its §1.6 oracle reference is `lut::Lut3d::sample_in` used directly in the lumit-gpu
test, the one effect whose reference lives outside its own `EffectDef::apply_cpu` (its parameter
is a file, not a number). Preview and export load and apply it identically (K-031). **Follow-ups:**
Tetrahedral interpolation, log input spaces, the content-hash cache key (the cache is
path-only for now, so an edited-on-disk LUT needs the app reopened), and embedding small LUTs
in the project (K-040).

### 3.12 Glitch family — block glitch, scanlines, datamosh

Three separate effects, formerly shipped as one "Glitch" effect with enableable sections
(K-104). **Status (K-107):** split into one-thing effects per the §1's one-effect-one-job
rule (K-090 — the same rule that split the v1 Grade into Colour balance and Saturation, and
gave the radial fringe its own Chromatic aberration effect, later leaving RGB split
linear-only, K-161). Stacking **Block glitch** →
**Scanlines**, each at Mix 100%, reproduces the old combined Glitch's look bit-for-bit — the
two sections never interacted beyond running in the same pass. Existing saved `glitch`
instances do not migrate (pre-v1, single user, no alias); each of the three below is added
to a layer independently going forward. Category **Distortion** for all three, matching
Shake and RGB split — their closest siblings (a seeded positional wobble; a channel split)
— not the additive-light Stylise pair (Glow, Flash).

#### Block glitch

**Parameters:** Intensity (0–1, default 0.35, the master dial), Seed, Block size (px@comp,
default 24), Rows/columns jitter (% of Block size, default 25), Displacement (px@comp, 0..300,
hard 0..1000, default 60), Channel offset (px@comp, 0..200, hard 0..1000, default 20), Slice
repeat (%, default 20), Mix.

**Algorithm sketch.** The image is partitioned into a seeded grid (Block size, px@comp);
per *nominal* block, a hash decides a jitter offset (Rows/columns jitter, scaled by
Intensity) that picks *which* block's content a pixel actually reads from — a cheap
stand-in for moving grid lines themselves, which would need a boundary search a single
pointwise pass cannot do. That block then hashes its own displacement (Displacement,
px@comp), R/B channel split (Channel offset, px@comp, alpha follows green exactly like RGB
split, for the same matte-fringing reason), and slice-repeat odds (Slice repeat, scaled by
Intensity: folds the block's own local Y to a short hashed repeat height instead of a plain
read). Every hashed quantity is scaled by Intensity, so Intensity 0 is a genuine,
single-knob bit-exact passthrough, pinned by an explicit early return (the same shape as
Glow's neutral short-circuit, not the box-blur family's tap-sum coincidence) — holding
regardless of Mix. The per-block hash runs inside the GPU kernel itself, not as a
host-precomputed table (the block index is a per-pixel quantity — there are too many blocks
at a small Block size to fit a table into the shared uniform binding): WGSL has no 64-bit
integer type, so it cannot host Shake's actual splitmix64 lattice; `splitmix32`, a
matching-spirit 32-bit sibling, was added alongside it in `lumit-core` for exactly this, and
both the CPU reference and the WGSL kernel run it, so the integer hash agrees bit-for-bit
(measured oracle worst: 1 fp16 ULP, same as the other hash/tap-based kernels — no looser
bound was needed despite the `cheap` cost class default suggesting one might be).
"Time-derived tick" (per-frame block variation) steps at a fixed, unexposed 8 Hz, chosen so
blocks visibly pop rather than blur into continuous noise; the spec text lists no rate
parameter, so this is pinned as an internal constant, not a control. `cheap` cost,
`full-frame` ROI (a hashed displacement can read from anywhere in the block grid). Frame
keys: declares `seeded: true` exactly like Shake, so the existing §2.4 mechanism already
carries the layer's local time into its cache key with no effect-specific plumbing.

#### Scanlines

**Parameters:** Intensity (0–1, default 0.35), Line period (px@comp, default 3), Roll speed
(lines/s, default 0, either direction), Interlace offset (Bool, default off), Mix.

**Algorithm sketch.** A pointwise periodic darken in raster Y (plus the roll offset — roll
speed × time × period, host-computed so the kernel never sees raw time), alternating which
half of each period darkens on odd periods when Interlace offset is on — the classic
interlaced-field look. **Intensity is the single darken dial** (FX-13, K-147): 0..1 is *how
dark the dark lines get* — 0 the bit-exact passthrough, 1 takes them to black; the bright
half is untouched. This collapses the former Intensity × Darkness pair (which multiplied to
one darken amount) into one control; a project saved with the old pair folds losslessly on
load — the single Intensity resolves to the old Intensity × Darkness product. No hash, no
neighbour read: reads the input pixel directly, so ROI is `exact` (tighter than Block
glitch's `full-frame`) and there is no Seed parameter. Intensity 0 is the bit-exact
passthrough, pinned by the same early-return shape as Block glitch's. `cheap` cost. Not
seeded (`seeded: false`) — its pixels are a pure function of the frame's own position and the
host-computed roll offset, not a random-looking hash, so it needs no extra cache-key plumbing
beyond the ordinary parameter-animation case.

#### Datamosh

**Parameters:** Intensity (default 1, open above per K-135 — pass 5 owner feedback raised it
from 0.5 so the drop-on default is the full melt), Displacement (frames, default
4, hard min 1, open above per K-135), Bloom (0–1, default 0.6), Reset interval (seconds,
default 0 = off, hard min 0, open above), Mix.

**Algorithm sketch (K-164/T19).** Simulates the compression-glitch look of removing I-frames:
the previous picture keeps being dragged along the current frame's motion, so moving regions
smear and *bloom* while static ones stay. It is a *look*, not real bitstream corruption —
deterministic and safe. Reuses the §3.2 flow machinery Motion blur introduced (the shared
`FlowEngine`) rather than needing new plumbing; only the flow's `.xy` is read (the shared
field's `.z` confidence lane is left untouched).

Per output pixel a short **streamline walk** follows the current→previous flow field out of
the -1 source neighbour: starting at the pixel centre, each step re-samples the flow at its
current position (so the smear curves with the motion) and advances by roughly one frame of
that motion, then samples the -1 neighbour there. The samples accumulate into a melting
prediction, which blends over the current frame by **Intensity × Mix**. The three shaping
controls:

- **Displacement** (frames) — how far the walk reaches, i.e. how many frames of predicted
  motion it accumulates (the P-frame run length before a clean reference frame; longer = more
  melt). The tap count is derived from it (one tap ≈ one frame of motion, clamped 2–64), so it
  supersedes K-148's Streak length; an old project's `streak_length` is still read as the
  reach so its look is unchanged.
- **Bloom** (0–1) — how much of that reach accumulates into the smear. Near 0 only the nearest
  step survives (a short, quickly-resetting trail — close to the old single-tap prediction);
  near 1 the whole walk averages evenly (a long, melting bloom). It is the "accumulates vs
  resets" dial, weighting the taps geometrically (`bloom^k`) from the near end.
- **Reset interval** (seconds) — the simulated I-frame period. Off (0) the melt runs
  constantly; set, the whole melt ramps from a *clean frame* just after each reset up to full
  by the next — the accumulating-P-frame look, restarting on a fixed cadence. The ramp is a
  pure function of layer time (a sawtooth), computed in resolve and folded into Intensity and
  Displacement, so the kernel stays time-agnostic and the frame-cache key already covers it (a
  param+time function, the K-093/K-094 reasoning). It is in *seconds*, not frames, because the
  resolve step is frame-rate-agnostic; a frame-count interval would need the comp frame index
  threaded through resolve, the deferred broad change K-148 avoided. A **content-driven reset**
  also fires regardless: where the flow is zero or unmeasurable (a still, a cut) the walk does
  not move, so the picture holds — exactly where a real codec inserts an I-frame.

**Intensity's hard ceiling is open** (K-135): above 1 the blend extrapolates past the moshed
frame for a punchier tear (`mix()` does not clamp in either the CPU or GPU path); 0 stays the
bit-exact passthrough regardless of the other parameters (pinned by test).

With no -1 neighbour or flow field (a dropped decode, a layer nothing can measure) it
degrades to a no-op, never a fault. On an **adjustment layer** and on a **Precomp** the pair
is now measured from the picture itself, exactly as §3.2's is and through the same machinery
(K-565) — this effect needs both halves of it, since it drags the previous picture along the
field. Temporal window `{-1, 0}` — static, exactly the shape
Motion blur's own `{0, +1}` has, so `effect_flow_neighbour` reads the match name the same
static way. **A layer measures one flow field per consuming effect (K-543's successor K-544,
superseding K-104's one-per-layer rule):** Motion blur wants the forward measurement to `+1`
and Datamosh the backward one to `-1`, so there was never a single field both could read — a
stack with both now carries both, each op binding the field keyed by the offset its own
effect asked for. Before K-544 the first of the two in stack order took the layer's single
slot and the other silently rendered its missing-field passthrough. A stack with only one of
them measures exactly once, as it always did. `moderate` cost (a multi-tap streamline like Motion blur's
streak, plus a bilinear flow re-sample each step), `full-frame` ROI (the flow can point
anywhere in the frame, the same unbounded-read reasoning Motion blur's own ROI carries). Not
seeded (`seeded: false`) — no hash or random-looking sequence, just flow-directed sampling.
Oracle: GPU matches `lumit_core::fx::cpu::datamosh` at ≤ 2 fp16 ULP (measured worst 1 across
the bloom and step sweep).

**Status (K-104, its own effect since K-107, reworked to a flow-driven melt by K-164/T19):**
originally a single motion-compensated tap that warped the -1 neighbour by its own flow vector
(a "reused old motion" prediction), added first as a toggle (`datamosh_enabled`) inside the
combined Glitch effect and split out at K-107. T19 rebuilt it referencing the well-known
datamoshing technique (removing I-frames so P-frame motion keeps being applied to the wrong
picture) into the streamline-melt above, adding the Bloom accumulation dial and the periodic
Reset. The schema bumps version 2 → 3; pre-release, no migration is required (K-148's
`streak_length` is still read as the Displacement reach as a courtesy, so an existing instance
keeps its look). `temporal: {-1, 0}` remains the schema's static declaration and
`effect_flow_neighbour` reads the match name the same static way it reads Motion blur's.

**The Matte, on all three (K-427, §2.6).** **Block glitch** scales its **Intensity** per
pixel, before any hash is read, so the jitter, the displacement, the channel split and the
slice odds all shrink together where the matte darkens — a genuinely calmer glitch, not a
loud one faded back. **Scanlines** cannot take that route, because scaling its Intensity
*is* the generic dissolve to the bit; the matte **divides its Line period** instead, so the
lines spread apart as the matte darkens and vanish at black (the divide floors at
`cpu::SCANLINES_MIN_K`, a period ten thousand times the set one), and Intensity is left
alone. **Datamosh** keeps the strength dissolve, for the rule's own reason: its output is
`current·(1 − Intensity) + melted·Intensity`, so scaling Intensity per pixel and dissolving
per pixel are the same arithmetic, and there is nothing for a claim to add.

### 3.13 Echo — frame echo and trails (speed lines)

**Parameters:** Echo count (1–32), Spacing (frames, may be negative to echo forward),
Decay (per-echo opacity multiplier 0–1), Mode (Behind / In front, then the standard blend set —
see status), Transform per echo (optional scale/rotation/offset step for stylised speed-line
fans).

**Algorithm sketch.** Composites N prior layer frames (window `{-n·spacing .. 0}`,
resolved through Retime so slow-motion echoes stretch correctly), each transformed and
attenuated. Temporal window declared dynamically from Count × Spacing so the prefetcher
plans decode. `moderate` cost, `full-frame` ROI.

**v1 status (shipped; blend modes + 16-echo cap FX-17/K-149).** Echo is the first temporal
effect — the render decodes the layer's source at each offset in the stack's temporal window
(`fx::stack_temporal_window`) and hands them to the pass; the frame-cache key hashes those
neighbour frames too (K-094). Pinned simplifications for v1: **Echoes 1–16 at a fixed
one-frame spacing** (the trait's `temporal` window is `&'static`, so the maximum reach is a
fixed cap — raised from 8 to 16 by FX-17; a Spacing control and a larger/dynamic window are a
later refinement) and **intensity `Decay^k`** per echo `k`. **Mode** (T21) lists two
effect-only compositing *orders* first — **Behind** (each echo behind the trail, ghosting) and
**In front** (over it) — then a divider, then the order-independent light-combine blend modes:
**Add, Screen, Multiply, Overlay, Soft light, Hard light, Lighten, Darken, Difference,
Exclusion, Subtract, Divide**. The **default is Screen**. "Max" is gone (it was just Lighten)
and the old "Normal" is now the clearer "In front". Each mode folds the weighted echo tap into
the running trail per channel in the **working linear premultiplied space** (not the
compositor's perceptual sRGB domain — Echo composites light trails, so it stays linear, which
also keeps the CPU oracle and WGSL kernel bit-for-bit identical). The comparative modes
(Difference / Exclusion / Subtract) therefore act on the premultiplied alpha too, so equal-alpha
taps zero the tap's coverage — the honest per-channel result. The HSL and colour burn/dodge
modes a layer offers are deliberately **not** in this list, being ill-defined on a premultiplied
light trail (see Open questions). Pre-release, mode indices were renumbered with no migration.
It reads the layer's
**source** frames, not the upstream stack's output at those times (full temporal stacking is
later), and echoes footage layers only — Sequence-clip and adjustment-layer temporal effects
are deferred. Marker-triggerable intensity spikes come with the §1.4 wiring already in place.

**The Matte scales Decay (K-429, §2.6):** the trail dies away sooner where the matte is
dark and reaches its full length where it is white, so the ghosts are genuinely shorter
rather than faded back. Because `(decay·k)^(i+1)` factorises as `decay^(i+1) · k^(i+1)`, a
half matte draws *exactly* the half-decay trail; and a tap the matte has taken to nothing is
skipped rather than folded in at zero, since a zero-weight tap is not a no-op under every
combine mode.

### 3.14 Vignette

**Parameters:** Amount (0–1, default 0.5), Radius (0–1, default 0.75), Softness (hard min 0,
unbounded above — slider 0–2, default 0.5), Roundness (0–1, default 1.0), Ramp (hard min 0.05,
slider 0.2–4, default 1.0), Mix.

**Algorithm sketch.** Darkens toward black away from the frame centre: a normalised distance
metric (blended by Roundness between a true circle and an ellipse matching the frame's
aspect) feeds a smoothstep between Radius and Radius + Softness, raised to the Ramp gamma
(1.0 leaves the smoothstep unchanged; below 1 pushes the darkening outward, above 1 draws it
inward toward the corners), scaled by Amount and multiplied into the premultiplied colour;
alpha is untouched. `cheap` cost, `exact` ROI — a pointwise per-pixel darken, no neighbour
sampling despite the spatial falloff.

**Status (v1, shipped):** §3.10's one-line mention names Amount, Size, Softness, Roundness
without ranges or a parameter shape — pinned here as Amount / Radius / Softness / Roundness,
plain fractions in the normalised distance metric rather than the %-diag or percentage figures
most of the catalogue uses. Amount, Radius and Roundness keep a 0–1 cap; **Softness is open
above** (K-135): the metric itself is not capped at 1 (a corner reaches ~√2 under circular
roundness), so a Softness beyond 1 is a legitimately wider feather, and only the ceiling is
lifted — the floor stays 0. The schema's Radius plays the role §3.10's text calls Size,
renamed for clarity against
Blur's and Glow's own Radius, which shares their unit family instead. Category is **Colour**,
alongside Colour balance and Saturation — matching where §3.10's text already lists it, not
Stylise, despite the spatial falloff. Roundness blends the distance metric between a circle
(1: both axes normalised by the frame's shorter side, so equal pixel distances read as equal)
and an ellipse that exactly reaches every edge of the frame (0: each axis normalised by its
own half-extent); Radius and Softness are read against that same normalised metric, so — despite
governing a spatial falloff — neither needs a %-diag conversion the way Blur's Radius does: the
metric is already resolution-relative by construction, derived from the raster's own width and
height at kernel time. Amount 0 is the neutral point (bit-exact passthrough, pinned by test,
mirroring Glow's own Intensity-0 short-circuit). A Colour param tinting the vignette away from
black is deferred — v1 always darkens toward black, the near-universal case; array literals for
such a default remain data, not the banned hex-literal shape (docs/15 §4's no-hex-outside-theme
rule only reaches `Color32`/hex-literal colours in widget code).

### 3.15 Chromatic aberration

**Parameters:** Amount (px@comp, default 4, open above per K-135), the three channel colours
(Colour 1 / 2 / 3, default red / green / blue), Wavelength (Bool, default off), Samples
(3–64, default 16), Mix.

**Algorithm sketch.** Three radial taps at offset fractions −1 / 0 / +1 from the frame centre
(toward centre / on the pixel / away), each sampled and multiplied component-wise by its
channel colour and summed; G and alpha stay put. Default tints red / green / blue keep only
their own channel, so R reads outward, B inward and G on its own pixel — the classic split.
Custom tints are normalised per output channel exactly as §3.6's classic mode is (K-167,
`normalise_tint_columns`): fringes recolour, aligned regions keep their exposure, and the
default primaries stay bit-exact. Premultiplied throughout, edges clamp. `cheap` cost,
`full-frame` ROI.

**Channel picker (P2, K-143):** the three tap colours are edited through the **reusable
three-colour channel picker** — three colour swatches (defaults red / green / blue), each
opening the colour picker. The widget is shared: any effect whose schema declares three
Colour parameters `channel_colour_1/2/3` gets it automatically (see `channel_picker` in the
inspector) — §3.6 RGB split now does too (K-161), and any future three-tinted-channel effect
adopts it without new UI code.

**Wavelength (K-144; picker-driven A1/K-163):** a `Wavelength` Bool (default off) reuses §3.6
RGB split's own spectral machinery — turning on resolves the effect to a radial spectral split
with a **Samples** control (3–64, default 16), the same many-tap dispersion RGB split's
Wavelength mode uses, for a smooth fringe rather than the three discrete tinted taps. The
channel colours drive both modes: in Wavelength mode they define the dispersion gradient
(Colour 1 → Colour 2 → Colour 3 across the offset span), so the default red / green / blue gives
a red→green→blue fringe.

**Status (v1, shipped):** the always-radial sibling of §3.6 RGB split (K-161, T17). RGB split
is linear-only — three tinted taps along an Angle — and this effect is the same three-tinted-tap
idea grown radially from the frame centre instead. It exists as a single-purpose, one-click
version: drop it on and it already looks right (§1.2), the same
shape rule that split the old Grade into Colour balance and Saturation (K-090). Because it has
no Angle to share a currency with, Amount is authored in px@comp (§2.3) —
scaled by the preview factor exactly like Block glitch's Block size (§3.12) — and its ROI is
declared `full-frame` rather than a tight %-diag padding, since a fixed pixel offset cannot be
bounded as a percentage of the diagonal across every comp resolution ahead of time. Category
is **Distortion**, matching RGB split. No explicit Amount-0 short circuit is needed in either
the CPU reference or the WGSL kernel: the radial offset's scale factor is an exact `0.0` at
Amount 0, so every tap collapses onto its own pixel and the tinted sum returns the input for
the primary defaults — the same un-guarded style RGB split's own kernel uses (asserted
bit-exact by test).

**The Matte scales Amount (K-427, §2.6),** in both tiers, exactly as §3.6's does: the
radial offset is multiplied by the matte at the pixel it is read for, so the fringe narrows
toward the frame's own colours where the matte darkens.

### 3.16 Exposure

**Parameters:** Stops (photographic stops, default 0, slider −5..+5, unbounded), Mix.

**Algorithm sketch.** A single scene-linear gain on RGB: `factor = 2^Stops` is computed
host-side (in the resolve step) so the CPU reference and the WGSL kernel multiply by the
identical number — no `exp2` runs per pixel or per path. Premultiplied throughout: a scalar
scales premultiplied colour consistently (straight × factor, then × the unchanged alpha), so
there is no unpremultiply round trip and alpha is untouched. `cheap` cost, `Exact` ROI.

**Status (v1, shipped, K-106):** the montage grade's brightness lever, beside Colour balance
and Saturation in the **Colour** category. Continuous (unlike a quantiser), so the §1.6 oracle
holds to ≤ 2 fp16 ULP. 0 stops (`factor` 1.0) short-circuits to the input on both paths (the
bit-exact neutral point, pinned by test); Mix 0 is likewise the identity. Distinct from Colour
balance's three-channel Gain: a single, animatable, photographic-stops control — the common
one-knob exposure move.

**The Matte scales Stops toward 0 (K-426, §2.6):** the gain under a matte of strength k is
`2^(stops·k)`, so a half-grey matte on +2 stops is +1 stop, not a blend of +2 and none.

### 3.17 Hue shift

**Parameters:** Angle (degrees, default 0, slider −180..+180, wraps), Preserve luminance
(bool, default on), Mix.

**Algorithm sketch.** A hue rotation built from the standard SVG `feColorMatrix` hue-rotate
construction, in one of two modes chosen by **Preserve luminance** (K-136):

- **On (default)** — the weights are Rec.709 luma, so it is a **constant-luminance** rotation:
  perceived brightness stays put as the hue turns (a saturated green stays as bright, a blue
  as dark). This is the historical behaviour; a project saved before the toggle existed reads
  it as on.
- **Off** — the weights are equal (⅓, ⅓, ⅓), a plain **geometric spin about the grey axis**:
  it preserves the raw R+G+B sum rather than perceived luminance, so brightness is free to
  ride with the hue (the way a naïve RGB hue wheel behaves).

Either way the result is a row-major 3×3 colour matrix computed host-side
(`lumit_core::fx::hue_matrix` / `hue_matrix_rgb` — the bool only picks the weights), so the
CPU reference and the WGSL kernel multiply by identical coefficients and preview equals export
(K-031); the kernel is matrix-general and unchanged. The nine coefficients travel as
individual `f32` uniform fields (tight 4-byte packing, matching the Rust `[f32; 9]` — a
uniform array would stride at 16). Premultiplied throughout: a linear matrix scales through
alpha, so no unpremultiply round trip and alpha is untouched. `cheap` cost, `Exact` ROI.

**Status (v1, shipped, K-108; Preserve-luminance toggle added K-136):** the third one-knob
grade, beside Exposure and Saturation in the **Colour** category. Continuous (a linear
matrix), so the §1.6 oracle holds to ≤ 2 fp16 ULP (measured 0–1 on the dev RTX) in **both**
modes. 0° resolves to the exact identity matrix in either mode — the bit-exact neutral point,
pinned by test — and Mix 0 is likewise the identity. Hue rotation runs in the compositor's
scene-linear working space (not gamma), consistent with every other grade here. (Note: the
constant-luminance mode is a Rec.709-weighted linear-RGB rotation, in the spirit of K-034's
perceptual hue handling but not literally an Oklab rotation — see docs/GUIDE.md.)

**The Matte scales Angle toward 0 (K-426, §2.6):** the rotation matrix for `Angle·k` is built
per pixel in the kernel from the same coefficients, so a half-grey matte on 90° turns the hue
45° — where a fade would mix the turned colour with the original and desaturate it.

### 3.18 Contrast

**Parameters:** Contrast (per cent, default 100, slider 0..200, hard min 0 and unbounded
above), Mix.

**Algorithm sketch.** Expand or compress every RGB channel about a fixed mid-grey pivot:
`out = (in − pivot) × k + pivot`, with `k = Contrast ÷ 100` and `pivot = 0.5`. Alpha is
untouched. The maths runs in the compositor's scene-linear working space, consistent with the
other grades, and the pivot subtraction happens in that same space. Because of the `− pivot`
offset this is an **affine** grade, not a pure scale, so — unlike Exposure and Hue shift — it
does **not** commute with premultiplied alpha: it declares `alpha mode: unpremultiplied` and
the host wraps it unpremultiply → grade → re-premultiply, exactly like Colour balance and
Saturation (§2.2), so matte edges do not shift. `cheap` cost, `Exact` ROI.

**Status (v1, shipped, K-110):** the fourth one-knob grade, beside Exposure, Hue shift and
Saturation in the **Colour** category. Purely continuous (no round/clamp/quantize — mid-grey
0.5 is the fixed point, and highlights are never clipped), so the §1.6 oracle holds to ≤ 2
fp16 ULP, exercised on a corpus that includes partial-alpha pixels since the premultiply round
trip is load-bearing here. Contrast 100 % (`k` 1.0) short-circuits to the input on both paths
(the bit-exact neutral point, pinned by test); Mix 0 is likewise the identity. The pivot is a
plain mid-grey 0.5 rather than the 0.18 scene-linear mid-grey, so the control matches the
familiar photo-editor contrast slider (symmetric about 50 %) rather than a light-meter grey
card — an editing-desk feel over a colour-science one.

### 3.19 Gamma

**Parameters:** Gamma (default 1, slider 0.1..4, hard min 0.01 and unbounded above), Mix.

**Algorithm sketch.** A per-channel power curve: `out = pow(max(in, 0), 1 ÷ gamma)` per RGB
channel, with alpha untouched. The maths runs in the compositor's scene-linear working space,
consistent with the other grades. The input is clamped to ≥ 0 **before** the power (scene-linear
colour can dip slightly negative, and a power of a negative base is undefined); that clamp is
byte-identical on the CPU reference and the WGSL kernel, so the §1.6 oracle holds. The exponent
is `1 ÷ gamma`, so a Gamma above 1 lifts the mid-tones (brightens) and below 1 lowers them — the
convention where the number reads like a display gamma. Because a power curve is **non-linear**
it does **not** commute with premultiplied alpha: it declares `alpha mode: unpremultiplied` and
the host wraps it unpremultiply → curve → re-premultiply, exactly like Contrast and Saturation
(§2.2), so matte edges do not shift. The hard floor 0.01 keeps `1 ÷ gamma` finite; there is no
ceiling. `cheap` cost, `Exact` ROI.

**Status (v1, shipped, K-112):** the fifth one-knob grade, beside Exposure, Hue shift,
Saturation and Contrast in the **Colour** category. Continuous everywhere for input ≥ 0 (the
power is smooth, and the pre-clamp removes the only discontinuity), so the §1.6 oracle holds to
≤ 2 fp16 ULP, exercised on a corpus that includes partial-alpha pixels since the premultiply
round trip is load-bearing here. Gamma 1.0 short-circuits to the input on both paths (the
bit-exact neutral point, pinned by test — a short-circuit, not a reliance on `pow(x, 1)` being
exactly `x`); Mix 0 is likewise the identity. 0 and 1 are fixed points of the curve at any
Gamma, so a 0..1 image stays in range, while scene-linear highlights above 1 are curved honestly
and never clipped (§2.1). Distinct from Colour balance's three-channel Gamma: a single,
animatable mid-tone control — the common one-knob gamma move.

**The Matte pulls Gamma toward 1 (K-426, §2.6):** a half-grey matte on Gamma 2 curves by
`pow(x, 1/1.5)` — a genuinely gentler curve — and not `lerp(x, pow(x, 1/2), ½)`.

### 3.20 Temperature

**Parameters:** Temperature (a plain number, default 0, slider −150..+150, hard ±200), Mix.

**Algorithm sketch.** A warm/cool white-balance shift as a per-channel gain in the
compositor's scene-linear working space: with `k = Temperature ÷ 100` (clamped to the ±2 hard
range), red is scaled by `gain_r = max(0, 1 + 0.75·k)` and blue by
`gain_b = max(0, 1 − 0.75·k)`, so warming (`+`) lifts red and drops blue and cooling (`−`)
does the mirror; green and alpha are untouched. The `0.75·k` gain (K-135, up from `0.5·k`)
makes full deflection a decisive orange or blue, and the `max(0, …)` floor stops an extreme
driving a channel negative. The two gains are
computed host-side (in the resolve step) so the CPU reference and the WGSL kernel multiply by
byte-identical `f32` factors — no arithmetic per pixel or per path beyond the multiply itself.
**Premultiplied throughout**, exactly like Exposure (§3.16): a per-channel scalar scales
premultiplied colour consistently (straight × gain, then × the unchanged alpha), so — unlike
the affine Contrast and Saturation grades, whose `− pivot`/luma offset breaks that commutation
(§2.2) — there is no unpremultiply round trip and matte edges do not shift. `cheap` cost,
`Exact` ROI.

**Status (v1, shipped, K-113):** the sixth one-knob grade, beside Exposure, Hue shift,
Saturation, Contrast and Gamma in the **Colour** category. Continuous everywhere (a linear
per-channel scale, no round/clamp/quantize, highlights never clipped), so the §1.6 oracle
holds to ≤ 2 fp16 ULP, exercised on a corpus that includes partial-alpha pixels to pin that
the premultiplied multiply comes out identical on both paths. Temperature 0 resolves to gains
exactly `(1.0, 1.0)` and short-circuits to the input on both paths (the bit-exact neutral
point, pinned by test); Mix 0 is likewise the identity. This is the simple montage-grade
warmth lever — a per-channel ±0.75·k R/B gain with green held (K-135) — not the fuller white
balance sketched for Tier 2 (§3.10: a Bradford-adapted CCT shift with a Tint axis); it is the
common one-click warm/cool move, animatable like every other grade.

**The Matte scales Temperature toward 0 (K-426, §2.6):** the two gains are rebuilt per pixel
from `Temperature·k` rather than lerped, because the blue gain floors at 0 past ±133 and a
lerp of a floored gain is not the gain of a smaller Temperature.

### 3.21 Matte key — Keylight-style colour-difference keyer (greenscreen removal)

Pulls a proper key off a green (or blue) screen: alpha is driven down where a pixel matches
a chosen **screen colour**, with the strength/balance/clip/despill controls a colourist
expects from Foundry's Keylight. It began (K-121) as a soft chroma-distance key and was
expanded (K-154) into the colour-difference keyer below. Everything is
`clamp`/`min`/`max`/`mix` — **continuous everywhere**, so it is safe under the §1.6 ULP
oracle, unlike a hard threshold.

**Parameters.** Top level, always visible:
- **View** (choice, default Final result): **Final result** the keyed picture, **Screen
  matte** the alpha as greyscale (white kept, black keyed), **Status** a continuous heat of
  the matte (greyscale, with the uncertain mid-tones tinted so at-risk edges and holes stand
  out) — so the user can see what they are keying.
- **Screen colour** (colour, default green ≈ `[0, 0.6, 0]` — the screen to remove; its
  largest channel picks the primary screen axis, so a blue screen keys too).
- **Screen gain** (%, default 100, the matte fall-off strength — 100 % keys the exact screen
  to zero, higher keys more aggressively).
- **Screen balance** (%, default 50, how the two non-screen channels are weighted into the
  reference — 0 their min, 100 their max, 50 their mean).
- **Despill bias** (colour, default neutral grey — shifts the reference the despill clamps
  the primary down to; grey is a no-op) and **Alpha bias** (colour, default grey — shifts
  what counts as neutral for the matte; grey is a no-op).
- **Despill amount** (%, default 100, the Keylight screen despill).

**Screen matte** twirl (collapsed), in the order the pipeline runs them: **Screen pre-blur**
(px@comp, default 0, softens the picture the key is *judged from*, never the picture that comes
out), **Clip black** (%, default 0, matte at/below maps to 0), **Clip white** (%, default 100,
matte at/above maps to 1), **Clip rollback** (%, default 0, eases the clips back toward the
un-clipped matte to recover fine edge detail), **Screen shrink/grow** (px@comp, default 0,
marches the matte's edge inward at negative values and outward at positive ones — morphological,
so the edge stays as crisp as it was), **Screen softness** (px@comp, default 0, blurs the matte
and only the matte), **Despot black** and **Despot white** (%, default 0, remove isolated dark
and bright specks), **Inside mask** and **Outside mask** (mask-path rows, default unset — the
garbage mattes: inside forces the matte opaque, outside forces it transparent), **Replace
method** (choice: Source / Hard colour / Soft colour / None, default Soft colour) and
**Replace colour** (colour, default grey). Then the shared **Mix**.

**Algorithm sketch.** Operates on **straight (unpremultiplied) colour** (`alpha mode:
unpremultiplied`, §2.2), wrapped unpremultiply → key + despill → re-premultiply exactly like
Saturation. The screen colour's largest channel is the **primary** axis (green for a green
screen); the two others are **secondaries**, blended by Screen balance into a *reference*. A
pixel's **screen difference** is `primary − reference`: large on the screen, small or
negative on the foreground. Normalising by the screen colour's own difference gives 1 on the
exact screen and 0 on a neutral, so `matte = clamp(1 − gain·raw, 0, 1)` keys the screen to 0
and holds the foreground at 1. **Alpha bias** subtracts a bias-colour neutral so a tinted
bias shifts what counts as neutral (grey ⇒ no-op). **Clip black/white** remap the matte's
ends and **Clip rollback** blends back toward the un-clipped matte. **Despill** pulls the
primary channel down toward the (despill-bias shifted) secondary reference by the despill
fraction, draining screen tint; **Replace method** then recolours where spill was removed —
Source keeps the original colour, Hard/Soft blend in the replace colour (Soft scaled by the
pixel's brightness), None leaves the despilled colour.

**The spatial stages (K-546).** Every control above judges a pixel on its own; these judge it
by its neighbours, so the matte becomes a **picture of its own** for the length of the effect
and is only spent on the colour at the end. Seven stages, in this order: pre-blur the picture
the key is judged from → the screen matte, clips and rollback → shrink/grow → softness →
despot → the garbage masks → despill, Replace, View and Mix on the *original* colour. The
matte is carried as an ordinary four-channel picture with the same number in every channel, so
Softness and the pre-blur are the **shared** Gaussian blur rather than second implementations.
Shrink/grow is a separable morphological min or max over a square, with the outermost ring
eased in by the fractional part of the radius so the control stays continuous. A **despot** is
an amount rather than a size and reaches exactly one pixel: a speck is a pixel every one of
whose eight neighbours is on the other side of it, so black lifts such a pixel to the darkest
of them and white drops it to the brightest, which leaves a real edge — always with a
neighbour on its own side — untouched. The **garbage masks** are two `ParamKind::MaskPath`
rows (§1.2, K-408) on this layer's own masks: the outline arrives as geometry, inside/outside
is an even-odd crossing count, and the soft edge is the **mask's own feather and expansion**
read through the same ramp the mask's ordinary coverage is read through, so a hold-out and the
shape it was drawn from soften alike. Both rows are unset by default and an unset row means no
garbage matte, never "the first mask".

**Nothing spatial asked for is the pointwise keyer**, byte for byte: the staged path is not
taken at all, on either the CPU or the GPU, so an existing project keys exactly what it keyed
before. `moderate` cost, `padded_px(251)` ROI — pre-blur, shrink/grow and softness at their own
hard maxima plus the despot's one pixel — `{0}` temporal. Category **Utility**, beside
Transform.

**Status (K-154, shipped — supersedes the K-121 chroma-distance key):** the colour-difference
screen matte, clips, despill and replace model above, with the default green screen + 100 %
gain visibly keying a typical green screen ("drop it on and it works", §1.2). The screen's
primary channel and reference are derived from the resolved Screen colour identically on the
CPU reference and in the WGSL kernel, so both paths use the same numbers (K-031); the effect
is continuous (no hard step), so the §1.6 oracle holds to ≤ 2 fp16 ULP over a corpus of
near-screen, far-from-screen, partial-alpha and HDR pixels swept across gain / balance /
clips / despill / replace / bias and all three View modes. There is **no neutral no-op
default** (the effect exists to key, §1.2 — the tasteful default keys); **Mix 0 is the
bit-exact identity**, pinned by test. The Screen colour and the bias/replace swatches render
through the inspector's existing `ParamKind::Colour` arm (each with the eyedropper); the twirl
uses the K-145 `ParamGroup`. **Migration:** a project saved before K-154 keeps its stored
Screen colour (`key`) and Spill (now Despill amount); its old Tolerance/Softness are
superseded by gain/balance/clip and simply go unread, and the new controls take their Keylight
defaults (version bumped 1 → 2, so the frame cache re-keys).

**Status (K-546, shipped — the spatial half):** Screen pre-blur, Screen shrink/grow, Screen
softness, Despot black/white and the Inside/Outside garbage masks, described above. The
garbage mattes are mask-path rows rather than the layer-input holdout the K-155 deferral
guessed at — a garbage matte is a shape drawn on this layer, which is what the K-408 carriage
already delivers. The §1.6 oracle is `cpu::matte_key_spatial`, matched by the WGSL pipeline one
control at a time, all of them at once, and with the two masks bound; the defaults are pinned
bit-for-bit against the pointwise kernel.

**Deferred still:** the **Colour correction** twirls (Foreground/Edge saturation, contrast,
brightness, colour balance) and the **Source crops** (per-axis edge method + crop amounts).
Both are pointwise and need no pipeline; the keyer above is what "properly key footage" needs,
and these refine it.

### 3.22 Depth of field — depth-driven lens blur with an iris (Frischluft / Camera Lens Blur-class)

A variable-radius lens blur driven by a **depth pass**: pixels near the focus plane stay
sharp, pixels far from it soften, the way a real lens throws the background out of focus.
The depth comes from **another layer** (a **Layer-reference** parameter, §1.2,
[impl/layer-input.md](impl/layer-input.md)) — the standard "footage + matching depth pass"
workflow, and the first effect to take a whole layer as an input rather than a number or a
file. The GPU kernel and its §1.6 CPU oracle are `lumit_gpu::fx::dof` / `fx_dof.wgsl` and
`lumit_core::fx::cpu::dof`.

**In plain terms.** Open a hole the size of the blur around each pixel and average what you
can see through it. Two things make that read as a *lens* rather than as a smudge, and both
arrived with K-313:

- **the hole is a polygon.** A real iris has blades, and a defocused highlight is a picture
  of the hole — which is why bokeh balls are hexagons on some lenses and circles on others.
- **the average is a power mean.** A flat average dissolves a small bright thing into its
  dark surroundings; raising each tap to a power first is what lets it survive and bloom
  into a ball instead.

Both are **off at their defaults**, so an existing project renders exactly what it always
rendered — see *Neutral means bit-identical* below.

**Parameters:** **Matte** with **Invert** beside it — the uniform matte row of §2.6, here
under this effect's own deeper meaning: the matte is a *depth* pass and it decides **focus**,
not strength. The stored ids are still `depth` and `depth_invert` (K-065 — a save is a save);
only the labels and the row are shared (K-395, which moved Invert up out of the Depth map
twirl to sit beside its picker, where every effect's Invert now is). The layer reference is
unset until picked — a labelled no-op — and Invert defaults off, so when on the depth is
inverted, `d' = 1 − d`, before the circle-of-confusion, swapping near and far. Then
Depth source (a combobox beside the Matte picker, K-142: **None** reads the depth layer's
raw pixels — no masks, no effects, the default; **Masks** reads it plus its masks; **Effects and
masks** runs the depth layer's own effect stack into the depth pass first, a graded/blurred depth
map — same v1 temporal boundary as the effects-and-masks matte; replaces K-125's "Depth after
effects" checkbox),
Focus distance (0–1, default 0.5, the in-focus depth; greys out while Use focus
point is on), Use focus point (bool, default off) and Focus point (an `_x`/`_y`
px@comp pair drawn as one row with a **crosshair pick**, §6.1 of
[07-UI-SPEC.md](07-UI-SPEC.md) — click the thing you want sharp instead of
hunting for a number; centred on the raster by `instantiate_for_raster`). Those
three sit together deliberately: a switch that hands one control's job to
another belongs beside both, not three twirls away. Then
Focus range (0–1, default 0.1, the
half-width of the sharp band around focus), Aperture (px@comp, default 8, slider 0–40, the
**master** maximum circle-of-confusion radius, scaling both per-side radii about its default 8),
Near blur (px@comp, default 8, slider 0–40, the max circle-of-confusion on the **near** side,
`d < focus`) and Far blur (px@comp, default 8, slider 0–40, the **far** side, `d ≥ focus`) — the
owner's "adjust close/far blur separately"; then three collapsed twirls:

- **Iris** — Blades (int, 3–8, default 6: the aperture's blade count, inert while Roundness
  is 1 because a circle has no blades), Roundness (−1…1, **default 1**: 1 is the circle, 0
  the straight-edged polygon, and **negative is concave** — five blades at −1 is a star),
  Rotation (degrees on a **dial** sitting beside its number, default 0, unbounded so it winds
  through full turns), Aspect ratio (−1…1, default 0: 0 is round, positive stretches the
  highlights wide and negative tall — the oval an anamorphic scope lens throws; a squeeze
  either side of round rather than a 1.33-or-2.0 ratio, which is why it runs −1…1),
  Rim brightness (−1…1, default 0: **where the light sits inside each ball**. A real lens does
  not throw a flat disc — an under-corrected one rings the edge bright, the "soap bubble"
  look, an over-corrected one pools light in the middle for creamy bokeh. That is spherical
  aberration; negative is the soft centre, 0 the flat disc, positive the bright rim).
- **Highlights** — Highlight threshold (default 1.0, scene white: the linear level each tap
  is split at) and Highlight exposure (stops, **default 0**: how hard the over-threshold part
  blooms). Exposure 0 is the plain arithmetic mean and the whole split is skipped.
- **Depth map** — how the pass is *read*, as against where focus is: Depth channel
  (**Luminance** by default, right for the grey map a depth pass usually is; the shortlist is
  Luminance / Alpha / Red / Green / Blue, and every entry has to be able to explain itself —
  nothing encodes a depth as a hue), Gamma (−10…10, default 0 — the gamma on the depth axis),
  Remove edge leak (0–1, default 0) and Detect edge threshold (0–1, default 0.1).

Then Repeat edge pixels (bool, default on), Display (choice, default Rendered — a diagnostic
view: **Rendered** the normal blurred output, **Depth map** the post-invert, post-channel-pick
depth as greyscale, **Focus map** the smooth in-focus mask, white where sharp), Mix.

**Algorithm sketch.** Per output pixel, read the depth from the **Depth channel** of the
referenced layer (0..1; by convention 0 = near, 1 = far, though the effect is symmetric about
Focus), and — when **Invert** is on — replace it with `1 − d`. Focus is **Focus
distance**, or the depth under **Focus point** when that is ticked. The depth's distance from
focus, beyond the sharp band `range`, is **scaled by Gamma** (`2^gamma`,
host-computed)
and then ramps by a smoothstep `s` to a circle-of-confusion radius: `s ·` (**Near blur** where
`d < focus`, else **Far blur**), each per-side radius already scaled by the **Aperture** master
(`radius · Aperture / 8`). Because the near/far select flips only at `d = focus`, where
`s = 0`, the radius is continuous, so the §1.6 ULP oracle still holds.

An aperture of that radius is then gathered from the source, edges repeated or transparent per
**Repeat edge pixels**, and blended back by Mix. The aperture is the
inscribed **Roundness/Blades/Rotation/Aspect ratio** polygon, its taps optionally weighted by
**Rim brightness** and pulled back across depth discontinuities by **Remove edge leak**, and the
average is the split-at-threshold power mean when **Highlight exposure** is non-zero.

**There is no composite menu.** The defocused result replaces the original, blended by Mix.
An effect that wants its balls added over a sharp plate is an adjustment layer with a blend
mode — the mechanism that already exists for exactly that, and does it in one obvious place
rather than in a dropdown on every effect that could plausibly want one.

**Why Gamma exists.** A real depth pass rarely spreads its content over 0..1: a linear depth
channel puts the sky or a distant ceiling at 1.0 and compresses an entire room into the bottom
fifth, so the depth *differences* that matter are a tenth of the range. Without scaling the
distance first, focus is all-or-nothing — the scene stays almost sharp and the one near object
is almost fully blurred, with nothing in between. The scale is **one doubling per unit**, so
the setting that reads well on game footage lands around 6 (64×) — the middle of the slider
rather than its end.

**The power mean cannot be computed naively.** `(Σ c^p / n)^(1/p)` underflows f32: at a high
Exposure a channel at scene-linear 0.08 raises to 8e-36 and one at 0.05 to 2e-42, below the
smallest normal, so every channel below roughly 0.116 linear collapses to black *per channel*
— black holes and saturated speckle rather than a blur. Both paths factor the brightest excess
`M` out first, which is an exact identity
(`(Σ w·c^p / Σw)^(1/p) = M · (Σ w·(c/M)^p / Σw)^(1/p)`) and puts every ratio in `[0, 1]`, so
nothing underflows and no floor is needed. It costs a second walk of the aperture, which is
why the gather is two loops when the split is on.

**The aperture stays inscribed in the circle of confusion at every setting.** The gather scans
a `ceil(coc)` box and tests each integer offset, which only bounds it if no accepted tap lies
outside that circle. Negative Roundness keeps the *vertices* on the circle while pulling the
edge midpoints in (at a vertex both terms of the inside test carry the same `k²r²`, so it
collapses to `r ≤ coc` whatever the coefficient), and Aspect ratio's multipliers are always
≥ 1 with exactly one > 1, so it can only shrink one axis. That is what keeps the ROI declaration honest,
and it is pinned by `the_dof_aperture_stays_inside_its_circle` rather than left to the oracle,
which would miss the same taps on both paths.

**Neutral means bit-identical, and it is reached by branching** (K-313). Roundness 1 takes the
plain `r² ≤ coc²` circle test, Rim brightness 0 and Remove edge leak 0 take the unweighted
accumulation, and Exposure 0 takes the unsplit sum — rather than multiplying every tap by one
and splitting it at a threshold it never crosses. None of those is an IEEE 754 identity:
`Σ(c·w)/Σw` is not `Σc/n` when every `w` is 1, `min(c,t) + max(c−t,0)` is not reliably `c`, and
scaling both sides of a comparison by `apothem2` can flip a boundary tap. At their defaults the
three branches leave exactly the box-weighted disc average this effect computed before the
iris existed, which is why the aperture could fold into the shipped effect rather than arrive
beside it as a second one. `the_default_aperture_is_the_historical_disc_bit_for_bit` pins it.
Gamma is the exception that proves the rule: its neutral is a multiply by exactly
1, which *is* exact, so it needs no branch.

Operates on **premultiplied** colour (the aperture gathers the working premultiplied image, so
coverage and colour blur together). The **Display** diagnostic modes short-circuit before the
gather and write their view directly, ignoring the blur, the composite and Mix; every shipped
mode is continuous, so the §1.6 oracle covers them all (none excluded). `moderate`
cost, ROI a padded gather (the static declaration is twice the 40 px Aperture slider, whose hard maximum is open), `{0}`
temporal. Category **Blur & sharpen**. A zero effective aperture (master or both sides at 0), a
depth everywhere inside the sharp band, or `Mix 0` are all bit-exact passthroughs, pinned by
the kernel oracle.

**Threading the depth (K-031).** The resolved bag carries only the scalars; the depth is a
whole texture, so — like the LUT's cube and Motion blur's flow field — the referenced layer's
render travels **beside** the resolved op (a parallel `layer_inputs` slot the k-th consuming
op binds, declared by the effect as `AuxKind::LayerInput` per K-387 and shared with §3.28's
Light wrap). Preview and export render the depth through **one shared helper**
(`fxops::render_layer_input`), so the viewport and the file match. The frame cache key hashes
the referenced layer's source and transform (the same content a matte's key hashes), so
editing the depth pass retires stale frames.

**Status (v1, shipped, K-124; extended K-128, K-313):** the depth-driven aperture blur above.
K-128 added the depth Invert, separate Near/Far blur under the Aperture master, and the
Rendered/Depth map/Focus map Display views. K-313 folded in the iris, the split-at-threshold
power mean and the fuller depth model (channel pick, focus point, Gamma,
edge-leak suppression), all neutral at their defaults.

Deliberate v1 limitations (documented, follow-ups tracked): the depth layer is sampled per its
**Depth source** mode (K-142) — None (raw), Masks, or Effects and masks (which runs its own
stack into the depth) — and **resampled to the consuming layer's raster** to align with the
pixels the blur runs on; a placement-aware depth is a follow-up (the referenced layer's own
transform is not applied). The depth layer only needs to be **in-span**
— it is expected to be *hidden* (a depth map should not render into the comp), and both the
preview decode planner and export decode a hidden layer-input reference exactly as they do a
matte source. The depth layer is chosen with the inspector's Layer picker
(a dropdown of the comp's layers, its own included — K-288, where it reads the effect's
own input), with the Depth source combobox beside it; an unset or dangling reference is a
no-op.

**Open (K-313).** Three things here are *our reading* of controls rather than measurements
against a reference plugin, and are the honest places to correct later: Rim brightness's curve and
Aspect ratio's mapping. So is the stops-to-power constant
(`Dof::EXPOSURE_STOPS_PER_DOUBLING = 12`): 6 put the top of the Exposure slider at
a power of 32, which is a maximum filter rather than a mean — flat hard-edged polygons instead
of bokeh discs — and 12 puts the top at about 5.7, which is strong but still an average. Turn
it if the onset feels early or late. Edge-leak removal only pulls back taps that are *nearer*
than the pixel being gathered.

**Not this effect: DOF PRO.** The physically-accurate, deliberately intensive depth of field —
a scene-referred aperture and f-stop, per-pixel scatter, occlusion and inpainting behind
foreground edges, spectral response — remains a separate planned effect (K-313). What landed
here is the base lens blur finished, not that.
### 3.23 Invert

**Parameters:** Mix.

**Algorithm sketch.** A simple colour inverse: `out.rgb = 1 − in.rgb` per channel, alpha
untouched. Because `1 − c` is affine (a `1 −` offset, not a pure scale) it does **not**
commute with premultiplied alpha, so — like Contrast and Gamma (§2.2) — it declares `alpha
mode: unpremultiplied` and the host wraps it unpremultiply → invert → re-premultiply, so
matte edges do not fringe. The inverse is taken in the compositor's **scene-linear working
space** as-is (the deliberately simple choice, K-126): scene-linear values above 1.0 invert
to honest negatives, never clipped (§2.1), and there is no display-referred round trip. There
is no neutral no-op default — invert always inverts, so the "no no-op default" rule (§1.2) is
satisfied trivially — and **Mix 0 is the bit-exact identity**. `cheap` cost, `Exact` ROI,
`{0}` temporal. Category **Colour**, beside its grade siblings.

**Status (v1, shipped, K-126):** the one-parameter inverse above. Continuous everywhere (a
plain `1 − c`, no round/clamp/quantize), so the §1.6 oracle holds to ≤ 2 fp16 ULP, exercised
on a corpus that includes partial-alpha pixels since the premultiply round trip is
load-bearing here. The scene-linear space choice is the owner's "simple inverse"; a
display-referred (perceptual) inversion is a possible later variant, not v1.

### 3.24 Tint

**Parameters:** Map black to (colour, default black `[0, 0, 0]`), Map white to (colour,
default white `[1, 1, 1]`), Mix.

**Algorithm sketch.** A luminance duotone / gradient map: `out.rgb = black + (white − black)
· luma(in.rgb)` per channel, with `luma` the Rec. 709 weighting (0.2126·R + 0.7152·G +
0.0722·B) on the **unpremultiplied** linear colour, alpha untouched. Every pixel's brightness
picks a colour on the black-to-white gradient, so the image is recoloured while its luminosity
structure is kept — the "select two colours, map everything between them" look. A luma-driven
colour remap does not commute with premultiplied alpha, so — like Contrast and Gamma (§2.2) —
it declares `alpha mode: unpremultiplied` and the host wraps it unpremultiply → map →
re-premultiply, so matte edges do not fringe. The lerp is written `black + (white − black)
· luma` (rather than `black·(1 − luma) + white·luma`) so the CPU reference and the WGSL kernel
reduce in the same order and the §1.6 oracle holds. The **default black→black / white→white
maps every pixel to its own luma — a greyscale**, a visible tasteful result (§1.2), not a
no-op; **Mix 0 is the bit-exact identity**. `cheap` cost, `Exact` ROI, `{0}` temporal.
Category **Colour**, beside its grade siblings.

**Status (v1, shipped, K-127):** the two-colour luma map above. Continuous everywhere (a
linear lerp of a luma), so the §1.6 oracle holds to ≤ 2 fp16 ULP, exercised on a corpus that
includes partial-alpha pixels since the premultiply round trip is load-bearing here. The two
colours render through the inspector's existing `ParamKind::Colour` arm — no inspector change
was needed. Distinct from Colour balance's three-channel trackballs: a two-colour duotone that
remaps by luma rather than grading in place. The fuller shadows/mids/highlights **Tritone**
(three colour stops) is tracked as a Tier 2 follow-up (§4).

### 3.25 Posterize time — temporal frame-rate hold (stop-motion look)

**Parameters:** Input frame rate (default 12), Phase (comp seconds, default 0). There is no
Scope parameter (K-166): what the hold covers is implied by the kind of layer carrying the
effect — see **Reach** below.

**Algorithm sketch.** A **temporal** effect, not a per-pixel one: it changes *what time* the
layers it covers render at. The current comp time snaps down to a coarser grid —
`held_t = floor((t − phase)·rate)/rate + phase` — and the covered content re-renders at
`held_t` instead of `t`, so the animation updates only `rate` times a second (the choppy
stop-motion / on-twos look). It re-resolves **transforms, effects, the camera AND which source
frame footage decodes to** at the held time, so a scene that is only footage playing back
visibly steps to the coarser rate (the decode planner snaps the covered layers' sample time via
`lumit_core::fx::posterize_sample_times`, the twin of the held re-render — FX-1). Smooth
sub-frame footage *motion blur* between the held frames is a different effect (the flow Motion
blur, §3.2); Posterize only *quantises* the playback grid. Because it re-renders
rather than filters, it lives at the frame-orchestration layer — detected where
`build_comp_draws` + realise (preview) and `render_comp_linear` (export) run, never in
`run_ops` — and so resolves to **no** per-pixel op. See
[docs/impl/temporal-rerender.md](impl/temporal-rerender.md).

**Reach (K-166 — implied by the carrier, no Scope parameter).** The effect holds whatever the
layer carrying it would feed its effect stack anyway, so no parameter is needed. On an
**adjustment layer** that input is the composite of everything beneath, so the whole scene
below re-renders at `held_t` and is laid back over the live composite by the adjustment's
coverage (its mask × opacity) — the owner's global "posterise the whole scene" pass is simply
the effect on a full-frame adjustment layer. On any **other layer** the input is the layer's
own source and stack, so the hold is per-layer: its **effect stack and its source sampling**
step at `held_t` (a per-layer time substitution — no re-render of others, no orchestration
re-entry) while the layer's **transform stays live**, so the layer moves smoothly but its own
effect animation and footage playback are choppy — the AE per-layer form. The held effect time
is `lumit_core::fx::this_layer_effect_time` (the grid computed on comp time, mapped into the
layer's own base), fed to `resolve_stack_temporal` as the sample time so a
`sample_temporally == false` effect still resolves at the live playhead; the held source frame
comes from the same `posterize_sample_times` snap the below-stack layers use. (An earlier
build carried an explicit Scope choice; K-166 removed it — a stored Scope value in an old
project is ignored and the kind rule above applies.)

**Determinism & cache.** `held_t` is a pure function of `t`, `rate` and `phase`, so many
frames share it and re-render identically; the frame key folds the effect's parameters, and
the held-time dedup (keying the below-stack at `held_t` so identical held frames collapse to
one cache entry) is a tracked optimisation on top — correctness never depends on it.

**Preview == export (K-031).** Both paths re-render the below-stack through the **one** shared
`render_below_at` = `build_comp_draws` at `held_t` (reusing the held decoded pixels) → the
shared `Realiser`. A still-scene re-render at the same time is bit-identical to no re-render,
and a full-coverage posterised frame is bit-identical to a plain render at the held time (both
pinned by test). **Boundaries (v1):** temporal effects *inside* the held below-stack (echo,
flow Motion blur, Datamosh) degrade to stills — the held re-render carries no *neighbour* frames
(only the primary source frame is snapped to the grid), the same boundary the after-effects
matte takes (K-125); footage is held everywhere below the adjustment (so a *masked* Posterize
reveals held footage outside the mask too, comp animation stepping only inside it — the
full-frame adjustment being the intended global pass); a Posterize adjustment *inside a
collapsed* Precomp degrades to a no-op (its held draws are sized for the nested comp); and the
footage *inside a collapsed Precomp that sits beneath* a Posterize is not guaranteed to step —
the collapse splice keeps its inner decode live (the same reason collapsed-Precomp temporal
effects are a follow-up), so that narrow case is a documented parity boundary rather than a
promise. `cheap` cost, `FullFrame` ROI, `{0}` temporal, Category **Temporal**.

### 3.26 Motion blur — the expensive, correct motion blur (accumulation)

Labelled **Motion blur** in the UI: the accumulation kind is the correct, whole-scene one, so it
takes the plain name; the optical-flow effect (§3.2) is *Fast motion blur*. Do not confuse
either with the per-layer transform motion-blur *switch* (docs/06 §4, K-120), which is a layer
switch, not an effect.

**Parameters:** Samples N (default 8), Shutter angle (degrees, default 180), Shutter phase
(degrees, default −90), Force on all layers (bool, default off), Mix (per cent, default 100).

**Algorithm sketch.** A **temporal** effect, not a per-pixel one, and the sibling of Posterize
time (§3.25): it renders the **whole scene below it** several times at in-between moments and
averages the finished frames. Per-layer motion blur (docs/06 §4, K-120) smears one layer along
its own transform; accumulation motion blur smears everything below — footage motion, animated
effects, depth passes, the camera — all correct per sample (no blurred-depth artefact). The
sub-frame sample times reuse the **same centred-shutter maths** as per-layer motion blur
(`MotionBlur::sample_offsets`): for Samples N the k-th offset is `phase/360 + (k + 0.5)/N ·
angle/360` frames, so `τ_k = t + off_k · dt` (dt = one comp frame). The N finished
below-composites are averaged by a **hardware additive-at-`1/N`** pass (`Compositor::accumulate`
— colour **and** alpha additive over a premultiplied-passthrough fragment, so a static scene is
unchanged; NOT the Add blend mode, which over-composites alpha). **Mix** blends the averaged
(blurred) result against the frame-time composite (a linear interpolation the same additive pass
gives exactly). Because it re-renders rather than filters, it lives at the frame-orchestration
layer — detected where `build_comp_draws` + realise (preview) and `render_comp_linear` (export)
run, never in `run_ops` — and so resolves to **no** per-pixel op. See
[docs/impl/temporal-rerender.md](impl/temporal-rerender.md).

**Adjustment behaviour.** Like Posterize on an adjustment layer, it is an adjustment effect: the
composite beneath the effect's layer is what re-renders, laid back over the live composite by
the adjustment's coverage (mask × opacity). The owner's global "motion-blur the whole scene"
pass is simply the effect on a full-frame adjustment layer.

**Force on all layers.** With this on, every layer in each sub-frame sample render also smears
along **its own transform** — per-layer motion blur (K-120) forced on for the whole below-stack,
the effect's own Shutter angle/phase/Samples standing in for the comp master and each layer's
own switch. So one effect blurs every moving layer without toggling each one, and because each
of the N accumulation samples is itself transform-smeared the result stays smooth at lower
sample counts. Implemented **without mutating the comp**: the forced shutter and per-layer
switches ride on the sample render's cloned comp only (`AccumulationMbParams::forced_layer_mb` →
`below_draws_at`), so the document and the live-below composite are untouched. Off by default.
Boundary: the force reaches the top-level below layers; the inner layers of a *nested* Precomp
keep their own switches (a v1 follow-up).

**Preview == export (K-031).** Both paths re-render each sub-frame below-stack through the
**one** shared `render_below_at` and average with the identical `Compositor::accumulate`, so a
preview frame equals an export frame. A **still scene** averaged over N is bit-identical to the
plain composite (pinned by test — `1/N` is exact in fp16, the N copies sum back exactly); a
**moving scene** smears (a coverage-widening test). **Boundaries (v1):** temporal effects inside
the sampled below-stack (echo, flow motion blur, datamosh) hold to stills (the same K-125
boundary Posterize takes), and an accumulation adjustment inside a collapsed Precomp degrades to
a no-op (its sampled draws are sized for the nested comp). Honours the per-effect
`sample_temporally` flag (K-132) — a particle system stays pinned to the playhead across the
samples. Sub-frame sample-count reduction under the draft/scrub path is a tracked follow-up
(full N always on export). `heavy` cost (≈ N× a full comp render), `FullFrame` ROI, `{0}`
temporal, Category **Temporal**.

**Per-effect sampling (K-132).** The held re-render honours each below-effect's
`sample_temporally` flag (a general `EffectInstance` property, default on): an effect with it
**off** resolves at the true frame time, not the held time `held_t`, so a costly or stochastic
effect (a particle system) is pinned to the playhead while the rest of the scene holds. The
split is `lumit_core::fx::resolve_stack_temporal`; with the frame and held times equal it is
byte-identical to the plain resolve, so an ordinary render is unchanged.

**The Matte scales Shutter angle per pixel (K-429, §2.6).** This is the one effect that
claims its matte in the **combine** rather than in a kernel, because it has no kernel: it
orchestrates a re-render, so it resolves to no op and the matte carriage `run_ops` walks
skips it on both sides. The matte is instead carried on the sub-frame plan itself, rendered
by the same helper every other matte goes through, and its Channel and Invert applied once
before the combine reads it. What the combine then does is treat the samples as *cells* —
sample *k* owns the span `[k/N, (k+1)/N]` of the open shutter — and average over the window
`[0, 1]` scaled toward the **shutter anchor**, the point where the frame's own time falls
across the open span (`−phase ÷ shutter`, clamped, which the standard −90° phase on a 180°
shutter puts in the middle). A cell's weight is how much of it that window covers, over the
window's own width, so the weights sum to one at every strength. At white the window is the
whole span and every cell is fully inside it — the equal-weight average the effect has
always drawn. At black it has shrunk to the frame's own instant, which is the unblurred
frame. Between, it is a genuinely shorter exposure over a shorter slice of the same N
moments, which is not the same picture as a blurred frame faded back. **No matte bound runs
the old hardware equal-weight pass unchanged, byte for byte** (K-258).

### 3.27 Lens flare — physically-based lens flare (Realflare-class)

A **simulated** lens flare, not a sprite stack: ghosts are ray-traced through a real lens
prescription (the element radii, glasses and spacings of an actual photographic lens), so
they scale, stretch, colour and slide across frame exactly as a camera's do, and the
starburst is the true diffraction pattern of the aperture, computed by Fourier transform.
This is the [Hullin et al. 2011] / [Ritschel et al. 2009] approach, studied end-to-end in
the realflare renderer (GPLv3, the reference implementation) and adapted to run per-frame
inside the compositor — the full derivation, formulas and deviations are pinned in
[docs/impl/lens-flare.md](impl/lens-flare.md). K-256.

**Why simulation matters.** Sprite-based flares (the stock-plugin kind) fake the ghost
train with drawn ellipses, so every light looks the same and nothing responds to the
aperture. Traced ghosts get the behaviour for free: iris-shaped discs that grow with a
wider f-stop, chromatic fringing from real glass dispersion, ghosts that flip through the
optical centre as the light crosses frame, and coating colours — each internal reflection
tinted by its anti-reflective coating's interference, which is where the blue/green/magenta
cast of real flares comes from.

**Algorithm sketch** (full detail in the impl note):
1. **Bake** (CPU, on parameter change only, cached): parse the selected .lens
   prescription; enumerate every two-surface bounce pair — and, under a
   reflectance-product prefilter, the best four-bounce paths (K-368) —
   filter by interface and an on-axis brightness probe, rank the two kinds
   brightest-first in one list; bake the
   **starburst sprite** (Fourier amplitude of the iris image under a Fresnel
   propagation term, integrated across the visible spectrum with CIE
   weights); close the **auto-exposure loop** by rendering a thumbnail **at the lens's
   native aperture** (K-432), so the gain describes the glass and not how far
   the iris happens to be closed.
2. **Trace** (GPU compute, per frame): for each surviving pair × wavelength,
   a regular grid of rays over the entrance pupil — each corner weighted by
   the iris mask (blades, roundness, softness) — refracts through every
   surface with per-surface Fresnel/MgF₂-coating weights (the FlareSim
   three-phase walk, K-261), reflecting at the path's two surfaces — or, for
   a four-bounce path, at its four (K-368) — landing on the focus-shifted
   sensor.
3. **Rasterise** (GPU raster, additive): each live grid cell draws as two
   triangles at density `launch cell area ÷ landed area` (energy
   conservation — a ghost focused small burns bright; fold caustics blow up
   into the bright rims real flares show), with sub-pixel fold quads
   inflated flux-exactly so no rasteriser can drop them, cells that straddle
   a fold dropped rather than stretched into streaks, and the caustic
   density capped (K-262), then the Ghost softness box blur.
4. **Combine** (GPU compute): `out = input + intensity · (flare + starburst)`
   in linear light, alpha saturating toward 1 (the Glow shape); the starburst
   is a baked sprite at each light; the whole flare takes Scale and the
   anamorphic squeeze. Mix blends against the untouched input.

**Parameters (K-257 panel design).** Top level: **Light** (one x/y point row —
the `_x`/`_y` pair convention of docs/07 §6.1 — with a pick-on-Viewer dropper;
**px@comp**, open both sides since an off-frame light keeps flaring — point
parameters are always authored in comp pixels, K-260),
**Intensity** (0–4, open above), **F-stop** (0.7–32 — stops the iris down
from the lens's native f-number; wide open the ghosts are big and round,
stopped down small, bladed **and dimmer**, since a smaller hole passes less
light and the auto-exposure is measured wide open — K-432; Intensity is the
knob that puts the brightness back), **Lens** (the embedded prescription library,
K-261, curated to **twenty real lenses** K-264 and re-verified K-265: every
entry bakes a live ghost train and keeps flaring with the light well
off-centre (the three-position probe), chosen for maximally different flare
characters — modern multicoated cine glass, 1930s uncoated exotics, a
four-element Tessar, f0.95 and f1.0 superspeeds, process glass, a pro
telezoom, long telephotos. Wide-angle and fisheye prescriptions are
deliberately absent: the trace's angular acceptance collapses off-axis for
retrofocus designs (recorded limit). Every prescription carries its own
per-surface anti-reflective coating layers, which is what replaced the
K-257 Coating-type presets; labels are `Maker · Model` and the default is
the Master Prime 50),
**Lens file** (K-264: a user's own `.lens` prescription in the same
FlareSim / PhotonsToPhotos Optical Bench format — set, it overrides the
Lens pick entirely, with the native f-number estimated from the geometry;
unset, missing or unparsable degrades to the picked lens, a labelled
fallback that never faults. Content-hashed into the bake key, so editing
the file takes effect on the next rendered frame), then three folds and
the tail:

| Group | Parameters |
|---|---|
| *Lens options* (twirl) | Focus (m) (0.5–100 slider, hard min 0.2 — the focus distance; K-260, refocusing shifts the sensor plane and visibly rearranges the whole ghost train, the "same lens, different focus" look), Anamorphic squeeze (0.5–3), Blades (int 3–16), Rotation, Coating (0 uncoated → 1 fully coated), Roundness, Softness |
| *Flare options* (twirl) | Ghost intensity (0–4), Ghost softness (0–22 slider, px@comp — FlareSim's Ghost Blur, K-261: a touch of out-of-focus softness on the ghost train; a blur radius is a distance, so it is pixels since K-558, and the default and both range ends are the old per cents at a nominal 1080p diagonal — default 0.44 px, K-264's 0.02 % — taste, not cover: the vertex-smoothed density and the multisampled raster leave nothing for it to hide, and **0 is a clean setting**), Max ghosts (int 0–200 — the brightest survive), **Detail** (0.25–4 slider, default 1; K-265 — multiplies the Quality tier's ray grid AND its traced wavelength count through one shared pair of helpers, so the budget is the user's dial: a lens whose rims still show structure buys more without jumping a tier, a preview buys less), Dispersion (0–2), Starburst intensity (0–4), Scale (0.05–20 — the WHOLE flare about the optical centre, ghosts and starbursts together) |
| *Source* | Source type (Manual light / Matte / Lights); **Light tint** (a colour, with picker and eyedropper — multiplies every light in every mode); then, shown conditionally: **Use source colour** (Matte *and* Lights) and — Matte only — **Matte** (the §2.6 row's layer picker; this effect's deeper meaning is source *detection*, and the stored id was already `matte`; defaults to **this layer**, K-288) with **Invert** beside it — on, detection reads `1 − rgb` of the matte, so its DARK parts are the lights; applied in the detect kernel and its CPU oracle rather than at the dispatch seam, because this matte is a picture the flare takes colour from and the seam's pass would flatten it to clamped grey (K-425's rule for an effect that owns its matte). Off by default, so a flare saved before v13 detects what it always did (K-258), Threshold (linear luma, slider 0–1, open above), Threshold softness |

and **Quality** (Draft / Normal / High / Ultra), **Blend** (K-289 — how the
flare element combines with the layer under it; see below), **Mix**. Blades
and Max ghosts are the first **Int-kind** parameters (§1.2): stored and
animated as Float scalars, but declared whole-number so the row steps,
displays and commits integers.

**Blend (K-289, superseding K-258's Background pair).** Everything the effect
renders is a black-backed light **element**: a frame that is pure black where
there is no flare. Blend says how that element combines with the layer
beneath it — the same question a layer's Mode dropdown asks — and offers the
curated light-combine set Echo does (§3.13, T21), for the same reason: the
HSL / burn / dodge modes are ill-defined on a premultiplied light overlay, so
they are not listed. In code order: **Normal**, a divider, then **Add**
(default), Screen, Multiply, Overlay, Soft light, Hard light, Lighten,
Darken, Difference, Exclusion, Subtract, Divide. Every mode runs per channel
on all four channels in premultiplied linear light, and the result's alpha
saturates at 1.

**Normal** heads the list because it is the odd one out: the element
*replaces* the layer, black background and all, so you see the flare on
opaque black. That is exactly what Background = Black existed to produce —
the flare-element-over-black export for a Screen/Add workflow — and it is
what a project saved with that option migrates to. **Add** is light
addition, bit-identical to what the effect did before this menu existed
(`out = in + flare`, alpha saturating), so a project saved with the old
default renders the same pixels. The neutral passthroughs (Intensity 0, Mix
0) stay bit-exact whatever the menu holds.

**Source modes (K-257).** **Manual light** is the tracked-point workflow: one
white source at the Light point. **Matte** detects the flare's sources in a
referenced layer's picture (impl note §6): the brightest points — up to
sixteen anchors, non-max suppressed — each spawn a full flare, positioned on
the source, gated by the soft Threshold; each anchor's brightness is the
**summed flux of every gated detection tile nearest it** (K-267), so a
practical spanning half the frame finally weighs as its whole lit area
where it used to count as one pixel, while a true point source reads
exactly as before. The matte layer defaults to **this layer** — the layer
the effect is on (K-288) — because "flare the lights in this picture" is
what asking for a matte source nearly always means; that reads the effect's
own input at its point in the stack rather than re-rendering anything, and
on an **adjustment layer** it is the composite of everything below, which is
the only picture an adjustment layer has. Point it at any *other* layer and
that layer renders alone exactly as a DoF depth pass does (its own masks and
effects apply, K-142 default) and is expected to be hidden. **Lights** is prepared for light
layers: the option exists and resolves as Manual until they land, so projects
built against it survive the wiring.

**Colouring the flare (K-259).** Every light's colour is `(use source ? the
source's own rgb : white) × gate × Light tint`. **Light tint** applies in all
three modes — in Manual it is simply the flare's colour, since the light is
otherwise white — and is a frame-time value outside the bake key, so animating
it costs no rebake. **Use source colour** (Matte and Lights, default on) is
what decides whether a warm practical flares warm and a cool one cool; turned
off, every detected source flares white through the tint alone, which is what a
matte used purely as a *position* mask wants.

**Reducing it.** The "it's doing too much" dials, in order: Intensity (everything),
Ghost intensity / Starburst intensity (each half separately), Max ghosts (thins the
train), Quality (cost), Mix (final blend). Defaults are tuned to read well on 1080p
footage without touching anything (§1.2).

**Quality.** Two axes ride the ladder: the traced wavelength count (3 bands
at Draft, 8 at Normal, 16 at High, 32 at Ultra — what separates a smooth
spectral fringe from a stacked-copies RGB-split look, each band weighted by
its **integral** of the CIE colour-matching functions rather than a point
sample, impl note deviation D5), and the pupil-grid base (32 / 64 / 96 / 144).
The grid base is only a budget: each ghost pair's own grid scales by its
measured image size (K-262, retuned K-265 — a tight blob keeps the full
base, its caustic rim carries structure the size probe cannot see), and a
**frame-time probe** (K-267) re-measures each pair's worst local stretch at
the actual light position every frame, raising — never lowering — its grid
under a bounded ray headroom, worst stretch first. That is what lets
**Normal stand on its own** rather than being the tier where cell facets
show, and what keeps corner lights from wearing their cells as polyline
edges. A Squeeze or Scale below 1 renders the ghost buffer padded (up to 2×
per axis, K-267) so the widened field carries real flare to the frame edge
instead of cutting to black.

**Cost and traits.** `heavy` cost (the one effect that owns a render pass), `full-frame`
ROI, `{0}` temporal, premultiplied (an additive light overlay), not seeded — the flare is
a pure function of parameters; even the starburst's sample jitter is a fixed hash baked
into the sprite. Category **Stylise**, beside Glow. The per-frame GPU work is a few
hundred thousand ray threads and ~2 ms of additive fill at Normal quality; the FFTs never
run per frame.

**Oracle (K-256, a documented §1.6 deviation).** The trace — the physics — has a CPU twin
compared ray-for-ray at tight absolute bounds (positions to microns; reflectance to 1%,
since GPU transcendental builtins are not correctly rounded); the baked textures are
CPU-built and consumed by both paths, so they are their own reference; the rasterised
frame is compared against a CPU scanline reference at a perceptual bound (mean error +
total energy), because hardware rasterisation pins no per-pixel fill contract a CPU twin
could hit at ULP tolerance — the same staged-oracle shape flow effects already use. The
CPU degradation rung renders the effect as a labelled no-op, like the LUT (K-114
precedent). Exact numbers in the impl note §8.

**Status (core K-256..K-260; FlareSim model K-261; artefact and picker
pass K-262; smooth-shading, curation and custom-file pass K-264; frame-time
grid probe, padded anamorphic buffer and area-flux sources K-267,
shipped):** everything above — the curated library with per-surface
coatings and the `.lens` file override, the three-phase pupil-grid trace
with the vertex-smoothed energy-conserving quad raster (4× multisampled)
and flux-exact caustic inflation, the Ghost softness blur, Focus distance,
the point-pair light row with its Viewer dropper, the Matte source mode,
quality ladder, anamorphic squeeze, Mix. Pinned follow-ups (TODO):
aperture **dirt / scratches** overlays and an **image aperture**; the
**lens designer** window; the **Lights** source wiring (waits on light
layers); an **Occlusion layer** reference. Every shipped parameter is
stable when they land.

### 3.28 Light wrap — the background's light spilled round a keyed edge

The oldest trick in compositing, and the cheapest way to make a keyed subject stop looking
pasted on. In a real camera the light behind a subject spills round its edges — off the
hair, along the shoulders — and a matte cut in software has none of it, which is most of
what makes a composite read as fake.

**Background** names the layer whose light spills (normally the plate the subject was keyed
onto — a layer-input parameter, the same machinery as Depth of field's depth pass and the
Lens flare's matte, [impl/layer-input.md](impl/layer-input.md)). **Width** is how far the
wrap reaches inside the edge, in px@comp, and is also the radius the background is softened
by — they are the same distance. **Intensity** gains the spill; **Mix** fades the whole
effect.

**How it finds the edge, and why it needs no mask.** The foreground's own alpha is the
edge. Blurring a solid matte leaves it 1 deep inside, about a half right at the outline and
less beyond it, so `1 − blurred` is zero in the middle and rises toward the edge — that is
the band, doubled to reach full strength at the outline and multiplied by the original
alpha so it can never paint on transparent pixels. Painting outside the matte would grow a
halo round the subject, which is the classic way to get this effect wrong, and the oracle
asserts against it directly.

The spill is **screened** on rather than added, so a bright plate brightens the edge toward
itself rather than past white. Width 0, Intensity 0, Mix 0 and an unset Background are each
the bit-exact passthrough (the labelled-no-op rule every layer-input effect follows).

Implementation: four passes, two of which are the ordinary gaussian — blur the background
for the spill, blur the foreground for its softened matte (only the alpha is wanted, and
blurring the whole thing gets it for nothing), fold the two into one texture, screen. The
CPU reference does the same four steps in the same order, so the twins agree by
construction rather than by resemblance.

### 3.29 Sprite flare — the art-directed flare

A **deliberately separate effect** from §3.27's physical simulation, not a mode of it: the
two answer different questions and mixing them is what made the first plan for this
muddled. §3.27 asks *what would this lens actually do*; this one asks *draw me a flare
here*.

Everything is placed from the light's **position** — a **Glow** on it, a train of iris
**Ghosts** marching along the line from the light through the frame's centre (which is
where a real lens puts its reflections, mirrored about the optical axis, so the train swings
to the far side of the middle as the light crosses frame), and an anamorphic **Streak**
through it. Ghost spacing is a *fraction* of the light→centre distance, so the train
stretches and gathers as the light moves rather than sliding rigidly.

**There is no bright-pass, and that is the point.** Nothing is read from the picture's
brightness, so there is no threshold for a source to cross and nothing to pop in or out as
grain moves — the complaint that sent §3.27's Matte mode back to the drawing board on
footage. The oracle asserts it directly: nudging the light by a pixel may not change any
pixel by more than a small bound.

One procedural pass, no inputs but the layer itself, so it is Cheap. Intensity 0 and Mix 0
are the bit-exact passthrough. Everything with a distance is px@comp (K-260) and shrinks
with the preview raster, the light's position included — or the flare would slide between
preview and export.

### 3.30 Curves — the per-channel tone curve

**Parameters:** five curves — **Master**, **Red**, **Green**, **Blue**, **Alpha** (After
Effects' own five) — each an ordered list of **2 to 16 control points in the unit square**,
the identity diagonal `[[0, 0], [1, 1]]` by default. Plus Mix. The panel draws them as
channel tabs over one editor, not as five stacked rows.

**The parameter form** (K-412, replacing K-396's twenty fixed knots). AE stores
`ADBE CurvesCustom` as an **arbitrary-data blob**: a list of control points per channel,
however many the user dragged, in a private serialisation. Arbitrary data is not interpolable,
so AE itself only ever *holds* it between keyframes — a curve does not animate, it steps.
Lumit's answer is a real parameter kind rather than a blob: §1.1's **curve** kind, whose value
is the point list itself and which is **static in v1** for exactly AE's reason. K-396's five
fixed knots per channel were the honest floor while no editor existed; the owner asked for the
editor, and a curve stored as its points is what an editor edits. The effect was days old and
unreleased, so the schema was replaced outright rather than migrated (the version went to 2,
which is what stops a cached frame from the knot generation appearing in the curve
generation's picture).

The domain semantics did not change with the form. The **x axis is the input** value and the
**y axis is the output**, both in the unit square; what moved is only how many points may bend
the line, and that a point may now slide sideways.

**Algorithm sketch.** Each channel's points are fitted with a **clamped cubic spline** —
Photoshop's family, since that is the curve every editor's hand already knows. Between two
points it is a cubic; at every interior point the second derivative agrees on both sides (the
C² condition that makes it *the* cubic spline rather than a piecewise guess), solved as a
tridiagonal system in `f64`; and at the two ends the slope is **clamped to the end secant** —
the straight line to the neighbouring point. That end condition is what makes a two-point
curve exactly its own straight line, which the identity diagonal depends on.

The fit is **host-side and once** (`Curves::packed`, Lightning's discipline in §3.74 applied to
a shape that is the same for every pixel): the spline is sampled into a **257-entry table per
channel** at inputs `i / 256`, in `f64`, and both render paths are handed the identical tables.
The CPU reference and the WGSL kernel then do nothing but look up and interpolate, so §1.6 is
checking the *lookup* rather than two spline fits agreeing by luck.

**The clamping rule.** Points live in the unit square, and so does the baked line: every
sample is clamped into 0..1. A cubic through monotone points can bulge past the highest of
them, and a tone curve that climbed above the white the user placed would ring a bright halo
into a roll-off — the same failure a plain Catmull–Rom had, cured here by the box rather than
by a monotone limiter. Clipping *inputs* is a different matter and does not happen: evaluation
is `t[i] + (t[i+1] − t[i]) · f` with the **index clamped and the fraction not**, so an input
below 0 or above 1 continues linearly along the table's first or last segment. Scene-linear
values above 1 are therefore carried on rather than flattened (§2.1), and slightly negative
light stays continuous.

The **per-channel curves run first, then Master** — Photoshop's and AE's order, so an imported
curve set lands the same way round. **Alpha is its own channel and Master does not touch it**,
as in AE; the graded colour is re-premultiplied by the *graded* alpha, so a curve that moves
coverage moves the picture with it rather than leaving a matte scaled twice. Unpremultiplied
(§2.2): a tone curve is non-linear, so it does not commute with premultiplied alpha. `cheap`
cost, `Exact` ROI.

**Neutral is the bit-exact identity:** all five channels at the identity diagonal
short-circuit the whole effect on both paths — decided host-side, on the *points*, because a
kernel cannot afford to compare 1285 numbers a pixel — and Mix 0 likewise. It is a
short-circuit, not a reliance on the table reproducing `y = x`; that the identity table
*does* reproduce it bit for bit is pinned separately, because the lookup's arithmetic is all
powers of two.

**Reading a malformed curve.** A point list that arrives out of x order, outside the square,
with two points at one x, with more than sixteen points, or with fewer than two is
**straightened, never refused**: sorted by x, clamped into the square, the first of a repeated
x kept, the tail past sixteen dropped, and the identity diagonal substituted when fewer than
two points survive. It comes off a document that a hand, an older build or an importer wrote,
and 14-ENGINEERING-RULES §4 forbids a panic on one.

**The import is unchanged** (docs/11 §5). AE's point list is the one property AE's own
scripting cannot read (K-410), so Curves still imports as a placeholder with its unreadable
property named. What changed is the ceiling: the day a blob decoder lands, the target can
carry the whole curve rather than a five-point sample of it.

### 3.31 Levels — input/output black and white with gamma

**Parameters:** the same four channel groups as Curves — **Master**, **Red**, **Green**,
**Blue** — each carrying **Input black** (default 0), **Input white** (default 1),
**Gamma** (default 1, slider 0.1..4, hard min 0.01, unbounded above), **Output black**
(default 0) and **Output white** (default 1); the four level values slide 0..1 and are
unbounded either way by typing. Plus Mix.

**Algorithm sketch.** Per channel, on unpremultiplied scene-linear colour:

```
n   = max((u − in_black) / (in_white − in_black), 0)
n   = n ^ (1 ÷ gamma)                        # gamma 1 skips
out = out_black + (out_white − out_black)·n
```

The reciprocals are computed **host-side** (`Levels::packed`): the input span is floored at
1e-4 so a white point dragged below a black point cannot divide by zero (it saturates
instead, which is what the picture should do), and Gamma is floored at 0.01 exactly as
§3.19 floors it. The kernel therefore multiplies and powers by numbers the CPU reference
computed the same way, and the §1.6 oracle holds.

**Highlights are not clipped, and that is the one deliberate divergence from AE.** AE's
Levels clamps to 0..1 because it grades display-referred integers; Lumit is scene-linear
(§2.1), so a value above Input white produces `n > 1` and travels on through the curve and
the output range rather than flattening. The negative side *is* clamped, before the power,
for the same reason §3.19 clamps: a power of a negative base is undefined, and the clamp
must be byte-identical on both paths.

Per-channel first, then Master, matching Curves. Unpremultiplied (§2.2) — the power makes
it non-linear. `cheap` cost, `Exact` ROI. Fully neutral parameters short-circuit to the
bit-exact identity; Mix 0 likewise.

Distinct from Curves in the way it is distinct in every other editor: Levels is five
numbers with names an eye can aim (set the black, set the white, bend the middle), Curves
is a shape. Distinct from §3.19 Gamma and §3.18 Contrast in being per-channel and carrying
its own end points.

### 3.32 Brightness — AE's Brightness & Contrast, as one effect

**Parameters:** Brightness (default 0, slider −100..+100, unbounded), Contrast (default 0,
slider −100..+100, hard min −100 and unbounded above), Mix.

**Why a sibling and not a mode of §3.18** (K-397; the open question in
[impl/ae-effect-parity.md](impl/ae-effect-parity.md)). Three reasons, and the third is the
one that decided it:

1. **The import needs one effect, not two.** `ADBE Brightness & Contrast 2` is a single AE
   effect with two properties that animate together. Mapping it onto Lumit's Contrast plus
   a second effect would split one keyframed property pair across two stack entries, and
   the report would have to explain a shape the user never authored.
2. **The two knobs are not the same knob.** Contrast (§3.18) is a per-cent scale where 100
   is neutral; AE's Contrast is a signed amount where **0** is neutral. Folding them into
   one control means one of the two spellings has to change meaning, and K-065 says a save
   is a save.
3. **Menu hygiene loses to honesty.** A mode switch that silently re-scales an existing
   slider is the kind of control that reads fine in a menu and wrong in a project file. Two
   small effects that each do one thing is what §3's shape rule (K-090) asks for anyway.

So: **Brightness** is its own effect in the **Colour** category, carrying both of AE's
sliders under AE's names and AE's neutral point. §3.18 Contrast is untouched.

**Algorithm sketch.** One affine grade per RGB channel about the same mid-grey pivot
Contrast uses:

```
b = Brightness ÷ 100      # ±1.0 of scene-linear light at full deflection
k = 1 + Contrast ÷ 100    # 0 flattens to grey, 2 doubles the spread
out = (u + b − 0.5)·k + 0.5
```

Both scalars are computed host-side so the two paths multiply by identical numbers. Affine,
so — like Contrast — it declares `alpha mode: unpremultiplied` (§2.2) and the host wraps
the premultiply round trip. Alpha is untouched, highlights are never clipped. `cheap` cost,
`Exact` ROI.

Brightness 0 with Contrast 0 short-circuits to the bit-exact identity on both paths, and
Mix 0 likewise. A neutral default is the grade family's sanctioned exception to the
"no no-op default" rule (§3.10: a grade's tasteful default is a preset choice), the same
one Exposure, Contrast and Gamma take.

**The Matte pulls both controls toward neutral (K-426, §2.6):** Brightness toward 0 and
Contrast toward 0 per pixel before the grade, which with both set is not a fade of the
finished grade (the offset rides through the scaled contrast).

### 3.33 Hue and saturation — the master and the six colour ranges

**Parameters:** seven groups of three. **Master** (open) and six colour ranges —
**Reds**, **Yellows**, **Greens**, **Cyans**, **Blues**, **Magentas** (collapsed) — each
carrying **Hue** (a dial, degrees, default 0, wrapping), **Saturation** (per cent, default
0, slider −100..+100, hard min −100 and unbounded above) and **Lightness** (per cent,
default 0, same range). Plus Mix. The existing one-knob **Hue shift** (§3.17) stays: it is
a constant-luminance matrix rotation and this is not.

**Algorithm sketch.** On unpremultiplied scene-linear colour, through HSV rather than a
matrix, because "the reds" is a statement about hue and there is no linear operator for it:

```
v = max(r,g,b);  s = (v − min(r,g,b)) ÷ v  (0 when v ≤ 0);  h = the usual HSV hue
w_i  = max(0, 1 − wrapped_distance(h, 60·i) ÷ 60) · s        # i = 0..5, red at 0°
h' = h + Master.Hue        + Σ w_i · Range_i.Hue
s' = clamp(s · (1 + (Master.Saturation + Σ w_i · Range_i.Saturation) ÷ 100), 0, 1)
v' = max(v · (1 + (Master.Lightness   + Σ w_i · Range_i.Lightness)   ÷ 100), 0)
```

Three things in that are decisions rather than arithmetic:

- **The six weights are hat functions 120° wide, centred every 60°, so they sum to exactly
  1** for any hue: a colour sitting between Reds and Yellows takes a proportion of each and
  never more than one range's worth in total. No range edges to tune, no discontinuity as a
  hue drifts across a boundary — which is the artefact a hard band would put on a gradient.
- **The range weights are scaled by the pixel's own saturation.** A neutral grey has no
  hue — the formula returns 0°, which is red — so an unweighted Reds lightness would darken
  every grey in the frame. Weighting by `s` means a grey takes the Master adjustment alone,
  which is what "the reds" means to a person. AE does not do this; the picture is better for
  it and the divergence is reported as a mapped conversion.
- **Lightness is a gain on V, not a fade toward white.** Scene-linear has no white to fade
  toward (§2.1), so +100 doubles the value and −100 takes it to black. Monotone,
  HDR-honest, and continuous where a toward-white lerp would clip.

V is unbounded above throughout, so HDR values keep their headroom; S is genuinely 0..1 and
is clamped there. Unpremultiplied (§2.2). `cheap` cost, `Exact` ROI. All twenty-one
adjustments at zero short-circuits to the bit-exact identity on both paths; Mix 0 likewise.

**Not in v1: Colourise.** AE's Hue/Saturation carries a Colorize switch with its own three
controls that discards the source hue entirely. That is a different effect wearing the same
panel — §3.24 Tint already maps luma to colour, which is the same picture by a shorter
route — so it is not built until someone wants the exact AE behaviour; the import reports
a colourised instance rather than approximating it.

**The Matte scales every range's Hue, Saturation and Lightness toward 0 (K-426, §2.6):**
applied to the pixel's summed adjustment, which is the same number as scaling all twenty-one
controls first, so a grey matte turns the hue part of the way rather than fading a turned
colour over the original.

### 3.34 Fill — flood the alpha with one colour

**Parameters:** Colour (scene-linear RGBA, default opaque white, per-channel range
0..4 so an HDR fill is typable), Mix.

**Algorithm sketch.** The layer's own coverage decides the shape; the effect only decides
the colour:

```
out.rgb = colour.rgb · a          # a is the pixel's existing alpha
out.a   = a                       # untouched
```

The source colour is never read, which is the whole of the effect: a shape layer, a
titled text layer or a keyed matte becomes a flat colour with its edges — antialiasing,
feather, motion blur — intact, because the alpha it was already carrying is what
multiplies through. Working directly on **premultiplied** values (§2.2) is not an
optimisation here but the correct arithmetic: `colour · a` *is* the premultiplied form of
"this colour at this coverage", so there is nothing to unpremultiply and no round trip
to lose precision in.

The colour's own alpha lane is ignored, as it is on every colour parameter in the
catalogue (§3.21, §3.29): a colour says what light, the layer says how much of it.

`trivial` cost, `Exact` ROI. There is no neutral short-circuit and none is wanted — a
Fill that changed nothing would be a Fill that had not been applied. **Mix 0 is the
bit-exact identity**, which is the way to fade a fill in, and the way an imported AE
Fill's Opacity arrives (AE's Opacity and Lumit's Mix are the same number; §3.34 does not
carry both, because two controls that multiply into one another is a control too many).

**Not in v1: AE's Fill Mask, Invert, and the two Feather controls.** All four exist in AE
because AE's Fill can be aimed at *one mask* of the layer rather than the layer's whole
alpha, and then needs its own softening because a mask edge is hard. Lumit has no
per-mask effect targeting; the same pictures are reached by putting the Fill on the layer
that carries the shape, and softness is a Gaussian blur (§3.8) away. The import maps a
whole-alpha Fill exactly and reports a mask-targeted one.

### 3.35 Gradient — the linear and radial two-colour ramp

**Parameters:** Shape (Linear / Radial, default Linear), Start x and Start y (px@comp,
default 960, 0), Start colour (default opaque white), End x and End y (px@comp, default
960, 1080), End colour (default opaque black), Scatter (per cent, default 0, slider
0..100, hard 0..100), Seed, Mix.

**Algorithm sketch.** A generator: it replaces the frame rather than grading it, so it
writes **opaque** pixels edge to edge, exactly as AE's Ramp does on a solid.

```
Linear:  t = ((p − start) · (end − start)) ÷ |end − start|²
Radial:  t = |p − start| ÷ |end − start|
t = clamp(t + (hash01(seed, x, y) − ½)·scatter, 0, 1)
out.rgb = mix(start colour, end colour, t)
out.a   = 1
```

`p` is the pixel centre in raster pixels; the two points are declared `px@comp` so the
resolve step converts them and the generic rescale moves them together if the stack is
reused at another size (K-266) — a ramp that slid when the preview resolution changed
would be a ramp nobody could grade against.

Three decisions in that:

- **Degenerate points do not fault.** Start and End at the same place gives a zero-length
  axis; both reciprocals are floored host-side (`Gradient::packed`) against the same
  epsilon, so the ramp collapses to one flat colour rather than dividing by zero
  (docs/14 §4). Which colour is whichever end the collapsed `t` lands on — Start under
  Linear, where the projection is zero, and End under Radial, where the floored
  reciprocal saturates — and neither is worth a branch to make uniform, because both are
  "the ramp has no length" said in a picture.
- **Scatter is a per-pixel dither of `t`, not of the colour.** It is what AE's Ramp
  Scatter is for: a long, shallow ramp in 8-bit banding shows contour rings, and a small
  scatter breaks them without changing the ramp's shape or its ends. The hash is the
  catalogue's existing `block_hash01` fold (§2.4, shared with Block glitch and the noise
  core of §3.37), so it is stateless, seeded, and identical on both paths.
- **The ramp interpolates in the working space** — scene-linear, §2.1 — not in a
  display-referred one. A white-to-black ramp therefore looks "dark early" against an
  sRGB intuition and is *photometrically* even, which is what it must be if the ramp is
  going to drive another effect's matte.

`cheap` cost, `Exact` ROI, `seeded`. Mix 0 is the bit-exact identity.

### 3.36 Noise — per-pixel uniform or gaussian grain

**Parameters:** Amount (per cent, default 25, slider 0..100, hard min 0 and unbounded
above), Distribution (Uniform / Gaussian, default Uniform), Colour noise (default off),
Animate (default on), Seed, Mix.

Unlike §3.35 and §3.37 this is a **modifier**, not a generator: it adds grain to the
picture that arrived rather than replacing it.

**Algorithm sketch.** On unpremultiplied scene-linear colour (§2.2), re-premultiplied on
the way out:

```
n_c = 2·hash01(seed, c, x, y, tick) − 1                     # Uniform, −1..+1
n_c = (Σ of four such draws) ÷ 2                            # Gaussian, ~N(0, ⅓)
out_c = u_c + n_c · amount                                  # amount = Amount ÷ 100
```

- **Mono is the default**, matching AE: all three channels take the same draw, so the
  grain reads as luminance noise and does not tint the picture. **Colour noise** draws
  the three channels independently.
- **Gaussian is four uniform draws averaged**, not a Box–Muller pair. A sum of uniforms
  is the standard cheap normal, it is exact in the same integer hash both paths already
  share, and — decisively — it has *bounded support*, so a gaussian grain cannot produce
  the single wild outlier a true normal eventually will. Four draws is where the shape
  stops visibly improving.
- **Animate** puts the frame in the hash so the grain crawls, which is what grain does;
  turning it off freezes one draw, which is what a texture does. The tick is the layer's
  own time discretised to the millisecond and computed at resolve
  (`Noise::DERIVED_TICK`), so the kernel never sees a clock (§2.4) and two exports agree
  bit-for-bit. A millisecond is finer than any frame rate up to 1000 fps, which is the
  documented ceiling of "a fresh draw every frame".
- **Nothing is clipped.** AE's Noise carries a "Clip result values" switch because it
  grades display-referred integers; scene-linear has headroom (§2.1), so grain rides on
  top of a highlight instead of flattening it. The negative side is not clipped either —
  a channel driven below zero by grain is a legal scene-linear value and the compositor
  handles it — which is the one deliberate divergence from AE.

`cheap` cost, `Exact` ROI, `seeded`. Amount 0 short-circuits to the bit-exact identity on
both paths, and Mix 0 likewise.

### 3.37 Fractal noise — the seeded multi-octave generator

The utility texture half of AE-land is built from: clouds, smoke, turbulence maps,
displacement fields, wipe mattes, grunge. It lands before Turbulent displace because the
displacer reuses its noise core (`lumit_core::fx::noise`,
[impl/ae-effect-parity.md](impl/ae-effect-parity.md)).

**Parameters**, in panel order:

| Parameter | Kind | Default | Notes |
|---|---|---|---|
| Noise type | Value / Perlin | Perlin | the basis function |
| Fractal type | Basic / Turbulent | Turbulent | how the octaves fold |
| Invert noise | switch | off | `1 − n` after contrast and brightness; named for the row, not AE's bare "Invert", because §2.6's matte pair already puts an **Invert** at the foot of every panel |
| Contrast | per cent, 0..400, hard min 0 | 100 | about the mid-grey pivot |
| Brightness | per cent, −200..200 | 0 | added after contrast |
| **Transform** | | | |
| Rotation | dial, degrees | 0 | turns the noise field, not the frame |
| Uniform scaling | switch | on | one Scale, or a width and a height |
| Scale | px@comp, 1..2000, hard min 1 | 200 | the size of one noise cell |
| Scale width | px@comp, 1..2000, hard min 1 | 200 | used only when Uniform scaling is off |
| Scale height | px@comp, 1..2000, hard min 1 | 200 | ditto |
| Offset x, Offset y | px@comp | 960, 540 | where the field's origin sits |
| Complexity | whole number, 1..10 | 6 | octave count |
| **Sub settings** | | | |
| Sub influence | per cent, 0..100, hard min 0 | 60 | each octave's amplitude, as a share of the last |
| Sub scaling | per cent, 5..100, hard min 5 | 55 | each octave's cell size, as a share of the last |
| Evolution | dial, degrees | 0 | one full turn advances the field by one cell of depth |
| **Evolution options** | | | |
| Cycle evolution | switch | off | make Evolution loop |
| Cycle | whole number of revolutions, 1..30 | 1 | the loop length |
| Seed | seed | per instance | |
| Mix | per cent | 100 | |

**Algorithm sketch.** Each output pixel maps its own centre into the noise field, samples
a fractal sum there, and shapes the result:

```
q   = R(−rotation) · (p − offset)                   # p, offset in raster pixels
s   = (q.x ÷ scale_x, q.y ÷ scale_y, evolution ÷ 360)
n   = Σ over o in 0..complexity of  basis(seed, o, s.xy·freq_o, s.z) · amp_o  ÷  Σ amp_o
amp_o  = sub_influence^o      freq_o = (100 ÷ sub_scaling)^o
n01 = n·½ + ½                                       # n is −1..+1
out = clamp((n01 − ½)·contrast + ½ + brightness, 0, 1)      # then 1 − out if Invert
out.rgb = (out, out, out);  out.a = 1
```

`R(−rotation)` arrives as a host-computed cosine/sine pair, like every other rotation in
the catalogue: WGSL's trigonometry is not correctly rounded and carries no guarantee of
agreeing with Rust's, so the kernel never runs its own (§1.6, and the same reason
`transform_inverse` and `aperture_blades` are host-side).

Six decisions worth stating, because none of them is arithmetic:

1. **Scale is a length, not a per cent.** AE's Scale is a percentage of a private base
   size; §2.3 forbids a spatial control that does not survive a resize, and a per cent of
   something unnamed is exactly that. Lumit's Scale is **the size of one noise cell in
   px@comp**, so it rescales with everything else and a number means the same thing on
   every comp. The import converts AE's per cent through AE's base and reports it as a
   mapped conversion.
2. **Basic and Turbulent, and no more.** AE ships a dozen Fractal Types; all but two are
   the same sum with a different fold, and the two that carry the look are the signed sum
   (soft, cloud-like) and the folded `|n|` sum (ridged, smoke-like). Turbulent is the
   default because it is AE's, and because the folded sum is what a displacement map
   usually wants.
3. **Evolution is a third dimension, not a reseed.** The field is sampled in 3-D and
   Evolution is the depth coordinate, so animating it *moves through* the noise
   continuously — the difference between smoke drifting and smoke flickering. One full
   turn of the dial advances one cell, matching AE's revolutions.
4. **Cycle loops exactly, because depth is not scaled by frequency.** Every octave shares
   the one depth coordinate and is decorrelated by its octave number entering the hash
   instead. That is a deliberate divergence: it means Cycle *n* is a genuine seamless
   loop at every complexity (the lattice wraps at an integer number of cells, which a
   frequency-scaled depth could not guarantee), and it stops the fine octaves boiling
   faster than the coarse ones, which in AE they do.
5. **The output is clamped to 0..1 and says so.** §2.1's no-clipping rule is about not
   destroying a picture's highlights; this effect has no input picture, and a generator
   whose range is not 0..1 cannot be read as a matte without a second control to tame it.
   AE's Overflow dropdown (Clip / Soft clamp / Wrap back / Allow HDR) is the same
   admission by a longer route; Lumit ships Clip and nothing else until someone wants the
   others.
6. **It is opaque, edge to edge.** Like §3.35 and unlike §3.34, this replaces the frame:
   the layer's own alpha is not consulted and the output alpha is 1. Everything that
   wants noise *shaped* by something already has the answer — put it on the layer whose
   alpha is the shape and follow it with a Set matte, or drive it with a matte (§2.6).

`moderate` cost (up to ten octaves of 3-D noise a pixel), `Exact` ROI, `seeded`. Mix 0 is
the bit-exact identity; there is no other neutral point, and there should not be — a
Fractal noise that generated nothing would be one that had not been applied.

**Determinism** (§2.4) is load-bearing here and is tested as such: the lattice hash is the
same `splitmix32` fold Block glitch uses, run as identical wrapping `u32` operations on
both paths, and every float step is written in one arithmetic order in
`lumit_core::fx::noise` and mirrored op-for-op in `fx_fractal_noise.wgsl`. The §1.6
oracle holds the two to fp16 ULPs on the full parameter sweep, which is what "the noise
core the distort batch will reuse" has to be worth before anything reuses it.

**Not in v1:** Sub rotation and Sub offset (each octave turned or shifted relative to the
last), Perspective offset, and Centre subscale. Each is one more scalar through the same
loop and none of them changes what the effect *is*; they land when a real project asks
rather than to fill out a panel.

### 3.38 Turbulent displace — the fractal-driven warp

The distort family's anchor, and the reason §3.37 landed first: this effect steers the
picture with the noise core §3.37 generates (`lumit_core::fx::noise`, one implementation
and one WGSL twin for both). It is also the owner's own example of why §2.6's matte can
mean more than a dissolve — see the override below.

**Parameters**, in panel order:

| Parameter | Kind | Default | Notes |
|---|---|---|---|
| Displacement | Turbulent / Horizontal / Vertical | Turbulent | which components of the warp are used |
| Amount | px@comp, −500..500 | 50 | the farthest a pixel is pulled |
| Size | px@comp, 1..2000, hard min 1 | 100 | the size of one swirl |
| Complexity | whole number, 1..10 | 3 | octave count |
| Offset x, Offset y | px@comp | 960, 540 | where the noise field's origin sits |
| Evolution | dial, degrees | 0 | one full turn advances the field by one cell of depth |
| **Evolution options** | | | |
| Cycle evolution | switch | off | make Evolution loop |
| Cycle | whole number of revolutions, 1..30 | 1 | the loop length |
| Pinning | None / All edges / Left and right / Top and bottom | All edges | which edges are held still |
| Seed | seed | per instance | |
| Mix | per cent | 100 | |

**Algorithm sketch.** Each output pixel asks the noise field where to read from:

```
q  = (p − offset) ÷ size                          # p, offset in raster pixels
nx = fractal(seed_x, q, evolution ÷ 360)          # §3.37's core, Perlin + Turbulent
ny = fractal(seed_y, q, evolution ÷ 360)          # a second, decorrelated field
d  = (nx, ny)·amount                              # Horizontal: (nx, 0); Vertical: (0, ny)
d  = d · pin(p) · k                               # k is the matte's, see below
out = bilinear(src, p + d, Repeat edges)
```

Five decisions, none of them arithmetic:

1. **The noise is §3.37's, not a private copy.** The core is a module
   (`lumit_core::fx::noise`, mirrored once in `fx_noise_core.wgsl` and included by both
   kernels) exactly so this effect and Fractal noise cannot drift apart. Point a Fractal
   noise and a Turbulent displace at the same Seed, Size, Complexity and Evolution and
   the swirls line up with the pattern, which is what makes the two usable together.
2. **The sub settings are fixed at ½ and 2**, the textbook halving of amplitude and
   doubling of frequency, and are not panel rows. AE does not expose them on this effect
   either, and a warp is judged by its shape rather than by its spectrum: Amount, Size and
   Complexity are the three that change the picture.
3. **Two fields, not one field read twice.** The x and y displacements come from the same
   core under two host-derived seeds (`seed` and `seed ^ 0x5bf0_3635`). Reading one field
   at two nearby points instead — the cheap trick — correlates the two components, and a
   correlated warp slides diagonally rather than swirling.
4. **Pinning is a ramp `|Amount|` wide, not a hard clamp.** A pinned edge means the frame's
   border must not move, so the displacement is scaled down to zero across the last
   `|Amount|` pixels before it: no pixel can then be pulled from outside the frame near a
   pinned edge, and the pin costs nothing anywhere else. AE offers ten pinning
   combinations; the three that are not "some edges, some not" are here (all, the
   horizontal pair, the vertical pair) and the seven mixed ones are reported by the import.
5. **Amount is signed and is a length.** Negative simply reads the field the other way, and
   the number is px@comp (§2.3) rather than AE's per cent of an unnamed base — the same
   divergence, for the same reason, that §3.37 decision 1 records for Scale.

**The matte scales the displacement** (§2.6's override, K-395/K-399): `k` multiplies the vector
before the sample, so where the matte is grey the picture is *warped less*, and where it is
black the picture is untouched. This is not what the generic dissolve produces. A dissolve
blends a fully-warped picture back towards the unwarped one, which shows both — a ghost,
two overlapping copies of every edge. A scaled vector shows one edge, in a place between
the two. That difference is the whole reason §2.6 has an override at all, and it is tested
by picture (`crates/lumit-render/tests/distort_proof.rs`) as well as by ULP.

`moderate` cost, `PaddedPx(1000)` ROI (twice the Amount slider's own reach, its hard maximum being open), `seeded`. Mix 0
is the bit-exact identity, and so is Amount 0.

**Not in v1:** AE's Bulge, Twist and the three "Smoother" variants (each a different vector
field over the same noise), Resize Layer, and the mixed pinning combinations. None changes
what the effect is.

### 3.39 Tile — the frame repeated across itself

**Parameters:** Tile centre x and Tile centre y (px@comp, default the frame's own centre),
Tile width and Tile height (px@comp, default the frame's own size), Output width and
Output height (px@comp, default the frame's own size), Mirror edges (default off),
Phase (dial, degrees, default 0), Horizontal phase shift (default off), Mix.

**Algorithm sketch.** One rectangle of the picture is copied across the frame, into a
raster that may be **larger** than the one it read:

```
raster = (W, H) · max(output size ÷ (W, H), 1), capped at 8 192 a side   # K-542
origin = (raster − (W, H)) ÷ 2, whole pixels                 # where the frame sits in it
p      = the output pixel's position in the INCOMING frame's coordinates
tile   = (tile_width, tile_height)                           # px@comp, already raster pixels
window = (output_width, output_height)
outside the window, centred on the frame → transparent
u  = (p − tile centre) ÷ tile + ½                            # position in tiles
    without Horizontal phase shift: u.x += floor(u.y)·phase ÷ 360
    with it:                        u.y += floor(u.x)·phase ÷ 360
f  = u − floor(u)                                            # position within this tile
    Mirror edges: f.axis = 1 − f.axis on every odd tile index
out = bilinear(src, tile centre + (f − ½)·tile, Repeat edges)
out = orig·(1 − mix) + out·mix      # orig is transparent outside the incoming frame
```

Four notes:

- **The default is the identity, and it is AE's** (K-542, which reverses this section's
  earlier 2×2 default). One whole-frame tile, cut from the middle of the frame, stamped
  over exactly the frame it came from: dropping Tile on a layer changes not one bit.
  §1.2's "drop it on and it already looks right" is met by *looking unchanged* here, for
  the reason §3.5's Transform meets it the same way — Tile is the effect whose controls
  have no meaning until the user has said where the picture is being repeated **to**, and a
  2×2 grid nobody asked for is a picture nobody can undo by eye. The exactness matters as
  much as the value: the mapping is a divide followed by the multiply that undoes it, which
  fp32 does not always answer exactly, so both kernels short-circuit the identity rather
  than resampling through it. The centre default is the raster's own, filled by
  `instantiate_for_raster` — a fixed 960, 540 would shift the picture on any comp that is
  not 1080p, which is precisely what the identity forbids. **The four sizes are px@comp
  and get their defaults the same way** (K-558, which supersedes K-542's per-cent
  rationale for them): a size is a distance, so it is pixels, and a whole-frame tile is
  1920 × 1080 on exactly one comp — `instantiate_for_raster` writes the comp's own, so
  the default is still the exact identity anywhere and the *stored* number is honest
  pixels. A project saved before the conversion has its per cents scaled against the
  comp on load (schema v1 → v2), each axis against its own extent.
- **An output window wider than the frame grows the working raster** (K-542). This is the
  point of the control, and AE's: the copies land *past* the frame's edges, the working
  picture grows evenly on all four sides to hold them, and every effect after Tile in the
  stack runs on that wider picture — so a warp or a directional blur below it finds tiled
  material where a layer's edge used to be transparency. The composite then places the
  wider picture by the layer's own transform (the quad grows, the anchor slides with it),
  so not one of the original pixels moves. Below 100 % nothing grows: the window only
  clips, which needs no more room than the frame already has. The growth stops at 8 192
  pixels a side, so a slider dragged to five frames wide on a 4K comp cannot ask for a
  third of a gigabyte of working texture.
  **Three places cannot grow, and crop back instead**: an adjustment layer's stack (what
  follows blends it against the composite beneath, which is comp-sized by definition), a
  matte source's own stack, and a referenced layer's own stack. On those a Tile whose window
  is wider than the frame reads as the plain clipped tiling. A layer mask is grown into the same margin with
  nothing in it, so the copies Tile puts outside the layer are outside the mask.
- **Mirror edges flips alternate tiles** rather than butting copies together, which is what
  makes a tiled texture seamless without a seamless source. With Output width and height a
  little over 100 % this is the standard way to give a stabiliser or a warp material to eat
  into: mirrored edges, a wider raster, nothing moved.
- **Phase shifts every other row** (or column, with the switch) along by a fraction of a
  tile, which is how a tiled pattern stops reading as a grid. The rows shift by whole
  multiples of `phase ÷ 360` tiles, so 180° is the brickwork offset.

`cheap` cost, `FullFrame` ROI. Mix 0 is the bit-exact identity, and so is a fresh instance.

### 3.40 Offset — the frame slid, wrapping round

**Parameters:** Shift x and Shift y (px@comp, default 0), Mix.

**Algorithm sketch.**

```
s = (p − shift) mod (W, H)          # wrapped into the frame, both axes
out = bilinear(src, s, wrapped)
```

The frame is a torus: what leaves one side arrives at the other, so nothing is ever
revealed and no edge policy is needed or offered. That is the whole effect, and it is worth
having for one reason — it is how a seamless texture is repositioned without a seam, and
how a scrolling background is made out of one still.

**Its default is the identity**, as §3.39's now is: a shift is a displacement of the
picture, exactly as the Transform effect's (§3.5) is, and a displacement's neutral is zero.
There is no "already looks right" default for "how far", because how far depends on the
picture.

**AE names the same control differently.** Its "Shift Center To" is a destination point:
the pixel that ends up in the middle. Lumit stores a shift, which is that point minus the
frame centre, because a shift is what animates sensibly (a linear keyframe pair scrolls at
a constant speed) and because it is the same number twice otherwise. The import converts.

`cheap` cost, `FullFrame` ROI. Mix 0 and a zero shift are both the bit-exact identity.

**The Matte scales the shift (K-427, §2.6):** each pixel reads through `shift · k` for the
matte's strength at that pixel, so part of the frame can slide while the rest stays — a
shear or a smear of the wrap rather than the whole picture moving. The wrap is unchanged;
only how far each pixel reaches into it is.

### 3.41 Mirror — one half reflected onto the other

**Parameters:** Centre x and Centre y (px@comp, default 960, 540), Angle (dial, degrees,
default 0), Mix.

**Algorithm sketch.** A line through Centre at Angle cuts the frame in two; the side the
angle points to is replaced by the reflection of the other side:

```
n = (cos angle, sin angle)          # host-computed, §1.6
d = (p − centre) · n
s = p − 2·d·n   when d > 0,  else  p
out = bilinear(src, s, Transparent edges)
```

Two notes:

- **A positive angle turns the axis clockwise on screen**, because the raster's y grows
  downward. Every angle in the catalogue reads the same way (§3.6, §3.8), so this is a
  statement about the frame rather than about this effect.
- **The reflection may reach outside the frame** — put the centre near an edge and the
  reflected half has nowhere to come from. Those pixels are transparent, not stretched:
  a repeat there would smear the border pixel into a fan, which reads as a fault rather
  than as a mirror.

`cheap` cost, `FullFrame` ROI. Mix 0 is the bit-exact identity; there is no other neutral,
since a mirror through the frame centre is what the effect is for.

### 3.42 Lens distort — barrel and pincushion, by field of view

Maps AE's Optics Compensation ([11-AE-IMPORT.md](11-AE-IMPORT.md)), and carries its
control: the distortion is described by **the frame's field of view**, not by an abstract
coefficient, because that is the number a shot actually has.

**Parameters:** Field of view (degrees, 0..160, hard max 179, default 40), Reverse (default
off), Orientation (Horizontal / Vertical / Diagonal, default Horizontal), Centre x and
Centre y (px@comp, default 960, 540), Edges (Transparent / Repeat / Mirror, default
Transparent), Mix.

**Algorithm sketch.** The two mappings are an exact inverse pair — a fisheye added, and the
same fisheye removed:

```
half = ½ · (W | H | √(W² + H²))          by Orientation, raster pixels
f    = half ÷ tan(fov ÷ 2)               host-computed focal length, raster pixels
r    = |p − centre|
Reverse off:  r' = f · tan(min(r ÷ f, 89°))      # add the fisheye — barrel
Reverse on:   r' = f · atan(r ÷ f)               # remove it — pincushion
s    = centre + (p − centre)·(r' ÷ r)            # r = 0 samples the centre
out  = bilinear(src, s, edges)
```

Four things that are decision rather than derivation:

- **Field of view means the frame's rectilinear field of view.** `f = half ÷ tan(fov ÷ 2)`
  says exactly that: at Orientation Horizontal, a 40° field of view is a frame whose width
  spans 40° of the world. That is why the control is an angle rather than a `k₁`
  coefficient — a shot's lens has a field of view written on it, and its `k₁` does not
  exist until someone fits one.
- **Reverse is the true inverse, not a sign flip.** `tan` and `atan` invert one another
  exactly, so a Lens distort and a reversed Lens distort at the same Field of view,
  Orientation and Centre return the picture (to sampling error). A negated coefficient
  would not, which is what makes the pair worth having over one signed slider.
- **Orientation decides which half-extent the angle spans**, and therefore how much
  distortion a given number produces on a wide frame. Horizontal is AE's default and the
  one a lens specification means.
- **The two transcendentals are per pixel**, which §1.6 usually forbids — the catalogue's
  rotations arrive as host-computed cosine/sine pairs precisely because WGSL's trigonometry
  is not correctly rounded. Here the angle *is* a function of the pixel and cannot be
  lifted out, so the divergence is admitted rather than hidden: the two paths compute the
  same function of the same input, they differ by the platforms' own `tan`, and the §1.6
  oracle runs on a smooth corpus where a sub-thousandth of a pixel of sampling error stays
  inside the fp16 tolerance. A hard-edged corpus would not, and would be measuring the
  platform's libm rather than this kernel. K-399 records the measurement and the rule.

`moderate` cost, `FullFrame` ROI. Mix 0 is the bit-exact identity, and so is Field of view
0 (the kernel short-circuits rather than dividing by a zero tangent).

**The Matte scales the distortion (K-427, §2.6):** the sample position is pulled back toward
the pixel's own centre by the matte, read where the pixel lands, so the Field of view's
effect fades toward the identity as the matte darkens — the bend is genuinely weaker there,
not a fully bent frame dissolved over a straight one. A black matte is the untouched
picture.

**Not in v1:** AE's Resize / Optimal Pixels, which grow the layer's own bounds. Lumit
renders effects at the frame's raster (§2.3) and has no per-effect resize; the same picture
is reached by scaling the layer down inside a larger comp.

### 3.43 Drop shadow — the layer's own shape, cast behind it

The most-used effect in After Effects, and the shape of it is not in doubt: the layer's
alpha, softened, tinted, moved, and drawn **underneath** the layer rather than over it.

**Parameters:** Shadow colour (scene-linear RGBA, default opaque black, per-channel range
0..4), Opacity (per cent, 0..100, default 50, hard 0..100), Direction (dial, degrees,
default 135), Distance (px@comp, 0..500, default 12, hard min 0), Softness (px@comp,
0..250, default 8, hard min 0), Shadow only (default off), Mix.

**Algorithm sketch.**

```
offset  = distance · (sin θ, −cos θ)          # host-computed sin/cos, raster px
soft    = gaussian(src, softness, Transparent edges)   # the shared §3.8 kernel
k       = soft.a sampled at (p − offset)      # bilinear, transparent outside
shadow  = (colour.rgb · opacity · k,  opacity · k)     # premultiplied
out     = src + shadow · (1 − src.a)          # src OVER shadow: the shadow is BELOW
Shadow only: out = shadow
out     = orig·(1 − mix) + out·mix
```

Four things are decision rather than derivation:

- **Direction is measured from straight up and turns clockwise**, which is AE's convention
  and the one that reads as a light direction: 135° is the default down-and-right shadow.
  This is the only angle in the catalogue whose zero is not the +x axis, and it is
  deliberate — §3.41's rule is that a positive angle turns *clockwise on screen*, which
  this obeys; where the turn begins is the effect's own business, and a shadow's begins
  where a light's does.
- **The blur and the offset commute, so the blur is taken once on the source.** A
  translation and a convolution can be applied in either order, so the kernel softens the
  frame where it stands and then reads the softened alpha at the shifted position. That is
  one gaussian instead of one gaussian plus a resample, and it is exactly the same picture.
- **The shadow goes under the source, not over it.** That is the whole reason the effect
  exists as an effect rather than as a duplicated layer, and it is why the composite is
  written `src + shadow·(1 − src.a)` — premultiplied "source over shadow", with the shadow
  as the destination.
- **Softness is a radius, not a per cent of the distance.** AE's Softness is the same
  gaussian this reaches for; keeping it an independent length means a shadow can be moved
  without changing how sharp it is, which is what animating one usually wants.

`moderate` cost (it carries a gaussian), `FullFrame` ROI — the shadow reaches Distance +
Softness outside every edge of the layer's shape and there is no honest smaller bound
without reading both sliders. Mix 0 is the bit-exact identity, and so is Opacity 0.

**The Matte scales the shadow's Opacity (K-428, §2.6),** read where the shadow *falls*
rather than where the shape stands — paint the matte over the wall and it is the shadow on
the wall that goes. The blur is left unmatted: it is taken where the shape is, and a
per-pixel softness there would be a softness of the wrong picture.

**Not in v1:** AE's "Shadow Only" is here; its per-mask targeting is not, for §3.34's
reason. Multiple shadows are multiple instances, which is what AE users do anyway.

### 3.44 Set matte — another layer's channel becomes this layer's alpha

**This effect is a K-395 matte consumer by nature**, and that resolves the open question
[impl/ae-effect-parity.md](impl/ae-effect-parity.md) recorded: Set matte lives in
**Utility** *and* its source is the universal Matte row, because those were never two
answers. Its matte does not scale a strength — it **is** the alpha, which is the sixth
override in §2.6's table (K-400).

**Parameters:** Matte (the universal layer row, §2.6), Invert (the row's own switch),
Channel (Luminance / Alpha / Red / Green / Blue, default Luminance), Combine with existing
alpha (default off), Mix.

**Algorithm sketch.** The matte layer is rendered alone at this raster like every other
matte (K-387), so no stretching or alignment step exists or is needed:

```
k  = channel(matte at p)               # by the Channel row, on straight values
k  = 1 − k                             # if Invert
a' = k                                 # or  src.a · k  with Combine
out.rgb = unpremultiply(src).rgb · a'
out.a   = a'
out = orig·(1 − mix) + out·mix
```

Three notes:

- **It runs unpremultiplied** (§2.2), and that is not a nicety: this effect's whole job is
  to change coverage without changing colour, and multiplying a premultiplied value by a
  new alpha would change it twice. The unpremultiply/re-premultiply is fused into the one
  pass, as §2.2 permits.
- **Luminance is the default channel, where AE's is the alpha.** A layer picked as a matte
  is very often an opaque grey picture — a Fractal noise, a ramp, a luma pass — whose alpha
  is 1 everywhere, and an effect that did nothing until a second control was also changed
  is the no-op default §1.2 forbids. The import writes the value, so nothing is lost.
- **Combine with existing alpha** intersects rather than replaces (AE's "Composite Matte
  with Original"): the layer keeps its own edge and is further cut by the matte. Off by
  default, because "set" is what the effect is called.

`trivial` cost, `Exact` ROI. **An unset Matte row is the labelled no-op** every
layer-input effect follows (§1.2) — the one sanctioned exception to the no-op-default
rule, since a layer the user must supply cannot have a tasteful default. Mix 0 is the
bit-exact identity.

**This effect carries no universal Matte row (K-429, §2.6).** The row above is its **own**
source picker and always was in spirit: every other effect's Matte answers "how much of me
happens here", and Set matte has no answer to give, because what it takes from another layer
is the coverage itself rather than an amount of it. It used to claim the universal row
(K-395/K-400); it now declares its own, on the ordinary auxiliary-layer carriage beside
Light wrap's Background and Texturize's Texture, so no dissolve stands beside the kernel,
the Channel above is the only channel pick there is, and Invert is applied once, inside the
kernel. The stored ids are unchanged — a save is a save (K-065) — and a project saved
before the change loads exactly as it did (K-258).

**Not in v1:** AE's "Stretch Matte to Fit" (Lumit renders the matte at this raster, so
there is nothing to stretch) and "Premultiply Matte Layer" (Lumit's compositing is
premultiplied throughout, §2.1, so the question does not arise).

### 3.45 Channel blur — a gaussian per channel

**Parameters:** Red blur, Green blur, Blue blur and Alpha blur (all px@comp, 0..500, hard
0..2000; defaults 0, 0, 40, 0), Repeat edge pixels (default on), Mix.

**Algorithm sketch.** The separable gaussian of §3.8, four times over, with four radii:

```
for each channel c:  σ_c = max(radius_c ÷ 2, 1e-3),  taps r_c = ceil(radius_c)
horizontal pass:  out_c = Σ src_c(x+i) · exp(−½(i ÷ σ_c)²)  ÷  Σ weights,  i ∈ −r_c..r_c
vertical pass:    the same down the column, then Mix against the untouched input
r_c = 0 takes the centre sample untouched — an unblurred channel is bit-exact
```

Three notes:

- **The weights are built in the loop, not from one table.** §3.8's plain gaussian
  precomputes a single normalised kernel because every channel shares it; here they do not,
  so both paths accumulate unnormalised and divide at the end — the same arrangement
  §2.6's matted blur uses, and for the same reason.
- **Blue defaults to 2 % and the rest to zero.** A real sensor resolves blue worst, so
  softening blue alone is both the effect's commonest single use and instantly legible as
  "this did something" (§1.2). It is also the cheapest honest default: three of the four
  channels are untouched, so a fresh instance costs one channel's gather.
- **Repeat edge pixels, not the three-way Edges enum.** AE's control is a switch and so is
  this one: on holds the border pixel outward (a bright edge does not darken), off lets the
  frame fall away into transparency. Depth of field's row (§3.22) is the precedent.

`moderate` cost, `PaddedPx(2000)` ROI — the largest radius any one channel can reach.
Mix 0 is the bit-exact identity, and so are four zero radii.

**The Matte scales all four radii (K-426, §2.6):** Gaussian blur's own override four times
over — each channel's blur is genuinely narrower where the matte is grey, both passes reading
the destination pixel's matte.

### 3.46 Linear wipe — a straight edge swept across the frame

**Parameters:** Wipe centre x and Wipe centre y (px@comp, default 960, 540), Completion
(per cent, 0..100, default 50, hard 0..100), Wipe angle (dial, degrees, default 90),
Feather (px@comp, 0..500, default 0, hard min 0), Mix.

**Algorithm sketch.** A straight edge, perpendicular to Wipe angle, swept from one side of
the frame to the other; everything behind it is gone:

```
n       = (sin θ, −cos θ)                    # host-computed; θ = 0 points up the screen
d       = (p − centre) · n                   # signed distance along the sweep, raster px
extent  = ½·(|W·n.x| + |H·n.y|)              # half the frame's reach along n
band    = max(feather, 1e-3)
edge    = −(extent + band÷2) + c·(2·extent + band)      # c = completion ÷ 100
keep    = clamp((d − edge) ÷ band + ½, 0, 1)
out     = src · keep                         # premultiplied: all four channels
out     = orig·(1 − mix) + out·mix
```

Three notes:

- **Completion defaults to 50, where AE's is 0**, for §1.2's reason: an effect whose
  default state has removed nothing is an effect that has not been applied. (§3.39 used to
  be cited here as the precedent; K-542 turned that one back to the identity, for a reason
  that does not reach a wipe — a wipe's control means something the moment it is dragged,
  a tiling's does not until there is somewhere to tile to.) Feather
  keeps AE's 0, because one divergence is enough to make the effect visible and a second
  would be taste imposed on a control that has a right answer of its own.
- **The edge travels half a feather past each end**, which is what the `± band÷2` in the
  `edge` expression buys: at Completion 0 the whole frame is kept **bit-exactly**, and at
  100 the whole frame is gone. Without it the last pixel row would sit at half strength at
  either extreme, and a wipe that cannot fully finish is not a transition.
- **Wipe angle turns clockwise from straight up**, matching §3.43 and AE: at 90° the edge
  is vertical and the **left** of the frame goes first; at 0° the top goes first.

`trivial` cost, `Exact` ROI. Mix 0 and Completion 0 are both the bit-exact identity.

**The Matte scales Completion per pixel (K-429, §2.6),** which is what turns a wipe into a
**gradient wipe**: the edge is further along where the matte is bright, so a grey ramp
sweeps the frame in the ramp's own shape rather than the schema's straight line, and a black
matte holds the frame back entirely.

### 3.47 Radial wipe — a hand sweeping round a clock

**Parameters:** Wipe centre x and Wipe centre y (px@comp, default 960, 540), Completion
(per cent, 0..100, default 50, hard 0..100), Start angle (dial, degrees, default 0), Wipe
(Clockwise / Anticlockwise / Both, default Clockwise), Feather (px@comp, 0..500, default 0,
hard min 0), Mix.

**Algorithm sketch.** One formula, three modes — the mode only moves where the removed
wedge is centred:

```
φ       = atan2(p.y − cy, p.x − cx) + ½π      # angle from straight up, clockwise (y grows down)
r       = |p − centre|
band    = clamp(max(feather, 1e-3) ÷ max(r, 1), 1e-4, π)     # a constant-width soft edge
hw      = c·(π + band) − band÷2               # the wedge's half-width, c = completion ÷ 100
mid     = start + hw·dir                      # dir = +1 clockwise, −1 anticlockwise, 0 both
u       = hw − angular_distance(φ, mid)       # > 0 inside the wedge
keep    = clamp(½ − u ÷ band, 0, 1)
out     = src · keep
out     = orig·(1 − mix) + out·mix
```

where `angular_distance(a, b)` wraps `a − b` into −π..π by `d − 2π·floor(d ÷ 2π + ½)` and
takes its magnitude. **`floor(x + ½)`, never `round`** — Rust rounds halves away from zero
and WGSL rounds them to even, and one pixel a frame landing on the wrong side of a wedge is
exactly the kind of disagreement §1.6 exists to catch.

Four notes:

- **The three modes are one expression.** Clockwise and Anticlockwise put the wedge's
  middle a half-width to either side of Start angle; Both leaves it *on* Start angle, so
  the wedge opens symmetrically. All three remove the same fraction of the circle at the
  same Completion, which is what makes the control mean one thing.
- **`hw` carries the same half-band lead-in as §3.46**, and for the same reason: Completion
  0 must be the bit-exact identity and 100 must be empty, whatever the feather is.
- **Feather is a width in pixels, measured at the arc**, so a soft edge stays the same
  thickness as it sweeps out from the centre rather than fanning open. Near the centre the
  angle that width subtends grows without bound, so it is clamped at π — the middle few
  pixels of a heavily feathered wipe are mush, which is true of AE's and of any polar
  feather.
- **One `atan2` per pixel** — §3.42's admission again, and recorded by the same decision
  (K-399): the angle *is* a function of the pixel and cannot be lifted host-side. The §1.6
  oracle is judged on absolute difference for it.

`cheap` cost, `Exact` ROI. Mix 0 and Completion 0 are both the bit-exact identity.

**The Matte scales Completion per pixel (K-429, §2.6):** the hand has swept further where
the matte is bright, so a grey ramp opens the wedge unevenly and a black matte holds the
frame back.

AE's Venetian Blinds, Iris Wipe and Card Wipe landed in this category as §3.70–§3.72. The
rest of AE's Transition family (Block Dissolve, Gradient Wipe) is still Tier B
([impl/ae-effect-parity.md](impl/ae-effect-parity.md)).

### 3.48 Corner pin — the picture pulled onto four points

Maps AE's Corner Pin ([11-AE-IMPORT.md](11-AE-IMPORT.md)), and it is the import workhorse of
the distort family: every screen replacement, every sign, every phone in a shot arrives as
one of these.

**Parameters:** Upper left x/y, Upper right x/y, Lower left x/y, Lower right x/y (px@comp;
the schema defaults are a nominal 1080p keystone, and `instantiate_for_raster` puts a fresh
instance's four points on the actual comp — 5 % in and 5 % down at the top, flush at the
sides 5 % up from the bottom), Edges (Transparent / Repeat / Mirror, default Transparent),
Mix.

**Algorithm sketch.** The four points define a projective map (a homography) taking the
frame's own corners to them; the kernel walks the *inverse*, because rendering asks "where
did this output pixel come from", never "where does this input pixel go":

```
# host-side, once: the unit square → quad map (Heckbert's form)
d1 = UR − LR ;  d2 = LL − LR ;  d3 = UL − UR + LR − LL
den = d1.x·d2.y − d1.y·d2.x            # 0 ⇒ the quad is degenerate ⇒ the exact identity
g   = (d3.x·d2.y − d3.y·d2.x) ÷ den
h   = (d1.x·d3.y − d1.y·d3.x) ÷ den
M   = [ UR.x−UL.x+g·UR.x   LL.x−UL.x+h·LL.x   UL.x ]
      [ UR.y−UL.y+g·UR.y   LL.y−UL.y+h·LL.y   UL.y ]
      [ g                  h                  1    ]
N   = adjugate(M), sign-normalised so N₂₂ > 0     # the inverse, up to a scale that cancels

# per pixel
(u', v', w') = N · (p.x, p.y, 1)
w' ≤ 0  →  transparent                 # the pixel is behind the projection's horizon
s   = (u' ÷ w' · W,  v' ÷ w' · H)
out = bilinear(src, s, edges)
out = orig·(1 − mix) + out·mix
```

Five things that are decision rather than derivation:

- **Four points, not a matrix.** A homography has eight degrees of freedom and so do four
  points; the points are the ones a person can drag, keyframe and read back. The matrix is
  derived at dispatch and never stored.
- **The inverse is taken host-side and its scale is dropped.** A homography is defined only
  up to a scale, so the adjugate *is* the inverse — no determinant division, no per-pixel
  cost, and one fewer place for the two paths to round differently (§1.6).
- **`w' ≤ 0` is transparent, not wrapped.** Pull two corners past each other and part of the
  frame lies *behind* the projection's horizon; the honest answer there is nothing at all. A
  renderer that ignores the sign draws a mirrored ghost of the picture, which is the classic
  corner-pin artefact and is never wanted.
- **A degenerate quad is the exact identity**, not a division by zero: three collinear
  corners, or two on top of one another, and the effect renders its input untouched
  ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) §4 — degrade, never fault).
- **Edges is Lumit's, not AE's.** AE's Corner Pin has no edge control and always leaves the
  outside transparent, which is the default here. Repeat is offered because a corner pin is
  also how a *camera tilt* is faked on a full-frame plate, and there the smear at the edge is
  wanted rather than a hole. §3.41's objection to repeat does not apply: that was a
  reflection with nothing behind it, this is a plate deliberately over-scanned.

`cheap` cost, `FullFrame` ROI. Mix 0 and a degenerate quad are both the bit-exact identity
(both short-circuit). The four points left on the frame's own corners return the picture to
within a last-bit rounding of the perspective divide, which is not the same claim and is not
made as one: the map is the identity but the sample still travels through a division.

**The Matte scales the pull from the corners (K-427, §2.6),** in the owner's words: the
matte multiplies the offset the handles set, so where the matte is black the pixel stays
where it was — the untouched picture, not a transparent one. It is read where the pixel
lands, so a soft matte pins part of the frame to its own corners while the rest travels to
the pinned quad. A pixel behind the projection's horizon stays transparent whatever the
matte says: there is no position to pull it back from.

**Not in v1:** AE's Bezier Warp and Mesh Warp, which bend the edges *between* the corners.
Both are Tier B ([impl/ae-effect-parity.md](impl/ae-effect-parity.md)). Nor is AE's
"expand output" — Lumit renders effects at the frame's raster (§2.3), as §3.42 records.

### 3.49 Displacement map — another layer's channels push this one

**This effect is a K-395 matte consumer by nature**, the seventh, and the second (after
§3.44) whose matte is the *subject* rather than a modifier: the layer on the Matte row **is
the map**. AE calls the same control "Displacement Map Layer"; Lumit already has one row that
names another layer and renders it at this raster, and a second picker beside it saying the
same thing would be a seam for nothing.

**Parameters:** Horizontal channel (Luminance / Alpha / Red / Green / Blue, default Red),
Horizontal amount (px@comp, −500..500, default 60), Vertical channel (default Green),
Vertical amount (px@comp, −500..500, default 60), Edges (Transparent / Repeat / Mirror,
default Repeat), Mix — plus the universal Matte row, which is the map, and its Invert.

**Algorithm sketch.**

```
m   = matte sampled at p            # this raster, one texel per pixel — no fitting, see below
kx  = channel(m, horizontal channel) ;  ky = channel(m, vertical channel)
Invert on:  kx = 1 − kx ;  ky = 1 − ky
s   = p + ((kx − ½)·2·amount_x,  (ky − ½)·2·amount_y)
out = bilinear(src, s, edges)
out = orig·(1 − mix) + out·mix
no matte bound → the exact identity
```

Five things that are decision rather than derivation:

- **Mid-grey is the neutral, ½ and not 0.** A map channel at 0.5 moves nothing, 1 pushes a
  full Amount one way and 0 a full Amount the other. That is AE's convention (its 128 of 255)
  and it is the only one that lets a single map push both ways — which is what makes a
  Fractal noise (§3.37) usable as a map with no grade in front of it.
- **The Amounts are lengths in px@comp**, not per cents of an unnamed base — §3.38 decision 5
  and §3.37 decision 1 again, and for the third time the same reason: a length survives a
  resize as §2.3 requires and a per cent of the layer does not. The import converts.
- **Signed Amounts, and one per axis.** Negative simply reads the map the other way on that
  axis. Two independent numbers rather than one, because the classic uses — a heat shimmer
  that only moves vertically, a glass ripple that only moves across — are exactly the ones a
  single control cannot express.
- **No map fitting, because there is nothing to fit.** AE's Displacement Map Behaviour
  (Centre Map / Stretch Map to Fit / Tile Map) exists because AE hands the effect the map
  layer at *its* size. The Matte row renders the referenced layer alone at **this** effect's
  raster (§2.6, [impl/layer-input.md](impl/layer-input.md)), which is "stretch to fit"
  already and is the only behaviour a matte has ever had here. The import reports the other
  two rather than approximating them.
- **Unset is the labelled no-op**, the sanctioned exception §1.2 grants a reference the user
  must supply — the same one §3.44 takes. A displacement map with no map is not a
  displacement.

`cheap` cost, `padded(1000 px@comp)` ROI — twice the Amount sliders' own reach, their hard maximum being open. Mix 0 is the bit-exact
identity, and so are both Amounts at 0 and an unbound Matte row.

**How it differs from Turbulent displace (§3.38),** since the two sit next to each other in
the menu: Turbulent displace *generates* its own field and the matte scales it; Displacement
map is *given* the field and has none of its own. Reach for §3.38 when the warp should be
procedural and animate by itself, and for this one when the warp has to line up with
something — a ripple map, a rendered normal pass, a hand-painted gradient, another layer's
luminance.

### 3.50 Polar coordinates — the frame bent into a circle, and back

**Parameters:** Conversion (Rectangular to polar / Polar to rectangular, default Rectangular
to polar), Interpolation (per cent, 0..100, default 100, hard 0..100), Mix.

**Algorithm sketch.** Both directions share one centre — the frame's own — and one radius
scale, **half the frame diagonal**, so that the whole picture is used and the whole frame is
covered:

```
centre = (W ÷ 2, H ÷ 2) ;  R = ½·√(W² + H²)

Rectangular to polar:                       # the output is polar: rows become rings
  d  = p − centre
  θ  = atan2(d.x, −d.y)                     # from straight up, clockwise
  q  = ( fract(θ ÷ 2π)·W,  |d| ÷ R · H )

Polar to rectangular:                       # the exact inverse: rings become rows
  θ  = p.x ÷ W · 2π ;  r = p.y ÷ H · R
  q  = centre + r·(sin θ, −cos θ)

s   = p + (q − p)·(interpolation ÷ 100)     # a morph, not a dissolve — see below
out = bilinear(src, s, Transparent edges)
out = orig·(1 − mix) + out·mix
```

Four things that are decision rather than derivation:

- **Interpolation moves the sample, Mix blends the picture.** They are not two names for one
  control. At Interpolation 50 every pixel is drawn from half-way along its own path into
  polar space, so the frame *bends* — the intermediate state of the transform. At Mix 50 the
  finished bend is laid over the untouched frame at half opacity and both are visible at
  once. AE's Interpolation is the first of those, and so is this one.
- **The radius spans half the diagonal.** Half the width or half the height would be simpler
  and both are wrong: with either, the corners of the frame sit outside the mapped disc and a
  "tiny planet" comes out with four bald corners. Half the diagonal reaches them, and costs
  only that the map's outermost rows are seen near the corners alone.
- **The angle starts at the top and turns clockwise** — the catalogue's convention (§3.43,
  §3.46, §3.47) — and here it also decides where the seam of a wrapped picture falls:
  straight up.
- **The two directions are an exact inverse pair**, like §3.42's Reverse. A Rectangular to
  polar followed by a Polar to rectangular at full Interpolation returns the picture to
  sampling error, and the §1.6 oracle proves it. That is what makes the pair worth having
  over one control with a sign.

`cheap` cost, `FullFrame` ROI. Mix 0 and Interpolation 0 are both the bit-exact identity.
Three transcendentals a pixel, which is §3.42's fourth note and K-399's rule again: the angle
is a function of the pixel, both paths run their own platform's `atan2`/`sin`/`cos`, and the
oracle is judged on absolute difference over a smooth corpus.

### 3.51 Twirl — the picture wrung round a point

**Parameters:** Angle (dial, degrees, default 90), Radius (px@comp, 0..2000, default 650, hard
min 0), Centre X and Centre Y (px@comp, default the frame centre), Mix.

**Algorithm sketch.**

```
d = p − centre ;  r = |d|
r ≥ radius → the exact identity
t = 1 − r ÷ radius                    # 1 at the centre, 0 at the rim
φ = angle·t²                          # squared: the rim eases out rather than creasing
s = centre + rotate(d, −φ)
out = bilinear(src, s, Transparent edges)
out = orig·(1 − mix) + out·mix
```

Three notes:

- **The falloff is squared, not linear.** A linear falloff leaves a visible crease at the
  rim, because the twist stops at a corner instead of easing out; `t²` has zero slope there,
  so the twirl blends into the untouched picture. AE's does the same and its manual does not
  say so.
- **A twirl samples inside its own circle**, always: a rotation about the centre preserves
  the radius, so no pixel inside the disc ever reads from outside it. The only samples that
  can leave the frame are the ones whose disc already hung over the edge, and those come out
  transparent.
- **One sine and cosine a pixel**, for §3.42's reason — the angle is a function of the radius
  and cannot be lifted host-side. K-399's metric applies.

`cheap` cost, `FullFrame` ROI. Mix 0, Angle 0 and Radius 0 are all the bit-exact identity.

**The Matte scales Angle (K-427, §2.6):** a half-grey matte on a 200° twirl draws the 100°
twirl, to the byte — the picture the control at half draws, which no dissolve of the 200°
picture can be. Read at the destination pixel, so the falloff over the disc is the matte's
own shape crossed with the radius ramp.

### 3.52 Spherize — a glass ball held over the picture

**Parameters:** Radius (px@comp, 0..2000, default 550, hard min 0), Bulge (per cent, −100..100,
default 100), Centre X and Centre Y (px@comp, default the frame centre), Mix.

**Algorithm sketch.** One pair of mutually inverse radial maps, blended by Bulge:

```
d = p − centre ;  r = |d| ;  ρ = r ÷ radius
ρ ≥ 1  or  r = 0 → the exact identity
bulge ≥ 0:  target = (2 ÷ π)·asin(ρ)         # magnify the middle — the glass ball
bulge < 0:  target = sin(ρ·π ÷ 2)            # its exact inverse — the pinch
ρ'  = ρ + (target − ρ)·|bulge| ÷ 100
s   = centre + d·(ρ' ÷ ρ)
out = bilinear(src, s, Transparent edges)
out = orig·(1 − mix) + out·mix
```

Four notes:

- **The two maps invert one another exactly**, `sin` and `asin` being what they are, so a
  Spherize at +100 and one at −100 at the same Radius and Centre return the picture (to
  sampling error). §3.42's Reverse is the same claim about a different pair, and it is worth
  making for the same reason: a sign flip on a coefficient would *not* invert.
- **Bulge is a per cent of that map, not a second radius.** 0 is the exact identity, 100 the
  full sphere; the values between are a lens being filled with water rather than a smaller
  ball. Keeping Radius and Bulge independent is what lets one be animated without the other,
  which is most of what a spherize is animated for.
- **The rim is where the compression lives.** `asin` has an infinite slope at 1, so the last
  few pixels before the rim carry a squeezed copy of everything the middle magnified away.
  That is what a glass ball looks like and it is deliberate; a formula that eased out
  smoothly there would read as a bulge in cling film.
- **AE's Spherize is one signed Radius in raster pixels.** Lumit splits it: the size of the
  ball is a length (px@comp, §2.3, so it survives a resize) and *which way it bends* is its
  own control, because a negative length is not a thing and a slider that passes through zero
  to mean "inside out" cannot also be resolution-independent. The import converts sign to
  Bulge and magnitude to Radius.

`cheap` cost, `FullFrame` ROI. Mix 0, Bulge 0 and Radius 0 are all the bit-exact identity.
One arc sine or sine a pixel — §3.42's fourth note, K-399's metric.

**The Matte scales Bulge (K-427, §2.6):** the matte multiplies Bulge toward 0 before the
map is blended, so the glass is genuinely shallower where the matte is grey — and the
Bulge-0 short-circuit above then catches a black matte, which is why it costs no
resampling.

### 3.53 Ripple — rings spreading from a point

Maps AE's Ripple ([11-AE-IMPORT.md](11-AE-IMPORT.md)).

**Parameters:** Radius (px@comp, 0..2000, default 650, hard min 0), Centre X and Centre Y
(px@comp, default the frame centre), Type (Symmetric / Asymmetric, default Asymmetric), Wave
height (px@comp, 0..200, default 10, hard min 0), Wave width (px@comp, 1..400, default 90, hard
min 1), Evolution (dial, degrees, default 0), Mix.

**Algorithm sketch.** One radial sine, inside a circle, under an envelope that is zero at
both ends of it:

```
d = p − centre ;  r = |d|
r ≥ radius  or  r = 0 → the exact identity
ρ    = r ÷ radius
env  = 27⁄4 · ρ·(1 − ρ)²             # 0 at the centre, 0 at the rim, exactly 1 at its peak
φ    = 2π·(r ÷ width − evolution ÷ 360)
n̂    = d ÷ r ;  t̂ = (−n̂.y, n̂.x)
Symmetric:   s = p + n̂·height·env·sin φ
Asymmetric:  s = p + (n̂·sin φ + t̂·cos φ)·height·env
out = bilinear(src, s, Repeat edges)
out = orig·(1 − mix) + out·mix
```

Four things that are decision rather than derivation:

- **The envelope is zero at the centre as well as at the rim, and it is normalised.** A
  ripple whose amplitude were flat in the middle would need a direction at `r = 0`, where
  there is none, and a frame with a pinched blob at the epicentre is what that looks like.
  `ρ(1 − ρ)²` removes the singularity exactly and is also the true shape of a spreading
  disturbance: the crests are strongest in a ring, not at the point the stone went in. It
  peaks at `ρ = ⅓` with the value `4⁄27`, so the factor `27⁄4` makes **Wave height literally
  the farthest a pixel moves** rather than a number the envelope quietly discounts.
- **Evolution replaces AE's Wave Speed, and it is an angle** (K-403). A control that means "cycles per
  second" reads the clock, and an effect that reads the clock is not deterministic (§2.4) —
  the same frame would render differently in a preview and an export. One turn of Evolution
  sends one whole wave outward, so an AE Speed of *s* is a linear Evolution keyframe of
  `360·s` degrees a second, which the import writes as two keyframes.
- **Asymmetric adds the tangential half of the same wave**, a quarter-turn out of phase, so a
  pixel travels a small circle rather than sliding in and out along the radius. That is what a
  particle of water actually does under a passing wave, and it is the difference between the
  two types: Symmetric is a lens breathing, Asymmetric is water.
- **Wave height and Wave width are px@comp** (§2.3), not per cents of an unnamed base, so a
  ripple survives a resize and a half-resolution preview matches the export — §3.37 decision
  1's reasoning again.
- **The edges repeat rather than fading**, as §3.38's and §3.54's warps do. A Radius wider
  than the frame's own half-height puts crests over the top and bottom borders, and a
  transparent edge there bites a hole out of the picture instead of rippling it.

`cheap` cost, `FullFrame` ROI. Mix 0, Radius 0 and Wave height 0 are all the bit-exact
identity. One sine and cosine a pixel, so K-399's metric applies: judged on absolute
difference over the smooth corpus.

**The Matte scales Wave height (K-427, §2.6):** the rings are shallower where the matte is
grey and flat where it is black, the envelope and the ring spacing untouched — so the water
can be still at one side of the disc and moving at the other.

### 3.54 Wave warp — a travelling wave across the frame

Maps AE's Wave Warp ([11-AE-IMPORT.md](11-AE-IMPORT.md)).

**Parameters:** Wave type (Sine / Square / Triangle / Sawtooth / Circle, default Sine), Wave
height (px@comp, −500..500, default 20), Wave width (px@comp, 1..2000, default 120, hard min
1), Direction (dial, degrees, default 90), Phase (dial, degrees, default 0), Pinning (None /
All edges / Left and right / Top and bottom / Left edge / Right edge / Top edge / Bottom
edge, default None), Mix.

**Algorithm sketch.** The wave *travels* along Direction and the picture *slides across* it —
the transverse wave, which is the one a flag makes:

```
dir  = (sin θ, −cos θ)                 # from straight up, clockwise (§3.43, §3.46, §3.47)
perp = (cos θ,  sin θ)                 # dir turned a quarter-turn clockwise on screen
t    = (p − frame centre)·dir ÷ width − phase ÷ 360      # in whole waves
w    = shape(t)                        # −1..1, see the table
s    = p + perp·height·w·pin(p)
out  = bilinear(src, s, Repeat edges)
out  = orig·(1 − mix) + out·mix
```

with the five shapes written on `f = t − ⌊t⌋`:

| Wave type | `shape(t)` |
|---|---|
| Sine | `sin 2πt` |
| Square | `f < ½ ? 1 : −1` |
| Triangle | `1 − 4·\|frac(t + ¼) − ½\|` |
| Sawtooth | `2f − 1` |
| Circle | `±√(1 − (4·frac(2f) − 1)²)`, the sign flipping each half wave |

Four notes:

- **Pinning is per edge, all eight of AE's combinations.** A pinned edge cannot move, so the
  whole displacement ramps to zero across the last `|Wave height|` pixels before it, measured
  to the outermost pixel centre; the factors multiply where two edges are pinned. §3.38 ships
  four of these combinations and reports the rest — this one ships all of them, because the
  ramp is per edge here rather than per axis and eight flags cost what two did.
- **Phase replaces AE's Wave Speed** (K-403), for §3.53's reason and with the same
  conversion.
- **The edges repeat rather than fading.** An unpinned wave carries the picture off the frame
  and something has to stand behind it; a transparent edge would put a hole where the wave
  crest was, which is never what a waving flag looks like. §3.38 makes the same choice.
- **Both lengths are px@comp** (§2.3), AE's being raster pixels; the import divides through.

`cheap` cost, `PaddedPx(1000)` ROI (twice Wave height's slider, its hard maximum being open). Mix 0 and Wave height 0 are both the bit-exact
identity. K-399's metric.

**The Matte scales Wave height (K-427, §2.6):** the slide is shorter where the matte is
grey — the wave's shape, width and speed are the host's and unchanged, only its amplitude
varies across the frame. The pinned edges keep their own ramp width, so a pinned border is
still exactly still whatever the matte says.

**Not shipped:** AE's Noise and Smooth Noise wave types (a wave shape that needs a seed is a
§3.37 field, and §3.38 is the effect that warps by one), its Warp Axis swap, and its
Antialiasing switch — Lumit resamples bilinearly everywhere (§2.2). All three are reported by
the import.

### 3.55 Bezier warp — the frame's four edges bent

Maps AE's Bezier Warp ([11-AE-IMPORT.md](11-AE-IMPORT.md)), and answers §3.48's "not in v1".

**Parameters:** the four corners — Upper left x/y, Upper right x/y, Lower right x/y, Lower
left x/y — and eight tangents in four collapsed groups (Top edge: Top left tangent x/y, Top
right tangent x/y; Right edge: Right top tangent x/y, Right bottom tangent x/y; Bottom edge:
Bottom left tangent x/y, Bottom right tangent x/y; Left edge: Left top tangent x/y, Left
bottom tangent x/y). All px@comp; the schema defaults are a nominal 1080p frame with its
tangents at the thirds, and `instantiate_for_raster` puts a fresh instance's twelve points on
the actual comp. Then Quality (whole numbers, 1..12, default 8) and Mix.

**Algorithm sketch.** The twelve points define four cubic Bezier curves — the frame's bent
edges — and the inside of the shape is the **Coons patch** they bound: the two horizontal
edges blended vertically, plus the two vertical edges blended horizontally, minus the bilinear
surface on the corners that would otherwise be counted twice.

```
T(u) = bezier(UL, TLtan, TRtan, UR)     B(u) = bezier(LL, BLtan, BRtan, LR)
L(v) = bezier(UL, LTtan, LBtan, LL)     R(v) = bezier(UR, RTtan, RBtan, LR)

S(u,v) = (1−v)·T(u) + v·B(u) + (1−u)·L(v) + u·R(v)
       − [ (1−u)(1−v)·UL + u(1−v)·UR + (1−u)v·LL + uv·LR ]
```

`S` maps the source frame onto the warped shape, and rendering needs the other direction, so
each output pixel **solves** `S(u,v) = p` by Newton's method from its own position:

```
(u, v) = (p.x ÷ W, p.y ÷ H)                          # the identity patch's own answer
repeat Quality times:
    F = S(u,v) − p
    J = [ ∂S/∂u  ∂S/∂v ]                             # both analytic — cubics differentiate
    (u, v) −= J⁻¹·F                                  # |det J| ≤ ε ⇒ stop, the patch folded
u ∉ [0,1] or v ∉ [0,1] → transparent
s = (u·W, v·H) ;  |s − p| < 10⁻³ px ⇒ s = p          # see the fourth note
out = bilinear(src, s, Transparent edges)
out = orig·(1 − mix) + out·mix
```

Four things that are decision rather than derivation:

- **Twelve points, and the four corners are §3.48's.** They carry the same names and the same
  units, so a Corner pin can be replaced by a Bezier warp without re-dragging anything, and a
  Bezier warp with its tangents at the thirds *is* an affine Corner pin. What Bezier warp adds
  is the bend between the corners; what §3.48 keeps is real perspective, which a patch cannot
  do. They are siblings, not one superseding the other.
- **Quality is Newton steps, not mesh subdivisions** (K-403). AE tessellates the patch and draws
  triangles, so its Quality buys smaller triangles. There are no triangles here — every pixel
  inverts the patch exactly — so the same slider buys convergence instead, and the default 8
  is well past the point where an ordinary warp stops moving. A folded patch (one whose
  Jacobian goes singular, which is a patch turned inside out) stops iterating rather than
  dividing by zero, exactly as §3.48's degenerate quad does.
- **Outside the patch is transparent, and the solve is *checked* before it is believed**
  (K-403). The warped shape is meant to be seen as a shape; an edge policy that repeated the
  border would fill the frame and hide the very thing the effect makes, and AE agrees. But
  "in range" is not enough on its own: outside the patch there is no answer, and an
  unchecked iteration wanders until it happens to land in `0..1`, which draws a scatter of
  stray opaque pixels across the empty part of the frame. One more patch evaluation asks
  whether the answer actually solves the problem, and a residual over a pixel is discarded.
- **A sample within a thousandth of a pixel of its own centre is snapped to it.** Newton
  returns the exact answer arithmetically and a number a hair off it in floating point, so an
  untouched region of a warped frame would otherwise be resampled — a whole picture of
  softening for a part of it nobody bent. §3.42's Field of view 0 and §3.52's Bulge 0
  short-circuit the same complaint at one setting each; this snaps it away everywhere, for one
  comparison and at a thousandth of a pixel, which is four orders of magnitude below anything
  a resampler could show.

`moderate` cost (Quality patch evaluations and Jacobians a pixel), `FullFrame` ROI. Mix 0 is
the bit-exact identity, and so — by the snap above — is the default patch. K-399's metric.

**The Matte scales the bend from the straight frame (K-427, §2.6),** in the owner's words:
the matte multiplies the offset the handles set, after the solve and its snap, so where the
matte is black the pixel stays where it was. It is read where the pixel lands. A pixel
outside the patch stays transparent whatever the matte says — there is no solution to pull
it toward.

### 3.56 Warp — the thirteen bend presets

Maps AE's Warp ([11-AE-IMPORT.md](11-AE-IMPORT.md)), which is Photoshop's.

**Parameters:** Style (Arc / Arc upper / Arc lower / Arch / Bulge / Flag / Wave / Fish / Rise
/ Fisheye / Inflate / Squeeze / Twist, default Arc), Bend (per cent, −100..100, default 50),
Horizontal distortion (per cent, −100..100, default 0), Vertical distortion (per cent,
−100..100, default 0), Mix.

**Algorithm sketch.** One kernel, thirteen maps. The frame is normalised to `−1..1` on each
axis, the style moves the sample there, the two distortions taper it, and the *difference* is
carried back to pixels — which is what makes Bend 0 the identity to the bit rather than to a
rounding of `(u + 1)⁄2·W`:

```
u = p.x ÷ (W⁄2) − 1 ;  v = p.y ÷ (H⁄2) − 1        # −1..1, v growing downward
a = bend ÷ 100 ;  d = 1 − u² ;  e = 1 − v² ;  r = √(u² + v²)
(u', v') = style(u, v)                            # the table below
u'' = u' ÷ (1 + vdist·v')                         # the two tapers, from the style's output
v'' = v' ÷ (1 + hdist·u')                         # each clamped to ±0.9, so no divide by 0
s   = p + ((u'' − u)·W⁄2, (v'' − v)·H⁄2)
out = bilinear(src, s, Transparent edges)
out = orig·(1 − mix) + out·mix
```

| Style | `style(u, v)` | The look |
|---|---|---|
| Arc | `v' = v + a·d` | the whole picture bows one way |
| Arc upper | `v' = v + a·d·(1 − v)⁄2` | only the top edge bows |
| Arc lower | `v' = v + a·d·(1 + v)⁄2` | only the bottom edge bows |
| Arch | `v' = v·(1 − a·d)` | top and bottom bow apart |
| Bulge | `u' = u·(1 − a·e⁄2)`, `v' = v·(1 − a·d⁄2)` | the middle swells on both axes |
| Flag | `v' = v + a·0.35·sin πu` | one wave across the width, rows in step |
| Wave | `v' = v − a·0.35·v·sin πu` | the same wave with the edges out of phase |
| Fish | `u' = u·(1 − a·e⁄2)` | the sides bow out and the ends taper |
| Rise | `v' = v + a·(u + 1)⁄2` | a diagonal lift, rising to the right |
| Fisheye | `k = 1 − a·(1 − ρ²)·0.6`, `(u,v)·k` | radial magnification, corners rounded |
| Inflate | `k = 1 − a·(1 − ρ)·0.6`, `(u,v)·k` | the same swell with a softer, rounder falloff |
| Squeeze | `v' = v·(1 + a·e)` | rows crowded toward the middle |
| Twist | `φ = a·π⁄2·v`, `(u·AR, v)` turned by `−φ` | the top turns one way, the bottom the other |

with `ρ = min(r, 1)`, and every unlisted component left alone.

Four notes:

- **The five swelling styles subtract their coefficient** (K-403). This is a gather — the map says
  where a pixel is *read from* — so pulling the sample inward is what makes the picture swell
  outward. Written the other way round, a positive Bend on a style called Bulge would pinch,
  which is the one thing a named preset may not do.
- **Bend 0 is the identity for every style**, by construction: each map is written so that
  `a = 0` leaves its argument untouched, and the sample is built from the *difference* rather
  than rebuilt from the normalised coordinate. That is the whole reason for the last line of
  the sketch.
- **The radial styles follow the frame's own ellipse, not a circle**, because both axes are
  normalised to `−1..1` separately. On a 16∶9 frame a Fisheye therefore reaches the left and
  right edges at the same Bend that reaches the top and bottom, which is what the style is
  for; a true circle would leave two bands untouched and read as a lens dropped on the middle
  — which is §3.52, and is the effect to reach for when that is what is wanted. **Twist is
  the one exception**, and has to be: an angle is not a shape, so it is taken in the
  isotropic space the aspect ratio `AR` buys, and a Bend of 100 really is a quarter turn at
  the top and bottom edges. Left elliptical it would be a colossal horizontal shear wearing
  a rotation's name.
- **Cousins are kept as cousins.** Bulge, Fish and Arch are one swell on two axes, on the
  horizontal, and on the vertical; Fisheye and Inflate differ only in the exponent of the
  falloff. Photoshop ships them as separate styles because people reach for them by name and
  by look, not by formula, and a menu that collapsed them into one style plus two switches
  would be a menu nobody could find "Flag" in.
- **The two distortions are perspective tapers, and they act on the style's output.** A style
  followed by a taper is a bent picture seen at an angle; a taper followed by a style is a
  bent trapezoid, which is not what either control is for.

`cheap` cost, `FullFrame` ROI. Mix 0 and Bend 0 with both distortions 0 are the bit-exact
identity. K-399's metric.

**The Matte scales Bend and both distortions (K-427, §2.6):** all three are multiplied by
the matte at the destination pixel before the style runs, so a grey matte is a shallower
bend seen at a shallower angle, and a black one is the identity the line above names.

**Not shipped:** AE's Warp Axis switch (which transposes the whole effect), and its Shell
Lower and Shell Upper styles. Both are reported by the import.

### 3.57 Roughen edges — the alpha edge chewed by a fractal

Maps AE's Roughen Edges ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Stylise** effect, not a
distortion: nothing inside the shape moves, only the shape's outline changes.

**Parameters:** Edge type (Roughen / Cut / Spiky, default Roughen), Border (px@comp, 0..500,
default 40, hard min 0), Edge sharpness (per cent, 0..100, default 70), Fractal influence (per
cent, 0..200, default 100, hard min 0), Scale (px@comp, 1..2000, default 100, hard min 1),
Offset x and Offset y (px@comp, default the nominal frame centre), Complexity (whole numbers,
1..10, default 2), Evolution (dial, degrees, default 0), Colour edge (default off), Edge
colour (default white, enabled by Colour edge), Seed, and — in a collapsed *Evolution options*
group — Cycle evolution and Cycle (1..30, default 1), plus Mix.

**Algorithm sketch.** Two passes. The first is **the shipped §3.8 gaussian**, at a radius of
Border, run on the picture: what comes back is a soft alpha whose `½` contour is the original
outline and whose gradient is Border wide, which is the distance field the roughening needs
and is a great deal cheaper than computing one. The second pass re-thresholds it, with the
threshold shifted per pixel by the §3.37 fractal field:

```
ā    = blurred alpha at p
n    = fractal(field, (p − offset) ÷ scale, evolution ÷ 360)      # −1..1
band = 1 − |2ā − 1|                     # 1 on the outline, 0 well inside or well outside
t    = ā + n·influence⁄2·band − ½
k    = smoothstep(−hw, +hw, t)          # hw = (1 − sharpness ÷ 100)⁄2, floored at 1⁄100
col  = src.rgb ÷ src.a                  # the pixel's own colour, straight
       (where src.a ≈ 0: blurred.rgb ÷ blurred.a — the neighbourhood's, so grown edges
        arrive with colour rather than black)
col  = col + (edge colour − col)·band   # only with Colour edge on
out  = (col, 1)·k                       # premultiplied by the new coverage
out  = orig·(1 − mix) + out·mix
```

Four things that are decision rather than derivation:

- **The blur is the distance field** (K-403). A real signed-distance transform is a second algorithm,
  two more passes and a great deal of care about ties; a gaussian at the radius you were going
  to chew anyway has a `½` contour in the same place and a gradient of exactly the right
  width, and it is a kernel that already ships. §3.43 reuses the same blur for the same reason
  and this is the second time it has paid.
- **Edge type is three shapes, and the colour is its own switch** (K-403). AE ships seven types, which
  are three shapes (Roughen, Cut, Spiky) times a colour flag, plus Photocopy. Multiplying two
  independent choices into one dropdown means neither can be animated or read on its own, so
  Lumit splits them: Roughen is the signed field smoothed by Edge sharpness, Cut is the same
  field with the sharpness ignored and the edge hard, Spiky folds the field about zero so its
  ridges become spikes. AE's Photocopy is Cut with Colour edge on, and the import says so.
  "Hard" means a hundredth of the alpha range, not a step: across a blurred edge of any real
  Border that is well under a pixel, so Cut arrives antialiased rather than stair-stepped —
  and a true step would also make the effect's own §1.6 oracle a coin toss on the pixels that
  land on the threshold, which is K-399's rule about a threshold on a coverage.
- **The band weights the noise, and that is what keeps the chewing at the edge.** Deep
  inside the shape the blurred alpha is 1, so `band` is 0 and the threshold cannot move at
  all — no amount of Fractal influence punches a hole in the middle of a solid layer, which
  a bare `ā + n⁄2` would do at the first octave that came back at −1. It also makes the
  chewing symmetric about the outline: the bite reaches out to where the band dies and
  stops, so **Border sets the band the roughening lives in** and the deepest bite is about
  half of it. The same band paints the edge colour, for the same reason and at no extra cost.
- **Border 0 is the exact identity**, by short-circuit — a zero-radius blur followed by a
  re-threshold would harden the picture's antialiasing for an effect the user has turned off,
  which is §3.52's complaint in another costume. **Fractal influence 0 is deliberately *not*
  the identity**: it is the outline re-cut at the `½` contour, which is a useful thing to ask
  for (it hardens a soft matte) and would be a lie to call "off".
- **Scale is px@comp**, not AE's per cent — §3.37 decision 1, a fourth time.

**The Matte scales Border (K-428, §2.6),** and it rides entirely on the gaussian: Border *is*
that blur's radius, and the matte already scales a radius per pixel (§3.8), so a grey matte
leaves a narrower ramp in the alpha and therefore a narrower band to chew. The outline is bitten
coarsely where the matte is white and finely — down to untouched — where it is dark, which no
dissolve can draw, since a dissolve only cross-fades one bite size over the whole shape.

`moderate` cost (one gaussian plus up to ten octaves a pixel), `PaddedPx(1000)` ROI (twice Border's slider, its hard maximum being open). Mix 0
and Border 0 are both the bit-exact identity. Seeded (§2.4): the field is a function of Seed
and Evolution and never of a clock, so the same frame roughens the same way in the preview and
in the export.

### 3.58 Posterize — the tone ladder cut into steps

Maps AE's Posterize ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Colour** effect.

**Parameters:** Levels (whole numbers, 2..64, default 8, hard min 2, hard max 255), Mix.

**Algorithm sketch.** Each RGB channel independently, on unpremultiplied colour (§2.2):

```
n    = Levels − 1
t    = √u                          # the perceptual position of the channel
out  = (⌊t·n + ½⌋ ÷ n)²            # the nearest rung of the ladder, back in light
```

Three things in that are decision rather than arithmetic:

1. **The rungs are spaced perceptually, not in light** (K-404). Scene-linear is a
   measurement of photons and the eye is not (§2.1): eight rungs spaced evenly in light put
   six of them above mid-grey, so a posterized picture would band its highlights to pieces
   and leave the shadows a smooth ramp. That is neither what a posterize is for nor what AE
   produces, AE quantising an 8-bit display value. Spacing the rungs evenly in a **square
   root** of the light puts them where a person sees them.
2. **The curve is √, not sRGB's 2.2** — and the reason is the oracle, not the picture. A
   quantiser's output is a *step*, so the two paths disagreeing by one bit on which side of
   a rung a value falls is a whole rung of colour, not a last-bit difference (K-399's rule
   about a threshold, arriving on a colour effect). `sqrt` is a single correctly-rounded
   instruction on both the CPU and the GPU; `pow(u, 1/2.2)` is a polynomial each vendor
   writes differently. Between a gamma of 2.0 and one of 2.2 there is no visible difference
   in where eight bands land, and between an exactly agreed answer and an approximately
   agreed one there is a flickering test.
3. **The ladder continues above 1** rather than clipping there (§2.1). A highlight at 4.0
   lands on the rung above the one at 3.6, which keeps the headroom a scene-linear picture
   carries; AE, which has no headroom, clips.

Unpremultiplied (§2.2) — quantising premultiplied colour would band a soft edge by its
*coverage* and fringe it. Alpha is untouched. `cheap` cost, `Exact` ROI. Mix 0 is the
bit-exact identity; there is no neutral level, because Levels 2 is a real setting and 255 is
the effect doing almost nothing rather than nothing at all.

**The Matte pulls Levels toward 256 (K-426, §2.6):** the step count is lerped per pixel from
255 (a black matte: the 8-bit ladder, a step too fine to see) to `Levels − 1` (white), so a
dark matte means finer rungs rather than a coarse ladder faded back over the picture.

### 3.59 Threshold — every pixel to black or to white

Maps AE's Threshold ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Colour** effect.

**Parameters:** Level (per cent, 0..100, default 50), Softness (per cent, 0..100, default 0,
hard min 0), Mix.

**Algorithm sketch.** One decision per pixel, on unpremultiplied colour:

```
t    = √luma(u)                                  # Rec. 709 luma, perceptually placed
hw   = max(Softness ÷ 200, 1⁄1000)               # half the width of the crossing
k    = smoothstep(level − hw, level + hw, t)     # level = Level ÷ 100
out  = (k, k, k)
```

Two decisions:

- **Level is a perceptual position** (K-404), the same √ §3.58 uses and for the same reason:
  a Level of 50 has to land on the grey a person calls middle, which in light is 0.25 and
  not 0.5. This is the one place the batch departs from §3.18's linear pivot, and it departs
  deliberately — Contrast's pivot is the middle of an *operation*, and this is the middle of
  a *judgement*.
- **The crossing is never a step, even at Softness 0** (§3.57 decision 2's rule, second
  outing). The floor is a thousandth of the range, which across any real edge is well under
  a pixel — so the default reads as the hard cut AE gives and still arrives antialiased, and
  the effect's own §1.6 oracle is not a coin toss on the pixels that land exactly on the
  level.

Alpha is untouched: a thresholded picture keeps its shape, and the frame does not become a
white rectangle. Unpremultiplied (§2.2). `cheap` cost, `Exact` ROI. Mix 0 is the bit-exact
identity.

**The Matte scales the Level (K-559, §2.6):** the cut's position is multiplied by the
matte at each pixel, so white cuts where the Level is set, black cuts at 0 — where every
pixel with any light in it comes back white — and grey cuts somewhere between. The
threshold *moves* across the frame, which is the one thing the strength dissolve cannot
do; it is not a lerp toward a neutral, because a Level of 0 is a real setting rather than
the effect doing nothing.

**Softness is not AE's.** AE's Threshold has one control. Softness defaults to 0, where it is
AE's picture, so an import is faithful (K-401).

### 3.60 Tritone — three colours mapped onto the tone range

Maps AE's Tritone ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Colour** effect. §3.24 Tint is
the two-colour form of the same idea and stays.

**Parameters:** Highlights (colour, default a warm white), Midtones (colour, default a warm
brown), Shadows (colour, default a deep blue), Mix.

**Algorithm sketch.** The pixel's luma picks a colour off a two-segment ramp:

```
t    = √luma(u)                                  # as §3.58 and §3.59
s    = min(t, 1)
c    = shadows   + (midtones   − shadows )·2s          when s < ½
c    = midtones  + (highlights − midtones)·(2s − 1)    otherwise
out  = c · max(t, 1)
```

Two decisions:

- **The three stops are placed perceptually** (K-404), so Midtones lands on the grey a person
  points at rather than on 0.5 of the light, which is nearly white.
- **Highlights above 1 keep their headroom** rather than clamping to the Highlights colour
  (§2.1). The ramp is chosen by the clamped position and then *scaled* by how far past white
  the pixel actually was, so a 4× specular stays 4× and takes the highlight colour. AE, with
  no headroom, clips.

Unpremultiplied (§2.2); alpha untouched. `cheap` cost, `Exact` ROI. Mix 0 is the bit-exact
identity, and the default three are a real duotone rather than a no-op (§3.10). AE's "Blend
With Original" is Mix.

### 3.61 Photo filter — a coloured glass held in front of the lens

Maps AE's Photo Filter ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Colour** effect.

**Parameters:** Filter (twenty-one options — the six warming and cooling filters, the eight
colours, Sepia, the four deep filters, Underwater and **Custom**; default Warming filter
(85)), Colour (default a warm amber, enabled while Filter is Custom), Density (per cent,
0..100, default 25, hard min 0 and hard max 100), Preserve luminosity (default on), Mix.

**Algorithm sketch.** One multiply and one optional renormalisation, on unpremultiplied
colour:

```
f       = the filter's scene-linear colour (or Colour, in Custom)
tinted  = u · f
mixed   = u + (tinted − u)·(Density ÷ 100)
out     = mixed · luma(u) ÷ max(luma(mixed), ε)      # only with Preserve luminosity on
```

Two things worth stating:

- **The twenty filters are Lumit's own chromaticities under Adobe's names**, the same
  look-for-look conversion §3.56's thirteen Warp styles are, and the import reports it as
  mapped. Adobe's exact values are not published; the names are Wratten designations and the
  pictures agree.
- **Preserve luminosity is what makes the filter a filter.** Without it a Deep red at full
  Density is very nearly a black frame, because a red filter really does stop most of the
  light — which is AE's behaviour and is kept, because a photographer's filter has a stop
  cost. With it, the pixel's Rec. 709 luma is restored afterwards, so the picture changes
  colour and not exposure. It is on by default, as AE's is.

Density 0 is the bit-exact identity on both paths, and so is Mix 0. Unpremultiplied (§2.2);
alpha untouched. `cheap` cost, `Exact` ROI.

**The Matte scales Density (K-426, §2.6):** thinner glass where the matte is grey, which with
Preserve luminosity on is a different picture from a fade, since the luma put back depends on
how dark the glass was.

### 3.62 Black and white — six weights, one grey

Maps AE's Black & White ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Colour** effect. §3.16
Saturation at −100 is the flat conversion; this is the one a photographer wants, where the
red of a jumper and the green of the grass behind it can still be told apart in the print.

**Parameters:** Reds (per cent, default 40), Yellows (60), Greens (40), Cyans (60), Blues
(20), Magentas (80) — each a slider −200..300, hard min −200 — plus Tint (default off), Tint
colour (default a warm sepia, enabled by Tint), and Mix. The six defaults are AE's.

**Algorithm sketch.** The colour is decomposed exactly into a grey, one **secondary** and one
**primary**, and the weights are applied to those two parts:

```
for r ≥ g ≥ b:   grey = b + (g − b)·Yellows + (r − g)·Reds
```

and the five other orderings by the same rule — the smallest channel is the grey the colour
sits on, the middle minus the smallest is the secondary between the two larger channels
(yellow, cyan or magenta), and the largest minus the middle is the primary. The
decomposition is exact (the three parts sum back to the original colour), so at all six
weights of 100 the result is the channel maximum, and on a grey pixel every difference is
zero and the weights do nothing at all — which is the property a slider set has to have
before an editor will trust it.

Three notes:

- **It is continuous across every boundary.** Where two channels are equal, the two orderings
  that could claim the pixel give the same answer, because the term that distinguishes them
  is zero. There is no seam on a gradient, which a nearest-primary scheme would have.
- **Nothing is clipped above**, so a weight of 300 on a specular highlight keeps its headroom
  (§2.1). The grey is floored at 0, because a negative weight would otherwise ask for
  negative light.
- **Tint changes hue, not exposure** (K-404). The grey is multiplied by the Tint colour
  divided by that colour's own luma, so choosing a darker tint tints the picture rather than
  darkening it, and the brightness stays where the six weights put it.

Unpremultiplied (§2.2); alpha untouched. `cheap` cost, `Exact` ROI. Mix 0 is the bit-exact
identity; the six defaults are a real conversion, not a no-op (§3.10).

### 3.63 Shadow highlight — the local rescue of a backlit shot

Maps AE's Shadow/Highlight ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Colour** effect, and the
first in the family that reads a pixel's **neighbours** rather than only the pixel.

**Parameters:** Shadow amount (per cent, 0..100, default 25), Shadow tonal width (per cent,
0..100, default 50), Highlight amount (0..100, default 25), Highlight tonal width (0..100,
default 50), Radius (px@comp, 0..500, default 30, hard max 2000), and — in a collapsed *More
options* group — Colour correction (per cent, −100..100, default 20) and Midtone contrast
(per cent, −100..100, default 0), plus Mix.

**Algorithm sketch.** One gaussian on the picture at Radius — **the shipped §3.8 blur again**
(§3.43's softening and §3.57's distance field were the first two) — and then one pass:

```
L    = luma(u)                                   # this pixel
t    = √min(luma(ū), 1)                          # where its neighbourhood sits
mₛ   = 1 − smoothstep(0, wₛ, t)                  # wₛ = Shadow tonal width ÷ 100, floored
m_h  = smoothstep(1 − w_h, 1, t)
L′   = L · (1 + 2·mₛ·Shadow amount ÷ 100) ÷ (1 + 2·m_h·Highlight amount ÷ 100)
p    = (√L′ − ½)·(1 + Midtone contrast ÷ 100) + ½, floored at 0;   L″ = p²
k    = L″ ÷ L        (1 where L is 0)
rgb  = u·k
out  = luma(rgb) + (rgb − luma(rgb))·(1 + Colour correction ÷ 100 · min(|k − 1|, 1))
```

Five decisions:

- **The blurred luma is the "local" in local-adaptive, and it steers only the *mask*.** What
  decides whether a pixel is being treated as a shadow is its *neighbourhood's* brightness,
  not its own — which is why a face against a bright window is lifted as a whole rather than
  dissolving into its own dark pixels. The pixel's own value is what gets multiplied, so
  nothing is softened and no detail is borrowed from a neighbour: the blur is a question, not
  an answer.
- **One Radius, not AE's two** (K-404). AE carries a Shadow Radius and a Highlight Radius,
  which is a second full-frame gaussian for a control whose whole job is the softness of a
  mask, and a shot that needs the shadows' mask measured at one scale and the highlights' at
  another in the same grade has not turned up. The import averages AE's two and reports it.
- **The lift is a multiply, not a gamma.** A multiplied lift is monotone, needs no clamp and
  no inverse, and cannot invert an ordering; the local mask is what makes the effect
  adaptive, and dressing the same mask in a per-pixel exponent buys nothing but a `pow` a
  pixel. 100 lifts a shadow threefold and pulls a highlight to a third.
- **Colour correction is a saturation boost weighted by how far the pixel moved.** Lifting a
  shadow scales all three channels together, which is correct in light and reads as
  desaturated, because that is what happens to real shadows when they are opened up. The
  boost applies exactly where the gain differs from 1 and nowhere else, so 0 is the identity
  in colour and the control cannot quietly saturate a picture it did not otherwise touch.
- **Midtone contrast pivots on the perceptual middle**, the same √ the rest of the batch uses
  (K-404): mid-grey, not 0.5 of the light.

**Not built, and reported by the import: Auto Amounts, Temporal Smoothing and Scene Detect.**
AE's default is to choose the two amounts from the frame's own histogram and then smooth that
choice over neighbouring frames. The first half is a whole-frame reduction, the second reads
frames this effect is not given, and together they make an effect whose answer at a frame
depends on the shot around it — which is a grade that cannot be scrubbed backwards and is not
what this effect is. Lumit's amounts are the user's, and an imported instance arrives with
AE's default pair written in.

Blurring means `PaddedPx(2000)` ROI — Radius' own hard maximum — and `moderate` cost. Unpremultiplied (§2.2); alpha
untouched. **Both amounts and Midtone contrast at 0 short-circuits to the bit-exact identity**
on both paths — the blur is not even run — and Mix 0 likewise.

**The Matte scales Shadow amount and Highlight amount (K-426, §2.6):** a grey matte lifts and
pulls less, the neighbourhood blur, the widths and Midtone contrast untouched by it.

### 3.64 Median — the middle value of a neighbourhood

Maps AE's Median ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Stylise** effect, and the one
effect in the catalogue whose cost grows as the **fourth power** of its one control.

**Parameters:** Radius (px@comp, 0..3, hard max 3, default 2), Operate on alpha (default
off), Mix.

**Algorithm sketch.** For each pixel, over the `(2r+1)²` window centred on it, on
unpremultiplied colour (§2.2) and per channel independently:

```
r    = round(Radius), clamped to 0..3          # host-side, so both paths get one integer
N    = (2r + 1)²                               # 1, 9, 25 or 49 samples
out  = the ⌈N⁄2⌉-th smallest of the N values   # the true median, not an approximation
```

It is a **real median** — the value that actually occurs in the window with as many
neighbours below it as above — and not a percentile estimate, a bilateral blur or a
separable pair of one-dimensional medians. That matters because the median is exactly the
filter that removes salt-and-pepper speckle and leaves an edge where it was, and every
cheaper stand-in gives that property up.

Four things that are decision rather than derivation:

- **The selection is a compare-exchange network, because a data-dependent sort is not a
  GPU program** (K-405). A branchy quickselect diverges every lane in a warp and is a
  different sequence of comparisons on the two paths, which §1.6 could not hold to
  agreement. Instead both paths sweep the window once, carrying a sorted register array of
  the `⌈N⁄2⌉` smallest values seen so far and inserting each new sample with a bubble of
  `min`/`max` pairs. Nothing branches on a value, the two paths execute the identical
  comparisons in the identical order, and — because `min` and `max` on a vector are
  componentwise — the three colour channels are selected **simultaneously**, three medians
  for the price of one network. Because the network never branches on a value, the GPU
  sweeps the **widest** window at every radius and pads the samples it does not want with
  a value larger than any pixel; the padding sorts above every real sample and so cannot
  reach the middle. The CPU, which may branch, sweeps only the window it was asked for —
  and the two answers are bit-identical, because `min` and `max` are exact and a sorted
  set does not depend on the order things were inserted in.
- **The radius is capped at 3, and the cap is the honest part.** The sweep costs
  `N × ⌈N⁄2⌉` compare-exchanges, which is `(2r+1)⁴⁄2`: 45 at radius 1, 325 at 2 and
  1 225 at radius 3. Radius 6 would be 17 000 a pixel and radius 12 a quarter of a
  million. The slider's **hard** maximum is therefore 3, not a soft one that could be
  typed past — a control that silently clamps is worse than a control that stops. This is
  the catalogue's only `heavy` single-pass kernel, and it says so in its cost class.
- **The radius is a length in px@comp** (§2.3) and is rounded to whole raster pixels
  host-side, so a Half-resolution preview medians a window of half the raster size — the
  same visible neighbourhood, as §2.3 requires. Radius 0 short-circuits to the bit-exact
  identity on both paths, rather than being a one-sample median through an unpremultiply
  round trip.
- **Alpha is left alone unless asked** (AE's "Operate on Alpha Channel"), because a median
  of the coverage moves the shape's outline, which is a separate thing to want from
  despeckling its colour. With it on, alpha is medianed in the same sweep — a fourth lane
  of the same network, free.

**The Matte scales Radius (K-428, §2.6):** each pixel's matte luma multiplies Radius before
its window is swept, rounded with the same `floor(x + ½)` the control itself is, so a half
matte on Radius 2 gives *exactly* the Radius 1 picture — a genuinely smaller window, not a
half-fade of a bigger one. That is what lets one painted matte despeckle a noisy sky and leave
a face alone. A pixel whose radius comes to 0 is left as it arrived, the same short-circuit the
whole effect takes at Radius 0.

Edges repeat (the border pixel is held outward), which is the only edge policy a median
wants: a transparent surround would win the vote on a corner pixel and eat the frame's own
border. Unpremultiplied (§2.2), `heavy` cost, `PaddedPx(3)` ROI (Radius' own hard maximum). Mix 0 is the
bit-exact identity.

### 3.65 Mosaic — the frame in flat blocks

Maps AE's Mosaic ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Stylise** effect.

**Parameters:** Horizontal blocks (whole numbers, 1..200, default 24, hard min 1, hard max
2000), Vertical blocks (whole numbers, 1..200, default 14, hard min 1, hard max 2000), Sharp
colours (default off), Mix.

**Algorithm sketch.** The frame is cut into a grid of `Horizontal blocks × Vertical blocks`
rectangles and every pixel takes its block's one colour. Every boundary is computed in
**integers**, so the two paths cannot disagree about which block a pixel is in:

```
i    = (x · hblocks) ÷ w                       # integer division throughout
x₀   = (i · w) ÷ hblocks        x₁ = ((i+1) · w) ÷ hblocks
Sharp colours on:   the pixel at (x₀ + (x₁−x₀)÷2, …)   — the block's centre
Sharp colours off:  the mean of an n×n stratified sample of the block,
                    n = min(8, x₁ − x₀), sample k at x₀ + (2k·span + span) ÷ (2n)
```

Three notes:

- **Nothing here is a float** until the averaging itself. A block edge decided by
  `floor(x ÷ block_width)` in floating point is a pixel that lands in different blocks on
  the two paths wherever the division is exact — K-399's rule about a threshold, arriving
  on a *coordinate*. Integer division has no such tie.
- **The average is a bounded sample, and that is deliberate** (K-405). A true mean of a
  block of a 1080p frame at the default grid is 3 500 taps, redone by every one of those
  3 500 pixels — a hundred-fold more work than the picture is worth. The block is instead
  sampled on a stratified grid of at most 8×8 positions, which for any block reads as the
  same flat colour and costs at most 64 taps a pixel. A block smaller than 8 pixels across
  is sampled **completely**, so a fine mosaic is an exact mean.
- **Sharp colours defaults off**, as AE's does, because the mean is the picture people
  expect from a mosaic; on, the block takes the single colour of its centre pixel, which is
  crisper on graphic material and noisier on film.

Premultiplied (§2.2) — averaging premultiplied colour is what compositing means, and the
alpha is blocked with it, so a mosaicked cut-out gets blocky edges rather than smooth ones
round a blocky middle. `cheap` cost, `FullFrame` ROI (a block reaches across the frame at
one block wide). Mix 0 is the bit-exact identity; there is no neutral block count, one
block being a real setting (the frame's average colour) and 2000 being the effect doing
almost nothing.

### 3.66 Find edges — the picture as a pencil drawing

Maps AE's Find Edges ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Stylise** effect.

**Parameters:** Invert edges (default off), Mix.

**Algorithm sketch.** A Sobel gradient per channel, on unpremultiplied colour, with each
tap taken **perceptually**:

```
p        = √u                                   # §3.58's curve, per channel
gₓ, g_y  = the 3×3 Sobel pair applied to p
e        = min(√(gₓ² + g_y²), 1)                 # edge strength, per channel
q        = e         with Invert edges on        # bright edges on black
q        = 1 − e     with Invert edges off       # dark edges on white — AE's default
out      = q²                                    # back into light
```

Two decisions:

- **The gradient is taken on the perceptual value, not on the light** (K-405, §3.58's rule
  a fifth time). A Sobel in scene-linear light is dominated by the highlights: the step
  from 3.0 to 4.0 in a sunlit sky is a stronger "edge" than the step from 0.01 to 0.05 in
  a shadow, though the eye sees the second and not the first. Taking the difference in √
  puts the edges where a person draws them, which is what makes this effect read as a
  pencil drawing rather than as a map of the specular highlights.
- **Invert edges is AE's Invert, and AE's default is the drawing.** With it off the frame is
  white with dark lines on it; with it on, black with glowing ones. The name is Lumit's: every
  effect carries the §2.6 Matte row, whose own switch is called Invert, and two rows of that
  name in one panel is a control nobody can point at. AE's "Blend With Original" is Mix.

Alpha is untouched, so the drawing keeps the layer's shape. Unpremultiplied (§2.2), edges
repeat. `cheap` cost, `PaddedPx(1)` ROI (one pixel, and a padding never resolves below one raster pixel). Mix 0 is the bit-exact
identity; there is no neutral setting, an edge map being the whole point.

### 3.67 Emboss — the picture as grey relief

Maps AE's Emboss ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Stylise** effect.

**Parameters:** Direction (dial, degrees, default 45), Relief (px@comp, 0..20, default 2,
hard min 0), Contrast (per cent, 0..200, default 100, hard min 0), Mix.

**Algorithm sketch.** Two taps either side of the pixel along the light's axis, differenced
perceptually and laid down as grey:

```
d    = Relief · (sin θ, −cos θ)          # θ from straight up, clockwise (§3.43's convention)
a    = √luma(u at p − d)
b    = √luma(u at p + d)
g    = ½ + (b − a)·(Contrast ÷ 100)
out  = (max(g, 0))²  in all three channels
```

Three notes:

- **The relief is grey, and that is the look.** AE's Emboss suppresses colour, and so does
  this: the difference is taken on luma and written to all three channels, so what comes
  back is a stamped-metal picture of the frame's edges lit from Direction. For a coloured
  relief, put it under a §3.24 Tint or a §3.60 Tritone — a second effect, rather than a
  switch on this one.
- **Relief 0 is not the identity, it is flat mid-grey**, and calling it "off" would be a
  lie: with no separation between the two taps there is no relief to see, and the honest
  answer is the surface with no light on it. §3.57's Fractal influence 0 makes the same
  point from the other side. Mix is what turns the effect down.
- **Direction is where the light is**, in AE's own convention (degrees from straight up,
  clockwise, as §3.43's Drop shadow reads it), and the slope facing it is the bright one.
  The perceptual difference is §3.58's curve for the sixth time, and for Find edges'
  reason: a relief taken in light would be all highlight and no shadow.

**The Matte scales Relief (K-428, §2.6):** each pixel's matte luma multiplies the tap offset
before the two taps are read, so the relief is genuinely shallower where the matte is grey. A
black matte therefore gives the **flat mid-grey sheet**, not the picture back — because Relief
0 is that sheet and not the identity, and the matte turns the relief down rather than turning
the effect off. Mix is still what turns the effect down.

Alpha is untouched. Unpremultiplied (§2.2), edges repeat. `cheap` cost,
`PaddedPx(40)` ROI (twice Relief's slider, its hard maximum being open). Mix 0 is the bit-exact identity.

### 3.68 Texturize — another layer pressed into this one as relief

Maps AE's Texturize ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Stylise** effect, and the
second in the catalogue after §3.28 Light wrap to take a **layer of its own** beside the
universal Matte row.

**Parameters:** Texture (layer, unset by default), Light direction (dial, degrees, default
45), Relief (px@comp, 0..20, default 1, hard min 0), Texture contrast (per cent, 0..200,
default 100, hard min 0), Placement (Stretch / Tile / Centre, default Stretch), Scale (per
cent, 10..400, default 100, hard min 1), Mix.

**Algorithm sketch.** The texture layer is embossed exactly as §3.67 embosses the picture,
and the relief that comes out multiplies this layer's colour:

```
uv   = (p ÷ size − ½) ÷ (Scale ÷ 100) + ½       # the texture's coordinate at this pixel
       Stretch: clamped to 0..1     Tile: wrapped     Centre: relief 0 outside 0..1
d    = Relief · (sin θ, −cos θ) ÷ (size · Scale ÷ 100)
r    = (√luma(t at uv + d) − √luma(t at uv − d)) · (Texture contrast ÷ 100)
out  = max(u · (1 + r), 0)
```

Four things worth stating:

- **The texture is its own Layer row, not the Matte row** (K-405). §3.49's map *is* the
  matte, because a displacement map has nothing else it could be; a texture is not, because
  an editor will want to press a canvas into a layer **and** limit the pressing to a
  region, and one row cannot say both. So Texturize declares its own Texture row — Light
  wrap's Background is the precedent — and keeps the generic §2.6 strength matte, which is
  what "only over the sky" means here.
- **Placement is a *fitting*, and Scale is the size** (K-405). The layer carriage renders
  a referenced layer alone at this raster (docs/impl/layer-input.md), so the texture arrives
  frame-shaped: "stretch to fit" is what the carriage does and always did, exactly as
  §3.49 records. Scale is therefore Lumit's own control — it says how big one copy of the
  texture is as a fraction of the frame — and **Placement says only what happens outside
  that copy**: Stretch holds the edge, Tile repeats it, Centre leaves the rest of the frame
  untextured. At Scale 100 all three coincide, and that one case is AE's Stretch Texture to
  Fit exactly, which is why Scale defaults to 100. AE's Tile and Centre are its texture
  layer's *native* size, which the carriage has not preserved, so the import converts the
  choice and reports the size as approximated.
- **Relief is a length in px@comp** where AE has no control at all — its relief is one
  pixel of whatever raster it was handed, which §2.3 forbids. The default of 1 is AE's
  behaviour at full resolution, so an import is faithful and a preview stops disagreeing
  with the export.
- **An unset Texture is the labelled no-op**, as every layer row is (docs/impl/
  layer-input.md), and so is a dangling or cyclic reference.

**The Matte scales Relief (K-428, §2.6)** — the light vector Relief is spent into, before the
texture's two taps are read, so a grey matte reads a *different pair* of texture pixels and not
a weaker version of the same difference. The Texture row is unaffected: it is the effect's
subject, the matte is how much of it presses in.

Premultiplied (§2.2) — the relief is a multiply, and multiplying premultiplied colour by a
scalar is the same operation as multiplying straight colour by it, so no round trip is
needed and the shape is untouched. The texture's own taps *are* unpremultiplied, so a
texture with a soft edge does not read as black there. `cheap` cost, `PaddedPx(40)`
ROI. Mix 0 and an unset Texture are both the bit-exact identity.

### 3.69 Broadcast safe — the signal clamped to a legal amplitude

Maps AE's Broadcast Colors ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Utility** effect: a
delivery tool, not a look.

**Parameters:** Standard (NTSC / PAL, default NTSC), How to treat (Reduce brightness /
Reduce saturation / Key out unsafe / Key out safe, default Reduce brightness), Maximum
signal (IRE, 90..120, default 110, hard min 90, hard max 120), Mix.

**Algorithm sketch.** The pixel is taken to an encoded signal, its composite amplitude is
measured, and — where that amplitude is over the limit — one of four things happens:

```
v      = √u                                     # the encoded signal (see below)
Y      = 0.2126v_r + 0.7152v_g + 0.0722v_b
U      = 0.493(v_b − Y)      V = 0.877(v_r − Y)      C = √(U² + V²)
ire    = 7.5 + 92.5·(Y + C)      NTSC — 7.5 IRE of setup, 92.5 of active range
ire    = 100·(Y + C)             PAL  — no setup

Reduce brightness:  v ← v·k,             k = min(1, (limit ÷ 100 − s) ÷ ((1 − s)·(Y + C)))
Reduce saturation:  v ← Y + (v − Y)·m,   m = clamp((limit ÷ 100 − s) ÷ (1 − s) − Y, 0, C) ÷ C
Key out unsafe:     alpha ← 0 where ire > limit
Key out safe:       alpha ← 0 where ire ≤ limit
out    = v²                                     # back into light
```

(`s` is the standard's setup as a fraction: 0.075 for NTSC, 0 for PAL.)

Three decisions:

- **It is a clamp, and it says so in its name** (K-405). AE calls the effect Broadcast
  Colors and offers "Key Out Unsafe" as a *diagnostic view* beside two real repairs; Lumit
  keeps all four, because seeing which pixels are illegal is half of why anyone reaches for
  this, but the name is what the effect **does** to the picture — docs/01's rule that a
  control is named for its effect and not for its heritage. Key out safe is the same
  diagnostic the other way up: everything legal is removed, so what is left is the problem,
  which composites straight over the frame as an overlay.
- **The encoding is §3.58's square root, not Rec. 709's transfer function.** A composite
  signal's amplitude is a statement about the *encoded* value, so scene-linear light has to
  be encoded before it can be measured. The batch's `√` is used for the same oracle reason
  it was chosen for in the first place (K-404) — the answer here is a **threshold**, so a
  last-bit disagreement between the two paths is a pixel that is keyed out on one and not
  on the other. Across the range the difference between `√` and the real OETF is under two
  IRE, which is inside the margin a Maximum signal of 110 already carries, and the control
  is a limit the user picks rather than a measurement they read.
- **Reducing saturation cannot rescue an over-bright pixel, and the effect does not
  pretend it can.** With `Y` alone already over the limit, the desaturation runs to zero
  and leaves a legal-luma grey that is still hot; the fix is Reduce brightness, or a grade.
  This is stated rather than hidden because a "safe" tool that quietly fails is worse than
  one that visibly does not.

Unpremultiplied (§2.2). `cheap` cost, `Exact` ROI. Mix 0 is the bit-exact identity, and so
is a frame that is already legal everywhere — the two repair modes are the identity on a
pixel under the limit, by construction rather than by short-circuit.

### 3.70 Venetian blinds — the frame closed by a rank of slats

Maps AE's Venetian Blinds ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Transition** effect, and
§3.46's wipe repeated: one straight edge becomes a rank of them.

**Parameters:** Completion (per cent, 0..100, default 50, hard 0..100), Direction (dial,
degrees, default 0), Width (px@comp, 1..500, default 20, hard min 1), Feather (px@comp,
0..500, default 0, hard min 0), Mix.

**Algorithm sketch.** §3.46's signed distance, folded into one slat before it is thresholded:

```
n       = (sin θ, −cos θ)                     # host-computed; θ = 0 points up the screen
d       = (p − centre) · n                     # centre is the frame's own middle, raster px
period  = max(Width, 1)                        # one slat, raster px
band    = max(Feather, 1e-3)
u       = d − period·floor(d ÷ period + ½)     # the slat-local position, −period÷2 .. period÷2
hw      = c·(period÷2 + band) − band÷2         # the removed half-slat, c = Completion ÷ 100
keep    = clamp((|u| − hw) ÷ band + ½, 0, 1)
out     = src · keep                           # premultiplied: all four channels
out     = orig·(1 − mix) + out·mix
```

Four notes:

- **It is one wipe folded into a slat, and that is the whole effect.** Everything §3.46
  establishes carries over unchanged: the direction convention (clockwise from straight up, so
  Direction 0 gives horizontal slats and the frame closes vertically), the half-band lead-in
  that makes Completion 0 the bit-exact identity and 100 the exactly empty frame, and the
  premultiplied scale. What is new is the fold — `u` — and it is written `floor(x + ½)` for
  §3.47's reason, never `round`.
- **The slats are anchored on the frame's middle**, not on a control. AE has no centre here
  either, and a rank of blinds has nothing a centre would say: moving them by half a slat
  gives the identical picture. Direction and Width are the whole geometry.
- **The gap opens at each slat's middle** and grows both ways, so the kept material is the
  slat's two edges. AE's opens the same way, and it is the reading that makes the control
  linear: at Completion 50 exactly half of every slat is gone, whatever the feather is.
- **Width is a length in px@comp** (§2.3), where AE's is raster pixels. The default of 20 is
  AE's own number, which at 1080p is AE's picture exactly; only Completion diverges, for
  §3.46's reason.

`trivial` cost, `Exact` ROI. Mix 0 and Completion 0 are both the bit-exact identity.

**The Matte scales Completion per pixel (K-429, §2.6):** the slats stand further open where
the matte is bright, so one part of the frame can be shut while another is wide open.

### 3.71 Iris wipe — a polygon or a star opened out of the middle

Maps AE's Iris Wipe ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Transition** effect, and the
only one in the family with no Completion: **the radius is the transition**, exactly as AE's
is, so the shape is animated by growing it.

**Parameters:** Iris centre x and Iris centre y (px@comp, default 960, 540), Iris points
(whole number, 6..32, default 6, hard 6..32), Outer radius (px@comp, 0..2000,
default 330, hard min 0), Use inner radius (default off), Inner radius (px@comp, 0..2000,
default 165, hard min 0), Rotation (dial, degrees, default 0), Feather
(px@comp, 0..500, default 0, hard min 0), Mix.

**Algorithm sketch.** The polygon is never rasterised. One sector of it is solved instead,
and the pixel's distance to that sector's edge decides the coverage:

```
period      = 2π ÷ Points
A           = (Outer, 0)                        # the vertex on the ray straight up
B           = (r_b·cos φ_b, r_b·sin φ_b)        # the next vertex round
              plain polygon:  φ_b = period,   r_b = Outer
              with inner:     φ_b = period÷2, r_b = Inner
m           = (B.y − A.y, A.x − B.x) ÷ |…|      # the edge's outward unit normal, host-computed

φ           = atan2(p.y − cy, p.x − cx) + ½π − Rotation
a           = |φ − period·floor(φ ÷ period + ½)|      # the angle into the sector, 0..period÷2
r           = |p − centre|
P           = (r·cos a, r·sin a)                # the pixel, reflected into that one sector
dist        = (P − A) · m                       # signed: positive outside the iris
keep        = clamp(dist ÷ band + ½, 0, 1)      # band = max(Feather, 1e-3)
out         = src · keep
out         = orig·(1 − mix) + out·mix
```

Five notes:

- **One sector answers for the whole shape.** A regular polygon and a star are both
  *rotationally symmetric*, so folding the pixel's angle into a single sector and reflecting it
  about that sector's own bisector reduces the entire boundary to one straight edge — and the
  distance to a straight edge is a dot product. No winding test, no per-edge loop, and the
  number that comes out is a **true perpendicular distance in pixels**, which is what lets
  Feather be a width rather than an angle (§3.47's problem, avoided rather than solved).
- **Plain and starred are the same expression**, differing only in where the host puts the
  second vertex: at the next point of the polygon, or halfway round at the inner radius. The
  toggle therefore costs nothing per pixel, and Inner radius is greyed out until it is on.
- **The iris removes what is inside it**, which is AE's behaviour: the effect opens a hole and
  the hole grows. To reveal *through* an iris instead, the Matte row's Invert is the wrong tool
  (it inverts which parts of the frame the iris opens in, not which side of the edge is kept)
  — use §3.44 Set matte with this effect on the matte layer, or an Outer radius large enough
  to leave only the star.
- **The two radii and the centre are all px@comp** (K-419): a radius is a *size* that the
  preview scaling keeps consistent across a reframe, a centre is a *place* the user clicks.
  AE's are both layer pixels, so the import carries them unchanged.
- **Outer radius 0 is the identity by short-circuit.** With no polygon there is no edge, the
  normal is undefined, and a kernel that divided by its length would paint half-grey over the
  frame; both paths test the radius instead. §3.51's and §3.52's short-circuits, a third time.

`cheap` cost — one `atan2` a pixel, §3.47's admission again (K-399) — `Exact` ROI. Mix 0 and
Outer radius 0 are both the bit-exact identity.

**The Matte scales the iris radius per pixel (K-429, §2.6),** this being the one transition
with no Completion to scale — which is the same sentence about the same thing, since the
radius *is* the transition. It costs one multiply: the solved sector's vertex is the only
place a radius survives into the expression, so scaling it scales the outer and inner radii
together and leaves the edge's direction, and so the normal, alone. A half matte draws
*exactly* the half-radius iris, and a black matte is the same exact identity Outer radius 0
already is.

### 3.72 Card wipe — the frame as a grid of cards, turning away

Maps AE's Card Wipe ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Transition** effect, and the
first in the catalogue to put a **camera** in front of a pixel — a fixed one, with no controls
on it (see the fourth decision).

**Parameters:** Completion (per cent, 0..100, default 50, hard 0..100), Transition width
(**px@comp**, K-558, 1..3840, default 960, hard min 1 — the flipping wave's width across the
frame, measured along whichever axis Flip order runs, and centred on the actual comp by
`instantiate_for_raster`), Rows (whole number, 1..64, default 6, hard 1..256),
Columns (whole number, 1..64, default 8, hard 1..256), Flip axis (Horizontal axis / Vertical
axis / Random, default Horizontal axis), Flip direction (Forwards / Backwards / Random,
default Forwards), Flip order (Left to right / Right to left / Top to bottom / Bottom to top,
default Left to right), Randomness (per cent, 0..100, default 0, hard 0..100), Seed, Mix.

**Algorithm sketch.** Every pixel finds its card, works out how far *that card* has turned, and
then asks the one question a gather kernel can ask: which point of the flat card is standing
where I am?

```
# 1. the card, in whole numbers (§3.65's rule)
i, j        = (x·Columns) ÷ W,  (y·Rows) ÷ H              # integer division
x0, x1      = ⌈i·W ÷ Columns⌉, ⌈(i+1)·W ÷ Columns⌉        # and likewise y0, y1
(f, g)      = the pixel inside its card, −1..1 on the flip axis and across it
(hf, hg)    = the card's half-extent on each of those, raster px

# 2. when this card flips
o           = Flip order's ramp at (i, j), 0..1
o           = o + (hash(seed, i, j) − o)·(Randomness ÷ 100)
t           = clamp((c − o·(1 − w)) ÷ w, 0, 1)             # c = Completion÷100
                                                          # w = width ÷ the frame's extent
                                                          #     along the order axis, host-side
θ           = ±t·½π                                        # the sign is Flip direction

# 3. where the card is now — the camera is at distance D = 3 card half-widths
   forward:  f = s·cos θ · D ÷ (D − s·sin θ)               # s is the point ON the card, −1..1
   inverse:  s = f·D ÷ (D·cos θ + f·sin θ)                 # one divide, exactly
   k        = D ÷ (D − s·sin θ)                            # the same foreshortening across
   g_card   = g ÷ k
   sample    = the card's centre + (s·hf, g_card·hg), bilinear
   coverage  = box overlap of |s| ≤ 1 and |g_card| ≤ 1, taken in screen pixels
out         = sample · coverage
out         = orig·(1 − mix) + out·mix
```

Six decisions:

- **Transition width is a distance across the frame, not a share of the wipe** (K-558). The
  Flip order ramp `o` is a *position* — where a card sits along the order axis, 0 to 1 — so the
  band running along it is measured in the same space, and since K-558 that space is quoted in
  pixels. The share is taken once, host-side, dividing by the raster's own extent along that
  axis, so both kernels are handed the same `1 ÷ w` and `1 − w` they always were and neither
  learns a new unit. Width and raster carry the same preview factor, so a Half preview wipes
  exactly as the export does.
- **It is geometry, not particles** (docs/impl/ae-effect-parity.md's standing exclusion). A
  card is a rectangle with one rotation on it, its position is a function of its grid index,
  and nothing is simulated, integrated or advected. That is what makes it a kernel with a
  closed-form inverse instead of a system with state.
- **The projection is inverted rather than drawn.** Every other way to flip a card scatters —
  transform the rectangle and rasterise it — and Lumit's effects gather (§1.1). The one-point
  projection above is a Möbius map in `s`, so it inverts in one line, which is the whole
  reason a card wipe can be a single pass with no geometry pipeline behind it. The cross-axis
  coordinate then divides by the same foreshortening the solved `s` produced, so a card
  narrows *and* the picture on it slides, which is what makes it read as a turn rather than a
  squash.
- **A card that has not started is untouched to the bit, and one that has finished is exactly
  gone.** `t` is clamped, so the two ends are tested for exactly rather than arrived at
  through a cosine: at `t = 0` the pixel is passed through and at `t = 1` it is cleared.
  Without that, Completion 100 would leave a hairline of quarter-strength pixels down each
  card's spine, because `cos(½π)` in `f32` is 6·10⁻⁸ and not zero.
- **AE's camera system is not carried, and the omission is deliberate.** Card Wipe in AE
  offers Camera Position, Corner Pins and Composite Camera, plus Lighting and Material groups
  and two jitter controls, all of which exist to place a *shared* 3D camera in front of the
  grid. Lumit has no 3D camera on an effect (docs/06 keeps cameras on the composition), so
  every card is projected in its own local frame from a fixed viewing distance of three card
  half-widths — the same perspective for every card, whatever the grid. That is the honest
  simple form: it flips, the direction of the flip is visible, and nothing pretends to a
  camera it does not have. The import reports all of it. **Back Layer, Card Scale, Position
  Jitter and Rotation Jitter are not carried either**; a card turns to nothing, which is AE's
  own picture when the back layer is empty.
- **Flip order's Gradient needs no row of its own** (revised by K-429). It used to be
  declined here on §3.68's test: AE picks the order from a gradient *layer*, the only layer
  row this effect has is the universal Matte (§2.6), and a card wipe wanted to say "only over
  the sky" as well as "in this order" — two things about *where*, which one row could not
  say. The owner's rule for mattes settled which of the two the row says: the Matte scales
  **Completion** per pixel, so painting a ramp on it *is* the gradient order, and "only over
  the sky" is what a mask on the layer is for. Randomness plus Seed still covers a shuffle
  nobody wants to paint.

A card never reads outside its own cell, so nothing bleeds between cards; at shallow angles the
near edge would reach about 6 % past the cell and is cropped there, which is what drawing each
card in its own frame costs. Premultiplied (§2.2). `cheap` cost, `FullFrame` ROI (one column is
a card the width of the frame). Mix 0 and Completion 0 are both the bit-exact identity, and
Completion 100 is the exactly empty frame.

**The Matte scales Completion per pixel (K-429, §2.6):** the cards have turned further where
the matte is bright. Note the grain — it is asked per *pixel*, not per card, so a matte can
leave one half of a card standing while the other half has flipped away, and it is read at
the **destination** pixel, where the card's point is standing rather than where the picture
was fetched from (K-427's rule for every gather). Painting a ramp on the matte is therefore
AE's **gradient flip order**, which is what settled the fourth decision above.

---

### 3.73 Beam — a tapered shaft of light travelling between two points

Maps AE's Beam ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Generate** effect, and the simplest
member of the draw family: one segment, two colours and a taper.

**Parameters:** Start x and Start y (px@comp, default 240, 840), End x and End y (px@comp,
default 1680, 240), Length (**px@comp**, K-558, 0..4000, default 1560 — the length of the run
those defaults describe, hard min 0), Time (per cent,
0..100, default 100, hard 0..100), Start thickness (px@comp, 0..200, default 14, hard min 0),
End thickness (px@comp, 0..200, default 3, hard min 0), Softness (per cent, 0..100, default
30, hard 0..100), Inside colour (default white), Outside colour (default a saturated blue),
Composite on original (default on), Mix.

**Algorithm sketch.** The beam is one capsule, so every pixel asks the one question a capsule
answers: how far am I from the segment, and how far along it is the nearest point?

```
u1     = clamp(Time ÷ 100, 0, 1)                    # the head, host-side
u0     = clamp(Time ÷ 100 − Length ÷ |d|, 0, 1)     # the tail, Length in px@comp
active = u1 > u0                                    # a zero-length beam draws nothing
d      = End − Start                                # raster px
s      = clamp((p − Start)·d ÷ max(|d|², ε), u0, u1)
q      = Start + s·d
r      = |p − q|
f      = (s − u0) ÷ (u1 − u0)                       # 0 at the tail, 1 at the head
half   = ½·(w0 + (w1 − w0)·f)                       # the two thicknesses, raster px
k      = clamp((r ÷ max(half, ε) − (1 − soft)) ÷ (soft ÷ 2), 0, 1)
colour = Inside + (Outside − Inside)·k
cov    = clamp(half + ½ − r, 0, 1)                  # one raster pixel of antialiasing
base   = src, or the empty pixel when Composite on original is off
out    = base·(1 − cov) + colour·cov                # premultiplied, all four channels
out    = src·(1 − mix) + out·mix
```

Four notes:

- **Time and Length are two ends of one interval, not a speed.** §3.53's ruling again: an
  effect that animated itself off the clock would make preview and export disagree. Time says
  where the beam's *head* has got to and Length how far its tail trails behind it, both
  ordinary controls the timeline keyframes. Time is a per cent of the run; Length is px@comp
  since K-558, because it is a distance and a distance is pixels — the kernel divides it by
  the run once, host-side, and both numbers carry the same preview factor, so a Half preview
  draws the same beam the export does. AE's Length is a fraction of the run, so the import
  multiplies it by the run those points describe.
- **The taper runs along the drawn beam**, not along Start→End, which is what makes a short
  beam read as a comet at every Time rather than only at the end of its travel. It is the one
  place this effect diverges from AE's picture, and it is invisible at the default Length 100
  — where the drawn beam *is* Start→End.
- **Softness is the share of the half-width the rim occupies**, and the crossover takes the
  rim's *inner half* — so the outside colour is reached before the edge and is a band rather
  than a hairline at the last antialiased pixel, which is where the obvious build puts it. At
  Softness 0 the beam is a flat slab of the inside colour and the outside colour has nothing to
  colour, which is AE's own degenerate; the default of 30 shows both the moment the effect is
  added (§1.2).
- **Time 0 is the bit-exact identity by short-circuit** — §3.71's fifth note again. With the
  head and the tail at the same place there is no segment, the projection is undefined, and a
  kernel that divided by the interval would paint a disc at the start point; both paths test
  `active` instead. With Composite on original off the same case is the exactly empty frame,
  which is the honest reading of "draw the beam, and nothing else".

`cheap` cost, `Exact` ROI. Premultiplied (§2.2): the beam's colour is written at its own
coverage, which is the premultiplied form of "this colour, this much of it". Mix 0 and Time 0
are both the bit-exact identity.

### 3.74 Lightning — a forked bolt between two points

Maps AE's Advanced Lightning ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Generate** effect, and
the first in the catalogue whose *geometry* is built host-side and handed to the kernel — see
the first decision.

**Parameters:** Origin x and Origin y (px@comp, default 300, 900), Direction x and Direction y
(px@comp, default 1620, 200), Type (Direction / Strike / Omni / Two-way strike, default
Direction), Conductivity state (per cent, 0..100, default 0, unbounded), Seed, Amplitude (per
cent of the bolt's length, 0..100, default 12, hard 0..100), Forking (per cent, 0..100,
default 45, hard 0..100), Decay (per cent, 0..100, default 30, hard 0..100), Core radius
(px@comp, 0..40, default 3, hard min 0), Core colour (default white), Glow radius (px@comp,
0..200, default 22, hard min 0), Glow colour (default a mid blue), Glow opacity (per cent,
0..100, default 70, hard 0..100), Composite on original (default on), Mix.

**Algorithm sketch.** In two halves. The bolt is *built* in Rust, once a frame, as a list of
straight segments; the kernel then only measures distances to them.

```
# host-side, in `packed()` — the whole of the randomness lives here
for each bolt (one, or five for Omni, or two for Two-way strike):
    n     = 24 steps between the bolt's two ends
    for i in 0..=n:
        t   = i ÷ n
        env = t                         # Direction: the far end is free
              sin(π·t)                  # Strike and Two-way strike: both ends are pinned
        w   = fbm(seed lane, t·6, Conductivity state)      # the shared noise core, turbulent
        P_i = lerp(A, B, t) + perp(B − A)·(Amplitude ÷ 100)·w·env
    emit the n segments P_i → P_i+1, each carrying a fade of 1 − Decay·t
forks: round(Forking ÷ 100 × 12) of them, each attached at a hashed step of a bolt,
       leaving at a hashed angle, a quarter as long, six segments, fade × 0.6

# per pixel, in the kernel
core = 0 ; glow = 0
for each segment:
    d    = distance from p to the segment (a capsule, clamped projection)
    core = max(core, fade · clamp((core_r + ½ − d) ÷ max(core_r, ½), 0, 1))
    glow = max(glow, fade · clamp((glow_r − d) ÷ max(glow_r, ε), 0, 1)²)
a      = clamp(core + glow·glow_opacity·(1 − core), 0, 1)
colour = Core·core + Glow·glow_opacity·(1 − core)
base   = src, or the empty pixel when Composite on original is off
out    = base·(1 − a) + colour        # colour is already premultiplied by its own weights
out    = src·(1 − mix) + out·mix
```

Five decisions:

- **The geometry is built once, host-side, and travels in the uniform.** A bolt is a recursive
  displacement, and recursion is exactly what a per-pixel kernel cannot afford to redo two
  million times a frame — the obvious build costs about two hundred hashes a pixel for a shape
  that is the same for all of them. Building it once in Rust costs a few hundred
  multiplications a frame, makes the kernel a plain minimum over capsules, and disposes of
  §1.6 for free: **both paths are handed the identical numbers**, so there is no second
  implementation of the generator to disagree with the first. The array is capped at 192
  segments, which is three kilobytes of uniform and more bolt than any Forking setting asks
  for. The rule to carry: *if the randomness does not vary per pixel, it does not belong in
  the kernel.*
- **Conductivity state is a coordinate, not a clock** (§3.53, §3.54, §3.73). It is the depth
  axis of the noise field, so animating it makes the bolt writhe and *scrubbing back gives the
  same bolt back*. AE's control is the same idea under the same name, which makes this the one
  place in the batch where the parity is exact rather than mapped.
- **Four of AE's eight types, and the four are chosen to be visibly different.** Direction (the
  far end wanders free), Strike (both ends pinned, so the bolt lands on the target), Omni (five
  bolts radiating out to the Direction point's radius) and Two-way strike (two bolts meeting in
  the middle). **Breaking, Bouncey, Anywhere and Vertical are not carried**: two of them are
  Direction with a different envelope, one needs an obstacle to bounce off, and one is
  Direction with the x of its endpoint ignored. The import maps them to the nearest of the four
  and reports it — §3.56's thirteen Warp styles again.
- **Core and glow are taken as a maximum over the segments, never a sum.** Every joint in the
  bolt is shared by two segments and every fork meets its parent, so a sum would put a bright
  bead at each of them — a string of pearls rather than a bolt. The maximum makes the union of
  the capsules exactly one shape. The glow's falloff is squared for the reason every glow's is:
  a linear ramp reads as a flat disc with a hard rim.
- **Alpha Obstacle, Turbulence's second axis, Decay Main Core and the Expert group are not
  carried.** Alpha Obstacle asks the bolt to route around the layer's own alpha, which is a
  search rather than a formula; the rest are refinements of a look this effect already reaches
  with Amplitude, Forking and Decay. All of it is reported by the import rather than
  approximated — §3.63's Auto Amounts a second time.

**The Matte scales the bolt's opacity (K-428, §2.6):** the core's own coverage and the Glow
opacity together, per pixel, before the composite — so the bolt fades along its length and
what lies under it is untouched. Not the dissolve, and doubly so: the glow only lights what
the core has not taken, so fading the two together is quadratic in the matte; and with
Composite on original off there is no layer left to fade back to, so a black matte leaves
transparency where a dissolve would hand the picture back.

`moderate` cost — up to 192 capsule distances a pixel, which is the price of not rebuilding the
bolt per pixel — `Exact` ROI. Premultiplied (§2.2). Mix 0 is the bit-exact identity, and so is
Core radius 0 with Glow radius 0.

### 3.75 Radio waves — shapes emitted from a point and expanding

Maps AE's Radio Waves ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Generate** effect: a producer
point emits a polygon or a star at a steady rate, each one expanding, spinning and fading as it
ages.

**Parameters:** Producer x and Producer y (px@comp, default 960, 540), Time (seconds, 0..10,
default 3, hard min 0), Frequency (waves per second, 0.1..20, default 2, hard min 0.01),
Expansion (px@comp per second, 0..1000, default 260, hard min 0), Lifespan (seconds, 0.1..10,
default 2, hard min 0.02), Sides (whole number, 3..64, default 32, hard 3..64), Star (default
off), Star depth (per cent, 0..100, default 50, hard 0..100), Rotation (dial, degrees, default
0), Spin (dial, degrees per second, default 0), Stroke width (px@comp, 0..100, default 4, hard
min 0), Colour (default a pale blue), Opacity (per cent, 0..100, default 100, hard 0..100),
Fade in (per cent of Lifespan, 0..100, default 15, hard 0..100), Fade out (per cent of
Lifespan, 0..100, default 45, hard 0..100), Composite on original (default on), Mix.

**Algorithm sketch.** Every wave is the *same shape* at a different size, so §3.71's sector
solve is done once host-side for a unit shape and scaled per wave:

```
# host-side, once
period   = 2π ÷ Sides
A        = (1, 0)                                   # the unit shape's vertex on the +x ray
B        = plain:  (cos period, sin period)
           star:   (1 − Star depth ÷ 100)·(cos ½period, sin ½period)
m        = the outward unit normal of the edge A→B
k_hi     = floor(Time × Frequency)                  # the newest wave's index (K-399: host-side)
count    = min(ceil(Lifespan × Frequency) + 1, 32)

# per pixel
rel = p − Producer ; r = |rel| ; φ = atan2(rel.y, rel.x) + ½π
acc = 0
for j in 0..count:
    k    = k_hi − j
    age  = Time − k ÷ Frequency
    if k < 0 or age < 0 or age > Lifespan: continue
    rad  = age × Expansion
    a    = |(φ − Rotation − Spin·age) folded into one sector|
    dist = ((r·cos a, r·sin a) − rad·A) · m         # signed, and in pixels
    cov  = clamp((halfw + ½ − |dist|) ÷ max(halfw, ½), 0, 1)
    u    = age ÷ Lifespan
    fade = min(clamp(u ÷ fade_in, 0, 1), clamp((1 − u) ÷ fade_out, 0, 1))
    acc  = max(acc, cov · fade)
a      = acc × Opacity
out    = base·(1 − a) + Colour·a
out    = src·(1 − mix) + out·mix
```

Five notes:

- **Time is a control, and that is the whole conversion.** AE's Radio Waves reads the clock: it
  knows what second it is and emits accordingly. §2.4 does not allow that, so Lumit's Time is
  an ordinary parameter in seconds that the timeline animates — keyframe it linearly and the
  effect is AE's exactly, scrub it and every wave goes back where it was. Frequency, Expansion,
  Lifespan and Spin all keep their per-second units and mean what they say against *that* Time.
  It is §3.53's missing Wave Speed for the third time, and the first where the replacement is a
  clock rather than a phase.
- **One shape, scaled, because a polygon is self-similar.** The sector solve is done for a
  radius of one and every wave multiplies it, so thirty-two waves cost thirty-two multiplies
  rather than thirty-two solves — and Star, Sides and Star depth cost nothing per wave.
  §3.71's fold and its mirror do the rest, which means the distance that comes out is again a
  **true perpendicular distance in pixels** and Stroke width is a width.
- **`floor(Time × Frequency)` is taken host-side**, K-399's rule about a threshold reaching a
  product: the newest wave's index decides *which* rings exist, and one bit of disagreement
  about it is a whole ring appearing on one path and not the other.
- **Thirty-two waves is the cap, and it is a budget** — §3.64's cap that cannot be typed past,
  a second time. Lifespan × Frequency above 32 keeps only the newest 32, which is the far side
  of "too many rings to see" on any frame.
- **The first wave leaves at Time 0.** Waves with a negative index are not drawn, so the effect
  starts empty and fills up — which is what an emitter does, and what makes Time 0 the
  bit-exact identity.

**The Matte scales Opacity (K-428, §2.6),** per pixel and before the composite, so the rings
fade out across the frame rather than the frame fading back to what was underneath. With
Composite on original off a black matte leaves the pixel transparent, which is what "draw
nothing here" means when the layer that arrived has already been discarded.

`cheap` cost — one `atan2` and up to 32 cheap rings a pixel, §3.71's admission again — `Exact`
ROI. Premultiplied (§2.2). AE's Wave Type of Image Contours and Mask are **not carried** (see
§3.76 for the contour half; the mask half is the seam recorded in
[impl/ae-effect-parity.md](impl/ae-effect-parity.md)), and with only Polygon left the control
is not shipped at all. Mix 0 and Time 0 are both the bit-exact identity.

### 3.76 Vegas — marching lights along a contour or a mask

Maps AE's Vegas ([11-AE-IMPORT.md](11-AE-IMPORT.md)), **both halves** since K-408. A
**Generate** effect that runs a dashed stroke along a line: either a contour it finds in the
picture it was given, or — with Source set to Mask/Path — a mask you have drawn (§1.2).

**Parameters:** Source (Luminance / Alpha / **Mask/Path**, default Luminance), Mask (mask-path
row, unset is First mask; greyed unless Source is Mask/Path), Threshold (per cent, 0..100,
default 50, hard 0..100; greyed *while* Source is Mask/Path, since a level needs a picture to
be a level of), Width (px@comp, 0..50, default 3, hard min 0), Hardness (per cent,
0..100, default 50, hard 0..100), Segment length (px@comp, 1..1000, default 80, hard min 1),
Length (per cent of a segment that is lit, 0..100, default 55, hard 0..100), Rotation (dial,
degrees — one full turn marches the dashes on by one segment), Colour (default a warm yellow),
Opacity (per cent, 0..100, default 100, hard 0..100), Composite on original (default on), Mix.

**Algorithm sketch.** The contour is a *level set*, and the two things a stroke needs — how far
across it a pixel is, and how far along it — both come out of one Sobel:

```
L      = perceptual(luma), or the alpha, smoothed by [1 4 6 4 1] each way ÷ 256
∇L     = the separable 5×5 Sobel over the same neighbourhood, ÷ 128   # per raster pixel
g      = |∇L|
sd     = (L − Threshold ÷ 100) ÷ max(g, ε)     # the signed distance to the contour, in pixels
band   = max((1 − Hardness)·halfw, ½)
across = clamp((halfw − |sd|) ÷ band + ½, 0, 1)

t̂      = (−∇L.y, ∇L.x) ÷ max(g, ε)             # the contour's own direction
phase  = ((p − frame centre) · t̂) ÷ Segment length + Rotation ÷ 360
frac   = phase − floor(phase)
along  = clamp((duty − frac) ÷ max(band ÷ Segment length, 1e-4) + ½, 0, 1)
         # duty = Length ÷ 100, or 2 at Length 100 — a continuous outline must not scallop
         # where the fraction wraps

a      = across · along · Opacity
out    = base·(1 − a) + Colour·a
out    = src·(1 − mix) + out·mix
```

Five decisions:

- **The contour is a level set, not an edge-detector's output.** The obvious build — threshold
  the gradient magnitude — gives a band whose thickness is decided by how steep the picture
  happens to be there, so Width would be a control that does nothing on a soft edge and
  everything on a hard one. Dividing the *distance in value* by the gradient turns it into a
  distance in **pixels**, which is Width's unit, and it costs one divide. The same expression
  also switches the effect off where the picture is flat, because a vanishing gradient sends
  the distance to infinity rather than to zero.
- **The gradient is 5×5, and the two extra taps each way are what make the dashes possible.**
  The stroke needs the contour's *direction*, not only its position, and a 3×3 Sobel taken on
  compressed footage points a different way in almost every pixel — the dashes come out as
  speckle rather than as a line. Five taps of smoothing either way steadies the direction
  without moving the contour, because the smoothing is symmetric. It is the same observation
  §3.66 makes about where a gradient should be taken, one step further along.
- **The dashes are laid out in screen space along the contour's own direction**, which is the
  honest form of AE's Segments for an effect that never traces a path. AE counts segments
  *around* a closed contour, so its Segments control is a number; Lumit has no contour to count
  around, so **Segment length is a length in px@comp** and Length keeps AE's meaning as the lit
  share of one. On a straight contour the two are the same picture. On a strongly curved one
  Lumit's dashes drift in phase where AE's stay evenly spaced, which is the price of not
  tracing, and the import reports it.
- **The phase is measured from the middle of the frame**, which is worth a line because it is
  not cosmetic. An error of ε in the contour's direction moves the dash's phase by `|p|·ε`, so
  measuring from the corner gives the far side of the frame twice the wobble of the near side.
  Halving the arm halves it, for nothing.
- **Rotation marches them, and it is an angle because AE's is.** One full turn advances the
  dashes by exactly one segment, so a linear keyframe on it is the marching-ants animation the
  effect exists for. It is the same trick §3.37's Evolution plays: an angle the timeline turns.
- **AE's Mask/Path source is carried, and on it Segment length means AE's Segments.** The
  decision above is a decision about *contours*, and it is the price of not having a path to
  trace. A mask is a path, and §1.2 measures its arc length on the way over, so this half spaces
  its dashes by distance **round the line** — evenly, however hard it curves, which is exactly
  what AE's Segments count does and what the contour half cannot do. Nothing else changes: the
  same Width, Hardness, Segment length, Length, Rotation, Colour and Opacity draw the same
  stroke, so the two sources are the same effect and not two. The path half is drawn by the
  shared kernel §3.78 and §3.79 use, since a dashed stroke round a curve is what all three of
  them are.
- **The gradient is taken on the perceptual value** (K-404), for §3.66's reason: a contour
  taken in scene-linear light sits wherever the highlights are, and Threshold would be a
  control that spends its first eighty per cent doing nothing.

**The Matte scales Opacity (K-428, §2.6),** per pixel and before the composite, on **both**
halves: the contour kernel and the shared path drawing claim it the same way, so a stroke
marching round a level set and one marching round a mask fade identically. With Composite on
original off a black matte leaves the pixel transparent.

`cheap` cost, `PaddedPx(1)` ROI (a 3×3 Sobel — declared for the contour half, and kept
on the path half, where it is merely generous). Premultiplied (§2.2). Mix 0 is the bit-exact
identity, and so are Width 0 and Opacity 0 — and on Mask/Path, so is an unset mask row, a
deleted mask or a layer with no masks (§1.2’s documented no-op).

### 3.77 Add grain — film grain laid on by tone

Maps AE's Add Grain ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Generate** effect beside §3.36
Noise, which is the same family of thing done plainly: this one has a *size*, a *softness* and
a tonal response, which is what separates grain from static.

**Parameters:** Intensity (per cent, 0..200, default 50, hard min 0), Size (px@comp, 0.5..20,
default 2, hard min 0.1), Softness (per cent, 0..100, default 50, hard 0..100), Red, Green and
Blue (per cent, 0..200, default 100 each, hard min 0), Monochrome (default off), Shadows,
Midtones and Highlights (per cent, 0..200, default 100 each, hard min 0), Animate (default on),
Seed, Mix.

**Algorithm sketch.** Grain is a signed wobble added to the picture's *perceptual* value, at a
scale of its own, weighted by where the pixel sits on the tone range:

```
u        = the unpremultiplied pixel (§2.2)
v        = perceptual(u_c)                         # √, the §3.58 curve
H0       = clamp(1 − 2v, 0, 1) ; H2 = clamp(2v − 1, 0, 1) ; H1 = 1 − H0 − H2
weight   = Shadows·H0 + Midtones·H1 + Highlights·H2      # three hats summing to one
q        = ((x + ½), (y + ½)) ÷ Size                     # grain cells, raster px
lane     = Monochrome ? 0 : the channel                  # 0, 1, 2
hard     = hash01(Seed, lane, ⌊q.x⌋, ⌊q.y⌋, tick)·2 − 1  # one flat cell
soft     = value3(Seed, lane, q.x, q.y, tick)            # the same lattice, interpolated
g        = hard + (soft − hard)·(Softness ÷ 100)
v'       = v + g·(Intensity ÷ 100)·0.1·weight·channel gain
out_c    = max(v', 0)²
```

Four notes:

- **The grain is added where the eye is** (K-404), which is what makes Intensity mean one thing
  across the frame. Added in scene-linear light, the same amount of grain is invisible in the
  shadows and a blizzard in the highlights, because linear light spends most of its range on
  the top stop. The curve is §3.58's `sqrt` and its exact inverse, so the round trip costs two
  instructions and is bit-stable on both paths.
- **Softness is a crossfade between the same field read two ways**, not a blur. The hard
  reading takes one value per cell — a flat square, which is what a grain particle is — and the
  soft reading interpolates the same lattice smoothly. Blending them costs one extra hash and
  gives a control whose two ends are both correct: 0 is a sharp scan-grain, 100 is a soft
  organic mottle. A real blur would have cost a second pass for a control nobody keyframes.
- **The three tonal weights are hat functions summing to one** — §3.62's argument in another
  costume. At the default 100/100/100 the weight is exactly 1 everywhere, so the three controls
  are provably neutral until one of them is moved, and no combination of them can put a seam in
  a gradient.
- **Monochrome is a lane, not an average.** The three channels read the noise core's `channel`
  argument — the same decorrelation the fractal sum uses for its octaves — so colour grain is
  three independent fields and mono grain is one field read three times. Neither is the other
  filtered.

The frame's own tick arrives through `resolve_derived` exactly as §3.36's does (K-385), so the
kernel never sees a clock and Animate off pins it to zero. `cheap` cost, `Exact` ROI.
Unpremultiplied (§2.2), for §3.36's reason: grain sprinkled onto premultiplied values would
fade out across a soft edge instead of lying evenly over it. Mix 0 and Intensity 0 are both the
bit-exact identity.

AE's Add Grain also carries a Blending Mode, a Viewing Mode, a Tonal Ranges group with movable
boundaries, and Channel Balance in an expert group. The blending mode is the layer's own
(docs/06), the viewing mode is a preview affordance rather than a look, the tonal boundaries
**The Matte scales Intensity (K-428, §2.6),** per pixel and before the grain is added. Unlike
§3.36's plain additive grain — whose Intensity·k *is* the dissolve, so it keeps the strength
semantic — this one adds its wobble on the **perceptual** value and squares it back, so half
the Intensity is a genuinely finer grain rather than a half-faded coarse one.

are pinned at the hats above, and Channel Balance is Red, Green and Blue. **Remove Grain is not
carried at all** and is not a gap: a denoiser is a programme, not an effect
([impl/ae-effect-parity.md](impl/ae-effect-parity.md)).

---

### 3.78 Scribble — a mask filled with pencil strokes

Maps AE's Scribble ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Generate** effect, and the first
of the two that read a **mask's geometry** rather than the picture (§1.2, K-408): you draw a
mask, this shades it in the way a hand shades a shape with a pencil — parallel strokes at an
angle, running a little past the edges, wavering as they go.

**Parameters:** Mask (mask-path row, unset is First mask), Colour (default a warm red), Angle
(dial, degrees, default 30), Stroke width (px@comp, 0.1..20, default 2, hard min 0.1), Spacing
(px@comp, 1..200, default 8, hard min 0.5), Path overlap (px@comp, −50..50, default 4), Start
and End (per cent of the whole drawing's length, 0..100, defaults 0 and 100), Wiggle type
(Static / Jagged / Wiggly, default Static), Wiggles per second (0..30, default 8, greyed while
the type is Static), Seed, Opacity (per cent, 0..100, default 100, hard 0..100), Composite on
original (default on), Mix.

**Algorithm sketch.** The hatch is laid out **host-side, once a frame**, exactly as §3.74's
bolt is; the kernel is handed straight pieces and draws them:

```
poly      = the mask flattened by the seam (§1.2), taken to raster px            # host
u         = (cos Angle, sin Angle) ; v = (−u.y, u.x)                             # host
for each line offset o = min(p·v) + Spacing/2 + k·Spacing:
    hits  = where the line o·v + t·u crosses the polygon, sorted by t            # host
    spans = the even-odd pairs of those, each grown by Path overlap either end   # host
chain     = the spans end to end, direction alternating line by line,
            with the pen LIFTED between two spans of the same line               # host
pieces    = chain trimmed to the Start..End share of its own drawn length        # host

q         = p + Amplitude·(noise(p·Frequency, tick)·2 − 1)      # the waver, per pixel
d         = min over pieces of the distance from q to the piece
cov       = clamp((Stroke width/2 − d) / ½ + ½, 0, 1) · Opacity
out       = base·(1 − cov) + Colour·cov         # or Colour·cov alone, Composite off
out       = src·(1 − mix) + out·mix
```

Four decisions:

- **The strokes are one continuous line, and the pen lifts across a hole.** A hatch drawn as
  independent segments has no beginning and no end, so Start and End would have nothing to
  measure along and the effect could not draw itself on — which is most of what it is used for.
  Laying it as one line, with the direction alternating so the join between one stroke and the
  next is a short hop along the edge, gives it a length to be trimmed by and makes it read as a
  scribble rather than as a comb. The cost is that a mask with a notch in it has *two* strokes
  on one line, and joining those would draw straight through the hole, so the chain carries a
  **pen lift** — and on a reversed line the strokes are taken in reverse order too, or the pen
  would fly back across the whole shape to start the next one.
- **The waver displaces the paper, not the strokes.** Wobbling the geometry means subdividing
  every stroke into eight or more pieces and hashing each joint, which multiplies the geometry
  budget by the same factor and puts a second copy of the noise in the host. Displacing the
  *sample point* by a smooth noise field costs one lookup a pixel, gives every stroke the same
  hand-drawn waver, and reuses the lattice §3.37 already shares with four other effects. The
  amplitude is a fifth of the Spacing and the wavelength four times it — tied to the gap rather
  than shipped as two more controls, because what the waver must not do is walk one stroke into
  its neighbour, and because at that ratio the displacement is nowhere near steep enough to fold
  the picture back on itself.
- **Wiggle type is about how the waver moves, not what shape it is** — AE's distinction, and one
  line of arithmetic. Static pins the evolution at zero; Jagged floors layer time × Wiggles per
  second, so the waver snaps to a new arrangement that many times a second; Wiggly passes the
  product through, so it drifts. The tick is taken at resolve in `f64` and handed over as a
  number, so the kernel never sees a clock (§2.4) — §3.36's rule again. **Static is pinned in
  two places**, at resolve and in the effect's own packing, for the reason §3.77's Animate
  toggle is: a bag can carry a derived value over from another setting (K-258).
- **Past the geometry budget the spacing widens; the fill never stops half way.** A fine hatch
  over a large mask can want more strokes than the uniform holds (§1.2's carriage is a fixed
  size, exactly as §3.74's is). The failure that keeps the picture whole is a coarser hatch, not
  half a shaded shape, so the spacing is opened until the strokes fit — docs/14 §4, degrade
  rather than fault.

`moderate` cost — up to 512 pieces a pixel, which is §3.74's admission at a larger number —
`Exact` ROI: the kernel reads its own pixel and nothing else, because the drawing arrives as
geometry rather than as a neighbourhood of the picture. Premultiplied (§2.2). **An unset mask
row, a deleted mask, or a layer with no masks all render the input unchanged** (§1.2's
documented no-op), and so do Mix 0, Opacity 0 and Stroke width 0.

**The Matte scales Opacity (K-428, §2.6),** per pixel — applied to the shared path drawing's
coverage, which is where Opacity enters and nowhere else — so the pencil fades along its own
line. On Paint on transparent a black matte leaves nothing at all, which a dissolve cannot do.

AE's Scribble also carries a Fill Type with five modes, Edge Options, Curviness and the three
Variation sliders, and a Blend Mode. Only the single-mask fill is shipped: the blend mode is the
layer's own (docs/06), the variations are per-stroke randomness the waver already supplies in
one control instead of six, and the multi-mask fill types wait on a seam that names more than
one mask ([impl/ae-effect-parity.md](impl/ae-effect-parity.md)).

### 3.79 Stroke — a brush walked round a mask path

Maps AE's Stroke ([11-AE-IMPORT.md](11-AE-IMPORT.md)). A **Generate** effect and the second
reader of §1.2's mask geometry: a round brush runs along the *line* of a mask, from one per cent
of the way round it to another. Keyframe End from 0 and the line draws itself on, which is most
of what the effect exists for — and it is the thing a coverage buffer cannot do, because a hole
cut in a picture cannot say which way is *along*.

**Parameters:** Mask (mask-path row, unset is First mask), Colour (default white), Brush size
(px@comp — a **width**, as Vegas' is, 0.5..100, default 8, hard min 0.1), Brush hardness (per
cent, 0..100, default 75, hard 0..100), Spacing (per cent of the brush's width, 1..500, default
15, hard min 1), Start and End (per cent of the way round, 0..100, defaults 0 and 100), Paint
style (On original / On transparent / Reveal original, default On original), Opacity (per cent,
0..100, default 100, hard 0..100), Mix.

**Algorithm sketch.** The brush's trail is laid out host-side, and which *shape* it is laid out
as depends on how far apart the stamps are:

```
s         = Spacing/100 · Brush size                                   # raster px
if s <= Brush size / 2:                                                # they overlap
    pieces = the mask's own polyline, trimmed to the Start..End share of its length
else:                                                                  # they do not
    pieces = a stamp with no length at each arc mark Start + k·s, up to End

d         = min over pieces of the distance from the pixel to the piece
band      = max((1 − Brush hardness)·Brush size/2, ½)
cov       = clamp((Brush size/2 − d) / band + ½, 0, 1) · Opacity
out       = base·(1 − cov) + Colour·cov       # On original
          = Colour·cov                        # On transparent
          = base·cov                          # Reveal original — colour and alpha alike
out       = src·(1 − mix) + out·mix
```

**The Matte scales Opacity (K-428, §2.6),** per pixel — applied, as Scribble's and Vegas'
Mask/Path half are, to the shared path drawing's `cov`, which is where Opacity enters and
nowhere else. The brush therefore fades along the line it is walking. On Reveal original a
black matte leaves nothing rather than handing the picture back, which is the honest reading:
the drawing is the hole, and where nothing is drawn there is no hole.

Three decisions:

- **A dense stroke is drawn as the path it sweeps, not as the stamps that make it.** The union
  of round stamps spaced under half a brush width apart *is* the capsule the brush sweeps: the
  deepest scallop between two of them is an eighth of the radius, under a pixel for any brush
  anyone would see it on. Drawing the path directly is the same picture for a fraction of the
  pieces, and it is the only form that fits a long path with a fine brush inside §1.2's budget —
  a four-thousand-pixel path with a two-pixel brush at AE's default Spacing wants sixteen
  hundred stamps and needs a hundred pieces. Once the stamps are further apart than half the
  brush they stop being a stroke and become a dotted line, which is what the control is *for*,
  and then they are drawn as what they are. Past the budget the **dots space out** rather than
  the trail stopping short of the End mark.
- **Brush size is a width, not a radius.** AE's is a radius; every length in Lumit's catalogue
  is the thing itself (§3.76's Width, §3.78's Stroke width), and one effect measuring halves
  would be the one somebody sets wrong. The import doubles it and says so.
- **Reveal original is a matte, and it takes the alpha with it.** The picture survives only
  where the brush went — colour *and* coverage, which is what premultiplied means (§2.2).
  Writing the colour through and leaving the alpha alone would give a stroke-shaped brightness
  over a full-coverage frame, which is not what the option is for.

`moderate` cost and `Exact` ROI, for §3.78's reasons. Premultiplied (§2.2). **An unset mask row,
a deleted mask, or a layer with no masks all render the input unchanged** (§1.2's no-op), and so
do Mix 0, Opacity 0 and Brush size 0.

AE's Stroke also carries **All Masks** and **Stroke Sequentially**, and neither is shipped:
§1.2's row names *one* mask by design, so both wait on a seam that names a set
([impl/ae-effect-parity.md](impl/ae-effect-parity.md)). Its Paint Style options are carried
whole.

**The three of them share one kernel**, and that is worth a line because it looks like three
effects and is not. A scribble, a brush trail and Vegas' dashes round a mask differ entirely in
*where the line goes* — which is decided host-side, in three quite different pieces of code —
and hardly at all in *how it is drawn*: a maximum over capsules, a soft edge, a dash gate, a
paint style. The dash gate is switched off by the same convention §3.76 already uses for a
continuous outline (a lit share of 2, which cannot be reached by wrapping), so there is not even
a branch between them. One kernel, one CPU reference, one §1.6 oracle covering all three.

### The Controls category — the effects that change no pixel (K-414)

The five sections below are one family and are read together. Each is a **parameter-only
identity effect**: it declares one row, renders nothing, and exists so that *other*
properties can read that row through an expression and so that the timeline can keyframe it
in one place. They are After Effects' Expression Controls, and half the CC-pack rigs in the
world are wired through them — one slider on a null driving six things at once, which is a
rig anybody can find and adjust rather than six copies of the same keyframes.

Four facts hold for all five, so none of them repeats it:

- **They have no image operation.** The declaration says so
  ([impl/effect-registry.md](impl/effect-registry.md)'s `is_image_op`, the same answer
  §3.25 Posterize time and §3.26 Motion blur give for their own reason), and the resolve
  step pushes no op for them at all — nothing is dispatched on either render path, so
  there is no WGSL kernel, no CPU reference and nothing for §1.6 to compare. They are the
  first effects in the catalogue for which that is true because they draw nothing *ever*,
  rather than because they act a level above the stack.
- **They take no matte.** §2.6's row is a picture that drives an effect, and an effect that
  touches no pixel cannot be driven by one; the schema declares `MatteRole::None`, which
  these five are the first to use.
- **They have no Mix.** Mix is the dissolve back towards the untouched picture (§1.5), and
  there is no touched picture to dissolve from.
- **§1.2's "no no-op default" rule does not apply to them.** A control's default is a
  starting value for someone else's expression, and a Slider control that arrived at some
  arbitrary non-zero number would be a lie about the rig.

They sit **last in the Add-effect menu** (K-137's order is the catalogue's, and the family
is appended at its end), in After Effects' own order: Slider, Angle, Checkbox, Colour,
Point.

### 3.80 Slider control — one number, held

**Parameters:** **Slider** (default 0, slider 0..100, unbounded).

The plain number an expression reads. Its range is **not** the §1.2 Slider *kind* K-414 also
introduced, and the distinction is the whole point of that kind: a Slider control's numbers
mean whatever the property reading them means, so a rig wanting 0..3000 is as ordinary as
one wanting 0..1, and the 0..100 travel is only where the thumb starts. The Slider kind is
for the opposite case — a parameter whose whole meaning lives inside a closed range, which
is why the four wipes' Completion adopted it and this row did not.

### 3.81 Angle control — one angle, held

**Parameters:** **Angle** (default 0°, dial, unbounded).

§3.80 with a dial. Unbounded on purpose, exactly as every other angle in the catalogue is
(§1.1's angle type): an angle animates *through* full turns rather than stopping at 360,
and a rig that spins something depends on that.

### 3.82 Checkbox control — one switch, held

**Parameters:** **Checkbox** (default off).

The two-way choice an expression branches on, made once in a visible place rather than
buried in a script. Off by default, which is the reading a rig nobody has set up yet
should get.

### 3.83 Colour control — one colour, held

**Parameters:** **Colour** (default white, scene-linear RGBA, edit range 0..4).

Where a rig keeps its colour, so that changing the swatch once changes the six effects
reading it. The 0..4 edit range is the one every colour that carries light declares
(§2.1): whatever reads this is going to put it in a picture, and a value above 1 is a real
value there.

### 3.84 Point control — one place in the frame, held

**Parameters:** **Point x**, **Point y** (px@comp, default the centre of the comp).

A crosshair that can be dragged on the picture and keyframed, so one dragged point moves a
flare, a mask and a light together. It is two parameters rather than one because a point in
Lumit *is* an adjacent `_x`/`_y` pair the panel folds into one row with a crosshair pick
(§1.1) — a point needs no schema kind of its own, only the naming convention. The numbers
are px@comp (K-260), and a fresh instance is centred on the actual comp rather than on the
schema's nominal 1080p default, for the reason §3.48's corners are.

**The import** (docs/11 §5) maps all five one for one — one property onto one row, same
units, nothing converted and nothing left behind, which makes them the only rows in the
table with no report of any kind. Their match names are the famous ones (`ADBE Slider
Control` and kin) but are **not yet in the audited set**, so they enter the table marked
**pending** until the next audit sitting confirms them; `tools/ae-audit/
claimed-matchnames.txt` carries them so that sitting is already prepared. A match name
that turns out to be wrong costs nothing worse than the placeholder road docs/11 §6
already specifies for every unclaimed name.

---

### 3.85 Camera track — the handle for a camera solve

**Parameters:** **Analyse** (action), **Cancel** (action), **Feature density** (Low /
Normal / High, default Normal), **Use masks** (default on), **Show points** (default on).

Applied to a footage or precomp layer, this effect **renders identity**: it is not a look,
it is a button and a readout. Pressing Analyse starts a camera solve (K-415's pipeline,
[impl/tracking.md](impl/tracking.md)) on its own thread, and you keep editing while it runs
— the working shape After Effects has, and the reason the controls are an effect on the
tracked layer rather than a modal window that owns the application (K-417's first ruling).

**The analysis is keyed to the source, not the clip** (K-248, K-417's second ruling). The
job tracks and solves the **entire unaltered source clip**, keyed by (media, analysis
settings), cached in the `track/` sidecar ([10-FILE-FORMAT.md](10-FILE-FORMAT.md) §3) and
rebuildable like every sidecar tier — deleting it at any moment costs one re-analysis and
nothing else. Every clip of that footage — trimmed, reordered, speed-ramped, retimed, in a
Sequence layer or not — reads the same solve through its own time mapping, so reordering
cuts or changing a speed never re-tracks anything. Because the key describes the *file*
rather than the project, a second project cutting the same rushes reads the same solve
without tracking it again. **Not built yet:** the effect on a *Precomp* layer analyses
nothing — the solve link already resolves through a precomp to the footage inside, but
analysing a nested comp means rendering it rather than decoding it (docs/TODO.md).

**Feature density** is the one quality knob worth a row: it sets the detection grid and the
best-N per bucket the tracker uses (Normal is the tracker's own default, so the middle
option changes nothing). **Use masks** is K-408's mask carriage put to work — a track is
neither born in nor allowed to wander into a masked region, which is how a moving subject is
excluded by hand rather than argued with. **Show points** draws the solved cloud over the
picture on this layer, depth-cued, on after a solve (K-417's fourth ruling); selected points
make a Null or a Solid at their mean solved position.

**The status is not a parameter.** A parameter is something the document stores and the
timeline animates, and "solving, frame 214 of 900" is neither: it is live job state, and it
crosses to the panel as job state. A string row pretending to be one would put a progress
bar in the save file. It draws as **one calm line under the two buttons**
([07-UI-SPEC.md](07-UI-SPEC.md) §6): how many frames have been followed, that the camera is
being solved, or — when it is done — the point count and the mean reprojection error, which
together are the one number that says whether the solve is any good. A refusal is a plain
sentence in the same line, and nothing about the shot has changed. **Create camera** sits
beside it once there is a solve to follow.

**A track may cover part of its clip, and says which part** (K-540). Not every shot can be
followed to its end — the lens racks, the frame whites out, a cut lands mid-clip — and where
the specks stop crossing from one frame to the next there is nothing after that point that
can be related to anything before it. The analysis **stops there**, solves the span that
worked and keeps it: a thin bar above the status line shows that span against the rest of the
clip, and the line says how far it got instead of quoting an error over frames it never saw.
A camera linked to a partial solve derives inside the span and **holds** the last derived
pose outside it, which is K-417's rule meeting a range that ends early. Re-analysing after
masking the offending region, or with a different Feature density, is the way past it; a
partial answer is honest, usable and cached like any other.

What the solve is *for* is the **solve link** on a Camera layer
([03-DATA-MODEL.md](03-DATA-MODEL.md) §5.6): the camera points at this layer and derives its
placement per frame, rather than being handed a copy. Create camera makes one; while it is
linked its Transform heading wears a calm badge saying which of the three link states it is in,
with **Convert to keyframes** beside it ([07-UI-SPEC.md](07-UI-SPEC.md) §2.3.6). Its rows are
still the user's to drag — what they hold is a correction on top of the solve (K-578) — and
this effect's status row carries the *edited since track* dot when one has been made.

Like the Controls family above, it declares no image operation, takes no matte and has no
Mix — for the same reason and by the same mechanism, though it is a Utility rather than a
Control: it holds a *job*, not a value.

### 3.86 Particulate — a particle system that is arithmetic, not history

A **Generate** effect (K-446), and the first that hands something out beside its picture: a
declared `Points` output carrying the same particles it drew, for the family that will
consume them ([impl/points-stream.md](impl/points-stream.md)). The design is
[impl/particulate.md](impl/particulate.md); this entry is the catalogue's account of it.

**Parameters**, in four groups. *Emitter*: Shape (Point · Line · Ellipse · Rectangle · Mask
path · **Ellipse outline** · **Rectangle outline** — K-597), Position (px@comp), **Position z** (px@comp, default 0 — K-561), Width / Height
(px@comp, 0–2000, default 400), **Depth** (px@comp, 0–2000, default 0 — K-561), Emitter
angle, Mask path (K-408), Emit rate (per second, 0–1000, default 150), Direction (degrees,
default −90), **Direction z** (degrees, default 0 — K-561), Spread (degrees, 0–360, default
360), **Spread z** (degrees, 0–180, default 0 — K-561), Initial speed (px@comp/s, default
90), Speed jitter (per cent, default 50). *Particle*: Life (seconds, default 2), Life jitter (per cent,
default 30), Size (px@comp, default 4), Size jitter (per cent, default 40), Size over life
and Opacity over life (curves, K-412 — flat, and 1 → 0), Colour and End colour
(scene-linear, values above 1 legal), Rotation, **Rotation jitter** (degrees, 0–360, default
360 — K-507), Spin (degrees/s), Align to motion. *Forces*: Gravity (px@comp/s², and **down stays down** — K-561),
Wind x / Wind y / **Wind z** (px@comp/s — they act **through** Drag, and with Drag 0 they
do nothing at all), Drag (per second, default 0.5), Turbulence amount / scale / speed. *Render*: Mode
(Disc · Sprite · Streak), Feather (per cent), Sprite layer (K-123, K-142), Streak length
(seconds), **Max particles** (1–200 000, hard 1 000 000, default 20 000 — not animatable),
Seed. Plus the host Mix with its Blend (K-425).

**Algorithm sketch.** No frame reads any other frame. The whole of a particle's life is
decided by its **birth index** and a seed, so "where is it at frame 500?" is arithmetic:

```
carry += rate(f)·Δt ; n_f = floor(carry) ; carry -= n_f     # host, one scalar a frame
t_b   = frame start + (j + ½)·Δt/n_f                        # birth time, spread in-frame
die(a)= hash(seed, birth index, a)                          # every per-particle draw
age   = t − t_b ;  alive if 0 ≤ age < life(die)
p(age)= p0 + w·age + (v0 − w)·age·r(k·age) + g·age²·s(k·age) # closed form, no g/k
Δp    = amount · noise3(p0/scale + phase, age·turb_speed)    # turbulence displaces
keep the newest `cap` alive by birth index                   # the cap rule
seen  = M·(x, y, z, 1) ; draw at seen.xy/seen.w, size ÷ seen.w  # the comp's camera
```

Eight notes:

- **The forces are the set with closed-form integrals** (K-474), and that is the selection
  criterion rather than a styling choice. `r` and `s` are `(1 − e^(−x))/x` and its
  companion, written so **neither divides by the drag** — the published `g/k` form is
  infinite at zero drag — with a series branch below `x = 0.1`. The note says `1e−4`; in
  f32 the cancelling form has lost three of its seven digits by there, so the two branches
  met parts-in-a-thousand apart, and 0.1 with one more term is where they genuinely agree.
- **Forces are sampled at the current frame** and treated as constant over each life. Move
  a Gravity keyframe and the whole field leans, which is physically wrong and what a motion
  designer expects. Integrating a changing force *is* the simulation this design excludes.
- **The GPU path is four passes** — count the live candidates, prefix-sum their ranks, place
  them, draw one instanced quad each — and the compaction is a **prefix sum, never atomics**
  (§2.4): a slot has to be a function of the birth index, or `id` order would be a
  scheduling artefact and two renders of one frame could disagree.
- **All three modes are one quad with a different coverage inside it.** A disc is a
  feathered circle; a streak is that circle swept to `p(t − length)`, found by the closed
  form again and not by history, and so exactly the disc when the length is zero; a sprite
  is the referenced layer's picture in the quad. **An unset Sprite layer draws discs** —
  the documented deviation from the unset-is-identity convention, because a render mode
  must always draw something.
- **Max particles is the budget dial** (K-475, [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)
  §2): it is the declared peak scratch, which is why it is a parameter and not a guess. Over
  budget, the **newest** cap by birth index survives — visible, deterministic, and identical
  from any scrub direction. Under governor pressure the same rule at half the cap is the
  degradation rung: interaction only, never on export (docs/06 §6.2).
- **Three things ride beside the op**, because none of them is a number anybody typed: the
  layer's clock, the birth schedule — the whole history of the Emit rate track rather than
  its value now — and, since K-561, the composition's camera. The draw builder works out all
  three, exactly as it flattens a mask polyline (§1.2).
- **The two outline shapes emit along a perimeter** (K-597), uniformly by arc length, and
  they are the *same walk* a Mask path emitter already does: the host flattens the ellipse
  or the rectangle into a polyline in the emitter's own local frame — vertices on the true
  curve, 128 chords for an ellipse — and both render paths walk that one polyline, so
  neither can come to flatten it differently. The outline is then turned by Emitter angle
  and placed at Position exactly as the area it hollows out would have been, and Depth
  fills through it, so a cylinder becomes a tube. The two codes are **appended** to the
  Shape list rather than slotted in beside their fills: a Choice is stored as its index
  (K-065), so inserting one would quietly turn every saved Mask path emitter into a
  Rectangle. Uniform by arc length rather than by angle is the whole point of the walk — on
  a 2:1 ellipse, parameterising by angle crowds the ends of the long axis by a factor of
  two.
- **The particles live in three axes, and the composition's camera sees them** (K-561,

  K-596). A
  particle carries a depth, the closed forms integrate it under the same drag and wind
  algebra (gravity excepted — down stays down), the emitter reaches through the layer's
  plane, and turbulence gained a third lattice channel on the rule that a jitter with an x
  and a y gains a z. **On a 3D layer** the stream projects through the comp's active camera
  exactly as the layer's own transform does — the same `camera_matrix` and `place_matrix`
  the compositor places layers with, restricted back onto the layer's plane, so there is no
  second camera in the engine (the precedent is K-406's ruling on Card wipe: Lumit keeps
  cameras on the composition). **On a 2D layer nothing changes**: the projection is flat and
  every project saved before the axis existed draws the picture it always drew, bit for bit,
  which is the K-258 gate and is tested as one. Depth-map occlusion and collision remain out
  of scope. The wire stays one type: a consumer reads the projected pair unless it declares
  3D awareness on its port, which is how Points sample keeps measuring its Nearest distance
  where the viewer sees the particles.

`moderate` cost, `FullFrame` ROI (a particle may travel anywhere), premultiplied, temporal
window `{0}` — the payoff of the closed forms, and what makes scrubbing one evaluation.
`sample_temporally` on, so accumulation Motion blur gets true particle motion for free
(K-132). Mix 0, Max particles 0 and an Emit rate that has produced nothing are all the
bit-exact identity, as is a Mask path emitter whose row comes to no mask — the documented
no-op (K-408).

**The Matte takes the generic strength semantic** (§2.6): the particles are drawn in full
and dissolved back by the matte's luma afterwards, which is the right reading for an effect
that *adds* light rather than grading what was there.

AE's nearest equivalents are CC Particle World and Particular, both of which are
simulations; what is deliberately absent here is collisions, flocking and any other
per-particle interaction, and the contract a future **Simulate** mode would have to meet is
written down rather than left to be improvised
([impl/particulate.md](impl/particulate.md) §8).

---

### 3.87 Planar track — one flat surface, followed onto a Corner pin

**Parameters:** **Analyse** (action), **Cancel** (action), **Create corner pin** (action),
Upper left / Upper right / Lower left / Lower right (px@comp — the quad, four point rows),
**Pin layer** (a layer reference), **Feature density** (Low / Normal / High, default
Normal), **Use masks** (default on).

Applied to a footage layer, this effect **renders identity**, exactly as the Camera track
above it does and for the same reason: it is a handle holding a job. The four points enclose
something flat in the shot on its **first frame** — a phone screen, a sign, a poster, a
laptop lid — and Analyse follows that surface through the clip, frame by frame, as four
corners. **Create corner pin** then puts a Corner pin (§3.48) on the layer **Pin layer**
names, with its eight numbers keyframed to those corners: one key per composition frame, in
px@comp, ordinary keyframes the graph editor draws and the user owns from the moment they
land. That is the whole screen-replacement gesture, and it is the reason the Corner pin's
Tier-2 row has always said "export target for the tracker".

**Why a second effect and not a mode on the Camera track** (K-579). The two share their
first step — the same detector, the same pyramidal KLT, the same exclusion masks — and
nothing after it. A camera solve answers *where the camera was*: one answer for a whole
file, shared by every clip of it, read by a Camera layer through a link, carrying a point
cloud and a focal length. A planar track answers *where this surface is*, which is a
property of the quad somebody drew rather than of the file: two quads on one shot are two
different answers, and neither is the other's. §4's Tracker row already frames planar
tracking as its own thing beside the camera solve, and folding them together would make
every row of both conditional on a mode and every reading downstream a union that has to be
unwrapped before it can be drawn.

**A flat surface is eight numbers, and that is the whole idea.** However the camera moves
and however the surface turns, what it does to the picture of a *plane* is always a
homography — the same four-corner projective stretch the Corner pin applies. So the analysis
does not ask where the surface went; it asks which homography this frame is, over every
feature the quad holds, robustly (LO-RANSAC over four-point DLT, the same machinery the
camera solve's two-view geometry uses). The maths and the drift handling are
[impl/tracking.md](impl/tracking.md) §6.

**It is measured from the reference frame, not from the frame before.** Chaining frame to
frame multiplies every step's small error into every step after it, and a long shot's quad
walks quietly off the surface. Each frame is fitted against the frame the quad was drawn on
instead, so no frame's error depends on any other's. When the reference frame's features run
out — the surface turns away, someone walks in front — the measurement **re-anchors** to a
recent frame and remembers the one homography that reaches it, so error accumulates once per
re-anchor rather than once per frame. The status line says how many re-anchors there were,
because that is the one number that says how much to trust the far end.

**Use masks** is K-408's mask carriage doing a second job here: the quad already says where
to look, and a mask says what to ignore *inside* it — the hand crossing the phone, the
reflection sliding over the sign. **Feature density** is the Camera track's own row, meaning
the same three things to the same detector.

**The status is not a parameter**, for §3.85's reason, and it draws the same way: one calm
line under the buttons, with the thin span bar above it when the track covers part of its
clip. A track can stop part-way exactly as a camera solve can, and says so rather than
inventing the rest. A refusal is a plain sentence — too little inside the quad to follow, or
contents that are not one flat surface — and nothing about the shot has changed.

**Known limits, stated.** The quad is read **statically**, at layer time zero: it is the
shape the surface has on the reference frame, and animating it would be asking the tracker
to follow a moving target from a moving start. The four points are edited as rows in the
panel rather than dragged on the picture; on-canvas handles are owed
([TODO.md](TODO.md)). And the effect analyses a **footage** layer only — a Camera track on a
Precomp layer renders the nested comp to track it (K-577), and the same for a planar track
is owed rather than pretended.

---

### 3.88 Grid — a lattice of points, and the discs that show it

A **Generate** effect (K-598), and the first of the points **generators**: like Particulate
it declares a `Points` output beside its picture, and unlike Particulate it has no time in
it at all. Rows, columns and planes, spaced by a distance you type, with a jitter dial per
axis. There is nothing to be born, nothing to age, nothing to carry off — a cell is where
the arithmetic says it is, at every frame, for ever.

**Parameters**, in two groups. *Grid*: Columns (1–100, hard 1 000, default 10), Rows
(default 6), **Planes** (default 1 — the count *through* the layer's plane, K-561), Spacing
x / Spacing y / Spacing z (px@comp, 0–1000, default 120), Position x / y / **z** (px@comp —
the lattice's centre), Jitter x / y / z (px@comp, 0–500, default 0), Seed. *Point*: Size
(px@comp, default 8 — the diameter of the disc a point is drawn as), Feather (per cent),
Colour (scene-linear, values above 1 legal), **Max points** (1–200 000, hard 1 000 000,
default 20 000 — the budget dial, not animatable). Plus the host Mix with its Blend (K-425).

**Algorithm sketch.** One expression, no walk:

```
i     = ((plane · Rows) + row) · Columns + column     # the point's id, for ever
p_i   = Position + (column − (Columns−1)/2)·Spacing x , … same in y and z
p_i  += (hash(Seed, i, axis) − ½) · Jitter axis       # the one seeded draw
keep the first `Max points` by index                  # the cap rule, a generator's shape
seen  = M·(x, y, z, 1) ; draw at seen.xy/seen.w, size ÷ seen.w   # the comp's camera
```

Five notes:

- **`id` is the index, and the index is the walk.** A consumer following one cell keeps
  following it while the lattice grows around it — the same promise Particulate's birth
  index makes, from the same place: an ordering that is a fact of the arithmetic rather
  than an artefact of how it was computed.
- **The cap rule keeps the *first* cap, not the newest.** Particulate keeps the newest
  because a particle set has a birth order and the newest are the ones the eye is
  following; a lattice has no birth order at all. What survives is a prefix of the one
  fixed ordering — deterministic, identical from any scrub direction, and reached by
  *stopping* rather than trimming, so an over-large lattice costs the cap and not the
  lattice.
- **The points are drawn on the host and posted to the card** (K-598). Particulate works
  its particles out in a compute pass because there can be a million of them and each is a
  page of algebra; a lattice is a few hundred cells of arithmetic, and posting the answers
  costs less than a pass would. What it buys is better than speed: the points the effect
  *draws* are bit for bit the points its CPU reference evaluated, through the very
  instanced quad Particulate's discs go through, so the two paths cannot describe different
  lattices.
- **Mix at nought emits the stream and draws nothing**, which is this effect's emit-only
  mode without a row of its own — the row every effect already carries, meaning what it
  always means.
- **Not seeded**, in the §1.3 sense, and deliberately unlike Particulate: `seeded` says the
  pixels are a function of *time* under constant parameters, and folds the layer's clock
  into the cache key for it. A lattice has no clock in it, and folding one in would retire
  every cached frame on every scrub for nothing. The jitter is still a seeded, stateless
  hash (§2.4) with the standard reseed button.

`moderate` cost, `FullFrame` ROI (a jittered cell may be anywhere), premultiplied, temporal
window `{0}`. Mix 0 and Max points 0 are the bit-exact identity. **The Matte takes the
generic strength semantic** (§2.6), the same reading Particulate takes: the points are
drawn in full and dissolved back by the matte afterwards.

AE has no equivalent; the closest things are the grid-of-copies rigs people build by hand
out of Repeaters and expressions, which is the work this exists to delete once Clone to
points lands ([impl/points-stream.md](impl/points-stream.md) §2.3).

---

### 3.89 Scatter — points thrown at the picture, kept where there is alpha

A **Generate** effect (K-599), the second points **generator** and the first thing in the
family whose stream depends on the *picture*. Grid puts points where the arithmetic says;
Scatter throws them at random and keeps the ones that land on something. "Something" is the
layer's own alpha — or a matte layer's, if one is bound — so a silhouette, a piece of text
or a keyed subject becomes a cloud of points in the shape of itself.

**Parameters**, in two groups. *Scatter*: Density (0–100, default 20 — candidate points per
hundred-pixel square **of the composition**), Seed. *Point*: Size (px@comp, default 6),
Feather (per cent), Colour (scene-linear), **Max points** (1–200 000, hard 1 000 000,
default 20 000 — the budget dial, not animatable). Plus the host Mix with its Blend (K-425),
and the Matte row, which this effect **claims inside its own maths** (§2.6, K-395).

**Algorithm sketch.** Rejection sampling, and nothing else:

```
n     = round(Density · comp area / 100²)   , capped at Max points   # the candidates
p_i   = ( hash(Seed, i, u)·w , hash(Seed, i, v)·h )                  # where i falls
a_i   = alpha under p_i    (of the matte if bound, else of the input; Invert flips it)
stands if a_i > hash(Seed, i, accept)                                # the acceptance die
```

Six notes:

- **Rejection, not thresholding**, and that is the whole design. A field of 1 keeps every
  candidate, a field of a half keeps half of them, a field of nothing keeps none — so a
  **soft edge comes out as a thinning crowd** rather than as a hard cut, which is what a
  threshold would give and what would look wrong over a feathered matte.
- **A point's place is raster-independent; its acceptance is not, and that is stated
  rather than hidden.** Density is counted against the *composition's* area, so a
  half-resolution preview throws the same candidates at the same places in composition
  pixels — changing the preview divisor never re-rolls the crowd. What it can change is
  whether a candidate sitting **on a soft edge** is admitted, because the alpha it reads is
  the picture at the raster being drawn and a half-resolution picture is a different
  picture. At full resolution preview and export are identical by construction (K-031),
  which is where that guarantee is actually made. Any fixed "working resolution" would have
  been a second resampling of the picture, a second cost and a second truth, to move the
  same wobble somewhere else.
- **Alpha, and only alpha** — which is why this effect carries no Channel row (§2.6): it
  owns the answer and answers it with a constant. A luma source becomes an alpha one
  through Matte key or Set matte, which is what those exist for. **Invert** works, and
  scatters the points *outside* the shape.
- **The rejection happens in the draw**, not in a pass of its own (K-599). The candidates
  are a set that exists only on the host and the field is a picture that exists only on the
  card, so the vertex stage is where the two can meet: what is posted is the whole candidate
  set, and a refused point is given no size — a disc of no radius covers no pixel. The
  compacted GPU stream lands with the first stack consumer that needs one, which is where
  points-stream.md §3.3 always said the family's carriage would be built.
- **Its stream cannot be sampled by a driver**, and this is the recorded answer to
  points-stream.md §2.2's constraint: the stream is a function of the input picture, and at
  resolve time — when the driver walk runs — no picture exists. A points wire from Scatter
  into a Points sample reads the documented empty stream rather than a guess at one, tested
  as such. Emit-from-image inherits the same answer.
- **Max points is a ceiling on the work, not on the look**: it bounds the *candidates*, and
  what stands is a subset of them. Mix at nought emits the stream and draws nothing, the
  emit-only mode §3.88 documents.

`moderate` cost, `FullFrame` ROI, premultiplied, temporal window `{0}`, not seeded in the
§1.3 sense (its pixels are a function of its input, which the frame key already covers).
Mix 0, Density 0 and a wholly transparent field are all the bit-exact identity.

AE's nearest equivalent is CC Pixel Polly and the various "shatter into particles" plugins,
none of which hands the points out as data; what this exists for is the wire, once Clone to
points and Connect points land.

---

### 3.90 Clone to points — a layer stamped at every point of a stream

A **Generate** effect (K-600), and the first thing in this engine that **reads** a points
wire rather than handing one out. Wire a producer's teal Points socket into it, pick a
layer, and that layer's picture is stamped once per point: at the point's place, turned by
the point's own rotation, sized by the point's own size, tinted by its colour. A hundred
snowflakes from Particulate, a lattice of thumbnails from Grid, a logo scattered inside a
silhouette — the rig people build by hand out of repeaters and expressions, as one wire.

**Parameters.** Clone layer (the picture stamped — **unset draws nothing**), Scale (per
cent, 0–1000, default 100 — multiplies each point's own size), Rotation (degrees, added to
each point's own), Tint (per cent, default 100 — how much of the point's colour and alpha
the stamp takes), **Max clones** (1–200 000, hard 1 000 000, default 2 000 — the budget
dial, not animatable). Plus the host Mix with its Blend (K-425). No panel row for the
stream: it is a **wire-only** input, declared on the signature, because a points stream has
no stored value to fall back on (impl/points-stream.md §4.1).

**Algorithm sketch.** There is nothing in it but a stamp:

```
stream = the wire's producer, evaluated at this frame in px@comp   # one evaluation
keep the newest `Max clones` by id                                 # the cap rule
size_i     = stream.size_i · Scale ; rot_i = stream.rotation_i + Rotation
colour_i   = 1 + (stream.colour_i − 1) · Tint      # towards opaque white
draw, in id order: the layer's picture in a square of size_i, turned by rot_i
```

Five notes:

- **Painter's order is `id` order**, and that is the determinism claim. A stream arrives
  ordered by birth index ascending — a fact of the evaluation rather than an artefact of how
  it was scheduled (impl/particulate.md §5) — and the stamps are laid down in that order, so
  a later point covers an earlier one on every machine and in every render (K-031).
- **It is Particulate's Sprite mode, pointed at somebody else's particles.** Not a second
  implementation: literally the same instanced quad, the same bilinear tap, the same
  premultiplied tint, through the shared points draw K-598 built. What changes is only where
  the points came from.
- **Nothing wired draws nothing**, and the box says so: an unwired Points input wears the
  `!` mark and the tooltip K-509 gave the family. So does an unset Clone layer — the
  ordinary unset-is-identity reading, deliberately unlike Particulate's Sprite mode, which
  falls back to discs because a *render mode* must always draw something. This has no mode,
  only a source.
- **Tint is a dial rather than a switch**, because a producer's colour usually carries the
  *fade* as well as the hue — Particulate's Opacity over life lives in that alpha. At 0 the
  stamp is the layer's own picture; at 100 it is that picture times the point's colour and
  alpha; between, both.
- **A wire from Scatter reads the empty stream** (K-599, K-600), which is that producer's
  recorded refusal rather than this effect's: a stream that is a function of the input
  picture cannot be answered where the stack's carriages are built. Grid and Particulate
  both feed this effect in full.

`moderate` cost, `FullFrame` ROI (a point may be anywhere, and a stamp reaches half its own
size past it), premultiplied, temporal window `{0}`, not seeded in the §1.3 sense. Mix 0, an
unset Clone layer, an unwired stream and Scale 0 are all the bit-exact identity.

AE's nearest equivalents are the Repeater on a shape layer and the third-party "clone to
particles" plugins; neither reads a particle system's own points, which is what the wire
here is for.

---

### 3.91 Trail — where every point has been, drawn without remembering it

A **Generate** effect (K-601), the second points **consumer** and the family's last named
one. Each point of a wired stream grows a tail: a line of dots, or one connected ribbon,
running back through the places it was a moment ago and fading as it goes.

**Nothing is stored, ever.** A trail is the obvious place to keep a history, and keeping one
would cost this engine everything it has — a frame that depended on the frame before it
cannot be scrubbed to, cannot be exported out of order, and cannot promise two renders agree
(K-474, K-031). So Trail does what Streak does and does it further: it evaluates the
producer's stream **again**, at `t − k·Spacing`, once per sample, and reads each point's
older self out of the answer. Frame 500's tail costs what frame 3's costs, from a cold start,
from either scrub direction.

**Parameters.** **Samples** (1–64, hard 256, default 8 — how many places back, *including*
where the point is now, so 1 is no tail at all), **Spacing** (seconds, default 0.033 — how
far apart in time those places are), Style (Dots · Segments), Scale (per cent, default 60 —
multiplies each point's own size), Feather (per cent), Fade (per cent, default 100 — how far
the far end fades away), **Max trails** (1–200 000, hard 1 000 000, default 2 000 — the
budget dial, not animatable). Plus the host Mix with its Blend (K-425). No panel row for the
stream: it is a wire-only input (impl/points-stream.md §4.1).

**Algorithm sketch.** One evaluation per sample, then a merge:

```
for k in 0..Samples:  s_k = the wire's producer, evaluated at t − k·Spacing   # px@comp
for k from the oldest sample to the newest, and by id inside each:
    where was this point then?  a forward walk of s_k by id; absent → no dab
    dim   = 1 − Fade · k/(Samples−1)
    dab at s_k.position, size · Scale, colour · dim
    Segments: run its capsule back to the same point in s_(k+1), else to itself
```

Five notes:

- **Samples is the budget as much as it is a look.** Every sample is another whole
  evaluation of the producer's stream, so this is the one row that decides what the effect
  costs — which is why it is `heavy` where its siblings are `moderate`, and why the default
  is short. The engine reads it, and Spacing, off the stored rows before the frame's
  parameters are resolved at all, because the samples have to be evaluated before there is
  anything to hand the kernel.
- **Points are matched by `id`, in a forward walk.** Both streams are ordered by birth index
  ascending — a fact of the evaluation rather than an artefact of scheduling — so "where was
  point 4 172 a moment ago?" never searches and never allocates.
- **A point with no older self has a shorter tail**, not an extrapolated one: it was not born
  yet. The tail simply stops there, which is the honest picture and is why the samples do not
  all carry the same number of dabs.
- **Painter's order is oldest sample first, `id` inside it**, so the near end of a tail lands
  on top of the far end and one point's tail lands on the next point's in a fixed order
  (K-031).
- **Dots and Segments are one kernel**: a capsule whose tail is its head is a disc, the same
  identity the three Particulate render modes rest on. Segments fills each dab's tail with
  the previous sample's place; Dots leaves it at the head. **Fade is over the tail, not over
  the frame** — a dab takes its own point's colour at its own sample, so a producer's Opacity
  over life still reads correctly along it, dimmed by how far back the sample is.

`heavy` cost, `FullFrame` ROI, premultiplied, temporal window `{0}` (it reads the producer at
other times through its own evaluation, never the *picture* at other times), not seeded in
the §1.3 sense. Mix 0, an unwired stream, Scale 0 and Samples 1 with Mix 0 are the bit-exact
identity; an unwired stream also wears the `!` mark K-509 gave the family. A wire from
Scatter reads the empty stream, for K-599's recorded reason.

AE's nearest equivalent is Echo, which repeats whole *frames* and therefore does keep them;
this repeats a point's own arithmetic instead, which is why it costs nothing to scrub.

---

### 3.92 Connect points — lines between the points that are near each other

A **Generate** effect (K-602), the third points **consumer**. Every pair of points in a
wired stream closer together than **Max distance** is joined by a line: particles drifting
past each other web up and let go again, a Grid becomes a mesh, a Scatter inside a
silhouette becomes a constellation. The plexus look, as one wire.

**A line is a capsule, and a capsule is a disc that has been stretched.** Nothing new is
drawn: the shared points draw already runs a dab from a head to a tail (K-601), so a segment
is one entry in an ordinary stream whose tail is somewhere other than its head. Four effects,
one rasteriser.

**Parameters.** **Max distance** (px@comp, default 120 — how far apart two points may be and
still be joined; nought joins nothing), **Max connections** (0–32, hard 64, default 4 — the
most lines that may meet at any one point, counted at **both** ends), Taper (per cent,
default 0 — how much a longer line thins), **Fade** (per cent, default 100 — how much a
longer line dims, so the web comes and goes instead of switching on), Width (px@comp,
default 2), Feather (per cent), Colour (multiplies the colour a line inherits), **Max
points** (1–200 000, hard 1 000 000, default 2 000 — the budget dial, not animatable). Plus
the host Mix with its Blend (K-425). No panel row for the stream: it is a wire-only input
(impl/points-stream.md §4.1).

**Algorithm sketch.** Cut the plane, then walk it once:

```
points = the wire's stream, newest Max points by birth index      # px@comp
cells  = the projected plane in squares of one Max distance, indices in id order
for i in id order, while point i has room:
    near = every j > i in the nine squares around i, within Max distance
    order near by distance, id breaking every tie
    for each (d, j) in near, while both ends have room:
        u    = d / Max distance
        line from point i to point j, width · (1 − Taper·u), colour · (1 − Fade·u)
```

Four notes:

- **The buckets are what keep this off the `n²` path.** Asking every point about every other
  is `n²/2` questions — a hundred million a frame at twenty thousand points. Two points more
  than one square apart cannot be within reach, so the nine squares around a point are the
  whole of what it has to ask, and the walk is `O(n·k)` for `k` the crowd in a neighbourhood.
  The remaining ceiling is a *clump*: a whole stream inside one square is `O(m²)` again,
  bounded by Max points and by Max connections cutting the inner walk short. Marked in the
  code with its upgrade trigger, and pinned by a test that holds the bucketed answer against
  a full comparison at five reaches.
- **Max connections is counted at both ends.** A pair is joined only while *both* points
  still have room, so the dial means what it says everywhere rather than only at the point
  being walked — and the total is bounded at `n · Max connections / 2` lines.
- **Determinism is the walk's order, twice over** (K-031): points in `id` order — a fact of
  the evaluation rather than of scheduling — and each point's candidates ordered by distance
  with `id` breaking every tie. Painter's order is the order the lines were found in.
- **A line inherits the mean of its two ends' own colours**, so a producer's Colour over life
  still reads along the web; the Colour row multiplies that, and is white by default.

`moderate` cost, `FullFrame` ROI, premultiplied, temporal window `{0}`, not seeded in the
§1.3 sense. Mix 0, an unwired stream, Max distance 0, Max connections 0 and Width 0 are the
bit-exact identity; an unwired stream also wears the `!` mark K-509 gave the family. A wire
from Scatter reads the empty stream, for K-599's recorded reason.

AE has no equivalent; the look is bought as a plugin there.

---

## 4. Tier 2 — AE parity direction (post-v1)

One-line scope each; specs written when scheduled ([16-ROADMAP.md](16-ROADMAP.md)). Order
roughly by demand.

| Effect | Scope |
|---|---|
| ~~Levels / curves per channel~~ | **Shipped** as §3.31 and §3.30. Curves is master + R/G/B + alpha on real control points (K-412); Levels is master + R/G/B, and its alpha channel is still Tier 2 |
| ~~Hue/saturation~~ | **Shipped** as §3.33 (master + six ranges). Colourise is still Tier 2 |
| Tritone / tint | Map shadows/mids/highlights to three colours |
| Keying | Luma key + colour key + a basic screen key (core matte generation, not Keylight parity at first) |
| Matte choker | Grow/shrink/soften mattes; companion to keying |
| ~~Fractal noise~~ | **Shipped** as §3.37, in the new Generate category (K-398). Sub rotation, Sub offset and AE's Overflow modes are still Tier 2 |
| ~~Gradient ramp~~ | **Shipped** as §3.35 **Gradient** (linear and radial, with scatter) |
| ~~Noise~~ | **Shipped** as §3.36 (uniform/gaussian, mono/colour, animated or frozen) |
| ~~Drop shadow~~ | **Shipped** as §3.43. Per-mask targeting is still Tier 2 |
| Bevel | Simple edge bevel (alpha and border variants) |
| Mosaic | Block-average pixelation |
| Find edges | Gradient-magnitude edge extraction |
| Posterise | Value quantisation (plus posterise-time as a separate temporal utility) |
| Turbulent displace | Noise-driven UV displacement |
| Wave warp | Parametric sinusoidal displacement |
| ~~Corner pin~~ | **Shipped** as §3.48, and it is the Planar track's export target (§3.87, K-579) |
| Mesh warp | Grid-based freeform warp |
| Stabiliser | Flow-engine-backed smoothing of unwanted camera motion (warp-stabiliser class) |
| Tracker | ~~Planar~~ tracking **shipped** as §3.87 **Planar track** (K-579): a quad followed as four corners, with Create corner pin writing them onto another layer. Still Tier 2: **point** tracking producing a keyframed *transform* — one or two points baked into position, rotation and scale on a layer — and the on-canvas quad handles the panel rows stand in for |

Tier 2 effects follow every rule in §1–2; nothing in Tier 1's architecture may assume the
suite stays small.

---

## 5. Presets

- **Per-effect presets**: a named parameter snapshot (keyframes and expressions included
  when marked "animated preset"). **Per-stack presets**: an ordered list of effect
  instances with their parameters — the unit the scene calls an editing/CC pack.
- Serialised as a single shareable file (`.lumfx`), a machine-independent JSON payload per
  K-065. (v1 writes plain JSON; bundling embedded assets such as LUTs into a zipped pack is a
  later extension — see §3.11.) Import by drag onto a layer, the Effect Controls panel, or the
  preset browser.
- Lumit ships a first-party library (grade presets §3.10, shake styles, zoom eases, glitch
  looks). Ship-with presets are data files, not code, and use only built-in effects.
- **Community packs**: preset import MUST tolerate unknown effects (imported as inert
  placeholders with their parameters preserved, mirroring
  [11-AE-IMPORT.md](11-AE-IMPORT.md) placeholder policy) so packs survive version skew.
  Post-v1 ambition: an `.ffx`/AE-preset converter for the existing pack ecosystem, tracked
  in [11-AE-IMPORT.md](11-AE-IMPORT.md); the montage scene onboards through shared packs,
  so this converter is growth infrastructure, not a courtesy.

---

## The universal strength matte (K-035) — delivered as §2.6

K-035's ambition — every effect drivable per pixel, implemented once by the host rather
than thirty-odd times by effect authors — is what **§2.6** now specifies and what shipped
under K-395. That section is the normative text; this one is kept only to say where the
idea went, because K-035 is cited from elsewhere.

Two details of the original wording were settled differently and §2.6 is the one to
believe. The source is **a layer, not a layer-or-mask-set**: a mask set is reachable by
pointing the row at the layer that carries it, so the second source bought nothing (the
row is the layer picker of §1.1, so it is animatable and expression-visible and hashes
like any input, as K-035 required). And there is **no gain control** — Invert alone, with
the layer's own effects available for anything shapelier.

K-035's warp clause — a displacement-class effect scaling its *vectors* by the matte
rather than dissolving its output — is exactly the override mechanism of §2.6 and is
listed there as one the displacement effects have not claimed yet
([TODO.md](TODO.md)).

## Open questions

1. **Flow algorithm choice — resolved (K-169).** v1 ships **dense inverse search** (DIS,
   Kroeger et al. 2016), pinned in [docs/impl/optical-flow.md](impl/optical-flow.md) and
   implemented in `lumit-flow`. A learned model (RAFT-class) beats it on quality but
   complicates the GPLv3 story, model distribution size, and the CPU reference oracle, so it
   stays an optional future producer behind the same API (dense vectors + occlusion +
   confidence), which is stable either way.
2. **Gamma stage in Colour balance.** Applying gamma on a display-referred intermediate feels
   familiar but is impure; a strictly scene-linear grade with a viewing-transform-aware UI
   is cleaner. Needs a side-by-side with real CC packs before locking.
3. **Where Shake lives.** Specced as an effect that resamples the layer; an alternative is
   a transform modifier that concatenates into the layer matrix (better quality, free
   engine motion blur, but a new concept in the data model). Decide with
   [03-DATA-MODEL.md](03-DATA-MODEL.md).
4. **Preset licensing.** Ship-with preset library licence (GPLv3 data? CC0?) affects
   whether community packs can embed ours. CC0 recommended; needs the owner's sign-off.
5. **fp16 oracle tolerances.** The per-cost-class tolerance defaults in §1.6 are
   placeholders until the first three effects are implemented on both NVIDIA and AMD and
   real cross-vendor deltas are measured.
6. **~~Should the three-colour picker drive Wavelength mode too?~~ RESOLVED (A1/K-163).** The
   owner chose to replace the physical `SPECTRAL_BASIS` with a smooth colour1→colour2→colour3
   gradient across the offset span, so the picker fully drives the Wavelength fringe (default
   red/green/blue → a red→green→blue dispersion). Implemented in `spectral_taps`; the old
   physical basis is retired. Both effects' Wavelength modes are now picker-driven.
7. **~~Full blend-mode parity for Echo's Mode?~~ RESOLVED (T21).** The owner confirmed it is fine
   to exclude the HSL / colour burn/dodge / contrast-group modes from Echo — they are ill-defined
   on a premultiplied linear light trail. Echo keeps its curated set (the two compositing orders
   plus the order-independent light-combine blends); the shared `BlendMode` catalogue drives the
   layer dropdown.

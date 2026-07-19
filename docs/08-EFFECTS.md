# Built-in effects

**Status: implementation-ready.** Specifies the effect model and the built-in effect suite
(K-064, K-019). Terminology per [01-GLOSSARY.md](01-GLOSSARY.md); render semantics per
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md); plugin-hosted effects (OFX, KFX) per
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
   space), curve (bezier LUT), seed (integer), file reference, and marker-trigger (§1.4).
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

### 1.2 Parameter conventions

- **Names** are sentence case in the UI, stable snake_case identifiers in the schema.
- **Ranges** declare a slider range and a hard range; sliders MAY be exceeded by typing,
  hard ranges MUST NOT be. Hard ranges MAY be one-sided (K-090): a threshold clamps at
  zero below and is unbounded above where that is the honest shape of the parameter.
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
  **another layer** in the same composition as an auxiliary picture an effect samples — a
  depth pass for Depth of field (§3.22). The stored value is an optional layer id (the shape
  a matte reference uses, §5.1 of [03-DATA-MODEL.md](03-DATA-MODEL.md)), static in v1. The
  host renders that layer alone and threads its texture to the effect, exactly as a matte
  layer is rendered alone. An **unset** or **dangling** reference resolves to identity — the
  same sanctioned exception to the "no no-op default" rule, since a layer the user must
  supply cannot have a tasteful default. Beside the picker sits a **source** combobox
  (K-142, revising K-125's before/after bool) choosing *what of* the referenced layer is
  read: **None** (its raw footage/solid — no masks, no effects), **Masks** (its source plus
  its masks) or **Effects and masks** (its finished picture — a graded or blurred input).
  The same three-way source applies to a track matte (§5.1 of
  [03-DATA-MODEL.md](03-DATA-MODEL.md)). Temporal effects on the referenced layer (echo,
  flow motion blur) are still not sub-sampled through the input in v1 — the spatial and
  colour stack applies, an echo/flow degrades to a still (the K-125 boundary).

### 1.3 Traits

Every effect declares, statically:

| Trait | Values | Consumed by |
|---|---|---|
| **Cost class** | `trivial` (pointwise), `cheap` (small fixed kernel), `moderate` (large-radius / multi-pass), `heavy` (iterative or flow-based) | Adaptive degradation ordering, background render budgeting |
| **ROI support** | `exact` (output pixel needs only the same input pixel), `padded(r)` (needs input dilated by radius r, in the effect's declared units), `full-frame` (needs the whole input) | Region-of-interest rendering, tiling |
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

All effects operate on **scene-linear, premultiplied-alpha fp16** buffers (fp32 when the
comp opts in, K-026). Effects MUST NOT assume display-referred input: values above 1.0 are
legal and meaningful (glow depends on them). Effects MUST NOT clip highlights except where
clipping is the documented behaviour of a parameter.

### 2.2 Unpremultiplied exceptions

Colour-manipulation effects operate on unpremultiplied colour, because grading
premultiplied values shifts matte edges. Effects declaring `alpha mode: unpremultiplied`
are wrapped by the host: unpremultiply → effect → re-premultiply, fused into the effect's
first/last passes where possible. The Tier 1 effects requiring this: **the colour effects
(Colour balance, Saturation, Contrast, Gamma), LUT, Sharpen, Matte key** (edge haloes
otherwise). Contrast and Gamma join the list because Contrast's `− pivot` offset makes it
*affine* and Gamma's power curve is *non-linear* — neither is a pure scale, so unlike Exposure
and Hue shift they do not commute with premultiplied alpha (§3.18, §3.19). Matte key joins it
because its colour-difference matte and despill read straight colour: keying the premultiplied
values would judge (and fringe) the edge pixels by their coverage rather than their true colour
(§3.21). All others consume premultiplied input directly (Block glitch, Scanlines and Datamosh
among them — §3.12).

### 2.3 Resolution-independent units

Parameters MUST be expressed in units that survive comp resizing and preview resolution:

- **% diag** — percentage of the comp diagonal. Default for radii, distances, displacement.
- **degrees** — all angles.
- **px@comp** — physical pixels at full comp resolution, for deliberately pixel-scale
  looks (scanlines, block sizes). The engine scales these by the preview resolution factor
  so Half preview matches Full preview framing.
- **seconds** or **frames** — durations; frames are comp-frame-rate frames.

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

---

## 3. Tier 1 — the montage staples (v1)

The in-box replacements for the scene's paid stack. Two shape rules (K-090): an effect
does **one thing** (multi-purpose designs split; an all-in-one grading suite may exist
later as a deliberate exception), and every schema declares a **category** — Blur &
sharpen, Colour, Distortion, Stylise, Temporal, Utility — which is how the Add-effect
menu groups. The flow engine is **not** in this list: it is a per-layer option (K-088),
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
| 3.12 | Datamosh | Universe / glitch packs | cheap | `{-1, 0}` |
| 3.13 | Echo | stock Echo / speed-lines packs | moderate | `{-n..0}` |
| 3.14 | Vignette | stock CC pack vignette | cheap | `{0}` |
| 3.15 | Chromatic aberration | stock CC pack fillers | cheap | `{0}` |
| 3.16 | Exposure | stock CC pack exposure/levels | cheap | `{0}` |
| 3.17 | Hue shift | stock CC pack hue/saturation | cheap | `{0}` |
| 3.18 | Contrast | stock CC pack contrast/levels | cheap | `{0}` |
| 3.19 | Gamma | stock CC pack gamma/levels | cheap | `{0}` |
| 3.20 | Temperature | stock CC pack white-balance | cheap | `{0}` |
| 3.21 | Matte key | Keylight-style colour-difference keyer | cheap | `{0}` |
| 3.22 | Depth of field | Frischluft / Camera Lens Blur | moderate | `{0}` |
| 3.23 | Invert | stock CC pack invert | cheap | `{0}` |
| 3.24 | Tint | AE Tint / duotone | cheap | `{0}` |
| 3.25 | Posterize time | AE Posterize Time | cheap | `{0}` |
| 3.26 | Motion blur (accumulation) | RSMB / ReelSmart (accumulation) | heavy | `{0}` |

### 3.1 Flow engine — optical-flow retime interpolation (Twixtor-class)

**K-088: not an effect.** Everything below stands as the engine specification, but flow is
surfaced as a **layer option**: a toggle in the footage layer's switch cluster, a **Flow**
group beside Transform and Effects carrying these parameters, engaging only when the
footage's rate (through any retime) undershoots the composition's — when a source frame
would otherwise hold across two or more comp frames.

**Input rate (conform, K-095).** The Flow group carries an **Input rate** control: the fps
the clip is *interpreted* at for flow. Native (the default) interpolates between adjacent
source frames; a rate below native conforms the clip to that rate, so flow brackets the
source frames spaced `1/rate` apart and interpolates between those — the standard way to get
real slow-motion out of high-framerate footage (whose adjacent frames barely move). It keys
the frame cache (the same source time synthesises from different frames under it) and applies
identically in preview and export.

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
2. Coarse-to-fine variational/patch-match hybrid flow: initialise each level from the
   upsampled coarser level, refine with local patch search + smoothness regularisation.
   Compute A→B and B→A fields.
3. **Occlusion detection** by forward-backward consistency: where `flow_AB` followed by
   `flow_BA` fails to return within a threshold, the pixel is occluded in one frame.
4. Synthesis at fraction t: splat/warp A forward by `t·flow_AB` and B backward by
   `(1−t)·flow_BA`; blend `(1−t)/t` weighted; in occluded regions take only the frame in
   which the pixel is visible; inpaint the (rare) both-occluded holes from neighbours.
5. HUD/overlay guard: static-region detection (near-zero flow with high texture) biases
   those pixels toward pure blending, reducing the classic Twixtor HUD smearing.

**Parameters** (surfaced per clip / per layer as render-policy options, not a stack entry):

| Parameter | Range / type | Default | Notes |
|---|---|---|---|
| Vector detail | Low / Medium / High / Ultra | Medium | Pyramid depth + refinement iterations |
| Smoothness | 0–100 | 50 | Regularisation weight; high = fewer tears, gloopier |
| Occlusion handling | Blend / Visible-only | Visible-only | Blend trades ghosting for fewer holes |
| Fallback | enum | Blend | Behaviour where confidence is low: **blend** (crossfade) or **nearest** |

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
| Quality | Draft / Normal / High | Normal |

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
motion boundaries; gating the blur on/off there leaves hard un-blurred cut regions. Instead each
pixel's streak length is **scaled smoothly by its confidence** — `lumit_flow::confidence`, a
0..1 forward–backward-consistency measure (1 where the two flows agree, tapering to 0 where they
disagree, an invalid patch fully suspect), 3×3 box-blurred so the falloff has no seam. So
suspect regions fade toward unblurred gradually rather than cutting. Confidence 0 is a bit-exact
passthrough for that pixel (the streak collapses onto it), so it composes with the zero-motion
and zero-shutter passthroughs.

The v1 parameter set
is trimmed to **Shutter angle** (0–720°, default 180 — streak length is shutter ÷ 360 of the
inter-frame motion, so 180° is half of it, the film-standard look), **Samples** (a fixed
per-frame tap count, slider 8–32, so the CPU and GPU integrate identically), a **View** enum
(*Rendered* | *Motion vectors* | *Confidence*, default Rendered — the diagnostic views output
the flow colour-coded, or the confidence as greyscale, so a user can see what the smear follows
and where it fades) and the host **Mix**. Blur length in pixels = motion vector × (shutter ÷ 360)
× confidence; the streak is a centred
box integral of `Samples` evenly spaced bilinear taps, edges clamped so a full-frame smear
never darkens the border. Pinned simplifications, each stable when the rest of §3.2 lands:
**Vector source is Flow only** (Auto's transform-derivative path and the engine-motion-blur
double-blur guard follow), **Amount** (the post-shutter vector scale) and the Quality /
adaptive-per-pixel tap count are deferred (Samples is the one fixed count), and it blurs
**footage layers only** — adjustment-layer and Sequence-clip temporal effects are deferred
exactly as they are for Echo. Zero motion or a zero shutter is a bit-exact passthrough
(pinned by test).

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
noise, 2–4 octaves) drive x, y (as % diag) and rotation (degrees), sampled at
`time · frequency`. A style preset sets octave count, lacunarity, and a per-axis frequency
multiplier. **Trigger mode** gates the noise with an envelope: on each trigger (beat marker
via §1.4, or manual keyframe on the Trigger parameter) the envelope jumps to 1 and decays
exponentially over Decay seconds, so shakes hit on the beat and settle.

**Parameters.**

| Parameter | Range / type | Default |
|---|---|---|
| Style | Subtle / Normal / Twitchy / Jumpy | Normal |
| Amplitude | 0–20 % diag | 1.5 % diag |
| Frequency | 0.1–30 Hz | 8 Hz |
| Rotation amount | 0–45° | 1° |
| *Per-axis wobble* (twirl) | | |
| — X amount / X frequency | ×0–2 / ×0–4 | ×1 / ×1 |
| — Y amount / Y frequency | ×0–2 / ×0–4 | ×1 / ×1 |
| — Z amount / Z frequency | 0–20% / ×0–4 | 0 / ×1 |
| Edges | Transparent / Repeat / Mirror | Repeat |
| Mode | Continuous / Triggered | Continuous |
| Trigger source | marker-trigger | comp beat markers |
| Decay | 0.05–2 s | 0.35 s |
| Motion blur shake | boolean | on |
| Seed | seed | per-instance |

The master Amplitude and Frequency drive the overall translational sway; the **Per-axis
wobble** twirl (K-146) biases each axis and adds depth. X and Y amount/frequency are
dimensionless multipliers on the master values (×1 reproduces the plain uniform shake); Z
is the depth/scale shake — Z amount is a scale-pump per cent (the old Zoom pump, same
range), Z frequency a rate multiplier. **Edges** (K-145, the reusable control) governs the
border the resample reveals: Transparent leaves it clear, Repeat holds the edge pixel,
Mirror reflects. "Motion blur shake" samples the wobble at shutter sub-times so fast shakes
streak naturally (the S_Shake feature wiggle expressions never had).

**Status (v1, continuous form, shipped):** Amplitude, Frequency, Rotation amount, the
Per-axis wobble twirl (X/Y/Z amount and frequency), an Edges control (Transparent / Repeat
/ Mirror, default Repeat) and Seed (per-instance default, with reseed). The generator is
pinned as two octaves of seeded value noise (lacunarity 2, gain 0.5, smoothstep-
interpolated, one independent channel per axis) sampled at local time × frequency —
deterministic and hop-free per §2.4. Resolved host-side into an affine and dispatched
through the §3.5 Transform kernel (which now carries the Edges policy): no kernel of its
own, and the zero-wobble state is a bit-exact passthrough (pinned by test). **Migration
(FX-11/K-146):** this reshape replaced the old Zoom pump and Auto-scale bool — a project
saved before it maps its Zoom pump to the Z amount, and its Auto-scale to the Edges control
(on → Repeat, which hides the border as the cover scale once did; off → Transparent). The
Auto-scale cover (which zoomed in to keep every corner covered) is gone; the Edges control
handles the revealed border instead. Style presets, Triggered mode (§1.4) and Motion blur
shake follow; shipped parameters are stable when they do.

### 3.5 Transform — the transform properties as an effect (K-090)

Position, Anchor, Scale, Rotation, Opacity — the layer transform group, as a stack entry.
Its point is adjustment layers: applied there, it transforms the composite of everything
below, which is the montage punch-in/whip gesture without touching per-layer transforms.
Parameters mirror the transform group exactly (same names, units, animatability); an
additional Skew pair arrives post-v1. Cost `trivial`, ROI `exact` under pure translation
and `full-frame` otherwise, `{0}` temporal.

### 3.6 RGB split — chromatic aberration

**Quality (K-090):** a `Wavelength` Bool (default off) switches from the three-channel
split to a wavelength-weighted dispersion (more samples across the visible spectrum,
recombined in linear) for the higher-quality look; parameters are shared between modes.

**Parameters:** Amount (0–10 % diag, default 0.4), Mode (Linear / Radial), Angle (degrees,
linear mode), Centre (radial mode), Falloff (radial: 0–4, aberration grows toward edges),
Blur split channels (0–100%).

**Algorithm sketch.** Sample R and B at offset positions (G stays put): linear mode
offsets along the angle; radial mode offsets along the vector from centre, scaled by
distance^falloff. Operates premultiplied; alpha follows the green channel to avoid fringed
mattes. Trivially animatable Amount is the scene's impact-frame staple.

**Per-channel amounts (FX-9, K-143):** three per-cent scales — **Red**, **Green**, **Blue**
(defaults 100 / 0 / 100, open both sides, K-135) — multiply the overall Amount per channel,
so R and B can fringe by different amounts and G can be nudged off its anchor. R and G
displace along −offset, B along +offset, so the 100 / 0 / 100 defaults reproduce the classic
split bit-for-bit. They apply to the classic (non-Wavelength) mode only.

**Wavelength samples (FX-9, K-144):** the Wavelength mode carries a **Samples** control (the
tap count, `3..=64`, default 16). More taps fill the same `±offset` span more densely, so a
large offset disperses as a smooth rainbow instead of a few discrete stacked copies. The taps
(each a column-normalised RGB weight plus its offset fraction) are resampled from the nine
`SPECTRAL_BASIS` anchors host-side and shared by the CPU reference and the WGSL kernel, so a
uniform image still passes through unchanged and preview equals export (K-031).

**§3.15 Chromatic aberration** is a separate, single-purpose sibling shipped alongside this
effect: same R-outward/B-inward radial shape as this effect's own Radial mode. It adds the
reusable three-colour channel picker and this effect's own Wavelength/Samples dispersion
(K-144) — see §3.15.

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

- **Gaussian blur** (match_name `blur`): Radius (% diag, default 1.5, slider 0–25, hard
  0–100). Separable two-pass; large radii switch to mip-assisted sampling. ROI
  `padded(radius)`. **Keeps match_name `blur`, so a project saved with the old combined effect
  loads here as Gaussian at its stored Radius, byte-identically** — whatever mode it had saved,
  its now-unread mode/length/centre parameters are simply ignored.
- **Directional blur** (match_name `directional_blur`): Length (% diag, default 10, slider
  0–200, **hard-unbounded above** per K-090) and Angle. Line-integral sampling along the
  angle. Length may now exceed 100 % of the diagonal, since it is its own effect rather than
  sharing the family's reach; the tap count still clamps (`cpu::dir_blur_taps`), so a long
  streak stays bounded in cost. ROI `full-frame` (an unbounded Length cannot be padded
  statically).
- **Radial blur** (match_name `radial_blur`): Centre X / Centre Y (% of comp width/height,
  50/50 default — the schema has no Point-shaped `ParamKind`, so this follows Transform's own
  `anchor_x`/`anchor_y` split), Amount (% diag, default 8, slider 0–100, hard-unbounded above),
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

- **Unsharp mask** (match_name `sharpen`): Amount (0–300 %), Radius (0.05–2 % diag), Threshold
  (0–1, suppresses noise amplification), and a luminance-only option (avoids chroma fringing on
  compressed game capture). Algorithm: `input + amount · (input − gaussian(input, radius))`
  gated by threshold — a radius-controlled detail lift.
- **Sharpen** (match_name `sharpen_simple`, K-138): the plain, radius-free sibling — a fixed
  3×3 high-pass convolution scaled by **Amount** (default 1 = the classic 5/−1 kernel, slider
  0–5, hard-clamped ≥ 0). `out = u + amount · (4·u − up − down − left − right)` per RGB channel,
  with the four axis neighbours clamp-addressed (so a border never invents dark detail); the
  result clamps ≥ 0, re-premultiplies by the centre alpha, and keeps alpha. Amount 0 (whatever
  the Mix) and Mix 0 are the bit-exact passthrough. Cheap, one pixel of reach; the honest "just
  sharpen it" control beside the Unsharp mask's knobs.

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
(stops; Temperature via Bradford-adapted CCT shift; Tint) and **curves** (master + R/G/B
bezier, evaluated as 1D LUTs baked per frame when animated).

**Preset browser.** Colour presets get a dedicated browser (per
[07-UI-SPEC.md](07-UI-SPEC.md)): a panel of live thumbnails, each preset applied to the
frame under the playhead, Magic Bullet Looks-style. Thumbnails are rendered by the normal
engine at thumbnail resolution through the real effect — never approximations. Ships with
≥ 40 presets across the genre families (clean/bright, teal-orange, moody desat, anime
vibrance, VHS warm). Selecting a preset sets parameters; it never locks editing.

### 3.11 LUT — .cube loader

**Parameters:** File (file reference, `.cube` 1D and 3D, sizes to 65³), Input space
(sRGB / Rec.709 / Linear — what the LUT expects), Interpolation (Trilinear /
Tetrahedral, default Tetrahedral), Mix.

**Algorithm sketch.** Host parses and uploads the LUT as a 3D texture at load, converts
working-space linear into the LUT's expected space, applies, converts back. Unpremultiplied.
Missing file behaviour: effect becomes a labelled no-op with a warning badge — never a
render failure ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) never-crash rule). The
file's content hash joins the cache key; project save embeds small LUTs (K-040) so shared
projects survive relinking.

**Status (v1, shipped, K-114):** **File + Mix** only. The File parameter picks a `.cube`
cube (animatable by stepping between paths with hold keys — two files cannot be blended,
K-111) and Mix blends the graded result over the input. **3D trilinear** only (the manual
eight-corner interpolation of [docs/impl/lut.md](impl/lut.md) §2–3, matching the CPU oracle
`lut::Lut3d::sample` to ≤ 2 fp16 ULP; Tetrahedral is deferred). The LUT is applied in the
compositor's **scene-linear working space as-is** — no Input-space transfer, so a `.cube`
authored for a display- or log-encoded input is applied directly (flagged for the owner).
Unpremultiplied (§2.2). An **unset, missing, 1D or unreadable** file is a labelled no-op,
never a fault. GPU-only: the parsed cube is threaded beside the resolved op (like Echo's
neighbour frames and Motion blur's flow field), so the CPU-degradation rung renders a LUT as
identity — its §1.6 oracle reference is `lut::Lut3d::sample` used directly in the lumit-gpu
test, the one effect whose reference lives outside `cpu::apply` (its parameter is a file, not
a number). Preview and export load and apply it identically (K-031). **Follow-ups:** the
Input-space control, Tetrahedral interpolation, the content-hash cache key (the cache is
path-only for now, so an edited-on-disk LUT needs the app reopened), and embedding small LUTs
in the project (K-040).

### 3.12 Glitch family — block glitch, scanlines, datamosh

Three separate effects, formerly shipped as one "Glitch" effect with enableable sections
(K-104). **Status (K-107):** split into one-thing effects per the §1's one-effect-one-job
rule (K-090 — the same rule that split the v1 Grade into Colour balance and Saturation, and
split Chromatic aberration off RGB split's own Radial mode). Stacking **Block glitch** →
**Scanlines**, each at Mix 100%, reproduces the old combined Glitch's look bit-for-bit — the
two sections never interacted beyond running in the same pass. Existing saved `glitch`
instances do not migrate (pre-v1, single user, no alias); each of the three below is added
to a layer independently going forward. Category **Distortion** for all three, matching
Shake and RGB split — their closest siblings (a seeded positional wobble; a channel split)
— not the additive-light Stylise pair (Glow, Flash).

#### Block glitch

**Parameters:** Intensity (0–1, default 0.35, the master dial), Seed, Block size (px@comp,
default 24), Rows/columns jitter (% of Block size, default 25), Displacement (% diag,
default 3), Channel offset (% diag, default 1), Slice repeat (%, default 20), Mix.

**Algorithm sketch.** The image is partitioned into a seeded grid (Block size, px@comp);
per *nominal* block, a hash decides a jitter offset (Rows/columns jitter, scaled by
Intensity) that picks *which* block's content a pixel actually reads from — a cheap
stand-in for moving grid lines themselves, which would need a boundary search a single
pointwise pass cannot do. That block then hashes its own displacement (Displacement, %
diag), R/B channel split (Channel offset, % diag, alpha follows green exactly like RGB
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

**Parameters:** Intensity (default 0.5, open above per K-135), Streak length (frames,
default 4, hard min 1, open above per K-135), Mix.

**Algorithm sketch.** Simulates I-frame removal by re-warping the previous source frame
with the flow field measured from the current frame to it, instead of showing the current
frame — blended by Intensity × Mix. It is a *look*, not real bitstream corruption —
deterministic and safe. Reuses the §3.2 flow machinery Motion blur introduced (`flow_pair`
on the shared `FlowEngine`) rather than needing new plumbing. A single bilinear tap per
pixel reads the -1 neighbour at the position its own flow vector displaces to — a
motion-compensated prediction, not Motion blur's multi-tap streak integral. **Streak length**
(FX-14, K-148) scales that flow displacement, so the single warp reaches that many frames of
predicted motion — the accumulated smear of a long P-frame run before a clean reference frame
(longer = more smearing), directly addressing the effect being too subtle at one frame's
reach. The clean "reset" is content-driven: where the flow is zero or unmeasurable (a still,
a cut) the warp lands on the pixel itself, exactly where a real codec inserts an I-frame; a
fixed-interval I-frame reset would need the comp frame index threaded into resolve and is a
later refinement. **Intensity's hard ceiling is open** (K-135): above 1 the blend
extrapolates past the moshed frame for a punchier tear (`mix()` does not clamp in either the
CPU or GPU path); 0 stays the bit-exact passthrough regardless of Streak length. Only the
flow's `.xy` is read (the shared field's `.z` confidence lane is left untouched).
Footage-only: with no -1 neighbour or flow field (a non-footage layer, or a dropped decode)
it degrades to a no-op, never a fault. Temporal window `{-1, 0}` — statically, unlike its
K-104-era shape as
a toggle inside the combined Glitch effect (see Status below). A layer can carry only one
flow field per frame in v1; if a stack somehow has both a live Motion blur and a live
Datamosh, whichever comes first in stack order wins the single slot and the other's
flow-dependent behaviour degrades to its own missing-field passthrough — never a fault,
pinned by test. `cheap` cost (one bilinear tap), `full-frame` ROI (the flow can point
anywhere in the frame, the same unbounded-read reasoning Motion blur's own ROI carries).
Not seeded (`seeded: false`) — no hash or random-looking sequence, just flow-directed
sampling.

**Status (K-104, its own effect since K-107):** originally shipped as a toggle
(`datamosh_enabled`, off by default) inside the combined Glitch effect — the one place
`stack_temporal_window`/`stack_flow_neighbour` read a param value instead of a schema's
static `temporal` trait, because the section was footage-only and opted in rather than
silently changing every existing Glitch instance's output the moment it landed. As its own
effect that per-instance toggle is gone: `temporal: {-1, 0}` is simply the schema's own
static declaration, exactly the shape Motion blur's own `{0, +1}` already has, and
`stack_flow_neighbour` reads the match name the same static way it reads Motion blur's.

### 3.13 Echo — frame echo and trails (speed lines)

**Parameters:** Echo count (1–32), Spacing (frames, may be negative to echo forward),
Decay (per-echo opacity multiplier 0–1), Blend (Behind / Add / Screen / Front), Transform
per echo (optional scale/rotation/offset step for stylised speed-line fans).

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
later refinement) and **intensity `Decay^k`** per echo `k`. **Blend modes** now mirror the
comp set — **Normal, Add, Multiply, Screen, Overlay, Soft light, Hard light, Lighten (= the
legacy Max), Darken** — plus the echo-specific **Behind** (ghosting); the **default is
Screen**. Each mode folds the weighted echo tap into the running trail per channel in the
**working linear premultiplied space** (not the compositor's perceptual sRGB domain — Echo
composites light trails, so it stays linear, which also keeps the CPU oracle and WGSL kernel
bit-for-bit identical). The legacy mode indices 0/1/2 (Add/Behind/Max) are held so a project
saved before FX-17 loads unchanged; the new modes are appended. It reads the layer's
**source** frames, not the upstream stack's output at those times (full temporal stacking is
later), and echoes footage layers only — Sequence-clip and adjustment-layer temporal effects
are deferred. Marker-triggerable intensity spikes come with the §1.4 wiring already in place.

### 3.14 Vignette

**Parameters:** Amount (0–1, default 0.5), Radius (0–1, default 0.75), Softness (hard min 0,
unbounded above — slider 0–2, default 0.5), Roundness (0–1, default 1.0), Mix.

**Algorithm sketch.** Darkens toward black away from the frame centre: a normalised distance
metric (blended by Roundness between a true circle and an ellipse matching the frame's
aspect) feeds a smoothstep between Radius and Radius + Softness, scaled by Amount and
multiplied into the premultiplied colour; alpha is untouched. `cheap` cost, `exact` ROI — a
pointwise per-pixel darken, no neighbour sampling despite the spatial falloff.

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
Premultiplied throughout, edges clamp. `cheap` cost, `full-frame` ROI.

**Channel picker (P2, K-143):** the three tap colours are edited through the **reusable
three-colour channel picker** — three colour swatches (defaults red / green / blue), each
opening the colour picker. The widget is shared: any effect whose schema declares three
Colour parameters `channel_colour_1/2/3` gets it automatically (see `channel_picker` in the
inspector), so a future three-tinted-channel effect adopts it without new UI code.

**Wavelength (K-144):** a `Wavelength` Bool (default off) reuses §3.6 RGB split's own spectral
machinery — turning on resolves the effect to a radial spectral split with a **Samples**
control (3–64, default 16), the same many-tap dispersion RGB split's Wavelength mode uses, for
a smooth rainbow fringe rather than the three tinted taps. The channel colours apply to the
non-Wavelength mode only.

**Status (v1, shipped):** a dedicated, always-radial sibling of §3.6 RGB split's own Radial
mode, not a replacement for it — RGB split's Radial mode already covers this exact shape as
one of its three modes (alongside Linear and the Wavelength quality tier), sharing its Amount
currency (% diag) with Linear mode's Angle-driven offset. This effect exists as a
single-purpose, one-click version: drop it on and it already looks right (§1.2), the same
shape rule that split the old Grade into Colour balance and Saturation (K-090). Because it has
no Angle to share a currency with, Amount is authored in raw px@comp (§2.3) instead of % diag —
scaled by the preview factor exactly like Block glitch's Block size (§3.12) — and its ROI is
declared `full-frame` rather than a tight %-diag padding, since a fixed pixel offset cannot be
bounded as a percentage of the diagonal across every comp resolution ahead of time. Category
is **Distortion**, matching RGB split. No explicit Amount-0 short circuit is needed in either
the CPU reference or the WGSL kernel: the radial offset's scale factor is an exact `0.0` at
Amount 0, so every tap collapses onto its own pixel and the tinted sum returns the input for
the primary defaults — the same un-guarded style RGB split's own kernel uses (asserted
bit-exact by test).

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

**Screen matte** twirl (collapsed): **Clip black** (%, default 0, matte at/below maps to 0),
**Clip white** (%, default 100, matte at/above maps to 1), **Clip rollback** (%, default 0,
eases the clips back toward the un-clipped matte to recover fine edge detail), **Replace
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
pixel's brightness), None leaves the despilled colour. `cheap` cost, `exact` ROI, `{0}`
temporal. Category **Utility**, beside Transform.

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

**Deferred to a follow-up (K-155):** the **spatial** Keylight controls — Screen pre-blur,
Screen shrink/grow, Screen softness, Screen despot black/white — need a multi-pass
morphology/blur pipeline with its own oracle and are out of scope of this pointwise landing;
the **Inside/Outside garbage masks** (a layer-input holdout, reusing the DoF layer-reference
pattern, §3.22); the **Colour correction** twirls (Foreground/Edge saturation, contrast,
brightness, colour balance); and the **Source crops** (per-axis edge method + crop amounts).
The core keyer above is what "properly key footage" needs; these refine it.

### 3.22 Depth of field — depth-driven lens blur (Frischluft / Camera Lens Blur-class)

A variable-radius lens blur driven by a **depth pass**: pixels near the focus plane stay
sharp, pixels far from it soften, the way a real lens throws the background out of focus.
The depth comes from **another layer** (a **Layer-reference** parameter, §1.2,
[impl/layer-input.md](impl/layer-input.md)) — the standard "footage + matching depth pass"
workflow, and the first effect to take a whole layer as an input rather than a number or a
file. The GPU kernel and its §1.6 CPU oracle predate the wiring (`lumit_gpu::fx::dof` /
`fx_dof.wgsl`); this is the effect that feeds them a real depth.

**Parameters:** Depth layer (a layer reference; unset until picked — a labelled no-op),
Depth source (a combobox beside the Depth layer picker, K-142: **None** reads the depth layer's
raw pixels — no masks, no effects, the default; **Masks** reads it plus its masks; **Effects and
masks** runs the depth layer's own effect stack into the depth pass first, a graded/blurred depth
map — same v1 temporal boundary as the effects-and-masks matte; replaces K-125's "Depth after
effects" checkbox), Depth invert (bool, default off — when on
the depth is inverted, `d' = 1 − d`, before the circle-of-confusion, swapping near and far),
Focus distance (0–1, default 0.5, the in-focus depth), Focus range (0–1, default 0.1, the
half-width of the sharp band around focus), Aperture (px@comp, default 8, slider 0–40, the
**master** maximum circle-of-confusion radius, scaling both per-side radii about its default 8),
Near blur (px@comp, default 8, slider 0–40, the max circle-of-confusion on the **near** side,
`d < focus`) and Far blur (px@comp, default 8, slider 0–40, the **far** side, `d ≥ focus`) — the
owner's "adjust close/far blur separately", Display (choice, default Rendered — a diagnostic
view: **Rendered** the normal blurred output, **Depth map** the post-invert depth as greyscale,
**Focus map** the smooth in-focus mask, white where sharp), Mix.

**Algorithm sketch.** Per output pixel, read the depth from the referenced layer's **red
channel** (0..1; by convention 0 = near, 1 = far, though the effect is symmetric about
Focus), and — when **Depth invert** is on — replace it with `1 − d` (swapping near and far).
Its distance from Focus, beyond the sharp band `range`, ramps by a smoothstep `s` to a
circle-of-confusion radius: `s ·` (**Near blur** where `d < focus`, else **Far blur**), each
per-side radius already scaled by the **Aperture** master (`radius · Aperture / 8`). Because the
near/far select flips only at `d = focus`, where `s = 0`, the radius is continuous, so the
§1.6 ULP oracle still holds. A box-weighted integer disc of that radius is averaged from the
source (edges clamped), then blended by Mix. The **Display** diagnostic modes (Depth map, Focus
map) short-circuit before the gather and write their view directly, ignoring the blur and Mix;
every shipped mode is continuous, so the §1.6 oracle covers them all (none excluded). Operates
on **premultiplied** colour (the disc gathers the working premultiplied image, so coverage and
colour blur together). `moderate`
cost, ROI a padded gather (the static declaration covers the 40 px aperture at ≥ 1080p), `{0}`
temporal. Category **Blur & sharpen**. A zero effective aperture (master or both sides at 0), a
depth everywhere inside the sharp band, or `Mix 0` are all bit-exact passthroughs, pinned by
the kernel oracle.

**Threading the depth (K-031).** `Resolved::Dof` carries only the scalars; the depth is a
whole texture, so — like the LUT's cube and Motion blur's flow field — the referenced layer's
render travels **beside** the resolved op (a parallel `layer_inputs` slot the k-th `Dof` op
binds). Preview and export render the depth through **one shared helper**
(`fxops::render_layer_input`), so the viewport and the file match. The frame cache key hashes
the referenced layer's source and transform (the same content a matte's key hashes), so
editing the depth pass retires stale frames.

**Status (v1, shipped, K-124; extended K-128):** the depth-driven disc blur above, with a depth
layer + Focus/Range/Aperture/Mix, plus (K-128) Depth invert, separate Near/Far blur under the
Aperture master, and the Rendered/Depth map/Focus map Display views. Deliberate v1 limitations
(documented, follow-ups tracked): the depth layer is sampled per its **Depth source** mode
(K-142) — None (raw), Masks, or Effects and masks (which runs its own stack into the depth) —
and **resampled to the consuming layer's raster** to align with the pixels the blur runs on;
a placement-aware depth is a follow-up (the referenced layer's own transform is not applied).
The depth layer only needs to be **in-span**
— it is expected to be *hidden* (a depth map should not render into the comp), and both the
preview decode planner and export decode a hidden layer-input reference exactly as they do a
matte source. The bokeh is a plain flat disc; shaped, bright-rimmed highlights are the
planned "DOF PRO" second effect. The depth layer is chosen with the inspector's Layer picker
(a dropdown of the comp's other layers), with the Depth source combobox beside it; an unset
or dangling reference is a no-op.

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

**Parameters:** Frame rate (fps, default 12), Phase (comp seconds, default 0), Scope
(*Everything below* | *This layer's effects*, default *Everything below*).

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

**Scope.** *Everything below* is **adjustment behaviour**: the composite of everything beneath
the effect's adjustment layer re-renders at `held_t` and is laid back over the live composite
by the adjustment's coverage (its mask × opacity), so the owner's global "posterise the whole
scene" pass is simply the effect on a full-frame adjustment layer. *This layer's effects* holds
only the layer's own **effect stack and its source sampling** at `held_t` (a per-layer time
substitution — no re-render of others, no orchestration re-entry): the effects and the footage
decode step on the coarse grid while the layer's **transform stays live**, so the layer moves
smoothly but its own effect animation and footage playback are choppy — the AE per-layer form.
The held effect time is `lumit_core::fx::this_layer_effect_time` (the grid computed on comp
time, mapped into the layer's own base), fed to `resolve_stack_temporal` as the sample time so a
`sample_temporally == false` effect still resolves at the live playhead; the held source frame
comes from the same `posterize_sample_times` snap the *Everything below* layers use. Both scopes
ship in v1.

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

**Adjustment behaviour.** Like Posterize's *Everything below*, it is an adjustment effect: the
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

---

## 4. Tier 2 — AE parity direction (post-v1)

One-line scope each; specs written when scheduled ([16-ROADMAP.md](16-ROADMAP.md)). Order
roughly by demand.

| Effect | Scope |
|---|---|
| Levels / curves per channel | Histogram-backed levels; curves land as their own colour effect (§3.10) — AE-parity versions with per-channel + alpha |
| Hue/saturation | Per-hue-range HSL adjustment (the AE "Hue/Saturation" workhorse) |
| Tritone / tint | Map shadows/mids/highlights to three colours |
| Keying | Luma key + colour key + a basic screen key (core matte generation, not Keylight parity at first) |
| Matte choker | Grow/shrink/soften mattes; companion to keying |
| Fractal noise | Seeded multi-octave noise generator, the utility texture everything is built from |
| Gradient ramp | Linear/radial two-colour generator |
| Drop shadow | Alpha-derived offset soft shadow |
| Bevel | Simple edge bevel (alpha and border variants) |
| Mosaic | Block-average pixelation |
| Find edges | Gradient-magnitude edge extraction |
| Posterise | Value quantisation (plus posterise-time as a separate temporal utility) |
| Turbulent displace | Noise-driven UV displacement |
| Wave warp | Parametric sinusoidal displacement |
| Corner pin | 4-point perspective pin (export target for the tracker) |
| Mesh warp | Grid-based freeform warp |
| Stabiliser | Flow-engine-backed smoothing of unwanted camera motion (warp-stabiliser class) |
| Tracker | Point/planar tracking producing keyframed transforms and corner-pin data |

Tier 2 effects follow every rule in §1–2; nothing in Tier 1's architecture may assume the
suite stays small.

---

## 5. Presets

- **Per-effect presets**: a named parameter snapshot (keyframes and expressions included
  when marked "animated preset"). **Per-stack presets**: an ordered list of effect
  instances with their parameters — the unit the scene calls an editing/CC pack.
- Serialised as a single shareable file (`.kpreset`, JSON payload zipped with any embedded
  small assets such as LUTs), machine-independent per K-065. Import by drag onto a layer,
  the Effect Controls panel, or the preset browser.
- Lumit ships a first-party library (grade presets §3.10, shake styles, zoom eases, glitch
  looks). Ship-with presets are data files, not code, and use only built-in effects.
- **Community packs**: preset import MUST tolerate unknown effects (imported as inert
  placeholders with their parameters preserved, mirroring
  [11-AE-IMPORT.md](11-AE-IMPORT.md) placeholder policy) so packs survive version skew.
  Post-v1 ambition: an `.ffx`/AE-preset converter for the existing pack ecosystem, tracked
  in [11-AE-IMPORT.md](11-AE-IMPORT.md); the montage scene onboards through shared packs,
  so this converter is growth infrastructure, not a courtesy.

---

## The universal strength matte (K-035)

Every effect instance carries a host-provided **strength matte** slot: none (default), the
layer's own mask set, or any layer in the comp (the matte dropdown model). The host
samples the matte in layer space, yielding per-pixel strength s ∈ [0,1] (with gain/invert
controls), and applies it uniformly:

- **Colour-type effects** (declared trait): `out = mix(input, effected, s)` — exact, cheap,
  works for every such effect with zero author effort.
- **Warp-type effects** that declare a displacement-vector output: the host scales the
  displacement field by s before resampling, so the *geometry* of the warp fades per
  pixel — the behaviour users actually want from a masked warp. Effects without vector
  output fall back to output-mix with a documented note in their reference entry.

The strength matte is a full property (animatable, expression-visible) and participates in
content hashing like any input. This replaces AE's per-effect workarounds (compound-effect
mask parameters, "composite on original", effect-only precomps).

## Open questions

1. **Flow algorithm choice.** Variational/patch-match hybrid is specced; a learned flow
   model (RAFT-class) beats it on quality but complicates the GPLv3 story, model
   distribution size, and the CPU reference oracle. Decide before flow-engine
   implementation starts; the API (dense vectors + occlusion + confidence) is stable
   either way.
2. **Gamma stage in Colour balance.** Applying gamma on a display-referred intermediate feels
   familiar but is impure; a strictly scene-linear grade with a viewing-transform-aware UI
   is cleaner. Needs a side-by-side with real CC packs before locking.
3. **Where Shake lives.** Specced as an effect that resamples the layer; an alternative is
   a transform modifier that concatenates into the layer matrix (better quality, free
   engine motion blur, but a new concept in the data model). Decide with
   [03-DATA-MODEL.md](03-DATA-MODEL.md).
4. **Preset licensing.** Ship-with preset library licence (GPLv3 data? CC0?) affects
   whether community packs can embed ours. CC0 recommended; needs Mack's sign-off.
5. **fp16 oracle tolerances.** The per-cost-class tolerance defaults in §1.6 are
   placeholders until the first three effects are implemented on both NVIDIA and AMD and
   real cross-vendor deltas are measured.

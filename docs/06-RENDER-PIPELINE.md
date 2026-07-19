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
snapshot; renders in flight keep the old snapshot. Users never see the graph.

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

### 1.2 Render order for one layer

For a visual layer at comp time `t`, the compiled subgraph is, in order:

1. **Source** — fetch or rasterise the layer source at the resolved source time. For footage:
   decode, colour-interpret, linearise, premultiply (§3). For text/shape/solid: rasterise
   vectors at the working raster size.
2. **Retime** — for a Footage layer, the retime map converts layer time to source time and the
   layer's frame-interpolation policy (nearest / blend / flow) synthesises non-integer source
   frames ([04-RETIMING.md](04-RETIMING.md)). Overrun holds the boundary frame. Retime affects
   only source fetch; keyframes on masks, effects, and transform remain in layer/comp time.
3. **Masks** — bezier paths combined top-to-bottom by mode (add, subtract, intersect, lighten,
   darken, difference, none), each with feather, expansion, opacity, inversion. Masks gate the
   layer's alpha before any effect runs, so effects see the masked image.
4. **Effect stack** — top-to-bottom ([08-EFFECTS.md](08-EFFECTS.md)). Each effect sees the
   output of the one above it, in working space, premultiplied unless it declares otherwise
   (§3.4).
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
project. There is no per-comp override: switching the project switches everything,
exactly like AE's project bit depth. The control lives as a small depth button at the
foot of the Project panel (AE's spot; click to cycle, dialogue for the long list
later), and Application Settings holds only the *default for newly created projects*.
Kernels MAY use wider internal accumulators where the algorithm needs them (large
iterative blurs, scopes), but everything a node reads or writes is project depth.

Why fp16 stays the default (2026-07-13, reviewed with Mack): fp16 here is floating
point, not AE's integer 16bpc — it already carries values above 1.0 (superwhites, glow
overshoot, up to 65504) and negatives, in linear light. fp32 buys extra mantissa (deep
shadow gradients under extreme pushes, very long chains) at 16 bytes/px: double the
bandwidth on a bandwidth-bound compositor and half the frames per cache byte. The
depth is part of every cache key's quality field, so switching depth simply re-keys
the project and the caches rebuild.

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

| Mode | Domain | Notes |
|---|---|---|
| Normal | linear | Premultiplied `over`: `A + B·(1−a_A)`. |
| Add | linear | Physically additive; the montage staple for glows/flashes. |
| Subtract | linear | `dst − src` per channel, clamped at black — Add's darkening twin (GEN-1, K-151). |
| Multiply | linear | Physical filter/shadow behaviour. |
| Darken, Lighten | either (invariant) | Per-channel min/max; monotonic transfer makes the domain irrelevant. Computed in linear. |
| Screen | perceptual | |
| Overlay, Soft light, Hard light | perceptual | |
| Colour dodge, Colour burn | perceptual | |
| Difference, Exclusion | perceptual | |
| Hue, Saturation, Colour, Luminosity | perceptual | HSL decomposition on encoded values. |
| Stencil alpha, Silhouette alpha | n/a (alpha only) | Gate the alpha of the entire composite below. |
| Stencil luma, Silhouette luma | luma per §3.5a | |
| Alpha add | n/a (alpha only) | Sums alphas without re-compositing colour; fixes seams on edge-abutting layers. |

**(a) Luma extraction** — everywhere luma is needed (luma mattes, stencil/silhouette luma):
luma = Rec.709 Y of the sRGB-encoded signal (perceptual luma), so a 50% grey solid yields
approximately 50% coverage, matching editor expectation. This is a single normative definition;
no per-feature variation.

Modes not listed (Dissolve, Linear/Vivid/Pin light, Hard mix, Divide, legacy
"Classic" variants) are post-v1; the enum is open-ended and serialised by name
([10-FILE-FORMAT.md](10-FILE-FORMAT.md)) so adding modes never breaks projects.

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
  deterministic (K-063), so their outputs are hashable values like any other.

**Invalidation is pure hash mismatch.** There is no invalidation machinery, no dirty flags, no
dependency walker: an edit changes evaluated values, values change hashes, old entries simply
stop being addressed and age out via eviction.

### 5.3 Eviction

Cost-aware LRU (GreedyDual-style), managed by the resource governor's budgets: each entry
records size in bytes and measured recompute cost in ms; eviction preference is stale ×
cheap-to-recompute × large. Additional rules: the displayed frame and a window around the
playhead are pinned; final comp frames outlive intermediates at equal staleness (playback needs
finals; intermediates rebuild from cached inputs); VRAM eviction demotes to RAM only when
recompute cost exceeds a readback-cost threshold, otherwise drops.

### 5.4 Disk cache format and location

The disk cache lives in the project's sidecar folder (`<project>.lum-cache/`,
[10-FILE-FORMAT.md](10-FILE-FORMAT.md)), deletable at any time with no correctness effect:

- `frames/<first two hex chars>/<hash>.kfr` — one file per entry: a small header (format
  version, dimensions, pixel format, colourspace marker) plus LZ4-compressed fp16 planes.
- `index.db` — SQLite: hash → file, size, recompute cost, last-use, quality. Rebuilt by scan if
  missing or corrupt; a corrupt entry is discarded silently and re-rendered.
- Default size cap 50 GB, user-set; evicted by the same cost-aware policy using the index.

### 5.5 Idle-time background cache fill

After ~200 ms without user input, an idle-priority scheduler renders final frames outward from
the playhead across the work area, at the current preview quality, into RAM (write-behind to
disk). It yields to any interactive request via epoch cancellation and is the first thing the
degradation ladder pauses. Concurrency adapts to measured per-frame cost and memory headroom
(the MFR lesson) — never a fixed thread count.

### 5.6 Cache bars

The timeline shows, per comp, a per-frame strip: **green** — final frame in RAM or VRAM at
current quality, plays in real time now; **blue** — on disk only, promotable; **dimmed
green/blue** — cached at a lower preview resolution than currently displayed. Redrawn from a
lock-free bitmap snapshot; the UI thread never queries the cache itself (K-017).

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
counts come from the deterministic formula (§4); expressions are deterministic (K-063); every
frame renders at full chosen quality regardless of load — under resource pressure export gets
slower, never different. Bit-exactness across different GPUs/driver versions is not promised
(floating-point variance); cross-machine consistency is visually lossless, same-machine
consistency is exact.

### 7.4 Encoders

All encoding goes through ffmpeg as the single abstraction: hardware encode via `h264_nvenc` /
`hevc_nvenc`, `*_amf`, `*_qsv` (probed and picked automatically), with **x264/x265 software
fallback** always available and used for quality-first masters. ProRes/DNxHR intermediates via
ffmpeg's encoders for interchange. Audio: AAC via ffmpeg; PCM in intermediates. Colour: working
space → the preset's output space (Rec.709/sRGB in v1) as the final export transform; alpha
export straight or premultiplied per output settings.

### 7.5 Preset set (v1)

| Preset | Frame | Codec | Bitrate |
|---|---|---|---|
| YouTube 1080p60 | 1920×1080 @ 60 | H.264 high, 4:2:0 | VBR target 16 Mbps, peak 24 |
| YouTube 1440p60 | 2560×1440 @ 60 | HEVC (H.264 fallback) | VBR target 25 Mbps, peak 35 |
| YouTube 4K60 | 3840×2160 @ 60 | HEVC (H.264 fallback) | VBR target 45 Mbps, peak 60 |
| Vertical 1080×1920 | 1080×1920 @ 60 | H.264 high | VBR target 16 Mbps, peak 24 |
| Master (intermediate) | comp size/rate | DNxHR HQX or ProRes 422 HQ | codec-defined |

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

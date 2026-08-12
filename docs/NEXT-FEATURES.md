# Next features — the implementation plan

**Status: living.** The implementation companion to [TODO.md](TODO.md) for the next
tranche of work: what to build, in what order, and exactly how — so a session can pick an
entry up cold. Delete an entry when it lands (its regression tests are the record, per
[14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)); when an entry changes a spec or
reverses a decision, that edit and the [02-DECISIONS.md](02-DECISIONS.md) entry land in
the same commit as the code.

In plain terms: this is the "how" file for the work the backlog only names. The two
biggest entries — the lens flare and lights — came out of two research passes
(2026-08-12) over how the industry actually does this on real footage; the sources are
cited inline so the reasoning can be checked rather than trusted.

**The flare entries were rewritten the same day**, after the owner set the budget: the
physically based flare is to be done *properly* at roughly **2 seconds per frame**, not
approximated down to milliseconds, and a sprite-based flare is welcome as a **separate
effect** rather than a replacement. Entry 1 is that plan; entry 2 is the sprite one. The
first draft of this file recommended the opposite trade and was wrong.

Standing obligations every entry inherits (they are why the estimates are not smaller):
each feature lands with its regression tests, its `app_en.arb` strings (plus
`engine_labels.dart` for anything the engine sends — the `engine_labels_test.dart` gate),
its GUIDE.md plain-English section, and its spec/decision edits, all in the same change.
New effects follow [08-EFFECTS.md](08-EFFECTS.md) §2's contract (CPU oracle beside the
WGSL kernel, bit-stable draw order, px@comp point parameters — K-260).

---

## 0. ~~The bit-stability blocker~~ — LANDED (K-353)

Kept here only as the reading order: everything below builds on a raster that now
renders the same frame twice. The cause was hardware 4× multisampling (additive fp16
into a multisample target is not reproducible on this hardware); the flare computes its
own coverage now. See K-353 and [impl/lens-flare.md](impl/lens-flare.md). Delete this
heading once the entries under it have landed.

---

## 1. The physically based flare, done properly — the ~2 s/frame target

**The brief changed, and for the better** (owner, 2026-08-12). The earlier plan here
proposed replacing the physically based path with a cheap sprite stack because the
literature says the real-time version is not usable on footage. That is the wrong trade
for Lumit. The owner's position: a sprite-based flare is welcome **as its own separate
effect** (entry 2), but the physically based one should be done *properly*, and **~2
seconds per frame is an acceptable budget** — the reference point being an acquaintance's
own generator, accurate enough to **remove** real flares from footage.

That budget changes everything. The anchor for "interactive physically based" is the
Hullin-style pipeline as actually implemented: a 27-interface lens → 351 two-bounce
ghosts, a 32×32 ray grid per ghost, **12 ms total**
([bitsquid](https://bitsquid.blogspot.com/2017/07/physically-based-lens-flare.html),
[jpgrenier](https://www.jpgrenier.org/lensflare.html)). 2000 ms is ~167× that. Lumit's
flare is already in this family — a real prescription, a pupil grid, per-wavelength
tracing, ranked ghosts, a baked starburst, an iris model, coating and f-stop. So this is
not a rewrite; it is spending a budget the effect never had on the approximations it was
forced into.

**A standing rule for this whole entry, from [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md):
every bake belongs on the CPU in Rust.** The coating tables, the starburst FFTs, the
colour-matching integration — all of them are functions of the lens and the parameters,
not of the frame. Putting them on the CPU makes them deterministic by construction (the
property K-353 just had to rescue) and removes them from the per-frame budget in the same
move. Only the ray grid needs the GPU.

### Phase A — accuracy that costs nothing (do these first)

**A1. Multi-layer anti-reflective coatings, by transfer matrix.** The single biggest
correctness gain per line of code, and the one that fixes *ghost colour*. Today (and in
every real-time implementation) the coating is an ideal single-layer quarter-wave stack
with a hand-tuned blend — one reflectance minimum, hence one broad colour cast. Real lens
coatings are 4–7 layer stacks whose reflectance R(λ) is W- or M-shaped, which is why real
ghosts run magenta, then green, then amber across a frame. The correct model is the
standard thin-film **transfer matrix**: per layer, phase thickness `δ = 2π n d cos θ / λ`
and admittance `η = n cos θ` (s) or `n / cos θ` (p); chain the 2×2 characteristic
matrices, take `Y = C/B`, then `r = (η₀ − Y)/(η₀ + Y)`, `R = |r|²`; average s and p for
unpolarised light.

Angle dependence is not a small correction: because `δ ∝ cos θ`, the whole reflectance
band **shifts blue as the angle of incidence rises**, which is exactly the observed effect
that a ghost changes hue as the source moves off-axis. Flare rays hit interfaces at large
and varied angles, so this dominates — and a scalar "coating" parameter cannot represent
any of it.

*Shape:* compute `R(λ, cos θ)` on the CPU into a 64×64 (or 128×128) f32 LUT **per
interface** at lens-load time, upload as a texture array, and have the trace do a bilinear
fetch. Per-frame cost ≈ zero. Put the layer stack in the lens file as data, and keep a
per-interface calibration offset — real coatings are measured, not predicted (see Phase E).
*Refs:* [COMSOL thin-film](https://doc.comsol.com/6.1/doc/com.comsol.help.roptics/roptics_ug_optics.6.73.html),
[AR coating](https://en.wikipedia.org/wiki/Anti-reflective_coating).

**A2. Real spectral radiometry.** Split geometry from radiometry, because they vary at
different rates. Ray *geometry* varies slowly with λ (dispersion is a smooth perturbation)
— 8–16 wavelengths, interpolated, is plenty. Ray *radiometry* varies fast, because A1's
reflectance oscillates several times across the visible — so evaluate it at **81 samples,
5 nm across 380–780 nm**, which costs LUT fetches rather than rays. Integrate against the
**CIE 1931 2° colour-matching functions** to XYZ, then one fixed 3×3 into working RGB.

Do *not* keep three RGB samples: three samples cannot represent a curve with three minima,
so ghost hues come out systematically wrong. (Hero-wavelength sampling is not applicable —
that is a Monte Carlo variance-reduction trick, and this is a deterministic quadrature.)
*Refs:* [CIE CMFs](https://www.fourmilab.ch/documents/specrend/),
[pbrt colour](https://pbr-book.org/4ed/Radiometry,_Spectra,_and_Color/Color).

### Phase B — spend the budget on rays

**B1. A dense, adaptive ray grid with per-ray splatting.** Raise 32² toward **256²** per
ghost, allocated adaptively: a 16² prepass gives each ghost a bounding box and peak
irradiance, then resolution is handed out in proportion to on-sensor area. Critically,
**replace the interpolated-quad rasterisation with per-ray splatting weighted by the exact
Jacobian**, and clip against every aperture **per ray** rather than per quad. Sparse grid
plus bilinear interpolation of the transfer is where the current model actually dies:
caustic folds inside a ghost get averaged away, and per-quad energy is wrong exactly where
the bundle folds.

This also retires the sliver/fold machinery that K-262 through K-353 kept patching
(`MIN_QUAD_PX`, sliver drops, the widening K-353 just added) — there are no quads to fold.
It is the largest single change in this entry and should be its own PR.

**B2. A field-angle-dependent starburst.** The starburst is Fraunhofer diffraction and is
correctly `|FFT(pupil)|²` — that part is already right. Two things are not. First, **the
diffracting aperture changes shape across the frame**: off-axis the limiting pupil is no
longer the iris but the front and rear mechanical stops clipping it into a **cat's-eye**,
plus a `cos θ` foreshortening. So ray-trace the true exit-pupil outline at 8–16 field
angles, FFT each at 2048²–4096², and interpolate. Second, the cheap **λ-rescale of one
FFT is exact only for a real-valued pupil**; the moment the pupil carries phase (dust,
scratches, coating defects — the things that make real starbursts asymmetric and
interesting) it needs one FFT per wavelength. At 81 FFTs of 2048² on the CPU with
`rustfft` that is well under a second, **and it is a bake, not a per-frame cost**.

A free correctness check: blade parity. An even blade count gives that many spikes, an odd
count twice as many. If the FFT does not reproduce that by itself, something is wrong.
*Refs:* [Fraunhofer/Fresnel](https://phys.libretexts.org/Bookshelves/Optics/BSc_Optics_(Konijnenberg_Adam_and_Urbach)/06:_Scalar_diffraction_optics/6.07:_Fresnel_and_Fraunhofer_Approximations),
[Tang's flare report](https://www.cs.toronto.edu/~hxtang/projects/flare_render/lensflare_huixuan.pdf).

### Phase C — the things nobody does in real time

**C1. Energy-ranked four-bounce ghosts.** For N interfaces there are exactly `N(N−1)/2`
two-bounce paths (351 at N=27) — what everyone traces. Four-bounce paths number ~10⁵ at
the same N. With a modern coating (R ≈ 0.3%) a four-bounce path carries ~10⁻⁵ of a
two-bounce one, so the vast majority are invisible; but the sun is ~10⁵× a normal
highlight, and a few four-bounce paths land as *tight, well-focused* spots rather than
broad discs. Those are exactly the small hard artefacts a removal model would otherwise
leave as residual. With uncoated or vintage glass (R ≈ 4%) they are plainly visible.

*Shape:* enumerate all of them on the CPU, run a 16² energy prepass, keep the top few
hundred by peak sensor irradiance, render survivors at full grid. This is the
ranked-path method stray-light analysis already uses (Zemax's Ghost Focus Generator does
the same), and against a 2 s budget it is nearly free.

**C2. Fresnel ringing on ghost edges, by fractional Fourier transform.** The starburst is
Fraunhofer; **the ghosts are Fresnel**, because each ghost image is defocused by its own
amount. The operator that interpolates between "identity" (in contact) and "Fourier" (far
field) as a function of defocus is the **fractional Fourier transform** — one parameter
gives both the hard aperture polygon and the diffraction pattern, and everything
real-time drops it, which is why real-time ghosts have hard edges. Budget: applying it to
all 351 ghosts × 12 λ at 256² is ~2 s on its own, so apply it to the **top ~32 ghosts by
energy** (~100 ms). Hardest item here; do it last. *Ref:* Joo et al. 2016,
[CGF 35(4)](https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.12953).

### Phase D — area and extended sources (the part that makes footage work)

This is the answer to "make it work with area lights", and the research is unambiguous
about the shape of it.

**The wrong model first, so it is not re-proposed.** Flare from an extended source is
**not** the convolution of the source with a single point-source flare PSF. Talvala et al.
measured this directly and their figure caption says it plainly — the glare patterns are
not shift-invariant ([Stanford, SIGGRAPH 2007](https://graphics.stanford.edu/papers/glare_removal/glare_removal.pdf)).
A ghost 400 px from the source moves and changes shape at a different rate than the source
does, so one global convolution cannot be right. (Hullin's own limitations section offers
exactly that approximation for area lights, unvalidated — it is the gap this fills.)

**The right model: convolution *per ghost*, in that ghost's own frame.** Each ghost path
is an imaging system, and its map from incident direction to sensor position is locally
affine — this is the classical paraxial ghost-imaging result, that each ghost sub-system
has its own cardinal points and its own **ghost magnification**, so a ghost is a
magnified, generally defocused *image of the source*. Linearising at the source centre
gives a 2×2 Jacobian `J_g` per ghost, and then

```
Ghost_g(x)  =  [ PointGhost_g  ⊛  (S ∘ J_g⁻¹) ] (x)
```

— the point-source ghost kernel convolved with the source's radiance map, affinely warped
by that ghost's Jacobian (flip, anisotropic scale and rotation included; ghosts on the far
side of frame are inverted images of the source). Shift-invariance holds *within* a ghost
over the source's angular support, and fails *between* ghosts — which is precisely why the
per-ghost decomposition is the correct treatment of a shift-variant system.

**What this buys, concretely:** a neon tube's ghosts become elongated bars, a window's
become rectangles, a headlight's become discs — and a near-focused ghost shows the *shape
of the source* while a defocused one takes the aperture polygon. Point sprites structurally
cannot do any of that.

**Cost:** one flare evaluation per source (unchanged from today) plus G small 2-D
convolutions — milliseconds, and independent of source area. Far inside the budget.

**Status (K-355):** area sources already work, by **direct sampling** — the source's extent
is measured from its flux and the flare is evaluated at a small grid of points across it,
each carrying a share of the flux. That is the reference method (it is what the Monte Carlo
oracle in D3 does, minus the randomness), so ghosts already take the source's shape. What
Phase D adds is *efficiency and exactness*: sampling converges to the warped convolution but
needs enough samples that neighbours land closer than a ghost is wide, and the cap is
currently 5×5 per source. Build it when a source is wide enough that replication shows.

*Shape:* get `J_g` per ghost by finite-differencing the existing trace at θ₀ ± δ (or read
it off the ray grid); warp the source's radiance tile; convolve with that ghost's kernel;
scale by the path's throughput; splat. It runs **inside** the per-wavelength loop, because
`J_g` and the kernel are both λ-dependent.

**D1. Guard the linearisation with adaptive subdivision.** Evaluate `J_g` at the source's
corners; if it varies across the source by more than a kernel width, quadtree-split the
source region and repeat per patch, blending with a windowed partition of unity (the
Efficient Filter Flow structure). This is the correctness dial, it degrades gracefully, and
2 s buys dozens of patches per source.

**D2. The starburst half *is* shift-invariant, so take the free win.** Veiling glare and
the starburst are centred on the source and vary only slowly with field angle, which is why
every eye-glare paper convolves the whole HDR frame with one PSF (Ritschel's *Temporal
Glare*, Kakimoto, Spencer). So convolve the source region's radiance map with the
diffraction kernel directly, per wavelength — one FFT pair per band, and softboxes and
windows get correct soft glare for nothing.

**D3. A brute-force Monte Carlo mode, as the oracle rather than the default.** The
production reference is Animal Logic's *Lego Movie 2* renderer: connect a random sample on
the light source to a random sample on the front lens, importance-sampled jointly over
paths and pupil cells (which lifted their sensor hit rate from ~15% to ~90%), and splat.
Use it in tests to assert the fast path matches within tolerance, and for the case where
linearity genuinely fails — **occlusion**, where a lens hood or foreground object clips
part of the source, which is not linear in the source and must be sampled.
*Ref:* [DigiPro 2019](https://animallogic.com/wp-content/uploads/2023/06/Physical-Based-Lens-Flare-Rendering.pdf).

**D4. Where the source region comes from, on real footage.** Log-luminance threshold →
connected components → morphological close and a small dilation → drop components below an
area/flux floor → cap at 16 by total flux → **track components frame to frame** so a
flickering practical does not pop. Keep the region's **radiance map**, not just a centroid
and a total — that map is the `S` above.

**Flux matters more than shape, and clipping destroys it.** Flare amplitude is linear in
source flux, so clipping a 1000:1 source at 1.0 under-drives the entire flare by orders of
magnitude. In descending order of trustworthiness: use raw/HDR footage; else standard
single-channel highlight reconstruction; else — and this is the production answer — an
explicit **per-region intensity multiplier** in the UI (Animal Logic flag lights for the
flare pass and give them separate intensity multipliers). A single-image HDR CNN may
provide an initial *guess* only; the literature is blunt that saturated content is
hallucinated. Never let a hallucinated highlight silently set flare amplitude — surface the
number and let the artist tune it. This is the calibration knob the physical world always
needs.

### Phase E — only if flare *removal* is genuinely a goal

Every serious attempt in the literature converges on the same conclusion: **coatings and
manufacturing deviations cannot be predicted, only measured.** Walch et al. built their
model by measuring real captured flares and fitting, precisely because the internal
parameters — especially the AR coatings — can only be approximated. The 2026
*Precomputed Lens Transport Maps* work identifies the gap that matters for invertibility:
prior polynomial and neural lens models **omit Fresnel intensity throughput**, which
precludes accurate simulation of internal reflections.

So if removal is the goal, what matters is not more wavelengths but: (1) differentiability
of the forward model in its parameters, because removal is *fitting*, not simulating;
(2) correct handling of the aperture occlusion discontinuity, which is the dominant fitting
error when a smooth model smears across it; (3) Fresnel throughput, not just ray geometry;
(4) sub-pixel ghost positions, controlled by the element spacings you are least likely to
know — so make them fitted, not fixed; (5) strict scene-referred linearity with exposure
modelled explicitly; (6) a **per-lens calibration workflow** — photograph a point source
across a grid of field angles and f-stops, fit spacings and coating stacks to the observed
ghosts.

**Read on the acquaintance's generator:** almost certainly a physically based forward model
*plus* per-lens calibration, then fit-and-subtract. The physics gets the right parametric
family; the calibration picks the right member of it. A simulator without a calibration
path will not reach that bar however many wavelengths it samples.

### What not to build

Lee & Eisemann 2013's paraxial matrix model and the polynomial-optics line (Hullin 2012,
Bodonyi 2025) are **speed devices**: they trade bounded fit error for evaluation cost. At
2 s/frame they cost accuracy and buy nothing. Keep polynomial optics only as a
ground-truth-comparison harness if it is ever worth quantifying how far the interactive
path was off. Also skip precomputed *flare-field* interpolation across source positions —
Hullin's team tried warping precomputed flares and reported it failed, flares being too
sensitive to subtle changes.

### Suggested build order within entry 1

A1 → A2 (free accuracy, no budget spent) → B2 (a bake, big visual payoff) → D4 + D2
(source regions and the free shift-invariant half) → D (per-ghost warped convolution, the
headline for footage) → B1 (dense grid + splatting, the big one) → C1 → C2 → E.

---

## 2. A sprite-based flare, as its own effect

Blessed explicitly by the owner alongside entry 1: an **Optical Flares-style** effect that
is not physically derived at all. No bright pass and no ray tracing — a light **position**
(px@comp, animatable and trackable) drives a designer-authored stack of elements, each
placed along the line from the light through frame centre at its own offset, scale, tint
and opacity: glow, iris ghosts, halo ring, streak, starburst. Deterministic on video, zero
flicker, art-directable, and cheap (one pass of N procedural quads).

This is a **new effect** in [08-EFFECTS.md](08-EFFECTS.md) §3, not a mode of the physical
one — the two answer different questions and mixing them is what made the plan muddled the
first time. Its element stack is data (a preset file), not shaders. The one genuinely new
kernel it wants is the **anamorphic streak**: a Kawase streak filter — downsample ~1/16,
then 3–4 passes each sampling 4 taps along the streak direction at distance 4^pass, weight
`a^(b·dist)`, attenuation ~0.9–0.95
([Oat](https://www.chrisoat.com/papers/Oat-ScenePostprocessing.pdf)) — a directional
variant of blur passes the engine already has, and useful to the physical flare too.

---

## 3. Harden Matte detection on footage (small, do early)

Matte mode already has half of what the literature asks for: the luma gate is soft
(`threshold` + `threshold_softness`, `lens_flare::threshold_gate`), and K-267's tile flux
summing already weighs an area source by its area rather than one pixel. What still
flickers on video, and the fixes:

1. ~~**Anchor jumping**~~ — **landed as K-354, completed by K-355.**
2. ~~**Fireflies**~~ — **landed as K-355.** Each tile now carries its whole gated flux, its
   own flux centroid and its mean colour rather than one pixel's, so no single hot pixel can
   move a light or define its colour. A 40× sparkle shifts a 64 px source by under a pixel.

**What is left of this entry:** nothing. Area sources are handled by direct sampling
(K-355), which is the reference method; Phase D's per-ghost warped convolution remains the
*optimisation* of that, not a correction to it.

**Temporal smoothing is a recorded non-option**, not an oversight: a frame must be a
function of the document and the frame alone (docs/14 determinism; the caches name frames
on exactly that promise), and a detector that remembers the previous frame breaks random
access, preview/export identity and the frame oracle in one move. The footage answer to
"the threshold pops" is entry 1's Phase D — real source regions with real flux — not
history buffers.

## 4. Light layers, and area lights via LTC

**What exists:** nothing in the model — `LayerKind` has no Light; the flare reserves
Lights mode "until light layers land (K-257)"; the roadmap parks lights in Phase 5.
The user-visible goal: a light you can aim at *footage* and have it read as light.

**Step 3a — the Light layer (model + UI, no shading yet).**
`LayerKind::Light` in [03-DATA-MODEL.md](03-DATA-MODEL.md) — a decision-sized model
change, logged in 02-DECISIONS. Kinds: **point**, **spot**, **area (rect)** — the rect
is the one that earns the entry. Properties (all animatable `Property`s): colour,
intensity, radius/size (a rect light has width × height), falloff. Transform reuses the
layer transform (a rect light is a rectangle in 2.5D space exactly as a layer is — same
position/rotation basis the camera pose already uses). Like a Camera, it draws no
pixels; like a Null, it needs a pickable gizmo in the Viewer (the Camera's no-box
carve-out in `viewer_panel_frb.dart::_boxes` is the pattern — do not repeat its "cannot
be picked" gap for lights). Bridge: fold into `BridgeLayerInfo`/the comp read model, an
`addLightLayer` op, Timeline identity colour (docs/15 §6.1 reserves token values).

**Step 3b — flare Lights mode (K-257, cheap once 3a lands).**
Resolve each Light layer's comp-space position at the frame, project through the active
camera pose (the same maths `comp.camera_pose(t)` feeds the realiser), fill the
`GpuLight` slots the flare already dispatches. A flare that follows a keyframed light is
the tracked-flare workflow with no tracker. Delete the "resolves as Manual" fallback and
its comment.

**Step 3c — area-light shading of layers: Linearly Transformed Cosines.**
The state of the art for real-time polygonal area lights, and comfortably WGSL-shaped
([Heitz et al. 2016](https://eheitzresearch.wordpress.com/415-2/),
[tutorial](https://learnopengl.com/Guest-Articles/2022/Area-Lights)):

- Two **64×64 LUT textures** (a 3×3 inverse-matrix in RGBA + Fresnel/form-factor
  scalars), indexed by (roughness, view angle), bilinear-filtered. The data is
  published — [selfshadow/ltc_code](https://github.com/selfshadow/ltc_code) — embed it
  as bytes, no fitting work. Licence-check the repo before vendoring; re-deriving the
  tables from the paper is the fallback.
- Per shaded pixel: fetch matrix, transform the light rect's four vertices into
  cosine space, sum an analytic integral per edge, apply the form-factor correction.
  Diffuse is the same integral with the identity matrix.
- Measured cost in the literature: ~0.5 ms *full-screen* on a 2014 laptop GPU; shading
  layer quads it is noise.

For a 2.5D compositor the geometry collapses beautifully: the shaded surface is a flat
layer plane (normal = layer orientation), the light is a rect in the same space — LTC
diffuse over a quad produces exactly the smooth gradient an editor expects a softbox to
throw across footage. Ship that as the default look; per-pixel normals (normal-map
AOVs, luminance-derived fake normals) are explicitly out of scope for the first landing
— both are content-dependent quality cliffs, and the flat-plane result is already the
honest 2.5D answer ([Nuke's Relight](https://learn.foundry.com/nuke/content/reference_guide/3d_nodes/relight.html)
is the ceiling to aim at later, not now).

Where it runs: a compositor pass on each lit layer's quad (the realiser already walks
layers with their 3D poses — the shading term multiplies into the layer sample). Gate it
behind a per-layer **"accepts lights"** switch defaulting on for 3D layers, so 2D
montage work pays nothing. **Spec edits:** 06-RENDER-PIPELINE gains the lighting pass;
08-EFFECTS is untouched (this is not an effect); 03-DATA-MODEL gains the switch.

**Test plan:** CPU oracle of the LTC integral for a handful of (roughness, angle, rect)
cases against the published reference implementation's numbers; a GPU oracle rendering
one lit quad and asserting the gradient's monotonic falloff and its peak under the
light's centre; determinism (two renders, identical bytes); a no-lights comp renders
byte-identical to today (the pass must be a true no-op when absent).

## 5. Light wrap — the cheapest "light meets footage" feature that exists

The compositing classic for keyed foregrounds: blur the *background*, mask it by the
inverted-and-blurred alpha edge of the foreground, screen it back over the edges — the
background's light "wraps" the subject
([explainer](https://max-klomeier.medium.com/introduction-light-wrapping-70b03f2092c3)).
One blur plus two mask multiplies, entirely out of kernels the engine already has; per
line of code nothing else in this file comes close, and it pairs naturally with entry 3
(a rect light behind a keyed subject + wrap = the money shot).

Build as an ordinary effect (docs/08 §2 contract): parameters **width** (px@comp),
**intensity**, **wrap source** (the layer stack below, the way Fast motion blur's
adjustment-layer case names it — note that TODO's "fast motion blur only works on
footage layers" entry describes the same below-stack plumbing; whichever lands first
digs the tunnel the other reuses). CPU oracle + WGSL kernel + arb/engine-label strings
+ an 08-EFFECTS §3 entry.

## 6. Viewer bar completion — the two owed halves of what just landed

Natural follow-ups to K-352 and the resolution dropdown, both small and both already
specified in docs/07 §2.2:

- **Third and Auto resolution rows, stored per comp** (item 2). Third = scale 1/3 (the
  adaptive ladder already renders it — `resolutionThird` exists as a tier name). Auto =
  render only the pixels the magnification can display, which is what
  `reportViewerScale` already measures — the row mostly *names* existing behaviour.
  Per-comp storage rides the session blob exactly as `viewerLooks` does (K-314's
  pattern, K-245's blob).
- **Background colour swatch** (item 10): per-comp background colour (a document write,
  undoable, unlike the looks) plus quick black/white/checker. The checker option is
  K-352's flag; the swatch is the first UI for `comp.background` at all.

## 7. Region of interest (docs/07 §2.2 item 7)

Drag a rectangle; the engine composites only that region. The realiser already
composites at a scaled raster (K-186 / the preview-scale work), so the mechanism is a
scissor/viewport on the composite target plus an offset in the present — not a new
pipeline. One-click clear; never affects export (same construction as the preview
scale: the export renderer never receives it). Frame names must fold the region in
(the K-346/K-352 mechanism — a cropped frame is not the full frame) or refuse names
while a region is set; folding is better, scrubbing inside a region is the use case.

---

## Suggested order

**Landed so far** (2026-08-12), newest first — each has its decision entry and its
regression tests, so this table is a reading order rather than a record:

| Entry | State |
|---|---|
| 0 — flare bit-stability | **Landed**, K-353. Everything else assumes it |
| 3 — harden Matte detection | **Landed**, K-354 + K-355 (both halves) |
| **Area sources** (part of 1·D) | **Landed**, K-355 — by direct sampling, the reference method |
| 1·A1 — multi-layer AR coatings | **Landed**, K-356 |
| 6 — viewer bar completion | **Landed**, K-357 |
| 5 — light wrap | **Landed**, K-358 |
| 2 — sprite-based flare | **Landed**, K-359 |
| 4a/4b — Light layer + flare Lights mode | **Landed**, K-360 |
| 4c — area lights shading layers | **Landed**, K-361 — the closed-form diffuse integral; LTC's fitted matrix tables deliberately not shipped, and the entry says why |
| 7 — region of interest | **Landed**, K-362 — a window on the composite. Saves the composite, the display encode and the publish; **not** the effect stack. Layer culling is the upgrade, and K-362 records why it is not here |

**Still to build**, in the order they are worth doing. Everything that is not flare
internals has now landed — what remains is the ray-tracing accuracy work on entry 1:

| # | Entry | Why this position |
|---|-------|-------------------|
| 1 | 1·A2 — spectral radiometry | The accuracy A1 opened up: now that the coating stack varies *fast* with λ, sampling reflectance at the band centre under-resolves it. **Not as cheap as this file first claimed** — the plan assumed a LUT fetch, but reflectance sits inside the per-ray, per-surface loop, so band-averaging it costs real trace time (~20 surfaces × N sub-samples per ray). Budget for it deliberately, or integrate per (surface, band) into the bake where the ray path does not vary |
| 4 | 1·B2 — field-angle starburst | A bake with a big visual payoff; independent of the ray work |
| 5 | 1·B1 — dense grid + per-ray splatting | The big one; retires the quad/sliver machinery. Its own PR |
| 9 | 1·D — per-ghost warped convolution | The *optimisation* of the area sampling K-355 shipped, not a correction to it. Build when a source is wide enough that sample replication shows |
| 10 | 1·C1, 1·C2 — four-bounce, Fresnel ringing | The last accuracy percent; C2 is the hardest item here |
| 11 | 1·E — calibration and invertibility | Only if flare *removal* is genuinely a goal |

Skipped deliberately, so they are not re-proposed: **paraxial and polynomial-optics lens
models** (Lee & Eisemann 2013, Hullin 2012, Bodonyi 2025 — speed devices that cost accuracy
and buy nothing at 2 s/frame); **precomputed flare-field interpolation across source
positions** (Hullin's team tried warping precomputed flares and reported failure); **a
global source ⊛ PSF convolution for ghosts** (measurably wrong — flare is shift-variant,
Talvala 2007); **temporal history buffers** for flare smoothing (they break the determinism
the caches are named on); **ML relighting** (non-deterministic, wrong weight class);
**representative-point sphere/tube lights** (LTC covers the rect case a compositor needs;
add spheres only if a use case shows up).

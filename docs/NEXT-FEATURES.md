# Next features — the implementation plan

**Status: living, and mostly landed.** The implementation companion to [TODO.md](TODO.md)
for this tranche of work. Its rule stands: delete an entry when it lands (the regression
tests are the record, per [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)) — so this
file is now two things. First, the **verification map**: everything that landed, which PR
holds it, and where it deviated from the plan as first written, so the stack can be checked
against intent before anything merges. Second, the **remaining plan**: the flare's
ray-tracing accuracy work (old entry 1), which is the only entry still open, plus a short
list of loose ends.

Standing obligations every remaining entry inherits (they are why the estimates are not
smaller): each feature lands with its regression tests, its `app_en.arb` strings (plus
`engine_labels.dart` for anything the engine sends — the `engine_labels_test.dart` gate),
its GUIDE.md plain-English section, and its spec/decision edits, all in the same change.
New effects follow [08-EFFECTS.md](08-EFFECTS.md) §2's contract (CPU oracle beside the
WGSL kernel, bit-stable draw order, px@comp point parameters — K-260).

---

## State of play: three stacked PRs, merge in order

Nothing has touched `main`. The work sits on three stacked branches; **merge #93 first,
then #94, then #95** — each later PR's diff is only its own commits, but its branch
contains everything beneath it.

| PR | Branch | Contains | Decisions |
|---|---|---|---|
| [#93](https://github.com/luminalmvm/Lumit/pull/93) | `claude/viewer-grid-and-startup` | Transparency grid sized/snapped to the comp and seeing through to nothing; opening a project shows one card | K-351, K-352 |
| [#94](https://github.com/luminalmvm/Lumit/pull/94) | `claude/flare-v2` | Flare bit-stability (own coverage, no MSAA); source anchored at its flux centroid; jitter killed by per-pixel tile statistics; area sources by direct sampling; multi-layer TMM coatings; this plan's rewrite | K-353, K-354, K-355, K-356 |
| [#95](https://github.com/luminalmvm/Lumit/pull/95) | `claude/viewer-completion` | Third/Auto resolution per comp + background swatch; Light wrap; Sprite flare; Light layers + flare Lights mode; area-light shading of layers; region of interest | K-357, K-358, K-359, K-360, K-361, K-362 |

Every commit carries its decision entry, regression tests, GUIDE.md section and arb
strings. Verified before each push: fmt + clippy clean (warnings-as-errors), 377
lumit-core / 71 lumit-eval / 99 lumit-render / 97 lumit-gpu (single-threaded) / 213
lumit-bridge, `flutter analyze` clean, l10n gates green.

**Crowdin upload owed** for the English keys added across the stack — listed per commit
in the messages and gathered in PR #95's description. Headlines: `fxStreak`,
`switchAcceptsLights(-On/-Off)`, `tipRegionOfInterest`, `tipDragRegionOfInterest`,
`tipClearRegionOfInterest`, plus the earlier viewer-bar and light-wrap batches.

**Known-flaky, not a regression:** running all of `viewer_panel_frb_test.dart` in one go
can fail 5–6 frame-arrival tests with `Could not create the renderer … Not enough memory
left` — GPU device exhaustion from many test shells on one machine. Each fails-in-bulk
test passes in isolation, and the signature reproduces with the changes disabled.

## Where the landed work deviates from the plan as first written

Read these before reviewing — each is deliberate, each is recorded in its decision entry,
and none is a silent narrowing:

- **Area lights shade layers without LTC's fitted tables** (K-361; the plan's entry 4c
  specified vendoring `selfshadow/ltc_code`). The diffuse case a 2.5D compositor needs is
  the *identity-matrix* case of LTC — a closed-form polygon form factor, four edges, four
  `acos`, no tables, no licence check, no vendoring. The code is shaped so a matrix fetch
  drops in ahead of the same integral when roughness/specular is ever wanted; K-361 records
  the reasoning at length.
- **Light adds, it does not replace** (K-361). The lighting pass multiplies by
  `1 + light`, not by the light alone — so a comp with no Light layers renders
  byte-for-byte as before (a test, not a hope), and dropping in a light can never plunge
  the unlit remainder into black.
- **`accepts_lights` defaults on for every layer**, not only 3D ones as the plan sketched
  — harmless, since a comp with no lights shades nothing either way, and it means placing
  a light lights the scene without hunting for a switch. A 2D layer is shaded flat at
  z = 0 whatever its transform stores, matching where the compositor actually draws it.
- **The region of interest is not a scissor** (K-362; the plan said "scissor/viewport on
  the composite target"). It is a *window*: the comp-pixels-to-NDC mapping shifts and the
  target is sized to the region, so the composite writes only the pixels asked for. Draw
  lists holding an adjustment layer or a motion-blurring layer **refuse the window** (both
  stage through comp-sized intermediates) and the frame is composited whole and cropped —
  same pixels either way, pinned by a regression test. Honest costing: it saves the
  composite, display encode and publish, **not** the per-layer effect stack. Frame names
  fold the region in (the "folding is better" option this file named).
- **Area flare sources landed by direct sampling** (K-355), the reference method — the
  per-ghost warped convolution below is now the *optimisation* of something correct, not
  a correction to something wrong.
- **Spectral radiometry was re-costed** (details in the entry below): reflectance sits
  inside the per-ray, per-surface loop, not behind a free LUT fetch, so it is a deliberate
  budget line rather than the freebie the first draft claimed.
- **Light wrap reads its background through the layer-input mechanism** (K-358,
  `layer_input_param → "background"`), the same tunnel DoF's depth input uses — not new
  below-stack plumbing.
- **`Resolved` stays `Copy` and carries an `#[allow(large_enum_variant)]`** with its
  reasoning: boxing the flare params would cost the byte-for-byte frame-key hashing.
  `LayerKind::Light` *is* boxed (eight animatable Properties made it the largest variant
  by far).

## Loose ends — small, none blocking a merge

1. **Light layers have no purpose-drawn Viewer gizmo.** The plan asked for one (centre
   mark; rect outline for an area light) and warned against repeating the Camera's
   "cannot be picked" gap. Today a Light falls through `_boxes()`'s generic path. Works,
   but an area light's emitting rectangle is not visualised in the Viewer.
2. **Flare Lights mode does not project through the camera pose.** `lights_at` yields
   comp-space x/y and the flare uses them directly; a light with z, viewed through a
   camera, flares at its unprojected position. The plan's 3b asked for projection through
   `comp.camera_pose(t)`. Shading (K-361) handles depth correctly; only the flare's
   source placement is flat. Small, contained in `resolved.rs`'s lens_flare arm.
3. **Flare WGSL cross-platform bit-stability** remains verified on this machine's
   adapter; CI's macOS lane is the second datapoint (standing note from the audit ledger).

---

## The one open entry: the physically based flare at ~2 s/frame

The brief (owner, 2026-08-12): the physically based flare done *properly* at roughly
**2 seconds per frame** — the reference point being an acquaintance's generator accurate
enough to **remove** real flares from footage — with the sprite flare as its own separate
effect (landed, K-359). Bit-stability (K-353), centroid anchoring (K-354), area sources
(K-355) and multi-layer TMM coatings (K-356) are in; what remains is the accuracy work
below, in the order it is worth doing.

**Standing rule, from [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md): every bake
belongs on the CPU in Rust.** Coating tables, starburst FFTs, colour-matching integration
— all functions of the lens and parameters, not the frame. CPU bakes are deterministic by
construction and free per frame. Only the ray grid needs the GPU.

### A2. Real spectral radiometry

Split geometry from radiometry, because they vary at different rates. Ray *geometry*
varies slowly with λ (dispersion is a smooth perturbation) — 8–16 wavelengths,
interpolated, is plenty. Ray *radiometry* varies fast, because the K-356 coating stacks
oscillate several times across the visible — so evaluate reflectance at **81 samples,
5 nm across 380–780 nm**, integrate against the **CIE 1931 2° colour-matching functions**
to XYZ, then one fixed 3×3 into working RGB.

Do *not* keep three RGB samples: three samples cannot represent a curve with three
minima, so ghost hues come out systematically wrong. (Hero-wavelength sampling is a Monte
Carlo variance trick; this is a deterministic quadrature.)

**Re-costed after an attempt:** the first draft called this free, assuming a LUT fetch.
It is not — reflectance sits inside the per-ray, per-surface loop, so band-averaging
costs ~20 surfaces × N sub-samples per ray of real trace time. Either budget for it
deliberately, or integrate per (surface, band) into the bake where the ray path does not
vary. *Refs:* [CIE CMFs](https://www.fourmilab.ch/documents/specrend/),
[pbrt colour](https://pbr-book.org/4ed/Radiometry,_Spectra,_and_Color/Color).

### B2. A field-angle-dependent starburst (a bake, big visual payoff)

The starburst is Fraunhofer diffraction and correctly `|FFT(pupil)|²` — that part is
right. Two things are not. First, **the diffracting aperture changes shape across the
frame**: off-axis the limiting pupil is the front and rear mechanical stops clipping the
iris into a **cat's-eye**, plus `cos θ` foreshortening. Ray-trace the true exit-pupil
outline at 8–16 field angles, FFT each at 2048²–4096², interpolate. Second, the cheap
λ-rescale of one FFT is exact only for a real-valued pupil; the moment the pupil carries
phase (dust, scratches, coating defects — what makes real starbursts asymmetric) it needs
one FFT per wavelength. 81 FFTs of 2048² on the CPU with `rustfft` is well under a
second, **and it is a bake, not a per-frame cost**.

Free correctness check: blade parity — an even blade count gives that many spikes, an odd
count twice as many. *Refs:*
[Fraunhofer/Fresnel](https://phys.libretexts.org/Bookshelves/Optics/BSc_Optics_(Konijnenberg_Adam_and_Urbach)/06:_Scalar_diffraction_optics/6.07:_Fresnel_and_Fraunhofer_Approximations),
[Tang's flare report](https://www.cs.toronto.edu/~hxtang/projects/flare_render/lensflare_huixuan.pdf).

### B1. Dense adaptive ray grid with per-ray splatting (the big one — its own PR)

Raise 32² toward **256²** per ghost, allocated adaptively: a 16² prepass gives each ghost
a bounding box and peak irradiance, then resolution is handed out in proportion to
on-sensor area. Critically, **replace interpolated-quad rasterisation with per-ray
splatting weighted by the exact Jacobian**, clipping against every aperture **per ray**.
Sparse grid + bilinear interpolation is where the current model actually dies: caustic
folds inside a ghost get averaged away, and per-quad energy is wrong exactly where the
bundle folds.

This retires the sliver/fold machinery K-262 through K-353 kept patching (`MIN_QUAD_PX`,
sliver drops, K-353's corner widening) — there are no quads to fold.

### D. Per-ghost warped convolution — the *optimisation* of K-355's area sampling

**The wrong model first, so it is not re-proposed:** flare from an extended source is
**not** source ⊛ one global PSF — glare is shift-variant
([Talvala 2007](https://graphics.stanford.edu/papers/glare_removal/glare_removal.pdf)).

**The right model:** each ghost path is an imaging system whose map from direction to
sensor position is locally affine (classical paraxial ghost imaging — each ghost has its
own magnification). Linearise at the source centre for a 2×2 Jacobian `J_g` per ghost:

```
Ghost_g(x)  =  [ PointGhost_g  ⊛  (S ∘ J_g⁻¹) ] (x)
```

— the point-source ghost kernel convolved with the source's radiance map, affinely warped
per ghost (flip, anisotropic scale, rotation). Shift-invariance holds *within* a ghost
and fails *between* ghosts, which is why the per-ghost decomposition is the correct
treatment. A neon tube's ghosts become bars, a window's rectangles — a near-focused ghost
shows the *shape of the source*, a defocused one takes the aperture polygon.

**Status:** K-355's direct sampling already converges to this (it is the Monte Carlo
oracle minus the randomness), capped at 5×5 samples per source. Build the convolution
when a source is wide enough that sample replication shows. Get `J_g` by
finite-differencing the existing trace at θ₀ ± δ; runs inside the per-wavelength loop
(`J_g` and the kernel are both λ-dependent). Sub-items when it is built:

- **D1** — guard the linearisation: evaluate `J_g` at the source's corners, quadtree-split
  when it varies by more than a kernel width, blend with a windowed partition of unity.
- **D2** — the starburst half *is* shift-invariant: convolve the source region's radiance
  map with the diffraction kernel per band — softboxes and windows get correct soft glare
  for one FFT pair each.
- **D3** — a brute-force Monte Carlo mode as the *oracle* (Animal Logic's method,
  [DigiPro 2019](https://animallogic.com/wp-content/uploads/2023/06/Physical-Based-Lens-Flare-Rendering.pdf)):
  joint importance sampling over paths and pupil cells. Tests assert the fast path
  matches; it is also the honest answer for occlusion (a lens hood clipping the source is
  not linear in the source).
- **D4** — source regions on real footage: log-luminance threshold → connected components
  → morphological close → area/flux floor → cap at 16 by flux → **track components frame
  to frame**. Keep the region's **radiance map**, not a centroid — the map is `S` above.
  Flux matters more than shape, and clipping destroys it: prefer HDR footage, else
  highlight reconstruction, else an explicit **per-region intensity multiplier** in the UI
  (the production answer). Never let a hallucinated highlight silently set flare
  amplitude — surface the number.

### C1. Energy-ranked four-bounce ghosts

`N(N−1)/2` two-bounce paths (351 at N=27) are what everyone traces; four-bounce paths
number ~10⁵. With modern coatings each carries ~10⁻⁵ of a two-bounce path — but the sun
is ~10⁵× a normal highlight, and a few four-bounce paths land as *tight, well-focused*
spots (the residuals a removal model would otherwise leave). With vintage glass (R ≈ 4%)
they are plainly visible. Enumerate on the CPU, 16² energy prepass, keep the top few
hundred by peak irradiance, render survivors at full grid — the ranked-path method
stray-light analysis already uses. Nearly free against 2 s.

### C2. Fresnel ringing on ghost edges, by fractional Fourier transform (hardest — last)

The starburst is Fraunhofer; **the ghosts are Fresnel**, each defocused by its own
amount. The fractional Fourier transform interpolates between identity and Fourier as a
function of defocus — one parameter gives both the hard aperture polygon and the
diffraction ringing everyone real-time drops. All 351 ghosts × 12 λ at 256² is ~2 s on
its own, so apply to the **top ~32 ghosts by energy** (~100 ms). *Ref:* Joo et al. 2016,
[CGF 35(4)](https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.12953).

### E. Calibration and invertibility — only if flare *removal* is genuinely a goal

The literature converges: **coatings and manufacturing deviations cannot be predicted,
only measured.** If removal is the goal, what matters is not more wavelengths but:
(1) differentiability of the forward model — removal is *fitting*; (2) correct handling
of the aperture occlusion discontinuity; (3) Fresnel throughput, not just geometry;
(4) element spacings as fitted parameters; (5) strict scene-referred linearity;
(6) a **per-lens calibration workflow** — photograph a point source across field angles
and f-stops, fit spacings and coating stacks. The acquaintance's generator is almost
certainly forward model *plus* per-lens calibration, then fit-and-subtract.

### Build order for the remainder

A2 → B2 → B1 → D (with D1–D4) → C1 → C2 → E(if wanted).

### What not to build (unchanged, so it is not re-proposed)

**Paraxial/polynomial-optics lens models** (Lee & Eisemann 2013, Hullin 2012, Bodonyi
2025) — speed devices that cost accuracy and buy nothing at 2 s/frame. **Precomputed
flare-field interpolation across source positions** — Hullin's team tried and reported
failure. **A global source ⊛ PSF convolution** — measurably wrong (Talvala 2007).
**Temporal history buffers** — they break the determinism the caches are named on.
**ML relighting** — non-deterministic, wrong weight class. **Representative-point
sphere/tube lights** — the rect case is covered; add spheres only if a use case shows up.

---

## Recorded upgrade paths — not scheduled, but not lost

Each is written into its decision entry so it survives this file's deletion:

- **LTC specular/roughness** on the lighting pass (K-361): drop the fitted matrix fetch
  in ahead of the existing integral. Wanted only when layers have something to be glossy
  with (normals), which is its own project.
- **Layer culling for the region of interest** (K-362): skip layers whose placement
  misses the region — the saving the current window does not make. Dangerous half-done
  (an adjustment layer's blur can pull off-region layers in), which is why it waits.
- **Nuke-Relight-style per-pixel normals** (K-361's stated ceiling): content-dependent
  quality cliff; explicitly out of scope for the flat-plane pass that shipped.

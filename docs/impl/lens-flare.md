# Lens flare — traced ghosts and Fourier starburst

**Status: authoritative implementation note** for the Lens flare effect
([08-EFFECTS.md](../08-EFFECTS.md) §3.27; K-256..K-266). Specs say *what*; this note is
the *how*: the optical model, the exact formulas, the GPU pass structure, and the test
plan. Sources: the FlareSim renderer (github.com/SeanBRVFX/FlareSim_Nuke_builded, itself
built on space55/blackhole-rt) for the optical model and the lens-file collection — its
model is reimplemented here from understanding, not translated; *Physically-Based
Real-Time Lens Flare Rendering* [Hullin et al. 2011] for the pupil-grid energy method the
renderer keeps (splatted per ray since K-366 rather than rasterised as quads); *Temporal Glare* [Ritschel et al. 2009] for the starburst maths.

**In plain terms.** A camera lens is a stack of curved glass discs with an iris somewhere
in the middle. Most light goes straight through to the sensor — that is the picture. A
tiny fraction reflects off the *inside* of a glass surface, bounces backward, reflects off
another surface, and lands on the sensor anyway: that faint doubly-reflected image is one
**ghost**, and a lens with 20 surfaces has dozens of such two-bounce pairs — the train of
coloured blobs you see when a bright light is in shot. The **starburst** is different
physics: light diffracting around the iris blades, which is why its spikes match the blade
count. This effect simulates both — the ghosts by refracting a grid of rays through a real
lens prescription each frame, the starburst by a Fourier transform of the iris shape,
baked once. Nothing is a drawn sprite; every shape falls out of the physics.

---

## 1. The lens prescription (K-261)

Lenses are plain-text **.lens files** (the FlareSim / PhotonsToPhotos Optical Bench
format): metadata lines (`name:`, `focal_length:`), then `surfaces:` rows of

```
radius  thickness  ior  abbe  semi_ap  coating
```

front to back — signed sphere radius in mm (`0`/`inf` flat, `stop` marks the aperture
stop), axial gap to the next surface, refractive index and Abbe number of the medium
AFTER the surface (`1.0 0.0` = air), clear semi-diameter, and the AR-coating layer count
(0 bare glass, 1 single-layer MgF₂, 2+ multicoat). The last thickness is the back-focal
distance: the running z sum is the sensor plane. **Twenty curated prescriptions are embedded** in
`lumit-core` (`lens_files/` + the generated `fx/lens_library.rs`) — K-264, down from
K-261's 1299 (a thousand-entry picker is a search problem, not a choice), re-verified
K-265: every entry must bake a live ghost train AND keep flaring at a three-position
light probe (centre, off-centre, far corner), because the first cut was judged from a
centred montage and shipped lenses that rendered nothing in the owner's hands. The
twenty span maximally different characters — multicoated cine glass, 1930s uncoated
exotics, a Tessar, f0.95/f1.0 superspeeds, process glass, a pro telezoom, long
telephotos; no wide-angles or fisheyes (§4's acceptance limit). Transcribed patent data — each file cites its patent — sorted by name; the
native f-number is parsed from the collection filename (estimated from
`focal / (2·front semi-aperture)` when absent). The **`lens_file` parameter** (K-264,
the LUT File pattern) overrides the pick with a user's own `.lens` file: lumit-render
reads and content-hashes it per frame (`lens_text_hash` into `bake_key_with`, so an
edit takes effect next frame and never collides in the bake caches) and hands the text
to `bake_with`; unset, missing or unparsable degrades to the picked lens.
`parse_lens` (no panics; malformed rows skipped, files under 3 surfaces rejected) turns
one into the flat `FlareSurface` table the trace consumes, with the (n_d, V) pair
pre-fitted to a two-term Cauchy model:

```
B = (n_d − 1) / (V · (1/λ_F² − 1/λ_C²))      λ_F = 486.13 nm, λ_C = 656.27 nm
A = n_d − B/λ_d²                              λ_d = 587.56 nm
n(λ) = A + B/λ²
```

This reproduces n_d and the Abbe number exactly, which is all the flare can see.

**Reflectance.** Bare glass is unpolarised Fresnel by incidence cosine. A coated surface
is a real multi-layer stack solved by the **characteristic transfer matrix** (K-356,
superseding the single-layer-times-a-quarter approximation): per layer
`δ = 2π n d cos θ / λ` and `η = n cos θ` (s) or `n / cos θ` (p), matrices
`[[cos δ, i sin δ/η], [i η sin δ, cos δ]]` chained and closed on the substrate for
`Y = C/B`, `r = (η₀ − Y)/(η₀ + Y)`, both polarisations meaned. The Coating dial blends
bare → coated per surface, so 0 is a vintage uncoated look and 1 the prescription's own
character.

**Why it is worth the matrix.** `δ` carries a `cos θ`, so the reflectance band shifts blue
as the angle of incidence rises — and flare rays strike interfaces at large, varied angles.
That is the observed effect a scalar cannot express: **a ghost changes hue as its source
crosses the frame.** A single-layer coating has one reflectance minimum and can only tint
ghosts one way; a real multicoat has two or more, which is where the magenta/green/amber
character of a modern lens comes from.

A `.lens` file gives a layer *count*, never the recipe (real designs are trade secrets, and
the literature is unanimous that coatings can only be measured), so `coating_stack` maps the
count to that order's textbook design: MgF₂ quarter, V-coat, the classic broadband
quarter/half/quarter W, then alternating quarter-wave pairs. The shape is the point, not the
recipe; per-lens calibration (fitting spacings and coating stacks to photographed
flares) was considered and deliberately not scheduled — the owner dropped it with the
rest of the removal-oriented work when the accuracy programme closed (K-364..K-369).

One trap worth recording: **do not benchmark coatings on n ≈ 1.9 glass.** MgF₂ is very
nearly the ideal single layer there (1.38² = 1.904), so a single layer beats any stack at
the design wavelength — a coincidence of that glass, not a property of coatings. The oracle
compares on ordinary n = 1.5 crown.

## 2. Ghost pairs: enumeration, filter, ranking

Every pair `(a, b)` with `a < b` is a candidate — including pairs straddling the stop
(the FlareSim rule; the stop is air-to-air and cannot reflect, which the interface filter
below removes naturally). At bake time:

1. **Interface filter**: both surfaces must change medium by ≥ 0.001 in n_d.
2. **Brightness probe**: one on-axis centre ray per pair at 650/550/450 nm with the
   file's coating fully on; the mean surviving weight must reach `PAIR_MIN_INTENSITY`
   (1e-7) or the pair is dropped.
3. **Ranking**: descending probe brightness, ties by pair order — deterministic. The
   frame renders the first `max_ghosts`.

No per-pair area boost (FlareSim's `ghost_normalize`) exists here: the pupil-grid energy
term makes defocus dilution physical, so compensating it would double-count.

### 2.1 Four-bounce paths (K-368)

Light that reflects **four** times reaches the sensor too. Each such path keeps ~10⁻⁵ of
a two-bounce one under modern coatings, but the sun is ~10⁵ times a normal highlight and
some of these paths focus tightly instead of washing out; on vintage uncoated glass
(R ≈ 4% a surface) they are the chains of doubled ghosts old lenses are known for.

A path is stored as `[a, b, c, d]`. Slots 0 and 1 mean what they always did — reflect at
`b` going forward, at `a` coming back — so `a < b`, and a two-bounce path is
`[a, b, NO_BOUNCE, NO_BOUNCE]` (`NO_BOUNCE = u32::MAX`). A four-bounce path repeats the
figure: forward from `a` to `c` reflecting there, back to `d` reflecting there, then out.
So `a < c` and `d < c`; `c` may be either side of `b`, and `d` may equal `a`. All four
must pass the interface filter.

They cannot all be probed — there are ~N⁴/4 of them, over a hundred thousand on a normal
prescription — so they are **prefiltered by a cheap upper bound**: the product of the
four surfaces' reflectances at normal incidence and 550 nm (one number per surface,
computed once; `stack_reflectance`, or `fresnel_cos` where the file says the surface is
bare). It bounds the path because an AR stack is at its worst on-axis and every later
factor only removes light, and being a product of numbers under one it bounds *partially*
too — which prunes a whole `(a, b)` sub-tree the moment its pair reflectance cannot beat
the worst candidate kept. The best `FOUR_BOUNCE_PROBE_CAP` = 1500 survive, chosen by a
bounded top-K heap keyed by (bound bits, tuple) so the kept set is deterministic rather
than order-of-arrival.

Those 1500 then face the **same** brightness probe and the same `PAIR_MIN_INTENSITY`
floor as the pairs, and the survivors are ranked **into one list with them** (descending
probe brightness, ties by tuple). Everything downstream — `MAX_RENDERED_PAIRS`, the
spread probes, the frame grid plan, the GPU combo table — consumes that list without
knowing the two kinds apart. The bound decides only what is *probed*; the probe decides
what renders.

Measured over the library, every two-bounce path outranks every four-bounce one, so what
matters is whether the pairs run out inside `MAX_RENDERED_PAIRS`: the 11-surface Biotar
has 45 pairs and renders over a hundred four-bounce ghosts, while the 24-surface Master
Prime has 252 pairs and renders none.

## 3. The per-frame trace (the FlareSim three-phase walk)

Rays launch from a **regular pupil grid**: `side²` corners over the pupil square, at
`z = front vertex − 20 mm`, all parallel to the light
direction `normalize(−x, −y, focal)` from the light's raster fraction (36 mm sensor
width, y up). The spray radius is the **entrance pupil** `focal / (2 · native f-number)
× 1.5` clamped to the front semi-aperture — spraying the whole front bezel instead
wastes most rays (the Master Prime's 63 mm bezel passes ~4% of a full-width spray), and
the ×1.5 margin keeps the ghost paths that accept rays the imaging pupil rejects.

Each corner carries an **iris mask weight**: the blade polygon's radial bound at the
corner's pupil angle, blended toward the unit circle by Roundness (plus the K-260
wide-open blend — at the native stop the iris retracts behind the circular bore),
feathered by Softness. **Zero-mask corners still trace** (K-264): their weight is zero
but their geometry is real, so the cells they belong to draw and the iris edge fades
inside the cell — killing them killed whole cells and quantised every iris-shaped ghost
edge to the pupil grid. The same `pupil_mask` renders the aperture image the starburst
FFT consumes, so the two agree by construction.

Per (path × wavelength), the walk is FlareSim's three phases: **forward** through
surfaces `0..=b` (transmitting with weight × (1−R), reflecting at `b` with weight × R),
**backward** through `b−1..=a` (reflecting at `a`), **forward** again through `a+1..end`,
then a final propagation to the sensor plane. A four-bounce path (§2.1) ends phase 3 at
`c` and reflects there, then walks **backward** through `c−1..=d` reflecting at `d` and
**forward** through `d+1..end` — the same two phases again, with the same reversed-media
handling phase 2 uses. A two-bounce path's `c` is the sentinel, which no surface index
can equal, so its phase 3 runs to the end with its reflect flag always false: the walk it
had before K-368, statement for statement. Then the propagation (shifted by the K-260 thin-lens focus term
`f²/(1000·d − f)` mm). Intersection picks the sphere solution closest to the surface vertex. **A ray never
dies at an aperture (K-264)** — the K-261 skirt clip is gone, and the three ways a walk
used to end now all continue with their weight forced to zero instead, because a dead
ray killed every grid cell touching it and any hard boundary wore the pupil grid as a
staircase:

- **Beyond the clear aperture**: the housing feather (`smoothstep` on the worst
  relative crossing rrel, full inside 0.95, gone at 1.0) zeroes the weight; the ray
  keeps walking. The feather's denominator is `min(semi_aperture, |R|)` — a transcribed
  prescription can claim a clear aperture wider than the sphere it sits on, and the
  feather must reach zero before the miss can happen.
- **Missing the sphere entirely** (or finding it behind): the ray continues VIRTUALLY
  through the surface's vertex plane with rrel forced past the feather — physically the
  mount absorbs it. Virtual landings are real geometry with no light: the trace oracle
  pins positions only for rays carrying weight, and the spread probe ignores them.
- **Total internal reflection**: the transmitted energy is already ~0 (Fresnel reaches
  1 smoothly on approach), so the ray continues STRAIGHT, weight zero.

Cells that span from lit geometry to a distant virtual landing would fan faint lines
out of a ghost's bore (drawn) or notch its edge (dropped — both shipped, both
reported): instead each **unlit corner is pulled toward the lit corners' centroid** to
within one local cell-width (`√(min live neighbour area)`), so the fade to zero lands
where the boundary is. The working f-stop scales the stop surface's semi-aperture and
the pupil spray together by `native/f` (clamped 0.05..1).

**The grid side is per PAIR, not per frame (K-262, retuned K-265).** The Quality ladder
sets a base (32 / 64 / 96 / 144), the **Detail dial scales it and the wavelength count**
(`detail_base` / `detail_lambda`, 0.25–4, λ capped at 64 — the dial must buy both axes,
because spectral banding is untouched by rays alone); each pair's own grid is
`pair_grid(base, spread)` where `spread` is the pair's image extent as a fraction of the
sensor diagonal, measured by an 8×8 probe at bake: under 0.5 → base, under 1.5 → 1.75×,
else 2.5× (clamped 8..512). K-262's ½-base rung for tight blobs is gone (K-265): a small
ghost is not a cheap ghost — its caustic rim carries structure the blob-size probe cannot
see. A frame-filling defocused ghost is undersampled by a flat grid and shows its cell
facets — spending the budget by size is what lets Normal hold up.

**The frame-time grid probe (K-267).** The bake spread is a bounding box, and the box
does not grow at corner lights — what grows is the worst LOCAL stretch: measured, a
pair whose image stayed the same overall size stretched ~6× near a fold, and those
cells were the owner's choppy polyline edges. `frame_grid_needs` traces a 12×12
weight-gated probe grid per renderable pair at the ACTUAL light direction, takes the
worst adjacent-landing distance `d_max`, and — cell size shrinking inversely with the
grid side — derives the side that puts the worst cell under `FRAME_CELL_FRAC` (0.5% of
the sensor diagonal): `(G−1)·d_max/target + 1`. Uncapped, every fold demanded its
maximum at once and the frame septupled; `plan_frame_grids` BUDGETS the raise instead:
`FRAME_RAY_HEADROOM` (half again over the frame's rung-grid ray baseline) spent
worst-stretch-first, partial grants when it runs short, per-pair cap 3× the rung
(`boost_grid`), hard clamp 512, never below the rung floor. Deterministic: the sort is
by stretch ratio with rank-order ties, and the whole plan is computed once in
lumit-core — lumit-render runs it through the same seam callback as the lazy bake
(`FlareProbeBake`) and hands the GPU the FINAL per-pair grids, so the CPU reference
and the dispatch cannot disagree about a single ray. Manual mode only: Matte lights
exist GPU-side, so both twins keep the bake grids there and parity holds.

**The padded flare buffer (K-267).** A Squeeze or Scale below 1 makes the combine
sample PAST the base buffer; K-266's zero-outside tap honestly showed nothing there
("cuts to black at the edges"). `flare_pad_dims` (mirrored `flare_pad_dims_of`,
pinned) grows the render target up to 2× per axis — `1/(squeeze·scale)` wide,
`1/scale` tall, both clamped 1..2 — with the optics centred in it: the raster dims in
the trace uniform are the padded ones, the screen transform and the Ghost-blur radius
stay derived from the BASE dims, and the combine's tap gains only the constant border
offset `(padded − base)/2`, zero when unpadded. Past even the 2× cap the zero-outside
rule still holds (squeeze 0.5 is the parameter floor; the cap only truncates
degenerate combinations), pinned by the same regression test.

**The frame's dispatch plan (K-263).** The GPU sorts combos grid-major, so the table
falls into runs of one grid; `plan_batches` cuts each run into batches of combos and
chunks of lights, and **every batch strides the scratch by its own grid**. Through K-262
one stride served the whole frame — the widest grid in it — and the build pass ran over
that stride to park the cells a narrower batch did not fill, so a single frame-filling
ghost made every compact ghost dispatch and draw at *that* ghost's cell count. With a
per-batch stride the batch's cells are contiguous from zero and nothing outside them is
dispatched, written or drawn; the parking that stride demanded is gone with it. Three
bounds hold the plan:

- **`SCRATCH_BYTE_BUDGET` (48 MB) is hard.** K-262's batch size bottomed out at one
  combo and then let eight lights at an Ultra grid ask for ~100 MB anyway. The light
  dimension splits too now (`light_offset` in the trace uniform), so no setting can
  push the allocation past the budget. Lights are chunked *inside* the combo batch, which
  keeps the drawn order light-major within a batch exactly as it was.
- **`STEPS_PER_SUBMIT` (48 M ray–surface steps) cuts the frame into command buffers.**
  See §7's trap: an over-long submission does not cost a frame, it costs the device.
  The count covers the trace's ray–surface steps AND the deposit's pixels (K-379,
  `combo_deposit_cost`: nine times each pair's spread-squared image area, per combo per
  light — a cost independent of the ray count, and for defocused ghosts more than ten
  times the trace's). Until K-379 only the trace was metered, so a frame of
  frame-filling ghosts packed seconds of atomic scatter into one submission — the
  owner's machine froze whole-desktop for minutes and the device died. The same
  estimate also caps a BATCH's slot count (`plan_batches`), because a batch is the
  atomic unit of encoding and a flush cannot split one.
- **The scratch is pooled, not allocated per frame** (`Scratch`, one slot deep). A driver
  recycles a dropped buffer only when the submission it belonged to retires, so a
  continuously re-rendering Viewer used to hold a rolling backlog of abandoned
  tens-of-megabyte buffers.

## 4. Splatting the ghosts (per-ray, energy-conserving — K-366)

**Each ray deposits on its own; rays are never joined.** Through K-353 the renderer built
quads out of neighbouring landings and drew those, which is a fair model of a smooth map
and a wrong one at a **caustic fold** — where the map folds back on itself, so a quad
across it spans geometry that is not one patch. Every rescue K-261..K-264 added
(sub-pixel inflation, sliver parking, the unlit-corner pull-in, vertex-smoothed density)
and K-353's pixel widening with its analytic four-sample coverage existed to survive that
join, and each moved the artefact rather than removing it. Splatting removes the join.

One ray's deposit is decided in three steps, spelled identically in
`lumit_core::fx::lens_flare` and in `fx_lens_flare_trace.wgsl`:

1. **The footprint** (`ray_axes`): the image of the ray's pupil cell under this ghost's
   map, as a 2×2 Jacobian over the four neighbouring rays' landings — per axis, the
   **longer of the two one-sided differences** (K-378; K-366 averaged them, and under an
   area source's jitter the average cancelled toward zero wherever both neighbours
   hopped to the same side, a collapsed splat between two wide gaps that stamped a
   quasi-periodic mesh across every ghost; on a smooth map the two sides agree and this
   is the central difference it was). One-sided at the grid edge or beside a dead ray, a
   right-angle borrow at the anti-alias floor when one axis has no live neighbour at
   all, the floor in both directions for a lone survivor. Half-axes, so the
   parallelogram `centre ± a1 ± a2` tiles the pupil grid exactly once — over-covering,
   never under-covering, where the two gaps disagree.
2. **The peak** (`splat_ray` up to the divide): flux is `weight × gain × cell_area_px`
   times the ray's band-integrated rgb times the light's colour; the divisor is the
   footprint's area, floored by the density cap. On the GPU this whole step is
   `build_splats`, one thread per RAY, writing `{centre, a1, a2, peak rgb, live}` —
   48 bytes.
3. **The deposit**: a separable quadratic B-spline (K-376, §5a-bis) over the
   parallelogram, `(u, v)` from the inverse 2×2. The kernel integrates to what the peak
   was divided by — so **flux is conserved exactly**, and a fold is simply several
   splats landing on top of one another, which is the correct integral.

The two guards, and nothing else:

- **Caustic density cap (K-262, kept unchanged)**: the divisor is floored at
  `MIN_AREA_FRAC` = 3e-3 of the launch cell, so density tops out near 333×. At a fold the
  density genuinely diverges but its *integral* over a pixel is finite; a discrete ray
  concentrates that whole divergence into a few pixels, and an uncapped divisor drew hard
  chromatic lines through the ghosts.
- **Anti-alias floor (`MIN_SPLAT_AXIS_PX` = 0.75 px)**: a footprint axis shorter than
  that is scaled up to it, direction preserved, so a caustic line is a line rather than a
  row of dropped sub-pixel points. The fold case — both axes long but nearly parallel —
  is caught by `|det| < MIN_SPLAT_AXIS_PX × |a1|` and pushes `a2` across `a1` up to the
  same floor, which is what stops an edge-on fold's flux vanishing into a zero-area
  parallelogram. This is the job `MIN_QUAD_PX`'s inflation used to do, without the sliver
  cases that came with connecting rays.

**Pass structure (K-366).** `trace` then `build_splats`, both dispatched over the batch's
rays — two passes, because the splat stage reads its neighbours' landings and a neighbour
traced in another workgroup needs a pass boundary to be visible. (`quad_area` and
`build_verts` are gone with the quads.) The draw is **one instanced six-vertex quad per
splat**: vertex `k` maps to `(u, v) ∈ {−1, 1}²`, position is `centre + a1·u + a2·v` mapped
to clip space through the raster-dims uniform, and the fragment evaluates the tent and
adds. A dead or unlit ray still occupies its slot — the batch's splats are one contiguous
instance range — and draws as a degenerate off-screen quad.

**The target is single-sampled and needs no coverage logic** (K-353's measurement stands,
K-366 makes its machinery unnecessary). A tent falls to zero at the quad's own edge, so
there is no silhouette to antialias: no widening, no `dpdx` barycentric reconstruction, no
four-sample test. Bit-stability follows by construction — fixed instance order, additive
blend, one sample. Additively blending fp16 into a 4× *multisample* target is what was not
reproducible run to run on this hardware, and that — not the draw order, not the bake, not
the pools — is what failed the §2.4 assertion for months; the ~66 MB pooled 4-sample
texture (K-265) and its resolve went with it.

The combine's flare tap is ZERO outside the buffer (K-266): squeeze or scale
below 1 asks for coordinates past it, and clamp-addressing repeated the edge
row outward as a smear.

The additive raster (hardware, one-one blend, fp16 buffer; Draft at half resolution) is
followed by the **Ghost blur**: 3 separable box passes (≈ Gaussian) at a radius of
`Ghost softness × 0.01 × frame diagonal` — FlareSim's Ghost Blur, a touch of
out-of-focus softness that also hides the point-splat grain at low qualities.

The blur radius is capped at 80 px (K-262) and the sum runs through a **workgroup line
cache** (K-263): a workgroup covers 64 consecutive pixels along the blur axis, so the
64 + 2r texels they need between them are fetched once into shared memory and every
thread sums out of that — about 3.5 fetches a pixel where the direct loop's worst case
was 161, across six passes. The summation order is unchanged, so the result is bit-for-bit
what the naive loop produced. The dispatch shape changes with it: x runs *along* the blur
axis in tiles of 64 and y across it, so the vertical pass dispatches `(⌈h/64⌉, w, 1)`.

**Known limits** (both wait on adaptive refinement at folds, the recorded follow-up):

- a lens whose ghosts are ALL extreme frame-filling defocus (some process lenses)
  still resolves one pupil cell to several pixels — an 8-diagonal wash samples mostly
  off-frame, which is why the Projection Optics prescription left the bundle (K-265);
  what remains on bundled lenses is a mild ripple on hard vignetted edges at Normal
  that Ultra resolves.
- a ghost rendered in an extrapolated regime (an f2.8 zoom shot at f1.5) can wear a
  toothed corona at its fold. K-265 ablated grid density (72→288), wavelength count
  (32→64), the pull-in reach, sub-sample inflation, a local branch-jump cull and a 3×
  feather, one at a time — the corona is invariant to all of them. It is the fold's
  own discontinuous structure; do not re-chase it with guards (the decision log holds
  the full list).
- wide-angle and fisheye prescriptions flare only near a centred light: the angular
  acceptance of the three-phase walk collapses off-axis for retrofocus designs. None
  are bundled (K-265's three-position probe is the curation gate); a user file loads
  them with the limit understood.

## 5. The bake (CPU, cached by parameter hash)

Pure and deterministic: parse the prescription, filter and rank the pairs (§2), render
the aperture image from `pupil_mask` and bake the **starburst sprite** — the aperture's
Fourier amplitude under the Fresnel propagation term at λ_mid, spectrally integrated
(100 samples, sample position scaled by λ_mid/λ so diffraction grows with wavelength)
with CIE weights into linear RGB, peak-normalised. Amplitude |F|, not power |F|² — the
power spectrum's DC core buries the blade spikes. The sprite then fades RADIALLY to
zero from r 0.7 to its edge (K-264): its diffraction pedestal ran to the texture
border, and the combine stamps it as a quad, so on a dark scene every starburst sat in
a hard-edged grey square. Radial, not square — light around a point falls off in
circles, and a square window merely softened the square.

**The sprite is baked `STARBURST_FIELDS` (8) times, at eight field angles** (K-365):
off-axis the diffracting hole is not the iris but the **cat's-eye** the front and rear
mechanical stops clip it to, so the starburst squashes and leans towards the frame
corner. Slice `f` renders at `θ = f/(F−1) · atan(half sensor diagonal / focal)`, its
aperture image the same `pupil_mask` polygon multiplied by the imaging path's vignette:
`trace_transmit` — the straight refract-only walk through every surface, no reflections
— accumulating the same `rrel2` housing feather the ghost trace does, at 550 nm. Two
things it deliberately does *not* do:

- **the stop surface contributes no feather.** The iris is already the polygon mask;
  counting it again would shrink every aperture image by its own edge;
- **the sampled disc is the entrance pupil, `focal / 2N`, not `FlareBaked::pupil_mm`.**
  That field is the pupil *spray* radius — the entrance pupil with half again as margin,
  because ghost paths accept rays the imaging pupil rejects. Traced at the wider radius
  the vignette clips the polygon into a circle at two thirds of its own edge, and every
  aperture image, on-axis included, comes out round.

The origin is back-projected to the front vertex plane, so a tilted slice samples the
same disc rather than a disc slid sideways by `START_Z_BACKOFF_MM · tan θ`. On-axis the
vignette is 1 across the whole pupil for every bundled prescription, so **slice 0 is
bit-identical to the pre-K-365 sprite** — a test pins that the blade spikes still count
(even N → N spikes, odd N → 2N). A lens that does not cover the full frame (the bundled
7Artisans is an APS-C design) passes nothing at the outer angles and would bake a black
sprite; a dead slice holds the last live one instead, so the starburst stops changing
rather than vanishing.

The eight slices are independent FFTs and bake in parallel, `collect`ed back into slice
order so the thread pool cannot reach the pixels; the whole bake costs roughly 1.3× its
one-slice self. They are stored concatenated slice-major in `FlareBaked::starburst` and
uploaded as ONE atlas texture, `STARBURST_RES` wide by `STARBURST_RES × F` tall, which
keeps `sb_tex` a plain `texture_2d`. The combine (§8) computes each light's field
fraction and azimuth with `starburst_field`, rotates the sprite-relative pixel by
−azimuth, and lerps the two bracketing slices; taps are offset by the slice's own rows,
so no slice bleeds into its neighbour. The shader spells `STARBURST_FIELDS` itself and
a test pins the two spellings together.

### 5a-bis. The splat reconstruction must partition unity (K-366, fixed K-373)

`ray_axes` returns **half**-axes: the step between neighbouring rays is `2·a1`. K-366's tent
had a support of `±a1` — half a step — so neighbouring tents met exactly where both were zero.
A linear B-spline partitions unity only at a support of *twice* the sample spacing, so the sum
over the grid was a lattice of separate pyramids with a seam of zero along every cell boundary:
a woven grid of dark lines at the ray spacing, ridges along the pupil axes through each ghost,
and stepped rims. On a uniform sheet of identical rays the reconstruction ran 0.0029 … 0.1436
about an expected 0.0469 — a 49x ripple.

Flux was conserved the whole time (the tent integrates to the parallelogram's area either way),
which is exactly why the suite passed: it measured how much light there was and never whether
it was smooth. `lens_flare_splats_reconstruct_a_flat_sheet_and_keep_their_flux` now measures
both.

The kernel reaches `±2·a1` (K-373) and is the **quadratic B-spline** rather than a tent
(K-376): a tent is only C0, and the crease at every cell boundary is itself visible on a real,
non-uniform ghost — 2.42% residual against the quadratic's 1.91% in the lit part of the frame,
4.59% against 3.81% in the faint part. Its support is 1.5 steps, so `±3·a1`. Both partition
unity; the integral grows by four either way, so the peak is divided by four and the
flux is unchanged. `area` and the density cap keep their K-366 meaning in half-axis units. The
GPU quad doubles; the fragment tent is untouched, because its `uv` still runs `±1` at the
quad's corners — those corners now sit on the next ray along. Cost: four times the fragments
per splat, on the hottest raster in the effect.

### 5a-ter. The deposit accumulates in f32 (K-366, fixed K-375)

The splat deposit was a raster pass blending additively into `flare_tex`, which is
`WORKING_FORMAT` = `Rgba16Float`. Adding a small increment to a large fp16 running sum drops
anything under half an ULP of the sum — systematically, and more the brighter the pixel. On
the padded-anamorphic oracle that read as the middle of the frame 4.5% dim, the border ring
0.7%, the outer fifths 0.1%, growing with the contributions per pixel.

`fx_lens_flare_deposit.wgsl` replaces it with two compute entry points. `deposit` scatters each
splat into an f32 accumulator (three channels a pixel, pooled in `Scratch`, cleared per frame)
in **fixed point** at 2^18 steps per unit of radiance. WGSL has no float atomics; a
compare-and-swap loop on the bit pattern is exact per add but order-dependent, and float
addition is not associative, so it broke bit-stability (K-353) the moment CI ran it. Integer
`atomicAdd` is associative, so the sum does not depend on thread order at all, and the
rounding it does do is unbiased at 3.8e-6. Above 16383.99 a channel wraps rather than
saturating — a test measures the reference's brightest pixel at 100x under that. `resolve`
writes the finished sums into the fp16 texture once. Everything downstream is untouched: a single stored value always had precision to spare.

`deposit` is an op-for-op twin of `splat_ray` — same bbox, same inverse 2×2, same tent, same
order — which the raster could not be, because its pixel selection was the rasteriser's fill
rule rather than the reference's `|u| < 2`.

The two options not taken: `Rgba32Float` blending needs `FLOAT32_BLENDABLE`, which is not
universally available and would make the picture machine-dependent (K-353); and restating the
oracle's bound would have recorded the loss rather than fixed it.

### 5a-quater. Big splats deposit into a pyramid (K-380)

A splat's deposit cost is its kernel's pixel count, and the kernel spans three grid steps —
so a frame's deposit costs about **nine times each ghost's image area, per combo per light,
whatever the ray count**. For defocused ghosts that is more than ten times the trace, and it
is what made Normal slow and Ultra a machine-freezer (K-379 bounds the submissions; this
bounds the work).

The accumulator is therefore a **level pyramid**: level 0 is the flare buffer, each level
below it ceil-halves both axes (`deposit_levels`, mirrored `deposit_levels_of`, pinned;
stops at 32 px or `MAX_DEPOSIT_LEVELS`; about 1.33× level 0's pixels in total). A splat
whose kernel span exceeds `DEPOSIT_SPAN_PX` (48) deposits at the shallowest level that
brings it under — the level pick is repeated exact halving, identical in both twins — with
centre, axes, det and bbox scaled into level pixels. Nothing else changes: the peak is a
density per level-0 pixel (a density survives resampling), the floors, fold guard and
density cap all ran at level 0 in `build_splats`/`splat_ray` before the level was chosen,
and the kernel code is untouched because `(u, v)` solve the same system when both sides
carry the scale. `resolve` bilinearly upsamples every level and sums; level 0's tap is the
identity, so a frame whose splats all fit level 0 reads back exactly as before the pyramid.
One splat now costs at most ~`DEPOSIT_SPAN_PX`² pixels, and the smoothing the coarse levels
cost is ~1/24 of the splat's own size — invisible on the defocused ghosts big enough to
take it. Measured on the frame-cost harness at 960×540 Normal/60 ghosts: **1.15 s → 87 ms**.

Two FXC (DX12) traps the shader works around, for the record: a uniform-buffer array
cannot be dynamically indexed without FXC unrolling every loop that touches it (it
refuses), so the level dims are not passed as a table — the shader derives them as
`ceil(raster / 2^level)`, which iterated ceil-halving provably equals; and a local vector
cannot be indexed as an l-value, so the resolve taps whole `vec3`s rather than looping
channels.

### 5b. Ghost edges are Fresnel: the knife-edge rim (K-369, re-derived K-370)

The starburst above is the aperture's **far** field. A ghost is its **near** field. Each
ghost image is defocused by its own amount, and a defocused aperture edge is not a clean
cut: it carries diffraction fringes just inside the rim, brighter than the middle of the
ghost, at a scale set by that ghost's defocus.

**How fine those fringes are is derivable, and deriving it is the whole of K-370.** The
ghost patch *is* the defocused aperture, so its radius on the sensor is `a`; the cone that
forms it leaves the pupil at the marginal-ray angle, which the working f-number fixes at
`1/(2N)`, so the defocus is `z ≈ 2Na`. One power of `a` cancels out of `F = a²/(λz)`:

```text
F = a / (2Nλ)
```

`ghost_fresnel_number(spread, fstop)` is that line, with `spread` the bake's measured image
diameter as a fraction of the sensor diagonal and the working stop's `stop_scale` already
folded in — the number moves with the iris, because stopping down shrinks the ghost and the
pupil together (`F ∝ stop_scale²`), so it is computed **per frame** rather than baked.

Real values: a 5%-of-frame ghost at f/2.8 is `F ≈ 350`, a frame-filling one `F ≈ 7000`, the
widest washes on the bundled lenses `F ≈ 50 000`. **That range is why the propagated ring
masks K-369 shipped had to go**: a single-FFT propagator's output window is `±(N−1)/(4F)`
aperture units and must cover the aperture, so `F ≤ (N−1)/3` — 85 at a 256² transform, and a
4096² one to reach 1000. K-369's ladder therefore ran `F = 64 … 2`, two to three orders low,
and its spread calibration spread real ghosts across rungs where the near field is not an
edge effect at all but a whole-aperture pattern: measured on the bundled default, the `F 2`
slice's interior ran 2.4× the flat mask on average and 4.7× at the very centre. Painted
across ghosts that fill the frame, that is a broad concentric interference pattern over the
whole picture — which is what it looked like, and what the owner reported.

At the real Fresnel numbers the blade is locally straight and the fringes hug the rim, so
the model is the **knife-edge asymptotic**, a closed form of one variable:

```text
I(v) = ½[(C(v) + ½)² + (S(v) + ½)²],   v = s·√(2F)
```

`C` and `S` are the Fresnel integrals in the `π/2` convention (`fresnel_cs`, by the standard
auxiliary-function rational approximation, error under 2e-3, odd in `v` so one evaluation
serves both sides of the edge). The profile is 1 deep inside, exactly ¼ on the geometric
edge, peaks at ≈ 1.37 at `v ≈ 1.22` with a fringe train decaying as `2/(πv)`, and falls to
nothing outside — the light real diffraction throws past the blade, which the propagated bake
had to normalise away instead. `s` is the **perpendicular** distance to the blade: the same
polygon `bound` `pupil_mask` computes, less `r`, times `cos α` (exact for the polygon, 1 for
a fully round iris).

Two properties are load-bearing, and both are pinned by test:

- **the interior of a ghost is flat by construction.** Whatever the rim does, this profile
  cannot shade or tint the middle of a ghost. That is the regression, stated as a property
  rather than a tolerance;
- **fringes nobody can sample are averaged, not drawn.** A fringe train finer than the pupil
  ray grid does not appear, it *aliases*, and an aliased train is a beat pattern smeared over
  the whole ghost — the other half of the artefact. A diffraction profile averages to the
  geometric edge it surrounds, so `ghost_mask` crosses from the ringed profile to the plain
  one over `blur_v` of 0.5 … 2, `blur_v` being the wider of the ray-grid step and the
  Softness feather in `v` units. A soft blade smears its own fringes exactly as a coarse grid
  does, so both enter the same way.

The practical consequence, stated plainly: **the big frame-filling ghosts now show
essentially the plain iris edge** — their fringes are far finer than any grid the effect
traces — and the tight bright ones, where a photograph does show a ringed rim, keep theirs.
There is no per-path budget any more: the closed form costs what the polygon it replaces
costs, so every ranked path carries its own `F` in the combo (a `f32` in the padding slot
K-369's `i32` slice index used), and the trace WGSL mirrors `fresnel_cs`,
`knife_edge_intensity` and `ghost_mask` op for op.

Recorded limits: the fringes are computed at one wavelength (`RING_LAMBDA_UM` = 0.55 µm —
their spacing goes as `√λ`, so ±15% across the visible band, far under the blur they are
already averaged by), and they are uniform round the rim, which is right along a blade and
wrong at a corner where two edges' diffraction would add. Both are the recorded upgrade path.

### 5c. Coatings are chosen per glass element (K-356, extended K-371)

K-356 made a surface's reflectance a real multi-layer stack by transfer matrix. K-371 makes
the *choice of stack* the user's, per element.

**Elements from surfaces.** `surface_elements` walks the prescription: a row whose medium is
glass opens an element, the row after closes it, numbering front to back. A cemented pair's
shared surface goes to the earlier element — it has cement on it, not air, so it carries no
AR coating in reality. `element_count` is the total, 4 (Tessar) to 18 (Canon 70-200) across
the bundled library.

**The palette** (`coating_design`) is seven entries: "As the lens file" (the default, and
byte-for-byte the pre-K-371 picture), uncoated, and five real designs whose residual
reflection was **measured** across 420–680 nm and kept only where it is both distinctly
coloured and dimmer than bare glass. They read as straw, magenta, green, amber and blue —
which is the mechanism behind a blue ghost sitting next to an amber one. The stacks are
written out rather than taken from `coating_stack`'s layer ladder: that ladder's 2-, 4- and
6-layer rungs measure 0.06 to 0.31 peak against bare glass's 0.04, so they are brighter than
no coating at all. Nothing exercises them (every bundled column is 0 or 1) and the palette
avoids them; correcting the ladder is its own change.

**It is a bake input.** `apply_element_coatings` stamps each surface's resolved design into
the row's former padding slot before `bake_reflectance` reads it, and `bake_key` folds the
choices in — so a coating change rebakes exactly as a lens change does, and the WGSL mirror's
stride is unchanged. The shader only ever reads the baked table and needs no change.

**The row count follows the lens** without teaching the frontend any optics. Twenty rows are
declared — the ceiling — each its own single-member group carrying
`visible_when_lens_elements`. The bridge turns that threshold into the visibility rule the
panel already has: sibling `lens`, values being the library indices with enough elements.
Recorded limit: a user `.lens` file overrides the dropdown, so the rows offered then follow
the picked lens; an element with no row keeps the file's own coating.

The **auto-exposure gain** closes the loop (K-258): the bake renders the CPU reference
at thumbnail size (96×54, fixed frame-time settings so only bake-key inputs steer it)
with gain 1 and normalises the mean to `TARGET_PROBE_MEAN` (0.010). The gain ceiling is
**64** (K-261): a wash-only lens has almost no probe energy, and an unbounded loop would
amplify the residue into a lit-up artefact field — capped, such a lens renders honestly
dim, which is what that glass does. The bake key is §5d.

**The bake costs about 0.66 s** for a 24-surface prescription on a middling CPU, of which
the exposure probe's trace is roughly 0.5 s and the starburst 0.12 s (K-365's eight field
slices go wide across the pool and cost about 1.3x that one, not eight times it) — the rest is pair
ranking. It used to spend that on the render thread, so choosing a lens froze the picture;
**it now runs on a bake thread beside the frame (K-350)** — see §5a. Three K-263 economies,
all exact, cut it to that figure:

- spreads are measured **after** the ranking and only for the first `MAX_RENDERED_PAIRS`
  (200, the Max ghosts ceiling), not for every surviving pair — a 60-surface prescription
  leaves well over a thousand, and a frame can never reach them. Each pair probes TWO
  directions (K-264) — on-axis and a representative off-axis beam — and takes the larger
  spread: some designs land a compact on-axis ghost that fills the frame off-centre, and
  the on-axis-only probe handed those the half grid (9 px staircase blocks on their
  wash). Zero-weight virtual continuations are excluded from the measured extent;
- the starburst's spectral ladder (chromatic scale and CIE weights per sample) is built
  once instead of inside the per-texel loop, which was interpolating the CIE table 6.5
  million times to produce a hundred distinct answers;
- `cpu_flare` computes the iris mask once per pair rather than once per pair *per
  wavelength* — it is the shape of the hole, not a function of colour, and it costs an
  `atan2` and a `cos` a corner.

What remains is the trace itself, near the arithmetic floor for scalar code — which is why
the fix was never a faster bake but a bake that does not block.

### 5d. The bake key, and what stays per-frame (K-425)

**The bake key holds the lens and the iris, and nothing else.** `bake_key_with` hashes the
library pick, the `lens_file` override's **content** hash, the per-element coatings
(K-371), the blade count, and the four continuous iris dials — working f-stop, aperture
rotation, Roundness, Iris softness. Light position, intensities, dispersion, the Coating
dial, Ghost softness, focus, quality, Detail, Source size and Mix are **frame-time**:
animating any of them never rebakes.

**The four continuous iris dials are snapped first** (`bake_params`), and the same snapped
values are what `bake_with` reads — the key and the bake it names cannot disagree, which is
the property the 24-entry cache rests on. The steps: the f-number to a **twentieth of a
stop** (`FSTOP_BAKE_STEP_STOPS`, applied in stops rather than f-numbers, so it is the same
proportional step wide open and at f/22), aperture rotation to **half a degree**, Roundness
and Iris softness to **1/256**.

Why at all: an animated aperture would otherwise want its own bake on every single frame,
none would arrive in time, and no frame would ever be worth keeping — the flare's
parameters would have made the whole project uncacheable. Snapped, a half-stop ramp needs
about ten bakes and the cache holds them.

Why it is safe: **nothing the frame computes is snapped.** `fstop_scale`, the iris mask the
trace weights every pupil corner by, `effective_roundness`'s wide-open blend and
`ghost_fresnel_number` all read the raw dial, so the ghosts shrink, turn and re-fringe
continuously. Only two things come out of the bake with an iris in them — the starburst
sprite and the auto-exposure gain — and those step, by about 1.7% a step.

**Why the aperture is not simply moved out of the bake**, which is the design that would
need no steps at all: it cannot be. The sprite is the aperture's Fourier amplitude
(§5, eight field slices, ~0.12 s) and the gain is a thumbnail render of the whole flare
(~0.5 s). Both are precomputations of the iris by their nature, not shaping that could be
applied per frame; a per-frame FFT would spend the effect's entire budget on the starburst
alone. The snapped key is the second-best answer and is recorded as such.

**A provisional frame is named, and then checked** (K-425, superseding K-350's rule). A
frame that fell back to the previous lens is still a frame nobody may bank — the tiers are
keyed by what is *in* a frame (K-178) — but that is now decided by *counting the
fallbacks*, not by "is any bake in flight?". `FxEngine::flare_substitutions` bumps at the
one place `baked()` hands back other optics than the key names; callers read it either side
of a render and drop the name only if it moved. `frame_key` names every frame. The old rule
took the whole project down with one animated dial.

**A file parameter's key covers the file itself** (K-425): `lumit-eval` folds a `.lens` (or
`.cube`) path's size and last-modified time into the frame key beside the path string. The
bake keys on the file's content, so path-alone let an edited prescription draw different
optics under the old file's name — an entry nothing could clear. Not the bytes: a LUT is
megabytes and the key is computed per frame. A file rewritten inside one filesystem tick at
exactly the same length is the recorded limit.

### 5a. The bake runs beside the frame (K-350)

`LensFlareFx` owns a **bake thread**. A frame that asks for a lens the engine does not hold
hands the bake to it and draws **the lens the previous frame drew** — or, with none yet, no
flare at all — and the frame after the bake lands draws the new one. The upload stays on
the render thread (the only thread with the device): finished bakes are collected at the
top of the next `baked()` call.

Four invariants, and none of them is optional:

- **Exact by default.** Deferring is a per-engine switch that is *off* until something
  turns it on, and only the Viewer's renderer does. The exporter builds its own renderer on
  its own device, so an export bakes inside the frame exactly as it always did and stays
  bit-for-bit what it was (K-031 preview-equals-export is untouched).
- **A provisional frame is not banked** (K-425, replacing "is unnameable"). A frame drawn
  with the previous lens and filed under the new lens's name is an entry that lies about
  its own content, and the tiers are keyed by content (K-178) — nothing later would ever
  clear it. What changed is how that frame is spotted: `FxEngine::flare_substitutions`
  counts the frames that actually fell back, read either side of a render because a fallback
  happens *during* one, and the name is dropped only for those. `frame_key` names every
  frame. It used to answer `None` for every comp while any bake was in flight, which an
  animated aperture turned into "the project does not cache" — see §5d.
- **Cancellation is by supersession and it is exact.** The bake key is a hash of the
  parameters, so a key nothing is asking for any more is a lens the user has moved past.
  The thread takes everything queued behind a job before starting, keeps only the newest,
  and answers the rest with *nothing* — which is what takes them off the in-flight list, so
  a lens abandoned mid-drag does not leave every frame permanently unnameable.
- **The bake is still pure.** Same key, same bake, wherever it runs; the frame that finally
  shows the new lens is the frame the blocking version would have drawn. Pinned by
  `lens_flare_a_deferred_bake_is_the_same_bake` (including the auto-exposure gain bit-for-
  bit) and `lens_flare_deferred_bakes_answer_with_the_previous_lens_then_the_new_one`.

The GPU side caches uploaded bakes by key, **evicting oldest-first at 24** (K-263).
Emptying the map at the cap, as K-262 did, made trying lenses quadratic: every ninth pick
threw away the eight bakes just paid for, so stepping back to a lens seen a moment ago
paid the half-second again.

**Wavelengths**: the ladder spreads `lambda_count` bands (3/8/16/32 by Quality) about
the 550 nm midpoint, scaled by the Dispersion dial; each band's RGB weight is the CIE
1931 integral over its band (2 nm steps), Y-normalised so the band count never changes
exposure. Point-sampling instead of integrating tints everything blue-green (found by
eye) — deviation D5, kept from K-256.

## 6. Matte source mode (shipped, K-257)

Shipped in the K-257 pass as the **Matte** source mode (docs/08 §3.27): the flare
sources itself from a referenced layer's picture. A compute reduction tiles the matte
into a 32 px grid, then a single-thread pass picks the top-16 ANCHOR tiles by luma
(`MAX_SOURCES`, 8 → 16 in K-267) with a 2-tile Chebyshev non-max suppression, each gated
by the soft Threshold. **The gate is one-sided (K-363)**: closed at and below the
threshold, fully open a Softness above it. Threshold is the absolute scene-linear luma a
pixel must *exceed* — at 1.0 only over-range highlights flare, at 0.0 anything brighter
than black does, and black itself never flares. (The earlier symmetric gate opened from
`threshold − softness`, which let pure black through at half strength when the threshold
sat at zero.)

**Each tile carries the statistics of its whole lit area, not one pixel (K-355)** — the
maximum luma and its argmax (still how anchors are *ranked*, so a small bright source is
still found), plus `Σ gate`, `Σ colour·gate`, and `Σ luma·gate` with its first moments
`Σ x·luma·gate` / `Σ y·luma·gate`. This is what stopped flares **jumping** on footage:
representing a tile by its brightest pixel meant the light's position was decided by a
lottery that sensor noise and specular sparkle re-ran every frame, so a flare twitched
across a practical that had not moved. A source's position is now the flux centroid of
every lit pixel feeding it and its colour their mean, neither of which one pixel can
move; a 40× sparkle shifts a 64 px source by under a pixel (pinned by
`lens_flare_centres_an_area_source_on_its_light`). Point sources are untouched — a single
lit pixel is its own centroid and its own mean.

The per-tile sums are order-dependent where the old maximum was not, so the CPU (row order
within a tile) and the GPU (64 partials merged in thread order) agree to the matte oracle's
perceptual bound rather than op-for-op. Each is internally deterministic in fixed order,
which is the §2.4 property that matters.

**Area sources are integrated per ray, not replicated (K-355 measured them, K-367 renders
them).** Each anchor's second moments give its half-extent — the standard deviation of its
flux about its centre — and that half-extent travels with the light, one slot per source,
into the trace. The flare of an extended source *is* the integral of the point flares
across it, and the trace already integrates over the pupil, so the two integrals are done
together: the ray at pupil-grid (i, j) offsets its light position inside the ±extent
rectangle by `source_jitter` and computes its own direction from there, carrying the
light's **full** colour (no flux shares — the pupil grid averages). An area source
therefore costs exactly what a point source costs, whatever its size.

The stratification is `offset_u = tri((i + ½)·PHI_U + band·PHI_BAND)·ext_x`, `offset_v`
likewise in j with `PHI_V`, where `tri(x) = 2·|2·(fract(x) − ½)| − 1` is a triangle wave,
`PHI_U` is 1/ρ of the plastic constant, `PHI_V` is 1/ψ of the supergolden ratio (K-378),
and `PHI_BAND` is the golden 0.618. Four points about that, all load-bearing:

- `fract` alone would jump the whole range at each wrap, and the footprints are
  differences over exactly the neighbours a jump would separate by the width of the
  source — one splat inflated to the source's width stamps a bar across the ghost.
- One irrational for both axes would put every offset on a diagonal of the rectangle,
  sampling a line rather than an area.
- **Each constant must be a good rotation alone** (K-378), because each drives its own
  axis by its own index. K-367 took the plastic constant's 2D pair (1/ρ, 1/ρ²), whose
  second number is within 0.002 of 4/7 — as a standalone 1D rotation its samples fall
  into seven combs that drift too slowly to wash out across a pupil grid, and every
  area source wore them as stripes. 1/ψ is the same family of cubic Pisot units,
  rationally independent of 1/ρ, and measured cleanest of a scanned battery
  (`lens_flare_an_area_source_renders_without_stripes` pins the metric).
- **Each band re-samples the source at its own phase** (K-378). Bands trace and splat
  independently and their pictures sum, so a per-band `PHI_BAND` phase multiplies the
  effective source sampling by the band count for free and averages each band's
  residual reconstruction ripple toward the mean.

Extent 0 offsets every ray by exactly zero at every band, so a point source is
bit-identical to what it always rendered.

K-355's replication — up to `AREA_SAMPLES_MAX²` = 25 point lights per source, expanded on
the CPU for Manual and inside `detect_pick` for Matte — is deleted, along with the
`MAX_LIGHTS` = 64 slot table that existed only to hold it. Wherever a ghost was smaller
than the sample spacing, replication showed as that many separate copies of the aperture;
per-ray integration makes copies impossible rather than rare, because no two rays share a
source position and each ray's footprint already inflates by the local source-to-sensor
stretch — precisely the gap a replica would have sat in. `MAX_SOURCES` (16) is again both
the sources detection may find and the slots the trace carries. The op seam carries the
extent (`manual_lights` rows are `[x, y, r, g, b, ext_x, ext_y]`; the WGSL `Light` spends
two of its three pads on it), and Manual mode sets it by dial (**Source width** /
**Source height**, px@comp half-extents, default 0). Determinism is by construction — the
offsets are a pure function of the ray's own grid indices, so K-353's bit-stability
survives. **Area sources (K-267): every
gated tile's flux then accumulates onto its nearest anchor** — per tile, `(use source ?
the tile's brightest pixel's RGB : white) × the gate`, nearest by Chebyshev with ties to
the lowest anchor index, tiles visited in index order so the float sum matches the CPU
reference op-for-op — and each anchor is written as a light: position at its own source
pixel, colour = the summed flux × the Light tint (K-259). A one-tile point source is its
own anchor's only contributor and reads exactly as before K-267; a practical spanning
many tiles finally weighs as its whole lit area instead of one pixel. Every downstream stage runs per light on the dispatch z axis — the trace
computes each light's direction in-shader, the vertex build tints by the light, and the
combine stamps one starburst per live light — or, for a source wider than `SB_MIN_EXTENT`
of the raster, a fixed `SB_STAMPS`×`SB_STAMPS` grid of them spanning ±extent at
`1/(nx·ny)` of the light each (K-367), which is the shift-invariant convolution of the
sprite with the source in quadrature form. The starburst is a baked sprite, so it cannot
integrate its source the way the traced ghosts do; it does not need to, being
shift-invariant, and stamping once at the centre would give a softbox a star's pinpoint
spike. The K-365 field slice and azimuth are worked out **per stamp**, so a smeared
starburst near the frame edge leans a little differently at each end of itself; a point
source is one stamp at full strength on its own position, bit-identical to before. Manual mode is the same pipeline with one
CPU-written light carrying the tint (white by default). The CPU twin is `lens_flare::detect_lights`, held to the GPU by
the matte-mode frame oracle. The original design sketch (kept for the record): top-K
tie-breaking, and the trace runs per detected light with that sample's colour × energy as
its tint — all on-GPU, no readback, K ≤ 16. The CPU reference runs the identical
reduction. Everything downstream (trace → raster → combine) is unchanged, which is why
the mode can land later without moving any shipped parameter. Full-image convolution
(every pixel a light source, the batch-tool approach) is a recorded non-goal: it is a
seconds-per-frame offline technique, not an interactive effect; the top-K model is what
fits a compositor.

### 6a. Which layer the matte reads (K-288)

The Matte parameter defaults to **this layer** — the layer the effect is on — and
that reference does not render a second picture: it binds the effect's own input at its
point in the stack (`fxops::LayerInput::ThisLayer`, chosen by the draw builder when the
reference equals the owning layer's id). Two things fall out of that, and both were
broken before it:

- **Alignment is free.** The effect's input is already at the raster the flare writes,
  so no resample stands between the picture and the detection grid. A separately
  rendered layer has to be stretched to the working raster first.
- **Adjustment layers work at all.** An adjustment layer has no picture of its own, so
  the old "point at another layer" model had no correct answer there: whatever you
  picked, you detected lights in the wrong image. Its input *is* the composite of
  everything below it, which is exactly the picture an adjustment-layer flare is meant
  to flare.

Pointing the parameter at any other layer keeps the K-257 behaviour unchanged (rendered
alone at this raster, its own masks and effects per the K-142 source mode). See K-288
for the general rule, which covers every layer-input parameter, not just this one.

### 6b. Blending the element in (K-289)

The combine stage no longer asks Transparent-or-Black. It builds the flare **element** —
`rgb = (ghosts + starbursts) × Intensity`, `a =` that element's Rec. 709 luma, a
premultiplied black-backed overlay — and hands it to `flare_blend(mode, layer, element)`,
whose thirteen modes mirror Echo's arithmetic order exactly (per channel, all four
channels, premultiplied linear). The alpha saturates at 1 afterwards, and Mix lerps the
whole thing against the untouched input as before.

Add is `layer + element`, which reproduces the pre-menu combine bit for bit, so the
default moves nothing. Normal returns `(element.rgb, 1)`, ignoring the layer — the flare
on opaque black, which is what Background = Black produced on the empty layer that
option was for. `lumit-gpu` keeps its own `BLEND_COUNT` (it does not depend on
lumit-core) and a test pins it to `BLEND_OPTIONS.len()`, so a mode added to one and not
the other cannot silently clamp to Divide.

## 7. Traps (learned the hard way — do not rediscover)

- **Sub-pixel deposits are silently dropped by every rasteriser.** The caustic flux that
  makes a flare's bright rims lives exactly there. The anti-alias floor of §4 is
  load-bearing; measured without its predecessor (the K-261 inflation), the frame's
  dynamic range collapsed from ~116× to 6.6×.
- **…and rescuing a sub-pixel SLIVER by scaling it draws a streak.** This is the trap
  K-366 removed rather than guarded: a *quad* straddling a fold has near-zero area and
  large extent, so scaling it about its centroid to reach a px² floor multiplies its
  *length* by up to 100× — a 20 px sliver becomes a 2000 px line, which shipped in K-261
  as the "random lines across the flare" the owner reported. Splats are per ray and never
  straddle anything, so no such shape exists; the floor scales an axis by at most
  `MIN_SPLAT_AXIS_PX / |axis|` about the ray's own landing. Note that both oracles agreed
  with each other while drawing the streak — the CPU and WGSL mirrored the same wrong
  formula — so **parity tests can never catch this class**: the pin is a unit test of the
  deposit itself (`lens_flare_splats_conserve_flux_and_survive_folds`).
- **A hard boundary anywhere in the walk becomes a staircase.** The pupil grid samples
  the boundary at cell granularity, so any binary kill — mask, skirt clip, sphere miss,
  TIR — draws its edge as blocks the size of a cell. K-264 removed every one: geometry
  always continues, only the WEIGHT reaches zero, and it must reach zero CONTINUOUSLY
  (the feather, the Fresnel approach to TIR) before the geometry would have died.
- **Do not spray the front bezel.** Prescriptions list housing semi-apertures far wider
  than the entrance beam; a full-width spray wastes ~96% of its rays on some lenses and
  the survivors render as noise. Size the spray to the entrance pupil (§3).
- **A ray must deposit over its FOOTPRINT, never as a point.** The Monte-Carlo variant
  of this model (tried first for K-261) splatted each ray as a point and needed orders of
  magnitude more rays for the same smooth rim. K-366's splat is not that: each ray spreads
  its flux over the image of its own pupil cell, which is the same noise-free energy
  integral the quad grid computed — without joining neighbours across a fold to get it.
- **The exposure loop amplifies whatever survives rendering.** Any attempt to fade an
  artefact that also carries the lens's probe energy is undone by the gain; suppress
  artefacts geometrically (feather, skirt) or cap the gain, never by scaling energy the
  probe can see.
- **Roundness must be an SDF lerp to the circle, not an additive bulge** (K-260): the
  sine-bulge form pinches into a flower near 1, and the wide-open blend drives it there.
- **An over-long submission costs the DEVICE, not a frame.** macOS and Windows both kill
  a graphics submission that runs too long, and wgpu then reports a lost device: every
  later frame fails, the Viewer freezes, and — the detail that identifies it — opening a
  different project does not help, because a project is a fresh worker but the *process*
  keeps the device. Any effect whose cost the user can wind up (here Quality, Max ghosts,
  eight matte sources) must break its frame into bounded submissions. This is the K-263
  fault the owner reported.
- **Dropping a big GPU buffer does not give the memory back promptly.** A driver recycles
  it when the submission it belonged to retires, so a per-frame allocation in a
  continuously re-rendering Viewer (a drag, or the idle cache fill) is a rolling backlog,
  not a steady state. Pool anything measured in tens of megabytes.
- **The backward walk's media indices flip.** Travelling backward through surface s, the
  ray leaves the medium AFTER s and enters the medium BEFORE it; reflecting at the far
  bounce restores the forward convention. Get one wrong and every ghost lands wrong by
  centimetres — the pair filter's on-axis probe catches it instantly.

## 8. Test plan (all shipped; names in `fx/tests.rs` and lumit-gpu `fx/tests.rs`)

1. **FFT**: round trip, 8-point DFT match, Parseval (ortho).
2. **Optics units**: Cauchy reproduces (n_d, V); Snell at 45°; TIR returns None;
   normal-incidence Fresnel = ((n1−n2)/(n1+n2))². Coatings on n = 1.5 crown (K-356): a
   single layer beats bare glass and the broadband stack beats the single layer; the stack
   is wavelength-selective (>3× across the visible, which is what colours ghosts) and
   angle-dependent (53° reflects >1.5× normal incidence, which is what makes a ghost change
   hue off axis); an empty stack equals bare Fresnel, so the matrix chain closes; Coating 0
   is bare glass exactly.
3. **Library**: all 1299 files parse with sane focal lengths (2..2000 mm), surface
   counts (3..64) and positive semi-apertures; the bake is deterministic (pairs, sprite,
   gain bit-equal across runs); every ranked pair indexes real surfaces.
4. **Trace**: the top pairs land a solid live population with finite positions and
   weights in [0, 1]; the pupil mask is 1 at centre, 0 far outside, and passes less area
   as a hexagon than as a circle.
4a. **Four-bounce paths (K-368)**: the sentinel form of a bright pair traces to a finite
   landing and is bit-equal twice; the Biotar ranks four-bounce paths inside the rendered
   `MAX_RENDERED_PAIRS` and at least one of them puts light on the sensor, while the
   Master Prime's top eight stay two-bounce; no more four-bounce paths survive than were
   ever probed (`lens_flare_four_bounce_ghosts_rank_and_render`). The bake-invariant test
   checks the path constraints (`a < b`, `a < c`, `d < c`, sentinels in pairs), and the
   GPU trace oracle carries a Biotar case deep enough to walk the extra phases, asserting
   that four-bounce rays were actually compared.
4b. **Splat guard (K-366)**: an ordinary footprint deposits exactly the flux put into it
   (the tent integrates to the area the peak was divided by); a fold — axes long but
   nearly parallel — deposits a finite bright line rather than nothing or a spike; a
   collapsed footprint never falls below the anti-alias floor. Tested where the old quad
   bugs lived (`lens_flare_splats_conserve_flux_and_survive_folds`).
4c. **Grid budget (K-262/K-265/K-267)**: `pair_grid` is monotonic in spread, clamped to
   8..512 for degenerate inputs, and every bundled pair carries a finite spread. The
   GPU's mirrored copies (`pair_grid_of`, `flare_pad_dims_of`) are pinned equal to
   lumit-core's across bases, spreads and pad factors. The frame-time probe
   (`lens_flare_frame_probe_sees_corner_stretch`) must see the 7Artisans corner
   stretch, raise at least one pair past its rung at the Normal base, respect the
   boost floor/caps, spend at most the headroom, and agree bit-for-bit through the
   raw-rows seam entry.
4d. **Per-ray source integration (K-367)**: a point source's jitter is exactly zero over a
   64² grid sweep and its render is independent of how the light was built and stable
   across runs (`lens_flare_a_point_source_jitters_by_nothing`); a bar-shaped area source's
   ghost profile — the line through the point render's brightest pixel, 3-tap smoothed,
   strict local maxima above 10% of its peak — has no more maxima than the point's, while
   the same source rebuilt as K-355's 5×5 replication has strictly more
   (`lens_flare_an_area_source_does_not_replicate_its_ghosts`); a wide source and a point
   source deposit total energy within 2% (`lens_flare_an_area_source_keeps_its_flux`). The
   `PHI_U` / `PHI_V` literals are compared bit-for-bit against the shader's
   (`lens_flare_wgsl_spectral_constants_match_lumit_core`), and the frame oracle carries a
   non-zero Source size case so a drift between the two jitters cannot hide. The
   starburst's stamp grid is pinned alongside: a sub-threshold extent renders bit-identically
   to a point, and a 60 px source's lit span along the row through its light — measured at a
   tenth of the POINT starburst's peak, so both are read against the same line — grows by
   roughly the source's own width (`lens_flare_an_area_source_smears_its_starburst`), with
   `SB_MIN_EXTENT` / `SB_STAMPS` checked against the combine shader's copies
   (`lens_flare_wgsl_starburst_fields_match_lumit_core`).
5. **GPU trace oracle** (§8.5 shape, K-261 bounds): corner-for-corner against
   `trace_splat` across two lenses × two lights — mean position error < 0.2 px, p99
   < 3 px (a few-ULP difference near a fold legitimately lands a ray on the other
   branch), p99 relative weight error < 5% (2e-4 absolute floor), live/dead flips < 1%.
6. **GPU frame oracle** (§8.6): full pipeline vs the CPU reference at mean |Δ| < 2e-3
   with total energy within 1%, visible energy floor, bit-stable across runs, and
   Intensity-0 / Mix-0 bit-exact passthroughs.
6b. **Ghost blur (K-263)**: the blur alone against the CPU reference on a frame large
   enough for a multi-tile radius (`wgsl_lens_flare_ghost_blur_matches_the_cpu_reference`).
   Its bounds are *tighter* than the frame oracle's on purpose — measured, removing the
   line cache's halo entirely left the frame oracle's mean at 8.5e-4, well inside its
   2e-3, so that bound cannot see a misaligned blur. The tight pair (mean 2.5e-4, worst
   3e-3) sits about 3× above the correct kernel and 3× below that break.
6c. **Frame plan (K-263, no GPU needed)**: every (combo × light) is dispatched exactly
   once, no batch straddles two grids, no batch's scratch passes the budget
   (`lens_flare_batches_cover_every_combo_within_the_scratch_budget`); a default Normal
   frame splits into several submissions and a light one into exactly one
   (`lens_flare_splits_a_heavy_frame_into_several_submissions`); the bake cache evicts
   oldest-first rather than emptying (`lens_flare_bake_cache_evicts_the_oldest_not_everything`).
6d. **Padded anamorphic (K-267)**: at squeeze 0.5 the buffer doubles in width, the
   outer fifths of the frame carry real flare energy (the zone the un-padded buffer
   rendered black), and the padded pipeline still meets the frame bound
   (`wgsl_lens_flare_padded_anamorphic_matches_and_fills_the_edge`); past the 2× cap
   the zero-outside rule holds (`lens_flare_combine_does_not_repeat_the_flare_past_its_buffer`).
7. **Matte mode**: GPU detection + per-light flare against the CPU reference at the
   frame bound; the shared MAX_SOURCES / DETECT_TILE constants pinned. Area flux
   (K-267): a multi-tile disc's anchor carries several times a one-pixel dot's flux,
   the dot reads exactly as the pre-K-267 point, both anchors sit on their sources
   (`lens_flare_detects_area_sources_as_summed_flux`).
7b. **Custom lens file (K-264)**: a bundled lens's text fed through `bake_with` as a
   "custom file" bakes identically to picking it; the bake key separates library /
   custom / edited-custom; unparsable text degrades to the picked lens bit-for-bit
   (`lens_flare_custom_lens_file_overrides_and_degrades`). Editing the file on disk
   renames the frames that read it (`an_edited_lens_file_renames_frames`, lumit-render).
7c. **An animated aperture caches (K-425)**: two f-stops inside one step key the same
   AND bake bit-identically — both halves, because a shared key with an unshared bake
   would hand one f-stop the other's optics — while a step apart they differ, and the
   per-frame stop scale is not snapped
   (`lens_flare_bakes_are_shared_across_one_step_of_aperture`). A bake in flight leaves
   every other frame's name and every op's name alone
   (`a_baking_flare_does_not_unname_other_frames`,
   `a_bake_in_flight_does_not_rename_every_op`), ten frames of a keyframed f-stop each
   take their own name (`a_keyframed_aperture_names_every_frame`), and the substitution
   count is 0 for an exact frame, 1 for the frame that stood the previous lens in, and
   still 1 once the bake has landed
   (`lens_flare_deferred_bakes_answer_with_the_previous_lens_then_the_new_one`).
8. **Neutrals and blend (K-289)**: Normal shows the element alone on opaque black; Add
   reproduces the historical `in + flare` with saturating alpha; every option resolves
   and an index past the menu clamps; the blend table matches its formulas by hand; the
   Intensity-0 / Mix-0 passthroughs are bit-exact whatever the menu holds. Migration:
   a saved Transparent becomes Add, a saved Black becomes Normal, the dead parameter
   goes, and loading twice changes nothing
   (`lens_flare_background_migrates_to_the_blend_menu`).
8b. **This layer (K-288)**: a fresh flare's Matte points at the layer it was added
   to, a preset's stays unset, DoF's depth is untouched
   (`lens_flare_matte_defaults_to_the_layer_it_is_added_to`); the draw builder answers
   `ThisLayer` for a self-reference on an ordinary layer AND on an adjustment layer, and
   `Absent` while Source is Manual or the reference is unset
   (`a_flare_matte_pointed_at_its_own_layer_reads_this_layers_input`).
9. **Focus**: the thin-lens shift is 0 at infinity, `f²/(1000·d − f)` near, ≤ f always.
10. **Shader validity** (K-263, `lumit-gpu/tests/wgsl_validates.rs`): every `.wgsl` in
   the crate parses and validates through naga, with no graphics card involved — so a
   broken kernel fails everywhere rather than only where a card exists to reject it.
11. **Cost** (`lens_flare_frame_cost`, `#[ignore]`d): prints one default frame's time.
   Not a gate — the number is whatever the machine can do — but run it either side of a
   pipeline change, because "the flare is faster now" rots quietly otherwise.
12. **Eyes** (`lens_flare_dump_frame`, `#[ignore]`d): renders one frame through the real
   GPU pipeline to a tone-mapped PPM (`LUMIT_FLARE_DUMP`, with lens / quality / light /
   gain overrides). The K-264 artefact work was driven by looking at these — no numeric
   bound in this file answers "does it look right", and a software Vulkan driver
   (`mesa-vulkan-drivers`) is enough to run it on any machine.

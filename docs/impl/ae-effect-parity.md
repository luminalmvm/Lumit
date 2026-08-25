# AE effect parity: the gap inventory and the build order

**In plain terms:** After Effects ships with a large set of built-in effects, and the
AE import (docs/11-AE-IMPORT.md, roadmap Phase 4) can only convert an effect Lumit
actually has. This note is the audit of what AE ships against Lumit's catalogue (35 at
the time of the audit; 53 after the four wave-1 batches below, 85 with wave 2's six and the
mask seam's three), and the order the
gaps get built in. It is a working inventory, not a spec: each
effect that gets built lands through the ordinary channel — a docs/08 section first,
then the registry declaration, kernel pair and oracle like every existing effect.

Three scope rules, all deliberate:

- **Parity is a floor, not a ceiling (K-401).** Every effect carries AE's parts with
  mappable semantics so the import converts cleanly - and goes beyond them wherever
  that genuinely improves the effect, with the extras defaulting neutral so an
  imported instance is faithful.

- **"Default AE effects" means Adobe's own** (`ADBE` match names). The Cycore (`CC …`)
  set ships in the AE installer but is third-party; it is tier C here and may end up
  plugin-era territory.
- **Audio effects, 3D-channel effects and the obsolete category are out of scope** —
  Lumit's audio effects are their own future programme (docs/09), 3D-channel needs
  AOV inputs that do not exist yet, and "obsolete" is obsolete.

## What Lumit already covers (mapped, not always 1:1)

| AE effect | Lumit answer | Note |
|---|---|---|
| Gaussian Blur / Fast Box Blur | Gaussian blur | box maps to gaussian, lossy but honest |
| Directional Blur | Directional blur | |
| Radial Blur | Radial blur | |
| Sharpen / Unsharp Mask | Sharpen simple / Unsharp mask | |
| Camera Lens Blur | Depth of field | richer than AE's |
| Glow | Glow | exposure-aware; mapped, not lossless |
| Tint | Tint | |
| Invert | Invert | RGB channel scope only today |
| Exposure | Exposure | |
| Color Balance | Colour balance | lift/gamma/gain form |
| Vibrance | Vibrancy | |
| Echo | Echo | |
| Posterize Time | Posterize time | |
| Transform | Transform | |
| Apply Color LUT | LUT | |
| Keylight (bundled) | Matte key | the keyer family's one member |
| Lens Flare | Lens flare | far beyond AE's |
| Timewarp | — deliberate non-effect | Retime with flow is the answer; import reports it |

## Tier A — the first wave (import-table anchors + daily-driver gaps)

Every one of these is either named in docs/11's seed table as a Lumit target that does
not exist yet, or is so common in real AE projects that import without it is theatre.
All are GPU-shaped, single-image, side-table-free (post-K-387 they can also take the
K-395 matte for free).

| AE effect | Lumit name (docs/01 voice) | Sketch |
|---|---|---|
| ~~Curves~~ | **Curves** (§3.30) | per-channel cubic spline LUT, master + R/G/B; the import target for `ADBE CurvesCustom` |
| ~~Levels~~ | **Levels** (§3.31) | in/out black-white + gamma, per channel |
| ~~Brightness & Contrast~~ | **Brightness** (§3.32) | AE's combined control; distinct from the existing Contrast (pivot differs) |
| ~~Hue/Saturation~~ | **Hue and saturation** (§3.33) | channelised hue/sat/light (master + six ranges); the existing Hue shift stays |
| ~~Fill~~ | **Fill** (§3.34) | flood the alpha with one colour |
| ~~Gradient Ramp~~ | **Gradient** (§3.35) | linear/radial two-colour ramp with scatter |
| ~~Fractal Noise~~ | **Fractal noise** (§3.37) | the big one: value/perlin fractal with evolution, sub-scaling, offsets; drives half of AE-land |
| ~~Turbulent Displace~~ | **Turbulent displace** (§3.38) | fractal-driven displacement; the owner's own matte example |
| ~~Motion Tile / Offset~~ | **Tile** (§3.39), **Offset** (§3.40) | wrap-around tiling and phase offset |
| ~~Mirror~~ | **Mirror** (§3.41) | reflection about a movable axis |
| ~~Optics Compensation~~ | **Lens distort** (§3.42) | barrel/pincushion with FOV |
| ~~Drop Shadow~~ | **Drop shadow** (§3.43) | the most-used effect in AE full stop |
| ~~Noise~~ | **Noise** (§3.36) | per-pixel uniform/gaussian noise, mono/colour |
| ~~Linear Wipe / Radial Wipe~~ | **Linear wipe** (§3.46), **Radial wipe** (§3.47) | transitions; trivial kernels, high import hit-rate — and the new **Transition** category (K-400) |
| ~~Set Matte~~ | **Set matte** (§3.44) | reads naturally as a K-395 sibling, and is one: the sixth override, where the matte *is* the alpha |
| ~~Channel Blur~~ | **Channel blur** (§3.45) | per-channel gaussian radii |

## Tier B — the second wave (real but rarer)

Bezier Warp, ~~Corner Pin~~, ~~Displacement Map~~ (arbitrary-layer displacement — subsumes some
of Turbulent Displace's uses; needs the K-395 carriage), ~~Polar Coordinates~~, Ripple,
~~Twirl~~, ~~Spherize~~, Wave Warp, Warp (mesh styles), Roughen Edges, Posterize, Threshold,
Tritone, Photo Filter, Black & White, Shadow/Highlight, Broadcast Colors (report-only?),
Grain family (Add Grain / Remove Grain — heavy, temporal), ~~Median~~, ~~Mosaic~~,
~~Find Edges~~, ~~Emboss~~, ~~Texturize~~, ~~Venetian Blinds~~, ~~Iris Wipe~~,
~~Card Wipe~~ (geometry, not particles — see the ruling below), Write-on
(paint already covers the intent), Audio spectrum/waveform (needs the audio tap),
~~Beam~~, ~~Advanced Lightning~~, ~~Radio Waves~~, ~~Scribble~~, ~~Stroke~~, ~~Vegas~~.

## Wave 2 — all of Tier B (owner-directed, 2026-08-20)

The owner's ruling: editors use most of Tier B constantly, so it all gets built — full
parity — with one standing exclusion: **no particle-world port**. Lumit's own particle
system is a future programme aimed at EmberGen/Particular class, and porting a lesser
one would anchor it low. Card wipe stays (a grid of flipping cards is geometry, not a
particle system).

Batches, in build order:

1. ~~**Distort I** — Corner pin, Displacement map (rides the K-395 carriage), Polar
   coordinates, Twirl, Spherize.~~ **Complete, 2026-08-20** — see below.
2. ~~**Distort II** — Ripple, Wave warp, Bezier warp, Warp (the style presets), Roughen
   edges.~~ **Complete, 2026-08-20** — see below.
3. ~~**Stylise I** — Posterize, Threshold, Tritone, Photo filter, Black and white,
   Shadow highlight.~~ **Complete, 2026-08-20** — see below. All six landed in
   **Colour**, not Stylise: AE files every one of them under Color Correction, and
   each is tone or colour maths on a pixel rather than a stylisation of a shape.
4. ~~**Stylise II** — Median, Mosaic, Find edges, Emboss, Texturize, Broadcast safe
   (AE's Broadcast Colors as a plain clamp effect).~~ **Complete, 2026-08-20** — see below.
5. ~~**Transitions** — Venetian blinds, Iris wipe, Card wipe.~~ **Complete, 2026-08-20** —
   see below.
6. ~~**Draw and grain** — Beam, Lightning, Radio waves, Scribble, Stroke, Vegas,
   Add grain (reuses the fractal noise core per channel).~~ **Complete, 2026-08-20** — see
   below. Wave 2 is complete with it. Scribble, Stroke and Vegas' Mask/Path half
   stopped on the mask seam and landed with it a day later (K-408, and the section
   below).

Recorded skips, with reasons: **Remove grain** (a denoiser is its own programme, not
an effect port), **Audio spectrum / Audio waveform** (blocked on the audio tap,
docs/09), **Write-on** (Paint covers the intent; the import reports the suggestion),
and everything particle-class per the ruling above. Batch 6 briefly added
**Scribble**, **Stroke** and **Vegas' Mask/Path half** to that list; the seam they
were blocked on is built and all three are, so the list is back to four.

## The mask seam — what Scribble and Stroke wanted, and how it was given to them

**In plain terms.** Three of AE's draw effects work on a *mask you have drawn* rather than on
the picture. Scribble fills a mask path with pencil strokes; Stroke walks a mask path with a
brush between a start and an end per cent; Vegas can march its segments round a mask path
instead of round a contour it found. All three need the path's **geometry** — where the curve
goes — and not the coverage the mask produces, which is a picture and says nothing about which
way is *along*.

Lumit's effect boundary did not carry geometry, and the gap was precise — this is the
inventory that was taken before it was closed, kept because the shape of the missing thing is
what made the seam the right shape:

- **Parameters.** An effect is handed `lumit_core::fx::Params`: floats, integers, angles,
  choices, booleans, colours, seeds, file slots and layer references (`ParamKind`, in
  `crates/lumit-core/src/fx/schema.rs`). There is no vector or path kind, and adding one is not
  a schema tweak — it needs a stored form, an animation story (`mask::PathKeyframe` already has
  one, per mask) and a bridge shape (docs/17).
- **Pictures.** The other half of an effect's input is `AuxSlot`
  (`crates/lumit-render/src/gpufx.rs`): the K-395 matte, layer inputs, temporal neighbours, a
  flow field, a lens prescription. Every one of them is a texture or a file handle.
- **Where the masks actually are.** They live on the layer — `Layer::masks`, of
  `lumit_core::mask::Mask`, each with a `path_at(t)` resolving its keyframes to a `BezierPath`
  — and `crates/lumit-render/src/build.rs` consumes them through
  `lumit_core::mask::apply_masks` **after** the effect stack has run, as a coverage buffer.
  Nothing between the layer and the effect carries the vertices.

So the honest shape of the work is *a new kind of effect input*: a resolved path list, sampled
at the frame's time, arriving beside the matte — with a `ParamKind` for "which of this layer's
masks", a resolve step that walks `path_at`, a carriage through `AuxSlot`, and a decision about
how a kernel that gathers is meant to read a curve at all (Stroke's brush is a scatter; the
gather form is a distance field over an arc-length parameterisation, which is a small
programme of its own). That is a docs/08 §1.1 and docs/17 change, and forcing it through a
float parameter would be the wrong shape permanently.

Until it existed, the import reported substitutes — Fill (§3.34) inside the mask for
Scribble, Vegas (§3.76) on Alpha for Stroke, each of which strokes the *shape* a mask cuts
rather than the path itself. **Both are retired**: docs/11 now maps all three effects
properly, and the only things it still reports against this seam are AE's **All Masks**,
**Stroke Sequentially** and Scribble's two multi-mask Fill Types, which want a row that
names a *set* of masks rather than one.

**Built, 2026-08-21 (K-408) — the seam, ahead of its consumers.** The gap above is closed
piece for piece, and the three effects are now blocked on nothing but themselves.
`ParamKind::MaskPath { self_default }` is the parameter (`#[mask_path]` in the derive; the
value is `EffectValue::MaskPath(Option<Uuid>)`, a mask id or the "First mask" entry, static
in v1 like a layer reference). `lumit_core::mask::mask_path_at` is the carriage's core: it
resolves the row against the layer's masks and flattens `path_at(t)` to a `MaskPolyline` —
points plus cumulative arc length, closed flag — within `MASK_PATH_TOLERANCE_PX` (0.5 px@comp,
a constant so the polyline cannot vary a frame's identity). `build.rs`'s `mask_paths_for`
fills one slot per op whose schema answers `EffectSchema::mask_path()`, `fxops::run_ops`
consumes one per such op on its own counter, and it arrives on `AuxSlot::mask_path()` beside
the K-395 matte. Empty polyline = the documented no-op.

**What the consumers did with it, 2026-08-21.** All three landed on the same day as each
other: **Scribble** (§3.78), **Stroke** (§3.79) and **Vegas' Mask/Path source** (§3.76).
Three answers came out of building them, and they are the reason this section is worth
keeping after the gap it describes has closed.

**The gather-form question this section raised has an answer, and it is smaller than the
question.** Stroke's brush is a scatter, and the gather form was expected to be a distance
field over an arc-length parameterisation. It is — but the field never needs building,
because a *chain of round stamps spaced under half a brush width apart is the capsule the
brush sweeps*, to within an eighth of a radius at the deepest scallop. So a dense stroke is
drawn as the path itself and a sparse one as separate dots, and both are the same
expression: a minimum distance to a list of straight pieces. There is no scatter pass and no
second buffer.

**One kernel serves all three.** They differ entirely in where the line goes and hardly at
all in how it is drawn. Where the line goes is decided host-side — a hatch clipped to the
mask, a brush trail along it, the mask itself — and what arrives at the kernel is the same
in every case: straight pieces in raster pixels, each carrying how far along the drawing its
start sits. §3.76's "a lit share of 2 means a continuous line" convention switches the dash
off without even a branch, so Scribble and Stroke ride Vegas' dash machinery for free. This
is §3.74's decision generalised: *if the geometry does not vary per pixel, it does not belong
in the kernel* — and once it is out of the kernel, three effects can share the kernel.

**The polyline is still a CPU slice, and still uploads as a uniform.** No storage buffer was
built, because none was needed: 512 pieces of four floats is a uniform the size of
Lightning's twice over, and the pieces are what the kernel wants rather than the vertices.
Past that budget every consumer *coarsens* — a wider hatch spacing, wider-spaced dots, a
straighter chain — rather than drawing part of a shape, which is docs/14 §4's rule applied to
geometry. A storage buffer becomes the right answer the day something wants tens of thousands
of pieces, and nothing does.

**What is still owed to this seam** is only what a row naming *one* mask cannot say: AE's
**All Masks** and **Stroke Sequentially**, and Scribble's two multi-mask Fill Types. A row
that names a set is a small extension of `ParamKind::MaskPath` and a list rather than a slot
in the carriage; nobody has asked for it, so it has not been built.

## Tier C — bundled-but-third-party and the heavy simulations

The Cycore set (`CC Particle World`, `CC Ball Action`, …), Particle Playground, Shatter,
Caustics, Wave World, Foam, Mesh Warp's advanced modes, Puppet (its engine is a Lumit
roadmap item of its own), Rotobrush (models — plugin era, K-390's rule).

## The build order

1. ~~**Wave 1 = Tier A**, batched by family: colour (Curves, Levels, Brightness,
   Hue and saturation), generate (Fill, Gradient, Fractal noise, Noise), distort
   (Turbulent displace, Tile, Offset, Mirror, Lens distort), composite utilities
   (Drop shadow, Set matte, Channel blur), transitions (Linear wipe, Radial wipe).
   Fractal noise lands before Turbulent displace because the displacer reuses its
   noise core.~~ **Complete, 2026-08-20** — all eighteen, in four batches, listed below.
2. Each batch: docs/08 sections first (the spec pins parameters, defaults, units in
   px@comp per §2.3), then declaration + CPU reference + WGSL kernel + oracle +
   manual page prose, K-303 strings, the K-395 matte free or overridden as fits.
3. docs/11's seed table gets its Lumit column trued as each target becomes real.
4. Tier B by demand once Wave 1 ships; Tier C is not scheduled.

## Open questions

- ~~Whether Brightness & Contrast folds into the existing Contrast as a mode rather
  than a sibling (AE-import fidelity says sibling; menu hygiene says mode).~~
  **RESOLVED (K-397): sibling.** It is **Brightness**, carrying both of AE's sliders
  under AE's names and neutral point (docs/08 §3.32); Contrast is untouched. The
  deciding reason was neither of the two above: one control cannot be both a per cent
  where 100 is neutral and a signed amount where 0 is, and a mode switch that re-scales
  a stored slider reads fine in a menu and wrong in a project file.
- ~~Whether Set matte belongs in Utility or as a documented pattern of K-395's row.~~
  **RESOLVED (K-400): both, and those were never two answers.** Set matte is an effect in
  **Utility** whose source *is* the universal Matte row, under K-395's `Own` role
  (docs/08 §3.44). What settled it was noticing that the question assumed the row and the
  effect were alternatives: the row says *which layer*, the effect says *what to do with
  it*, and "take its red channel as my alpha, intersected with what I already had" is not
  something a row can say. It is the sixth override, and the first for which the matte is
  the **output** rather than a modifier of one.

## What has landed

- **The Expression Controls family (2026-08-21, K-414), pending audit** — Slider control
  (§3.80), Angle control (§3.81), Checkbox control (§3.82), Colour control (§3.83) and
  Point control (§3.84), in a new **Controls** category, taking **the catalogue to 90**.
  They are the first effects that declare no image operation because they draw nothing
  *ever* (Posterize time and the accumulation motion blur declare it because they act a
  level above the stack), the first to declare `MatteRole::None`, and the first parity
  entries with no picture to compare — so there is no WGSL kernel, no CPU oracle and
  nothing for §1.6 to hold together.
  **Their five match names are in the import table marked PENDING AUDIT.** They are the
  famous ones (`ADBE Slider Control` and kin) but were not in the 2026-08-20 sitting's
  audited set, so `tools/ae-audit/claimed-matchnames.txt` grew from 60 names to 65 and the
  next sitting confirms them. Shipping a pending row is safe here for the reason docs/11 §6
  gives: a match name this table has wrong is a name nothing claims, and an unclaimed name
  takes the placeholder road with every parameter kept.
  It also landed `ParamKind::Slider` (K-414's other half): a closed range drawn as a track
  and thumb, whose value side is an ordinary float exactly as Int's and Angle's are. The
  four wipes' **Completion** adopted it and nothing moved — no stored value, no keyframe,
  no pixel. **Temperature was K-414's named first candidate and declined it**: its ±150
  slider runs to a ±200 hard range, so there is a picture beyond the slider's end and the
  range is not the parameter's nature.
- **The mask seam and the three path effects (2026-08-21, K-408/K-409)** — the mask-path
  input kind (`ParamKind::MaskPath`, the arc-length polyline carriage, the panel's mask
  picker) plus Scribble (§3.78), Stroke (§3.79) and Vegas' Mask/Path half, all three riding
  one shared path-drawing kernel (K-409: 512-piece uniform, coarsen never truncate). Import
  substitutes retired; **the catalogue stands at 85**. The full story is in the mask-seam
  section above.
- **Wave 2, Draw and grain (2026-08-20, K-407)** — Beam (§3.73), Lightning (§3.74), Radio
  waves (§3.75), Vegas (§3.76) and Add grain (§3.77), with their WGSL kernels, CPU oracles,
  manual pages and docs/11 seed-table entries. **The catalogue stands at 83**, all five are
  **Generate** — §3.36 Noise's reason, that what they do is put something *on* a frame rather
  than change the colour of what is there — and **Wave 2 is complete with this batch**.
  **Scribble and Stroke did not land in this batch**, and Vegas landed on its Image Contours
  half only: all three wanted their layer's mask *geometry*, and no seam carried it. That was
  written up above rather than worked around, because the shape of the missing thing was the
  useful part — and a day later it was built (K-408) and all three landed, taking the
  catalogue to 85.
  Five things came out of it beyond the five effects.
  **If the randomness does not vary per pixel, it does not belong in the kernel.** Lightning's
  bolt is built once a frame in `packed()` into at most 192 straight segments and travels in
  the uniform; the kernel is a minimum over capsules with no hashing in it. That is a few
  hundred multiplications a frame instead of the few hundred million a per-pixel rebuild would
  cost — and it **disposes of §1.6 for free**, both paths being handed the identical numbers.
  The test for the next effect that wants a hash: does the random thing vary per pixel? Card
  wipe's per-card shuffle does; a bolt does not.
  **A clock becomes a control, and the control is richer than the clock.** Radio waves' Time is
  an ordinary parameter in seconds, with Frequency, Expansion, Lifespan and Spin measured
  against it. Keyframed linearly it is AE's effect exactly, held it freezes the waves
  mid-flight, scrubbed back it restores them — and unlike AE's rate it can be varied. Third
  time a missing rate has been the faithful conversion (K-403's Wave Speed twice, §3.63's Auto
  Amounts); first time the replacement can do something the original cannot.
  **A contour is a level set, and a stroke on one needs its direction as well as its
  position.** Vegas divides the value's distance from the threshold by the gradient's
  magnitude, which turns it into a distance in *pixels* — so Width is a width, and the effect
  switches itself off where the picture is flat because a vanishing gradient sends that
  distance to infinity. Two companions worth carrying: the gradient is a separable **5×5**
  Sobel, because a 3×3 one on compressed footage points a different way in every pixel and the
  dashes come out as speckle; and the dash phase is measured **from the frame's middle**,
  because a direction error of e moves the phase by the pixel's distance times e.
  **Softness can be a crossfade between two readings of one field.** Add grain reads the same
  lattice hard (one flat value per cell) and soft (interpolated) and blends them — one extra
  hash instead of a second full-frame pass, with both ends of the control a look somebody
  wants. Its companion: **Monochrome is a lane, not an average**, using the noise core's
  `channel` argument exactly as the fractal sum uses it for octaves.
  **When a gradient's endpoint coincides with a coverage's endpoint, the endpoint is not
  reachable.** Beam's rim colour is reached at the *inner half* of the soft band rather than at
  the beam's own edge, where the pixel is half-covered and about to vanish — otherwise Outside
  colour would name a colour nobody ever sees.
  Five deliberate divergences from AE, all recorded in docs/08: **Beam's thicknesses, Add
  grain's Size and Radio waves' Expansion are px@comp**; **Beam's 3D Perspective is not
  carried** (K-406's camera ruling again); **four of AE's eight Lightning Types are built** and
  Alpha Obstacle is not carried at all; **Radio waves ships one Stroke width** where AE tapers,
  and only its Polygon wave type; and **Vegas' Segments count becomes a Segment length**, since
  an effect that never traces a path has no arc length to count round. All five took the
  generic strength matte — what a matte says on a draw effect is *where the drawing is*.
- **Wave 2, Transitions (2026-08-20, K-406)** — Venetian blinds (§3.70), Iris wipe (§3.71)
  and Card wipe (§3.72), with their WGSL kernels, CPU oracles, manual pages and docs/11
  seed-table entries. **The catalogue stands at 78**, and all three are **Transition**, which
  finishes the category K-400 opened for exactly them. Four things came out of it beyond the
  three effects.
  **A gather can hold a camera, provided the projection inverts.** Card wipe is the first
  kernel in the catalogue to put one in front of a pixel, and the obvious build — transform the
  rectangle, rasterise, composite — is a scatter, which Lumit's effects are not. A one-point
  projection of a rotating card is a Möbius map in the card's own coordinate, so it inverts in
  **one division**, and the whole effect becomes a single cheap pass with no geometry pipeline.
  The rule to carry: *before building a scatter, check whether the map inverts in closed form.*
  §3.55's Bezier warp is the case where it does not; this is the case where it does.
  **The camera has no controls, and that omission is the conversion.** AE's Card Wipe carries
  three camera systems, a lighting group, a material group and two jitters. Lumit keeps cameras
  on the composition (docs/06), so each card is projected in its own local frame at a fixed
  viewing distance and all of that is reported rather than approximated — §3.53's missing Wave
  Speed and §3.63's absent Auto Amounts a third time. What *survives* the cut is chosen by one
  test: **is it still visible?** Flip direction only means something because the perspective is
  there, which is why the perspective is there.
  **A rotationally symmetric shape is one sector.** Iris wipe never rasterises its polygon: the
  pixel's angle folds into a single wedge and mirrors about that wedge's bisector, so the whole
  boundary — polygon or star, six sides or sixty-four — becomes one straight edge and the
  distance to it is a dot product. Plain and starred are the *same expression*, differing only
  in where the host puts the second vertex, and the distance that comes out is a **true
  perpendicular one in pixels**, which makes Feather a width — §3.47's radial-feather problem
  avoided rather than clamped around.
  **Both ends of an animated range are tested for, never arrived at.** `cos(½π)` in `f32` is
  6·10⁻⁸, so a card wipe that trusted its trigonometry would leave a hairline of
  quarter-strength pixels at Completion 100. Both paths test the clamped progress instead. It is
  §3.42's and §3.52's short-circuit — which were about an effect turned *off* — extended to the
  far end of the range.
  Three deliberate divergences from AE, all recorded in docs/08: **Venetian blinds' Width is a
  length in px@comp** where AE's is raster pixels; **Iris wipe's two radii are lengths in px@comp
  while its centre is a place** (§3.51's split of a size from a place; both px@comp since K-419); and **Card wipe loses AE's
  Gradient flip order**, which needs a gradient *layer* — §3.68's test says a card wipe has a
  second thing to say about *where*, so its one layer row stays the universal Matte, and
  Randomness plus Seed covers the intent that is left. All three took the generic strength
  matte: a transition is already a statement about how much of a pixel there is, and a matte on
  one says *where the transition is*, which is what a dissolve says.
- **Wave 2, Stylise II (2026-08-20, K-405)** — Median (§3.64), Mosaic (§3.65), Find edges
  (§3.66), Emboss (§3.67), Texturize (§3.68) and Broadcast safe (§3.69), with their WGSL
  kernels, CPU oracles, manual pages and docs/11 seed-table entries. **The catalogue stands at
  75.** Five of the six are **Stylise** — this is the batch the name finally fits, since what
  each does to a frame is change how it *looks* rather than what colour a pixel is — and
  Broadcast safe is **Utility**, a delivery tool rather than a look. Five things came out of it
  beyond the six effects.
  **A kernel may not branch on a pixel's value, so a median is a compare-exchange network.**
  Median is the first effect in the catalogue whose answer is *chosen* rather than computed,
  and the textbook way to choose — a quickselect — executes a different sequence of
  comparisons on the two paths, which §1.6 cannot hold to agreement. Both paths instead sweep
  the window once, carrying the smallest half in a sorted array and inserting each sample with
  a bubble of `min`/`max` pairs. Nothing branches; the answers are bit-identical even though
  the GPU sweeps the widest window and pads while the CPU sweeps only what it was asked for,
  because `min` and `max` are exact and a sorted set does not depend on insertion order. And
  because both are componentwise on a vector, the three channels and the alpha come out of
  **one** network.
  **A cap you can type past is not a cap** (§3.64 decision 2). The sweep costs `(2r+1)⁴ ÷ 2` a
  pixel, so Median's Radius stops at 3 and stops *hard* — the one place in the catalogue where
  §1.2's "a slider may be exceeded by typing" is deliberately not true, because a control that
  silently clamps answers a different question from the one it was asked. It makes the AE
  import's first conversion limited by a **budget** rather than by a semantic.
  **K-399's rule about a threshold reaches a *coordinate*** (§3.65 note 1). Mosaic decides which
  block a pixel is in, and `floor(x ÷ block_width)` in floating point puts a pixel in different
  blocks on the two paths wherever the division is exact. Every boundary and sample position is
  integer arithmetic instead. Its companion: **the averaged mode samples the block rather than
  reading it**, at most 8×8, which is the same flat colour on any block worth mosaicking and an
  *exact* mean on any block under eight pixels across.
  **A gradient belongs on the perceptual value too** — K-404's rule arriving on two effects
  that are not grades. Find edges and Emboss both difference their neighbours in `√` light,
  because a Sobel taken in scene-linear draws the specular highlights and nothing else and a
  relief taken there is all highlight and no shadow. It is what makes the two read as AE's
  pencil drawing and AE's grey relief rather than as maps of where the picture is brightest.
  **A second layer is not always the matte** (§3.68 decision 1). Texturize takes a layer as
  §3.49's Displacement map does and pointedly does *not* take it on the Matte row: a
  displacement map has nothing else it could be, a texture does, and an editor wants to press a
  canvas in **and** limit the pressing to a region. The test for the next effect that wants a
  layer: does it have a second thing to say about *where*? Its Placement then splits AE's
  question in two — **Scale** says how big one copy is, **Placement** says only what happens
  outside it — so all three names survive on a carriage that only ever stretches to fit, and at
  Scale 100 all three coincide, which is AE's own case.
  Two notes that are not decisions: **Broadcast safe's kernel writes its luma out longhand**
  instead of using `dot`, because two of its four modes turn that number into a threshold on
  the alpha; and **`target` is a WGSL reserved keyword**, which compiles into a texture of
  zeros rather than into an error — the §1.6 oracle caught it at 15 584 fp16 ULP.

- **Wave 2, Stylise I (2026-08-20, K-404)** — Posterize (§3.58), Threshold (§3.59), Tritone
  (§3.60), Photo filter (§3.61), Black and white (§3.62) and Shadow highlight (§3.63), with
  their WGSL kernels, CPU oracles, manual pages and docs/11 seed-table entries. **The
  catalogue stands at 69**, and all six are **Colour** effects rather than Stylise ones —
  the batch is named for its place in the build order, not for where the effects live. Five
  things came out of it beyond the six effects.
  **A tone control belongs where the eye is, and the operation stays in light.** Four of the
  six place a control *on the tone range* — Posterize's rungs, Threshold's Level, Tritone's
  three stops, Shadow highlight's midtone pivot — and in scene-linear light the middle of the
  range is 0.25, not 0.5. All four run through one shared curve
  (`lumit_core::fx::cpu::perceptual`) so that 50 means mid-grey; §3.18's linear pivot is not a
  counter-example, being the middle of an operation rather than of a judgement. Recorded once
  in §3.58 decision 1 and cited from the other three.
  **That curve is a `sqrt`, and the reason is the oracle rather than the picture** (§3.58
  decision 2). A quantiser's output is a step, so one bit of disagreement about which side of
  a rung a value falls on is a whole rung of colour — K-399's threshold rule arriving on an
  effect where nothing moves. `sqrt` is one correctly-rounded instruction on both paths;
  `pow(u, 1/2.2)` is a per-vendor polynomial. The rounding is written `floor(x + ½)` in both
  paths for the same reason: WGSL's `round` breaks a tie to even and Rust's away from zero.
  **The shipped blur pays for a third effect, and here it is a *question*.** Shadow highlight
  blurs the picture at Radius and uses only its luma, and only to decide whether a pixel is
  being treated as a shadow — no colour is ever taken from it, so nothing is softened. That is
  the whole of local adaptation, and it is why a white button inside a dark jacket is lifted
  with the jacket. §3.43's softening and §3.57's distance field were the first two reuses.
  **Auto Amounts is not built, and the omission is the conversion** (§3.63). AE chooses the
  two amounts from the frame's histogram and smooths that choice across neighbouring frames;
  the result is a grade whose answer at one frame depends on the shot around it. An imported
  instance gets AE's default manual pair and a report — §3.53's missing Wave Speed in another
  costume.
  **Black and white's six weights ride an exact decomposition**, not a weighted sum, which is
  what makes them provably harmless on a neutral and seamless on a gradient (§3.62). The same
  shape of argument as §3.33's hat functions summing to one, from the other end.
  Four deliberate divergences from AE, all recorded in docs/08: **Threshold gains a Softness**
  defaulting to AE's hard cut; **Photo filter's twenty named filters are Lumit's own
  chromaticities** under Adobe's names, a look-for-look conversion the import reports as
  mapped (§3.56's thirteen Warp styles again); **Black and white's Tint colour is divided
  through by its own luma**, so it tints rather than darkens; and **Shadow highlight ships one
  Radius where AE ships two**, the import averaging them. All six took the generic strength
  matte: a tone or colour operation dissolved by a matte is exactly that operation applied
  where the matte is bright, which is the colour batch's reasoning (K-396) a second time.
- **Wave 2, Distort II (2026-08-20, K-403)** — Ripple (§3.53), Wave warp (§3.54), Bezier warp
  (§3.55), Warp (§3.56) and Roughen edges (§3.57), with their WGSL kernels, CPU oracles,
  manual pages and docs/11 seed-table entries. **The catalogue stands at 63**, and Roughen
  edges is the batch's one Stylise effect — nothing inside the shape moves, only its
  outline. Four things came out of it beyond the five effects.
  **AE's Wave Speed does not survive contact with §2.4, and the conversion is the answer.**
  Both Ripple and Wave Warp animate themselves off the clock, which makes preview and export
  disagree and a cached frame a lie. Neither Lumit effect has a speed: they have Evolution
  and Phase, angles the timeline animates, and the import writes AE's speed as two keyframes
  of `360 × speed` degrees a second. Recorded in both §3.53 and §3.54 because it is the
  first time a *missing* control has been the faithful conversion.
  **Bezier warp is the first kernel in the catalogue that solves rather than computes**, and
  a solver brings two obligations a formula does not. It has to be allowed to **give up** —
  a patch folded over itself has no single answer, so the Newton iteration stops on a
  singular Jacobian exactly as §3.48's degenerate quad short-circuits — and its answer has
  to be **checked**. The unchecked version scattered stray opaque pixels across the empty
  part of the frame, because outside the patch the iteration wanders until it happens to
  land in `0..1`; one extra patch evaluation asking "does this answer solve the problem?"
  removes them. Both are in §3.55, and the second is worth remembering for anything else
  that iterates.
  **The blurred alpha is a distance field, for free**, which is how Roughen edges chews an
  outline without a distance transform: the half-way contour of a picture blurred by Border
  sits exactly where the edge was, and the ramp either side of it is Border wide. That is
  the shipped §3.8 gaussian used as plumbing for the second time — §3.43's softening was the
  first — and it is the reason this effect is `moderate` rather than a research project. The
  companion decision is that **the noise's wobble is weighted by that same band**: without
  it, one low octave punches a hole in the middle of a solid layer, and with it Border means
  exactly what its name says.
  **The whole batch took the generic strength matte.** None of the five wants the matte as
  its *subject* (§3.44's and §3.49's role) and none is a field-painting effect (§3.38's), and
  each already carries its own spatial envelope — Ripple's Radius, Wave warp's Pinning,
  Bezier warp's and Warp's frame-wide geometry, Roughen edges' Border. What a matte is for on
  these is saying *where the effect is*, which is what a dissolve says. To paint a warp's
  strength per pixel, §3.38 is still the effect that does it.
  Four deliberate divergences from AE, all recorded in docs/08: **Ripple's three lengths and
  Wave warp's two are px@comp** (% diag until K-419) (§2.3, §3.37 decision 1's reasoning a fourth
  and fifth time); **Wave warp ships all eight Pinning combinations** where §3.38 ships four
  and reports the rest, the ramp being per edge here rather than per axis; **Warp's thirteen
  styles are Lumit's own curves** under AE's names, a look-for-look conversion the import
  reports as mapped, with AE's Shell Lower, Shell Upper and Warp Axis skipped; and **Roughen
  edges' seven AE edge types become three plus a switch** (§3.57 decision 2), which is
  lossless in both directions and animatable in a way a seven-way dropdown is not. One
  general note that is not a divergence: **Warp's five swelling styles subtract their
  coefficient**, because a gather that reads further out shrinks the picture, and a style
  called Bulge that pinched at a positive Bend would be a bug wearing a name.
- **Wave 2, Distort I (2026-08-20)** — Corner pin (§3.48), Displacement map (§3.49),
  Polar coordinates (§3.50), Twirl (§3.51) and Spherize (§3.52), with their WGSL kernels,
  CPU oracles, manual pages and docs/11 seed-table entries. **Wave 2's first batch, and the
  catalogue stands at 58.** Four things came out of it beyond the five effects.
  **Displacement map is the seventh matte override** (K-395), and the second — after Set
  matte — for which the matte is the effect's *subject* rather than a modifier of one: the
  layer on the Matte row **is** the displacement field. That also disposes of AE's
  Displacement Map Behaviour entirely, since the matte carriage renders the referenced
  layer at this raster and "stretch to fit" is the only fitting there has ever been.
  **A hazard in the shared bilinear tap was found and fixed, and it is worth knowing
  about**: every distort kernel carries a copy of `tap`, which guarded the coordinate and
  then early-returned before `textureLoad`. The load is side-effect-free, the compiler
  hoists it above the branch, and on this Windows backend the hoisted out-of-range fetch
  comes back with a *live alpha lane* — so a pixel whose four taps are all outside the
  frame arrived opaque-and-wrong instead of empty. Polar coordinates is simply the first
  kernel whose samples leave the frame in bulk, which is why it surfaced there. The five
  new kernels clamp the coordinate and choose the value afterwards (`select`), which has no
  such hazard and costs one instruction; **the older copies of the pattern have not been
  touched and should be** (mirror, tile, lens distort, drop shadow, transform, shake and
  the blur family all carry it). **Two more exact-inverse pairs**, both proved by test
  rather than asserted: Polar coordinates' two directions, and Spherize's bulge and pinch
  (`asin` and `sin`, which invert one another where a negated coefficient would not). The
  §3.42 precedent now has three siblings. And **Spherize needed the §3.42 short-circuit for
  a new reason**: at Bulge 0 the blend leaves the sample scale at `ρ ÷ ρ`, which this
  backend compiles as a reciprocal-multiply and answers a hair under 1 — a whole picture of
  resampling for an effect the user has turned off. Both paths now short-circuit.
  Three deliberate divergences from AE, all recorded in docs/08: **Corner pin gains an
  Edges control** (§3.48 decision 5) defaulting to AE's only behaviour; **Displacement
  map's Amounts are lengths in px@comp** (§3.49, §3.38 decision 5's reasoning a third
  time); and **Spherize's single signed Radius becomes a length plus a signed Bulge**
  (§3.52's fourth note) — a negative length cannot also be resolution-independent.
- **Utility and transition batch (2026-08-20)** — Drop shadow (§3.43), Set matte (§3.44),
  Channel blur (§3.45), Linear wipe (§3.46) and Radial wipe (§3.47), with their WGSL
  kernels, CPU oracles, manual pages and docs/11 seed-table entries. **Wave 1 is complete
  with it.** Three things came out of it beyond the five effects, all recorded as K-400.
  **An eighth category, Transition**, for the two wipes and for the Tier B wipes that will
  join them — the same bar K-398's Generate cleared, and the same one-variant cost.
  **Set matte is the sixth matte override**, which resolves this note's own open question
  above: the matte *is* the alpha, so an effect that dissolved its result would produce a
  frame with a faint ring in it rather than a frame cut to the matte's shape, and the
  proof is a picture. And **the wipes are judged on K-399's metric extended one step** —
  their real output is a *threshold on a position*, which magnifies a fused multiply-add
  exactly as a sample position does. Two reuses worth noting for the next batch: Drop
  shadow's softening is the shipped §3.8 gaussian called twice (the blur and the offset
  commute, so no resample is needed), while Channel blur needed a kernel of its own —
  four radii cannot share one weight table, and widening the shipped blur's uniform would
  have risked its byte-for-byte guarantee for nothing.
- **Distort batch (2026-08-20)** — Turbulent displace (§3.38), Tile (§3.39), Offset
  (§3.40), Mirror (§3.41) and Lens distort (§3.42), with their WGSL kernels, CPU oracles,
  manual pages and docs/11 seed-table entries. Three things came out of it beyond the five
  effects. **The noise core's WGSL half became a file of its own**
  (`crates/lumit-gpu/src/fx_noise_core.wgsl`), prepended to both `fx_fractal_noise.wgsl`
  and `fx_turbdisplace.wgsl` at pipeline build — WGSL has no `include`, and two copies of
  a hash that must agree to the bit is exactly the arrangement the module was created to
  avoid. **Turbulent displace is the fifth matte override** (K-395, recounted in K-399): its matte scales the
  displacement vector, which is the owner's own example, and it is proved by picture as
  well as by ULP (a quarter matte must move a pixel about a quarter as far, which a
  dissolve cannot do). And **the batch's oracles are judged on absolute difference over a
  smooth corpus, not fp16 ULPs over the hard-edged one** — every effect here computes a
  *sample position*, and where the two paths contract a multiply-add differently the
  position moves in its last bits, which a hard edge magnifies into a whole pixel of
  colour. Offset alone kept the ULP metric: its arithmetic has no expression to fuse. Both are recorded as K-399.
  Two deliberate divergences from AE, both recorded in docs/08: **Turbulent displace's
  Amount is a length in px@comp** (§3.38 decision 5, §3.37 decision 1's reasoning again),
  and **Tile's default tiles** (§3.39) where AE's is the identity, per §1.2.
- **Generate batch (2026-08-20)** — Fill (§3.34), Gradient (§3.35), Noise (§3.36) and
  Fractal noise (§3.37), with their WGSL kernels, CPU oracles (worst 1 fp16 ULP measured
  across the whole sweep, the noise core included), manual pages and docs/11 seed-table
  entries. Two things came out of it beyond the four effects. **K-398 added a seventh
  category, Generate**, because three of the four never read the incoming picture and no
  existing category describes that. And the **noise core is a module of its own**
  (`lumit-core/src/fx/noise.rs` and its WGSL twin): seeded 3-D value and Perlin noise plus
  the fractal sum, written once because Turbulent displace steers by the same field.
  Two deliberate divergences from AE, both recorded in §3.37: **Scale is a length in
  px@comp**, not a per cent of an unnamed base, so it survives a resize as §2.3 requires;
  and the **evolution depth is not scaled by octave frequency**, which makes Cycle an
  exact loop at any Complexity and stops the fine octaves boiling faster than the coarse
  ones. All four took the generic strength matte — a generator dissolved by a matte is
  exactly a generator drawn where the matte is bright, and Noise is a grain the same
  reasoning covers.
- **Colour batch (2026-08-20)** — Curves (§3.30), Levels (§3.31), Brightness (§3.32) and
  Hue and saturation (§3.33), all four with their WGSL kernels, CPU oracles (worst 1 fp16
  ULP measured), manual pages and docs/11 seed-table entries. Two decisions came out of
  it: K-396 (Curves stores five fixed knots a channel, because AE's point blob has no
  animatable form here) and K-397 above. **K-396 has since been superseded by K-412**
  (2026-08-22): Curves stores a real point list — 2 to 16 points a channel on five
  channels including Alpha, static for AE's own reason — so the import's ceiling rose
  from a five-point sample to the whole curve, and Levels grew the histogram behind its
  handles (K-413). The row itself is unmoved: the blob is still unreadable through the
  Bridge, so Curves still imports as a placeholder. The four **took the generic strength matte**:
  none of them wanted the matte inside its maths, since a colour grade dissolved by a
  matte is exactly a colour grade applied where the matte is bright.

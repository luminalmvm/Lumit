# Next features — the implementation plan

**Status: living.** The implementation companion to [TODO.md](TODO.md) for the next
tranche of work: what to build, in what order, and exactly how — so a session can pick an
entry up cold. Delete an entry when it lands (its regression tests are the record, per
[14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)); when an entry changes a spec or
reverses a decision, that edit and the [02-DECISIONS.md](02-DECISIONS.md) entry land in
the same commit as the code.

In plain terms: this is the "how" file for the work the backlog only names. The two
biggest entries — the lens flare rework and lights — came out of a research pass
(2026-08-12) over how the industry actually does this on real footage; the sources are
cited inline so the reasoning can be checked rather than trusted.

Standing obligations every entry inherits (they are why the estimates are not smaller):
each feature lands with its regression tests, its `app_en.arb` strings (plus
`engine_labels.dart` for anything the engine sends — the `engine_labels_test.dart` gate),
its GUIDE.md plain-English section, and its spec/decision edits, all in the same change.
New effects follow [08-EFFECTS.md](08-EFFECTS.md) §2's contract (CPU oracle beside the
WGSL kernel, bit-stable draw order, px@comp point parameters — K-260).

---

## 0. First, the standing blocker: the flare is not bit-stable

Before any flare work below, the existing regression in TODO's **Now** must be
understood: `wgsl_lens_flare_matches_the_cpu_frame_reference_and_neutrals` fails its
"GPU lens flare must be bit-stable" assertion on clean `main` — two runs of the same
flare give different pixels. Every flare entry below adds passes to that pipeline, and
none of them can be validated honestly while the baseline itself wobbles. Find which
stage varies (bisect by reading back after each pass: bright-pass → trace → blur → draw)
before changing anything. [impl/lens-flare.md](impl/lens-flare.md) §2.4 explains why the
additive draw order is the property everything else protects.

---

## 1. Lens flare v2 — flares that follow a light, not a threshold

**The problem, precisely.** The shipped flare (docs/08 §3.27) is screen-space: a
bright-pass finds hot pixels, ghosts are traced from them. On graphics that is fine; on
*footage* it is the wrong tool, and the research is blunt about why — any bright pixel
(sky, white shirt, specular glint) spawns ghosts, and pixels crossing the threshold
frame to frame make the whole flare **flicker**. Even the technique's author recommends
sprite-based flares for prominent light sources
([Chapman](https://john-chapman.github.io/2017/11/05/pseudo-lens-flare.html)). What
Video Copilot's Optical Flares actually is, conceptually: **no bright pass at all** — a
user-supplied 2D light position drives a designer-authored stack of elements placed
along the line from the light through frame centre
([Optical Flares](https://www.videocopilot.net/products/opticalflares)). Deterministic on
video, zero flicker, art-directable. That is the single biggest "usable with footage"
win available.

**What already exists to build on** — more than the pitch above implies, so read
[impl/lens-flare.md](impl/lens-flare.md) first (`crates/lumit-gpu/src/fx/lens_flare.rs`):

- **Manual mode is already position-driven** — a px@comp pair (K-260) drives the whole
  physically-traced flare, so the deterministic-on-footage workflow half-exists today;
  what it lacks is art-directable *elements* around the physical ghosts.
- **The starburst already exists and is already baked** per lens/aperture; the owed
  **image aperture file** parameter (TODO's flare follow-ups) plugs into that bake
  rather than needing a new mechanism.
- The source-mode enum already reserves `2 = Lights` "until light layers land (K-257)";
  `MAX_LIGHTS = 16` and the `GpuLight` layout mean every element below runs per light
  for free on the existing dispatch axis.
- The lens designer and `lens_file` (K-264) — where element-stack presets live.

**The build, in two steps:**

1. **Element stack in the lens file.** Beside the traced ghosts, a flare gains a list
   of authored elements, each: `kind` (glow, iris polygon, ring/halo, streak), `offset`
   (signed position along the light→centre axis: 1.0 = on the light, −1.0 = mirrored
   across centre — the Optical Flares model), `scale`, `tint`, `opacity`. Data, not
   shaders: one draw pass of N procedural quads, trivially cheap next to the ray
   tracer, feeding the same K-289 combine. Keep the additive order fixed (element
   index, never anything measured) so §2.4's bit-stability holds by construction.
2. **The one genuinely new kernel: the anamorphic streak** — the Kawase streak filter:
   downsample ~1/16, then 3–4 passes each sampling 4 taps along the streak direction at
   distance 4^pass, weight `a^(b·dist)`, attenuation ~0.9–0.95
   ([Oat, scene postprocessing](https://www.chrisoat.com/papers/Oat-ScenePostprocessing.pdf)).
   A directional variant of the blur passes already in the file.

Sources stay as they are: Manual (soon: Lights, entry 3) is the footage workflow;
Matte keeps the detector for graphics, hardened by entry 2. Full lens-prescription
ray tracing beyond what the bake already does (Hullin 2011) is ~12 ms/frame class —
wrong genre; do not chase it.

**Files:** `lens_flare.rs` + its WGSL siblings, `lumit-core/src/fx/lens_flare.rs`
(params), the lens-file schema (K-264), Effect controls rows (the pair row's dropper on
px@comp pairs is already owed — K-260). **Spec edits:** docs/08 §3.27 gains the element
stack; [impl/lens-flare.md](impl/lens-flare.md) gains a §for it (algorithms above, test
plan below); a K-entry records the design.

**Test plan:** CPU oracle for each element kind at a known position (docs/08 §2 pattern);
bit-stability (two renders, identical bytes); a two-frame "video" fixture where a bright
region moves — assert the element stack's output moves *continuously* (no threshold pop);
the baked starburst keyed by aperture hash (same aperture → cache hit, no re-FFT).

**Size:** the biggest entry here — the element system is real design work. Land it in
slices: streak element first (pure shader work, immediate visual payoff), then the stack,
then the baked starburst.

## 2. Harden Matte mode's detection on footage (small, do early)

Matte mode already has half of what the literature asks for: the luma gate is soft
(`threshold` + `threshold_softness`, `lens_flare::threshold_gate`), and K-267's tile
flux summing already weighs an area source as its area rather than one pixel. What
still flickers on video, and the fixes
([Froyok's UE writeup](https://www.froyok.fr/blog/2021-09-ue4-custom-lens-flare/),
[LearnOpenGL PBB](https://learnopengl.com/Guest-Articles/2022/Phys.-Based-Bloom)):

1. **Fireflies** — a single hot pixel (sensor sparkle, specular glint) rides the tile
   sum and pops a source for one frame. The Karis-average idea, adapted to the
   detector: weight each pixel's contribution to its tile's flux by `1/(1+luma)`, so
   one outlier cannot own the anchor. Same formula in `detect_lights` (the CPU twin)
   and the WGSL reduction, or the matte-mode frame oracle fails — which is the test.
2. **Anchor jumping** — a source's anchor position quantises to its brightest pixel,
   which wanders inside the practical frame to frame. A flux-weighted centroid over
   the anchor's tiles (still deterministic, still index-ordered summation per §2.4)
   steadies the position without any cross-frame state.

**Temporal smoothing is a recorded non-option**, not an oversight: a frame must be a
function of the document and the frame alone (docs/14 determinism; the caches name
frames on exactly that promise), and a detector that remembers the previous frame
breaks random access, export/preview identity and the frame oracle in one move. The
footage answer to "the threshold pops" is the Manual/Lights element path (entries 1
and 3), not history buffers.

This entry must not land before the bit-stability blocker (entry 0) is understood —
it edits the passes under suspicion, and would muddy the bisect.

## 3. Light layers, and area lights via LTC

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

## 4. Light wrap — the cheapest "light meets footage" feature that exists

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

## 5. Viewer bar completion — the two owed halves of what just landed

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

## 6. Region of interest (docs/07 §2.2 item 7)

Drag a rectangle; the engine composites only that region. The realiser already
composites at a scaled raster (K-186 / the preview-scale work), so the mechanism is a
scissor/viewport on the composite target plus an offset in the present — not a new
pipeline. One-click clear; never affects export (same construction as the preview
scale: the export renderer never receives it). Frame names must fold the region in
(the K-346/K-352 mechanism — a cropped frame is not the full frame) or refuse names
while a region is set; folding is better, scrubbing inside a region is the use case.

---

## Suggested order

| # | Entry | Why this position |
|---|-------|-------------------|
| 1 | 0 — flare bit-stability | Blocks honest validation of 1 and 2 |
| 2 | 2 — bright-pass hardening | Small, immediate payoff on footage, de-risks 1 |
| 3 | 5 — viewer bar completion | Small, finishes surfaces this week opened |
| 4 | 1 — flare element stack | The headline; slice it (streak → stack → starburst) |
| 5 | 3a/3b — Light layer + flare wiring | Model decision first, wiring is cheap |
| 6 | 4 — light wrap | Anytime after its below-stack plumbing exists |
| 7 | 3c — LTC area lights | The deep cut; lands on a model already proven by 3b |
| 8 | 6 — region of interest | Independent; whenever the Viewer is next open |

Skipped deliberately, so they are not re-proposed: per-frame FFT diffraction and full
lens simulation (offline-class cost), temporal history buffers for flare smoothing
(tracked positions make them unnecessary), ML relighting (non-deterministic, wrong
weight class), representative-point sphere/tube lights (LTC covers the rect case a
compositor actually needs; add spheres only if a use case shows up).

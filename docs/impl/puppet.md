# The Puppet tools (K-704)

**Status: PU1 built (`lumit_core::puppet`); PU2 and PU3 outstanding.**
[07-UI-SPEC.md](../07-UI-SPEC.md) §1.7 names the four tools — Puppet position pin, Puppet
starch pin, Puppet overlap pin, Puppet bend pin — and K-228 keeps them on the strip,
disabled, until there is an engine behind them. This note is the binding *how* for that
engine: the mesh, the deformer, the storage, the render seam, the overlay, the refusals, the
tests, and the ordered work packages PU1–PU3. The mesh, the two-step solve and the warp
function exist and carry §7's tests 1–12; nothing is stored, wired to the render seam, or
reachable from the UI yet.

## In plain terms

You have a picture of a character — a cutout with an arm, say — and you want to wave the
arm without cutting the layer apart. Puppet does it in three steps. First it lays a **mesh**
of small triangles over the opaque part of the picture, like chicken wire bent to the
silhouette. Then you place **pins**: a pin says "this spot of the picture is under my
thumb". When you drag a pin (or keyframe it), the engine moves the mesh so that the pinned
spots follow your thumbs while every triangle in between tries as hard as it can to keep
its own shape — to turn and slide rather than stretch. That "tries to keep its shape" is
the whole trick; it is what makes the arm *bend at the elbow* instead of smearing like
taffy. Finally the picture is redrawn through the moved mesh: each triangle carries its
patch of pixels to wherever it ended up.

The other three pins season that basic move. A **starch** pin stiffens a region, so a
torso stays rigid while the limbs bend. An **overlap** pin says which part draws in front
when the picture folds over itself, so a hand crossing the body passes in front of it
rather than tearing. A **bend** pin rotates and scales a region about itself, so a hand
can wave from the wrist without the wrist travelling.

## 1. The mesh

### 1.1 What the mesh is built from

The mesh is built over the layer's **own rendered picture at natural size**, at the puppet
block's **reference time** — the layer time at which the first pin was placed — with paint
and masks already applied, because a mask gates the picture and the mesh should cover what
the mask leaves ([paint.md](paint.md) shows the same closure in `lumit-render`'s
`build.rs`; PU2 factors an engine helper that renders one layer solo this way). Coverage is
the alpha channel of that buffer.

- **Coverage iso-value: alpha ≥ 25** (10%), not 128. Faint content — smoke, glow, soft
  antialiased edges — is still content, and a mesh cut at 50% alpha drops the fringe of
  everything. Content fainter than 10% alpha falls outside the mesh and does not deform
  (and, because the warp draws only what the mesh covers, does not draw); this is the
  recorded trade, same as After Effects' threshold, and the constant is not exposed in v1.
- **Expansion: default 3 px**, a parameter on the puppet block. The coverage is grown
  outward by this distance before contour extraction, using the same distance-field
  machinery `mask.rs` already uses for mask expansion (a pixel is covered when its distance
  to the iso region is ≤ expansion). Expansion is why edge pixels sit comfortably *inside*
  the mesh rather than on its knife edge.
- **Density: default 24 px**, a parameter on the puppet block: the target triangle edge
  length, in pixels at natural size. Smaller = more triangles = suppler and dearer.

### 1.2 Boundary extraction: marching squares

Run **marching squares** over the expanded coverage bitmap at iso 127.5 of the expanded
soft coverage (after boolean expansion the crossings sit at cell midpoints; where soft
values survive, linearly interpolate the crossing along the cell edge). Cell corners are
pixel centres. The **saddle** cases (two opposite corners above, two below) are resolved by
the average of the four corners: average above the iso joins the higher-valued pair. This
yields a set of **closed, non-self-intersecting polylines** — outer silhouettes and holes
alike — and, being level-set contours, no two of them cross. Contours are emitted in scan
order (top-to-bottom, left-to-right discovery), which is the determinism anchor for
everything downstream.

Then simplify each contour with **Douglas–Peucker at tolerance 1.25 px**, and afterwards
collapse consecutive vertices closer than density/4 along the contour, so constraint edges
do not force slivers into the triangulation.

**Trap: simplification can introduce intersections.** Raw level-set contours never cross,
but two simplified ones (or one and itself, around a narrow neck) can. If constraint
insertion (below) reports an intersection, retry that build with the Douglas–Peucker
tolerance halved, down to a floor of 0 — the raw contours, which cannot intersect. Never a
panic (docs/14).

### 1.3 Triangulation: constrained Delaunay via `spade`

**Cargo.lock first:** the workspace has no triangulation crate, and writing a robust
constrained Delaunay with quality refinement in-tree is exactly the wheel not to reinvent.
**Pin: the `spade` crate** (MIT/Apache-2.0, pure Rust; its dependencies — `smallvec`,
`robust`, `num-traits`, `hashbrown` — are all already in the lockfile), added to
**`lumit-core`**, where `mask.rs` and the new `puppet` module live.

1. Insert every simplified contour's edges as **constraint edges** into a
   `ConstrainedDelaunayTriangulation<Point2<f64>>`, contours in discovery order, vertices
   in path order (fixed insertion order = deterministic output).
2. **Refine** with spade's angle limit at its guaranteed-termination default (30°) and a
   maximum triangle area of `density² · √3 / 4`, so interior edges come out near the
   density.
3. **Discard outside triangles** by sampling: a triangle is kept iff the expanded coverage,
   bilinearly sampled at its centroid, is ≥ 127.5. Holes and the region outside the
   silhouette fail the sample and fall away, and disjoint blobs come out as disjoint mesh
   components (which is load-bearing: two limbs separated by a gap must never be welded —
   this is the reason for a conforming mesh rather than a clipped grid).

   The centroid sample is *first* filtered by the faces spade's own flood fill already
   called outer (`exclude_outer_faces`, whose result the refinement hands back). The note
   originally said the centroid alone, with no outer classification, and §7's weld test is
   what corrected it: two blobs whose silhouettes share a straight edge — two legs on the
   same ground line — leave the hull triangulator a long thin sliver bridging the gap,
   unrefined because it is outside, and that sliver's centroid sits a fraction of a pixel
   *inside* the shared edge. It survives the sample and welds the two components. Using
   spade's classification costs one flood fill it was going to run anyway (excluding those
   faces from refinement is what keeps the vertex budget on the silhouette), and the
   decision is still made by the same closed contours.
4. Re-index kept vertices densely, in first-use order over kept triangles sorted by their
   spade creation order. The mesh is `vertices: Vec<[f64; 2]>` (layer px) +
   `triangles: Vec<[u32; 3]>`.

**Vertex cap: 1500, by auto-coarsening.** If refinement produces more than 1500 vertices,
double the area bound and re-refine, up to five times, then refuse the build (a
full-frame layer at fine density is the pathological case; puppet is a cutout tool). The
cap exists because the deformer's factorisation is dense (§2.5) — the cap is its budget.
Auto-coarsening changes the *effective* density, not the stored parameter.

### 1.4 The mesh cache

The mesh is **never stored in the project file**. The puppet block stores only reference
time, density, expansion and the pins; the mesh is rebuilt on demand and cached by
**`blake3(coverage bitmap bytes ‖ density bits ‖ expansion bits)`** (blake3 is already in
the lockfile). This is the forward-compatibility pin: a future, better triangulator changes
nothing in any saved project, because no project ever contained a triangle. Pins are stored
as positions in layer px; their binding to the mesh (containing triangle + barycentric
coordinates, §2.2) is computed after each mesh build, never stored.

If the source changes so that the alpha at the reference time changes, the hash misses, the
mesh rebuilds, and every pin re-binds by position. A pin whose position now falls outside
the new mesh goes **inert** — kept in the document, drawn hollow in the overlay,
contributing nothing to the solve — never silently deleted (§6).

## 2. The deformer

### 2.1 The formulation, pinned

**Igarashi, Moscovich & Hughes 2005, "As-Rigid-As-Possible Shape Manipulation"** — the
registration-free, **two-step, closed-form** variant. Not the iterative local/global ARAP
(Sorkine 2007): the 2005 method is two sparse linear solves with **no iteration budget at
all**, its matrices depend only on the rest mesh and on *which* points are pinned (not
where they are), so they are factored once and reused every frame, and it is the algorithm
the trade's puppet tools are built on. Determinism is structural: fixed assembly order,
f64 throughout, no data-dependent iteration counts.

- **Step 1 — similarity.** For each triangle, each vertex is expressed in the local frame
  of its opposite edge in the *rest* pose: `v2 = v0 + x·(v1−v0) + y·R90·(v1−v0)` with
  `R90` the quarter-turn. The error is the squared distance between each deformed vertex
  and that same expression over the deformed edge, summed over all three rotations of
  every triangle. This is quadratic in the deformed coordinates; minimising it is one
  sparse SPD system `G v' = b` over all 2n coordinates. It permits per-triangle
  *similarity* transforms — rotation is free, but so is scale.
- **Step 2 — scale adjustment.** Per triangle, fit the similarity transform carrying the
  rest triangle onto its step-1 image, divide out its scale factor to get a pure rotation,
  and build the "fitted" triangle: the rest triangle rigidly rotated to the step-1
  orientation. Then minimise the squared difference between the deformed edges and the
  fitted triangles' edges. The x and y coordinates decouple into two n×n SPD systems with
  the same matrix. This step is what stops limbs inflating as they bend.

### 2.2 Constraints

All pins enter both steps as **soft constraints, weight w = 1000**, one row per constrained
point: the point is the **barycentric combination** of its containing triangle's three
vertices (a pin sits anywhere in a triangle, not on a mesh vertex), the target is the pin's
animated position at the frame. Soft constraints keep the systems SPD and unchanged in
*structure* when pin values move — only the right-hand side changes per frame.

- **Position pin**: one constrained point at the pin, target = its `x`/`y` Properties at
  the frame.
- **Bend pin**: constrains its own position like a position pin, *plus* every mesh vertex
  within its **extent** (default 50 px, rest-pose distance) gets a soft target
  `p_now + s·R(θ)·(v_rest − p_rest)`, weighted `100 · falloff` where falloff runs linearly
  1 → 0 across the extent. Weight 100, not 1000: a position pin inside a bend region must
  win the argument. θ (degrees) and s (%) are the pin's animatable rotation and scale.
  Which vertices are in the extent is a rest-pose fact, so the matrices still factor once.
- **Starch pin** is not a constraint but a **per-triangle weight**: each triangle's error
  terms in both steps are multiplied by `w_t = 1 + 9 · influence(centroid)`, where
  `influence = amount · falloff(rest distance / extent)`, amount 0..1, linear falloff,
  and multiple starch pins combine by **max** (not sum — three overlapping pins should not
  stack to absurd stiffness). Starch amounts are animatable; a change in starch changes the
  matrices, so it invalidates the factorisation (§2.5) — starch is normally set once, and
  the cost is honest.
- **Overlap pin** does not touch the solve at all; it feeds the warp's draw order (§3).

### 2.3 Degenerate pin counts

- **No pins, or all inert**: identity — the warp is skipped entirely and the layer draws
  untouched.
- **One pin**: the similarity step is underdetermined (global rotation about the pin is
  free). Do not solve: translate by the pin's delta from rest. This is also what the user
  meant.
- **Two or more pins**: the full two-step solve.

These counts are **per mesh component**, not per mesh, and the count is of *constrained
points* rather than of pins (a bend pin contributes its own point plus one per vertex in
its extent, so a lone bend pin is not a translation). A component nothing constrains stays
exactly at rest, which is what makes the weld test exact rather than merely close: an
unconstrained component in one global system would be a singular block, and a singular
block is a Cholesky failure and a mean-translation fallback that moves the far limb. So
only the vertices of components carrying two or more constrained points enter the systems;
the rest are filled in directly.
- **Every pin exactly at its rest position**: identity, checked before solving, so an
  untouched puppet block is byte-for-byte a no-op (§7 test 12).

### 2.4 The solver

Dense Cholesky, in-tree — the ~40-line factor/solve pair `lumit-track`'s `bundle.rs`
already contains is the pattern (it is `pub(crate)` there; the puppet module carries its
own copy in `lumit-core` rather than growing a cross-crate seam for forty lines). At the
default density a mesh is ~200 vertices → a 400×400 factor, well under a millisecond; at
the 1500-vertex cap the 3000×3000 factor costs on the order of a second, once.

<!-- ponytail: dense Cholesky, O((2n)³) at (re)factorisation — a sparse factorisation
     (the matrices are mesh-Laplacian sparse) is the upgrade when capped-out meshes make
     pin placement feel sticky. Observable trigger: factorisation time > 250 ms in the
     PU1 bench at default density, or users hitting the 1500-vertex cap in anger. -->

A failed factorisation (not positive definite — numerically conceivable with pathological
starch weights) falls back to translating the mesh by the mean pin delta, never a panic
(docs/14). Factorisation runs as a cancellable engine job like any other render work.

### 2.5 Caching

Two cache levels, both keyed deterministically:

- **Factorisation cache**: key = (mesh hash, sorted active-pin ids + their kinds +
  extents, starch amounts at the frame quantised to f64 bits). Hit = per-frame work is
  RHS assembly plus three back-substitutions (one 2n, two n) — microseconds to a
  millisecond at the cap.
- **Frame cache**: the puppet block — density, expansion, reference time, and every pin's
  evaluated values at the frame — **feeds the frame cache key** (unlike Volume and Pan,
  which are sound: puppet is pixels). The solve is deterministic, so cached frames stay
  honest.

## 3. The render seam

**CPU warp for v1**, in `lumit_core::puppet::apply_puppet(&mut rgba, w, h, natural_w,
natural_h, block, mesh, solved, lt)`, called from `lumit-render`'s `build.rs` in the same
closure as — and immediately after — `apply_strokes` and `apply_masks`: the layer's pixels
are already in hand there, in a CPU buffer at whatever size this frame renders at, which is
exactly what a warp wants and why the paint rasteriser lives at the same seam. A GPU warp
is deliberately not v1: it would be the only GPU stage at a CPU seam, and the CPU cost
(§below) fits the budget.

<!-- ponytail: CPU warp — the GPU version (vertex buffer of the deformed mesh, one draw)
     is the upgrade. Observable trigger: the PU2 bench scenario exceeding its 8 ms gate,
     or puppet layers showing up in docs/13 budget traces. -->

The warp, exactly:

1. Copy the buffer (source); clear the original to transparent (destination).
2. Scale solved vertex positions from layer px to buffer px with the per-axis factors
   `w/natural_w`, `h/natural_h` (the same convention paint uses).
3. Sort triangles by **overlap depth** — per-vertex depth = Σ over overlap pins of
   `amount · falloff(rest distance / extent)` (amount −1..1, extent default 50 px, linear
   falloff), triangle depth = mean of its three, stable-sorted ascending by (depth,
   triangle index) so ties are deterministic. More "in front" draws later: painter's
   algorithm, which is all a self-overlapping single layer needs.
4. For each triangle, scanline-fill its *deformed* footprint with edge functions; at each
   pixel, barycentric coordinates give the rest-pose position; bilinearly sample the
   premultiplied source there and write over the destination. A degenerate deformed
   triangle (|area| < 1e-9 px²) is skipped, so crossed pins draw an honest fold rather
   than NaNs.

Cost: one bilinear-resample pass over the covered pixels — the same order of work as a
paint stamp or a mask apply at the same size. **Budget, gated in `lumit-bench` (PU2): a
1920×1080 layer fully covered at default density warps in ≤ 8 ms single-threaded on the
CI baseline machine; mesh build (extraction + simplification + triangulation) ≤ 100 ms at
natural 1080p; per-frame solve ≤ 1 ms at the vertex cap.**

## 4. Storage

On `Layer`, beside `masks` and `paint`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub puppet: Option<PuppetBlock>,

pub struct PuppetBlock {
    /// Layer time the mesh's alpha is taken at: when the first pin was placed.
    pub reference_time: /* rational layer time, same type masks key on */,
    /// Target triangle edge, px at natural size.  Default 24.
    pub density: f64,
    /// Coverage growth before meshing, px.  Default 3.
    pub expansion: f64,
    pub pins: Vec<PuppetPin>,
    #[serde(flatten)] pub extra: serde_json::Map<String, serde_json::Value>,
}

pub struct PuppetPin {
    pub id: Uuid,
    pub name: String,
    pub kind: PuppetPinKind,       // Position | Starch | Overlap | Bend
    pub x: Property,               // px, layer space — like masks and paint,
    pub y: Property,               //   and px because point params are px, never %
    /// Bend only: degrees / percent (100 = natural).  Serde-defaulted.
    pub rotation: Property,
    pub scale: Property,
    /// Starch: 0..100.  Overlap: −100..100 (in front / behind).  Serde-defaulted.
    pub amount: Property,
    /// Starch / overlap / bend falloff radius, rest px.  Default 50.  Not animatable.
    pub extent: f64,
    #[serde(flatten)] pub extra: serde_json::Map<String, serde_json::Value>,
}
```

One puppet block per layer in v1 (After Effects allows several meshes per layer; nothing
drawn in the mockups needs it, and a second block is an additive file-format change when it
does). Every animatable field is an ordinary `Property` — the same stopwatch, lanes,
diamonds and graph as everything else, no new animation machinery. Unknown fields ride in
`extra`, the file-format §1.1 rule.

AE import: docs/11 already preserves AE puppet pins in the `ae` namespace "for a future
engine". Mapping them onto this block is a follow-up once PU1–PU3 stand — the meshes will
not match (different triangulators), which is exactly why pins are positions, not vertex
indices.

## 5. The overlay, the tools, the panel

- **The four strip tools come alive** (they exist, disabled, under K-228; the keymap's
  `tool.puppet` actions exist). Tool options area, per K-225's table: **Density** and
  **Expansion**, live; committing a change re-meshes.
- **Overlay** (Viewer, while a puppet tool is armed): the **mesh ghost** — the deformed
  mesh's wireframe, theme-coloured, thin — and the **pins**: position pins as filled dots,
  starch/overlap/bend each with their glyph per 15-DESIGN, inert pins hollow. Extent for
  starch/overlap/bend drawn as a faint circle while that pin is being dragged.
- **Gestures**: click with a pin tool adds that kind of pin at the click (building the
  mesh first if this is the first pin — that click sets `reference_time`); dragging a pin
  moves its x/y at the playhead, through the ordinary property/keyframe rules (stopwatch
  on = a keyframe lands). Dragging with the bend tool on a bend pin rotates (θ from the
  drag angle about the pin); Alt-drag scales. Delete removes the pin under the cursor.
  All edits are document commands — the same undo plumbing as mask vertices.
- **Timeline**: the puppet block is a group row under the layer (as masks are), one child
  row per pin, each with its animatable lanes. Pins are renamed inline like masks.
- Panels follow WP-2's listenable/boundary patterns; no bridge calls in rebuild paths
  (the mesh ghost and pin positions ride the same per-frame overlay data the transform
  gizmos use).

## 6. Refusals and traps

| Situation | Behaviour |
|---|---|
| First pin on a layer with no pixel at alpha ≥ 25 at that frame | Refuse the click, status-line message ("nothing opaque to build a mesh from" — exact string and arb key land with PU3). No block is created. |
| Pin click outside the mesh | Refuse the click, status-line message. Never a floating pin. |
| Source alpha changed → mesh rebuilt → existing pin outside new mesh | Pin goes **inert**: kept, hollow, ignored by solve and warp. Comes back by itself if the mesh grows back. |
| Refinement over 1500 vertices | Auto-coarsen (double area bound, ≤ 5 times), then refuse with a message naming density. |
| Simplified contours intersect | Halve Douglas–Peucker tolerance and retry, floor 0 (§1.2). |
| Cholesky refuses | Mean-translation fallback, never a panic. |
| Pins dragged across each other (fold) | Legal: degenerate triangles skipped, overlap order decides the fold's front. |
| One pin | Pure translation, no solve (§2.3). |
| Content below 10% alpha | Outside the mesh: neither deforms nor draws while a non-identity puppet is active. Recorded trade (§1.1). |

Engine rules apply throughout: no panics (every refusal above is a value, not an unwrap),
rational reference time, allocations budgeted at mesh build (the per-frame path allocates
only the destination copy), factorisation cancellable, everything deterministic (fixed
iteration orders, stable sorts, f64, no HashMap iteration in assembly).

## 7. Test plan

Synthetic alpha shapes with hand-checkable deformations, in `lumit-core`'s puppet tests
(PU1 unless marked):

1. **Rectangle contour**: a filled axis-aligned rectangle → one contour; after
   simplification, 4 corners within tolerance (marching squares chamfers a right angle
   across the corner cell, so "within tolerance" is one cell's diagonal, not zero).
2. **Ring**: rectangle with a hole → two contours; no kept triangle's centroid inside the
   hole.
3. **Two blobs, the weld test**: disjoint blobs → disjoint components; dragging a pin in
   blob A moves no vertex of blob B, exactly.
4. **Determinism**: same alpha, density, expansion, twice → byte-identical vertex and
   triangle lists; same pins → bit-identical solve.
5. **Rigid reproduction**: two pins on a bar, both translated by the same delta → every
   vertex within 1e-6 of rest + delta.
6. **Rotation**: two pins, one rotated 90° about the other → vertices within 0.5 px of the
   rigid rotation (ARAP recovers a rigid motion when one exists).
7. **Starch**: a bent three-pin bar; with a full-strength starch pin mid-bar, the summed
   deviation-from-similarity of mid-region triangles is strictly less than without.
8. **Overlap**: a bar folded over itself; the pixel at the overlap's centre matches the
   region with the higher overlap amount; swapping the two amounts swaps the pixel.
9. **Bend**: θ on a bend pin turns vertices inside its extent by θ about it, within
   tolerance; a vertex outside the extent moves less than one inside. The far end needs an
   ordinary position pin holding it, or "outside the extent" means nothing: with only the
   bend pin, as-rigid-as-possible correctly swings the whole shape round it and the far end
   travels furthest of all.
10. **One pin** short-circuits to translation.
11. **Refusals**: transparent layer refuses a mesh; a click outside the mesh refuses a
    pin; > 1500 vertices coarsens then refuses.
12. **Identity**: all pins at rest → output buffer byte-identical to input (the early-out).
13. **Cache honesty** (PU2): changing a pin value at a frame changes that frame's cache
    key; changing Volume does not touch it.
14. **Budget** (PU2): the bench scenario of §3 inside its gates, wired into the docs/13
    harness.
15. **Bridge/UI** (PU3): pin add/drag/delete round-trips through the document; undo
    restores; the tools arm only once built (K-228 flips per tool as its behaviour lands).

## 8. Ordered work packages

- **PU1 — engine: mesh + solve** (`lumit-core::puppet`, `spade` added to the lockfile) —
  **built**: coverage → marching squares → simplification → CDT + refinement →
  outside-discard → cap/coarsen; the two-step solve with pins, starch, bend; the in-tree
  Cholesky pair; the mesh and factorisation caches; tests 1–12. Pure engine, no bridge, no
  UI. `apply_puppet` itself lands here rather than in PU2, because tests 8 and 12 are pixel
  tests and a test that cannot run is not a test; what PU2 adds is the *call* at the seam.
- **PU2 — model, seam, bridge**: `PuppetBlock`/`PuppetPin` on `Layer` (serde
  round-trip), the solo-render-at-reference-time helper, the `apply_puppet` call at the
  paint/masks seam, frame-cache key, bench scenario + budget gates,
  bridge API for the block and pins (codegen once), tests 13–14.
- **PU3 — tools, overlay, panel**: arm the four tools (K-228 flip), tool options (Density,
  Expansion), the mesh ghost and pin overlay, gestures and document commands, the Timeline
  rows, every string through `app_en.arb` (refusal messages, pin lane labels, options) —
  arb last, keys listed in the commit; test 15 plus the K-681 redraw and bridge-budget
  gates for the new overlay.

Each package leaves the tree green on its own; PU2 and PU3 must not start ahead of the
package before them, because each consumes the previous one's public seam.

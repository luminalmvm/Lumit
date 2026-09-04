# Roto brush and Refine edge: flow-propagated segmentation

**Built: RB1 (`crates/lumit-roto`), RB2 and RB3.**
[07-UI-SPEC.md](../07-UI-SPEC.md) §1 puts the Roto pair on the
tool strip (Alt+W, armed since RB3) and §2.3.7 the tools themselves; [16-ROADMAP.md](../16-ROADMAP.md) Phase 5 lists
rotoscoping; this note is the binding *how*: the algorithms (pinned, with their ceilings
named), the stroke model, the propagation and correction loop, the sidecar cache and its
invalidation rules, determinism, budgets, refusals, the test plan, and the ordered work
packages RB1–RB3. The real asset this design leans on is `crates/lumit-flow` — the
deterministic optical flow the project already owns
([optical-flow.md](optical-flow.md)) — and the job/sidecar/store architecture the camera
tracker already proved ([tracking.md](tracking.md) §5b, docs/10 §3).

**In plain terms:** rotoscoping is cutting a moving thing out of its shot — making, for
every frame, a greyscale picture (a **matte**) that is white where the subject is and black
where it is not, so the subject can be composited elsewhere or treated separately. Doing it
by hand means drawing the outline on every frame, which is the most tedious job in
compositing. The Roto brush shortens it: the user scribbles a few strokes on one frame —
green-side strokes on the subject, red-side strokes on the background — and the engine
works out the matte for that frame from the colours around the strokes. Then, instead of
asking again on the next frame, it *watches how the picture moved* (the optical flow the
engine already computes for retiming) and carries the matte along with the motion,
re-deciding only the pixels near the edge. Where it drifts wrong, the user corrects with
another stroke on that frame, and the correction carries forward too. Refine edge is the
finishing pass: a narrow band around the matte's boundary where the edge is softened to
match what is actually there — hair, blur, smoke — instead of a hard cut-out line.

## 0. What v1 honestly is, and is not

Adobe's current Roto Brush is a neural network, and nothing classical will match it on
hair, overlapping limbs or translucency. Shipping a network is not a v1 option here: there
is no model Lumit can train tonight, redistributing someone else's inside a GPLv3 native
app raises licence and size questions that deserve their own decision, and inference is
non-deterministic across GPUs and runtime versions, which collides with docs/14's
determinism rules the same way RIFE does ([optical-flow.md](optical-flow.md) §0).

So v1 is the classical machine, stated plainly: **stroke-seeded geodesic segmentation,
propagated frame-to-frame by the in-tree optical flow, with per-frame corrections layered
on, and a guided-filter matting band at the boundary.** On well-separated subjects —
a player model against sky, a talking head against a wall, the high-contrast game footage
this project exists for — it does the job with a handful of strokes. On hair
against similar tones it needs more correction strokes than AE 2024 does, and the note says
so rather than promising otherwise. The neural upgrade is §9, recorded as growth with a
defined seam, exactly as `rife` is for flow.

## 1. The model: strokes are the document, mattes are derived

Two kinds of state, and keeping them apart is the whole design:

- **Strokes** are the user's edit. They live in the document, on a **Roto brush** effect
  instance on the layer ([08-EFFECTS.md](../08-EFFECTS.md) gains the row in RB2): ordered,
  undoable, journaled, saved in the `.lum`. A stroke is
  `RotoStroke { id, points, radius, kind, frame }` — a thinned polyline exactly as paint
  strokes are ([paint.md](paint.md): samples of a gesture, not a designed shape, thinned at
  two screen pixels), with `kind` one of `Foreground`, `Background`, `Refine`, and `frame`
  the **source frame index** it was drawn on. Points are in **source raster pixels** on
  the full, unaltered footage, the stance the tracker takes for the same reason: the
  matte describes the *file's* frames, so it survives every comp-side transform, retime
  and preview tier, and one shot's mattes serve every comp that cuts it.
- **Mattes** are derived data, cached in the sidecar (§5) like camera solves — one gray8
  plane per source frame, at source raster. The project file never references them
  (docs/10 §3's binding rule); deleting the sidecar costs a re-propagation and nothing
  else.

**The base frame.** The first frame the user strokes becomes the effect's **base frame**
(stored on the instance, re-assignable from the panel). Propagation runs **both directions
from the base** to the ends of the analysed span — a user scrubs to a clear, well-separated
frame to start, and that frame is rarely frame 0. Strokes on any other frame are
**corrections**: they join the seeds for their own frame's solve and carry onward *away
from the base*, never back toward it. That single rule — influence flows outward from the
base — is what keeps the cache invalidation (§5) one sentence long.

**The matte at frame F is a pure function of** the media bytes, the effect's roto
settings, the base frame, and the strokes on frames between the base and F inclusive
(on F's side of the base), in document order. Nothing else — not the comp, not the
playhead, not the order of propagation runs. Every cache key and every determinism claim
below is this sentence restated.

## 2. The single-frame solve: geodesic distance transform (pinned)

**The algorithm is the geodesic distance transform** (Bai & Sapiro 2009; Criminisi et
al., "GeoS", ECCV 2008), not graph cut. For each pixel, compute the geodesic distance to
the nearest foreground seed and to the nearest background seed, where a path's cost
accumulates colour change as well as space:

```
cost(step from x to y) = sqrt( ‖y − x‖² + γ² · ‖I(y) − I(x)‖² )
D_F(x) = min over paths to any foreground seed;  D_B(x) likewise
α_raw(x) = D_B(x) / (D_F(x) + D_B(x) + ε)
```

`I` is the frame's **encoded** RGB (the perceptual choice the flow's correlation already
made and measured — [optical-flow.md](optical-flow.md) §1); `γ` weights colour against
space (default such that a full-scale colour step costs ~50 px of travel; a setting, not
a constant). A pixel is labelled by the nearer seed; `α_raw` is already soft near
equidistant boundaries, and §4 shapes the edge properly.

**Computation: Toivanen's raster-scan chamfer passes, a fixed count.** A forward scan
(top-left to bottom-right, each pixel relaxing over its four causal 8-neighbours) and the
mirrored backward scan; **three pass pairs, always** — no convergence test, no priority
queue, no per-pixel allocation. Complexity **O(N)** with a small constant: six passes,
four causal neighbours each, ~10 flops per relaxation, over 2.07 Mpx at 1080p. Deterministic
by construction — a fixed scan order and f32 arithmetic in that order — which is what a
Dijkstra front with a heap and tie-breaking is not without care, and what graph cut's
max-flow is not at all when cuts tie.

**Why not graph cut**, since the classical literature offers both: Boykov–Kolmogorov
max-flow is sequential, memory-heavy (two edges per neighbour pair), worst-case
super-linear, awkward to make deterministic on tied cuts, and it hands back a *hard*
binary label that still needs a separate softening pass. The GDT is linear, trivially
deterministic, produces the soft field directly, and is pass-structured — six raster
scans — so its WGSL port, if the budget ever demands one (§7), is mechanical. Its known
ceiling is equally plain: distance leaks through any low-contrast gap in the boundary
(the classic failure on a subject touching a same-coloured wall), and the fix is a
correction stroke, which is exactly the loop §6 builds. `ponytail:` the ceiling is
leaks-through-low-contrast-gaps; the upgrade path is an evidence-bearing pairwise term
(graph cut or a learned cost), not more passes.

**Seeds.** A `Foreground`/`Background` stroke seeds every pixel under its stamped path
(the paint rasteriser's dab-along-a-polyline, radius honoured). Where strokes conflict,
**later wins** — the user's most recent word is the verdict. On the base frame, if the
user has drawn no background stroke, the frame border (a 2 px ring) seeds background by
default: AE's users never paint the background first, and a solve with no background seed
has no answer. On propagated frames the warped matte supplies both seed sets (§3) and the
frame's own correction strokes override it.

## 3. Propagation: the flow carries the matte

For each step from solved frame `P` to its neighbour `N` (outward from the base, one
direction at a time):

1. **Flow pair** `(P, N)` from `lumit-flow` at the effect's flow settings (half
   resolution default, the engine's own default) — both directions, plus
   `lumit_flow::confidence(fwd, bwd)`, the forward–backward agreement already built and
   shipped for Fast motion blur.
2. **Warp** the matte: `α_w(x) = α_P(x + flow_{N→P}(x))`, bilinear — the same
   backward-warp synthesis uses, holes and z-fighting avoided by construction.
3. **Derive seeds** for N's solve: `α_w > 0.9` with confidence above a floor → foreground
   seeds; `α_w < 0.1` likewise → background seeds; both seed sets **eroded by 2 px** so a
   motion-boundary error never seeds the wrong side. Low-confidence pixels (occlusions,
   reveals, flow failures) seed nothing — they are exactly the pixels that must be
   re-decided from N's own colours.
4. **Solve** frame N with §2 over those seeds plus N's own correction strokes (which
   override warped seeds where they overlap — the user outranks the machine), then §4's
   refine band. The solve is full-frame: it is O(N) anyway, and a band-limited solve
   would add a second code path to save milliseconds the budget does not need.

The warp carries the *decision* and the solve re-decides the *boundary*: drift cannot
accumulate as a soft smear the way pure warping compounds, because every frame's matte is
re-anchored to that frame's own colour evidence. What does accumulate is topological
error — a leak through a low-contrast gap persists once seeded — and that is what
corrections are for.

**Backward steps reuse everything.** A step toward frame `N < P` is the same four stages
with the pair reversed; the flow engine is symmetric in its inputs and the code takes a
direction, not two copies.

## 4. Refine edge: the guided filter (pinned)

**The matting filter is the guided filter** (He, Sun & Tang, ECCV 2010): guide = the
frame's encoded RGB, input = the solved matte, radius `r` (default 8 px at source raster),
regulariser `ε` (default 1e-3). **O(N)** exactly, via box filters over running sums in
fixed order — no iteration, no solver, deterministic. It is the standard fast
approximation to closed-form matting: where the true edge is soft (hair, motion blur,
smoke) the local linear model `α ≈ aᵀI + b` recovers a soft alpha from the colours; where
the edge is hard it stays hard.

Applied **in a band, kept elsewhere**: the filter runs full-frame (it is cheaper to run
than to mask), but its output replaces the matte only where `|α_raw − ½| < 0.45` dilated
by `r` — the boundary band — plus anywhere a `Refine` stroke has painted (per-stroke
radius, for the one lock of hair that needs a wider band). Outside the band the GDT's
answer is snapped to 0/1: the filter must never turn a solid interior grey because the
guide happened to have texture there.

The ceiling, named: the guided filter is a *local linear* matting model, not the matting
Laplacian — long translucent strands crossing a busy background will come back muddier
than AE's network manages. `ponytail:` local-linear matting; the upgrade path is a
learned matting head on the §9 seam, not a global Laplacian solve (O(N) is the budget's
load-bearing fact).

## 5. The cache: the `roto/` sidecar tier and its one invalidation rule

A new global sidecar tier beside `track/` (docs/10 §3 gains the section in RB2), one file
per propagation run: `<media-fp-hash>-<key-hash>.lrot`, where `key-hash` is a blake3 over
(media fingerprint, roto settings, base frame, the full stroke table, this tier's format
version), and the `media-fp-hash` prefix exists so candidates for one shot are cheap to
enumerate. The file: magic `LUMROT\0`, little-endian u16 version, a bincode record of the
key, then per-frame records — frame index, that frame's **chain hash** (blake3 over the
settings, the base, and the strokes on frames from the base through this frame on its
side, in document order), matte bounding box, and the gray8 matte LZ4-compressed inside
its box. Written whole at the end of a run, temp-and-rename like every save. All of
`track/`'s binding rules apply verbatim: refuse a newer version, refuse a wrong key,
delete-safe, rebuild byte-identical (asserted, not assumed).

**The invalidation rule, whole:** editing strokes on frame `n` invalidates every cached
frame `F` with `|F − base| ≥ |n − base|` on `n`'s side of the base, and nothing else.
That is §1's purity sentence read backwards, and the chain hash enforces it
mechanically — a frame whose contributing strokes did not change keeps its chain hash.

**Prefix reuse is what makes the correction loop breathe.** A re-propagation after a
correction at frame `n` looks up any existing `.lrot` for the same (media, settings,
base), and every frame record whose chain hash matches the new stroke table's is
**copied, not re-solved** — so correcting frame 200 of 300 re-solves ~100 frames, not
300. Asserted by counting solves in the test plan, never by timing.

**Size honesty.** Gray8 at 1080p is 2.07 MB raw per frame; box-cropped and LZ4'd, a
typical matte (long runs of 0 and 255) lands in the tens of kilobytes, a 600-frame shot
in the tens of megabytes. The tier rides the cache root's existing size budget and
eviction (docs/13); nothing new.

**The render path reads the store, never the file.** `RotoStore` mirrors
`CameraSolveStore` ([tracking.md](tracking.md) §5b): a process-wide map published to when
a run lands or the warm pass finds a file, read per frame by `build.rs` with no lock held
across anything (docs/14 §1.3), with a small decompressed-frame LRU so scrubbing one
region does not re-inflate per repaint. The effect applies the matte where it sits in the
stack (`is_image_op → true`): multiply the layer's alpha (modes: Matte, Matte inverted;
views: Result, Matte, Boundary overlay). **The frame key stamps the frame's chain hash**
through the stamper seam the camera link already built (§5b deviation 8), so a stroke
edit renames exactly the frames it invalidates and a cached frame is never served stale.
A frame outside the propagated span renders **passthrough** — an honest unsegmented
picture with the panel saying how far the span reaches — never a held neighbouring matte,
which would be a wrong answer wearing a right one's face.

## 6. The correction loop, as the user meets it

1. Pick the Roto brush (Alt+W), on a layer with footage. Stroke the subject on a clear
   frame; that frame becomes the base — **and if the layer carries no Roto brush yet, the
   stroke brings one with it**, the two landing as one op and one undo step. The matte
   appears on that frame on release: the commit fires the propagation job stopped at the
   scribbled frame (`RotoJob::stop_after`), the card shows the solving status while the
   second or so passes, and the solo run is filed in the ordinary sidecar so nothing is
   ever solved twice. A base-only solo never opens the flow engine, so this first feedback
   works even where a full propagation would refuse `FlowUnavailable`.
2. Press **Propagate** (a `ParamKind::Action`, existing machinery). A background
   job — one at a time, `Busy` refusal, progress as a polled value, the whole §5b thread
   discipline — walks outward from the base, filing mattes. The timeline's span reading
   updates as it goes; the user can keep working.
3. Scrub the result. Where the matte leaks or drops a limb, stroke that frame — release
   re-solves **that frame** at once (the same stop-after job, everything between the base
   and it copied from the cache), so the correction is judged on the spot; pressing
   Propagate carries it onward, prefix reused. The frames the correction invalidated
   leave the span honestly until then — passthrough, never a stale matte.
4. **Refine edge** (the tool the strip already pairs with the brush) paints the band
   wider where the edge needs more room; the Radius and band parameters cover the
   ordinary case without any refine stroke at all.
5. **Cancel finalises rather than discards** (the tracker's pattern): the frames already
   solved are correct and correctly keyed, so they are kept and the span says how far it
   got; a later Propagate resumes from the cache.

## 7. Budgets (docs/13 stance: measured, then gated)

Target, on the owner's machine, 1080p source, defaults: **≤ 60 ms per propagated frame**,
end to end — flow pair ~7 ms (GPU, half res, measured in
[optical-flow.md](optical-flow.md) §4.7's table), warp and seeding ~2 ms, the GDT's six
raster passes ~25 ms CPU, the guided filter's box sums ~15 ms CPU, headroom the rest. A
600-frame shot propagates in well under a minute as a background job. The interactive
single-frame solve (base-frame stroking) must land under 100 ms stroke-to-matte. A
`--ignored` perf test measures both, tracking.md-style ("measured, not a gate" until the
numbers are real); the render-path matte fetch is bounded at 1 ms and *is* budget-gated,
because it sits inside the frame walk. If the CPU halves blow the budget the upgrade path
is WGSL ports — both algorithms are fixed-count pass pyramids shaped exactly like the
flow kernels — not an algorithm change.

**Measured, and the CPU halves did blow it.** RB2's `--ignored` test reports **895 ms
a frame at 1080p**, fifteen times the target above. The flow estimate holds; the two CPU
estimates do not, by more than an order of magnitude — fifty million geodesic relaxations is
not 25 ms of real arithmetic, the guided filter runs several box passes over six planes rather
than one, the seed erosion reads a 5×5 window per pixel, and every frame converts RGBA8 to
interleaved f32 and the previous matte from gray8 before any of that starts. So a 600-frame
shot is about nine minutes rather than "well under a minute", and the sentence above is a
target rather than a description until the WGSL ports land. They are therefore **owed, not
optional**; the conversions and the erosion fold into those kernels rather than surviving
beside them. Nothing else moves: a convergence test or a band-limited solve would trade
determinism, or what a matte is, for the same speed the ports give honestly.

## 8. Determinism and refusals (docs/14)

**Deterministic given (media bytes, strokes, settings, base):** the GDT and guided filter
are fixed-order f32 arithmetic; the flow is the DIS engine, GPU with its CPU oracle held
to parity. Two propagation runs on one machine are bit-identical, and the sidecar rebuild
test pins it byte-for-byte. Across machines the mattes agree to the flow's cross-backend
tolerance, and — the property that matters — **once cached, the matte is the input**:
export reads the cached plane, so an export is stable across driver updates until the
user re-propagates, and the project can record nothing weaker than "these bytes". The
`rife` backend and any future model never feed roto propagation (§0's non-determinism);
the flow settings the effect carries name the deterministic engine only.

**Refusals, each a named error and never a fault:** `Offline` (no resolved media
fingerprint — nothing to key a cache with), `FlowUnavailable` (no GPU flow on this
device: the CPU oracle at ~2 s a pair would misrepresent a minutes-long job as hung, and
mixing backends breaks the byte-identical rebuild claim — the honest answer is the
refusal, the same stance as the texture door's documented passthrough), `Busy` (one
propagation at a time), `NoBaseFrame` (Propagate pressed before any stroke). Cancellation
between frames, writing nothing partial *within* a frame and keeping whole frames (§6).
No panics, allocations budgeted per run (the planes are reused across frames), no lock
across the flow dispatch.

## 9. The growth path: a model on the seed seam, opt-in, not v1

The classical machine has one narrow waist: **§3 stage 3, where seeds for a frame are
derived**. A neural upgrade slots exactly there — an ONNX segmentation or matting model
(RVM, SAM-class, MatAnyone-class) proposing the per-frame matte, with the GDT demoted to
reconciling the proposal with the user's strokes and the guided filter unchanged — and
nothing about the document model, the strokes, the cache, the bridge or the tools moves.
The stance is `rife`'s, verbatim: optional download, licence checked per model, `ort`
with the platform execution provider, **non-deterministic across GPU/EP versions and
therefore recorded** — the sidecar key gains the backend and model hash, and the "cached
matte is the input" property of §8 is what keeps exports stable anyway. A future decision
entry owns the choice of model; this note only guarantees the seam stays where it is.

## 10. Test plan (implement with each package)

Synthetic shots with **known mattes** — shapes rendered by the tests, the tracker's own
fixture philosophy — so every claim is an assertion, not a look:

1. **Single-frame solve:** a textured disc on a textured background, one stroke inside
   and the border default outside — IoU ≥ 0.98 against the analytic disc; α monotone
   across the boundary band. A dumbbell with a low-contrast neck leaks by design — the
   test *pins the leak* (it is the documented ceiling, and if it silently stops leaking
   the algorithm changed).
2. **Propagation:** the disc translating ≤ 8 px/frame over 30 frames, strokes on the
   base only — per-frame IoU ≥ 0.95 including the last frame; the same with the base at
   frame 15, both directions solved, both ends' floors held.
3. **Occlusion:** a second shape crossing in front — the matte neither leaks onto the
   occluder nor loses the subject when it re-emerges (the low-confidence-seeds-nothing
   rule doing its job). **Two shapes of occluder are outside that claim, and RB1's test
   states which it uses:** one that *severs* the subject (a full-height pole across it)
   leaves a piece with no seeds that no path can reach from the piece that has them, since
   every route crosses the occluder's colour; and one that floats *inside* the subject,
   touching no background, is absorbed, because no seed and no colour step says otherwise.
   Both are a correction stroke — §6's loop — not a defect, and the RB1 fixture is
   therefore a pole hanging in from the top of frame across the subject's upper half.
4. **Correction loop:** a deliberate wrong stroke at the base, IoU low downstream; a
   correction at frame k — frames beyond k recover, frames between base and k
   **byte-identical** to before the correction (the invalidation rule asserted in both
   directions), and the solve count equals the invalidated span (prefix reuse asserted
   by counting, not timing).
5. **Refine edge:** a disc composited with a known Gaussian-feathered edge — the guided
   filter's band recovers the gradient to a pinned MSE, better than the snapped GDT
   alone; a Refine stroke widens the band exactly where painted.
6. **Determinism and the sidecar:** two runs bit-identical; round trip, rebuild
   byte-identical, wrong key refused, newer version refused, deleted file rebuilt.
7. **The render seam:** a stroke edit renames exactly the invalidated frames (frame-key
   assertion through the stamper); outside the span the effect is passthrough; the
   store's per-frame read holds its 1 ms bound.
8. **Refusals and cancel:** each named refusal produced; a cancel keeps its finalised
   prefix and a re-run resumes from it.
9. **Perf, `--ignored`:** the §7 numbers measured and printed, 1080p.

## 11. Ordered work packages

**RB1 — the engine crate. Built.** `crates/lumit-roto`: `RotoStroke`, seed stamping, the §2 GDT
solve, the §4 guided filter, the §3 warp-and-seed derivation taking flow, validity and
confidence as plain slices — **no `lumit-flow` dependency** (it pulls wgpu; the tracker's
§1 stance, for the same reason). Pure CPU, deterministic, no panics. Tests 1–5 of §10 in
their single-crate forms, driven by written-down flow fields as well as none at all.

**RB2 — the job, the cache, the document and the bridge.** The Roto brush effect row
(docs/08), strokes on the instance with their ops and undo, the base frame; the
propagation job, `RotoStore` and warm/clear in `lumit-render` (the §5b thread discipline,
cancel-finalises), flow via `FlowEngine`, the `roto/` tier with docs/10 §3's section, the
frame-key stamp, the matte applied in `build.rs`; the bridge surface (strokes down,
Propagate/Cancel actions, span and progress up) with its docs/17 section. Engine labels
into `engine_labels.dart` + arb, tests 3–8.

**RB3 — the tools, the overlay and the panel. Built.** `tool.roto` armed
(`ToolMode.ready` — the strip, flyout and Alt+W chord all wake together); the
brush and refine-edge gestures on the viewer writing source-pixel strokes through the
comp→layer chain, `Alt` claiming background, samples thinned at paint's two screen pixels and
pressures ignored; the overlay (this frame's strokes in theme colours, and the Boundary view's
matte edge); the effect card's span bar, status line and base-frame row; arb keys.
**Two bridge calls were owed and landed with it**, because the frontend could answer neither:
`roto_source_frame` (the decode planner's own comp→source frame arithmetic — a Retime is a
property curve) and `roto_boundary` (the matte's outline, thinned to a fixed cap; the matte
itself still never crosses). §10 item 9's Flutter halves are in
`flutter_ui/test/frb/roto_tools_frb_test.dart`.

## 12. Traps, collected

- **Seeding the warped matte without eroding it** plants foreground seeds on the wrong
  side of every motion boundary the flow blurred; the 2 px erosion plus the confidence
  floor is load-bearing, not tidiness.
- **Holding the nearest matte outside the span** looks helpful and ships wrong pictures;
  passthrough plus an honest span reading is the tracker's lesson re-applied.
- **A convergence test in the GDT** trades determinism for nothing — three pass pairs is
  already conservative for seeds this dense; fixed counts everywhere.
- **Running the guided filter's output everywhere** greys solid interiors wherever the
  guide has texture; the band-and-snap in §4 is the fence.
- **Keying the cache by anything reachable from the comp** (transform, retime, preview
  tier) breaks the one-shot-many-comps sharing that source-raster strokes buy; the key
  is §1's purity sentence and nothing else.
- **Expecting a correction stroke to outrank a warped seed it does not cover.** The
  override in §3 stage 4 is per pixel, so a dab beside a leak leaves the leak's own warped
  foreground seeds in place and the solve keeps them: a correction is a scribble *across*
  the wrong region, not near it. Where the region has real colour edges — a distinct
  object wrongly claimed — one covering stroke holds for every frame after it, because the
  corrected matte reseeds itself from evidence; where it has none (a same-coloured lobe
  joined to the subject) the boundary has nothing to anchor it and the leak creeps back
  over a handful of frames. That is the §2 ceiling seen from the correction side, and the
  reason the panel must make re-stroking cheap rather than promise one-and-done.
- **Reading γ as if it only priced the subject's edge.** It prices *every* neighbouring
  colour step, so per-pixel sensor noise costs real distance and heavily textured footage
  walks more slowly than flat footage does. The default is honest for ordinary material
  and the setting exists for the rest; RB1's fixtures use spatially smooth texture for the
  same reason, and say so.
- **Letting corrections influence toward the base** feels symmetric and destroys the
  one-sentence invalidation rule; influence flows outward, and a user who wants the base
  re-decided moves the base.

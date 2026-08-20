# The flow engine: optical flow, frame synthesis, and flow motion blur

The hardest algorithmic component in Lumit, feeding Retime's flow interpolation
([04-RETIMING.md](../04-RETIMING.md) §10) and the flow motion blur effect
([08-EFFECTS.md](../08-EFFECTS.md)). This note commits to specific algorithms so
implementation is engineering, not research.

## 0. Strategy: two backends behind one interface

```rust
trait FlowBackend {
    /// Dense forward flow A→B, half or full res, in pixels of the full-res frame.
    fn flow(&mut self, a: &GpuFrame, b: &GpuFrame, quality: FlowQuality) -> FlowField;
}
```

1. **`dis` (v1, always available)**: Dense Inverse Search flow implemented in WGSL.
   Deterministic, no model files, ~2–4 ms at 1080p half-res on the reference GPU. Quality
   ≈ Twixtor's easy-80% on game footage (high-contrast, sharp, high-fps sources — the
   favourable case).
2. **`rife` (post-v1, optional)**: RIFE v4.x ONNX via `ort` with the DirectML execution
   provider (CoreML on the dev Mac). The community already pre-processes with RIFE
   (research: Flowframes), so this is a known-good ceiling. It synthesises frames directly
   (no explicit flow field), so it slots in at the *synthesis* level (§3) rather than as a
   FlowField producer; motion blur keeps using `dis` vectors. Keep it optional: model
   download, licence (RIFE is MIT), non-determinism across GPU/EP versions — export
   determinism rules mean the project stores which backend rendered.

Do not implement Farnebäck (too smeary), Horn–Schunck (too slow at quality), or RAFT-class
training pipelines (research project). DIS is the studied sweet spot: OpenCV's
DISOpticalFlow documents the algorithm; the paper is Kroeger et al., ECCV 2016.

## 1. DIS flow in WGSL — exact structure

All passes on grayscale (BT.709 luma of the linear frame, then **gamma-encode before
correlating** — flow works better on perceptual values; this matches OpenCV practice).

**Pyramid build**: `L0` = luma at working res (default **half** comp res; `FlowQuality`
selects), then box-downsample ×2 per level to ~24 px min dimension (≈ 5 levels at 1080p
half res). Any deeper and the 8×8 patches are frame-scale: every patch straddles every
motion boundary and whole strips of the coarsest field start as garbage the finer levels
cannot always heal (measured in the §6.1 occlusion test; originally ~16 px).
Also build Sobel gradients per level (v1: f32 storage buffers throughout, not fp16
textures — fp16 rounding would eat the §6.5 CPU-parity budget; textures return when
synthesis itself moves GPU-side).

**Per level, coarse → fine:**

1. **Init**: upsample flow from coarser level (bilinear, ×2 magnitude). Each patch
   samples the init at its centre, its four corners, **and one patch-length outside
   each edge**, and starts from the lowest-SSD candidate — near a blurred motion
   boundary only a sample from beyond the blur puts the true motion on the ballot
   (the data-parallel stand-in for OpenCV's sequential neighbour propagation).
2. **Inverse search (the core)**: for each 8×8 patch on a stride-4 grid, refine its flow
   vector by Lucas–Kanade-style Gauss–Newton, *inverse compositional*: the Hessian comes
   from patch A's gradients (precomputable per patch, once per level):
   `H = Σ [gx², gx·gy; gx·gy, gy²]` over the patch (2×2, invert analytically; if
   `det < 1e-6` mark the patch invalid — textureless). Then ≤ 12 iterations of:
   `residual r = Σ g·(A(x) − B(x+u))`, `Δu = H⁻¹ r`, `u += Δu`, stop when `|Δu| < 0.02 px`
   (sign note: the update must *reduce* the residual; the earlier draft had the residual
   reversed, which diverges — caught by the §6.1 tests). Track the best cost seen and
   revert a step that made matching worse (guards near-singular H). A patch whose final
   cost stays above `0.25 × its own variance + 0.05` never found its content — it is
   straddling a motion boundary or occluded — and is marked invalid too.
3. **Densification**: each pixel's flow = weighted average of the ≤ 9 valid patch vectors
   covering it, weight `exp(−‖B(x+u_patch) − A(x)‖² / σ²)` (σ ≈ 0.08 in encoded luma) —
   photometric-error weighting is what keeps edges crisp; plain bilinear here is the
   classic mistake that produces rubber-sheet output. Two refinements, both test-driven:
   average only the votes that agree (within ~2 px) with the best-matching vote —
   averaging *across* a motion boundary manufactures a vector belonging to neither
   motion — and when no covering patch explains a pixel, retry against the wider 5×5
   patch neighbourhood's hypotheses (photometrically gated, so nothing leaks across a
   content edge) before falling back to the init flow with the pixel marked invalid.
4. **Smoothing**: one 3×3 edge-aware blur of the flow field — bilateral on luma *and* on
   flow difference, so vectors from the two sides of a motion boundary never average into
   a phantom in-between motion.
5. **Variational refinement** — DIS part three, and **not optional** (K-332). This note
   previously said to skip it in v1 and "measure first"; the measurement happened and both
   halves of the reasoning were wrong. Untextured regions are not rare in game capture (smoke,
   sky, muzzle flash, water, darkness are most of a frame during the fast moments a montage
   slows down), and without refinement they fail *hard* rather than softly: densification
   leaves the coarse guess, flags the pixel invalid, §2 counts invalid as occluded, and §3
   crossfades it — patches of ghosted mush, the reported artefact.

   Per the paper (§3.3), minimise `E(U) = ∫ σ·Ψ(E_I) + γ·Ψ(E_G) + α·Ψ(E_S) dx` with
   `Ψ(a²) = √(a² + ε²)`, ε = 0.001, σ = 5, γ = 10, α = 10. `E_I` is intensity constancy,
   `E_G` gradient constancy — the term that survives a brightness step, which a muzzle flash
   is and which plain intensity constancy reads as motion everywhere — and `E_S = ‖∇u‖² +
   ‖∇v‖²`. Both data tensors are normalised by their own gradient energy plus ζ² (ζ = 0.1) so
   a high-contrast pixel cannot shout down a low-contrast one. Run once per pyramid level,
   `1·(s+1)` fixed-point iterations at scale `s` counting from the coarsest, each linearising
   about the current warp and solving for the increment with `θ_vi = 5` SOR sweeps at ω = 1.6.

   **Sweeps are red–black, not raster order.** Plain SOR wants each pixel to read its
   neighbours' just-updated values, which is strictly sequential. On a checkerboard every
   neighbour of a red pixel is black, so a whole colour updates with no pixel reading another
   of its own colour — the identical algorithm, reordered into something the WGSL can run in
   parallel. **The CPU oracle is written this way deliberately**: a sequential oracle would
   have condemned the shader to disagree with it by construction, and the §6.5 parity contract
   would have had to be abandoned rather than met.

   **Validity changes meaning.** It was "at least one patch covered me photometrically"; it
   becomes "the refined flow explains these pixels", from the residual after refinement
   (`VR_RESIDUAL_MAX`). A refined field has an answer everywhere, so the honest question is
   whether the answer is right, not whether one was found.

   **Cost, measured (960×540 pair, dev machine):** parts 1–2 on the CPU 456 ms, all three
   1.82 s — 4×. Parts 1–2 on the GPU 4.8 ms. The refinement therefore *must* reach WGSL: the
   CPU oracle at 1.8 s per pair is a correctness reference, not a preview path.

**Output**: the dense flow at working res plus a per-pixel validity mask (v1: one f32
storage buffer read back to the CPU, since synthesis still runs there; `Rg16Float`
texture + R8 mask when the GPU-resident synthesis path lands).

**Kernel shape (v1)**: one *thread* per patch rather than one workgroup — the sums then
run in the same sequential order as the CPU oracle (which makes the §6.5 parity bound
meaningful), the WGSL needs no shared-memory/uniformity choreography, and the whole
search is far inside budget (measured ~4 ms per 960×540 flow *pair* including readback
on the dev RTX). Revisit workgroup-per-patch with shared memory only if profiling ever
says the search dominates.

## 2. Occlusion: forward–backward consistency

Compute flow both directions (F: A→B, B: B→A — reuse everything; it is 2× cost).
Pixel x is **occluded in B** (i.e. visible only in A) when
`‖F(x) + B(x + F(x))‖ > max(1.5, 0.05·(‖F‖+‖B‖))` (the standard consistency test with a
relative term for large motions). Output an occlusion mask per direction (R8: 0 = ok,
1 = occluded, plus the invalid-patch bits from §1). Dilate by 1 px — consistency tests
under-detect at exact boundaries.

## 3. Frame synthesis at phase φ ∈ (0,1) between A and B

Backward-warp both endpoints and blend with occlusion-aware weights (the RSMB/Twixtor
family approach; avoids forward-splatting's holes and z-fighting):

```
uA(x) = −φ · F_scaled(x)        // sample A at x + uA   (F scaled: flow A→B over Δt=1)
uB(x) = (1−φ) · B_scaled(x)     // sample B at x + uB   (B_scaled: the *forward* velocity
                                //  at B's grid, i.e. the negated B→A field)
wA = (1−φ) · (1 − occB(x)) + ε ;  wB = φ · (1 − occA(x)) + ε
out = (wA·A(x+uA) + wB·B(x+uB)) / (wA + wB)
```

- The flow sampled for warping at x should ideally be the flow *at the destination*;
  approximate with one fixed-point iteration: sample F at x, then re-sample F at
  `x − φ·F₀(x)`, use that. Two lines in the shader, visibly reduces edge doubling.
- Where **both** endpoints are occluded/invalid (revealed background with no source):
  fall back to blend `lerp(A, B, φ)` — soft failure identical to Frame-Mix, which is the
  documented graceful-degradation behaviour ([08-EFFECTS.md](../08-EFFECTS.md): confidence-
  gated fallback). Also expose the per-pixel confidence as an optional debug view; editors
  mask flow failures by hand today and will want to see them.
- Everything here operates on **linear premultiplied fp16** (warping/blending is where
  linear matters most); only the *correlation* in §1 used encoded luma.

Phase quantisation for cache keys: per [04-RETIMING.md](../04-RETIMING.md), φ rounds to
1/1024. Flow fields themselves are cached per (A,B, quality) pair in the sidecar `flow/`
tier — they are the expensive part; synthesis is ~free.

## 4. Flow motion blur (RSMB-class)

Given the frame N and flow to its neighbours (F₋ to N−1, F₊ to N+1), per-pixel blur along
the motion trajectory with shutter s ∈ (0,1] (from shutter angle/360) and amount k:

```
v(x) = k · s · 0.5 · (F₊(x) − F₋(x))          // central-difference velocity, px/frame
S = clamp(ceil(‖v‖ / 2), 1, 64)               // adaptive taps, ≤ 2 px per tap
out = (1/W) Σ_{i=−S..S} w_i · frame(x + v·(i/(2S)))   // w_i = 1 (box) — a shutter is a box
```

- Iterate the same destination-flow fixed-point trick per tap for long streaks; without it,
  streaks curve wrongly around rotating objects.
- Occluded taps (mask from §2) drop out of the sum (renormalise by W) — this is what stops
  foreground smearing across revealed background, the visible difference between cheap and
  good motion blur.
- Respect the no-double-blur rule: when the host already applied transform multi-sampling
  to a layer, the effect receives a flag and must not add transform-derived velocity
  ([06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md) §motion-blur).

**Shipped v1 (labelled "Fast motion blur", FX-19).** The v1 effect measures the single forward
neighbour (+1) and streaks each pixel with a fixed centred box of `Samples` taps. Crucially it
does **not** drop occluded taps from the sum (a per-tap on/off gate showed as hard blurred /
un-blurred cut regions). Instead the *streak length* is scaled smoothly by a per-pixel
**confidence** in 0..1: `lumit_flow::confidence(fwd, bwd)` — the raw forward–backward consistency
mapped to 1 (agree) … 0 (disagree, at the same rel/abs scale the binary occlusion cut uses, an
invalid patch fully suspect), then 3×3 box-blurred so the taper has no seam. The confidence
rides in the flow texture's `.z` (an `rgba32float` field), and the kernel does `sv = flow ·
shutter_frac · conf`; confidence 0 collapses the streak to the pixel (a passthrough there). A
**View** enum outputs the finished blur, the flow colour-coded, or the confidence as greyscale.
CPU oracle (`lumit_core::fx::cpu::motion_blur`) and WGSL stay op-for-op (§1.6).

**Shipped v2 (K-392, the Guertin-class reconstruction).** §4.7 carries it. The confidence
taper survives as the *smooth* quantity v1 measured it to need, but it now steers rather
than shortens: an uncertain pixel borrows its neighbourhood's motion instead of collapsing
to none. Taps became adaptive, the reconstruction became a two-direction weighted gather,
and High re-samples the field along the streak. The ±1 central difference did **not** land
— §4.7 names the seam that stops it.

## 5. Parameters and defaults (user-facing, per [08-EFFECTS.md](../08-EFFECTS.md))

Resist adding more knobs — Twixtor's manual is a warning, not a target. The set is closed at
the §3.1 table, which ships in full as of K-331.

**Engine-side (`lumit_flow::FlowSettings`).** `lumit-flow` is an engine crate and knows
nothing of the document, so the stored `FlowParams` are translated into plain numbers by
`lumit_render::decode::flow_settings` — one function, so preview, export and the flow cache
cannot translate the same parameters into two different measurements.

| Setting | From | Effect on the algorithm |
|---|---|---|
| `divisor` | Flow resolution | 1/2/4 on the source dims before §1's pyramid. Repeated box-halving, never a second resampler, so the WGSL mirrors it. A source under `8·d·2` px stays whole rather than starving the pyramid |
| `iterations` | Vector detail | §1 step 2's cap: 6 / 12 / 20 / 32 (Medium is the paper's ≤ 12) |
| `min_level_dim` | Vector detail | §1's pyramid floor: 48 / 24 / 24 / 16. Below ~24 the 8×8 patches go frame-scale — the failure §6.1 measured |
| `smoothness` | Smoothness | Scales `FLOW_SIGMA2` in §1 step 4's bilateral, quadratically over a 4× span each way, clamped. 50 is exactly the tuned constant, so the default is bit-identical to the pre-parameter engine |
| `refine_iters` | Vector detail | §1 step 5's fixed-point iterations per level: 1 / 1 / 2 / 3. `0` disables DIS part three and is **not user-reachable** — it is the two-part engine K-332 replaced, kept only so the A/B test and the GPU parity test can address it |
| `occlusion` | Occlusion handling | §3's weights: Visible-only keeps the `(1 − occ)` terms, Blend drops them |
| `fallback` | Fallback | §3's both-occluded branch: crossfade or the nearer endpoint |
| `hud_guard` | HUD guard | Runs §3.1 step 5's `hud_weights` and mixes synthesis back toward the plain blend by it |

Fast motion blur's own controls are docs/08 §3.2's table, not this one: they describe the
*reconstruction*, not the measurement. The blur reads whatever field these settings
produced — `MbQuality` (Normal/High) chooses how it is integrated (§4.7), the tap cap how
finely, and neither changes a vector.

**Every setting has a GPU path.** The iteration cap and the smoothing sigma ride in the
per-level `Params` uniform; the pyramid floor shapes the plan, which is rebuilt when the
settings change (`Plan::set`). The refinement is seven kernels — `vr_warp`, `vr_init_duv`,
`vr_deriv`, `vr_sor_red`/`vr_sor_black`, `vr_apply`, `vr_validity` — reusing the existing
eight-binding layout: `duv` packs `(du, dv, u, v)` into one vec4 so the solver needs no fifth
read slot, the increment travelling with the flow it is an increment of. `FlowError::Unsupported`
therefore no longer fires for any real setting, and the parity test covers all three parts of
the algorithm again.

**Measured (960×540 pair, dev machine):** GPU parts 1–2 4.3 ms, all three **8.9 ms**; CPU all
three 1.9 s. The refinement roughly doubles GPU cost and is comfortably inside budget.

## 4.5 The quality programme (K-390): census matching, edge-held boundaries, feature-aware blur

What §5.5 measured is the brief: flow wins convincingly on game capture and loses to a
crossfade on line art by the worst-block measure, because the matching cost has no
evidence in flat regions and the smoothness term diffuses motion across the exact edges
it should respect. No knob fixes it (§5.5's sweep), so the method moves — three
upgrades, each classical, GPU-shaped, and model-free (models stay a plugin question):

1. **Census (ternary) matching cost in the inverse search.** SSD on grey patches treats
   a lighting change as motion and flat regions as agreement. The census transform
   compares each pixel only with its neighbours — invariant to monotonic brightness
   change, and it concentrates evidence exactly where line art keeps it: on edges.
   Both backends change op-for-op; the oracle stays the proof.
   **Built, measured, and reverted.** It is the only one of the three that moves
   §5.5's numbers at all — the right way on both animation clips, the wrong way on
   game capture (§5.5.1) — and choosing the cost per patch instead of per build
   (K-393, §5.5.2) shrank the miss without closing it. Neither form clears the bar,
   so the shipped inverse search scores by SSD exactly as it did before K-390.
2. **Edge-aware densification of the field.** After the pyramid, solve the field against
   the picture's own edges: a fast-bilateral-solver-shaped pass (Barron & Poole) with
   the luma image as the guide and §4's confidence as the data weight, so vectors stop
   at boundaries instead of bleeding a moving object's motion onto the background cel.
   Normal runs a bounded number of iterations; High runs it to convergence.
   **Built, measured, and deleted — see §4.6 for the tables and the reason.**
3. **Feature-aware motion blur reconstruction (Guertin-class).** v1 gathers along each
   pixel's own vector, so a fast object never smears over the background — the visible
   half of the scatter problem — and the listed follow-ups (±1 central difference,
   destination fixed point, adaptive taps) all land with it: tile-max + neighbour-max
   dominant directions (two per tile), confidence-weighted taps in place of the binary
   occlusion gate v1 already rejected, jittered sampling against banding, and curved
   trails (re-sampling the field along the streak) on High.
   **Built and shipped — see §4.7**, with two departures: the ±1 central difference is
   blocked by a seam §4.7 names, and jitter was not needed once taps became adaptive.

**Acceptance is §5.5's own harness**, no new judges: worst-5% block SSIM must rise on
the animation clip and must not fall on gameplay (the recorded 0.036-for-0.012 trade
was removed once already; the bar is a strict no-regression on game capture). The blur
half is judged by its oracles op-for-op plus the View outputs (flow colour, confidence,
and a tile-max debug view) that make it checkable by eye.

**Outcome: the flow half ships nothing; the blur half ships in full.** Item 1 met
three of the four conditions and missed game capture by 0.0023 (§5.5.1); choosing the
cost per patch (K-393, §5.5.2) moved the miss to 0.0002 on the cinematic and fixed the
synthetic clip, but three of four is still three of four, and the frontier's shape says
a hard per-patch switch cannot close the last of it. **The bar was written strict on
purpose and it is not negotiated with**, so both forms of the cost change were reverted:
the shipped inverse search is the pre-K-390 SSD one, and the shipped densification is
§1 step 3 unchanged. Item 2's code is deleted (§4.6); item 1's and K-393's code is gone
too. What the programme leaves behind is item 3, the measurement machinery, and four
sections of numbers that mean a later attempt starts from evidence rather than from
the same guess.

**Tiers.** Only the blur's Quality survives the programme as a tier — Normal/High, buying
half the tap spacing and curved trails (§4.7). The flow tier the plan sketched had solver
convergence to sell and there is no solver left to converge. Measured on the owner's
machine; if High ever costs more than a second a frame at 1080p the tier split is mandatory
(owner's rule), otherwise both tiers ship anyway because the knob is nearly free.

## 4.6 What edge-aware densification measured, and why it is gone (K-391)

§4.5 item 2 predicted the win on line art. It was built — CPU reference and WGSL twin,
op-for-op, `gpu_matches_the_cpu_oracle` green at both sweep parities — and measured on
all five clips of the §5.5 harness. **It loses on four of them, and the stage's own
acceptance bar (worst-5% up on both animation clips, not down more than 0.005 on game
capture) fails at every setting tried.** Worst-5% block SSIM, arrangement A, λ = 0.05:

| clip | base | ζ=.100 | ζ=.050 | ζ=.020 | ζ=.010 | ζ=.005 |
|---|---|---|---|---|---|---|
| anime (stride 3) | 0.7025 | **0.7071** | 0.7068 | 0.7063 | 0.7059 | 0.7055 |
| cartoon | 0.4155 | 0.4048 | 0.4083 | 0.4113 | 0.4131 | **0.4152** |
| gameplay-pov | 0.3492 | 0.3361 | 0.3373 | 0.3395 | 0.3411 | **0.3428** |
| cinematic (stride 10) | 0.2330 | 0.2103 | 0.2155 | 0.2219 | 0.2258 | **0.2279** |
| synthetic | 0.8689 | 0.8534 | 0.8536 | 0.8537 | 0.8540 | **0.8553** |

Read the rows against each other and the answer is in the shape, not the size. `ζ` sets
how much gradient a pixel needs before the pass believes its vector, so a large ζ means
the pass overrules more of the frame. **Anime is the only clip that wants ζ large; every
other clip's best column is the one where the pass does least**, and on four of five the
optimum is "as close to off as it can get". λ behaves the same way (0.02 → 0.40 costs
cartoon 0.4096 → 0.3986 monotonically), and arrangement A — composing with §1 step 4's
blur — beat arrangement B, replacing it, on both animation clips (anime 0.7068 vs 0.7059,
cartoon 0.4083 vs 0.4072), so the blur stays.

**The data weight was tried both ways.** §4.5 item 2 says to weight the data term by §4's
forward–backward confidence; the version above weights it by local gradient energy — how
much evidence the picture gave that vector — which is what a flat cel actually lacks. Both
were measured (cartoon, half res, CPU oracle, 36 triplets): base 0.4069, gradient evidence
0.3991, evidence × forward–backward confidence 0.4020. The literal reading is the milder
of the two and still a loss, so the choice of weight is not what decides this.

**Why it cannot work, which is the part worth keeping.** By this point in the pipeline the
field has already been through variational refinement, which regularises it *using the
picture's actual photometric evidence*. This pass regularises it again using no evidence
at all — only luma similarity and the field's own values. It can therefore add no
information; it can only trade one smoothing for another. Where refinement had nothing to
go on it occasionally guesses better (anime, +0.005 at best), and everywhere refinement was
already right it does damage. That the best setting on four clips is the one nearest to
disabled is not a tuning failure, it is the measurement saying the incoming field is better
than anything luma-guided diffusion produces from it.

The edge-stopping half was sound, and was held by its own test while the pass existed
(`densification_fills_a_flat_band_without_crossing_its_edge`): motion crossed a flat band
from the evidence inside it and stopped dead at a luma boundary. The mechanism did what it
said. It is the premise — that a *field-space* solve can fix line art — that the numbers
refuse. What line art lacked was evidence, and evidence was item 1's job (census, K-390),
which did move anime: worst-5% 0.697 → 0.7025 against the same blend baseline, closing the
gap to a crossfade from −0.015 to −0.0095 — though census did not clear its own bar either
and does not ship (§5.5.1, §5.5.2). Line art is still behind a crossfade on the worst
blocks, and both attempts at it are now measured rather than argued.

**Status: deleted, both backends.** It was first parked disabled — `dense_iters` defaulting
to `0`, every `VectorDetail` tier mapping to `0` — and that was the wrong resting place: a
knob no setting reaches, a shader nothing dispatches and a `FlowSettings` field no shipped
`.lum` can carry is dead weight that the next reader has to re-derive the verdict on. The
CPU pass, the `densify_edge` kernel, the ping-pong buffers, the `dense_iters` field and its
`VectorDetail` mapping are gone; the tables above are the record, and they are the part
worth keeping. A future attempt at item 2 would have to be evidence-bearing rather than
field-space, which means a different pass rather than this one re-enabled, so it loses
nothing by starting from these numbers instead of from this skeleton.

The edge-stopping property that pass proved is not lost either: it was one Jacobi solve
with a luma-similarity affinity, and the paragraph above records both what it does and what
it costs. Anything rebuilding it starts from a page of measurements rather than from §4.5's
prediction, which is the difference between the two attempts.

## 4.7 The Guertin-class blur, as built (K-392)

§4.5 item 3, shipped. Two passes now: `fx_mb_tilemax.wgsl` reduces the flow field to one
dominant vector per 16 px tile, then `fx_motionblur.wgsl` blurs. The reduction lives inside
`FxEngine::motion_blur`, computed from the flow texture the kernel already holds, so the
decode worker, the render plan and the aux slots still carry exactly one field and nothing
outside lumit-gpu changed shape. `lumit_core::fx::cpu::motion_blur` mirrors all of it
op-for-op; `wgsl_motion_blur_matches_the_cpu_oracle` covers six cases across both tiers and
all four views at ≤ 2 fp16 ULP.

**Two summaries of the neighbourhood, and conflating them was the one real bug.** A tile's
*dominant* vector (confidence-weighted longest, neighbour-maxed over 3×3) answers "which way
might something have flown into me" — an extremum, correctly, because the point is to catch
the fast thing. It is also, at first, what uncertain pixels were given to borrow, and that is
wrong: borrowing asks "what is my neighbourhood *doing*", and an extremum is the single most
unusual vector out of 256, selected exactly where the measurement is least trustworthy.
Measured on cartoon.mp4 frame 200 (a fast zoom, 78 px/frame, 70% of pixels below half
confidence): neighbouring tiles won unrelated wild vectors and the blur came out in
**rectangular patches of different angles** across the characters' faces — plainly out of
place, the exact failure the stage exists to prevent. The borrow now samples the tile field
*bilinearly between tile centres*. The borrowed direction is continuous, so no tile edge can
show; and disagreement cancels, so four tiles that agree reinforce into a full-strength
vector while four pointing at random average toward zero — with no consensus the blur backs
off toward not blurring, which is the right answer arriving from the arithmetic rather than
from a special case.

**The pieces.** Scoring a tile by `conf · ‖v‖` alone made a wholly unmatched region — smoke,
a flash, fast water — score zero everywhere and read as *still*, handing a zero direction to
the pixels that most need one; the trust weight is therefore floored at 0.25, so an untrusted
vector can represent its tile when there is nothing better while a trusted vector four times
shorter still outranks it. Taps alternate between the dominant direction and the pixel's own
(Guertin's two per tile) and are weighted McGuire-style — the sample's own streak reaching out
(cone), this pixel's reaching in (cone), and a cylinder term for the ordinary case where the
two agree, which is what keeps uniform motion integrating like the box a shutter is rather
than the triangle two cones alone would give. Tap *count* is adaptive (§4's `S = ceil(‖v‖/2)`,
High halving the spacing), with the schema's `Samples` demoted from a count to a cap. High also
re-samples the field at each own-direction tap's midpoint, bending the trail; the dominant
sweep stays straight, being one direction by construction. Jitter was dropped: adaptive taps
already hold the ≤ 2 px spacing that banding needs, and a hash agreeing bit-for-bit across two
languages is a parity hazard bought for nothing.

**Confidence 0 is no longer a passthrough.** Zero blur now survives in exactly one place —
where the tile itself is still — which is the owner's stated rule. `MbQuality` (Normal/High)
is the only method choice a user sees; there is no picker, one method adapts internally.

**Measured on the owner's clips, frame-level mean |Δ| out of 255.** gameplay-pov f300
(9 px/frame, 21% below half confidence): the genuinely static desktop around the game window
moves **0.014** and the taskbar **0.000**, against **3.99** inside the moving viewport — v1
scored 0.007 / 0.001 / 3.39, so v2 buys ~18% more blur where there is motion and still leaves
still content alone. cartoon f200: 3.06 → 9.83, correct for a 78 px/frame zoom v1 was badly
under-blurring.

**The known limitation, which is depth, not tuning.** McGuire's reconstruction separates
foreground from background with a depth buffer. There is none here, so the depth-free
symmetric weighting cannot tell "the fast thing is behind me" from "in front of me", and a
*small* static object entirely surrounded by fast motion receives its neighbours' smear. On
cartoon f200 the burnt-in station logo moves 8.95 where v1 left it at 0.37. Large static
regions are unaffected (the gameplay desktop above), because the reach is a tile or two plus
the streak length; it is only objects smaller than that which are wholly inside it. For real
footage this is the correct behaviour — something passing close does cover you — and the fix,
if one is ever wanted, is a depth input, not a constant.

**What it costs, re-confirmed against the shipping tree (2026-08-19).**
`blur_proof.rs::flow_and_blur_frame_cost`, owner's machine, half-res flow, kernel time with
the readback subtracted. The flow rows are the reverted engine — census and the selector are
out, so these are the pre-K-390 figures, unchanged by the programme.

| frame | flow Normal | flow High | blur Normal | blur High |
|---|---|---|---|---|
| gameplay-pov 1920×1080 | 7.26 ms | 10.53 ms | **0.37 ms** | **0.47 ms** |
| anime 1920×1080 | 7.92 ms | 10.28 ms | **0.31 ms** | **0.31 ms** |
| gameplay-pov 3840×2160 (`LUMIT_COST_SCALE=2`) | 39.65 ms | 65.14 ms | **1.00 ms** | **1.57 ms** |

The blur is a rounding error beside the measurement that feeds it — 5% of the flow's cost at
1080p and 4% at 4k — so §4.5's tier rule (High must ship as a separate tier if it ever costs
more than a second a frame at 1080p) is nowhere near being triggered: High is 0.47 ms, three
orders of magnitude inside it, and both tiers ship because the knob is nearly free.

**Cadence check (the clip set's open question).** anime.mp4's `(60)` filename does **not**
mean a tool interpolated it: `clip_cadence` reports 78% held pairs, 67.2% flat, cadence runs
of 2 (×17) and 3 (×6) — the original animation cadence preserved inside a 60 fps container,
reproducing §5.5's recorded figures exactly. An interpolated clip would show almost no held
pairs. The §5.5 held-frame exclusion therefore applies as written, with no extra caveat.
cartoon.mp4 by contrast is 17% held, 56.1% flat, mostly on 1s — much more continuous motion,
which is why it is the harder clip of the two.

## 5.5 Measured quality (the harness, K-332 follow-up)

`crates/lumit-render/tests/flow_quality.rs` scores the engine on real footage by
rebuilding a frame from its two neighbours and comparing against the frame that
was actually there — ground truth out of ordinary film. It reports against
**nearest** (hold the previous frame) and **blend** (crossfade). Blend is the one
that matters: flow costs far more, and its failure is tearing rather than a soft
double image, so losing to a crossfade makes it worse than useless.

**Three measures, and the third is the one that matters.** PSNR scores an error
by size, SSIM by shape, and the **5th-percentile block SSIM** by the worst
twentieth of the picture. Flow does not go uniformly slightly wrong: it goes
badly wrong in a few places and stays right everywhere else, which over a 1080p
frame averages to a rounding error. A clip that looks unusable can score level
with a crossfade on the mean and be a fifth of a point worse on the worst blocks.

**Triplets where any two of the three frames are held are excluded**, compared
loosely because a held cel is not bit-identical after encoding. Animation drawn
on 2s and 3s holds most of its frames — 78% of neighbouring pairs on the clip
below — so a middle frame that duplicates an end is the norm rather than the
exception, and leaving those in scores every method against a target one of its
own inputs already is. An earlier run of this harness did leave them in and
concluded that *holding* was the best method on animation; it is not, it is
comfortably the worst, and the difference was entirely the sampling.

| footage | rate | nearest | blend | flow | Δ PSNR | Δ worst |
|---|---|---|---|---|---|---|
| gameplay 600 fps | native | 29.02 / 0.8688 | 31.86 / 0.9012 | **35.62 / 0.9707** | +3.77 | — |
| gameplay | 60 (÷10) | 22.20 / 0.6530 / 0.033 | 24.35 / 0.6871 / 0.083 | **26.93 / 0.8208 / 0.342** | +2.58 | **+0.259** |
| gameplay | 24 (÷25) | 20.38 / 0.6012 | 21.49 / 0.6170 | **22.00 / 0.6663** | +0.51 | — |
| anime on 2s | stride 2 | 32.74 / 0.9532 / 0.666 | 37.08 / **0.9536** / **0.699** | 37.07 / 0.9506 / 0.681 | −0.00 | −0.018 |
| anime | stride 3 | 31.52 / 0.9511 / 0.671 | 35.99 / **0.9541** / **0.712** | **36.87** / 0.9519 / 0.697 | +0.88 | −0.015 |

(PSNR dB / SSIM / worst-5% where measured.)

**What it says.** On game capture flow is not marginally better than a crossfade,
it is holding structure together where a crossfade falls apart: +0.26 of
worst-block SSIM at a 60 fps effective rate, against a blend that has essentially
collapsed there (0.083). This is the footage the project exists for (K-002) and
the engine is doing its job on it.

On cel animation flow is level with a crossfade on PSNR and consistently *worse*
on both structural measures. Interpolation itself is clearly worth doing — nearest
is far behind — so this is not "the content cannot be interpolated". It is that
warping introduces localised damage a crossfade does not, and the damage lands on
line art where it is most visible. Cel animation is flat regions bounded by hard
edges: no photometric evidence across most of the frame, and a smoothness term
that diffuses motion straight over boundaries it should stop at.

**Two corollaries.** Parameters do not decide this — the full sweep spans about
0.2 dB on either clip, so whatever fixes animation is not a knob. And a
confidence-weighted bias toward the fallback, written to stop flow ever losing to
a crossfade, was measured and removed: it cost gameplay 0.036 of worst-block SSIM
to gain animation 0.012 and changed neither verdict.

**Content is separable, cheaply.** `clip_cadence.rs` reports held-frame fraction
and flat fraction: 78% held and 67% flat on the animation clip, 0% held and 23%
flat on the game capture. Either statistic alone separates them, which is what
makes choosing an engine automatically (§0's `rife` backend) a tractable thing
rather than a guess.

### 5.5.1 The K-390 result, measured end to end

The programme's own acceptance bar (§4.5): worst-5% block SSIM **up** on both
animation clips, and **not down by more than 0.005** on game capture or the
cinematic. Measured on the owner's five clips, the shipping `flow (defaults)` row,
the same clips and the same sampling either side of the change — the pre-K-390
engine against the census one. Only **census matching** moves these numbers:
densification was measured out (§4.6) and the blur does not take part in frame
synthesis, so the "after" column is the census stage alone, isolated on purpose.

**This table is a record, not a release note.** The census column did not clear the
bar, neither did K-393's per-patch refinement of it (§5.5.2), and both were reverted
— so the *shipping* engine is the "before" column. The reading below is why.

| clip | stride, triplets | before (PSNR / SSIM / worst-5%) | census | Δ worst-5% | bar |
|---|---|---|---|---|---|
| anime.mp4 | 2, 72 | 37.95 / 0.9591 / 0.7312 | 37.98 / 0.9597 / **0.7337** | **+0.0025** | up — **met** |
| cartoon.mp4 | 1, 90 | 24.12 / 0.8951 / 0.4087 | 24.04 / 0.8932 / **0.4133** | **+0.0046** | up — **met** |
| cinematic-4k.avi | 1, 200 | 38.14 / 0.9724 / 0.9068 | 38.15 / 0.9717 / 0.9034 | −0.0034 | ≥ −0.005 — **met** |
| gameplay-pov.mp4 | 1, 115 | 30.20 / 0.8903 / 0.3671 | 29.99 / 0.8884 / 0.3598 | **−0.0073** | ≥ −0.005 — **missed** |
| synthetic.mp4 | 1, 200 | 26.08 / 0.9718 / 0.8922 | 25.72 / 0.9708 / 0.8863 | −0.0059 | (not in the bar) |

**Three of the four conditions hold; game capture does not.** The animation clips
both improve, which is what the programme was for, and the cinematic is inside the
tolerance. Game capture loses 0.0073 of worst-block SSIM, half again the 0.005 the
bar allows, and it is not a sampling artefact: the same measurement at 26 triplets
reads −0.0091 and at 115 reads −0.0073, the same sign and the same order.

**What it means, and what it does not.** Flow still beats a crossfade on game
capture by a wide margin (+0.116 of worst-block SSIM after the change, against
+0.124 before) — this is a smaller win, not a loss, and the footage the project
exists for is still comfortably served. But the bar was written strict on purpose,
because a game-capture regression traded for an animation gain is the exact trade
§5.5 recorded and removed once already. **Census as it stands is that trade again,
smaller in both directions.** The honest reading of the table is that the census
cost helps where evidence is scarce and costs a little where evidence was already
plentiful and high-contrast — game capture being the extreme of the second case.

The obvious next move is to stop making it a global choice: score a patch with
census where the SSD cost has no discrimination and with SSD where it does, chosen
per patch from a quantity both backends can compute identically. That is a change
to §1 step 2 and a new measurement, not a knob, and it is not in this stage.
§5.5.2 is that measurement.

**Final status (2026-08-19).** §5.5.2 was built, swept and also missed — three of
four again, by 0.0002 on the cinematic instead of 0.0023 on game capture. A smaller
miss is not a met bar. **Census matching and the per-patch selector are both reverted;
the inverse search ships as SSD, unchanged since before K-390**, and the tree carries
no census constant, no `CENSUS_GRAD_RMS`, and no third code path to maintain. The
flow ships nothing this round. What it gained is this table and §5.5.2's frontier:
the next attempt at line art knows what census is worth, what it costs, and which
clip fails first, which is more than the last one knew.

### 5.5.2 Choosing the cost per patch, and the frontier it buys (K-393)

§5.5.1 named the fix; this is it, built and measured. The choice census made
globally is made per patch, from the one quantity that *is* SSD's
discrimination. The Gauss–Newton Hessian is the second-order term of the SSD
cost about its own minimum, so its trace `h11 + h22 = Σ(gx² + gy²)` — already
summed for the step, divided by the patch's 64 pixels — says how sharply SSD can
tell right from wrong here. Below a threshold on its root, `CENSUS_GRAD_RMS`,
the patch is scored by census; at or above it, by SSD. Whichever is chosen is
used throughout that patch: its candidate ballot, its keep-or-revert test (`>`
for the piecewise-constant census cost, `>=` for the continuous SSD one), its
residual (Huber-capped for census, plain for SSD) and its validity test
(evidence-relative for census, contrast-relative for SSD). Free, deterministic,
and the same arithmetic in both backends.

**The two ends of the sweep are §5.5.1's two columns, which is the
implementation's own proof.** At τ = 0 no patch is ever census and the engine
reproduces the pre-K-390 baseline; at τ = ∞ every patch is and it reproduces the
shipped K-390 figures. Measured, not asserted: three of the five clips land on
§5.5.1's recorded numbers to four decimals at *both* ends, and the other two
differ by 0.0002 at most. An SSD-mode patch really is the old engine and a
census-mode patch really is the new one.

**The sweep.** One content-blind grid — octaves 0.01…0.16, closed to
half-octaves where the interesting region turned out to be — scored on all five
clips at once, GPU (shipping) path, `flow (defaults)` row, the same sampling as
§5.5.1 either side. Worst-5% block SSIM:

| τ = `CENSUS_GRAD_RMS` | anime | cartoon | gameplay-pov | cinematic-4k | synthetic |
|---|---|---|---|---|---|
| §5.5.1 baseline | 0.7312 | 0.4087 | 0.3671 | 0.9068 | 0.8922 |
| 0 (never census) | 0.7313 | 0.4085 | 0.3671 | 0.9068 | 0.8922 |
| 0.01 | 0.7308 | 0.4087 | 0.3657 | 0.9042 | 0.8927 |
| 0.02 | 0.7302 | 0.4098 | 0.3663 | 0.9017 | 0.8951 |
| 0.04 | 0.7308 | 0.4130 | 0.3639 | 0.9014 | 0.8955 |
| **0.0566** | **0.7324** | **0.4132** | **0.3628** | **0.9016** | **0.8948** |
| 0.08 | 0.7326 | 0.4151 | 0.3602 | 0.9023 | 0.8963 |
| 0.113 | 0.7335 | 0.4170 | 0.3592 | 0.9032 | 0.8961 |
| 0.16 | 0.7323 | 0.4143 | 0.3599 | 0.9034 | 0.8927 |
| ∞ (always census) | 0.7337 | 0.4133 | 0.3598 | 0.9034 | 0.8863 |

Against the baseline row, and the bar (anime **up**, cartoon **up**, gameplay and
cinematic **≥ −0.005**):

| τ | Δ anime | Δ cartoon | Δ gameplay | Δ cinematic | bar |
|---|---|---|---|---|---|
| 0.01 | −0.0004 | 0.0000 | −0.0014 | −0.0026 | animation flat or down |
| 0.02 | −0.0010 | +0.0011 | −0.0008 | −0.0051 | anime down, cinematic short |
| 0.04 | −0.0004 | +0.0043 | −0.0032 | −0.0054 | anime down, cinematic short |
| **0.0566** | **+0.0012** | **+0.0045** | **−0.0043** | **−0.0052** | **3 of 4 — cinematic short by 0.0002** |
| 0.08 | +0.0014 | +0.0064 | −0.0069 | −0.0045 | 3 of 4 — gameplay short by 0.0019 |
| 0.113 | +0.0023 | +0.0083 | −0.0079 | −0.0036 | 3 of 4 — gameplay short by 0.0029 |
| 0.16 | +0.0011 | +0.0056 | −0.0072 | −0.0034 | 3 of 4 — gameplay short by 0.0022 |
| ∞ (K-390 as shipped) | +0.0025 | +0.0046 | −0.0073 | −0.0034 | 3 of 4 — gameplay short by 0.0023 |

**No setting clears the bar, and the shape of the table says why it cannot.**
Game capture falls monotonically as τ rises — every patch the threshold hands to
census is a patch where SSD was doing better — while the cinematic is **U-shaped**,
worst in the middle of the grid and recovering toward both ends. Interpolating,
gameplay holds ≥ −0.005 only below τ ≈ 0.063 and the cinematic only below
τ ≈ 0.019 or above τ ≈ 0.070. **Those windows do not intersect**, and the low
branch is closed anyway because anime is still down at τ ≤ 0.04. The bar is
unreachable on this grid, not narrowly missed at a point the grid stepped over.

**What the U says, and it is the useful part.** A hard switch costs most on
content whose patches sit *near* the threshold, because the field then becomes a
patchwork of two estimators with different biases voting together in
densification. Film grain puts the cinematic squarely in that band — which is
exactly where its minimum is — while line art and game capture sit at the two
extremes and are largely unmixed. That reading also names the next move without
this stage taking it: **blend the two costs across a band rather than switching
at a point**, or add hysteresis so a patch's mode agrees with its neighbours'.
Either would need the two costs put on a common scale, which is a real design
question and a second measurement, so it is not smuggled in here.

**The best row is 0.0566, and it is not what shipped.** Against the K-390 state it is a strictly better
position on the bar's own terms: the miss moves from **0.0023 on game capture**
— the footage the project exists for (K-002), and the clip the bar was written
strict to protect — to **0.0002 on the cinematic**, which is a tenth the size and
inside the harness's own scatter (the anime column wanders 0.0035 across
neighbouring thresholds on 72 triplets). Both animation clips still rise, game
capture recovers 0.0030 of the 0.0073 census cost it, and the synthetic clip is
fixed outright: −0.0059 under global census becomes **+0.0026**. It is the same
trade as K-390, three times smaller and pointed at a less important clip — but it
is still a trade, so the bar says no.

**Reverted (2026-08-19).** The bar is not a target to get close to; the whole
reason §5.5 recorded and removed a 0.036-for-0.012 trade once already is that a
strict bar is the only thing that stops a campaign bargaining its way to a slower,
more complex engine that is not better. Census, the selector, `CENSUS_GRAD_RMS`
and the Huber residual are all out of the tree, and `inverse_search` is
byte-for-byte the function it was before K-390 in both backends. The two tables
above are what the stage produced, and they are worth more than the code was: they
say that the two costs are each better *somewhere*, that a hard switch between them
cannot be tuned into a win, and — from the cinematic's U — that the damage lives at
the boundary between the two modes rather than inside either. That is the brief for
the follow-up named above (blend across a band, or hysteresis on a patch's mode),
should it ever be funded.

**Coverage note, for whoever rebuilds it.** Perlin's finest octave is a 10 px
period, so at 8 px patch scale **every** patch of the two analytic parity scenes is
census-scored at τ = 0.0566; the SSD branch went unproven until
`gpu_matches_the_cpu_oracle` gained a translated 16 px checkerboard, 92% of whose
patches take the SSD branch. That scene left with the revert, and any second attempt
at a two-cost inverse search needs it back. A flat cel with a hard outline is the
tempting scene for this and the wrong one: its interior constrains nothing, so
float noise alone picks different candidates on the two backends and it fails
parity at every threshold, including τ = ∞ where the code is exactly K-390's.

## 6. Test plan

1. Analytic: translating/rotating checkerboard and Perlin textures with known flow —
   endpoint error < 0.3 px mean at half res on translation ≤ 32 px; occlusion mask matches
   the analytic occlusion of a sliding square to ≥ 90% IoU (measured on the raw §2 mask —
   the 1 px safety dilation is for synthesis, and its perimeter alone would exceed the
   IoU budget; the square must slide off-axis, as a motion-parallel silhouette edge is
   aperture-blind).
2. Real-footage goldens: 5 clips (slow pan, fast strafe, rotation, particle spam,
   smoke/gradient sky) — synthesis at φ=0.5 compared visually once, then pixel-locked as
   regression goldens (deterministic by construction).
3. Round-trip: φ=0 and φ=1 return A and B bit-exactly (degenerate-path correctness).
4. The Gate-2 criterion ([16-ROADMAP.md](../16-ROADMAP.md)): 240→60 fps ramp on reference
   game footage, side-by-side against Twixtor output — comparable on clean shots, no
   crash/garbage on the hostile ones (fallback engages instead).
5. Perf: flow pair ≤ 4 ms half-res 1080p, synthesis ≤ 0.5 ms, blur ≤ 2 ms at defaults on
   the reference GPU; CPU reference implementation (required by K-019) matches WGSL within
   1e-3 on the analytic tests — it is the oracle, speed is irrelevant.

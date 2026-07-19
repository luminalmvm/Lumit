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
   a phantom in-between motion. Skip the paper's full variational refinement in v1 —
   measure first; it is the difference between 2 ms and 10 ms and mostly helps large
   untextured regions, rare in game footage.

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
CPU oracle (`lumit_core::fx::cpu::motion_blur`) and WGSL stay op-for-op (§1.6). Adaptive per-tap
counts, the ±1 central difference and the destination-flow fixed point remain follow-ups.

## 5. Parameters and defaults (user-facing, per [08-EFFECTS.md](../08-EFFECTS.md))

Flow interpolation: quality Half/Full (working res), smoothness σ (densification), and
"fallback sensitivity" (the confidence threshold for §3's blend fallback; default
mid). Motion blur: amount k (default 1.0), shutter from comp settings or override,
max taps. Resist adding more knobs — Twixtor's manual is a warning, not a target.

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

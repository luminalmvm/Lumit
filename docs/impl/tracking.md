# Tracking — implementation note

**Decision:** K-415 (classical, global, zoom-aware; learned trackers are a plugin
road). **Related:** K-248 (the tracker runs on full, unaltered footage), K-408 (mask
geometry reaches the engine), docs/08 §7's Tracker and Corner pin rows,
docs/16-ROADMAP Phase 5. This note pins the algorithms, the crate shape, and the test
plan so they are not re-derived per phase.

## In plain terms

Camera tracking watches a shot and works out, from nothing but the pixels, where the
camera was and which way it pointed on every frame — so a 3D layer added afterwards
sits in the scene as if it had been filmed there. It works in two big steps. First it
follows hundreds of small, distinctive patches of the picture from frame to frame
("tracks"). Then it asks: what camera motion, and what arrangement of those patches
in 3D space, would explain all of that sliding at once? The second step is one large
maths problem solved in a single pass. Things that move by themselves — people, cars
— would poison the answer, so the solver finds the *majority* story (the world is
mostly still) and sets aside the tracks that disagree with it; the user can also
simply mask such regions out. A zoom is the one camera change that looks like the
whole world rushing at you: the solver treats the lens's zoom as one more unknown to
recover, and a sudden zoom — a scope-in between two frames — is detected and treated
as "same camera position, new lens" rather than as impossible motion.

## 1. Crate and phases

`crates/lumit-track`, an engine crate: depends on `lumit-core` (time, masks) and may
read flow fields produced by `lumit-flow` (handed in as slices — no direct
dependency unless genuinely needed). No panics, deterministic (fixed iteration
orders, no HashMap iteration into results), budgeted allocations, cancellation
between frames.

| Phase | Delivers | Status |
|---|---|---|
| **1 — tracks** | Feature detection + pyramidal affine KLT + track store + masks + forward-backward verification. The substrate everything else reads. | **Built** (see §2's "As built") |
| **2 — two-view** | Normalised 8-point/7-point fundamental, LO-RANSAC, keyframe selection, epipolar-based dynamic-track segmentation, the zoom-burst detector. | Open |
| **3 — solve** | Rotation averaging → global positions → triangulation → sparse-Schur Levenberg–Marquardt bundle adjustment with per-segment focal. | Open |
| **4 — surface** | Bridge + Tracking UI: run, point cloud over the picture, solved camera → a Camera layer, 2D track → keyframed transform / corner-pin export, mask pick. | Open |

Phase 1 first and alone: every later phase's quality is decided by track quality.

## 2. Phase 1 — the track substrate

- **Detection:** Shi–Tomasi (min eigenvalue of the 2×2 gradient normal matrix) on a
  grid-bucketed frame (e.g. 16×16 buckets, best-N per bucket) so features stay
  spread; quality floor relative to the frame's best. Deterministic ordering: bucket
  row-major, then response descending, ties by (y, x).
- **Tracking:** pyramidal Lucas–Kanade with an **affine** patch model (2×2 A + d),
  3–4 pyramid levels, window ~15 px at level 0, inverse-compositional update. The
  affine model is what survives scale change (zoom) and rotation; translation-only
  KLT is what dies there.
- **Seeding:** the previous frame's displacement (constant-velocity prior), and —
  where a flow field for the pair is available — the flow vector at the point wins
  as the seed. Flow is a *seed*, never a verdict.
- **Verification:** forward–backward error < 0.5 px at level 0, and NCC of the
  settled patch against the track's *reference* patch (updated on drift, kept on
  success) above a floor. Fail → the track ends; it is never silently teleported.
- **Masks:** a track is neither detected in nor allowed to wander into a masked
  region. Masks arrive as `lumit_core::mask` polylines (K-408's flattening) at comp
  scale; point-in-polygon on the flattened path, inverted per the mask's flag.
- **Store:** `Track { id, points: Vec<(frame, x, y)>, state }` in a `TrackSet` keyed
  by contiguous frame ranges; f64 coordinates at source raster scale (K-248: the
  full, unaltered footage — no comp scaling, no retime; mapping through retimes
  happens at export).
- **Re-detection:** when live tracks in a bucket fall below a floor, detect there
  again — long shots keep density without global re-seeds.

### Phase 1, as built

`crates/lumit-track` (2026-08-20). Everything above holds; the surface is
`Tracker::new(TrackSettings) → push(frame, FramePlane, Option<FlowSeed>) →
finish() → TrackSet`, one frame per call, so the caller's frame loop is the
cancellation seam and the crate never owns a long uninterruptible run.
`TrackSet` answers what the later phases ask: `tracks`, `get`, `frame_range`,
`tracks_over(from, to)`, `correspondences(from, to)` and
`median_log_scale(frame)` — the last being phase 2's zoom-burst input, exposed
and tested now so the detector inherits a measurement rather than a promise.
Every step keeps its affine `A` (plus its NCC and forward-backward numbers) on
`TrackStep`.

Nine things are deviations from, or decisions under, the wording above. Each was
forced by building it:

1. **The pyramid's box downsample is re-implemented here**, not shared.
   `lumit-flow`'s is `pub(crate)`, and that crate pulls in `wgpu`; an engine
   crate does not take a GPU dependency to borrow four lines of arithmetic. The
   arithmetic is identical, so the two crates see the same coarse pictures.
2. **A minimum separation between features** (6 px by default) sits inside
   "best-N per bucket". Without it every one of a bucket's N picks lands on the
   same corner, because a strong corner scores highly on all of its own pixels.
3. **The NCC rule is two thresholds, not one.** Below `ncc_floor` (0.8) the
   track ends; between it and `ncc_refresh` (0.95) the reference patch is
   replaced by the current one; above, the reference is kept. That is the note's
   "updated on drift, kept on success" made operable — one threshold either ends
   healthy tracks that changed slowly or lets a track walk off its feature.
4. **The comp→source conversion happens at the mask, once.** Masks arrive in
   px@comp (K-408) and tracks live in source raster pixels (K-248), so
   `ExclusionMask::from_mask`/`from_polyline` take the factor and store source
   pixels; the per-point test does no arithmetic.
5. **Template gradients are central differences of the bilinear-sampled
   template**, not a precomputed gradient plane. Patch centres are sub-pixel
   almost always, and a gradient plane would be the gradient of the wrong point.
6. **A pyramid level whose window does not fit is skipped, not failed.** A track
   fails only when level 0 cannot hold its window. Failing at any level would
   put a dead band the width of the coarsest level's window round the frame.
   Levels are also clamped down on small frames.
7. **`TrackStep` carries `ncc` and `fb` as well as `A`** — one line each, and
   they are what phase 2 weights tracks by and what a UI would draw.
8. **`TrackState::Moving` exists and is never set.** Phase 2 sets it; naming it
   now keeps the store's shape stable across the two.
9. **Accuracy is asserted as a distribution, not a single bound.** Features
   sitting on a near-straight edge are ill-conditioned along that edge and
   always will be, so the tests pin the median hard (a fiftieth of a pixel on
   translation) and the tail loosely (p90, worst) — a single threshold would
   either be meaningless or would fail on physics.

**Measured, not a gate** (`perf_100_features_over_30_frames_of_640x360`,
`--ignored`, release): 100 features over thirty 640×360 frames runs at **11.0
ms/frame** with re-detection off and **24.4 ms/frame** with it on every frame.
The difference is one whole-frame Shi–Tomasi response pass; its box sums are
separable and nobody has separated them yet. That is the first place to look if
the tracker ever needs to be faster, before any of this moves to WGSL.

## 3. Phase 2 — geometry (pinned choices)

- Hartley-normalised 8-point for hypotheses inside LO-RANSAC (local optimisation:
  re-fit on inliers each time a better model appears); 7-point for minimal samples.
  Sampson distance as the residual; threshold in normalised units derived from a
  pixel threshold (~1.5 px) at the frame's scale.
- **Keyframes:** pick pairs by parallax (median rotation-compensated displacement)
  and inlier ratio — GRIC-style H-vs-F comparison decides whether a pair carries
  parallax at all (homography-explained pairs are rotation/zoom-only and are still
  usable for rotation and focal, not for translation).
- **Dynamic segmentation:** per track, the lifetime profile of epipolar residuals
  against the dominant model over its keyframe pairs; a track consistently outside
  is marked `moving` (excluded from the solve, kept for object tracking); a track
  with a clean prefix and a dirty suffix is split at the change. This is the
  classical reading of the trajectory-based methods (ParticleSfM/RoMo) without their
  networks.
- **Zoom-burst detector:** per adjacent-frame pair, the median over tracks of
  `log(scale)` recovered from the affine KLT A-matrices (and, as a cross-check, the
  scale of a similarity fit to the track displacements about their centroid). A
  burst above threshold for one pair, with rotation/translation residual small under
  a scale-only model, is a **zoom cut**: a segment boundary where pose is continuous
  and focal is free. A sustained non-zero value is a zoom ramp (smooth focal within
  the segment).

## 4. Phase 3 — the solve (pinned choices)

- Rotation averaging over the pairwise relative rotations (robust L1→IRLS L2, the
  standard ladder), then global positions (BATA-style translation averaging), then
  DLT triangulation with cheirality checks, then **one bundle adjustment**:
  Levenberg–Marquardt on reprojection error with the sparse Schur complement (poses
  + per-segment focal in the reduced camera system, points marginalised). Huber loss
  on residuals. All f64.
- Camera model: pinhole, principal point fixed at centre, focal per segment (one
  unknown for a locked segment, spline knots for a ramp), optional k1/k2 per
  segment. Focal initialisation from the F→K self-calibration of the best
  keyframe pairs (Kruppa-lite / Bougnoux formula), clamped to a plausible FoV range.
- Determinism: fixed RANSAC seeds derived from (frame indices, track count) — the
  same clip solves to the same answer bit-for-bit on one machine, and the tests pin
  statistics rather than raw floats across machines.

## 5. Test plan

- **Synthetic first, real second.** Phase 1: rendered test sequences (a textured
  quad under known affine/projective motion, generated in-test) — recovered tracks
  vs analytic ground truth within thresholds; a synthetic zoom pair (same centre,
  scale s) must read as `log s` in the burst detector within 2%. Masked-region
  tests: no track born in or entering the mask. Determinism: two runs, identical
  `TrackSet`. **Landed** as `crates/lumit-track/src/tests.rs` — nineteen tests
  over one procedural texture with no assets: translation, rotation, scale and a
  proper affine shear against analytic ground truth; the zoom pair at three
  scales; a moving occluder ending tracks and the buckets behind it
  repopulating; both mask polarities at birth and on entry; an eleven-pixel jump
  carried first by the pyramid alone and then by a flow seed where only one
  level is allowed; the whole `TrackSet` compared with `assert_eq!` across two
  runs.
- **The accuracy tests measure followed points only.** A track's *birth* point
  has zero error by construction — the ground truth is `M_f(M_f0⁻¹(p))`, which
  is `p` itself at `f = f0`, for any `p` and any tracker — so the error
  distribution excludes it. This is not fastidiousness: a tracker broken badly
  enough that every track dies after one step still yields a set of one-point
  tracks, all of them "exact", and every median and quantile threshold passes on
  it. The same run must also be pinned by `tracks_over(first, last)` rather than
  by a live count, because a track born on the final frame is live too. Both
  traps were live in the first cut of these tests and were found by breaking the
  KLT update's sign and watching four tests not notice.
- Phase 2: synthetic two-view geometry with known F (points from a known camera
  pair + noise + a planted fraction of "moving object" points following a second
  motion): the dominant model's inliers must exclude the planted movers; the
  GRIC gate must call a rotation-only pair rotation-only.
- Phase 3: a synthetic orbit + a synthetic dolly with a mid-shot zoom cut: solved
  poses within tolerance of ground truth (ATE after similarity alignment), focal
  recovered per segment within 2%, and the zoom cut landing on the right frame.
- Real clips (the flow-quality staging folder) are the eyeball harness, not CI.

## Open questions

- GPU KLT: the pyramid and gradient passes are natural WGSL; phase 1 is CPU-first
  and profiled before anything moves (13-PERFORMANCE budgets decide). Now
  profiled, and the answer is not the KLT: the whole-frame Shi–Tomasi response
  pass costs more than the tracking does whenever re-detection runs (§2's "As
  built"). Separable box sums first; WGSL only if that is not enough.
- Object rigid-pose solve (track group → 6-DoF against the solved camera) is phase
  4+ and needs its own note section when reached; 2D export ships first.
- Lens distortion beyond k1/k2 (anamorphic) — revisit against real footage.

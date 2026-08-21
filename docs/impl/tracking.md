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
| **2 — two-view** | Normalised 8-point/7-point fundamental, LO-RANSAC, keyframe selection, epipolar-based dynamic-track segmentation, the zoom-burst detector. | **Built** (see §3's "As built") |
| **3 — solve** | Rotation averaging → global positions → triangulation → sparse-Schur Levenberg–Marquardt bundle adjustment with per-segment focal. | **Built** (see §4's "As built") |
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

### Phase 2, as built

`crates/lumit-track/src/{geom,pairs,segment}.rs` (2026-08-21). Everything above
holds. The surface is four free functions plus two store operations, all in
**source raster pixels** so nothing downstream converts:

- `fundamental_eight_point`, `fundamental_seven_point`, `homography_dlt`,
  `sampson_distance`, `transfer_distance`, `project` — the models and residuals,
  usable on any `&[Correspondence]`, which is what lets the tests drive them from
  a written-down camera pair instead of from tracks.
- `estimate_pair(pts, source_size, from, to, &GeometrySettings) -> Option<PairGeometry>`
  and `TrackSet::pair_geometry(from, to, …)`, the same thing over a stored pair.
  `PairGeometry` carries both models, the inlier ids, the inlier ratio, the
  parallax, both GRIC scores and the `PairVerdict`.
- `select_keyframes(&TrackSet, &GeometrySettings) -> Vec<PairGeometry>`.
- `segment_dynamic_tracks(&mut TrackSet, &[PairGeometry], &SegmentSettings) -> Segmentation`,
  and the store operation it uses, `TrackSet::split_track(id, after_frame) -> Option<u32>`.
- `detect_zoom(&TrackSet, &ZoomSettings) -> Vec<ZoomBoundary>`.

Nine things are deviations from, or decisions under, the wording above:

1. **There is no SVD in the crate and there does not need to be.** Both null
   spaces are the smallest eigenvectors of the 9×9 normal matrix `AᵀA`, found
   with a cyclic Jacobi eigensolver (fixed sweep order, fixed cap), and the
   rank-2 enforcement uses the same solver on `FᵀF`: since `σᵢuᵢ = F vᵢ`,
   summing `(F vᵢ)vᵢᵀ` over the two largest is exactly the rank-2 truncation
   with no `U` ever formed. An SVD would have been a dependency or two hundred
   lines, for this.
2. **The 7-point cubic's coefficients are read off four evaluations of the
   determinant** (`α` = 0, 1, −1, 2) rather than expanded symbolically. Same
   four numbers, a tenth of the algebra, and no transcription risk. The roots
   come from the trigonometric form where there are three and Cardano where
   there is one, sorted ascending, so the "walked deterministically" is the sort
   and not a convention anyone has to remember.
3. **Two normalisations, doing different jobs.** Each fit applies its own
   data-driven Hartley conditioning, as pinned. On top of that the *estimator*
   works in a fixed frame-sized normalisation (centre at the frame's middle,
   long edge at ±1), which is what turns the pixel threshold into the units the
   residuals are measured in without that conversion depending on which sample
   was drawn. Sampson distance is covariant under an isotropic similarity, so
   thresholding at `pixel_threshold · s` there is exactly thresholding at
   `pixel_threshold` pixels — the models are handed back denormalised, in pixels.
4. **The RANSAC loop is one function used twice**, for F (7-point samples,
   Sampson) and for H (4-point DLT samples, symmetric transfer). The second gets
   a different seed stream: the same seed would draw the same index sequence,
   and four of seven indices is not an independent search.
5. **The early exit is recomputed from the best-so-far inlier count**, so the
   iteration at which the loop stops is a function of the data alone. It is also
   floored at the current iteration, so a late improvement cannot make the cap
   retreat behind where the loop already is.
6. **GRIC's σ is a setting, not a constant** (`sigma_px`, default 0.75 —
   half the inlier threshold). The rest is Torr's standard: `r = 4`, `λ₁ = ln r`,
   `λ₂ = ln(r·n)`, `λ₃ = 2`, `d`/`k` of 3/7 for F and 2/8 for H.
7. **A split drops the one step that straddled it.** `steps[i]` measures the
   motion from `points[i]` to `points[i+1]`; the split has just declared that
   motion was two different things, so it belongs to neither half and both
   halves keep `steps.len() + 1 == points.len()`. The prefix keeps the id and
   becomes `Ended`; the suffix takes a fresh id, inherits the state, and records
   `parent: Some(id)` — one additive field on `Track`.
8. **Keyframe selection returns a short final pair rather than leaving a gap.**
   At the tail of a shot there are no frames left to reach for, so the last pair
   can be below the parallax floor. Dropping it would hand phase 3 a stretch of
   shot with no geometry at all, which is worse; the pair carries its own
   `parallax` and phase 3 can weigh it.
9. **The zoom detector has two thresholds and a cross-check, not one
   threshold.** `ramp_threshold` (0.004) is the floor above which the lens is
   moving at all; `cut_threshold` (0.05) is what an isolated pair must clear to
   be a cut rather than a one-frame ramp; and a cut must additionally be
   explained by a scale about a single centre (`scale_only_px`, and the fitted
   log-scale agreeing with the affine matrices' median within `cross_check`).
   That last is what stops a hard forward dolly reading as a scope-in — near
   things grow more than far ones, and a scale-only fit says so. It has its own
   test.

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

### Phase 3, as built

`crates/lumit-track/src/{solve,bundle}.rs` (2026-08-21). The ladder above holds
end to end. One public entry point:

```
solve_camera(&TrackSet, &[PairGeometry], &[ZoomBoundary], &SolveSettings)
    -> Result<CameraSolve, SolveError>
```

`CameraSolve` is the shape phase 4's export reads: a `SolvedPose` per frame
(world→camera rotation, camera centre, segment index, the segment's focal
repeated per frame, that frame's mean reprojection error, and a `PoseSource` of
`Keyframe`/`Resection`/`Interpolated`), the `SolveSegment` table (frame range,
focal in source pixels, a `ramp` flag), the `ScenePoint` cloud (track id and
position, colourless — the tracker reads luma and has no colour to give), the
keyframe list, the mean reprojection error over every observation the bundle
saw, and a `notes` list. `SolveError` is a refusal, never a fault: `NoTracks`,
`NoKeyframes`, `RotationOnly`, `NoPoints`.

Ten things are deviations from, or decisions under, the wording above. Every one
of them was forced by measuring the version that followed the note literally:

1. **Bougnoux's closed form was written, measured, and replaced.** On the
   synthetic orbit its per-pair answers ran from 57 px to 578 px for a true
   300 px, because the formula divides by a quantity that vanishes at the
   critical configurations and every real shot spends time near one. What
   survives is the *constraint* it solves — that `K_toᵀ·F·K_from` must have two
   equal singular values and one zero — minimised numerically over a bounded
   focal range instead of solved in closed form, on the median over **all** the
   shot's pairs at once. A 96-step sweep in log-focal finds the basin and forty
   ternary steps polish it; both are fixed, so the work and the answer are the
   same every run.
2. **A zoom cut ties the segments' focals together rather than splitting them
   apart.** The detector already measured the ratio: a cut's `log_scale` is the
   median over hundreds of tracks of how much every patch grew, and with pose
   continuous across the cut that growth *is* the focal ratio. So the whole shot
   has one focal unknown, each segment's focal is that unknown times the product
   of the cut ratios before it, and every pair in the shot votes on it. That is
   both stronger and simpler than per-segment self-calibration, and it is what
   makes the 300→420 px scope-in come back at 297.3 and 416.7.
3. **The solve runs twice.** The focal a first pass stands on is the weakest
   number in the file, and a focal 30 % out bends every relative rotation with
   it — the essential decomposition's angle scales with it almost proportionally.
   The second pass re-derives every relative pose from the focal the first pass's
   bundle settled on, which is the *strongest* number available because it was
   fitted to every observation at once. Bounded at two (`SolveSettings::passes`):
   a third moves nothing measurable, and an unbounded loop is not a solve, it is
   a hope.
4. **Triangulation does not gate on reprojection; the bundle does.** The first
   cut of this file refused any point whose initial reprojection exceeded the
   threshold, and the honest result was `NoPoints` on a shot that solves
   perfectly well — because before the bundle the model is an *initialisation*,
   and judging it by the number the bundle exists to minimise throws away exactly
   the points that are about to be fixed. Triangulation now checks cheirality and
   ray parallax only, which are the qualitative failures. Reprojection is the
   gate immediately after the bundle, where it is the right question, and
   anything it drops is followed by a second bundle over what is left.
5. **The colinear guard is reported, not fatal.** A dead straight dolly gives
   every pair the same baseline direction, and direction constraints then say
   nothing about the *ratios* between baselines — the classic degeneracy of
   translation averaging. But the point cloud and the bundle do constrain the
   spacing, so refusing the shot would be wrong; the solve runs and returns
   `SolveNote::ColinearBaselines`, which says where the answer came from. The
   test measures the separation rather than trusting it: the straight dolly's
   direction scatter reads 0.0006, an arced one 0.02–0.04, against a threshold
   of 0.001.
6. **A rotation-only shot is a hard refusal.** Where no pair carries a baseline
   there is no position to solve and no depth to recover, and `SolveError::
   RotationOnly` says so rather than returning a trajectory the pictures do not
   contain. The rotations *are* recoverable from the homographies, and a nodal
   solve — a Camera layer that only turns — is a real product; it is phase 4+
   work and is in TODO, not smuggled in here.
7. **A per-segment focal ramp is owed.** The note allows one focal per segment
   for phase 3 with the ramp case recorded, and that is what shipped: a segment
   containing a detected zoom ramp is flagged `ramp: true` and reported as
   `SolveNote::ZoomRamp`, and its focal is one number over the whole run rather
   than the note's spline knots. The bundle's parameter layout already carries
   one focal per segment as an independent unknown, so knots are additional
   columns in the same reduced system rather than a rewrite.
8. **Resection is a trim, not a random sample.** The note says "RANSAC-lite";
   what earns its place is a normalised DLT over the frame's tracks, two rounds
   of dropping whatever disagrees and refitting, then a six-parameter
   Gauss–Newton refinement sharing the bundle's own Jacobian. Phase 2 has already
   removed the moving tracks and triangulation has already refused anything that
   would not sit still, so the outlier rate a random sampler would be paying for
   is not there. A frame that still cannot be explained is not given a fitted
   pose: it is interpolated between its neighbours and labelled
   `PoseSource::Interpolated`, and the count is reported.
9. **There is still no SVD in the crate.** The essential matrix's factorisation
   needs `U` and `V`, which is exactly what phase 2 avoided — but the same
   identity does it again: `V` is the eigenvectors of `EᵀE` from the Jacobi
   solver, and `uᵢ = E·vᵢ/σᵢ` gives `U` with `u₃ = u₁×u₂`, which also forces
   `det U = det V = +1` and so makes both `R` candidates proper rotations by
   construction. The nearest rotation to a matrix (`M·(MᵀM)^(−½)`, with the
   reflection case handled by flipping the smallest eigendirection) comes from
   the same solver and is used three times: the homography's rotation, the DLT
   resection's, and the tests' Umeyama alignment.
10. **The reduced camera system is dense and factorised outright**, so the
    bundle is cubic in the *keyframe* count — marked with a `ponytail:` comment
    at the function. The points, which are the thousands, are marginalised
    sparsely: each point's `W` block lists only the parameters that actually see
    it, so the Schur update is quadratic in one point's observation count and not
    in the problem. The note's own "keyframe counts are small" is what makes the
    dense half the right trade; a shot that somehow lands thousands of keyframes
    wants a sparse factorisation there, not a bigger machine.

**Measured on the phase-3 tests** (synthetic, deterministic, in-test): a 25-frame
orbit recovers its focal as 300.07 px against a true 300 (0.02 %), with an
absolute trajectory error after similarity alignment of 0.039 % of the
trajectory's extent and a mean reprojection of 0.10 px over 150 points; a
30-frame arcing dolly through a 1.4× scope-in at frame 14 finds the cut on the
right frame and recovers 297.3 px and 416.7 px against 300 and 420 (0.9 % and
0.8 %), ATE 0.054 %, mean reprojection 0.10 px; with thirty planted movers in the
orbit, the cloud is exactly the hundred and twenty still tracks and the ATE is
unchanged at 0.042 %. Phase 2 marks fifteen of the thirty movers as `Moving`;
the other fifteen are refused later, by cheirality and by the post-bundle
reprojection gate — both halves of that are asserted, so neither can quietly
stop working.

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
  GRIC gate must call a rotation-only pair rotation-only. **Landed** as the
  second half of `crates/lumit-track/src/tests.rs` — sixteen tests, and not one
  of them renders a picture. Phase 1's job was pixels-to-tracks and its tests had
  to draw; phase 2's job is arithmetic over correspondences, so its ground truth
  is a camera pair written down exactly and its footage is the projection of a
  known cloud. A test that rendered here would be measuring the tracker again.
  In order: the eight-point recovered on held-out points (compared by epipolar
  residual, never by matrix entries — F is only defined up to scale); the same
  under half a pixel of jitter, which is also the only condition under which the
  rank-2 enforcement is visible and so is where it is pinned; the seven-point
  cubic's roots, one exact and the others visibly not; the GRIC gate both ways,
  each pinning the *homography's own* quality as well as the verdict; LO-RANSAC
  against twenty planted movers; keyframe selection by parallax; a lifelong mover
  marked `Moving` with the still world untouched; a mover that starts mid-shot
  split at the change with both halves' points rejoining to the original exactly;
  a refused split leaving the store byte-identical; the zoom cut, the ramp, a
  still lens, and a forward dolly that bursts like a scope-in and must not be
  called one; the whole pipeline run twice and compared with `assert_eq!`.
- **The phase-2 thresholds are mutation-checked, not eyeballed.** Flipping one
  sign in the Sampson numerator fails eight tests; one sign in the DLT row fails
  two; one coefficient in the cubic fails one; removing the rank-2 enforcement
  fails one. Every threshold in that half of the file was placed after measuring
  both the true value and the broken one, and sits in the gap rather than beside
  either — the determinant bound is nine orders of magnitude from the value that
  would pass it wrongly.
- Phase 3: a synthetic orbit + a synthetic dolly with a mid-shot zoom cut: solved
  poses within tolerance of ground truth (ATE after similarity alignment), focal
  recovered per segment within 2%, and the zoom cut landing on the right frame.
  **Landed** as the third part of `crates/lumit-track/src/tests.rs` — eight
  tests, and like phase 2's they draw nothing: a camera path written down
  exactly, a known cloud, its projection, and a deterministic sub-pixel jitter
  standing in for what a good KLT step leaves behind. In order: the orbit,
  checked on focal, on ATE after a Umeyama alignment (which is not a courtesy —
  the seven similarity numbers are precisely what no photograph carries), on
  mean reprojection, on the count of in-between frames actually resectioned
  rather than guessed, and on nothing having needed interpolation; the arcing
  dolly through a 1.4× scope-in, checked on the cut's frame, on both segment
  focals, on pose *continuity* across the cut (the step across it against the
  typical step either side, because the camera is moving and a step of zero
  would be the wrong assertion), and on every frame carrying its own segment's
  focal; thirty planted movers, checked both on how many phase 2 marked and on
  the cloud being exactly the still tracks; a nodal pan refused with
  `SolveError::RotationOnly`; a dead straight dolly reporting
  `SolveNote::ColinearBaselines` while an arced one over the same ground does
  not; the boundary refusing an empty set and a set with no pairs; the whole
  solve run twice and compared with `assert_eq!`; and the bundle driven
  directly, from a start knocked off by a fifth of a unit in position, three
  quarters of a degree in rotation, a twentieth of a unit per point and four per
  cent in focal, required to come back to a millionth of a pixel.
- **The phase-3 thresholds are mutation-checked too.** Flipping the sign of the
  rotation Jacobian's `−[v]×` term in `bundle.rs` — one `-` — fails five of the
  eight, including the three that only look at the pipeline's output; flipping
  the sign of the Schur back-substitution's `Wᵀ·Δc` term fails four, and takes
  the recovered focal from 300 px to 223.
- **The margins are not uniform, and the reason is worth writing down.** Where
  the input carries jitter the threshold sits two to eight times above the
  measured value — the reprojection means at 0.10 px against 0.2, the dolly's
  focals at 0.8–0.9 % against 2 %, the trajectory errors at 0.04–0.05 % against
  0.3–0.4 %. That band is close enough to bite and far enough not to flake on a
  different libm. But `the_bundle_converges_from_a_perturbed_start` feeds the
  bundle *noiseless* observations, so its true minimum is exactly zero and it
  reaches 1.6e−14 px; a threshold picked from the jittered band (0.02 px) was
  a thousand billion times slack, and a deliberately broken Schur
  back-substitution slid under it at 0.022 px — the test all but missed the one
  mutation it exists to catch. Its three thresholds are therefore a millionth
  of a pixel, a millionth of a pixel of focal, and a millionth of the
  trajectory's extent: still six to eight orders above the measurement, and
  decisive. **The rule is that a threshold is set from what the test's own input
  can achieve, not from a house number.**
- Two other phase-3 thresholds were placed by measurement rather than by
  eye after the fact, and say what their tests claim. The mover test's
  trajectory bound is the clean orbit's own (0.3 %, measured 0.042 %) rather
  than a looser one, because its stated claim is that movers cost the solve
  nothing — a slacker bound there would let them start costing something in
  silence. And the zoom cut's pose-continuity bound is ±15 % of the median step
  rather than ±60 %: the dolly is a constant-rate arc whose every true step is
  the same length to five decimals, and every *solved* step lands within 2.3 %
  of the median, so ±60 % would have passed a camera that jumped half a step at
  the cut.
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

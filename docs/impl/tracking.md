# Tracking — implementation note

**Decision:** classical, global, zoom-aware; learned trackers are a plugin road.
**Related:** the tracker runs on full, unaltered footage, and mask geometry reaches
the engine. The planar tracker and its Corner pin, and the same track onto a layer's
transform, are in §6, and one- and two-point tracking is in §7. Also docs/08 §4's
Tracker and Corner pin rows, and docs/16-ROADMAP Phase 5. This note pins the
algorithms, the crate shape, and the test plan so they are not re-derived per phase.

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
| **4 — surface** | The Camera track *effect* (identity render, Analyse/Cancel actions, status), the background analysis job keyed to (media, settings) with the sidecar `track/` cache, the solve-linked dynamic Camera layer reading through the comp→clip→source time chain, Convert to keyframes, the point-cloud overlay with select → Null/Solid, and `ParamKind::Action`. 2D track → keyframed transform / corner-pin export rides the same store. | **Built** (§5a, §5b, §5c) |
| **4b — planar** | The Planar track *effect*, a quad followed as a homography per frame against the reference frame, filed under the effect instance in the same store and sidecar, with **Create corner pin** writing the four corner pairs onto another layer as keyframes — and **Create transform keys** writing the same corners as Position, Rotation and Scale. | **Built** (§6) |
| **4c — points** | The same effect's **Follow** row turned to one or two small boxes, each followed on its own by the median step of the features inside it, reported as the same `PlanarTrack` under a translation or a similarity. | **Built** (§7) |

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
  region. Masks arrive as `lumit_core::mask` polylines, already flattened, at comp
  scale; point-in-polygon on the flattened path, inverted per the mask's flag.
- **Store:** `Track { id, points: Vec<(frame, x, y)>, state }` in a `TrackSet` keyed
  by contiguous frame ranges; f64 coordinates at source raster scale (the
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
   px@comp and tracks live in source raster pixels, so
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
  scale of a similarity fit to the track displacements about their centroid). A pair
  is never judged against zero (that reading lost the train POV — the resolved Open
  question below): each hot pair is first classified by the
  **radial-flow-versus-parallax signature** — a zoom is a scale about one centre and
  leaves only noise behind a scale-only fit, while travel moves near things more than
  far ones and leaves a residual that is a roughly constant *fraction* of the flow —
  and the pairs the signature reads as travel form a windowed-median **baseline** the
  others are measured as an excess above. An isolated pair of excess above the cut
  threshold, still passing the scale-only cross-check, is a **zoom cut**: a segment
  boundary where pose is continuous and focal is free. A run of excess is a zoom ramp
  (smooth focal within the segment, solved as knots — §4). A dolly-like burst is
  *no boundary at all*: it is camera motion, which is phase 3's subject.

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
9. **The zoom detector judges a pair against its neighbours, not against
   zero** (redesigned 2026-08-26 after the train-POV failure). Three
   mechanisms, in order. *The signature:* a hot pair's scale-only fit leaves a
   residual, and that residual as a fraction of the radial displacement the
   scale accounts for separates lens from travel — a zoom leaves noise, travel
   leaves parallax, and parallax is a constant fraction of the flow however
   slow the travel (`parallax_fraction`, 0.15;
   measured ≈0.03–0.06 on synthetic zooms against ≈0.2–0.4 on dollies). A pair
   too slow to judge alone is pooled with up to `signature_window` (6)
   neighbours either side until the scale displacement reaches `signature_px`
   (4.0) — parallax accumulates coherently with travel while tracker noise
   does not, which is what makes a slow dolly and a slow zoom separable at
   all. *The baseline:* the pairs the signature reads as travel (or that are
   cold) form a windowed median (`baseline_window`, 12 either side) — the
   shot's own growth. *The excess:* a boundary is lens-like growth above the
   baseline; `ramp_threshold` (0.004) is the floor on the excess,
   `cut_threshold` (0.05) is what an isolated pair's excess must clear to be a
   cut, and a cut must still be explained by a scale about a single centre
   (`scale_only_px`, and the fitted log-scale agreeing with the affine
   matrices' median within `cross_check`). A boundary's `log_scale` is the
   median excess — during travel that is the focal ratio itself, with the
   shot's own growth subtracted. A purely still or purely zooming shot has a
   baseline of zero everywhere, so every pre-redesign behaviour is unchanged
   there; a dolly-like burst, which used to come back as a one-pair `Ramp`,
   now raises no boundary at all. Each mechanism has its own test.

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
7. **A ramp segment's focal is spline knots** (closed 2026-08-26; it shipped
   as one averaged number first, and the debt was recorded here). A segment
   flagged `ramp: true` lays sparse knots over its detected ramp runs — one
   per `knot_spacing_frames` (25, about a second), capped at
   `max_knots_per_segment` (8) — and each knot is an additional column of the
   same reduced camera system, exactly as predicted: a camera inside a ramp
   reads a *linear blend* of its two bracketing knots, so its observations
   contribute to two focal columns with weights `1 − t` and `t`, and a
   constant segment still reads one. The knots are piecewise-linear rather
   than cubic — over the dozen frames a rack spans, the chord of an
   exponential ramp is within about a per cent of the curve, and the bundle
   fits the knot values freely anyway. Knot initialisation compounds the
   detector's own measured per-pair rate along the run, so the whole shot
   still hangs off one base focal for self-calibration (deviation 2's tie,
   extended along ramps). `SolveNote::ZoomRamp` keeps reporting where the lens
   moved; the per-frame values ride out on `SolvedPose::focal_px`, which the
   export already reads, so nothing downstream changed shape. Measured: the
   forward-travel-then-rack fixture recovers the 1.4× ramp's end-to-end ratio
   as 1.404 (0.3%), with the knot Jacobian pinned separately to a noiseless
   floor of ~1e-5 px of focal.
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
stop working. The train-POV fixtures (2026-08-26): a 48-frame forward travel
along a gently curving track with a 300→420 px rack over twelve mid-shot pairs
finds one `Ramp` boundary on exactly those pairs (rate 0.0274 against a true
0.0280 once the travel baseline is subtracted) and solves it to a focal *curve*
whose end-to-end ratio is 1.404 against a true 1.4; the same travel with a
single-pair scope-in returns one `Cut` on the right frame with `log_scale`
within 0.01 of ln 1.4; the travel alone returns nothing. The absolute focal on
the travelling shot sits a uniform 3.5 % low and the ATE at 1.3 % of the extent
— an order looser than the sideways shots, because a shot dominated by forward
motion is the classical near-critical configuration for self-calibration; the
ramp's *shape* is what the knots recover, and they recover it to a third of a
per cent.

## 5a. Phase 4, stage 1 — the model half, as built

`crates/lumit-core` (2026-08-21). Stage 1 is everything the design decides that can be
built and tested **without decoding a single frame**: the schema kind, the effect,
the link, and the bake. A written-down solve stands in for a real one, injected
through a trait, and every claim below is asserted against it.

- **`ParamKind::Action`** (`fx/schema.rs`), declared `#[action(label = "…")]`. A
  button, not a value: no `EffectValue` variant, no `EffectParam` written by
  `instantiate`, nothing appended by `backfill_builtin_params`, nothing pushed
  into the resolved arena — and therefore nothing in the frame key, so pressing
  Analyse renames no cached frame. `default_param_value` became
  `-> Option<EffectValue>` to say that in the type; three call sites skip `None`.
  `reference.rs` prints it as kind `action` with no default and no range, which is
  all the manual's table can honestly say about a button.
- **The Camera track effect** (`fx/effects/camera_track.rs`), Utility, catalogue
  entry 91, identity render (`is_image_op → false`, the Controls family's
  convention, here for a different reason: this one holds a *job*). Analyse
  and Cancel are the two Actions; Feature density, Use masks and Show points are
  the value rows. `DENSITY` is the `(grid.0, grid.1, per_bucket)` table the
  analysis job reads into `TrackSettings` — it lives on the effect because the
  crate that owns the control cannot depend on the crate that owns the tracker,
  and the job reads it the other way round, which is the only direction there is.
- **`LayerKind::Camera` gained `solve_link: Option<Uuid>`** (`model.rs`,
  docs/03 §5.6), serde-defaulted and skipped when absent, so no saved project
  changes a byte. `Composition::active_camera` was split out of `camera_pose` and
  `stored_camera_pose` out of its body, because the link needs the *layer* as well
  as the numbers and one rule for which camera is active has to serve both.
- **`crates/lumit-core/src/track.rs`** is the whole of the derivation:
  `CameraSolveStore` (two methods — the solved range for one media, and one pose
  at one solved frame), `camera_pose_at` / `camera_pose_of` returning a
  `LinkedPose` of pose plus `LinkState`, and `bake_solve_link` building the
  Convert-to-keyframes batch.
- **`Op::SetCameraSolveLink`** and **`OpError::CameraLinked`** (`ops.rs`). The
  refusal is a `solve_link_guards` function shaped exactly like the existing
  `lock_guards`, checked in `apply` before the match — one guard for every caller
  and every op yet to be written. *Reversed later: the refusal and the error
  are gone, and the same two ops now write the correction lane; see §5f.*

Six things are deviations from, or decisions under, the design's wording:

1. **The store hands back poses already in Lumit's camera terms**, not
   `lumit_track::CameraSolve`. `lumit-core` cannot depend on `lumit-track` (the
   crate graph runs the other way), and more to the point the conversion —
   world-to-camera rotation to AE rotation, solve units to comp pixels, focal in
   source pixels to `zoom` — is a real piece of work that belongs in the store,
   next to the solve it is converting. Stage 2 writes it; stage 1 would only have
   guessed at it.
2. **"Last-derived-hold" is a clamp into the solved frame range**, not a cached
   value. A cache would make the same frame answer differently depending on what
   was asked for before it, which is the opposite of what a render needs. Clamping
   *is* the hold — the nearest solved frame is the last derived motion — and it is
   stateless, deterministic and free. Where the link resolves nowhere at all there
   is nothing derived to hold, so the fallback is the camera's own stored
   properties, which are the ones it had when the link was made — plus, now
   that corrections exist (§5f), whatever has been nudged since, read as a pose
   rather than as a correction. The two cases are different `LinkState`s, so
   the interface can say which.
3. **The tracked layer inside a precomp is found by the effect on it.** The design says
   the chain resolves through a Precomp layer to the tracked layer inside but does
   not say how the inside one is identified; the effect *is* the handle, so the
   layer carrying an enabled Camera track is the answer, first in stack order so
   it never depends on the playhead. A precomp with no tracked layer resolves to
   nothing and says `Unresolved` rather than guessing at the first footage it finds.
   *Amended later: a precomp layer that wears the effect **itself** stops the
   walk there and is tracked as a nested comp; see §5e.*
4. **The bake is a `Batch`, built by a function, not an `Op` that computes.** An
   `Op` is serialisable and replayed by `apply(doc, op)`, which has no store to
   read — so a computing op would either need the store in `apply` or would replay
   differently. `bake_solve_link` reads the store once, at the gesture, and emits
   the numbers; the journal then holds exactly what happened. The link is cleared
   as the batch's **first** member and therefore restored as the inverse's **last**,
   which is what lets the read-only guard stay unconditional in both directions.
5. **`camera_pose` keeps its signature and gains a sibling.** Fifteen call sites
   read `Composition::camera_pose(t)`; threading a store through all of them before
   there is a store to thread is churn for nothing. `camera_pose` now answers what
   the document holds, `track::camera_pose_at` answers with the link followed, and
   a linked camera under the old call reads `Unresolved` — the correct degrade.
   Stage 2 moves the render path over.
6. **The Action row does not cross the bridge yet.** `list_parameters` filters it
   out rather than inventing a `BridgeParamKind` the panel cannot draw and an event
   nothing sends; both are stage 3, and the frb generated code is regenerated, never
   hand-edited. Until then the panel draws the effect's value rows and no button,
   which is honest. *Stage 3 removed the filter and added both — see §5c.*

**Stage 1's tests** are eight in `crates/lumit-core/src/track.rs` and two in
`fx/tests.rs`, and not one of them decodes anything. The solve is a function of
the frame number, distinct in every field, so a walk that lands on the wrong frame
cannot accidentally pass: a plain clip frame for frame; a half-speed ramp into a
freeze (the freeze reads `Derived`, not `Held` — the solve *has* that frame, the
retime simply keeps asking for it); a Sequence layer whose two clips are played in
both orders, with the same source moment giving the same pose either way; a
precomp chain with a retime on the precomp layer; the hold past the end, the
deleted layer, the empty store and the unlinked camera, each with its own
`LinkState`; the two refusals and the one edit a linked camera still accepts; and
the bake, checked frame by frame over a span that is half derived and half held,
against the derived path it replaces, with undo restoring the layer exactly.

**Owed to stage 2** (and listed in docs/TODO.md): the real `CameraSolveStore` over
the `track/` sidecar, the analysis job and its thread, the status as live job
state, and threading `camera_pose_at` into the render path and the frame key so a
linked camera's frames are named by the pose they were drawn with. *All of that
landed — see §5b.*

## 5b. Phase 4, stage 2 — the job, the sidecar and the real store, as built

`crates/lumit-render/src/track.rs` (2026-08-21), with the cancellation seam in
`crates/lumit-track/src/{solve,bundle}.rs` and the luma tap in
`crates/lumit-media/src/decode.rs`. Stage 2 is everything stage 1 stood on a
written-down solve for: the analysis that decodes a real clip, the `track/`
sidecar that keeps the answer, and the `CameraSolveStore` the render path
actually reads.

**Where it lives, and why.** `lumit-render` is the only engine crate that can
both decode a clip and hold a solve where `build.rs`, `headless.rs` and the frame
key can read it; `lumit-track` cannot decode, `lumit-core` may not know the
tracker exists, and the bridge is above all of them. So the job, the cache and
the store are one module there, and `lumit-render` gains a `lumit-track`
dependency. The bridge (stage 3) presses the button; nothing about the work
itself is in the bridge.

**The thread.** One analysis at a time, on a thread spawned for it and named
`lumit-track` — never a pool worker (docs/05 §2's decode rule holds for the
same reason: it holds a decoder open and stalls on seeks). `request` claims the
one running slot and returns immediately; the *cache probe happens on that thread
too*, so no caller — least of all the interface — ever waits on the disk. A
second request while one is in flight answers `Busy` rather than queueing: this is
a minutes-long, disk-bound job, and two of them share one drive and halve each
other. `cancel(media)` raises an `AtomicBool` the frame loop reads between frames
and the solve reads per pass and per LM iteration; a cancelled run writes nothing
and publishes nothing, so there is no partial state to clean up. Progress is a
value in a map (`Queued` → `Tracking { done, total }` → `Solving` → `Done` /
`Cancelled` / `Failed`), sampled by whoever repaints, not a subscription anybody
has to hold.

**The store.** A process-wide `RwLock<HashMap<media, Arc<Solved>>>`, read once per
frame by the render path with the guard dropped inside the accessor — nothing is
held across a decode, a submit or an FFI call (docs/14 §1.3). The poses are
converted **once**, when a solve lands, into a `Vec<CameraPose>` indexed by frame,
so the per-frame read is a slice index and no trigonometry runs in the render path
at all.

Eight things are deviations from, or decisions under, the design's wording:

1. **The conversion is derived, not chosen, and tested against the real matrix.**
   The tracker puts a world point at `centre + f · p.xy / p.z` with
   `p = R(P − C)`; `lumit_gpu::composite::camera_matrix` puts it at
   `centre + zoom · a.xy / (a.z + zoom)` with `a = Rot⁻¹(P − position)` and
   `Rot = Ry·Rx·Rz`. Setting those equal for every `P` leaves no freedom:
   `zoom = f`, `Rot = Rᵀ`, and `position = C + Rᵀ·(0, 0, f)` — the camera centre
   pushed forward along its own optical axis by the focal length, because Lumit's
   perspective matrix has already put the eye `zoom` behind `position`. The Euler
   angles come out of `Rᵀ` in the compositor's own `Ry·Rx·Rz` order. The test does
   not re-derive any of this: it calls `camera_matrix` and asserts that every
   solved point lands within a twentieth of a pixel of where the tracker put it,
   over every frame — an algebraic identity, so it cannot flake on solve quality
   and cannot pass on a solve that failed.
2. **The solve's world is read as comp pixels at the footage's own raster size.**
   `CameraSolveStore` is asked about a *media item*, not about a comp — one clip is
   in many comps — so there is nowhere else for a reference frame to come from.
   Exact for the ordinary case (a comp made from the shot), off by the size ratio
   otherwise. Recorded rather than hidden; a comp-aware scale is a change to the
   trait, not to this arithmetic.
3. **The sidecar is global, like `media-index/`, not per project.** The design says
   "the project's sidecar (`track/`)", and docs/10 §3 calls the whole cache root
   the sidecar — of whose two existing tiers one is already global for precisely
   this reason. The key is (media fingerprint, settings, mask geometry): the solve
   describes the *file*, so two projects on the same rushes share one and a copy of
   a project finds its solves already there. Format in docs/10 §3.
4. **The stored record is the `CameraSolve` and the media's frame rate, not the
   `TrackSet`.** The store needs the poses and the range; the overlay and the
   Null/Solid gesture need the point cloud; all three are in the solve. The 2D
   track export will want the tracks themselves, and when it does the honest move
   is a format-version bump and a re-analysis, not carrying megabytes of trajectory
   that nothing reads today.
5. **The luma tap is one swscale call to gray8, for every pixel format.**
   `VideoDecoder::frame_luma` shares the whole seek-and-decode path with
   `frame_rgba` — the split is `frame_exact` — and then converts to `GRAY8` at the
   source's own size rather than to RGBA. For the planar YUV video actually arrives
   in, swscale's gray8 output *is* the Y plane, so a fast path for it would buy
   nothing but a second thing to keep correct; and asking for RGBA to discard two
   thirds of it would cost a full colour conversion and four times the bytes on
   every frame. Deliberately unscaled: the tracker measures sub-pixel motion in
   source pixels, and a preview-tier downsample would silently change what a solved
   focal means. The test compares the tap with the RGBA decode **by correlation**,
   because the two conversions weight R, G and B differently and legitimately
   disagree by tens of per cent on a colour bar — correlation is blind to the gain
   and lift the tracker is blind to, and not blind to a wrong plane, a wrong frame,
   a wrong raster or upside-down rows.
6. **The frame source is a trait (`LumaFrames`) with two implementations.** The
   real one wraps `VideoDecoder` and is opened on the analysis thread; the test one
   renders a synthetic shot. That is the seam that lets the whole job — progress,
   cancellation, the cache, the store, the link — be tested with no asset, no
   encoder and no ffmpeg, in the shape `SourceProbes` already uses in this crate.
7. **The mask factor is one, and that is a statement.** A mask's vertices are in
   the layer's own pixel coordinates, and for a footage layer those *are* the source
   raster — `build.rs` rasterises them at the layer's natural size, which is the
   file's own size whatever the preview tier decodes at. The tracker works in the
   same pixels, so nothing converts. The mask is flattened at layer time
   zero: a tracker takes one fixed set of regions for a whole run, so a mask
   keyframed to follow a mover cannot be honoured yet (owed, in TODO).
   *Superseded — the regions are now re-flattened per frame; see §5e.*
8. **The frame key asks the stamper for the camera.** `lumit_eval::SourceStamper`
   gained a defaulted `camera(doc, comp, t)` answering `comp.camera_pose(t)`, and
   `lumit-render`'s `Stamper` overrides it to follow the link. Without it a frame
   drawn through a derived pose would be *named* by the transform the document
   holds — which is not the transform it was drawn with — and the frames banked
   before a solve landed would be served back after it. One defaulted method rather
   than a store threaded through `comp_frame_key`'s callers, and `lumit-eval` still
   does not know what a camera solve is.

**The cancellation seam phase 3 owed.** `solve_camera_cancellable` takes a
`&dyn Fn() -> bool` asked between passes, once per Levenberg–Marquardt iteration,
and after each bundle; `solve_camera` is it with a flag that is never raised, so
every existing caller and every existing test is untouched. A raised flag returns
`SolveError::Cancelled` and **nothing partial**: the half-adjusted model is thrown
away rather than filled out into frames and handed back looking finished. Its test
pins all three claims — refused before the first pass, refused part-way through
the bundle with the flag provably consulted, and bit-identical to `solve_camera`
when never raised.

**Stage 2's tests** are six in `lumit-render/src/track.rs`, one in
`lumit-track/src/tests.rs` and one in `lumit-media/src/decode.rs`. The six drive a
synthetic shot: two textured planes at different depths, the near one present in a
coarse checker of patches so both are visible at once and the occlusion is exact,
ray-cast per pixel under a camera path written down in the test. A single plane
would not do — however it is moved it is explained by a homography, and the solve
refuses it as rotation-only, correctly. In order: the whole job end to end
(progress readings, the recovered focal against the true 320 px, the conversion
against `camera_matrix`, the solve into the store, and a linked Camera layer
reading it frame for frame and then holding past the end); a solve landing renaming
the frames it changes; a cancel five frames in leaving no sidecar entry, no
published solve, and a clean re-run; the sidecar's whole contract (round trip,
rebuild-equals-hit byte for byte, a different key refused, a newer version refused,
a deleted file rebuilt to the identical bytes); a masked region with no track
living in it while the rest of the frame still tracks; and the worker thread
accepting one analysis, refusing a second, filing the answer, and a warm pass
finding it without opening the media at all.

**Mutation-checked.** Reading the focal offset off the wrong row of `R`, or
swapping the y and z Euler extractions, moves the compositor-versus-tracker
disagreement from 0.02 px to 930 px and 27 px respectively — the conversion test is
the one that has to bite, and it does.

**Owed to stage 3** (and listed in docs/TODO.md): the Action press event and the
`BridgeParamKind`; `Progress` and `LinkState` surfaced as the effect's status row
and the camera's badge; the point-cloud overlay; the warm pass wired to project
open and `clear()` to project close; a Camera track on a **Precomp** layer (allowed
by design, and the analysis decodes media, so tracking a nested comp means
rendering it first); keyframed masks as time-varying exclusion regions; and more
than one analysis at a time if anyone ever wants it. *All but the last landed —
see §5e.*

## 5c. Phase 4, stage 3 — the surface, as built

`crates/lumit-bridge/src/api/track.rs`, `flutter_ui/lib/panels/{viewer_track,
camera_track_display_frb}.dart` (2026-08-21). Stage 3 is everything the user
touches: the buttons, the line that says how the analysis is getting on, the dots
over the picture, and the badge a derived camera wears.

**The bridge is a doorway, not a worker.** Nine functions in one module, all
`#[frb(sync)]` and none of them doing anything a caller waits on. Down:
`fire_effect_action`, `add_solved_camera`, `set_camera_solve_link`,
`convert_camera_to_keyframes`, `add_layer_at_points`. Up:
`track_status`, `tracked_points`, `camera_link`. The whole surface, with its
reasoning, is docs/17's "The camera track: an event down, readings up".

Seven things are deviations from, or decisions under, the design's wording:

1. **An Action press is not an edit, and so has nothing to poll against.** Every
   other reading in the interface is refreshed by a document revision moving; a
   press moves nothing, deliberately. So the panel counts its own presses and the
   status line watches that number — one `int` down the tree, rather than an event
   stream for a thing that happens twice a session.
2. **The status is polled, and only while it is moving.** Twice a second, from the
   moment a press starts something to the moment the reading stops changing, and
   never otherwise. The engine already keeps progress as a value in a map
   precisely so nobody has to hold a subscription (§5b), and a stream would have
   been a second mechanism for the same fact.
3. **The failure reason crosses as an enum with no text in it.** `AnalysisError`
   carries English (`thiserror`'s messages) and English crossing the seam ships
   untranslated inside a translated window. `BridgeTrackFailure` is six variants;
   Dart's switch over the generated enum picks the arb key, so a reason added to
   the engine is a Dart compile error rather than a blank line. That is the
   translation chain one step stricter than the import report's, which sends an
   id *and* its English as a fallback — this one has no free text to fall back to.
4. **The cloud is drawn always and clickable only when its layer is selected.**
   Show points says whether the dots are there; a cloud that also took every click
   would make the shot unselectable, and clicking the picture is how a layer is
   selected. Recorded in docs/07 §2.3.6 as the rule, because it is the one
   thing about the overlay that is not obvious from looking at it.
5. **The creation affordance is a floating row, not a context menu.** The gesture
   that makes the selection is a drag on the picture; asking for a second, hidden
   gesture to act on it would look calmer and be slower. Two buttons, under the
   picked points, clamped onto the panel.
6. **"Create camera" had to be invented.** The design describes what a solve-linked
   camera *does* but not how one comes to exist, and without a gesture the link,
   the badge and Convert to keyframes are all unreachable. It sits beside the
   status line, where the solve it links to is being reported, and is one op:
   a Camera layer whose `solve_link` is the tracked layer. `set_camera_solve_link`
   is the primitive under it, so a picker on an existing camera is a panel away
   rather than an engine change.
7. **The overlay's placement inherits §5b's reference frame.** Points come back in
   composition pixels as the footage's raster centred on the comp — the layer's own
   transform is not applied. Exact for the ordinary case and wrong by that
   transform otherwise; recorded in docs/TODO.md rather than hidden, and the fix is
   a change to `CameraSolveStore`, not arithmetic in the overlay.

**Where the claims are asserted, and why the split.** Four tests in
`crates/lumit-bridge/src/api/tests.rs` drive a **written-down** solve published
straight into the store (`lumit_render::track::publish`, public for exactly this):
the cloud landing in composition pixels with its depth cue, on the frame the
playhead is on and on a *later* frame; a Null landing at the mean solved position
in 3D, with the refusal when nothing named was solved and undo taking it back; the
badge reading `Derived` and Convert to keyframes writing fifty keys, ending the
link, and leaving a camera that takes an edit again; and the status reading a
solve's numbers, with each button's refusal. Six in
`flutter_ui/test/frb/camera_track_frb_test.dart` drive the interface: the Action
row drawing a button and a press of Cancel reaching the engine and changing the
line (which is the wiring proved end to end, since the engine files a `Cancelled`
reading a press with no wiring could not produce); every failure reason having
words; a dot per point with its depth drawn rather than recomputed; click,
shift-click, marquee and `Escape`; the cloud asked for **once per frame and not
once per rebuild**, counted directly; and a linked camera's badge with Convert
asserted through the read model rather than through the widget.

The split is the honest one. A solve cannot be put into the engine's store from
Dart — it is the answer to a minutes-long analysis of a real media file — so the
*arithmetic* claims live in Rust where a solve can be written down, and the
*interface* claims live in Dart with the cloud handed in through one optional
callback (`ViewerTrackLayer.fetch`, defaulting to the engine's own answer). That
is the same seam, one level up, that `LumaFrames` is in §5b.

**Owed** (docs/TODO.md): the 2D track export, a Tracking workspace, a picker that
links an existing Camera layer, the layer-transform-aware cloud placement, and the
cloud's own affordances — a count, a filter, deleting a point, hiding points behind
the shot, and setting the ground plane and origin from a selection.

## 5d. A partial track, as built

`crates/lumit-render/src/track.rs`, with the two things it needed from
`crates/lumit-track/src/lib.rs` (2026-08-24). Until this, a shot that
stopped being followable part-way was analysed to its end regardless: the run
carried on decoding frames nothing crossed, the solve placed cameras on them
anyway, and the interface reported the result as a whole answer. This is the
half that makes a partial track honest.

**The signal is the carried count, and it is the solver's own minimum.**
`Tracker::carried_count()` is the number of tracks that survived the step *into*
the frame just pushed, read between the carry and the re-detection. It is not
`live_count`, and the difference is the whole point: re-detection refills
whatever buckets emptied, and the detector's quality floor is relative to each
frame's own best, so the live count recovers within one frame however completely
a shot fails. Live count says how many specks are being followed; carried count
says how many of them tie this frame to the last one, which is the only thing
any later phase can use. The job stops when it falls below **eight** — the
smallest correspondence set that can be *verified* rather than merely fitted,
the minimal sample for the 7-point fundamental being seven. That is a statement
about the arithmetic rather than a threshold anyone tuned: below it the chain of
correspondence is severed at that frame, and nothing after it can be related to
anything before it however well it tracks among itself.

**It finalises rather than discards.** The frames up to the boundary are a real
shot with a real camera move in them, so the tracks are cut to the last frame
that carried (`TrackSet::truncate`) and the whole of phase 2 and phase 3 runs
over that span exactly as it would over a whole clip. Cutting rather than
leaving the tail in matters: keyframe selection returns a short final pair rather
than leaving a gap (§3's deviation 8), so a set that runs on past the failure
would have the solve stand on a pair nothing spans. A track cut short is left
`Ended` unless it was already `Moving`, which is a verdict about the track rather
than about its extent — overwriting it would put a mover back into a later
solve.

**What says it is partial is the clip's length, not a flag on the solve.** After
the truncation the `CameraSolve` is a complete answer *about its span* — there is
nothing partial inside it. The partiality is the relation between that span and
the clip, so `Solved` (and the sidecar record, at format version 2) carries the
clip's own frame count and `Solved::is_partial()` is one comparison. The span is
always a prefix, because the job follows the source from its first frame and can
only ever stop early, so the pair of numbers is also the whole of what the
interface draws.

Four things are worth recording as decisions under that:

1. **The two ways a run ends early are one case downstream.** Frames that stop
   decoding (`LumaFrames::luma` answering `None`, which already existed) and
   tracking that fails are reported identically: the solve covers what was
   followed and the clip says how much more there was. They mean the same thing
   to the store, to the link and to the panel, and giving them separate
   readings would have been two spellings of "this is as far as it got".
2. **A partial solve is `Done`, not a failure.** `Progress` gained no variant.
   A refusal is a shot with *no* answer in it; this one has an answer, and the
   status row's job is to say how far it reaches, not to colour it as a fault.
   The failure reasons crossing to Dart (§5c's third deviation) are unchanged.
3. **It is cached like any other solve.** The sidecar keeps a partial answer
   under the same key, because it *is* the honest answer for that file at those
   settings, and re-deriving it would take the same minutes to stop in the same
   place. The format version went to 2 so a version 1 record — which could not
   say how long its clip was — is simply never asked for.
4. **The hold needed no new mechanism, and that was worth checking.** The
   hold is a clamp into the store's solved range (§5a's second deviation), and
   the range now ends where the track does, so a camera linked to a partial
   solve derives inside the span and holds the last derived pose outside it with
   no code changed at all. It is asserted twice — in the engine over a real
   analysis, and across the bridge over a written-down one — precisely because
   "it already works" is the kind of claim that stops being true silently.

**The surface.** `BridgeTrackStatus` gained `clip_frames` beside `frames`;
`frames < clip_frames` is the partial reading (docs/17). The panel draws a thin
bar above the status line — the analysed span in the accent, the rest of the clip
in a surface tone, two weights in one row and no painter — and the line says
"Analysed *n* of *m* frames — the shot could not be followed further" in place of
the point count and error, because how far it reaches is the fact that decides
what the user does next. docs/07 §6 records why that bar does not contradict the
one-progress-bar rule: it appears only once the work is over, it does not move,
and it measures the answer's extent rather than the work's completeness.

**Stage 5d's tests** are two in `lumit-track` (the carried count falling to zero
on a frame nothing crossed while the live count recovers one frame later; a
truncated set keeping its step-per-gap invariant, its ended states and nothing
past the cut), one in `lumit-render` (a synthetic shot that runs into featureless
frames: the run stops where the carrying stops, the solve covers exactly the span
that worked and still recovers the focal, the store reads partial, and the linked
camera derives at the last solved frame and holds through the tail), one in
`lumit-bridge` (the span and the clip crossing, and the badge reading Derived
inside and Held outside), and one in Flutter (the partial sentence leading with
its span rather than its point count, and the bar's two weights being the two
frame counts). The sidecar's round-trip test now also pins the clip length, so a
cached partial solve cannot read back as a whole one.

**Why the tail is featureless rather than merely dim** in that fixture: the
verification a track ends on is normalised correlation, which is blind to gain
and lift by construction, so a picture that fades down in contrast is followed
happily and *should* be. Only a frame with no structure at all severs the chain —
the gradient normal matrix is singular and every KLT solve refuses — which is
both the deterministic thing to write and a real thing footage does.

## 5e. The finishing items, as built

The three things stages 2 and 3 left owed, and the small debts beside them.

**Warm and clear are wired to a project's life** (2026-08-25).
`lumit_render::track::warm_jobs(doc)` reads one warm job off the document for
every footage item a layer wears an enabled Camera track on — one per *media*,
not per layer, skipping anything offline (no resolved path, no fingerprint,
nothing to name a solve with). `warm(jobs)` reads all of them back off the
sidecar. `api::state::adopt` collects the jobs beside the probe warm — after
`resolve_all_media`, which is what stamps the fingerprints, and before the
document moves into the store — and fires them after the registry lock, with
`clear()` immediately in front so the departing project's solves go before this
one's arrive. `ProjectReference::close` calls `forget` with the ids `owned_ids` reads
off its document, so another project's solves stay put, and touches no file.

Two things about it are choices rather than transcription:

1. **The warm pass is not [`request`].** `request` owns the
   one-analysis-at-a-time slot, so warming the second tracked clip of a project
   would answer `Busy` and simply never happen — the wiring would look done and
   work for exactly one clip. `warm` takes no slot: it is a small file read per
   clip, the whole batch on one thread of its own, and it cannot collide with an
   analysis the user starts while it is going. It forces `analyse` off on the way
   past, so nothing handed to it can start tracking a clip nobody asked about.
2. **The test writes the sidecar rather than earning one.** A solve is written
   down by hand — a camera sliding along x, every frame distinct — filed under
   the key the warm pass will ask for, and the assertion is that the linked
   camera reads `Derived` with nobody pressing Analyse. Earning the file would
   have re-run a whole synthetic analysis to test a disk read, and the key
   equality is asserted directly, which is the part a warm pass can silently get
   wrong: asking for a *different* analysis finds nothing and looks exactly like
   having no cache at all.

**Masks follow their keyframes** (2026-08-25). `MaskTrack` replaces the flattened
`Vec<ExclusionMask>` a job carried: it keeps the layer's masks *as masks*, and
answers `at(t)` with the regions as they stand at layer time `t`. The frame loop
asks it once per frame and hands the answer to `Tracker::set_masks` before the
push, so where a feature may be born and whether a carried track has strayed are
both judged against the shape that frame actually has.

Four things worth stating:

1. **Per frame, not per span.** A path flatten is a few hundred line segments off
   a handful of cubics — microseconds — against the pyramid build and several
   hundred KLT solves the same frame costs, which are milliseconds. A span table
   would be a second thing to keep honest about where the shape is, to save a
   cost that does not show. A **still** mask is flattened once for the whole run
   (`MaskTrack::animated` is false), so the ordinary case pays nothing at all.
2. **The clock is the source's own.** Source frame `n` is read at layer time
   `n / fps` — exact for the ordinary case, and the generalisation of the old
   flatten-at-zero, which was its `n = 0` instance. A retime between layer time
   and source time is not inverted: the analysis is of the *file*, from its first
   frame at its own rate, and one clip lives in many layers with many retimes,
   only one of which could ever be honoured. The factor from comp to source
   pixels is still one, for §5b's seventh deviation's reason.
3. **The key is honest about the animation.** A still mask hashes exactly the
   bytes it always did, so every solve already in the sidecar keeps the name it
   was filed under; a keyed path then **appends** its own keys — each moment, its
   shape, and the eases either side, which are what decide every shape in between.
   Hashing only the shape at zero would have handed a re-keyed mask its own stale
   solve back as though nothing had changed. No `FORMAT_VERSION` bump: the
   animated keys simply name a different file, and nothing already written is
   orphaned.
4. **The test's second claim is the one that bites.** That nothing is tracked
   inside the moving region is easy to satisfy by tracking nothing near it; the
   assertion beside it is that *plenty* is tracked where the region began, on
   frames it has since left — which a flatten-at-zero run could not produce. The
   old behaviour fails the first assertion too, for the mirror-image reason: the
   features it allowed in the mask's destination are still there when the mask
   arrives.

**A Camera track on a Precomp layer analyses** (2026-08-25). `LumaFrames`
gains a second real implementation: `CompLuma` renders the nested comp through
`HeadlessRenderer` — its own device, built inside `Job::open` on the analysis
thread, the same walk an export takes — instead of decoding a file. Everything
above it is unchanged, which is the point: the frame loop, the progress
readings, the mask exclusion and the cancellation seam are one loop whatever is
feeding it.

Four things, all of them decisions under that:

1. **The walk stops at the precomp layer** when that layer wears the effect, and
   descends into the comp when it does not. `lumit_core::track::wears_camera_track`
   is the one predicate, and `tracked_source_id` the one place that says which
   uuid a tracked layer's source is — a footage item, or the nested comp.
2. **`analysis_scale` on the trait.** `MediaLuma` answers one and says nothing;
   `CompLuma` answers its render scale, and `rescale` multiplies the finished
   solve back into comp pixels. Defaulted on the trait so the file path and the
   test shot are untouched.
3. **The masks are the one case where the factor is not one.** A precomp layer's
   mask vertices are in the nested comp's raster, and the analysis reads a
   reduced one, so `MaskTrack` carries `to_analysis` and the precomp job passes
   the render scale.
4. **No sidecar entry, and `Job::key` is an `Option` to say so.** A `None` key is
   neither read nor written; a precomp solve lives in the store for the session.

**The test's fixture is noise, not rectangles, and that is worth knowing.** The
first attempt panned a field of solid squares: sixty corners were detected on the
first frame and not one survived the step to the second. The tracker's step is a
linearisation of the picture around each patch, and a hard-edged rectangle offers
a one-pixel cliff and nothing else — there is no gradient over the window for the
solve to descend. Fractal noise on one oversized solid, panned rigidly, gives
texture with width, and the same run follows features the whole way. It is the
same reason §5's synthetic shot is procedurally textured, met from the other
direction.

## 5f. Track once, then nudge — the correction lane, as built

`crates/lumit-core/src/{model,ops,track}.rs`, with the read in
`crates/lumit-bridge/src/api/{layer,track}.rs` and the two rows in
`flutter_ui/lib/panels/{camera_track_display_frb,effect_controls_panel_frb}.dart`
(2026-08-25). Until this, a solve-linked camera's transform rows were
read-only badges: the engine refused `SetTransformProperty` and `SetCameraZoom`
with `OpError::CameraLinked`, and the only way past a solve that was slightly
wrong was to bake it and lose the link. This is the half that lets a measurement
be adjusted without being replaced.

**The arithmetic is `derived = solved + (stored − base)`, per channel.** Seven
numbers, seven independent additions, and no matrix. `base` is a new
`correction_base: Option<Box<CameraPose>>` on `LayerKind::Camera`, captured by
`Op::SetCameraSolveLink` when a link is made on a camera that had none, kept when
a link is re-pointed, dropped when it is cleared. `lumit_core::track::correct` is
the whole composition; `has_correction` is the dot's reading;
`clear_corrections` is the batch that puts the seven properties back.

Five things are decisions rather than transcription:

1. **A separate base, rather than reinterpreting the rows.** The obvious saving
   is to say the rows *are* the correction and let nought be nought — no new
   field. It cannot be done: the same numbers are already the
   [`LinkState::Unresolved`] fallback, the pose a camera falls back to when its
   media goes offline, and `add_solved_camera` puts that at the comp's centre
   with a 50 mm zoom. Read as a correction, a camera created at the centre of a
   1920-wide comp would be nudged 960 px sideways the moment it resolved. One
   set of numbers cannot be a pose and an offset at once; the base is what makes
   them both.
2. **Channel-wise addition, not a composed transform.** The parent-child
   alternative — the solve as a parent, the correction as its child — is what a
   rig would do, and it was rejected for two reasons: a row stops meaning what
   it says (a "position x" nudge would run along the shot's axis and swing with
   every pan), and the curve the graph editor draws stops being the curve that
   was dragged. Addition also commutes with itself, so two corrections in either
   order are the same camera.
3. **The base is computed inside `apply`.** §5a's fourth deviation argued that a
   computing op is a trap, and it is — for an op that would need the *store*, or
   whose answer depends on anything outside the document. This one reads seven
   properties off the layer it is about to edit, so it replays identically and
   its inverse re-derives the same numbers from a document the earlier inverses
   have already put back. Putting it in `apply` rather than in the bridge means
   every caller gets it, including the bake and every op yet to be written.
4. **Zoom is corrected too.** It could have been left refused — the focal is
   what was solved, and moving it detaches the point cloud from the picture. It
   is not, because a solved focal is exactly as capable of being a little wrong
   as a solved position, and because leaving one row of seven refused would be a
   rule with no shape. The cloud is drawn from the *solve*, so a corrected zoom
   makes the dots and the picture disagree; that is a visible, reversible
   consequence of an edit the user made, which is different from a silent one.
5. **The dot is in the read model, not a call.** `BridgeLayerInfo.track_corrected`
   answers for a camera ("this one is nudged") and for a tracked layer ("a camera
   following me is"), which is one fact from where the user stands. Both rows
   that draw it repaint on every document revision and a correction *is* a
   document revision, so a call there is the cost a read model exists to remove.
   The tracked-layer half scans the comp's cameras, and only for a layer wearing
   a Camera track — one layer in a comp at most times, none in most comps.

**The surface.** The Transform heading's badge gains the dot and **Clear
corrections**, offered only when there is something to clear; the Camera track's
status row gains the same dot ahead of its sentence. The badge's own sentence
became `Flexible` with an ellipsis and the badge itself is `Expanded` at its call
site, because a heading's action row lays its children out unbounded and a badge
carrying a sentence has to be told how much room it has before it can clip it —
without that the third word pushed the commands off the panel's edge.

**Stage 5f's tests** are four in `lumit-core` (the composition read at derived,
held and unresolved frames with every channel checked; a *keyed* correction added
frame by frame, which a static-only implementation passes and a per-frame one has
to earn; Clear keeping the link and undoing in one step; and the base captured,
kept through a re-point, dropped on unlink and restored by undo), one in
`lumit-render` (a correction moving the frame key, and the same nudge sitting on
top of a *second*, different solve — the claim that corrections survive
re-analysing, which is the whole point of not folding them into the camera), one
in `lumit-bridge` (both rows' dots, the corrected pose read back through the bake
at 5 + 30, Clear refused on an untouched camera and accepted on a nudged one), and
one in Flutter (the dot and the command appearing and going out, and one undo
bringing both back).

**Owed.** The Viewer's point cloud is still drawn from the solve alone, so a
corrected camera's dots no longer sit on the features they were found on. That is
arguably correct — the cloud is what was *measured* — but it is not said
anywhere, and a line saying so, or a cloud drawn through the correction, is the
next honest step.

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
  called one (since the 2026-08-26 redesign it raises no boundary at all); the
  train-POV trio — forward travel alone raising nothing, travel keeping its
  scope-in as a cut with the travel subtracted out of its `log_scale`, and a
  mid-travel rack spanning its own pairs at the lens's own rate; the whole
  pipeline run twice and compared with `assert_eq!`.
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
  solve run twice and compared with `assert_eq!`; the bundle driven
  directly, from a start knocked off by a fifth of a unit in position, three
  quarters of a degree in rotation, a twentieth of a unit per point and four per
  cent in focal, required to come back to a millionth of a pixel; the same
  drive over a lens riding a straight 300→420 px line — exactly what two knots
  describe, so the true minimum is exactly zero — with both knots knocked off
  by different amounts and required back to a ten-thousandth (measured floor
  ≈1e-5 px of focal: two knots are more correlated than one, since every camera
  between them reads a blend); and the forward-travel-then-rack shot solved end
  to end against the true per-frame focal curve.
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

## 6. The planar tracker, as built

`crates/lumit-track/src/planar.rs`, with the render-side job in
`crates/lumit-render/src/track.rs`, the model half in `crates/lumit-core/src/track.rs`
and the surface in `crates/lumit-bridge/src/api/track.rs` and
`flutter_ui/lib/panels/planar_track_display_frb.dart` (2026-08-25). Phases 1–4
answer *where the camera was*. This answers *where one flat thing is*, which is a smaller
question with a much shorter route to it — and the route reuses every line of phase 1.

### In plain terms

Somebody in the shot is holding a phone and you want your own picture on its screen. The
phone is **flat**, and a flat thing filmed by a camera has a very convenient property:
however the camera moves and however the phone turns, what it does to the *picture* of that
surface is always the same kind of warp — a homography, the four-corner projective stretch
a Corner pin already applies. Eight numbers describe it completely.

So the job is never "where did the phone go". It is "which eight numbers, this frame".

### The recipe

1. The user puts the effect's four points round the flat thing on the clip's **first
   frame** — the *reference frame*, which everything else is measured against.
2. The ordinary [`Tracker`] follows specks, confined to the quad. Confining it needs no new
   mechanism: an **inverted** `ExclusionMask` already means "the tracker works only within
   this shape" (§2), so the quad is one region prepended to the layer's own masks.
   Those still apply *inside* it, which is how the hand crossing the phone is excluded.
3. For each later frame, take `TrackSet::correspondences(anchor, frame)` — every speck
   present on both — and fit a homography robustly: `homography_ransac`, which is the
   LO-RANSAC of §3 over four-point DLT samples with symmetric transfer distance, under the
   same frame-sized conditioning and with the same seed stream `estimate_pair` gives its
   homography.
4. Push the four corners through the composed reference→frame homography. Those are the
   answer.

The surface is `solve_planar(&TrackSet, reference_frame, quad, source_size,
&PlanarSettings) -> Result<PlanarTrack, PlanarError>`, with
`solve_planar_cancellable` taking a `&dyn Fn() -> bool` asked **between frames** — the
crate's own cancellation shape, as `solve_camera_cancellable` is (§5b). `PlanarTrack` is
`reference_frame`, `reference_quad`, one `PlanarFrame` per followed frame (its corners,
its inlier count, whether it was re-anchored) and the run's `reanchors` count.
`PlanarError` is a refusal, never a fault: `TooFewFeatures`, `NotPlanar`, `Cancelled`.

### Drift: re-anchored, not chained

The obvious implementation warps frame 1 onto frame 2, frame 2 onto frame 3, and multiplies
as it goes. It is also the one that **drifts**: every step's small error is multiplied into
every step after it, and by frame three hundred the quad has walked off the phone. Measuring
each frame against the *reference* frame instead makes every frame's error independent —
frame three hundred is no worse than frame two, because nothing about frame two went into
it.

That is what shipped, and it is what the tests pin: a clean ten-frame slide comes back with
`reanchors == 0` and every `PlanarFrame::reanchored` false, which a chained implementation
could not produce.

The price is real and is paid for explicitly. The reference frame's specks die out — they
leave the picture, the surface turns away, someone walks in front — so the
reference-anchored correspondence set thins. When it falls below `reanchor_below` (12
inliers), the run **re-anchors**: it adopts the *previous* frame as the anchor, keeps the
composed reference→previous homography it has already paid for, and refits. Error then
accumulates once per re-anchor rather than once per frame, which over a long shot is the
difference between a handful of small errors and thousands. The count crosses the bridge
and the panel says it, because it is the one number that says how much to trust the far
end of a track.

Six things are decisions under that, rather than transcription:

1. **A re-anchor is tried once per frame and only if it is better.** The retry must clear
   `min_inliers` *and* beat what the current anchor managed; otherwise the old anchor is
   kept. A fresh anchor that cannot explain the very next frame either means the surface has
   gone, and a third attempt would be looking for it in the same place.
2. **`min_inliers` is six, and it is arithmetic rather than taste.** Four correspondences
   are the homography's minimal sample, so a fit through exactly four has nothing left over
   to disagree with it; six is the smallest set that is *verified*, with two observations
   spare. `reanchor_below` sits above it (12) deliberately: re-anchoring while the fit is
   still sound costs one composition's error, and doing it after the fit has gone soft costs
   the frames in between as well.
3. **A run that stops being followable ends there**, with the span that worked returned as a
   whole answer about it — §5d's shape, met again. The frames after such a boundary are not
   a poorer answer, they are no answer. What says the track is partial is the relation
   between its span and the clip's length, which is `PlanarSolved::is_partial`, one
   comparison, exactly as `Solved::is_partial` is.
4. **`quad_outline` reorders the corners, and it has to.** [`Quad`] is in Corner pin's
   declaration order — upper left, upper right, lower left, lower right — and walking a
   polygon in that order draws a **bow tie**: a self-crossing outline whose even-odd test
   excludes the middle of the quad and includes two triangles outside it. One reordering, in
   one place, rather than a second convention every caller has to remember.
5. **`warp_quad` refuses a corner that maps to infinity** rather than warping the other
   three. A quad with one corner on the projection's horizon is not a quad, and half a
   warped one would be a silent lie about where the surface is.
6. **The two refusals are told apart by whether there were correspondences at all.** No
   correspondences over the first pair is a starved patch (`TooFewFeatures`);
   correspondences that never agreed on a warp is a surface that is not one (`NotPlanar`).
   Both are refusals; only the second means "you drew the quad round something that moves
   against itself".

### The job, the store and the sidecar

One branch, taken **after** the frames have been followed. `Job` gained a `JobKind` —
`Camera`, or `Planar { quad }` — and everything above that branch is untouched: the same
decode, the same detector, the same per-frame mask flatten, the same
`MIN_CARRIED` severance check, the same progress readings, the same cancellation flag.
`analyse` returns an `Answer` (`Camera(CameraSolve)` or `Planar(PlanarTrack)`), which is
also the sidecar's body.

Four things worth stating:

1. **A planar track is filed under the effect instance, not the media.** A camera solve
   describes a *file* and two layers cutting the same shot are one analysis; a planar track
   describes the *quad*, and two Planar tracks on one clip are two answers of which a
   media-keyed store could hold only one. `Job::media` is therefore documented as "what the
   answer is filed under" and carries the `EffectInstance` id for a planar job. The store is
   a second table (`planars()`), read through `lumit_core::track::PlanarTrackStore` —
   `planar_range` and `planar_corners`, the two-method shape `CameraSolveStore` is.
2. **The quad rides in `MaskTrack`, which is where it belongs.** It is a region deciding
   where features may live, so `MaskTrack::within(outline)` carries it, `at(t)` prepends it
   inverted, and `feed` hashes it into the analysis key with the rest of the geometry —
   which means a re-drawn quad names a different file and cannot read a stale answer back.
   Putting it in `AnalysisSettings` would have needed that struct to stop being `Eq`, and
   would have been a second place for geometry to live.
3. **`FORMAT_VERSION` went to 3** for the `Answer` enum. Version 2 records are orphaned —
   one re-analysis each — which is the disposal that constant exists to perform, and is
   cheaper than a second magic, a second record type and a second reader kept in step.
4. **The warm pass reads planar tracks too**, one job per effect instance beside the one
   job per media the camera half asks for. There is nothing to deduplicate on the planar
   side, because two instances are two answers by construction.

### The Corner pin it writes

`lumit_core::track::corner_pin_from_track(doc, comp, tracked, effect, target, store)`
builds one `Op::SetLayerEffects` appending a Corner pin to `target`, its eight point
parameters keyframed one key per composition frame of the target layer's own extent. The
shape is `bake_solve_link`'s, deliberately: eight tracks filled in one walk of the frames,
keyframe times in the target layer's own **layer** time, linear both sides because the
samples are one per frame and there is nothing between two of them to shape.

Which source frame each key reads comes from `tracked_source_time` — the camera link's own
walk (§5a), now public because two questions ask it and two walks over one time chain would
be two chances to disagree about which moment is on screen. So a trimmed clip, a speed ramp,
a reordered Sequence layer and a precomp all come out right for free, and a comp frame past
the track's span clamps into it, which is the same hold a linked camera performs.

Three decisions there:

1. **Keys, not a link.** A solve-linked camera is a link because a camera solve describes
   the file and survives every edit to the clips cutting it. A corner pin is a *look* on one
   layer of one length that the user is going to adjust — soften a corner, ease a hit, trim
   the tail — and keys are what that wants: real, editable, drawn by the graph editor, taken
   back by the ordinary undo, with nothing left resolving behind them.
2. **Appended, not replacing.** A warp belongs last in a stack, and a layer that already has
   a Corner pin keeps it — the user asked for a pin, not for a tidy-up.
3. **`corner_pin_from_track` refuses rather than writing defaults.** Nothing tracked under
   this instance, no target layer, or no frames to key, and it returns `None`; the bridge
   turns that into `NoSolve` or `InvalidLayer`. A pin full of the schema's own keystone
   would be the hardest kind of fault to see.

### The surface

`fire_effect_action` learned the Planar track's three buttons — Analyse, Cancel and **pin**
— so a press is one crossing whichever effect made it. `planar_status(layer, effect)`
answers the status row: the stage, the frames done and total, the failure reason, the span
against the clip, and the re-anchor count. It is a struct of its own rather than
`BridgeTrackStatus` with two fields ignored, because a planar track has no point cloud and
no reprojection error and a camera solve has no re-anchor count; one struct carrying both
would have four rows that mean nothing in half the places they are read.
`create_corner_pin(tracked, effect)` is the gesture, reading the **Pin layer** row for its
target. `BridgeTrackFailure` gained `NotPlanar`; the other five reasons are shared, because
the two effects share a tracker and therefore share its refusals.

The panel is `PlanarTrackDisplayFrb`: the Camera track's polling shape exactly — twice a
second, only while the reading is moving, never subscribed to — the same `TrackSpanBar`
above the line, and one extra line when the run re-anchored.

### The transform it writes

docs/08 §4's Tracker row names two 2D deliverables, and the second one — a **point** track
baked into a layer's transform — needed no second tracker. A quad's four corners already
carry the whole of it: their centroid is a position, the direction of their two horizontal
edges is a turn, the length of those edges is a growth.

`lumit_core::track::transform_from_track(doc, comp, tracked, effect, target,
scale_and_rotation, store)` builds one `Op::Batch` of `Op::SetTransformProperty` — Position
x and y always, Rotation and Scale x and y when asked for — keyframed one key per
composition frame of the target's own extent. It shares `corner_pin_from_track`'s frame walk
outright: `tracked_quads` was lifted out of the pin so that which source moment a frame
reads, how a frame past the track clamps into it, and which layer time a key lands at are one
answer rather than two that could drift apart. `TRANSFORM_PARAM` and `TRANSFORM_POSITION` are
the row and the narrow option; the bridge reads them in `create_transform_keys`, which is
`create_corner_pin`'s sibling down to its refusals and is *not* on the frb surface (nothing in
Dart calls it — the press arrives through `fire_effect_action`, and a generated binding
nobody uses is a codegen run nobody needed).

Five things are decisions rather than transcription:

1. **"One point or two" is a row, not a mode.** A per-patch tracker needs two points before
   it can speak about rotation or scale, and that is where the user's mental model comes
   from. This tracker fits a warp over every feature inside the quad, so one analysis carries
   position, rotation and scale together — a second, narrower kind of track would be a second
   answer that could disagree with the first, over a smaller region, for less. What survives
   of the distinction is real and is what the row says: *how much of the answer to trust*.
2. **Added, not stamped.** Each key is the property's own value at that moment plus the
   delta since the reference frame (times it, for scale). Stamping the tracked centre
   absolutely would teleport the layer onto the tracked feature, which is never the gesture,
   and sampling per frame rather than once means an already-animated property keeps its
   animation underneath for free.
3. **Position alone writes two properties and touches nothing else.** Writing a rotation of
   nought over one the user animated would be the button quietly deleting their work, so the
   op list is truncated rather than filled with no-ops.
4. **The pose comes from both horizontal edges, summed as vectors.** A projective warp treats
   opposite edges differently, so the pair together is the honest middle; and adding two
   vectors then taking one `atan2` cannot wrap round ±180°, where averaging two angles can —
   one wrong frame in the middle of an otherwise clean spin. The turn is then unwrapped
   across frames, so a shot that spins past a full circle keeps counting.
5. **The angle needs no sign flip.** Composition space runs y down, so `atan2` there is
   clockwise-positive — which is exactly what `Mat4::from_rotation_z` means in the same
   space. Nothing to forget.

Recorded rather than fixed: the layer turns and scales about its **own anchor point**, not
about the tracked feature. That is the ordinary compositing gesture (move the anchor to where
the pin should pivot) and making the bake move the anchor as well would be it editing a
property the user did not ask about.

### Stage 6's tests

**Seven in `lumit-track`**, and unlike phases 2 and 3 they *draw*, for phase 1's reason: the
claim is about pixels reaching corners, so the input has to be pixels. The ground truth is
the projective warp each frame was rendered under, applied to the quad as drawn — an exact
formula, not a measurement. In order: a clean translation (median corner error 0.006 px,
worst 0.09, and **zero re-anchors**); a rotation of eight degrees with a 1.11× scale
(median 0.032, worst 1.19); a genuine perspective tilt, with a second assertion that the two
top corners moved by visibly different amounts so the test cannot silently become the
translation one again; an occluder painted across the middle of the plane with a mask over
it, where the second claim is the one that bites — that *plenty* was still tracked outside
the band, since a run that tracked almost nothing would pass the error bound by having
nothing to be wrong about; a quad over the flat surround refused as `TooFewFeatures`; two
runs compared with `assert_eq!`; and the cancellation seam, refused part-way with the flag
provably consulted per frame and bit-identical to `solve_planar` when never raised.

**The accuracy is asserted as a distribution**, median hard and tail loose, which is §2's
ninth deviation met from the other end and for a sharper reason: the most warped frame's
corners are the furthest extrapolation from the features that decided them, so the tail is
legitimately looser than the middle. A single bound tight enough to mean anything about the
homography would fail on geometry rather than on a defect.

**Three in `lumit-core`**: the pin's eight numbers frame for frame against a written-down
store, with one undo taking the whole pin back; the same through a half-speed retime into a
freeze, which is the full-footage rule restated from the planar side; and both refusals,
including a store holding the right shape of answer under a *different* effect id — the
failure a media-keyed store would have made silently.

**Two in `lumit-render`**: a real analysis of a rendered sliding plane, checked on the
corners against the warp each frame was drawn under (worst 0.94 px), on `reanchors == 0`, on
the inlier crowd, on the answer reaching the store through `PlanarTrackStore`, and on the
Corner pin written from it landing on the surface — the whole path in one test; and a quad
over a blank patch refusing and leaving nothing in the store. Its fixture is deliberately
*not* §5b's `Shot`: that one is two planes at different depths so the camera solve has
parallax to find, and parallax is exactly what a planar track has no use for.

**Three more in `lumit-core` for the transform half**: a rigid quad that slides by
(2, 3) px a frame, turns 7° a frame and grows 1 % a frame — an exact formula, so the bake's
Position, Rotation and Scale are checked against arithmetic rather than against a
measurement, including the turn reaching 413° at frame 59 rather than snapping back to −53°,
and the reference frame's keys landing on the layer's own untouched numbers; *position alone*
leaving Rotation and Scale static; and both refusals. **One more in `lumit-bridge`**: the
press crossing `fire_effect_action`, the target layer moving a hundred pixels over ten
frames, one undo taking the whole transform back, and the Transform row narrowing the next
press to two properties.

**One in `lumit-bridge`** (the status crossing, the pin written through the read model, and
both refusals) and **three in Flutter** (the three Action rows with a press proved to reach
the engine, the partial sentence leading with its span, and the span bar and re-anchor line
appearing only when earned — the last through the display's optional `fetch`, which is
§5c's seam one level up).

### Owed

On-canvas handles for the quad: the four corners are panel rows today, and dragging them on
the picture is what a compositor expects. A **keyframed** quad, so the reference shape can
move — refused for now, and the refusal is a real one rather than an omission (§6's opening:
the quad is the shape the surface has on the reference frame), and the same for the point
boxes of §7. And a Planar track on a **Precomp** layer, which §5e already built the
machinery for on the camera side.

## 7. Point tracking, as built

`crates/lumit-track/src/planar.rs` again — the same file, because the answer is the same
type — with the job's branch in `crates/lumit-render/src/track.rs` and the row in
`crates/lumit-core/src/fx/effects/planar_track.rs` (2026-08-31).

### In plain terms

§6 can only ask its question of something flat. A light on a car, a badge on a moving
shoulder, two marks on opposite walls of a room: there is no surface there to ask about, and
the honest question is much smaller — **where did this speck go** — asked of one small patch
at a time.

One patch gives a position. Two give a position, a turn and a growth, from the line between
them. Because each patch is followed entirely on its own, two of them need no relation to
each other whatever: different depths, different objects, opposite corners of the shot.

### The recipe

1. **Follow** says one point or two. The user puts Point 1 — and, for two, Point 2 — on the
   clip's first frame, with **Region size** setting how wide each search box is.
2. The tracker is confined to those boxes, by the same inverted `ExclusionMask` the quad
   uses. Two boxes are **one region of two contours**, not two regions: regions are unioned
   as *exclusions*, so two inverted boxes would forbid everything outside either of them,
   which is everything. `ExclusionMask` therefore holds `Vec<Vec<[f64; 2]>>` and tests
   even-odd across all of them, which for disjoint boxes reads as "inside either".
3. For each later frame, each box takes the **median** step of the correspondences that were
   within `radius` of it on the anchor frame. Two numbers is all a point has to give, and a
   median is already the robust estimate of them.
4. One point's positions make a translation; two points' make a similarity, by one complex
   division: the map taking `a₀ → b₀` and `a₁ → b₁` while keeping angles is
   `z ↦ b₀ + w·(z − a₀)` with `w = (b₁ − b₀) / (a₁ − a₀)`, whose argument is the turn and
   whose modulus is the growth.
5. That warp is applied to the region box, and the box's corners are the answer.

The surface is `solve_points(&TrackSet, reference_frame, points, quad, &PointSettings)`,
with `solve_points_cancellable` taking the crate's usual per-frame `stop`. `points_quad` and
`point_outlines` build the region box and the search boxes.

Five things are decisions rather than transcription:

1. **The answer is a `PlanarTrack`.** Downstream — the store, the sidecar, the status row,
   the span bar, the Corner pin, the transform keys — a track is a quad per frame. A second
   answer shape would have made every one of those a union to unwrap before it could be
   drawn, to say something the first shape can already carry: a point track is a planar
   track whose warp is constrained, and saying so costs nothing.
2. **A median, not a RANSAC.** §6 fits eight numbers and needs a robust search to do it.
   A point fits two, and the median *is* the robust estimator of two — a feature that
   crawled onto a passing hand is outvoted rather than weighted. `min_inliers` is three, so
   the median has a middle rather than an average of two.
3. **Re-anchoring composes nothing.** §6 must remember the homography from the reference
   frame to its anchor and multiply. Here the positions are absolute, so a re-anchor simply
   starts measuring from a nearer frame; the drift argument is met a second time and more
   cheaply.
4. **Detection is denser inside a point's box.** The bucket grid is over the whole frame, so
   a box the size of a badge lands inside a bucket or two and the ordinary two-per-bucket
   would leave a point standing on almost nothing. A point job raises `per_bucket` to 32.
   Everything outside the boxes is excluded, so this costs nothing anywhere else — the
   buckets the boxes do not touch have nothing to detect. A per-region detector is the
   upgrade if a very small box ever starves; the number is marked `ponytail:` where it sits.
5. **The Follow index is an `AnalysisSettings` field, fed into the key only when it is not
   the surface.** It changes what the analysis *finds*, so it belongs in the key — a
   one-point box and a quad drawn to the same outline are the same geometry asked two
   different questions. Feeding it unconditionally would have renamed every answer already
   in a sidecar, camera solves included, for a collision nobody has hit.

### Stage 7's tests

**Four in `lumit-track`**, and they draw, for §6's reason. The fixture is deliberately *not*
§6's warped plane: two textured squares on a flat background, each moved by its **own**
translation, so no single warp explains both and the claim is that none is asked for. In
order: one point's slide, with every corner of the reported box within 0.5 px of the
reference box moved by exactly what the patch did; two independent points, checked against
the similarity the render actually applied, with a second assertion that the box really did
turn and stretch so the test cannot silently become the first one; a box over the flat
background refused as `TooFewFeatures`, and two runs compared with `assert_eq!`; and the
cancellation seam, refused part-way with the flag provably consulted per frame and
bit-identical to `solve_points` when never raised.

**Two in `lumit-render`**: a real analysis of two independently sliding patches through a
`JobKind::Points` job — the multi-contour confinement, the raised bucket count and the branch
— checked on the corners against the fixture's own similarity (worst under 2 px), on
`reanchors == 0`, and on the answer reaching the store through `PlanarTrackStore`; and the
one-point case, where the box slides and its edges provably keep their length and their
angle.

**One in `lumit-bridge`**, folded into the transform-keys test: the **Follow** row set to
one point, and the next press writing Position alone.

### Owed

On-canvas handles for the two points, exactly as for the quad. A third point, which would
make an affine fit possible — refused for now because two is what the ask was and a third
row nobody uses is a row to keep working. And the region box is one size for both points;
two boxes of different sizes would be a second row for a second thing.

## Open questions

- GPU KLT: the pyramid and gradient passes are natural WGSL; phase 1 is CPU-first
  and profiled before anything moves (13-PERFORMANCE budgets decide). Now
  profiled, and the answer is not the KLT: the whole-frame Shi–Tomasi response
  pass costs more than the tracking does whenever re-detection runs (§2's "As
  built"). Separable box sums first; WGSL only if that is not enough.
- Object rigid-pose solve (track group → 6-DoF against the solved camera) is phase
  4+ and needs its own note section when reached; 2D export ships first.
- Lens distortion beyond k1/k2 (anamorphic) — revisit against real footage.
- ~~A zoom inside a shot that is also moving forward is not detected at all~~ —
  **resolved 2026-08-26**: the detector now judges a pair against its
  neighbours (§3's deviation 9 — the radial-flow-versus-parallax signature and
  the travel baseline), and a ramp's focal is solved as knots in the bundle
  (§4's deviation 7). The failing-first fixtures reproduced the 2026-08-24
  measurements — the creep-with-scope-in came back as one whole-shot `Ramp`
  whose median (0.00554) was indistinguishable from the creep alone (0.00551),
  and the end-to-end solve's worst per-frame focal error was 105 % — and the
  same fixtures now recover the cut, the ramp's span and the focal curve (the
  measured numbers are in §4 and §5). Two ends remain open: `SolveNote` still
  does not cross the bridge, so the panel cannot yet say "the lens moved during
  this shot"; and the *absolute* focal of a forward-dominated shot is weakly
  observable (measured 3.5 % low on the synthetic) — a focal hint from the user
  is the honest lever if it matters in practice.

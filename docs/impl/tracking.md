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
| **4 — surface** | K-417's shape: the Camera track *effect* (identity render, Analyse/Cancel actions, status), the background analysis job keyed to (media, settings) with the sidecar `track/` cache, the solve-linked dynamic Camera layer reading through the comp→clip→source time chain, Convert to keyframes, the point-cloud overlay with select → Null/Solid, and `ParamKind::Action`. 2D track → keyframed transform / corner-pin export rides the same store. | **Built** (§5a, §5b, §5c) |

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

## 5a. Phase 4, stage 1 — the model half, as built

`crates/lumit-core` (2026-08-21). Stage 1 is everything K-417 decides that can be
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
  convention from K-414 for a different reason: this one holds a *job*). Analyse
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
  and every op yet to be written.

Six things are deviations from, or decisions under, K-417's wording:

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
   properties, which (being read-only while linked) are the ones it had when the
   link was made. The two cases are different `LinkState`s, so the interface can
   say which.
3. **The tracked layer inside a precomp is found by the effect on it.** K-417 says
   the chain resolves through a Precomp layer to the tracked layer inside but does
   not say how the inside one is identified; the effect *is* the handle, so the
   layer carrying an enabled Camera track is the answer, first in stack order so
   it never depends on the playhead. A precomp with no tracked layer resolves to
   nothing and says `Unresolved` rather than guessing at the first footage it finds.
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
`lumit-track` — never a pool worker (K-417, and docs/05 §2's decode rule for the
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

Eight things are deviations from, or decisions under, K-417's wording:

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
3. **The sidecar is global, like `media-index/`, not per project.** K-417 says
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
   same pixels (K-248), so nothing converts. The mask is flattened at layer time
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
open and `clear()` to project close; a Camera track on a **Precomp** layer (K-417
allows it, and the analysis decodes media, so tracking a nested comp means
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

Seven things are deviations from, or decisions under, K-417's wording:

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
   the engine is a Dart compile error rather than a blank line. That is K-303's
   chain one step stricter than the import report's, which sends an id *and* its
   English as a fallback — this one has no free text to fall back to.
4. **The cloud is drawn always and clickable only when its layer is selected.**
   Show points says whether the dots are there; a cloud that also took every click
   would make the shot unselectable, and clicking the picture is how a layer is
   selected (K-217). Recorded in docs/07 §2.3.6 as the rule, because it is the one
   thing about the overlay that is not obvious from looking at it.
5. **The creation affordance is a floating row, not a context menu.** The gesture
   that makes the selection is a drag on the picture; asking for a second, hidden
   gesture to act on it would look calmer and be slower. Two buttons, under the
   picked points, clamped onto the panel.
6. **"Create camera" had to be invented.** K-417 describes what a solve-linked
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
`crates/lumit-track/src/lib.rs` (2026-08-24, K-540). Until this, a shot that
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
4. **The hold needed no new mechanism, and that was worth checking.** K-417's
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
one's arrive. `ProjectReference::close` calls `clear()` and touches no file.

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
- **A zoom inside a shot that is also moving forward is not detected at all, and
  a zoom that takes more than one frame pair can never be a cut.** Measured on
  `detect_zoom` (2026-08-24), from a real failure: a train POV of 7135 frames
  that solved acceptably until the shot scoped in and was wrong from there on.
  Two mechanisms, both in the run-merging at the top of the detector, and each
  sufficient on its own:
  - **A cut must be an isolated hot pair** (`end == frame`). A real lens rack
    takes a handful of frames, so its pairs are hot in a row and the whole run
    is classified `Ramp`. Measured: a 1.4× scope-in spread over four pairs comes
    back as one `Ramp` and no cut.
  - **Forward motion makes every pair hot**, because everything in the frame
    grows as the camera closes on it. A creep of 0.006 log-scale per pair — a
    slow dolly — is above `ramp_threshold` (0.004) on every pair of the shot, so
    the `while` loop swallows the *entire clip* into one boundary. Measured: the
    same creep with a genuine 1.4× single-pair scope-in planted in the middle
    returns one whole-shot `Ramp` whose median log-scale (0.0058) is
    indistinguishable from the creep alone (0.0058) — the scope-in is one sample
    in a median of thousands and disappears.
  The consequence is the one the owner saw. With no boundary there is one
  segment, and §4's deviation 2 gives the whole shot **one focal**; a shot
  carrying two lens settings is then fitted to a focal right for neither, and
  §4's deviation 3 already records that a focal well out bends every relative
  rotation with it. The solve does not refuse — it returns a camera path that is
  wrong from the rack onwards, with `SolveNote::ZoomRamp` set and nothing
  surfacing it.
  **This is not a small fix and should not be attempted as one.** Detecting the
  cut inside a run means judging a pair against its neighbours rather than
  against zero, and the `scale_only` cross-check that stops a lunge reading as a
  scope-in has to keep working while it does; and even with the boundary found,
  a *multi-frame* rack is a ramp, and a ramp still gets one averaged focal until
  the focal knots this note already owes (§4's deviation 7) exist in the bundle.
  The honest order is: solve the ramp's focal as a curve, then make the detector
  able to find a change of lens inside a moving shot. Until then the tracker's
  own reading of a shot like this should at least be *said* — `SolveNote` does
  not cross the bridge, and a line saying "the lens moved during this shot and
  the solve gave it one focal" is a day's work that would have answered this
  question without anyone reading this file.

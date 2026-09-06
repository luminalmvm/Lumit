//! Robust two-view geometry per frame pair: LO-RANSAC, the GRIC gate that tells
//! a pan from a move, and keyframe selection (docs/impl/tracking.md §3).
//!
//! # In plain terms
//!
//! [`crate::geom`] can work out the geometry of a frame pair from a list of
//! correspondences, but only if every correspondence is honest. In a real shot
//! some are not: a tracker point sitting on a passing car describes the car's
//! motion, not the camera's, and a handful of those is enough to drag a
//! least-squares answer somewhere useless.
//!
//! The cure is to stop trusting the list and start voting on it. Pick seven
//! correspondences at random, work out the geometry they imply, and count how
//! many of the *others* agree with it. Repeat. The camera's own motion is the
//! story the majority tells, so the sample that scores highest is the one drawn
//! entirely from still-world points, and everything that disagrees with it is
//! the moving object — identified, not by knowing what a car is, but by being
//! outvoted. Each time a better answer turns up it is refined once more using
//! *all* the points that agreed with it, which is the "LO" — local optimisation
//! — in LO-RANSAC, and is worth far more than more random samples.
//!
//! "At random" would be the end of determinism, so the random numbers are not
//! random: the sequence is seeded from the two frame numbers and the track
//! count, so the same pair of frames always draws the same samples in the same
//! order and the same clip solves to the same answer twice.
//!
//! One more question has to be settled per pair, and it is the one that decides
//! whether the pair is usable at all. If the camera only turned on the spot, or
//! only zoomed, there is no depth information in the pair whatsoever — every
//! point moves exactly as a flat sheet would, and asking where the camera
//! *travelled to* is asking a question the pictures cannot answer. The gate is
//! a GRIC comparison: score how well the fundamental matrix explains the pair,
//! score how well a homography does, charge each for the freedom it used, and
//! believe the cheaper one. A pair the homography wins is called
//! [`PairVerdict::RotationOnly`] and is kept for rotation and focal but never
//! fed to translation.

use crate::geom::{
    self, fundamental_eight_point, fundamental_seven_point, homography_dlt, sampson_distance,
    transfer_distance, Mat3,
};
use crate::{Correspondence, TrackSet};

/// Every knob the two-view geometry takes. The defaults are
/// docs/impl/tracking.md §3's numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometrySettings {
    /// Inlier threshold in source raster pixels (§3: ~1.5 px). Converted to the
    /// normalised units the estimator works in against the frame's own size,
    /// so it means the same thing on an SD plate and a 4K one.
    pub pixel_threshold: f64,
    /// The measurement noise GRIC weighs residuals against, in source pixels —
    /// about half the inlier threshold, since the threshold is meant to be a
    /// couple of standard deviations out.
    pub sigma_px: f64,
    /// Hard iteration cap per model. The inlier-ratio early exit usually stops
    /// long before this; the cap is what bounds the worst case.
    pub max_iterations: usize,
    /// The probability the search is required to have drawn at least one clean
    /// sample, which is what the early exit is computed from.
    pub confidence: f64,
    /// Correspondences below which a pair is not worth estimating at all.
    pub min_correspondences: usize,
    /// Inlier ratio below which no model explains the pair and it is called
    /// [`PairVerdict::Degenerate`].
    pub min_inlier_ratio: f64,
    /// Parallax, in source pixels, a pair must carry before keyframe selection
    /// will take it (§3: pairs are picked by parallax and inlier support).
    pub min_parallax_px: f64,
    /// Frames beyond an anchor that keyframe selection will look for a partner.
    pub max_keyframe_span: i64,
}

impl Default for GeometrySettings {
    fn default() -> Self {
        GeometrySettings {
            pixel_threshold: 1.5,
            sigma_px: 0.75,
            max_iterations: 300,
            confidence: 0.999,
            min_correspondences: 12,
            min_inlier_ratio: 0.5,
            min_parallax_px: 3.0,
            max_keyframe_span: 30,
        }
    }
}

/// What the GRIC gate decided a pair is (docs/impl/tracking.md §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairVerdict {
    /// The fundamental matrix earns its extra freedom: the pair carries
    /// parallax and can be used for translation.
    Translating,
    /// A homography explains the pair as well or better — the camera turned or
    /// zoomed but did not travel, or everything visible is on one plane. Usable
    /// for rotation and focal, never for translation.
    RotationOnly,
    /// Neither model explains enough of the pair. Not a fault: a shot can
    /// simply have a frame pair nothing carries across.
    Degenerate,
}

/// One frame pair's geometry, in **source raster pixels** — the same
/// coordinates the tracks live in, so a caller never converts.
#[derive(Clone, Debug, PartialEq)]
pub struct PairGeometry {
    pub from: i64,
    pub to: i64,
    /// The dominant epipolar geometry.
    pub fundamental: Mat3,
    /// The best homography over the same correspondences — the gate's other
    /// half, and the compensation [`Self::parallax`] is measured against.
    pub homography: Mat3,
    /// Track ids agreeing with [`Self::fundamental`], ascending.
    pub inliers: Vec<u32>,
    /// How many correspondences the pair had in all.
    pub correspondences: usize,
    /// `inliers.len() / correspondences`.
    pub inlier_ratio: f64,
    /// Median displacement left over once the homography's rotation and zoom
    /// are taken out, in source pixels. This is the number that says whether
    /// the pair has any depth information in it.
    pub parallax: f64,
    pub gric_fundamental: f64,
    pub gric_homography: f64,
    pub verdict: PairVerdict,
}

impl PairGeometry {
    /// Whether the track with `id` agreed with the dominant model.
    #[must_use]
    pub fn is_inlier(&self, id: u32) -> bool {
        self.inliers.binary_search(&id).is_ok()
    }
}

/// Estimate one frame pair's geometry from its correspondences.
///
/// `source_size` is the raster the points live in; it fixes the conditioning
/// transform everything is computed in and converts
/// [`GeometrySettings::pixel_threshold`] into that space. `from`/`to` are the
/// frame indices, which are recorded on the result **and** seed the sampler —
/// the same pair always draws the same samples.
///
/// `None` when the pair has too few correspondences, the raster is degenerate,
/// or no model could be fitted at all.
#[must_use]
pub fn estimate_pair(
    pts: &[Correspondence],
    source_size: (usize, usize),
    from: i64,
    to: i64,
    settings: &GeometrySettings,
) -> Option<PairGeometry> {
    if pts.len() < settings.min_correspondences.max(8) {
        return None;
    }
    let raster = Raster::of(source_size)?;
    let normalised: Vec<Correspondence> = pts
        .iter()
        .map(|c| Correspondence {
            id: c.id,
            from: raster.apply(c.from),
            to: raster.apply(c.to),
        })
        .collect();
    let thr = settings.pixel_threshold * raster.s;
    let sigma = settings.sigma_px * raster.s;
    let seed = seed_for(from, to, pts.len());

    let mut scratch: Vec<Mat3> = Vec::with_capacity(3);
    let f_fit = lo_ransac(
        &normalised,
        &Budget {
            thr,
            max_iterations: settings.max_iterations,
            confidence: settings.confidence,
            seed,
            sample: 7,
        },
        fundamental_seven_point,
        fundamental_eight_point,
        |m, c| sampson_distance(m, c.from, c.to),
        &mut scratch,
    )?;
    let h_fit = lo_ransac(
        &normalised,
        &Budget {
            thr,
            max_iterations: settings.max_iterations,
            confidence: settings.confidence,
            // A different stream for the second model: the same seed would
            // draw the same index sequence, and four of seven indices is not
            // an independent search.
            seed: mix(seed ^ 0x5DEE_CE66_D2FF_9C41),
            sample: 4,
        },
        |sample, out| {
            out.clear();
            if let Some(h) = homography_dlt(sample) {
                out.push(h);
            }
        },
        homography_dlt,
        |m, c| transfer_distance(m, c.from, c.to),
        &mut scratch,
    )?;

    let f_res: Vec<f64> = normalised
        .iter()
        .map(|c| sampson_distance(&f_fit.model, c.from, c.to))
        .collect();
    let h_res: Vec<f64> = normalised
        .iter()
        .map(|c| transfer_distance(&h_fit.model, c.from, c.to))
        .collect();
    let gric_fundamental = gric(&f_res, sigma, 3, 7);
    let gric_homography = gric(&h_res, sigma, 2, 8);

    // Parallax: what the homography could not explain. A pure pan or zoom
    // leaves nothing, which is exactly the reading that keeps such a pair out
    // of the translation solve.
    let mut left_over: Vec<f64> = normalised
        .iter()
        .filter_map(|c| {
            let q = geom::project(&h_fit.model, c.from)?;
            Some((q[0] - c.to[0]).hypot(q[1] - c.to[1]))
        })
        .collect();
    let parallax = geom::median(&mut left_over).unwrap_or(0.0) / raster.s;

    let mut inliers: Vec<u32> = f_fit
        .inliers
        .iter()
        .filter_map(|&i| normalised.get(i).map(|c| c.id))
        .collect();
    inliers.sort_unstable();
    let inlier_ratio = inliers.len() as f64 / normalised.len() as f64;
    let verdict = if gric_homography <= gric_fundamental {
        PairVerdict::RotationOnly
    } else if inlier_ratio >= settings.min_inlier_ratio {
        PairVerdict::Translating
    } else {
        PairVerdict::Degenerate
    };

    let rm = raster.matrix();
    let fundamental = geom::mul3(&geom::transpose3(&rm), &geom::mul3(&f_fit.model, &rm));
    let homography = geom::mul3(&raster.inverse_matrix(), &geom::mul3(&h_fit.model, &rm));
    Some(PairGeometry {
        from,
        to,
        fundamental,
        homography,
        inliers,
        correspondences: normalised.len(),
        inlier_ratio,
        parallax,
        gric_fundamental,
        gric_homography,
        verdict,
    })
}

/// The robust homography between two frames, and the ids that agreed with it.
///
/// Half of [`estimate_pair`] — the same LO-RANSAC over four-point DLT samples
/// under the same frame-sized conditioning, without the fundamental matrix, the
/// GRIC comparison or the parallax that a *camera* pair needs and a **planar**
/// track has no use for (docs/impl/tracking.md §6). Sharing the machinery rather
/// than the entry point is what keeps the planar tracker from paying for a
/// fundamental fit on every frame of a shot.
///
/// The model comes back **in pixels**: conditioning is an implementation detail
/// of the fit, and a caller composing two of these must not have to know which
/// space they are in.
///
/// `None` when there are too few correspondences to fit at all, when the raster
/// is degenerate, or when no model was found.
#[must_use]
pub fn homography_ransac(
    pts: &[Correspondence],
    source_size: (usize, usize),
    from: i64,
    to: i64,
    settings: &GeometrySettings,
) -> Option<(Mat3, Vec<u32>)> {
    if pts.len() < 4 {
        return None;
    }
    let raster = Raster::of(source_size)?;
    let normalised: Vec<Correspondence> = pts
        .iter()
        .map(|c| Correspondence {
            id: c.id,
            from: raster.apply(c.from),
            to: raster.apply(c.to),
        })
        .collect();
    let mut scratch: Vec<Mat3> = Vec::with_capacity(1);
    let fit = lo_ransac(
        &normalised,
        &Budget {
            thr: settings.pixel_threshold * raster.s,
            max_iterations: settings.max_iterations,
            confidence: settings.confidence,
            // The same second stream `estimate_pair` gives its homography, so
            // the two agree frame for frame where both are run.
            seed: mix(seed_for(from, to, pts.len()) ^ 0x5DEE_CE66_D2FF_9C41),
            sample: 4,
        },
        |sample, out| {
            out.clear();
            if let Some(h) = homography_dlt(sample) {
                out.push(h);
            }
        },
        homography_dlt,
        |m, c| transfer_distance(m, c.from, c.to),
        &mut scratch,
    )?;
    let rm = raster.matrix();
    let homography = geom::mul3(&raster.inverse_matrix(), &geom::mul3(&fit.model, &rm));
    let mut inliers: Vec<u32> = fit
        .inliers
        .iter()
        .filter_map(|&i| normalised.get(i).map(|c| c.id))
        .collect();
    inliers.sort_unstable();
    Some((homography, inliers))
}

/// Walk the set's frames and pick the keyframe pairs the solve will stand on
/// (docs/impl/tracking.md §3).
///
/// From each anchor frame, candidate partners are tried in order until one
/// carries enough parallax and enough inlier support to be worth solving, and
/// that partner becomes the next anchor. A pair the GRIC gate calls
/// rotation-only is never chosen as a keyframe pair — it has no translation to
/// contribute — but it is still returned when nothing better exists over the
/// span, so phase 3 can see that the stretch was a pan and use it for rotation.
///
/// Returned in frame order, which is what the segmentation downstream assumes.
#[must_use]
pub fn select_keyframes(set: &TrackSet, settings: &GeometrySettings) -> Vec<PairGeometry> {
    let mut out = Vec::new();
    let Some((first, last)) = set.frame_range() else {
        return out;
    };
    let size = set.source_size();
    let mut anchor = first;
    while anchor < last {
        let mut chosen: Option<PairGeometry> = None;
        let mut fallback: Option<PairGeometry> = None;
        let limit = last.min(anchor.saturating_add(settings.max_keyframe_span));
        let mut to = anchor + 1;
        while to <= limit {
            let pts = set.correspondences(anchor, to);
            if pts.len() < settings.min_correspondences {
                // Support has collapsed; nothing further out will have more.
                break;
            }
            if let Some(g) = estimate_pair(&pts, size, anchor, to, settings) {
                if g.inlier_ratio >= settings.min_inlier_ratio {
                    let good = g.verdict == PairVerdict::Translating
                        && g.parallax >= settings.min_parallax_px;
                    if good {
                        chosen = Some(g);
                        break;
                    }
                    fallback = Some(g);
                }
            }
            to += 1;
        }
        match chosen.or(fallback) {
            Some(g) => {
                let next = g.to;
                out.push(g);
                // `next > anchor` always, because `to` starts at `anchor + 1`;
                // the guard is here so a future change cannot loop forever.
                anchor = if next > anchor { next } else { anchor + 1 };
            }
            None => anchor += 1,
        }
    }
    out
}

// --- LO-RANSAC --------------------------------------------------------------

struct Budget {
    thr: f64,
    max_iterations: usize,
    confidence: f64,
    seed: u64,
    sample: usize,
}

struct Fit {
    model: Mat3,
    inliers: Vec<usize>,
}

/// LO-RANSAC: minimal samples, a fixed iteration cap, a deterministic early
/// exit, and a re-fit on the inliers every time the best model improves.
fn lo_ransac<Min, Ref, Res>(
    pts: &[Correspondence],
    budget: &Budget,
    minimal: Min,
    refit: Ref,
    residual: Res,
    scratch: &mut Vec<Mat3>,
) -> Option<Fit>
where
    Min: Fn(&[Correspondence], &mut Vec<Mat3>),
    Ref: Fn(&[Correspondence]) -> Option<Mat3>,
    Res: Fn(&Mat3, &Correspondence) -> f64,
{
    let n = pts.len();
    if n < budget.sample || budget.sample > 8 {
        return None;
    }
    let mut rng = SplitMix(budget.seed);
    let mut best: Option<Fit> = None;
    let mut sample: Vec<Correspondence> = Vec::with_capacity(budget.sample);
    let mut inlier_pool: Vec<Correspondence> = Vec::with_capacity(n);
    let mut cap = budget.max_iterations;

    let mut iteration = 0usize;
    while iteration < cap {
        iteration += 1;
        sample.clear();
        let mut idx = [usize::MAX; 8];
        let mut drawn = 0usize;
        let mut guard = 0usize;
        while drawn < budget.sample && guard < 128 {
            guard += 1;
            let r = (rng.next() % n as u64) as usize;
            if idx.iter().take(drawn).any(|&k| k == r) {
                continue;
            }
            idx[drawn] = r;
            drawn += 1;
        }
        if drawn < budget.sample {
            continue;
        }
        for &i in idx.iter().take(budget.sample) {
            if let Some(c) = pts.get(i) {
                sample.push(*c);
            }
        }
        if sample.len() < budget.sample {
            continue;
        }
        minimal(&sample, scratch);
        for k in 0..scratch.len() {
            let Some(model) = scratch.get(k).copied() else {
                continue;
            };
            let candidate = score(&model, pts, budget.thr, &residual);
            if candidate.inliers.len() <= best.as_ref().map_or(0, |b| b.inliers.len()) {
                continue;
            }
            // Local optimisation: re-fit on everything that agreed, and keep
            // going while that keeps growing the agreement. This is what makes
            // LO-RANSAC converge in tens of samples where plain RANSAC needs
            // thousands.
            let mut improved = candidate;
            for _ in 0..3 {
                inlier_pool.clear();
                inlier_pool.extend(improved.inliers.iter().filter_map(|&i| pts.get(i)).copied());
                let Some(m) = refit(&inlier_pool) else {
                    break;
                };
                let next = score(&m, pts, budget.thr, &residual);
                if next.inliers.len() <= improved.inliers.len() {
                    break;
                }
                improved = next;
            }
            let count = improved.inliers.len();
            best = Some(improved);
            // Recomputed from the best-so-far count alone, so the exit point is
            // a function of the data and not of when the loop happened to look.
            cap = required_iterations(
                count,
                n,
                budget.sample,
                budget.confidence,
                budget.max_iterations,
            )
            .max(iteration);
        }
    }
    best
}

fn score<Res>(model: &Mat3, pts: &[Correspondence], thr: f64, residual: &Res) -> Fit
where
    Res: Fn(&Mat3, &Correspondence) -> f64,
{
    let mut inliers = Vec::new();
    for (i, c) in pts.iter().enumerate() {
        let r = residual(model, c);
        if r.is_finite() && r <= thr {
            inliers.push(i);
        }
    }
    Fit {
        model: *model,
        inliers,
    }
}

/// How many samples are still needed for `confidence` that one was clean, given
/// the best inlier ratio seen so far. Capped, and never below one.
fn required_iterations(
    inliers: usize,
    n: usize,
    sample: usize,
    confidence: f64,
    cap: usize,
) -> usize {
    if inliers == 0 || n == 0 || !(0.0..1.0).contains(&confidence) {
        return cap;
    }
    let w = (inliers as f64 / n as f64).clamp(0.0, 1.0);
    let ws = w.powi(sample as i32);
    if ws >= 1.0 {
        return 1;
    }
    let den = (1.0 - ws).ln();
    if !den.is_finite() || den >= 0.0 {
        return cap;
    }
    let it = ((1.0 - confidence).ln() / den).ceil();
    if !it.is_finite() || it < 1.0 {
        return 1;
    }
    (it as usize).min(cap).max(1)
}

/// Torr's geometric robust information criterion: the residuals' cost, capped
/// so an outlier pays a fixed price rather than an unbounded one, plus a charge
/// for the model's dimension and its parameter count. The smaller score wins,
/// and the whole point of the capping is that a model cannot buy a better score
/// by explaining noise.
fn gric(residuals: &[f64], sigma: f64, dimension: usize, parameters: usize) -> f64 {
    let n = residuals.len();
    if n == 0 || sigma <= 0.0 || !sigma.is_finite() {
        return f64::INFINITY;
    }
    // r: a correspondence is four numbers. λ₃: the standard 2.
    let r = 4.0f64;
    let cap = 2.0 * (r - dimension as f64);
    let s2 = sigma * sigma;
    let mut sum = 0.0f64;
    for e in residuals {
        let x = if e.is_finite() { e * e / s2 } else { f64::MAX };
        sum += x.min(cap);
    }
    sum + r.ln() * dimension as f64 * n as f64 + (r * n as f64).ln() * parameters as f64
}

// --- Determinism ------------------------------------------------------------

/// Splitmix64's finaliser — the same mixer `tests.rs` builds its texture hash
/// from, and the house style for "a number derived from these numbers".
fn mix(mut z: u64) -> u64 {
    z ^= z >> 30;
    z = z.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The sampler's seed for one pair: `(from, to, correspondence count)` through
/// the mixer, per docs/impl/tracking.md §4's determinism ruling.
fn seed_for(from: i64, to: i64, count: usize) -> u64 {
    let a = mix((from as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let b = mix((to as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ a);
    mix((count as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ b)
}

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }
}

// --- Raster conditioning ----------------------------------------------------

/// The fixed similarity that puts the frame's centre at the origin and its long
/// edge at ±1.
///
/// Distinct from [`geom`]'s Hartley normalisation, which is data-driven and
/// applies inside each fit. This one is data-*independent* on purpose: it is
/// what turns a threshold in pixels into a threshold in the estimator's units
/// without that conversion depending on which correspondences a sample happened
/// to draw. Sampson distance is covariant under an isotropic similarity, so
/// thresholding at `pixel_threshold · s` here is exactly thresholding at
/// `pixel_threshold` pixels.
#[derive(Clone, Copy)]
struct Raster {
    s: f64,
    cx: f64,
    cy: f64,
}

impl Raster {
    fn of(size: (usize, usize)) -> Option<Raster> {
        let (w, h) = size;
        if w == 0 || h == 0 {
            return None;
        }
        Some(Raster {
            s: 2.0 / w.max(h) as f64,
            cx: w as f64 / 2.0,
            cy: h as f64 / 2.0,
        })
    }

    fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        [(p[0] - self.cx) * self.s, (p[1] - self.cy) * self.s]
    }

    fn matrix(&self) -> Mat3 {
        [
            [self.s, 0.0, -self.s * self.cx],
            [0.0, self.s, -self.s * self.cy],
            [0.0, 0.0, 1.0],
        ]
    }

    fn inverse_matrix(&self) -> Mat3 {
        [
            [1.0 / self.s, 0.0, self.cx],
            [0.0, 1.0 / self.s, self.cy],
            [0.0, 0.0, 1.0],
        ]
    }
}

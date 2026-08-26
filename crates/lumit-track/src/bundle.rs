//! Bundle adjustment — Levenberg–Marquardt on reprojection error with a sparse
//! Schur complement (docs/impl/tracking.md §4) — and the small dense linear
//! algebra the rest of phase 3 shares.
//!
//! # In plain terms
//!
//! By the time this file runs there is already an answer: a camera pose for
//! every keyframe, a focal length for every segment, and a cloud of 3D points.
//! It is a *good* answer, assembled from pieces that each looked at a fraction
//! of the shot. It is not the *best* answer, because no step so far has asked
//! the one question that matters: if all of that were true, would each point
//! land where the tracker actually saw it?
//!
//! That question has a number — the distance in pixels between where a point
//! ought to appear and where it was seen — and this file nudges every unknown
//! at once until the total is as small as it can be. The nudging is
//! Levenberg–Marquardt: work out which direction reduces the error fastest,
//! take a step, and if the step made things worse take a shorter and more
//! cautious one next time.
//!
//! The catch is size. A shot with fifty keyframes and three thousand points has
//! more than nine thousand unknowns, and the honest way to find the step is to
//! solve a nine-thousand-square system of equations. The trick that makes this
//! tractable is old and exact: every 3D point is seen by only a handful of
//! cameras, and no point interacts with any other point. So the point unknowns
//! can be **eliminated algebraically** — folded into the camera equations one
//! point at a time — leaving a system the size of the cameras alone (a few
//! hundred), which is small enough to solve outright. Once the cameras have
//! their step, each point's own step falls out by back-substitution. That
//! elimination is the Schur complement, and it is the difference between a
//! solve that takes a second and one that does not finish.
//!
//! One last guard: a tracker point that is simply wrong would, left alone, drag
//! everything towards itself, because its error is large and squaring it makes
//! it enormous. Errors past a threshold are therefore charged linearly rather
//! than quadratically (Huber), so a bad point can complain but cannot dictate.
//!
//! Everything is `f64`, every loop has a fixed order, and there is no iteration
//! whose count depends on anything but the input: two runs give the same bits.

use crate::geom::Mat3;

// --- Small dense linear algebra --------------------------------------------

/// A square dense matrix, row-major, with bounds-checked accessors so that an
/// index slip is a zero rather than a panic (14-ENGINEERING-RULES §4).
pub(crate) struct Dense {
    n: usize,
    a: Vec<f64>,
}

impl Dense {
    pub(crate) fn zero(n: usize) -> Dense {
        Dense {
            n,
            a: vec![0.0; n.saturating_mul(n)],
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.n
    }

    pub(crate) fn at(&self, r: usize, c: usize) -> f64 {
        if r >= self.n || c >= self.n {
            return 0.0;
        }
        self.a.get(r * self.n + c).copied().unwrap_or(0.0)
    }

    pub(crate) fn add(&mut self, r: usize, c: usize, v: f64) {
        if r >= self.n || c >= self.n {
            return;
        }
        let i = r * self.n + c;
        if let Some(x) = self.a.get_mut(i) {
            *x += v;
        }
    }

    pub(crate) fn set(&mut self, r: usize, c: usize, v: f64) {
        if r >= self.n || c >= self.n {
            return;
        }
        let i = r * self.n + c;
        if let Some(x) = self.a.get_mut(i) {
            *x = v;
        }
    }
}

/// Cholesky factor `L` with `A = L·Lᵀ`, or `None` when `A` is not positive
/// definite — which is the caller's signal to damp harder rather than an error.
pub(crate) fn cholesky(a: &Dense) -> Option<Dense> {
    let n = a.size();
    let mut l = Dense::zero(n);
    for i in 0..n {
        for j in 0..=i {
            let mut s = a.at(i, j);
            for k in 0..j {
                s -= l.at(i, k) * l.at(j, k);
            }
            if i == j {
                if !s.is_finite() || s <= 0.0 {
                    return None;
                }
                l.set(i, j, s.sqrt());
            } else {
                let d = l.at(j, j);
                if d.abs() < 1e-300 {
                    return None;
                }
                l.set(i, j, s / d);
            }
        }
    }
    Some(l)
}

/// Solve `L·Lᵀ·x = b` for the factor `L` from [`cholesky`].
pub(crate) fn cholesky_solve(l: &Dense, b: &[f64]) -> Vec<f64> {
    let n = l.size();
    let mut y = vec![0.0f64; n];
    for i in 0..n {
        let mut s = b.get(i).copied().unwrap_or(0.0);
        for k in 0..i {
            s -= l.at(i, k) * y.get(k).copied().unwrap_or(0.0);
        }
        let d = l.at(i, i);
        if let Some(slot) = y.get_mut(i) {
            *slot = if d.abs() > 1e-300 { s / d } else { 0.0 };
        }
    }
    let mut x = vec![0.0f64; n];
    for i in (0..n).rev() {
        let mut s = y.get(i).copied().unwrap_or(0.0);
        for k in (i + 1)..n {
            s -= l.at(k, i) * x.get(k).copied().unwrap_or(0.0);
        }
        let d = l.at(i, i);
        if let Some(slot) = x.get_mut(i) {
            *slot = if d.abs() > 1e-300 { s / d } else { 0.0 };
        }
    }
    x
}

/// Inverse of a symmetric 3×3, by cofactors. `None` when it is singular — a
/// point observed from one place has no 3D position, and that is the shape the
/// degeneracy takes here.
pub(crate) fn invert_sym3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !d.is_finite() || d.abs() < 1e-300 {
        return None;
    }
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            let (r0, r1) = ((c + 1) % 3, (c + 2) % 3);
            let (c0, c1) = ((r + 1) % 3, (r + 2) % 3);
            *cell = (m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]) / d;
        }
    }
    Some(out)
}

// --- The problem ------------------------------------------------------------

/// Which focal parameters a camera reads, and how: the value is
/// `(1 − t)·focals[a] + t·focals[b]` — one knot for a camera inside a
/// constant-focal segment (`a == b`), a linear blend of the two bracketing
/// knots for a camera inside a zoom ramp (docs/impl/tracking.md §4). Each
/// observation therefore contributes to at most two focal columns of the
/// reduced camera system, weighted `1 − t` and `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FocalRef {
    pub(crate) a: usize,
    pub(crate) b: usize,
    /// `0.0..=1.0` between knot `a` and knot `b`; `0.0` when `a == b`.
    pub(crate) t: f64,
}

impl FocalRef {
    /// A camera reading exactly one focal parameter.
    pub(crate) fn fixed(index: usize) -> FocalRef {
        FocalRef {
            a: index,
            b: index,
            t: 0.0,
        }
    }

    /// The focal this reference reads out of `focals`, or `None` where a knot
    /// index is out of range — the caller's signal to skip the observation, as
    /// a missing segment focal always was.
    pub(crate) fn value(&self, focals: &[f64]) -> Option<f64> {
        let fa = focals.get(self.a).copied()?;
        if self.a == self.b || self.t <= 0.0 {
            return Some(fa);
        }
        let fb = focals.get(self.b).copied()?;
        Some((1.0 - self.t) * fa + self.t * fb)
    }
}

/// One camera in the bundle: pose plus which focal parameters it reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BundleCamera {
    /// World → camera rotation.
    pub(crate) rot: Mat3,
    /// Camera centre in world coordinates.
    pub(crate) pos: [f64; 3],
    /// The focal knot or pair of knots this camera's frame reads.
    pub(crate) focal: FocalRef,
}

/// One observation: point `point` seen by camera `cam` at `image`, in source
/// raster pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BundleObs {
    pub(crate) cam: usize,
    pub(crate) point: usize,
    pub(crate) image: [f64; 2],
}

/// What the bundle did, so the caller can report it rather than assert it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BundleReport {
    pub(crate) iterations: usize,
    pub(crate) initial_mean_px: f64,
    pub(crate) mean_px: f64,
}

/// Where `x` lands in `cam`'s image, together with the camera-frame vector the
/// projection went through. `None` when the point is behind the camera.
pub(crate) fn project_point(
    cam: &BundleCamera,
    focal: f64,
    centre: [f64; 2],
    x: &[f64; 3],
) -> Option<([f64; 2], [f64; 3])> {
    let d = [x[0] - cam.pos[0], x[1] - cam.pos[1], x[2] - cam.pos[2]];
    let mut v = [0.0f64; 3];
    for (o, row) in v.iter_mut().zip(cam.rot.iter()) {
        *o = row[0] * d[0] + row[1] * d[1] + row[2] * d[2];
    }
    if !v[2].is_finite() || v[2] <= 1e-9 {
        return None;
    }
    Some((
        [
            centre[0] + focal * v[0] / v[2],
            centre[1] + focal * v[1] / v[2],
        ],
        v,
    ))
}

/// `∂image/∂(camera-frame vector)`, 2×3.
fn dproject(focal: f64, v: [f64; 3]) -> [[f64; 3]; 2] {
    let iz = 1.0 / v[2];
    let k = focal * iz;
    [[k, 0.0, -k * v[0] * iz], [0.0, k, -k * v[1] * iz]]
}

/// Huber weight and cost for one residual of norm `r`.
fn huber(r: f64, delta: f64) -> (f64, f64) {
    if r <= delta {
        (1.0, r * r)
    } else {
        (delta / r, delta * (2.0 * r - delta))
    }
}

/// Total Huber cost and the mean reprojection distance, or `INFINITY` where any
/// observation went behind its camera — which is the reading that makes
/// Levenberg–Marquardt refuse such a step rather than walk into it.
fn evaluate(
    cams: &[BundleCamera],
    focals: &[f64],
    points: &[[f64; 3]],
    obs: &[BundleObs],
    centre: [f64; 2],
    huber_px: f64,
) -> (f64, f64) {
    let mut cost = 0.0f64;
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for o in obs {
        let (Some(cam), Some(x)) = (cams.get(o.cam), points.get(o.point)) else {
            continue;
        };
        let Some(focal) = cam.focal.value(focals) else {
            continue;
        };
        let Some((p, _)) = project_point(cam, focal, centre, x) else {
            return (f64::INFINITY, f64::INFINITY);
        };
        let r = (p[0] - o.image[0]).hypot(p[1] - o.image[1]);
        cost += huber(r, huber_px).1;
        sum += r;
        n += 1;
    }
    if n == 0 {
        return (f64::INFINITY, f64::INFINITY);
    }
    (cost, sum / n as f64)
}

/// Rodrigues: the rotation `exp([w]×)`.
pub(crate) fn so3_exp(w: [f64; 3]) -> Mat3 {
    let t2 = w[0] * w[0] + w[1] * w[1] + w[2] * w[2];
    let t = t2.sqrt();
    // Below a milli-radian the series and the closed form agree to well past
    // f64's precision, and the closed form divides by t.
    let (s, c) = if t < 1e-8 {
        (1.0 - t2 / 6.0, 0.5 - t2 / 24.0)
    } else {
        (t.sin() / t, (1.0 - t.cos()) / t2)
    };
    let k = [[0.0, -w[2], w[1]], [w[2], 0.0, -w[0]], [-w[1], w[0], 0.0]];
    let mut out = [[0.0f64; 3]; 3];
    for (r, (row, krow)) in out.iter_mut().zip(k.iter()).enumerate() {
        for (col, cell) in row.iter_mut().enumerate() {
            let mut kk = 0.0;
            for (kv, kj) in krow.iter().zip(k.iter()) {
                kk += kv * kj[col];
            }
            *cell = if r == col { 1.0 } else { 0.0 } + s * krow[col] + c * kk;
        }
    }
    out
}

/// `a · b` for a 2×3 by a 3×3 — the chain rule every Jacobian here is one of.
fn compose23(a: &[[f64; 3]; 2], b: &Mat3) -> [[f64; 3]; 2] {
    let mut out = [[0.0f64; 3]; 2];
    for (orow, arow) in out.iter_mut().zip(a.iter()) {
        for (col, cell) in orow.iter_mut().enumerate() {
            let mut s = 0.0;
            for (av, brow) in arow.iter().zip(b.iter()) {
                s += av * brow[col];
            }
            *cell = s;
        }
    }
    out
}

/// One camera's parameter block start, and the layout's total width.
fn camera_base(index: usize) -> Option<usize> {
    // Camera 0 is the gauge: it holds the world still and takes no parameters.
    index.checked_sub(1).map(|i| i * 6)
}

fn layout_width(cameras: usize, focal_params: usize) -> usize {
    cameras.saturating_sub(1) * 6 + focal_params
}

/// Columns of the Jacobian for one observation: `(parameter index, ∂r/∂p)`.
/// At most eight — six pose, one or two focal knots — and the pose six are
/// absent for the gauge camera. `∂r/∂f = (v_x/v_z, v_y/v_z)`; a blended focal
/// splits that across its two knots by the blend weights, which is the chain
/// rule for `f = (1 − t)·f_a + t·f_b`.
fn columns(
    out: &mut Vec<(usize, [f64; 2])>,
    cam_index: usize,
    focal_base: usize,
    focal: FocalRef,
    rot: &Mat3,
    v: [f64; 3],
    jv: &[[f64; 3]; 2],
) {
    out.clear();
    if let Some(base) = camera_base(cam_index) {
        // ∂v/∂ω = −[v]× for the left-multiplied update R ← exp([ω]×)·R.
        let vx = [[0.0, -v[2], v[1]], [v[2], 0.0, -v[0]], [-v[1], v[0], 0.0]];
        let jw = compose23(jv, &vx);
        for (a, (x, y)) in jw[0].iter().zip(jw[1].iter()).enumerate() {
            out.push((base + a, [-x, -y]));
        }
        // ∂v/∂c = −R.
        let jc = compose23(jv, rot);
        for (a, (x, y)) in jc[0].iter().zip(jc[1].iter()).enumerate() {
            out.push((base + 3 + a, [-x, -y]));
        }
    }
    let g = [v[0] / v[2], v[1] / v[2]];
    if focal.a == focal.b || focal.t <= 0.0 {
        out.push((focal_base.saturating_add(focal.a), g));
    } else {
        let (wa, wb) = (1.0 - focal.t, focal.t);
        out.push((focal_base.saturating_add(focal.a), [wa * g[0], wa * g[1]]));
        out.push((focal_base.saturating_add(focal.b), [wb * g[0], wb * g[1]]));
    }
}

/// `∂r/∂X`, 2×3: the same chain rule as the position column, without the sign.
fn point_jacobian(rot: &Mat3, jv: &[[f64; 3]; 2]) -> [[f64; 3]; 2] {
    compose23(jv, rot)
}

/// One point's contribution, kept between damping trials so a rejected step
/// costs a linear solve rather than a whole re-linearisation.
struct PointBlock {
    v: [[f64; 3]; 3],
    b: [f64; 3],
    /// `(camera-or-focal parameter, the 3 numbers of that parameter's row of
    /// W)`, ascending by parameter so the reduction is order-independent.
    w: Vec<(usize, [f64; 3])>,
}

impl PointBlock {
    fn accumulate_w(&mut self, param: usize, add: [f64; 3]) {
        match self.w.binary_search_by_key(&param, |e| e.0) {
            Ok(i) => {
                if let Some(e) = self.w.get_mut(i) {
                    for (dst, src) in e.1.iter_mut().zip(add.iter()) {
                        *dst += src;
                    }
                }
            }
            Err(i) => self.w.insert(i, (param, add)),
        }
    }
}

/// Levenberg–Marquardt over poses, focal knots and points, with the points
/// marginalised by a Schur complement. `focals` is the knot vector: one entry
/// for a constant-focal segment, a sparse row of them for a zoom ramp, each an
/// independent column of the reduced system (docs/impl/tracking.md §4).
///
/// `cams[0]` is the gauge and never moves; the overall scale is left free and
/// held by the damping, which is what LM's diagonal term is for.
///
/// ponytail: the reduced camera system is dense and factorised outright, so the
/// cost is cubic in its width — six columns per keyframe plus one per focal
/// knot. That is the right trade at the tens-to-low-hundreds this pipeline
/// selects (docs/impl/tracking.md §4 says as much): 100 keyframes is a 600×600
/// factorisation, tens of megaflops, lost in the noise beside the residuals.
/// Cubic is unforgiving past that — 300 keyframes is 27× that work and 1000 is
/// a thousand times it, minutes inside a single LM iteration. The trigger is
/// therefore countable before it is felt: a solve whose keyframe selection
/// comes back with more than ~300 cameras (a long handheld take solved end to
/// end rather than in segments), or a camera track whose progress sits still
/// for minutes between iterations while `cancel` is the only way out. That
/// shot wants a sparse factorisation of the reduced system here, not a bigger
/// machine.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bundle_adjust(
    cams: &mut [BundleCamera],
    focals: &mut [f64],
    points: &mut [[f64; 3]],
    obs: &[BundleObs],
    centre: [f64; 2],
    huber_px: f64,
    max_iterations: usize,
    // Asked once per iteration; `true` stops the loop where it stands. The
    // caller discards the half-adjusted model — see `solve_camera_cancellable`.
    cancel: &dyn Fn() -> bool,
) -> BundleReport {
    let width = layout_width(cams.len(), focals.len());
    let (mut cost, initial_mean_px) = evaluate(cams, focals, points, obs, centre, huber_px);
    let mut report = BundleReport {
        iterations: 0,
        initial_mean_px,
        mean_px: initial_mean_px,
    };
    if width == 0 || points.is_empty() || obs.is_empty() || !cost.is_finite() {
        return report;
    }
    let focal_base = cams.len().saturating_sub(1) * 6;

    // Observations grouped by point, in a fixed order: the Schur reduction
    // walks one point at a time and must walk them identically every run.
    let mut order: Vec<usize> = (0..obs.len()).collect();
    order.sort_by_key(|&i| {
        obs.get(i)
            .map_or((usize::MAX, usize::MAX), |o| (o.point, o.cam))
    });

    let mut lambda = 1e-4f64;
    let mut cols: Vec<(usize, [f64; 2])> = Vec::with_capacity(8);
    let mut blocks: Vec<PointBlock> = Vec::new();

    for _ in 0..max_iterations {
        if cancel() {
            break;
        }
        // --- linearise ------------------------------------------------------
        let mut u = Dense::zero(width);
        let mut bc = vec![0.0f64; width];
        blocks.clear();
        blocks.reserve(points.len());
        for _ in 0..points.len() {
            blocks.push(PointBlock {
                v: [[0.0; 3]; 3],
                b: [0.0; 3],
                w: Vec::new(),
            });
        }
        for &oi in &order {
            let Some(o) = obs.get(oi) else { continue };
            let (Some(cam), Some(x)) = (cams.get(o.cam), points.get(o.point)) else {
                continue;
            };
            let Some(focal) = cam.focal.value(focals) else {
                continue;
            };
            let Some((p, v)) = project_point(cam, focal, centre, x) else {
                continue;
            };
            let r = [p[0] - o.image[0], p[1] - o.image[1]];
            let (w, _) = huber(r[0].hypot(r[1]), huber_px);
            let jv = dproject(focal, v);
            columns(&mut cols, o.cam, focal_base, cam.focal, &cam.rot, v, &jv);
            let jp = point_jacobian(&cam.rot, &jv);
            let Some(block) = blocks.get_mut(o.point) else {
                continue;
            };
            for (a, ca) in cols.iter() {
                for (b, cb) in cols.iter() {
                    u.add(*a, *b, w * (ca[0] * cb[0] + ca[1] * cb[1]));
                }
                if let Some(slot) = bc.get_mut(*a) {
                    *slot -= w * (ca[0] * r[0] + ca[1] * r[1]);
                }
                let mut add = [0.0f64; 3];
                for (k, cell) in add.iter_mut().enumerate() {
                    *cell = w * (ca[0] * jp[0][k] + ca[1] * jp[1][k]);
                }
                block.accumulate_w(*a, add);
            }
            for k in 0..3 {
                for l in 0..3 {
                    block.v[k][l] += w * (jp[0][k] * jp[0][l] + jp[1][k] * jp[1][l]);
                }
                block.b[k] -= w * (jp[0][k] * r[0] + jp[1][k] * r[1]);
            }
        }

        // A floor under the damping so an unobserved parameter — a segment with
        // no keyframe, say — cannot make the factorisation singular.
        let mut trace = 0.0f64;
        for i in 0..width {
            trace += u.at(i, i);
        }
        let floor = 1e-9 * (trace / width as f64).max(1e-9);

        // --- trial steps ----------------------------------------------------
        let mut improved = false;
        for _ in 0..8 {
            let mut s = Dense::zero(width);
            for i in 0..width {
                for j in 0..width {
                    s.set(i, j, u.at(i, j));
                }
                s.add(i, i, lambda * u.at(i, i) + floor);
            }
            let mut rhs = bc.clone();
            let mut inverses: Vec<Option<[[f64; 3]; 3]>> = Vec::with_capacity(blocks.len());
            for block in &blocks {
                let mut vd = block.v;
                for (k, row) in vd.iter_mut().enumerate() {
                    row[k] += lambda * block.v[k][k] + floor;
                }
                let Some(vi) = invert_sym3(&vd) else {
                    inverses.push(None);
                    continue;
                };
                // S ← S − W·V⁻¹·Wᵀ and rhs ← rhs − W·V⁻¹·b, over this point's
                // observing parameters only. That sparsity is the whole point.
                for (a, wa) in block.w.iter() {
                    let mut wv = [0.0f64; 3];
                    for (k, cell) in wv.iter_mut().enumerate() {
                        *cell = wa[0] * vi[0][k] + wa[1] * vi[1][k] + wa[2] * vi[2][k];
                    }
                    for (b, wb) in block.w.iter() {
                        let d = wv[0] * wb[0] + wv[1] * wb[1] + wv[2] * wb[2];
                        s.add(*a, *b, -d);
                    }
                    if let Some(slot) = rhs.get_mut(*a) {
                        *slot -= wv[0] * block.b[0] + wv[1] * block.b[1] + wv[2] * block.b[2];
                    }
                }
                inverses.push(Some(vi));
            }
            let Some(l) = cholesky(&s) else {
                lambda = (lambda * 10.0).min(1e12);
                continue;
            };
            let dc = cholesky_solve(&l, &rhs);
            if dc.iter().any(|v| !v.is_finite()) {
                lambda = (lambda * 10.0).min(1e12);
                continue;
            }

            // Back-substitution for the points, then a trial state.
            let mut trial_cams = cams.to_vec();
            let mut trial_focals = focals.to_vec();
            let mut trial_points = points.to_vec();
            for (i, cam) in trial_cams.iter_mut().enumerate() {
                let Some(base) = camera_base(i) else { continue };
                let mut w = [0.0f64; 3];
                let mut dp = [0.0f64; 3];
                for (k, (cell, pcell)) in w.iter_mut().zip(dp.iter_mut()).enumerate() {
                    *cell = dc.get(base + k).copied().unwrap_or(0.0);
                    *pcell = dc.get(base + 3 + k).copied().unwrap_or(0.0);
                }
                cam.rot = crate::geom::mul3(&so3_exp(w), &cam.rot);
                for (c, d) in cam.pos.iter_mut().zip(dp.iter()) {
                    *c += d;
                }
            }
            for (s_index, focal) in trial_focals.iter_mut().enumerate() {
                *focal += dc.get(focal_base + s_index).copied().unwrap_or(0.0);
                if !focal.is_finite() || *focal <= 1.0 {
                    *focal = 1.0;
                }
            }
            for (pi, block) in blocks.iter().enumerate() {
                let Some(Some(vi)) = inverses.get(pi) else {
                    continue;
                };
                let mut rp = block.b;
                for (a, wa) in block.w.iter() {
                    let d = dc.get(*a).copied().unwrap_or(0.0);
                    for (cell, wk) in rp.iter_mut().zip(wa.iter()) {
                        *cell -= wk * d;
                    }
                }
                let Some(x) = trial_points.get_mut(pi) else {
                    continue;
                };
                for (k, cell) in x.iter_mut().enumerate() {
                    *cell += vi[k][0] * rp[0] + vi[k][1] * rp[1] + vi[k][2] * rp[2];
                }
            }

            let (trial_cost, trial_mean) = evaluate(
                &trial_cams,
                &trial_focals,
                &trial_points,
                obs,
                centre,
                huber_px,
            );
            if trial_cost.is_finite() && trial_cost < cost {
                cams.copy_from_slice(&trial_cams);
                focals.copy_from_slice(&trial_focals);
                points.copy_from_slice(&trial_points);
                let relative = (cost - trial_cost) / cost.max(1e-300);
                cost = trial_cost;
                report.mean_px = trial_mean;
                report.iterations += 1;
                lambda = (lambda / 3.0).max(1e-12);
                improved = relative > 1e-12;
                break;
            }
            lambda = (lambda * 10.0).min(1e12);
        }
        if !improved {
            break;
        }
    }
    report
}

/// Refine one camera's pose against fixed 3D points — six unknowns, Gauss–
/// Newton with the same Huber weighting and the same Jacobian as the bundle.
///
/// This is what turns the DLT resection of an in-between frame from an
/// algebraic answer into a geometric one; the DLT minimises the wrong thing by
/// a fraction of a pixel, and this minimises the right one.
pub(crate) fn refine_pose(
    cam: &mut BundleCamera,
    focal: f64,
    world: &[[f64; 3]],
    image: &[[f64; 2]],
    centre: [f64; 2],
    huber_px: f64,
    iterations: usize,
) {
    let mut cols: Vec<(usize, [f64; 2])> = Vec::with_capacity(8);
    for _ in 0..iterations {
        let mut a = Dense::zero(6);
        let mut b = vec![0.0f64; 6];
        let mut used = 0usize;
        for (x, obs) in world.iter().zip(image.iter()) {
            let Some((p, v)) = project_point(cam, focal, centre, x) else {
                continue;
            };
            let r = [p[0] - obs[0], p[1] - obs[1]];
            let (w, _) = huber(r[0].hypot(r[1]), huber_px);
            let jv = dproject(focal, v);
            // Camera index 1 so the pose columns exist and start at zero; the
            // focal column lands out of range and is ignored by every add.
            columns(
                &mut cols,
                1,
                0,
                FocalRef::fixed(usize::MAX),
                &cam.rot,
                v,
                &jv,
            );
            used += 1;
            for (i, ci) in cols.iter().take(6) {
                for (j, cj) in cols.iter().take(6) {
                    a.add(*i, *j, w * (ci[0] * cj[0] + ci[1] * cj[1]));
                }
                if let Some(slot) = b.get_mut(*i) {
                    *slot -= w * (ci[0] * r[0] + ci[1] * r[1]);
                }
            }
        }
        if used < 3 {
            return;
        }
        for i in 0..6 {
            a.add(i, i, 1e-9 * a.at(i, i) + 1e-12);
        }
        let Some(l) = cholesky(&a) else { return };
        let d = cholesky_solve(&l, &b);
        if d.iter().any(|v| !v.is_finite()) {
            return;
        }
        let mut w = [0.0f64; 3];
        let mut dp = [0.0f64; 3];
        for (k, (cell, pcell)) in w.iter_mut().zip(dp.iter_mut()).enumerate() {
            *cell = d.get(k).copied().unwrap_or(0.0);
            *pcell = d.get(3 + k).copied().unwrap_or(0.0);
        }
        cam.rot = crate::geom::mul3(&so3_exp(w), &cam.rot);
        for (c, delta) in cam.pos.iter_mut().zip(dp.iter()) {
            *c += delta;
        }
        if w.iter().chain(dp.iter()).all(|v| v.abs() < 1e-12) {
            return;
        }
    }
}

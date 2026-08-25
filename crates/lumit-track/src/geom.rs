//! Two-view models and the residuals that judge them (docs/impl/tracking.md §3).
//!
//! # In plain terms
//!
//! Two pictures of the same still scene, taken from two places, are not
//! independent. A speck seen at one spot in the first picture can only appear
//! somewhere along one *line* in the second, and which line it is depends
//! entirely on how the camera moved between the two. The whole of that
//! relationship packs into a single 3×3 table of numbers — the **fundamental
//! matrix**. This file works that table out from a list of "this speck here went
//! to there" pairs, and measures how far a pair sits from the line the table
//! predicts. That distance is what later decides which specks belong to the
//! still world and which belong to something walking through the shot.
//!
//! A second table — the **homography** — describes the special case where the
//! second picture is something a flat sheet of paper could have done to the
//! first: the camera only turned on the spot or zoomed, or everything visible
//! lies on one plane. Comparing how well the two tables explain the same pairs
//! is how the next file up tells a pan from a move.
//!
//! Both are worked out the same way: write one linear equation per pair, stack
//! them, and find the direction that comes closest to satisfying all of them at
//! once — the eigenvector of the stack's normal matrix with the smallest
//! eigenvalue. Before any of that the points are shifted and scaled so they sit
//! around the origin at a typical distance of √2 (Hartley's normalisation),
//! because the raw pixel numbers differ in size by four orders of magnitude
//! across the equation and the answer would otherwise be arithmetic noise.
//!
//! Everything here is `f64`, has no state, and has no iteration whose order
//! depends on anything but its input: two runs give the same bits.

use crate::Correspondence;

/// A 3×3 matrix, row-major. The two-view models are all one of these.
pub type Mat3 = [[f64; 3]; 3];

/// Jacobi sweeps before the eigensolver gives up. Convergence is quadratic; a
/// 9×9 normal matrix is done in six or seven, and the cap only bounds the
/// pathological case.
const MAX_SWEEPS: usize = 20;

// --- The models ------------------------------------------------------------

/// The Hartley-normalised eight-point fundamental matrix, rank-2 enforced.
///
/// Needs at least eight correspondences; more are a least-squares fit over all
/// of them, which is what the LO-RANSAC local optimisation re-fits with.
/// `None` where the points are degenerate — all coincident, or collinear enough
/// that the normal matrix has no single smallest direction.
#[must_use]
pub fn fundamental_eight_point(pts: &[Correspondence]) -> Option<Mat3> {
    if pts.len() < 8 {
        return None;
    }
    let (na, nb) = (Hartley::of(pts, Side::From)?, Hartley::of(pts, Side::To)?);
    let mut ata = [[0.0f64; 9]; 9];
    for c in pts {
        let (p, q) = (na.apply(c.from), nb.apply(c.to));
        accumulate(&mut ata, &epipolar_row(p, q));
    }
    let (_, vecs) = eigen_ascending(&ata);
    let f = enforce_rank_two(&mat_from_column(&vecs, 0))?;
    normalise_scale(mul3(&transpose3(&nb.matrix()), &mul3(&f, &na.matrix())))
}

/// The seven-point fundamental matrices — LO-RANSAC's minimal sample.
///
/// Seven correspondences leave the linear system a two-dimensional null space
/// `F₁, F₂`; the missing constraint is `det F = 0`, which turns into a cubic in
/// the blend `α·F₁ + (1 − α)·F₂`. That cubic is solved in closed form and its
/// real roots are walked in ascending order, so one, two or three candidates
/// come back in an order that does not depend on anything but the input.
///
/// Candidates are appended to `out`, which is cleared first: the caller keeps
/// one vector across a whole RANSAC run rather than allocating per iteration
/// (14-ENGINEERING-RULES §5).
pub fn fundamental_seven_point(pts: &[Correspondence], out: &mut Vec<Mat3>) {
    out.clear();
    let Some(seven) = pts.get(..7) else {
        return;
    };
    let (Some(na), Some(nb)) = (Hartley::of(seven, Side::From), Hartley::of(seven, Side::To))
    else {
        return;
    };
    let mut ata = [[0.0f64; 9]; 9];
    for c in seven {
        let (p, q) = (na.apply(c.from), nb.apply(c.to));
        accumulate(&mut ata, &epipolar_row(p, q));
    }
    let (_, vecs) = eigen_ascending(&ata);
    let (f1, f2) = (mat_from_column(&vecs, 0), mat_from_column(&vecs, 1));

    // det(α·F₁ + (1 − α)·F₂) is a cubic in α; four evaluations pin its four
    // coefficients exactly, which is shorter and no less exact than expanding
    // the determinant symbolically.
    let at = |a: f64| det3(&blend(&f1, &f2, a));
    let (d0, d1, dm1, d2) = (at(0.0), at(1.0), at(-1.0), at(2.0));
    let c0 = d0;
    let c2 = 0.5 * (d1 + dm1) - d0;
    let half = 0.5 * (d1 - dm1);
    let c3 = (0.5 * (d2 - 4.0 * c2 - c0) - half) / 3.0;
    let c1 = half - c3;

    let (roots, count) = real_roots_cubic(c3, c2, c1, c0);
    let (ta, tb) = (na.matrix(), transpose3(&nb.matrix()));
    for a in roots.iter().take(count) {
        let m = blend(&f1, &f2, *a);
        if let Some(f) = normalise_scale(mul3(&tb, &mul3(&m, &ta))) {
            out.push(f);
        }
    }
}

/// The Hartley-normalised direct linear transform homography.
///
/// Four correspondences are the minimal sample; more are a least-squares fit.
/// `None` where the points are degenerate or the result is not invertible — a
/// singular homography maps the whole plane onto a line and explains nothing.
#[must_use]
pub fn homography_dlt(pts: &[Correspondence]) -> Option<Mat3> {
    if pts.len() < 4 {
        return None;
    }
    let (na, nb) = (Hartley::of(pts, Side::From)?, Hartley::of(pts, Side::To)?);
    let mut ata = [[0.0f64; 9]; 9];
    for c in pts {
        let (p, q) = (na.apply(c.from), nb.apply(c.to));
        accumulate(
            &mut ata,
            &[
                0.0,
                0.0,
                0.0,
                -p[0],
                -p[1],
                -1.0,
                q[1] * p[0],
                q[1] * p[1],
                q[1],
            ],
        );
        accumulate(
            &mut ata,
            &[
                p[0],
                p[1],
                1.0,
                0.0,
                0.0,
                0.0,
                -q[0] * p[0],
                -q[0] * p[1],
                -q[0],
            ],
        );
    }
    let (_, vecs) = eigen_ascending(&ata);
    let h = mul3(
        &nb.inverse_matrix(),
        &mul3(&mat_from_column(&vecs, 0), &na.matrix()),
    );
    let h = normalise_scale(h)?;
    if det3(&h).abs() < 1e-12 {
        return None;
    }
    Some(h)
}

// --- The residuals ---------------------------------------------------------

/// Sampson distance: the first-order approximation of the distance from a
/// correspondence to the nearest pair that `f` explains exactly, in the units
/// the points are given in.
///
/// This is the residual docs/impl/tracking.md §3 pins, and the reason it is not
/// the plain point-to-epipolar-line distance is that it is symmetric in the two
/// images without costing a second evaluation.
#[must_use]
pub fn sampson_distance(f: &Mat3, p: [f64; 2], q: [f64; 2]) -> f64 {
    let fp = mat_vec(f, [p[0], p[1], 1.0]);
    let ftq = mat_vec(&transpose3(f), [q[0], q[1], 1.0]);
    let num = q[0] * fp[0] + q[1] * fp[1] + fp[2];
    let den = (fp[0] * fp[0] + fp[1] * fp[1] + ftq[0] * ftq[0] + ftq[1] * ftq[1]).sqrt();
    if !den.is_finite() || den < 1e-15 {
        return f64::INFINITY;
    }
    (num / den).abs()
}

/// Symmetric transfer error of a homography: the mean of how far `h` puts `p`
/// from `q` and how far `h⁻¹` puts `q` from `p`, in the units of the points.
///
/// Symmetric rather than one-way because a homography that squashes half the
/// frame to a point would otherwise score beautifully in the direction of the
/// squash.
#[must_use]
pub fn transfer_distance(h: &Mat3, p: [f64; 2], q: [f64; 2]) -> f64 {
    let Some(hi) = invert3(h) else {
        return f64::INFINITY;
    };
    let (Some(fwd), Some(back)) = (project(h, p), project(&hi, q)) else {
        return f64::INFINITY;
    };
    0.5 * ((fwd[0] - q[0]).hypot(fwd[1] - q[1]) + (back[0] - p[0]).hypot(back[1] - p[1]))
}

/// Apply a homography to a point. `None` where the point maps to infinity.
#[must_use]
pub fn project(h: &Mat3, p: [f64; 2]) -> Option<[f64; 2]> {
    let v = mat_vec(h, [p[0], p[1], 1.0]);
    if !v[2].is_finite() || v[2].abs() < 1e-12 {
        return None;
    }
    Some([v[0] / v[2], v[1] / v[2]])
}

// --- Small shared arithmetic ------------------------------------------------

/// The median of `v`, sorted in place under a total order so a NaN cannot make
/// the answer depend on the comparison order. `None` for an empty slice.
pub(crate) fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let mid = *v.get(n / 2)?;
    Some(if n % 2 == 1 {
        mid
    } else {
        0.5 * (*v.get(n / 2 - 1)? + mid)
    })
}

pub(crate) fn mul3(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            let mut s = 0.0;
            for k in 0..3 {
                s += a[r][k] * b[k][c];
            }
            *cell = s;
        }
    }
    out
}

pub(crate) fn transpose3(m: &Mat3) -> Mat3 {
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in m.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            out[c][r] = *v;
        }
    }
    out
}

pub(crate) fn det3(m: &Mat3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

pub(crate) fn invert3(m: &Mat3) -> Option<Mat3> {
    let d = det3(m);
    if !d.is_finite() || d.abs() < 1e-15 {
        return None;
    }
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            // Cofactor of (c, r) — the adjugate is the transposed cofactor
            // matrix, so the indices swap here on purpose.
            let (r0, r1) = ((c + 1) % 3, (c + 2) % 3);
            let (c0, c1) = ((r + 1) % 3, (r + 2) % 3);
            *cell = (m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]) / d;
        }
    }
    Some(out)
}

pub(crate) fn mat_vec(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for (o, row) in out.iter_mut().zip(m.iter()) {
        *o = row[0] * v[0] + row[1] * v[1] + row[2] * v[2];
    }
    out
}

fn blend(a: &Mat3, b: &Mat3, t: f64) -> Mat3 {
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = t * a[r][c] + (1.0 - t) * b[r][c];
        }
    }
    out
}

/// Scale a model to unit Frobenius norm and fix its sign, so one geometry has
/// exactly one representation and two runs compare equal. `None` for a matrix
/// that is not finite, or is all zeros.
fn normalise_scale(m: Mat3) -> Option<Mat3> {
    let mut n = 0.0f64;
    for row in &m {
        for v in row {
            if !v.is_finite() {
                return None;
            }
            n += v * v;
        }
    }
    let n = n.sqrt();
    if n < 1e-300 {
        return None;
    }
    // Sign is free in both models (F and H are defined up to scale). Pinning it
    // to "the largest-magnitude entry is positive" is what makes the returned
    // matrix itself, and not merely its predictions, reproducible.
    let mut biggest = 0.0f64;
    let mut sign = 1.0f64;
    for row in &m {
        for v in row {
            if v.abs() > biggest {
                biggest = v.abs();
                sign = if *v < 0.0 { -1.0 } else { 1.0 };
            }
        }
    }
    let k = sign / n;
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = m[r][c] * k;
        }
    }
    Some(out)
}

/// The closest rank-2 matrix to `f`, which is what makes a fundamental matrix
/// an epipolar geometry rather than a general 3×3.
///
/// The usual recipe zeroes the smallest singular value of an SVD. There is no
/// SVD in this crate and there does not need to be: the right singular vectors
/// are the eigenvectors of `FᵀF`, and `σᵢuᵢ = F vᵢ`, so summing `(F vᵢ) vᵢᵀ`
/// over the two largest is exactly the rank-2 truncation with no `U` computed
/// at all. `None` when the second singular value is zero too — a rank-1 matrix
/// has no epipolar geometry to salvage.
fn enforce_rank_two(f: &Mat3) -> Option<Mat3> {
    let (vals, vecs) = eigen_ascending(&mul3(&transpose3(f), f));
    let mut out = [[0.0f64; 3]; 3];
    for i in [2usize, 1] {
        if vals[i].max(0.0).sqrt() < 1e-12 {
            return None;
        }
        let v = [vecs[0][i], vecs[1][i], vecs[2][i]];
        let u = mat_vec(f, v);
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell += u[r] * v[c];
            }
        }
    }
    Some(out)
}

/// The row of the linear system `qᵀ F p = 0` for one correspondence.
fn epipolar_row(p: [f64; 2], q: [f64; 2]) -> [f64; 9] {
    [
        q[0] * p[0],
        q[0] * p[1],
        q[0],
        q[1] * p[0],
        q[1] * p[1],
        q[1],
        p[0],
        p[1],
        1.0,
    ]
}

/// Add one row's contribution to the normal matrix `AᵀA`, so the 2n × 9 design
/// matrix is never materialised.
fn accumulate(ata: &mut [[f64; 9]; 9], row: &[f64; 9]) {
    for (dst, ri) in ata.iter_mut().zip(row.iter()) {
        for (cell, rj) in dst.iter_mut().zip(row.iter()) {
            *cell += ri * rj;
        }
    }
}

fn mat_from_column(vecs: &[[f64; 9]; 9], j: usize) -> Mat3 {
    let mut out = [[0.0f64; 3]; 3];
    for (i, cell) in out.iter_mut().flatten().enumerate() {
        *cell = vecs[i][j];
    }
    out
}

// --- Hartley normalisation --------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    From,
    To,
}

/// The similarity that puts a point set's centroid at the origin with a mean
/// distance of √2 from it — Hartley's conditioning, without which the eight-
/// point algorithm is a well-known way to get noise back.
#[derive(Clone, Copy)]
struct Hartley {
    s: f64,
    cx: f64,
    cy: f64,
}

impl Hartley {
    fn of(pts: &[Correspondence], side: Side) -> Option<Hartley> {
        if pts.is_empty() {
            return None;
        }
        let n = pts.len() as f64;
        let pick = |c: &Correspondence| if side == Side::From { c.from } else { c.to };
        let (mut cx, mut cy) = (0.0f64, 0.0f64);
        for c in pts {
            let p = pick(c);
            cx += p[0];
            cy += p[1];
        }
        cx /= n;
        cy /= n;
        let mut d = 0.0f64;
        for c in pts {
            let p = pick(c);
            d += (p[0] - cx).hypot(p[1] - cy);
        }
        d /= n;
        if !d.is_finite() || d < 1e-12 || !cx.is_finite() || !cy.is_finite() {
            return None;
        }
        Some(Hartley {
            s: std::f64::consts::SQRT_2 / d,
            cx,
            cy,
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

// --- The eigensolver --------------------------------------------------------

/// Eigenvalues and eigenvectors of a symmetric matrix, ascending by eigenvalue,
/// with the eigenvectors as **columns**.
///
/// Cyclic Jacobi: rotate away the largest off-diagonal entries in a fixed
/// (p, q) order until the off-diagonal sum is negligible. Fixed order and a
/// fixed sweep cap are what make it deterministic; a power method or a QR with
/// shifts would both be shorter to describe and longer to make reproducible.
pub(crate) fn eigen_ascending<const N: usize>(m: &[[f64; N]; N]) -> ([f64; N], [[f64; N]; N]) {
    let mut a = *m;
    let mut v = [[0.0f64; N]; N];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..MAX_SWEEPS {
        let mut off = 0.0f64;
        for (i, row) in a.iter().enumerate() {
            for cell in row.iter().skip(i + 1) {
                off += cell * cell;
            }
        }
        if off <= 1e-30 {
            break;
        }
        for p in 0..N {
            for q in (p + 1)..N {
                let apq = a[p][q];
                if apq.abs() <= 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let (rp, rq) = (row[p], row[q]);
                    row[p] = c * rp - s * rq;
                    row[q] = s * rp + c * rq;
                }
                let (rp, rq) = (a[p], a[q]);
                let (mut np, mut nq) = ([0.0f64; N], [0.0f64; N]);
                for (k, (x, y)) in rp.iter().zip(rq.iter()).enumerate() {
                    np[k] = c * x - s * y;
                    nq[k] = s * x + c * y;
                }
                a[p] = np;
                a[q] = nq;
                for row in v.iter_mut() {
                    let (rp, rq) = (row[p], row[q]);
                    row[p] = c * rp - s * rq;
                    row[q] = s * rp + c * rq;
                }
            }
        }
    }

    let mut vals = [0.0f64; N];
    for (i, val) in vals.iter_mut().enumerate() {
        *val = a[i][i];
    }
    // Insertion sort on an index array: N is nine at most, and this is one
    // place a comparison sort's tie-breaking must not depend on the sort's
    // internals, so ties keep their original order explicitly.
    let mut order = [0usize; N];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i;
    }
    for i in 1..N {
        let mut j = i;
        while j > 0 && vals[order[j - 1]] > vals[order[j]] {
            order.swap(j - 1, j);
            j -= 1;
        }
    }
    let mut sorted_vals = [0.0f64; N];
    let mut sorted_vecs = [[0.0f64; N]; N];
    for (dst, &src) in order.iter().enumerate() {
        sorted_vals[dst] = vals[src];
        for (r, row) in v.iter().enumerate() {
            sorted_vecs[r][dst] = row[src];
        }
    }
    (sorted_vals, sorted_vecs)
}

/// The real roots of `c3·x³ + c2·x² + c1·x + c0`, ascending.
///
/// Closed form throughout — Cardano for the one-real-root case and the
/// trigonometric form for three, which is the numerically sane branch and
/// avoids cube roots of complex numbers entirely. Degenerate leading
/// coefficients drop cleanly to a quadratic and then a linear.
fn real_roots_cubic(c3: f64, c2: f64, c1: f64, c0: f64) -> ([f64; 3], usize) {
    let mut r = [0.0f64; 3];
    if !c3.is_finite() || !c2.is_finite() || !c1.is_finite() || !c0.is_finite() {
        return (r, 0);
    }
    let scale = c3.abs().max(c2.abs()).max(c1.abs()).max(c0.abs());
    if scale < 1e-300 {
        return (r, 0);
    }
    if c3.abs() < 1e-12 * scale {
        if c2.abs() < 1e-12 * scale {
            if c1.abs() < 1e-12 * scale {
                return (r, 0);
            }
            r[0] = -c0 / c1;
            return (r, 1);
        }
        let disc = c1 * c1 - 4.0 * c2 * c0;
        if disc < 0.0 {
            return (r, 0);
        }
        let s = disc.sqrt();
        let (a, b) = ((-c1 - s) / (2.0 * c2), (-c1 + s) / (2.0 * c2));
        r[0] = a.min(b);
        r[1] = a.max(b);
        return (r, 2);
    }

    let (b, c, d) = (c2 / c3, c1 / c3, c0 / c3);
    let shift = -b / 3.0;
    let p = c - b * b / 3.0;
    let q = 2.0 * b * b * b / 27.0 - b * c / 3.0 + d;
    let disc = q * q / 4.0 + p * p * p / 27.0;

    if disc > 0.0 {
        let s = disc.sqrt();
        r[0] = (-q / 2.0 + s).cbrt() + (-q / 2.0 - s).cbrt() + shift;
        if !r[0].is_finite() {
            return ([0.0; 3], 0);
        }
        return (r, 1);
    }
    // disc ≤ 0 forces p ≤ 0; p = 0 then forces q = 0, a triple root.
    let m = 2.0 * (-p / 3.0).max(0.0).sqrt();
    if m < 1e-300 {
        r[0] = shift;
        return (r, 1);
    }
    let phi = ((3.0 * q) / (p * m)).clamp(-1.0, 1.0).acos() / 3.0;
    let third = std::f64::consts::TAU / 3.0;
    for (k, root) in r.iter_mut().enumerate() {
        *root = m * (phi - third * k as f64).cos() + shift;
    }
    if r.iter().any(|v| !v.is_finite()) {
        return ([0.0; 3], 0);
    }
    r.sort_by(f64::total_cmp);
    (r, 3)
}

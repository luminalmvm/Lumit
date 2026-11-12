//! The 2D transforms a stabiliser works with, and the robust fits that measure
//! them from pairs of points.
//!
//! In plain terms: a frame's motion is one of three things. A slide moves every
//! point by the same amount. A similarity also turns and grows the picture. A
//! homography is the four-corner stretch a corner pin makes. Each is fitted to
//! where hundreds of specks went, giving the specks that disagree (a passing car,
//! a hand) less and less say until only the picture's own motion is left.
//!
//! Everything here is deterministic: fixed loop orders and fixed iteration
//! counts, so the same points give the same bits.

/// A 3×3 matrix taking column vectors `(x, y, 1)`, row-major.
pub type Mat3 = [[f64; 3]; 3];

/// The identity.
pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// `a · b`: `b` applied first.
#[must_use]
pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0f64; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    out
}

/// The inverse, or `None` for a matrix that flattens the plane.
#[must_use]
pub fn invert(m: &Mat3) -> Option<Mat3> {
    let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
    let c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
    let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
    let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [
            c00 * d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
        ],
        [
            c01 * d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
        ],
        [
            c02 * d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
        ],
    ])
}

/// Where `m` takes `p`, or `None` past its horizon.
#[must_use]
pub fn apply(m: &Mat3, p: [f64; 2]) -> Option<[f64; 2]> {
    let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
    if !w.is_finite() || w.abs() < 1e-12 {
        return None;
    }
    let x = (m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / w;
    let y = (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / w;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

/// A similarity `(a, b, tx, ty)` as a matrix.
#[must_use]
pub fn from_similarity(s: [f64; 4]) -> Mat3 {
    [[s[0], -s[1], s[2]], [s[1], s[0], s[3]], [0.0, 0.0, 1.0]]
}

/// Nine row-major numbers as a matrix.
#[must_use]
pub fn from_rows(h: [f64; 9]) -> Mat3 {
    [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], h[8]]]
}

/// A matrix as nine row-major numbers, scaled so the last is one where it can
/// be.
#[must_use]
pub fn to_rows(m: &Mat3) -> [f64; 9] {
    let s = if m[2][2].abs() > 1e-12 {
        1.0 / m[2][2]
    } else {
        1.0
    };
    [
        m[0][0] * s,
        m[0][1] * s,
        m[0][2] * s,
        m[1][0] * s,
        m[1][1] * s,
        m[1][2] * s,
        m[2][0] * s,
        m[2][1] * s,
        m[2][2] * s,
    ]
}

/// The homography taking the four points `from` to the four points `to`, or
/// `None` when three of either lie on a line.
#[must_use]
pub fn four_point(from: [[f64; 2]; 4], to: [[f64; 2]; 4]) -> Option<Mat3> {
    let mut a = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let ([x, y], [u, v]) = (from[i], to[i]);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    let h = solve_augmented::<8, 9>(a)?;
    Some([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])
}

/// Solve the square system held as `N` rows of `N` coefficients and a right-hand
/// side, by Gaussian elimination with partial pivoting in a fixed order.
fn solve_augmented<const N: usize, const M: usize>(mut a: [[f64; M]; N]) -> Option<[f64; N]> {
    for col in 0..N {
        let mut pivot = col;
        for row in (col + 1)..N {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if !a[pivot][col].is_finite() || a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        for row in (col + 1)..N {
            let f = a[row][col] / a[col][col];
            let top = a[col];
            for (k, v) in a[row].iter_mut().enumerate().skip(col) {
                *v -= f * top[k];
            }
        }
    }
    let mut x = [0.0f64; N];
    for i in (0..N).rev() {
        let mut s = a[i][N];
        for j in (i + 1)..N {
            s -= a[i][j] * x[j];
        }
        x[i] = s / a[i][i];
        if !x[i].is_finite() {
            return None;
        }
    }
    Some(x)
}

/// Accumulates a weighted least-squares problem in `N` unknowns.
struct Normal<const N: usize> {
    ata: [[f64; N]; N],
    atb: [f64; N],
}

impl<const N: usize> Normal<N> {
    fn new() -> Self {
        Normal {
            ata: [[0.0; N]; N],
            atb: [0.0; N],
        }
    }

    fn add(&mut self, row: &[f64; N], rhs: f64, w: f64) {
        for i in 0..N {
            for j in 0..N {
                self.ata[i][j] += w * row[i] * row[j];
            }
            self.atb[i] += w * row[i] * rhs;
        }
    }

    fn solve(&self) -> Option<[f64; N]> {
        // Gaussian elimination on the augmented copy, as above, without the
        // const-generic width arithmetic stable Rust cannot spell.
        let mut a = self.ata;
        let mut b = self.atb;
        for col in 0..N {
            let mut pivot = col;
            for row in (col + 1)..N {
                if a[row][col].abs() > a[pivot][col].abs() {
                    pivot = row;
                }
            }
            if !a[pivot][col].is_finite() || a[pivot][col].abs() < 1e-12 {
                return None;
            }
            a.swap(col, pivot);
            b.swap(col, pivot);
            for row in (col + 1)..N {
                let f = a[row][col] / a[col][col];
                let top = a[col];
                for (k, v) in a[row].iter_mut().enumerate().skip(col) {
                    *v -= f * top[k];
                }
                b[row] -= f * b[col];
            }
        }
        let mut x = [0.0f64; N];
        for i in (0..N).rev() {
            let mut s = b[i];
            for j in (i + 1)..N {
                s -= a[i][j] * x[j];
            }
            x[i] = s / a[i][i];
            if !x[i].is_finite() {
                return None;
            }
        }
        Some(x)
    }
}

/// Moves the points to their centroid and scales them so their mean distance
/// from it is √2: the conditioning every linear fit here is done in.
#[derive(Clone, Copy)]
struct Frame {
    cx: f64,
    cy: f64,
    s: f64,
}

impl Frame {
    fn of(points: impl Iterator<Item = [f64; 2]> + Clone) -> Option<Frame> {
        let n = points.clone().count() as f64;
        if n < 1.0 {
            return None;
        }
        let (sx, sy) = points
            .clone()
            .fold((0.0, 0.0), |(a, b), p| (a + p[0], b + p[1]));
        let (cx, cy) = (sx / n, sy / n);
        let mean = points.map(|p| (p[0] - cx).hypot(p[1] - cy)).sum::<f64>() / n;
        (mean > 1e-9).then_some(Frame {
            cx,
            cy,
            s: std::f64::consts::SQRT_2 / mean,
        })
    }

    fn to(&self, p: [f64; 2]) -> [f64; 2] {
        [(p[0] - self.cx) * self.s, (p[1] - self.cy) * self.s]
    }

    /// The matrix doing [`Self::to`].
    fn matrix(&self) -> Mat3 {
        [
            [self.s, 0.0, -self.cx * self.s],
            [0.0, self.s, -self.cy * self.s],
            [0.0, 0.0, 1.0],
        ]
    }
}

/// How hard each pair's residual is believed, Cauchy-weighted against a scale
/// taken from the residuals themselves, so a crowd of points on a mover is
/// talked down rather than averaged in.
fn weights(residuals: &[f64], floor: f64) -> Vec<f64> {
    let mut sorted: Vec<f64> = residuals
        .iter()
        .copied()
        .filter(|r| r.is_finite())
        .collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(floor);
    let c = 2.385 * (1.4826 * median).max(floor);
    residuals
        .iter()
        .map(|&r| {
            if r.is_finite() {
                1.0 / (1.0 + (r / c) * (r / c))
            } else {
                0.0
            }
        })
        .collect()
}

/// The least residual scale [`weights`] believes, from `close`: half a pixel
/// of the raster the specks were followed on, wherever the pairs are judged,
/// as every other threshold is.
fn floor(close: f64) -> f64 {
    0.5 * close / CLOSE_TRACKED_PX
}

/// A pair of points the motion takes one to the other: `(from, to)`.
pub type Pair = ([f64; 2], [f64; 2]);

/// How far, in pixels of the raster the specks were followed on, a pair may
/// miss a fitted motion and still count as moving with it: the planar
/// tracker's inlier threshold, a couple of times the tracker's own noise on
/// soft detail.
pub const AGREE_TRACKED_PX: f64 = 1.5;

/// How far, in pixels of the raster the specks were followed on, a pair may
/// miss the robust fit and still be used in the last plain least-squares fit:
/// tighter than agreement, since every pair it lets in pulls on the answer.
pub const CLOSE_TRACKED_PX: f64 = 1.0;

/// `px` tracked pixels in the pixels of a raster `scale` times the tracked
/// one, which is what every pair is judged in.
fn on_layer(px: f64, scale: f64) -> f64 {
    let s = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    px * s
}

/// [`AGREE_TRACKED_PX`] on a raster `scale` times the tracked one.
#[must_use]
pub fn agree_px(scale: f64) -> f64 {
    on_layer(AGREE_TRACKED_PX, scale)
}

/// [`CLOSE_TRACKED_PX`] on a raster `scale` times the tracked one.
#[must_use]
pub fn close_px(scale: f64) -> f64 {
    on_layer(CLOSE_TRACKED_PX, scale)
}

/// The share of the frame's width and of its height a four-corner fit's pairs
/// must spread over. Its perspective terms are read off how the motion
/// changes across the pairs, and outside them they are only extrapolation.
const SPAN: f64 = 0.5;

/// Whether `pairs` spread far enough over a `size` frame, both ways, for a
/// four-corner fit to be made from them.
#[must_use]
pub fn spans(pairs: &[Pair], size: [f64; 2]) -> bool {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (p, _) in pairs {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (0..2).all(|k| hi[k] - lo[k] >= SPAN * size[k])
}

/// How far a four-corner fit may put a frame corner from where the step's
/// similarity puts it, as a share of the frame's diagonal. Between neighbouring
/// frames the two differ by the lean of a turning camera, a few pixels; a fit
/// further off than this was bent by a handful of pairs.
const CORNER_REACH: f64 = 0.03;

/// The step's four-corner motion from `pairs`, started from its similarity
/// `sim`: the four-corner fit where it passes its own checks, and the
/// similarity where it does not. The checks are that the pairs spread over the
/// frame, and that the fit keeps every frame corner near where the similarity
/// puts it.
#[must_use]
pub fn four_corner(pairs: &[Pair], sim: [f64; 4], size: [f64; 2], close: f64) -> [f64; 9] {
    let similar = from_similarity(sim);
    let fitted = spans(pairs, size)
        .then(|| fit_homography(pairs, &similar, close))
        .flatten()
        .map(|(h, _)| h)
        .filter(|h| {
            let m = from_rows(*h);
            let reach = CORNER_REACH * size[0].hypot(size[1]);
            [[0.0, 0.0], [size[0], 0.0], [0.0, size[1]], size]
                .iter()
                .all(|&c| match (apply(&m, c), apply(&similar, c)) {
                    (Some(a), Some(b)) => (a[0] - b[0]).hypot(a[1] - b[1]) <= reach,
                    _ => false,
                })
        });
    fitted.unwrap_or_else(|| to_rows(&similar))
}

/// Whether `m` takes the pair's first point within `agree` pixels of its
/// second.
#[must_use]
pub fn within(m: &Mat3, p: &Pair, agree: f64) -> bool {
    apply(m, p.0).is_some_and(|q| (q[0] - p.1[0]).hypot(q[1] - p.1[1]) < agree)
}

/// The similarity that best explains `pairs`, robustly, and which pairs lie
/// within `close` pixels of it, the ones its last plain fit is made over.
/// `None` for fewer than three pairs or a degenerate spread.
#[must_use]
pub fn fit_similarity(pairs: &[Pair], close: f64) -> Option<([f64; 4], Vec<bool>)> {
    // Started from the middle slide: between two neighbouring frames the
    // picture mostly slides, so the median move is the picture's whenever more
    // than half the points are on it, and the first weights are already right.
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let dx = median(pairs.iter().map(|p| p.1[0] - p.0[0]).collect());
    let dy = median(pairs.iter().map(|p| p.1[1] - p.0[1]).collect());
    fit_similarity_from(pairs, [1.0, 0.0, dx, dy], close)
}

/// [`fit_similarity`] started from the similarity `start` rather than the
/// middle slide: for pairs most of which are known to move one way, so the
/// rest are talked down from the first weights on.
#[must_use]
pub fn fit_similarity_from(
    pairs: &[Pair],
    start: [f64; 4],
    close: f64,
) -> Option<([f64; 4], Vec<bool>)> {
    if pairs.len() < 3 {
        return None;
    }
    let f = Frame::of(pairs.iter().map(|p| p.0))?;
    let first = from_similarity(start);
    let mut w = weights(
        &pairs
            .iter()
            .map(|p| {
                apply(&first, p.0).map_or(f64::INFINITY, |q| (q[0] - p.1[0]).hypot(q[1] - p.1[1]))
            })
            .collect::<Vec<_>>(),
        floor(close),
    );
    let mut model = [1.0, 0.0, 0.0, 0.0];
    for _ in 0..8 {
        let mut n = Normal::<4>::new();
        for (p, &wi) in pairs.iter().zip(&w) {
            let [x, y] = f.to(p.0);
            let [u, v] = f.to(p.1);
            n.add(&[x, -y, 1.0, 0.0], u, wi);
            n.add(&[y, x, 0.0, 1.0], v, wi);
        }
        model = n.solve()?;
        let res: Vec<f64> = pairs
            .iter()
            .map(|p| {
                let [x, y] = f.to(p.0);
                let [u, v] = f.to(p.1);
                let (ex, ey) = (
                    model[0] * x - model[1] * y + model[2] - u,
                    model[1] * x + model[0] * y + model[3] - v,
                );
                ex.hypot(ey) / f.s
            })
            .collect();
        w = weights(&res, floor(close));
    }
    // A last plain fit over the pairs the robust one agrees with: a Cauchy
    // weight talks a crowd down but never quite to nothing, and the crowd's
    // faint pull is a few hundredths of a pixel on every frame.
    let near: Vec<f64> = pairs
        .iter()
        .map(|p| {
            let [x, y] = f.to(p.0);
            let [u, v] = f.to(p.1);
            let (ex, ey) = (
                model[0] * x - model[1] * y + model[2] - u,
                model[1] * x + model[0] * y + model[3] - v,
            );
            if ex.hypot(ey) / f.s < close {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    if near.iter().sum::<f64>() >= 3.0 {
        let mut n = Normal::<4>::new();
        for (p, &wi) in pairs.iter().zip(&near) {
            let [x, y] = f.to(p.0);
            let [u, v] = f.to(p.1);
            n.add(&[x, -y, 1.0, 0.0], u, wi);
            n.add(&[y, x, 0.0, 1.0], v, wi);
        }
        model = n.solve()?;
    }
    // Back out of the conditioning: x' = F⁻¹ S F x.
    let s = mul(
        &invert(&f.matrix())?,
        &mul(&from_similarity(model), &f.matrix()),
    );
    let sim = [s[0][0], s[1][0], s[0][2], s[1][2]];
    let agreed = pairs
        .iter()
        .map(|p| {
            apply(&from_similarity(sim), p.0)
                .is_some_and(|q| (q[0] - p.1[0]).hypot(q[1] - p.1[1]) < close)
        })
        .collect();
    Some((sim, agreed))
}

/// The homography that best explains `pairs`, robustly, started from `start`
/// (a similarity fit's matrix is a good start), and which pairs lie within
/// `close` pixels of it, the ones its last plain fit is made over. `None` for
/// fewer than eight pairs or a degenerate spread.
#[must_use]
pub fn fit_homography(pairs: &[Pair], start: &Mat3, close: f64) -> Option<([f64; 9], Vec<bool>)> {
    if pairs.len() < 8 {
        return None;
    }
    let fa = Frame::of(pairs.iter().map(|p| p.0))?;
    let fb = Frame::of(pairs.iter().map(|p| p.1))?;
    let residual = |h: &Mat3, p: &Pair| {
        apply(h, p.0).map_or(f64::INFINITY, |q| (q[0] - p.1[0]).hypot(q[1] - p.1[1]))
    };
    let mut w = weights(
        &pairs.iter().map(|p| residual(start, p)).collect::<Vec<_>>(),
        floor(close),
    );
    let mut h = *start;
    for _ in 0..8 {
        // Linearised in the eight free entries with the last fixed at one,
        // which a frame-to-frame motion is always far from violating.
        let mut n = Normal::<8>::new();
        for (p, &wi) in pairs.iter().zip(&w) {
            let [x, y] = fa.to(p.0);
            let [u, v] = fb.to(p.1);
            n.add(&[x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u, wi);
            n.add(&[0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v, wi);
        }
        let k = n.solve()?;
        let hn = [[k[0], k[1], k[2]], [k[3], k[4], k[5]], [k[6], k[7], 1.0]];
        h = mul(&invert(&fb.matrix())?, &mul(&hn, &fa.matrix()));
        let res: Vec<f64> = pairs.iter().map(|p| residual(&h, p)).collect();
        w = weights(&res, floor(close));
    }
    // The same last plain fit over the agreeing pairs as the similarity's.
    let near: Vec<f64> = pairs
        .iter()
        .map(|p| f64::from(u8::from(residual(&h, p) < close)))
        .collect();
    if near.iter().sum::<f64>() >= 8.0 {
        let mut n = Normal::<8>::new();
        for (p, &wi) in pairs.iter().zip(&near) {
            let [x, y] = fa.to(p.0);
            let [u, v] = fb.to(p.1);
            n.add(&[x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u, wi);
            n.add(&[0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v, wi);
        }
        if let Some(k) = n.solve() {
            let hn = [[k[0], k[1], k[2]], [k[3], k[4], k[5]], [k[6], k[7], 1.0]];
            h = mul(&invert(&fb.matrix())?, &mul(&hn, &fa.matrix()));
        }
    }
    let agreed = pairs.iter().map(|p| residual(&h, p) < close).collect();
    Some((to_rows(&h), agreed))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_crowd_moving_its_own_way_does_not_pull_the_fit() {
        // Two hundred points on the picture, slid and turned a little, and sixty
        // on something crossing it the other way.
        let truth = [0.999_8f64, 0.02, 5.0, -3.0];
        let mut pairs = Vec::new();
        for i in 0..200 {
            let p = [
                f64::from(i % 20) * 90.0 + 30.0,
                f64::from(i / 20) * 100.0 + 20.0,
            ];
            let q = apply(&from_similarity(truth), p).unwrap();
            pairs.push((p, q));
        }
        for i in 0..60 {
            let p = [
                800.0 + f64::from(i % 6) * 10.0,
                400.0 + f64::from(i / 6) * 10.0,
            ];
            pairs.push((p, [p[0] - 40.0, p[1] + 12.0]));
        }
        let (s, agree) = fit_similarity(&pairs, 2.0).unwrap();
        for (a, b) in s.iter().zip(truth) {
            assert!((a - b).abs() < 1e-3, "{s:?} against {truth:?}");
        }
        assert!(agree[..200].iter().all(|&a| a));
        assert!(agree[200..].iter().all(|&a| !a));
        let (h, _) = fit_homography(&pairs, &from_similarity(s), 2.0).unwrap();
        let m = from_rows(h);
        let q = apply(&m, [500.0, 500.0]).unwrap();
        let t = apply(&from_similarity(truth), [500.0, 500.0]).unwrap();
        assert!((q[0] - t[0]).hypot(q[1] - t[1]) < 0.01);
    }
}

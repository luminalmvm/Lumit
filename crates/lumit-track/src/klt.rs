//! Pyramidal affine Kanade–Lucas–Tomasi tracking (docs/impl/tracking.md §2).
//!
//! # In plain terms
//!
//! Take a small square of the previous frame around a feature — the *template*.
//! Ask: where in the new frame do I have to put that square, and how do I have
//! to stretch it, so the pixels match? "Where" is two numbers (a shift) and "how
//! stretched" is four more (a 2×2 matrix), and together they are the six numbers
//! this file solves for. Six rather than two matters because a zoom or a rotate
//! does not merely move a patch, it changes its shape — a tracker that can only
//! shift its square loses the feature the moment the lens moves, which is
//! exactly the failure K-415 exists to avoid.
//!
//! The solve is *inverse compositional* (Baker & Matthews): the expensive part
//! of the arithmetic — the template's gradients and the 6×6 matrix built from
//! them — depends only on the template, so it is computed once per pyramid level
//! and reused by every iteration. Each iteration then only has to resample the
//! new frame, work out how wrong it is, and compose a small correction onto the
//! warp it already has.

use crate::pyramid::{Plane, Pyramid};

/// The result of one frame-to-frame track: where the feature landed, and the
/// linear part of the warp that took it there.
pub(crate) struct Step {
    pub(crate) pos: [f64; 2],
    /// The 2×2 `A` of the affine warp (docs/impl/tracking.md §2's `(A, d)`).
    /// Stored per step because the phase-2 zoom-burst detector reads
    /// `log(scale)` out of it.
    pub(crate) a: [[f64; 2]; 2],
}

/// Solve the warp taking the patch at `from` in `prev` to its match in `cur`.
///
/// `seed` is the initial displacement guess in level-0 pixels (the
/// constant-velocity prior, or a flow vector where one was handed in). `half` is
/// the window half-width, so the window is `2·half + 1` on a side.
///
/// `None` means the track failed: the patch left the frame at every usable
/// level, the normal matrix was singular, or the affine part ran away.
pub(crate) fn solve(
    prev: &Pyramid,
    cur: &Pyramid,
    from: [f64; 2],
    seed: [f64; 2],
    half: i64,
    max_iters: usize,
    eps: f64,
) -> Option<Step> {
    let mut m = [[1.0f64, 0.0], [0.0, 1.0]];
    let mut d = seed;
    let mut solved_any = false;
    let hf = half as f64;

    for lvl in (0..prev.levels.len().min(cur.levels.len())).rev() {
        let scale = (1usize << lvl) as f64;
        let tpl = &prev.levels[lvl];
        let img = &cur.levels[lvl];
        let c = [from[0] / scale, from[1] / scale];
        // The template needs one pixel beyond the window for its central
        // differences. A level too coarse to hold the window is skipped, not
        // failed — that is what keeps features near the frame edge alive.
        if !tpl.inside(c[0], c[1], hf + 2.0) {
            continue;
        }
        let (t, sd, hess) = template(tpl, c, half);
        let mut dl = [d[0] / scale, d[1] / scale];

        for _ in 0..max_iters {
            if !patch_fits(img, c, dl, &m, hf) {
                break; // drifted off this level: keep what we have and stop
            }
            let mut b = [0.0f64; 6];
            let mut k = 0usize;
            for dy in -half..=half {
                for dx in -half..=half {
                    let (x, y) = (dx as f64, dy as f64);
                    let px = c[0] + dl[0] + m[0][0] * x + m[0][1] * y;
                    let py = c[1] + dl[1] + m[1][0] * x + m[1][1] * y;
                    let e = img.sample(px, py) - t[k];
                    for i in 0..6 {
                        b[i] += sd[k][i] * e;
                    }
                    k += 1;
                }
            }
            let dp = solve6(&hess, &b)?;
            // Compose the inverse of the incremental warp onto the current one
            // (Baker & Matthews' update rule): W ← W ∘ ΔW⁻¹.
            let dm = [[1.0 + dp[0], dp[2]], [dp[1], 1.0 + dp[3]]];
            let ddet = dm[0][0] * dm[1][1] - dm[0][1] * dm[1][0];
            if !ddet.is_finite() || ddet.abs() < 1e-9 {
                return None;
            }
            let dinv = [
                [dm[1][1] / ddet, -dm[0][1] / ddet],
                [-dm[1][0] / ddet, dm[0][0] / ddet],
            ];
            let nm = mul(&m, &dinv);
            let dt = [dp[4], dp[5]];
            // The warp's translation is the patch centre, so the update lands
            // on `dl` once the level's own centre is taken back off.
            dl = [
                dl[0] - (nm[0][0] * dt[0] + nm[0][1] * dt[1]),
                dl[1] - (nm[1][0] * dt[0] + nm[1][1] * dt[1]),
            ];
            m = nm;
            if !sane(&m) || !dl[0].is_finite() || !dl[1].is_finite() {
                return None;
            }
            // Converged when the patch's corners stopped moving: the shift plus
            // the shape change measured at the window's edge.
            let moved =
                dt[0].hypot(dt[1]) + hf * (dp[0].abs() + dp[1].abs() + dp[2].abs() + dp[3].abs());
            if moved < eps {
                break;
            }
        }
        d = [dl[0] * scale, dl[1] * scale];
        solved_any = true;
    }

    if !solved_any {
        return None;
    }
    Some(Step {
        pos: [from[0] + d[0], from[1] + d[1]],
        a: m,
    })
}

/// The template's values, steepest-descent images and normal matrix — the
/// per-level constants of the inverse-compositional solve.
fn template(tpl: &Plane, c: [f64; 2], half: i64) -> (Vec<f64>, Vec<[f64; 6]>, [[f64; 6]; 6]) {
    let n = ((2 * half + 1) * (2 * half + 1)) as usize;
    let mut t = Vec::with_capacity(n);
    let mut sd = Vec::with_capacity(n);
    let mut hess = [[0.0f64; 6]; 6];
    for dy in -half..=half {
        for dx in -half..=half {
            let (x, y) = (dx as f64, dy as f64);
            let (px, py) = (c[0] + x, c[1] + y);
            t.push(tpl.sample(px, py));
            // Central differences of the bilinear image: the gradient the warp
            // Jacobian is multiplied by. Sub-pixel centres are the rule here,
            // not the exception, so a precomputed integer-grid gradient plane
            // would be the wrong gradient for most templates.
            let gx = 0.5 * (tpl.sample(px + 1.0, py) - tpl.sample(px - 1.0, py));
            let gy = 0.5 * (tpl.sample(px, py + 1.0) - tpl.sample(px, py - 1.0));
            // ∇T · ∂W/∂p for W(x; p) = [[1+p₁, p₃], [p₂, 1+p₄]]·x + (p₅, p₆).
            let row = [gx * x, gy * x, gx * y, gy * y, gx, gy];
            for i in 0..6 {
                for j in 0..6 {
                    hess[i][j] += row[i] * row[j];
                }
            }
            sd.push(row);
        }
    }
    (t, sd, hess)
}

/// Whether all four corners of the warped window still sit inside `img`.
fn patch_fits(img: &Plane, c: [f64; 2], d: [f64; 2], m: &[[f64; 2]; 2], hf: f64) -> bool {
    for (x, y) in [(-hf, -hf), (hf, -hf), (-hf, hf), (hf, hf)] {
        let px = c[0] + d[0] + m[0][0] * x + m[0][1] * y;
        let py = c[1] + d[1] + m[1][0] * x + m[1][1] * y;
        if !img.inside(px, py, 1.0) {
            return false;
        }
    }
    true
}

/// A guard against the affine part running away: a patch that claims to have
/// grown fourfold or flipped over between two frames has lost its feature.
fn sane(m: &[[f64; 2]; 2]) -> bool {
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    det.is_finite() && (0.25..=4.0).contains(&det)
}

fn mul(a: &[[f64; 2]; 2], b: &[[f64; 2]; 2]) -> [[f64; 2]; 2] {
    [
        [
            a[0][0] * b[0][0] + a[0][1] * b[1][0],
            a[0][0] * b[0][1] + a[0][1] * b[1][1],
        ],
        [
            a[1][0] * b[0][0] + a[1][1] * b[1][0],
            a[1][0] * b[0][1] + a[1][1] * b[1][1],
        ],
    ]
}

/// Solve `H · x = b` by Gaussian elimination with partial pivoting. Fixed
/// pivoting order, so the answer is the same bits on every run (§1's
/// determinism rule). `None` where `H` is singular — a textureless or
/// one-dimensional patch, which must end the track rather than be inverted.
fn solve6(h: &[[f64; 6]; 6], b: &[f64; 6]) -> Option<[f64; 6]> {
    let mut a = [[0.0f64; 7]; 6];
    for i in 0..6 {
        a[i][..6].copy_from_slice(&h[i]);
        a[i][6] = b[i];
    }
    for col in 0..6 {
        let mut pivot = col;
        for row in (col + 1)..6 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        for row in (col + 1)..6 {
            let f = a[row][col] / a[col][col];
            let pivot_row = a[col];
            for (k, v) in a[row].iter_mut().enumerate().skip(col) {
                *v -= f * pivot_row[k];
            }
        }
    }
    let mut x = [0.0f64; 6];
    for i in (0..6).rev() {
        let mut s = a[i][6];
        for j in (i + 1)..6 {
            s -= a[i][j] * x[j];
        }
        x[i] = s / a[i][i];
        if !x[i].is_finite() {
            return None;
        }
    }
    Some(x)
}

/// Sample the `2·half + 1` square window of `p` centred on `c`. `None` where the
/// window would leave the plane.
pub(crate) fn patch(p: &Plane, c: [f64; 2], half: i64) -> Option<Vec<f64>> {
    let hf = half as f64;
    if !p.inside(c[0], c[1], hf) {
        return None;
    }
    let n = ((2 * half + 1) * (2 * half + 1)) as usize;
    let mut out = Vec::with_capacity(n);
    for dy in -half..=half {
        for dx in -half..=half {
            out.push(p.sample(c[0] + dx as f64, c[1] + dy as f64));
        }
    }
    Some(out)
}

/// Zero-mean normalised cross-correlation of two equal-length patches, in
/// −1..=1. Zero-mean and normalised so a brightness or contrast change between
/// the reference and now is not read as the feature having been lost.
///
/// A flat patch (no variance at all) scores 0: nothing to correlate is not the
/// same as a match, and treating it as one is how a track slides into a dark
/// corner and stays there.
pub(crate) fn ncc(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let n = a.len() as f64;
    let ma = a.iter().sum::<f64>() / n;
    let mb = b.iter().sum::<f64>() / n;
    let (mut num, mut da, mut db) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b.iter()) {
        let (u, v) = (x - ma, y - mb);
        num += u * v;
        da += u * u;
        db += v * v;
    }
    let den = (da * db).sqrt();
    if den < 1e-12 {
        0.0
    } else {
        (num / den).clamp(-1.0, 1.0)
    }
}

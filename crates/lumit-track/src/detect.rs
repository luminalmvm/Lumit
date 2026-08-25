//! Shi–Tomasi feature detection on a bucketed grid (docs/impl/tracking.md §2).
//!
//! # In plain terms
//!
//! A patch of sky can be matched to any other patch of sky, so it is useless to
//! track. A straight edge is only half useful: you can tell how far it moved
//! across itself, never along itself (the "aperture problem"). What is worth
//! tracking is a spot that pins down *both* directions — a corner, a speck, the
//! meeting of two edges. Shi–Tomasi measures exactly that: it looks at how the
//! brightness changes across a little window in every direction and takes the
//! *weakest* of those changes as the score. Weak in every direction means flat;
//! weak in one direction means an edge; strong even at its weakest means a
//! corner.
//!
//! Scores alone would pile every feature onto the one high-contrast object in
//! the shot and leave the rest of the frame bare, and a camera solve needs
//! features spread across the picture. So the frame is cut into a grid of
//! buckets and each bucket keeps its own best few.

use crate::exclude::{excluded, ExclusionMask};
use crate::pyramid::Plane;

/// The three frame-sized buffers the response pass needs, owned by the
/// [`Tracker`](crate::Tracker) and reused for its whole life.
///
/// Re-detection runs on most frames, and allocating a frame's worth of `f32`
/// three times over per frame is exactly the churn 14-ENGINEERING-RULES §5
/// forbids. This crate cannot reach `lumit-media`'s frame arena — an engine
/// crate does not depend on the media layer to borrow a buffer — so it does what
/// the pyramids do: allocate once for a given raster, then overwrite.
#[derive(Default)]
pub(crate) struct Scratch {
    pub(crate) resp: Vec<f32>,
    gx: Vec<f32>,
    gy: Vec<f32>,
}

/// Resize to `n` zeroes, keeping the capacity already paid for.
fn fit(v: &mut Vec<f32>, n: usize) {
    v.clear();
    v.resize(n, 0.0);
}

/// Write the frame's Shi–Tomasi min-eigenvalue response into `s.resp`, one value
/// per pixel.
///
/// `radius` is the half-width of the window the gradient normal matrix is summed
/// over. Borders are clamped, so a response exists everywhere and the caller's
/// margin — not a special case here — decides where features may sit.
pub(crate) fn response_map_into(s: &mut Scratch, p: &Plane, radius: usize) {
    let (w, h) = (p.w, p.h);
    let n = w * h;
    fit(&mut s.resp, n);
    if w < 3 || h < 3 {
        return;
    }
    // Sobel gradients, normalised to intensity-per-pixel — the same ÷8 scaling
    // `lumit-flow` uses, so the two crates' "how much contrast is here" numbers
    // are on one scale.
    fit(&mut s.gx, n);
    fit(&mut s.gy, n);
    let (out, gx, gy) = (&mut s.resp, &mut s.gx, &mut s.gy);
    for y in 0..h {
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(h - 1);
            let at = |px: usize, py: usize| p.data[py * w + px];
            let (tl, t, tr) = (at(xm, ym), at(x, ym), at(xp, ym));
            let (l, r) = (at(xm, y), at(xp, y));
            let (bl, b, br) = (at(xm, yp), at(x, yp), at(xp, yp));
            gx[y * w + x] = ((tr + 2.0 * r + br) - (tl + 2.0 * l + bl)) / 8.0;
            gy[y * w + x] = ((bl + 2.0 * b + br) - (tl + 2.0 * t + tr)) / 8.0;
        }
    }
    let r = radius as i64;
    for y in 0..h {
        for x in 0..w {
            let (mut sxx, mut sxy, mut syy) = (0.0f64, 0.0f64, 0.0f64);
            for oy in -r..=r {
                let qy = (y as i64 + oy).clamp(0, h as i64 - 1) as usize;
                for ox in -r..=r {
                    let qx = (x as i64 + ox).clamp(0, w as i64 - 1) as usize;
                    let q = qy * w + qx;
                    let (a, b) = (f64::from(gx[q]), f64::from(gy[q]));
                    sxx += a * a;
                    sxy += a * b;
                    syy += b * b;
                }
            }
            // Smaller eigenvalue of [[sxx, sxy], [sxy, syy]].
            let mid = 0.5 * (sxx + syy);
            let half_diff = 0.5 * (sxx - syy);
            let lo = mid - (half_diff * half_diff + sxy * sxy).sqrt();
            out[y * w + x] = lo.max(0.0) as f32;
        }
    }
}

/// The detection grid: the frame cut into `gx × gy` buckets
/// (docs/impl/tracking.md §2's 16×16).
#[derive(Clone, Copy)]
pub(crate) struct BucketGrid {
    pub(crate) gx: usize,
    pub(crate) gy: usize,
    pub(crate) w: usize,
    pub(crate) h: usize,
}

impl BucketGrid {
    pub(crate) fn count(&self) -> usize {
        self.gx * self.gy
    }

    /// The bucket a source-raster position falls in, row-major.
    pub(crate) fn index_of(&self, x: f64, y: f64) -> Option<usize> {
        if !(x >= 0.0 && y >= 0.0 && x < self.w as f64 && y < self.h as f64) {
            return None;
        }
        let bx = ((x as usize) * self.gx / self.w.max(1)).min(self.gx - 1);
        let by = ((y as usize) * self.gy / self.h.max(1)).min(self.gy - 1);
        Some(by * self.gx + bx)
    }

    /// The half-open pixel range a bucket column covers. Derived from
    /// [`Self::index_of`]'s integer division so the two can never disagree.
    fn span(i: usize, n: usize, dim: usize) -> (usize, usize) {
        let lo = (i * dim).div_ceil(n);
        let hi = ((i + 1) * dim).div_ceil(n).min(dim);
        (lo, hi.max(lo))
    }
}

/// Detect up to `want` new features in each named bucket.
///
/// Deterministic ordering, exactly as docs/impl/tracking.md §2 pins it: buckets
/// row-major (`need` is walked in the order given, which the caller keeps
/// ascending), then response descending, ties by `(y, x)`. `floor` is the
/// absolute response cut-off the caller derived from the frame's best.
///
/// `occupied` are the positions of tracks already live; a new feature is kept
/// `min_sep` pixels clear of them and of its own siblings, because "best-N in a
/// bucket" without that lands all N on the same corner.
#[allow(clippy::too_many_arguments)]
pub(crate) fn detect(
    resp: &[f32],
    grid: &BucketGrid,
    need: &[(usize, usize)],
    floor: f32,
    margin: usize,
    min_sep: f64,
    occupied: &[[f64; 2]],
    masks: &[ExclusionMask],
) -> Vec<[f64; 2]> {
    let (w, h) = (grid.w, grid.h);
    let mut out: Vec<[f64; 2]> = Vec::new();
    let sep2 = min_sep * min_sep;
    let mut candidates: Vec<(f32, usize, usize)> = Vec::new();
    for &(bucket, want) in need {
        if want == 0 || bucket >= grid.count() {
            continue;
        }
        let (bx, by) = (bucket % grid.gx, bucket / grid.gx);
        let (x0, x1) = BucketGrid::span(bx, grid.gx, w);
        let (y0, y1) = BucketGrid::span(by, grid.gy, h);
        candidates.clear();
        for y in y0.max(margin)..y1.min(h.saturating_sub(margin)) {
            for x in x0.max(margin)..x1.min(w.saturating_sub(margin)) {
                let v = resp[y * w + x];
                if v >= floor && v > 0.0 {
                    candidates.push((v, y, x));
                }
            }
        }
        // Stable sort on response alone: the scan above emitted candidates in
        // (y, x) order, so equal responses keep it. That is the tie rule.
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut taken = 0usize;
        for &(_, y, x) in &candidates {
            if taken >= want {
                break;
            }
            let (fx, fy) = (x as f64, y as f64);
            if excluded(masks, fx, fy) {
                continue;
            }
            let clash = occupied
                .iter()
                .chain(out.iter())
                .any(|p| (p[0] - fx).powi(2) + (p[1] - fy).powi(2) < sep2);
            if clash {
                continue;
            }
            out.push([fx, fy]);
            taken += 1;
        }
    }
    out
}

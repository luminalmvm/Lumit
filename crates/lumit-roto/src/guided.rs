//! The guided filter (He, Sun & Tang, ECCV 2010) — the refine-edge pass
//! (docs/impl/roto.md §4, pinned).
//!
//! In plain terms: inside every small window, assume the matte is some fixed
//! recipe of the frame's own colours — so much red plus so much green plus so
//! much blue, plus an offset. Work out the recipe that best matches the
//! segmentation's answer in that window, then read the matte back out of the
//! colours through it. Where the true edge is soft — hair, motion blur, smoke —
//! the colours are a mixture and the recipe returns the mixture's proportion,
//! which is exactly the alpha that was wanted. Where the edge is hard, the
//! colours are not mixed and the answer stays hard.
//!
//! Every average is a box sum over running totals in a fixed order: O(N)
//! exactly, no iteration, no solver, deterministic.

use crate::FrameRgb;

/// Below this the 3×3 colour covariance is treated as unsolvable and the
/// window falls back to its plain mean — with the regulariser on the diagonal
/// it takes a degenerate window to get here, and a silent NaN would poison the
/// whole matte.
const MIN_DET: f32 = 1e-12;

/// The filter's working set: fifteen frame-sized planes, allocated once and
/// reused for every frame of a propagation run (14 §5). At 1080p that is about
/// 125 MB of f32 — the price of the colour guided filter, which needs every
/// window's colour mean, its 3×3 colour covariance and its cross-covariance
/// with the matte all live at the same moment.
#[derive(Debug, Default)]
pub(crate) struct Guided {
    len: usize,
    mi_r: Vec<f32>,
    mi_g: Vec<f32>,
    mi_b: Vec<f32>,
    mp: Vec<f32>,
    mip_r: Vec<f32>,
    mip_g: Vec<f32>,
    mip_b: Vec<f32>,
    s_rr: Vec<f32>,
    s_rg: Vec<f32>,
    s_rb: Vec<f32>,
    s_gg: Vec<f32>,
    s_gb: Vec<f32>,
    s_bb: Vec<f32>,
    src: Vec<f32>,
    tmp: Vec<f32>,
}

impl Guided {
    /// Size the working set to a frame, keeping the allocation when the size
    /// has not changed.
    pub(crate) fn resize(&mut self, len: usize) {
        if self.len == len {
            return;
        }
        self.len = len;
        for plane in [
            &mut self.mi_r,
            &mut self.mi_g,
            &mut self.mi_b,
            &mut self.mp,
            &mut self.mip_r,
            &mut self.mip_g,
            &mut self.mip_b,
            &mut self.s_rr,
            &mut self.s_rg,
            &mut self.s_rb,
            &mut self.s_gg,
            &mut self.s_gb,
            &mut self.s_bb,
            &mut self.src,
            &mut self.tmp,
        ] {
            plane.clear();
            plane.resize(len, 0.0);
        }
    }

    /// Filter `p` (the segmentation's matte) with the frame as guide, writing
    /// the result to `out`.
    ///
    /// Does nothing if the working set has not been sized to this frame — the
    /// one check that makes every index below provably in range.
    pub(crate) fn filter(
        &mut self,
        frame: FrameRgb<'_>,
        p: &[f32],
        radius: u32,
        eps: f32,
        out: &mut [f32],
    ) {
        let n = self.len;
        let w = frame.width();
        let h = frame.height();
        if n == 0 || self.mi_r.len() != n || p.len() < n || out.len() < n {
            return;
        }

        // Every box mean the local linear model needs, one plane at a time
        // through `src` so the working set stays at two scratch planes.
        stage(&mut self.src, |i| frame.px(i)[0]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mi_r);
        stage(&mut self.src, |i| frame.px(i)[1]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mi_g);
        stage(&mut self.src, |i| frame.px(i)[2]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mi_b);
        stage(&mut self.src, |i| p[i]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mp);
        stage(&mut self.src, |i| frame.px(i)[0] * p[i]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mip_r);
        stage(&mut self.src, |i| frame.px(i)[1] * p[i]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mip_g);
        stage(&mut self.src, |i| frame.px(i)[2] * p[i]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.mip_b);
        stage(&mut self.src, |i| frame.px(i)[0] * frame.px(i)[0]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.s_rr);
        stage(&mut self.src, |i| frame.px(i)[0] * frame.px(i)[1]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.s_rg);
        stage(&mut self.src, |i| frame.px(i)[0] * frame.px(i)[2]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.s_rb);
        stage(&mut self.src, |i| frame.px(i)[1] * frame.px(i)[1]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.s_gg);
        stage(&mut self.src, |i| frame.px(i)[1] * frame.px(i)[2]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.s_gb);
        stage(&mut self.src, |i| frame.px(i)[2] * frame.px(i)[2]);
        box_mean(&self.src, w, h, radius, &mut self.tmp, &mut self.s_bb);

        // The per-window recipe. `a` overwrites the cross-covariance planes and
        // `b` overwrites the matte mean: both are finished with by the time
        // they are read here, and reusing them keeps the working set at fifteen
        // planes rather than nineteen.
        //
        // ponytail: the ceiling here is **local-linear matting** — this is the
        // fast approximation to closed-form matting, not the matting Laplacian,
        // so long translucent strands crossing a busy background come back
        // muddier than a learned matting head manages. The upgrade path is that
        // head on the §9 seed seam, not a global Laplacian solve; O(N) is the
        // budget's load-bearing fact. Observable trigger: a refine band over
        // hair against a similarly-toned background returning a grey wash
        // instead of separated strands.
        for i in 0..n {
            let mr = self.mi_r[i];
            let mg = self.mi_g[i];
            let mb = self.mi_b[i];
            let mp = self.mp[i];
            let cov_r = self.mip_r[i] - mr * mp;
            let cov_g = self.mip_g[i] - mg * mp;
            let cov_b = self.mip_b[i] - mb * mp;
            let rr = self.s_rr[i] - mr * mr + eps;
            let rg = self.s_rg[i] - mr * mg;
            let rb = self.s_rb[i] - mr * mb;
            let gg = self.s_gg[i] - mg * mg + eps;
            let gb = self.s_gb[i] - mg * mb;
            let bb = self.s_bb[i] - mb * mb + eps;
            // The inverse of a symmetric 3×3 by its adjugate — three cofactors,
            // one determinant, no library and no iteration.
            let c00 = gg * bb - gb * gb;
            let c01 = rb * gb - rg * bb;
            let c02 = rg * gb - rb * gg;
            let det = rr * c00 + rg * c01 + rb * c02;
            let (a_r, a_g, a_b) = if det.abs() > MIN_DET {
                let inv = 1.0 / det;
                let c11 = rr * bb - rb * rb;
                let c12 = rb * rg - rr * gb;
                let c22 = rr * gg - rg * rg;
                (
                    (c00 * cov_r + c01 * cov_g + c02 * cov_b) * inv,
                    (c01 * cov_r + c11 * cov_g + c12 * cov_b) * inv,
                    (c02 * cov_r + c12 * cov_g + c22 * cov_b) * inv,
                )
            } else {
                (0.0, 0.0, 0.0)
            };
            self.mip_r[i] = a_r;
            self.mip_g[i] = a_g;
            self.mip_b[i] = a_b;
            self.mp[i] = mp - a_r * mr - a_g * mg - a_b * mb;
        }

        // Average the recipe over the windows a pixel belongs to, then read the
        // matte back out of the guide through it.
        box_mean(&self.mip_r, w, h, radius, &mut self.tmp, &mut self.s_rr);
        box_mean(&self.mip_g, w, h, radius, &mut self.tmp, &mut self.s_rg);
        box_mean(&self.mip_b, w, h, radius, &mut self.tmp, &mut self.s_rb);
        box_mean(&self.mp, w, h, radius, &mut self.tmp, &mut self.s_gg);
        for (i, slot) in out.iter_mut().enumerate().take(n) {
            let c = frame.px(i);
            *slot = self.s_rr[i] * c[0] + self.s_rg[i] * c[1] + self.s_rb[i] * c[2] + self.s_gg[i];
        }
    }
}

/// Fill a scratch plane from a per-pixel expression.
fn stage<F: FnMut(usize) -> f32>(src: &mut [f32], mut f: F) {
    for (i, slot) in src.iter_mut().enumerate() {
        *slot = f(i);
    }
}

/// The mean over the `(2r+1)²` window round each pixel, clipped at the frame's
/// edge and divided by however many pixels the clipped window actually holds.
///
/// Sums are separable and means are not, so both passes accumulate **sums** and
/// the division by the window's true area happens once at the end. Running
/// totals slide across each row and column in a fixed order; the totals carry
/// in f64 so a long row of additions and subtractions does not fray.
pub(crate) fn box_mean(src: &[f32], w: u32, h: u32, radius: u32, tmp: &mut [f32], dst: &mut [f32]) {
    let (w, h, r) = (w as usize, h as usize, radius as usize);
    if w == 0 || h == 0 {
        return;
    }
    // Horizontal sums.
    for y in 0..h {
        let row = y * w;
        let mut sum = 0.0f64;
        for x in 0..=r.min(w - 1) {
            sum += f64::from(src.get(row + x).copied().unwrap_or(0.0));
        }
        for x in 0..w {
            if x > 0 {
                if x + r < w {
                    sum += f64::from(src.get(row + x + r).copied().unwrap_or(0.0));
                }
                if x > r {
                    sum -= f64::from(src.get(row + x - r - 1).copied().unwrap_or(0.0));
                }
            }
            if let Some(slot) = tmp.get_mut(row + x) {
                *slot = sum as f32;
            }
        }
    }
    // Vertical sums, then the division by the clipped window's area.
    for x in 0..w {
        let mut sum = 0.0f64;
        for y in 0..=r.min(h - 1) {
            sum += f64::from(tmp.get(y * w + x).copied().unwrap_or(0.0));
        }
        let count_x = (x + r).min(w - 1) - x.saturating_sub(r) + 1;
        for y in 0..h {
            if y > 0 {
                if y + r < h {
                    sum += f64::from(tmp.get((y + r) * w + x).copied().unwrap_or(0.0));
                }
                if y > r {
                    sum -= f64::from(tmp.get((y - r - 1) * w + x).copied().unwrap_or(0.0));
                }
            }
            let count_y = (y + r).min(h - 1) - y.saturating_sub(r) + 1;
            let area = (count_x * count_y) as f64;
            if let Some(slot) = dst.get_mut(y * w + x) {
                *slot = if area > 0.0 { (sum / area) as f32 } else { 0.0 };
            }
        }
    }
}

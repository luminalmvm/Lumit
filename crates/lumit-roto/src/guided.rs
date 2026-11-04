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

/// The filter's working set: thirteen frame-sized planes, allocated once and
/// reused for every frame of a propagation run (14 §5). At 1080p that is about
/// 110 MB of f32 — the price of the colour guided filter, which needs every
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
    boxes: BoxScratch,
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

        // Every box mean the local linear model needs, each read straight off
        // the frame and the matte.
        box_mean(w, h, radius, &mut self.boxes, &mut self.mi_r, |i| {
            frame.px(i)[0]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.mi_g, |i| {
            frame.px(i)[1]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.mi_b, |i| {
            frame.px(i)[2]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.mp, |i| p[i]);
        box_mean(w, h, radius, &mut self.boxes, &mut self.mip_r, |i| {
            frame.px(i)[0] * p[i]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.mip_g, |i| {
            frame.px(i)[1] * p[i]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.mip_b, |i| {
            frame.px(i)[2] * p[i]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_rr, |i| {
            frame.px(i)[0] * frame.px(i)[0]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_rg, |i| {
            frame.px(i)[0] * frame.px(i)[1]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_rb, |i| {
            frame.px(i)[0] * frame.px(i)[2]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_gg, |i| {
            frame.px(i)[1] * frame.px(i)[1]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_gb, |i| {
            frame.px(i)[1] * frame.px(i)[2]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_bb, |i| {
            frame.px(i)[2] * frame.px(i)[2]
        });

        // The per-window recipe. `a` overwrites the cross-covariance planes and
        // `b` overwrites the matte mean: both are finished with by the time
        // they are read here, and reusing them keeps the working set at thirteen
        // planes rather than seventeen.
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
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_rr, |i| {
            self.mip_r[i]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_rg, |i| {
            self.mip_g[i]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_rb, |i| {
            self.mip_b[i]
        });
        box_mean(w, h, radius, &mut self.boxes, &mut self.s_gg, |i| {
            self.mp[i]
        });
        for (i, slot) in out.iter_mut().enumerate().take(n) {
            let c = frame.px(i);
            *slot = self.s_rr[i] * c[0] + self.s_rg[i] * c[1] + self.s_rb[i] * c[2] + self.s_gg[i];
        }
    }
}

/// What a box mean works through: one row of the source, the row sums of the
/// last `2r + 2` rows in a ring, and one running total per column. A few
/// rows rather than whole planes, so the passes stay in cache. Sized on first
/// use and kept.
#[derive(Debug, Default)]
pub(crate) struct BoxScratch {
    line: Vec<f32>,
    rows: Vec<f32>,
    cols: Vec<f64>,
}

/// The mean over the `(2r+1)²` window round each pixel, clipped at the frame's
/// edge and divided by however many pixels the clipped window actually holds.
/// `src` gives the value at each flat index and is asked once per pixel.
///
/// Sums are separable and means are not, so both passes accumulate **sums** and
/// the division by the window's true area happens once at the end. Running
/// totals slide across each row and down each column in a fixed order; the
/// totals carry in f64 so a long row of additions and subtractions does not
/// fray. A row's sums are made just before the column totals take them in,
/// and dropped once they have been taken back out.
pub(crate) fn box_mean<F: Fn(usize) -> f32>(
    w: u32,
    h: u32,
    radius: u32,
    scratch: &mut BoxScratch,
    dst: &mut [f32],
    src: F,
) {
    let (w, h, r) = (w as usize, h as usize, radius as usize);
    if w == 0 || h == 0 {
        return;
    }
    // Rows `y - r - 1` to `y + r` are live at once: one leaving, the window,
    // and one arriving.
    let ring = 2 * r + 2;
    let BoxScratch { line, rows, cols } = scratch;
    line.clear();
    line.resize(w, 0.0);
    rows.clear();
    rows.resize(ring * w, 0.0);
    cols.clear();
    cols.resize(w, 0.0);

    // Horizontal sums of row `y` into its slot in the ring.
    let mut row_sums = |y: usize, rows: &mut [f32]| {
        for (x, slot) in line.iter_mut().enumerate() {
            *slot = src(y * w + x);
        }
        let Some(out) = rows.get_mut((y % ring) * w..(y % ring + 1) * w) else {
            return;
        };
        let mut sum = 0.0f64;
        for v in line.iter().take(r + 1) {
            sum += f64::from(*v);
        }
        for x in 0..w {
            if x > 0 {
                if x + r < w {
                    sum += f64::from(line[x + r]);
                }
                if x > r {
                    sum -= f64::from(line[x - r - 1]);
                }
            }
            out[x] = sum as f32;
        }
    };
    let held = |rows: &[f32], y: usize| -> std::ops::Range<usize> {
        let at = (y % ring) * w;
        at..(at + w).min(rows.len())
    };

    // Vertical sums, then the division by the clipped window's area.
    for y in 0..=r.min(h - 1) {
        row_sums(y, rows);
        for (sum, v) in cols.iter_mut().zip(&rows[held(rows, y)]) {
            *sum += f64::from(*v);
        }
    }
    for y in 0..h {
        if y > 0 {
            if y + r < h {
                row_sums(y + r, rows);
                for (sum, v) in cols.iter_mut().zip(&rows[held(rows, y + r)]) {
                    *sum += f64::from(*v);
                }
            }
            if y > r {
                for (sum, v) in cols.iter_mut().zip(&rows[held(rows, y - r - 1)]) {
                    *sum -= f64::from(*v);
                }
            }
        }
        let count_y = (y + r).min(h - 1) - y.saturating_sub(r) + 1;
        let Some(out) = dst.get_mut(y * w..(y + 1) * w) else {
            return;
        };
        for (x, (slot, sum)) in out.iter_mut().zip(cols.iter()).enumerate() {
            let count_x = (x + r).min(w - 1) - x.saturating_sub(r) + 1;
            let area = (count_x * count_y) as f64;
            *slot = if area > 0.0 { (sum / area) as f32 } else { 0.0 };
        }
    }
}

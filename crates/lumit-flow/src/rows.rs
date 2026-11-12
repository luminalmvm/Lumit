//! Per-pixel passes shared out across the cores, a row at a time: the luma a
//! measurement starts from, the pyramid step, and the confidence of a pair.
//! The crate root's functions of the same names come here.
//!
//! Each pixel of these passes is its own sum, so the answer is the same
//! however the rows are shared out. They run on the shared worker pool, as
//! every other pixel job does (docs/14 §1), from whichever thread asked.

use rayon::prelude::*;

use crate::{sample_scalar, FlowField, Gray, Texel, OCC_ABS, OCC_REL, VALID_DIM};

/// Run `each` over `data` a row of `len` at a time, across the cores, with the
/// row's index.
pub(crate) fn rows<T: Send>(data: &mut [T], len: usize, each: impl Fn(usize, &mut [T]) + Sync) {
    data.par_chunks_mut(len.max(1))
        .enumerate()
        .for_each(|(y, row)| each(y, row));
}

/// [`crate::to_gray_as`].
pub(crate) fn to_gray_as<T: Texel>(rgba: &[u8], w: usize, h: usize) -> Gray {
    let mut data = vec![0f32; w * h];
    let scale = if T::BYTES == 4 { 1.0 / 255.0 } else { 1.0 };
    rows(&mut data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let (r, g, b) = (T::get(rgba, i, 0), T::get(rgba, i, 1), T::get(rgba, i, 2));
            *px = (0.2126 * r + 0.7152 * g + 0.0722 * b) * scale;
        }
    });
    Gray { w, h, data }
}

/// [`crate::downsample`].
pub(crate) fn downsample(g: &Gray) -> Gray {
    let w = (g.w / 2).max(1);
    let h = (g.h / 2).max(1);
    let mut data = vec![0f32; w * h];
    rows(&mut data, w, |y, row| {
        let (y0, y1) = ((2 * y).min(g.h - 1), (2 * y + 1).min(g.h - 1));
        for (x, px) in row.iter_mut().enumerate() {
            let (x0, x1) = ((2 * x).min(g.w - 1), (2 * x + 1).min(g.w - 1));
            *px = 0.25 * (g.at(x0, y0) + g.at(x1, y0) + g.at(x0, y1) + g.at(x1, y1));
        }
    });
    Gray { w, h, data }
}

/// `raw` (`w × h`) under a 3×3 box blur with the edges held: what
/// [`crate::confidence`] does to [`agreement`], ramping the confidence over a
/// pixel so the streak-length taper has no visible seam.
pub(crate) fn blur3(raw: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; raw.len()];
    rows(&mut out, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let (mut acc, mut cnt) = (0f32, 0f32);
            for oy in -1i32..=1 {
                for ox in -1i32..=1 {
                    let qx = (x as i32 + ox).clamp(0, w as i32 - 1) as usize;
                    let qy = (y as i32 + oy).clamp(0, h as i32 - 1) as usize;
                    acc += raw[qy * w + qx];
                    cnt += 1.0;
                }
            }
            *px = acc / cnt;
        }
    });
    out
}

/// [`crate::agreement`].
pub(crate) fn agreement(f: &FlowField, g: &FlowField) -> Vec<f32> {
    let (w, h) = (f.w, f.h);
    let n = w * h;
    if g.w != w || g.h != h || f.u.len() != n || g.u.len() != n {
        return vec![1.0; n];
    }
    let mut raw = vec![0f32; n];
    rows(&mut raw, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let (fu, fv) = (f.u[i], f.v[i]);
            let gu = sample_scalar(&g.u, w, h, x as f32 + fu, y as f32 + fv);
            let gv = sample_scalar(&g.v, w, h, x as f32 + fu, y as f32 + fv);
            let cn = ((fu + gu) * (fu + gu) + (fv + gv) * (fv + gv)).sqrt();
            let fn_ = (fu * fu + fv * fv).sqrt();
            let gn = (gu * gu + gv * gv).sqrt();
            // Same rel/abs scale the occlusion cut-off uses (§2): cn == 0 → 1,
            // cn == thr → 0, linear and clamped between. Smooth, no step.
            let thr = (OCC_REL * (fn_ + gn)).max(OCC_ABS);
            let agree = (1.0 - cn / thr).clamp(0.0, 1.0);
            // Validity *dims* confidence rather than extinguishing it. FX-19's
            // whole point was that a hard cut-off shows as a hard edge between
            // blurred and unblurred; a binary term inside a smooth measure is
            // that cut-off wearing a disguise. An unexplained pixel whose two
            // directions still agree has a vector worth some of its streak.
            *px = agree * if f.valid[i] == 0 { VALID_DIM } else { 1.0 };
        }
    });
    raw
}

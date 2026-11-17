//! The temporal half: a neighbour frame moved along the measured motion so it
//! lines up with this one, then averaged in where it agrees.

use super::space::{pixel_to_ycc, tone};
use super::DenoiseParams;

/// A measured motion field at the size it was measured at: per pixel `u` and
/// `v` in its own pixels, and how far each vector is trusted, 0..1.
#[derive(Clone, Copy)]
pub struct Field<'a> {
    pub u: &'a [f32],
    pub v: &'a [f32],
    pub conf: &'a [f32],
    pub w: u32,
    pub h: u32,
}

impl Field<'_> {
    /// Bilinear `(u, v, conf)` at field pixel `(fx, fy)`, edges held.
    fn sample(&self, fx: f32, fy: f32) -> [f32; 3] {
        let n = (self.w as usize) * (self.h as usize);
        if self.w == 0 || self.h == 0 || self.u.len() < n || self.v.len() < n || self.conf.len() < n
        {
            return [0.0, 0.0, 0.0];
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let at = |x: i32, y: i32| {
            let x = x.clamp(0, self.w as i32 - 1) as usize;
            let y = y.clamp(0, self.h as i32 - 1) as usize;
            let i = y * self.w as usize + x;
            [self.u[i], self.v[i], self.conf[i]]
        };
        let (a, b, c, d) = (
            at(x0, y0),
            at(x0 + 1, y0),
            at(x0, y0 + 1),
            at(x0 + 1, y0 + 1),
        );
        let mut out = [0.0f32; 3];
        for (k, o) in out.iter_mut().enumerate() {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            *o = top + (bottom - top) * ty;
        }
        out
    }
}

/// The four Catmull-Rom weights at fraction `t`.
#[must_use]
pub fn catmull_rom(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// `src` (four lanes a pixel) at pixel position `(px, py)`, Catmull-Rom, held
/// inside the four texels around it so an edge does not ring.
#[must_use]
pub fn bicubic(src: &[f32], w: u32, h: u32, px: f32, py: f32) -> [f32; 4] {
    let (fx, fy) = (px.floor(), py.floor());
    let (wx, wy) = (catmull_rom(px - fx), catmull_rom(py - fy));
    let (ix, iy) = (fx as i32, fy as i32);
    let at = |x: i32, y: i32| {
        let x = x.clamp(0, w as i32 - 1) as usize;
        let y = y.clamp(0, h as i32 - 1) as usize;
        let i = (y * w as usize + x) * 4;
        [src[i], src[i + 1], src[i + 2], src[i + 3]]
    };
    let mut out = [0.0f32; 4];
    for (j, wyj) in wy.iter().enumerate() {
        let mut row = [0.0f32; 4];
        for (i, wxi) in wx.iter().enumerate() {
            let t = at(ix - 1 + i as i32, iy - 1 + j as i32);
            for c in 0..4 {
                row[c] += wxi * t[c];
            }
        }
        for c in 0..4 {
            out[c] += wyj * row[c];
        }
    }
    let inner = [
        at(ix, iy),
        at(ix + 1, iy),
        at(ix, iy + 1),
        at(ix + 1, iy + 1),
    ];
    for (c, o) in out.iter_mut().enumerate() {
        let lo = inner.iter().map(|t| t[c]).fold(f32::INFINITY, f32::min);
        let hi = inner.iter().map(|t| t[c]).fold(f32::NEG_INFINITY, f32::max);
        *o = o.clamp(lo, hi);
    }
    out
}

/// `neighbour` (premultiplied linear) moved onto this frame along `field`,
/// in Y'CbCr with the field's confidence in the fourth lane. With no field it
/// is read in place at full confidence, which is Follow motion off.
#[must_use]
pub fn warp(neighbour: &[f32], w: u32, h: u32, field: Option<Field<'_>>) -> Vec<f32> {
    let n = (w as usize) * (h as usize) * 4;
    if neighbour.len() != n || w == 0 || h == 0 {
        return vec![0.0; n];
    }
    let mut out = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let (px, py, conf) = match field {
                Some(f) if f.w > 0 && f.h > 0 => {
                    let (sx, sy) = (f.w as f32 / w as f32, f.h as f32 / h as f32);
                    let [u, v, c] =
                        f.sample((x as f32 + 0.5) * sx - 0.5, (y as f32 + 0.5) * sy - 0.5);
                    (x as f32 + u / sx, y as f32 + v / sy, c)
                }
                _ => (x as f32, y as f32, 1.0),
            };
            let [yy, cb, cr, _] = pixel_to_ycc(bicubic(neighbour, w, h, px, py));
            let at = ((y * w + x) * 4) as usize;
            out[at..at + 4].copy_from_slice(&[yy, cb, cr, conf]);
        }
    }
    out
}

/// The current frame alone: Y'CbCr and one frame blended.
#[must_use]
pub fn plain(current: &[f32]) -> Vec<f32> {
    current
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2], 1.0])
        .collect()
}

/// How far apart two 3x3 patches are in units of the noise: 1 is two pictures
/// of the same thing under independent noise.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn patch_distance(
    current: &[f32],
    other: &[f32],
    w: u32,
    h: u32,
    x: u32,
    y: u32,
    inv_luma: f32,
    inv_chroma: f32,
) -> f32 {
    let mut d = 0.0f32;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
            let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
            let i = (sy * w as usize + sx) * 4;
            let (dl, db, dr) = (
                current[i] - other[i],
                current[i + 1] - other[i + 1],
                current[i + 2] - other[i + 2],
            );
            d += dl * dl * inv_luma + (db * db + dr * dr) * inv_chroma;
        }
    }
    d / 27.0
}

/// The two noise reciprocals the patch distance divides by at a pixel whose
/// luma is `y`: `1 / (2 s^2)` for luma and for chroma.
#[must_use]
pub fn inverse_noise(y: f32, p: &DenoiseParams) -> (f32, f32) {
    let sl = p.luma_sigma * tone(y, p.tone);
    let sc = p.chroma_sigma;
    (
        1.0 / (2.0 * (sl * sl).max(1e-12)),
        1.0 / (2.0 * (sc * sc).max(1e-12)),
    )
}

/// The weight a patch distance `d` earns at full confidence.
#[must_use]
pub fn agreement(d: f32, p: &DenoiseParams) -> f32 {
    if p.strength <= 0.0 || p.t1 <= 0.0 {
        return 0.0;
    }
    let k = (1.0 - (d - p.t0) / p.t1).clamp(0.0, 1.0);
    k * k
}

/// `current` (Y'CbCr) with every aligned neighbour averaged in by how well it
/// agrees, in the order given. The fourth lane of the answer is how many
/// frames' worth of noise the average stands for, `(sum w)^2 / sum w^2`.
#[must_use]
pub fn accumulate(
    current: &[f32],
    aligned: &[&[f32]],
    w: u32,
    h: u32,
    p: &DenoiseParams,
) -> Vec<f32> {
    let n = (w as usize) * (h as usize) * 4;
    let usable: Vec<&[f32]> = aligned.iter().copied().filter(|a| a.len() == n).collect();
    if current.len() != n {
        return plain(current);
    }
    let mut out = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (inv_luma, inv_chroma) = inverse_noise(current[i], p);
            let mut sum = [current[i], current[i + 1], current[i + 2]];
            let (mut sw, mut sw2) = (1.0f32, 1.0f32);
            for a in &usable {
                let d = patch_distance(current, a, w, h, x, y, inv_luma, inv_chroma);
                let wt = a[i + 3] * agreement(d, p);
                for c in 0..3 {
                    sum[c] += wt * a[i + c];
                }
                sw += wt;
                sw2 += wt * wt;
            }
            out[i..i + 4].copy_from_slice(&[sum[0] / sw, sum[1] / sw, sum[2] / sw, sw * sw / sw2]);
        }
    }
    out
}

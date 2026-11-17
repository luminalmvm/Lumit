//! The a-trous wavelet: a B3-spline smoothing at dilations 1, 2, 4 and 8, each
//! level the last one smoothed again with its taps twice as far apart. The
//! difference between two levels is one band of detail, finest first.
//!
//! Pictures here are four lanes a pixel, row-major. Every lane is smoothed, so
//! the three colour lanes and whatever rides in the fourth go through together,
//! as they do on the card.

/// The B3-spline taps, `[1, 4, 6, 4, 1] / 16`.
pub const TAPS: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];

/// How many bands the transform splits a picture into.
pub const LEVELS: usize = 4;

/// An index reflected back inside `0..n` at either edge, as the kernels read.
#[must_use]
pub fn mirror(i: i32, n: u32) -> usize {
    let last = n.saturating_sub(1) as i32;
    let i = if i < 0 { -i } else { i };
    let i = if i > last { 2 * last - i } else { i };
    i.clamp(0, last) as usize
}

/// One pass of the taps along a row (`across`) or down a column, `dil` pixels
/// apart, over `LANES` lanes a pixel. `src` must be `w * h * LANES` long;
/// anything else comes back as it is.
#[must_use]
pub fn smooth<const LANES: usize>(src: &[f32], w: u32, h: u32, dil: i32, across: bool) -> Vec<f32> {
    let n = (w as usize) * (h as usize) * LANES;
    if src.len() != n {
        return src.to_vec();
    }
    let mut out = vec![0.0f32; n];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut acc = [0.0f32; LANES];
            for (k, tap) in TAPS.iter().enumerate() {
                let step = (k as i32 - 2) * dil;
                let (sx, sy) = if across {
                    (mirror(x + step, w), y as usize)
                } else {
                    (x as usize, mirror(y + step, h))
                };
                let at = (sy * w as usize + sx) * LANES;
                for (c, a) in acc.iter_mut().enumerate() {
                    *a += tap * src[at + c];
                }
            }
            let at = (y as usize * w as usize + x as usize) * LANES;
            out[at..at + LANES].copy_from_slice(&acc);
        }
    }
    out
}

/// The next level down from `src` (four lanes): across, then down, at
/// dilation `dil`.
#[must_use]
pub fn level(src: &[f32], w: u32, h: u32, dil: i32) -> Vec<f32> {
    level_of::<4>(src, w, h, dil)
}

/// [`level`] over `LANES` lanes a pixel.
#[must_use]
pub fn level_of<const LANES: usize>(src: &[f32], w: u32, h: u32, dil: i32) -> Vec<f32> {
    smooth::<LANES>(&smooth::<LANES>(src, w, h, dil, true), w, h, dil, false)
}

/// The dilation of band `b` (0 for the finest).
#[must_use]
pub fn dilation(b: usize) -> i32 {
    1 << b
}

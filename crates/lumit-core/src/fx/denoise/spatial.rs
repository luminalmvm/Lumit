//! The spatial half: four bands of detail, each kept in proportion to how far
//! it stands above the noise expected there, then the picture put back
//! together and returned to linear RGB.

use super::atrous::{dilation, level, LEVELS};
use super::space::{eotf, from_ycc, tone, ycc_to_linear, CLEAR};
use super::DenoiseParams;

/// How far the window a band's local energy is read over reaches either side.
const REACH: i32 = 2;

/// The mean of `(a - b)^2` in lane `c` over the 5x5 window round `(x, y)`,
/// leaving `(x, y)` itself out, edges held. Left out because a lone noise
/// spike would otherwise be its own evidence of picture and survive the
/// shrink as a speck; an edge or a texture has neighbours that carry it.
fn local_energy(a: &[f32], b: &[f32], w: u32, h: u32, x: u32, y: u32, c: usize) -> f32 {
    let mut e = 0.0f32;
    for dy in -REACH..=REACH {
        for dx in -REACH..=REACH {
            if dx == 0 && dy == 0 {
                continue;
            }
            let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
            let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
            let i = (sy * w as usize + sx) * 4 + c;
            let d = a[i] - b[i];
            e += d * d;
        }
    }
    e / ((2 * REACH + 1) * (2 * REACH + 1) - 1) as f32
}

/// How much of a band coefficient survives: the local Wiener gain `g` from
/// its energy `e` and the noise variance `var` expected in it, with Detail's
/// give-back on the two finest bands. Returns `(g, kept)`, `kept` the factor
/// the coefficient is multiplied by before Sharpen.
#[must_use]
pub fn band_gain(e: f32, var: f32, band: usize, amount: f32, p: &DenoiseParams) -> (f32, f32) {
    let mut g = (1.0 - var / (e + 1e-12)).max(0.0);
    if band < 2 && var > 0.0 {
        g += p.detail * (1.0 - g) * (e / var - 1.0).clamp(0.0, 1.0);
    }
    (g, 1.0 - amount * (1.0 - g))
}

/// The noise sigma of band `band` in lane `c` at a pixel whose luma is `y`.
#[must_use]
pub fn band_sigma(c: usize, band: usize, y: f32, p: &DenoiseParams) -> f32 {
    if c == 0 {
        p.luma_sigma * tone(y, p.tone) * p.luma_bands[band]
    } else {
        p.chroma_sigma * p.chroma_bands[band]
    }
}

/// The averaged picture split and shrunk: the sum of every band as kept, and
/// the coarsest level, which is kept whole. `averaged` carries the frames
/// blended in its fourth lane; `current` is read for its luma, which sets the
/// tone.
#[must_use]
pub fn shrink(
    averaged: &[f32],
    current: &[f32],
    w: u32,
    h: u32,
    p: &DenoiseParams,
) -> (Vec<f32>, Vec<f32>) {
    let n = (w as usize) * (h as usize) * 4;
    let mut details = vec![0.0f32; n];
    if averaged.len() != n || current.len() != n {
        return (details, averaged.to_vec());
    }
    let mut prev = averaged.to_vec();
    for band in 0..LEVELS {
        let next = level(&prev, w, h, dilation(band));
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let blended = averaged[i + 3].max(1.0);
                for c in 0..3 {
                    let e = local_energy(&prev, &next, w, h, x, y, c);
                    let s = band_sigma(c, band, current[i], p);
                    let amount = if c == 0 {
                        p.luma_amount
                    } else {
                        p.chroma_amount
                    } * p.band_amounts[band];
                    let (g, kept) = band_gain(e, s * s / blended, band, amount, p);
                    let mut d = (prev[i + c] - next[i + c]) * kept;
                    if band < 2 && c == 0 {
                        d *= 1.0 + p.sharpen * 0.5 * g;
                    }
                    details[i + c] += d;
                }
            }
        }
        prev = next;
    }
    (details, prev)
}

/// How much of the reduction a matte pixel lets through: its Rec. 709 luma,
/// clamped, as every matte that scales an amount reads it. Full with none.
#[must_use]
pub fn matte_strength(matte: &[f32], i: usize) -> f32 {
    match matte.get(i..i + 3) {
        Some(m) => (m[0] * 0.2126 + m[1] * 0.7152 + m[2] * 0.0722).clamp(0.0, 1.0),
        None => 1.0,
    }
}

/// The finished picture: the bands put back on the coarsest level, Grain,
/// the matte's share of the reduction, the view asked for, back to
/// premultiplied linear and mixed with `input`. `matte` is RGBA the size of
/// the picture, or empty for none.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn finish(
    input: &[f32],
    current: &[f32],
    details: &[f32],
    coarse: &[f32],
    averaged: &[f32],
    w: u32,
    h: u32,
    p: &DenoiseParams,
    matte: &[f32],
) -> Vec<f32> {
    let n = (w as usize) * (h as usize) * 4;
    if [current, details, coarse, averaged]
        .iter()
        .any(|s| s.len() != n)
        || input.len() != n
    {
        return input.to_vec();
    }
    let result = |i: usize| -> [f32; 3] {
        [
            coarse[i] + details[i],
            coarse[i + 1] + details[i + 1],
            coarse[i + 2] + details[i + 2],
        ]
    };
    let removed = |x: i32, y: i32| {
        let x = x.clamp(0, w as i32 - 1) as usize;
        let y = y.clamp(0, h as i32 - 1) as usize;
        let i = (y * w as usize + x) * 4;
        current[i] - result(i)[0]
    };
    let mut out = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let mut r = result(i);
            if p.grain > 0.0 {
                let mut box5 = 0.0f32;
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        box5 += removed(x as i32 + dx, y as i32 + dy);
                    }
                }
                r[0] += p.grain * (removed(x as i32, y as i32) - box5 / 25.0);
            }
            let k = matte_strength(matte, i);
            for c in 0..3 {
                r[c] = current[i + c] + (r[c] - current[i + c]) * k;
            }
            let src = [input[i], input[i + 1], input[i + 2], input[i + 3]];
            let a = src[3];
            let made = match p.view {
                1 => {
                    let (was, now) = (
                        from_ycc([current[i], current[i + 1], current[i + 2]]),
                        from_ycc(r),
                    );
                    let v = |c: usize| eotf(0.5 + 4.0 * (was[c] - now[c]));
                    [v(0), v(1), v(2), 1.0]
                }
                2 => {
                    let g = if p.frames > 0 {
                        ((averaged[i + 3] - 1.0) / (2.0 * p.frames as f32)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let g = eotf(g);
                    [g, g, g, 1.0]
                }
                _ if a <= CLEAR => src,
                _ => {
                    let [lr, lg, lb] = ycc_to_linear(r);
                    [lr * a, lg * a, lb * a, a]
                }
            };
            for c in 0..4 {
                out[i + c] = src[c] + (made[c] - src[c]) * p.mix;
            }
        }
    }
    out
}

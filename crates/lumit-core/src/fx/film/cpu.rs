//! Film's CPU twin, pass for pass what `fx_film.wgsl` and
//! `lumit_gpu::fx::film` run: the oracle the GPU is held to.

use super::tables::{self, cube_at, curve_at, grain_at};
use super::{Spread, Stage};
use crate::fx::cpu::{glow_exponential, glow_gaussian, glow_gaussian_plan};
use crate::fx::effects::film::{FilmFrame, FilmKernel, Shown};
use crate::fx::noise::hash01;

/// Film over a premultiplied scene-linear picture, in place.
pub fn film(rgba: &mut [f32], w: u32, h: u32, f: &FilmFrame) {
    let k = &f.k;
    let source = rgba.to_vec();
    let mut lit = expose(&source, k);
    for stage in [&f.bloom, &f.scatter, &f.halation] {
        lit = spread(lit, w, h, stage);
    }
    let held = k
        .couplers_on
        .then(|| spread(couple(&lit, k, &f.tables), w, h, &f.couplers));
    let noise = (k.grain > 0.0).then(|| {
        let mut blur = Stage {
            keep: [0.0; 3],
            terms: Vec::new(),
        };
        blur.gauss(f.grain_blur, [1.0; 3]);
        spread(grain_noise(w, h, k), w, h, &blur)
    });
    let shown = develop(&lit, held.as_deref(), noise.as_deref(), k, &f.tables);
    finish(rgba, &shown, &source, w, h, k);
}

/// Linear light to the layers' exposure, unpremultiplied.
pub fn expose(src: &[f32], k: &FilmKernel) -> Vec<f32> {
    let mut out = vec![0.0f32; src.len()];
    for (o, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let a = s[3];
        let mut u = if a > 0.0 {
            [s[0] / a, s[1] / a, s[2] / a]
        } else {
            [0.0; 3]
        };
        let gain = lift(u, k.boost);
        u = u.map(|v| v * gain);
        for (r, row) in k.camera.iter().enumerate() {
            o[r] = row[0] * u[0] + row[1] * u[1] + row[2] * u[2];
        }
        o[3] = a;
    }
    out
}

/// Where the clipped highlights start, linear.
pub const KNEE: f32 = 0.75;

/// How much a pixel is lifted by Clipped highlights: from nothing at [`KNEE`]
/// to `boost` stops at white, easing in, and the whole `boost` above it.
pub fn lift(u: [f32; 3], boost: f32) -> f32 {
    let m = u[0].max(u[1]).max(u[2]);
    if boost <= 0.0 || m <= KNEE {
        return 1.0;
    }
    let t = ((m - KNEE) / (1.0 - KNEE)).min(1.0);
    (boost * t * t).exp2()
}

/// Colour outside Rec. 709 pulled in toward its own luminance until no
/// channel is below zero, keeping its hue and brightness.
pub fn in_gamut(rgb: [f32; 3]) -> [f32; 3] {
    let low = rgb[0].min(rgb[1]).min(rgb[2]);
    if low >= 0.0 {
        return rgb;
    }
    let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    if luma <= 0.0 {
        return [0.0; 3];
    }
    let t = luma / (luma - low);
    rgb.map(|v| (luma + (v - luma) * t).max(0.0))
}

/// One spreading pass: the picture times `keep`, plus each blur times its
/// weight, added in order. Alpha is the picture's own.
pub fn spread(src: Vec<f32>, w: u32, h: u32, stage: &Stage) -> Vec<f32> {
    if stage.is_identity() {
        return src;
    }
    let mut acc = src.clone();
    for px in acc.chunks_exact_mut(4) {
        for (v, keep) in px.iter_mut().zip(stage.keep) {
            *v *= keep;
        }
    }
    for (how, weight) in &stage.terms {
        let blurred = match *how {
            Spread::Gauss(sigma) => {
                glow_gaussian(&src, w, h, &glow_gaussian_plan(2.0 * sigma, w, h))
            }
            Spread::Exp { radius_px, falloff } => glow_exponential(&src, w, h, radius_px, falloff),
        };
        for (a, b) in acc.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
            for c in 0..3 {
                a[c] += b[c] * weight[c];
            }
        }
    }
    acc
}

/// What each layer's development holds back in each layer, before it spreads.
pub fn couple(lit: &[f32], k: &FilmKernel, t: &[f32]) -> Vec<f32> {
    let curve = &t[tables::AT_CURVE..];
    let mut out = vec![0.0f32; lit.len()];
    for (o, e) in out.chunks_exact_mut(4).zip(lit.chunks_exact(4)) {
        let mut silver = [0.0f32; 3];
        for c in 0..3 {
            let d = curve_at(curve, c, log_e(e[c]) * k.gamma);
            silver[c] = if k.positive { k.top[c] - d } else { d };
        }
        for (m, om) in o.iter_mut().take(3).enumerate() {
            *om = silver[0] * k.couplers[0][m]
                + silver[1] * k.couplers[1][m]
                + silver[2] * k.couplers[2][m];
        }
        o[3] = 1.0;
    }
    out
}

fn log_e(v: f32) -> f32 {
    (v.max(0.0) + 1e-10).log10()
}

/// A normal draw for `lane` at this pixel and frame (Box and Muller).
pub fn normal(seed: u32, lane: u32, x: i32, y: i32, tick: i32) -> f32 {
    let u1 = hash01(seed, lane * 2, x, y, tick).max(1e-7);
    let u2 = hash01(seed, lane * 2 + 1, x, y, tick);
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

/// One normal draw per layer per pixel, sharing a fourth as Colour says.
pub fn grain_noise(w: u32, h: u32, k: &FilmKernel) -> Vec<f32> {
    let mut out = vec![0.0f32; (w * h * 4) as usize];
    let (own, shared) = (k.grain_colour.sqrt(), (1.0 - k.grain_colour).sqrt());
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = (y as usize * w as usize + x as usize) * 4;
            let common = normal(k.seed, 3, x, y, k.tick);
            for c in 0..3 {
                out[i + c] = own * normal(k.seed, c as u32, x, y, k.tick) + shared * common;
            }
            out[i + 3] = 1.0;
        }
    }
    out
}

/// The developed film, shown: premultiplied linear Rec. 709.
pub fn develop(
    lit: &[f32],
    held: Option<&[f32]>,
    noise: Option<&[f32]>,
    k: &FilmKernel,
    t: &[f32],
) -> Vec<f32> {
    let mut out = vec![0.0f32; lit.len()];
    for (i, o) in out.chunks_exact_mut(4).enumerate() {
        let e = &lit[i * 4..i * 4 + 4];
        let corr = held.map_or([0.0; 3], |c| [c[i * 4], c[i * 4 + 1], c[i * 4 + 2]]);
        let n = noise.map_or([0.0; 3], |n| [n[i * 4], n[i * 4 + 1], n[i * 4 + 2]]);
        let rgb = pixel([e[0], e[1], e[2]], corr, n, k, t);
        for c in 0..3 {
            o[c] = rgb[c] * e[3];
        }
        o[3] = e[3];
    }
    out
}

/// One pixel of [`develop`], unpremultiplied.
pub fn pixel(e: [f32; 3], corr: [f32; 3], n: [f32; 3], k: &FilmKernel, t: &[f32]) -> [f32; 3] {
    let grain = &t[tables::AT_GRAIN..];
    let sigma = |c: usize, d: f32| grain_at(grain, c, d, k.top[c]).max(0.0).sqrt() * k.grain;
    if k.shown == Shown::Own {
        let mut out = e.map(|v| v * 0.184);
        if k.grain > 0.0 {
            for c in 0..3 {
                let d = k.grey[c] + k.slope[c] * (e[c].max(1e-10)).log10();
                out[c] *= 10f32.powf(sigma(c, d) * n[c] / k.slope[c].max(1e-3));
            }
        }
        return out;
    }
    let mut d = [0.0f32; 3];
    for c in 0..3 {
        let x = log_e(e[c]);
        d[c] = if k.couplers_on {
            curve_at(&t[tables::AT_CURVE0..], c, (x - corr[c]) * k.gamma)
        } else {
            curve_at(&t[tables::AT_CURVE..], c, x * k.gamma)
        };
        if k.grain > 0.0 {
            d[c] += sigma(c, d[c]) * n[c];
        }
    }
    let lut_scan = &t[tables::AT_LUT_SCAN..];
    let out = match k.shown {
        Shown::Print => {
            let floor = [tables::DENSITY_FLOOR; 3];
            let lp = cube_at(&t[tables::AT_LUT_PRINT..], floor, k.top, d);
            let mut dp = [0.0f32; 3];
            for c in 0..3 {
                let x = (lp[c] + k.print_gain[c]) * k.print_gamma;
                dp[c] = curve_at(&t[tables::AT_PRINT_CURVE..], c, x);
            }
            let xyz = cube_at(lut_scan, k.scan_low, k.scan_top, dp).map(|v| 10f32.powf(v));
            mul(&k.to_rec709, xyz)
        }
        Shown::Slide => {
            let xyz = cube_at(lut_scan, k.scan_low, k.scan_top, d).map(|v| 10f32.powf(v));
            mul(&k.to_rec709, xyz)
        }
        _ => {
            let mut v = [0.0f32; 3];
            for c in 0..3 {
                v[c] = 10f32.powf((d[c] - k.grey[c]) / k.slope[c].max(1e-3));
            }
            mul(&k.inverse, v)
        }
    };
    in_gamut(out)
}

fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

/// The gate's place and the Mix: the developed picture moved by the weave,
/// edges held, then blended over the original.
pub fn finish(out: &mut [f32], shown: &[f32], source: &[f32], w: u32, h: u32, k: &FilmKernel) {
    let (wi, hi) = (w as i32, h as i32);
    let at = |x: i32, y: i32| {
        (y.clamp(0, hi - 1) as usize * w as usize + x.clamp(0, wi - 1) as usize) * 4
    };
    let still = k.weave == [0.0, 0.0];
    for y in 0..hi {
        for x in 0..wi {
            let i = (y as usize * w as usize + x as usize) * 4;
            let mut px = [0.0f32; 4];
            if still {
                px.copy_from_slice(&shown[i..i + 4]);
            } else {
                let (sx, sy) = (x as f32 - k.weave[0], y as f32 - k.weave[1]);
                let (x0, y0) = (sx.floor(), sy.floor());
                let (fx, fy) = (sx - x0, sy - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                let (a, b, c, d) = (
                    at(x0, y0),
                    at(x0 + 1, y0),
                    at(x0, y0 + 1),
                    at(x0 + 1, y0 + 1),
                );
                for ch in 0..4 {
                    let top = shown[a + ch] + (shown[b + ch] - shown[a + ch]) * fx;
                    let bottom = shown[c + ch] + (shown[d + ch] - shown[c + ch]) * fx;
                    px[ch] = top + (bottom - top) * fy;
                }
            }
            for ch in 0..4 {
                out[i + ch] = source[i + ch] * (1.0 - k.mix) + px[ch] * k.mix;
            }
        }
    }
}

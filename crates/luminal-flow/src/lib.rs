//! Optical flow and frame synthesis — the CPU reference (docs/impl/optical-flow.md
//! §6.5, the oracle the WGSL/DIS backend must match). Pure, deterministic, no
//! GPU: this is what makes `Flow` retiming produce in-between frames.
//!
//! In plain terms: given two frames A and B, we work out how every pixel moved
//! from one to the other (the *flow*), then paint a brand-new frame that sits
//! part-way between them — A and B each dragged along their motion to where
//! they'd be at that moment, then blended. That's what smooth slow motion is:
//! frames that were never filmed, invented from the motion between real ones.
//!
//! This first implementation uses pyramidal Lucas–Kanade (coarse-to-fine so it
//! catches large motion) — slow but correct, exactly the reference role the
//! spec asks for. The fast WGSL DIS backend and occlusion masks come later.

/// A single-channel image in 0..1 (encoded luma), row-major.
#[derive(Clone)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Gray {
    fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }

    /// Bilinear sample with edge clamp.
    fn sample(&self, x: f32, y: f32) -> f32 {
        sample_scalar(&self.data, self.w, self.h, x, y)
    }
}

/// A dense flow field: `(u, v)` per pixel, in pixels, such that `A(x) ≈
/// B(x + (u, v))` — the displacement of the pixel at `x` from A to B.
pub struct FlowField {
    pub w: usize,
    pub h: usize,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
}

/// BT.709 luma of sRGB-encoded RGBA bytes, in 0..1 (correlation happens on
/// perceptual/encoded values — docs/impl/optical-flow.md §1).
pub fn to_gray(rgba: &[u8], w: usize, h: usize) -> Gray {
    let mut data = vec![0f32; w * h];
    for (i, px) in data.iter_mut().enumerate() {
        let base = i * 4;
        if base + 2 < rgba.len() {
            let r = f32::from(rgba[base]);
            let g = f32::from(rgba[base + 1]);
            let b = f32::from(rgba[base + 2]);
            *px = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
        }
    }
    Gray { w, h, data }
}

fn sample_scalar(data: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    if w == 0 || h == 0 {
        return 0.0;
    }
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let a = data[y0 * w + x0] * (1.0 - fx) + data[y0 * w + x1] * fx;
    let b = data[y1 * w + x0] * (1.0 - fx) + data[y1 * w + x1] * fx;
    a * (1.0 - fy) + b * fy
}

fn downsample(g: &Gray) -> Gray {
    let w = (g.w / 2).max(1);
    let h = (g.h / 2).max(1);
    let mut data = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let x0 = (2 * x).min(g.w - 1);
            let y0 = (2 * y).min(g.h - 1);
            let x1 = (2 * x + 1).min(g.w - 1);
            let y1 = (2 * y + 1).min(g.h - 1);
            data[y * w + x] = 0.25 * (g.at(x0, y0) + g.at(x1, y0) + g.at(x0, y1) + g.at(x1, y1));
        }
    }
    Gray { w, h, data }
}

/// Central-difference spatial gradients of `g`.
fn gradients(g: &Gray) -> (Vec<f32>, Vec<f32>) {
    let (w, h) = (g.w, g.h);
    let mut ix = vec![0f32; w * h];
    let mut iy = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(h - 1);
            ix[y * w + x] = 0.5 * (g.at(xp, y) - g.at(xm, y));
            iy[y * w + x] = 0.5 * (g.at(x, yp) - g.at(x, ym));
        }
    }
    (ix, iy)
}

/// Bilinearly resample a flow component `src` (`sw×sh`) to `dw×dh`, scaling the
/// *values* by `dw/sw` (a flow field grows with the image).
fn upsample_flow(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut out = vec![0f32; dw * dh];
    let scale = dw as f32 / sw.max(1) as f32;
    for y in 0..dh {
        for x in 0..dw {
            let sx = x as f32 * sw as f32 / dw as f32;
            let sy = y as f32 * sh as f32 / dh as f32;
            out[y * dw + x] = sample_scalar(src, sw, sh, sx, sy) * scale;
        }
    }
    out
}

/// Refine `(u, v)` at one pyramid level with windowed Lucas–Kanade iterations
/// (forward-additive; the 2×2 normal equations from A's gradients).
fn lk_refine(a: &Gray, b: &Gray, u: &mut [f32], v: &mut [f32], iters: usize, win: i32) {
    let (w, h) = (a.w, a.h);
    let (ix, iy) = gradients(a);
    for _ in 0..iters {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let (mut h11, mut h12, mut h22, mut b1, mut b2) = (0f32, 0f32, 0f32, 0f32, 0f32);
                for wy in -win..=win {
                    for wx in -win..=win {
                        let px = x as i32 + wx;
                        let py = y as i32 + wy;
                        if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                            continue;
                        }
                        let (px, py) = (px as usize, py as usize);
                        let gx = ix[py * w + px];
                        let gy = iy[py * w + px];
                        // Residual A(x) − B(x + current flow).
                        let it = a.at(px, py) - b.sample(px as f32 + u[i], py as f32 + v[i]);
                        h11 += gx * gx;
                        h12 += gx * gy;
                        h22 += gy * gy;
                        b1 += gx * it;
                        b2 += gy * it;
                    }
                }
                let det = h11 * h22 - h12 * h12;
                if det.abs() > 1e-6 {
                    // Δu = H⁻¹ b, capped per step for stability.
                    let du = (h22 * b1 - h12 * b2) / det;
                    let dv = (h11 * b2 - h12 * b1) / det;
                    u[i] += du.clamp(-1.0, 1.0);
                    v[i] += dv.clamp(-1.0, 1.0);
                }
            }
        }
    }
}

/// Dense forward flow A→B by pyramidal Lucas–Kanade (coarse-to-fine).
pub fn flow(a: &Gray, b: &Gray) -> FlowField {
    // Build both pyramids down to a small top level.
    let (mut pa, mut pb) = (vec![a.clone()], vec![b.clone()]);
    while pa.last().map(|g| g.w.min(g.h)).unwrap_or(0) > 24 {
        let na = downsample(pa.last().unwrap_or(a));
        let nb = downsample(pb.last().unwrap_or(b));
        pa.push(na);
        pb.push(nb);
    }
    let levels = pa.len();
    let top = &pa[levels - 1];
    let mut u = vec![0f32; top.w * top.h];
    let mut v = vec![0f32; top.w * top.h];
    let (mut pw, mut ph) = (top.w, top.h);
    for lvl in (0..levels).rev() {
        let (ga, gb) = (&pa[lvl], &pb[lvl]);
        if ga.w != pw || ga.h != ph {
            u = upsample_flow(&u, pw, ph, ga.w, ga.h);
            v = upsample_flow(&v, pw, ph, ga.w, ga.h);
        }
        lk_refine(ga, gb, &mut u, &mut v, 8, 3);
        pw = ga.w;
        ph = ga.h;
    }
    FlowField {
        w: a.w,
        h: a.h,
        u,
        v,
    }
}

fn sample_rgba(rgba: &[u8], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    if w == 0 || h == 0 {
        return [0.0; 4];
    }
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let mut out = [0f32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let p = |px: usize, py: usize| f32::from(rgba[(py * w + px) * 4 + c]);
        let a = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
        let b = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
        *o = a * (1.0 - fy) + b * fy;
    }
    out
}

/// Synthesise the frame at phase `phi` ∈ [0,1] between A and B by backward-warping
/// each endpoint along its flow and blending (docs/impl/optical-flow.md §3). `phi`
/// = 0 returns A, 1 returns B, bit-exactly. `fwd` is flow A→B, `bwd` is B→A. Where
/// the two warps disagree badly (occlusion / bad flow) it falls back toward a plain
/// crossfade — the documented graceful degradation.
#[allow(clippy::too_many_arguments)]
pub fn synthesize(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    fwd: &FlowField,
    bwd: &FlowField,
    phi: f32,
) -> Vec<u8> {
    if phi <= 0.0 {
        return a.to_vec();
    }
    if phi >= 1.0 {
        return b.to_vec();
    }
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (fx, fy) = (fwd.u[i], fwd.v[i]);
            let (bx, by) = (bwd.u[i], bwd.v[i]);
            // Warp A back by −phi·F, B back by (1−phi)·G.
            let sa = sample_rgba(a, w, h, x as f32 - phi * fx, y as f32 - phi * fy);
            let sb = sample_rgba(
                b,
                w,
                h,
                x as f32 + (1.0 - phi) * bx,
                y as f32 + (1.0 - phi) * by,
            );
            // Photometric disagreement → this pixel's flow is unreliable
            // (occlusion, textureless): blend toward a plain crossfade.
            let err: f32 = (0..3).map(|c| (sa[c] - sb[c]).abs()).sum::<f32>() / 3.0;
            let conf = (1.0 - err / 48.0).clamp(0.0, 1.0);
            let la = sample_rgba(a, w, h, x as f32, y as f32);
            let lb = sample_rgba(b, w, h, x as f32, y as f32);
            for c in 0..4 {
                let warped = sa[c] * (1.0 - phi) + sb[c] * phi;
                let plain = la[c] * (1.0 - phi) + lb[c] * phi;
                out[i * 4 + c] = (warped * conf + plain * (1.0 - conf))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// End-to-end: the flow-interpolated frame at `phi` between RGBA frames `a` and
/// `b` (`w×h`). Computes flow both ways and synthesises. This is what the `Flow`
/// retiming policy calls.
pub fn interpolate(a: &[u8], b: &[u8], w: usize, h: usize, phi: f32) -> Vec<u8> {
    if phi <= 0.0 {
        return a.to_vec();
    }
    if phi >= 1.0 {
        return b.to_vec();
    }
    let ga = to_gray(a, w, h);
    let gb = to_gray(b, w, h);
    let fwd = flow(&ga, &gb);
    let bwd = flow(&gb, &ga);
    synthesize(a, b, w, h, &fwd, &bwd, phi)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A smooth, well-textured test image (sum of a few non-aliasing sines) so
    /// Lucas–Kanade is well-conditioned (no aperture problem).
    fn texture(w: usize, h: usize, dx: f32, dy: f32) -> Gray {
        let mut data = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 - dx;
                let fy = y as f32 - dy;
                let v = 0.5
                    + 0.2 * (fx * 0.21).sin() * (fy * 0.17).cos()
                    + 0.15 * (fx * 0.11 + fy * 0.13).sin()
                    + 0.1 * (fx * 0.37).cos();
                data[y * w + x] = v.clamp(0.0, 1.0);
            }
        }
        Gray { w, h, data }
    }

    #[test]
    fn recovers_a_known_translation() {
        let (w, h) = (72, 72);
        let a = texture(w, h, 0.0, 0.0);
        let b = texture(w, h, 3.0, 2.0); // content shifted by (3, 2)
        let f = flow(&a, &b);
        // Mean endpoint error over the interior (away from clamped borders).
        let (mut sum, mut n) = (0.0f32, 0usize);
        for y in 12..h - 12 {
            for x in 12..w - 12 {
                let i = y * w + x;
                let e = ((f.u[i] - 3.0).powi(2) + (f.v[i] - 2.0).powi(2)).sqrt();
                sum += e;
                n += 1;
            }
        }
        let epe = sum / n as f32;
        assert!(epe < 0.5, "mean endpoint error too high: {epe}");
    }

    #[test]
    fn synthesis_round_trips_at_the_endpoints() {
        let (w, h) = (16, 16);
        let a: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let b: Vec<u8> = (0..w * h * 4).map(|i| ((i * 7) % 251) as u8).collect();
        // phi 0 and 1 return the endpoints bit-exactly (degenerate path).
        assert_eq!(interpolate(&a, &b, w, h, 0.0), a);
        assert_eq!(interpolate(&a, &b, w, h, 1.0), b);
    }

    #[test]
    fn midpoint_beats_a_plain_crossfade_on_textured_motion() {
        // On well-textured motion, the flow-synthesised midpoint should be
        // closer to the *true* midpoint frame than a naive crossfade is — that
        // difference (sharp vs ghosted) is the whole point of flow interpolation.
        let (w, h) = (64, 64);
        let to_rgba = |g: &Gray| -> Vec<u8> {
            let mut f = vec![0u8; w * h * 4];
            for i in 0..w * h {
                let v = (g.data[i] * 255.0).round().clamp(0.0, 255.0) as u8;
                f[i * 4] = v;
                f[i * 4 + 1] = v;
                f[i * 4 + 2] = v;
                f[i * 4 + 3] = 255;
            }
            f
        };
        let a = to_rgba(&texture(w, h, 0.0, 0.0));
        let b = to_rgba(&texture(w, h, 8.0, 0.0));
        let truth = to_rgba(&texture(w, h, 4.0, 0.0)); // the real in-between frame
        let synth = interpolate(&a, &b, w, h, 0.5);
        let crossfade: Vec<u8> = a
            .iter()
            .zip(&b)
            .map(|(x, y)| ((u16::from(*x) + u16::from(*y)) / 2) as u8)
            .collect();
        let err = |frame: &[u8]| -> f64 {
            let (mut s, mut n) = (0.0f64, 0usize);
            for y in 12..h - 12 {
                for x in 12..w - 12 {
                    let i = (y * w + x) * 4;
                    s += (f64::from(frame[i]) - f64::from(truth[i])).abs();
                    n += 1;
                }
            }
            s / n as f64
        };
        let (e_synth, e_cross) = (err(&synth), err(&crossfade));
        assert!(
            e_synth < e_cross,
            "flow synth error {e_synth} should beat crossfade {e_cross}"
        );
    }
}

//! Optical flow and frame synthesis (docs/impl/optical-flow.md). This file is
//! the CPU DIS implementation — the deterministic oracle (K-019) the WGSL
//! backend must match within 1e-3 — plus the shared synthesis code and the
//! backend-choosing `FlowEngine` that callers use.
//!
//! In plain terms: given two frames A and B, we work out how every pixel moved
//! from one to the other (the *flow*), then paint a brand-new frame that sits
//! part-way between them — A and B each dragged along their motion to where
//! they'd be at that moment, then blended. That's what smooth slow motion is:
//! frames that were never filmed, invented from the motion between real ones.
//!
//! The algorithm is DIS — Dense Inverse Search (Kroeger et al., ECCV 2016) —
//! exactly as pinned in the impl note: a coarse-to-fine pyramid; at each level
//! small 8×8 patches each hunt for where they went (the "inverse search",
//! a handful of Newton steps per patch); every pixel then averages the
//! patches covering it, trusting each patch by how well it photometrically
//! matches (that weighting is what keeps edges crisp); finally one edge-aware
//! blur tidies the field. Occlusion — pixels visible in only one frame —
//! is found by checking the two flow directions against each other, and the
//! synthesis step falls back to a plain crossfade wherever both frames lost
//! sight of a pixel (the documented graceful degradation).

pub mod gpu;

/// Patch side in pixels (impl note §1: 8×8 patches).
pub(crate) const PATCH: usize = 8;
/// Patch grid stride (impl note §1: stride-4 grid).
pub(crate) const STRIDE: usize = 4;
/// Inverse-search iteration cap (impl note §1: ≤ 12 iterations).
pub(crate) const MAX_ITERS: usize = 12;
/// Convergence: stop when |Δu| < 0.02 px (squared here).
pub(crate) const CONV2: f32 = 0.02 * 0.02;
/// Hessian determinant floor — below this the patch is textureless (§1).
pub(crate) const DET_MIN: f32 = 1e-6;
/// A patch whose final matching cost stays above this fraction of its own
/// variance never actually found its content in the other frame — it is
/// straddling a motion boundary or occluded. Contrast-relative, so it means
/// the same thing at every pyramid level.
pub(crate) const COST_VAR_RATIO: f32 = 0.25;
/// Absolute cost allowance under the same test: sub-pixel convergence and
/// bilinear interpolation leave ~0.03 residual per pixel even on a perfect
/// match, and low-contrast patches must not fail on that noise.
pub(crate) const COST_FLOOR: f32 = 0.05;
/// Densification / smoothing photometric sigma, in encoded luma (§1: σ ≈ 0.08).
pub(crate) const SIGMA2: f32 = 0.08 * 0.08;
/// Flow-range sigma (squared, px²) in the smoothing bilateral: vectors more
/// than a couple of pixels apart belong to different motions and must not mix.
pub(crate) const FLOW_SIGMA2: f32 = 1.0;
/// Pyramid floor: stop when the next level would drop under ~24 px. Any
/// smaller and the 8×8 patches are frame-scale — every patch straddles every
/// motion boundary, and whole strips of the coarsest field start as garbage
/// that finer levels can't always heal (measured in the §6.1 occlusion test).
pub(crate) const MIN_LEVEL_DIM: usize = 24;
/// Occlusion consistency test constants (§2).
pub(crate) const OCC_ABS: f32 = 1.5;
pub(crate) const OCC_REL: f32 = 0.05;
/// Synthesis weight epsilon (§3).
const SYNTH_EPS: f32 = 1e-4;

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
/// `valid` marks pixels whose flow came from at least one photometrically
/// trusted patch (0 = textureless or mismatched everywhere; treat as suspect).
pub struct FlowField {
    pub w: usize,
    pub h: usize,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    pub valid: Vec<u8>,
}

impl FlowField {
    /// An all-zero, all-invalid field (the degenerate answer for tiny images).
    fn zeroed(w: usize, h: usize) -> Self {
        FlowField {
            w,
            h,
            u: vec![0.0; w * h],
            v: vec![0.0; w * h],
            valid: vec![0; w * h],
        }
    }
}

/// Working resolution for flow (impl note §1): `Half` is the default — flow on
/// a half-size copy, scaled back up — `Full` computes at the frames' own size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlowQuality {
    Half,
    Full,
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

/// Box-downsample by 2 (the pyramid step; mirrored exactly in WGSL).
pub(crate) fn downsample(g: &Gray) -> Gray {
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

/// Sobel gradients of `g`, normalised to intensity-per-pixel (÷8), clamped
/// borders (impl note §1: Sobel gradient textures per level).
pub(crate) fn sobel(g: &Gray) -> (Vec<f32>, Vec<f32>) {
    let (w, h) = (g.w, g.h);
    let mut gx = vec![0f32; w * h];
    let mut gy = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(h - 1);
            let (tl, t, tr) = (g.at(xm, ym), g.at(x, ym), g.at(xp, ym));
            let (l, r) = (g.at(xm, y), g.at(xp, y));
            let (bl, b, br) = (g.at(xm, yp), g.at(x, yp), g.at(xp, yp));
            gx[y * w + x] = ((tr + 2.0 * r + br) - (tl + 2.0 * l + bl)) / 8.0;
            gy[y * w + x] = ((bl + 2.0 * b + br) - (tl + 2.0 * t + tr)) / 8.0;
        }
    }
    (gx, gy)
}

/// Bilinearly resample a flow component `src` (`sw×sh`) to `dw×dh`, scaling the
/// *values* by `dw/sw` (a flow field grows with the image).
pub(crate) fn upsample_flow(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
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

/// Resample a whole field (bilinear flow, nearest validity) to `dw×dh`.
fn upsample_field(f: &FlowField, dw: usize, dh: usize) -> FlowField {
    let u = upsample_flow(&f.u, f.w, f.h, dw, dh);
    let v = upsample_flow(&f.v, f.w, f.h, dw, dh);
    let mut valid = vec![0u8; dw * dh];
    for y in 0..dh {
        for x in 0..dw {
            let sx = (x * f.w / dw.max(1)).min(f.w.saturating_sub(1));
            let sy = (y * f.h / dh.max(1)).min(f.h.saturating_sub(1));
            valid[y * dw + x] = f.valid[sy * f.w + sx];
        }
    }
    FlowField {
        w: dw,
        h: dh,
        u,
        v,
        valid,
    }
}

/// Patch grid along one dimension: positions `4·i` clamped so the last patch
/// ends exactly at the image edge (mirrored in WGSL).
pub(crate) fn patch_count(dim: usize) -> usize {
    if dim < PATCH {
        return 0;
    }
    let span = dim - PATCH;
    let mut n = span / STRIDE + 1;
    if !span.is_multiple_of(STRIDE) {
        n += 1;
    }
    n
}

pub(crate) fn patch_origin(i: usize, dim: usize) -> usize {
    (i * STRIDE).min(dim - PATCH)
}

/// One patch's answer: its flow vector and whether the solve was trustworthy.
struct PatchField {
    npx: usize,
    npy: usize,
    u: Vec<f32>,
    v: Vec<f32>,
    valid: Vec<u8>,
}

/// The inverse search (impl note §1 step 2): every 8×8 patch refines its flow
/// by inverse-compositional Gauss–Newton — the Hessian comes from the template
/// patch's gradients (fixed across iterations), so each step only re-samples B.
fn inverse_search(
    a: &Gray,
    b: &Gray,
    gx: &[f32],
    gy: &[f32],
    init_u: &[f32],
    init_v: &[f32],
) -> PatchField {
    let (w, h) = (a.w, a.h);
    let (npx, npy) = (patch_count(w), patch_count(h));
    let mut out = PatchField {
        npx,
        npy,
        u: vec![0.0; npx * npy],
        v: vec![0.0; npx * npy],
        valid: vec![0; npx * npy],
    };
    for pj in 0..npy {
        for pi in 0..npx {
            let x0 = patch_origin(pi, w);
            let y0 = patch_origin(pj, h);
            // Template Hessian H = Σ [gx², gx·gy; gx·gy, gy²] over the patch,
            // plus the patch's own mean and energy (for the variance-relative
            // cost test below).
            let (mut h11, mut h12, mut h22) = (0f32, 0f32, 0f32);
            let (mut sum_a, mut sum_a2) = (0f32, 0f32);
            for dy in 0..PATCH {
                for dx in 0..PATCH {
                    let i = (y0 + dy) * w + (x0 + dx);
                    h11 += gx[i] * gx[i];
                    h12 += gx[i] * gy[i];
                    h22 += gy[i] * gy[i];
                    sum_a += a.data[i];
                    sum_a2 += a.data[i] * a.data[i];
                }
            }
            let np = (PATCH * PATCH) as f32;
            let var = sum_a2 - sum_a * sum_a / np; // Σ(a − ā)² over the patch
            let det = h11 * h22 - h12 * h12;
            // Start from the coarser level's flow — sampled at the patch
            // centre *and* its corners, keeping the candidate whose SSD is
            // lowest. Near a blurred motion edge the corners straddle both
            // motions, so the true one is always on the ballot (the
            // data-parallel stand-in for OpenCV's neighbour propagation).
            let cands = [
                (x0 as f32 + 3.5, y0 as f32 + 3.5),
                (x0 as f32 + 0.5, y0 as f32 + 0.5),
                (x0 as f32 + 6.5, y0 as f32 + 0.5),
                (x0 as f32 + 0.5, y0 as f32 + 6.5),
                (x0 as f32 + 6.5, y0 as f32 + 6.5),
                // Far samples one patch out: near a blurred motion boundary
                // the whole patch sits inside the blur, and only a sample
                // from beyond it puts the true motion on the ballot.
                (x0 as f32 - 4.5, y0 as f32 + 3.5),
                (x0 as f32 + 11.5, y0 as f32 + 3.5),
                (x0 as f32 + 3.5, y0 as f32 - 4.5),
                (x0 as f32 + 3.5, y0 as f32 + 11.5),
            ];
            let (mut u, mut v) = (0f32, 0f32);
            let mut cand_best = f32::INFINITY;
            for (sx, sy) in cands {
                let cu = sample_scalar(init_u, w, h, sx, sy);
                let cv = sample_scalar(init_v, w, h, sx, sy);
                let mut ssd = 0f32;
                for dy in 0..PATCH {
                    for dx in 0..PATCH {
                        let i = (y0 + dy) * w + (x0 + dx);
                        let e = a.data[i] - b.sample((x0 + dx) as f32 + cu, (y0 + dy) as f32 + cv);
                        ssd += e * e;
                    }
                }
                if ssd < cand_best {
                    cand_best = ssd;
                    u = cu;
                    v = cv;
                }
            }
            let mut ok = det >= DET_MIN; // textureless patches are invalid (§1)
            if ok {
                // Best-so-far bookkeeping: a Gauss–Newton step that makes the
                // patch match *worse* is reverted and the search stops (the
                // classic guard against near-singular H throwing the patch
                // somewhere absurd; mirrored exactly in WGSL).
                let (mut bu, mut bv) = (u, v);
                let mut best = f32::INFINITY;
                for _ in 0..MAX_ITERS {
                    // r = Σ g·(A(x) − B(x+u)); Δu = H⁻¹ r reduces the residual.
                    let (mut r1, mut r2, mut cost) = (0f32, 0f32, 0f32);
                    for dy in 0..PATCH {
                        for dx in 0..PATCH {
                            let i = (y0 + dy) * w + (x0 + dx);
                            let e =
                                a.data[i] - b.sample((x0 + dx) as f32 + u, (y0 + dy) as f32 + v);
                            r1 += gx[i] * e;
                            r2 += gy[i] * e;
                            cost += e * e;
                        }
                    }
                    if cost >= best {
                        u = bu; // the last step made things worse: revert
                        v = bv;
                        break;
                    }
                    best = cost;
                    bu = u;
                    bv = v;
                    let du = (h22 * r1 - h12 * r2) / det;
                    let dv = (h11 * r2 - h12 * r1) / det;
                    u += du;
                    v += dv;
                    if du * du + dv * dv < CONV2 {
                        break;
                    }
                }
                // A patch that never got close to explaining its own contrast
                // is straddling a motion boundary or occluded — its vector
                // must not vote in densification.
                ok = best <= COST_VAR_RATIO * var + COST_FLOOR;
            }
            let p = pj * npx + pi;
            out.u[p] = u;
            out.v[p] = v;
            out.valid[p] = u8::from(ok);
        }
    }
    out
}

/// Densification (impl note §1 step 3): each pixel averages the patch vectors
/// covering it, weighted by how well each patch's motion photometrically
/// explains this pixel — that weighting is what keeps edges crisp.
fn densify(
    a: &Gray,
    b: &Gray,
    patches: &PatchField,
    init_u: &[f32],
    init_v: &[f32],
) -> (Vec<f32>, Vec<f32>, Vec<u8>) {
    let (w, h) = (a.w, a.h);
    let mut u = vec![0f32; w * h];
    let mut v = vec![0f32; w * h];
    let mut valid = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let gi = (x / STRIDE) as i32;
            let gj = (y / STRIDE) as i32;
            // First pass: collect the covering patches' votes and find the
            // best-matching one (the winner).
            let (mut best_w, mut best_u, mut best_v) = (0f32, 0f32, 0f32);
            let mut votes = [(0f32, 0f32, 0f32); 9];
            let mut n_votes = 0usize;
            for oj in -1i32..=1 {
                let cj = gj + oj;
                if cj < 0 || cj >= patches.npy as i32 {
                    continue;
                }
                for oi in -1i32..=1 {
                    let ci = gi + oi;
                    if ci < 0 || ci >= patches.npx as i32 {
                        continue;
                    }
                    let (ci, cj) = (ci as usize, cj as usize);
                    let x0 = patch_origin(ci, w);
                    let y0 = patch_origin(cj, h);
                    if x < x0 || x > x0 + (PATCH - 1) || y < y0 || y > y0 + (PATCH - 1) {
                        continue; // this candidate patch doesn't cover the pixel
                    }
                    let p = cj * patches.npx + ci;
                    if patches.valid[p] == 0 {
                        continue;
                    }
                    let err =
                        b.sample(x as f32 + patches.u[p], y as f32 + patches.v[p]) - a.data[i];
                    let wgt = (-(err * err) / SIGMA2).exp();
                    votes[n_votes] = (wgt, patches.u[p], patches.v[p]);
                    n_votes += 1;
                    if wgt > best_w {
                        best_w = wgt;
                        best_u = patches.u[p];
                        best_v = patches.v[p];
                    }
                }
            }
            // Second pass: average only the votes that agree with the winner.
            // Averaging *across* a motion boundary would manufacture a vector
            // belonging to neither motion — the classic rubber-sheet edge.
            let (mut acc_u, mut acc_v, mut wsum) = (0f32, 0f32, 0f32);
            for &(wgt, vu, vv) in votes.iter().take(n_votes) {
                let d2 = (vu - best_u) * (vu - best_u) + (vv - best_v) * (vv - best_v);
                if d2 <= FLOW_SIGMA2 * 4.0 {
                    wsum += wgt;
                    acc_u += wgt * vu;
                    acc_v += wgt * vv;
                }
            }
            if wsum <= 1e-12 {
                // Second chance: no covering patch explains this pixel (its
                // own patches straddled a motion boundary, or it is occluded).
                // Borrow motion *hypotheses* from the wider 5×5 patch
                // neighbourhood and keep whichever photometrically fit —
                // the gate means a hypothesis can never leak across a content
                // edge, unlike smoothing the flow field harder.
                for oj in -2i32..=2 {
                    let cj = gj + oj;
                    if cj < 0 || cj >= patches.npy as i32 {
                        continue;
                    }
                    for oi in -2i32..=2 {
                        let ci = gi + oi;
                        if ci < 0 || ci >= patches.npx as i32 {
                            continue;
                        }
                        let p = cj as usize * patches.npx + ci as usize;
                        if patches.valid[p] == 0 {
                            continue;
                        }
                        let err =
                            b.sample(x as f32 + patches.u[p], y as f32 + patches.v[p]) - a.data[i];
                        let wgt = (-(err * err) / SIGMA2).exp();
                        wsum += wgt;
                        acc_u += wgt * patches.u[p];
                        acc_v += wgt * patches.v[p];
                    }
                }
            }
            if wsum > 1e-12 {
                u[i] = acc_u / wsum;
                v[i] = acc_v / wsum;
                valid[i] = 1;
            } else {
                // Nothing explains this pixel (occlusion / textureless):
                // keep the coarse initialisation and mark it suspect.
                u[i] = init_u[i];
                v[i] = init_v[i];
            }
        }
    }
    (u, v, valid)
}

/// Smoothing (impl note §1 step 4): one 3×3 edge-aware blur. Neighbours count
/// less the more their luma differs (flow must not bleed across image edges)
/// and the more their *flow* differs (vectors from the two sides of a motion
/// boundary must never average into a phantom in-between motion).
fn smooth(a: &Gray, u: &[f32], v: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (w, h) = (a.w, a.h);
    let mut su = vec![0f32; w * h];
    let mut sv = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let c = a.at(x, y);
            let (mut acc_u, mut acc_v, mut wsum) = (0f32, 0f32, 0f32);
            for oy in -1i32..=1 {
                for ox in -1i32..=1 {
                    let qx = (x as i32 + ox).clamp(0, w as i32 - 1) as usize;
                    let qy = (y as i32 + oy).clamp(0, h as i32 - 1) as usize;
                    let d = a.at(qx, qy) - c;
                    let q = qy * w + qx;
                    let fd = (u[q] - u[i]) * (u[q] - u[i]) + (v[q] - v[i]) * (v[q] - v[i]);
                    let wgt = (-(d * d) / SIGMA2).exp() * (-fd / FLOW_SIGMA2).exp();
                    wsum += wgt;
                    acc_u += wgt * u[q];
                    acc_v += wgt * v[q];
                }
            }
            su[i] = acc_u / wsum; // centre weight is 1, so wsum ≥ 1
            sv[i] = acc_v / wsum;
        }
    }
    (su, sv)
}

/// Build the pyramid: L0 is the input, then box-downsample ×2 until the next
/// level would drop under ~16 px in either dimension.
pub(crate) fn build_pyramid(g: &Gray) -> Vec<Gray> {
    let mut p = vec![g.clone()];
    loop {
        let last = &p[p.len() - 1];
        if (last.w / 2).max(1).min((last.h / 2).max(1)) < MIN_LEVEL_DIM {
            break;
        }
        let next = downsample(last);
        p.push(next);
    }
    p
}

/// Coarse-to-fine DIS over prebuilt pyramids (`grads` are `pa`'s Sobel fields
/// per level — the template side).
fn flow_core(pa: &[Gray], pb: &[Gray], grads: &[(Vec<f32>, Vec<f32>)]) -> FlowField {
    let (w0, h0) = (pa[0].w, pa[0].h);
    if w0 < PATCH || h0 < PATCH {
        return FlowField::zeroed(w0, h0); // too small to search — degrade
    }
    let levels = pa.len();
    let top = &pa[levels - 1];
    let mut du = vec![0f32; top.w * top.h];
    let mut dv = vec![0f32; top.w * top.h];
    let mut valid = vec![0u8; top.w * top.h];
    let (mut pw, mut ph) = (top.w, top.h);
    for lvl in (0..levels).rev() {
        let (a, b) = (&pa[lvl], &pb[lvl]);
        if a.w != pw || a.h != ph {
            du = upsample_flow(&du, pw, ph, a.w, a.h);
            dv = upsample_flow(&dv, pw, ph, a.w, a.h);
        }
        let (gx, gy) = (&grads[lvl].0, &grads[lvl].1);
        let patches = inverse_search(a, b, gx, gy, &du, &dv);
        let (tu, tv, tvalid) = densify(a, b, &patches, &du, &dv);
        let (su, sv) = smooth(a, &tu, &tv);
        du = su;
        dv = sv;
        valid = tvalid;
        pw = a.w;
        ph = a.h;
    }
    FlowField {
        w: w0,
        h: h0,
        u: du,
        v: dv,
        valid,
    }
}

/// Dense forward flow A→B by DIS (coarse-to-fine inverse search).
pub fn flow(a: &Gray, b: &Gray) -> FlowField {
    if a.w < PATCH || a.h < PATCH || a.w != b.w || a.h != b.h {
        return FlowField::zeroed(a.w, a.h);
    }
    let pa = build_pyramid(a);
    let pb = build_pyramid(b);
    let grads: Vec<(Vec<f32>, Vec<f32>)> = pa.iter().map(sobel).collect();
    flow_core(&pa, &pb, &grads)
}

/// Both directions at once (A→B, B→A), sharing the pyramids — the impl note's
/// "reuse everything; it is 2× cost".
pub fn flow_pair(a: &Gray, b: &Gray) -> (FlowField, FlowField) {
    if a.w < PATCH || a.h < PATCH || a.w != b.w || a.h != b.h {
        return (FlowField::zeroed(a.w, a.h), FlowField::zeroed(b.w, b.h));
    }
    let pa = build_pyramid(a);
    let pb = build_pyramid(b);
    let ga: Vec<(Vec<f32>, Vec<f32>)> = pa.iter().map(sobel).collect();
    let gb: Vec<(Vec<f32>, Vec<f32>)> = pb.iter().map(sobel).collect();
    (flow_core(&pa, &pb, &ga), flow_core(&pb, &pa, &gb))
}

/// Forward–backward occlusion mask (impl note §2), on `f`'s pixel grid:
/// 1 where the pixel has no consistent match in the other frame (it got
/// covered, or its flow was untrustworthy). Dilated by one pixel, as the
/// consistency test under-detects at exact boundaries.
pub fn occlusion(f: &FlowField, g: &FlowField) -> Vec<u8> {
    dilate3(&occlusion_raw(f, g), f.w, f.h)
}

/// The §2 consistency test itself, before the safety dilation (the §6.1
/// accuracy test measures this; synthesis uses the dilated form).
fn occlusion_raw(f: &FlowField, g: &FlowField) -> Vec<u8> {
    let (w, h) = (f.w, f.h);
    let n = w * h;
    if g.w != w || g.h != h || f.u.len() != n || g.u.len() != n {
        return vec![0; n]; // mismatched fields: claim nothing
    }
    let mut raw = vec![0u8; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if f.valid[i] == 0 {
                raw[i] = 1; // invalid-patch bits count as occluded (§2)
                continue;
            }
            let (fu, fv) = (f.u[i], f.v[i]);
            let gu = sample_scalar(&g.u, w, h, x as f32 + fu, y as f32 + fv);
            let gv = sample_scalar(&g.v, w, h, x as f32 + fu, y as f32 + fv);
            let cn = ((fu + gu) * (fu + gu) + (fv + gv) * (fv + gv)).sqrt();
            let fn_ = (fu * fu + fv * fv).sqrt();
            let gn = (gu * gu + gv * gv).sqrt();
            let thr = (OCC_REL * (fn_ + gn)).max(OCC_ABS);
            raw[i] = u8::from(cn > thr);
        }
    }
    raw
}

/// A smooth per-pixel **confidence** in 0..1 for the forward flow `f` measured
/// against its backward twin `g` (docs/08 §3.2, FX-19): 1 where the two agree
/// (a trustworthy vector), tapering to 0 where they disagree — occlusion, a
/// motion boundary, or textureless drift. The *smooth* cousin of the binary
/// [`occlusion`] mask, with **no hard threshold**: Fast motion blur scales each
/// pixel's streak length by this, so unreliable regions fade toward unblurred
/// gradually instead of leaving a hard cut. The raw consistency (1 at a perfect
/// match, ramping linearly to 0 at the same rel/abs mismatch the binary test
/// cuts at, an invalid patch fully suspect) is then 3×3 box-blurred, so the
/// falloff widens by a pixel and has no seam. Deterministic and side-effect
/// free, so preview and export derive the identical field (K-031). A
/// mismatched-size `g` returns all-1 (claim nothing suspect — degrade to the
/// plain smear, never a fault).
pub fn confidence(f: &FlowField, g: &FlowField) -> Vec<f32> {
    let (w, h) = (f.w, f.h);
    let n = w * h;
    if g.w != w || g.h != h || f.u.len() != n || g.u.len() != n {
        return vec![1.0; n];
    }
    let mut raw = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if f.valid[i] == 0 {
                raw[i] = 0.0; // nothing explained this patch: fully suspect
                continue;
            }
            let (fu, fv) = (f.u[i], f.v[i]);
            let gu = sample_scalar(&g.u, w, h, x as f32 + fu, y as f32 + fv);
            let gv = sample_scalar(&g.v, w, h, x as f32 + fu, y as f32 + fv);
            let cn = ((fu + gu) * (fu + gu) + (fv + gv) * (fv + gv)).sqrt();
            let fn_ = (fu * fu + fv * fv).sqrt();
            let gn = (gu * gu + gv * gv).sqrt();
            // Same rel/abs scale the occlusion cut-off uses (§2): cn == 0 → 1,
            // cn == thr → 0, linear and clamped between. Smooth, no step.
            let thr = (OCC_REL * (fn_ + gn)).max(OCC_ABS);
            raw[i] = (1.0 - cn / thr).clamp(0.0, 1.0);
        }
    }
    // 3×3 box blur: ramp the confidence over a pixel so the streak-length taper
    // has no visible seam.
    let mut out = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            let (mut acc, mut cnt) = (0f32, 0f32);
            for oy in -1i32..=1 {
                for ox in -1i32..=1 {
                    let qx = (x as i32 + ox).clamp(0, w as i32 - 1) as usize;
                    let qy = (y as i32 + oy).clamp(0, h as i32 - 1) as usize;
                    acc += raw[qy * w + qx];
                    cnt += 1.0;
                }
            }
            out[y * w + x] = acc / cnt;
        }
    }
    out
}

/// 3×3 max filter (grow a mask by one pixel).
fn dilate3(mask: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut m = 0u8;
            for oy in -1i32..=1 {
                for ox in -1i32..=1 {
                    let qx = (x as i32 + ox).clamp(0, w as i32 - 1) as usize;
                    let qy = (y as i32 + oy).clamp(0, h as i32 - 1) as usize;
                    m = m.max(mask[qy * w + qx]);
                }
            }
            out[y * w + x] = m;
        }
    }
    out
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

/// Plain crossfade — the soft-failure floor everything degrades to.
fn crossfade(a: &[u8], b: &[u8], phi: f32) -> Vec<u8> {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            (f32::from(*x) * (1.0 - phi) + f32::from(*y) * phi)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// Synthesise the frame at phase `phi` ∈ [0,1] between A and B (impl note §3):
/// backward-warp both endpoints along their flow and blend with occlusion-aware
/// weights. `phi` = 0 returns A, 1 returns B, bit-exactly. `fwd` is flow A→B,
/// `bwd` is B→A, both at the frames' full resolution. Where **both** frames
/// lost sight of a pixel, it falls back to a plain crossfade — the documented
/// graceful degradation.
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
    let n = w * h;
    // Anything inconsistent degrades to a crossfade rather than faulting.
    if fwd.w != w
        || fwd.h != h
        || bwd.w != w
        || bwd.h != h
        || fwd.u.len() != n
        || bwd.u.len() != n
        || a.len() < n * 4
        || b.len() < n * 4
        || a.len() != b.len()
    {
        return crossfade(a, b, phi);
    }
    // Occlusion masks (§2): occ_a marks A-pixels with no match in B (content
    // that gets covered); occ_b marks B-pixels with no match in A (content
    // that gets revealed).
    let occ_a = occlusion(fwd, bwd);
    let occ_b = occlusion(bwd, fwd);
    let mut out = vec![0u8; n * 4];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (xf, yf) = (x as f32, y as f32);
            // One fixed-point iteration toward the flow *at the destination*
            // (§3): sample the field, step along it, sample again.
            let (f0u, f0v) = (fwd.u[i], fwd.v[i]);
            let f1u = sample_scalar(&fwd.u, w, h, xf - phi * f0u, yf - phi * f0v);
            let f1v = sample_scalar(&fwd.v, w, h, xf - phi * f0u, yf - phi * f0v);
            let (b0u, b0v) = (bwd.u[i], bwd.v[i]);
            let b1u = sample_scalar(&bwd.u, w, h, xf - (1.0 - phi) * b0u, yf - (1.0 - phi) * b0v);
            let b1v = sample_scalar(&bwd.v, w, h, xf - (1.0 - phi) * b0u, yf - (1.0 - phi) * b0v);
            let sa = sample_rgba(a, w, h, xf - phi * f1u, yf - phi * f1v);
            // The backward field points B→A; the forward velocity seen from
            // B's grid is its negation, hence the minus sign here too.
            let sb = sample_rgba(b, w, h, xf - (1.0 - phi) * b1u, yf - (1.0 - phi) * b1v);
            let (oa, ob) = (occ_a[i], occ_b[i]);
            if oa == 1 && ob == 1 {
                // Revealed background with no source in either frame: plain
                // blend, identical to Frame-Mix (§3 soft failure).
                for c in 0..4 {
                    let la = f32::from(a[i * 4 + c]);
                    let lb = f32::from(b[i * 4 + c]);
                    out[i * 4 + c] = (la * (1.0 - phi) + lb * phi).round().clamp(0.0, 255.0) as u8;
                }
            } else {
                // A's warp is trusted unless the content only exists in B
                // (revealed, occ_b), and vice versa (§3 weights).
                let wa = (1.0 - phi) * (1.0 - f32::from(ob)) + SYNTH_EPS;
                let wb = phi * (1.0 - f32::from(oa)) + SYNTH_EPS;
                for c in 0..4 {
                    out[i * 4 + c] = ((wa * sa[c] + wb * sb[c]) / (wa + wb))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}

/// Luma pair at the working resolution for `quality` (impl note §1: default
/// half). Tiny frames stay at full size — halving would starve the pyramid.
fn grays_at(a: &[u8], b: &[u8], w: usize, h: usize, quality: FlowQuality) -> (Gray, Gray, bool) {
    let ga = to_gray(a, w, h);
    let gb = to_gray(b, w, h);
    if quality == FlowQuality::Half && w.min(h) >= 64 {
        (downsample(&ga), downsample(&gb), true)
    } else {
        (ga, gb, false)
    }
}

/// End-to-end on the CPU: the flow-interpolated frame at `phi` between RGBA
/// frames `a` and `b` (`w×h`), at the given working quality. This is the
/// K-019 reference path (export with no capable GPU).
pub fn interpolate_at(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    phi: f32,
    quality: FlowQuality,
) -> Vec<u8> {
    if phi <= 0.0 {
        return a.to_vec();
    }
    if phi >= 1.0 {
        return b.to_vec();
    }
    let (ga, gb, halved) = grays_at(a, b, w, h, quality);
    let (fwd, bwd) = flow_pair(&ga, &gb);
    let (fwd, bwd) = if halved {
        (upsample_field(&fwd, w, h), upsample_field(&bwd, w, h))
    } else {
        (fwd, bwd)
    };
    synthesize(a, b, w, h, &fwd, &bwd, phi)
}

/// CPU convenience at the default (half) quality — what the `Flow` retiming
/// policy calls when no engine is held.
pub fn interpolate(a: &[u8], b: &[u8], w: usize, h: usize, phi: f32) -> Vec<u8> {
    interpolate_at(a, b, w, h, phi, FlowQuality::Half)
}

/// The backend-choosing engine callers hold on to: WGSL DIS on a GPU when one
/// is available, the CPU oracle otherwise. A GPU failure mid-flight degrades
/// permanently to CPU for this engine — never a fault (the K-018 spirit:
/// device trouble costs speed, not the frame).
pub struct FlowEngine {
    gpu: Option<gpu::GpuFlow>,
}

impl FlowEngine {
    /// Try for a GPU of our own (headless); fall back to CPU quietly.
    pub fn new_auto() -> Self {
        match lumit_gpu::GpuContext::headless() {
            Ok(ctx) => Self::with_context(&ctx),
            Err(_) => Self::cpu(),
        }
    }

    /// Share an existing device (the app's). Falls back to CPU if the flow
    /// pipelines cannot be built on it.
    pub fn with_context(ctx: &lumit_gpu::GpuContext) -> Self {
        FlowEngine {
            gpu: gpu::GpuFlow::new(ctx).ok(),
        }
    }

    /// CPU only (tests, or by explicit choice).
    pub fn cpu() -> Self {
        FlowEngine { gpu: None }
    }

    /// Which backend this engine currently uses.
    pub fn backend(&self) -> &'static str {
        if self.gpu.is_some() {
            "dis-gpu"
        } else {
            "dis-cpu"
        }
    }

    /// Both flow directions at the frames' own resolution, on whichever
    /// backend is live.
    pub fn flow_pair(&mut self, a: &Gray, b: &Gray) -> (FlowField, FlowField) {
        if let Some(g) = self.gpu.as_mut() {
            match g.flow_pair(a, b) {
                Ok(pair) => return pair,
                Err(_) => self.gpu = None, // degrade to CPU from here on
            }
        }
        flow_pair(a, b)
    }

    /// The flow-interpolated frame at `phi`, at the given working quality.
    pub fn interpolate_at(
        &mut self,
        a: &[u8],
        b: &[u8],
        w: usize,
        h: usize,
        phi: f32,
        quality: FlowQuality,
    ) -> Vec<u8> {
        if phi <= 0.0 {
            return a.to_vec();
        }
        if phi >= 1.0 {
            return b.to_vec();
        }
        let (ga, gb, halved) = grays_at(a, b, w, h, quality);
        let (fwd, bwd) = self.flow_pair(&ga, &gb);
        let (fwd, bwd) = if halved {
            (upsample_field(&fwd, w, h), upsample_field(&bwd, w, h))
        } else {
            (fwd, bwd)
        };
        synthesize(a, b, w, h, &fwd, &bwd, phi)
    }

    /// Default-quality interpolation (half working resolution).
    pub fn interpolate(&mut self, a: &[u8], b: &[u8], w: usize, h: usize, phi: f32) -> Vec<u8> {
        self.interpolate_at(a, b, w, h, phi, FlowQuality::Half)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod testutil {
    use super::Gray;

    /// Deterministic integer hash → [0,1) (no rand dependency; same every run).
    fn hash01(ix: i32, iy: i32, seed: u32) -> f32 {
        let mut n = (ix as u32)
            .wrapping_mul(1_619)
            .wrapping_add((iy as u32).wrapping_mul(31_337))
            .wrapping_add(seed.wrapping_mul(1_013));
        n = (n >> 13) ^ n;
        n = n
            .wrapping_mul(
                n.wrapping_mul(n)
                    .wrapping_mul(60_493)
                    .wrapping_add(19_990_303),
            )
            .wrapping_add(1_376_312_589);
        (n & 0x00ff_ffff) as f32 / 16_777_216.0
    }

    fn smoothstep(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }

    /// Value noise: smooth interpolation between lattice hashes.
    fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
        let ix = x.floor();
        let iy = y.floor();
        let fx = smoothstep(x - ix);
        let fy = smoothstep(y - iy);
        let (ix, iy) = (ix as i32, iy as i32);
        let a = hash01(ix, iy, seed);
        let b = hash01(ix + 1, iy, seed);
        let c = hash01(ix, iy + 1, seed);
        let d = hash01(ix + 1, iy + 1, seed);
        (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
    }

    /// A Perlin-style multi-octave texture, evaluated continuously — sampling
    /// it at shifted/rotated coordinates gives exact ground-truth motion.
    pub fn perlin(x: f32, y: f32, seed: u32) -> f32 {
        0.40 * value_noise(x / 64.0, y / 64.0, seed)
            + 0.35 * value_noise(x / 24.0, y / 24.0, seed.wrapping_add(7))
            + 0.25 * value_noise(x / 10.0, y / 10.0, seed.wrapping_add(13))
    }

    /// Fine-grained detail octave — mix into `perlin` when a scene needs real
    /// 2D texture at patch (8 px) scale, as photographed surfaces have.
    pub fn detail(x: f32, y: f32, seed: u32) -> f32 {
        value_noise(x / 8.0, y / 8.0, seed.wrapping_add(23))
    }

    /// An anti-aliased checkerboard, also continuous.
    pub fn checker(x: f32, y: f32, cell: f32) -> f32 {
        let s = (std::f32::consts::PI * x / cell).sin() * (std::f32::consts::PI * y / cell).sin();
        0.5 + 0.45 * (s * 6.0).clamp(-1.0, 1.0)
    }

    /// Render a continuous scalar field into a Gray image.
    pub fn render(w: usize, h: usize, f: impl Fn(f32, f32) -> f32) -> Gray {
        let mut data = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                data[y * w + x] = f(x as f32, y as f32).clamp(0.0, 1.0);
            }
        }
        Gray { w, h, data }
    }

    /// Mean endpoint error against an analytic flow, over the interior
    /// (borders are unknowable — content enters/leaves the frame there).
    pub fn mean_epe(
        f: &super::FlowField,
        margin: usize,
        truth: impl Fn(usize, usize) -> (f32, f32),
    ) -> f32 {
        let (mut sum, mut n) = (0.0f64, 0usize);
        for y in margin..f.h - margin {
            for x in margin..f.w - margin {
                let i = y * f.w + x;
                let (tu, tv) = truth(x, y);
                let e = f64::from(((f.u[i] - tu).powi(2) + (f.v[i] - tv).powi(2)).sqrt());
                sum += e;
                n += 1;
            }
        }
        (sum / n as f64) as f32
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::testutil::*;
    use super::*;

    /// A smooth, well-textured test image (sum of a few non-aliasing sines).
    fn texture(w: usize, h: usize, dx: f32, dy: f32) -> Gray {
        render(w, h, |x, y| {
            let fx = x - dx;
            let fy = y - dy;
            0.5 + 0.2 * (fx * 0.21).sin() * (fy * 0.17).cos()
                + 0.15 * (fx * 0.11 + fy * 0.13).sin()
                + 0.1 * (fx * 0.37).cos()
        })
    }

    #[test]
    fn recovers_a_known_translation() {
        let (w, h) = (96, 96);
        let a = texture(w, h, 0.0, 0.0);
        let b = texture(w, h, 3.0, 2.0); // content shifted by (3, 2)
        let f = flow(&a, &b);
        let epe = mean_epe(&f, 16, |_, _| (3.0, 2.0));
        assert!(epe < 0.3, "mean endpoint error too high: {epe}");
    }

    /// Impl note §6.1: translation ≤ 32 px recovered at half resolution to
    /// < 0.3 px mean endpoint error (measured in full-res pixels).
    #[test]
    fn large_translation_at_half_res() {
        let (w, h) = (256, 256);
        let (dx, dy) = (26.0f32, -14.0f32); // ‖d‖ ≈ 29.5 px ≤ 32
        let a = render(w, h, |x, y| perlin(x, y, 1));
        let b = render(w, h, |x, y| perlin(x - dx, y - dy, 1));
        let (ha, hb) = (downsample(&a), downsample(&b));
        let (f, _) = flow_pair(&ha, &hb);
        // Error measured at the working (half) resolution, in its own pixels.
        let epe = mean_epe(&f, 24, |_, _| (dx / 2.0, dy / 2.0));
        assert!(epe < 0.3, "mean endpoint error too high at half res: {epe}");
    }

    /// Impl note §6.1: a known rotation field.
    #[test]
    fn recovers_a_known_rotation() {
        let (w, h) = (192, 192);
        let (cx, cy) = (95.5f32, 95.5f32);
        let ang = 4.0f32.to_radians();
        let (sin, cos) = ang.sin_cos();
        let a = render(w, h, |x, y| perlin(x, y, 2));
        // B is A rotated by `ang` about the centre: B(x) = A(R⁻¹(x−c)+c).
        let b = render(w, h, |x, y| {
            let (rx, ry) = (x - cx, y - cy);
            perlin(cx + cos * rx + sin * ry, cy - sin * rx + cos * ry, 2)
        });
        let f = flow(&a, &b);
        // Analytic flow: u(x) = R(x−c) − (x−c).
        let epe = mean_epe(&f, 24, |x, y| {
            let (rx, ry) = (x as f32 - cx, y as f32 - cy);
            (cos * rx - sin * ry - rx, sin * rx + cos * ry - ry)
        });
        assert!(epe < 0.3, "mean endpoint error too high on rotation: {epe}");
    }

    /// Impl note §6.1: the checkerboard case (aperture-prone texture).
    #[test]
    fn recovers_translation_on_a_checkerboard() {
        let (w, h) = (192, 192);
        let (dx, dy) = (6.0f32, 4.0f32);
        let a = render(w, h, |x, y| checker(x, y, 16.0));
        let b = render(w, h, |x, y| checker(x - dx, y - dy, 16.0));
        let f = flow(&a, &b);
        let epe = mean_epe(&f, 24, |_, _| (dx, dy));
        assert!(epe < 0.3, "mean endpoint error too high on checker: {epe}");
    }

    /// Impl note §6.1: occlusion mask of a sliding square vs the analytic
    /// answer, ≥ 90% IoU.
    #[test]
    fn occlusion_mask_matches_a_sliding_square() {
        let (w, h) = (384, 288);
        let (sq_x, sq_y, sq) = (96usize, 64usize, 144usize);
        // Diagonal slide: a motion-parallel silhouette edge is aperture-blind
        // (the boundary line itself does not move), so the square must move
        // off-axis for the mask edges to be well-posed everywhere.
        let (dx, dy) = (24usize, 16usize);
        let in_sq = |x: usize, y: usize, ox: usize, oy: usize| {
            x >= ox && x < ox + sq && y >= oy && y < oy + sq
        };
        // Textures with detail at patch scale (real surfaces are not smooth).
        // Two independently-seeded detail lattices so no 8×8 window is flat.
        let busy = |x: f32, y: f32, seed: u32| {
            0.70 * perlin(x, y, seed)
                + 0.15 * detail(x, y, seed)
                + 0.15 * detail(x + 3.5, y + 3.5, seed.wrapping_add(101))
        };
        let scene = |sx: usize, sy: usize| {
            render(w, h, move |x, y| {
                let (xi, yi) = (x as usize, y as usize);
                if in_sq(xi, yi, sq_x + sx, sq_y + sy) {
                    // The square carries its own texture, anchored to itself,
                    // and sits brighter than the background — a real object
                    // has a silhouette the tracker can hold on to. Full
                    // contrast: mismatched content must photometrically
                    // separate from matched content (σ = 0.08).
                    0.35 + 0.65 * busy(x - sx as f32, y - sy as f32, 40)
                } else {
                    busy(x, y, 41)
                }
            })
        };
        let a = scene(0, 0);
        let b = scene(dx, dy);
        let (f, g) = flow_pair(&a, &b);
        // The raw §2 test is what accuracy is measured on; the 1 px dilation
        // is a synthesis-safety margin whose perimeter alone would eat the
        // whole IoU error budget of a strip this size.
        let occ_a = occlusion_raw(&f, &g);
        // Analytic: A-pixels occluded in B = background covered by the moved
        // square = S_B \ S_A.
        let (mut inter, mut uni) = (0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                let truth = in_sq(x, y, sq_x + dx, sq_y + dy) && !in_sq(x, y, sq_x, sq_y);
                let got = occ_a[y * w + x] == 1;
                if truth && got {
                    inter += 1;
                }
                if truth || got {
                    uni += 1;
                }
            }
        }
        let iou = inter as f64 / uni.max(1) as f64;
        assert!(iou >= 0.9, "occlusion IoU too low: {iou}");
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
        let (w, h) = (96, 96);
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
            for y in 16..h - 16 {
                for x in 16..w - 16 {
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

    /// Engine crates never fault: degenerate inputs degrade, not crash.
    #[test]
    fn tiny_frames_degrade_gracefully() {
        let (w, h) = (6, 6);
        let a = vec![10u8; w * h * 4];
        let b = vec![200u8; w * h * 4];
        let f = flow(&to_gray(&a, w, h), &to_gray(&b, w, h));
        assert!(f.u.iter().all(|&u| u == 0.0));
        assert!(f.valid.iter().all(|&v| v == 0));
        let mid = interpolate(&a, &b, w, h, 0.5);
        assert_eq!(mid.len(), w * h * 4);
    }

    /// Same inputs → same flow, bit for bit (docs/14 §3 determinism).
    #[test]
    fn flow_is_deterministic() {
        let (w, h) = (128, 96);
        let a = render(w, h, |x, y| perlin(x, y, 5));
        let b = render(w, h, |x, y| perlin(x - 4.3, y + 2.1, 5));
        let (f1, g1) = flow_pair(&a, &b);
        let (f2, g2) = flow_pair(&a, &b);
        assert_eq!(f1.u, f2.u);
        assert_eq!(f1.v, f2.v);
        assert_eq!(f1.valid, f2.valid);
        assert_eq!(g1.u, g2.u);
        assert_eq!(g1.v, g2.v);
    }

    // confidence (docs/08 §3.2, FX-19): high where forward and backward flow
    // agree, low where they disagree, always in 0..1, and a graceful all-1 for a
    // mismatched-size pair (the smooth cut-free replacement for a hard gate).
    #[test]
    fn confidence_is_high_for_a_consistent_pair_and_low_when_they_disagree() {
        let (w, h) = (4usize, 4usize);
        let n = w * h;
        let field = |u: f32, v: f32, valid: u8| FlowField {
            w,
            h,
            u: vec![u; n],
            v: vec![v; n],
            valid: vec![valid; n],
        };
        // Forward (1,0), backward (-1,0): f + g(x+f) ≈ 0 → near-full confidence.
        let f = field(1.0, 0.0, 1);
        let g = field(-1.0, 0.0, 1);
        let c = confidence(&f, &g);
        assert_eq!(c.len(), n);
        assert!(
            c.iter().all(|&x| (0.0..=1.0).contains(&x) && x > 0.9),
            "a consistent pair is near-full confidence"
        );
        // Backward pointing the SAME way: f + g is large → confidence drops.
        let c2 = confidence(&f, &field(1.0, 0.0, 1));
        assert!(
            c2.iter().all(|&x| x < 0.9),
            "an inconsistent pair loses confidence"
        );
        // An all-invalid forward is fully suspect everywhere (0 after the blur).
        let c3 = confidence(&field(1.0, 0.0, 0), &g);
        assert!(c3.iter().all(|&x| x == 0.0));
        // A mismatched-size twin degrades to all-1 (claim nothing suspect).
        let small = FlowField {
            w: 2,
            h: 2,
            u: vec![0.0; 4],
            v: vec![0.0; 4],
            valid: vec![1; 4],
        };
        assert!(confidence(&f, &small).iter().all(|&x| x == 1.0));
    }

    // ---- The WGSL backend against the CPU oracle (impl note §6.5) ----

    fn gpu_flow() -> Option<gpu::GpuFlow> {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            eprintln!("skipping: no GPU adapter available");
            return None;
        };
        match gpu::GpuFlow::new(&ctx) {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("skipping: flow pipelines failed: {e}");
                None
            }
        }
    }

    /// Mean absolute difference between two fields, per component.
    fn mean_abs_diff(a: &FlowField, b: &FlowField) -> f32 {
        let n = a.u.len();
        let mut sum = 0f64;
        for i in 0..n {
            sum += f64::from((a.u[i] - b.u[i]).abs()) + f64::from((a.v[i] - b.v[i]).abs());
        }
        (sum / (2 * n) as f64) as f32
    }

    /// The CPU implementation is the oracle: the WGSL backend must match it
    /// within 1e-3 on the analytic scenes (impl note §6.5).
    #[test]
    fn gpu_matches_the_cpu_oracle() {
        let Some(mut g) = gpu_flow() else { return };
        let (w, h) = (192, 160);
        // Translation and rotation, same scenes the CPU tests use.
        let scenes = [
            (
                render(w, h, |x, y| perlin(x, y, 1)),
                render(w, h, |x, y| perlin(x - 7.3, y - 3.9, 1)),
            ),
            (
                render(w, h, |x, y| perlin(x, y, 2)),
                render(w, h, |x, y| {
                    let (rx, ry) = (x - 95.5, y - 79.5);
                    let ang = 4.0f32.to_radians();
                    perlin(
                        95.5 + ang.cos() * rx + ang.sin() * ry,
                        79.5 - ang.sin() * rx + ang.cos() * ry,
                        2,
                    )
                }),
            ),
        ];
        for (i, (a, b)) in scenes.iter().enumerate() {
            let (cf, cg) = flow_pair(a, b);
            let (gf, gg) = g.flow_pair(a, b).unwrap();
            let (df, dg) = (mean_abs_diff(&cf, &gf), mean_abs_diff(&cg, &gg));
            assert!(df < 1e-3, "scene {i}: fwd CPU/GPU diff {df}");
            assert!(dg < 1e-3, "scene {i}: bwd CPU/GPU diff {dg}");
            let same_valid = cf
                .valid
                .iter()
                .zip(&gf.valid)
                .filter(|(a, b)| a == b)
                .count();
            assert!(
                same_valid as f64 / cf.valid.len() as f64 > 0.999,
                "scene {i}: validity masks diverge"
            );
        }
    }

    /// Same inputs → same flow on the GPU too, bit for bit against itself.
    #[test]
    fn gpu_flow_is_deterministic() {
        let Some(mut g) = gpu_flow() else { return };
        let (w, h) = (160, 128);
        let a = render(w, h, |x, y| perlin(x, y, 9));
        let b = render(w, h, |x, y| perlin(x - 5.2, y + 3.4, 9));
        let (f1, g1) = g.flow_pair(&a, &b).unwrap();
        let (f2, g2) = g.flow_pair(&a, &b).unwrap();
        assert_eq!(f1.u, f2.u);
        assert_eq!(f1.v, f2.v);
        assert_eq!(f1.valid, f2.valid);
        assert_eq!(g1.u, g2.u);
        assert_eq!(g1.v, g2.v);
    }

    /// The engine degrades, interpolates, and honours the endpoint contract
    /// whichever backend it holds.
    #[test]
    fn engine_interpolates_on_any_backend() {
        let mut eng = FlowEngine::new_auto();
        eprintln!("engine backend: {}", eng.backend());
        let (w, h) = (96, 96);
        let a = vec![40u8; w * h * 4];
        let b = vec![200u8; w * h * 4];
        assert_eq!(eng.interpolate(&a, &b, w, h, 0.0), a);
        assert_eq!(eng.interpolate(&a, &b, w, h, 1.0), b);
        let mid = eng.interpolate(&a, &b, w, h, 0.5);
        assert_eq!(mid.len(), w * h * 4);
        // A CPU-only engine must behave identically to the free function.
        let mut cpu = FlowEngine::cpu();
        assert_eq!(cpu.backend(), "dis-cpu");
        assert_eq!(
            cpu.interpolate(&a, &b, w, h, 0.5),
            interpolate(&a, &b, w, h, 0.5)
        );
    }

    /// Perf numbers (impl note §6.5: flow pair ≤ 4 ms at half-res 1080p on
    /// the reference GPU). Run by hand:
    /// `cargo test -p lumit-flow --release bench_flow -- --ignored --nocapture`
    #[test]
    #[ignore = "manual benchmark; prints timings"]
    fn bench_flow_1080p() {
        let Some(mut g) = gpu_flow() else { return };
        // 1080p at the default Half working quality = 960×540 flow fields.
        let (w, h) = (960, 540);
        let a = render(w, h, |x, y| perlin(x, y, 3));
        let b = render(w, h, |x, y| perlin(x - 9.7, y + 4.3, 3));
        for _ in 0..3 {
            let _ = g.flow_pair(&a, &b); // warm-up (plan + pipeline caches)
        }
        let runs = 20;
        let t0 = std::time::Instant::now();
        for _ in 0..runs {
            let _ = g.flow_pair(&a, &b);
        }
        let per_pair = t0.elapsed() / runs;
        eprintln!("gpu flow pair (960x540): {per_pair:?}");

        let t0 = std::time::Instant::now();
        let _ = flow_pair(&a, &b);
        eprintln!("cpu flow pair (960x540): {:?}", t0.elapsed());

        // End-to-end 1080p interpolate (gray + halve + flow + synthesis).
        let px = |g: &Gray| -> Vec<u8> {
            let mut f = vec![0u8; g.w * g.h * 4];
            for i in 0..g.w * g.h {
                let v = (g.data[i] * 255.0).round().clamp(0.0, 255.0) as u8;
                f[i * 4] = v;
                f[i * 4 + 1] = v;
                f[i * 4 + 2] = v;
                f[i * 4 + 3] = 255;
            }
            f
        };
        let (fw, fh) = (1920, 1080);
        let fa = px(&render(fw, fh, |x, y| perlin(x, y, 4)));
        let fb = px(&render(fw, fh, |x, y| perlin(x - 9.7, y + 4.3, 4)));
        let mut eng = FlowEngine::new_auto();
        eprintln!("engine backend: {}", eng.backend());
        let _ = eng.interpolate(&fa, &fb, fw, fh, 0.5); // warm-up
        let t0 = std::time::Instant::now();
        let _ = eng.interpolate(&fa, &fb, fw, fh, 0.5);
        eprintln!(
            "end-to-end 1080p interpolate at phi 0.5: {:?}",
            t0.elapsed()
        );
    }
}

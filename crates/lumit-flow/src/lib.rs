//! Optical flow and frame synthesis (docs/impl/optical-flow.md). This file is
//! the CPU DIS implementation — the deterministic oracle the WGSL
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
pub mod synth;

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
/// How much confidence a pixel keeps when the flow does not explain it but the
/// two directions still agree (FX-19). Not zero: a hard cut-off is exactly the
/// visible seam that decision existed to remove.
const VALID_DIM: f32 = 0.4;

// --- Variational refinement (impl note §1 step 4) ---------------------------
// The paper's weights, unchanged: intensity constancy, gradient constancy,
// smoothness. Gradient constancy is weighted as heavily as smoothness because
// it is the term that survives a brightness change — a muzzle flash is a step
// in intensity across a moving frame, which plain intensity constancy reads as
// motion everywhere.
/// Intensity-constancy weight σ (Kroeger et al., §3.3).
pub(crate) const VR_SIGMA: f32 = 5.0;
/// Gradient-constancy weight γ.
pub(crate) const VR_GAMMA: f32 = 10.0;
/// Smoothness weight α.
pub(crate) const VR_ALPHA: f32 = 10.0;
/// Robust penaliser floor: Ψ(a²) = √(a² + ε²).
pub(crate) const VR_EPS2: f32 = 0.001 * 0.001;
/// Successive over-relaxation factor. Above 1 the sweep overshoots on purpose,
/// which is what makes it converge in a handful of passes instead of dozens;
/// 1.6 is the usual choice for this class of system and is stable here.
pub(crate) const VR_OMEGA: f32 = 1.6;
/// SOR sweeps per fixed-point iteration (the paper's θ_vi = 5).
pub(crate) const VR_SOR: usize = 5;
/// Normalisation floor for the motion tensors, so a flat region divides by its
/// own noise rather than by zero.
pub(crate) const VR_ZETA2: f32 = 0.1 * 0.1;
/// After refinement, a pixel whose residual exceeds its allowance is not
/// explained by the flow it was given. Replaces "no patch covered me" as the
/// meaning of invalid: a refined field has an answer everywhere, so the
/// honest question is whether the answer is right, not whether one was found.
///
/// The allowance is **relative to local contrast**, not a flat number. A busy
/// region leaves a larger residual than a flat one *even when the flow is
/// exactly right* — a half-pixel error across a strong edge moves the value far
/// more than the same error across a wall — so a single absolute threshold
/// calls detailed footage invalid and flat footage valid regardless of whether
/// either is correct. That is not a hypothetical: shipping one cost Fast motion
/// blur most of its picture, because `confidence` zeroes on invalid and a fast
/// camera move over detailed geometry failed the test nearly everywhere.
pub(crate) const VR_RESIDUAL_FLOOR: f32 = 0.12;
/// How much of the local gradient magnitude to forgive on top of the floor.
/// Generous on purpose: this decides whether a vector is *usable*, and the
/// forward–backward test in §2 is the sharper instrument for whether it is
/// right.
pub(crate) const VR_RESIDUAL_REL: f32 = 3.0;

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
#[derive(Clone)]
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

/// What synthesis does where a pixel exists in only one of the two frames
/// (docs/08 §3.1 "Occlusion handling").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OcclusionMode {
    /// Take only the frame the pixel is visible in.
    #[default]
    VisibleOnly,
    /// Weight both anyway: ghosting instead of holes when the mask is wrong.
    Blend,
}

/// What shows where confidence is too low to synthesise (docs/08 §3.1
/// "Fallback"). Flow failure degrades to a picture, never to garbage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Fallback {
    /// Crossfade the two frames — soft, identical to the Blend policy.
    #[default]
    Blend,
    /// Show the nearer source frame — crisp, no ghosted double image.
    Nearest,
}

/// Every knob the flow engine takes (docs/08 §3.1).
///
/// `lumit-flow` is an engine crate and knows nothing of the document, so this
/// is the plain-numbers form its caller translates `FlowParams` into — the same
/// split the effect ops use. Defaults match `FlowParams::default`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FlowSettings {
    /// Divide the source dimensions by this before measuring: 1 native (the
    /// default), 2 half, 4 quarter. Never derived from the preview scale —
    /// flow measured on a shrunk decode is a different measurement.
    pub divisor: u32,
    /// Inverse-search iterations per patch per level ("Vector detail").
    pub iterations: u32,
    /// Pyramid floor: stop when the next level would drop under this.
    pub min_level_dim: u32,
    /// Regularisation, 0–100 ("Smoothness"): scales the flow-range sigma of the
    /// edge-aware smoothing pass, so high means fewer tears and a gloopier
    /// field, low means crisper motion boundaries.
    pub smoothness: f32,
    pub occlusion: OcclusionMode,
    pub fallback: Fallback,
    /// Bias static, well-textured regions toward pure blending (docs/08 §3.1
    /// step 5) — what stops a game HUD smearing across the frame.
    pub hud_guard: bool,
    /// Variational-refinement fixed-point iterations per pyramid level, scaled
    /// by depth (the paper's θ_vo base). `0` disables the third part of DIS
    /// entirely — which is what Lumit shipped before refinement landed, and
    /// what the A/B test measures against. Vector detail sets it.
    pub refine_iters: u32,
}

impl Default for FlowSettings {
    fn default() -> Self {
        Self {
            divisor: 1,
            iterations: MAX_ITERS as u32,
            min_level_dim: MIN_LEVEL_DIM as u32,
            smoothness: 50.0,
            occlusion: OcclusionMode::VisibleOnly,
            fallback: Fallback::Blend,
            hud_guard: true,
            refine_iters: 1,
        }
    }
}

impl FlowSettings {
    /// The flow-range sigma² the smoothing bilateral uses, from `smoothness`.
    ///
    /// Smoothness is a 0–100 dial; the thing it actually moves is how far apart
    /// two vectors may be and still average together. At 50 it is the tuned
    /// [`FLOW_SIGMA2`] the analytic tests were fitted against, so the default
    /// behaves exactly as it did before the dial existed. It scales
    /// quadratically — the sigma is a squared distance — over a 4× span each
    /// way, which is the range where the difference is visible without either
    /// end degenerating (0 would refuse to smooth at all, 100 would average
    /// across any motion boundary).
    pub fn flow_sigma2(&self) -> f32 {
        let s = (self.smoothness / 50.0).clamp(0.25, 2.0);
        FLOW_SIGMA2 * s * s
    }

    /// Working dimensions for a source of `w × h`.
    ///
    /// A source too small to divide stays whole: halving an already-tiny frame
    /// starves the pyramid, and a frame under one patch cannot be searched at
    /// all (`flow` degrades to a zero field there).
    pub fn working_size(&self, w: usize, h: usize) -> (usize, usize) {
        let d = self.divisor.max(1) as usize;
        if d == 1 || w.min(h) < PATCH * d * 2 {
            (w, h)
        } else {
            (w / d, h / d)
        }
    }
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

/// [`to_gray`] over whichever width [`Texel`] describes.
///
/// The 0..1 range is kept for the eight-bit case exactly as it was. A float
/// frame is scene-linear and may run above white, and the HUD guard's own
/// gradient measure works on differences, so nothing is clamped on the way in.
pub fn to_gray_as<T: Texel>(rgba: &[u8], w: usize, h: usize) -> Gray {
    let mut data = vec![0f32; w * h];
    let scale = if T::BYTES == 4 { 1.0 / 255.0 } else { 1.0 };
    for (i, px) in data.iter_mut().enumerate() {
        let r = T::get(rgba, i, 0);
        let g = T::get(rgba, i, 1);
        let b = T::get(rgba, i, 2);
        *px = (0.2126 * r + 0.7152 * g + 0.0722 * b) * scale;
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
    sobel_slice(&g.data, g.w, g.h)
}

/// [`sobel`] on a bare scalar plane, so a caller holding plain `Vec<f32>`s
/// (the refine loop's warped gradients) need not clone them into a `Gray`.
pub(crate) fn sobel_slice(data: &[f32], w: usize, h: usize) -> (Vec<f32>, Vec<f32>) {
    let mut gx = vec![0f32; w * h];
    let mut gy = vec![0f32; w * h];
    let at = |x: usize, y: usize| data[y * w + x];
    for y in 0..h {
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(h - 1);
            let (tl, t, tr) = (at(xm, ym), at(x, ym), at(xp, ym));
            let (l, r) = (at(xm, y), at(xp, y));
            let (bl, b, br) = (at(xm, yp), at(x, yp), at(xp, yp));
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
    iterations: u32,
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
                for _ in 0..iterations {
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
fn smooth(a: &Gray, u: &[f32], v: &[f32], flow_sigma2: f32) -> (Vec<f32>, Vec<f32>) {
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
                    let wgt = (-(d * d) / SIGMA2).exp() * (-fd / flow_sigma2).exp();
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

/// Variational refinement (impl note §1 step 4) — the third part of DIS,
/// run once per pyramid level after densification.
///
/// # In plain terms
///
/// Everything before this point is *local*: each patch hunted for its own
/// match, and each pixel took a vote among the patches covering it. That works
/// wherever there is something to match, and has nothing to say wherever there
/// isn't — a patch of sky, smoke, or a dark corner offers no evidence at all, so
/// those pixels came out of densification with whatever the coarse level
/// guessed, flagged as untrustworthy. Since occlusion counts untrustworthy as
/// occluded and synthesis crossfades occluded, whole regions of frame turned
/// into ghosted mush. That is the artefact this pass exists to remove.
///
/// The fix is to stop treating pixels one at a time and solve the *whole field*
/// at once, balancing three demands:
///
/// - **Intensity constancy**: a pixel should land on a pixel of the same
///   brightness in the other frame.
/// - **Gradient constancy**: it should also land on the same *edge structure*.
///   This is the term that survives a brightness change — a muzzle flash or an
///   explosion lifts the whole frame's intensity, which the first term reads as
///   motion in every direction at once, while edges stay put and stay matchable.
/// - **Smoothness**: neighbouring pixels should move alike, *unless* the first
///   two terms give a strong reason otherwise.
///
/// Smoothness is what fills the empty regions: a pixel with no evidence of its
/// own inherits motion from neighbours that do have some, diffusing inward from
/// the textured edges of the region over a few passes. All three demands are
/// wrapped in the robust penaliser `Ψ(a²) = √(a² + ε²)`, which grows far more
/// slowly than a square, so one badly-matched pixel bends the field near it
/// instead of dragging the whole neighbourhood off — that is what keeps a motion
/// boundary sharp rather than smearing across it.
///
/// Balancing the three is a system of equations too big to solve directly at
/// video sizes, so it is swept iteratively: repeatedly nudge every pixel toward
/// agreement with its neighbours and its own evidence, over-shooting each nudge
/// slightly (the "over-relaxation" in SOR) because that converges in a handful
/// of passes rather than dozens.
///
/// Returns the refined flow and a per-pixel validity: after this pass every
/// pixel *has* an answer, so validity means "the residual says this answer is
/// right", not "somebody found one".
fn refine(
    a: &Gray,
    b: &Gray,
    u_in: &[f32],
    v_in: &[f32],
    outer: usize,
) -> (Vec<f32>, Vec<f32>, Vec<u8>) {
    let (w, h) = (a.w, a.h);
    let n = w * h;
    let (mut u, mut v) = (u_in.to_vec(), v_in.to_vec());
    if w < 3 || h < 3 {
        return (u, v, vec![0; n]);
    }
    let (ax, ay) = sobel(a);
    // B's gradients once, not once per outer iteration: the frame never
    // changes inside the loop, only where it is sampled.
    let (bx, by) = sobel(b);
    let idx = |x: usize, y: usize| y * w + x;

    for _ in 0..outer {
        // Warp B by the current flow and take its gradients there. The system
        // below solves for the *increment* (du, dv) around this linearisation,
        // which is what makes one sweep of a non-linear problem legitimate.
        let mut bw = vec![0f32; n];
        let mut bwx = vec![0f32; n];
        let mut bwy = vec![0f32; n];
        for y in 0..h {
            for x in 0..w {
                let i = idx(x, y);
                let (sx, sy) = (x as f32 + u[i], y as f32 + v[i]);
                bw[i] = b.sample(sx, sy);
                bwx[i] = sample_scalar(&bx, w, h, sx, sy);
                bwy[i] = sample_scalar(&by, w, h, sx, sy);
            }
        }
        // Per-pixel data-term coefficients, held fixed across the sweeps.
        let mut du = vec![0f32; n];
        let mut dv = vec![0f32; n];
        // Second derivatives of the warped frame, for the gradient term.
        let (bwxx, bwxy) = sobel_slice(&bwx, w, h);
        let (bwyx, bwyy) = sobel_slice(&bwy, w, h);
        // Red–black (checkerboard) sweeps rather than plain raster order.
        //
        // SOR wants each pixel to use its neighbours' *just-updated* values,
        // which in raster order makes the sweep strictly sequential — every
        // pixel waits for the one before it. On a checkerboard the four
        // neighbours of any red pixel are all black, so a whole colour updates
        // at once with no pixel reading another pixel of its own colour: the
        // same algorithm, reordered into something a GPU can run a million
        // threads of. The oracle is written this way so the WGSL can mirror it
        // exactly and the 1e-3 parity contract still means something —
        // a sequential oracle would have condemned the shader to disagree with
        // it by construction.
        for _ in 0..VR_SOR {
            for colour in 0..2usize {
                for y in 0..h {
                    for x in 0..w {
                        if (x + y) % 2 != colour {
                            continue;
                        }
                        let i = idx(x, y);
                        // Intensity constancy: Iz + Ix·du + Iy·dv ≈ 0.
                        let iz = bw[i] - a.data[i];
                        let (ix, iy) = (bwx[i], bwy[i]);
                        let e_i = iz + ix * du[i] + iy * dv[i];
                        // Normalised so a high-contrast pixel does not shout down a
                        // low-contrast one (the paper's J̄ tensors).
                        let n_i = 1.0 / (ix * ix + iy * iy + VR_ZETA2);
                        let psi_i = VR_SIGMA * n_i / (2.0 * (e_i * e_i + VR_EPS2).sqrt());

                        // Gradient constancy, one residual per gradient channel.
                        let gzx = bwx[i] - ax[i];
                        let gzy = bwy[i] - ay[i];
                        let e_gx = gzx + bwxx[i] * du[i] + bwxy[i] * dv[i];
                        let e_gy = gzy + bwyx[i] * du[i] + bwyy[i] * dv[i];
                        let n_g = 1.0
                            / (bwxx[i] * bwxx[i]
                                + bwxy[i] * bwxy[i]
                                + bwyx[i] * bwyx[i]
                                + bwyy[i] * bwyy[i]
                                + VR_ZETA2);
                        let psi_g =
                            VR_GAMMA * n_g / (2.0 * (e_gx * e_gx + e_gy * e_gy + VR_EPS2).sqrt());

                        // Data system: A·[du dv]ᵀ = b.
                        let a11 = psi_i * ix * ix + psi_g * (bwxx[i] * bwxx[i] + bwyx[i] * bwyx[i]);
                        let a12 = psi_i * ix * iy + psi_g * (bwxx[i] * bwxy[i] + bwyx[i] * bwyy[i]);
                        let a22 = psi_i * iy * iy + psi_g * (bwxy[i] * bwxy[i] + bwyy[i] * bwyy[i]);
                        let b1 = -(psi_i * ix * iz + psi_g * (bwxx[i] * gzx + bwyx[i] * gzy));
                        let b2 = -(psi_i * iy * iz + psi_g * (bwxy[i] * gzx + bwyy[i] * gzy));

                        // Smoothness: pull toward the neighbours' *total* flow, each
                        // neighbour weighted by how smooth the field already is
                        // across that edge. A strong flow discontinuity earns a low
                        // weight, so a motion boundary survives instead of being
                        // averaged away.
                        let (mut s_acc_u, mut s_acc_v, mut s_wsum) = (0f32, 0f32, 0f32);
                        for (nx, ny) in [
                            (x.wrapping_sub(1), y),
                            (x + 1, y),
                            (x, y.wrapping_sub(1)),
                            (x, y + 1),
                        ] {
                            if nx >= w || ny >= h {
                                continue; // outside: no neighbour, no pull
                            }
                            let j = idx(nx, ny);
                            let (dux, dvy) =
                                (u[j] + du[j] - u[i] - du[i], v[j] + dv[j] - v[i] - dv[i]);
                            let wgt = VR_ALPHA / (2.0 * (dux * dux + dvy * dvy + VR_EPS2).sqrt());
                            s_wsum += wgt;
                            s_acc_u += wgt * (u[j] + du[j] - u[i]);
                            s_acc_v += wgt * (v[j] + dv[j] - v[i]);
                        }

                        // One SOR step per component, each using the other's current
                        // value (Gauss–Seidel), over-relaxed by ω.
                        let den_u = a11 + s_wsum;
                        if den_u > 1e-12 {
                            let target = (b1 - a12 * dv[i] + s_acc_u) / den_u;
                            du[i] += VR_OMEGA * (target - du[i]);
                        }
                        let den_v = a22 + s_wsum;
                        if den_v > 1e-12 {
                            let target = (b2 - a12 * du[i] + s_acc_v) / den_v;
                            dv[i] += VR_OMEGA * (target - dv[i]);
                        }
                    }
                }
            }
        }
        for i in 0..n {
            u[i] += du[i];
            v[i] += dv[i];
        }
    }

    // Validity from the residual of the *refined* field, forgiven in
    // proportion to how much contrast is there to be wrong about.
    let mut valid = vec![0u8; n];
    for y in 0..h {
        for x in 0..w {
            let i = idx(x, y);
            let r = b.sample(x as f32 + u[i], y as f32 + v[i]) - a.data[i];
            let contrast = (ax[i] * ax[i] + ay[i] * ay[i]).sqrt();
            valid[i] = u8::from(r.abs() <= VR_RESIDUAL_FLOOR + VR_RESIDUAL_REL * contrast);
        }
    }
    (u, v, valid)
}

/// Build the pyramid: L0 is the input, then box-downsample ×2 until the next
/// level would drop under `min_level_dim` in either dimension (Vector detail
/// sets the floor — see [`FlowSettings`]).
pub(crate) fn build_pyramid_to(g: &Gray, min_level_dim: usize) -> Vec<Gray> {
    let mut p = vec![g.clone()];
    loop {
        let last = &p[p.len() - 1];
        if (last.w / 2).max(1).min((last.h / 2).max(1)) < min_level_dim {
            break;
        }
        let next = downsample(last);
        p.push(next);
    }
    p
}

/// Coarse-to-fine DIS over prebuilt pyramids (`grads` are `pa`'s Sobel fields
/// per level — the template side).
fn flow_core(
    pa: &[Gray],
    pb: &[Gray],
    grads: &[(Vec<f32>, Vec<f32>)],
    set: &FlowSettings,
) -> FlowField {
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
        let patches = inverse_search(a, b, gx, gy, &du, &dv, set.iterations);
        let (tu, tv, tvalid) = densify(a, b, &patches, &du, &dv);
        let (su, sv) = smooth(a, &tu, &tv, set.flow_sigma2());
        if set.refine_iters > 0 {
            // DIS part three. The paper runs more fixed-point
            // iterations at finer scales — θ_vo = 1·(s+1), s counting down from
            // the coarsest — because that is where the field has the most detail
            // left to resolve and the most room to be wrong.
            let scale_from_coarse = levels - lvl;
            let outer = set.refine_iters as usize * scale_from_coarse;
            let (ru, rv, rvalid) = refine(a, b, &su, &sv, outer);
            du = ru;
            dv = rv;
            valid = rvalid;
        } else {
            du = su;
            dv = sv;
            valid = tvalid;
        }
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

/// Dense forward flow A→B by DIS (coarse-to-fine inverse search) under
/// explicit settings.
pub fn flow_with(a: &Gray, b: &Gray, set: &FlowSettings) -> FlowField {
    if a.w < PATCH || a.h < PATCH || a.w != b.w || a.h != b.h {
        return FlowField::zeroed(a.w, a.h);
    }
    let floor = set.min_level_dim.max(PATCH as u32) as usize;
    let pa = build_pyramid_to(a, floor);
    let pb = build_pyramid_to(b, floor);
    let grads: Vec<(Vec<f32>, Vec<f32>)> = pa.iter().map(sobel).collect();
    flow_core(&pa, &pb, &grads, set)
}

/// Both directions at once (A→B, B→A), sharing the pyramids — the impl note's
/// "reuse everything; it is 2× cost".
pub fn flow_pair_with(a: &Gray, b: &Gray, set: &FlowSettings) -> (FlowField, FlowField) {
    if a.w < PATCH || a.h < PATCH || a.w != b.w || a.h != b.h {
        return (FlowField::zeroed(a.w, a.h), FlowField::zeroed(b.w, b.h));
    }
    let floor = set.min_level_dim.max(PATCH as u32) as usize;
    let pa = build_pyramid_to(a, floor);
    let pb = build_pyramid_to(b, floor);
    let ga: Vec<(Vec<f32>, Vec<f32>)> = pa.iter().map(sobel).collect();
    let gb: Vec<(Vec<f32>, Vec<f32>)> = pb.iter().map(sobel).collect();
    (flow_core(&pa, &pb, &ga, set), flow_core(&pb, &pa, &gb, set))
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
/// free, so preview and export derive the identical field. A
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
            raw[i] = agree * if f.valid[i] == 0 { VALID_DIM } else { 1.0 };
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

/// Speed (px per frame pair) below which a pixel counts as fully static, and
/// above which it counts as fully moving. Between the two the guard tapers.
const HUD_STATIC_LO: f32 = 0.25;
const HUD_STATIC_HI: f32 = 1.0;
/// Local gradient energy (encoded luma per px) below which a region is too
/// smooth to be an overlay, and above which it is definitely drawn detail.
const HUD_TEX_LO: f32 = 0.02;
const HUD_TEX_HI: f32 = 0.08;

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The HUD/overlay guard (docs/08 §3.1 step 5): per-pixel 0..1, how much this
/// pixel should be left to a plain blend instead of being warped.
///
/// # In plain terms
///
/// A game's health bar, killfeed and minimap are painted on *after* the camera
/// moves — they sit still while the whole world slides underneath them. Flow
/// sees a frame where everything moves except a few sharp rectangles, and the
/// motion of the background inevitably bleeds into them: the classic Twixtor
/// artefact where the HUD smears across the screen during a fast turn.
///
/// The tell is a region that is **not moving** but **is full of detail**. A
/// static patch of sky is smooth; a static patch of *text* is not. So the guard
/// looks for near-zero flow sitting on high local gradient, and where it finds
/// both it hands the pixel to a plain crossfade — which for genuinely static
/// content is the correct picture anyway, since A and B agree there.
///
/// Both tests taper rather than switch, and the result is box-blurred, because
/// a hard boundary between "warped" and "not warped" is itself a visible
/// artefact — the thing FX-19 learned the expensive way on motion blur.
pub fn hud_weights(a: &Gray, f: &FlowField) -> Vec<f32> {
    let (w, h) = (a.w, a.h);
    let n = w * h;
    if f.w != w || f.h != h || f.u.len() != n {
        return vec![0.0; n]; // mismatched: guard nothing, never fault
    }
    let (gx, gy) = sobel(a);
    // "Is there detail *near* this pixel", not "is this pixel itself an edge".
    // A gradient is zero inside every stroke of a letter and only spikes at its
    // rim, so a per-pixel test guards the outlines of a HUD and leaves its
    // insides to be smeared — which is the artefact, not the fix. The 3×3 max
    // spreads each piece of evidence over its neighbourhood first.
    let mut tex = vec![0f32; n];
    for y in 0..h {
        for x in 0..w {
            let mut m = 0f32;
            for oy in -1i32..=1 {
                for ox in -1i32..=1 {
                    let qx = (x as i32 + ox).clamp(0, w as i32 - 1) as usize;
                    let qy = (y as i32 + oy).clamp(0, h as i32 - 1) as usize;
                    let q = qy * w + qx;
                    m = m.max((gx[q] * gx[q] + gy[q] * gy[q]).sqrt());
                }
            }
            tex[y * w + x] = m;
        }
    }
    let mut raw = vec![0f32; n];
    for i in 0..n {
        let speed = (f.u[i] * f.u[i] + f.v[i] * f.v[i]).sqrt();
        // 1 where still, 0 where moving.
        let stillness = 1.0 - smoothstep(HUD_STATIC_LO, HUD_STATIC_HI, speed);
        if stillness <= 0.0 {
            continue;
        }
        raw[i] = stillness * smoothstep(HUD_TEX_LO, HUD_TEX_HI, tex[i]);
    }
    // Widen by a pixel and remove the seam, exactly as `confidence` does: an
    // overlay's anti-aliased edge is a gradient the per-pixel test reads
    // differently from its interior.
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

/// How wide one frame's samples are, for the synthesis that warps them.
///
/// The synthesis is already arithmetic in `f32` from end to end — the warp,
/// the occlusion weights, the guard, all of it. The only thing that ever
/// depended on a byte was reading a frame in and writing one out, so this is
/// that, and the synthesis itself is written once.
pub trait Texel {
    /// Bytes one pixel occupies.
    const BYTES: usize;
    /// Channel `c` of pixel `i`, in the units the synthesis works in.
    fn get(buf: &[u8], i: usize, c: usize) -> f32;
    /// Write channel `c` of pixel `i` back.
    fn put(buf: &mut [u8], i: usize, c: usize, v: f32);
}

/// Eight-bit frames, counted 0..255 — what the synthesis has always carried,
/// down to the rounding and the clamp.
pub struct Bytes;

impl Texel for Bytes {
    const BYTES: usize = 4;
    fn get(buf: &[u8], i: usize, c: usize) -> f32 {
        buf.get(i * 4 + c).map_or(0.0, |v| f32::from(*v))
    }
    fn put(buf: &mut [u8], i: usize, c: usize, v: f32) {
        if let Some(slot) = buf.get_mut(i * 4 + c) {
            *slot = v.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Scene-linear float frames (`lumit_media::PixelFormat::LinearF32`), counted
/// in their own units — so nothing rounds and nothing clips at white.
pub struct Floats;

impl Texel for Floats {
    const BYTES: usize = 16;
    fn get(buf: &[u8], i: usize, c: usize) -> f32 {
        let at = (i * 4 + c) * 4;
        buf.get(at..at + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map_or(0.0, f32::from_le_bytes)
    }
    fn put(buf: &mut [u8], i: usize, c: usize, v: f32) {
        let at = (i * 4 + c) * 4;
        if let Some(slot) = buf.get_mut(at..at + 4) {
            slot.copy_from_slice(&v.to_le_bytes());
        }
    }
}

/// [`sample_rgba`] over whichever width [`Texel`] describes.
fn sample_texel<T: Texel>(rgba: &[u8], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
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
        let p = |px: usize, py: usize| T::get(rgba, py * w + px, c);
        let a = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
        let b = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
        *o = a * (1.0 - fy) + b * fy;
    }
    out
}

/// [`crossfade`] over whichever width [`Texel`] describes.
fn crossfade_as<T: Texel>(a: &[u8], b: &[u8], phi: f32) -> Vec<u8> {
    let n = a.len().min(b.len()) / T::BYTES;
    let mut out = vec![0u8; n * T::BYTES];
    for i in 0..n {
        for c in 0..4 {
            T::put(
                &mut out,
                i,
                c,
                T::get(a, i, c) * (1.0 - phi) + T::get(b, i, c) * phi,
            );
        }
    }
    out
}

/// Synthesise the frame at phase `phi` ∈ [0,1] between A and B (impl note §3):
/// backward-warp both endpoints along their flow and blend with occlusion-aware
/// weights. `phi` = 0 returns A, 1 returns B, bit-exactly. `fwd` is flow A→B,
/// `bwd` is B→A, both at the frames' full resolution. Where **both** frames
/// lost sight of a pixel, it falls back to a plain crossfade — the documented
/// graceful degradation. `hud` is an optional per-pixel HUD guard weight
/// (from [`hud_weights`], at the frames' own size).
///
/// The three §3.1 knobs land here. **Occlusion handling** chooses whether a
/// pixel that exists in only one frame takes that frame alone or is weighted
/// from both. **Fallback** chooses what shows where *neither* frame can explain
/// the pixel — a crossfade (soft, ghosted) or the nearer frame (crisp, but it
/// jumps). The **HUD guard** overrides both, per pixel, by mixing the whole
/// synthesised result back toward a plain blend wherever the guard fired.
#[allow(clippy::too_many_arguments)]
pub fn synthesize_with(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    fwd: &FlowField,
    bwd: &FlowField,
    phi: f32,
    set: &FlowSettings,
    hud: Option<&[f32]>,
) -> Vec<u8> {
    synthesize_with_as::<Bytes>(a, b, w, h, fwd, bwd, phi, set, hud)
}

/// [`synthesize_with`] over whichever width [`Texel`] describes.
#[allow(clippy::too_many_arguments)]
pub fn synthesize_with_as<T: Texel>(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    fwd: &FlowField,
    bwd: &FlowField,
    phi: f32,
    set: &FlowSettings,
    hud: Option<&[f32]>,
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
        || a.len() < n * T::BYTES
        || b.len() < n * T::BYTES
        || a.len() != b.len()
    {
        return crossfade_as::<T>(a, b, phi);
    }
    // Occlusion masks (§2): occ_a marks A-pixels with no match in B (content
    // that gets covered); occ_b marks B-pixels with no match in A (content
    // that gets revealed).
    let occ_a = occlusion(fwd, bwd);
    let occ_b = occlusion(bwd, fwd);
    let mut out = vec![0u8; n * T::BYTES];
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
            let sa = sample_texel::<T>(a, w, h, xf - phi * f1u, yf - phi * f1v);
            // The backward field points B→A; the forward velocity seen from
            // B's grid is its negation, hence the minus sign here too.
            let sb = sample_texel::<T>(b, w, h, xf - (1.0 - phi) * b1u, yf - (1.0 - phi) * b1v);
            let (oa, ob) = (occ_a[i], occ_b[i]);
            // How much this pixel should be left alone (HUD guard, §3.1 step 5):
            // a static, detailed overlay must not be dragged by the motion of
            // the world sliding underneath it.
            let guard = hud.map_or(0.0, |g| g.get(i).copied().unwrap_or(0.0));
            for c in 0..4 {
                let la = T::get(a, i, c);
                let lb = T::get(b, i, c);
                let synth = if oa == 1 && ob == 1 {
                    // Neither frame can explain this pixel — revealed
                    // background with no source anywhere (§3 soft failure).
                    match set.fallback {
                        Fallback::Blend => la * (1.0 - phi) + lb * phi,
                        Fallback::Nearest => {
                            if phi < 0.5 {
                                la
                            } else {
                                lb
                            }
                        }
                    }
                } else {
                    // A's warp is trusted unless the content only exists in B
                    // (revealed, occ_b), and vice versa (§3 weights). Under
                    // Blend handling the occlusion terms are dropped, so both
                    // warps contribute by phase alone: ghosting where the mask
                    // was right, but no hole where it was wrong.
                    let (ga, gb) = match set.occlusion {
                        OcclusionMode::VisibleOnly => (1.0 - f32::from(ob), 1.0 - f32::from(oa)),
                        OcclusionMode::Blend => (1.0, 1.0),
                    };
                    let wa = (1.0 - phi) * ga + SYNTH_EPS;
                    let wb = phi * gb + SYNTH_EPS;
                    (wa * sa[c] + wb * sb[c]) / (wa + wb)
                };
                // The guard mixes back toward the unwarped blend. For genuinely
                // static content that *is* the correct picture, since A and B
                // agree there — so a full guard costs nothing but the smear.
                let plain = la * (1.0 - phi) + lb * phi;
                let v = synth * (1.0 - guard) + plain * guard;
                T::put(&mut out, i, c, v);
            }
        }
    }
    out
}

/// Luma pair at the settings' working resolution. Returns the pair and whether
/// it was actually reduced (a source too small to divide stays whole, since
/// halving an already-tiny frame starves the pyramid).
fn grays_at<T: Texel>(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    set: &FlowSettings,
) -> (Gray, Gray, bool) {
    let ga = to_gray_as::<T>(a, w, h);
    let gb = to_gray_as::<T>(b, w, h);
    let (ww, _) = set.working_size(w, h);
    if ww == w {
        return (ga, gb, false);
    }
    // Repeated halving reaches quarter and beyond with the one box filter the
    // WGSL mirrors, rather than needing a second resampler.
    let (mut ra, mut rb) = (ga, gb);
    while ra.w > ww {
        ra = downsample(&ra);
        rb = downsample(&rb);
    }
    (ra, rb, true)
}

/// The luma pair flow is measured on, at the settings' working resolution —
/// the first half of [`FlowEngine::interpolate_at`], exposed so a caller that
/// caches measurements can do the two halves separately.
///
/// Returns `(A, B, reduced)`, `reduced` saying whether the working resolution
/// is below the frames' own.
pub fn flow_grays(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    set: &FlowSettings,
) -> (Gray, Gray, bool) {
    grays_at::<Bytes>(a, b, w, h, set)
}

/// [`flow_grays`] for scene-linear float frames
/// (`lumit_media::PixelFormat::LinearF32`).
///
/// Motion is measured on brightness, and brightness is brightness at any
/// width — this exists so the read finds the samples where they actually are,
/// not because the measurement wants anything different.
#[must_use]
pub fn flow_grays_f32(
    a: &[u8],
    b: &[u8],
    w: usize,
    h: usize,
    set: &FlowSettings,
) -> (Gray, Gray, bool) {
    grays_at::<Floats>(a, b, w, h, set)
}

/// Bring a measured field and its confidence up to `w × h`, for a consumer
/// that wants them at the frame's own size.
///
/// The vectors are scaled by the size ratio — a 3 px displacement measured at
/// half resolution is a 6 px displacement of the full-size picture — while the
/// confidence is a 0..1 weight and is resampled without scaling. Getting that
/// asymmetry wrong is silent: the streaks come out half length, or the taper
/// saturates, and neither looks like a bug in the flow.
pub fn field_to_size(
    f: &FlowField,
    conf: &[f32],
    w: usize,
    h: usize,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    if f.w == w && f.h == h {
        return (f.u.clone(), f.v.clone(), conf.to_vec());
    }
    let u = upsample_flow(&f.u, f.w, f.h, w, h);
    let v = upsample_flow(&f.v, f.w, f.h, w, h);
    (u, v, weights_to_size(conf, f.w, f.h, w, h))
}

/// Resample a 0..1 weight plane measured at `sw × sh` up to `w × h`.
///
/// `upsample_flow` applies the scaling flow vectors want (a flow field grows
/// with the image); a weight is the same number at every resolution, so the
/// scale is undone and the result clamped back to 0..1.
fn weights_to_size(g: &[f32], sw: usize, sh: usize, w: usize, h: usize) -> Vec<f32> {
    let scale = w as f32 / sw.max(1) as f32;
    upsample_flow(g, sw, sh, w, h)
        .into_iter()
        .map(|v| (v / scale).clamp(0.0, 1.0))
        .collect()
}

/// The backend-choosing engine callers hold on to: WGSL DIS on a GPU when one
/// is available, the CPU oracle otherwise. A GPU failure mid-flight degrades
/// permanently to CPU for this engine — never a fault: device trouble costs
/// speed, not the frame.
pub struct FlowEngine {
    gpu: Option<gpu::GpuFlow>,
    /// GPU synthesis. Independent of `gpu`: the field could come from
    /// the CPU oracle and still be painted on the card, and losing one does not
    /// have to cost the other.
    synth: Option<synth::GpuSynth>,
}

impl FlowEngine {
    /// Try for a GPU of our own (headless); fall back to CPU quietly.
    pub fn new_auto() -> Self {
        match lumit_gpu::GpuContext::headless() {
            Ok(ctx) => Self::with_context(&ctx),
            Err(_) => Self::cpu(),
        }
    }

    /// True when synthesis runs on the card.
    pub fn gpu_synthesis(&self) -> bool {
        self.synth.is_some()
    }

    /// Share an existing device (the app's). Falls back to CPU if the flow
    /// pipelines cannot be built on it.
    pub fn with_context(ctx: &lumit_gpu::GpuContext) -> Self {
        FlowEngine {
            gpu: gpu::GpuFlow::new(ctx).ok(),
            synth: synth::GpuSynth::new(ctx).ok(),
        }
    }

    /// CPU only (tests, or by explicit choice).
    pub fn cpu() -> Self {
        FlowEngine {
            gpu: None,
            synth: None,
        }
    }

    /// Which backend this engine currently uses.
    pub fn backend(&self) -> &'static str {
        match (self.gpu.is_some(), self.synth.is_some()) {
            (true, true) => "dis-gpu",
            (true, false) => "dis-gpu/cpu-synth",
            (false, true) => "dis-cpu/gpu-synth",
            (false, false) => "dis-cpu",
        }
    }

    /// Both flow directions at the frames' own resolution, on whichever
    /// backend is live, under explicit settings.
    ///
    /// The GPU expresses every setting (the plan takes the pyramid floor, the
    /// per-level uniform the iteration cap and smoothing sigma, the refinement
    /// count scales by depth as the CPU's does). A GPU *fault* - a lost device,
    /// a failed pipeline - degrades this engine to the CPU oracle for the rest
    /// of its life rather than risking a differently-measured field.
    pub fn flow_pair_with(
        &mut self,
        a: &Gray,
        b: &Gray,
        set: &FlowSettings,
    ) -> (FlowField, FlowField) {
        if let Some(g) = self.gpu.as_mut() {
            match g.flow_pair_with(a, b, set) {
                Ok(pair) => return pair,
                Err(_) => self.gpu = None, // degrade to CPU from here on
            }
        }
        flow_pair_with(a, b, set)
    }

    /// Both flow directions between two **GPU pictures**, at the settings'
    /// working resolution (docs/08 §3.2).
    ///
    /// This is how a picture that only exists on the card gets measured: an
    /// adjustment layer's composite of everything below it, or a Precomp's own
    /// render. `None` — no GPU backend, a size mismatch, a device fault — means
    /// the caller has no field and its flow-consuming effect degrades to a
    /// passthrough, which is the same answer it got before any of this existed.
    /// There is deliberately no CPU fallback: the oracle cannot see a texture
    /// without the readback this exists to avoid.
    pub fn flow_pair_textures(
        &mut self,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        set: &FlowSettings,
    ) -> Option<(FlowField, FlowField)> {
        let g = self.gpu.as_mut()?;
        match g.flow_pair_textures(a, b, set) {
            Ok(pair) => Some(pair),
            Err(_) => {
                self.gpu = None; // degrade to CPU-only from here on
                None
            }
        }
    }

    /// The flow-interpolated frame at `phi` under explicit settings.
    pub fn interpolate_at(
        &mut self,
        a: &[u8],
        b: &[u8],
        w: usize,
        h: usize,
        phi: f32,
        set: &FlowSettings,
    ) -> Vec<u8> {
        if phi <= 0.0 {
            return a.to_vec();
        }
        if phi >= 1.0 {
            return b.to_vec();
        }
        let (ga, gb, reduced) = grays_at::<Bytes>(a, b, w, h, set);
        let (fwd, bwd) = self.flow_pair_with(&ga, &gb, set);
        // The card paints straight from the working-resolution field: no
        // upsample, no per-pixel CPU, no round trip. A failure here
        // costs speed, never the frame.
        if let Some(s) = self.synth.as_ref() {
            match s.synthesize(a, b, w, h, &fwd, &bwd, phi, set) {
                Ok(px) => return px,
                Err(_) => self.synth = None, // degrade for the rest of this engine
            }
        }
        let hud = set.hud_guard.then(|| hud_weights(&ga, &fwd));
        let (fwd, bwd) = if reduced {
            (upsample_field(&fwd, w, h), upsample_field(&bwd, w, h))
        } else {
            (fwd, bwd)
        };
        let hud = hud.map(|g| {
            if reduced {
                weights_to_size(&g, ga.w, ga.h, w, h)
            } else {
                g
            }
        });
        synthesize_with(a, b, w, h, &fwd, &bwd, phi, set, hud.as_deref())
    }

    /// Paint the frame at `phi` from flow that has *already* been measured —
    /// the second half of [`Self::interpolate_at`].
    ///
    /// Separating the halves is what lets a caller cache the measurement: the
    /// field between two source frames is one field however many phases are
    /// drawn from it, and a slow ramp draws many. `fwd`/`bwd` are at their own
    /// (working) resolution, which need not be the frames'.
    #[allow(clippy::too_many_arguments)]
    pub fn synthesize_at(
        &mut self,
        a: &[u8],
        b: &[u8],
        w: usize,
        h: usize,
        fwd: &FlowField,
        bwd: &FlowField,
        phi: f32,
        set: &FlowSettings,
    ) -> Vec<u8> {
        if phi <= 0.0 {
            return a.to_vec();
        }
        if phi >= 1.0 {
            return b.to_vec();
        }
        if let Some(s) = self.synth.as_ref() {
            match s.synthesize(a, b, w, h, fwd, bwd, phi, set) {
                Ok(px) => return px,
                Err(_) => self.synth = None,
            }
        }
        self.synthesize_cpu::<Bytes>(a, b, w, h, fwd, bwd, phi, set)
    }

    /// [`Self::synthesize_at`] for scene-linear float frames
    /// (`lumit_media::PixelFormat::LinearF32`), sixteen bytes a pixel.
    ///
    /// **This one always takes the CPU path**, and deliberately. The compute
    /// kernel keeps its two frames in storage buffers of packed bytes, four to
    /// a pixel, so a float frame does not fit it — and widening those buffers
    /// would quadruple them for the eight-bit case that is nearly every case.
    /// The arithmetic is the same either way: the synthesis has always worked
    /// in `f32` internally, and the only thing the card was doing faster was
    /// the same sums. So a float plate retimed by Flow is correct and lossless
    /// and costs a CPU pass per frame, where an eight-bit one costs a compute
    /// dispatch.
    #[allow(clippy::too_many_arguments)]
    pub fn synthesize_at_f32(
        &mut self,
        a: &[u8],
        b: &[u8],
        w: usize,
        h: usize,
        fwd: &FlowField,
        bwd: &FlowField,
        phi: f32,
        set: &FlowSettings,
    ) -> Vec<u8> {
        if phi <= 0.0 {
            return a.to_vec();
        }
        if phi >= 1.0 {
            return b.to_vec();
        }
        self.synthesize_cpu::<Floats>(a, b, w, h, fwd, bwd, phi, set)
    }

    /// The CPU synthesis, at whichever width. Here the field *must* be brought
    /// up to frame size, since the synthesis indexes it per output pixel.
    #[allow(clippy::too_many_arguments)]
    fn synthesize_cpu<T: Texel>(
        &mut self,
        a: &[u8],
        b: &[u8],
        w: usize,
        h: usize,
        fwd: &FlowField,
        bwd: &FlowField,
        phi: f32,
        set: &FlowSettings,
    ) -> Vec<u8> {
        let reduced = fwd.w != w || fwd.h != h;
        let hud = set.hud_guard.then(|| {
            let ga = to_gray_as::<T>(a, w, h);
            let ga = if reduced {
                let mut g = ga;
                while g.w > fwd.w {
                    g = downsample(&g);
                }
                g
            } else {
                ga
            };
            let raw = hud_weights(&ga, fwd);
            if reduced {
                weights_to_size(&raw, ga.w, ga.h, w, h)
            } else {
                raw
            }
        });
        let (f, g);
        let (fwd, bwd) = if reduced {
            f = upsample_field(fwd, w, h);
            g = upsample_field(bwd, w, h);
            (&f, &g)
        } else {
            (fwd, bwd)
        };
        synthesize_with_as::<T>(a, b, w, h, fwd, bwd, phi, set, hud.as_deref())
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
        let f = flow_with(&a, &b, &FlowSettings::default());
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
        let (f, _) = flow_pair_with(&ha, &hb, &FlowSettings::default());
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
        let f = flow_with(&a, &b, &FlowSettings::default());
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
        let f = flow_with(&a, &b, &FlowSettings::default());
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
        let (f, g) = flow_pair_with(&a, &b, &FlowSettings::default());
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
        let mut eng = FlowEngine::cpu();
        let set = FlowSettings::default();
        assert_eq!(eng.interpolate_at(&a, &b, w, h, 0.0, &set), a);
        assert_eq!(eng.interpolate_at(&a, &b, w, h, 1.0, &set), b);
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
        let synth = FlowEngine::cpu().interpolate_at(&a, &b, w, h, 0.5, &FlowSettings::default());
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

    /// The HUD guard (docs/08 §3.1 step 5) fires on the thing it is named for:
    /// a static, detailed overlay sitting over a moving world.
    ///
    /// Built as the artefact itself — a sharp textured block that does not
    /// move, and a flow field that (wrongly, as DIS does near a strong moving
    /// edge) claims it does not — so the assertion is that the guard picks out
    /// the overlay and leaves the moving background alone.
    #[test]
    fn the_hud_guard_fires_on_static_detail_and_not_on_moving_content() {
        let (w, h) = (128, 96);
        // Left half: a busy static "HUD" panel. Right half: smooth gradient.
        let img = render(w, h, |x, y| {
            if x < 48.0 {
                // High-frequency detail, like text.
                0.5 + 0.45 * ((x * 1.7).sin() * (y * 1.9).sin()).signum()
            } else {
                0.2 + 0.5 * (x / w as f32)
            }
        });
        // Flow: the HUD region measured as still, the rest as moving fast.
        let n = w * h;
        let mut f = FlowField {
            w,
            h,
            u: vec![0.0; n],
            v: vec![0.0; n],
            valid: vec![1; n],
        };
        for y in 0..h {
            for x in 0..w {
                if x >= 48 {
                    f.u[y * w + x] = 6.0;
                }
            }
        }
        let g = hud_weights(&img, &f);
        // Sample well inside each region, clear of the blurred boundary.
        let at = |x: usize, y: usize| g[y * w + x];
        assert!(
            at(24, 48) > 0.8,
            "static detailed overlay should be guarded, got {}",
            at(24, 48)
        );
        assert!(
            at(100, 48) < 0.05,
            "moving content must not be guarded, got {}",
            at(100, 48)
        );
    }

    /// Static but *smooth* content is not an overlay — a locked-off sky must
    /// not trip the guard, or the guard is just "blend everything still".
    #[test]
    fn the_hud_guard_ignores_static_smooth_content() {
        let (w, h) = (64, 64);
        let flat = render(w, h, |_, _| 0.5);
        let n = w * h;
        let still = FlowField {
            w,
            h,
            u: vec![0.0; n],
            v: vec![0.0; n],
            valid: vec![1; n],
        };
        let g = hud_weights(&flat, &still);
        assert!(
            g.iter().all(|&v| v < 0.01),
            "a featureless still region is not a HUD"
        );
    }

    /// The guard changes the picture in the direction it claims: guarded
    /// pixels come back as the plain blend, unwarped.
    #[test]
    fn a_guarded_pixel_synthesises_as_the_plain_blend() {
        let (w, h) = (32, 32);
        let n = w * h;
        let a: Vec<u8> = (0..n * 4).map(|i| (i % 251) as u8).collect();
        let b: Vec<u8> = (0..n * 4).map(|i| ((i * 7) % 251) as u8).collect();
        // A flow field that would drag every pixel a long way sideways.
        let f = FlowField {
            w,
            h,
            u: vec![5.0; n],
            v: vec![0.0; n],
            valid: vec![1; n],
        };
        let bwd = FlowField {
            w,
            h,
            u: vec![-5.0; n],
            v: vec![0.0; n],
            valid: vec![1; n],
        };
        let set = FlowSettings::default();
        let guarded = synthesize_with(&a, &b, w, h, &f, &bwd, 0.5, &set, Some(&vec![1.0; n]));
        // Full guard everywhere == the crossfade, exactly.
        for i in 0..n * 4 {
            let want = (f32::from(a[i]) * 0.5 + f32::from(b[i]) * 0.5).round() as u8;
            assert_eq!(
                guarded[i], want,
                "guarded pixel {i} should be the plain blend"
            );
        }
        // With no guard the warp is visible, so the result differs.
        let warped = synthesize_with(&a, &b, w, h, &f, &bwd, 0.5, &set, None);
        assert_ne!(warped, guarded);
    }

    /// The Fallback knob (docs/08 §3.1) picks what shows where neither frame
    /// can explain a pixel: a crossfade, or the nearer frame.
    #[test]
    fn the_fallback_knob_chooses_blend_or_nearest_where_both_are_occluded() {
        let (w, h) = (16, 16);
        let n = w * h;
        let a = vec![0u8; n * 4];
        let b = vec![200u8; n * 4];
        // All-invalid fields make every pixel occluded in both directions.
        let dead = || FlowField {
            w,
            h,
            u: vec![0.0; n],
            v: vec![0.0; n],
            valid: vec![0; n],
        };
        let blend = synthesize_with(
            &a,
            &b,
            w,
            h,
            &dead(),
            &dead(),
            0.25,
            &FlowSettings {
                fallback: Fallback::Blend,
                hud_guard: false,
                ..FlowSettings::default()
            },
            None,
        );
        assert_eq!(blend[0], 50, "0.25 of the way from 0 to 200");
        let nearest = synthesize_with(
            &a,
            &b,
            w,
            h,
            &dead(),
            &dead(),
            0.25,
            &FlowSettings {
                fallback: Fallback::Nearest,
                hud_guard: false,
                ..FlowSettings::default()
            },
            None,
        );
        assert_eq!(nearest[0], 0, "nearer frame at phi 0.25 is A");
    }

    /// Smoothness moves the regularisation it claims to move, and the default
    /// leaves the tuned constant exactly where the analytic tests found it.
    #[test]
    fn smoothness_scales_the_flow_sigma_around_the_tuned_default() {
        let at = |s: f32| {
            FlowSettings {
                smoothness: s,
                ..FlowSettings::default()
            }
            .flow_sigma2()
        };
        assert_eq!(at(50.0), FLOW_SIGMA2);
        assert!(at(10.0) < at(50.0));
        assert!(at(90.0) > at(50.0));
        // Clamped at both ends: never zero (which would refuse to smooth) and
        // never unbounded (which would average across any motion boundary).
        assert!(at(0.0) > 0.0);
        assert!(at(1000.0) <= FLOW_SIGMA2 * 4.0);
    }

    /// Flow resolution is a divisor on the source, and a frame too small to
    /// divide stays whole rather than starving the pyramid.
    #[test]
    fn working_size_divides_but_never_starves() {
        let full = FlowSettings::default();
        assert_eq!(full.working_size(1920, 1080), (1920, 1080));
        let half = FlowSettings {
            divisor: 2,
            ..FlowSettings::default()
        };
        assert_eq!(half.working_size(1920, 1080), (960, 540));
        let quarter = FlowSettings {
            divisor: 4,
            ..FlowSettings::default()
        };
        assert_eq!(quarter.working_size(1920, 1080), (480, 270));
        // Too small to divide: unchanged, not reduced into uselessness.
        assert_eq!(quarter.working_size(40, 40), (40, 40));
    }

    /// Vector detail buys accuracy: the same hard motion is recovered at least
    /// as well at Ultra's iteration count as at Low's.
    #[test]
    fn more_vector_detail_is_never_worse() {
        let (w, h) = (192, 192);
        let (dx, dy) = (7.0f32, -5.0f32);
        let a = render(w, h, |x, y| perlin(x, y, 9));
        let b = render(w, h, |x, y| perlin(x - dx, y - dy, 9));
        let epe_at = |iters: u32| {
            let f = flow_with(
                &a,
                &b,
                &FlowSettings {
                    iterations: iters,
                    ..FlowSettings::default()
                },
            );
            mean_epe(&f, 24, |_, _| (dx, dy))
        };
        let (low, ultra) = (epe_at(6), epe_at(32));
        assert!(
            ultra <= low + 1e-4,
            "more iterations should not be worse: low {low}, ultra {ultra}"
        );
    }

    /// The reported artefact: a large low-texture region moving with the frame.
    /// Without variational refinement the patches find nothing there, so
    /// densification leaves the coarse guess and flags it untrustworthy —
    /// occlusion counts that as occluded and synthesis crossfades it, which is
    /// the ghosted mush the owner saw. With refinement, smoothness diffuses a
    /// sensible field in from the textured edges.
    ///
    /// The scene is deliberately the hostile one: a wide, nearly flat band
    /// (smoke/sky) across the middle of an otherwise detailed frame.
    #[test]
    fn refinement_recovers_motion_in_untextured_regions() {
        let (w, h) = (192, 192);
        let (dx, dy) = (4.0f32, 2.0f32);
        // Detailed everywhere except a broad horizontal band with almost no
        // contrast — the case local patch matching cannot answer.
        let scene = |ox: f32, oy: f32| {
            render(w, h, move |x, y| {
                let (sx, sy) = (x - ox, y - oy);
                let band = ((sy - 96.0) / 40.0).abs().min(1.0);
                let detail = perlin(sx, sy, 21) + 0.3 * detail(sx, sy, 22);
                // band == 0 in the middle: flat grey. band == 1 at the edges.
                0.5 * (1.0 - band) + band * detail + 0.01 * (sx * 0.05).sin()
            })
        };
        let a = scene(0.0, 0.0);
        let b = scene(dx, dy);
        let epe_in_band = |set: &FlowSettings| {
            let f = flow_with(&a, &b, set);
            // Measure only inside the flat band, where the artefact lives.
            let (mut sum, mut n) = (0.0f64, 0usize);
            for y in 86..106 {
                for x in 32..160 {
                    let i = y * w + x;
                    let e = ((f.u[i] - dx).powi(2) + (f.v[i] - dy).powi(2)).sqrt();
                    sum += f64::from(e);
                    n += 1;
                }
            }
            (sum / n as f64) as f32
        };
        let without = epe_in_band(&FlowSettings {
            refine_iters: 0,
            ..FlowSettings::default()
        });
        let with = epe_in_band(&FlowSettings::default());
        assert!(
            with < without,
            "variational refinement should improve untextured regions: \
             with {with} vs without {without}"
        );
    }

    /// Refinement must not cost accuracy where the old path was already fine —
    /// a plain textured translation stays within the §6.1 budget.
    #[test]
    fn refinement_keeps_the_analytic_accuracy_budget() {
        let (w, h) = (192, 192);
        let (dx, dy) = (5.0f32, -3.0f32);
        let a = render(w, h, |x, y| perlin(x, y, 31));
        let b = render(w, h, |x, y| perlin(x - dx, y - dy, 31));
        let f = flow_with(&a, &b, &FlowSettings::default());
        let epe = mean_epe(&f, 24, |_, _| (dx, dy));
        assert!(epe < 0.3, "refined flow must still meet §6.1: {epe}");
    }

    /// Validity now means "the flow explains these pixels", not "a patch
    /// covered me". On a clean textured translation nearly everything
    /// should be valid — under the old rule, flat areas were not.
    #[test]
    fn refined_validity_marks_explained_pixels() {
        let (w, h) = (128, 128);
        let a = render(w, h, |x, y| perlin(x, y, 41));
        let b = render(w, h, |x, y| perlin(x - 3.0, y - 2.0, 41));
        let f = flow_with(&a, &b, &FlowSettings::default());
        let valid: usize = f.valid.iter().filter(|&&v| v == 1).count();
        let frac = valid as f32 / (w * h) as f32;
        assert!(
            frac > 0.9,
            "a clean translation should be almost entirely explained: {frac}"
        );
    }

    /// GPU synthesis agrees with the CPU oracle to within a few 8-bit steps.
    ///
    /// Not bit-equality, on purpose: docs/08 §3.1 pins the contract as
    /// "vector-field tolerance, then bit-tolerant synthesis". The GPU path
    /// samples the working-resolution field directly where the CPU upsamples it
    /// first, which is the whole reason it is fast, and the two orders of
    /// bilinear interpolation differ in the last digit rather than in what they
    /// mean. What must hold is that they are the same *picture*.
    #[test]
    fn gpu_synthesis_matches_the_cpu_within_tolerance() {
        let Some(_) = gpu_flow() else { return };
        let ctx = match lumit_gpu::GpuContext::headless() {
            Ok(c) => c,
            Err(_) => return,
        };
        let Ok(gs) = synth::GpuSynth::new(&ctx) else {
            return;
        };
        let (w, h) = (128, 96);
        let to_rgba = |g: &Gray| -> Vec<u8> {
            let mut f = vec![0u8; w * h * 4];
            for i in 0..w * h {
                let v = (g.data[i] * 255.0).round().clamp(0.0, 255.0) as u8;
                f[i * 4] = v;
                f[i * 4 + 1] = v.wrapping_add(20);
                f[i * 4 + 2] = v.wrapping_sub(15);
                f[i * 4 + 3] = 255;
            }
            f
        };
        let ga = render(w, h, |x, y| perlin(x, y, 51));
        let gb = render(w, h, |x, y| perlin(x - 4.0, y - 2.0, 51));
        let (a, b) = (to_rgba(&ga), to_rgba(&gb));
        let set = FlowSettings::default();
        let (fwd, bwd) = flow_pair_with(&ga, &gb, &set);
        let gpu = gs
            .synthesize(&a, &b, w, h, &fwd, &bwd, 0.5, &set)
            .expect("gpu synthesis");
        let hud = hud_weights(&ga, &fwd);
        let cpu = synthesize_with(&a, &b, w, h, &fwd, &bwd, 0.5, &set, Some(&hud));
        let (mut worst, mut sum) = (0i32, 0i64);
        for (g, c) in gpu.iter().zip(&cpu) {
            let d = (i32::from(*g) - i32::from(*c)).abs();
            worst = worst.max(d);
            sum += i64::from(d);
        }
        let mean = sum as f64 / cpu.len() as f64;
        assert!(
            mean < 2.0,
            "gpu and cpu synthesis differ by {mean} on average (worst {worst})"
        );
    }

    /// Real footage is not a clean synthetic translation, and validity must
    /// survive that.
    ///
    /// The regression this pins: validity changed from "a patch covered me" to
    /// "the residual is under an absolute 0.12", and on a fast camera move —
    /// where the source is itself motion-blurred, compressed and noisy —
    /// almost nothing met that. `confidence` hard-zeros on invalid, so Fast
    /// motion blur's streak collapsed and the blur appeared only in scattered
    /// patches. A detailed region has a larger residual than a flat one *even
    /// when the flow is right*, so the test has to be relative to contrast.
    #[test]
    fn validity_survives_real_footage_conditions() {
        let (w, h) = (256, 192);
        let (dx, dy) = (24.0f32, 6.0f32); // a fast flick, not a gentle pan
                                          // What makes this hard is not the distance, it is that a frame taken
                                          // during a fast move is *itself* smeared: the shutter was open while
                                          // the camera turned. So each frame is the scene averaged along its own
                                          // motion, and the two smears do not line up. Add per-frame grain and a
                                          // slight exposure change, both of which real capture has.
        let grain = |x: f32, y: f32, seed: u32| detail(x * 3.1, y * 2.7, seed) - 0.5;
        let scene = |x: f32, y: f32| 0.55 * perlin(x, y, 61) + 0.35 * detail(x, y, 62);
        // Box-average along the motion — the source's own motion blur.
        let smeared = move |x: f32, y: f32, ox: f32, oy: f32| {
            let mut acc = 0.0;
            for k in 0..7 {
                let t = k as f32 / 6.0 - 0.5;
                acc += scene(x - ox + t * dx, y - oy + t * dy);
            }
            acc / 7.0
        };
        let a = render(w, h, |x, y| {
            smeared(x, y, 0.0, 0.0) + 0.05 * grain(x, y, 90)
        });
        let b = render(w, h, |x, y| {
            0.97 * smeared(x, y, dx, dy) + 0.05 * grain(x, y, 91)
        });
        let f = flow_with(&a, &b, &FlowSettings::default());
        let valid = f.valid.iter().filter(|&&v| v == 1).count();
        let frac = valid as f32 / (w * h) as f32;
        assert!(
            frac > 0.75,
            "most of a moving, noisy, slightly-dimmer frame must still be \
             explained — got {frac}"
        );

        // And the confidence that rides on it must not collapse: Fast motion
        // blur scales its streak by this, so a mostly-zero field is a mostly
        // unblurred picture (FX-19).
        let (fwd, bwd) = flow_pair_with(&a, &b, &FlowSettings::default());
        let conf = confidence(&fwd, &bwd);
        let mean = conf.iter().sum::<f32>() / conf.len() as f32;
        assert!(
            mean > 0.5,
            "confidence over ordinary moving footage should be mostly high — \
             got {mean}"
        );
    }

    /// Engine crates never fault: degenerate inputs degrade, not crash.
    #[test]
    fn tiny_frames_degrade_gracefully() {
        let (w, h) = (6, 6);
        let a = vec![10u8; w * h * 4];
        let b = vec![200u8; w * h * 4];
        let set = FlowSettings::default();
        let f = flow_with(&to_gray(&a, w, h), &to_gray(&b, w, h), &set);
        assert!(f.u.iter().all(|&u| u == 0.0));
        assert!(f.valid.iter().all(|&v| v == 0));
        let mid = FlowEngine::cpu().interpolate_at(&a, &b, w, h, 0.5, &set);
        assert_eq!(mid.len(), w * h * 4);
    }

    /// Same inputs → same flow, bit for bit (docs/14 §3 determinism).
    #[test]
    fn flow_is_deterministic() {
        let (w, h) = (128, 96);
        let a = render(w, h, |x, y| perlin(x, y, 5));
        let b = render(w, h, |x, y| perlin(x - 4.3, y + 2.1, 5));
        let (f1, g1) = flow_pair_with(&a, &b, &FlowSettings::default());
        let (f2, g2) = flow_pair_with(&a, &b, &FlowSettings::default());
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
        // An all-invalid forward is *dimmed*, not extinguished. It used to go
        // to zero, and that hard cut-off is what left Fast motion blur with
        // scattered patches of blur and hard edges between them on a fast
        // camera move — the very artefact FX-19's smooth confidence exists to
        // avoid, reintroduced by a binary term inside it. A vector nothing
        // could explain photometrically, but whose two directions still agree,
        // is worth some of its streak.
        let c3 = confidence(&field(1.0, 0.0, 0), &g);
        assert!(
            c3.iter().all(|&x| x > 0.0 && x < 0.5),
            "invalid dims confidence rather than killing it"
        );
        // ...and it is still clearly below a pair that is both valid and
        // consistent, or the term would mean nothing.
        assert!(c3.iter().zip(&c).all(|(a, b)| a < b));
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

    fn gpu_flow_on(ctx: &lumit_gpu::GpuContext) -> Option<gpu::GpuFlow> {
        match gpu::GpuFlow::new(ctx) {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("skipping: flow pipelines failed: {e}");
                None
            }
        }
    }

    fn gpu_flow() -> Option<gpu::GpuFlow> {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return None;
        };
        gpu_flow_on(&ctx)
    }

    /// The texture entry point must measure exactly what the CPU-grey entry
    /// point measures (docs/08 §3.2).
    ///
    /// It is the same solver either way — the only difference is where the
    /// level-0 luma comes from — so the two must agree to float noise. The
    /// scene is handed over twice: once as the `Gray` the decode path builds,
    /// and once as the scene-linear RGBA texture a composite actually is, which
    /// `luma.wgsl` puts back through the sRGB transfer before taking BT.709
    /// luma. If that transfer were dropped, the correlation would run on linear
    /// numbers and the fields would visibly part company.
    #[test]
    fn gpu_texture_entry_matches_the_gray_entry() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let Some(mut g) = gpu_flow_on(&ctx) else {
            return;
        };
        let (w, h) = (192, 160);
        let a = render(w, h, |x, y| perlin(x, y, 1));
        let b = render(w, h, |x, y| perlin(x - 7.3, y - 3.9, 1));

        // sRGB-encoded grey → the scene-linear RGBA a composite carries.
        let decode = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let upload = |g: &Gray| {
            let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("flow-entry-test"),
                size: wgpu::Extent3d {
                    width: g.w as u32,
                    height: g.h as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let mut px = vec![0f32; g.w * g.h * 4];
            for (i, &v) in g.data.iter().enumerate() {
                let lin = decode(v);
                px[i * 4] = lin;
                px[i * 4 + 1] = lin;
                px[i * 4 + 2] = lin;
                px[i * 4 + 3] = 1.0;
            }
            ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&px),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(g.w as u32 * 16),
                    rows_per_image: Some(g.h as u32),
                },
                wgpu::Extent3d {
                    width: g.w as u32,
                    height: g.h as u32,
                    depth_or_array_layers: 1,
                },
            );
            tex
        };
        let (ta, tb) = (upload(&a), upload(&b));

        let set = FlowSettings::default();
        let (grey_fwd, _) = g.flow_pair_with(&a, &b, &set).expect("grey entry");
        let (tex_fwd, _) = g.flow_pair_textures(&ta, &tb, &set).expect("texture entry");
        assert_eq!(
            (tex_fwd.w, tex_fwd.h),
            (w, h),
            "measured at the working size"
        );
        // A looser bound than the 1e-3 the two *backends* are held to (§6.5),
        // and for a reason that is not slack: those two are given identical
        // numbers, where these two are given the same picture through a
        // decode-and-re-encode round trip whose `pow` is the shader's own
        // approximation. A hundredth of a pixel on a seven-pixel displacement
        // is that round trip; a dropped transfer, a wrong luma weight or a
        // transposed axis lands orders of magnitude past it.
        let diff = mean_abs_diff(&grey_fwd, &tex_fwd);
        assert!(
            diff < 0.01,
            "the texture entry point must measure what the grey one does; \
             mean absolute difference {diff}"
        );

        // And it is deterministic — a second measurement of the same pair is
        // the same field, which is what the render path's two runs rest on.
        let (again, _) = g.flow_pair_textures(&ta, &tb, &set).expect("texture entry");
        assert_eq!(tex_fwd.u, again.u, "the same pair measures the same way");
        assert_eq!(tex_fwd.v, again.v);

        // Half resolution measures at half the size — the working-size rule the
        // decode path's effect settings use, expressed by the same divisor.
        let half = FlowSettings {
            divisor: 2,
            ..FlowSettings::default()
        };
        let (small, _) = g.flow_pair_textures(&ta, &tb, &half).expect("half res");
        assert_eq!((small.w, small.h), (w / 2, h / 2));
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
        // All three parts of DIS, both backends: the shader now has
        // the refinement, and the oracle's red-black sweeps mean the two can
        // agree step for step rather than merely in spirit.
        //
        // Default settings AND a deliberately non-default set: every knob the
        // settings carry has a GPU expression (the old refusal is gone), and
        // this is what holds the two backends to the same answer under the
        // knobs a user actually turns - fewer iterations, heavier smoothing,
        // a higher pyramid floor, a different refinement budget.
        let tuned = FlowSettings {
            iterations: 8,
            smoothness: 80.0,
            min_level_dim: 24,
            refine_iters: 2,
            ..FlowSettings::default()
        };
        for (i, (a, b)) in scenes.iter().enumerate() {
            for set in [FlowSettings::default(), tuned] {
                let (cf, _) = flow_pair_with(a, b, &set);
                let (gf, _) = g.flow_pair_with(a, b, &set).unwrap();
                let df = mean_abs_diff(&cf, &gf);
                assert!(
                    df < 1e-3,
                    "scene {i} ({:?} iters): fwd CPU/GPU diff {df}",
                    set.iterations
                );
            }
            let (cf, cg) = flow_pair_with(a, b, &FlowSettings::default());
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
        let set = FlowSettings::default();
        assert_eq!(eng.interpolate_at(&a, &b, w, h, 0.0, &set), a);
        assert_eq!(eng.interpolate_at(&a, &b, w, h, 1.0, &set), b);
        let mid = eng.interpolate_at(&a, &b, w, h, 0.5, &set);
        assert_eq!(mid.len(), w * h * 4);
        // A CPU-only engine reports itself honestly and still interpolates.
        let mut cpu = FlowEngine::cpu();
        assert_eq!(cpu.backend(), "dis-cpu");
        assert_eq!(cpu.interpolate_at(&a, &b, w, h, 0.5, &set).len(), mid.len());
    }

    /// Perf numbers (impl note §6.5: flow pair ≤ 4 ms at half-res 1080p on
    /// the reference GPU). Run by hand:
    /// `cargo test -p lumit-flow --release bench_flow -- --ignored --nocapture`
    #[test]
    #[ignore = "manual benchmark; prints timings"]
    fn bench_flow_1080p() {
        let Some(mut g) = gpu_flow() else { return };
        let (w, h) = (960, 540);
        let a = render(w, h, |x, y| perlin(x, y, 3));
        let b = render(w, h, |x, y| perlin(x - 9.7, y + 4.3, 3));
        // Bench the two-part configuration (no variational refinement), the
        // older baseline: it is what the adaptive path runs when the refinement
        // budget is zero, and it keeps this number comparable across the change
        // that added refinement. The GPU runs all three parts these days.
        let two_part = FlowSettings {
            refine_iters: 0,
            ..FlowSettings::default()
        };
        for _ in 0..3 {
            let _ = g.flow_pair_with(&a, &b, &two_part); // warm-up
        }
        let runs = 20;
        let t0 = std::time::Instant::now();
        for _ in 0..runs {
            let _ = g
                .flow_pair_with(&a, &b, &two_part)
                .expect("two-part GPU path");
        }
        let per_pair = t0.elapsed() / runs;
        eprintln!("gpu flow pair, parts 1-2 (960x540): {per_pair:?}");

        for _ in 0..3 {
            let _ = g.flow_pair(&a, &b); // warm the refined plan
        }
        let t0 = std::time::Instant::now();
        for _ in 0..runs {
            let _ = g.flow_pair(&a, &b).expect("refined GPU path");
        }
        eprintln!(
            "gpu flow pair, all three parts (960x540): {:?}",
            t0.elapsed() / runs
        );

        let t0 = std::time::Instant::now();
        let _ = flow_pair_with(&a, &b, &two_part);
        eprintln!(
            "cpu flow pair, parts 1-2 only (960x540): {:?}",
            t0.elapsed()
        );
        let t0 = std::time::Instant::now();
        let _ = flow_pair_with(&a, &b, &FlowSettings::default());
        eprintln!(
            "cpu flow pair, with refinement (960x540): {:?}",
            t0.elapsed()
        );

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
        let set = FlowSettings::default();
        let _ = eng.interpolate_at(&fa, &fb, fw, fh, 0.5, &set); // warm-up
        let t0 = std::time::Instant::now();
        let _ = eng.interpolate_at(&fa, &fb, fw, fh, 0.5, &set);
        eprintln!(
            "end-to-end 1080p interpolate at phi 0.5: {:?}",
            t0.elapsed()
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod float_synth_tests {
    use super::*;

    /// `n` pixels of float RGBA, every colour channel `v`, opaque.
    fn frame(n: usize, v: f32) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..n {
            for c in [v, v, v, 1.0] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }

    fn read(buf: &[u8], i: usize, c: usize) -> f32 {
        Floats::get(buf, i, c)
    }

    /// A still field at half phase is a plain average of the two frames, and on
    /// float frames that average keeps its range: 2.0 and 6.0 give 4.0, not the
    /// 1.0 an eight-bit synthesis would have clamped both ends to first.
    #[test]
    fn float_synthesis_keeps_values_above_white() {
        let (w, h) = (4, 4);
        let n = w * h;
        let a = frame(n, 2.0);
        let b = frame(n, 6.0);
        let still = FlowField {
            w,
            h,
            u: vec![0.0; n],
            valid: vec![1; n],
            v: vec![0.0; n],
        };
        let out = synthesize_with_as::<Floats>(
            &a,
            &b,
            w,
            h,
            &still,
            &still,
            0.5,
            &FlowSettings::default(),
            None,
        );

        assert_eq!(out.len(), n * 16);
        for i in 0..n {
            let got = read(&out, i, 0);
            assert!(
                (got - 4.0).abs() < 1e-5,
                "pixel {i} synthesised as {got}, wanted 4.0"
            );
        }
    }

    /// The degrade path — anything inconsistent falls back to a crossfade — has
    /// to keep the width too, or a mismatched field would turn a float frame
    /// into a quarter-length buffer of nonsense.
    #[test]
    fn the_float_crossfade_fallback_keeps_its_width() {
        let (w, h) = (4, 4);
        let n = w * h;
        let a = frame(n, 2.0);
        let b = frame(n, 6.0);
        // A field of the wrong size is what sends it down the fallback.
        let wrong = FlowField {
            w: 2,
            h: 2,
            u: vec![0.0; 4],
            valid: vec![1; 4],
            v: vec![0.0; 4],
        };
        let out = synthesize_with_as::<Floats>(
            &a,
            &b,
            w,
            h,
            &wrong,
            &wrong,
            0.5,
            &FlowSettings::default(),
            None,
        );

        assert_eq!(out.len(), n * 16);
        assert!((read(&out, 0, 0) - 4.0).abs() < 1e-5);
    }
}

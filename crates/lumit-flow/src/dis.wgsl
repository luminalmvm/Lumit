// DIS optical flow, compute passes (docs/impl/optical-flow.md §1).
//
// Every arithmetic step here mirrors the CPU oracle in lib.rs exactly —
// same loop orders, same clamps, same constants — so the two backends agree
// within float noise (§6.5: within 1e-3 on the analytic tests). One *thread*
// per patch rather than one workgroup: sums then run in the same sequential
// order as the CPU, and the whole search is still far inside the frame
// budget on any real GPU.

struct Params {
    w: u32,   // this level's width
    h: u32,   // this level's height
    pw: u32,  // parent (finer) level's width  — downsample source
    ph: u32,  // parent (finer) level's height
    cw: u32,  // coarser level's width         — init upsample source
    ch: u32,  // coarser level's height
    npx: u32, // patch grid columns
    npy: u32, // patch grid rows
    iters: u32,       // inverse-search iteration cap (Vector detail)
    flow_sigma2: f32, // smoothing flow-range sigma² (Smoothness)
    pad0: u32,
    pad1: u32,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> luma_t: array<f32>;      // template
@group(0) @binding(2) var<storage, read> luma_o: array<f32>;      // other
@group(0) @binding(3) var<storage, read> grad_t: array<vec4f>;    // template Sobel (xy)
@group(0) @binding(4) var<storage, read> flow_in: array<vec4f>;   // init / smooth input
@group(0) @binding(5) var<storage, read> patch_in: array<vec4f>;  // patch results
@group(0) @binding(6) var<storage, read_write> out_vec: array<vec4f>;
@group(0) @binding(7) var<storage, read_write> out_f32: array<f32>;

// Constants — identical to the lib.rs values.
const SIGMA2: f32 = 0.08 * 0.08;
const FLOW_SIGMA2: f32 = 1.0;
// Variational refinement (§1 step 5) — the paper's weights, mirroring lib.rs.
const VR_SIGMA: f32 = 5.0;
const VR_GAMMA: f32 = 10.0;
const VR_ALPHA: f32 = 10.0;
const VR_EPS2: f32 = 0.001 * 0.001;
const VR_OMEGA: f32 = 1.6;
const VR_ZETA2: f32 = 0.1 * 0.1;
const VR_RESIDUAL_FLOOR: f32 = 0.12;
const VR_RESIDUAL_REL: f32 = 3.0;
const DET_MIN: f32 = 1e-6;
const COST_VAR_RATIO: f32 = 0.25;
const COST_FLOOR: f32 = 0.05;
const CONV2: f32 = 0.02 * 0.02;

// Bilinear sample of the template luma, edge-clamped (mirrors sample_scalar).
fn sample_t(xf: f32, yf: f32) -> f32 {
    let x = clamp(xf, 0.0, f32(P.w - 1u));
    let y = clamp(yf, 0.0, f32(P.h - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.w - 1u);
    let y1 = min(y0 + 1u, P.h - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = luma_t[y0 * P.w + x0] * (1.0 - fx) + luma_t[y0 * P.w + x1] * fx;
    let b = luma_t[y1 * P.w + x0] * (1.0 - fx) + luma_t[y1 * P.w + x1] * fx;
    return a * (1.0 - fy) + b * fy;
}

// Same, for the other frame.
fn sample_o(xf: f32, yf: f32) -> f32 {
    let x = clamp(xf, 0.0, f32(P.w - 1u));
    let y = clamp(yf, 0.0, f32(P.h - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.w - 1u);
    let y1 = min(y0 + 1u, P.h - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = luma_o[y0 * P.w + x0] * (1.0 - fx) + luma_o[y0 * P.w + x1] * fx;
    let b = luma_o[y1 * P.w + x0] * (1.0 - fx) + luma_o[y1 * P.w + x1] * fx;
    return a * (1.0 - fy) + b * fy;
}

// Bilinear sample of flow_in's xy, over arbitrary dims (sw, sh).
fn sample_flow(xf: f32, yf: f32, sw: u32, sh: u32) -> vec2f {
    let x = clamp(xf, 0.0, f32(sw - 1u));
    let y = clamp(yf, 0.0, f32(sh - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, sw - 1u);
    let y1 = min(y0 + 1u, sh - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = flow_in[y0 * sw + x0].xy * (1.0 - fx) + flow_in[y0 * sw + x1].xy * fx;
    let b = flow_in[y1 * sw + x0].xy * (1.0 - fx) + flow_in[y1 * sw + x1].xy * fx;
    return a * (1.0 - fy) + b * fy;
}

fn patch_origin_x(i: u32) -> u32 {
    return min(i * 4u, P.w - 8u);
}
fn patch_origin_y(j: u32) -> u32 {
    return min(j * 4u, P.h - 8u);
}

// Box-downsample ×2: parent luma (pw×ph, binding 1) → this level (binding 7).
@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let x0 = min(2u * x, P.pw - 1u);
    let y0 = min(2u * y, P.ph - 1u);
    let x1 = min(2u * x + 1u, P.pw - 1u);
    let y1 = min(2u * y + 1u, P.ph - 1u);
    out_f32[y * P.w + x] = 0.25
        * (luma_t[y0 * P.pw + x0] + luma_t[y0 * P.pw + x1]
            + luma_t[y1 * P.pw + x0] + luma_t[y1 * P.pw + x1]);
}

// Sobel gradients (÷8), clamped borders — template luma → out_vec.xy.
@compute @workgroup_size(8, 8)
fn sobel(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let xm = select(x - 1u, 0u, x == 0u);
    let xp = min(x + 1u, P.w - 1u);
    let ym = select(y - 1u, 0u, y == 0u);
    let yp = min(y + 1u, P.h - 1u);
    let tl = luma_t[ym * P.w + xm];
    let t = luma_t[ym * P.w + x];
    let tr = luma_t[ym * P.w + xp];
    let l = luma_t[y * P.w + xm];
    let r = luma_t[y * P.w + xp];
    let bl = luma_t[yp * P.w + xm];
    let b = luma_t[yp * P.w + x];
    let br = luma_t[yp * P.w + xp];
    let gx = ((tr + 2.0 * r + br) - (tl + 2.0 * l + bl)) / 8.0;
    let gy = ((bl + 2.0 * b + br) - (tl + 2.0 * t + tr)) / 8.0;
    out_vec[y * P.w + x] = vec4f(gx, gy, 0.0, 0.0);
}

// Init: coarser dense flow (cw×ch, binding 4) → this level, values ×(w/cw).
@compute @workgroup_size(8, 8)
fn upsample_init(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let scale = f32(P.w) / f32(max(P.cw, 1u));
    let sx = f32(x) * f32(P.cw) / f32(P.w);
    let sy = f32(y) * f32(P.ch) / f32(P.h);
    let f = sample_flow(sx, sy, P.cw, P.ch) * scale;
    out_vec[y * P.w + x] = vec4f(f, 0.0, 0.0);
}

// The inverse search (§1 step 2), one thread per patch — sequential sums in
// the same order as the CPU oracle.
@compute @workgroup_size(8, 8)
fn inverse_search(@builtin(global_invocation_id) gid: vec3u) {
    let pi = gid.x;
    let pj = gid.y;
    if (pi >= P.npx || pj >= P.npy) {
        return;
    }
    let x0 = patch_origin_x(pi);
    let y0 = patch_origin_y(pj);
    // Template Hessian, mean and energy.
    var h11 = 0.0;
    var h12 = 0.0;
    var h22 = 0.0;
    var sum_a = 0.0;
    var sum_a2 = 0.0;
    for (var dy = 0u; dy < 8u; dy++) {
        for (var dx = 0u; dx < 8u; dx++) {
            let i = (y0 + dy) * P.w + (x0 + dx);
            let g = grad_t[i].xy;
            h11 += g.x * g.x;
            h12 += g.x * g.y;
            h22 += g.y * g.y;
            sum_a += luma_t[i];
            sum_a2 += luma_t[i] * luma_t[i];
        }
    }
    let np = 64.0;
    let variance = sum_a2 - sum_a * sum_a / np;
    let det = h11 * h22 - h12 * h12;
    // Candidate inits: centre, corners, one patch-length out (same order as
    // the CPU table).
    let cands = array<vec2f, 9>(
        vec2f(3.5, 3.5),
        vec2f(0.5, 0.5),
        vec2f(6.5, 0.5),
        vec2f(0.5, 6.5),
        vec2f(6.5, 6.5),
        vec2f(-4.5, 3.5),
        vec2f(11.5, 3.5),
        vec2f(3.5, -4.5),
        vec2f(3.5, 11.5),
    );
    var u = 0.0;
    var v = 0.0;
    var cand_best = 1e30;
    for (var c = 0u; c < 9u; c++) {
        let s = vec2f(f32(x0), f32(y0)) + cands[c];
        let cf = sample_flow(s.x, s.y, P.w, P.h);
        var ssd = 0.0;
        for (var dy = 0u; dy < 8u; dy++) {
            for (var dx = 0u; dx < 8u; dx++) {
                let i = (y0 + dy) * P.w + (x0 + dx);
                let e = luma_t[i] - sample_o(f32(x0 + dx) + cf.x, f32(y0 + dy) + cf.y);
                ssd += e * e;
            }
        }
        if (ssd < cand_best) {
            cand_best = ssd;
            u = cf.x;
            v = cf.y;
        }
    }
    var ok = det >= DET_MIN;
    if (ok) {
        var bu = u;
        var bv = v;
        var best = 1e30;
        for (var it = 0u; it < P.iters; it++) {
            var r1 = 0.0;
            var r2 = 0.0;
            var cost = 0.0;
            for (var dy = 0u; dy < 8u; dy++) {
                for (var dx = 0u; dx < 8u; dx++) {
                    let i = (y0 + dy) * P.w + (x0 + dx);
                    let e = luma_t[i] - sample_o(f32(x0 + dx) + u, f32(y0 + dy) + v);
                    r1 += grad_t[i].x * e;
                    r2 += grad_t[i].y * e;
                    cost += e * e;
                }
            }
            if (cost >= best) {
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
            if (du * du + dv * dv < CONV2) {
                break;
            }
        }
        ok = best <= COST_VAR_RATIO * variance + COST_FLOOR;
    }
    out_vec[pj * P.npx + pi] = vec4f(u, v, select(0.0, 1.0, ok), 0.0);
}

// Densification (§1 step 3): winning-cluster weighted average of covering
// patch votes, with the photometrically-gated 5×5 rescue.
@compute @workgroup_size(8, 8)
fn densify(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    let gi = i32(x / 4u);
    let gj = i32(y / 4u);
    var best_w = 0.0;
    var best_u = 0.0;
    var best_v = 0.0;
    var votes: array<vec3f, 9>;
    var n_votes = 0u;
    for (var oj = -1; oj <= 1; oj++) {
        let cj = gj + oj;
        if (cj < 0 || cj >= i32(P.npy)) {
            continue;
        }
        for (var oi = -1; oi <= 1; oi++) {
            let ci = gi + oi;
            if (ci < 0 || ci >= i32(P.npx)) {
                continue;
            }
            let x0 = patch_origin_x(u32(ci));
            let y0 = patch_origin_y(u32(cj));
            if (x < x0 || x > x0 + 7u || y < y0 || y > y0 + 7u) {
                continue;
            }
            let p = patch_in[u32(cj) * P.npx + u32(ci)];
            if (p.z < 0.5) {
                continue;
            }
            let err = sample_o(f32(x) + p.x, f32(y) + p.y) - luma_t[i];
            let wgt = exp(-(err * err) / SIGMA2);
            votes[n_votes] = vec3f(wgt, p.x, p.y);
            n_votes += 1u;
            if (wgt > best_w) {
                best_w = wgt;
                best_u = p.x;
                best_v = p.y;
            }
        }
    }
    var acc_u = 0.0;
    var acc_v = 0.0;
    var wsum = 0.0;
    for (var k = 0u; k < n_votes; k++) {
        let vt = votes[k];
        let d2 = (vt.y - best_u) * (vt.y - best_u) + (vt.z - best_v) * (vt.z - best_v);
        if (d2 <= FLOW_SIGMA2 * 4.0) {
            wsum += vt.x;
            acc_u += vt.x * vt.y;
            acc_v += vt.x * vt.z;
        }
    }
    if (wsum <= 1e-12) {
        // Second chance: borrow hypotheses from the wider 5×5 neighbourhood.
        for (var oj = -2; oj <= 2; oj++) {
            let cj = gj + oj;
            if (cj < 0 || cj >= i32(P.npy)) {
                continue;
            }
            for (var oi = -2; oi <= 2; oi++) {
                let ci = gi + oi;
                if (ci < 0 || ci >= i32(P.npx)) {
                    continue;
                }
                let p = patch_in[u32(cj) * P.npx + u32(ci)];
                if (p.z < 0.5) {
                    continue;
                }
                let err = sample_o(f32(x) + p.x, f32(y) + p.y) - luma_t[i];
                let wgt = exp(-(err * err) / SIGMA2);
                wsum += wgt;
                acc_u += wgt * p.x;
                acc_v += wgt * p.y;
            }
        }
    }
    if (wsum > 1e-12) {
        out_vec[i] = vec4f(acc_u / wsum, acc_v / wsum, 1.0, 0.0);
    } else {
        out_vec[i] = vec4f(flow_in[i].xy, 0.0, 0.0);
    }
}

// Smoothing (§1 step 4): one 3×3 bilateral on luma and flow difference.
// Validity rides through in z.
@compute @workgroup_size(8, 8)
fn smooth_flow(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    let c = luma_t[i];
    let centre = flow_in[i];
    var acc_u = 0.0;
    var acc_v = 0.0;
    var wsum = 0.0;
    for (var oy = -1; oy <= 1; oy++) {
        for (var ox = -1; ox <= 1; ox++) {
            let qx = u32(clamp(i32(x) + ox, 0, i32(P.w) - 1));
            let qy = u32(clamp(i32(y) + oy, 0, i32(P.h) - 1));
            let q = qy * P.w + qx;
            let d = luma_t[q] - c;
            let fd = (flow_in[q].x - centre.x) * (flow_in[q].x - centre.x)
                + (flow_in[q].y - centre.y) * (flow_in[q].y - centre.y);
            let wgt = exp(-(d * d) / SIGMA2) * exp(-fd / P.flow_sigma2);
            wsum += wgt;
            acc_u += wgt * flow_in[q].x;
            acc_v += wgt * flow_in[q].y;
        }
    }
    out_vec[i] = vec4f(acc_u / wsum, acc_v / wsum, centre.z, 0.0);
}

// ---------------------------------------------------------------------------
// Variational refinement â€” DIS part three (Â§1 step 5, K-332).
//
// Four short kernels plus a two-colour solver, mirroring `refine` in lib.rs
// step for step. The SOR sweeps are redâ€“black in the oracle *because* of this
// file: on a checkerboard every neighbour of a red pixel is black, so a whole
// colour updates with no thread reading a value another thread is writing.
//
// Buffer roles while refining (the shared layout, no new bindings):
//   binding 1  luma_t   frame A
//   binding 2  luma_o   frame B
//   binding 3  grad_t   A's Sobel (ax, ay) â€” or B's, in vr_warp
//   binding 4  flow_in  the dense field, or warp/duv depending on the pass
//   binding 5  patch_in warp2 (bwxx, bwxy, bwyx, bwyy)
//   binding 6  out_vec  the pass's output
//
// `duv` packs (du, dv, u, v) into one vec4 so the solver needs no fifth read
// slot: the increment being solved for and the flow it is an increment of
// travel together.
// ---------------------------------------------------------------------------

// Bilinear sample of grad_t's xy at this level's dims.
fn sample_grad(xf: f32, yf: f32) -> vec2f {
    let x = clamp(xf, 0.0, f32(P.w - 1u));
    let y = clamp(yf, 0.0, f32(P.h - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.w - 1u);
    let y1 = min(y0 + 1u, P.h - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = grad_t[y0 * P.w + x0].xy * (1.0 - fx) + grad_t[y0 * P.w + x1].xy * fx;
    let b = grad_t[y1 * P.w + x0].xy * (1.0 - fx) + grad_t[y1 * P.w + x1].xy * fx;
    return a * (1.0 - fy) + b * fy;
}

// Warp B (and its gradients) by the current flow, and pre-form the temporal
// difference. Out: (iz, ix, iy, 0) â€” grad_t is bound to B's Sobel here.
@compute @workgroup_size(8, 8)
fn vr_warp(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    let f = flow_in[i].xy;
    let sx = f32(x) + f.x;
    let sy = f32(y) + f.y;
    let bw = sample_o(sx, sy);
    let bg = sample_grad(sx, sy);
    out_vec[i] = vec4f(bw - luma_t[i], bg.x, bg.y, 0.0);
}

// Seed the solver: the increment starts at zero, carrying the flow it refines.
@compute @workgroup_size(8, 8)
fn vr_init_duv(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    out_vec[i] = vec4f(0.0, 0.0, flow_in[i].x, flow_in[i].y);
}

// Second derivatives of the warped frame: Sobel of (ix, iy) from vr_warp.
// Out: (bwxx, bwxy, bwyx, bwyy).
@compute @workgroup_size(8, 8)
fn vr_deriv(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let xm = select(x - 1u, 0u, x == 0u);
    let xp = min(x + 1u, P.w - 1u);
    let ym = select(y - 1u, 0u, y == 0u);
    let yp = min(y + 1u, P.h - 1u);
    // .y is bwx, .z is bwy; Sobel each, as sobel(&Gray{..bwx}) does on the CPU.
    let tl = flow_in[ym * P.w + xm].yz;
    let t = flow_in[ym * P.w + x].yz;
    let tr = flow_in[ym * P.w + xp].yz;
    let l = flow_in[y * P.w + xm].yz;
    let r = flow_in[y * P.w + xp].yz;
    let bl = flow_in[yp * P.w + xm].yz;
    let b = flow_in[yp * P.w + x].yz;
    let br = flow_in[yp * P.w + xp].yz;
    let gx = ((tr + 2.0 * r + br) - (tl + 2.0 * l + bl)) / 8.0;
    let gy = ((bl + 2.0 * b + br) - (tl + 2.0 * t + tr)) / 8.0;
    // gx.x = d(bwx)/dx = bwxx, gy.x = bwxy, gx.y = bwyx, gy.y = bwyy.
    out_vec[y * P.w + x] = vec4f(gx.x, gy.x, gx.y, gy.y);
}

// One SOR step for the pixels of one checkerboard colour. out_vec is duv, read
// and written in place; every neighbour read is the opposite colour, so no
// thread ever sees a value another thread is part-way through writing.
fn vr_sor(x: u32, y: u32) {
    let i = y * P.w + x;
    let d = out_vec[i];
    let du = d.x;
    let dv = d.y;
    let u = d.z;
    let v = d.w;

    let warp = flow_in[i];         // (iz, ix, iy, _)
    let iz = warp.x;
    let ix = warp.y;
    let iy = warp.z;
    let e_i = iz + ix * du + iy * dv;
    let n_i = 1.0 / (ix * ix + iy * iy + VR_ZETA2);
    let psi_i = VR_SIGMA * n_i / (2.0 * sqrt(e_i * e_i + VR_EPS2));

    let w2 = patch_in[i];          // (bwxx, bwxy, bwyx, bwyy)
    let ag = grad_t[i].xy;         // A's Sobel
    let gzx = ix - ag.x;
    let gzy = iy - ag.y;
    let e_gx = gzx + w2.x * du + w2.y * dv;
    let e_gy = gzy + w2.z * du + w2.w * dv;
    let n_g = 1.0 / (w2.x * w2.x + w2.y * w2.y + w2.z * w2.z + w2.w * w2.w + VR_ZETA2);
    let psi_g = VR_GAMMA * n_g / (2.0 * sqrt(e_gx * e_gx + e_gy * e_gy + VR_EPS2));

    let a11 = psi_i * ix * ix + psi_g * (w2.x * w2.x + w2.z * w2.z);
    let a12 = psi_i * ix * iy + psi_g * (w2.x * w2.y + w2.z * w2.w);
    let a22 = psi_i * iy * iy + psi_g * (w2.y * w2.y + w2.w * w2.w);
    let b1 = -(psi_i * ix * iz + psi_g * (w2.x * gzx + w2.z * gzy));
    let b2 = -(psi_i * iy * iz + psi_g * (w2.y * gzx + w2.w * gzy));

    // Smoothness, four-neighbour, each weighted by how smooth the field already
    // is across that edge. Out-of-range neighbours are skipped, so a border
    // pixel is pulled by fewer â€” identical to the oracle's bounds test.
    var s_acc_u = 0.0;
    var s_acc_v = 0.0;
    var s_wsum = 0.0;
    for (var k = 0u; k < 4u; k++) {
        var nx = i32(x);
        var ny = i32(y);
        switch k {
            case 0u: { nx -= 1; }
            case 1u: { nx += 1; }
            case 2u: { ny -= 1; }
            default: { ny += 1; }
        }
        if (nx < 0 || ny < 0 || nx >= i32(P.w) || ny >= i32(P.h)) {
            continue;
        }
        let nd = out_vec[u32(ny) * P.w + u32(nx)];
        let dux = nd.z + nd.x - u - du;
        let dvy = nd.w + nd.y - v - dv;
        let wgt = VR_ALPHA / (2.0 * sqrt(dux * dux + dvy * dvy + VR_EPS2));
        s_wsum += wgt;
        s_acc_u += wgt * (nd.z + nd.x - u);
        s_acc_v += wgt * (nd.w + nd.y - v);
    }

    var out_du = du;
    var out_dv = dv;
    let den_u = a11 + s_wsum;
    if (den_u > 1e-12) {
        out_du += VR_OMEGA * ((b1 - a12 * dv + s_acc_u) / den_u - du);
    }
    let den_v = a22 + s_wsum;
    if (den_v > 1e-12) {
        // Gauss-Seidel: v steps on u's just-updated value, as the oracle does.
        out_dv += VR_OMEGA * ((b2 - a12 * out_du + s_acc_v) / den_v - dv);
    }
    out_vec[i] = vec4f(out_du, out_dv, u, v);
}

@compute @workgroup_size(8, 8)
fn vr_sor_red(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x >= P.w || gid.y >= P.h || (gid.x + gid.y) % 2u != 0u) {
        return;
    }
    vr_sor(gid.x, gid.y);
}

@compute @workgroup_size(8, 8)
fn vr_sor_black(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x >= P.w || gid.y >= P.h || (gid.x + gid.y) % 2u != 1u) {
        return;
    }
    vr_sor(gid.x, gid.y);
}

// Fold the solved increment back into the flow field.
@compute @workgroup_size(8, 8)
fn vr_apply(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    let d = flow_in[i]; // duv
    out_vec[i] = vec4f(d.z + d.x, d.w + d.y, 0.0, 0.0);
}

// Validity from the residual of the refined field (K-332): does this flow
// actually explain these pixels? Rewrites .z of the dense field in place.
@compute @workgroup_size(8, 8)
fn vr_validity(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    let f = out_vec[i].xy;
    let r = sample_o(f32(x) + f.x, f32(y) + f.y) - luma_t[i];
    // Forgiven in proportion to local contrast: a busy region leaves a bigger
    // residual than a flat one even when the flow is right.
    let ag = grad_t[i].xy;
    let allow = VR_RESIDUAL_FLOOR + VR_RESIDUAL_REL * length(ag);
    out_vec[i] = vec4f(f, select(0.0, 1.0, abs(r) <= allow), 0.0);
}

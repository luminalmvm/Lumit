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
        for (var it = 0u; it < 12u; it++) {
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
            let wgt = exp(-(d * d) / SIGMA2) * exp(-fd / FLOW_SIGMA2);
            wsum += wgt;
            acc_u += wgt * flow_in[q].x;
            acc_v += wgt * flow_in[q].y;
        }
    }
    out_vec[i] = vec4f(acc_u / wsum, acc_v / wsum, centre.z, 0.0);
}

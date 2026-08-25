// Frame synthesis and its supporting masks (docs/impl/optical-flow.md §2, §3).
//
// Unlike dis.wgsl, this is *not* held to bit-parity with the CPU oracle: docs/08
// §3.1 pins the tolerance as "vector-field tolerance, then bit-tolerant
// synthesis", because the field is the measurement and synthesis is a resample
// of it. That freedom buys the two things that make this fast — the flow stays
// at its working resolution instead of being upsampled to frame size first, and
// nothing round-trips through the CPU.
//
// Everything below still runs identically in preview and export (K-031): same
// kernels, same order, same inputs.

struct SP {
    w: u32,        // frame width
    h: u32,        // frame height
    fw: u32,       // flow-field width
    fh: u32,       // flow-field height
    phi: f32,      // phase in (0, 1)
    occ_mode: u32, // 0 visible-only, 1 blend
    fallback: u32, // 0 blend, 1 nearest
    hud_on: u32,
}

@group(0) @binding(0) var<uniform> P: SP;
@group(0) @binding(1) var<storage, read> frame_a: array<u32>;
@group(0) @binding(2) var<storage, read> frame_b: array<u32>;
@group(0) @binding(3) var<storage, read> fwd: array<vec4f>;
@group(0) @binding(4) var<storage, read> bwd: array<vec4f>;
// Working-res scratch: .x raw occ_a, .y raw occ_b, .z raw hud, .w luma of A.
@group(0) @binding(5) var<storage, read_write> aux: array<vec4f>;
// Working-res finished masks: .x occ_a dilated, .y occ_b dilated, .z hud blurred.
@group(0) @binding(6) var<storage, read_write> aux2: array<vec4f>;
@group(0) @binding(7) var<storage, read_write> out_px: array<u32>;

const OCC_ABS: f32 = 1.5;
const OCC_REL: f32 = 0.05;
const SYNTH_EPS: f32 = 1e-4;
const HUD_STATIC_LO: f32 = 0.25;
const HUD_STATIC_HI: f32 = 1.0;
const HUD_TEX_LO: f32 = 0.02;
const HUD_TEX_HI: f32 = 0.08;

fn unpack(p: u32) -> vec4f {
    return vec4f(
        f32(p & 0xffu),
        f32((p >> 8u) & 0xffu),
        f32((p >> 16u) & 0xffu),
        f32((p >> 24u) & 0xffu),
    );
}

fn pack(c: vec4f) -> u32 {
    let q = vec4u(round(clamp(c, vec4f(0.0), vec4f(255.0))));
    return q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
}

// Bilinear RGBA sample of a frame buffer, edge-clamped.
fn sample_frame(which: u32, xf: f32, yf: f32) -> vec4f {
    let x = clamp(xf, 0.0, f32(P.w - 1u));
    let y = clamp(yf, 0.0, f32(P.h - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.w - 1u);
    let y1 = min(y0 + 1u, P.h - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    var p00: u32; var p10: u32; var p01: u32; var p11: u32;
    if (which == 0u) {
        p00 = frame_a[y0 * P.w + x0];
        p10 = frame_a[y0 * P.w + x1];
        p01 = frame_a[y1 * P.w + x0];
        p11 = frame_a[y1 * P.w + x1];
    } else {
        p00 = frame_b[y0 * P.w + x0];
        p10 = frame_b[y0 * P.w + x1];
        p01 = frame_b[y1 * P.w + x0];
        p11 = frame_b[y1 * P.w + x1];
    }
    let a = unpack(p00) * (1.0 - fx) + unpack(p10) * fx;
    let b = unpack(p01) * (1.0 - fx) + unpack(p11) * fx;
    return a * (1.0 - fy) + b * fy;
}

// Bilinear sample of a flow field at working-res coordinates.
fn sample_field(which: u32, xf: f32, yf: f32) -> vec2f {
    let x = clamp(xf, 0.0, f32(P.fw - 1u));
    let y = clamp(yf, 0.0, f32(P.fh - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.fw - 1u);
    let y1 = min(y0 + 1u, P.fh - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    var v00: vec2f; var v10: vec2f; var v01: vec2f; var v11: vec2f;
    if (which == 0u) {
        v00 = fwd[y0 * P.fw + x0].xy; v10 = fwd[y0 * P.fw + x1].xy;
        v01 = fwd[y1 * P.fw + x0].xy; v11 = fwd[y1 * P.fw + x1].xy;
    } else {
        v00 = bwd[y0 * P.fw + x0].xy; v10 = bwd[y0 * P.fw + x1].xy;
        v01 = bwd[y1 * P.fw + x0].xy; v11 = bwd[y1 * P.fw + x1].xy;
    }
    let a = v00 * (1.0 - fx) + v10 * fx;
    let b = v01 * (1.0 - fx) + v11 * fx;
    return a * (1.0 - fy) + b * fy;
}

fn field_scale() -> f32 {
    return f32(P.w) / f32(max(P.fw, 1u));
}

// Raw forward-backward consistency both ways (§2), plus A's luma for the guard.
@compute @workgroup_size(8, 8)
fn syn_prep(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.fw || y >= P.fh) {
        return;
    }
    let i = y * P.fw + x;
    let f = fwd[i];
    let g = bwd[i];
    // occ_a: A-pixels with no match in B. occ_b: the mirror.
    var occ_a = 1.0;
    if (f.z >= 0.5) {
        let gv = sample_field(1u, f32(x) + f.x, f32(y) + f.y);
        let cn = length(f.xy + gv);
        let thr = max(OCC_REL * (length(f.xy) + length(gv)), OCC_ABS);
        occ_a = select(0.0, 1.0, cn > thr);
    }
    var occ_b = 1.0;
    if (g.z >= 0.5) {
        let fv = sample_field(0u, f32(x) + g.x, f32(y) + g.y);
        let cn = length(g.xy + fv);
        let thr = max(OCC_REL * (length(g.xy) + length(fv)), OCC_ABS);
        occ_b = select(0.0, 1.0, cn > thr);
    }
    // A's luma at this flow pixel, for the HUD guard's texture test.
    let s = field_scale();
    let c = sample_frame(0u, f32(x) * s, f32(y) * s);
    let luma = (0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z) / 255.0;
    aux[i] = vec4f(occ_a, occ_b, 0.0, luma);
}

fn luma_at(x: i32, y: i32) -> f32 {
    let qx = u32(clamp(x, 0, i32(P.fw) - 1));
    let qy = u32(clamp(y, 0, i32(P.fh) - 1));
    return aux[qy * P.fw + qx].w;
}

// Sobel magnitude of A's luma at (x, y).
fn grad_mag(x: i32, y: i32) -> f32 {
    let tl = luma_at(x - 1, y - 1);
    let t = luma_at(x, y - 1);
    let tr = luma_at(x + 1, y - 1);
    let l = luma_at(x - 1, y);
    let r = luma_at(x + 1, y);
    let bl = luma_at(x - 1, y + 1);
    let b = luma_at(x, y + 1);
    let br = luma_at(x + 1, y + 1);
    let gx = ((tr + 2.0 * r + br) - (tl + 2.0 * l + bl)) / 8.0;
    let gy = ((bl + 2.0 * b + br) - (tl + 2.0 * t + tr)) / 8.0;
    return sqrt(gx * gx + gy * gy);
}

fn smoothstep_lh(lo: f32, hi: f32, v: f32) -> f32 {
    let t = clamp((v - lo) / (hi - lo), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The HUD guard (§3.1 step 5): still × detailed. The texture term takes a 3×3
// max of the gradient first — a gradient is zero inside every stroke of a glyph
// and spikes only at its rim, so the per-pixel form guards a HUD's outlines and
// leaves its insides to smear.
@compute @workgroup_size(8, 8)
fn syn_hud(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.fw || y >= P.fh) {
        return;
    }
    let i = y * P.fw + x;
    let speed = length(fwd[i].xy);
    let stillness = 1.0 - smoothstep_lh(HUD_STATIC_LO, HUD_STATIC_HI, speed);
    var w = 0.0;
    if (stillness > 0.0 && P.hud_on != 0u) {
        var m = 0.0;
        for (var oy = -1; oy <= 1; oy++) {
            for (var ox = -1; ox <= 1; ox++) {
                m = max(m, grad_mag(i32(x) + ox, i32(y) + oy));
            }
        }
        w = stillness * smoothstep_lh(HUD_TEX_LO, HUD_TEX_HI, m);
    }
    let a = aux[i];
    aux[i] = vec4f(a.x, a.y, w, a.w);
}

// Dilate the occlusion masks by a pixel (§2: the consistency test under-detects
// at exact boundaries) and box-blur the guard so its taper has no seam.
@compute @workgroup_size(8, 8)
fn syn_post(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.fw || y >= P.fh) {
        return;
    }
    var ma = 0.0;
    var mb = 0.0;
    var hsum = 0.0;
    for (var oy = -1; oy <= 1; oy++) {
        for (var ox = -1; ox <= 1; ox++) {
            let qx = u32(clamp(i32(x) + ox, 0, i32(P.fw) - 1));
            let qy = u32(clamp(i32(y) + oy, 0, i32(P.fh) - 1));
            let v = aux[qy * P.fw + qx];
            ma = max(ma, v.x);
            mb = max(mb, v.y);
            hsum += v.z;
        }
    }
    aux2[y * P.fw + x] = vec4f(ma, mb, hsum / 9.0, 0.0);
}

// Bilinear read of the finished masks at working-res coordinates.
fn sample_masks(xf: f32, yf: f32) -> vec3f {
    let x = clamp(xf, 0.0, f32(P.fw - 1u));
    let y = clamp(yf, 0.0, f32(P.fh - 1u));
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, P.fw - 1u);
    let y1 = min(y0 + 1u, P.fh - 1u);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = aux2[y0 * P.fw + x0].xyz * (1.0 - fx) + aux2[y0 * P.fw + x1].xyz * fx;
    let b = aux2[y1 * P.fw + x0].xyz * (1.0 - fx) + aux2[y1 * P.fw + x1].xyz * fx;
    return a * (1.0 - fy) + b * fy;
}

// Synthesis at phase φ (§3): backward-warp both endpoints and blend with
// occlusion-aware weights, one fixed-point step toward the flow at the
// destination, then mix back toward the plain blend where the guard fired.
@compute @workgroup_size(8, 8)
fn syn_blend(@builtin(global_invocation_id) gid: vec3u) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.w || y >= P.h) {
        return;
    }
    let i = y * P.w + x;
    let phi = P.phi;
    let s = field_scale();
    // Working-res coordinates of this frame pixel.
    let fx = f32(x) / s;
    let fy = f32(y) / s;

    // Flow in *frame* pixels: the field is measured at working res, so its
    // vectors scale with the image exactly as upsample_flow scales them.
    let f0 = sample_field(0u, fx, fy) * s;
    let f1 = sample_field(0u, fx - phi * f0.x / s, fy - phi * f0.y / s) * s;
    let b0 = sample_field(1u, fx, fy) * s;
    let b1 = sample_field(1u, fx - (1.0 - phi) * b0.x / s, fy - (1.0 - phi) * b0.y / s) * s;

    let sa = sample_frame(0u, f32(x) - phi * f1.x, f32(y) - phi * f1.y);
    // The backward field points B→A; the forward velocity seen from B's grid
    // is its negation, hence the minus sign here too.
    let sb = sample_frame(1u, f32(x) - (1.0 - phi) * b1.x, f32(y) - (1.0 - phi) * b1.y);

    let m = sample_masks(fx, fy);
    let oa = step(0.5, m.x);
    let ob = step(0.5, m.y);
    let la = unpack(select(frame_a[i], frame_a[i], true));
    let lb = unpack(frame_b[i]);
    let plain = la * (1.0 - phi) + lb * phi;

    var synth: vec4f;
    if (oa > 0.5 && ob > 0.5) {
        // Neither frame can explain this pixel.
        if (P.fallback == 1u) {
            synth = select(lb, la, phi < 0.5);
        } else {
            synth = plain;
        }
    } else {
        var ga = 1.0 - ob;
        var gb = 1.0 - oa;
        if (P.occ_mode == 1u) {
            ga = 1.0;
            gb = 1.0;
        }
        let wa = (1.0 - phi) * ga + SYNTH_EPS;
        let wb = phi * gb + SYNTH_EPS;
        synth = (wa * sa + wb * sb) / (wa + wb);
    }
    let guard = clamp(m.z, 0.0, 1.0);
    out_px[i] = pack(synth * (1.0 - guard) + plain * guard);
}

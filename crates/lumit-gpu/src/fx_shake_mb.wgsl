// Shake motion blur (docs/08-EFFECTS.md §3.4, T18/K-165): the shake's own
// motion blur. The wobble is a pure function of time, so it is sampled at up to
// SHAKE_MB_SAMPLES sub-frame placements across the shutter — each a full
// transform-domain inverse affine (host-computed: WGSL has no 64-bit integer
// for the splitmix64 noise lattice, docs/08 §3.12, so the host samples the
// noise and hands over ready affines) — and the premultiplied resamples are
// averaged. This is the premultiplied-linear mean the accumulation motion blur
// uses (docs/06 §4), applied to this one effect. Mirrors
// lumit_core::fx::cpu::transform_average op-for-op (§1.6: the CPU is the
// oracle): the taps are summed in order and divided by the count, one bilinear
// tap each, the revealed border handled by the shared `edge` policy (0
// Transparent / 1 Repeat / 2 Mirror == cpu::edge_index), premultiplied
// throughout. The host only dispatches this when motion blur is on, so the
// count is always ≥ 1; a single tap equal to the frame wobble reproduces the
// plain Shake.

// Must match lumit_core::fx::SHAKE_MB_SAMPLES (a compile-time assert in
// stylise.rs pins the two together).
const MAX_TAPS: u32 = 9u;

struct Tap {
    m: vec4<f32>,   // row-major inverse linear 2×2: (m00, m01, m10, m11)
    off: vec4<f32>, // .xy = inverse translation; .zw unused padding
};

struct Params {
    taps: array<Tap, 9>,
    count: u32,     // active taps, 1..=MAX_TAPS
    edge: u32,      // 0 transparent, 1 repeat, 2 mirror
    mix_amt: f32,   // 0..1, blended against the unprocessed input
    matte_on: f32,  // 1 = the matte scales every tap's displacement (K-427)
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// Resolve a tap index under the edge policy; -1 means transparent (no tap).
// == fx_transform.wgsl's edge_idx and cpu::edge_index.
fn edge_idx(i: i32, len: i32) -> i32 {
    if (i >= 0 && i < len) {
        return i;
    }
    if (p.edge == 1u) {
        return clamp(i, 0, len - 1);
    }
    if (p.edge == 2u) {
        var m = i;
        if (m < 0) {
            m = -m;
        } else {
            m = 2 * (len - 1) - m;
        }
        return clamp(m, 0, len - 1);
    }
    return -1;
}

// One integer tap under the edge policy; transparent taps contribute 0 while
// keeping their bilinear weight (== the cpu::bilinear_edge rule).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xi = edge_idx(x, size.x);
    let yi = edge_idx(y, size.y);
    if (xi < 0 || yi < 0) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, vec2<i32>(xi, yi), 0);
}

// Edge-policy bilinear sample at continuous pixel-centre coordinates
// (== cpu::bilinear_edge, same arithmetic order).
fn bilinear_edge(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn shake_mb(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let n = min(p.count, MAX_TAPS);
    // The matte scales every tap's displacement toward none, read at the
    // destination pixel (K-427, == cpu::transform_average_matted).
    var mk = 1.0;
    if (p.matte_on != 0.0) {
        mk = matte_k(xy);
    }
    var acc = vec4<f32>(0.0);
    for (var k: u32 = 0u; k < n; k = k + 1u) {
        let m = p.taps[k].m;
        let off = p.taps[k].off;
        // q = m·p + off, in the CPU reference's exact expression order.
        var qx = m.x * pos.x + m.y * pos.y + off.x;
        var qy = m.z * pos.x + m.w * pos.y + off.y;
        if (p.matte_on != 0.0) {
            qx = matte_toward(qx, pos.x, mk);
            qy = matte_toward(qy, pos.y, mk);
        }
        acc = acc + bilinear_edge(qx, qy, size);
    }
    let avg = acc / f32(n);
    let outv = o * (1.0 - p.mix_amt) + avg * p.mix_amt;
    textureStore(dst, xy, outv);
}

// Lens distort (docs/08-EFFECTS.md §3.42): barrel and pincushion by field of
// view. Mirrors lumit_core::fx::cpu::lens_distort op-for-op (§1.6: the CPU is
// the oracle).
//
// The one trig call that CAN be lifted out of the pixel loop is — tan(fov ÷ 2)
// arrives in the uniform. The two that cannot are here, and §3.42's fourth note
// records the divergence honestly: the ray angle is a function of the pixel, so
// both paths run their own platform's tan/atan on the same input and the oracle
// is judged on a smooth corpus where a sub-thousandth of a pixel of sampling
// error stays inside the fp16 tolerance.
//
// Field of view 0 and Mix 0 are both the bit-exact identity.

struct Params {
    centre: vec2<f32>,    // raster pixels
    tan_half_fov: f32,    // host-computed
    mix_amt: f32,         // 0..1, blended against the unprocessed input
    half_kind: u32,       // 0 width, 1 height, 2 diagonal
    edge: u32,            // 0 transparent, 1 repeat, 2 mirror
    enabled: u32,         // 0 = the exact identity
    reverse: u32,         // 1 = remove the fisheye rather than add it
    matte_on: f32,        // 1 = the matte scales the displacement (K-427)
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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

// == fx_transform.wgsl's edge_idx and cpu::edge_index. -1 means transparent.
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

fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xi = edge_idx(x, size.x);
    let yi = edge_idx(y, size.y);
    if (xi < 0 || yi < 0) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, vec2<i32>(xi, yi), 0);
}

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
fn lens_distort(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let fw = f32(size.x);
    let fh = f32(size.y);
    var half_extent = fw * 0.5;
    if (p.half_kind == 1u) {
        half_extent = fh * 0.5;
    } else if (p.half_kind == 2u) {
        half_extent = sqrt(fw * fw + fh * fh) * 0.5;
    }
    // Floored so an inactive effect cannot divide by zero on its way to the
    // short-circuit below.
    let f = half_extent / max(p.tan_half_fov, 1e-6);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let dx = px - p.centre.x;
    let dy = py - p.centre.y;
    let r = sqrt(dx * dx + dy * dy);
    var sx = px;
    var sy = py;
    if (p.enabled != 0u && r > 0.0) {
        let theta = r / f;
        var radius: f32;
        if (p.reverse != 0u) {
            radius = f * atan(theta);
        } else {
            radius = f * tan(min(theta, 1.553343));
        }
        let scale = radius / r;
        sx = p.centre.x + dx * scale;
        sy = p.centre.y + dy * scale;
        // The matte scales the displacement toward none, read at the
        // destination pixel (K-427, == cpu::lens_distort_matted).
        if (p.matte_on != 0.0) {
            let k = matte_k(xy);
            sx = matte_toward(sx, px, k);
            sy = matte_toward(sy, py, k);
        }
    }
    let v = bilinear_edge(sx, sy, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

// Transform (docs/08-EFFECTS.md §3.5, K-090): the layer transform group as
// a stack effect — its point is adjustment layers, where it transforms the
// composite of everything below (the montage punch-in/whip gesture). Shake
// (§3.4) dispatches through this same kernel. Mirrors
// lumit_core::fx::cpu::transform op-for-op (§1.6: the CPU is the oracle):
// each output pixel centre samples the input through the host-computed
// inverse affine (lumit_core::fx::transform_inverse — the kernel never runs
// its own trigonometry), one bilinear tap, the revealed border handled by
// the `edge` policy (0 Transparent / 1 Repeat / 2 Mirror — the shared blur
// convention, == cpu::edge_index), premultiplied throughout, opacity
// multiplied into all four channels. Identity parameters reproduce the input
// bit-exactly, and `edge = 0` is bit-identical to the old transparent-only
// kernel (the Transform effect passes 0).

struct Params {
    m: vec4<f32>,     // row-major inverse linear 2×2: (m00, m01, m10, m11)
    off: vec2<f32>,   // inverse translation
    opacity: f32,     // 0..1, multiplied into premultiplied RGBA
    mix_amt: f32,     // 0..1, blended against the unprocessed input
    edge: u32,        // 0 transparent, 1 repeat, 2 mirror
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Resolve a tap index under the edge policy; -1 means transparent (no tap).
// == fx_dirblur.wgsl's edge_idx and cpu::edge_index.
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
fn transform(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    // q = m·p + off, in the CPU reference's exact expression order.
    let qx = p.m.x * pos.x + p.m.y * pos.y + p.off.x;
    let qy = p.m.z * pos.x + p.m.w * pos.y + p.off.y;
    let v = bilinear_edge(qx, qy, size) * p.opacity;
    let outv = o * (1.0 - p.mix_amt) + v * p.mix_amt;
    textureStore(dst, xy, outv);
}

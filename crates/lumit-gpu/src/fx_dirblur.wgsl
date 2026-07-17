// Directional blur (docs/08-EFFECTS.md §3.8): a line integral along an
// angle. Mirrors lumit_core::fx::cpu::blur_directional op-for-op (§1.6: the
// CPU is the oracle): the same tap count (host-computed, dir_blur_taps),
// the same evenly spaced bilinear taps in the same order, box weighted and
// normalised over the full kernel whatever the edge policy. The unit
// direction arrives host-computed (WGSL cos/sin are not correctly rounded).

struct Params {
    dx: f32,        // unit streak direction (host-computed)
    dy: f32,
    length: f32,    // full streak length, raster px
    taps: i32,      // == cpu::dir_blur_taps(length)
    edge: u32,      // 0 transparent, 1 repeat, 2 mirror
    mix_amt: f32,   // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Resolve a tap index under the edge policy; -1 means transparent (no tap).
// == fx_blur.wgsl's edge_idx and cpu::edge_index.
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

// One integer tap under the edge policy; transparent corners contribute 0
// while keeping their bilinear weight (== the cpu::bilinear_edge rule).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xi = edge_idx(x, size.x);
    let yi = edge_idx(y, size.y);
    if (xi < 0 || yi < 0) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, vec2<i32>(xi, yi), 0);
}

// Edge-policy bilinear at continuous pixel-centre coordinates
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
fn dir_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let nf = f32(p.taps);
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < p.taps; k++) {
        let t = ((f32(k) + 0.5) / nf - 0.5) * p.length;
        acc += bilinear_edge(pos.x + t * p.dx, pos.y + t * p.dy, size);
    }
    let o = textureLoad(src, xy, 0);
    let v = acc / nf;
    textureStore(dst, xy, mix(o, v, p.mix_amt));
}

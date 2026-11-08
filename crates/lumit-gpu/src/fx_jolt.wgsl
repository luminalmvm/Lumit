// Jolt: each pixel read through a scale and a slide, red and blue parted
// along it, a colour mixed in and the light multiplied. The blur, when there
// is one, has already run through fx_blur.wgsl. Mirrors
// lumit_core::fx::effects::jolt::jolt op-for-op (the CPU is the oracle).

struct Params {
    offset: vec2<f32>,  // the sample point is pixel * zoom + offset
    split: vec2<f32>,   // red read this far back, blue this far on
    tint: vec4<f32>,    // the colour mixed in, and how far in .w
    zoom: f32,
    gain: f32,          // the light, multiplied in
    opacity: f32,       // the whole pixel, multiplied in
    mix_amt: f32,       // 0..1, blended against `orig`
    edge: u32,          // 0 transparent, 1 repeat, 2 mirror
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == cpu::edge_index; -1 is transparent.
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
    let c = clamp(vec2<i32>(xi, yi), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(textureLoad(src, c, 0), vec4<f32>(0.0), xi < 0 || yi < 0);
}

// == cpu::bilinear_edge, same arithmetic order.
fn bilinear_edge(q: vec2<f32>, size: vec2<i32>) -> vec4<f32> {
    let f = q - vec2<f32>(0.5);
    let f0 = floor(f);
    let t = f - f0;
    let i = vec2<i32>(f0);
    let c00 = tap(i.x, i.y, size);
    let c10 = tap(i.x + 1, i.y, size);
    let c01 = tap(i.x, i.y + 1, size);
    let c11 = tap(i.x + 1, i.y + 1, size);
    let top = c00 * (1.0 - t.x) + c10 * t.x;
    let bottom = c01 * (1.0 - t.x) + c11 * t.x;
    return top * (1.0 - t.y) + bottom * t.y;
}

@compute @workgroup_size(8, 8)
fn jolt(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let q = pos * p.zoom + p.offset;
    var v = bilinear_edge(q, size);
    if (any(p.split != vec2<f32>(0.0))) {
        let r = bilinear_edge(q - p.split, size);
        let b = bilinear_edge(q + p.split, size);
        v = vec4<f32>(r.r, v.g, b.b, max(max(r.a, v.a), b.a));
    }
    // The colour is laid over the picture's own light, so its detail stays.
    let k = p.tint.w;
    let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
    let lit = p.tint.rgb * (dot(v.rgb, luma) / max(dot(p.tint.rgb, luma), 1e-3));
    let rgb = (v.rgb * (1.0 - k) + lit * k) * p.gain;
    let o = textureLoad(orig, xy, 0);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + vec4<f32>(rgb, v.a) * p.opacity * p.mix_amt);
}

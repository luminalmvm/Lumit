// Gradient (docs/08-EFFECTS.md §3.35): a linear or radial two-colour ramp with
// scatter. Mirrors lumit_core::fx::cpu::gradient op-for-op (§1.6: the CPU is
// the oracle).
//
// A generator: it replaces the frame edge to edge and writes opaque alpha, so
// nothing of the input's colour is read and there is nothing to unpremultiply.
// Both reciprocals arrive precomputed and floored from `Gradient::packed`, so
// nothing divides per pixel and a zero-length axis collapses the ramp to one
// flat colour rather than faulting. Interpolation is in the working space (scene-linear,
// §2.1). Mix 0 is the bit-exact identity.

struct Params {
    start_axis: vec4<f32>,  // xy = start (raster px), zw = end − start
    c0: vec4<f32>,          // start colour; alpha lane ignored (the ramp is opaque)
    c1: vec4<f32>,          // end colour
    inv_len2: f32,          // 1 ÷ |axis|², floored
    inv_len: f32,           // 1 ÷ |axis|, floored
    scatter: f32,           // 0..1 dither of t
    mix_amt: f32,           // 0..1, blended against the unprocessed input
    seed: u32,
    radial: u32,            // 0 linear, 1 radial
    clip_to_alpha: u32,     // 1 = clip to the layer's coverage, keep its alpha (K-706)
    _pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// A 32-bit avalanche mixer (== lumit_core::fx::splitmix32, identical wrapping
// u32 ops in the same order — exact on every GPU, so CPU and GPU agree on the
// integer hash bit-for-bit).
fn splitmix32(xin: u32) -> u32 {
    var x = xin;
    x = x + 0x9e3779b9u;
    x = x ^ (x >> 16u);
    x = x * 0x21f0aaadu;
    x = x ^ (x >> 15u);
    x = x * 0x735a2d97u;
    x = x ^ (x >> 15u);
    return x;
}

// == lumit_core::fx::noise::hash01, same fold order; bitcast (not a value
// conversion) matches Rust's same-width `as u32` reinterpretation exactly.
fn hash01(channel: u32, x: i32, y: i32, z: i32) -> f32 {
    var h = p.seed;
    h = splitmix32(h ^ channel);
    h = splitmix32(h ^ bitcast<u32>(x));
    h = splitmix32(h ^ bitcast<u32>(y));
    h = splitmix32(h ^ bitcast<u32>(z));
    return f32(h >> 8u) / 16777216.0;
}

@compute @workgroup_size(8, 8)
fn gradient(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let dx = f32(xy.x) + 0.5 - p.start_axis.x;
    let dy = f32(xy.y) + 0.5 - p.start_axis.y;
    var t: f32;
    if (p.radial != 0u) {
        t = sqrt(dx * dx + dy * dy) * p.inv_len;
    } else {
        t = (dx * p.start_axis.z + dy * p.start_axis.w) * p.inv_len2;
    }
    if (p.scatter > 0.0) {
        t = t + (hash01(0u, xy.x, xy.y, 0) - 0.5) * p.scatter;
    }
    let tc = clamp(t, 0.0, 1.0);
    // Clipped to alpha (K-706, == cpu::gradient): `ramp · a` is the
    // premultiplied form of "this ramp at this coverage", and the layer's own
    // alpha is then left exactly as it was — the whole difference between the
    // Gradient overlay style and the generator this kernel also serves.
    var cover = 1.0;
    var outa = o.a * (1.0 - p.mix_amt) + p.mix_amt;
    if (p.clip_to_alpha != 0u) {
        cover = o.a;
        outa = o.a;
    }
    let g = (p.c0.rgb + (p.c1.rgb - p.c0.rgb) * tc) * cover;
    let outv = o.rgb * (1.0 - p.mix_amt) + g * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}

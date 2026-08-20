// Posterize (docs/08-EFFECTS.md §3.58): the tone ladder cut into steps. Mirrors
// lumit_core::fx::cpu::posterize op-for-op (§1.6: the CPU is the oracle).
//
// The rungs are spaced evenly in a square root of the light rather than in the
// light itself, so they land where a person sees them — and `sqrt` is the curve
// because it is one correctly-rounded instruction on both paths, which is what
// keeps the two from disagreeing about which side of a rung a value falls on
// (§3.58 decision 2). `floor(x + 0.5)` rather than `round`, for the same
// reason: WGSL breaks a tie to even and Rust breaks it away from zero.

struct Params {
    n: f32,        // Levels - 1, computed host-side
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::perceptual.
fn perceptual(v: vec3<f32>) -> vec3<f32> {
    return sqrt(max(v, vec3<f32>(0.0)));
}

@compute @workgroup_size(8, 8)
fn posterize(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    if (p.n <= 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let t = perceptual(u) * p.n;
    let step_v = floor(t + vec3<f32>(0.5)) / p.n;
    let v = step_v * step_v;
    let outv = o.rgb * (1.0 - p.mix_amt) + v * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

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
    matte_on: f32,     // 1 = the matte drives the control below (K-395)
    _pad1: f32,
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
    // The matte pulls the step count toward 255 (256 levels, the 8-bit
    // ladder nobody can see) per pixel (K-395, == cpu::posterize_matted).
    var n = p.n;
    if (p.matte_on != 0.0) {
        n = matte_toward(n, 255.0, matte_k(xy));
    }
    let t = perceptual(u) * n;
    let step_v = floor(t + vec3<f32>(0.5)) / n;
    let v = step_v * step_v;
    let outv = o.rgb * (1.0 - p.mix_amt) + v * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

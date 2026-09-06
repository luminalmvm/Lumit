// Saturation (docs/08-EFFECTS.md §3.10, amended when the v1 Grade split
// into single-purpose colour effects): scale colourfulness about
// Rec. 709 luma, in linear light on unpremultiplied colour (§2.2, the wrap
// fused into the kernel). Mirrors lumit_core::fx::cpu::saturate op-for-op
// (§1.6: the CPU is the oracle); saturation 1 short-circuits the whole
// effect, so a neutral Saturation is the bit-exact identity.

struct Params {
    saturation: f32,   // 0 = greyscale, 1 = neutral, 2 = doubled, open above
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    matte_on: f32,     // 1 = the matte drives the control below
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// Rec. 709 luma weights, in linear light (== cpu::LUMA).
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn saturate_fx(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.saturation == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let luma = u.r * LUMA.r + u.g * LUMA.g + u.b * LUMA.b;
    // The matte pulls Saturation toward 1 per pixel.
    var sat = p.saturation;
    if (p.matte_on != 0.0) {
        sat = matte_toward(sat, 1.0, matte_k(xy));
    }
    let v = max(vec3<f32>(luma) + (u - vec3<f32>(luma)) * sat, vec3<f32>(0.0));
    let s = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + s * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

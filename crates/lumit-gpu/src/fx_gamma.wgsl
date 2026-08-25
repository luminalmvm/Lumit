// Gamma (docs/08-EFFECTS.md §3.19): a per-channel power curve in the
// compositor's scene-linear working space — out = pow(max(u, 0), 1/gamma) per
// RGB channel, on unpremultiplied colour (§2.2, the wrap fused into the
// kernel). Mirrors lumit_core::fx::cpu::gamma op-for-op (§1.6: the CPU is the
// oracle). pow is non-linear, so — like Contrast and Saturation — it cannot run
// through premultiplied alpha: the pixel is unpremultiplied, curved, then
// re-premultiplied. The input is clamped to >= 0 before the pow (scene-linear
// colour can dip slightly negative, and pow of a negative base is undefined);
// the clamp is byte-identical to the CPU reference so the oracle holds.
// gamma == 1.0 short-circuits the whole effect, so a neutral Gamma is the
// bit-exact identity (a short-circuit, not a reliance on pow(x, 1) == x).
// Continuous for input >= 0 (no round/clamp/quantize on the output).

struct Params {
    gamma: f32,    // curve raises to 1/gamma; 1.0 = neutral
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

@compute @workgroup_size(8, 8)
fn gamma(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.gamma == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    // The matte pulls Gamma toward 1 per pixel, before the curve (K-395):
    // a half-grey matte on Gamma 2 is pow(x, 1/1.5), not a fade.
    var gamma = p.gamma;
    if (p.matte_on != 0.0) {
        gamma = matte_toward(gamma, 1.0, matte_k(xy));
    }
    let inv = 1.0 / gamma;
    let u = unpremult(o);
    // Clamp to >= 0 before the pow, byte-identical to the CPU reference.
    let curved = pow(max(u, vec3<f32>(0.0)), vec3<f32>(inv));
    let graded = curved * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

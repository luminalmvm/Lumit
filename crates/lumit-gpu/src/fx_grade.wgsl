// Grade (docs/08-EFFECTS.md §3.10, minimal v1): per-channel gain → lift →
// gamma, then saturation about Rec. 709 luma, in linear light on
// unpremultiplied colour (§2.2, the wrap fused into the kernel). Mirrors
// lumit_core::fx::cpu::grade op-for-op (§1.6: the CPU is the oracle);
// neutral gamma and saturation short-circuit on both sides so a neutral
// grade never rounds through `pow`.

struct Params {
    lift: vec4<f32>,   // rgb used
    gamma: vec4<f32>,  // rgb used, > 0
    gain: vec4<f32>,   // rgb used
    saturation: f32,   // 0 = greyscale, 1 = neutral, 2 = doubled
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Rec. 709 luma weights, in linear light (== cpu::LUMA).
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// One channel through gain → lift → gamma (== the cpu::grade channel loop).
fn channel(x0: f32, gain: f32, lift: f32, gamma: f32) -> f32 {
    var x = max(x0 * gain + lift, 0.0);
    if (gamma != 1.0) {
        x = pow(x, 1.0 / gamma);
    }
    return x;
}

@compute @workgroup_size(8, 8)
fn grade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    var v = vec3<f32>(
        channel(u.r, p.gain.r, p.lift.r, p.gamma.r),
        channel(u.g, p.gain.g, p.lift.g, p.gamma.g),
        channel(u.b, p.gain.b, p.lift.b, p.gamma.b),
    );
    if (p.saturation != 1.0) {
        let luma = v.r * LUMA.r + v.g * LUMA.g + v.b * LUMA.b;
        v = max(vec3<f32>(luma) + (v - vec3<f32>(luma)) * p.saturation, vec3<f32>(0.0));
    }
    let graded = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

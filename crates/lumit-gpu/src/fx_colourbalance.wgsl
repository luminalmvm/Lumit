// Colour balance (docs/08-EFFECTS.md §3.10, after the v1 Grade split into
// single-purpose colour effects): per-channel gain → lift → gamma, in
// linear light on unpremultiplied colour (§2.2, the wrap fused into the
// kernel). Mirrors lumit_core::fx::cpu::colour_balance op-for-op
// (§1.6: the CPU is the oracle); fully neutral parameters short-circuit
// the whole effect, so a balance at defaults is the bit-exact identity —
// never a round trip through `pow` and the unpremultiply divide.

struct Params {
    lift: vec4<f32>,   // rgb used
    gamma: vec4<f32>,  // rgb used, > 0
    gain: vec4<f32>,   // rgb used
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    matte_on: f32,     // 1 = the matte drives the control below
    _pad1: f32,
    _pad2: f32,
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

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// One channel through gain → lift → gamma (== the cpu channel loop).
fn channel(x0: f32, gain: f32, lift: f32, gamma: f32) -> f32 {
    var x = max(x0 * gain + lift, 0.0);
    if (gamma != 1.0) {
        x = pow(x, 1.0 / gamma);
    }
    return x;
}

@compute @workgroup_size(8, 8)
fn colour_balance(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (all(p.lift.rgb == vec3<f32>(0.0))
        && all(p.gamma.rgb == vec3<f32>(1.0))
        && all(p.gain.rgb == vec3<f32>(1.0))) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    // The matte pulls Lift toward 0 and Gamma and Gain toward 1 per pixel,
    // before the grade (== cpu::colour_balance_matted).
    var lift = p.lift.rgb;
    var gamma = p.gamma.rgb;
    var gain = p.gain.rgb;
    if (p.matte_on != 0.0) {
        let k = matte_k(xy);
        lift = vec3<f32>(matte_toward(lift.r, 0.0, k), matte_toward(lift.g, 0.0, k), matte_toward(lift.b, 0.0, k));
        gamma = vec3<f32>(matte_toward(gamma.r, 1.0, k), matte_toward(gamma.g, 1.0, k), matte_toward(gamma.b, 1.0, k));
        gain = vec3<f32>(matte_toward(gain.r, 1.0, k), matte_toward(gain.g, 1.0, k), matte_toward(gain.b, 1.0, k));
    }
    let v = vec3<f32>(
        channel(u.r, gain.r, lift.r, gamma.r),
        channel(u.g, gain.g, lift.g, gamma.g),
        channel(u.b, gain.b, lift.b, gamma.b),
    );
    let graded = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

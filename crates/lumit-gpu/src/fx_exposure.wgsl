// Exposure (docs/08-EFFECTS.md §3.16). Mirrors lumit_core::fx::cpu::exposure
// op-for-op (§1.6: the CPU is the oracle): a single scene-linear gain on the
// RGB channels, alpha untouched. `factor` is 2^stops, computed host-side so
// the CPU and this kernel multiply by the identical number. factor == 1.0
// (0 stops) short-circuits to the input, the bit-exact neutral point.

struct Params {
    factor: f32,   // 2^stops linear gain
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    stops: f32,    // the Stops behind `factor`, read only under a matte
    matte_on: f32,     // 1 = the matte drives the control below (K-395)
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

@compute @workgroup_size(8, 8)
fn exposure(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.factor == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    // The matte scales Stops toward 0 per pixel: the gain there is
    // exp2(stops * k) (K-395, == cpu::exposure_matted). Unmatted, the host's
    // factor is used as it always was.
    var factor = p.factor;
    if (p.matte_on != 0.0) {
        factor = exp2(p.stops * matte_k(xy));
    }
    let scaled = o.rgb * factor;
    let outv = o.rgb * (1.0 - p.mix_amt) + scaled * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

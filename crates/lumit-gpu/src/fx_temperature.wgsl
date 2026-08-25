// Temperature (docs/08-EFFECTS.md §3.20). Mirrors lumit_core::fx::cpu::temperature
// op-for-op (§1.6: the CPU is the oracle): a warm/cool white-balance shift as a
// per-channel gain in scene-linear light — red × gain_r, blue × gain_b, green
// and alpha untouched. The two gains are computed host-side (in the resolve
// step) so the CPU and this kernel multiply by the identical numbers.
// Premultiplied throughout, exactly like Exposure: a per-channel scalar scales
// premultiplied colour consistently, so there is no unpremultiply round trip
// (unlike the affine Contrast/Saturation grades). gain_r == 1.0 && gain_b == 1.0
// (Temperature 0) short-circuits to the input, the bit-exact neutral point.

struct Params {
    gain_r: f32,   // scene-linear red gain, max(0, 1 + 0.75·k)
    gain_b: f32,   // scene-linear blue gain, max(0, 1 − 0.75·k)
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    t: f32,        // Temperature / 100 clamped, read only under a matte
    matte_on: f32,     // 1 = the matte drives the control below (K-395)
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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
fn temperature(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.gain_r == 1.0 && p.gain_b == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    // The matte scales Temperature toward 0 per pixel, and the gains are
    // rebuilt from t * k (K-395, == cpu::temperature_gains) — the blue gain
    // floors at 0, so a lerp of the gains would not be a smaller Temperature.
    var gain_r = p.gain_r;
    var gain_b = p.gain_b;
    if (p.matte_on != 0.0) {
        let tk = p.t * matte_k(xy);
        gain_r = max(1.0 + 0.75 * tk, 0.0);
        gain_b = max(1.0 - 0.75 * tk, 0.0);
    }
    let sr = o.r * gain_r;
    let sb = o.b * gain_b;
    let out_r = o.r * (1.0 - p.mix_amt) + sr * p.mix_amt;
    let out_b = o.b * (1.0 - p.mix_amt) + sb * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(out_r, o.g, out_b, o.a));
}

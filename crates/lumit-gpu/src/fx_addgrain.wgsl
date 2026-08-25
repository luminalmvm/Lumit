// Add grain (docs/08-EFFECTS.md §3.77): film grain laid on by tone. Mirrors
// lumit_core::fx::cpu::add_grain and ::grain_at op-for-op (§1.6: the CPU is the
// oracle).
//
// fx_noise_core.wgsl is prepended to this file at pipeline build, exactly as it
// is to fx_fractal_noise.wgsl — the hash and the interpolated lattice must agree
// to the bit with Rust, and two copies of them is the arrangement that module
// exists to avoid.
//
// SOFTNESS IS A CROSSFADE BETWEEN THE SAME FIELD READ TWO WAYS, not a blur: the
// hard reading takes one value per cell (a flat square, which is what a grain
// particle is), the soft reading interpolates the same lattice. One extra hash,
// and both ends of the control are correct.
//
// The grain is added on the PERCEPTUAL value (§3.58's curve, K-404) and squared
// back, which is what makes Intensity mean one thing across the frame.
// Unpremultiplied (§2.2), for §3.36's reason.
//
// Mix 0 and Intensity 0 are both the bit-exact identity — the second by
// short-circuit, because sqrt(v)² is not v in the last bit.

struct Params {
    amplitude: vec4<f32>,  // per channel; the w lane is unused
    tonal: vec4<f32>,      // shadows, midtones, highlights, each ÷ 100
    inv_size: f32,         // 1 ÷ Size, raster px
    softness: f32,         // 0..1
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    _pad0: f32,
    seed: u32,
    tick: i32,             // the frame's draw; 0 when Animate is off
    monochrome: u32,       // 1 reads one lane for all three channels
    matte_on: f32,         // 1 = the matte scales Intensity per pixel (K-428)
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

// == lumit_core::fx::cpu::grain_at.
fn grain_at(qx: f32, qy: f32, lane: u32) -> f32 {
    let hard = nc_hash01(p.seed, lane, i32(floor(qx)), i32(floor(qy)), p.tick) * 2.0 - 1.0;
    let soft = nc_value3(p.seed, lane, qx, qy, f32(p.tick), 0);
    return hard + (soft - hard) * p.softness;
}

@compute @workgroup_size(8, 8)
fn add_grain(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    if (p.amplitude.x == 0.0 && p.amplitude.y == 0.0 && p.amplitude.z == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    var u = vec3<f32>(0.0);
    if (o.a > 0.0) {
        u = o.rgb / o.a;
    }
    let qx = (f32(xy.x) + 0.5) * p.inv_size;
    let qy = (f32(xy.y) + 0.5) * p.inv_size;
    var outv = o.rgb;
    for (var c = 0u; c < 3u; c = c + 1u) {
        let v = sqrt(max(u[c], 0.0));
        // Three hats summing to one, so 100/100/100 is provably neutral.
        let h0 = clamp(1.0 - 2.0 * v, 0.0, 1.0);
        let h2 = clamp(2.0 * v - 1.0, 0.0, 1.0);
        let weight = p.tonal.x * h0 + p.tonal.y * (1.0 - h0 - h2) + p.tonal.z * h2;
        var lane = c;
        if (p.monochrome != 0u) {
            lane = 0u;
        }
        let g = grain_at(qx, qy, lane);
        // The matte pulls Intensity toward 0 per pixel, before the grain is
        // added (K-428): half the Intensity is a finer grain, not a half-fade.
        var amp = p.amplitude[c];
        if (p.matte_on != 0.0) {
            amp = matte_toward(amp, 0.0, matte_k(xy));
        }
        let lit = max(v + g * amp * weight, 0.0);
        outv[c] = o.rgb[c] * (1.0 - p.mix_amt) + lit * lit * o.a * p.mix_amt;
    }
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

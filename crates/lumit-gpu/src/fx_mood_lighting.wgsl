// Mood lighting (docs/08-EFFECTS.md §3.98): pools of coloured light laid over
// the picture, and a firmer contrast under them. Mirrors
// lumit_core::fx::cpu::mood_lighting op-for-op (§1.6: the CPU is the oracle);
// the field itself comes from fx_noise_core.wgsl, which is prepended to this
// file at compile time and is the one WGSL twin of lumit_core::fx::noise —
// shared with fx_fractal_noise.wgsl and fx_turbdisplace.wgsl, so the three
// cannot drift apart.
//
// The light multiplies. A mid-grey tint is a factor of one and changes nothing,
// the two colours pull either side of that, and no highlight is clipped on the
// way — which a screen or a soft-light blend could not promise in a
// scene-linear working space (§2.1).
//
// The pool size arrives as a reciprocal and Contrast as the factor it
// multiplies by, so this kernel divides by nothing. Alpha is untouched. Mix 0
// is the bit-exact identity.

struct Params {
    light: vec4<f32>,           // scene-linear rgb in the pools; w unused
    shade: vec4<f32>,           // scene-linear rgb between them; w unused
    inv_scale_z_int_con: vec4<f32>, // x = 1 ÷ pool size, y = depth, z = Intensity ÷ 100, w = Contrast's factor
    mix_amt: f32,               // 0..1, blended against the unprocessed input
    matte_on: f32,              // 1 = the matte scales Intensity and Contrast per pixel
    gain: f32,                  // each octave's amplitude, as a share of the last
    lacunarity: f32,            // each octave's frequency, as a multiple of the last
    seed: u32,
    octaves: u32,
    cycle: i32,                 // depth loop length in cells; 0 = no loop
    flags: u32,                 // bit 0 Perlin, bit 1 Turbulent
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

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn mood_lighting(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // The matte turns the light down and the grade back toward neutral, before
    // either runs: a dimmer lamp and a gentler contrast, not a fade between two
    // pictures (§2.6's rule for mattes).
    var intensity = p.inv_scale_z_int_con.z;
    var contrast = p.inv_scale_z_int_con.w;
    if (p.matte_on != 0.0) {
        let k = matte_k(xy);
        intensity = matte_toward(intensity, 0.0, k);
        contrast = matte_toward(contrast, 1.0, k);
    }
    let field = FractalField(p.seed, p.octaves, p.gain, p.lacunarity, p.flags, p.cycle);
    let n = nc_fractal(field,
                       (f32(xy.x) + 0.5) * p.inv_scale_z_int_con.x,
                       (f32(xy.y) + 0.5) * p.inv_scale_z_int_con.x,
                       p.inv_scale_z_int_con.y);
    // The field, spread to fill 0..1: a Perlin sum rarely reaches its own ends,
    // and a light that never arrives is not a light.
    let f = clamp(n + 0.5, 0.0, 1.0);
    let u = unpremult(o);
    let tint = p.shade.rgb + (p.light.rgb - p.shade.rgb) * f;
    let lit = u * (vec3<f32>(1.0) + (tint - vec3<f32>(0.5)) * 2.0 * intensity);
    let v = (lit - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
    let outv = o.rgb * (1.0 - p.mix_amt) + v * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

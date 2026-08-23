// Radio waves (docs/08-EFFECTS.md §3.75): shapes emitted from a point and
// expanding. Mirrors lumit_core::fx::cpu::radio_waves and ::radio_waves_sample
// op-for-op (§1.6: the CPU is the oracle).
//
// Every wave is the SAME shape at a different size, so §3.71's sector solve was
// done host-side for a unit shape and this kernel only multiplies it by each
// wave's radius. Sides, Star and Star depth therefore cost nothing per wave.
//
// `newest` — floor(Time × Frequency) — is taken host-side, because it decides
// WHICH rings exist and one bit of disagreement about it is a whole ring
// (K-399).
//
// The sector fold is floor(x + 0.5) and NOT round(): Rust rounds halves away
// from zero and WGSL rounds them to even.
//
// Mix 0 and Time 0 are both the bit-exact identity.

const PI: f32 = 3.14159265358979323846;

struct Params {
    centre_vertex: vec4<f32>,   // producer.xy raster px, unit vertex.xy
    normal_period_rot: vec4<f32>, // unit edge normal.xy, one sector (rad), rotation (rad)
    spin_time_step_exp: vec4<f32>, // spin (rad/s), Time (s), 1 ÷ Frequency (s), expansion (px/s)
    life_half_fades: vec4<f32>,  // lifespan (s), stroke half-width (px), fade in, fade out
    colour: vec4<f32>,           // scene-linear; the alpha lane is ignored
    opacity: f32,                // 0..1
    mix_amt: f32,                // 0..1, blended against the unprocessed input
    newest: i32,                 // the newest wave's index
    count: i32,                  // how many waves to walk back from it
    composite: u32,              // 1 keeps the layer under the waves
    matte_on: f32,               // 1 = the matte scales Opacity per pixel (K-428)
    _pad1: u32,
    _pad2: u32,
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

@compute @workgroup_size(8, 8)
fn radio_waves(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let rel = vec2<f32>(f32(xy.x) + 0.5, f32(xy.y) + 0.5) - p.centre_vertex.xy;
    let r = sqrt(dot(rel, rel));
    let phi = atan2(rel.y, rel.x) + PI * 0.5;
    let period = p.normal_period_rot.z;
    let rotation = p.normal_period_rot.w;
    let spin = p.spin_time_step_exp.x;
    let time = p.spin_time_step_exp.y;
    let step_s = p.spin_time_step_exp.z;
    let expansion = p.spin_time_step_exp.w;
    let lifespan = p.life_half_fades.x;
    let half_w = p.life_half_fades.y;
    let fade_in = p.life_half_fades.z;
    let fade_out = p.life_half_fades.w;
    var acc = 0.0;
    for (var j = 0; j < p.count; j = j + 1) {
        let k = p.newest - j;
        if (k < 0) {
            continue;
        }
        let age = time - f32(k) * step_s;
        if (age < 0.0 || age > lifespan) {
            continue;
        }
        let rad = age * expansion;
        let turned = phi - rotation - spin * age;
        let a = abs(turned - period * floor(turned / period + 0.5));
        let d = vec2<f32>(r * cos(a), r * sin(a)) - rad * p.centre_vertex.zw;
        let dist = abs(dot(d, p.normal_period_rot.xy));
        let cov = clamp((half_w + 0.5 - dist) / max(half_w, 0.5), 0.0, 1.0);
        let u = age / lifespan;
        let fade = min(clamp(u / fade_in, 0.0, 1.0), clamp((1.0 - u) / fade_out, 0.0, 1.0));
        acc = max(acc, cov * fade);
    }
    // The matte pulls Opacity toward 0 per pixel, before the composite (K-428).
    var opacity = p.opacity;
    if (p.matte_on != 0.0) {
        opacity = matte_toward(opacity, 0.0, matte_k(xy));
    }
    let cov = acc * opacity;
    let keep = (1.0 - cov) * f32(p.composite);
    let lit = vec4<f32>(o.rgb * keep + p.colour.rgb * cov, o.a * keep + cov);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + lit * p.mix_amt);
}

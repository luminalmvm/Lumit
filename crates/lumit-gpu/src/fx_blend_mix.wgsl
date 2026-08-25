// The effect Blend and Mix, once at the seam (K-425, docs/08 §1.5).
//
// Every effect's Mix row carries a Blend choice — the layer blend modes,
// verbatim — saying how the effect's result combines with its input. The
// kernel has already run into `processed` AT MIX 100 (the seam forces it, so
// the Mix is not applied twice); this pass blends that result onto `input`
// by the chosen mode and then applies the effect's own Mix, once:
//
//     out = input · (1 − mix) + blend(input, processed) · mix
//
// The op-for-op twin of `lumit_core::fx::cpu::blend_mix` / `blend_pixel`:
// the same mode table (BlendMode::ALL order), the same domains as the
// compositor's layer modes (docs/06 §blend domains — Add, Multiply, Lighten,
// Darken and Subtract per channel in linear; the rest encoded to sRGB, the
// W3C formula, decoded), alpha the effect's own. Never dispatched for Normal:
// an unset Blend row runs no pass and renders byte for byte what it did
// (K-258). Shares the adjustment blend's bind-group layout; the third sampled
// slot is bound but unread.

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var processed: texture_2d<f32>;
@group(0) @binding(2) var unused_c: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;

struct Params {
    // BlendMode::ALL index (never 0 here).
    mode: u32,
    // The effect's own Mix, 0..1.
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}
@group(0) @binding(4) var<uniform> params: Params;

// == cpu::blend_encode / blend_decode: the compositor's sRGB curve, one
// clamped channel at a time.
fn blend_encode(v: f32) -> f32 {
    if (v <= 0.0031308) {
        return v * 12.92;
    }
    return 1.055 * pow(max(v, 0.0), 1.0 / 2.4) - 0.055;
}

fn blend_decode(v: f32) -> f32 {
    if (v <= 0.04045) {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

fn colour_burn(s: f32, d: f32) -> f32 {
    if (d >= 1.0) {
        return 1.0;
    } else if (s <= 0.0) {
        return 0.0;
    }
    return 1.0 - min((1.0 - d) / s, 1.0);
}

fn colour_dodge(s: f32, d: f32) -> f32 {
    if (d <= 0.0) {
        return 0.0;
    } else if (s >= 1.0) {
        return 1.0;
    }
    return min(d / (1.0 - s), 1.0);
}

fn hard_light(s: f32, d: f32) -> f32 {
    if (s <= 0.5) {
        return 2.0 * s * d;
    }
    return 1.0 - 2.0 * (1.0 - s) * (1.0 - d);
}

fn vivid(s: f32, d: f32) -> f32 {
    if (s <= 0.5) {
        return colour_burn(2.0 * s, d);
    }
    return colour_dodge(2.0 * s - 1.0, d);
}

// == cpu::blend_separable: one channel of the W3C separable blends, `s` the
// effect's output and `d` its input, both encoded.
fn blend_separable(mode: u32, s: f32, d: f32) -> f32 {
    switch mode {
        case 3u: { return colour_burn(s, d); }
        case 4u: { return clamp(s + d - 1.0, 0.0, 1.0); }
        case 8u: { return s + d - s * d; }
        case 9u: { return colour_dodge(s, d); }
        case 11u: { return hard_light(d, s); }
        case 12u: {
            var dd: f32;
            if (d <= 0.25) {
                dd = ((16.0 * d - 12.0) * d + 4.0) * d;
            } else {
                dd = sqrt(d);
            }
            if (s <= 0.5) {
                return d - (1.0 - 2.0 * s) * d * (1.0 - d);
            }
            return d + (2.0 * s - 1.0) * (dd - d);
        }
        case 13u: { return hard_light(s, d); }
        case 14u: { return clamp(d + 2.0 * s - 1.0, 0.0, 1.0); }
        case 15u: { return vivid(s, d); }
        case 16u: {
            if (s <= 0.5) {
                return min(d, 2.0 * s);
            }
            return max(d, 2.0 * s - 1.0);
        }
        case 17u: {
            if (vivid(s, d) >= 0.5) {
                return 1.0;
            }
            return 0.0;
        }
        case 18u: { return abs(s - d); }
        case 19u: { return s + d - 2.0 * s * d; }
        case 21u: { return clamp(d / max(s, 1e-6), 0.0, 1.0); }
        default: { return s; }
    }
}

// == cpu::blend_lum / blend_clip / blend_set_lum / blend_sat / blend_set_sat.
fn blend_lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn blend_clip(c: vec3<f32>) -> vec3<f32> {
    let l = blend_lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    var r = c;
    if (n < 0.0) {
        r = l + (r - l) * (l / max(l - n, 1e-6));
    }
    if (x > 1.0) {
        r = l + (r - l) * ((1.0 - l) / max(x - l, 1e-6));
    }
    return r;
}

fn blend_set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let d = l - blend_lum(c);
    return blend_clip(c + d);
}

fn blend_sat(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn blend_set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let mn = min(c.r, min(c.g, c.b));
    let mx = max(c.r, max(c.g, c.b));
    if (mx > mn) {
        let k = s / max(mx - mn, 1e-6);
        return (c - mn) * k;
    }
    return vec3<f32>(0.0);
}

// == cpu::blend_pixel: the effect's output `s` combined with its input `d`.
fn blend_pixel(mode: u32, d: vec4<f32>, s: vec4<f32>) -> vec4<f32> {
    var o = vec4<f32>(0.0, 0.0, 0.0, s.a);
    switch mode {
        case 0u: { return s; }
        case 6u: { o = vec4<f32>(d.rgb + s.rgb, s.a); }
        case 2u: { o = vec4<f32>(d.rgb * s.rgb, s.a); }
        case 7u: { o = vec4<f32>(max(d.rgb, s.rgb), s.a); }
        case 1u: { o = vec4<f32>(min(d.rgb, s.rgb), s.a); }
        case 20u: { o = vec4<f32>(max(d.rgb - s.rgb, vec3<f32>(0.0)), s.a); }
        default: {
            let se = vec3<f32>(
                blend_encode(clamp(s.r, 0.0, 1.0)),
                blend_encode(clamp(s.g, 0.0, 1.0)),
                blend_encode(clamp(s.b, 0.0, 1.0)),
            );
            let de = vec3<f32>(
                blend_encode(clamp(d.r, 0.0, 1.0)),
                blend_encode(clamp(d.g, 0.0, 1.0)),
                blend_encode(clamp(d.b, 0.0, 1.0)),
            );
            var b: vec3<f32>;
            switch mode {
                case 5u: {
                    if (blend_lum(se) < blend_lum(de)) { b = se; } else { b = de; }
                }
                case 10u: {
                    if (blend_lum(se) > blend_lum(de)) { b = se; } else { b = de; }
                }
                case 22u: { b = blend_set_lum(blend_set_sat(se, blend_sat(de)), blend_lum(de)); }
                case 23u: { b = blend_set_lum(blend_set_sat(de, blend_sat(se)), blend_lum(de)); }
                case 24u: { b = blend_set_lum(se, blend_lum(de)); }
                case 25u: { b = blend_set_lum(de, blend_lum(se)); }
                default: {
                    b = vec3<f32>(
                        blend_separable(mode, se.r, de.r),
                        blend_separable(mode, se.g, de.g),
                        blend_separable(mode, se.b, de.b),
                    );
                }
            }
            o = vec4<f32>(blend_decode(b.r), blend_decode(b.g), blend_decode(b.b), s.a);
        }
    }
    return o;
}

@compute @workgroup_size(8, 8)
fn blend_mix(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let d = textureLoad(input, p, 0);
    let s = textureLoad(processed, p, 0);
    let b = blend_pixel(params.mode, d, s);
    textureStore(dst, p, d * (1.0 - params.mix_amt) + b * params.mix_amt);
}

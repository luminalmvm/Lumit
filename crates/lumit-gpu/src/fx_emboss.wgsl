// Emboss (docs/08-EFFECTS.md §3.67): the picture as grey relief. Mirrors
// lumit_core::fx::cpu::emboss op-for-op (§1.6: the CPU is the oracle).
//
// Two taps either side of the pixel along the light's axis, differenced
// PERCEPTUALLY (§3.58's curve, for Find edges' reason: a relief taken in light
// would be all highlight and no shadow) and written as grey to all three
// channels. The offset arrives as a vector in raster pixels — Direction and
// Relief are folded together host-side, so this kernel never sees an angle
// (§3.5's rule).
//
// Relief 0 is flat mid-grey rather than the identity, and deliberately: with no
// separation between the taps there is no relief to see, and calling that "off"
// would be a lie. Mix is what turns the effect down.
//
// Edges repeat. Alpha is untouched. Mix 0 is the bit-exact identity.

struct Params {
    offset: vec2<f32>,  // toward the light, raster pixels
    contrast: f32,      // Contrast / 100
    mix_amt: f32,       // 0..1, blended against the unprocessed input
    matte_on: f32,      // 1 = the matte scales Relief per pixel (K-428)
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

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::bilinear_edge with the Repeat policy (edge == 1): the coordinate is
// clamped, which IS the policy, so there is no out-of-range fetch to guard
// (K-402's hazard does not arise here).
fn bilinear_repeat(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let hi = size - vec2<i32>(1, 1);
    let c00 = textureLoad(src, clamp(vec2<i32>(x0i, y0i), vec2<i32>(0, 0), hi), 0);
    let c10 = textureLoad(src, clamp(vec2<i32>(x0i + 1, y0i), vec2<i32>(0, 0), hi), 0);
    let c01 = textureLoad(src, clamp(vec2<i32>(x0i, y0i + 1), vec2<i32>(0, 0), hi), 0);
    let c11 = textureLoad(src, clamp(vec2<i32>(x0i + 1, y0i + 1), vec2<i32>(0, 0), hi), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

fn luma_at(sx: f32, sy: f32, size: vec2<i32>) -> f32 {
    let u = unpremult(bilinear_repeat(sx, sy, size));
    return sqrt(max(dot(u, LUMA), 0.0));
}

@compute @workgroup_size(8, 8)
fn emboss(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    // The matte pulls Relief toward 0 per pixel, before the taps are read
    // (K-428): a shallower relief, not a fade — and at black the flat sheet,
    // because Relief 0 is mid-grey and not the identity.
    var off = p.offset;
    if (p.matte_on != 0.0) {
        let k = matte_k(xy);
        off = vec2<f32>(matte_toward(off.x, 0.0, k), matte_toward(off.y, 0.0, k));
    }
    let a = luma_at(px - off.x, py - off.y, size);
    let b = luma_at(px + off.x, py + off.y, size);
    let g = max(0.5 + (b - a) * p.contrast, 0.0);
    let v = g * g;
    let outv = o.rgb * (1.0 - p.mix_amt) + vec3<f32>(v) * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

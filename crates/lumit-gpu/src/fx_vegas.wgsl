// Vegas (docs/08-EFFECTS.md §3.76): marching lights along the picture's
// contours. Mirrors lumit_core::fx::cpu::vegas and ::vegas_stroke op-for-op
// (§1.6: the CPU is the oracle).
//
// THE CONTOUR IS A LEVEL SET, not an edge detector's output: the value's
// distance from the threshold divided by the gradient's magnitude is a distance
// in PIXELS, which is what makes Width a width. A flat neighbourhood sends that
// distance to infinity, which switches the stroke off rather than lighting it.
//
// The gradient is a separable 5×5 Sobel, and the two extra taps each way are
// what make the dashes possible: a 3×3 gradient on compressed footage points a
// different way in almost every pixel, and the dashes come out as speckle.
//
// The value is the PERCEPTUAL luma (§3.58's curve, K-404), or the alpha, and the
// pixel's position is measured from the middle of the frame — see cpu::
// vegas_stroke for why the arm matters.
//
// Mix 0, Width 0 and Opacity 0 are all the bit-exact identity.

struct Params {
    colour: vec4<f32>,   // scene-linear; the alpha lane is ignored
    level: f32,          // the contour's level, 0..1 in the read value
    half_width: f32,     // the stroke's half-width, raster px
    band: f32,           // the soft band either side, raster px, floored
    inv_segment: f32,    // 1 ÷ Segment length, raster px
    duty: f32,           // the lit share of a segment; 2 for a continuous outline
    phase: f32,          // Rotation in turns
    opacity: f32,        // 0..1
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    from_alpha: u32,     // 1 reads the alpha rather than the luma
    composite: u32,      // 1 keeps the layer under the stroke
    matte_on: f32,       // 1 = the matte scales Opacity per pixel (K-428)
    _pad1: u32,
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

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == lumit_core::fx::cpu::vegas_value: one clamped tap, as the perceptual luma
// of the unpremultiplied colour or as the alpha.
fn tap(xy: vec2<i32>, d: vec2<i32>, size: vec2<i32>) -> f32 {
    let c = clamp(xy + d, vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    let v = textureLoad(src, c, 0);
    if (p.from_alpha != 0u) {
        return v.a;
    }
    let u = unpremult(v);
    return sqrt(max(u.r, 0.0)) * 0.2126
         + sqrt(max(u.g, 0.0)) * 0.7152
         + sqrt(max(u.b, 0.0)) * 0.0722;
}

@compute @workgroup_size(8, 8)
fn vegas(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // The CPU reference's own weights, in its own loop order.
    var smooth_w = array<f32, 5>(1.0, 4.0, 6.0, 4.0, 1.0);
    var deriv_w = array<f32, 5>(-1.0, -2.0, 0.0, 2.0, 1.0);
    var l = 0.0;
    var gx = 0.0;
    var gy = 0.0;
    for (var j = 0; j < 5; j = j + 1) {
        for (var i = 0; i < 5; i = i + 1) {
            let v = tap(xy, vec2<i32>(i - 2, j - 2), size);
            l = l + smooth_w[i] * smooth_w[j] * v;
            gx = gx + deriv_w[i] * smooth_w[j] * v;
            gy = gy + smooth_w[i] * deriv_w[j] * v;
        }
    }
    // 16 for each smoothing pass, 8 for the derivative's own scale.
    let lv = l * (1.0 / 256.0);
    let gxv = gx * (1.0 / 128.0);
    let gyv = gy * (1.0 / 128.0);

    let g = sqrt(gxv * gxv + gyv * gyv);
    let sd = (lv - p.level) / max(g, 1e-6);
    let across = clamp((p.half_width - abs(sd)) / p.band + 0.5, 0.0, 1.0);
    let inv = 1.0 / max(g, 1e-6);
    let tx = -gyv * inv;
    let ty = gxv * inv;
    let px = f32(xy.x) + 0.5 - f32(size.x) * 0.5;
    let py = f32(xy.y) + 0.5 - f32(size.y) * 0.5;
    let phase = (px * tx + py * ty) * p.inv_segment + p.phase;
    let frac = phase - floor(phase);
    let soft = max(p.band * p.inv_segment, 1e-4);
    let along = clamp((p.duty - frac) / soft + 0.5, 0.0, 1.0);

    // The matte pulls Opacity toward 0 per pixel, before the composite (K-428).
    var opacity = p.opacity;
    if (p.matte_on != 0.0) {
        opacity = matte_toward(opacity, 0.0, matte_k(xy));
    }
    let cov = across * along * opacity;
    let keep = (1.0 - cov) * f32(p.composite);
    let lit = vec4<f32>(o.rgb * keep + p.colour.rgb * cov, o.a * keep + cov);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + lit * p.mix_amt);
}

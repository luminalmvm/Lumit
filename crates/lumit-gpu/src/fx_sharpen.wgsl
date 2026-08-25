// Unsharp mask (docs/08-EFFECTS.md §3.9) in linear light on unpremultiplied
// colour (§2.2). Mirrors lumit_core::fx::cpu::sharpen op-for-op (§1.6: the
// CPU is the oracle). Two entry points: `unpremultiply` prepares the colour
// the internal gaussian blurs (the blur passes reuse fx_blur.wgsl with
// Repeat edges), and `sharpen_combine` recomputes the unpremultiplied
// original from the untouched input — same formula, same values as the CPU —
// gates the detail with the soft threshold, re-premultiplies and applies the
// host Mix.

struct Params {
    amount: f32,     // fraction of detail added back, 0..3
    threshold: f32,  // linear-light soft gate
    luma_only: u32,  // 1 = sharpen Rec. 709 luma only
    mix_amt: f32,    // 0..1, blended against `orig`
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

// Rec. 709 luma weights, in linear light (== cpu::LUMA).
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The unpremultiplied colour of a premultiplied pixel; a fully transparent
// pixel's colour is undefined and reads as black (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// Soft threshold gate, explicit branches to match the CPU bit-for-bit
// (== cpu::soft_gate).
fn soft_gate(d: f32, t: f32) -> f32 {
    if (d > t) {
        return d - t;
    }
    if (d < -t) {
        return d + t;
    }
    return 0.0;
}

// src (= orig here) premultiplied → dst unpremultiplied, alpha carried.
@compute @workgroup_size(8, 8)
fn unpremultiply(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let c = textureLoad(src, xy, 0);
    textureStore(dst, xy, vec4<f32>(unpremult(c), c.a));
}

// src = the blurred unpremultiplied colour; orig = the untouched
// premultiplied input the result mixes against.
@compute @workgroup_size(8, 8)
fn sharpen_combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(orig, xy, 0);
    let b = textureLoad(src, xy, 0).rgb;
    let u = unpremult(o);
    // The matte scales Amount per pixel (K-395): less detail added back.
    var amount = p.amount;
    if (p.matte_on != 0.0) {
        amount = amount * matte_k(xy);
    }
    var v: vec3<f32>;
    if (p.luma_only == 1u) {
        let d = soft_gate(
            (u.r * LUMA.r + u.g * LUMA.g + u.b * LUMA.b)
                - (b.r * LUMA.r + b.g * LUMA.g + b.b * LUMA.b),
            p.threshold,
        );
        v = u + vec3<f32>(amount * d);
    } else {
        v = vec3<f32>(
            u.r + amount * soft_gate(u.r - b.r, p.threshold),
            u.g + amount * soft_gate(u.g - b.g, p.threshold),
            u.b + amount * soft_gate(u.b - b.b, p.threshold),
        );
    }
    // Undershoot clamps at zero (no negative light); re-premultiply.
    let sharpened = vec4<f32>(max(v, vec3<f32>(0.0)) * o.a, o.a);
    textureStore(dst, xy, mix(o, sharpened, p.mix_amt));
}

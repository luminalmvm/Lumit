// Glow (docs/08-EFFECTS.md §3.3, v1 core): exposure-aware bloom in
// scene-linear light. Mirrors lumit_core::fx::cpu::glow op-for-op (§1.6:
// the CPU is the oracle). Two entry points: `glow_bright` keeps only the
// light above the threshold (soft knee, all four premultiplied channels
// alike — the halo carries alpha so glow spreads over transparency like
// light); the host then blurs that leftover with the shared gaussian
// (fx_blur.wgsl, Repeat edges), and `glow_combine` adds the halo back:
// out = input + intensity · tint · halo, alpha saturating at 1, highlights
// never clipped (§2.1). Intensity 0 short-circuits to the bit-exact
// identity, exactly like the CPU reference's early return.

struct Params {
    tint: vec4<f32>,   // scene-linear halo tint; alpha unused
    threshold: f32,    // linear-light bright threshold, ≥ 0 (K-090: open above)
    knee: f32,         // soft-knee width around the threshold, 0..1
    intensity: f32,    // halo gain; 0 is the neutral point
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    matte_on: f32,     // 1 = gate the bright pass by the matte (K-395)
    _pad0: f32,        // was Invert; applied once at the seam since K-425
    _pad: vec2<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The bright pass on one channel (== lumit_core::fx::glow_bright, same
// arithmetic order): max(0, x − threshold), its onset weighted by a
// smoothstep over threshold ± knee; knee 0 is the hard subtract.
fn bright(x: f32) -> f32 {
    let d = x - p.threshold;
    if (d <= 0.0) {
        return 0.0;
    }
    if (p.knee > 0.0) {
        let t = clamp((x - (p.threshold - p.knee)) / (2.0 * p.knee), 0.0, 1.0);
        let w = t * t * (3.0 - 2.0 * t);
        return d * w;
    }
    return d;
}

// src premultiplied → dst, the light above the threshold.
//
// **The Matte gates the seed** (K-395, docs/08 §2.6). Glow is one of the effects
// that claim the matte inside their own maths, and this is where: the source is
// multiplied by the matte's luma BEFORE the threshold, so only what the matte
// lights is allowed to bloom. The halo then spreads from those pixels normally,
// across dark matte and past the matte's edge — which is exactly what dissolving
// the finished glow cannot do, since that would clip the halo to the matte.
//
// `orig` carries the MATTE on this entry point, not the untouched input: the
// bright pass is dispatched with src == orig (it needs one picture), so the
// second slot was free, and using it costs no binding and no second layout.
// With `matte_on == 0` the matte is never read and the store is bit-for-bit what
// it was before K-395 (K-258).
@compute @workgroup_size(8, 8)
fn glow_bright(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var c = textureLoad(src, xy, 0);
    if (p.matte_on != 0.0) {
        let m = textureLoad(orig, xy, 0);
        // Channel pick and Invert already applied, once, by
        // fx_matte_prepare.wgsl (K-425).
        let k = clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
        c = c * k;
    }
    textureStore(dst, xy, vec4<f32>(bright(c.r), bright(c.g), bright(c.b), bright(c.a)));
}

// src = the blurred halo; orig = the untouched premultiplied input.
@compute @workgroup_size(8, 8)
fn glow_combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(orig, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.intensity == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let hl = textureLoad(src, xy, 0);
    let glowed = vec3<f32>(
        o.r + p.intensity * (hl.r * p.tint.r),
        o.g + p.intensity * (hl.g * p.tint.g),
        o.b + p.intensity * (hl.b * p.tint.b),
    );
    let a = min(o.a + p.intensity * hl.a, 1.0);
    let outv = o.rgb * (1.0 - p.mix_amt) + glowed * p.mix_amt;
    let outa = o.a * (1.0 - p.mix_amt) + a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}

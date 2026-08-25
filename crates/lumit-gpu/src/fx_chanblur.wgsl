// Channel blur (docs/08-EFFECTS.md §3.45) — one pass per axis, direction in the
// uniform, a radius and σ per channel. Mirrors lumit_core::fx::cpu::channel_blur
// op-for-op (§1.6: the CPU is the oracle): same σ = radius/2 floored at 1e-3,
// same tap count ceil(radius), same in-loop weights normalised over the taps
// actually summed, fixed tap order.
//
// The four channels no longer share one weight table, so both paths accumulate
// unnormalised and divide at the end — the arrangement fx_blur.wgsl's matted
// path uses, for the same reason.
//
// A channel whose radius is zero takes its own sample untouched, which is what
// makes the common case (one channel softened, three left alone) bit-exact on
// the three and cost one channel's gather.

struct Params {
    radius: vec4<f32>,   // per channel, raster px
    sigma: vec4<f32>,    // per channel, radius * 0.5 floored at 1e-3
    dir: vec2<f32>,      // (1,0) horizontal pass, (0,1) vertical pass
    mix_amt: f32,        // 0..1, blended against `orig` (1 on the h-pass)
    edge: u32,           // 0 transparent, 1 repeat
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

// == cpu::edge_index. -1 means transparent (no tap, full weight).
fn edge_idx(i: i32, len: i32) -> i32 {
    if (i >= 0 && i < len) {
        return i;
    }
    if (p.edge == 1u) {
        return clamp(i, 0, len - 1);
    }
    return -1;
}

@compute @workgroup_size(8, 8)
fn channel_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    // Held in `var`s so the per-channel loop indexes references rather than
    // values: dynamic indexing of a vector *value* is the one WGSL construct
    // this kernel would otherwise need and does not have to.
    var here = textureLoad(src, xy, 0);
    var rad = p.radius;
    var sig = p.sigma;
    // The matte scales all four radii per pixel (K-395): the destination
    // pixel's, on both passes, so the two halves agree on its kernel width
    // (== cpu::channel_blur_matted, the same ceil and floor as the host).
    if (p.matte_on != 0.0) {
        rad = rad * matte_k(xy);
        sig = max(rad * 0.5, vec4<f32>(1e-3));
    }
    let axis_len = select(size.y, size.x, p.dir.x > 0.5);
    let along = select(xy.y, xy.x, p.dir.x > 0.5);
    var acc = vec4<f32>(0.0);
    for (var c = 0; c < 4; c++) {
        let r = i32(ceil(rad[c]));
        if (r == 0) {
            acc[c] = here[c];
            continue;
        }
        var sum = 0.0;
        var wsum = 0.0;
        for (var i = -r; i <= r; i++) {
            let d = f32(i) / sig[c];
            let wt = exp(-0.5 * d * d);
            wsum += wt;
            let q = edge_idx(along + i, axis_len);
            if (q >= 0) {
                var tapxy = xy;
                if (p.dir.x > 0.5) {
                    tapxy.x = q;
                } else {
                    tapxy.y = q;
                }
                var t = textureLoad(src, tapxy, 0);
                sum += t[c] * wt;
            }
        }
        acc[c] = sum / wsum;
    }
    let o = textureLoad(orig, xy, 0);
    textureStore(dst, xy, mix(o, acc, p.mix_amt));
}

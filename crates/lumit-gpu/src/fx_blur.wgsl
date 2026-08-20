// Separable gaussian blur (docs/08-EFFECTS.md §3.8) — one pass per axis,
// direction in the uniform. Mirrors lumit_core::fx::cpu::blur_gaussian and
// ::blur_gaussian_matted op-for-op (§1.6: the CPU is the oracle): same
// σ = radius/2, same tap count ceil(radius), weights normalised over the FULL
// kernel regardless of edge policy, fixed tap order.
//
// **The Matte scales the radius, per pixel** (K-395, docs/08 §2.6). The
// Gaussian blur is one of the effects that claim the matte inside their own
// maths rather than taking the generic strength dissolve, and this is where:
// each destination pixel's own matte luma multiplies the radius before the
// kernel is built, so grey blurs narrowly and black not at all. That is a
// picture a dissolve cannot make — dissolving a 40 px blur to 50 % is a sharp
// image with a wide halo over it, not a 20 px blur.
//
// Both passes read the DESTINATION pixel's matte, which is what makes the two
// separable halves agree about how wide this pixel's kernel is.
//
// With `matte_on == 0` the radius and σ are used exactly as the host computed
// them, untouched — the byte-for-byte path every project saved before K-395
// takes (K-258).

struct Params {
    dir: vec2<f32>,     // (1,0) horizontal pass, (0,1) vertical pass
    radius: f32,        // kernel half-width, px
    sigma: f32,         // radius * 0.5, clamped ≥ 1e-3
    edge: u32,          // 0 transparent, 1 repeat, 2 mirror
    mix_amt: f32,       // 0..1, blended against `orig` (1 on the h-pass)
    matte_on: f32,      // 1 = scale the radius by the matte's luma
    invert: f32,        // 1 = the matte drives where it is DARK
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
// The generic Matte (K-395), bound for every kernel on this layout and read
// only by the ones that claim it. Bound to `src` when there is none, and gated
// by `matte_on` — a texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength: premultiplied Rec. 709 luma, clamped, then
// inverted — the same reading `fx_matte_mix.wgsl` and cpu::matte_mix use, so
// "how much matte is here" means one thing across the whole campaign.
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    let k = clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
    if (p.invert != 0.0) {
        return 1.0 - k;
    }
    return k;
}

// Resolve a tap index under the edge policy; -1 means transparent (no tap).
fn edge_idx(i: i32, len: i32) -> i32 {
    if (i >= 0 && i < len) {
        return i;
    }
    if (p.edge == 1u) {
        return clamp(i, 0, len - 1);
    }
    if (p.edge == 2u) {
        var m = i;
        if (m < 0) {
            m = -m;
        } else {
            m = 2 * (len - 1) - m;
        }
        return clamp(m, 0, len - 1);
    }
    return -1;
}

@compute @workgroup_size(8, 8)
fn blur_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var rad = p.radius;
    var sig = p.sigma;
    if (p.matte_on != 0.0) {
        rad = p.radius * matte_k(xy);
        sig = max(rad * 0.5, 1e-3);
    }
    let r = i32(ceil(rad));
    var acc = vec4<f32>(0.0);
    if (r == 0) {
        acc = textureLoad(src, xy, 0);
    } else {
        let axis_len = select(size.y, size.x, p.dir.x > 0.5);
        let along = select(xy.y, xy.x, p.dir.x > 0.5);
        var wsum = 0.0;
        for (var i = -r; i <= r; i++) {
            let d = f32(i) / max(sig, 1e-3);
            let wt = exp(-0.5 * d * d);
            wsum += wt;
            let q = edge_idx(along + i, axis_len);
            if (q >= 0) {
                var tap = xy;
                if (p.dir.x > 0.5) {
                    tap.x = q;
                } else {
                    tap.y = q;
                }
                acc += textureLoad(src, tap, 0) * wt;
            }
        }
        acc /= wsum;
    }
    let o = textureLoad(orig, xy, 0);
    textureStore(dst, xy, mix(o, acc, p.mix_amt));
}

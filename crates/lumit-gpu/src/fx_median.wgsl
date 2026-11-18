// Median (docs/08-EFFECTS.md §3.64): the middle value of a neighbourhood.
// Mirrors lumit_core::fx::cpu::median op-for-op (§1.6: the CPU is the oracle).
//
// The selection is a COMPARE-EXCHANGE NETWORK and nothing here branches on a
// value. A quickselect would diverge every lane in a warp and would execute a
// different sequence of comparisons on the two paths, which §1.6 could not hold
// to agreement; this sweeps the window once, carrying the `keep` smallest values
// seen so far in a sorted run and inserting each new sample by bubbling it
// down with min/max pairs. Because min and max on a vector are componentwise,
// the three colour channels — and the alpha — are selected SIMULTANEOUSLY, four
// medians for the price of one network.
//
// Only the requested window is swept, as on the CPU. The run starts full of PAD,
// a value larger than any real pixel, so the first slot still holding PAD takes
// the carried value and passes PAD on, which changes nothing further down. Both
// paths carry the same run, and the answers are bit-identical because min and
// max are exact and a sorted set does not depend on insertion order.
//
// Edges repeat (the coordinate is clamped): a transparent surround would win the
// vote on a corner pixel and eat the frame's own border. Radius 0 is the
// bit-exact identity.

struct Params {
    radius: i32,     // 0..=3, whole raster pixels, rounded host-side
    keep: i32,       // ceil(N / 2) for this radius, computed host-side
    alpha_on: f32,   // 1 to median the coverage too, 0 to leave it
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    matte_on: f32,   // 1 = the matte scales Radius per pixel
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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

// == cpu::MEDIAN_MAX_RADIUS and cpu::MEDIAN_KEEP. Held to those constants by the
// §1.6 oracle test.
const MAX_RADIUS: i32 = 3;
const KEEP: i32 = 25;
// == cpu::MEDIAN_PAD.
const PAD: f32 = 1e30;

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// One rung of the network: the slot keeps the smaller value and the larger
// one carries on down.
fn ce(slot: ptr<function, vec4<f32>>, v: ptr<function, vec4<f32>>) {
    let lo = min(*slot, *v);
    let hi = max(*slot, *v);
    *slot = lo;
    *v = hi;
}

@compute @workgroup_size(8, 8)
fn median(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // The matte pulls Radius toward 0 per pixel, before the window is swept:
    // a genuinely narrower window, rounded with `floor(x + ½)` so
    // WGSL's tie-to-even cannot part company with Rust's tie-away-from-zero.
    var r = clamp(p.radius, 0, MAX_RADIUS);
    if (p.matte_on != 0.0) {
        let scaled = matte_toward(f32(r), 0.0, matte_k(xy));
        r = clamp(i32(floor(scaled + 0.5)), 0, MAX_RADIUS);
    }
    if (r <= 0) {
        textureStore(dst, xy, o);
        return;
    }
    // The rank the network has to carry, for THIS pixel's window (== the CPU
    // reference's `keep`); the host's own `p.keep` is the same number at r.
    let keep = ((2 * r + 1) * (2 * r + 1) + 1) / 2;
    // The sorted run is KEEP named values, not an array. An array indexed by a
    // loop counter compiles to an indexable one, which some drivers keep in
    // memory rather than registers, and then every rung is a round trip to it.
    // Named, the run stays in registers. Rungs past `keep` cannot change the
    // answer, so a smaller window skips them.
    let pad = vec4<f32>(PAD, PAD, PAD, PAD);
    var s0 = pad;
    var s1 = pad;
    var s2 = pad;
    var s3 = pad;
    var s4 = pad;
    var s5 = pad;
    var s6 = pad;
    var s7 = pad;
    var s8 = pad;
    var s9 = pad;
    var s10 = pad;
    var s11 = pad;
    var s12 = pad;
    var s13 = pad;
    var s14 = pad;
    var s15 = pad;
    var s16 = pad;
    var s17 = pad;
    var s18 = pad;
    var s19 = pad;
    var s20 = pad;
    var s21 = pad;
    var s22 = pad;
    var s23 = pad;
    var s24 = pad;
    for (var dy = -r; dy <= r; dy++) {
        for (var dx = -r; dx <= r; dx++) {
            let s = clamp(xy + vec2<i32>(dx, dy), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
            let c = textureLoad(src, s, 0);
            var v = vec4<f32>(unpremult(c), c.a);
            ce(&s0, &v);
            ce(&s1, &v);
            ce(&s2, &v);
            ce(&s3, &v);
            ce(&s4, &v);
            if (keep > 5) {
                ce(&s5, &v);
                ce(&s6, &v);
                ce(&s7, &v);
                ce(&s8, &v);
                ce(&s9, &v);
                ce(&s10, &v);
                ce(&s11, &v);
                ce(&s12, &v);
            }
            if (keep > 13) {
                ce(&s13, &v);
                ce(&s14, &v);
                ce(&s15, &v);
                ce(&s16, &v);
                ce(&s17, &v);
                ce(&s18, &v);
                ce(&s19, &v);
                ce(&s20, &v);
                ce(&s21, &v);
                ce(&s22, &v);
                ce(&s23, &v);
                ce(&s24, &v);
            }
        }
    }
    // keep is 5, 13 or 25: the middle of a 3, 5 or 7 pixel square.
    var med = s4;
    if (keep == 13) {
        med = s12;
    } else if (keep == 25) {
        med = s24;
    }
    // `select`, not a lerp: the CPU reference chooses one of the two values
    // outright, and `a + (b - a)·1` is not bit-exactly `b`.
    let out_a = select(o.a, med.a, p.alpha_on > 0.5);
    let outv = o.rgb * (1.0 - p.mix_amt) + med.rgb * out_a * p.mix_amt;
    let outa = o.a * (1.0 - p.mix_amt) + out_a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}

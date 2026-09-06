// Curves (docs/08-EFFECTS.md §3.30): a tone curve per channel, baked
// host-side into a 257-entry table and read here as a lookup, evaluated on
// unpremultiplied colour (§2.2, the wrap fused into the kernel) and
// re-premultiplied on the way out. Mirrors lumit_core::fx::cpu::curves
// op-for-op (§1.6: the CPU is the oracle).
//
// The spline is NOT fitted here. `Curves::packed` fits the clamped cubic once
// a frame in f64 and writes down 257 samples a channel, so this kernel and the
// CPU reference read identical numbers and neither draws a curve per pixel —
// Lightning's discipline (§3.74) on a shape that is the same for every pixel.
//
// Channel 0 is Master, 1..3 are R/G/B, 4 is Alpha. The per-channel curve runs
// first, then Master — Photoshop's and AE's order. Master never touches alpha.

// 257 entries a channel, 65 vec4s a channel, five channels.
const N = 257;
const V = 65;

struct Params {
    t: array<vec4<f32>, 5 * V>,  // the five tables, four entries a vec4
    mix_amt: f32,                // 0..1, blended against the unprocessed input
    neutral: u32,                // every channel is the identity diagonal
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// Entry i of channel c (== cpu's `t[c][i]`).
fn tab(c: i32, i: i32) -> f32 {
    let v = p.t[c * V + (i >> 2)];
    return v[i & 3];
}

// One channel's baked curve at x (== cpu::curve_at): a table lookup with
// linear interpolation. The index is clamped and the fraction is not, so an
// input outside 0..1 extrapolates along the first or last segment rather than
// clipping — a scene-linear value above 1 keeps being curved honestly (§2.1)
// and a slightly negative one stays continuous.
fn curve_at(x: f32, c: i32) -> f32 {
    let last = f32(N - 1);
    let s = x * last;
    let fi = floor(clamp(s, 0.0, last - 1.0));
    let i = i32(fi);
    let f = s - fi;
    let a = tab(c, i);
    return a + (tab(c, i + 1) - a) * f;
}

@compute @workgroup_size(8, 8)
fn curves(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return): the
    // identity curve on all five channels, decided host-side.
    if (p.neutral != 0u) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let graded_a = curve_at(o.a, 4);
    let v = vec3<f32>(
        curve_at(curve_at(u.r, 1), 0),
        curve_at(curve_at(u.g, 2), 0),
        curve_at(curve_at(u.b, 3), 0),
    );
    let graded = v * graded_a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    let outa = o.a * (1.0 - p.mix_amt) + graded_a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}

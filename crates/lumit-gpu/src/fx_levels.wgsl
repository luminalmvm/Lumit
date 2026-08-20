// Levels (docs/08-EFFECTS.md §3.31): input black/white, gamma and output
// black/white per channel, on unpremultiplied colour (§2.2, the wrap fused
// into the kernel) and re-premultiplied on the way out. Mirrors
// lumit_core::fx::cpu::levels op-for-op (§1.6: the CPU is the oracle).
//
// Both reciprocals arrive precomputed from `Levels::packed`, so nothing
// divides per pixel and the two paths cannot disagree in the last bit. Lane 0
// of every row is Master, lanes 1..3 are R/G/B; the per-channel map runs
// first, then Master.

struct Params {
    r: array<vec4<f32>, 5>,  // in_black, 1/in_span, 1/gamma, out_black, out_span
    mix_amt: f32,            // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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

// One channel's level map (== cpu::level_at). Clamped at zero before the
// power exactly as the gamma effect clamps — a power of a negative base is
// undefined — and deliberately NOT clamped above: a value past the input
// white travels on rather than clipping (§2.1).
fn level_at(x: f32, c: i32) -> f32 {
    var n = max((x - p.r[0][c]) * p.r[1][c], 0.0);
    let ig = p.r[2][c];
    if (ig != 1.0) {
        n = pow(n, ig);
    }
    return p.r[3][c] + p.r[4][c] * n;
}

@compute @workgroup_size(8, 8)
fn levels(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (all(p.r[0] == vec4<f32>(0.0))
        && all(p.r[1] == vec4<f32>(1.0))
        && all(p.r[2] == vec4<f32>(1.0))
        && all(p.r[3] == vec4<f32>(0.0))
        && all(p.r[4] == vec4<f32>(1.0))) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let v = vec3<f32>(
        level_at(level_at(u.r, 1), 0),
        level_at(level_at(u.g, 2), 0),
        level_at(level_at(u.b, 3), 0),
    );
    let graded = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

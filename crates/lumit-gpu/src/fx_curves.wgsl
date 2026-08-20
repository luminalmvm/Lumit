// Curves (docs/08-EFFECTS.md §3.30, K-396): a monotone-cubic tone curve per
// channel — five knots at the fixed inputs 0, 0.25, 0.5, 0.75, 1, evaluated on
// unpremultiplied colour (§2.2, the wrap fused into the kernel) and
// re-premultiplied on the way out. Mirrors lumit_core::fx::cpu::curves
// op-for-op (§1.6: the CPU is the oracle).
//
// The spline is NOT fitted here: the knots and their Fritsch-Carlson limited
// tangents both arrive in the uniform, computed once host-side by
// `Curves::packed`, so this kernel and the CPU reference evaluate identical
// numbers and neither fits a curve per pixel.
//
// Lane 0 of every knot is Master, lanes 1..3 are R/G/B. The per-channel curve
// runs first, then Master — Photoshop's and AE's order.

struct Params {
    y: array<vec4<f32>, 5>,  // knot outputs, [knot][channel]
    m: array<vec4<f32>, 5>,  // monotone tangents, same indexing
    mix_amt: f32,            // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The knot spacing (== cpu::CURVE_H). Uniform by construction, which is what
// keeps the interval lookup a multiply and a floor.
const H = 0.25;

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// One channel's curve at x (== cpu::curve_at): cubic Hermite between the two
// knots either side, a straight line along the end tangent outside 0..1 — so
// scene-linear values above 1 curve honestly and are never clipped (§2.1).
fn curve_at(x: f32, c: i32) -> f32 {
    if (x <= 0.0) {
        return p.y[0][c] + p.m[0][c] * x;
    }
    if (x >= 1.0) {
        return p.y[4][c] + p.m[4][c] * (x - 1.0);
    }
    let fi = floor(x * 4.0);
    let i = min(i32(fi), 3);
    let t = (x - fi * H) / H;
    let t2 = t * t;
    let t3 = t2 * t;
    return p.y[i][c] * (2.0 * t3 - 3.0 * t2 + 1.0)
        + p.m[i][c] * H * (t3 - 2.0 * t2 + t)
        + p.y[i + 1][c] * (-2.0 * t3 + 3.0 * t2)
        + p.m[i + 1][c] * H * (t3 - t2);
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
    // identity curve on all four channels.
    if (all(p.y[0] == vec4<f32>(0.0))
        && all(p.y[1] == vec4<f32>(0.25))
        && all(p.y[2] == vec4<f32>(0.5))
        && all(p.y[3] == vec4<f32>(0.75))
        && all(p.y[4] == vec4<f32>(1.0))) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let v = vec3<f32>(
        curve_at(curve_at(u.r, 1), 0),
        curve_at(curve_at(u.g, 2), 0),
        curve_at(curve_at(u.b, 3), 0),
    );
    let graded = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

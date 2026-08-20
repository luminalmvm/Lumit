// Find edges (docs/08-EFFECTS.md §3.66): the picture as a pencil drawing.
// Mirrors lumit_core::fx::cpu::find_edges op-for-op (§1.6: the CPU is the
// oracle).
//
// THE GRADIENT IS TAKEN ON THE PERCEPTUAL VALUE, not on the light (§3.58's curve
// again). In scene-linear light the step from 3.0 to 4.0 in a sunlit sky is a
// bigger number than the step from 0.01 to 0.05 in a shadow, though the eye sees
// the second and not the first — a Sobel in light draws the specular highlights
// and nothing else.
//
// The nine taps are unrolled in the CPU reference's own order, and the zero
// weights are written out as skipped terms rather than as multiplications,
// because adding `t * 0` is exactly adding nothing.
//
// Edges repeat. Alpha is untouched, so the drawing keeps the layer's shape.
// Mix 0 is the bit-exact identity.

struct Params {
    invert: f32,     // 1 = bright edges on black, 0 = AE's dark on white
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// One clamped tap, already unpremultiplied and put on the perceptual curve
// (== cpu::perceptual of cpu::unpremult).
fn pt(xy: vec2<i32>, dx: i32, dy: i32, size: vec2<i32>) -> vec3<f32> {
    let c = clamp(xy + vec2<i32>(dx, dy), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return sqrt(max(unpremult(textureLoad(src, c, 0)), vec3<f32>(0.0)));
}

@compute @workgroup_size(8, 8)
fn find_edges(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let t00 = pt(xy, -1, -1, size);
    let t01 = pt(xy, 0, -1, size);
    let t02 = pt(xy, 1, -1, size);
    let t10 = pt(xy, -1, 0, size);
    let t12 = pt(xy, 1, 0, size);
    let t20 = pt(xy, -1, 1, size);
    let t21 = pt(xy, 0, 1, size);
    let t22 = pt(xy, 1, 1, size);
    var gx = t00 * -1.0;
    gx = gx + t02 * 1.0;
    gx = gx + t10 * -2.0;
    gx = gx + t12 * 2.0;
    gx = gx + t20 * -1.0;
    gx = gx + t22 * 1.0;
    var gy = t00 * -1.0;
    gy = gy + t01 * -2.0;
    gy = gy + t02 * -1.0;
    gy = gy + t20 * 1.0;
    gy = gy + t21 * 2.0;
    gy = gy + t22 * 1.0;
    let e = min(sqrt(gx * gx + gy * gy), vec3<f32>(1.0));
    // `1 - e` for the pencil drawing, `e` for the glow. One lerp so neither path
    // takes a branch on the switch.
    let base = vec3<f32>(1.0) - e;
    let q = base + (e - base) * p.invert;
    let outv = o.rgb * (1.0 - p.mix_amt) + q * q * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

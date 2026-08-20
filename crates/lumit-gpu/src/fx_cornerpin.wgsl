// Corner pin (docs/08-EFFECTS.md §3.48): the picture pulled onto four points.
// Mirrors lumit_core::fx::cpu::corner_pin op-for-op (§1.6: the CPU is the
// oracle).
//
// The whole projective derivation — the unit-square-to-quad map, its adjugate,
// the sign normalisation — was taken host-side in CornerPin::packed, so this
// kernel is one matrix multiply, one divide and one bilinear tap. The inverse
// arrives up to a scale, which the perspective divide cancels; the sign was
// normalised so that "in front of the projection's horizon" is a plain w > 0
// here whichever way round the four points were dragged.
//
// Mix 0 and a degenerate quad are both the bit-exact identity.

struct Params {
    n0: vec4<f32>,    // inverse homography row 0 in xyz; w unused
    n1: vec4<f32>,    // row 1
    n2: vec4<f32>,    // row 2
    mix_amt: f32,     // 0..1, blended against the unprocessed input
    edge: u32,        // 0 transparent, 1 repeat, 2 mirror
    enabled: u32,     // 0 = a degenerate quad = the exact identity
    _pad0: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == fx_lensdistort.wgsl's edge_idx and cpu::edge_index. -1 means transparent.
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

// The tap NEVER loads out of bounds. A guard that early-returns before the
// textureLoad reads correctly on paper, but the load is side-effect-free and
// gets hoisted above the branch; on at least one Windows backend the hoisted
// out-of-range fetch comes back with a live alpha lane, so a pixel whose four
// taps are all outside the frame arrives opaque-and-wrong instead of empty.
// Clamping the coordinate and choosing the value afterwards has no such hazard,
// and costs one `select`. (Found by the §1.6 oracle for Polar coordinates,
// docs/08 §3.50 — the first kernel in the batch whose samples leave the frame.)
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xi = edge_idx(x, size.x);
    let yi = edge_idx(y, size.y);
    let c = clamp(vec2<i32>(xi, yi), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(textureLoad(src, c, 0), vec4<f32>(0.0, 0.0, 0.0, 0.0), xi < 0 || yi < 0);
}

fn bilinear_edge(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn corner_pin(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    if (p.enabled == 0u) {
        textureStore(dst, xy, o);
        return;
    }
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let u = p.n0.x * px + p.n0.y * py + p.n0.z;
    let t = p.n1.x * px + p.n1.y * py + p.n1.z;
    let d = p.n2.x * px + p.n2.y * py + p.n2.z;
    var v = vec4<f32>(0.0);
    if (d > 0.0) {
        v = bilinear_edge(u / d * f32(size.x), t / d * f32(size.y), size);
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

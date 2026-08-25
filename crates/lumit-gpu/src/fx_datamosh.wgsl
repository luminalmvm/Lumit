// Datamosh (docs/08-EFFECTS.md §3.12; reworked to a flow-driven melt by
// K-164/T19): simulate I-frame removal — the previous picture keeps being
// dragged along the current frame's motion, so moving regions smear and bloom
// while static ones stay. Mirrors lumit_core::fx::cpu::datamosh op-for-op
// (§1.6: the CPU is the oracle) — the same streamline walk, the same tap count
// and order, the same geometric bloom weights, edges clamped.
//
// binding 0 is the current (already-effected) frame the melt mixes over;
// binding 1 is the raw -1 neighbour source frame the walk samples; binding 2
// is the dense current→previous flow field (the same rgba32float convention
// fx_motionblur.wgsl uses, .xy the flow, .z an unread confidence lane).
// Reuses the shared three-sampled-input layout Motion blur's pass uses.
//
// Per pixel a streamline of `steps` taps follows the flow out of `prev`:
// starting at the pixel centre, each step re-samples the flow at the current
// position and advances by `displacement / steps` of it (~one frame of motion
// per step), then samples `prev`. Samples accumulate with weight `bloom^k`
// from the near end, so bloom 0 keeps the nearest step (a short trail) and
// bloom 1 averages the whole walk (a long melting bloom). The weighted mean
// blends over `cur` by intensity (> 1 extrapolates).

struct Params {
    intensity: f32,    // blended over the current frame (> 1 extrapolates)
    displacement: f32, // frames of motion the streamline walk reaches
    bloom: f32,        // 0..1, how much of the reach accumulates
    steps: i32,        // bilinear taps along the walk (== cpu::datamosh's steps)
};

@group(0) @binding(0) var cur: texture_2d<f32>;
@group(0) @binding(1) var prev: texture_2d<f32>;
@group(0) @binding(2) var flow: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<uniform> p: Params;

// Clamp-addressed bilinear of `prev` at continuous pixel-centre coordinates
// (== the cpu::bilinear rule, same arithmetic order): the texel at index x
// covers [x, x+1), centre x+0.5; out-of-frame taps read the edge.
fn bilinear_prev(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(prev, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c10 = textureLoad(prev, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c01 = textureLoad(prev, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let c11 = textureLoad(prev, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

// The same bilinear rule for the flow field's .xy (== cpu::bilinear_uv), so the
// walk follows curved motion identically on both paths.
fn bilinear_flow(sx: f32, sy: f32, size: vec2<i32>) -> vec2<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(flow, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0).xy;
    let c10 = textureLoad(flow, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0).xy;
    let c01 = textureLoad(flow, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0).xy;
    let c11 = textureLoad(flow, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0).xy;
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn datamosh(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(cur));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let n = max(p.steps, 1);
    let step = p.displacement / f32(n);
    var px = f32(xy.x) + 0.5;
    var py = f32(xy.y) + 0.5;
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    var wt = 1.0;
    for (var k = 0; k < n; k++) {
        let fuv = bilinear_flow(px, py, size);
        px += fuv.x * step;
        py += fuv.y * step;
        acc += bilinear_prev(px, py, size) * wt;
        wsum += wt;
        wt *= p.bloom;
    }
    let warped = acc / wsum;
    let c = textureLoad(cur, xy, 0);
    textureStore(dst, xy, mix(c, warped, p.intensity));
}

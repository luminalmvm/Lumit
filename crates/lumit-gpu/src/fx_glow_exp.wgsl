// The halo a glow builds once Falloff is above 0 (docs/08 §3.3). Three passes
// an exponential: shrink the bright pass onto a grid with a tent, convolve it
// there with a round exponential, then scale it back up and add it onto the
// halo. Mirrors lumit_core::fx::cpu::glow_exponential op for op.

struct Params {
    step: f32,    // grid step, full-size pixels a texel
    lambda: f32,  // decay length, grid texels
    weight: f32,  // this exponential's share of the light
    axis: u32,    // tent pass: 0 across, 1 down
    first: u32,   // 1 = the halo is empty, so orig isn't read
    taps: i32,    // how many texels out the convolution reads
    reach: f32,   // where it has faded to nothing, grid texels
    _pad: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// One axis of the tent onto the grid.
@compute @workgroup_size(8, 8)
fn glow_exp_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(dst));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let n = vec2<i32>(textureDimensions(src));
    var j = xy.x;
    var len = n.x;
    if (p.axis == 1u) {
        j = xy.y;
        len = n.y;
    }
    let c = (f32(j) + 0.5) * p.step - 0.5;
    let lo = i32(floor(c - p.step)) + 1;
    let hi = i32(ceil(c + p.step)) - 1;
    var acc = vec4<f32>(0.0);
    var ws = 0.0;
    for (var i = lo; i <= hi; i++) {
        let wt = 1.0 - abs(f32(i) - c) / p.step;
        if (wt > 0.0) {
            let k = clamp(i, 0, len - 1);
            var at = vec2<i32>(k, xy.y);
            if (p.axis == 1u) {
                at = vec2<i32>(xy.x, k);
            }
            acc += wt * textureLoad(src, at, 0);
            ws += wt;
        }
    }
    textureStore(dst, xy, acc / ws);
}

// The round exponential, faded out over the last third of its reach.
@compute @workgroup_size(8, 8)
fn glow_exp_conv(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(dst));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var acc = vec4<f32>(0.0);
    var ws = 0.0;
    for (var dy = -p.taps; dy <= p.taps; dy++) {
        for (var dx = -p.taps; dx <= p.taps; dx++) {
            let r = sqrt(f32(dx * dx + dy * dy));
            let q = r / p.reach;
            if (q < 1.0) {
                let s = clamp(q * 3.0 - 2.0, 0.0, 1.0);
                let wt = exp(-r / p.lambda) * (1.0 - s * s * (3.0 - 2.0 * s));
                let at = clamp(xy + vec2<i32>(dx, dy), vec2<i32>(0), size - 1);
                acc += wt * textureLoad(src, at, 0);
                ws += wt;
            }
        }
    }
    textureStore(dst, xy, acc / ws);
}

fn catmull_rom(t: f32) -> f32 {
    let a = abs(t);
    if (a < 1.0) {
        return (1.5 * a - 2.5) * a * a + 1.0;
    }
    if (a < 2.0) {
        return ((-0.5 * a + 2.5) * a - 4.0) * a + 2.0;
    }
    return 0.0;
}

// src = the convolved grid, orig = the halo so far, dst = the halo after.
// Below zero is clipped, since a halo never takes light away.
@compute @workgroup_size(8, 8)
fn glow_exp_up(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(dst));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let n = vec2<i32>(textureDimensions(src));
    let u = (f32(xy.x) + 0.5) / p.step - 0.5;
    let v = (f32(xy.y) + 0.5) / p.step - 0.5;
    let jx = floor(u);
    let jy = floor(v);
    let fx = u - jx;
    let fy = v - jy;
    var acc = vec4<f32>(0.0);
    for (var ky = 0; ky < 4; ky++) {
        let wy = catmull_rom(fy - f32(ky - 1));
        for (var kx = 0; kx < 4; kx++) {
            let wx = catmull_rom(fx - f32(kx - 1));
            let sx = clamp(i32(jx) + kx - 1, 0, n.x - 1);
            let sy = clamp(i32(jy) + ky - 1, 0, n.y - 1);
            acc += (wx * wy) * textureLoad(src, vec2<i32>(sx, sy), 0);
        }
    }
    var halo = vec4<f32>(0.0);
    if (p.first == 0u) {
        halo = textureLoad(orig, xy, 0);
    }
    textureStore(dst, xy, halo + p.weight * max(acc, vec4<f32>(0.0)));
}

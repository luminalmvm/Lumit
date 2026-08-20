// Mirror (docs/08-EFFECTS.md §3.41): one half of the frame reflected onto the
// other. Mirrors lumit_core::fx::cpu::mirror op-for-op (§1.6: the CPU is the
// oracle).
//
// The axis normal arrives as a host-computed (cos, sin) pair — this kernel runs
// no trigonometry (§1.6). Samples that land outside the frame read as
// transparent, contributing nothing while keeping their bilinear weight, exactly
// as the shared edge policy's Transparent does. Mix 0 is the bit-exact identity.

struct Params {
    centre_normal: vec4<f32>,  // xy = centre (raster px), zw = (cos, sin) of Angle
    mix_amt: f32,              // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == cpu::bilinear_edge with the Transparent policy (edge == 0).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    if (x < 0 || x >= size.x || y < 0 || y >= size.y) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, vec2<i32>(x, y), 0);
}

fn bilinear_transparent(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
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
fn mirror(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let d = (px - p.centre_normal.x) * p.centre_normal.z
          + (py - p.centre_normal.y) * p.centre_normal.w;
    var sx = px;
    var sy = py;
    if (d > 0.0) {
        sx = px - 2.0 * d * p.centre_normal.z;
        sy = py - 2.0 * d * p.centre_normal.w;
    }
    let v = bilinear_transparent(sx, sy, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

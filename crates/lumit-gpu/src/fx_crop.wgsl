// Crop: takes the picture away past four edges, each a fraction of the frame
// from its own side, with a soft edge `band` pixels wide. Mirrors
// lumit_core::fx::effects::crop::crop and ::crop_keep op for op (the CPU is
// the oracle).
//
// Every edge at 0, or Mix 0, is the identity to the bit.

struct Params {
    edges: vec4<f32>,  // left, top, right, bottom, each 0..1 from its own side
    band: f32,         // the feather's width in raster px, floored above zero
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn crop(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let w = f32(size.x);
    let h = f32(size.y);
    let x = f32(xy.x) + 0.5;
    let y = f32(xy.y) + 0.5;
    let inside = min(min(x - p.edges.x * w, w - p.edges.z * w - x),
                     min(y - p.edges.y * h, h - p.edges.w * h - y));
    let keep = clamp(inside / p.band + 0.5, 0.0, 1.0);
    // `1 − mix·(1 − keep)`, so a kept pixel scales by exactly 1.
    textureStore(dst, xy, o * (1.0 - p.mix_amt * (1.0 - keep)));
}

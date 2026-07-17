// Chromatic aberration (docs/08-EFFECTS.md §3.6). Mirrors
// lumit_core::fx::cpu::rgb_split op-for-op (§1.6: the CPU is the oracle):
// R samples behind the offset, B ahead, G and alpha stay put. The linear
// offset vector arrives host-computed in the uniform (WGSL cos/sin are not
// correctly rounded, so the kernel never computes its own); radial mode
// derives each pixel's offset from the frame centre with IEEE-exact ops.

struct Params {
    dx: f32,        // linear-mode offset, raster px (host-computed)
    dy: f32,
    amount: f32,    // radial-mode peak offset, raster px
    radial: u32,    // 1 = offsets grow from the frame centre
    mix_amt: f32,   // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Clamp-addressed bilinear sample at continuous pixel-centre coordinates
// (== cpu::bilinear, same arithmetic order).
fn bilinear(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(
        src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c10 = textureLoad(
        src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c01 = textureLoad(
        src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let c11 = textureLoad(
        src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn rgb_split(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    var off = vec2<f32>(p.dx, p.dy);
    if (p.radial == 1u) {
        let fsize = vec2<f32>(size);
        let diag = sqrt(fsize.x * fsize.x + fsize.y * fsize.y);
        let k = p.amount / (0.5 * diag);
        off = vec2<f32>((pos.x - fsize.x * 0.5) * k, (pos.y - fsize.y * 0.5) * k);
    }
    let o = textureLoad(src, xy, 0);
    let r = bilinear(pos.x - off.x, pos.y - off.y, size).r;
    let b = bilinear(pos.x + off.x, pos.y + off.y, size).b;
    // Alpha follows the green channel (§3.6): both stay put.
    let split = vec4<f32>(r, o.g, b, o.a);
    textureStore(dst, xy, mix(o, split, p.mix_amt));
}

// Chromatic aberration (docs/08-EFFECTS.md §3.15). Mirrors
// lumit_core::fx::cpu::chromatic_aberration op-for-op (§1.6: the CPU is the
// oracle): a dedicated, always-radial sibling of RGB split's own Radial
// mode (fx_rgbsplit.wgsl) — R pulled outward, B pulled inward, G and alpha
// stay put, growing from the frame centre. No explicit Amount-0 short
// circuit is needed: `k` is an exact `0.0` at Amount 0, so every tap lands
// on its own pixel — the same un-guarded style fx_rgbsplit.wgsl uses.

struct Params {
    amount: f32,   // raster px: peak offset, reached at the corner distance
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Clamp-addressed bilinear sample at continuous pixel-centre coordinates
// (== cpu::bilinear / fx_rgbsplit.wgsl's own copy, same arithmetic order).
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
fn chromatic_aberration(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let fsize = vec2<f32>(size);
    let diag = sqrt(fsize.x * fsize.x + fsize.y * fsize.y);
    let k = p.amount / (0.5 * diag);
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let off = (pos - fsize * 0.5) * k;
    let r = bilinear(pos.x - off.x, pos.y - off.y, size).r;
    let b = bilinear(pos.x + off.x, pos.y + off.y, size).b;
    let split = vec4<f32>(r, o.g, b, o.a);
    textureStore(dst, xy, mix(o, split, p.mix_amt));
}

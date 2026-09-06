// Offset (docs/08-EFFECTS.md §3.40): the frame slid, wrapping round both axes.
// Mirrors lumit_core::fx::cpu::offset op-for-op (§1.6: the CPU is the oracle).
//
// The frame is a torus, so nothing is ever revealed and there is no edge policy
// to choose — which is why this kernel carries its own wrapping sampler rather
// than the shared three-way one. A zero shift and Mix 0 are both the bit-exact
// identity: a sample at the pixel's own centre is reproduced exactly.

struct Params {
    shift: vec2<f32>,   // raster pixels
    mix_amt: f32,       // 0..1, blended against the unprocessed input
    matte_on: f32,      // 1 = the matte scales the shift
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// == cpu::bilinear_wrap's `at`: WGSL's `%` on i32 keeps the sign of the dividend
// exactly as Rust's does, so the double fold lands in 0..len on both.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xw = ((x % size.x) + size.x) % size.x;
    let yw = ((y % size.y) + size.y) % size.y;
    return textureLoad(src, vec2<i32>(xw, yw), 0);
}

fn bilinear_wrap(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
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
fn offset(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // The matte scales the shift per pixel (== cpu::offset_matted).
    var k = 1.0;
    if (p.matte_on != 0.0) {
        k = matte_k(xy);
    }
    let v = bilinear_wrap(f32(xy.x) + 0.5 - p.shift.x * k,
                          f32(xy.y) + 0.5 - p.shift.y * k,
                          size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

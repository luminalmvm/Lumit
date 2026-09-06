// RGB split (docs/08-EFFECTS.md §3.6, T17). Mirrors
// lumit_core::fx::cpu::rgb_split op-for-op (§1.6: the CPU is the oracle):
// three tinted taps — taps 0/1 sample behind the offset, tap 2 ahead — each
// read in full colour, multiplied by its tint, and summed. The offset vector
// arrives host-computed in the uniform (WGSL cos/sin are not correctly
// rounded, so the kernel never computes its own). The always-radial variant
// is chromatic aberration.

struct Params {
    tints: array<vec4<f32>, 3>,  // per-tap tint (w unused)
    dx: f32,        // offset, raster px (host-computed)
    dy: f32,
    scale_r: f32,   // per-tap displacement scale (FX-9)
    scale_g: f32,
    scale_b: f32,
    mix_amt: f32,   // 0..1, blended against the unprocessed input
    matte_on: f32,  // 1 = the matte scales Amount
    _pad1: f32,
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
    // The matte scales Amount per pixel (== cpu::rgb_split_matted).
    if (p.matte_on != 0.0) {
        off = off * matte_k(xy);
    }
    let o = textureLoad(src, xy, 0);
    // Three tinted taps (T17): taps 0/1 along −offset·scale, tap 2 along
    // +offset·scale, each read in full colour then multiplied by its tint and
    // summed. Sampling at scale 0 lands on the pixel's own centre, matching
    // the CPU oracle's `bilinear` read. Alpha stays put (§3.6).
    let s0 = bilinear(pos.x - off.x * p.scale_r, pos.y - off.y * p.scale_r, size);
    let s1 = bilinear(pos.x - off.x * p.scale_g, pos.y - off.y * p.scale_g, size);
    let s2 = bilinear(pos.x + off.x * p.scale_b, pos.y + off.y * p.scale_b, size);
    let rgb = p.tints[0].rgb * s0.rgb + p.tints[1].rgb * s1.rgb + p.tints[2].rgb * s2.rgb;
    let split = vec4<f32>(rgb, o.a);
    textureStore(dst, xy, mix(o, split, p.mix_amt));
}

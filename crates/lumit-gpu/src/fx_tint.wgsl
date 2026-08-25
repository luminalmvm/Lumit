// Tint (docs/08-EFFECTS.md §3.24): a luminance duotone / gradient map —
// `out.rgb = black + (white - black) * luma(u)` per channel, with Rec.709 luma
// on the unpremultiplied colour (§2.2, the wrap fused into the kernel). Mirrors
// lumit_core::fx::cpu::tint op-for-op (§1.6: the CPU is the oracle). A
// luma-driven colour remap does not commute with premultiplied alpha: the pixel
// is unpremultiplied, mapped, then re-premultiplied -- exactly like Contrast and
// Gamma, so matte edges do not fringe. The lerp is written `black + (white -
// black) * luma` (not `mix()`, which reduces `black*(1-luma) + white*luma` in a
// different order) so the CPU and this kernel agree. The default black->black /
// white->white maps every pixel to its own luma (a greyscale); Mix 0 is the
// bit-exact identity. Purely continuous.

struct Params {
    black: vec4<f32>,  // rgb used, a ignored: the colour the darkest input maps to
    white: vec4<f32>,  // rgb used, a ignored: the colour the brightest input maps to
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Rec. 709 luma weights, in linear light (== cpu::LUMA).
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn tint(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    // Explicit left-to-right reduction to match the CPU oracle bit-for-bit.
    let luma = u.r * LUMA.r + u.g * LUMA.g + u.b * LUMA.b;
    let mapped = p.black.rgb + (p.white.rgb - p.black.rgb) * luma;
    let graded = mapped * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

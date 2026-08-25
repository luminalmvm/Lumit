// Invert (docs/08-EFFECTS.md §3.23): a simple colour inverse — `out.rgb =
// 1 - u` per channel, in the compositor's scene-linear working space, on
// unpremultiplied colour (§2.2, the wrap fused into the kernel). Mirrors
// lumit_core::fx::cpu::invert op-for-op (§1.6: the CPU is the oracle). `1 - c`
// is affine, so it does not commute with premultiplied alpha: the pixel is
// unpremultiplied, inverted, then re-premultiplied -- exactly like Contrast
// and Gamma, so matte edges do not fringe. HDR values above 1 invert to honest
// negatives, never clipped. There is no neutral short-circuit (invert always
// inverts); Mix 0 is the bit-exact identity. Purely continuous.

struct Params {
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn invert(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    let inverted = (vec3<f32>(1.0) - u) * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + inverted * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

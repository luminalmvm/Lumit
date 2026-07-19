// Sharpen (docs/08-EFFECTS.md §3.9, K-138): a plain 3x3 high-pass convolution
// scaled by Amount, on unpremultiplied colour (§2.2). Mirrors
// lumit_core::fx::cpu::sharpen_simple op-for-op (§1.6: the CPU is the oracle).
// out = u + amount*(4*u - up - down - left - right) per RGB channel, with the
// four axis neighbours clamp-addressed (so a border never invents dark
// detail); the result clamps >= 0, re-premultiplies by the centre alpha, and
// keeps alpha. Amount 0 short-circuits to the bit-exact original (matching the
// CPU's early return, whatever the Mix); Mix 0 is the identity.

struct Params {
    amount: f32,   // high-pass strength; 0 = passthrough, 1 = classic 5/-1 kernel
    radius: f32,   // neighbour distance in pixels (1 = 3x3 kernel)
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad1: f32,
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

// Unpremultiplied colour at a clamp-addressed integer neighbour.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec3<f32> {
    let xi = clamp(x, 0, size.x - 1);
    let yi = clamp(y, 0, size.y - 1);
    return unpremult(textureLoad(src, vec2<i32>(xi, yi), 0));
}

@compute @workgroup_size(8, 8)
fn sharpen_simple(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Amount 0 is the bit-exact passthrough (matches cpu::sharpen_simple's
    // early return), whatever the Mix.
    if (p.amount == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let c = unpremult(o);
    // Neighbour distance in pixels (T15), host-rounded on the CPU side too so
    // both sample the same taps; 1 = the classic 3x3 kernel.
    let r = max(i32(round(p.radius)), 1);
    let up = tap(xy.x, xy.y - r, size);
    let down = tap(xy.x, xy.y + r, size);
    let left = tap(xy.x - r, xy.y, size);
    let right = tap(xy.x + r, xy.y, size);
    let hp = 4.0 * c - up - down - left - right;
    let sharpened = max(c + p.amount * hp, vec3<f32>(0.0)) * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + sharpened * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

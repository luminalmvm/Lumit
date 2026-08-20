// Noise (docs/08-EFFECTS.md §3.36): per-pixel uniform or gaussian grain, mono
// or per channel. Mirrors lumit_core::fx::cpu::noise op-for-op (§1.6: the CPU
// is the oracle).
//
// A modifier, not a generator: it adds grain to the picture that arrived, on
// unpremultiplied colour (§2.2, the wrap fused into the kernel) and
// re-premultiplied on the way out. Nothing is clipped at either end (§2.1).
// `tick` arrives already discretised from layer time, so the kernel never sees
// a clock (§2.4). Amount 0 short-circuits to the input; Mix 0 likewise.

struct Params {
    amount: f32,      // Amount ÷ 100
    mix_amt: f32,     // 0..1, blended against the unprocessed input
    seed: u32,
    tick: i32,        // layer time discretised to the millisecond, 0 when frozen
    gaussian: u32,    // 0 uniform, 1 gaussian
    colour_noise: u32,// 0 mono, 1 per channel
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == lumit_core::fx::splitmix32.
fn splitmix32(xin: u32) -> u32 {
    var x = xin;
    x = x + 0x9e3779b9u;
    x = x ^ (x >> 16u);
    x = x * 0x21f0aaadu;
    x = x ^ (x >> 15u);
    x = x * 0x735a2d97u;
    x = x ^ (x >> 15u);
    return x;
}

// == lumit_core::fx::noise::hash01, same fold order.
fn hash01(channel: u32, x: i32, y: i32, z: i32) -> f32 {
    var h = p.seed;
    h = splitmix32(h ^ channel);
    h = splitmix32(h ^ bitcast<u32>(x));
    h = splitmix32(h ^ bitcast<u32>(y));
    h = splitmix32(h ^ bitcast<u32>(z));
    return f32(h >> 8u) / 16777216.0;
}

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::noise_draw: one uniform draw, or four averaged for the gaussian. The
// four are offset by 4 channels so a mono gaussian and a colour gaussian never
// share a draw.
fn noise_draw(channel: u32, x: i32, y: i32) -> f32 {
    let d0 = hash01(channel, x, y, p.tick) * 2.0 - 1.0;
    if (p.gaussian == 0u) {
        return d0;
    }
    let d1 = hash01(channel + 4u, x, y, p.tick) * 2.0 - 1.0;
    let d2 = hash01(channel + 8u, x, y, p.tick) * 2.0 - 1.0;
    let d3 = hash01(channel + 12u, x, y, p.tick) * 2.0 - 1.0;
    return (d0 + d1 + d2 + d3) * 0.5;
}

@compute @workgroup_size(8, 8)
fn noise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.amount == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    // Mono draws channel 0 for all three, which is what makes it read as
    // luminance noise rather than a tint.
    let c1 = select(0u, 1u, p.colour_noise != 0u);
    let c2 = select(0u, 2u, p.colour_noise != 0u);
    let n = vec3<f32>(
        noise_draw(0u, xy.x, xy.y),
        noise_draw(c1, xy.x, xy.y),
        noise_draw(c2, xy.x, xy.y),
    );
    let u = unpremult(o);
    let grained = (u + n * p.amount) * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + grained * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

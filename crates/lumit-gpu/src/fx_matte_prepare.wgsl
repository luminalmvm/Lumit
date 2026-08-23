// The matte's Channel pick and Invert, once at the seam (K-425, docs/08 §2.6).
//
// Every kernel that reads a matte reads its premultiplied Rec. 709 luma, and
// so does the generic dissolve. Rather than teach each of them which channel
// the user chose and whether to flip it, this pass rewrites the matte into a
// grey picture — R = G = B = the chosen channel, clamped to 0..1 and inverted
// if asked, alpha 1 — so everything downstream reads luma of that and gets the
// chosen channel back, and Invert is applied in exactly one place.
//
// The op-for-op twin of `lumit_core::fx::cpu::matte_prepare`: the same
// `channel_of` table (0 Luminance, 1 Alpha, 2 Red, 3 Green, 4 Blue), the same
// clamp-then-invert order. Never dispatched for Luminance with Invert off —
// the kernels already read exactly that, and a pass through an fp16 texture
// would requantise it (K-258). Shares the adjustment blend's bind-group
// layout: the matte is bound in all three sampled slots and only the first is
// read.

@group(0) @binding(0) var matte: texture_2d<f32>;
@group(0) @binding(1) var unused_b: texture_2d<f32>;
@group(0) @binding(2) var unused_c: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;

struct Params {
    // CHANNEL_OPTIONS index.
    channel: u32,
    // 1 = invert.
    invert: u32,
    _pad0: u32,
    _pad1: u32,
}
@group(0) @binding(4) var<uniform> params: Params;

// == lumit_core::fx::cpu::channel_of, the same weights in the same order.
fn channel_of(m: vec4<f32>, which: u32) -> f32 {
    switch which {
        case 1u: { return m.a; }
        case 2u: { return m.r; }
        case 3u: { return m.g; }
        case 4u: { return m.b; }
        default: { return 0.2126 * m.r + 0.7152 * m.g + 0.0722 * m.b; }
    }
}

@compute @workgroup_size(8, 8)
fn matte_prepare(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(matte);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let m = textureLoad(matte, p, 0);
    var k = clamp(channel_of(m, params.channel), 0.0, 1.0);
    if (params.invert != 0u) {
        k = 1.0 - k;
    }
    textureStore(dst, p, vec4<f32>(k, k, k, 1.0));
}

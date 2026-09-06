// The generic Matte strength semantic (docs/08 §2.6): every effect can
// be driven by a second picture whose luma says how much of the effect each
// pixel gets. The effect has already run into `processed`; this pass dissolves
// it back towards the picture it was given, by the matte's premultiplied
// Rec. 709 luma — after the effect's own Mix, which is inside its kernel.
//
// The op-for-op twin of `lumit_core::fx::cpu::matte_mix`: same weights, same
// clamp-then-invert order, and the lerp spelled the way WGSL defines `mix`, so
// a white matte is exactly the effect's output and a black one exactly its
// input on both paths. Shares the adjustment blend's bind-group layout — three
// sampled inputs, a storage output, one uniform — because the shapes are the
// same and a second identical layout is a second thing to keep in step.

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var processed: texture_2d<f32>;
@group(0) @binding(2) var matte: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;

struct Params {
    // 1 = invert the matte (the effect applies where the matte is dark).
    invert: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn matte_mix(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let a = textureLoad(input, p, 0);
    let b = textureLoad(processed, p, 0);
    let m = textureLoad(matte, p, 0);
    // Rec. 709 on the premultiplied colour, exactly as the CPU twin reads it.
    var k = clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
    if (params.invert != 0.0) {
        k = 1.0 - k;
    }
    textureStore(dst, p, a * (1.0 - k) + b * k);
}

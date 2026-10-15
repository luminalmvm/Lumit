// The effect Blend and Mix, once at the seam (docs/08 §1.5).
//
// Every effect's Mix row carries a Blend choice — the layer blend modes,
// verbatim — saying how the effect's result combines with its input. The
// kernel has already run into `processed` AT MIX 100 (the seam forces it, so
// the Mix is not applied twice); this pass blends that result onto `input`
// by the chosen mode and then applies the effect's own Mix, once:
//
//     out = input · (1 − mix) + blend(input, processed) · mix
//
// The op-for-op twin of `lumit_core::fx::cpu::blend_mix` / `blend_pixel`:
// the same mode table (BlendMode::ALL order), the same domains as the
// compositor's layer modes (docs/06 §blend domains — Add, Multiply, Lighten,
// Darken and Subtract per channel in linear; the rest encoded to sRGB, the
// W3C formula, decoded), alpha the effect's own. Never dispatched for Normal:
// an unset Blend row runs no pass and renders byte for byte what it did.
// Shares the adjustment blend's bind-group layout; the third sampled
// slot is bound but unread. The mode table itself is `fx_blend_core.wgsl`,
// prepended.

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var processed: texture_2d<f32>;
@group(0) @binding(2) var unused_c: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;

struct Params {
    // BlendMode::ALL index (never 0 here).
    mode: u32,
    // The effect's own Mix, 0..1.
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn blend_mix(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let d = textureLoad(input, p, 0);
    let s = textureLoad(processed, p, 0);
    let b = blend_pixel(params.mode, d, s);
    textureStore(dst, p, d * (1.0 - params.mix_amt) + b * params.mix_amt);
}

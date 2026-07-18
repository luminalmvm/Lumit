// Exposure (docs/08-EFFECTS.md §3.16). Mirrors lumit_core::fx::cpu::exposure
// op-for-op (§1.6: the CPU is the oracle): a single scene-linear gain on the
// RGB channels, alpha untouched. `factor` is 2^stops, computed host-side so
// the CPU and this kernel multiply by the identical number. factor == 1.0
// (0 stops) short-circuits to the input, the bit-exact neutral point.

struct Params {
    factor: f32,   // 2^stops linear gain
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn exposure(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.factor == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    let scaled = o.rgb * p.factor;
    let outv = o.rgb * (1.0 - p.mix_amt) + scaled * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

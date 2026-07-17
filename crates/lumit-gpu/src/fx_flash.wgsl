// Flash (docs/08-EFFECTS.md §3.7, manual form). Mirrors
// lumit_core::fx::cpu::flash op-for-op (§1.6: the CPU is the oracle): each
// pixel blends toward the flash colour by the host-evaluated strength (the
// trigger envelope × intensity, computed once per frame on the CPU). The
// colour is scaled by the pixel's own alpha so the flash respects the
// layer's footprint; alpha passes through untouched.

struct Params {
    colour: vec4<f32>,  // scene-linear; alpha unused
    strength: f32,      // 0..1
    mix_amt: f32,       // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn flash(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let lit = o.rgb * (1.0 - p.strength) + p.colour.rgb * o.a * p.strength;
    let outv = o.rgb * (1.0 - p.mix_amt) + lit * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

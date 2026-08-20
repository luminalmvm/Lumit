// Fill (docs/08-EFFECTS.md §3.34): flood the layer's own coverage with one
// colour. Mirrors lumit_core::fx::cpu::fill op-for-op (§1.6: the CPU is the
// oracle).
//
// The source colour is never read. `colour · a` IS the premultiplied form of
// "this colour at this coverage", so this works directly on premultiplied
// values (§2.2) with no unpremultiply round trip, and alpha passes through
// untouched. There is no neutral short-circuit and none is wanted; Mix 0 is the
// bit-exact identity.

struct Params {
    colour: vec4<f32>,  // scene-linear; the alpha lane is ignored
    mix_amt: f32,       // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn fill(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let filled = p.colour.rgb * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + filled * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

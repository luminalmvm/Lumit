// The adjustment-layer blend (docs/06 §1.5): an adjustment layer's effect
// stack has already produced `processed` from the accumulated composite
// `below`; this pass lerps between them by the layer's coverage — the
// comp-space mask raster's alpha times the layer opacity. A straight
// per-channel mix (alpha included) is the §1.5 attenuation law; routing it
// through the compositor's premultiplied-over would inflate alpha wherever
// the composite is semi-transparent, which is why this is its own kernel.

@group(0) @binding(0) var below: texture_2d<f32>;
@group(0) @binding(1) var processed: texture_2d<f32>;
@group(0) @binding(2) var coverage: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;

struct Params {
    // Layer opacity, already 0..1.
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn adjust_blend(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(below);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let b = textureLoad(below, p, 0);
    let f = textureLoad(processed, p, 0);
    let c = clamp(textureLoad(coverage, p, 0).a * params.opacity, 0.0, 1.0);
    textureStore(dst, p, mix(b, f, c));
}

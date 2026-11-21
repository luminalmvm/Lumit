// Flicker: this frame's gains on the premultiplied picture. Mirrors
// lumit_core::fx::effects::flicker op-for-op (the CPU is the oracle). The
// noise is read once a frame on the CPU, so none of it is here.

struct Params {
    gain: vec4<f32>,  // light times colour per channel; .w is the contrast
    rest: vec4<f32>,  // .x the opacity, .y the mix
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == FLICKER_PIVOT.
const PIVOT: f32 = 0.18;

@compute @workgroup_size(8, 8)
fn flicker(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let px = textureLoad(src, xy, 0);
    let grey = vec3<f32>(PIVOT * px.a);
    let rgb = max(grey + (px.rgb * p.gain.rgb - grey) * p.gain.w, vec3<f32>(0.0)) * p.rest.x;
    let out = vec4<f32>(rgb, px.a * p.rest.x);
    textureStore(dst, xy, px + p.rest.y * (out - px));
}

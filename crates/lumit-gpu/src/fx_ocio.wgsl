// The OCIO effects' kernel (docs/08-EFFECTS.md §3.97; docs/impl/ocio.md §6.6):
// one baked colour table applied to a layer in the middle of its stack.
//
// `ocio_sample.wgsl` is prepended to this file at pipeline creation and is
// where every line of colour maths lives - `ocio_apply` and what it reads.
// This file only declares the bindings that sampler expects, at the group the
// effect kernels use, and wraps it in the discipline every colour effect
// follows: straight colour in, straight colour out, then Mix. The alpha branch
// is `ocio_shade` in `colour.wgsl` line for line, so a layer put through an
// OCIO display transform effect and the same layer shown through the Viewer's
// view are one picture.

struct FxParams {
    mix: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> fxp: FxParams;
// The baked table, as `ocio_sample.wgsl` reads it.
@group(0) @binding(3) var curve: texture_2d<f32>;
@group(0) @binding(4) var cube: texture_3d<f32>;
@group(0) @binding(5) var<uniform> p: OcioParams;

@compute @workgroup_size(8, 8)
fn ocio_effect(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= dims.x || xy.y >= dims.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    var graded: vec3<f32>;
    if (o.a > 0.0 && o.a != 1.0) {
        graded = ocio_apply(o.rgb / o.a) * o.a;
    } else {
        graded = ocio_apply(o.rgb);
    }
    // `a + (b - a) * t`, the blend every Mix uses, so mix 0 is the input to
    // the bit and mix 1 is the table's answer to the bit.
    let outv = o.rgb + (graded - o.rgb) * fxp.mix;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

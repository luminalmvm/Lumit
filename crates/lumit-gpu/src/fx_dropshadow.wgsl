// Drop shadow (docs/08-EFFECTS.md §3.43), the combine pass: the layer's shape,
// already softened by the shared §3.8 gaussian, read at the shifted position,
// painted in one colour and composited UNDERNEATH the layer. Mirrors the second
// half of lumit_core::fx::cpu::drop_shadow op-for-op (§1.6: the CPU is the
// oracle).
//
// binding 0 is the sharp source (which doubles as the unprocessed original for
// Mix, this being one logical pass); binding 1 is the blurred copy. The blur and
// the offset commute, so the shape is softened where it stands and read where
// the shadow goes — one gaussian instead of a gaussian plus a resample.
//
// Mix 0 is the bit-exact identity, and so is Opacity 0.

struct Params {
    colour: vec4<f32>,     // scene-linear; the alpha lane is ignored
    offset: vec2<f32>,     // where the shadow sits relative to the shape, raster px
    opacity: f32,          // 0..1
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    shadow_only: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var soft: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == cpu::bilinear_edge with the Transparent policy (edge == 0): a shape
// touching the frame border casts a shadow that leaves the frame, and repeating
// the border pixel outward would smear it into a fan.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    if (x < 0 || x >= size.x || y < 0 || y >= size.y) {
        return vec4<f32>(0.0);
    }
    return textureLoad(soft, vec2<i32>(x, y), 0);
}

fn bilinear_transparent(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn drop_shadow(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let k = bilinear_transparent(f32(xy.x) + 0.5 - p.offset.x,
                                 f32(xy.y) + 0.5 - p.offset.y,
                                 size).a * p.opacity;
    let shadow = vec4<f32>(p.colour.rgb * k, k);
    // Source OVER shadow, premultiplied — the shadow is BELOW, which is the
    // whole reason this is an effect and not a duplicated layer.
    var over = o + shadow * (1.0 - o.a);
    if (p.shadow_only != 0u) {
        over = shadow;
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + over * p.mix_amt);
}

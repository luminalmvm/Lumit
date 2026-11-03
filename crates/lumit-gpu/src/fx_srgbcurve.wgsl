// sRGB curve: the picture onto the sRGB curve, or back off it, on
// unpremultiplied colour. Mirrors lumit_core::fx::effects::srgb_curve
// op for op. A value under zero is mirrored and one over one carries on along
// the curve, so the two directions undo each other.

struct Params {
    decode: f32,   // 1 = sRGB to linear, 0 = linear to sRGB
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

fn curve(v: f32) -> f32 {
    let x = abs(v);
    var y: f32;
    if (p.decode > 0.5) {
        if (x <= 0.04045) {
            y = x / 12.92;
        } else {
            y = pow((x + 0.055) / 1.055, 2.4);
        }
    } else {
        if (x <= 0.0031308) {
            y = x * 12.92;
        } else {
            y = 1.055 * pow(x, 1.0 / 2.4) - 0.055;
        }
    }
    return select(y, -y, v < 0.0);
}

@compute @workgroup_size(8, 8)
fn srgb_curve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    if (o.a <= 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let u = o.rgb / o.a;
    let curved = vec3<f32>(curve(u.r), curve(u.g), curve(u.b)) * o.a;
    textureStore(dst, xy, vec4<f32>(o.rgb * (1.0 - p.mix_amt) + curved * p.mix_amt, o.a));
}

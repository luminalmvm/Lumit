// A shadow caster's square (fx/lighting.rs). Mirrors
// lumit_core::lighting::caster_square: the layer's picture shrunk into its
// place in the middle of a square, its opacity applied, its colour display
// encoded for a comp lit that way, an empty margin round it, and then each
// smaller level averaged from the one before.

struct Params {
    // The picture's place, texels: corner, width and height.
    place: vec4<u32>,
    opacity: f32,
    encoded: u32,
    _a: u32,
    _b: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> q: Params;

fn texel(c: vec2<i32>) -> vec4<f32> {
    let n = vec2<i32>(textureDimensions(src));
    return textureLoad(src, clamp(c, vec2<i32>(0), n - 1), 0);
}

// Bilinear between texel centres, the edges held, as the oracle's.
fn bilinear(at: vec2<f32>) -> vec4<f32> {
    let x = at - 0.5;
    let b = floor(x);
    let f = x - b;
    let c = vec2<i32>(b);
    let top = mix(texel(c), texel(c + vec2<i32>(1, 0)), f.x);
    let bottom = mix(texel(c + vec2<i32>(0, 1)), texel(c + vec2<i32>(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

// The sRGB curve, carried through nought, as lumit_core::lighting::encode.
fn encode(x: vec3<f32>) -> vec3<f32> {
    let v = abs(x);
    let e = select(1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
    return sign(x) * e;
}

@compute @workgroup_size(8, 8)
fn square(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(dst);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    var out = vec4<f32>(0.0);
    let lo = q.place.xy;
    let hi = q.place.xy + q.place.zw;
    if (gid.x >= lo.x && gid.x < hi.x && gid.y >= lo.y && gid.y < hi.y) {
        let uv = vec2<f32>(f32(gid.x - lo.x) + 0.5, f32(gid.y - lo.y) + 0.5);
        let n = vec2<f32>(textureDimensions(src));
        var s = bilinear(uv / vec2<f32>(q.place.zw) * n) * q.opacity;
        if (q.encoded != 0u && s.a > 0.0) {
            s = vec4<f32>(encode(s.rgb / s.a) * s.a, s.a);
        }
        out = clamp(s, vec4<f32>(0.0), vec4<f32>(1.0));
    }
    textureStore(dst, vec2<i32>(gid.xy), out);
}

@compute @workgroup_size(8, 8)
fn halve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(dst);
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    let b = vec2<i32>(gid.xy) * 2;
    let sum = textureLoad(src, b, 0) + textureLoad(src, b + vec2<i32>(1, 0), 0)
        + textureLoad(src, b + vec2<i32>(0, 1), 0) + textureLoad(src, b + vec2<i32>(1, 1), 0);
    textureStore(dst, vec2<i32>(gid.xy), sum * 0.25);
}

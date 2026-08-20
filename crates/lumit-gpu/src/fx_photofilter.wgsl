// Photo filter (docs/08-EFFECTS.md §3.61): a coloured glass held in front of
// the lens. Mirrors lumit_core::fx::cpu::photo_filter op-for-op (§1.6: the CPU
// is the oracle).
//
// The filter colour arrives already decoded to scene-linear, so no transfer
// function lives in either kernel. Density 0 short-circuits, so it is the
// bit-exact identity on both paths.

struct Params {
    // Named "glass" and not "filter": the latter is a WGSL reserved word.
    glass: vec4<f32>,   // .rgb only; the filter colour in scene-linear
    density: f32,       // Density / 100; 0.0 = identity
    preserve: f32,      // 1 to restore the pixel own luma, 0 to let it cost light
    mix_amt: f32,       // 0..1, blended against the unprocessed input
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn photo_filter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // No glass (== the CPU reference early return).
    if (p.density == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let v = u + (u * p.glass.rgb - u) * p.density;
    let before = dot(u, LUMA);
    let after = dot(v, LUMA);
    // A filter dark enough to take the luma to nothing has nothing to restore.
    let gain = before / max(after, 1e-6);
    let k = 1.0 + (gain - 1.0) * p.preserve;
    let outv = o.rgb * (1.0 - p.mix_amt) + v * k * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

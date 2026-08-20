// Tritone (docs/08-EFFECTS.md §3.60): three colours mapped onto the tone range.
// Mirrors lumit_core::fx::cpu::tritone op-for-op (§1.6: the CPU is the oracle).
//
// The position is perceptual (§3.58's square root), so Midtones lands on the
// grey a person points at; anything past white keeps its headroom by scaling
// the chosen colour rather than clamping to it (§2.1).

struct Params {
    shadows: vec4<f32>,     // .rgb only; the alpha is unused
    midtones: vec4<f32>,
    highlights: vec4<f32>,
    mix_amt: f32,           // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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
fn tritone(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    let t = sqrt(max(dot(u, LUMA), 0.0));
    let s = min(t, 1.0);
    var lo = p.shadows.rgb;
    var hi = p.midtones.rgb;
    var x = s * 2.0;
    if (s >= 0.5) {
        lo = p.midtones.rgb;
        hi = p.highlights.rgb;
        x = s * 2.0 - 1.0;
    }
    // The `lo + (hi - lo) * x` form, as the CPU reference spells it.
    let v = (lo + (hi - lo) * x) * max(t, 1.0);
    let outv = o.rgb * (1.0 - p.mix_amt) + v * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

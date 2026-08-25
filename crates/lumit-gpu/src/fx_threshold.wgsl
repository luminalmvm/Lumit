// Threshold (docs/08-EFFECTS.md §3.59): every pixel to black or to white.
// Mirrors lumit_core::fx::cpu::threshold op-for-op (§1.6: the CPU is the
// oracle).
//
// The level is a position on the *perceptual* tone range (§3.58's square root),
// so 50 lands on mid-grey; the crossing is a smoothstep whose half-width is
// floored host-side, so the cut is antialiased and the two paths cannot
// disagree about a pixel sitting exactly on the line.

struct Params {
    level: f32,    // Level / 100
    hw: f32,       // half the crossing width, floored at 1/1000
    mix_amt: f32,  // 0..1, blended against the unprocessed input
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

// == cpu::smoothstep_between, written out rather than borrowed so the two paths
// cannot differ on the clamp or the polynomial.
fn smoothstep_between(lo: f32, hi: f32, x: f32) -> f32 {
    let t = clamp((x - lo) / (hi - lo), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(8, 8)
fn threshold(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    let t = sqrt(max(dot(u, LUMA), 0.0));
    let k = smoothstep_between(p.level - p.hw, p.level + p.hw, t);
    let outv = o.rgb * (1.0 - p.mix_amt) + vec3<f32>(k) * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

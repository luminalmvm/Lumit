// Vibrancy (docs/08-EFFECTS.md §3.10, K-152): a saturation boost weighted by
// each pixel's current colourfulness — low-saturation pixels lift more,
// already-vivid ones little, so skin tones and near-neutrals gain while
// saturated areas are protected from clipping (unlike Saturation's uniform
// scale). In linear light on unpremultiplied colour (§2.2, the wrap fused into
// the kernel). Mirrors lumit_core::fx::cpu::vibrance op-for-op (§1.6: the CPU
// is the oracle); amount 0 short-circuits the whole effect, so a neutral
// Vibrancy is the bit-exact identity.

struct Params {
    amount: f32,       // 0 = neutral; higher lifts less-saturated pixels more
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Rec. 709 luma weights, in linear light (== cpu::LUMA).
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn vibrance_fx(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.amount == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let luma = u.r * LUMA.r + u.g * LUMA.g + u.b * LUMA.b;
    // Scale-invariant HSV saturation in 0..1 (== the CPU branch).
    let mx = max(max(u.r, u.g), u.b);
    let mn = min(min(u.r, u.g), u.b);
    let sat = select(0.0, clamp((mx - mn) / mx, 0.0, 1.0), mx > 0.0);
    // More boost where sat is low; none where already saturated.
    let factor = 1.0 + p.amount * (1.0 - sat);
    let v = max(vec3<f32>(luma) + (u - vec3<f32>(luma)) * factor, vec3<f32>(0.0));
    let s = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + s * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

// Vignette (docs/08-EFFECTS.md §3.14). Mirrors lumit_core::fx::cpu::vignette
// op-for-op (§1.6: the CPU is the oracle): darkens toward black away from
// the frame centre, on premultiplied colour — alpha is never touched, this
// is a coverage-like darken, not a colour grade. Roundness blends the
// distance metric between a true circle (1: both axes normalised by the
// shorter side, so equal pixel distances read as equal) and an ellipse that
// exactly reaches the frame's own edges (0: each axis normalised by its own
// half-extent). Radius is the clear centre's reach in that normalised
// metric (1.0 = the metric's own reference edge) and Softness the feather
// beyond it, floored so Softness 0 reads as a hard edge rather than a
// division by zero. Amount 0 short-circuits to the input, matching the CPU
// reference's own neutral point.

struct Params {
    amount: f32,     // 0..1: darkening strength
    radius: f32,     // 0..1: the clear centre's reach
    softness: f32,   // 0..1: feather width beyond radius
    roundness: f32,  // 0..1: 1 = circular, 0 = follows the frame's aspect
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn vignette(@builtin(global_invocation_id) gid: vec3<u32>) {
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
    let fsize = vec2<f32>(size);
    let half = min(fsize.x, fsize.y) * 0.5;
    let rx = (fsize.x * 0.5) * (1.0 - p.roundness) + half * p.roundness;
    let ry = (fsize.y * 0.5) * (1.0 - p.roundness) + half * p.roundness;
    let centre = fsize * 0.5;
    let pos = vec2<f32>(xy) + vec2<f32>(0.5) - centre;
    let nx = pos.x / rx;
    let ny = pos.y / ry;
    let dist = sqrt(nx * nx + ny * ny);
    let edge0 = p.radius;
    let edge1 = p.radius + max(p.softness, 1e-6);
    let t = clamp((dist - edge0) / (edge1 - edge0), 0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    let vig = clamp(s * p.amount, 0.0, 1.0);
    let keep = 1.0 - vig;
    let darkened = o.rgb * keep;
    let outv = o.rgb * (1.0 - p.mix_amt) + darkened * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}

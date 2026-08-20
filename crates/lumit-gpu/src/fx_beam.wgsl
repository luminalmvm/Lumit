// Beam (docs/08-EFFECTS.md §3.73): a tapered shaft of light travelling between
// two points. Mirrors lumit_core::fx::cpu::beam and ::beam_sample op-for-op
// (§1.6: the CPU is the oracle).
//
// One capsule: the pixel is projected onto the drawn interval, clamped to its
// two ends, and the distance to that point decides both the colour and the
// coverage. Every reciprocal arrives floored from the host, so nothing here
// divides by a zero-length beam.
//
// `is_active` is the §3.73 short-circuit: with the head and the tail in the
// same place there is no segment, and Time 0 is the bit-exact identity because
// of it. It is spelled with the prefix because `active` is a WGSL reserved
// keyword — §3.69's `target` in another costume.
//
// Mix 0 and Time 0 are both the bit-exact identity.

struct Params {
    start_axis: vec4<f32>,  // start.xy, (end − start).xy, raster px
    inside: vec4<f32>,      // the core's colour; the alpha lane is ignored
    outside: vec4<f32>,     // the rim's colour
    inv_len2: f32,          // 1 ÷ |axis|², floored
    u0: f32,                // the tail, as a fraction of the axis
    u1: f32,                // the head
    inv_span: f32,          // 1 ÷ (u1 − u0), floored
    half0: f32,             // the half-thickness at the tail, raster px
    half1: f32,             // and at the head
    soft: f32,              // Softness ÷ 100, floored above zero
    mix_amt: f32,           // 0..1, blended against the unprocessed input
    is_active: f32,         // 0 when the drawn interval is empty
    composite: f32,         // 1 keeps the layer under the beam, 0 replaces it
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn beam(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    var colour = vec3<f32>(0.0);
    var cov = 0.0;
    if (p.is_active > 0.5) {
        let start = p.start_axis.xy;
        let axis = p.start_axis.zw;
        let r0 = vec2<f32>(f32(xy.x) + 0.5, f32(xy.y) + 0.5) - start;
        let s = clamp(dot(r0, axis) * p.inv_len2, p.u0, p.u1);
        let q = r0 - s * axis;
        let r = sqrt(dot(q, q));
        let f = (s - p.u0) * p.inv_span;
        let half_w = p.half0 + (p.half1 - p.half0) * f;
        // The crossover takes the rim's INNER HALF, so the outside colour is a
        // band rather than a hairline at the last antialiased pixel.
        let k = clamp((r / max(half_w, 1e-3) - (1.0 - p.soft)) / (p.soft * 0.5), 0.0, 1.0);
        cov = clamp(half_w + 0.5 - r, 0.0, 1.0);
        colour = (p.inside.rgb + (p.outside.rgb - p.inside.rgb) * k) * cov;
    }
    let keep = (1.0 - cov) * p.composite;
    let lit = vec4<f32>(o.rgb * keep + colour, o.a * keep + cov);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + lit * p.mix_amt);
}

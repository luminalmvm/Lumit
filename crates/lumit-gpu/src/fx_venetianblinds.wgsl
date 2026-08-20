// Venetian blinds (docs/08-EFFECTS.md §3.70): the frame closed by a rank of
// slats. Mirrors lumit_core::fx::cpu::venetian_blinds and ::venetian_blinds_keep
// op-for-op (§1.6: the CPU is the oracle).
//
// It is fx_linearwipe.wgsl with one line added — the distance across the frame
// is folded into a single slat before it is thresholded — and the same half-band
// lead-in at each end, so Completion 0 keeps the frame exactly and 100 removes
// it exactly.
//
// The fold uses floor(x + 0.5) and NOT round(): Rust rounds halves away from
// zero and WGSL rounds them to even, and one pixel landing on the wrong side of
// a slat is exactly what §1.6 exists to catch.
//
// The slats sit on the frame's own middle, which is a function of the raster and
// so is taken here rather than host-side (§3.46's precedent for the extent).
//
// Mix 0 and Completion 0 are both the bit-exact identity.

struct Params {
    normal: vec2<f32>,   // host-computed (sin θ, −cos θ)
    period: f32,         // one slat, raster px, floored at 1
    completion: f32,     // 0..1
    band: f32,           // the feather's width, raster px, floored above zero
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn venetian_blinds(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5 - f32(size.x) * 0.5;
    let py = f32(xy.y) + 0.5 - f32(size.y) * 0.5;
    let d = px * p.normal.x + py * p.normal.y;
    let u = d - p.period * floor(d / p.period + 0.5);
    // The removed half-slat, with a half-band lead-in at each end.
    let hw = p.completion * (p.period * 0.5 + p.band) - p.band * 0.5;
    let keep = clamp((abs(u) - hw) / p.band + 0.5, 0.0, 1.0);
    textureStore(dst, xy, o * (1.0 - p.mix_amt * (1.0 - keep)));
}

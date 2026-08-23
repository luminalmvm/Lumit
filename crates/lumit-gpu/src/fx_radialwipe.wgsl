// Radial wipe (docs/08-EFFECTS.md §3.47): a wedge swept round a centre.
// Mirrors lumit_core::fx::cpu::radial_wipe and ::radial_wipe_keep op-for-op
// (§1.6: the CPU is the oracle).
//
// One expression for all three sweep directions: `dir` only moves where the
// wedge's middle sits from Start angle (+1 clockwise, −1 anticlockwise, 0 for
// Both, which opens it symmetrically).
//
// The wrap into −π..π uses floor(x + ½) and NOT round(): Rust rounds halves
// away from zero and WGSL rounds them to even, and one pixel landing on the
// wrong side of the wedge is exactly what §1.6 exists to catch.
//
// One atan2 a pixel — §3.42's admission again (K-399): the angle IS a function
// of the pixel and cannot be lifted host-side, so the oracle is judged on
// absolute difference rather than in fp16 ULPs.
//
// Mix 0 and Completion 0 are both the bit-exact identity.

const PI: f32 = 3.14159265358979323846;
const TAU: f32 = 6.28318530717958647692;

struct Params {
    centre: vec2<f32>,   // raster px
    start: f32,          // radians, from straight up, clockwise
    dir: f32,            // +1 clockwise, −1 anticlockwise, 0 both
    completion: f32,     // 0..1
    feather: f32,        // the soft edge's width at the arc, raster px, floored above zero
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    matte_on: f32,       // 1 = the matte scales Completion per pixel (K-429)
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

@compute @workgroup_size(8, 8)
fn radial_wipe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let dx = f32(xy.x) + 0.5 - p.centre.x;
    let dy = f32(xy.y) + 0.5 - p.centre.y;
    // From straight up, clockwise, on a raster whose y grows downward.
    let phi = atan2(dy, dx) + PI * 0.5;
    let r = sqrt(dx * dx + dy * dy);
    // A constant-width soft edge: the angle a `feather`-wide band subtends at
    // this radius, clamped at π because near the centre it grows without bound.
    let band = clamp(p.feather / max(r, 1.0), 1e-4, PI);
    // The wedge's half-width, with a half-band lead-in at each end so
    // Completion 0 and 100 are the exact identity and the exact empty frame.
    // The matte pulls Completion toward 0 per pixel, before the edge is
    // placed (K-429): a gradient wipe, which a strength dissolve cannot make.
    var completion = p.completion;
    if (p.matte_on != 0.0) {
        completion = matte_toward(p.completion, 0.0, matte_k(xy));
    }
    let hw = completion * (PI + band) - band * 0.5;
    let mid = p.start + hw * p.dir;
    var delta = phi - mid;
    delta = delta - TAU * floor(delta / TAU + 0.5);
    let keep = clamp(0.5 - (hw - abs(delta)) / band, 0.0, 1.0);
    textureStore(dst, xy, o * (1.0 - p.mix_amt * (1.0 - keep)));
}

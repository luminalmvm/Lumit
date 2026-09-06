// Scanlines — standalone periodic darken (docs/08-EFFECTS.md §3.12, split
// out of the old combined Glitch effect: one of three now-separate
// one-thing effects, alongside Block glitch and Datamosh). Mirrors
// lumit_core::fx::cpu::scanlines op-for-op (§1.6: the CPU is the oracle).
// Pointwise — the output pixel needs only the same input pixel, no hash and
// no neighbour sample (`Roi::Exact`, tighter than Block glitch's
// full-frame).

struct Params {
    intensity: f32,  // 0..1: how dark the dark lines get (1 = black)
    period: f32,     // raster px: the scanline pitch
    roll_px: f32,    // the scanline pattern's pixel offset this frame
    interlace: u32,  // 1 = alternate which half darkens on odd periods
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    matte_on: f32,   // 1 = the matte widens Line period
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

@compute @workgroup_size(8, 8)
fn scanlines(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.intensity == 0.0) {
        textureStore(dst, xy, o);
        return;
    }

    var period = max(p.period, 1.0);
    // The matte widens Line period to period / k, floored at
    // cpu::SCANLINES_MIN_K so black is lines too far apart to see
    // (== cpu::scanlines_matted). Intensity is untouched.
    if (p.matte_on != 0.0) {
        period = period / max(matte_k(xy), 1e-4);
    }
    let yp = (f32(xy.y) + 0.5) + p.roll_px;
    let cell = yp / period;
    let cell_floor = floor(cell);
    let t = cell - cell_floor;
    // WGSL's % is truncated (can be negative); folding to {0,1} via abs
    // matches Rust's rem_euclid(2) for parity purposes exactly (==
    // cpu::scanlines's `(cell_floor as i64).rem_euclid(2) != 0`).
    let odd = abs(i32(cell_floor) % 2) != 0;
    let bright = (t < 0.5) != (p.interlace == 1u && odd);
    // The dark half's base is black (band 0), so eff_mult is 1 − intensity
    // there and 1 on the bright half.
    var band = 1.0;
    if (!bright) {
        band = 0.0;
    }
    let eff_mult = 1.0 - p.intensity * (1.0 - band);
    let darkened = vec4<f32>(o.r * eff_mult, o.g * eff_mult, o.b * eff_mult, o.a);

    textureStore(dst, xy, mix(o, darkened, p.mix_amt));
}

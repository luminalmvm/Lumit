// Scanlines — standalone periodic darken (docs/08-EFFECTS.md §3.12, split
// out of the old combined Glitch effect by K-107: one of three now-separate
// one-thing effects, alongside Block glitch and Datamosh). Mirrors
// lumit_core::fx::cpu::scanlines op-for-op (§1.6: the CPU is the oracle).
// Pointwise — the output pixel needs only the same input pixel, no hash and
// no neighbour sample (`Roi::Exact`, tighter than Block glitch's
// full-frame).

struct Params {
    intensity: f32,  // 0..1: the master dial, scales the darken strength
    period: f32,     // raster px: the scanline pitch
    darkness: f32,   // 0..1
    roll_px: f32,    // the scanline pattern's pixel offset this frame
    interlace: u32,  // 1 = alternate which half darkens on odd periods
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

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

    let period = max(p.period, 1.0);
    let yp = (f32(xy.y) + 0.5) + p.roll_px;
    let cell = yp / period;
    let cell_floor = floor(cell);
    let t = cell - cell_floor;
    // WGSL's % is truncated (can be negative); folding to {0,1} via abs
    // matches Rust's rem_euclid(2) for parity purposes exactly (==
    // cpu::scanlines's `(cell_floor as i64).rem_euclid(2) != 0`).
    let odd = abs(i32(cell_floor) % 2) != 0;
    let bright = (t < 0.5) != (p.interlace == 1u && odd);
    var band = 1.0;
    if (!bright) {
        band = 1.0 - p.darkness;
    }
    let eff_mult = 1.0 - p.intensity * (1.0 - band);
    let darkened = vec4<f32>(o.r * eff_mult, o.g * eff_mult, o.b * eff_mult, o.a);

    textureStore(dst, xy, mix(o, darkened, p.mix_amt));
}

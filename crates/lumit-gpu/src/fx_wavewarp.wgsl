// Wave warp (docs/08-EFFECTS.md §3.54): a travelling wave across the frame.
// Mirrors lumit_core::fx::cpu::wave_warp op-for-op (§1.6: the CPU is the
// oracle).
//
// The wave TRAVELS along Direction and the picture slides ACROSS it — the
// transverse wave, which is the one a flag makes. Both unit vectors are
// host-computed, so the kernel runs no trigonometry beyond the wave shape.
//
// The edges repeat rather than fading: an unpinned wave carries the picture off
// the frame, and a transparent edge would leave a hole where the crest was.
//
// Mix 0 and Wave height 0 are both the bit-exact identity.

struct Params {
    dir_perp: vec4<f32>,   // xy = the travel direction, zw = the slide direction
    pin: vec4<f32>,        // per edge: left, right, top, bottom
    height: f32,           // raster pixels, signed
    inv_width: f32,        // 1 / Wave width, raster pixels
    turns: f32,            // Phase / 360, in whole waves
    inv_pin_band: f32,     // 1 / |Wave height|
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    shape: u32,            // 0 Sine .. 8 Smooth noise, as cpu::wave_shape
    matte_on: f32,         // 1 = the matte scales Wave height
    _pad1: u32,
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

const TAU: f32 = 6.2831855;

// == lumit_core::fx::splitmix32.
fn splitmix32(xin: u32) -> u32 {
    var x = xin;
    x = x + 0x9e3779b9u;
    x = x ^ (x >> 16u);
    x = x * 0x21f0aaadu;
    x = x ^ (x >> 15u);
    x = x * 0x735a2d97u;
    x = x ^ (x >> 15u);
    return x;
}

// == cpu::wave_knot.
fn wave_knot(k: i32) -> f32 {
    return f32(splitmix32(bitcast<u32>(k)) >> 8u) / 8388608.0 - 1.0;
}

// == cpu::wave_shape: the nine shapes, each running -1..1 over one whole wave.
fn wave_shape(shape: u32, t: f32) -> f32 {
    let f = t - floor(t);
    if (shape == 1u) {
        return select(-1.0, 1.0, f < 0.5);
    }
    if (shape == 2u) {
        let q = t + 0.25;
        return 1.0 - 4.0 * abs((q - floor(q)) - 0.5);
    }
    if (shape == 3u) {
        return 2.0 * f - 1.0;
    }
    if (shape == 4u) {
        // Two half-circles a wave, the second one below the line.
        let b = 2.0 * f;
        let u = 2.0 * (b - floor(b)) - 1.0;
        let arc = sqrt(max(1.0 - u * u, 0.0));
        return select(-arc, arc, f < 0.5);
    }
    if (shape == 5u || shape == 6u) {
        // One arch a wave, all on one side. Uncircle is it upside down.
        let u = 2.0 * f - 1.0;
        let arch = 2.0 * sqrt(max(1.0 - u * u, 0.0)) - 1.0;
        return select(-arch, arch, shape == 5u);
    }
    if (shape == 7u || shape == 8u) {
        // A random height every half wave, joined straight or eased.
        let s = 2.0 * t;
        let k = floor(s);
        var g = s - k;
        if (shape == 8u) {
            g = g * g * (3.0 - 2.0 * g);
        }
        let a = wave_knot(i32(k));
        return a + (wave_knot(i32(k) + 1) - a) * g;
    }
    return sin(TAU * t);
}

// == cpu::bilinear_edge with the Repeat policy (edge == 1).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(src, vec2<i32>(clamp(x, 0, size.x - 1), clamp(y, 0, size.y - 1)), 0);
}

fn bilinear_repeat(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

fn ramp(d: f32) -> f32 {
    return clamp(d * p.inv_pin_band, 0.0, 1.0);
}

@compute @workgroup_size(8, 8)
fn wave_warp(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let fw = f32(size.x);
    let fh = f32(size.y);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let along = (px - fw * 0.5) * p.dir_perp.x + (py - fh * 0.5) * p.dir_perp.y;
    let wave = wave_shape(p.shape, along * p.inv_width - p.turns);
    // Each pinned edge ramps the WHOLE slide to zero across the last
    // |Wave height| pixels before it, measured to the outermost pixel centre so
    // the border row is exactly still. A lerp toward 1 rather than a branch, so
    // the four factors simply multiply.
    let pin = (1.0 + p.pin.x * (ramp(px - 0.5) - 1.0))
            * (1.0 + p.pin.y * (ramp(fw - 0.5 - px) - 1.0))
            * (1.0 + p.pin.z * (ramp(py - 0.5) - 1.0))
            * (1.0 + p.pin.w * (ramp(fh - 0.5 - py) - 1.0));
    // The matte scales Wave height per pixel (== cpu::wave_warp_matted).
    var height = p.height;
    if (p.matte_on != 0.0) {
        height = height * matte_k(xy);
    }
    let s = height * wave * pin;
    let v = bilinear_repeat(px + p.dir_perp.z * s, py + p.dir_perp.w * s, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}

// Luma fade: the picture goes away in order of brightness. Mirrors
// lumit_core::fx::effects::luma_fade::luma_fade op-for-op (the CPU is the oracle).
//
// The brightness is the matte's when one is bound, the pattern's when one is
// set, and the pixel's own otherwise, on the sRGB curve with the alpha taken
// out. Completion 0 and Mix 0 are both the bit-exact identity. The noise core
// is prepended.

struct Params {
    completion: f32,  // 0..1
    band: f32,        // softness as a share of 0..1, floored above zero
    mix_amt: f32,     // 0..1, blended against the unprocessed input
    invert: f32,      // 1 = the brightest parts go first
    matte_on: f32,    // 1 = read the brightness from the matte
    pattern: u32,     // 0 = the picture
    step: f32,        // pattern cells a pixel
    reach: f32,       // one over the centre-to-corner distance, pixels
    centre: vec2<f32>,
    // Always 0, no loop. Read from here rather than written in, so the
    // shader compiler can't fold the depth wrap into a divide by zero.
    cycle: i32,
    _pad: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Bound to `src` when there is no matte, since a binding can't be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// == luma_fade_pattern, for a pattern other than Picture.
fn pattern(xy: vec2<i32>) -> f32 {
    let dx = f32(xy.x) + 0.5 - p.centre.x;
    let dy = f32(xy.y) + 0.5 - p.centre.y;
    let u = dx * p.step;
    let v = dy * p.step;
    var l = 0.0;
    switch p.pattern {
        case 1u: {
            let n = 0.65 * nc_value3(0u, 0u, u, v, 0.0, p.cycle)
                + 0.35 * nc_value3(0u, 1u, u * 2.0, v * 2.0, 0.0, p.cycle);
            l = 0.5 + 0.9 * n;
        }
        case 2u: {
            l = nc_hash01(0u, 2u, i32(floor(u)), i32(floor(v)), 0);
        }
        case 3u: {
            l = abs(u - floor(u) - 0.5) + abs(v - floor(v) - 0.5);
        }
        case 4u: {
            l = u - floor(u);
        }
        default: {
            l = sqrt(dx * dx + dy * dy) * p.reach;
        }
    }
    return clamp(l, 0.0, 1.0);
}

@compute @workgroup_size(8, 8)
fn luma_fade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    var l = 0.0;
    if (p.matte_on == 0.0 && p.pattern != 0u) {
        l = pattern(xy);
    } else {
        var m = o;
        if (p.matte_on != 0.0) {
            m = textureLoad(matte, xy, 0);
        }
        // == luma_fade_brightness: the luma with the alpha taken back out, so
        // a soft edge goes with the rest of its shape, on the sRGB curve;
        // see-through reads as black.
        var y = m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722;
        if (m.a > 0.0) {
            y = y / m.a;
        } else {
            y = 0.0;
        }
        y = clamp(y, 0.0, 1.0);
        l = 12.92 * y;
        if (y > 0.0031308) {
            l = 1.055 * pow(y, 1.0 / 2.4) - 0.055;
        }
    }
    if (p.invert != 0.0) {
        l = 1.0 - l;
    }
    // The edge travels half a band past each end, so Completion 0 keeps
    // everything exactly and 100 removes everything exactly.
    let edge = -0.5 * p.band + p.completion * (1.0 + p.band);
    let keep = clamp((l - edge) / p.band + 0.5, 0.0, 1.0);
    textureStore(dst, xy, o * (1.0 - p.mix_amt * (1.0 - keep)));
}

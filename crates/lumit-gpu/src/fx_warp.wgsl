// Warp (docs/08-EFFECTS.md §3.56): the thirteen bend presets, one kernel.
// Mirrors lumit_core::fx::cpu::warp op-for-op (§1.6: the CPU is the oracle).
//
// The frame is normalised to -1..1 on each axis, the chosen style moves the
// sample there, the two perspective tapers act on the style's output, and the
// DIFFERENCE is carried back to pixels — which is what makes Bend 0 the
// bit-exact identity rather than a rebuilt coordinate a rounding away from one.
//
// Mix 0 and Bend 0 with both distortions 0 are the bit-exact identity.

struct Params {
    bend: f32,           // -1..1
    h_distort: f32,      // ±0.9
    v_distort: f32,      // ±0.9
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    style: u32,          // 0..12, docs/08 §3.56's table order
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const PI: f32 = 3.1415927;

// == cpu::bilinear_edge with the Transparent policy (edge == 0); the tap never
// loads out of bounds (docs/08 §3.50's note).
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let inside = x >= 0 && x < size.x && y >= 0 && y < size.y;
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(vec4<f32>(0.0, 0.0, 0.0, 0.0), textureLoad(src, c, 0), inside);
}

fn bilinear_transparent(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
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

// == cpu::warp_style. Each map is written so that a == 0 returns its argument
// untouched, which is what makes Bend 0 the identity for every style.
// `ar` is the frame's aspect ratio and is read by TWIST ALONE: a rotation has to
// happen in a space where both axes measure the same thing. Every other style is
// deliberately elliptical (docs/08 §3.56's second note).
fn warp_style(style: u32, u: f32, v: f32, a: f32, ar: f32) -> vec2<f32> {
    let d = 1.0 - u * u;
    let e = 1.0 - v * v;
    if (style == 1u) {
        return vec2<f32>(u, v + a * d * (1.0 - v) * 0.5);
    }
    if (style == 2u) {
        return vec2<f32>(u, v + a * d * (1.0 + v) * 0.5);
    }
    // The coefficient is SUBTRACTED in styles 3, 4, 7, 9 and 10: this is a
    // gather, so pulling the sample inward is what makes the picture swell
    // outward, and a positive Bend has to do what the style's name promises.
    if (style == 3u) {
        return vec2<f32>(u, v * (1.0 - a * d));
    }
    if (style == 4u) {
        return vec2<f32>(u * (1.0 - a * e * 0.5), v * (1.0 - a * d * 0.5));
    }
    if (style == 5u) {
        return vec2<f32>(u, v + a * 0.35 * sin(PI * u));
    }
    if (style == 6u) {
        return vec2<f32>(u, v - a * 0.35 * v * sin(PI * u));
    }
    if (style == 7u) {
        return vec2<f32>(u * (1.0 - a * e * 0.5), v);
    }
    if (style == 8u) {
        return vec2<f32>(u, v + a * (u + 1.0) * 0.5);
    }
    if (style == 9u) {
        let rho = min(sqrt(u * u + v * v), 1.0);
        let k = 1.0 - a * (1.0 - rho * rho) * 0.6;
        return vec2<f32>(u * k, v * k);
    }
    if (style == 10u) {
        let rho = min(sqrt(u * u + v * v), 1.0);
        let k = 1.0 - a * (1.0 - rho) * 0.6;
        return vec2<f32>(u * k, v * k);
    }
    if (style == 11u) {
        return vec2<f32>(u, v * (1.0 + a * e));
    }
    if (style == 12u) {
        let x = u * ar;
        let phi = a * PI * 0.5 * v;
        let sn = sin(phi);
        let cs = cos(phi);
        let rx = x * cs + v * sn;
        // The horizontal component is carried back as a DIFFERENCE, so that a
        // zero angle returns u to the bit rather than u*ar/ar.
        return vec2<f32>(u + (rx - x) / ar, -x * sn + v * cs);
    }
    return vec2<f32>(u, v + a * d);
}

@compute @workgroup_size(8, 8)
fn warp(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let half_w = f32(size.x) * 0.5;
    let half_h = f32(size.y) * 0.5;
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let u = px / half_w - 1.0;
    let v = py / half_h - 1.0;
    let m = warp_style(p.style, u, v, p.bend, half_w / half_h);
    // Both tapers are taken from the style's output, so neither feeds the other.
    let du = m.x / (1.0 + p.v_distort * m.y);
    let dv = m.y / (1.0 + p.h_distort * m.x);
    let sx = px + (du - u) * half_w;
    let sy = py + (dv - v) * half_h;
    let val = bilinear_transparent(sx, sy, size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + val * p.mix_amt);
}

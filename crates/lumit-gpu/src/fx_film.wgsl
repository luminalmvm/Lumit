// Film's passes (lumit_core::fx::film): exposure, the spreading passes'
// sums, the couplers, the grain's draw, development and the gate. Mirrors
// lumit_core::fx::film::cpu op for op; that is the oracle.
//
// fx_noise_core.wgsl is prepended at pipeline build, for the grain's hash.
//
// The tables (curves, grain, the two cubes) are one storage buffer laid out as
// lumit_core::fx::film::tables says, at the offsets the uniform carries.

struct Params {
    cam0: vec4<f32>,
    cam1: vec4<f32>,
    cam2: vec4<f32>,
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    inv2: vec4<f32>,
    cpl0: vec4<f32>,    // the couplers, donor by row
    cpl1: vec4<f32>,
    cpl2: vec4<f32>,
    out0: vec4<f32>,    // XYZ to linear Rec. 709
    out1: vec4<f32>,
    out2: vec4<f32>,
    top: vec4<f32>,
    grey: vec4<f32>,
    slope: vec4<f32>,
    print_gain: vec4<f32>,
    scan_low: vec4<f32>,
    scan_top: vec4<f32>,
    k0: vec4<f32>,      // film_acc: what orig is scaled by
    k1: vec4<f32>,      // then src, aux_a and aux_b, each a blur of orig's stage
    k2: vec4<f32>,
    k3: vec4<f32>,
    weave: vec4<f32>,   // x and y px, then the Mix
    gamma: f32,
    grain: f32,
    grain_colour: f32,
    print_gamma: f32,
    shown: u32,         // 0 print, 1 scan, 2 slide, 3 the picture's own
    flags: u32,         // 1 couplers, 2 a slide's silver, 4 grain
    seed: u32,
    tick: i32,
    at_curve: u32,
    at_curve0: u32,
    at_print_curve: u32,
    at_grain: u32,
    at_lut_print: u32,
    at_lut_scan: u32,
    curve_n: u32,
    grain_n: u32,
    lut_n: u32,
    dfloor: f32,        // the lowest density the print cube covers
    boost: f32,         // stops Clipped highlights lifts by
    _p2: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var aux_a: texture_2d<f32>;
@group(0) @binding(5) var aux_b: texture_2d<f32>;
@group(0) @binding(6) var<storage, read> tables: array<f32>;

// == tables::curve_at. Log exposure -3 to 4 across the samples.
fn curve_at(at: u32, c: u32, x: f32) -> f32 {
    let n = f32(p.curve_n - 1u);
    let t = clamp((x - -3.0) / 7.0 * n, 0.0, n);
    let i = min(u32(floor(t)), p.curve_n - 2u);
    let f = t - f32(i);
    let a = tables[at + i * 3u + c];
    return a + (tables[at + (i + 1u) * 3u + c] - a) * f;
}

// == tables::grain_at.
fn grain_at(c: u32, d: f32, top: f32) -> f32 {
    let n = f32(p.grain_n - 1u);
    let t = clamp(d / max(top, 1e-6) * n, 0.0, n);
    let i = min(u32(floor(t)), p.grain_n - 2u);
    let f = t - f32(i);
    let a = tables[p.at_grain + i * 3u + c];
    return a + (tables[p.at_grain + (i + 1u) * 3u + c] - a) * f;
}

fn cube_load(at: u32, r: u32, g: u32, b: u32, c: u32) -> f32 {
    return tables[at + ((b * p.lut_n + g) * p.lut_n + r) * 3u + c];
}

// == tables::cube_at: trilinear, red fastest.
fn cube_at(at: u32, low: vec3<f32>, top: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let n = f32(p.lut_n - 1u);
    var i = vec3<u32>(0u);
    var f = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let t = clamp((d[c] - low[c]) / max(top[c] - low[c], 1e-6) * n, 0.0, n);
        i[c] = min(u32(floor(t)), p.lut_n - 2u);
        f[c] = t - f32(i[c]);
    }
    var out = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let x00 = mix(cube_load(at, i.x, i.y, i.z, c), cube_load(at, i.x + 1u, i.y, i.z, c), f.x);
        let x10 = mix(cube_load(at, i.x, i.y + 1u, i.z, c), cube_load(at, i.x + 1u, i.y + 1u, i.z, c), f.x);
        let x01 = mix(cube_load(at, i.x, i.y, i.z + 1u, c), cube_load(at, i.x + 1u, i.y, i.z + 1u, c), f.x);
        let x11 = mix(cube_load(at, i.x, i.y + 1u, i.z + 1u, c), cube_load(at, i.x + 1u, i.y + 1u, i.z + 1u, c), f.x);
        out[c] = mix(mix(x00, x10, f.y), mix(x01, x11, f.y), f.z);
    }
    return out;
}

fn log_e(v: f32) -> f32 {
    return log(max(v, 0.0) + 1e-10) * 0.4342944819;
}

fn inside(xy: vec2<i32>) -> bool {
    let size = vec2<i32>(textureDimensions(src));
    return xy.x < size.x && xy.y < size.y;
}

// == cpu::lift.
fn lift(u: vec3<f32>) -> f32 {
    let m = max(max(u.x, u.y), u.z);
    if (p.boost <= 0.0 || m <= 0.75) {
        return 1.0;
    }
    let t = min((m - 0.75) / (1.0 - 0.75), 1.0);
    return exp2(p.boost * t * t);
}

// == cpu::in_gamut.
fn in_gamut(rgb: vec3<f32>) -> vec3<f32> {
    let low = min(min(rgb.x, rgb.y), rgb.z);
    if (low >= 0.0) {
        return rgb;
    }
    let luma = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
    if (luma <= 0.0) {
        return vec3<f32>(0.0);
    }
    let t = luma / (luma - low);
    return max(vec3<f32>(luma) + (rgb - vec3<f32>(luma)) * t, vec3<f32>(0.0));
}

// == cpu::expose.
@compute @workgroup_size(8, 8)
fn film_expose(@builtin(global_invocation_id) gid: vec3<u32>) {
    let xy = vec2<i32>(gid.xy);
    if (!inside(xy)) {
        return;
    }
    let s = textureLoad(src, xy, 0);
    var u = vec3<f32>(0.0);
    if (s.a > 0.0) {
        u = s.rgb / s.a;
    }
    u = u * lift(u);
    let e = vec3<f32>(
        p.cam0.x * u.x + p.cam0.y * u.y + p.cam0.z * u.z,
        p.cam1.x * u.x + p.cam1.y * u.y + p.cam1.z * u.z,
        p.cam2.x * u.x + p.cam2.y * u.y + p.cam2.z * u.z,
    );
    textureStore(dst, xy, vec4<f32>(e, s.a));
}

// Up to three terms of cpu::spread at once, added in its order, orig's alpha.
@compute @workgroup_size(8, 8)
fn film_acc(@builtin(global_invocation_id) gid: vec3<u32>) {
    let xy = vec2<i32>(gid.xy);
    if (!inside(xy)) {
        return;
    }
    let o = textureLoad(orig, xy, 0);
    var sum = o.rgb * p.k0.xyz + textureLoad(src, xy, 0).rgb * p.k1.xyz;
    sum += textureLoad(aux_a, xy, 0).rgb * p.k2.xyz;
    sum += textureLoad(aux_b, xy, 0).rgb * p.k3.xyz;
    textureStore(dst, xy, vec4<f32>(sum, o.a));
}

// == cpu::couple.
@compute @workgroup_size(8, 8)
fn film_couple(@builtin(global_invocation_id) gid: vec3<u32>) {
    let xy = vec2<i32>(gid.xy);
    if (!inside(xy)) {
        return;
    }
    let e = textureLoad(src, xy, 0);
    var silver = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let d = curve_at(p.at_curve, c, log_e(e[c]) * p.gamma);
        silver[c] = select(d, p.top[c] - d, (p.flags & 2u) != 0u);
    }
    let held = silver.x * p.cpl0.xyz + silver.y * p.cpl1.xyz + silver.z * p.cpl2.xyz;
    textureStore(dst, xy, vec4<f32>(held, 1.0));
}

// == cpu::normal.
fn normal(lane: u32, x: i32, y: i32) -> f32 {
    let u1 = max(nc_hash01(p.seed, lane * 2u, x, y, p.tick), 1e-7);
    let u2 = nc_hash01(p.seed, lane * 2u + 1u, x, y, p.tick);
    return sqrt(-2.0 * log(u1)) * cos(6.2831853 * u2);
}

// == cpu::grain_noise.
@compute @workgroup_size(8, 8)
fn film_noise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let xy = vec2<i32>(gid.xy);
    if (!inside(xy)) {
        return;
    }
    let own = sqrt(p.grain_colour);
    let mono = sqrt(1.0 - p.grain_colour);
    let both = normal(3u, xy.x, xy.y);
    let n = vec3<f32>(
        own * normal(0u, xy.x, xy.y) + mono * both,
        own * normal(1u, xy.x, xy.y) + mono * both,
        own * normal(2u, xy.x, xy.y) + mono * both,
    );
    textureStore(dst, xy, vec4<f32>(n, 1.0));
}

fn sigma(c: u32, d: f32) -> f32 {
    return sqrt(max(grain_at(c, d, p.top[c]), 0.0)) * p.grain;
}

fn to_rec709(m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        m0.x * v.x + m0.y * v.y + m0.z * v.z,
        m1.x * v.x + m1.y * v.y + m1.z * v.z,
        m2.x * v.x + m2.y * v.y + m2.z * v.z,
    );
}

// == cpu::pixel.
fn shown(e: vec3<f32>, corr: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let grain_on = (p.flags & 4u) != 0u;
    if (p.shown == 3u) {
        var out = e * 0.184;
        if (grain_on) {
            for (var c = 0u; c < 3u; c++) {
                let d = p.grey[c] + p.slope[c] * (log(max(e[c], 1e-10)) * 0.4342944819);
                out[c] *= pow(10.0, sigma(c, d) * n[c] / max(p.slope[c], 1e-3));
            }
        }
        return out;
    }
    var d = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let x = log_e(e[c]);
        if ((p.flags & 1u) != 0u) {
            d[c] = curve_at(p.at_curve0, c, (x - corr[c]) * p.gamma);
        } else {
            d[c] = curve_at(p.at_curve, c, x * p.gamma);
        }
        if (grain_on) {
            d[c] += sigma(c, d[c]) * n[c];
        }
    }
    var out = vec3<f32>(0.0);
    if (p.shown == 0u) {
        let lp = cube_at(p.at_lut_print, vec3<f32>(p.dfloor), p.top.xyz, d);
        var dp = vec3<f32>(0.0);
        for (var c = 0u; c < 3u; c++) {
            dp[c] = curve_at(p.at_print_curve, c, (lp[c] + p.print_gain[c]) * p.print_gamma);
        }
        let xyz = pow(vec3<f32>(10.0), cube_at(p.at_lut_scan, p.scan_low.xyz, p.scan_top.xyz, dp));
        out = to_rec709(p.out0, p.out1, p.out2, xyz);
    } else if (p.shown == 2u) {
        let xyz = pow(vec3<f32>(10.0), cube_at(p.at_lut_scan, p.scan_low.xyz, p.scan_top.xyz, d));
        out = to_rec709(p.out0, p.out1, p.out2, xyz);
    } else {
        var v = vec3<f32>(0.0);
        for (var c = 0u; c < 3u; c++) {
            v[c] = pow(10.0, (d[c] - p.grey[c]) / max(p.slope[c], 1e-3));
        }
        out = to_rec709(p.inv0, p.inv1, p.inv2, v);
    }
    return in_gamut(out);
}

// == cpu::develop. aux_a is the couplers' hold, aux_b the grain's draw; each
// bound to src and unread when its part is off.
@compute @workgroup_size(8, 8)
fn film_develop(@builtin(global_invocation_id) gid: vec3<u32>) {
    let xy = vec2<i32>(gid.xy);
    if (!inside(xy)) {
        return;
    }
    let e = textureLoad(src, xy, 0);
    var corr = vec3<f32>(0.0);
    if ((p.flags & 1u) != 0u) {
        corr = textureLoad(aux_a, xy, 0).rgb;
    }
    var n = vec3<f32>(0.0);
    if ((p.flags & 4u) != 0u) {
        n = textureLoad(aux_b, xy, 0).rgb;
    }
    textureStore(dst, xy, vec4<f32>(shown(e.rgb, corr, n) * e.a, e.a));
}

// == cpu::finish: the gate's place, edges held, then the Mix.
@compute @workgroup_size(8, 8)
fn film_finish(@builtin(global_invocation_id) gid: vec3<u32>) {
    let xy = vec2<i32>(gid.xy);
    if (!inside(xy)) {
        return;
    }
    let size = vec2<i32>(textureDimensions(src));
    var px = vec4<f32>(0.0);
    if (p.weave.x == 0.0 && p.weave.y == 0.0) {
        px = textureLoad(src, xy, 0);
    } else {
        let s = vec2<f32>(xy) - p.weave.xy;
        let f0 = floor(s);
        let f = s - f0;
        let i0 = vec2<i32>(f0);
        let hi = size - 1;
        let a = textureLoad(src, clamp(i0, vec2<i32>(0), hi), 0);
        let b = textureLoad(src, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0);
        let c = textureLoad(src, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0);
        let d = textureLoad(src, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0);
        let top = a + (b - a) * f.x;
        let bottom = c + (d - c) * f.x;
        px = top + (bottom - top) * f.y;
    }
    let o = textureLoad(orig, xy, 0);
    textureStore(dst, xy, o * (1.0 - p.weave.z) + px * p.weave.z);
}

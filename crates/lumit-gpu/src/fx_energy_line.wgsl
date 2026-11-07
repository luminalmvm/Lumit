// Energy line. Mirrors lumit_core::fx::effects::energy_line::draw op for op
// (the CPU is the oracle).
//
// THE PATH IS NOT BUILT HERE. It arrives as straight pieces in raster pixels,
// two vec4s each (the ends; then how far along the lit stretch each end sits,
// the shutter moment it belongs to and how present its stretch is), with
// every piece already filed into the 16 px tiles it can reach.
// A pixel measures only its own tile's pieces, so the cost follows how much
// path is near it rather than how long the path is.
//
// Two kernels: el_core writes the core's coverage and heat, which the shared
// exponential glow then spreads into a halo, and el_combine lays the light
// over the picture.

const TILE: u32 = 16u;
const MAX_SAMPLES: u32 = 16u;
// == draw::HALO_REACH and draw::HALO_FALL.
const HALO_REACH: f32 = 8.0;
const HALO_FALL: f32 = 1.2;

struct Params {
    core_colour: vec4<f32>,
    glow_colour: vec4<f32>,
    w: u32,
    h: u32,
    cols: u32,
    rows: u32,
    core_half: f32,
    taper0: f32,
    taper1: f32,
    softness: f32,
    core_warp: f32,
    glow_warp: f32,
    inv_size: f32,
    evolution: f32,
    drift_x: f32,
    drift_y: f32,
    glow_gain: f32,
    flicker: f32,
    composite: u32,
    mix_amt: f32,
    seed: u32,
    octaves: u32,
    gain: f32,
    lacunarity: f32,
    flags: u32,
    items_at: u32,
    // Always 0. From the uniform rather than a literal, since FXC rejects the
    // noise core's modulo by a constant zero.
    cycle: i32,
    samples: u32,
    halo: f32,
    _pad0: u32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> pieces: array<vec4<f32>>;
// The first cols * rows + 1 words are where each tile's list starts; the
// lists follow from `items_at`.
@group(0) @binding(2) var<storage, read> bins: array<u32>;
@group(0) @binding(3) var core_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var src: texture_2d<f32>;
@group(0) @binding(5) var core_in: texture_2d<f32>;
@group(0) @binding(6) var halo: texture_2d<f32>;
@group(0) @binding(7) var dst: texture_storage_2d<rgba16float, write>;

// == draw::WARP_TAPS.
fn tap(i: u32) -> vec2<f32> {
    switch i {
        case 1u: { return vec2<f32>(17.31, -9.73); }
        case 2u: { return vec2<f32>(41.2, 23.9); }
        case 3u: { return vec2<f32>(-28.6, 57.1); }
        default: { return vec2<f32>(0.0, 0.0); }
    }
}

// == draw::warp.
fn warp(x: f32, y: f32, amount: f32, pair: u32) -> vec2<f32> {
    if (amount <= 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    let f = FractalField(p.seed, p.octaves, p.gain, p.lacunarity, p.flags, p.cycle);
    let fx = x * p.inv_size + p.drift_x;
    let fy = y * p.inv_size + p.drift_y;
    let a = tap(pair);
    let b = tap(pair + 1u);
    let nx = clamp(nc_fractal(f, fx + a.x, fy + a.y, p.evolution), -1.0, 1.0) * amount;
    let ny = clamp(nc_fractal(f, fx + b.x, fy + b.y, p.evolution), -1.0, 1.0) * amount;
    return vec2<f32>(nx, ny);
}

// == draw::halo_at.
fn halo_at(out: f32, r: f32) -> f32 {
    let floor_v = exp(-HALO_REACH / HALO_FALL);
    return clamp((exp(-out / (r * HALO_FALL)) - floor_v) / (1.0 - floor_v), 0.0, 1.0);
}

// == draw::core_at.
@compute @workgroup_size(8, 8)
fn el_core(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let px = f32(gid.x) + 0.5;
    let py = f32(gid.y) + 0.5;
    let push = warp(px, py, p.core_warp, 0u);
    let q = vec2<f32>(px + push.x, py + push.y);
    var moments: array<vec3<f32>, 16>;
    for (var i = 0u; i < MAX_SAMPLES; i = i + 1u) {
        moments[i] = vec3<f32>(0.0);
    }
    let c = gid.x / TILE;
    let r = gid.y / TILE;
    if (c < p.cols && r < p.rows) {
        let tile = r * p.cols + c;
        let first = bins[tile];
        let last = bins[tile + 1u];
        for (var n = first; n < last; n = n + 1u) {
            let k = bins[p.items_at + n];
            let s = pieces[k * 2u];
            let u = pieces[k * 2u + 1u];
            let d = s.zw - s.xy;
            let rv = q - s.xy;
            let len2 = max(d.x * d.x + d.y * d.y, 1e-6);
            let t = clamp((rv.x * d.x + rv.y * d.y) / len2, 0.0, 1.0);
            let o = rv - t * d;
            let dist = sqrt(o.x * o.x + o.y * o.y);
            let along = u.x + (u.y - u.x) * t;
            let rad = p.core_half * (p.taper0 + (p.taper1 - p.taper0) * along);
            if (rad <= 0.0) {
                continue;
            }
            let band = max(p.softness * rad, 0.5);
            let cv = clamp((rad - dist) / band + 0.5, 0.0, 1.0) * u.w;
            let m = min(u32(u.z), MAX_SAMPLES - 1u);
            let was = moments[m];
            moments[m] = vec3<f32>(
                max(was.x, cv),
                max(was.y, clamp(1.0 - dist / rad, 0.0, 1.0) * cv),
                max(was.z, halo_at(max(dist - rad, 0.0), rad) * u.w),
            );
        }
    }
    let n = clamp(p.samples, 1u, MAX_SAMPLES);
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < n; i = i + 1u) {
        sum = sum + moments[i] / f32(n);
    }
    textureStore(core_out, vec2<i32>(gid.xy), vec4<f32>(sum, 0.0));
}

// == draw::sample.
fn sample(x: f32, y: f32) -> f32 {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let hi = vec2<i32>(i32(p.w) - 1, i32(p.h) - 1);
    let a = textureLoad(halo, clamp(vec2<i32>(i32(x0), i32(y0)), vec2<i32>(0), hi), 0).r;
    let b = textureLoad(halo, clamp(vec2<i32>(i32(x0 + 1.0), i32(y0)), vec2<i32>(0), hi), 0).r;
    let c = textureLoad(halo, clamp(vec2<i32>(i32(x0), i32(y0 + 1.0)), vec2<i32>(0), hi), 0).r;
    let d = textureLoad(halo, clamp(vec2<i32>(i32(x0 + 1.0), i32(y0 + 1.0)), vec2<i32>(0), hi), 0).r;
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    return top + (bottom - top) * ty;
}

// == draw::combine, after draw::render's glow push.
@compute @workgroup_size(8, 8)
fn el_combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
    let px = f32(gid.x) + 0.5;
    let py = f32(gid.y) + 0.5;
    var glow = 0.0;
    if (p.glow_gain > 0.0) {
        let push = warp(px, py, p.glow_warp, 2u);
        glow = sample(px + push.x, py + push.y);
    }
    let core = textureLoad(core_in, xy, 0);
    let s = textureLoad(src, xy, 0);
    let t = clamp(core.y * 1.6, 0.0, 1.0);
    let hot = t * t * (3.0 - 2.0 * t);
    let rim = p.glow_colour.rgb * (1.0 - hot) + p.core_colour.rgb * hot;
    let near = p.glow_colour.rgb * (max(glow, 0.0) * p.glow_gain + core.z * p.halo);
    let light = (core.x * rim + near) * p.flicker;
    let alpha = clamp(max(light.r, max(light.g, light.b)), 0.0, 1.0);
    // Straight in and out, as the compositor reads a layer (draw::combine).
    let before = vec4<f32>(s.rgb * s.a, s.a);
    var under = vec4<f32>(0.0);
    if (p.composite != 0u) {
        under = before;
    }
    let lit = vec4<f32>(under.rgb + light, under.a + alpha * (1.0 - under.a));
    let m = p.mix_amt;
    let o = before * (1.0 - m) + lit * m;
    if (o.a <= 0.0) {
        textureStore(dst, xy, vec4<f32>(0.0));
        return;
    }
    textureStore(dst, xy, vec4<f32>(o.rgb / o.a, o.a));
}

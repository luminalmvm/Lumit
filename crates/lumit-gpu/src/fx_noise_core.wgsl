// The procedural noise core: the WGSL twin of lumit_core::fx::noise
// (docs/08-EFFECTS.md §3.37, docs/impl/ae-effect-parity.md).
//
// This file is not a kernel. It is PREPENDED to every kernel that reads the
// noise field (fx_fractal_noise.wgsl draws it, fx_turbdisplace.wgsl steers by
// it), which is WGSL's only way of having a shared module — and it is worth the
// trick, because the alternative is two copies of a hash that must agree to the
// bit with each other and with Rust. Nothing here touches a uniform: every
// function takes what it needs, so the file compiles into any kernel's params.
//
// Every float step is written in one fixed arithmetic order, mirroring
// lumit_core::fx::noise op-for-op (§1.6: the CPU is the oracle), and the hash is
// integer u32 arithmetic that WGSL performs identically.

struct FractalField {
    seed: u32,
    octaves: u32,       // 1..10
    gain: f32,          // each octave's amplitude, as a share of the last
    lacunarity: f32,    // each octave's frequency, as a multiple of the last
    flags: u32,         // bit 0 Perlin, bit 1 Turbulent
    cycle: i32,         // depth loop length in cells; 0 = no loop
};

// == lumit_core::fx::splitmix32.
fn nc_splitmix32(xin: u32) -> u32 {
    var x = xin;
    x = x + 0x9e3779b9u;
    x = x ^ (x >> 16u);
    x = x * 0x21f0aaadu;
    x = x ^ (x >> 15u);
    x = x * 0x735a2d97u;
    x = x ^ (x >> 15u);
    return x;
}

// == lumit_core::fx::maths::lattice_hash.
fn nc_lattice_hash(seed: u32, channel: u32, x: i32, y: i32, z: i32) -> u32 {
    var h = seed;
    h = nc_splitmix32(h ^ channel);
    h = nc_splitmix32(h ^ bitcast<u32>(x));
    h = nc_splitmix32(h ^ bitcast<u32>(y));
    h = nc_splitmix32(h ^ bitcast<u32>(z));
    return h;
}

// == lumit_core::fx::noise::hash01.
fn nc_hash01(seed: u32, channel: u32, x: i32, y: i32, z: i32) -> f32 {
    return f32(nc_lattice_hash(seed, channel, x, y, z) >> 8u) / 16777216.0;
}

// == lumit_core::fx::noise::wrap_z. WGSL's `%` on i32 keeps the sign of the
// dividend exactly as Rust's does, so the double fold lands in 0..cycle on both.
fn nc_wrap_z(z: i32, cycle: i32) -> i32 {
    if (cycle <= 0) {
        return z;
    }
    return ((z % cycle) + cycle) % cycle;
}

fn nc_smooth3(t: f32) -> f32 {
    return t * t * (3.0 - 2.0 * t);
}

fn nc_fade5(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn nc_lerp1(a: f32, b: f32, t: f32) -> f32 {
    return a + (b - a) * t;
}

// == lumit_core::fx::noise::grad_dot: one of the twelve edge-midpoint gradients,
// computed from the index rather than looked up (index bits 0 and 1 are the two
// signs; the index over four says which axis is the zero one).
fn nc_grad_dot(idx: u32, dx: f32, dy: f32, dz: f32) -> f32 {
    var s0 = 1.0;
    if ((idx & 1u) != 0u) { s0 = -1.0; }
    var s1 = 1.0;
    if ((idx & 2u) != 0u) { s1 = -1.0; }
    let group = idx / 4u;
    if (group == 0u) {
        return s0 * dx + s1 * dy;
    }
    if (group == 1u) {
        return s0 * dx + s1 * dz;
    }
    return s0 * dy + s1 * dz;
}

// == lumit_core::fx::noise::value3.
fn nc_value3(seed: u32, channel: u32, x: f32, y: f32, z: f32, cycle: i32) -> f32 {
    let xf = floor(x);
    let yf = floor(y);
    let zf = floor(z);
    let ix = i32(xf);
    let iy = i32(yf);
    let iz = i32(zf);
    let u = nc_smooth3(x - xf);
    let v = nc_smooth3(y - yf);
    let w = nc_smooth3(z - zf);
    let z0 = nc_wrap_z(iz, cycle);
    let z1 = nc_wrap_z(iz + 1, cycle);
    let x00 = nc_lerp1(nc_hash01(seed, channel, ix, iy, z0),
                       nc_hash01(seed, channel, ix + 1, iy, z0), u);
    let x10 = nc_lerp1(nc_hash01(seed, channel, ix, iy + 1, z0),
                       nc_hash01(seed, channel, ix + 1, iy + 1, z0), u);
    let x01 = nc_lerp1(nc_hash01(seed, channel, ix, iy, z1),
                       nc_hash01(seed, channel, ix + 1, iy, z1), u);
    let x11 = nc_lerp1(nc_hash01(seed, channel, ix, iy + 1, z1),
                       nc_hash01(seed, channel, ix + 1, iy + 1, z1), u);
    let y0 = nc_lerp1(x00, x10, v);
    let y1 = nc_lerp1(x01, x11, v);
    return nc_lerp1(y0, y1, w) * 2.0 - 1.0;
}

// == lumit_core::fx::noise::perlin3. The normalisation is the literal the CPU
// reference multiplies by, not a computed reciprocal.
fn nc_perlin_corner(seed: u32, channel: u32, ix: i32, iy: i32, zz: i32,
                    dx: i32, dy: i32, dz: i32, fx: f32, fy: f32, fz: f32) -> f32 {
    let idx = nc_lattice_hash(seed, channel, ix + dx, iy + dy, zz) % 12u;
    return nc_grad_dot(idx, fx - f32(dx), fy - f32(dy), fz - f32(dz));
}

fn nc_perlin3(seed: u32, channel: u32, x: f32, y: f32, z: f32, cycle: i32) -> f32 {
    let xf = floor(x);
    let yf = floor(y);
    let zf = floor(z);
    let ix = i32(xf);
    let iy = i32(yf);
    let iz = i32(zf);
    let fx = x - xf;
    let fy = y - yf;
    let fz = z - zf;
    let u = nc_fade5(fx);
    let v = nc_fade5(fy);
    let w = nc_fade5(fz);
    let z0 = nc_wrap_z(iz, cycle);
    let z1 = nc_wrap_z(iz + 1, cycle);
    let x00 = nc_lerp1(nc_perlin_corner(seed, channel, ix, iy, z0, 0, 0, 0, fx, fy, fz),
                       nc_perlin_corner(seed, channel, ix, iy, z0, 1, 0, 0, fx, fy, fz), u);
    let x10 = nc_lerp1(nc_perlin_corner(seed, channel, ix, iy, z0, 0, 1, 0, fx, fy, fz),
                       nc_perlin_corner(seed, channel, ix, iy, z0, 1, 1, 0, fx, fy, fz), u);
    let x01 = nc_lerp1(nc_perlin_corner(seed, channel, ix, iy, z1, 0, 0, 1, fx, fy, fz),
                       nc_perlin_corner(seed, channel, ix, iy, z1, 1, 0, 1, fx, fy, fz), u);
    let x11 = nc_lerp1(nc_perlin_corner(seed, channel, ix, iy, z1, 0, 1, 1, fx, fy, fz),
                       nc_perlin_corner(seed, channel, ix, iy, z1, 1, 1, 1, fx, fy, fz), u);
    let y0 = nc_lerp1(x00, x10, v);
    let y1 = nc_lerp1(x01, x11, v);
    return nc_lerp1(y0, y1, w) * 1.1547005;
}

// == lumit_core::fx::noise::fractal. The depth coordinate is deliberately NOT
// scaled by frequency — every octave shares it and is decorrelated by its octave
// number entering the hash instead, which is what makes Cycle an exact loop at
// any Complexity (§3.37 decision 4).
fn nc_fractal(f: FractalField, x: f32, y: f32, z: f32) -> f32 {
    var amp = 1.0;
    var freq = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    let octaves = clamp(f.octaves, 1u, 10u);
    for (var o = 0u; o < octaves; o = o + 1u) {
        var n: f32;
        if ((f.flags & 1u) != 0u) {
            n = nc_perlin3(f.seed, o, x * freq, y * freq, z, f.cycle);
        } else {
            n = nc_value3(f.seed, o, x * freq, y * freq, z, f.cycle);
        }
        if ((f.flags & 2u) != 0u) {
            n = abs(n) * 2.0 - 1.0;
        }
        sum = sum + n * amp;
        norm = norm + amp;
        amp = amp * f.gain;
        freq = freq * f.lacunarity;
    }
    return sum / max(norm, 1e-6);
}

// ── Typed fractals: the WGSL twin of lumit_core::fx::noise::fractal_typed ───
//
// AE's fifteen fractal types over five noise bases. Only Fractal noise calls
// these; nc_fractal above is untouched for every other kernel. The value of an
// octave comes with its slope, worked out from the same corners, so the
// warping types cost no extra lattice lookups; the hash is folded one
// coordinate at a time with the shared prefixes reused, the same integers as
// nc_lattice_hash in fewer steps.

const NC_NOISE_VALUE: u32 = 0u;
const NC_NOISE_PERLIN: u32 = 1u;
const NC_NOISE_BLOCK: u32 = 2u;
const NC_NOISE_LINEAR: u32 = 3u;
const NC_NOISE_SPLINE: u32 = 4u;

const NC_BASIC: u32 = 0u;
const NC_TURBULENT_SMOOTH: u32 = 2u;
const NC_TURBULENT_SHARP: u32 = 3u;
const NC_DYNAMIC: u32 = 4u;
const NC_DYNAMIC_PROGRESSIVE: u32 = 5u;
const NC_DYNAMIC_TWIST: u32 = 6u;
const NC_MAX: u32 = 7u;
const NC_SMEARY: u32 = 8u;
const NC_SWIRLY: u32 = 9u;
const NC_ROCKY: u32 = 10u;
const NC_CLOUDY: u32 = 11u;
const NC_TERRAIN: u32 = 12u;
const NC_SUBSCALE: u32 = 13u;
const NC_STRINGS: u32 = 14u;

const NC_WARP_DYNAMIC: f32 = 0.12;
const NC_WARP_TWIST: f32 = 0.2;
const NC_WARP_SMEARY: f32 = 0.35;
const NC_WARP_SWIRLY: f32 = 0.3;

fn nc_warps(kind: u32) -> bool {
    return kind == NC_DYNAMIC || kind == NC_DYNAMIC_PROGRESSIVE || kind == NC_DYNAMIC_TWIST
        || kind == NC_SMEARY || kind == NC_SWIRLY;
}

fn nc_fold(h: u32, v: i32) -> u32 {
    return nc_splitmix32(h ^ bitcast<u32>(v));
}

fn nc_unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn nc_smooth_d(t: f32) -> f32 {
    return 6.0 * t * (1.0 - t);
}

fn nc_fade_d(t: f32) -> f32 {
    return 30.0 * t * t * (t * (t - 2.0) + 1.0);
}

fn nc_grad_xy(idx: u32) -> vec2<f32> {
    var s0 = 1.0;
    if ((idx & 1u) != 0u) { s0 = -1.0; }
    var s1 = 1.0;
    if ((idx & 2u) != 0u) { s1 = -1.0; }
    let group = idx / 4u;
    if (group == 0u) {
        return vec2<f32>(s0, s1);
    }
    if (group == 1u) {
        return vec2<f32>(s0, 0.0);
    }
    return vec2<f32>(0.0, s0);
}

fn nc_spline(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    return p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
}

fn nc_spline_d(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    return 0.5 * (p2 - p0)
        + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3)
        + 1.5 * t * t * (3.0 * (p1 - p2) + p3 - p0);
}

// == lumit_core::fx::noise::octave: (value, d/dx, d/dy).
fn nc_octave(base: u32, seed: u32, channel: u32, x: f32, y: f32, z: f32,
             cycle: i32, slope: bool) -> vec3<f32> {
    let xf = floor(x);
    let yf = floor(y);
    let zf = floor(z);
    let ix = i32(xf);
    let iy = i32(yf);
    let iz = i32(zf);
    let fx = x - xf;
    let fy = y - yf;
    let fz = z - zf;
    let z0 = nc_wrap_z(iz, cycle);
    let z1 = nc_wrap_z(iz + 1, cycle);
    let h0 = nc_splitmix32(seed ^ channel);
    if (base == NC_NOISE_SPLINE) {
        let w = nc_smooth3(fz);
        var hxy: array<array<u32, 4>, 4>;
        for (var i = 0; i < 4; i = i + 1) {
            let hx = nc_fold(h0, ix + i - 1);
            for (var j = 0; j < 4; j = j + 1) {
                hxy[j][i] = nc_fold(hx, iy + j - 1);
            }
        }
        var slice: array<vec3<f32>, 2>;
        for (var s = 0; s < 2; s = s + 1) {
            var zz = z0;
            if (s == 1) { zz = z1; }
            var rows: array<f32, 4>;
            var drows: array<f32, 4>;
            for (var j = 0; j < 4; j = j + 1) {
                let r0 = nc_unit(nc_fold(hxy[j][0], zz)) * 2.0 - 1.0;
                let r1 = nc_unit(nc_fold(hxy[j][1], zz)) * 2.0 - 1.0;
                let r2 = nc_unit(nc_fold(hxy[j][2], zz)) * 2.0 - 1.0;
                let r3 = nc_unit(nc_fold(hxy[j][3], zz)) * 2.0 - 1.0;
                rows[j] = nc_spline(r0, r1, r2, r3, fx);
                if (slope) {
                    drows[j] = nc_spline_d(r0, r1, r2, r3, fx);
                } else {
                    drows[j] = 0.0;
                }
            }
            let v = nc_spline(rows[0], rows[1], rows[2], rows[3], fy);
            var dx = 0.0;
            var dy = 0.0;
            if (slope) {
                dx = nc_spline(drows[0], drows[1], drows[2], drows[3], fy);
                dy = nc_spline_d(rows[0], rows[1], rows[2], rows[3], fy);
            }
            slice[s] = vec3<f32>(v, dx, dy);
        }
        return vec3<f32>(nc_lerp1(slice[0].x, slice[1].x, w),
                         nc_lerp1(slice[0].y, slice[1].y, w),
                         nc_lerp1(slice[0].z, slice[1].z, w));
    }
    let hx0 = nc_fold(h0, ix);
    let hx1 = nc_fold(h0, ix + 1);
    let h00 = nc_fold(hx0, iy);
    let h01 = nc_fold(hx0, iy + 1);
    let h10 = nc_fold(hx1, iy);
    let h11 = nc_fold(hx1, iy + 1);
    let c000 = nc_fold(h00, z0);
    let c001 = nc_fold(h00, z1);
    let c010 = nc_fold(h01, z0);
    let c011 = nc_fold(h01, z1);
    let c100 = nc_fold(h10, z0);
    let c101 = nc_fold(h10, z1);
    let c110 = nc_fold(h11, z0);
    let c111 = nc_fold(h11, z1);
    if (base == NC_NOISE_PERLIN) {
        let u = nc_fade5(fx);
        let v = nc_fade5(fy);
        let w = nc_fade5(fz);
        let i000 = c000 % 12u;
        let i100 = c100 % 12u;
        let i010 = c010 % 12u;
        let i110 = c110 % 12u;
        let i001 = c001 % 12u;
        let i101 = c101 % 12u;
        let i011 = c011 % 12u;
        let i111 = c111 % 12u;
        let a000 = nc_grad_dot(i000, fx, fy, fz);
        let a100 = nc_grad_dot(i100, fx - 1.0, fy, fz);
        let a010 = nc_grad_dot(i010, fx, fy - 1.0, fz);
        let a110 = nc_grad_dot(i110, fx - 1.0, fy - 1.0, fz);
        let a001 = nc_grad_dot(i001, fx, fy, fz - 1.0);
        let a101 = nc_grad_dot(i101, fx - 1.0, fy, fz - 1.0);
        let a011 = nc_grad_dot(i011, fx, fy - 1.0, fz - 1.0);
        let a111 = nc_grad_dot(i111, fx - 1.0, fy - 1.0, fz - 1.0);
        let x00 = nc_lerp1(a000, a100, u);
        let x10 = nc_lerp1(a010, a110, u);
        let x01 = nc_lerp1(a001, a101, u);
        let x11 = nc_lerp1(a011, a111, u);
        let y0 = nc_lerp1(x00, x10, v);
        let y1 = nc_lerp1(x01, x11, v);
        let n = nc_lerp1(y0, y1, w) * 1.1547005;
        if (!slope) {
            return vec3<f32>(n, 0.0, 0.0);
        }
        let g000 = nc_grad_xy(i000);
        let g100 = nc_grad_xy(i100);
        let g010 = nc_grad_xy(i010);
        let g110 = nc_grad_xy(i110);
        let g001 = nc_grad_xy(i001);
        let g101 = nc_grad_xy(i101);
        let g011 = nc_grad_xy(i011);
        let g111 = nc_grad_xy(i111);
        let gx = nc_lerp1(
            nc_lerp1(nc_lerp1(g000.x, g100.x, u), nc_lerp1(g010.x, g110.x, u), v),
            nc_lerp1(nc_lerp1(g001.x, g101.x, u), nc_lerp1(g011.x, g111.x, u), v),
            w);
        let gy = nc_lerp1(
            nc_lerp1(nc_lerp1(g000.y, g100.y, u), nc_lerp1(g010.y, g110.y, u), v),
            nc_lerp1(nc_lerp1(g001.y, g101.y, u), nc_lerp1(g011.y, g111.y, u), v),
            w);
        let du = nc_fade_d(fx);
        let dv = nc_fade_d(fy);
        let ex = nc_lerp1(nc_lerp1(a100 - a000, a110 - a010, v),
                          nc_lerp1(a101 - a001, a111 - a011, v), w);
        let ey = nc_lerp1(x10 - x00, x11 - x01, w);
        return vec3<f32>(n, (gx + du * ex) * 1.1547005, (gy + dv * ey) * 1.1547005);
    }
    let w = nc_smooth3(fz);
    var u = nc_smooth3(fx);
    var v = nc_smooth3(fy);
    if (base == NC_NOISE_BLOCK) {
        u = 0.0;
        v = 0.0;
    } else if (base == NC_NOISE_LINEAR) {
        u = fx;
        v = fy;
    }
    let v000 = nc_unit(c000);
    let v100 = nc_unit(c100);
    let v010 = nc_unit(c010);
    let v110 = nc_unit(c110);
    let v001 = nc_unit(c001);
    let v101 = nc_unit(c101);
    let v011 = nc_unit(c011);
    let v111 = nc_unit(c111);
    let x00 = nc_lerp1(v000, v100, u);
    let x10 = nc_lerp1(v010, v110, u);
    let x01 = nc_lerp1(v001, v101, u);
    let x11 = nc_lerp1(v011, v111, u);
    let y0 = nc_lerp1(x00, x10, v);
    let y1 = nc_lerp1(x01, x11, v);
    let n = nc_lerp1(y0, y1, w) * 2.0 - 1.0;
    if (!slope) {
        return vec3<f32>(n, 0.0, 0.0);
    }
    // Block and Linear take the smoothstep's slope of the same corners.
    let su = nc_smooth3(fx);
    let sv = nc_smooth3(fy);
    let du = nc_smooth_d(fx);
    let dv = nc_smooth_d(fy);
    let ex = nc_lerp1(nc_lerp1(v100 - v000, v110 - v010, sv),
                      nc_lerp1(v101 - v001, v111 - v011, sv), w);
    let ey = nc_lerp1(nc_lerp1(v010, v110, su) - nc_lerp1(v000, v100, su),
                      nc_lerp1(v011, v111, su) - nc_lerp1(v001, v101, su), w);
    return vec3<f32>(n, 2.0 * du * ex, 2.0 * dv * ey);
}

// == lumit_core::fx::noise::fractal_typed.
fn nc_fractal_typed(f: FractalField, base: u32, kind: u32, x: f32, y: f32, z: f32) -> f32 {
    var amp = 1.0;
    var freq = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    var peak = 0.0;
    var weight = 1.0;
    var wx = 0.0;
    var wy = 0.0;
    let warping = nc_warps(kind);
    let octaves = clamp(f.octaves, 1u, 10u);
    for (var o = 0u; o < octaves; o = o + 1u) {
        var sx = x * freq;
        var sy = y * freq;
        if (warping) {
            sx = (x + wx) * freq;
            sy = (y + wy) * freq;
        }
        // == OCTAVE_SHIFT: the octaves' lattices must not meet at the origin.
        sx = sx + f32(o) * 0.7548777;
        sy = sy + f32(o) * 0.5698403;
        var zz = z;
        var cycle = f.cycle;
        if (kind == NC_DYNAMIC_PROGRESSIVE) {
            let m = i32(o / 2u + 1u);
            zz = z * f32(m);
            cycle = f.cycle * m;
        }
        let ng = nc_octave(base, f.seed, o, sx, sy, zz, cycle, warping);
        let n = ng.x;
        let t = abs(n);
        var shaped = t * 2.0 - 1.0;
        if (kind == NC_BASIC || kind == NC_DYNAMIC_TWIST) {
            shaped = n;
        } else if (kind == NC_TURBULENT_SMOOTH) {
            shaped = t * t * 2.0 - 1.0;
        } else if (kind == NC_DYNAMIC || kind == NC_DYNAMIC_PROGRESSIVE || kind == NC_SMEARY) {
            let c = max(1.0 - t, 0.0);
            shaped = (1.0 - c * c) * 2.0 - 1.0;
        } else if (kind == NC_TURBULENT_SHARP) {
            let c = max(1.0 - t, 0.0);
            shaped = (1.0 - c * c * c) * 2.0 - 1.0;
        } else if (kind == NC_ROCKY) {
            shaped = clamp(n * 2.5, -1.0, 1.0);
        } else if (kind == NC_CLOUDY) {
            let c = clamp(0.5 - n * 0.5, 0.0, 1.0);
            shaped = (1.0 - c * c) * 2.0 - 1.0;
        } else if (kind == NC_TERRAIN) {
            let r0 = max(1.0 - t, 0.0);
            let r = r0 * r0 * weight;
            weight = clamp(r * 2.0, 0.0, 1.0);
            shaped = r * 2.0 - 1.0;
        } else if (kind == NC_SUBSCALE) {
            var lift = 1.0;
            if (norm > 0.0) {
                lift = (sum / norm) * 0.5 + 0.5;
            }
            shaped = n * (0.4 + 1.2 * lift);
        } else if (kind == NC_STRINGS) {
            let r = max(1.0 - t, 0.0);
            let r2 = r * r;
            shaped = r2 * r2 * 2.0 - 1.0;
        }
        var a = amp;
        if (kind == NC_STRINGS) {
            a = amp * amp;
        }
        if (kind == NC_MAX) {
            peak = max(peak, t * a);
        } else {
            sum = sum + shaped * a;
        }
        norm = norm + a;
        if (warping) {
            let s = amp / freq;
            let gx = ng.y;
            let gy = ng.z;
            if (kind == NC_DYNAMIC_TWIST) {
                wx = wx - NC_WARP_TWIST * gy * s;
                wy = wy + NC_WARP_TWIST * gx * s;
            } else if (kind == NC_SWIRLY) {
                wx = wx - NC_WARP_SWIRLY * gy * s;
                wy = wy + NC_WARP_SWIRLY * gx * s;
            } else if (kind == NC_SMEARY) {
                if (o == 0u) {
                    wx = wx + NC_WARP_SMEARY * gx;
                    wy = wy + NC_WARP_SMEARY * gy;
                }
            } else {
                wx = wx + NC_WARP_DYNAMIC * gx * s;
                wy = wy + NC_WARP_DYNAMIC * gy * s;
            }
        }
        amp = amp * f.gain;
        freq = freq * f.lacunarity;
    }
    if (kind == NC_MAX) {
        return peak * 2.0 - 1.0;
    }
    return sum / max(norm, 1e-6);
}

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

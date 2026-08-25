//! The procedural noise core (docs/08 §3.37): seeded 3-D value and Perlin
//! noise, and the fractal sum over them.
//!
//! **In plain terms.** "Noise" here does not mean random — it means a
//! *repeatable* pattern of hills and valleys that looks random. Ask this module
//! for the height at a point and it always answers the same number for the same
//! seed and the same point, on any machine, in any run. That is what lets a
//! noise texture be a picture at all: a genuinely random one would flicker into
//! a different picture every time it was drawn.
//!
//! It is built in three layers, each standing on the one below:
//!
//! 1. **A hash.** Every point on an integer grid ("the lattice") gets a number
//!    stirred out of its coordinates and the seed. This is the only randomness
//!    there is; everything above is smooth interpolation.
//! 2. **One octave.** Between the grid points the value is blended smoothly —
//!    either from the corner *values* (Value noise: cheap, slightly blocky) or
//!    from corner *slopes* (Perlin noise: dearer, and the shape everyone means
//!    when they say "clouds").
//! 3. **The fractal sum.** Several octaves are added, each one smaller and
//!    fainter than the last. That is what makes a noise field look like a real
//!    surface, which has both large shapes and fine detail.
//!
//! **Why it lives here rather than inside the Fractal noise effect.** The
//! displacement family (Turbulent displace, docs/impl/ae-effect-parity.md) drives
//! its warp with exactly this field. One implementation, one WGSL twin, one
//! oracle — or two effects that agree until the day they quietly stop.
//!
//! **Determinism** (docs/08 §2.4) is the whole contract. The hash is integer
//! `u32` arithmetic that WGSL performs identically; every float step below is
//! written in one fixed arithmetic order and mirrored op-for-op in
//! `fx_fractal_noise.wgsl`, and nothing here reads a clock, a thread id or a
//! platform trig function.

use super::maths::lattice_hash;

/// One of the twelve Perlin gradients dotted with `(dx, dy, dz)`.
///
/// The gradients are the midpoints of a cube's twelve edges — Ken Perlin's
/// "improved noise" set — and they are *computed from the index* rather than
/// looked up in a table: index bits 0 and 1 are the two signs, and the index
/// divided by four says which axis is the zero one. Written this way for one
/// reason, and it is the reason that matters here: WGSL's rules for indexing a
/// constant array with a runtime value vary by backend, and a table that has to
/// become a private array on one driver and a switch on another is a table the
/// §1.6 oracle cannot promise agrees with this one. Two selects and two
/// multiplies are the same maths on every path.
fn grad_dot(idx: u32, dx: f32, dy: f32, dz: f32) -> f32 {
    let s0 = if idx & 1 == 0 { 1.0 } else { -1.0 };
    let s1 = if idx & 2 == 0 { 1.0 } else { -1.0 };
    match idx / 4 {
        0 => s0 * dx + s1 * dy,
        1 => s0 * dx + s1 * dz,
        _ => s0 * dy + s1 * dz,
    }
}

/// Perlin's own normalisation: with the edge-midpoint gradients the raw noise
/// reaches about `±0.866` at its extremes, so this reciprocal brings a
/// well-exercised field out to roughly `±1`. Written as a literal (not
/// `1.0 / 0.866…`) so the CPU and the kernel multiply by the identical constant.
const PERLIN_NORM: f32 = 1.154_700_5;

/// One lattice value in `[0, 1)` — the top 24 bits of the fold, which are
/// exactly representable in `f32`.
#[must_use]
pub fn hash01(seed: u32, channel: u32, x: i32, y: i32, z: i32) -> f32 {
    (lattice_hash(seed, channel, x, y, z) >> 8) as f32 / 16_777_216.0
}

/// The lattice depth index, wrapped when the field is cycling.
///
/// `cycle` is the loop length in whole cells; `0` means "do not loop" and the
/// index passes through. Wrapping the *index* rather than the coordinate is what
/// makes a cycle seamless: cell `cycle − 1` interpolates towards cell `0`, which
/// is the same lattice row the loop started on.
const fn wrap_z(z: i32, cycle: i32) -> i32 {
    if cycle <= 0 {
        z
    } else {
        // Rust's `%` keeps the sign of the dividend; a negative depth must still
        // land in 0..cycle.
        ((z % cycle) + cycle) % cycle
    }
}

/// The C¹ smoothstep the value lattice interpolates with (`3t² − 2t³`).
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Perlin's C² fade curve (`6t⁵ − 15t⁴ + 10t³`). Dearer than [`smooth`] and
/// worth it here: gradient noise shows second-derivative creases at the cell
/// boundaries with the cheaper curve, which is the artefact "improved noise"
/// was published to fix.
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// One octave of seeded 3-D **value** noise, in `−1..=1`.
///
/// The eight lattice corners of the containing cell, smoothstep-interpolated.
/// `channel` decorrelates the octaves (each one hashes a different lattice);
/// `cycle` loops the depth axis, `0` for no loop.
#[must_use]
pub fn value3(seed: u32, channel: u32, x: f32, y: f32, z: f32, cycle: i32) -> f32 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (xf as i32, yf as i32, zf as i32);
    let (u, v, w) = (smooth(x - xf), smooth(y - yf), smooth(z - zf));
    let z0 = wrap_z(iz, cycle);
    let z1 = wrap_z(iz.wrapping_add(1), cycle);
    let corner = |dx: i32, dy: i32, zz: i32| hash01(seed, channel, ix + dx, iy + dy, zz);
    let x00 = lerp(corner(0, 0, z0), corner(1, 0, z0), u);
    let x10 = lerp(corner(0, 1, z0), corner(1, 1, z0), u);
    let x01 = lerp(corner(0, 0, z1), corner(1, 0, z1), u);
    let x11 = lerp(corner(0, 1, z1), corner(1, 1, z1), u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    lerp(y0, y1, w) * 2.0 - 1.0
}

/// One octave of seeded 3-D **Perlin** (gradient) noise, in roughly `−1..=1`.
///
/// Each of the eight corners contributes the dot product of its own gradient
/// with the vector from that corner to the sample point; the eight are faded
/// together with [`fade`]. Same `channel` and `cycle` meaning as [`value3`].
///
/// The result is scaled by [`PERLIN_NORM`] and is **not** clamped: an octave may
/// stray a little past 1 in a rare cell, the fractal sum divides by the total
/// amplitude anyway, and the effect's own contrast step clamps at the end. A
/// clamp here would put a flat spot in the middle of a gradient.
#[must_use]
pub fn perlin3(seed: u32, channel: u32, x: f32, y: f32, z: f32, cycle: i32) -> f32 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (xf as i32, yf as i32, zf as i32);
    let (fx, fy, fz) = (x - xf, y - yf, z - zf);
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let z0 = wrap_z(iz, cycle);
    let z1 = wrap_z(iz.wrapping_add(1), cycle);
    let corner = |dx: i32, dy: i32, dz: i32, zz: i32| {
        let idx = lattice_hash(seed, channel, ix + dx, iy + dy, zz) % 12;
        grad_dot(idx, fx - dx as f32, fy - dy as f32, fz - dz as f32)
    };
    let x00 = lerp(corner(0, 0, 0, z0), corner(1, 0, 0, z0), u);
    let x10 = lerp(corner(0, 1, 0, z0), corner(1, 1, 0, z0), u);
    let x01 = lerp(corner(0, 0, 1, z1), corner(1, 0, 1, z1), u);
    let x11 = lerp(corner(0, 1, 1, z1), corner(1, 1, 1, z1), u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    lerp(y0, y1, w) * PERLIN_NORM
}

/// The most octaves a fractal sum will run (docs/08 §3.37's Complexity ceiling).
/// Bounds the per-pixel loop, exactly as the depth-of-field blade count and the
/// motion blur's tap count are bounded — a `moderate` effect stays moderate.
pub const MAX_OCTAVES: u32 = 10;

/// One fractal field's shape, already reduced to the numbers both paths read
/// (docs/impl/effect-registry.md §2.4). The effect's `packed` builds it; the CPU
/// reference and the WGSL uniform carry the identical values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FractalField {
    pub seed: u32,
    /// Octave count, `1..=`[`MAX_OCTAVES`].
    pub octaves: u32,
    /// Each octave's amplitude as a share of the last (Sub influence ÷ 100).
    pub gain: f32,
    /// Each octave's frequency as a multiple of the last (100 ÷ Sub scaling).
    pub lacunarity: f32,
    /// Perlin rather than value noise.
    pub perlin: bool,
    /// Fold each octave to `|n|` — the ridged, smoke-like sum.
    pub turbulent: bool,
    /// Depth loop length in whole cells; `0` for a field that never repeats.
    pub cycle: i32,
}

/// The fractal sum at `(x, y, z)`, in `−1..=1`.
///
/// **Depth is deliberately not scaled by frequency** (docs/08 §3.37 decision 4):
/// every octave samples the same `z` and is decorrelated by its octave number
/// entering the hash instead. That is what makes [`FractalField::cycle`] an
/// exact loop at any complexity, and it stops the fine octaves boiling faster
/// than the coarse ones.
#[must_use]
pub fn fractal(f: &FractalField, x: f32, y: f32, z: f32) -> f32 {
    let mut amp = 1.0f32;
    let mut freq = 1.0f32;
    let mut sum = 0.0f32;
    let mut norm = 0.0f32;
    let octaves = f.octaves.clamp(1, MAX_OCTAVES);
    for o in 0..octaves {
        let n = if f.perlin {
            perlin3(f.seed, o, x * freq, y * freq, z, f.cycle)
        } else {
            value3(f.seed, o, x * freq, y * freq, z, f.cycle)
        };
        // Turbulent folds the octave about zero and re-spreads it, so the sum
        // keeps the same −1..1 range as the signed one and the two types are
        // comparable at the same Contrast.
        let n = if f.turbulent { n.abs() * 2.0 - 1.0 } else { n };
        sum += n * amp;
        norm += amp;
        amp *= f.gain;
        freq *= f.lacunarity;
    }
    sum / norm.max(1e-6)
}

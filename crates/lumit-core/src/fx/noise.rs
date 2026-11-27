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
    pub perlin: bool,
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

// ── Typed fractals: AE's fifteen fractal types and five noise bases ─────────
//
// Everything above stays exactly as it was: Turbulent displace, Roughen edges,
// Lightning and the rest read [`fractal`] and see no change. Fractal noise reads
// [`fractal_typed`], which gives the same first octave for the two types and two
// bases it shipped with (pinned by a test), shifts the finer octaves so they do
// not draw a cross at the origin, and adds the rest.
//
// A type changes one of three things:
// 1. how each octave is shaped before it is added (folded, sharpened, ridged);
// 2. how the octaves are combined (summed, or the brightest kept);
// 3. where the next octave samples: the Dynamic family and its relatives move
//    each octave along the slope of the one before (a domain warp), and the
//    Twist variants move it across the slope instead, which curls it.
//
// The slope comes from the octave's own interpolation, worked out alongside
// the value from the same corners, so a warped octave costs a handful of
// multiply-adds and no extra lattice lookups. The lattice hash is also folded
// one coordinate at a time with the shared prefixes reused, which is the same
// integer arithmetic as `lattice_hash` in fewer steps.

/// Noise type options, in the panel's order. The first two are the bases
/// Fractal noise shipped with; the rest are AE's other interpolations.
pub const NOISE_VALUE: u32 = 0;
pub const NOISE_PERLIN: u32 = 1;
pub const NOISE_BLOCK: u32 = 2;
pub const NOISE_LINEAR: u32 = 3;
pub const NOISE_SPLINE: u32 = 4;
pub const NOISE_CHIPS: u32 = 5;
pub const NOISE_CIRCULAR: u32 = 6;
/// Cinema 4D's noise types, each an octave shaped from gradient noise or from
/// cells. Mod noise is not here: it is the Block base.
pub const NOISE_BOX: u32 = 7;
pub const NOISE_BUYA: u32 = 8;
pub const NOISE_CRANAL: u32 = 9;
pub const NOISE_DENTS: u32 = 10;
pub const NOISE_ELECTRIC: u32 = 11;
pub const NOISE_FIRE: u32 = 12;
pub const NOISE_HAMA: u32 = 13;
pub const NOISE_LUKA: u32 = 14;
pub const NOISE_NAKI: u32 = 15;
pub const NOISE_NUTOUS: u32 = 16;
pub const NOISE_OBER: u32 = 17;
pub const NOISE_PEZO: u32 = 18;
pub const NOISE_POXO: u32 = 19;
pub const NOISE_SEMA: u32 = 20;
pub const NOISE_STUPL: u32 = 21;
pub const NOISE_ZADA: u32 = 22;
pub const NOISE_BLISTERED_TURBULENCE: u32 = 23;
pub const NOISE_DISPLACED_TURBULENCE: u32 = 24;
pub const NOISE_WAVY_TURBULENCE: u32 = 25;
pub const NOISE_DISPLACED_VORONOI: u32 = 26;
pub const NOISE_VL: u32 = 27;
pub const NOISE_STUCCI: u32 = 28;
pub const NOISE_STUCCI_WALL_IN: u32 = 29;
pub const NOISE_STUCCI_WALL_OUT: u32 = 30;
pub const NOISE_WINDY: u32 = 31;
pub const NOISE_BILLOW: u32 = 32;
pub const NOISE_WAVES: u32 = 33;
pub const NOISE_LAST: u32 = NOISE_WAVES;
/// How far over the Stucci bases read their field a second time, in cells:
/// the difference is the relief, lit from the top left.
const STUCCI_STEP: f32 = 0.12;

/// Fractal type options, in the panel's order.
pub const FRACTAL_BASIC: u32 = 0;
pub const FRACTAL_TURBULENT: u32 = 1;
pub const FRACTAL_TURBULENT_SMOOTH: u32 = 2;
pub const FRACTAL_TURBULENT_SHARP: u32 = 3;
pub const FRACTAL_DYNAMIC: u32 = 4;
pub const FRACTAL_DYNAMIC_PROGRESSIVE: u32 = 5;
pub const FRACTAL_DYNAMIC_TWIST: u32 = 6;
pub const FRACTAL_MAX: u32 = 7;
pub const FRACTAL_SMEARY: u32 = 8;
pub const FRACTAL_SWIRLY: u32 = 9;
pub const FRACTAL_ROCKY: u32 = 10;
pub const FRACTAL_CLOUDY: u32 = 11;
pub const FRACTAL_TERRAIN: u32 = 12;
pub const FRACTAL_SUBSCALE: u32 = 13;
pub const FRACTAL_STRINGS: u32 = 14;
pub const FRACTAL_LAST: u32 = FRACTAL_STRINGS;

/// How far a warping type moves the next octave, per unit of slope, in cells of
/// the base octave.
const WARP_DYNAMIC: f32 = 0.12;
const WARP_TWIST: f32 = 0.2;
const WARP_SMEARY: f32 = 0.35;
const WARP_SWIRLY: f32 = 0.3;

#[must_use]
pub const fn warps(kind: u32) -> bool {
    matches!(
        kind,
        FRACTAL_DYNAMIC
            | FRACTAL_DYNAMIC_PROGRESSIVE
            | FRACTAL_DYNAMIC_TWIST
            | FRACTAL_SMEARY
            | FRACTAL_SWIRLY
    )
}

fn fold(h: u32, v: i32) -> u32 {
    super::maths::splitmix32(h ^ (v as u32))
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth_d(t: f32) -> f32 {
    6.0 * t * (1.0 - t)
}

fn fade_d(t: f32) -> f32 {
    30.0 * t * t * (t * (t - 2.0) + 1.0)
}

fn grad_xy(idx: u32) -> (f32, f32) {
    let s0 = if idx & 1 == 0 { 1.0 } else { -1.0 };
    let s1 = if idx & 2 == 0 { 1.0 } else { -1.0 };
    match idx / 4 {
        0 => (s0, s1),
        1 => (s0, 0.0),
        _ => (0.0, s0),
    }
}

fn spline(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

fn spline_d(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    0.5 * (p2 - p0)
        + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3)
        + 1.5 * t * t * (3.0 * (p1 - p2) + p3 - p0)
}

// ── The shaped bases: Octane's cells and Cinema 4D's noise types ────────────
//
// Each is one octave built from the gradient octave (`perlin3`, several
// channels of it) or from moving cells, then shaped: thresholded into
// speckles, folded into veins, banded into contours. The looks follow the
// pictures Maxon publishes of each type; the formulas are this file's own.
// No trigonometry and no `powf`: a periodic wave is a smoothed triangle, so
// the WGSL twin agrees op for op.

/// The central-difference step a warping type reads a shaped base's slope
/// with, in the octave's cells.
const SLOPE_STEP: f32 = 0.25;

/// A sine-like wave of period one, `−1..=1`: a triangle with its corners
/// rounded by a cubic. Smooth enough for contours, and exact in WGSL.
fn wave(t: f32) -> f32 {
    let f = t - t.floor();
    let g = 1.0 - 4.0 * (f - 0.5).abs();
    g * (1.5 - 0.5 * g * g)
}

#[allow(clippy::too_many_arguments)]
fn grad(seed: u32, channel: u32, c: u32, x: f32, y: f32, z: f32, cycle: i32) -> f32 {
    perlin3(seed, channel ^ (c << 20), x, y, z, cycle)
}

/// The nearest two distances to the cell points around `(x, y)`, in cells,
/// and the nearest cell's hash. Each point circles inside its cell as the
/// depth runs: once per loop when Evolution cycles, a quarter turn per turn
/// when it does not.
fn cells(seed: u32, channel: u32, x: f32, y: f32, z: f32, cycle: i32) -> (f32, f32, u32) {
    // The depth's whole part wrapped as the lattice wraps it, so a loop
    // lands on exactly the same phase.
    let zf = z.floor();
    let zw = wrap_z(zf as i32, cycle) as f32 + (z - zf);
    let phase = if cycle > 0 {
        zw / cycle as f32
    } else {
        z * 0.25
    };
    let (xf, yf) = (x.floor(), y.floor());
    let (ix, iy) = (xf as i32, yf as i32);
    let (fx, fy) = (x - xf, y - yf);
    let h0 = super::maths::splitmix32(seed ^ channel ^ 0x5bd1_e995);
    let (mut f1, mut f2, mut near) = (9.0f32, 9.0f32, 0u32);
    for cy in -1i32..=1 {
        for cx in -1i32..=1 {
            let h = fold(fold(h0, ix.wrapping_add(cx)), iy.wrapping_add(cy));
            let a = unit(fold(h, 3)) + phase;
            let px = cx as f32 + 0.15 + 0.7 * unit(fold(h, 1)) + 0.12 * wave(a + 0.25);
            let py = cy as f32 + 0.15 + 0.7 * unit(fold(h, 2)) + 0.12 * wave(a);
            let (dx, dy) = (fx - px, fy - py);
            let d = (dx * dx + dy * dy).sqrt();
            if d < f1 {
                f2 = f1;
                f1 = d;
                near = h;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, near)
}

fn ramp(t: f32) -> f32 {
    smooth(t.clamp(0.0, 1.0))
}

/// One octave of a shaped base, in about `−1..=1`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn shaped_octave(base: u32, seed: u32, ch: u32, x: f32, y: f32, z: f32, cycle: i32) -> f32 {
    let g = |c: u32, x: f32, y: f32| grad(seed, ch, c, x, y, z, cycle);
    match base {
        NOISE_CHIPS => {
            let (_, _, h) = cells(seed, ch, x, y, z, cycle);
            unit(h) * 2.0 - 1.0
        }
        NOISE_CIRCULAR => {
            let (f1, _, h) = cells(seed, ch, x, y, z, cycle);
            let r = 0.25 + 0.2 * unit(fold(h, 9));
            ramp((r - f1) / (0.4 * r)) * 2.0 - 1.0
        }
        NOISE_BOX => {
            let (xf, yf) = (x.floor(), y.floor());
            let bx = xf + ramp((x - xf - 0.3) * 2.5);
            let by = yf + ramp((y - yf - 0.3) * 2.5);
            octave(NOISE_LINEAR, seed, ch, bx, by, z, cycle, false).0
        }
        NOISE_BUYA => {
            let t = (g(0, x * 1.5, y * 1.5) * 1.6).clamp(0.0, 1.0);
            let t2 = t * t;
            t2 * t2 * 2.0 - 1.0
        }
        NOISE_CRANAL => {
            let (f1, f2, _) = cells(seed, ch, x, y, z, cycle);
            let t = 0.25 + 0.55 * ramp((f2 - f1) * 4.0) + 0.15 * g(0, x, y);
            t * 2.0 - 1.0
        }
        NOISE_DENTS => {
            let t = (g(0, x, y).abs() * 1.5).min(1.0);
            t * t * 2.0 - 1.0
        }
        NOISE_ELECTRIC => {
            let w = g(1, x, y) * 0.8;
            let u = (x + y) * std::f32::consts::FRAC_1_SQRT_2;
            let v = (y - x) * std::f32::consts::FRAC_1_SQRT_2;
            g(0, u * 0.4 + w, v * 2.2 + w)
        }
        NOISE_FIRE => {
            let t = (g(0, x, y) * 1.5 + 0.1).clamp(0.0, 1.0);
            t * t * 2.0 - 1.0
        }
        NOISE_HAMA => {
            let (f1, f2, h) = cells(seed, ch, x, y, z, cycle);
            let t = ramp((f2 - f1) * 4.0) * (0.35 + 0.65 * unit(fold(h, 5))) * (1.0 - 0.5 * f1);
            t * 2.0 - 1.0
        }
        NOISE_LUKA => {
            let m = g(1, x * 2.3, y * 2.3);
            (0.9 * g(0, x, y) + 0.9 * (0.5 - m.abs())).clamp(-1.0, 1.0)
        }
        NOISE_NAKI => {
            let n = g(0, x, y);
            (1.2 * n + 0.75 * (0.4 - n.abs())).clamp(-1.0, 1.0)
        }
        NOISE_NUTOUS => {
            let r = (1.0 - g(0, x, y).abs()).max(0.0);
            let r3 = r * r * r;
            r3 * r3 * 2.0 - 1.0
        }
        NOISE_OBER => {
            let s = wave(g(0, x, y) * 1.6);
            let a = s.abs();
            s.signum() * (a * (2.0 - a)) * 0.9
        }
        NOISE_PEZO => ramp((g(0, x, y) - 0.35) / 0.35) * 2.0 - 1.0,
        NOISE_POXO => ramp((g(0, x * 2.0, y * 2.0) - 0.45) / 0.4) * 1.2 - 1.0,
        NOISE_SEMA => (1.0 - ramp(g(0, x, y).abs() / 0.08)) * 2.0 - 1.0,
        NOISE_STUPL => {
            let n = g(0, x, y);
            let c = 1.0 - n.abs();
            (1.05 * n + 0.75 * (1.0 - c * c) - 0.45).clamp(-1.0, 1.0)
        }
        NOISE_ZADA => {
            let n = g(0, x, y);
            let m = g(1, x * 0.7, y * 0.7);
            wave((n + 0.6 * m) * 2.2) * 0.8 + 0.2 * n
        }
        NOISE_BLISTERED_TURBULENCE => {
            let m = g(1, x * 3.1, y * 3.1).max(0.0);
            (0.75 * g(0, x, y).abs() - 0.4 + 0.9 * m * m * m).clamp(-1.0, 1.0)
        }
        NOISE_DISPLACED_TURBULENCE => {
            let dx = g(1, x, y);
            let dy = g(2, x, y);
            g(0, x + 0.7 * dx, y + 0.7 * dy) * 0.7
        }
        NOISE_WAVY_TURBULENCE => {
            let n = g(0, x, y + 0.35 * wave(x * 0.64 + 0.32 * g(1, x, y)));
            (n.abs() * 2.0 - 1.0) * 0.7
        }
        NOISE_DISPLACED_VORONOI => {
            let dx = g(1, x, y);
            let dy = g(2, x, y);
            let (f1, _, _) = cells(seed, ch, x + 0.6 * dx, y + 0.6 * dy, z, cycle);
            f1.min(1.0) * 2.0 - 1.0
        }
        NOISE_STUCCI => {
            let n = g(0, x, y);
            let e = n - g(0, x + STUCCI_STEP, y + STUCCI_STEP);
            (0.4 * n + 4.0 * e).clamp(-1.0, 1.0)
        }
        NOISE_STUCCI_WALL_IN | NOISE_STUCCI_WALL_OUT => {
            let h0 = 1.0 - g(0, x, y).abs();
            let h1 = 1.0 - g(0, x + STUCCI_STEP, y + STUCCI_STEP).abs();
            let e = 2.5 * (h0 - h1);
            let r = 0.3 * (2.0 * h0 - 1.0) + e;
            if base == NOISE_STUCCI_WALL_IN {
                (-r).clamp(-1.0, 1.0)
            } else {
                r.clamp(-1.0, 1.0)
            }
        }
        NOISE_WINDY => {
            let wind = g(1, x * 0.2, y * 0.2).abs();
            g(0, x, y) * (wind * 2.5).min(1.0)
        }
        NOISE_BILLOW => {
            let dx = g(1, x, y);
            let dy = g(2, x, y);
            let (f1, _, h) = cells(seed, ch, x + 0.35 * dx, y + 0.35 * dy, z, cycle);
            let r = 0.45 + 0.35 * unit(fold(h, 7));
            let t = ramp(1.0 - f1 / r);
            t * (2.0 - t) * 2.0 - 1.0
        }
        NOISE_WAVES => {
            let n = g(0, x, y);
            let m = g(1, x, y);
            let a = wave(0.7 * x + 0.6 * n);
            let b = wave(0.7 * (-0.5 * x + 0.866_025_4 * y) + 0.6 * m);
            let c = wave(0.7 * (-0.5 * x - 0.866_025_4 * y) + 0.6 * (n - m));
            ((a + b + c) * 0.45).clamp(-1.0, 1.0)
        }
        _ => {
            let dx = g(1, x * 0.5, y * 0.5);
            let dy = g(2, x * 0.5, y * 0.5);
            g(0, x + 1.5 * dx, y + 1.5 * dy)
        }
    }
}

/// One octave of any noise base: the value in about `−1..=1` and, when `slope`
/// is asked for, its x and y derivatives in the octave's own cells (zero
/// otherwise). For Value and Perlin the value is bit-identical to [`value3`] and
/// [`perlin3`].
#[must_use]
#[allow(clippy::too_many_arguments, clippy::many_single_char_names)]
pub fn octave(
    base: u32,
    seed: u32,
    channel: u32,
    x: f32,
    y: f32,
    z: f32,
    cycle: i32,
    slope: bool,
) -> (f32, f32, f32) {
    if base >= NOISE_CHIPS {
        // The shaped bases have no interpolation of their own to take a slope
        // from; a warping type reads it as a central difference instead.
        let n = shaped_octave(base, seed, channel, x, y, z, cycle);
        if !slope {
            return (n, 0.0, 0.0);
        }
        let e = SLOPE_STEP;
        let gx = (shaped_octave(base, seed, channel, x + e, y, z, cycle)
            - shaped_octave(base, seed, channel, x - e, y, z, cycle))
            * (0.5 / e);
        let gy = (shaped_octave(base, seed, channel, x, y + e, z, cycle)
            - shaped_octave(base, seed, channel, x, y - e, z, cycle))
            * (0.5 / e);
        return (n, gx, gy);
    }
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (xf as i32, yf as i32, zf as i32);
    let (fx, fy, fz) = (x - xf, y - yf, z - zf);
    let z0 = wrap_z(iz, cycle);
    let z1 = wrap_z(iz.wrapping_add(1), cycle);
    let h0 = super::maths::splitmix32(seed ^ channel);
    if base == NOISE_SPLINE {
        let w = smooth(fz);
        let mut hxy = [[0u32; 4]; 4];
        for (i, cx) in (-1..=2).enumerate() {
            let hx = fold(h0, ix.wrapping_add(cx));
            for (j, cy) in (-1..=2).enumerate() {
                hxy[j][i] = fold(hx, iy.wrapping_add(cy));
            }
        }
        let mut slice = [(0.0f32, 0.0f32, 0.0f32); 2];
        for (s, zz) in [z0, z1].into_iter().enumerate() {
            let mut rows = [0.0f32; 4];
            let mut drows = [0.0f32; 4];
            for j in 0..4 {
                let r = hxy[j].map(|h| unit(fold(h, zz)) * 2.0 - 1.0);
                rows[j] = spline(r[0], r[1], r[2], r[3], fx);
                if slope {
                    drows[j] = spline_d(r[0], r[1], r[2], r[3], fx);
                }
            }
            let v = spline(rows[0], rows[1], rows[2], rows[3], fy);
            let (dx, dy) = if slope {
                (
                    spline(drows[0], drows[1], drows[2], drows[3], fy),
                    spline_d(rows[0], rows[1], rows[2], rows[3], fy),
                )
            } else {
                (0.0, 0.0)
            };
            slice[s] = (v, dx, dy);
        }
        return (
            lerp(slice[0].0, slice[1].0, w),
            lerp(slice[0].1, slice[1].1, w),
            lerp(slice[0].2, slice[1].2, w),
        );
    }
    let hx0 = fold(h0, ix);
    let hx1 = fold(h0, ix.wrapping_add(1));
    let (h00, h01) = (fold(hx0, iy), fold(hx0, iy.wrapping_add(1)));
    let (h10, h11) = (fold(hx1, iy), fold(hx1, iy.wrapping_add(1)));
    let (c000, c001) = (fold(h00, z0), fold(h00, z1));
    let (c010, c011) = (fold(h01, z0), fold(h01, z1));
    let (c100, c101) = (fold(h10, z0), fold(h10, z1));
    let (c110, c111) = (fold(h11, z0), fold(h11, z1));
    if base == NOISE_PERLIN {
        let (u, v, w) = (fade(fx), fade(fy), fade(fz));
        let corner = |h: u32, dx: f32, dy: f32, dz: f32| {
            let idx = h % 12;
            (grad_dot(idx, fx - dx, fy - dy, fz - dz), grad_xy(idx))
        };
        let (a000, g000) = corner(c000, 0.0, 0.0, 0.0);
        let (a100, g100) = corner(c100, 1.0, 0.0, 0.0);
        let (a010, g010) = corner(c010, 0.0, 1.0, 0.0);
        let (a110, g110) = corner(c110, 1.0, 1.0, 0.0);
        let (a001, g001) = corner(c001, 0.0, 0.0, 1.0);
        let (a101, g101) = corner(c101, 1.0, 0.0, 1.0);
        let (a011, g011) = corner(c011, 0.0, 1.0, 1.0);
        let (a111, g111) = corner(c111, 1.0, 1.0, 1.0);
        let x00 = lerp(a000, a100, u);
        let x10 = lerp(a010, a110, u);
        let x01 = lerp(a001, a101, u);
        let x11 = lerp(a011, a111, u);
        let y0 = lerp(x00, x10, v);
        let y1 = lerp(x01, x11, v);
        let n = lerp(y0, y1, w) * PERLIN_NORM;
        if !slope {
            return (n, 0.0, 0.0);
        }
        let tri = |a: [f32; 8]| {
            lerp(
                lerp(lerp(a[0], a[1], u), lerp(a[2], a[3], u), v),
                lerp(lerp(a[4], a[5], u), lerp(a[6], a[7], u), v),
                w,
            )
        };
        let gx = tri([
            g000.0, g100.0, g010.0, g110.0, g001.0, g101.0, g011.0, g111.0,
        ]);
        let gy = tri([
            g000.1, g100.1, g010.1, g110.1, g001.1, g101.1, g011.1, g111.1,
        ]);
        let (du, dv) = (fade_d(fx), fade_d(fy));
        let ex = lerp(
            lerp(a100 - a000, a110 - a010, v),
            lerp(a101 - a001, a111 - a011, v),
            w,
        );
        let ey = lerp(x10 - x00, x11 - x01, w);
        return (
            n,
            (gx + du * ex) * PERLIN_NORM,
            (gy + dv * ey) * PERLIN_NORM,
        );
    }
    // The value family: Value (smoothstep), Linear and Block, all blended in
    // depth with the smoothstep so Evolution glides on every one of them.
    let w = smooth(fz);
    let ((u, du), (v, dv)) = match base {
        NOISE_BLOCK => ((0.0, 0.0), (0.0, 0.0)),
        NOISE_LINEAR => ((fx, 1.0), (fy, 1.0)),
        _ => ((smooth(fx), smooth_d(fx)), (smooth(fy), smooth_d(fy))),
    };
    let (v000, v100, v010, v110) = (unit(c000), unit(c100), unit(c010), unit(c110));
    let (v001, v101, v011, v111) = (unit(c001), unit(c101), unit(c011), unit(c111));
    let x00 = lerp(v000, v100, u);
    let x10 = lerp(v010, v110, u);
    let x01 = lerp(v001, v101, u);
    let x11 = lerp(v011, v111, u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    let n = lerp(y0, y1, w) * 2.0 - 1.0;
    if !slope {
        return (n, 0.0, 0.0);
    }
    // Block's slope is zero and Linear's jumps at every cell edge, either of
    // which would tear a warp into tiles; both take the smoothstep's slope of
    // the same corners, the field's shape without its steps.
    let ((u, du), (v, dv)) = if base == NOISE_VALUE {
        ((u, du), (v, dv))
    } else {
        ((smooth(fx), smooth_d(fx)), (smooth(fy), smooth_d(fy)))
    };
    let ex = lerp(
        lerp(v100 - v000, v110 - v010, v),
        lerp(v101 - v001, v111 - v011, v),
        w,
    );
    let ey = lerp(
        lerp(v010, v110, u) - lerp(v000, v100, u),
        lerp(v011, v111, u) - lerp(v001, v101, u),
        w,
    );
    (n, 2.0 * du * ex, 2.0 * dv * ey)
}

/// How far each octave after the first is moved, in its own lattice cells:
/// the R2 low-discrepancy step, so the octaves' lattices never line up.
const OCTAVE_SHIFT: [f32; 2] = [0.754_877_7, 0.569_840_3];

/// The fractal sum of one of the fifteen types over one of the five bases, in
/// about `−1..=1`. One octave of Basic or Turbulent over Value or Perlin is
/// bit-identical to [`fractal`]; the finer octaves are shifted so their
/// lattices do not all meet at the origin. `f.perlin` and `f.turbulent` are not read, `base` and `kind`
/// say it all.
#[must_use]
pub fn fractal_typed(f: &FractalField, base: u32, kind: u32, x: f32, y: f32, z: f32) -> f32 {
    let mut amp = 1.0f32;
    let mut freq = 1.0f32;
    let mut sum = 0.0f32;
    let mut norm = 0.0f32;
    let mut peak = 0.0f32;
    let mut weight = 1.0f32;
    let (mut wx, mut wy) = (0.0f32, 0.0f32);
    let warping = warps(kind);
    let octaves = f.octaves.clamp(1, MAX_OCTAVES);
    for o in 0..octaves {
        let (sx, sy) = if warping {
            ((x + wx) * freq, (y + wy) * freq)
        } else {
            (x * freq, y * freq)
        };
        // Every octave scales about the same origin, so unshifted their lattice
        // lines all meet there and stack into a thin cross. Each octave after
        // the first moves by an irrational step, so no two share a line.
        let k = o as f32;
        let (sx, sy) = (sx + k * OCTAVE_SHIFT[0], sy + k * OCTAVE_SHIFT[1]);
        // Dynamic progressive: the finer octaves evolve faster, by whole
        // multiples so Cycle evolution still closes exactly.
        let (zz, cycle) = if kind == FRACTAL_DYNAMIC_PROGRESSIVE {
            let m = (o / 2 + 1) as i32;
            (z * m as f32, f.cycle * m)
        } else {
            (z, f.cycle)
        };
        let (n, gx, gy) = octave(base, f.seed, o, sx, sy, zz, cycle, warping);
        let t = n.abs();
        let shaped = match kind {
            FRACTAL_BASIC | FRACTAL_DYNAMIC_TWIST => n,
            FRACTAL_TURBULENT_SMOOTH => t * t * 2.0 - 1.0,
            FRACTAL_DYNAMIC | FRACTAL_DYNAMIC_PROGRESSIVE | FRACTAL_SMEARY => {
                let c = (1.0 - t).max(0.0);
                (1.0 - c * c) * 2.0 - 1.0
            }
            FRACTAL_TURBULENT_SHARP => {
                let c = (1.0 - t).max(0.0);
                (1.0 - c * c * c) * 2.0 - 1.0
            }
            FRACTAL_ROCKY => (n * 2.5).clamp(-1.0, 1.0),
            FRACTAL_CLOUDY => {
                let c = (0.5 - n * 0.5).clamp(0.0, 1.0);
                (1.0 - c * c) * 2.0 - 1.0
            }
            FRACTAL_TERRAIN => {
                let r = (1.0 - t).max(0.0);
                let r = r * r * weight;
                weight = (r * 2.0).clamp(0.0, 1.0);
                r * 2.0 - 1.0
            }
            FRACTAL_SUBSCALE => {
                let lift = if norm > 0.0 {
                    (sum / norm) * 0.5 + 0.5
                } else {
                    1.0
                };
                n * (0.4 + 1.2 * lift)
            }
            FRACTAL_STRINGS => {
                let r = (1.0 - t).max(0.0);
                let r2 = r * r;
                r2 * r2 * 2.0 - 1.0
            }
            _ => t * 2.0 - 1.0,
        };
        // Strings lets the fine octaves fade twice as fast, so the lines stay the
        // coarse octave's and do not dissolve into a lattice of fine ones.
        let a = if kind == FRACTAL_STRINGS {
            amp * amp
        } else {
            amp
        };
        if kind == FRACTAL_MAX {
            peak = peak.max(t * a);
        } else {
            sum += shaped * a;
        }
        norm += a;
        if warping {
            let s = amp / freq;
            match kind {
                FRACTAL_DYNAMIC_TWIST => {
                    wx -= WARP_TWIST * gy * s;
                    wy += WARP_TWIST * gx * s;
                }
                FRACTAL_SWIRLY => {
                    wx -= WARP_SWIRLY * gy * s;
                    wy += WARP_SWIRLY * gx * s;
                }
                FRACTAL_SMEARY => {
                    if o == 0 {
                        wx += WARP_SMEARY * gx;
                        wy += WARP_SMEARY * gy;
                    }
                }
                _ => {
                    wx += WARP_DYNAMIC * gx * s;
                    wy += WARP_DYNAMIC * gy * s;
                }
            }
        }
        amp *= f.gain;
        freq *= f.lacunarity;
    }
    if kind == FRACTAL_MAX {
        return peak * 2.0 - 1.0;
    }
    sum / norm.max(1e-6)
}

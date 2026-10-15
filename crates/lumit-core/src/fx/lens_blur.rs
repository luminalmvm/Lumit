//! Depth of field pro's bake: the chosen lens traced once into a table of real
//! out-of-focus shapes, across the frame and through focus.
//!
//! For each place in the frame, from the centre out to the corner, and each
//! turn of the iris against that place, a bundle of parallel rays at the
//! matching field angle goes through every surface at three wavelengths.
//! Every element's clear aperture and the iris clip it. What leaves the last
//! surface is then cut at a run of planes either side of best focus, and each
//! cut is splatted into a small kernel.
//!
//! That is all the physics there is. Cat's eyes come from the clipping, rings
//! and soft edges from spherical aberration, comet tails from coma, coloured
//! edges from the glass. Nothing is painted on.
//!
//! The frame only looks kernels up, so the bake depends on the lens, the
//! f-stop, the iris, the sensor and the frame's shape, and not on focus or
//! depth. Racking focus never rebakes.

use rayon::prelude::*;

use super::lens_flare::{
    effective_roundness, lens_entry, parse_lens, pupil_mask, refract3, FlareSurface,
};
use super::noise;

/// Places from the frame centre to its corner.
pub const FIELD_BINS: usize = 10;
/// Defocus planes, half in front of best focus and half behind.
pub const DEFOCUS_BINS: usize = 24;
/// Turns of the iris against the radial direction, over half a blade.
pub const AZIMUTH_BINS: usize = 4;
/// Each kernel is this many cells square.
pub const KERNEL_RES: usize = 40;
/// Kernels in the table.
pub const SLICES: usize = FIELD_BINS * DEFOCUS_BINS * AZIMUTH_BINS;
/// The furthest defocus plane, as a fraction of the focal length. Past it the
/// last kernel is scaled up rather than traced.
pub const DZ_MAX_OF_FOCAL: f32 = 0.12;
/// The widest a kernel may grow, half-width in pixels. The effect pads by it.
pub const MAX_SPAN_PX: f32 = 256.0;
/// Sensor widths for the Sensor row, mm.
pub const SENSOR_WIDTHS_MM: [f32; 4] = [36.0, 24.89, 23.6, 17.3];

const HALF: usize = DEFOCUS_BINS / 2;
const PUPIL_GRID: usize = 160;
const LAMBDA_UM: [f32; 3] = [0.62, 0.55, 0.46];
const LAMBDA_D_UM: f32 = 0.587_56;

/// What the bake depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BokehLens {
    /// Index into the bundled library, used when no lens text is given.
    pub lens: u32,
    pub fstop: f32,
    pub blades: u32,
    /// 0 is the blade polygon, 1 a circle.
    pub roundness: f32,
    /// Scales the glass's dispersion: 0 none, 1 as designed.
    pub fringing: f32,
    /// Smears and specks on the front glass, 0 clean.
    pub glass_dirt: f32,
    pub sensor_w_mm: f32,
    /// Frame height over width.
    pub aspect: f32,
}

/// The traced table. Kernel cells run tangential along x and radial (outward)
/// along y, each channel summing to 1.
pub struct BokehBake {
    /// `SLICES * KERNEL_RES²` cells, slice-major. Slice
    /// `(field * DEFOCUS_BINS + defocus) * AZIMUTH_BINS + azimuth`.
    pub kernels: Vec<[f32; 3]>,
    /// Each slice's half-width on the sensor at its own defocus plane, mm.
    pub span_mm: Vec<f32>,
    /// The furthest defocus plane, mm.
    pub dz_max_mm: f32,
    /// The image height of the last field bin: the sensor's half-diagonal.
    pub field_max_mm: f32,
    pub focal_mm: f32,
}

/// The defocus of plane `bin`, mm: negative in front of best focus.
/// Spaced on a square law, so the planes crowd in near focus where the shape
/// changes fastest.
pub fn dz_centre(bin: usize, dz_max: f32) -> f32 {
    if bin < HALF {
        let k = (HALF - bin) as f32 / HALF as f32;
        -dz_max * k * k
    } else {
        let k = (bin - HALF + 1) as f32 / HALF as f32;
        dz_max * k * k
    }
}

/// The plane nearest to `dz`, the inverse of [`dz_centre`]. The shader has
/// the same function.
pub fn dz_bin(dz: f32, dz_max: f32) -> usize {
    let s = ((dz.abs() / dz_max.max(1e-6)).sqrt() * HALF as f32).round();
    let k = (s as usize).clamp(1, HALF);
    if dz < 0.0 {
        HALF - k
    } else {
        HALF - 1 + k
    }
}

/// The key a cached bake is kept under.
pub fn bake_key(p: &BokehLens, text_hash: Option<u64>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.lens.hash(&mut h);
    p.blades.hash(&mut h);
    for v in [
        p.fstop,
        p.roundness,
        p.fringing,
        p.glass_dirt,
        p.sensor_w_mm,
        p.aspect,
    ] {
        v.to_bits().hash(&mut h);
    }
    text_hash.hash(&mut h);
    h.finish()
}

fn ior_at(s: &FlareSurface, lambda_um: f32, fringing: f32) -> f32 {
    let d = 1.0 / (LAMBDA_D_UM * LAMBDA_D_UM);
    let l = 1.0 / (lambda_um * lambda_um);
    s.cauchy_a + s.cauchy_b * (d + fringing * (l - d))
}

/// Where a ray meets one surface, and the normal facing it. The root nearest
/// the vertex, as the flare picks it.
fn intersect(p: [f32; 3], d: [f32; 3], radius: f32, z: f32) -> Option<([f32; 3], [f32; 3])> {
    if d[2].abs() < 1e-9 {
        return None;
    }
    let at = |t: f32| [p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t];
    if radius.abs() < 1e-6 {
        let t = (z - p[2]) / d[2];
        return (t > 1e-6).then(|| (at(t), [0.0, 0.0, -d[2].signum()]));
    }
    let c = [0.0, 0.0, z + radius];
    let oc = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
    let b = oc[0] * d[0] + oc[1] * d[1] + oc[2] * d[2];
    let cc = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius;
    let disc = b * b - cc;
    if disc < 0.0 {
        return None;
    }
    let sd = disc.sqrt();
    let (t1, t2) = (-b - sd, -b + sd);
    let t = match (t1 > 1e-6, t2 > 1e-6) {
        (true, true) => {
            if (at(t1)[2] - z).abs() < (at(t2)[2] - z).abs() {
                t1
            } else {
                t2
            }
        }
        (true, false) => t1,
        (false, true) => t2,
        _ => return None,
    };
    let hit = at(t);
    let inv = 1.0 / radius.abs();
    let mut n = [
        (hit[0] - c[0]) * inv,
        (hit[1] - c[1]) * inv,
        (hit[2] - c[2]) * inv,
    ];
    if n[0] * d[0] + n[1] * d[1] + n[2] * d[2] > 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    Some((hit, n))
}

/// The iris: blade count, roundness and turn, in the bake's frame.
#[derive(Clone, Copy)]
struct Iris {
    blades: u32,
    roundness: f32,
    rot: f32,
}

/// A ray after the last surface: position, then slope dx/dz and dy/dz, and the
/// weight the iris edge left it.
#[derive(Clone, Copy)]
struct Exit {
    p: [f32; 3],
    t: [f32; 2],
    w: f32,
}

struct Lens {
    surfaces: Vec<FlareSurface>,
    stop_r: f32,
    sensor_z: f32,
    focal: f32,
    glass_dirt: f32,
}

/// How much light the front glass lets through at `(x, y)`, its clear radius
/// 1: wiped smears running one way, and specks. Never quite 0, so a ray under
/// a speck still counts for the pupil's size.
fn front_glass(x: f32, y: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return 1.0;
    }
    let (s, c) = 0.6_f32.sin_cos();
    let (u, v) = (x * c + y * s, y * c - x * s);
    let wave = |k: f32| (0.5 + 0.5 * k.sin()).powi(2);
    let smear = wave(v * 17.0 + 2.5 * (u * 1.7 + 0.8).sin() + 1.4 * (v * 6.1).sin())
        * (0.35 + 0.65 * (0.5 + 0.5 * (u * 2.3 + v * 1.1 + 2.0).sin()))
        + 0.5 * wave(v * 43.0 + 1.3 * (u * 3.1).sin());
    let mut t = 1.0 - 0.85 * amount.min(1.0) * smear.min(1.0);
    let cells = 18.0;
    let (cx, cy) = ((x * cells).floor() as i32, (y * cells).floor() as i32);
    for oy in -1..=1 {
        for ox in -1..=1 {
            let (gx, gy) = (cx + ox, cy + oy);
            if noise::hash01(11, 0, gx, gy, 0) > 0.35 * amount.min(1.0) {
                continue;
            }
            let at = [
                (gx as f32 + noise::hash01(11, 1, gx, gy, 0)) / cells,
                (gy as f32 + noise::hash01(11, 2, gx, gy, 0)) / cells,
            ];
            let r = (0.01 + 0.03 * noise::hash01(11, 3, gx, gy, 0)) * (0.5 + 0.5 * amount);
            let d = ((x - at[0]).powi(2) + (y - at[1]).powi(2)).sqrt();
            if d < r {
                t *= 0.25;
            }
        }
    }
    t.max(0.05)
}

impl Lens {
    /// Trace one ray front to back. `clip` false ignores every aperture, which
    /// is what finding the chief ray wants.
    fn trace(
        &self,
        mut p: [f32; 3],
        mut d: [f32; 3],
        lambda: f32,
        fr: f32,
        iris: Option<Iris>,
    ) -> Option<Exit> {
        let mut n1 = 1.0;
        let mut w = 1.0;
        for (i, s) in self.surfaces.iter().enumerate() {
            let (hit, n) = intersect(p, d, s.radius_mm, s.z_mm)?;
            let r2 = hit[0] * hit[0] + hit[1] * hit[1];
            if let Some(iris) = iris {
                if i == 0 {
                    let r = s.semi_ap_mm.max(1e-3);
                    w *= front_glass(hit[0] / r, hit[1] / r, self.glass_dirt);
                }
                if s.is_stop > 0.5 {
                    let r = self.stop_r;
                    w *= pupil_mask(
                        hit[0] / r,
                        hit[1] / r,
                        iris.blades,
                        iris.rot,
                        iris.roundness,
                        0.04,
                    );
                    if w <= 0.0 {
                        return None;
                    }
                } else if r2 > s.semi_ap_mm * s.semi_ap_mm {
                    return None;
                }
            }
            let n2 = ior_at(s, lambda, fr);
            d = refract3(d, n, n1 / n2)?;
            n1 = n2;
            p = hit;
        }
        (d[2] > 1e-6).then(|| Exit {
            p,
            t: [d[0] / d[2], d[1] / d[2]],
            w,
        })
    }

    /// The ray's height where it crosses the stop, apertures ignored.
    fn stop_height(&self, y0: f32, dir: [f32; 3], start_z: f32) -> Option<f32> {
        let mut p = [0.0, y0, start_z];
        let mut d = dir;
        let mut n1 = 1.0;
        for s in &self.surfaces {
            let (hit, n) = intersect(p, d, s.radius_mm, s.z_mm)?;
            if s.is_stop > 0.5 {
                return Some(hit[1]);
            }
            let n2 = ior_at(s, LAMBDA_UM[1], 1.0);
            d = refract3(d, n, n1 / n2)?;
            n1 = n2;
            p = hit;
        }
        None
    }

    /// Where a ray at `dir` must start to pass through the middle of the stop:
    /// a few secant steps on its starting height.
    fn chief_start(&self, dir: [f32; 3], start_z: f32) -> f32 {
        let (mut a, mut b) = (0.0_f32, -1.0_f32);
        let (Some(mut fa), Some(mut fb)) = (
            self.stop_height(a, dir, start_z),
            self.stop_height(b, dir, start_z),
        ) else {
            return 0.0;
        };
        for _ in 0..12 {
            if (fb - fa).abs() < 1e-9 {
                break;
            }
            let c = b - fb * (b - a) / (fb - fa);
            let Some(fc) = self.stop_height(c, dir, start_z) else {
                break;
            };
            (a, fa, b, fb) = (b, fb, c, fc);
            if fb.abs() < 1e-5 {
                break;
            }
        }
        b
    }

    /// The radius of the entrance pupil on axis, with every aperture in play.
    fn entrance_pupil(&self, start_z: f32) -> f32 {
        let front = self.surfaces[0].semi_ap_mm;
        let passes = |h: f32| {
            self.trace(
                [0.0, h, start_z],
                [0.0, 0.0, 1.0],
                LAMBDA_UM[1],
                1.0,
                Some(Iris {
                    blades: 16,
                    roundness: 1.0,
                    rot: 0.0,
                }),
            )
            .is_some()
        };
        let (mut lo, mut hi) = (0.0_f32, front);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if passes(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

/// One field and iris turn: every surviving ray at all three wavelengths.
fn bundle(lens: &Lens, theta: f32, iris: Iris, fr: f32, ep: f32, start_z: f32) -> [Vec<Exit>; 3] {
    let dir = [0.0, theta.sin(), theta.cos()];
    let y0 = lens.chief_start(dir, start_z);
    // The pupil seen at an angle is longer in y across the starting plane.
    let half = ep * 1.6;
    let half_y = half / theta.cos().max(0.2);
    let (step, step_y) = (
        2.0 * half / PUPIL_GRID as f32,
        2.0 * half_y / PUPIL_GRID as f32,
    );
    let mut out: [Vec<Exit>; 3] = Default::default();
    for (li, &lambda) in LAMBDA_UM.iter().enumerate() {
        for gy in 0..PUPIL_GRID {
            for gx in 0..PUPIL_GRID {
                let x = -half + (gx as f32 + 0.5) * step;
                let y = y0 - half_y + (gy as f32 + 0.5) * step_y;
                if let Some(e) = lens.trace([x, y, start_z], dir, lambda, fr, Some(iris)) {
                    out[li].push(e);
                }
            }
        }
    }
    out
}

/// The point the bundle images to: the green weighted centroid at the sensor.
fn image_point(rays: &[Exit], z: f32) -> [f32; 2] {
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for r in rays {
        let dz = z - r.p[2];
        sx += r.w * (r.p[0] + r.t[0] * dz);
        sy += r.w * (r.p[1] + r.t[1] * dz);
        sw += r.w;
    }
    if sw > 0.0 {
        [sx / sw, sy / sw]
    } else {
        [0.0, 0.0]
    }
}

/// Splat one bundle cut `dz` from the sensor into a kernel, returning its
/// half-width.
///
/// `errors` scales how far each ray already misses the image point at the
/// sensor, the lens's own aberrations. It is faded in from nothing at focus,
/// so what the footage has in focus stays sharp, and reaches 1 once the blur
/// is several times wider than the errors, where they give the shape its
/// rims and tails.
fn splat(
    rays: &[Vec<Exit>; 3],
    sensor_z: f32,
    dz: f32,
    errors: f32,
    origin: [f32; 2],
    out: &mut [[f32; 3]],
) -> f32 {
    let at = |r: &Exit| {
        let to = sensor_z - r.p[2];
        [
            (r.p[0] + r.t[0] * to - origin[0]) * errors + r.t[0] * dz,
            (r.p[1] + r.t[1] * to - origin[1]) * errors + r.t[1] * dz,
        ]
    };
    // Sized to hold 99.7% of the light: a handful of stray rays at the edge
    // of the glass would otherwise stretch a tiny spot over a wide kernel.
    let mut reach: Vec<(f32, f32)> = rays
        .iter()
        .flatten()
        .map(|r| {
            let q = at(r);
            (q[0].abs().max(q[1].abs()), r.w)
        })
        .collect();
    reach.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f32 = reach.iter().map(|r| r.1).sum();
    let (mut acc, mut span) = (0.0_f32, 1e-4_f32);
    for (d, w) in reach {
        span = d;
        acc += w;
        if acc >= total * 0.997 {
            break;
        }
    }
    span = span.max(1e-4) * 1.06;
    let k = KERNEL_RES as f32;
    let mut raw = vec![[0.0_f32; 3]; KERNEL_RES * KERNEL_RES];
    for (c, list) in rays.iter().enumerate() {
        for r in list {
            let q = at(r);
            let u = (q[0] / span * 0.5 + 0.5) * k - 0.5;
            let v = (q[1] / span * 0.5 + 0.5) * k - 0.5;
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = (u - x0, v - y0);
            for (dx, dy, wt) in [
                (0, 0, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                let (x, y) = (x0 as i32 + dx, y0 as i32 + dy);
                if (0..KERNEL_RES as i32).contains(&x) && (0..KERNEL_RES as i32).contains(&y) {
                    raw[y as usize * KERNEL_RES + x as usize][c] += r.w * wt;
                }
            }
        }
    }
    // A 1-2-1 tent each way smooths the ray grid out of the shape.
    let tap = |x: i32, y: i32, c: usize| {
        if (0..KERNEL_RES as i32).contains(&x) && (0..KERNEL_RES as i32).contains(&y) {
            raw[y as usize * KERNEL_RES + x as usize][c]
        } else {
            0.0
        }
    };
    let mut sum = [0.0_f32; 3];
    for y in 0..KERNEL_RES as i32 {
        for x in 0..KERNEL_RES as i32 {
            for c in 0..3 {
                let mut v = 0.0;
                for (oy, wy) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                    for (ox, wx) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                        v += wx * wy * tap(x + ox, y + oy, c);
                    }
                }
                out[y as usize * KERNEL_RES + x as usize][c] = v;
                sum[c] += v;
            }
        }
    }
    for cell in out.iter_mut() {
        for c in 0..3 {
            if sum[c] > 0.0 {
                cell[c] /= sum[c];
            }
        }
    }
    span
}

/// One field and iris turn's kernels and half-widths, every defocus plane.
type Traced = (Vec<[f32; 3]>, Vec<f32>);

/// Trace the table. `None` only if neither the given text nor the library
/// entry is a lens, which the library's own tests rule out.
pub fn bake(p: &BokehLens, lens_text: Option<&str>) -> Option<BokehBake> {
    let entry = lens_entry(p.lens);
    let custom = lens_text.and_then(parse_lens);
    let stated = if custom.is_some() {
        0.0
    } else {
        entry.native_fstop
    };
    let mut rx = custom.or_else(|| parse_lens(entry.text))?;
    // The files' `semi_ap` column is the clear diameter: halved, every
    // bundled lens traces to its stated f-number.
    for s in &mut rx.surfaces {
        s.semi_ap_mm *= 0.5;
    }
    let stop_full = rx
        .surfaces
        .iter()
        .find(|s| s.is_stop > 0.5)
        .map(|s| s.semi_ap_mm)?;
    let start_z = rx.surfaces[0].z_mm - 10.0;
    let mut lens = Lens {
        surfaces: rx.surfaces,
        stop_r: stop_full,
        sensor_z: rx.sensor_z_mm,
        focal: rx.focal_mm,
        glass_dirt: p.glass_dirt.max(0.0),
    };
    let native = if stated > 0.0 {
        stated
    } else {
        lens.focal / (2.0 * lens.entrance_pupil(start_z).max(0.05))
    };
    lens.stop_r = stop_full * (native / p.fstop.max(native)).clamp(0.02, 1.0);
    let ep = lens.entrance_pupil(start_z).max(0.02);

    // Wide open the blades fold back behind the bore and the iris is round.
    let roundness = effective_roundness(p.roundness.clamp(0.0, 1.0), p.fstop, native);
    let round = roundness >= 0.999;
    let sector = std::f32::consts::PI / p.blades.clamp(3, 16) as f32;
    let iris = |a: usize| Iris {
        blades: p.blades.clamp(3, 16),
        roundness,
        rot: std::f32::consts::FRAC_PI_2 + a as f32 / (AZIMUTH_BINS - 1) as f32 * sector,
    };

    // Best focus: where the on-axis green bundle's spread is least. The spread
    // is quadratic in the plane, so it has a closed form.
    let axis = bundle(&lens, 0.0, iris(0), p.fringing, ep, start_z);
    let (mut ab, mut bb) = (0.0_f32, 0.0_f32);
    for r in &axis[1] {
        let dz = lens.sensor_z - r.p[2];
        let a = [r.p[0] + r.t[0] * dz, r.p[1] + r.t[1] * dz];
        ab += r.w * (a[0] * r.t[0] + a[1] * r.t[1]);
        bb += r.w * (r.t[0] * r.t[0] + r.t[1] * r.t[1]);
    }
    if bb > 0.0 {
        lens.sensor_z -= ab / bb;
    }
    let axis_weight: f32 = axis[1].iter().map(|r| r.w).sum();

    let field_max = 0.5 * p.sensor_w_mm * (1.0 + p.aspect * p.aspect).sqrt();
    let dz_max = DZ_MAX_OF_FOCAL * lens.focal;
    let cells = KERNEL_RES * KERNEL_RES;
    let azimuths = if round { 1 } else { AZIMUTH_BINS };

    // Each field and iris turn is its own bundle, so they trace side by side.
    let jobs: Vec<(usize, usize)> = (0..FIELD_BINS)
        .flat_map(|f| (0..azimuths).map(move |a| (f, a)))
        .collect();
    let traced: Vec<Option<Traced>> = jobs
        .par_iter()
        .map(|&(f, a)| {
            let h = field_max * f as f32 / (FIELD_BINS - 1) as f32;
            let theta = (h / lens.focal).atan();
            let rays = bundle(&lens, theta, iris(a), p.fringing, ep, start_z);
            let w: f32 = rays[1].iter().map(|r| r.w).sum();
            if w < axis_weight * 0.02 {
                return None;
            }
            let origin = image_point(&rays[1], lens.sensor_z);
            // How far the green rays miss the image point at focus, and the
            // defocus by which that is a sixth of the blur.
            let (mut e2, mut ew) = (0.0_f32, 0.0_f32);
            for r in &rays[1] {
                let to = lens.sensor_z - r.p[2];
                let (x, y) = (
                    r.p[0] + r.t[0] * to - origin[0],
                    r.p[1] + r.t[1] * to - origin[1],
                );
                e2 += r.w * (x * x + y * y);
                ew += r.w;
            }
            let fade_dz = 6.0 * p.fstop.max(0.5) * (e2 / ew.max(1e-9)).sqrt() * 2.0;
            let mut kernels = vec![[0.0_f32; 3]; DEFOCUS_BINS * cells];
            let mut spans = vec![0.0_f32; DEFOCUS_BINS];
            for (b, (k, span)) in kernels.chunks_mut(cells).zip(spans.iter_mut()).enumerate() {
                let dz = dz_centre(b, dz_max);
                let t = (dz.abs() / fade_dz.max(1e-6)).min(1.0);
                *span = splat(&rays, lens.sensor_z, dz, t * t * (3.0 - 2.0 * t), origin, k);
            }
            Some((kernels, spans))
        })
        .collect();

    // A place the lens no longer covers keeps the shape of the last one it did.
    let mut kernels = vec![[0.0_f32; 3]; SLICES * cells];
    let mut span_mm = vec![0.0_f32; SLICES];
    let mut last: Vec<Option<&Traced>> = vec![None; AZIMUTH_BINS];
    for (i, &(f, a)) in jobs.iter().enumerate() {
        if let Some(t) = &traced[i] {
            last[a] = Some(t);
        }
        let Some((k, s)) = last[a].or(last[0]) else {
            continue;
        };
        let fill = if round { AZIMUTH_BINS } else { 1 };
        for az in a..a + fill {
            for b in 0..DEFOCUS_BINS {
                let slice = (f * DEFOCUS_BINS + b) * AZIMUTH_BINS + az;
                kernels[slice * cells..(slice + 1) * cells]
                    .copy_from_slice(&k[b * cells..(b + 1) * cells]);
                span_mm[slice] = s[b];
            }
        }
    }
    Some(BokehBake {
        kernels,
        span_mm,
        dz_max_mm: dz_max,
        field_max_mm: field_max,
        focal_mm: lens.focal,
    })
}

/// One ray of the lens window's drawing: `(z, y)` points in mm, front vertex
/// at z 0, and whether an element edge or the iris stopped it.
#[derive(Debug, Clone, PartialEq)]
pub struct FanRay {
    pub points: Vec<[f32; 2]>,
    pub clipped: bool,
}

/// The lens window's ray drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct RayFan {
    pub rays: Vec<FanRay>,
    /// Where the sensor sits at best focus, mm.
    pub sensor_z_mm: f32,
    /// The iris's radius at this f-stop, and wide open, mm.
    pub stop_r_mm: f32,
    pub stop_full_mm: f32,
    /// The widest the lens opens.
    pub native_fstop: f32,
}

impl Lens {
    /// One green ray in the drawing's plane, every hit kept, run on a little
    /// past the sensor if it gets through.
    fn path(&self, mut p: [f32; 3], mut d: [f32; 3]) -> FanRay {
        let mut points = vec![[p[2], p[1]]];
        let mut n1 = 1.0;
        for s in &self.surfaces {
            let Some((hit, n)) = intersect(p, d, s.radius_mm, s.z_mm) else {
                return FanRay::stopped(points);
            };
            points.push([hit[2], hit[1]]);
            let r = if s.is_stop > 0.5 {
                self.stop_r
            } else {
                s.semi_ap_mm
            };
            if hit[1].abs() > r {
                return FanRay::stopped(points);
            }
            let n2 = ior_at(s, LAMBDA_UM[1], 1.0);
            let Some(next) = refract3(d, n, n1 / n2) else {
                return FanRay::stopped(points);
            };
            (d, n1, p) = (next, n2, hit);
        }
        let last = self.surfaces.last().map_or(0.0, |s| s.z_mm);
        let end = self.sensor_z + 0.3 * (self.sensor_z - last).abs();
        if d[2] > 1e-6 {
            points.push([end, p[1] + d[1] * (end - p[2]) / d[2]]);
        }
        FanRay {
            points,
            clipped: false,
        }
    }
}

impl FanRay {
    fn stopped(points: Vec<[f32; 2]>) -> Self {
        Self {
            points,
            clipped: true,
        }
    }
}

/// `count` parallel rays across the front glass at `field_deg` off the axis,
/// traced with the bake's own surfaces, iris and best focus.
pub fn ray_fan(text: &str, fstop: f32, field_deg: f32, count: usize) -> Option<RayFan> {
    let mut rx = parse_lens(text)?;
    for s in &mut rx.surfaces {
        s.semi_ap_mm *= 0.5;
    }
    let stop_full = rx
        .surfaces
        .iter()
        .find(|s| s.is_stop > 0.5)
        .map(|s| s.semi_ap_mm)?;
    let front_z = rx.surfaces[0].z_mm;
    let start_z = front_z - 10.0;
    let mut lens = Lens {
        surfaces: rx.surfaces,
        stop_r: stop_full,
        sensor_z: rx.sensor_z_mm,
        focal: rx.focal_mm,
        glass_dirt: 0.0,
    };
    let native = lens.focal / (2.0 * lens.entrance_pupil(start_z).max(0.05));
    lens.stop_r = stop_full * (native / fstop.max(native)).clamp(0.02, 1.0);
    let front = lens.surfaces[0].semi_ap_mm;
    let across = |n: usize, i: usize| front * ((i as f32 + 0.5) / n as f32 * 2.0 - 1.0);

    // Best focus as the bake finds it, from rays on the axis, each weighted by
    // its height so the fan counts like the round pupil.
    let iris = Iris {
        blades: 16,
        roundness: 1.0,
        rot: 0.0,
    };
    let (mut ab, mut bb) = (0.0_f32, 0.0_f32);
    for i in 0..64 {
        let y = across(64, i);
        if let Some(r) = lens.trace(
            [0.0, y, start_z],
            [0.0, 0.0, 1.0],
            LAMBDA_UM[1],
            1.0,
            Some(iris),
        ) {
            let a = r.p[1] + r.t[1] * (lens.sensor_z - r.p[2]);
            ab += y.abs() * a * r.t[1];
            bb += y.abs() * r.t[1] * r.t[1];
        }
    }
    if bb > 0.0 {
        lens.sensor_z -= ab / bb;
    }

    let theta = field_deg.to_radians();
    let dir = [0.0, theta.sin(), theta.cos()];
    let rays = (0..count.max(1))
        .map(|i| {
            lens.path(
                [
                    0.0,
                    across(count, i) - theta.tan() * (front_z - start_z),
                    start_z,
                ],
                dir,
            )
        })
        .collect();
    Some(RayFan {
        rays,
        sensor_z_mm: lens.sensor_z,
        stop_r_mm: lens.stop_r,
        stop_full_mm: stop_full,
        native_fstop: native,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::fx::lens_library::LENS_LIBRARY;

    /// Every bundled lens traces to the f-number it is sold at, which is what
    /// halving the files' diameter column rests on, and stopping it down
    /// stops rays at the iris.
    #[test]
    fn bundled_lenses_trace_to_their_stated_f_number() {
        for entry in &LENS_LIBRARY {
            let open = ray_fan(entry.text, 0.1, 0.0, 21).unwrap();
            let shut = ray_fan(entry.text, 16.0, 0.0, 21).unwrap();
            let ratio = open.native_fstop / entry.native_fstop;
            assert!(
                (0.9..1.1).contains(&ratio),
                "{}: traces to f/{}",
                entry.name,
                open.native_fstop
            );
            let through = |f: &RayFan| f.rays.iter().filter(|r| !r.clipped).count();
            assert!(through(&shut) < through(&open), "{}", entry.name);
        }
    }
}

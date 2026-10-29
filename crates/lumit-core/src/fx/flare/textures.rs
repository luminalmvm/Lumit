//! The pictures Ghost, Dirt and Picture can use: Dust, Smudge, Caustic,
//! Rings, Specks, Hex bokeh, Scratches, Rainbow, Water drops and Grain, each
//! a square of brightness made here in code, once, with its smaller copies
//! so a small sprite doesn't shimmer.
//!
//! The GPU is handed the very same floats and reads them with the same
//! sums as [`sample`], so the two draw the same pixels.

use std::f32::consts::TAU;
use std::sync::OnceLock;

use crate::fx::noise::hash01;

/// Each texture's side, texels.
pub const SIZE: usize = 256;

/// How many there are. The controls number them from 1, 0 being none.
pub const COUNT: usize = 10;

/// The Rainbow texture's number, which [`super::raster`] colours.
pub const RAINBOW: u32 = 8;

/// The number past the made ones: the effect's Texture layer.
pub const LAYER: u32 = COUNT as u32 + 1;

/// Each texture's copies, halving from `SIZE` down to one texel.
pub const LEVELS: u32 = 9;

/// Floats one texture takes with all its copies.
pub const STRIDE: usize = (4 * SIZE * SIZE - 1) / 3;

/// Where copy `level` starts inside a texture.
fn level_start(level: u32) -> usize {
    (4 * SIZE * SIZE - ((4 * SIZE * SIZE) >> (2 * level))) / 3
}

/// Every texture, one after another, each `SIZE` by `SIZE` row by row and
/// then its smaller copies, every texel 0..1. Made on first use.
pub fn all() -> &'static [f32] {
    static MADE: OnceLock<Vec<f32>> = OnceLock::new();
    MADE.get_or_init(|| {
        let mut out = Vec::with_capacity(COUNT * STRIDE);
        let makers: [fn() -> Vec<f32>; COUNT] = [
            dust, smudge, caustic, rings, specks, hex_bokeh, scratches, rainbow, drops, grain,
        ];
        for make in makers {
            let mut t = make();
            let top = t.iter().copied().fold(0.0f32, f32::max).max(1e-6);
            t.iter_mut().for_each(|v| *v = (*v / top).clamp(0.0, 1.0));
            out.extend_from_slice(&t);
            // Each copy is the average of four texels of the one before.
            let mut side = SIZE;
            while side > 1 {
                let from = out.len() - side * side;
                side /= 2;
                for y in 0..side {
                    for x in 0..side {
                        let at =
                            |dx: usize, dy: usize| out[from + (2 * y + dy) * side * 2 + 2 * x + dx];
                        let v = 0.25 * (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1));
                        out.push(v);
                    }
                }
            }
        }
        out
    })
}

/// Texture `k` (1 to [`COUNT`]) at `x`, `y` in texels, wrapping, read
/// bilinearly between texel middles, from the copies that suit a sprite
/// where one pixel spans `footprint` texels. 0 for a `k` that names none.
pub fn sample(k: u32, x: f32, y: f32, footprint: f32) -> f32 {
    if k == 0 || k as usize > COUNT {
        return 0.0;
    }
    let lod = footprint.max(1.0).log2().min((LEVELS - 1) as f32);
    let level = lod.floor();
    let fine = sample_level(k, x, y, level as u32);
    let t = lod - level;
    if t > 0.0 {
        fine + (sample_level(k, x, y, level as u32 + 1) - fine) * t
    } else {
        fine
    }
}

fn sample_level(k: u32, x: f32, y: f32, level: u32) -> f32 {
    let texels = all();
    let base = (k as usize - 1) * STRIDE + level_start(level);
    let scale = (1u32 << level) as f32;
    let (x, y) = (x / scale - 0.5, y / scale - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let n = (SIZE >> level) as i32;
    let at = |dx: i32, dy: i32| {
        let tx = (x0 as i32 + dx).rem_euclid(n) as usize;
        let ty = (y0 as i32 + dy).rem_euclid(n) as usize;
        texels[base + ty * n as usize + tx]
    };
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * fx;
    let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * fx;
    top + (bottom - top) * fy
}

/// Value noise in 0..1 that repeats every `period` cells, so the textures
/// tile without a seam.
fn tiled(seed: u32, channel: u32, x: f32, y: f32, period: i32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (s(x - xf), s(y - yf));
    let corner = |dx: i32, dy: i32| {
        let cx = (xf as i32 + dx).rem_euclid(period);
        let cy = (yf as i32 + dy).rem_euclid(period);
        hash01(seed, channel, cx, cy, 0)
    };
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    top + (bottom - top) * v
}

/// A few octaves of [`tiled`], 0..1, starting at `cells` across the texture.
fn fbm(seed: u32, x: f32, y: f32, cells: i32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut total, mut cells) = (0.0, 1.0, 0.0, cells);
    for o in 0..octaves {
        let k = cells as f32 / SIZE as f32;
        sum += amp * tiled(seed, o, x * k, y * k, cells);
        total += amp;
        amp *= 0.5;
        cells *= 2;
    }
    sum / total
}

fn each(mut f: impl FnMut(f32, f32) -> f32) -> Vec<f32> {
    let mut out = vec![0.0; SIZE * SIZE];
    for (i, v) in out.iter_mut().enumerate() {
        *v = f((i % SIZE) as f32 + 0.5, (i / SIZE) as f32 + 0.5);
    }
    out
}

/// Add `f(distance)` round `(cx, cy)` out to `radius`, wrapping round the
/// edges.
fn splat(t: &mut [f32], cx: f32, cy: f32, radius: f32, f: impl Fn(f32) -> f32) {
    let r = radius.ceil() as i32 + 1;
    for dy in -r..=r {
        for dx in -r..=r {
            let (px, py) = (cx.floor() as i32 + dx, cy.floor() as i32 + dy);
            let d = (px as f32 + 0.5 - cx).hypot(py as f32 + 0.5 - cy);
            if d > radius {
                continue;
            }
            let (x, y) = (
                px.rem_euclid(SIZE as i32) as usize,
                py.rem_euclid(SIZE as i32) as usize,
            );
            t[y * SIZE + x] += f(d);
        }
    }
}

fn ramp(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Out-of-focus dust on the glass: a faint haze, many small soft motes and
/// a few big pale discs with brighter rims.
fn dust() -> Vec<f32> {
    let mut t = each(|x, y| 0.04 * fbm(11, x, y, 4, 4));
    for i in 0..360 {
        let (cx, cy) = (
            hash01(12, 0, i, 0, 0) * SIZE as f32,
            hash01(12, 1, i, 0, 0) * SIZE as f32,
        );
        let r = 0.8 + 2.6 * hash01(12, 2, i, 0, 0).powi(3);
        let b = 0.2 + 0.8 * hash01(12, 3, i, 0, 0);
        splat(&mut t, cx, cy, r * 2.0, |d| b * (-(d / r).powi(2)).exp());
    }
    for i in 0..24 {
        let (cx, cy) = (
            hash01(13, 0, i, 0, 0) * SIZE as f32,
            hash01(13, 1, i, 0, 0) * SIZE as f32,
        );
        let r = 6.0 + 12.0 * hash01(13, 2, i, 0, 0);
        let b = 0.08 + 0.12 * hash01(13, 3, i, 0, 0);
        splat(&mut t, cx, cy, r + 1.0, |d| {
            let inside = 1.0 - ramp(r - 1.5, r + 0.5, d);
            b * inside * (0.6 + 0.8 * ramp(0.5 * r, r, d))
        });
    }
    t
}

/// Wiped fingerprints of grease: long, curving streaks in patches.
fn smudge() -> Vec<f32> {
    each(|x, y| {
        // Bend the streaks with a slow field, then stretch them up and down.
        // The stretch is a whole number of tiles across, so the texture
        // still tiles both ways.
        let bend = fbm(21, x, y, 2, 3) * 60.0;
        let (u, v) = (x * 4.0 + bend * 4.0, y + bend * 1.2);
        let ridge = 1.0 - (2.0 * fbm(22, u, v, 2, 4) - 1.0).abs();
        let patch = ramp(0.35, 0.75, fbm(23, x, y, 2, 3));
        ridge.powi(6) * patch + 0.06 * patch
    })
}

/// The bright net light makes through rippled water: the edges between
/// cells, at two sizes.
fn caustic() -> Vec<f32> {
    let net = |seed: u32, cells: i32, x: f32, y: f32| {
        let size = SIZE as f32 / cells as f32;
        let (gx, gy) = ((x / size).floor() as i32, (y / size).floor() as i32);
        let (mut f1, mut f2) = (f32::MAX, f32::MAX);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (gx + dx, gy + dy);
                let (wx, wy) = (cx.rem_euclid(cells), cy.rem_euclid(cells));
                let px = (cx as f32 + hash01(seed, 0, wx, wy, 0)) * size;
                let py = (cy as f32 + hash01(seed, 1, wx, wy, 0)) * size;
                let d = (px - x).hypot(py - y) / size;
                if d < f1 {
                    f2 = f1;
                    f1 = d;
                } else if d < f2 {
                    f2 = d;
                }
            }
        }
        (-(f2 - f1) / 0.06).exp()
    };
    each(|x, y| {
        // A gentle wobble so the cells are not straight sided.
        let wx = x + 6.0 * (fbm(31, x, y, 4, 2) - 0.5);
        let wy = y + 6.0 * (fbm(32, x, y, 4, 2) - 0.5);
        0.7 * net(33, 6, wx, wy) + 0.4 * net(34, 12, wx, wy)
    })
}

/// Concentric rings that crowd together outwards, like light through a
/// coated element, fading at the rim.
fn rings() -> Vec<f32> {
    let half = SIZE as f32 * 0.5;
    each(|x, y| {
        let d = (x - half).hypot(y - half) / half;
        let a = (y - half).atan2(x - half);
        // The wobble grows outwards, so the middle stays round.
        let wobble = d * (0.015 * (a * 7.0).sin() + 0.02 * (fbm(41, x, y, 8, 2) - 0.5));
        let phase = 9.0 * (d + wobble).powi(2) * TAU;
        let band = (0.5 + 0.5 * phase.cos()).powi(2);
        band * (1.0 - ramp(0.75, 0.98, d)) * ramp(0.0, 0.6, d)
    })
}

/// Sparse sharp glints of grit, a few much brighter than the rest.
fn specks() -> Vec<f32> {
    let mut t = vec![0.0; SIZE * SIZE];
    for i in 0..220 {
        let (cx, cy) = (
            hash01(51, 0, i, 0, 0) * SIZE as f32,
            hash01(51, 1, i, 0, 0) * SIZE as f32,
        );
        let r = 0.5 + 0.9 * hash01(51, 2, i, 0, 0);
        let b = 0.1 + 0.9 * hash01(51, 3, i, 0, 0).powi(4);
        splat(&mut t, cx, cy, r * 2.5, |d| b * (-(d / r).powi(2)).exp());
    }
    t
}

/// The shortest way from `a` to `b` round a texture that wraps.
fn wrapped(a: f32, b: f32) -> f32 {
    let n = SIZE as f32;
    let d = (b - a).rem_euclid(n);
    if d > n * 0.5 {
        d - n
    } else {
        d
    }
}

/// Add `f(qx, qy)` over the square `reach` round `(cx, cy)`, wrapping, where
/// `qx`, `qy` is the texel's middle from the centre.
fn stamp(t: &mut [f32], cx: f32, cy: f32, reach: f32, f: impl Fn(f32, f32) -> f32) {
    let r = reach.ceil() as i32 + 1;
    for dy in -r..=r {
        for dx in -r..=r {
            let (px, py) = (cx.floor() as i32 + dx, cy.floor() as i32 + dy);
            let v = f(px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            let (x, y) = (
                px.rem_euclid(SIZE as i32) as usize,
                py.rem_euclid(SIZE as i32) as usize,
            );
            t[y * SIZE + x] += v;
        }
    }
}

/// Out-of-focus highlights through a six-bladed iris: hexagons of many
/// sizes, all turned the same way, each with a brighter rim.
fn hex_bokeh() -> Vec<f32> {
    let mut t = vec![0.0; SIZE * SIZE];
    let seg = TAU / 6.0;
    for i in 0..46 {
        let (cx, cy) = (
            hash01(61, 0, i, 0, 0) * SIZE as f32,
            hash01(61, 1, i, 0, 0) * SIZE as f32,
        );
        let r = 5.0 + 22.0 * hash01(61, 2, i, 0, 0).powi(2);
        let b = 0.15 + 0.6 * hash01(61, 3, i, 0, 0);
        stamp(&mut t, cx, cy, r * 1.2, |qx, qy| {
            let a = qy.atan2(qx) + seg * 0.5;
            let a = a - seg * (a / seg).floor() - seg * 0.5;
            let d = qx.hypot(qy) * a.cos() / (seg * 0.5).cos() / r;
            let inside = 1.0 - ramp(0.94, 1.02, d);
            b * inside * (0.55 + 0.45 * ramp(0.7, 0.98, d))
        });
    }
    t
}

/// Fine scratches on the glass: long hairlines, most running one way as a
/// wipe leaves them, a few deeper and brighter, each fading at its ends.
fn scratches() -> Vec<f32> {
    let mut t = each(|x, y| 0.03 * fbm(71, x, y, 8, 3));
    for i in 0..70 {
        let (cx, cy) = (
            hash01(72, 0, i, 0, 0) * SIZE as f32,
            hash01(72, 1, i, 0, 0) * SIZE as f32,
        );
        let a = if hash01(72, 2, i, 0, 0) < 0.7 {
            0.5 + 0.5 * (hash01(72, 3, i, 0, 0) - 0.5)
        } else {
            hash01(72, 3, i, 0, 0) * TAU
        };
        let len = 20.0 + 110.0 * hash01(72, 4, i, 0, 0);
        let width = 0.5 + 0.8 * hash01(72, 5, i, 0, 0).powi(3);
        let b = 0.15 + 0.85 * hash01(72, 6, i, 0, 0).powi(3);
        let (s, c) = a.sin_cos();
        for (k, v) in t.iter_mut().enumerate() {
            let (x, y) = ((k % SIZE) as f32 + 0.5, (k / SIZE) as f32 + 0.5);
            let (dx, dy) = (wrapped(cx, x), wrapped(cy, y));
            let along = (dx * c + dy * s) / (0.5 * len);
            if along.abs() >= 1.0 {
                continue;
            }
            let across = (dy * c - dx * s) / width;
            *v += b * (1.0 - along * along) * (-across * across).exp();
        }
    }
    t
}

/// A rainbow's bow: a broad soft band round the middle with two faint ones
/// inside it. [`super::raster`] colours it across the band.
fn rainbow() -> Vec<f32> {
    let half = SIZE as f32 * 0.5;
    each(|x, y| {
        let d = (x - half).hypot(y - half) / half;
        let band = |at: f32, width: f32| (-((d - at) / width).powi(2)).exp();
        band(0.75, 0.11) + 0.25 * band(0.5, 0.035) + 0.15 * band(0.43, 0.03)
    })
}

/// How far across the Rainbow texture's band `x`, `y` (texels) is, 0 inside
/// to 1 outside, for its colour.
pub fn rainbow_at(x: f32, y: f32) -> f32 {
    let half = SIZE as f32 * 0.5;
    let n = SIZE as f32;
    let d = (x.rem_euclid(n) - half).hypot(y.rem_euclid(n) - half) / half;
    ((d - 0.6) / 0.3).clamp(0.0, 1.0)
}

/// Rain on the glass: round drops of many sizes, a few running, each dim
/// inside with a bright rim below and a glint above.
fn drops() -> Vec<f32> {
    let mut t = vec![0.0; SIZE * SIZE];
    for i in 0..90 {
        let (cx, cy) = (
            hash01(81, 0, i, 0, 0) * SIZE as f32,
            hash01(81, 1, i, 0, 0) * SIZE as f32,
        );
        let r = 1.5 + 11.0 * hash01(81, 2, i, 0, 0).powi(3);
        // A running drop is drawn longer downwards.
        let run = if hash01(81, 3, i, 0, 0) < 0.12 {
            1.6
        } else {
            1.0
        };
        let b = 0.4 + 0.6 * hash01(81, 4, i, 0, 0);
        stamp(&mut t, cx, cy, r * run + 1.0, |qx, qy| {
            let qy = qy / run;
            let d = qx.hypot(qy) / r;
            let inside = 1.0 - ramp(0.9, 1.05, d);
            // The rim catches the light most on its lower side.
            let lower = 0.5 + 0.5 * qy / (qx.hypot(qy) + 1e-3);
            let rim = ramp(0.65, 0.95, d) * (0.3 + 0.7 * lower);
            let gx = qx + 0.35 * r;
            let gy = qy + 0.4 * r;
            let glint = (-(gx * gx + gy * gy) / (0.03 * r * r + 0.3)).exp();
            b * inside * (0.08 + 0.6 * rim + glint)
        });
    }
    t
}

/// Fine grain: small bright points thick across the whole square, so a
/// glow or a ghost through it sparkles faintly.
fn grain() -> Vec<f32> {
    each(|x, y| {
        let fine = tiled(91, 0, x, y, SIZE as i32);
        let coarse = fbm(92, x, y, 16, 3);
        fine.powi(4) * (0.5 + coarse)
    })
}

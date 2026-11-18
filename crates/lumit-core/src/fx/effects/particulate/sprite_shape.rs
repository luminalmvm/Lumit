//! The ready-made pictures Particulate can stamp in Sprite mode instead of a
//! layer's: a circle, a square, a triangle, a star and a cloud.
//!
//! Each is drawn on the host, white with its coverage in alpha (white times
//! alpha is the same picture premultiplied or not), and the particle's own
//! colour tints it exactly as it tints a layer's picture. The renderer uploads
//! each once a device, keeps it, and hands it to both render paths as a
//! sprite.

/// What Sprite mode stamps: the Sprite layer's picture, or one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpriteShape {
    Layer,
    Circle,
    Square,
    Triangle,
    Star,
    Cloudy,
}

impl SpriteShape {
    /// The Choice option labels, in code order. New shapes go at the end: a
    /// Choice is saved as its index.
    pub const OPTIONS: &'static [&'static str] =
        &["Layer", "Circle", "Square", "Triangle", "Star", "Cloudy"];

    /// The shape for a stored Choice index; anything unknown is Layer, the
    /// default.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => SpriteShape::Circle,
            2 => SpriteShape::Square,
            3 => SpriteShape::Triangle,
            4 => SpriteShape::Star,
            5 => SpriteShape::Cloudy,
            _ => SpriteShape::Layer,
        }
    }
}

/// The side of every ready-made picture, px. Large enough that a big particle
/// still has a clean edge; the stamp scales it to the particle's size.
pub const SIZE: u32 = 256;

/// The fewest and most points a Star sprite has.
pub const STAR_POINTS_MIN: u32 = 4;
pub const STAR_POINTS_MAX: u32 = 8;

/// How far in a star's inner corners sit, as a share of its outer radius.
const STAR_INNER: f32 = 0.45;

/// Samples a side per pixel for the hard-edged shapes' antialiasing.
const SS: u32 = 4;

/// What a picture is made by: the shape and, for a Star only, its points.
/// `None` for Layer, which has no picture of its own.
#[must_use]
pub fn key(shape: SpriteShape, star_points: u32) -> Option<(SpriteShape, u32)> {
    match shape {
        SpriteShape::Layer => None,
        SpriteShape::Star => Some((shape, star_points.clamp(STAR_POINTS_MIN, STAR_POINTS_MAX))),
        _ => Some((shape, 0)),
    }
}

/// The picture for a [`key`] (RGBA, [`SIZE`] × [`SIZE`]), drawn afresh on
/// every call, so the caller keeps what it uploads.
#[must_use]
pub fn picture((shape, n): (SpriteShape, u32)) -> Vec<f32> {
    let star = star_corners(n);
    let mut out = vec![0.0f32; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let a = match shape {
                SpriteShape::Cloudy => cloud(x, y),
                _ => {
                    let mut hit = 0;
                    for sy in 0..SS {
                        for sx in 0..SS {
                            // -1..1 across the picture, y down.
                            let u = ((x * SS + sx) as f32 + 0.5) / (SIZE * SS) as f32 * 2.0 - 1.0;
                            let v = ((y * SS + sy) as f32 + 0.5) / (SIZE * SS) as f32 * 2.0 - 1.0;
                            if inside(shape, &star, u, v) {
                                hit += 1;
                            }
                        }
                    }
                    hit as f32 / (SS * SS) as f32
                }
            };
            let d = ((y * SIZE + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&[a, a, a, a]);
        }
    }
    out
}

/// The shapes reach a hair short of the picture's edge, so the stamp's
/// bilinear tap, clamped at the edge, never smears a solid border outwards.
const REACH: f32 = 0.98;

fn inside(shape: SpriteShape, star: &[[f32; 2]], u: f32, v: f32) -> bool {
    match shape {
        SpriteShape::Circle => u * u + v * v <= REACH * REACH,
        SpriteShape::Square => u.abs() <= REACH && v.abs() <= REACH,
        SpriteShape::Triangle => {
            // Point up, base down, as wide as it is tall.
            let t = (v + REACH) / (2.0 * REACH); // 0 at the top, 1 at the base
            (0.0..=1.0).contains(&t) && u.abs() <= t * REACH
        }
        SpriteShape::Star => in_polygon(star, u, v),
        SpriteShape::Layer | SpriteShape::Cloudy => false,
    }
}

/// A star's corners, outer and inner in turn, the first straight up.
fn star_corners(n: u32) -> Vec<[f32; 2]> {
    if n == 0 {
        return Vec::new();
    }
    (0..2 * n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / n as f32;
            let r = if i % 2 == 0 {
                REACH
            } else {
                REACH * STAR_INNER
            };
            [r * a.cos(), r * a.sin()]
        })
        .collect()
}

fn in_polygon(c: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut j = c.len().wrapping_sub(1);
    for i in 0..c.len() {
        let ([xi, yi], [xj, yj]) = (c[i], c[j]);
        if (yi > y) != (yj > y) && x < xi + (y - yi) / (yj - yi) * (xj - xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A soft puff: a round falloff broken up by a few octaves of value noise, so
/// the edge billows instead of being a clean disc.
fn cloud(x: u32, y: u32) -> f32 {
    let u = (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
    let v = (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
    let mut f = 0.0;
    let mut amp = 0.5;
    let mut freq = 3.0;
    for octave in 0..4u32 {
        f += amp
            * value_noise(
                u * freq + 7.3 * octave as f32,
                v * freq - 3.1 * octave as f32,
            );
        amp *= 0.5;
        freq *= 2.0;
    }
    // f is about 0..1; it pushes the edge in and out and mottles the middle.
    let r = (u * u + v * v).sqrt() / REACH;
    let edge = 0.55 + 0.4 * f;
    let fall = 1.0 - smoothstep(edge - 0.35, edge, r);
    (fall * (0.65 + 0.35 * f)).clamp(0.0, 1.0)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |ix: f32, iy: f32| -> f32 {
        let mut n = (ix as i32 as u32).wrapping_mul(0x9E37_79B1)
            ^ (iy as i32 as u32).wrapping_mul(0x85EB_CA77);
        n ^= n >> 15;
        n = n.wrapping_mul(0x2C1B_3C6D);
        n ^= n >> 12;
        (n & 0xFFFF) as f32 / 65535.0
    };
    let top = h(x0, y0) + (h(x0 + 1.0, y0) - h(x0, y0)) * sx;
    let bot = h(x0, y0 + 1.0) + (h(x0 + 1.0, y0 + 1.0) - h(x0, y0 + 1.0)) * sx;
    top + (bot - top) * sy
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Every shape draws something in its middle and nothing in its corner,
    /// and Layer draws nothing of its own.
    #[test]
    fn layer_has_no_picture_and_the_others_do() {
        assert!(key(SpriteShape::Layer, 5).is_none());
        for code in 1..SpriteShape::OPTIONS.len() as u32 {
            let shape = SpriteShape::from_code(code);
            let p = picture(key(shape, 5).unwrap());
            let alpha = |x: u32, y: u32| p[((y * SIZE + x) * 4 + 3) as usize];
            assert_eq!(p.len(), (SIZE * SIZE * 4) as usize);
            assert!(
                alpha(SIZE / 2, SIZE / 2) > 0.5,
                "code {code} is empty in the middle"
            );
            assert_eq!(alpha(0, 0), 0.0, "code {code} fills its corner");
        }
    }
}

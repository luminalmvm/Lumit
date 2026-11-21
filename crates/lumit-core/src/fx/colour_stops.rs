//! A Gradient row's stops: the colours a ramp passes through between the two
//! colour rows it runs from and to.

use crate::fx::cpu::CURVE_TABLE;
use crate::shape::{GradientStop, MAX_GRADIENT_STOPS};

/// A Gradient row's stops at one time, in order along the ramp.
///
/// Fixed-size and `Copy` so it can live in the arena, as a curve does. A
/// stop's place and alpha are kept as 16-bit fractions, so sixteen stops are
/// no wider than a curve. Only the first `len` entries mean anything; the rest
/// are zero, so two equal rows compare and hash alike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourStops {
    rgb: [[f32; 3]; MAX_GRADIENT_STOPS],
    at_alpha: [[u16; 2]; MAX_GRADIENT_STOPS],
    len: u32,
}

const _: () = assert!(size_of::<ColourStops>() <= size_of::<super::params::CurvePoints>());

fn fraction(v: f64) -> u16 {
    let v = if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (v * f64::from(u16::MAX)).round() as u16
}

fn unfraction(q: u16) -> f32 {
    f32::from(q) / f32::from(u16::MAX)
}

impl ColourStops {
    /// No stops: the plain blend from one end to the other.
    pub const EMPTY: Self = Self {
        rgb: [[0.0; 3]; MAX_GRADIENT_STOPS],
        at_alpha: [[0; 2]; MAX_GRADIENT_STOPS],
        len: 0,
    };

    /// `stops` as they read at layer time `t`, as a shape's gradient reads
    /// them: the first sixteen, with any whose place is not a number left
    /// out, then sorted by place. A stop's alpha is its colour's alpha times
    /// its opacity.
    #[must_use]
    pub fn sanitised(stops: &[GradientStop], t: f64) -> Self {
        let mut read: Vec<(f64, &GradientStop)> = stops
            .iter()
            .take(MAX_GRADIENT_STOPS)
            .map(|s| (s.position.value_at(t), s))
            .filter(|(at, _)| at.is_finite())
            .collect();
        read.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = Self::EMPTY;
        for (i, (at, stop)) in read.into_iter().enumerate() {
            let c = stop.colour.0;
            let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
            out.rgb[i] = [finite(c[0]), finite(c[1]), finite(c[2])];
            let alpha = f64::from(c[3]) * stop.opacity.value_at(t) / 100.0;
            out.at_alpha[i] = [fraction(at / 100.0), fraction(alpha)];
            out.len += 1;
        }
        out
    }

    /// How many stops there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Each stop's place along the ramp, 0..1, and its colour with alpha.
    pub fn stops(&self) -> impl Iterator<Item = (f32, [f32; 4])> + '_ {
        self.rgb
            .iter()
            .zip(self.at_alpha.iter())
            .take(self.len())
            .map(|(c, q)| (unfraction(q[0]), [c[0], c[1], c[2], unfraction(q[1])]))
    }

    /// The length, then the live stops, so padding never feeds a key.
    pub fn feed_hash(&self, feed: &mut dyn FnMut(&[u8])) {
        feed(&self.len.to_le_bytes());
        for (c, q) in self.rgb.iter().zip(self.at_alpha.iter()).take(self.len()) {
            for v in c {
                feed(&v.to_le_bytes());
            }
            feed(&q[0].to_le_bytes());
            feed(&q[1].to_le_bytes());
        }
    }

    /// The ramp from `from` to `to` through these stops, baked to red, green,
    /// blue and alpha tables of [`CURVE_TABLE`] entries, or nothing when there
    /// are no stops. The ends are opaque, and it mixes in working space with a
    /// shape's gradient's rule: two stops at one place make a hard edge.
    #[must_use]
    pub fn tables(&self, from: [f32; 4], to: [f32; 4]) -> Vec<[f32; CURVE_TABLE]> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut points = Vec::with_capacity(self.len() + 2);
        points.push((0.0, [from[0], from[1], from[2], 1.0]));
        points.extend(self.stops());
        points.push((1.0, [to[0], to[1], to[2], 1.0]));
        let mut out = vec![[0.0; CURVE_TABLE]; 4];
        let mut piece = 0;
        for i in 0..CURVE_TABLE {
            let u = i as f32 / (CURVE_TABLE - 1) as f32;
            while piece + 2 < points.len() && u >= points[piece + 1].0 {
                piece += 1;
            }
            let (a_at, a) = points[piece];
            let (b_at, b) = points[piece + 1];
            let span = b_at - a_at;
            let v = if span > 0.0 {
                ((u - a_at) / span).clamp(0.0, 1.0)
            } else {
                1.0
            };
            for (k, table) in out.iter_mut().enumerate() {
                table[i] = a[k] + (b[k] - a[k]) * v;
            }
        }
        out
    }
}

impl Default for ColourStops {
    fn default() -> Self {
        Self::EMPTY
    }
}

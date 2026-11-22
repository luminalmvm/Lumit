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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::Property;
    use crate::model::LinearColour;

    fn stop(at: f64, colour: [f32; 4], opacity: f64) -> GradientStop {
        GradientStop {
            position: Property::fixed(at),
            colour: LinearColour(colour),
            opacity: Property::fixed(opacity),
        }
    }

    /// Stops are read in order of place, a place that is not a number is left
    /// out, and a stop's alpha is its colour's times its opacity.
    #[test]
    fn stops_are_read_in_order_with_their_alpha() {
        assert!(ColourStops::sanitised(&[], 0.0).is_empty());
        assert_eq!(ColourStops::default(), ColourStops::EMPTY);
        let read = ColourStops::sanitised(
            &[
                stop(80.0, [0.0, 0.0, 1.0, 1.0], 100.0),
                stop(f64::NAN, [1.0, 1.0, 1.0, 1.0], 100.0),
                stop(20.0, [1.0, f32::NAN, 0.0, 0.5], 50.0),
            ],
            0.0,
        );
        assert_eq!(read.len(), 2);
        let got: Vec<_> = read.stops().collect();
        assert!((got[0].0 - 0.2).abs() < 1e-4 && (got[1].0 - 0.8).abs() < 1e-4);
        assert_eq!(got[0].1[..3], [1.0, 0.0, 0.0]);
        assert!((got[0].1[3] - 0.25).abs() < 1e-4, "alpha is 0.5 times 50%");
        assert_eq!(got[1].1[..3], [0.0, 0.0, 1.0]);
        // Padding never feeds a key, and a different ramp does.
        let hash = |s: &ColourStops| {
            let mut bytes = Vec::new();
            s.feed_hash(&mut |b| bytes.extend_from_slice(b));
            bytes
        };
        assert_ne!(hash(&read), hash(&ColourStops::EMPTY));
        let one = ColourStops::sanitised(&[stop(20.0, [1.0, 0.0, 0.0, 1.0], 100.0)], 0.0);
        assert_ne!(hash(&read), hash(&one));
    }

    /// The baked ramp runs from one end to the other through the stops, with
    /// opaque ends and a hard edge where two stops share a place.
    #[test]
    fn the_ramp_runs_through_its_stops() {
        let from = [1.0, 0.0, 0.0, 0.2];
        let to = [0.0, 0.0, 1.0, 0.2];
        assert!(ColourStops::EMPTY.tables(from, to).is_empty());
        let mid = ColourStops::sanitised(&[stop(50.0, [0.0, 1.0, 0.0, 1.0], 100.0)], 0.0);
        let t = mid.tables(from, to);
        assert_eq!(t.len(), 4);
        let last = CURVE_TABLE - 1;
        assert_eq!([t[0][0], t[1][0], t[2][0], t[3][0]], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            [t[0][last], t[1][last], t[2][last], t[3][last]],
            [0.0, 0.0, 1.0, 1.0]
        );
        let half = last / 2;
        assert!(t[1][half] > 0.99, "green at the stop");
        assert!((t[0][half / 2] - 0.5).abs() < 0.02, "half way to it");
        let hard = ColourStops::sanitised(
            &[
                stop(50.0, [0.0, 1.0, 0.0, 1.0], 100.0),
                stop(50.0, [1.0, 1.0, 0.0, 1.0], 100.0),
            ],
            0.0,
        );
        let h = hard.tables(from, to);
        assert!(h[0][half - 4] < 0.1 && h[0][half + 4] > 0.9, "a hard edge");
    }
}

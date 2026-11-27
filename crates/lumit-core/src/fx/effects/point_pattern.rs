//! The patterns Vary points and Pick points number a stream's points by.
//!
//! Each point gets a number from 0 to 1: its place in the stream, its own
//! dice, a noise at its position, its distance from a centre, and so on.
//! Both effects read the same pattern, so they number a stream the same way.

use crate::fx::cpu;
use crate::fx::noise::value3;
use crate::fx::points::{self, Column, PointsStream};

/// The dice a Random pattern rolls and the lattice a Noise one reads, kept off
/// the numbers the producers use for their own jitter.
const RANDOM_ATTR: u32 = 32;
const NOISE_CHANNEL: u32 = 80;

/// What gives each point its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatternKind {
    /// Its place in the stream: 0 for the first point, 1 for the last.
    Index,
    /// A roll of its own dice, from its `id`, so the same number every frame.
    #[default]
    Random,
    /// A smooth noise read at its position.
    Noise,
    /// How far it is from Centre in the pattern's plane, as a share of
    /// Radius.
    Distance,
    /// How far through its life it is.
    Age,
    /// Always 1, so every point takes the whole of each change.
    Constant,
    /// Where it is from left to right: 0 at Centre X less Radius, 1 at Centre
    /// X plus Radius.
    Across,
    /// The same from top to bottom, about Centre Y.
    Down,
    /// How bright the picture under it is.
    Image,
    /// 1 if it is picked, 0 if not.
    Picked,
    /// The number it carries, held to 0 to 1.
    Number,
    /// Bands across the frame: 1 inside a band, 0 between.
    Stripes,
    /// A chequerboard: 1 and 0 in alternate squares.
    Checker,
    /// A wave that repeats over space and can travel over time.
    Waves,
    /// Where it is from near to far: 0 at Centre Z less Radius, 1 at Centre
    /// Z plus Radius.
    Depth,
}

impl PatternKind {
    /// The Choice option labels, in code order. A Choice is stored as its
    /// index, so a new pattern goes on the end.
    pub const OPTIONS: &'static [&'static str] = &[
        "Index", "Random", "Noise", "Distance", "Age", "Constant", "Across", "Down", "Image",
        "Picked", "Number", "Stripes", "Checker", "Waves", "Depth",
    ];

    /// The pattern for a stored Choice index. Anything unknown is Random.
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            0 => PatternKind::Index,
            2 => PatternKind::Noise,
            3 => PatternKind::Distance,
            4 => PatternKind::Age,
            5 => PatternKind::Constant,
            6 => PatternKind::Across,
            7 => PatternKind::Down,
            8 => PatternKind::Image,
            9 => PatternKind::Picked,
            10 => PatternKind::Number,
            11 => PatternKind::Stripes,
            12 => PatternKind::Checker,
            13 => PatternKind::Waves,
            14 => PatternKind::Depth,
            _ => PatternKind::Random,
        }
    }
}

/// A pattern and the rows it reads. Shared by Vary points and Pick points, so
/// both number a stream the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub seed: u32,
    /// Spatial wavelength of the noise, in the stream's own units.
    pub noise_scale: f32,
    /// How fast the noise evolves, and how many waves pass, per second.
    pub noise_speed: f32,
    /// Where Distance, Across, Down and Depth are measured from, and where
    /// the stripes, squares and waves start, in the stream's own units.
    pub centre: [f32; 3],
    /// The plane Distance, the stripes, the squares and the waves lie in:
    /// XY, XZ, YZ, or XYZ for a distance through all three.
    pub plane: u32,
    /// The distance that reads as 1.
    pub radius: f32,
    /// The number that reads as 1 under the Number pattern.
    pub number_range: f32,
    /// Which of a point's numbers the Number pattern reads, found on the
    /// stream [`value`](Self::value) is then asked about.
    pub number: Column,
    /// How far it is from one stripe, square or wave to the next, in the
    /// stream's own units.
    pub spacing: f32,
    /// How far the stripes and squares are turned about Centre, and the way
    /// the waves travel, degrees.
    pub angle: f32,
    /// The share of each Spacing a stripe fills, 0 to 1.
    pub width: f32,
    /// The shape of a wave: sine, triangle, square or sawtooth.
    pub wave: u32,
    /// The waves run out from Centre as rings.
    pub rings: bool,
}

impl Pattern {
    /// Point `i`'s number, `0..=1`, at layer time `t`.
    ///
    /// Everything but Index is keyed on the point itself, so a neighbour
    /// coming or going doesn't reshuffle it. `sampled` is the picture's
    /// colour under each point, which only Image reads. `axis` says which
    /// roll a Random or Noise pattern gives: 0 is the usual one, and the
    /// others let one effect push a point a different way along each axis.
    #[must_use]
    pub fn value(
        &self,
        s: &PointsStream,
        i: usize,
        t: f64,
        sampled: Option<&[[f32; 4]]>,
        axis: u32,
    ) -> f32 {
        let p = s.position.get(i).copied().unwrap_or([0.0; 3]);
        // 0 at the centre less the radius, 1 at the centre plus it.
        let along = |at: f32, centre: f32| (at - centre) / (2.0 * self.radius.max(1e-3)) + 0.5;
        // Where it is from Centre in the pattern's plane: across it, down it,
        // and the depth a distance through all three adds.
        let d = [0, 1, 2].map(|k| p[k] - self.centre[k]);
        let [x, y, deep] = match self.plane {
            1 => [d[0], d[2], 0.0],
            2 => [d[2], d[1], 0.0],
            3 => d,
            _ => [d[0], d[1], 0.0],
        };
        let far = x.hypot(y).hypot(deep);
        // Where it is from Centre in the pattern's own turned frame, along
        // Angle and then across it, counted in Spacings.
        let turned = || {
            let (sin, cos) = self.angle.to_radians().sin_cos();
            let k = 1.0 / self.spacing.max(1e-3);
            [(x * cos + y * sin) * k, (y * cos - x * sin) * k]
        };
        let on = |yes: bool| if yes { 1.0 } else { 0.0 };
        let v = match self.kind {
            PatternKind::Index => {
                // A single point reads as the first.
                let last = s.len().saturating_sub(1).max(1) as f32;
                i as f32 / last
            }
            PatternKind::Random => {
                let id = s.id.get(i).copied().unwrap_or(0);
                points::draw(self.seed, id, RANDOM_ATTR + axis)
            }
            PatternKind::Noise => {
                let k = 1.0 / self.noise_scale.max(1e-3);
                let z = p[2] * k + (t as f32) * self.noise_speed;
                value3(self.seed, NOISE_CHANNEL + axis, p[0] * k, p[1] * k, z, 0) * 0.5 + 0.5
            }
            PatternKind::Distance => far / self.radius.max(1e-3),
            PatternKind::Age => {
                let life = s.life.get(i).copied().unwrap_or(1.0);
                s.age.get(i).copied().unwrap_or(0.0) / life.max(1e-6)
            }
            PatternKind::Constant => 1.0,
            PatternKind::Across => along(p[0], self.centre[0]),
            PatternKind::Down => along(p[1], self.centre[1]),
            PatternKind::Depth => along(p[2], self.centre[2]),
            PatternKind::Image => {
                let c = sampled.and_then(|c| c.get(i)).copied().unwrap_or([0.0; 4]);
                // The picture is premultiplied, so the coverage is divided
                // back out first: a half-covered white pixel is white, not
                // grey. No coverage is no light.
                if c[3] > 0.0 {
                    (c[0] * cpu::LUMA[0] + c[1] * cpu::LUMA[1] + c[2] * cpu::LUMA[2]) / c[3]
                } else {
                    0.0
                }
            }
            PatternKind::Picked => {
                if s.picked(i) {
                    1.0
                } else {
                    0.0
                }
            }
            PatternKind::Number => {
                s.read(self.number, i).unwrap_or(0.0) / self.number_range.max(1e-3)
            }
            // At no Angle the stripes lie across, one below another.
            PatternKind::Stripes => on(turned()[1].rem_euclid(1.0) < self.width),
            PatternKind::Checker => {
                let [u, v] = turned();
                on((u.floor() + v.floor()).rem_euclid(2.0) < 1.0)
            }
            PatternKind::Waves => {
                let far = if self.rings {
                    far / self.spacing.max(1e-3)
                } else {
                    turned()[0]
                };
                // Less the time, so the waves move the way Angle points and
                // the rings move outwards.
                let x = (far - t as f32 * self.noise_speed).rem_euclid(1.0);
                match self.wave {
                    1 => 1.0 - (2.0 * x - 1.0).abs(),
                    2 => on(x < 0.5),
                    3 => x,
                    _ => 0.5 + 0.5 * (x * std::f32::consts::TAU).sin(),
                }
            }
        };
        // A NaN reads as 0 so it never reaches a position.
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

//! The two samplers, and the tables they read.
//!
//! In plain terms: a look-up table is a list of "this colour in, that colour
//! out" answers on a regular grid. A real pixel almost never lands on a grid
//! point, so the sampler has to make one up from the neighbours. For a curve
//! (one dimension) that is a straight line between two samples. For a cube
//! (three dimensions) Lumit uses **tetrahedral** interpolation: it splits the
//! little box around the pixel into six wedges, works out which wedge the pixel
//! is in, and blends that wedge's four corners. It costs about the same as the
//! eight-corner blend the LUT effect uses and keeps greys exactly grey, which is
//! why it is the film industry's reference and what our golden fixtures are
//! generated with.
//!
//! The formulation here is **binding**: the WGSL sampler WP3 writes must be this
//! arithmetic byte for byte, ties broken the same way, or preview stops equalling
//! export (K-031). It is copied from docs/impl/ocio.md §4.3, top branch first,
//! `≥` exactly as written.

use crate::error::{ColourError, Result};

/// Adobe's cap, and the one `lumit-core::lut` already enforces: at three `f32`
/// per point a 256³ cube is already ~200 MB (docs/14 §5, budgeted allocations).
pub const MAX_CUBE_SIZE: usize = 256;
/// The matching cap for a curve.
pub const MAX_CURVE_SIZE: usize = 65536;

/// A 3D colour cube: `size³` samples, red changing fastest, so the sample for
/// grid cell `(r, g, b)` lives at flat index `r + g·size + b·size²`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cube {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `data.len() == size³`.
    pub data: Vec<[f32; 3]>,
}

impl Cube {
    /// Build a cube, checking the size and the data length.
    pub fn new(
        what: &str,
        size: usize,
        domain_min: [f32; 3],
        domain_max: [f32; 3],
        data: Vec<[f32; 3]>,
    ) -> Result<Self> {
        if size < 2 {
            return Err(ColourError::Parse {
                what: what.to_string(),
                reason: format!("a 3D look-up table needs at least 2 points per axis, not {size}"),
            });
        }
        if size > MAX_CUBE_SIZE {
            return Err(ColourError::TableTooLarge {
                what: what.to_string(),
                size,
                limit: MAX_CUBE_SIZE,
            });
        }
        let expected = size * size * size;
        if data.len() != expected {
            return Err(ColourError::Parse {
                what: what.to_string(),
                reason: format!("expected {expected} samples but found {}", data.len()),
            });
        }
        Ok(Self {
            size,
            domain_min,
            domain_max,
            data,
        })
    }

    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        let i = r + g * self.size + b * self.size * self.size;
        // `size³ == data.len()` is an invariant of `new`, and every caller
        // clamps its indices to `size - 1`; the fallback keeps the no-panic
        // rule (docs/14 §4) true by construction rather than by argument.
        self.data.get(i).copied().unwrap_or([0.0; 3])
    }

    /// Sample the cube tetrahedrally (docs/impl/ocio.md §4.3 — binding).
    #[must_use]
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let last = self.size - 1;
        let mut i0 = [0_usize; 3];
        let mut i1 = [0_usize; 3];
        let mut f = [0.0_f32; 3];
        for c in 0..3 {
            let lo = self.domain_min[c];
            let hi = self.domain_max[c];
            let span = hi - lo;
            // A zero-span axis reads as 0 rather than dividing (the guard
            // docs/impl/lut.md §3 pinned for the LUT effect after K-271).
            let g = if span == 0.0 {
                0.0
            } else {
                (rgb[c] - lo) / span * last as f32
            };
            let g = if g.is_nan() {
                0.0
            } else {
                g.clamp(0.0, last as f32)
            };
            let base = g.floor();
            i0[c] = (base as usize).min(last);
            i1[c] = (i0[c] + 1).min(last);
            f[c] = g - base;
        }
        let (fr, fg, fb) = (f[0], f[1], f[2]);
        let c000 = self.at(i0[0], i0[1], i0[2]);
        let c111 = self.at(i1[0], i1[1], i1[2]);

        // Six tetrahedra, chosen by ordering the fractions. The `≥` and the
        // branch order are load-bearing: ties must break identically here and
        // in WGSL.
        let (a, b, c, wa, wb, wc) = if fr >= fg && fg >= fb {
            (
                self.at(i1[0], i0[1], i0[2]),
                self.at(i1[0], i1[1], i0[2]),
                c111,
                fr,
                fg,
                fb,
            )
        } else if fr >= fb && fb >= fg {
            (
                self.at(i1[0], i0[1], i0[2]),
                self.at(i1[0], i0[1], i1[2]),
                c111,
                fr,
                fb,
                fg,
            )
        } else if fb >= fr && fr >= fg {
            (
                self.at(i0[0], i0[1], i1[2]),
                self.at(i1[0], i0[1], i1[2]),
                c111,
                fb,
                fr,
                fg,
            )
        } else if fg >= fr && fr >= fb {
            (
                self.at(i0[0], i1[1], i0[2]),
                self.at(i1[0], i1[1], i0[2]),
                c111,
                fg,
                fr,
                fb,
            )
        } else if fg >= fb && fb >= fr {
            (
                self.at(i0[0], i1[1], i0[2]),
                self.at(i0[0], i1[1], i1[2]),
                c111,
                fg,
                fb,
                fr,
            )
        } else {
            (
                self.at(i0[0], i0[1], i1[2]),
                self.at(i0[0], i1[1], i1[2]),
                c111,
                fb,
                fg,
                fr,
            )
        };

        let mut out = [0.0_f32; 3];
        for (k, o) in out.iter_mut().enumerate() {
            *o = c000[k] + wa * (a[k] - c000[k]) + wb * (b[k] - a[k]) + wc * (c[k] - b[k]);
        }
        out
    }
}

/// Which way a curve runs, decided once at construction so inversion never has
/// to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slope {
    Rising,
    Falling,
    /// Neither: the channel doubles back, so it has no single inverse.
    Mixed,
}

/// A per-channel curve over one shared input domain: `data[i]` holds the red,
/// green and blue outputs for grid point `i`.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub domain: [f32; 2],
    /// `data.len() >= 2`.
    pub data: Vec<[f32; 3]>,
}

impl Curve {
    /// Build a curve, checking the length.
    pub fn new(what: &str, domain: [f32; 2], data: Vec<[f32; 3]>) -> Result<Self> {
        if data.len() < 2 {
            return Err(ColourError::Parse {
                what: what.to_string(),
                reason: format!("a curve needs at least 2 points, not {}", data.len()),
            });
        }
        if data.len() > MAX_CURVE_SIZE {
            return Err(ColourError::TableTooLarge {
                what: what.to_string(),
                size: data.len(),
                limit: MAX_CURVE_SIZE,
            });
        }
        Ok(Self { domain, data })
    }

    fn value(&self, i: usize, c: usize) -> f32 {
        self.data.get(i).map_or(0.0, |s| s[c])
    }

    /// Sample the curve: per channel, map into the grid, clamp, lerp two
    /// neighbours. Out-of-domain clamps to the end sample.
    #[must_use]
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let last = self.data.len() - 1;
        let span = self.domain[1] - self.domain[0];
        let mut out = [0.0_f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let g = if span == 0.0 {
                0.0
            } else {
                (rgb[c] - self.domain[0]) / span * last as f32
            };
            let g = if g.is_nan() {
                0.0
            } else {
                g.clamp(0.0, last as f32)
            };
            let base = g.floor();
            let i0 = (base as usize).min(last);
            let i1 = (i0 + 1).min(last);
            let f = g - base;
            let a = self.value(i0, c);
            *o = a + f * (self.value(i1, c) - a);
        }
        out
    }

    fn slope(&self, c: usize) -> Slope {
        let mut rising = false;
        let mut falling = false;
        for i in 1..self.data.len() {
            let d = self.value(i, c) - self.value(i - 1, c);
            if d > 0.0 {
                rising = true;
            } else if d < 0.0 {
                falling = true;
            }
        }
        match (rising, falling) {
            (true, false) => Slope::Rising,
            (false, true) => Slope::Falling,
            // Wholly flat has no inverse either — every output maps back to the
            // whole domain, so it is refused with the doubling-back case.
            _ => Slope::Mixed,
        }
    }

    /// Whether every channel rises consistently or falls consistently, which is
    /// exactly the condition for [`Curve::sample_inverse`] to mean anything.
    #[must_use]
    pub fn is_monotone(&self) -> bool {
        (0..3).all(|c| self.slope(c) != Slope::Mixed)
    }

    /// Refuse a curve that cannot be inverted, naming the file it came from.
    pub fn check_invertible(&self, path: &str) -> Result<()> {
        if self.is_monotone() {
            Ok(())
        } else {
            Err(ColourError::NonMonotoneCurve {
                path: path.to_string(),
            })
        }
    }

    /// Read the curve backwards: given an output, which input produced it.
    ///
    /// Bisection over the forward table (docs/impl/ocio.md §4.3), so the answer
    /// is the exact inverse of the forward lerp rather than an approximation.
    /// A flat run takes its **lower** edge, matching the reference. Values off
    /// either end clamp to that end of the domain. A channel that doubles back
    /// is refused at parse, not here, so this is total.
    #[must_use]
    pub fn sample_inverse(&self, rgb: [f32; 3]) -> [f32; 3] {
        let last = self.data.len() - 1;
        let step = (self.domain[1] - self.domain[0]) / last as f32;
        let mut out = [0.0_f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let rising = self.slope(c) != Slope::Falling;
            let y = rgb[c];
            let key = |i: usize| {
                if rising {
                    self.value(i, c)
                } else {
                    -self.value(i, c)
                }
            };
            let target = if rising { y } else { -y };
            if target <= key(0) {
                *o = self.domain[0];
                continue;
            }
            if target >= key(last) {
                *o = self.domain[1];
                continue;
            }
            // First index whose value reaches the target: bisection, so a flat
            // run at the target answers with its first (lower) point.
            let (mut lo, mut hi) = (0_usize, last);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if key(mid) < target {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            let i = lo;
            let vi = key(i);
            if vi == target || i == 0 {
                *o = self.domain[0] + step * i as f32;
                continue;
            }
            let prev = key(i - 1);
            let d = vi - prev;
            let t = if d == 0.0 { 0.0 } else { (target - prev) / d };
            *o = self.domain[0] + step * (i as f32 - 1.0 + t);
        }
        out
    }
}

impl From<lumit_core::lut::Lut3d> for Cube {
    fn from(l: lumit_core::lut::Lut3d) -> Self {
        Self {
            size: l.size,
            domain_min: l.domain_min,
            domain_max: l.domain_max,
            data: l.data,
        }
    }
}

impl From<lumit_core::lut::Lut1d> for Curve {
    fn from(l: lumit_core::lut::Lut1d) -> Self {
        // `.cube` gives a per-channel domain; a curve carries one. Real 1D
        // `.cube` files use the same domain on all three, and the red one is
        // what the grammar's `DOMAIN_MIN`/`MAX` first column says.
        Self {
            domain: [l.domain_min[0], l.domain_max[0]],
            data: l.data,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    /// A cube whose samples are the grid coordinates themselves: the identity.
    fn identity_cube(size: usize) -> Cube {
        let last = (size - 1) as f32;
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push([r as f32 / last, g as f32 / last, b as f32 / last]);
                }
            }
        }
        Cube::new("identity", size, [0.0; 3], [1.0; 3], data).expect("well-formed")
    }

    #[test]
    fn every_corner_is_returned_exactly() {
        let cube = identity_cube(5);
        for b in [0.0, 1.0] {
            for g in [0.0, 1.0] {
                for r in [0.0, 1.0] {
                    assert!(close(cube.sample([r, g, b]), [r, g, b], 0.0));
                }
            }
        }
    }

    #[test]
    fn the_neutral_axis_stays_neutral() {
        // Tetrahedral's headline property, and the reason it is preferred to
        // trilinear here: an identity cube returns greys exactly.
        let cube = identity_cube(9);
        for i in 0..=100 {
            let v = i as f32 / 100.0;
            let got = cube.sample([v, v, v]);
            assert!(close(got, [v, v, v], 1e-6), "at {v}: {got:?}");
            assert_eq!(got[0], got[1], "grey stayed grey at {v}");
            assert_eq!(got[1], got[2], "grey stayed grey at {v}");
        }
    }

    #[test]
    fn a_known_wedge_matches_the_written_formula() {
        // Size 2, so grid coordinates are the input and the fractions are the
        // sample point. Corner values chosen non-separable so a wrong wedge or
        // a transposed fetch shows up.
        let corners: [[f32; 3]; 8] = [
            [0.10, 0.20, 0.30], // 000
            [0.90, 0.15, 0.05], // 100
            [0.40, 0.80, 0.10], // 010
            [0.55, 0.35, 0.95], // 110
            [0.05, 0.60, 0.70], // 001
            [0.25, 0.45, 0.85], // 101
            [0.65, 0.05, 0.15], // 011
            [1.00, 0.90, 0.20], // 111
        ];
        let cube =
            Cube::new("wedge", 2, [0.0; 3], [1.0; 3], corners.to_vec()).expect("well-formed");
        // fr ≥ fg ≥ fb — the first branch.
        let (fr, fg, fb) = (0.8_f32, 0.5_f32, 0.2_f32);
        let (c000, c100, c110, c111) = (corners[0], corners[1], corners[3], corners[7]);
        let mut want = [0.0_f32; 3];
        for (k, w) in want.iter_mut().enumerate() {
            *w = c000[k]
                + fr * (c100[k] - c000[k])
                + fg * (c110[k] - c100[k])
                + fb * (c111[k] - c110[k]);
        }
        assert!(close(cube.sample([fr, fg, fb]), want, 1e-6));
    }

    #[test]
    fn out_of_domain_clamps_to_the_edge() {
        let cube = identity_cube(5);
        assert!(close(cube.sample([-4.0, 9.0, 0.5]), [0.0, 1.0, 0.5], 1e-6));
    }

    #[test]
    fn a_zero_span_axis_reads_as_zero_rather_than_dividing() {
        let cube = Cube::new(
            "flat",
            2,
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 1.0],
            vec![[0.0; 3]; 8],
        )
        .expect("well-formed");
        assert!(cube.sample([5.0, 0.5, 0.5]).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn an_oversized_cube_is_refused_not_allocated() {
        let err = Cube::new("huge", 512, [0.0; 3], [1.0; 3], Vec::new());
        assert!(matches!(err, Err(ColourError::TableTooLarge { .. })));
    }

    fn ramp_curve() -> Curve {
        Curve::new(
            "ramp",
            [0.0, 1.0],
            (0..5)
                .map(|i| {
                    let v = i as f32 / 4.0;
                    [v * v, v, v * 0.5]
                })
                .collect(),
        )
        .expect("well-formed")
    }

    #[test]
    fn a_curve_inverts_back_to_where_it_started() {
        let curve = ramp_curve();
        for i in 0..=40 {
            let x = i as f32 / 40.0;
            let y = curve.sample([x, x, x]);
            let back = curve.sample_inverse(y);
            assert!(close(back, [x, x, x], 1e-5), "at {x}: {back:?}");
        }
    }

    #[test]
    fn a_flat_run_inverts_to_its_lower_edge() {
        let curve = Curve::new(
            "plateau",
            [0.0, 1.0],
            vec![[0.0; 3], [0.5; 3], [0.5; 3], [0.5; 3], [1.0; 3]],
        )
        .expect("well-formed");
        // Grid points sit at 0, 0.25, 0.5, 0.75, 1; the plateau starts at 0.25.
        assert!(close(curve.sample_inverse([0.5; 3]), [0.25; 3], 1e-6));
    }

    #[test]
    fn a_curve_that_doubles_back_is_refused_by_name() {
        let curve = Curve::new("bumpy", [0.0, 1.0], vec![[0.0; 3], [1.0; 3], [0.5; 3]])
            .expect("well-formed");
        assert!(!curve.is_monotone());
        let err = curve.check_invertible("bumpy.spi1d");
        assert!(
            matches!(&err, Err(ColourError::NonMonotoneCurve { path }) if path == "bumpy.spi1d"),
            "{err:?}"
        );
    }

    #[test]
    fn a_falling_curve_inverts_too() {
        let curve = Curve::new(
            "falling",
            [0.0, 1.0],
            (0..9).map(|i| [1.0 - i as f32 / 8.0; 3]).collect(),
        )
        .expect("well-formed");
        assert!(curve.is_monotone());
        assert!(close(curve.sample_inverse([0.25; 3]), [0.75; 3], 1e-5));
    }
}

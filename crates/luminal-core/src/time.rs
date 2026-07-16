//! Rational time, per docs/impl/rational-time.md (binding).
//!
//! Invariants: always normalised (den > 0, gcd(|num|, den) == 1, zero is 0/1);
//! every intermediate multiplication in i128; construction is the only place
//! normalisation happens; no general f64 → Rational conversion.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("zero denominator")]
    ZeroDenominator,
    #[error("rational overflow")]
    Overflow,
    #[error("value is not finite or not representable on the grid")]
    NotRepresentable,
}

/// Exact rational number of seconds. See module docs for invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64, // > 0 always
}

fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

impl Rational {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const ONE: Self = Self { num: 1, den: 1 };

    /// The flick grid (1/705,600,000 s) — divides every common video and audio rate.
    pub const FLICK_DEN: i64 = 705_600_000;

    pub fn new(num: i64, den: i64) -> Result<Self, TimeError> {
        Self::from_i128(i128::from(num), i128::from(den))
    }

    /// Reduce in i128, then require the result to fit i64.
    pub fn from_i128(num: i128, den: i128) -> Result<Self, TimeError> {
        if den == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        let (mut n, mut d) = if den < 0 { (-num, -den) } else { (num, den) };
        if n == 0 {
            return Ok(Self::ZERO);
        }
        let g = gcd_i128(n, d);
        n /= g;
        d /= g;
        match (i64::try_from(n), i64::try_from(d)) {
            (Ok(num), Ok(den)) => Ok(Self { num, den }),
            _ => Err(TimeError::Overflow),
        }
    }

    #[inline]
    pub fn num(self) -> i64 {
        self.num
    }
    #[inline]
    pub fn den(self) -> i64 {
        self.den
    }
    #[inline]
    pub fn is_negative(self) -> bool {
        self.num < 0
    }
    #[inline]
    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, TimeError> {
        let num =
            i128::from(self.num) * i128::from(rhs.den) + i128::from(rhs.num) * i128::from(self.den);
        Self::from_i128(num, i128::from(self.den) * i128::from(rhs.den))
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, TimeError> {
        self.checked_add(rhs.checked_neg()?)
    }

    pub fn checked_neg(self) -> Result<Self, TimeError> {
        self.num
            .checked_neg()
            .map(|num| Self { num, den: self.den })
            .ok_or(TimeError::Overflow)
    }

    pub fn checked_mul(self, rhs: Self) -> Result<Self, TimeError> {
        Self::from_i128(
            i128::from(self.num) * i128::from(rhs.num),
            i128::from(self.den) * i128::from(rhs.den),
        )
    }

    pub fn checked_div(self, rhs: Self) -> Result<Self, TimeError> {
        if rhs.num == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        Self::from_i128(
            i128::from(self.num) * i128::from(rhs.den),
            i128::from(self.den) * i128::from(rhs.num),
        )
    }

    /// Evaluation-only conversion (docs/impl/rational-time.md §4).
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// The ONLY route from f64 back to rational: quantise onto an explicit grid,
    /// rounding half to even.
    pub fn from_f64_on_grid(x: f64, grid_den: i64) -> Result<Self, TimeError> {
        if grid_den <= 0 {
            return Err(TimeError::ZeroDenominator);
        }
        let scaled = x * grid_den as f64;
        if !scaled.is_finite() || scaled.abs() >= i64::MAX as f64 {
            return Err(TimeError::NotRepresentable);
        }
        let n = round_ties_even(scaled) as i64;
        Self::new(n, grid_den)
    }
}

fn round_ties_even(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    /// Cross-multiply in i128 — never in i64 (docs/impl/rational-time.md §2).
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let lhs = i128::from(self.num) * i128::from(other.den);
        let rhs = i128::from(other.num) * i128::from(self.den);
        lhs.cmp(&rhs)
    }
}

// Serialised as [num, den] per docs/10-FILE-FORMAT.md §1.1; deserialisation
// re-normalises through the constructor so invalid input cannot enter.
impl Serialize for Rational {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.num, self.den).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Rational {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (num, den) = <(i64, i64)>::deserialize(d)?;
        Self::new(num, den).map_err(serde::de::Error::custom)
    }
}

/// A span of time, shared across timebases. Non-negativity is by convention,
/// validated at model boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Duration(pub Rational);

macro_rules! timebase {
    ($(#[$doc:meta])* $T:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize,
        )]
        pub struct $T(pub Rational);

        impl $T {
            pub const ZERO: Self = Self(Rational::ZERO);

            pub fn add_dur(self, d: Duration) -> Result<Self, TimeError> {
                Ok(Self(self.0.checked_add(d.0)?))
            }
            pub fn sub_dur(self, d: Duration) -> Result<Self, TimeError> {
                Ok(Self(self.0.checked_sub(d.0)?))
            }
            /// self − earlier, as a span.
            pub fn delta(self, earlier: Self) -> Result<Duration, TimeError> {
                Ok(Duration(self.0.checked_sub(earlier.0)?))
            }
        }
    };
}

timebase!(
    /// Seconds within a media file or nested comp, before any retiming.
    SourceTime
);
timebase!(
    /// Seconds from the start of a clip within a Sequence layer.
    ClipTime
);
timebase!(
    /// Seconds from a layer's in point.
    LayerTime
);
timebase!(
    /// Seconds from the start of a composition.
    CompTime
);

/// Rational frame rate, e.g. 60000/1001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    num: u32,
    den: u32,
}

impl FrameRate {
    pub fn new(num: u32, den: u32) -> Result<Self, TimeError> {
        if num == 0 || den == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        Ok(Self { num, den })
    }

    pub fn fps(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    pub fn frame_duration(self) -> Result<Duration, TimeError> {
        Ok(Duration(Rational::new(
            i64::from(self.den),
            i64::from(self.num),
        )?))
    }

    /// Comp time of frame n (exact).
    pub fn time_of_frame(self, n: i64) -> Result<CompTime, TimeError> {
        Ok(CompTime(Rational::from_i128(
            i128::from(n) * i128::from(self.den),
            i128::from(self.num),
        )?))
    }

    /// The frame containing time t (floor).
    pub fn frame_at(self, t: CompTime) -> i64 {
        let num = i128::from(t.0.num()) * i128::from(self.num);
        let den = i128::from(t.0.den()) * i128::from(self.den);
        i64::try_from(num.div_euclid(den)).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn normalisation_and_equality() {
        assert_eq!(rat(1, 2), rat(2, 4));
        assert_eq!(rat(-1, 2), rat(1, -2));
        assert_eq!(rat(0, 5), Rational::ZERO);
        assert_eq!(rat(0, -5).den(), 1);
        assert!(Rational::new(1, 0).is_err());
    }

    #[test]
    fn hash_agrees_with_eq() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(rat(1, 2));
        assert!(set.contains(&rat(2, 4)));
    }

    #[test]
    fn ntsc_walk_three_hours_is_exact() {
        // 3 h at 30000/1001: frame 323,676 lands exactly back on rational time.
        let fr = FrameRate::new(30000, 1001).unwrap();
        let n = 3 * 3600 * 30000 / 1001;
        let t = fr.time_of_frame(n).unwrap();
        assert_eq!(fr.frame_at(t), n);
        // and the next flick-grid point still round-trips through f64 quantisation
        let q = Rational::from_f64_on_grid(t.0.to_f64(), Rational::FLICK_DEN).unwrap();
        assert!((q.to_f64() - t.0.to_f64()).abs() < 1e-9);
    }

    #[test]
    fn overflow_chain_stays_tame() {
        // 90000-den pts plus flicks, chained: denominators must stay bounded.
        let mut acc = Rational::ZERO;
        let pts = rat(1, 90000);
        let flick = rat(1, Rational::FLICK_DEN);
        for _ in 0..100_000 {
            acc = acc.checked_add(pts).unwrap();
            acc = acc.checked_add(flick).unwrap();
        }
        assert!(acc.den() <= Rational::FLICK_DEN);
    }

    proptest! {
        #[test]
        fn add_sub_round_trip(a in -1_000_000i64..1_000_000, b in 1i64..100_000,
                              c in -1_000_000i64..1_000_000, d in 1i64..100_000) {
            let x = rat(a, b);
            let y = rat(c, d);
            let back = x.checked_add(y).unwrap().checked_sub(y).unwrap();
            prop_assert_eq!(back, x);
        }

        #[test]
        fn canonical_after_every_op(a in -100_000i64..100_000, b in 1i64..10_000,
                                    c in -100_000i64..100_000, d in 1i64..10_000) {
            for r in [rat(a,b).checked_add(rat(c,d)).unwrap(),
                      rat(a,b).checked_mul(rat(c,d)).unwrap()] {
                prop_assert!(r.den() > 0);
                prop_assert_eq!(gcd_i128(r.num().into(), r.den().into()), if r.num()==0 {r.den().into()} else {1});
            }
        }

        #[test]
        fn ordering_matches_f64(a in -100_000i64..100_000, b in 1i64..10_000,
                                c in -100_000i64..100_000, d in 1i64..10_000) {
            let (x, y) = (rat(a,b), rat(c,d));
            if x.to_f64() < y.to_f64() - 1e-9 { prop_assert!(x < y); }
            if x.to_f64() > y.to_f64() + 1e-9 { prop_assert!(x > y); }
        }

        #[test]
        fn grid_round_trips_frames(n in 0i64..1_000_000) {
            // every frame index at 59.94 over ~4.6 h survives f64 → grid quantisation
            let fr = FrameRate::new(60000, 1001).unwrap();
            let t = fr.time_of_frame(n).unwrap();
            let q = Rational::from_f64_on_grid(t.0.to_f64(), Rational::FLICK_DEN).unwrap();
            prop_assert_eq!(fr.frame_at(CompTime(q)), n);
        }
    }
}

//! Turning After Effects' clock into Lumit's
//! ([docs/impl/rational-time.md](../../../../docs/impl/rational-time.md),
//! [ae-import.md](../../../../docs/impl/ae-import.md) §4).
//!
//! # In plain terms
//!
//! After Effects hands every time over as an ordinary decimal number of
//! seconds — `2.0400000000000001` for the frame a person thinks of as "frame 51
//! at 25 fps". Lumit does not store times that way: it stores an exact
//! fraction, so a walk of ten thousand frames lands on the same moment however
//! it was arrived at. So every time in a capture has to be read back as the
//! fraction it was meant to be.
//!
//! The rule is the impl note's, and it has two halves. If the number is within
//! a millionth of a frame, it *is* that frame, and the exact frame time is
//! used. If it is not — and a keyframe is perfectly entitled to sit between two
//! frames — the nearest thousandth-of-a-frame is used instead. **A key that is
//! not on a frame is never snapped onto one**: that would quietly re-time
//! somebody's animation, which is the one thing an importer must not do.

use lumit_core::time::{Duration, FrameRate, Rational};

/// The frame rate a composition imports at when the capture carries none.
pub(crate) const DEFAULT_FPS: f64 = 25.0;

/// The duration a composition imports with when the capture carries none.
pub(crate) const DEFAULT_DURATION: f64 = 10.0;

/// One composition's clock: its frame rate, plus the sub-frame grid an
/// off-frame time is quantised onto.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimeBase {
    pub rate: FrameRate,
    /// Denominator of the off-frame grid — `fps × 1000` per the impl note, so
    /// a thousand distinguishable positions inside every frame.
    grid_den: i64,
}

impl TimeBase {
    /// The clock for `rate`.
    pub(crate) fn new(rate: FrameRate) -> Self {
        let grid = (rate.fps() * 1000.0).round();
        let grid_den = if grid.is_finite() && (1.0..9e15).contains(&grid) {
            grid as i64
        } else {
            Rational::FLICK_DEN
        };
        Self { rate, grid_den }
    }

    /// The clock for a capture's decimal frame rate, or `None` when the number
    /// is absent or not a rate at all.
    pub(crate) fn of_fps(fps: Option<f64>) -> Option<Self> {
        frame_rate_of(fps?).map(Self::new)
    }

    /// The clock a composition falls back to when its own rate did not arrive
    /// ([`DEFAULT_FPS`]).
    pub(crate) fn fallback() -> Self {
        Self::new(FrameRate::FPS_25)
    }

    /// The composition's frame rate as Lumit stores it.
    pub(crate) fn rate(&self) -> FrameRate {
        self.rate
    }

    /// A capture time, exactly. Lands on the frame when it is within 1e-6 of
    /// one, and on the nearest sub-frame grid point otherwise.
    pub(crate) fn seconds(&self, t: f64) -> Rational {
        if !t.is_finite() {
            return Rational::ZERO;
        }
        let frames = t * self.rate.fps();
        let nearest = frames.round();
        if (frames - nearest).abs() <= 1e-6 && nearest.abs() < 9e15 {
            if let Ok(exact) = self.rate.time_of_frame(nearest as i64) {
                return exact.0;
            }
        }
        Rational::from_f64_on_grid(t, self.grid_den).unwrap_or(Rational::ZERO)
    }

    /// A capture span, on the same grid.
    pub(crate) fn duration(&self, d: f64) -> Duration {
        Duration(self.seconds(d))
    }
}

/// The exact rate behind a decimal frame rate.
///
/// After Effects reports 23.976023976023978 for what is really 24000/1001, and
/// a project that stored the decimal would drift a frame every twenty minutes.
/// So an NTSC-family rate is recognised first (the decimal times 1001/1000
/// comes out whole), then a whole rate, and anything else is kept to a
/// thousandth — which covers the hand-typed rates AE also allows.
fn frame_rate_of(fps: f64) -> Option<FrameRate> {
    if !fps.is_finite() || fps <= 0.0 || fps > 10_000.0 {
        return None;
    }
    // The tolerance is loose (a thousandth of a frame) because the decimal
    // arrives rounded — a capture may say 29.97 rather than the full
    // 29.970029970029970. It is still far tighter than the gap to any whole
    // rate: 25 × 1.001 misses a whole number by 0.025, twenty-five times over.
    let ntsc = fps * 1.001;
    if (ntsc - ntsc.round()).abs() < 1e-3 && ntsc.round() >= 1.0 {
        return FrameRate::new((ntsc.round() as u32).saturating_mul(1000), 1001).ok();
    }
    if (fps - fps.round()).abs() < 1e-9 {
        return FrameRate::new(fps.round() as u32, 1).ok();
    }
    FrameRate::new((fps * 1000.0).round() as u32, 1000).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **An NTSC rate comes back as the fraction it always was.**
    ///
    /// The decimal After Effects reports is 24000/1001 rounded to a double.
    /// Storing that decimal would put Lumit's frame 43,200 (half an hour) a
    /// whole frame away from AE's, which is the kind of drift nobody notices
    /// until the audio is out.
    #[test]
    fn an_ntsc_frame_rate_is_recovered_exactly() {
        let tb = TimeBase::of_fps(Some(24000.0 / 1001.0)).expect("a rate");
        assert_eq!((tb.rate().num(), tb.rate().den()), (24000, 1001));

        let tb = TimeBase::of_fps(Some(29.97)).expect("a rate");
        assert_eq!((tb.rate().num(), tb.rate().den()), (30000, 1001));

        let tb = TimeBase::of_fps(Some(25.0)).expect("a rate");
        assert_eq!((tb.rate().num(), tb.rate().den()), (25, 1));
    }

    /// **A time on a frame lands exactly on it; a time between two frames
    /// stays between them.**
    ///
    /// The impl note's rule, and the one that decides whether an imported
    /// animation still reads the way its author drew it. Snapping an off-frame
    /// key would be a silent re-time.
    #[test]
    fn an_off_frame_key_is_not_snapped_onto_a_frame() {
        let tb = TimeBase::of_fps(Some(25.0)).expect("a rate");

        // On a frame: exactly 51/25, whatever the decimal's last bit says.
        let on = tb.seconds(51.0 / 25.0);
        assert_eq!(on, Rational::new(51, 25).unwrap());

        // Half a frame late: still half a frame late.
        let between = tb.seconds(51.5 / 25.0);
        assert!(between > Rational::new(51, 25).unwrap());
        assert!(between < Rational::new(52, 25).unwrap());
        assert!((between.to_f64() - 51.5 / 25.0).abs() < 1e-6);
    }

    /// **An absent or nonsensical rate is not a rate.** The caller substitutes
    /// a default and says so in the report, rather than dividing by zero.
    #[test]
    fn a_missing_or_impossible_frame_rate_is_refused() {
        assert!(TimeBase::of_fps(None).is_none());
        assert!(TimeBase::of_fps(Some(0.0)).is_none());
        assert!(TimeBase::of_fps(Some(-25.0)).is_none());
        assert!(TimeBase::of_fps(Some(f64::NAN)).is_none());
    }
}

//! Time in a vector export: Lumit's seconds as the output's frames.
//!
//! A Lottie counts in frames of its own rate, which need not be the comp's:
//! a key at 1.25 s is frame 37.5 at 30 fps and is written as that. Nothing
//! is snapped to whole frames except the samples taken of what has to be
//! baked, which fall on the output's own frames so the file is exact where a
//! player draws.
//!
//! Authoritative time stays rational in the document; this module turns it
//! into the output's float frames at the leaf, once, as the rules ask.

use lumit_core::model::Composition;
use lumit_core::time::FrameRate;

use crate::settings::{Settings, Span};

/// The output's clock: its rate, and the comp span written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timebase {
    /// Output frames per second.
    pub fps: f64,
    /// The rate as a fraction, for anything that has to name it exactly.
    pub rate: FrameRate,
    /// The first comp second written.
    pub start: f64,
    /// The comp second the file stops before.
    pub end: f64,
}

impl Timebase {
    /// The clock `settings` ask of `comp`.
    #[must_use]
    pub fn of(comp: &Composition, settings: &Settings) -> Self {
        let rate = settings.frame_rate.unwrap_or(comp.frame_rate);
        let whole = (0.0, comp.duration.0.to_f64());
        let (start, end) = match (settings.span, comp.work_area) {
            (Span::WorkArea, Some((a, b))) => (a.0.to_f64(), b.0.to_f64()),
            _ => whole,
        };
        // A work area that has drifted outside the comp (a comp shortened
        // after it was set) is held to the comp, and an empty one is the whole
        // comp, so the file always has at least one frame to play.
        let start = start.clamp(whole.0, whole.1);
        let end = end.clamp(whole.0, whole.1);
        let (start, end) = if end > start { (start, end) } else { whole };
        Self {
            fps: rate.fps(),
            rate,
            start,
            end,
        }
    }

    /// Seconds as output frames.
    #[must_use]
    pub fn frame(&self, seconds: f64) -> f64 {
        seconds * self.fps
    }

    /// Output frames as seconds.
    #[must_use]
    pub fn seconds(&self, frame: f64) -> f64 {
        if self.fps > 0.0 {
            frame / self.fps
        } else {
            0.0
        }
    }

    /// The first frame written (Lottie `ip`).
    #[must_use]
    pub fn in_frame(&self) -> f64 {
        self.frame(self.start)
    }

    /// The frame the file stops before (Lottie `op`).
    #[must_use]
    pub fn out_frame(&self) -> f64 {
        self.frame(self.end)
    }

    /// How many frames a player shows, at least one.
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        let n = (self.out_frame() - self.in_frame()).ceil();
        if n.is_finite() && n >= 1.0 {
            // A comp is hours at most; u32 frames is years at any rate.
            n.min(f64::from(u32::MAX)) as u32
        } else {
            1
        }
    }

    /// The output frames from `from` to `to` comp seconds, both ends kept,
    /// on this clock's grid: every whole output frame between them, plus the
    /// two ends themselves when they fall between frames. What a property
    /// that has to be baked is sampled at.
    #[must_use]
    pub fn grid(&self, from: f64, to: f64) -> Vec<f64> {
        let (a, b) = (self.frame(from), self.frame(to));
        if !(a.is_finite() && b.is_finite()) || b < a {
            return Vec::new();
        }
        let mut out = vec![a];
        let mut f = a.floor() + 1.0;
        // Bounded: a property is sampled across one layer's span, and the
        // cap keeps a runaway span (a typo of an hour at 240 fps) finite.
        let cap = a + MAX_SAMPLES as f64;
        while f < b && f < cap {
            out.push(f);
            f += 1.0;
        }
        if b > a {
            out.push(b.min(cap));
        }
        out
    }
}

/// The most samples one baked property takes: about an hour at 60 fps.
pub const MAX_SAMPLES: usize = 216_000;

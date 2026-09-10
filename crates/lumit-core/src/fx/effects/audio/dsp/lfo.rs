//! A low-frequency oscillator and the running phase that drives it.
//!
//! # In plain terms
//!
//! The phase lives in the effect's own state and moves one frame at a time at
//! whatever this block's Rate row says. That is what lets Rate be keyframed:
//! the cycle carries on from where it stood and only its speed changes. A
//! phase worked out from the frame index instead would jump every time the
//! rate moved, by further the deeper into the run the block sat.
//!
//! Nothing here reads wall time and nothing carries over from a previous
//! bake, so a preview and an export agree sample for sample and a second bake
//! is bit-identical (docs/impl/audio-effects.md §3).
//!
//! Every shape rises through zero at phase zero, so two of them at the same
//! rate stay in step and a stereo pair offset by 180 degrees is exactly
//! opposite.

use std::f64::consts::TAU;

/// How the LFO's cycle is shaped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The plain sine: what a vibrato and most tremolos want.
    Sine,
    /// Straight ramps up and down.
    Triangle,
    /// Two levels with a short slope between them. The slope is
    /// [`SQUARE_SLEW_FRAMES`] long, because a true step is a click.
    Square,
}

/// Frames the square's edge is slewed over. Short enough that it still reads
/// as a square, long enough that the edge does not click.
pub const SQUARE_SLEW_FRAMES: f64 = 8.0;

/// Where an oscillator stands, in cycles from 0 up to 1.
///
/// Held in the processor's state and stepped a frame at a time, so a run
/// split at a block edge with the state carried lands exactly where the whole
/// run would have put it. One phase serves however many oscillators an effect
/// runs: each applies its own offset when it reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phase(f64);

impl Phase {
    /// The phase `offset_deg` further along, from 0 up to 1.
    #[must_use]
    pub fn along(self, offset_deg: f64) -> f64 {
        let at = (self.0 + offset_deg / 360.0).rem_euclid(1.0);
        if at.is_finite() {
            at
        } else {
            0.0
        }
    }

    /// One frame on at `rate_hz`.
    ///
    /// A rate driven to something silly would otherwise poison the phase for
    /// the rest of the bake, so a step that is not a finite number leaves it
    /// where it stood.
    pub fn advance(&mut self, rate_hz: f64, sample_rate: f64) {
        let next = (self.0 + rate_hz / sample_rate.max(1.0)).rem_euclid(1.0);
        if next.is_finite() {
            self.0 = next;
        }
    }
}

/// The triangle at a phase: 0 rising, 1 at a quarter, -1 at three quarters.
fn triangle(phase: f64) -> f64 {
    if phase < 0.25 {
        4.0 * phase
    } else if phase < 0.75 {
        2.0 - 4.0 * phase
    } else {
        4.0 * phase - 4.0
    }
}

/// The square at a phase, with both edges slewed over `width` cycles.
fn square(phase: f64, width: f64) -> f64 {
    let half = (width * 0.5).clamp(1e-9, 0.24);
    // Signed distance to each edge, the wrap taken into account.
    let to_rise = if phase > 0.5 { phase - 1.0 } else { phase };
    let to_fall = phase - 0.5;
    if to_rise.abs() < half {
        to_rise / half
    } else if to_fall.abs() < half {
        -to_fall / half
    } else if phase < 0.5 {
        1.0
    } else {
        -1.0
    }
}

/// A shape, a rate and a phase offset. Holds no phase of its own, so it is
/// built afresh at each block's start from that block's rows.
///
/// The rate is kept only for the square, whose slew is a fixed number of
/// frames and so a share of the cycle that depends on how fast the cycle
/// runs.
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    pub shape: Shape,
    pub rate_hz: f64,
    pub offset_deg: f64,
}

impl Lfo {
    /// An oscillator at `rate_hz`, its cycle read `offset_deg` along.
    #[must_use]
    pub fn new(shape: Shape, rate_hz: f64, offset_deg: f64) -> Self {
        Self {
            shape,
            rate_hz,
            offset_deg,
        }
    }

    /// The value at `phase`, from -1 to 1.
    #[must_use]
    pub fn value(&self, phase: Phase, sample_rate: f64) -> f64 {
        let at = phase.along(self.offset_deg);
        match self.shape {
            Shape::Sine => (TAU * at).sin(),
            Shape::Triangle => triangle(at),
            Shape::Square => {
                let sample_rate = sample_rate.max(1.0);
                square(at, SQUARE_SLEW_FRAMES * self.rate_hz.abs() / sample_rate)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// One run of `frames` frames at a steady rate, every value kept.
    fn walk(lfo: &Lfo, frames: usize) -> Vec<f64> {
        let mut phase = Phase::default();
        (0..frames)
            .map(|_| {
                let value = lfo.value(phase, RATE);
                phase.advance(lfo.rate_hz, RATE);
                value
            })
            .collect()
    }

    #[test]
    fn a_cycle_later_is_the_same_sine_and_the_same_bits() {
        let lfo = Lfo::new(Shape::Sine, 1.0, 0.0);
        let run = walk(&lfo, 72_000);
        for index in [0, 1, 17, 4_321, 23_999] {
            let (here, there) = (run[index], run[index + 48_000]);
            assert!((here - there).abs() < 1e-9, "{index}: {here} vs {there}");
            let sine = (TAU * index as f64 / RATE).sin();
            assert!((here - sine).abs() < 1e-9, "{index}: {here} vs {sine}");
        }
        // The same walk twice is the same numbers.
        assert_eq!(run, walk(&lfo, 72_000));
    }

    #[test]
    fn every_shape_rises_through_zero_and_stays_inside_one() {
        for shape in [Shape::Sine, Shape::Triangle, Shape::Square] {
            let run = walk(&Lfo::new(shape, 2.0, 0.0), 48_000);
            assert!(run[0].abs() < 1e-9, "{shape:?} starts off zero");
            assert!(run[200] > 0.0, "{shape:?} should rise first");
            for v in &run {
                assert!((-1.0..=1.0).contains(v), "{shape:?} reached {v}");
            }
        }
    }

    #[test]
    fn the_triangle_and_the_square_hit_their_corners() {
        let run = walk(&Lfo::new(Shape::Triangle, 1.0, 0.0), 48_000);
        assert!((run[12_000] - 1.0).abs() < 1e-9);
        assert!(run[24_000].abs() < 1e-9);
        assert!((run[36_000] + 1.0).abs() < 1e-9);

        // The square is flat between its edges and slewed across them.
        let run = walk(&Lfo::new(Shape::Square, 1.0, 0.0), 48_000);
        assert!((run[12_000] - 1.0).abs() < 1e-12);
        assert!((run[36_000] + 1.0).abs() < 1e-12);
        // The slew is short, so the last bit of a carried phase is worth a
        // good deal more here than it is out on the flat.
        assert!(run[24_000].abs() < 1e-6, "the fall should cross zero");
        assert!(run[24_002] < -0.4, "the fall should be short");
    }

    #[test]
    fn an_offset_moves_the_cycle_and_a_silly_rate_does_not_break_it() {
        let plain = Lfo::new(Shape::Sine, 3.0, 0.0);
        let flipped = Lfo::new(Shape::Sine, 3.0, 180.0);
        let mut phase = Phase::default();
        for index in 0..8_000 {
            let sum = plain.value(phase, RATE) + flipped.value(phase, RATE);
            assert!(sum.abs() < 1e-12, "{index} summed to {sum}");
            phase.advance(3.0, RATE);
        }
        // A rate that is not a number, and a sample rate of nothing, leave a
        // phase something can still be read from.
        phase.advance(f64::NAN, RATE);
        phase.advance(1.0, 0.0);
        assert!(phase.along(0.0).is_finite());
    }

    #[test]
    fn a_rate_that_moves_leaves_the_cycle_where_it_stood() {
        // Ten seconds into a run, an automated Rate row goes from 5 Hz to
        // 6 Hz. A phase read from the frame index would jump by the index
        // times the change over the sample rate, which is fifty cycles here;
        // a carried one steps by one frame's worth of the new rate.
        let lfo = Lfo::new(Shape::Sine, 5.0, 0.0);
        let mut phase = Phase::default();
        for _ in 0..10 * 48_000 {
            phase.advance(5.0, RATE);
        }
        let before = lfo.value(phase, RATE);
        phase.advance(6.0, RATE);
        let after = lfo.value(phase, RATE);
        let step = after - before;
        assert!(step.abs() < 1e-3, "the cycle jumped by {step}");
    }
}

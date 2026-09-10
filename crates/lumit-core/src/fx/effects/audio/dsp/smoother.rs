//! A one-pole parameter smoother: the ramp that stops a knob zippering.
//!
//! # In plain terms
//!
//! A baked value changes once a block, so a gain moved by hand steps 512
//! frames at a time and the step is heard as a click. Feeding the step
//! through a one pole turns it into a short ramp instead. The time constant
//! is in milliseconds, so the same setting sounds the same at 44.1 and at
//! 96 kHz.

/// The one-pole coefficient for a time constant of `ms` at `rate` frames a
/// second: how much of the distance to the target survives one frame. Zero or
/// less means no smoothing at all, which is what a row wanting the raw value
/// asks for.
#[must_use]
pub fn coeff_of_ms(ms: f64, rate: f64) -> f64 {
    if ms <= 0.0 || rate <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * rate)).exp()
    }
}

/// A value on its way to a target.
#[derive(Clone, Copy, Debug)]
pub struct Smoother {
    coeff: f64,
    target: f64,
    value: f64,
}

impl Smoother {
    /// A smoother sitting at `value`, covering about 63% of any new distance
    /// in `ms`.
    #[must_use]
    pub fn new(ms: f64, rate: f64, value: f64) -> Self {
        Self {
            coeff: coeff_of_ms(ms, rate),
            target: value,
            value,
        }
    }

    /// A new time constant. Whatever is in flight keeps going.
    pub fn set_time(&mut self, ms: f64, rate: f64) {
        self.coeff = coeff_of_ms(ms, rate);
    }

    /// Aim at a new value: what a block start does with a baked row.
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    /// What it is aiming at, arrived or not.
    #[must_use]
    pub fn target(&self) -> f64 {
        self.target
    }

    /// Where it stands, without moving it.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    /// One frame along.
    pub fn step(&mut self) -> f64 {
        self.value = self.target + (self.value - self.target) * self.coeff;
        self.value
    }

    /// Jump to a value and aim at it. An effect does this in its first block,
    /// so a bake does not open with a ramp up from zero.
    pub fn snap(&mut self, value: f64) {
        self.target = value;
        self.value = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn run(frames: usize) -> Vec<f64> {
        let mut s = Smoother::new(10.0, RATE, 0.0);
        s.set_target(1.0);
        (0..frames).map(|_| s.step()).collect()
    }

    #[test]
    fn a_snapped_smoother_run_twice_gives_the_same_ramp() {
        assert_eq!(run(2_000), run(2_000));
    }

    #[test]
    fn one_time_constant_covers_most_of_the_distance() {
        let mut s = Smoother::new(10.0, RATE, 0.0);
        s.set_target(1.0);
        let frames = (0.010 * RATE) as usize;
        for _ in 0..frames {
            s.step();
        }
        // A one pole is at 1 - 1/e after its time constant.
        assert!((s.value() - 0.632).abs() < 0.005, "got {}", s.value());
    }

    #[test]
    fn snap_arrives_at_once_and_a_zero_time_never_ramps() {
        let mut s = Smoother::new(50.0, RATE, 0.0);
        s.snap(0.25);
        assert_eq!(s.value(), 0.25);
        assert_eq!(s.step(), 0.25);

        s.set_time(0.0, RATE);
        s.set_target(-1.0);
        assert_eq!(s.step(), -1.0);
    }

    #[test]
    fn a_silly_rate_does_not_stall_the_ramp() {
        assert_eq!(coeff_of_ms(10.0, 0.0), 0.0);
        assert_eq!(coeff_of_ms(-1.0, RATE), 0.0);
    }
}

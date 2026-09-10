//! A topology-preserving state-variable filter: low, band and high at once.
//!
//! # In plain terms
//!
//! The wah sweeps its filter every sample, and a biquad whose coefficients
//! are rebuilt that often is not well behaved while it moves: the recursion
//! holds the old frequency's answer and the new frequency's coefficients at
//! the same time, which rings. This form keeps the topology of the analogue
//! circuit instead. The cutoff is a coefficient on two integrators rather
//! than part of the recursion, so it can be moved every sample and the filter
//! stays where it is put.
//!
//! It hands back all three outputs, because they cost nothing extra and an
//! effect that wants a band pass usually wants the low as well. This is
//! Simper's zero-delay-feedback form of Zavalishin's filter.

/// One sample's three answers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

/// Resonance is held between these. Below the first the filter is barely a
/// filter, above the second it rings for minutes.
const Q_MIN: f64 = 0.05;
const Q_MAX: f64 = 100.0;

/// Two integrators, their coefficients, and nothing else.
#[derive(Clone, Copy, Debug)]
pub struct Svf {
    a1: f64,
    a2: f64,
    a3: f64,
    k: f64,
    ic1: f64,
    ic2: f64,
}

impl Svf {
    /// A filter at this cutoff and resonance, starting from silence.
    #[must_use]
    pub fn new(cutoff_hz: f64, q: f64, rate: f64) -> Self {
        let mut svf = Self {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            k: 1.0,
            ic1: 0.0,
            ic2: 0.0,
        };
        svf.set(cutoff_hz, q, rate);
        svf
    }

    /// A new cutoff and resonance. Cheap enough to call every sample, which
    /// is what a swept filter does. Both are held inside their range, so a
    /// row driven through zero or past Nyquist does not blow the filter up.
    pub fn set(&mut self, cutoff_hz: f64, q: f64, rate: f64) {
        let rate = rate.max(1.0);
        let cutoff = cutoff_hz.clamp(1.0, rate * 0.49);
        let q = q.clamp(Q_MIN, Q_MAX);
        let g = (std::f64::consts::PI * cutoff / rate).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Forget the state. The next sample starts from silence.
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    /// One sample in, three out.
    pub fn process(&mut self, sample: f32) -> SvfOut {
        let v0 = f64::from(sample);
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut {
            low: v2 as f32,
            band: v1 as f32,
            high: (v0 - self.k * v1 - v2) as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const RATE: f64 = 48_000.0;

    /// The settled gain of one output at one frequency, measured with a sine
    /// over a whole number of cycles.
    fn measured_gain(cutoff: f64, q: f64, freq: f64, pick: fn(SvfOut) -> f32) -> f64 {
        let mut svf = Svf::new(cutoff, q, RATE);
        let cycle = (RATE / freq).round().max(1.0) as usize;
        let settle = cycle * 60;
        let window = cycle * 60;
        let mut input = 0.0;
        let mut output = 0.0;
        for n in 0..settle + window {
            let x = (TAU * freq * n as f64 / RATE).sin();
            let y = f64::from(pick(svf.process(x as f32)));
            if n >= settle {
                input += x * x;
                output += y * y;
            }
        }
        (output / input).sqrt()
    }

    #[test]
    fn reset_then_the_same_input_twice_gives_the_same_output() {
        let mut svf = Svf::new(900.0, 2.0, RATE);
        let run = |svf: &mut Svf| -> Vec<f32> {
            (0..4_000)
                .map(|n| svf.process(((n % 53) as f32 - 26.0) / 26.0).band)
                .collect()
        };
        let first = run(&mut svf);
        svf.reset();
        assert_eq!(first, run(&mut svf));
    }

    #[test]
    fn the_band_output_peaks_at_the_cutoff() {
        let at = |freq| measured_gain(1_000.0, 4.0, freq, |out| out.band);
        let peak = at(1_000.0);
        assert!(peak > at(500.0), "{peak} was not above the octave below");
        assert!(peak > at(2_000.0), "{peak} was not above the octave above");
        assert!(peak > at(250.0));
        assert!(peak > at(4_000.0));
    }

    #[test]
    fn low_and_high_lean_the_ways_their_names_say() {
        let low = |freq| measured_gain(1_000.0, 0.707, freq, |out| out.low);
        assert!((low(100.0) - 1.0).abs() < 0.05, "low read {}", low(100.0));
        assert!(low(8_000.0) < 0.05);

        let high = |freq| measured_gain(1_000.0, 0.707, freq, |out| out.high);
        assert!((high(16_000.0) - 1.0).abs() < 0.1);
        assert!(high(100.0) < 0.05);
    }

    #[test]
    fn a_sweep_across_the_whole_range_stays_finite() {
        let mut svf = Svf::new(1_000.0, 8.0, RATE);
        for n in 0..48_000 {
            // Straight from below zero to past Nyquist and back, every sample.
            let t = (n as f64 / 24_000.0 - 1.0).abs();
            svf.set(-50.0 + t * 40_000.0, 8.0, RATE);
            let out = svf.process(((n % 31) as f32 - 15.0) / 15.0);
            assert!(
                out.low.is_finite() && out.band.is_finite() && out.high.is_finite(),
                "frame {n} gave {out:?}"
            );
            assert!(out.band.abs() < 100.0, "frame {n} rang up to {}", out.band);
        }
    }
}

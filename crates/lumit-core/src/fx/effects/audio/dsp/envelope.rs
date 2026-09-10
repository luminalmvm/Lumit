//! Followers: how loud the sound is now, and how a gain reduction moves.
//!
//! # In plain terms
//!
//! A compressor, a gate and a limiter all ask the same two questions. How
//! loud is this, which is a follower over the signal, and how fast may the
//! gain move, which is the same one pole with two time constants and a branch
//! between them. Attack is the coefficient used while the answer is growing
//! and release the one used while it falls, so a burst is caught quickly and
//! let go slowly.
//!
//! Peak follows the rectified signal and is what a limiter wants: it must
//! never miss a transient. RMS follows the square and roots it on the way
//! out, so a sine reads 0.707 of its peak rather than 1, which is nearer to
//! how loud the ear finds it and is what a compressor's other detector is.

use super::smoother::coeff_of_ms;

/// The rectified signal, chased up at the attack time and down at the
/// release.
#[derive(Clone, Copy, Debug)]
pub struct PeakFollower {
    attack: f64,
    release: f64,
    env: f64,
}

impl PeakFollower {
    /// A follower resting at silence.
    #[must_use]
    pub fn new(attack_ms: f64, release_ms: f64, rate: f64) -> Self {
        Self {
            attack: coeff_of_ms(attack_ms, rate),
            release: coeff_of_ms(release_ms, rate),
            env: 0.0,
        }
    }

    /// New times, the envelope in flight kept.
    pub fn set_times(&mut self, attack_ms: f64, release_ms: f64, rate: f64) {
        self.attack = coeff_of_ms(attack_ms, rate);
        self.release = coeff_of_ms(release_ms, rate);
    }

    /// Back to silence.
    pub fn reset(&mut self) {
        self.env = 0.0;
    }

    /// Where it stands, without feeding it anything.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.env
    }

    /// One sample in, the envelope out.
    pub fn process(&mut self, sample: f32) -> f64 {
        let x = f64::from(sample).abs();
        let coeff = if x > self.env {
            self.attack
        } else {
            self.release
        };
        self.env = x + (self.env - x) * coeff;
        self.env
    }
}

/// The same ballistics over the square of the signal, rooted on the way out.
#[derive(Clone, Copy, Debug)]
pub struct RmsFollower {
    attack: f64,
    release: f64,
    mean_square: f64,
}

impl RmsFollower {
    /// A follower resting at silence.
    #[must_use]
    pub fn new(attack_ms: f64, release_ms: f64, rate: f64) -> Self {
        Self {
            attack: coeff_of_ms(attack_ms, rate),
            release: coeff_of_ms(release_ms, rate),
            mean_square: 0.0,
        }
    }

    /// New times, the envelope in flight kept.
    pub fn set_times(&mut self, attack_ms: f64, release_ms: f64, rate: f64) {
        self.attack = coeff_of_ms(attack_ms, rate);
        self.release = coeff_of_ms(release_ms, rate);
    }

    /// Back to silence.
    pub fn reset(&mut self) {
        self.mean_square = 0.0;
    }

    /// Where it stands, without feeding it anything.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.mean_square.max(0.0).sqrt()
    }

    /// One sample in, the envelope out.
    pub fn process(&mut self, sample: f32) -> f64 {
        let x = f64::from(sample);
        let square = x * x;
        let coeff = if square > self.mean_square {
            self.attack
        } else {
            self.release
        };
        self.mean_square = square + (self.mean_square - square) * coeff;
        self.value()
    }
}

/// The gain reduction a dynamics effect is applying, in dB, moved by
/// branching one-pole ballistics.
///
/// Zero is no reduction and the value never goes above it: a compressor that
/// let go into a boost would be a compressor and a fader at once. Reduction
/// deepening takes the attack time, letting go takes the release, which is
/// the branch that makes a compressor sound like one.
#[derive(Clone, Copy, Debug)]
pub struct GainReduction {
    attack: f64,
    release: f64,
    db: f64,
}

impl GainReduction {
    /// No reduction, to start with.
    #[must_use]
    pub fn new(attack_ms: f64, release_ms: f64, rate: f64) -> Self {
        Self {
            attack: coeff_of_ms(attack_ms, rate),
            release: coeff_of_ms(release_ms, rate),
            db: 0.0,
        }
    }

    /// New times, the reduction in flight kept.
    pub fn set_times(&mut self, attack_ms: f64, release_ms: f64, rate: f64) {
        self.attack = coeff_of_ms(attack_ms, rate);
        self.release = coeff_of_ms(release_ms, rate);
    }

    /// Back to no reduction.
    pub fn reset(&mut self) {
        self.db = 0.0;
    }

    /// Where it stands, in dB, without moving it.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.db
    }

    /// One frame towards `target_db`, which is what the curve asks for before
    /// the ballistics have their say.
    pub fn process(&mut self, target_db: f64) -> f64 {
        let target = if target_db.is_finite() {
            target_db.min(0.0)
        } else {
            0.0
        };
        let coeff = if target < self.db {
            self.attack
        } else {
            self.release
        };
        self.db = target + (self.db - target) * coeff;
        self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_1_SQRT_2, TAU};

    const RATE: f64 = 48_000.0;

    fn sine(index: usize, freq: f64) -> f32 {
        (TAU * freq * index as f64 / RATE).sin() as f32
    }

    #[test]
    fn reset_then_the_same_sound_twice_gives_the_same_envelope() {
        let mut peak = PeakFollower::new(5.0, 80.0, RATE);
        let run = |f: &mut PeakFollower| -> Vec<u64> {
            (0..4_000)
                .map(|n| f.process(sine(n, 700.0) * 0.6).to_bits())
                .collect()
        };
        let first = run(&mut peak);
        peak.reset();
        assert_eq!(first, run(&mut peak));

        let mut rms = RmsFollower::new(20.0, 200.0, RATE);
        let run = |f: &mut RmsFollower| -> Vec<u64> {
            (0..4_000)
                .map(|n| f.process(sine(n, 700.0) * 0.6).to_bits())
                .collect()
        };
        let first = run(&mut rms);
        rms.reset();
        assert_eq!(first, run(&mut rms));
    }

    #[test]
    fn rms_on_a_steady_sine_settles_at_point_seven_of_the_peak() {
        let mut rms = RmsFollower::new(50.0, 50.0, RATE);
        // Five time constants of a 1 kHz sine at full scale.
        for n in 0..(0.25 * RATE) as usize {
            rms.process(sine(n, 1_000.0));
        }
        assert!(
            (rms.value() - FRAC_1_SQRT_2).abs() < 0.01,
            "settled at {}",
            rms.value()
        );
    }

    #[test]
    fn peak_catches_the_top_and_lets_go_slowly() {
        let mut peak = PeakFollower::new(1.0, 100.0, RATE);
        // Full scale either side of zero: the follower reads the size of it.
        for n in 0..(0.05 * RATE) as usize {
            peak.process(if n % 2 == 0 { 0.5 } else { -0.5 });
        }
        let loud = peak.value();
        assert!((loud - 0.5).abs() < 1e-6, "peak read {loud}");

        // Silence after it: still nearly there a millisecond later, because
        // the release is what governs the fall.
        for _ in 0..(0.001 * RATE) as usize {
            peak.process(0.0);
        }
        assert!(peak.value() > loud * 0.95, "let go too fast");
        for _ in 0..(0.5 * RATE) as usize {
            peak.process(0.0);
        }
        assert!(peak.value() < 0.01, "never let go");
    }

    #[test]
    fn a_gain_reduction_attacks_faster_than_it_recovers() {
        let mut gr = GainReduction::new(1.0, 100.0, RATE);
        let frames = (0.005 * RATE) as usize;
        for _ in 0..frames {
            gr.process(-12.0);
        }
        let down = gr.value();
        assert!(down < -11.0, "attack only reached {down}");

        for _ in 0..frames {
            gr.process(0.0);
        }
        assert!(gr.value() < -5.0, "release let go far too fast");
        assert!(gr.value() > down, "release went the wrong way");

        // A boost is never a reduction, and a nonsense target is no reduction.
        gr.reset();
        assert_eq!(gr.process(6.0), 0.0);
        assert_eq!(gr.process(f64::NAN), 0.0);
    }
}

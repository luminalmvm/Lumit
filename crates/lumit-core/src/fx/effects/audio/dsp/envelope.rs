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

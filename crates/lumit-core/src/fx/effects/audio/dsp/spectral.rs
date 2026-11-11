//! The gain rule the spectral repair effects share: how much of each bin to
//! keep, given how loud it is and how loud the unwanted part is thought to be.
//!
//! # In plain terms
//!
//! Noise reduction and reverb reduction both end the same way. Each frame,
//! each bin has a power and an estimate of the part of that power to remove,
//! and the bin is turned down by a gain between a floor and one. The rule is
//! Ephraim and Malah's log-spectral amplitude estimator with the
//! decision-directed a priori SNR (alpha 0.98, never under -25 dB), which is
//! the classic rule that does not leave isolated bins twinkling.
//!
//! What twinkles anyway is dealt with after the gain is known: in a frame
//! that holds only noise it is averaged in the log domain across
//! neighbouring bins, a few at the bottom and a twelfth of an octave above
//! 2 kHz, where bins are many and narrow; and then it opens fast (5 ms) and
//! closes at the release the Smoothing row asks for. A frame with a voice in
//! it keeps each bin's own gain, because averaging there would pull the
//! gain down on each harmonic and up in the gaps between them, dulling the
//! voice and leaving the noise between its harmonics. How much of a voice a
//! frame holds is the share of its bins from 100 Hz to 4 kHz standing 6 dB
//! over the noise, a third of them counting as all voice. The floor keeps the residue a quieter copy of what it was rather
//! than a hole. Both smoothings can only lift a gain, so the raw gain may go
//! 12 dB under the floor first; steady noise then lands within a couple of dB
//! of Reduction rather than six short of it.

/// The decision-directed estimator's weight on the previous frame.
const DD_ALPHA: f64 = 0.98;

/// The lowest a priori SNR is allowed to fall: -25 dB.
const XI_MIN: f64 = 0.003_162_277_660_168_379_4;

/// How far under the floor the raw gain may go before it is smoothed, as a
/// factor: 12 dB.
const INNER: f64 = 0.25;

/// How fast a gain opens, in milliseconds.
const ATTACK_MS: f64 = 5.0;

/// Where frequency smoothing starts widening to a twelfth of an octave.
const WIDE_FROM_HZ: f64 = 2_000.0;

/// The band a frame's voice is judged in, in hertz.
const VOICE_LOW_HZ: f64 = 100.0;
const VOICE_HIGH_HZ: f64 = 4_000.0;

/// A bin this many times over its noise counts towards a voice: 6 dB.
const VOICE_OVER: f64 = 4.0;

/// The share of the voice band's bins that counts as all voice.
const VOICE_SHARE: f64 = 0.3;

/// Half of a twelfth of an octave, as a fraction of the frequency:
/// `(2^(1/24) - 2^(-1/24)) / 2`.
const TWELFTH_HALF: f64 = 0.028_88;

/// The exponential integral `E1(x)` for `x > 0`, from Abramowitz and Stegun
/// 5.1.53 below one and 5.1.56 above it. Good to about 5e-5 relative, which
/// is far finer than a gain is heard.
#[must_use]
pub fn exp_integral(x: f64) -> f64 {
    let x = x.max(1e-12);
    if x <= 1.0 {
        -x.ln() - 0.577_215_66
            + x * (0.999_991_93
                + x * (-0.249_910_55 + x * (0.055_199_68 + x * (-0.009_760_04 + x * 0.001_078_57))))
    } else if x < 60.0 {
        let num = x * (x * (x * (x + 8.573_328_740_1) + 18.059_016_973) + 8.634_760_892_5)
            + 0.267_773_734_3;
        let den = x * (x * (x * (x + 9.573_322_345_4) + 25.632_956_148_6) + 21.099_653_082_7)
            + 3.958_496_922_8;
        num / den / (x * x.exp())
    } else {
        0.0
    }
}

/// The log-spectral amplitude gain for a priori SNR `xi` and a posteriori
/// SNR `gamma`, never more than one.
#[must_use]
pub fn lsa_gain(xi: f64, gamma: f64) -> f64 {
    let v = xi * gamma / (1.0 + xi);
    (xi / (1.0 + xi) * (0.5 * exp_integral(v)).exp()).min(1.0)
}

/// One channel's gain state, carried frame to frame.
pub struct SpectralGain {
    bin_hz: f64,
    /// The previous frame's clean power estimate, per bin.
    clean: Vec<f32>,
    /// The time-smoothed gain, per bin.
    held: Vec<f32>,
    /// The final gains, per bin.
    gains: Vec<f32>,
    /// Running sum of the log gains, one longer than the bins.
    logs: Vec<f64>,
    /// How many bins either side the frequency smoothing reaches, per bin.
    reach: Vec<usize>,
    /// The Smoothing row the reach and release were last worked out for.
    smoothing: f64,
    attack: f32,
    release: f32,
    hop_s: f64,
}

impl SpectralGain {
    /// State for `bins` bins `bin_hz` apart, frames `hop_s` seconds apart.
    #[must_use]
    pub fn new(bins: usize, bin_hz: f64, hop_s: f64) -> Self {
        let mut out = Self {
            bin_hz,
            clean: vec![0.0; bins],
            // Closed, so the first frame opens through the attack rather
            // than letting the noise through while the release closes it.
            held: vec![0.0; bins],
            gains: vec![1.0; bins],
            logs: vec![0.0; bins + 1],
            reach: vec![1; bins],
            smoothing: f64::NAN,
            attack: coeff(ATTACK_MS, hop_s),
            release: 0.0,
            hop_s,
        };
        out.set_smoothing(50.0);
        out
    }

    /// Set the Smoothing row, 0..100: the release runs from 12.5 to 200 ms
    /// on a log scale (50 ms in the middle) and the frequency reach from one
    /// to eight bins. Cheap when nothing changed.
    pub fn set_smoothing(&mut self, percent: f64) {
        let s = if percent.is_finite() {
            percent.clamp(0.0, 100.0) / 100.0
        } else {
            0.5
        };
        if s == self.smoothing {
            return;
        }
        self.smoothing = s;
        self.release = coeff(12.5 * 16f64.powf(s), self.hop_s);
        let k = 1 + (7.0 * s).round() as usize;
        for (i, reach) in self.reach.iter_mut().enumerate() {
            let hz = i as f64 * self.bin_hz;
            *reach = if hz > WIDE_FROM_HZ && self.bin_hz > 0.0 {
                k.max((TWELFTH_HALF * hz / self.bin_hz).round() as usize)
            } else {
                k
            };
        }
    }

    /// This frame's gains, from each bin's `power`, the `noise` power to take
    /// out of it, and the lowest gain each bin may fall to.
    pub fn gains(&mut self, power: &[f32], noise: &[f32], floor: &[f32]) -> &[f32] {
        // The raw gain, allowed [`INNER`] under the floor so that the
        // smoothing after it, which can only ever lift a gain, leaves steady
        // noise close to the floor rather than well above it.
        let (mut voiced, mut judged) = (0usize, 0usize);
        for (k, (&p, &n)) in power.iter().zip(noise).enumerate() {
            let hz = k as f64 * self.bin_hz;
            if (VOICE_LOW_HZ..VOICE_HIGH_HZ).contains(&hz) {
                judged += 1;
                if f64::from(p) > VOICE_OVER * f64::from(n) {
                    voiced += 1;
                }
            }
        }
        // How much of this frame keeps its own gains: nought for noise
        // alone, one for a voice.
        let own = (voiced as f64 / judged.max(1) as f64 / VOICE_SHARE).clamp(0.0, 1.0);
        for (((&p, &n), &lo), (clean, raw)) in power
            .iter()
            .zip(noise)
            .zip(floor)
            .zip(self.clean.iter_mut().zip(self.gains.iter_mut()))
        {
            let n = f64::from(n).max(1e-30);
            let p = f64::from(p);
            let gamma = (p / n).min(1e8);
            let xi = (DD_ALPHA * f64::from(*clean) / n + (1.0 - DD_ALPHA) * (gamma - 1.0).max(0.0))
                .max(XI_MIN);
            let g = lsa_gain(xi, gamma);
            let kept = g.max(f64::from(lo));
            *clean = (kept * kept * p) as f32;
            *raw = g.max(f64::from(lo) * INNER) as f32;
        }
        // The geometric mean across each bin's reach, from a running sum of
        // the logs so a wide reach costs the same as a narrow one. Across
        // frequency first, so one twinkling bin is averaged away before the
        // release can hold it open.
        let mut acc = 0.0f64;
        if let Some(first) = self.logs.first_mut() {
            *first = 0.0;
        }
        for (slot, g) in self.logs.iter_mut().skip(1).zip(&self.gains) {
            acc += f64::from(g.max(1e-6)).ln();
            *slot = acc;
        }
        let bins = self.gains.len();
        for (i, ((out, held), (&reach, &lo))) in self
            .gains
            .iter_mut()
            .zip(self.held.iter_mut())
            .zip(self.reach.iter().zip(floor))
            .enumerate()
        {
            let a = i.saturating_sub(reach);
            let b = (i + reach + 1).min(bins);
            let sum =
                self.logs.get(b).copied().unwrap_or(0.0) - self.logs.get(a).copied().unwrap_or(0.0);
            let mean = sum / (b - a).max(1) as f64;
            let mine = f64::from((*out).max(1e-6)).ln();
            let g = (own * mine + (1.0 - own) * mean).exp().min(1.0) as f32;
            let k = if g > *held { self.attack } else { self.release };
            *held = k * *held + (1.0 - k) * g;
            *out = held.max(lo);
        }
        &self.gains
    }
}

/// A one-pole coefficient for a time constant of `ms` at frames `hop_s` apart.
fn coeff(ms: f64, hop_s: f64) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-hop_s / (ms * 0.001)).exp() as f32
    }
}

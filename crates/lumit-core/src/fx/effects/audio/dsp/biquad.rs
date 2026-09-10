//! An RBJ Audio EQ Cookbook biquad, and a cascade of them.
//!
//! # In plain terms
//!
//! Every band of the EQ, every octave of the graphic EQ and the tone control
//! inside the distortion is one of these: two poles, two zeros, and five
//! numbers built from a frequency, a Q and a gain. The eight builders are the
//! cookbook's, unchanged, because they are what every other product's bands
//! are and a band that measures differently would sound wrong to a hand
//! arriving from one.
//!
//! The form is transposed direct form II: two state words rather than four,
//! and the arithmetic that keeps its accuracy at low frequencies, which is
//! where a 30 Hz shelf lives. Coefficients are `f64` and stay that way
//! through the recursion; only the sample crossing the boundary is `f32`.

/// Q below this makes the widest useful band, above it the narrowest. A row
/// swept by a keyframe can pass either end, and a filter that blows up there
/// stays blown up.
const Q_MIN: f64 = 0.05;
const Q_MAX: f64 = 100.0;

/// The cookbook's intermediate terms for one set of rows: cos w0, sin w0 and
/// alpha, with the frequency held off zero and off Nyquist.
fn shape(freq_hz: f64, q: f64, rate: f64) -> (f64, f64, f64) {
    let rate = rate.max(1.0);
    let freq = freq_hz.clamp(1.0, rate * 0.49);
    let q = q.clamp(Q_MIN, Q_MAX);
    let w0 = std::f64::consts::TAU * freq / rate;
    let (sin_w0, cos_w0) = w0.sin_cos();
    (cos_w0, sin_w0, sin_w0 / (2.0 * q))
}

/// A gain in dB as the cookbook's `A`. The response of a bell at its centre
/// is `A` squared, which is the gain the row asked for.
fn amplitude(gain_db: f64) -> f64 {
    10f64.powf(gain_db / 40.0)
}

/// One section's five coefficients, already divided through by a0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Default for Coeffs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Coeffs {
    /// Sound through unchanged: what a band that is switched off holds, so
    /// switching it off costs the same arithmetic as leaving it on and the
    /// block does not branch.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalised(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        if a0 == 0.0 || !a0.is_finite() {
            return Self::IDENTITY;
        }
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// 12 dB an octave below the corner, unity above it. Q is 1/sqrt(2) for
    /// the flat Butterworth answer.
    #[must_use]
    pub fn low_pass(freq_hz: f64, q: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        let b1 = 1.0 - cos_w0;
        Self::normalised(
            b1 * 0.5,
            b1,
            b1 * 0.5,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    /// The low pass turned over: the EQ's first band by default.
    #[must_use]
    pub fn high_pass(freq_hz: f64, q: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        let b1 = -(1.0 + cos_w0);
        Self::normalised(
            -b1 * 0.5,
            b1,
            -b1 * 0.5,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    /// A band around the centre, unity at the centre whatever the Q. The
    /// cookbook's 0 dB peak form, which is the one a wah wants.
    #[must_use]
    pub fn band_pass(freq_hz: f64, q: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        Self::normalised(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha)
    }

    /// A hole at the centre: mains hum, and the one band an EQ needs that a
    /// bell cannot make.
    #[must_use]
    pub fn notch(freq_hz: f64, q: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        Self::normalised(
            1.0,
            -2.0 * cos_w0,
            1.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    /// A bell: `gain_db` at the centre, unity well away from it.
    #[must_use]
    pub fn peaking(freq_hz: f64, q: f64, gain_db: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        let a = amplitude(gain_db);
        Self::normalised(
            1.0 + alpha * a,
            -2.0 * cos_w0,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_w0,
            1.0 - alpha / a,
        )
    }

    /// A shelf holding `gain_db` below the corner and unity above it.
    #[must_use]
    pub fn low_shelf(freq_hz: f64, q: f64, gain_db: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        let a = amplitude(gain_db);
        let root = 2.0 * a.sqrt() * alpha;
        Self::normalised(
            a * ((a + 1.0) - (a - 1.0) * cos_w0 + root),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
            a * ((a + 1.0) - (a - 1.0) * cos_w0 - root),
            (a + 1.0) + (a - 1.0) * cos_w0 + root,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - root,
        )
    }

    /// The same shelf the other way up: `gain_db` above the corner.
    #[must_use]
    pub fn high_shelf(freq_hz: f64, q: f64, gain_db: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        let a = amplitude(gain_db);
        let root = 2.0 * a.sqrt() * alpha;
        Self::normalised(
            a * ((a + 1.0) + (a - 1.0) * cos_w0 + root),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
            a * ((a + 1.0) + (a - 1.0) * cos_w0 - root),
            (a + 1.0) - (a - 1.0) * cos_w0 + root,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - root,
        )
    }

    /// Level everywhere, phase turned through 360 degrees around the centre.
    /// The phaser is a row of these.
    #[must_use]
    pub fn all_pass(freq_hz: f64, q: f64, rate: f64) -> Self {
        let (cos_w0, _, alpha) = shape(freq_hz, q, rate);
        Self::normalised(
            1.0 - alpha,
            -2.0 * cos_w0,
            1.0 + alpha,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }
}

/// One section and its state, for one channel. Stereo is two of these.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    coeffs: Coeffs,
    s1: f64,
    s2: f64,
}

impl Biquad {
    /// A section holding these coefficients, starting from silence.
    #[must_use]
    pub fn new(coeffs: Coeffs) -> Self {
        Self {
            coeffs,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// New coefficients, state kept. This is what a block start does when a
    /// frequency row has moved: throwing the state away instead would click.
    pub fn set(&mut self, coeffs: Coeffs) {
        self.coeffs = coeffs;
    }

    /// Forget the state. The next sample starts from silence.
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    /// One sample.
    pub fn process(&mut self, sample: f32) -> f32 {
        let x = f64::from(sample);
        let y = self.coeffs.b0 * x + self.s1;
        self.s1 = self.coeffs.b1 * x - self.coeffs.a1 * y + self.s2;
        self.s2 = self.coeffs.b2 * x - self.coeffs.a2 * y;
        y as f32
    }
}

/// `N` sections in series, one channel's worth: the parametric EQ's five
/// bands, the graphic EQ's ten, the phaser's all-passes.
#[derive(Clone, Copy, Debug)]
pub struct Cascade<const N: usize> {
    sections: [Biquad; N],
}

impl<const N: usize> Default for Cascade<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Cascade<N> {
    /// A cascade passing sound through unchanged.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sections: [Biquad::new(Coeffs::IDENTITY); N],
        }
    }

    /// Set one section, state kept. An index past the end does nothing,
    /// because a mode row that lost a band should not take the sound out.
    pub fn set(&mut self, index: usize, coeffs: Coeffs) {
        if let Some(section) = self.sections.get_mut(index) {
            section.set(coeffs);
        }
    }

    /// Forget every section's state.
    pub fn reset(&mut self) {
        for section in &mut self.sections {
            section.reset();
        }
    }

    /// One sample through all `N` sections in order.
    pub fn process(&mut self, sample: f32) -> f32 {
        let mut y = sample;
        for section in &mut self.sections {
            y = section.process(y);
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_1_SQRT_2, TAU};

    const RATE: f64 = 48_000.0;

    /// What an ear would measure: a sine in, the ratio of the settled
    /// amplitudes out. The window is a whole number of cycles, so the two
    /// sums are exact.
    fn measured_gain(coeffs: Coeffs, freq: f64) -> f64 {
        let mut section = Biquad::new(coeffs);
        let cycle = (RATE / freq).round().max(1.0) as usize;
        let settle = cycle * 50;
        let window = cycle * 50;
        let mut input = 0.0;
        let mut output = 0.0;
        for n in 0..settle + window {
            let x = (TAU * freq * n as f64 / RATE).sin();
            let y = f64::from(section.process(x as f32));
            if n >= settle {
                input += x * x;
                output += y * y;
            }
        }
        (output / input).sqrt()
    }

    fn measured_db(coeffs: Coeffs, freq: f64) -> f64 {
        20.0 * measured_gain(coeffs, freq).log10()
    }

    fn ramp(section: &mut Biquad, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| section.process(((n % 97) as f32 - 48.0) / 48.0))
            .collect()
    }

    #[test]
    fn reset_then_the_same_input_twice_gives_the_same_output() {
        let mut section = Biquad::new(Coeffs::low_pass(800.0, FRAC_1_SQRT_2, RATE));
        let first = ramp(&mut section, 4_000);
        section.reset();
        let second = ramp(&mut section, 4_000);
        assert_eq!(first, second);

        let mut cascade = Cascade::<3>::new();
        cascade.set(0, Coeffs::peaking(1_000.0, 1.0, 6.0, RATE));
        cascade.set(1, Coeffs::high_pass(120.0, FRAC_1_SQRT_2, RATE));
        cascade.set(2, Coeffs::high_shelf(4_000.0, FRAC_1_SQRT_2, -3.0, RATE));
        let one: Vec<f32> = (0..2_000)
            .map(|n| cascade.process(((n % 61) as f32 - 30.0) / 30.0))
            .collect();
        cascade.reset();
        let two: Vec<f32> = (0..2_000)
            .map(|n| cascade.process(((n % 61) as f32 - 30.0) / 30.0))
            .collect();
        assert_eq!(one, two);
    }

    #[test]
    fn a_bell_at_its_centre_is_the_gain_the_row_asked_for() {
        for gain in [-12.0, -6.0, 3.0, 9.0] {
            for q in [0.5, 1.0, 4.0] {
                let db = measured_db(Coeffs::peaking(1_000.0, q, gain, RATE), 1_000.0);
                assert!(
                    (db - gain).abs() < 0.1,
                    "bell of {gain} dB at Q {q} measured {db}"
                );
            }
        }
    }

    #[test]
    fn a_low_pass_is_three_down_at_its_corner() {
        let db = measured_db(Coeffs::low_pass(1_000.0, FRAC_1_SQRT_2, RATE), 1_000.0);
        assert!((db + 3.0103).abs() < 0.2, "corner measured {db}");

        let db = measured_db(Coeffs::high_pass(1_000.0, FRAC_1_SQRT_2, RATE), 1_000.0);
        assert!((db + 3.0103).abs() < 0.2, "corner measured {db}");
    }

    #[test]
    fn the_other_shapes_land_where_the_cookbook_says() {
        // A band pass is unity at its centre, whatever the Q.
        let db = measured_db(Coeffs::band_pass(1_000.0, 3.0, RATE), 1_000.0);
        assert!(db.abs() < 0.1, "band pass centre measured {db}");

        // A notch is a hole.
        let gain = measured_gain(Coeffs::notch(1_000.0, 1.0, RATE), 1_000.0);
        assert!(gain < 1e-3, "notch centre measured {gain}");

        // An all-pass moves phase and nothing else.
        for freq in [200.0, 1_000.0, 6_000.0] {
            let db = measured_db(Coeffs::all_pass(1_000.0, 1.0, RATE), freq);
            assert!(db.abs() < 0.1, "all-pass at {freq} measured {db}");
        }

        // The shelves reach their gain well past the corner.
        let db = measured_db(Coeffs::low_shelf(2_000.0, FRAC_1_SQRT_2, 6.0, RATE), 100.0);
        assert!((db - 6.0).abs() < 0.2, "low shelf measured {db}");
        let db = measured_db(
            Coeffs::high_shelf(1_000.0, FRAC_1_SQRT_2, -6.0, RATE),
            16_000.0,
        );
        assert!((db + 6.0).abs() < 0.2, "high shelf measured {db}");
    }

    #[test]
    fn a_row_swept_past_either_end_stays_a_filter() {
        for freq in [-100.0, 0.0, 1e9] {
            for q in [0.0, -1.0, 1e9] {
                let coeffs = Coeffs::peaking(freq, q, 6.0, RATE);
                assert!(
                    coeffs.b0.is_finite() && coeffs.a2.is_finite(),
                    "{freq} Hz at Q {q} built {coeffs:?}"
                );
            }
        }
        assert_eq!(Coeffs::default(), Coeffs::IDENTITY);
        let mut section = Biquad::new(Coeffs::IDENTITY);
        assert_eq!(section.process(0.25), 0.25);
    }
}

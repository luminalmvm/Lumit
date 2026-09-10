//! A 4x oversampler: one fixed linear-phase half-band FIR, twice up and
//! twice down.
//!
//! # In plain terms
//!
//! A waveshaper makes harmonics, and the ones above Nyquist fold back down as
//! aliasing: grit that moves the wrong way when the pitch moves. Running the
//! shaper at four times the rate puts those harmonics where a filter can
//! throw them away instead.
//!
//! One half-band filter does all four jobs. It is symmetric, so its delay is
//! exactly its centre tap and every frequency is delayed by the same amount:
//! [`Oversample4x::LATENCY_FRAMES`] frames, which the effect reports so the
//! chain can place it earlier and the sound lands where the dry did
//! (docs/impl/audio-effects.md §8). Half of a half-band's taps are zero,
//! which is why it is the cheap filter to pick here: the two phases are the
//! centre tap alone and the odd taps, so a stage costs sixteen multiplies a
//! sample rather than thirty-three.

use std::f64::consts::{PI, TAU};

/// Taps in the half-band. Odd, with an even centre, so the zero taps fall
/// where the polyphase split wants them.
const TAPS: usize = 33;

/// The centre tap, which is the filter's delay in samples at its own rate.
const CENTRE: usize = 16;

/// Non-zero taps either side of the centre.
const ODD: usize = 16;

/// The ring one decimating stage needs: back to `v[2m - 31]`.
const DOWN_RING: usize = 33;

/// The half-band's coefficients, already split into the two phases.
#[derive(Clone, Copy, Debug)]
struct Halfband {
    /// Taps 1, 3, ... 31, in that order.
    odd: [f64; ODD],
    /// Tap 16.
    centre: f64,
}

impl Halfband {
    /// A Blackman-windowed sinc at a quarter of the working rate, scaled to
    /// unity at DC. Worked out here rather than pasted in as a table, because
    /// the arithmetic is the same every time and a table nobody can rederive
    /// is a table nobody can fix.
    fn design() -> Self {
        let mut taps = [0.0f64; TAPS];
        let mut sum = 0.0;
        for (n, tap) in taps.iter_mut().enumerate() {
            let offset = n as f64 - CENTRE as f64;
            // Every other tap of a half-band is exactly zero. Say so, rather
            // than trusting sin of a multiple of pi to come back as zero.
            let sinc = if n == CENTRE {
                1.0
            } else if n % 2 == CENTRE % 2 {
                0.0
            } else {
                let x = 0.5 * offset;
                (PI * x).sin() / (PI * x)
            };
            let t = n as f64 / (TAPS - 1) as f64;
            let window = 0.42 - 0.5 * (TAU * t).cos() + 0.08 * (2.0 * TAU * t).cos();
            *tap = 0.5 * sinc * window;
            sum += *tap;
        }

        let mut odd = [0.0f64; ODD];
        let mut centre = 0.0;
        for (n, tap) in taps.iter().enumerate() {
            let scaled = tap / sum;
            if n == CENTRE {
                centre = scaled;
            } else if n % 2 == 1 {
                if let Some(slot) = odd.get_mut(n / 2) {
                    *slot = scaled;
                }
            }
        }
        Self { odd, centre }
    }
}

/// One 2x interpolating stage: one sample in, two out.
#[derive(Clone, Debug)]
struct Up2x {
    half: Halfband,
    hist: [f32; ODD],
    pos: usize,
}

impl Up2x {
    fn new(half: Halfband) -> Self {
        Self {
            half,
            hist: [0.0; ODD],
            pos: 0,
        }
    }

    fn reset(&mut self) {
        self.hist = [0.0; ODD];
        self.pos = 0;
    }

    /// `back` samples behind the newest.
    fn at(&self, back: usize) -> f64 {
        let index = (self.pos + back) % ODD;
        f64::from(self.hist.get(index).copied().unwrap_or(0.0))
    }

    fn process(&mut self, sample: f32) -> [f32; 2] {
        self.pos = (self.pos + ODD - 1) % ODD;
        if let Some(slot) = self.hist.get_mut(self.pos) {
            *slot = sample;
        }
        // Zero stuffing halves the level, so both phases are doubled. The
        // even phase is the centre tap on its own, which is a plain delay.
        let even = 2.0 * self.half.centre * self.at(CENTRE / 2);
        let mut odd = 0.0;
        for (j, coeff) in self.half.odd.iter().enumerate() {
            odd += coeff * self.at(j);
        }
        [even as f32, (2.0 * odd) as f32]
    }
}

/// One 2x decimating stage: two samples in, one out.
#[derive(Clone, Debug)]
struct Down2x {
    half: Halfband,
    hist: [f32; DOWN_RING],
    pos: usize,
}

impl Down2x {
    fn new(half: Halfband) -> Self {
        Self {
            half,
            hist: [0.0; DOWN_RING],
            pos: 0,
        }
    }

    fn reset(&mut self) {
        self.hist = [0.0; DOWN_RING];
        self.pos = 0;
    }

    fn push(&mut self, sample: f32) {
        self.pos = (self.pos + DOWN_RING - 1) % DOWN_RING;
        if let Some(slot) = self.hist.get_mut(self.pos) {
            *slot = sample;
        }
    }

    /// `back` samples behind the newest.
    fn at(&self, back: usize) -> f64 {
        let index = (self.pos + back) % DOWN_RING;
        f64::from(self.hist.get(index).copied().unwrap_or(0.0))
    }

    fn process(&mut self, first: f32, second: f32) -> f32 {
        self.push(first);
        self.push(second);
        // The newest sample is v[2m + 1], so v[2m - k] is k + 1 back.
        let mut sum = self.half.centre * self.at(CENTRE + 1);
        for (j, coeff) in self.half.odd.iter().enumerate() {
            sum += coeff * self.at(2 * j + 2);
        }
        sum as f32
    }
}

/// Up to four times the rate and back again.
#[derive(Clone, Debug)]
pub struct Oversample4x {
    up1: Up2x,
    up2: Up2x,
    down1: Down2x,
    down2: Down2x,
}

impl Default for Oversample4x {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversample4x {
    /// The delay through up and back down, in frames at the base rate.
    /// Exact, because the filter is linear phase: eight frames for each 2x
    /// stage at the base rate and four for each at twice it.
    pub const LATENCY_FRAMES: u32 = 24;

    /// An oversampler holding silence.
    #[must_use]
    pub fn new() -> Self {
        let half = Halfband::design();
        Self {
            up1: Up2x::new(half),
            up2: Up2x::new(half),
            down1: Down2x::new(half),
            down2: Down2x::new(half),
        }
    }

    /// Forget every stage's state.
    pub fn reset(&mut self) {
        self.up1.reset();
        self.up2.reset();
        self.down1.reset();
        self.down2.reset();
    }

    /// One frame in, four samples at four times the rate out, oldest first.
    /// Shape these, then hand them straight back to [`Oversample4x::down`].
    pub fn up(&mut self, sample: f32) -> [f32; 4] {
        let [a, b] = self.up1.process(sample);
        let [p, q] = self.up2.process(a);
        let [r, s] = self.up2.process(b);
        [p, q, r, s]
    }

    /// Four shaped samples back to one frame.
    pub fn down(&mut self, samples: [f32; 4]) -> f32 {
        let [p, q, r, s] = samples;
        let a = self.down1.process(p, q);
        let b = self.down1.process(r, s);
        self.down2.process(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;
    const LATENCY: usize = Oversample4x::LATENCY_FRAMES as usize;

    /// Straight through, nothing shaped: what the pair costs on its own.
    fn through(input: &[f32]) -> Vec<f32> {
        let mut os = Oversample4x::new();
        input
            .iter()
            .map(|&x| {
                let up = os.up(x);
                os.down(up)
            })
            .collect()
    }

    #[test]
    fn reset_then_the_same_input_twice_gives_the_same_output() {
        let input: Vec<f32> = (0..2_000)
            .map(|n| ((n % 71) as f32 - 35.0) / 35.0)
            .collect();
        let mut os = Oversample4x::new();
        let run = |os: &mut Oversample4x| -> Vec<f32> {
            input
                .iter()
                .map(|&x| {
                    let up = os.up(x);
                    os.down(up)
                })
                .collect()
        };
        let first = run(&mut os);
        os.reset();
        assert_eq!(first, run(&mut os));
    }

    #[test]
    fn an_impulse_arrives_at_the_delay_the_pair_reports() {
        let mut input = vec![0.0f32; 128];
        if let Some(slot) = input.get_mut(0) {
            *slot = 1.0;
        }
        let out = through(&input);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| {
                a.1.abs()
                    .partial_cmp(&b.1.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(0, |(n, _)| n);
        assert_eq!(peak, LATENCY, "the impulse landed at {peak}");
    }

    #[test]
    fn a_low_tone_comes_back_at_the_level_it_went_in() {
        let freq = 100.0;
        let frames = 4_800 + LATENCY;
        let input: Vec<f32> = (0..frames)
            .map(|n| (std::f64::consts::TAU * freq * n as f64 / RATE).sin() as f32)
            .collect();
        let out = through(&input);

        // Eight whole cycles, started well past the filter's ramp-in.
        let mut sent = 0.0f64;
        let mut back = 0.0f64;
        for n in 480..4_320 {
            let x = f64::from(input.get(n).copied().unwrap_or(0.0));
            let y = f64::from(out.get(n + LATENCY).copied().unwrap_or(0.0));
            sent += x * x;
            back += y * y;
        }
        let db = 10.0 * (back / sent).log10();
        assert!(db.abs() < 0.1, "the tone came back {db} dB off");
    }

    #[test]
    fn the_half_band_is_symmetric_and_unity_at_dc() {
        let half = Halfband::design();
        let sum = half.centre + 2.0 * half.odd.iter().take(ODD / 2).sum::<f64>();
        assert!((sum - 1.0).abs() < 1e-12, "DC gain came to {sum}");
        assert!(
            (half.centre - 0.5).abs() < 1e-3,
            "centre tap is {}",
            half.centre
        );
        for j in 0..ODD / 2 {
            let front = half.odd.get(j).copied().unwrap_or(0.0);
            let back = half.odd.get(ODD - 1 - j).copied().unwrap_or(0.0);
            assert!((front - back).abs() < 1e-12, "tap {j} is not symmetric");
        }
    }
}

//! The master's limiter: the last stage before the device, after the master
//! fader (docs/09-AUDIO.md).
//!
//! It looks 1.5 ms ahead. Every sample's true peak (the loudest point of the
//! signal upsampled four times, from [`crate::loudness`]'s upsampler) says how
//! far the sound has to come down to stay under the ceiling. The gain is the
//! deepest of those over the look ahead, so it is already down when the peak
//! arrives, and it comes back up with a 50 ms release. The two sides share one
//! gain, so the stereo picture does not lean. A final clamp at the ceiling is
//! the guarantee.
//!
//! Runs inside [`crate::plan::MixPlan::mix_block`], on the audio callback or
//! an export. Everything it keeps is made when it is, so running it allocates
//! nothing.

use crate::loudness::Upsampler;

/// How far ahead the limiter looks, in seconds.
const LOOKAHEAD_SECONDS: f64 = 0.0015;

/// The release's time constant, in seconds.
const RELEASE_SECONDS: f64 = 0.05;

/// The master's lookahead true-peak limiter, with its state.
pub struct Limiter {
    /// The look ahead in frames: how late the limiter's output is.
    lookahead: usize,
    /// The input of the last `lookahead` frames, stereo, oldest at `pos`.
    delay: Box<[f32]>,
    pos: usize,
    /// The gains needed over the look ahead, as a queue whose values rise
    /// from the front: the front is the deepest. Frame numbers with each.
    need: Box<[(u64, f32)]>,
    head: usize,
    len: usize,
    /// How many frames have gone in.
    frame: u64,
    gain: f32,
    release: f32,
    upsampler: Upsampler,
}

impl Limiter {
    /// A limiter for sound at `rate` frames a second.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        let rate = f64::from(rate.max(1_000));
        let lookahead = ((rate * LOOKAHEAD_SECONDS).round() as usize).max(8);
        Self {
            lookahead,
            delay: vec![0.0; lookahead * 2].into_boxed_slice(),
            pos: 0,
            need: vec![(0, 1.0); lookahead + 2].into_boxed_slice(),
            head: 0,
            len: 0,
            frame: 0,
            gain: 1.0,
            release: (1.0 - (-1.0 / (RELEASE_SECONDS * rate)).exp()) as f32,
            upsampler: Upsampler::new(),
        }
    }

    /// How many frames late the limiter's output is: its look ahead.
    #[must_use]
    pub fn latency(&self) -> u32 {
        self.lookahead as u32
    }

    /// Forget everything: silence in the look ahead and no gain reduction.
    pub fn reset(&mut self) {
        self.delay.fill(0.0);
        self.pos = 0;
        self.head = 0;
        self.len = 0;
        self.gain = 1.0;
        self.upsampler.clear();
    }

    /// Limit `frames` (interleaved stereo) in place to `ceiling`, a linear
    /// true peak. What comes out is [`Self::latency`] frames late. Answers the
    /// smallest gain used.
    pub fn process(&mut self, ceiling: f32, frames: &mut [f32]) -> f32 {
        let ceiling = ceiling.max(1e-6);
        let cap = self.need.len();
        let mut lowest = 1.0f32;
        for frame in frames.chunks_exact_mut(2) {
            let (l, r) = (frame[0], frame[1]);
            let peak = self
                .upsampler
                .peak(0, f64::from(l))
                .max(self.upsampler.peak(1, f64::from(r))) as f32;
            let need = if peak > ceiling { ceiling / peak } else { 1.0 };
            // Into the queue: anything not deeper than this goes, since this
            // one outlasts it.
            while self.len > 0 {
                let back = (self.head + self.len - 1) % cap;
                if self.need[back].1 < need {
                    break;
                }
                self.len -= 1;
            }
            self.need[(self.head + self.len) % cap] = (self.frame, need);
            self.len += 1;
            // Out of it: what has fallen out of the look ahead.
            while self.len > 1 && self.need[self.head].0 + (self.lookahead as u64) < self.frame {
                self.head = (self.head + 1) % cap;
                self.len -= 1;
            }
            let target = self.need[self.head].1;
            self.gain = if target < self.gain {
                target
            } else {
                self.gain + (target - self.gain) * self.release
            };
            let at = self.pos * 2;
            let (dl, dr) = (self.delay[at], self.delay[at + 1]);
            self.delay[at] = l;
            self.delay[at + 1] = r;
            self.pos = (self.pos + 1) % self.lookahead;
            frame[0] = (dl * self.gain).clamp(-ceiling, ceiling);
            frame[1] = (dr * self.gain).clamp(-ceiling, ceiling);
            lowest = lowest.min(self.gain);
            self.frame += 1;
        }
        lowest
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn sine(rate: u32, amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let v = amplitude * (std::f32::consts::TAU * 1000.0 * n as f32 / rate as f32).sin();
                [v, v]
            })
            .collect()
    }

    /// A sine at +6 dBFS comes out with no true peak over the ceiling, as the
    /// loudness meter measures it. A quiet one comes out untouched, only late
    /// by the look ahead.
    #[test]
    fn a_hot_sine_stays_under_the_ceiling_and_a_quiet_one_passes() {
        let rate = 48_000;
        let ceiling = crate::mix::MASTER_CEILING;
        let mut limiter = Limiter::new(rate);
        let mut meter = crate::loudness::Loudness::new(rate);
        let mut hot = sine(rate, 2.0, rate as usize);
        for buffer in hot.chunks_mut(480 * 2) {
            limiter.process(ceiling, buffer);
            meter.add(buffer);
        }
        let peak = meter.reading().true_peak;
        assert!(peak <= ceiling * 1.0001, "true peak {peak} over {ceiling}");
        assert!(peak > ceiling * 0.95, "and not pulled far under it: {peak}");

        let mut limiter = Limiter::new(rate);
        let late = limiter.latency() as usize;
        let quiet = sine(rate, 0.25, 4_800);
        let mut out = quiet.clone();
        for buffer in out.chunks_mut(480 * 2) {
            limiter.process(ceiling, buffer);
        }
        assert!(out[..late * 2].iter().all(|v| *v == 0.0));
        assert!(out[late * 2..] == quiet[..quiet.len() - late * 2]);
    }
}

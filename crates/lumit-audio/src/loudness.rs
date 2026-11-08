//! Loudness as a mix is delivered against: short-term and integrated loudness
//! in LUFS, and the true peak, measured on the master as ITU-R BS.1770-4 and
//! EBU R128 define them.
//!
//! The callback hands [`Loudness::add`] every frame it gives the device and
//! publishes the reading once per buffer beside the bars. The state is fixed
//! in size and belongs to the callback alone, so measuring takes no lock and
//! allocates nothing, and the same sound always reads the same.
//!
//! - **K-weighting** is the standard's two filters: a shelf that lifts what
//!   the head hears more of, and a low cut. Each channel is filtered, squared
//!   and summed with a weight of one.
//! - **Short-term** is the mean of the last three seconds, in 100 ms steps.
//! - **Integrated** gates 400 ms blocks that overlap by three quarters: blocks
//!   under -70 LUFS are dropped, then blocks more than 10 LU under the mean of
//!   the rest. The blocks are kept as a histogram of a tenth of a LU a bin,
//!   holding each bin's exact energy, so an hour costs what a second does and
//!   the only rounding is which side of the relative gate a block inside that
//!   one bin falls.
//! - **True peak** is the loudest point of the output upsampled four times,
//!   since the measure began.

use crate::meter::LoudnessReading;

/// How many 100 ms steps the short-term window holds: three seconds.
const SHORT_STEPS: usize = 30;

/// How many steps make one gating block: 400 ms, moving on by one step.
const BLOCK_STEPS: usize = 4;

/// Blocks at or under this many LUFS are silence to the integrated measure.
const ABSOLUTE_GATE: f64 = -70.0;

/// Blocks this many LU under the loudness of what passed the absolute gate
/// are dropped as well.
const RELATIVE_GATE: f64 = 10.0;

/// The histogram's bins: a tenth of a LU each, from the absolute gate up to
/// +10 LUFS, louder than anything under the master ceiling can read.
const BIN_WIDTH: f64 = 0.1;
const BINS: usize = 800;

/// The upsampler for the true peak: four phases of twelve taps.
const PHASES: usize = 4;
const TAPS: usize = 12;

/// One second-order filter, transposed direct form II, with its state.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The K-weighting at `rate`, from the analogue designs behind the
/// standard's 48 kHz table, so every device rate reads alike.
fn k_weighting(rate: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    // The shelf: about +4 dB above 2 kHz.
    let (f0, gain_db, q) = (
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
    );
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    // The low cut at 38 Hz.
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let cut = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    [shelf, cut]
}

/// The upsampler's taps: a windowed sinc cut at the input's Nyquist, split
/// into four phases, each scaled so a steady level comes through unchanged.
fn upsampler() -> [[f64; TAPS]; PHASES] {
    use std::f64::consts::PI;
    let len = PHASES * TAPS;
    let centre = (len as f64 - 1.0) / 2.0;
    let mut phases = [[0.0; TAPS]; PHASES];
    for (p, phase) in phases.iter_mut().enumerate() {
        for (j, tap) in phase.iter_mut().enumerate() {
            let m = (j * PHASES + p) as f64;
            let x = (m - centre) / PHASES as f64;
            let sinc = if x == 0.0 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let hann = 0.5 - 0.5 * (2.0 * PI * (m + 0.5) / len as f64).cos();
            *tap = sinc * hann;
        }
        let sum: f64 = phase.iter().sum();
        if sum != 0.0 {
            phase.iter_mut().for_each(|tap| *tap /= sum);
        }
    }
    phases
}

/// One sample through a channel's K-weighting.
#[inline]
fn weigh(filters: &mut [Biquad; 2], x: f64) -> f64 {
    let shelved = filters[0].run(x);
    filters[1].run(shelved)
}

/// Mean square to LUFS. Silence is minus infinity.
fn lufs(mean_square: f64) -> f64 {
    if mean_square > 0.0 {
        -0.691 + 10.0 * mean_square.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// The master's loudness meter, owned by the audio callback.
pub struct Loudness {
    /// Each channel's K-weighting.
    weighting: [[Biquad; 2]; 2],
    /// How many frames one 100 ms step holds.
    step_frames: u32,
    /// The step under way: its weighted squares and how many frames it has.
    step_sum: f64,
    step_count: u32,
    /// The finished steps' sums, oldest overwritten first.
    steps: [f64; SHORT_STEPS],
    next: usize,
    /// How many of `steps` hold sound since the transport last started.
    filled: usize,
    /// The gated blocks: how many fell in each bin, and their energy.
    counts: [u32; BINS],
    energy: [f64; BINS],
    upsampler: [[f64; TAPS]; PHASES],
    /// The last `TAPS` samples of each channel, newest first.
    history: [[f64; TAPS]; 2],
    /// What is published: LUFS, and the true peak as an amplitude.
    short_term: f64,
    integrated: f64,
    true_peak: f64,
}

impl Loudness {
    /// A meter for output at `rate` frames a second, with nothing measured.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        let rate = rate.max(10);
        let weighting = k_weighting(f64::from(rate));
        Self {
            weighting: [weighting, weighting],
            step_frames: rate / 10,
            step_sum: 0.0,
            step_count: 0,
            steps: [0.0; SHORT_STEPS],
            next: 0,
            filled: 0,
            counts: [0; BINS],
            energy: [0.0; BINS],
            upsampler: upsampler(),
            history: [[0.0; TAPS]; 2],
            short_term: f64::NEG_INFINITY,
            integrated: f64::NEG_INFINITY,
            true_peak: 0.0,
        }
    }

    /// Measure a stretch of the output: interleaved stereo, as the device is
    /// handed it.
    pub fn add(&mut self, frames: &[f32]) {
        for frame in frames.chunks_exact(2) {
            let (l, r) = (f64::from(frame[0]), f64::from(frame[1]));
            self.peak(0, l);
            self.peak(1, r);
            let [left, right] = &mut self.weighting;
            let (wl, wr) = (weigh(left, l), weigh(right, r));
            self.step_sum += wl * wl + wr * wr;
            self.step_count += 1;
            if self.step_count >= self.step_frames {
                self.close_step();
            }
        }
    }

    /// The transport stopped. The short-term window empties and the filters
    /// forget, so playing again measures from silence. The integrated measure
    /// and the true peak hold.
    pub fn pause(&mut self) {
        if self.filled == 0 && self.step_count == 0 {
            return;
        }
        for w in &mut self.weighting {
            for f in w {
                f.z = [0.0; 2];
            }
        }
        self.step_sum = 0.0;
        self.step_count = 0;
        self.filled = 0;
        self.history = [[0.0; TAPS]; 2];
        self.short_term = f64::NEG_INFINITY;
    }

    /// Start every measure afresh.
    pub fn reset(&mut self) {
        self.pause();
        self.counts = [0; BINS];
        self.energy = [0.0; BINS];
        self.integrated = f64::NEG_INFINITY;
        self.true_peak = 0.0;
    }

    /// What the meter reads now.
    #[must_use]
    pub fn reading(&self) -> LoudnessReading {
        LoudnessReading {
            short_term: self.short_term as f32,
            integrated: self.integrated as f32,
            true_peak: self.true_peak as f32,
        }
    }

    /// One sample into a channel's upsampler, keeping the loudest point.
    #[inline]
    fn peak(&mut self, channel: usize, x: f64) {
        let history = &mut self.history[channel];
        history.copy_within(..TAPS - 1, 1);
        history[0] = x;
        self.true_peak = self.true_peak.max(x.abs());
        for phase in &self.upsampler {
            let y: f64 = phase.iter().zip(history.iter()).map(|(h, s)| h * s).sum();
            self.true_peak = self.true_peak.max(y.abs());
        }
    }

    /// A step is finished: the short-term reading moves on, and once four
    /// steps are in hand they make a gating block.
    fn close_step(&mut self) {
        self.steps[self.next] = self.step_sum;
        self.next = (self.next + 1) % SHORT_STEPS;
        self.filled = (self.filled + 1).min(SHORT_STEPS);
        self.step_sum = 0.0;
        self.step_count = 0;
        let frames = f64::from(self.step_frames);
        self.short_term = lufs(self.last(self.filled) / (self.filled as f64 * frames));
        if self.filled < BLOCK_STEPS {
            return;
        }
        let block = self.last(BLOCK_STEPS) / (BLOCK_STEPS as f64 * frames);
        let loudness = lufs(block);
        if loudness <= ABSOLUTE_GATE {
            return;
        }
        let bin = (((loudness - ABSOLUTE_GATE) / BIN_WIDTH) as usize).min(BINS - 1);
        self.counts[bin] = self.counts[bin].saturating_add(1);
        self.energy[bin] += block;
        self.integrated = self.gated();
    }

    /// The sum of the newest `n` finished steps, oldest first.
    fn last(&self, n: usize) -> f64 {
        (0..n)
            .map(|back| self.steps[(self.next + SHORT_STEPS - n + back) % SHORT_STEPS])
            .sum()
    }

    /// The integrated loudness of the blocks binned so far.
    fn gated(&self) -> f64 {
        let mean = |from: usize| {
            let (count, energy) = (from..BINS).fold((0u64, 0.0), |(c, e), bin| {
                (c + u64::from(self.counts[bin]), e + self.energy[bin])
            });
            if count == 0 {
                0.0
            } else {
                energy / count as f64
            }
        };
        let threshold = lufs(mean(0)) - RELATIVE_GATE;
        // A bin counts when its middle is over the relative gate.
        let first = ((threshold - ABSOLUTE_GATE) / BIN_WIDTH).round().max(0.0) as usize;
        lufs(mean(first.min(BINS)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A 1 kHz tone at -23 dBFS in both channels reads -23 LUFS short-term and
    /// integrated, and its true peak is its own level, at either common rate.
    #[test]
    fn a_tone_at_minus_23_reads_minus_23() {
        for rate in [44_100u32, 48_000] {
            let mut meter = Loudness::new(rate);
            let level = 10f64.powf(-23.0 / 20.0);
            let frames: Vec<f32> = (0..rate as usize * 10)
                .flat_map(|n| {
                    let s = (level
                        * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / f64::from(rate)).sin())
                        as f32;
                    [s, s]
                })
                .collect();
            // In callback-sized pieces, as it arrives.
            for buffer in frames.chunks(441 * 2) {
                meter.add(buffer);
            }
            let read = meter.reading();
            assert!((read.short_term + 23.0).abs() < 0.1, "{rate}: {read:?}");
            assert!((read.integrated + 23.0).abs() < 0.1, "{rate}: {read:?}");
            let peak_db = 20.0 * read.true_peak.log10();
            assert!((peak_db + 23.0).abs() < 0.2, "{rate}: {read:?}");
        }
    }
}

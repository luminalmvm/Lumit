//! Loudness as a mix is delivered against: momentary, short-term and
//! integrated loudness in LUFS, the loudness range, and the true peak,
//! measured on the master as ITU-R BS.1770-4, EBU R128 and EBU Tech 3342
//! define them.
//!
//! The callback hands [`Loudness::add`] every frame it gives the device and
//! publishes the reading once per buffer beside the bars. The state is fixed
//! in size and belongs to the callback alone, so measuring takes no lock and
//! allocates nothing, and the same sound always reads the same.
//!
//! - **K-weighting** is the standard's two filters: a shelf that lifts what
//!   the head hears more of, and a low cut. Each channel is filtered, squared
//!   and summed with a weight of one.
//! - **Momentary** is the mean of the last 400 ms, and **short-term** of the
//!   last three seconds, both in 100 ms steps.
//! - **Integrated** gates 400 ms blocks that overlap by three quarters: blocks
//!   under -70 LUFS are dropped, then blocks more than 10 LU under the mean of
//!   the rest. The blocks are kept as a histogram of a tenth of a LU a bin,
//!   holding each bin's exact energy, so an hour costs what a second does and
//!   the only rounding is which side of the relative gate a block inside that
//!   one bin falls.
//! - **Loudness range** takes a short-term reading every 100 ms once three
//!   seconds are in hand, drops those under -70 LUFS and then those more than
//!   20 LU under the mean of the rest, and is the spread between the 10th and
//!   95th percentiles of what is left. Kept in a histogram like the integrated
//!   one.
//! - **True peak** is the loudest point of the output upsampled four times,
//!   since the measure began. The upsampler is shared with the master's
//!   limiter ([`crate::limiter`]).
//!
//! Runs on the audio callback (and in an export's own mix), owned by it.

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

/// The loudness range's relative gate, in LU under the short-term mean.
const RANGE_GATE: f64 = 20.0;

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
fn upsampler_taps() -> [[f64; TAPS]; PHASES] {
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

/// A stereo signal upsampled four times, for its true peak: the loudest point
/// between the samples as well as on them. About six samples late: the
/// filter's half length.
#[derive(Clone, Debug)]
pub(crate) struct Upsampler {
    taps: [[f64; TAPS]; PHASES],
    /// The last `TAPS` samples of each channel, newest first.
    history: [[f64; TAPS]; 2],
}

impl Upsampler {
    pub(crate) fn new() -> Self {
        Self {
            taps: upsampler_taps(),
            history: [[0.0; TAPS]; 2],
        }
    }

    /// One sample into `channel` (0 or 1), answering the loudest absolute
    /// value of it and of the four points the upsampler makes.
    #[inline]
    pub(crate) fn peak(&mut self, channel: usize, x: f64) -> f64 {
        let Some(history) = self.history.get_mut(channel) else {
            return x.abs();
        };
        history.copy_within(..TAPS - 1, 1);
        history[0] = x;
        let mut peak = x.abs();
        for phase in &self.taps {
            let y: f64 = phase.iter().zip(history.iter()).map(|(h, s)| h * s).sum();
            peak = peak.max(y.abs());
        }
        peak
    }

    /// Forget the samples before.
    pub(crate) fn clear(&mut self) {
        self.history = [[0.0; TAPS]; 2];
    }
}

/// Put a reading of mean square `energy` in a histogram, unless it is under
/// the absolute gate. True when it went in.
fn bin_of(energy: f64, counts: &mut [u32; BINS], energies: &mut [f64; BINS]) -> bool {
    let loudness = lufs(energy);
    if loudness <= ABSOLUTE_GATE {
        return false;
    }
    let bin = (((loudness - ABSOLUTE_GATE) / BIN_WIDTH) as usize).min(BINS - 1);
    counts[bin] = counts[bin].saturating_add(1);
    energies[bin] += energy;
    true
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
    /// The short-term readings for the loudness range, binned the same way.
    range_counts: [u32; BINS],
    range_energy: [f64; BINS],
    upsampler: Upsampler,
    /// What is published: LUFS, the range in LU, and the true peak as an
    /// amplitude.
    momentary: f64,
    short_term: f64,
    integrated: f64,
    range: f64,
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
            range_counts: [0; BINS],
            range_energy: [0.0; BINS],
            upsampler: Upsampler::new(),
            momentary: f64::NEG_INFINITY,
            short_term: f64::NEG_INFINITY,
            integrated: f64::NEG_INFINITY,
            range: 0.0,
            true_peak: 0.0,
        }
    }

    /// Measure a stretch of the output: interleaved stereo, as the device is
    /// handed it.
    pub fn add(&mut self, frames: &[f32]) {
        for frame in frames.chunks_exact(2) {
            let (l, r) = (f64::from(frame[0]), f64::from(frame[1]));
            let peak = self.upsampler.peak(0, l).max(self.upsampler.peak(1, r));
            self.true_peak = self.true_peak.max(peak);
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
        self.upsampler.clear();
        self.momentary = f64::NEG_INFINITY;
        self.short_term = f64::NEG_INFINITY;
    }

    /// Start every measure afresh.
    pub fn reset(&mut self) {
        self.pause();
        self.counts = [0; BINS];
        self.energy = [0.0; BINS];
        self.range_counts = [0; BINS];
        self.range_energy = [0.0; BINS];
        self.integrated = f64::NEG_INFINITY;
        self.range = 0.0;
        self.true_peak = 0.0;
    }

    /// What the meter reads now.
    #[must_use]
    pub fn reading(&self) -> LoudnessReading {
        LoudnessReading {
            momentary: self.momentary as f32,
            short_term: self.short_term as f32,
            integrated: self.integrated as f32,
            range: self.range as f32,
            true_peak: self.true_peak as f32,
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
        let short = self.last(self.filled) / (self.filled as f64 * frames);
        self.short_term = lufs(short);
        let recent = self.filled.min(BLOCK_STEPS);
        self.momentary = lufs(self.last(recent) / (recent as f64 * frames));
        if self.filled == SHORT_STEPS
            && bin_of(short, &mut self.range_counts, &mut self.range_energy)
        {
            self.range = self.spread();
        }
        if self.filled < BLOCK_STEPS {
            return;
        }
        let block = self.last(BLOCK_STEPS) / (BLOCK_STEPS as f64 * frames);
        if bin_of(block, &mut self.counts, &mut self.energy) {
            self.integrated = self.gated();
        }
    }

    /// The loudness range of the short-term readings binned so far: the 95th
    /// percentile less the 10th, over those that pass the relative gate.
    fn spread(&self) -> f64 {
        let (count, energy) = (0..BINS).fold((0u64, 0.0), |(c, e), bin| {
            (
                c + u64::from(self.range_counts[bin]),
                e + self.range_energy[bin],
            )
        });
        if count == 0 {
            return 0.0;
        }
        let gate = lufs(energy / count as f64) - RANGE_GATE;
        let first = (((gate - ABSOLUTE_GATE) / BIN_WIDTH).round().max(0.0) as usize).min(BINS);
        let kept: u64 = self.range_counts[first..]
            .iter()
            .map(|&c| u64::from(c))
            .sum();
        if kept == 0 {
            return 0.0;
        }
        // The readings at these places in loudness order, counted from 0.
        let place = |p: f64| ((kept - 1) as f64 * p + 0.5) as u64;
        let at = |want: u64| {
            let mut seen = 0u64;
            for bin in first..BINS {
                let n = u64::from(self.range_counts[bin]);
                if n > 0 && seen + n > want {
                    return lufs(self.range_energy[bin] / n as f64);
                }
                seen += n;
            }
            f64::NEG_INFINITY
        };
        (at(place(0.95)) - at(place(0.10))).max(0.0)
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

    /// A 1 kHz tone of `level_db` dBFS in both channels, `seconds` long.
    fn tone(rate: u32, level_db: f64, seconds: f64) -> Vec<f32> {
        let level = 10f64.powf(level_db / 20.0);
        (0..(f64::from(rate) * seconds) as usize)
            .flat_map(|n| {
                let s = (level
                    * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / f64::from(rate)).sin())
                    as f32;
                [s, s]
            })
            .collect()
    }

    /// A 1 kHz tone at -23 dBFS in both channels reads -23 LUFS momentary,
    /// short-term and integrated, a range of nought, and its true peak is its
    /// own level, at either common rate.
    #[test]
    fn a_tone_at_minus_23_reads_minus_23() {
        for rate in [44_100u32, 48_000] {
            let mut meter = Loudness::new(rate);
            let frames = tone(rate, -23.0, 10.0);
            // In callback-sized pieces, as it arrives.
            for buffer in frames.chunks(441 * 2) {
                meter.add(buffer);
            }
            let read = meter.reading();
            assert!((read.momentary + 23.0).abs() < 0.1, "{rate}: {read:?}");
            assert!((read.short_term + 23.0).abs() < 0.1, "{rate}: {read:?}");
            assert!((read.integrated + 23.0).abs() < 0.1, "{rate}: {read:?}");
            assert!(read.range.abs() < 0.2, "{rate}: {read:?}");
            let peak_db = 20.0 * read.true_peak.log10();
            assert!((peak_db + 23.0).abs() < 0.2, "{rate}: {read:?}");
        }
    }

    /// Ten seconds each of -20 and -30 LUFS, three times over: the short-term
    /// readings sit at the two levels most of the time, so the loudness range
    /// is the ten LU between them.
    #[test]
    fn two_alternating_levels_read_their_spread_as_the_range() {
        let rate = 48_000;
        let mut meter = Loudness::new(rate);
        for n in 0..6 {
            let db = if n % 2 == 0 { -20.0 } else { -30.0 };
            for buffer in tone(rate, db, 10.0).chunks(480 * 2) {
                meter.add(buffer);
            }
        }
        let read = meter.reading();
        assert!((read.range - 10.0).abs() < 0.3, "{read:?}");
    }
}

//! Noise floors: the learned profile Noise reduction keeps in its
//! instance, and the floor it estimates for itself when there is none.
//!
//! # In plain terms
//!
//! A floor is how loud the noise is at each frequency. Stored, it is a power
//! spectral density, power per hertz, on 512 points spaced evenly in log
//! frequency from 20 Hz to 24 kHz, so a profile learned at 48 kHz means the
//! same noise when the bake runs at 44.1 or 96 kHz: the transform size scales
//! with the rate, and turning a density into each bin's expected power is one
//! multiplication by the rate and the window's energy ([`Framing::psd_scale`]).
//!
//! The blob is small and plain: four bytes of magic, a version, a spare byte,
//! the point count, then each point as a signed 16-bit hundredth of a dB,
//! little-endian. 1032 bytes in all.
//!
//! Without a profile the floor is estimated from the whole input: per bin,
//! the 20th percentile over every frame of the power averaged over three
//! frames, times 1.5 to correct for the percentile sitting under the mean,
//! then smoothed across a sixth of an octave. The percentile is read from a
//! histogram of 0.2 dB steps, so a ten-minute file costs a few megabytes
//! rather than its whole spectrogram. Frames of digital silence are skipped,
//! because a gap between takes is not the noise.

use super::{from_grid, log_grid, to_grid};
use crate::fx::effects::audio::dsp::stft::{frame_at, frame_count, size_at, sqrt_hann, Fft};

/// The frame size at 48 kHz: 42.7 ms, a hop of a quarter of it.
pub const FRAME_AT_48K: usize = 2048;

/// What a profile blob starts with.
const MAGIC: [u8; 4] = *b"LNPF";

/// The blob layout's version.
const VERSION: u8 = 1;

/// Points on the profile's frequency grid.
pub const POINTS: usize = 512;

/// The grid's ends, in hertz.
const LOW_HZ: f64 = 20.0;
const HIGH_HZ: f64 = 24_000.0;

/// The bytes ahead of the points.
const HEADER: usize = 8;

/// A frame whose windowed samples hold less energy than this per sample is
/// digital silence, about -120 dBFS.
const SILENT: f32 = 1e-12;

/// The percentile of the smoothed power the adaptive floor reads.
const PERCENTILE: f64 = 0.2;

/// The percentile's bias correction: the 20th percentile of an exponential
/// spread averaged over three frames sits about this far under its mean.
const BIAS: f64 = 1.5;

/// The adaptive histogram's range and step, in dB of power per hertz.
const HIST_LOW_DB: f64 = -220.0;
const HIST_STEP_DB: f64 = 0.2;
const HIST_BUCKETS: usize = 1_100;

/// A tonal peak stands at least this far over the floor around it, in dB.
pub const TONAL_DB: f64 = 3.0;

/// The framing Noise reduction and Reverb reduction use at one rate.
pub struct Framing {
    pub rate: u32,
    /// Samples in a frame.
    pub n: usize,
    /// Samples between frames.
    pub hop: usize,
    /// The analysis window, a square-root Hann.
    pub window: Vec<f32>,
    /// Bins in a spectrum.
    pub bins: usize,
    /// Hertz between bins.
    pub bin_hz: f64,
    /// A bin's expected power for a density of one per hertz: half the rate
    /// times the window's energy.
    pub psd_scale: f64,
}

impl Framing {
    /// The framing at `rate`.
    #[must_use]
    pub fn at(rate: u32) -> Self {
        let n = size_at(rate, FRAME_AT_48K);
        let window = sqrt_hann(n);
        let energy: f64 = window.iter().map(|w| f64::from(w * w)).sum();
        Self {
            rate,
            n,
            hop: n / 4,
            bins: n / 2 + 1,
            bin_hz: f64::from(rate.max(1)) / n as f64,
            psd_scale: 0.5 * f64::from(rate.max(1)) * energy,
            window,
        }
    }

    /// Seconds between frames.
    #[must_use]
    pub fn hop_s(&self) -> f64 {
        self.hop as f64 / f64::from(self.rate.max(1))
    }
}

/// The profile's frequency points.
fn grid() -> Vec<f64> {
    log_grid(LOW_HZ, HIGH_HZ, POINTS)
}

/// Every frame of every channel of `interleaved`, as each bin's power, handed
/// to `each` with the channel; digital silence skipped.
fn each_frame(
    interleaved: &[f32],
    channels: usize,
    only: Option<usize>,
    framing: &Framing,
    mut each: impl FnMut(&[f32]),
) {
    let len = interleaved.len() / channels.max(1);
    let mut fft = Fft::new(framing.n);
    let mut frame = vec![0.0f32; framing.n];
    let mut spectrum = fft.spectrum();
    let mut power = vec![0.0f32; framing.bins];
    for c in 0..channels {
        if only.is_some_and(|o| o != c) {
            continue;
        }
        for j in 0..frame_count(len, framing.n, framing.hop) {
            frame_at(
                interleaved,
                channels,
                c,
                j,
                framing.hop,
                &framing.window,
                &mut frame,
            );
            let energy: f32 = frame.iter().map(|x| x * x).sum();
            if energy < SILENT * framing.n as f32 {
                continue;
            }
            fft.forward(&mut frame, &mut spectrum);
            for (p, x) in power.iter_mut().zip(&spectrum) {
                *p = x.norm_sqr();
            }
            each(&power);
        }
    }
}

/// The mean power per bin over every frame of every channel, as a density in
/// dB per profile point, a whine narrower than its point keeping its peak:
/// the body of a learned profile.
#[must_use]
pub fn learned_points(interleaved: &[f32], channels: usize, rate: u32) -> Vec<f64> {
    let framing = Framing::at(rate);
    let mut sum = vec![0.0f64; framing.bins];
    let mut count = 0usize;
    each_frame(interleaved, channels, None, &framing, |power| {
        for (s, p) in sum.iter_mut().zip(power) {
            *s += f64::from(*p);
        }
        count += 1;
    });
    let density: Vec<f64> = sum
        .iter()
        .map(|s| {
            if count == 0 {
                0.0
            } else {
                s / count as f64 / framing.psd_scale
            }
        })
        .collect();
    to_grid(&density, framing.bin_hz, &grid())
}

/// A profile's points as the plugin_state blob.
#[must_use]
pub fn encode(points_db: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + POINTS * 2);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(0);
    out.extend_from_slice(&(POINTS as u16).to_le_bytes());
    for i in 0..POINTS {
        let db = points_db.get(i).copied().unwrap_or(-300.0);
        let centi = (db * 100.0)
            .round()
            .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
        out.extend_from_slice(&centi.to_le_bytes());
    }
    out
}

/// A blob's points, in dB of power per hertz, or `None` for anything that is
/// not a profile this version wrote.
#[must_use]
pub fn decode(blob: &[u8]) -> Option<Vec<f64>> {
    if blob.get(..4)? != MAGIC || *blob.get(4)? != VERSION {
        return None;
    }
    let count = u16::from_le_bytes([*blob.get(6)?, *blob.get(7)?]) as usize;
    if count != POINTS || blob.len() != HEADER + POINTS * 2 {
        return None;
    }
    Some(
        blob.get(HEADER..)?
            .chunks_exact(2)
            .map(|b| f64::from(i16::from_le_bytes([b[0], b[1]])) / 100.0)
            .collect(),
    )
}

/// A blob's floor as each bin's expected power at this framing.
#[must_use]
pub fn floor_of_blob(blob: &[u8], framing: &Framing) -> Option<Vec<f64>> {
    let points = decode(blob)?;
    Some(
        from_grid(&points, &grid(), framing.bins, framing.bin_hz)
            .into_iter()
            .map(|d| d * framing.psd_scale)
            .collect(),
    )
}

/// The adaptive floor of one channel, as each bin's expected power, or `None`
/// when the channel holds nothing but silence.
#[must_use]
pub fn adaptive_floor(
    interleaved: &[f32],
    channels: usize,
    channel: usize,
    framing: &Framing,
) -> Option<Vec<f64>> {
    let bins = framing.bins;
    let mut hist = vec![0u32; bins * HIST_BUCKETS];
    let mut ring = vec![vec![0.0f32; bins]; 3];
    let mut seen = 0usize;
    let scale = 1.0 / framing.psd_scale;
    let add = |hist: &mut [u32], ring: &[Vec<f32>], frames: usize| {
        for k in 0..bins {
            let mean: f64 = ring
                .iter()
                .take(frames)
                .map(|r| f64::from(r.get(k).copied().unwrap_or(0.0)))
                .sum::<f64>()
                / frames as f64;
            let db = 10.0 * (mean * scale).max(1e-30).log10();
            let b =
                ((db - HIST_LOW_DB) / HIST_STEP_DB).clamp(0.0, (HIST_BUCKETS - 1) as f64) as usize;
            if let Some(slot) = hist.get_mut(k * HIST_BUCKETS + b) {
                *slot += 1;
            }
        }
    };
    each_frame(interleaved, channels, Some(channel), framing, |power| {
        if let Some(slot) = ring.get_mut(seen % 3) {
            slot.copy_from_slice(power);
        }
        seen += 1;
        if seen >= 3 {
            add(&mut hist, &ring, 3);
        }
    });
    match seen {
        0 => return None,
        1 | 2 => add(&mut hist, &ring, seen),
        _ => {}
    }
    let raw: Vec<f64> = (0..bins)
        .map(|k| {
            let row = hist
                .get(k * HIST_BUCKETS..(k + 1) * HIST_BUCKETS)
                .unwrap_or(&[]);
            let total: u64 = row.iter().map(|&c| u64::from(c)).sum();
            let want = ((total as f64 * PERCENTILE).ceil() as u64).max(1);
            let mut acc = 0u64;
            let mut bucket = 0usize;
            for (b, &c) in row.iter().enumerate() {
                acc += u64::from(c);
                if acc >= want {
                    bucket = b;
                    break;
                }
            }
            let db = HIST_LOW_DB + (bucket as f64 + 0.5) * HIST_STEP_DB;
            10f64.powf(db / 10.0) * BIAS * framing.psd_scale
        })
        .collect();
    let smooth = sixth_octave(&raw);
    let excess = tonal_excess(&raw);
    // A tonal peak keeps its own height: smoothing would spread a whine over
    // its neighbours and leave its centre under-removed.
    Some(
        smooth
            .iter()
            .zip(&raw)
            .zip(&excess)
            .map(|((&s, &r), &e)| if f64::from(e) > TONAL_DB { r } else { s })
            .collect(),
    )
}

/// The mean power across a sixth of an octave around each bin, at least one
/// bin either side.
fn sixth_octave(power: &[f64]) -> Vec<f64> {
    let bins = power.len();
    let mut sums = Vec::with_capacity(bins + 1);
    let mut acc = 0.0f64;
    sums.push(0.0);
    for p in power {
        acc += p;
        sums.push(acc);
    }
    (0..bins)
        .map(|k| {
            let reach = ((0.028_9 * k as f64).round() as usize).max(1);
            let lo = k.saturating_sub(reach);
            let hi = (k + reach + 1).min(bins);
            (sums.get(hi).copied().unwrap_or(0.0) - sums.get(lo).copied().unwrap_or(0.0))
                / (hi - lo).max(1) as f64
        })
        .collect()
}

/// How far each bin of a floor stands over the median of a sixth of an octave
/// around it, in dB, never below nought: what a steady whine or hum looks
/// like in a profile.
#[must_use]
pub fn tonal_excess(floor: &[f64]) -> Vec<f32> {
    let bins = floor.len();
    let db: Vec<f64> = floor.iter().map(|p| 10.0 * p.max(1e-30).log10()).collect();
    let mut scratch = Vec::with_capacity(256);
    (0..bins)
        .map(|k| {
            let reach = ((0.057_8 * k as f64).round() as usize).clamp(3, 120);
            let lo = k.saturating_sub(reach);
            let hi = (k + reach + 1).min(bins);
            scratch.clear();
            scratch.extend_from_slice(db.get(lo..hi).unwrap_or(&[]));
            let mid = scratch.len() / 2;
            if scratch.is_empty() {
                return 0.0;
            }
            let (_, median, _) = scratch.select_nth_unstable_by(mid, f64::total_cmp);
            (db.get(k).copied().unwrap_or(0.0) - *median).max(0.0) as f32
        })
        .collect()
}

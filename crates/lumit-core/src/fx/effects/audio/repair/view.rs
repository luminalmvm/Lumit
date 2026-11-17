//! What the repair effects' displays draw, worked out from a window of the
//! effect's input around the playhead.
//!
//! # In plain terms
//!
//! Each function here is pure: the instance's row values (as the bake would
//! hand them, Switches as nought or one), its plugin_state where it has one, a
//! mono window of the sound arriving at the effect, and the rate. It answers
//! with numbers for the bridge to draw, and draws nothing itself.
//!
//! A spectrum is [`POINTS`] dB values on one log frequency grid from 20 Hz to
//! 20 kHz ([`grid_hz`]), scaled so a full-scale sine reads nought dB. A
//! quarter of a second is a good window for a spectrum and two seconds for
//! clicks. A curve the effect has no use for is `None`.
//!
//! The windows are short, so where an effect learns from its whole input the
//! display estimates the same thing from the window instead: Noise
//! reduction's adaptive floor, Hum removal's frequency, Reverb reduction's
//! reverb time. Each call allocates its own scratch, and nothing here runs per
//! block.

use super::declick::find_clicks;
use super::deess::{mean_reduction, AudioDeess};
use super::dehum::HumFinder;
use super::denoise::{AudioDenoise, Channel};
use super::dereverb::{decay_per_hop, find_reverb_time, AudioDereverb, Room};
use super::profile::{adaptive_floor, floor_of_blob, Framing};
use super::{log_grid, on, row, to_grid};
use crate::fx::effects::audio::dsp::stft::{frame_at, frame_count, hann, Fft};
use crate::fx::params::ParamId;

/// Points on a display's frequency axis.
pub const POINTS: usize = 128;

/// The lowest a display value goes, in dB.
pub const FLOOR_DB: f32 = -150.0;

/// The display's frequency axis: [`POINTS`] points from 20 Hz to 20 kHz,
/// evenly spaced in log frequency.
#[must_use]
pub fn grid_hz() -> Vec<f64> {
    log_grid(20.0, 20_000.0, POINTS)
}

/// The four curves a spectral repair effect's display draws, each
/// [`POINTS`] dB values on [`grid_hz`].
#[derive(Clone, Debug, PartialEq)]
pub struct SpectrumView {
    /// What arrives at the effect.
    pub input: Vec<f32>,
    /// The unwanted part's level the effect is working from: Noise
    /// reduction's floor, Reverb reduction's late reverb.
    pub floor: Option<Vec<f32>>,
    /// Where the effect starts to act: the floor raised by Threshold for
    /// Noise reduction; for the De-esser, the level the band above Frequency
    /// would have to reach, [`FLOOR_DB`] below it.
    pub threshold: Option<Vec<f32>>,
    /// What the effect is expected to let through.
    pub output: Vec<f32>,
}

/// What Hum removal's display draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HumView {
    /// The mains frequency found in the window, in hertz, or `None` when no
    /// hum stands out.
    pub frequency: Option<f32>,
    /// How many of its first 24 harmonics agree on that frequency.
    pub harmonics: u32,
}

/// Every frame's power per bin over a mono window, at the shared framing.
fn powers(window: &[f32], framing: &Framing) -> Vec<Vec<f32>> {
    let mut fft = Fft::new(framing.n);
    let mut frame = vec![0.0f32; framing.n];
    let mut spectrum = fft.spectrum();
    (0..frame_count(window.len(), framing.n, framing.hop))
        .map(|j| {
            frame_at(window, 1, 0, j, framing.hop, &framing.window, &mut frame);
            fft.forward(&mut frame, &mut spectrum);
            spectrum.iter().map(|x| x.norm_sqr()).collect()
        })
        .collect()
}

/// The mean of each bin over the frames.
fn mean(frames: &[Vec<f32>], bins: usize) -> Vec<f64> {
    let mut sum = vec![0.0f64; bins];
    for frame in frames {
        for (s, p) in sum.iter_mut().zip(frame) {
            *s += f64::from(*p);
        }
    }
    let count = frames.len().max(1) as f64;
    sum.iter().map(|s| s / count).collect()
}

/// Power per bin onto the display's grid, so a full-scale sine reads nought.
fn display(power: &[f64], framing: &Framing) -> Vec<f32> {
    let sum: f64 = framing.window.iter().map(|w| f64::from(*w)).sum();
    let scale = 4.0 / (sum * sum).max(1e-30);
    let scaled: Vec<f64> = power.iter().map(|p| p * scale).collect();
    to_grid(&scaled, framing.bin_hz, &grid_hz())
        .into_iter()
        .map(|d| (d as f32).max(FLOOR_DB))
        .collect()
}

/// Noise reduction's display: the input, the floor in use (the learned
/// profile, or with Adaptive on or nothing learned an estimate from the
/// window), the floor raised by Threshold, and the output.
#[must_use]
pub fn noise_reduction(
    values: &[(ParamId, f64)],
    state: Option<&[u8]>,
    window: &[f32],
    rate: u32,
) -> SpectrumView {
    let framing = Framing::at(rate);
    let frames = powers(window, &framing);
    let adaptive = on(values, AudioDenoise::ADAPTIVE, true);
    let learned = state
        .filter(|_| !adaptive)
        .and_then(|blob| floor_of_blob(blob, &framing));
    let floor = learned.or_else(|| adaptive_floor(window, 1, 0, &framing));
    let input = mean(&frames, framing.bins);
    let Some(floor) = floor else {
        let shown = display(&input, &framing);
        return SpectrumView {
            input: shown.clone(),
            floor: None,
            threshold: None,
            output: shown,
        };
    };
    let threshold = row(values, AudioDenoise::THRESHOLD, 0.0).clamp(-10.0, 20.0);
    let raise = 10f64.powf(threshold / 10.0);
    let raised: Vec<f64> = floor.iter().map(|f| f * raise).collect();

    let mut channel = Channel::new(&framing);
    channel.set_floor(&floor);
    channel.set_lows(
        row(values, AudioDenoise::REDUCTION, 12.0).clamp(0.0, 40.0),
        row(values, AudioDenoise::TONAL, 0.0).clamp(0.0, 40.0),
    );
    channel
        .gain
        .set_smoothing(row(values, AudioDenoise::SMOOTHING, 50.0));
    let noise: Vec<f32> = raised.iter().map(|&n| n as f32).collect();
    let mut out = vec![0.0f64; framing.bins];
    for frame in &frames {
        let gains = channel.gain.gains(frame, &noise, &channel.lows);
        for ((o, p), g) in out.iter_mut().zip(frame).zip(gains) {
            *o += f64::from(*p) * f64::from(g * g);
        }
    }
    let count = frames.len().max(1) as f64;
    let out: Vec<f64> = out.iter().map(|o| o / count).collect();
    SpectrumView {
        input: display(&input, &framing),
        floor: Some(display(&floor, &framing)),
        threshold: Some(display(&raised, &framing)),
        output: display(&out, &framing),
    }
}

/// Hum removal's display: the mains frequency found in the window and how
/// many harmonics of it stand out. The window is zero-padded to, or cut to
/// the middle of, the finder's 1.4 s transform.
#[must_use]
pub fn hum_removal(_values: &[(ParamId, f64)], window: &[f32], rate: u32) -> HumView {
    let mut finder = HumFinder::new(rate);
    let n = finder.n;
    let take = window.len().min(n);
    let skip = (window.len() - take) / 2;
    let shape = hann(take);
    let mut frame = vec![0.0f32; n];
    for ((f, x), w) in frame
        .iter_mut()
        .zip(window.get(skip..).unwrap_or(&[]))
        .zip(&shape)
    {
        *f = x * w;
    }
    finder.analyse(&mut frame);
    // One window has nothing to check a reading against but itself, so a
    // family counts only where its harmonics agree on the fundamental.
    let best = match (finder.agreed(50.0, 24), finder.agreed(60.0, 24)) {
        (Some(fifty), Some(sixty)) => Some(if sixty.1 > fifty.1 { sixty } else { fifty }),
        (fifty, sixty) => fifty.or(sixty),
    };
    HumView {
        frequency: best.map(|(f0, _)| f0 as f32),
        harmonics: best.map_or(0, |(_, agreeing)| agreeing),
    }
}

/// Click removal's display: each click found in the window as its first
/// sample in the window and the sample after its last, in order.
#[must_use]
pub fn click_removal(values: &[(ParamId, f64)], window: &[f32], rate: u32) -> Vec<(u32, u32)> {
    find_clicks(window, values, rate)
        .into_iter()
        .map(|(at, len)| (at as u32, len as u32))
        .collect()
}

/// The De-esser's display: the input, where the band above Frequency would
/// start being turned down, and the output with the window's mean reduction
/// applied above Frequency (or everywhere, with Whole sound on). No floor.
#[must_use]
pub fn de_esser(values: &[(ParamId, f64)], window: &[f32], rate: u32) -> SpectrumView {
    let framing = Framing::at(rate);
    let frames = powers(window, &framing);
    let input = mean(&frames, framing.bins);
    let hz =
        row(values, AudioDeess::FREQUENCY, super::deess::FREQUENCY_HZ).clamp(2_000.0, 12_000.0);
    let threshold = row(values, AudioDeess::THRESHOLD, -20.0).clamp(-40.0, 0.0);
    let wideband = on(values, AudioDeess::WIDEBAND, false);
    let split = (hz / framing.bin_hz).round() as usize;
    let full: f64 = input.iter().sum();
    let high: f64 = input.iter().skip(split).sum();
    // The level the high band would need for its share of the whole to reach
    // the threshold, spread across it in the shape it has now.
    let reach = full * 10f64.powf(threshold / 10.0) / high.max(1e-30);
    let line: Vec<f64> = input
        .iter()
        .enumerate()
        .map(|(k, p)| if k >= split { p * reach } else { 0.0 })
        .collect();
    let cut = 10f64.powf(-mean_reduction(window, values, rate) / 10.0);
    let out: Vec<f64> = input
        .iter()
        .enumerate()
        .map(|(k, p)| if wideband || k >= split { p * cut } else { *p })
        .collect();
    SpectrumView {
        input: display(&input, &framing),
        floor: None,
        threshold: Some(display(&line, &framing)),
        output: display(&out, &framing),
    }
}

/// Reverb reduction's display: the input, the late reverb estimated over the
/// window at the reverb time in use, and the output. With Find reverb time on
/// the time is measured from the window where it can be, the row's otherwise.
#[must_use]
pub fn reverb_reduction(values: &[(ParamId, f64)], window: &[f32], rate: u32) -> SpectrumView {
    let framing = Framing::at(rate);
    let frames = powers(window, &framing);
    let input = mean(&frames, framing.bins);
    let found = if on(values, AudioDereverb::AUTO, true) {
        find_reverb_time(window, 1, rate)
    } else {
        None
    };
    let t60 = found
        .unwrap_or_else(|| row(values, AudioDereverb::REVERB_TIME, 0.8))
        .clamp(0.1, 3.0);
    let decay = decay_per_hop(t60, framing.hop_s());
    let low =
        10f64.powf(-row(values, AudioDereverb::REDUCTION, 12.0).clamp(0.0, 30.0) / 20.0) as f32;
    let mut room = Room::new(&framing);
    room.set_smoothing(row(values, AudioDereverb::SMOOTHING, 50.0));
    let mut gains = vec![1.0f32; framing.bins];
    let mut late = vec![0.0f32; framing.bins];
    let mut late_sum = vec![0.0f64; framing.bins];
    let mut out = vec![0.0f64; framing.bins];
    for frame in &frames {
        room.late(frame, decay, &mut late);
        room.gains(frame, &late, low, &mut gains);
        for (((o, l), p), (g, s)) in out
            .iter_mut()
            .zip(&late)
            .zip(frame)
            .zip(gains.iter().zip(late_sum.iter_mut()))
        {
            *o += f64::from(*p) * f64::from(g * g);
            *s += f64::from(*l);
        }
    }
    let count = frames.len().max(1) as f64;
    let out: Vec<f64> = out.iter().map(|o| o / count).collect();
    let late: Vec<f64> = late_sum.iter().map(|l| l / count).collect();
    SpectrumView {
        input: display(&input, &framing),
        floor: Some(display(&late, &framing)),
        threshold: None,
        output: display(&out, &framing),
    }
}

/// The reverb time found in a mono window, in seconds, or `None` where it
/// holds too few decays to say. It wants several seconds with pauses in them;
/// four ending at the playhead is a good window.
#[must_use]
pub fn reverb_time(window: &[f32], rate: u32) -> Option<f32> {
    find_reverb_time(window, 1, rate).map(|t| t as f32)
}

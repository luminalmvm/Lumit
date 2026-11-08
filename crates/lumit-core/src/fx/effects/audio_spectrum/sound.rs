//! What Audio spectrum reads of the sound: a level per band, or the wave itself.
//!
//! Done here rather than in the host so the picture and the points stream can't
//! read the sound differently, and so it can be tested against a made-up tone.
//! Deterministic per (sound, time, rows): the same samples through the same sums.

use std::sync::Arc;

use crate::expression::ExpressionContext;
use crate::fx::drivers::audio_level::{SOURCE_CLIP, SOURCE_LAYER};
use crate::fx::fft::{fft_inplace, Cx};
use crate::fx::registry::AudioTap;
use crate::model::{EffectInstance, EffectValue};

use super::{AudioSpectrum, SHAPE_WAVEFORM};

/// The samples one measurement takes. At the tap's 48 kHz that is 85 ms, which
/// tells 12 Hz apart at the low end and still follows a drum.
const WINDOW: usize = 4096;

/// How far below full scale a band still shows, in dB. Quieter draws nothing.
const FLOOR_DB: f32 = -60.0;

/// The reading of a silent sound.
#[must_use]
pub fn silence(s: &AudioSpectrum) -> Vec<f32> {
    vec![0.0; s.bands.max(1) as usize]
}

/// This frame's reading of the instance's sound: `bands` numbers, nought to one
/// for a spectrum and minus one to one for a Waveform. All silence without a tap
/// or a sound, which is the same no-op a dangling reference gives everywhere.
///
/// `lt` is layer time, for the rows; the tap centres its window on the host's
/// own clock. The rows are read off the instance, so a driver wired into Bands
/// or a frequency moves the drawing but not the measurement.
#[must_use]
pub fn reading(
    inst: &EffectInstance,
    lt: f64,
    fps: f64,
    context: &Arc<ExpressionContext>,
    tap: Option<&dyn AudioTap>,
) -> Vec<f32> {
    let row = |id: &str, or: f64| {
        inst.float_at_with_context(id, lt, context.clone())
            .unwrap_or(or)
    };
    let bands = (row("bands", 64.0).round() as i64).clamp(2, 512) as usize;
    let quiet = vec![0.0; bands];
    let Some(tap) = tap else {
        return quiet;
    };
    let frames = row("smoothing", 2.0).round().clamp(0.0, 120.0) as usize;
    let step = 1.0 / fps.max(1.0);
    let wave = matches!(
        inst.param("shape"),
        Some(EffectValue::Choice(SHAPE_WAVEFORM))
    );
    // The whole stretch is read as one window centred on the frame, with the
    // frames before it inside.
    let half = if wave {
        step * (1 + frames) as f64 / 2.0
    } else {
        step * frames as f64 + WINDOW as f64 / 48_000.0
    };
    let mut samples = Vec::new();
    let Some(rate) = read(inst, tap, half, &mut samples) else {
        return quiet;
    };
    if samples.is_empty() || rate <= 0.0 {
        return quiet;
    }
    if wave {
        return blocks(&samples, bands);
    }
    let low = row("start_frequency", 40.0).max(1.0);
    let high = row("end_frequency", 16000.0).max(low + 1.0);
    let edges = band_edges(bands, low, high);
    let mid = samples.len() / 2;
    let back = (step * rate).round() as usize;
    let mut sum = vec![0.0f32; bands];
    let mut count = 0.0f32;
    for k in 0..=frames {
        let Some(centre) = mid.checked_sub(k * back) else {
            break;
        };
        let bins = magnitudes(&samples, centre);
        for (band, level) in sum.iter_mut().enumerate() {
            *level += band_level(&bins, edges[band], edges[band + 1], rate);
        }
        count += 1.0;
    }
    if count > 0.0 {
        for level in &mut sum {
            *level /= count;
        }
    }
    sum
}

/// The window of sound the Source row asks for, and the rate it came at.
fn read(inst: &EffectInstance, tap: &dyn AudioTap, half: f64, out: &mut Vec<f32>) -> Option<f64> {
    let layer = inst.layer_ref("audio");
    match inst.param("source") {
        Some(EffectValue::Choice(SOURCE_LAYER)) => {
            layer.and_then(|l| tap.strip(Some(l), None, half, out))
        }
        Some(EffectValue::Choice(SOURCE_CLIP)) => layer
            .zip(inst.clip_ref("clip"))
            .and_then(|(l, c)| tap.strip(Some(l), Some(c), half, out)),
        _ => tap.mix(half, out),
    }
}

/// `bands + 1` edges spaced evenly in pitch from `low` to `high` hertz, which is
/// how the ear spaces them: an octave at the bottom gets as many bars as an
/// octave at the top.
fn band_edges(bands: usize, low: f64, high: f64) -> Vec<f64> {
    let ratio = high / low;
    (0..=bands)
        .map(|i| low * ratio.powf(i as f64 / bands as f64))
        .collect()
}

/// One Hann-windowed measurement centred on `centre`: each bin's magnitude, with
/// a full-scale sine reading 1. Samples off either end read as silence.
fn magnitudes(samples: &[f32], centre: usize) -> Vec<f32> {
    let start = centre as i64 - (WINDOW / 2) as i64;
    let mut data = Vec::with_capacity(WINDOW);
    let mut gain = 0.0f64;
    for i in 0..WINDOW {
        let hann = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (WINDOW - 1) as f64).cos();
        gain += hann;
        let at = start + i as i64;
        let sample = usize::try_from(at)
            .ok()
            .and_then(|at| samples.get(at))
            .copied()
            .unwrap_or(0.0);
        data.push(Cx::new(f64::from(sample) * hann, 0.0));
    }
    fft_inplace(&mut data, false);
    // The transform is unitary, so a bin comes back √N down on the plain sum.
    let scale = 2.0 * (WINDOW as f64).sqrt() / gain.max(1e-9);
    data[..WINDOW / 2]
        .iter()
        .map(|c| (c.norm_sq().sqrt() * scale) as f32)
        .collect()
}

/// The loudest bin between two pitches, as nought to one in dB. A band too narrow
/// to hold a bin, as the low ones are, reads the bin it falls in.
fn band_level(bins: &[f32], low: f64, high: f64, rate: f64) -> f32 {
    let per_bin = rate / WINDOW as f64;
    let last = bins.len().saturating_sub(1);
    let from = ((low / per_bin).floor() as usize).min(last);
    let to = ((high / per_bin).ceil() as usize).clamp(from + 1, bins.len().max(from + 1));
    let peak = bins[from..to.min(bins.len())]
        .iter()
        .copied()
        .fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// The wave cut into `count` blocks, each giving the sample furthest from nought
/// with its sign, so a short sound still fills the line and a peak isn't averaged
/// away.
fn blocks(samples: &[f32], count: usize) -> Vec<f32> {
    let n = samples.len();
    (0..count)
        .map(|i| {
            let (a, b) = (i * n / count, ((i + 1) * n / count).max(i * n / count + 1));
            samples[a..b.min(n)]
                .iter()
                .copied()
                .fold(
                    0.0f32,
                    |held, s| if s.abs() > held.abs() { s } else { held },
                )
                .clamp(-1.0, 1.0)
        })
        .collect()
}

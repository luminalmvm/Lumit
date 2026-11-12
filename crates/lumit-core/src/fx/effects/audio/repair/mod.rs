//! Repair: the audio effects that take something unwanted out of a recording.
//!
//! # In plain terms
//!
//! Five effects, each removing one kind of damage: steady noise, mains hum,
//! clicks, harsh esses and room reverb. Each is the house shape, a row struct,
//! a definition and a processor, and each has an "only" switch that plays
//! what it took away, which is how a user hears whether the voice went with
//! it.
//!
//! Three of them look at their whole input before the first block
//! ([`AudioProcessor::look`](crate::fx::audio_chain::AudioProcessor::look)):
//! Noise reduction for its floor, Hum removal for the mains frequency, its
//! drift and each harmonic's level, Reverb reduction for the room's decay and
//! Best quality's prediction. Each says so through `bakes_whole`, so a rack
//! holding one is baked from its first sample and the preview hears what the
//! export writes; and each copes with a first sample in the middle of a
//! sound, since a trimmed clip starts wherever it was cut.
//!
//! [`view`] holds the pure functions the effect displays draw from, and
//! [`denoise::learn_profile`] the one the Learn button calls.
//!
//! Runs on whichever thread bakes the chain; the heavy looks fan out over the
//! rayon pool and come back in a fixed order.

pub mod declick;
pub mod deess;
pub mod dehum;
pub mod denoise;
pub mod dereverb;
mod hum_fit;
pub mod profile;
pub mod view;
mod wpe;

use crate::fx::params::ParamId;

/// What `id` holds this block, or `default` where the bake sent nothing.
fn row(values: &[(ParamId, f64)], id: ParamId, default: f64) -> f64 {
    values
        .iter()
        .find(|(held, _)| *held == id)
        .map_or(default, |(_, value)| *value)
        .clamp(-1e9, 1e9)
}

/// A switch row: the bake sends a switch as nought or one.
fn on(values: &[(ParamId, f64)], id: ParamId, default: bool) -> bool {
    row(values, id, f64::from(u8::from(default))) > 0.5
}

/// The first block's values, which is what a row read once per look means.
fn first(values: &[Vec<(ParamId, f64)>]) -> &[(ParamId, f64)] {
    values.first().map_or(&[][..], Vec::as_slice)
}

/// Power per bin onto points of a log frequency grid, in dB.
///
/// A point whose share of the axis covers whole bins takes their mean power,
/// so a wide cell at the top hears every bin in it, or their peak where the
/// peak stands [`profile::TONAL_DB`] over the mean; a point narrower than a
/// bin, at the bottom, reads straight between its two neighbours.
fn to_grid(power: &[f64], bin_hz: f64, grid: &[f64]) -> Vec<f64> {
    let bins = power.len();
    let at = |k: usize| power.get(k).copied().unwrap_or(0.0);
    let edge = |i: usize| -> f64 {
        // The geometric midpoint between neighbouring points.
        let a = grid.get(i.saturating_sub(1)).copied().unwrap_or(1.0);
        let b = grid.get(i).copied().unwrap_or(a);
        (a * b).sqrt()
    };
    (0..grid.len())
        .map(|i| {
            let hz = grid.get(i).copied().unwrap_or(0.0);
            let lo = if i == 0 { hz / 1.01 } else { edge(i) };
            let hi = if i + 1 == grid.len() {
                hz * 1.01
            } else {
                edge(i + 1)
            };
            let k0 = (lo / bin_hz).ceil().max(1.0) as usize;
            let k1 = ((hi / bin_hz).floor() as usize).min(bins.saturating_sub(1));
            let p = if k1 >= k0 && bin_hz > 0.0 {
                let mean = (k0..=k1).map(at).sum::<f64>() / (k1 - k0 + 1) as f64;
                let peak = (k0..=k1).map(at).fold(0.0f64, f64::max);
                // A whine narrower than the cell keeps its own height, or
                // the cell's mean would hide it under the floor around it.
                if peak > mean * 10f64.powf(profile::TONAL_DB / 10.0) {
                    peak
                } else {
                    mean
                }
            } else {
                let x = (hz / bin_hz.max(1e-9)).clamp(1.0, bins.saturating_sub(1).max(1) as f64);
                let k = x.floor() as usize;
                let f = x - k as f64;
                at(k) * (1.0 - f) + at(k + 1) * f
            };
            10.0 * p.max(1e-30).log10()
        })
        .collect()
}

/// A log frequency grid's dB values back onto `bins` bins `bin_hz` apart, as
/// power: straight lines between points in dB against log frequency, the
/// ends held flat.
fn from_grid(db: &[f64], grid: &[f64], bins: usize, bin_hz: f64) -> Vec<f64> {
    let first_hz = grid.first().copied().unwrap_or(1.0).max(1e-9);
    let ratio = (grid.last().copied().unwrap_or(first_hz) / first_hz).max(1.0 + 1e-9);
    let last = grid.len().saturating_sub(1).max(1) as f64;
    (0..bins)
        .map(|k| {
            let hz = (k as f64 * bin_hz).max(first_hz);
            let x = ((hz / first_hz).ln() / ratio.ln() * last).clamp(0.0, last);
            let i = x.floor() as usize;
            let f = x - i as f64;
            let a = db.get(i).copied().unwrap_or(-300.0);
            let b = db.get(i + 1).copied().unwrap_or(a);
            10f64.powf((a * (1.0 - f) + b * f) / 10.0)
        })
        .collect()
}

/// `count` points spaced evenly in log frequency from `low` to `high` Hz.
fn log_grid(low: f64, high: f64, count: usize) -> Vec<f64> {
    let steps = count.saturating_sub(1).max(1) as f64;
    (0..count)
        .map(|i| low * (high / low).powf(i as f64 / steps))
        .collect()
}

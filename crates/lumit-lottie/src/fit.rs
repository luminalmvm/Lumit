//! Fitting: values sampled frame by frame as the fewest straight keys that
//! reproduce them.
//!
//! What can't be written as keys (an expression, a colour Lumit blends in
//! linear light, a number driven by a Lumit-only modifier) is sampled at
//! every output frame and then thinned: a sample is dropped wherever the
//! straight line between the samples kept either side of it passes within
//! the tolerance of it. So a still result is one key, a steady move two, and
//! a wiggle keeps the frames it needs and no more. The file is then exact at
//! every frame a player draws, to the tolerance.
//!
//! Iterative, with an explicit stack, so a long bake can't run the stack out.

use crate::keys::{Track, TrackKey, LINEAR_IN, LINEAR_OUT};

/// The fewest keys, straight between each pair, that pass within `tolerance`
/// of every sample, dimension by dimension. `samples` are (seconds, values),
/// in time order, each with as many values as `tolerance` has entries.
#[must_use]
pub fn linear(samples: &[(f64, Vec<f64>)], tolerance: &[f64]) -> Track {
    let Some(first) = samples.first() else {
        return Track { keys: Vec::new() };
    };
    let dims = tolerance.len();
    let still = samples.iter().all(|(_, v)| {
        (0..dims).all(|d| {
            let (a, b) = (
                v.get(d).copied().unwrap_or(0.0),
                first.1.get(d).copied().unwrap_or(0.0),
            );
            (a - b).abs() <= tolerance.get(d).copied().unwrap_or(0.0)
        })
    });
    if still || samples.len() == 1 {
        let mut track = Track::still(first.1.clone());
        if let Some(key) = track.keys.first_mut() {
            key.seconds = first.0;
        }
        return track;
    }

    let last = samples.len() - 1;
    let mut keep = vec![false; samples.len()];
    keep[0] = true;
    keep[last] = true;
    let mut stack = vec![(0usize, last)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let (Some((ti, vi)), Some((tj, vj))) = (samples.get(i), samples.get(j)) else {
            continue;
        };
        // The sample furthest outside the tolerance, in units of it.
        let mut worst = (0usize, 1.0f64);
        for (k, (tk, vk)) in samples.iter().enumerate().take(j).skip(i + 1) {
            let s = if tj > ti { (tk - ti) / (tj - ti) } else { 0.0 };
            for d in 0..dims {
                let (a, b, v) = (
                    vi.get(d).copied().unwrap_or(0.0),
                    vj.get(d).copied().unwrap_or(0.0),
                    vk.get(d).copied().unwrap_or(0.0),
                );
                let line = a + (b - a) * s;
                let tol = tolerance.get(d).copied().unwrap_or(0.0).max(1e-12);
                let off = (v - line).abs() / tol;
                if off > worst.1 {
                    worst = (k, off);
                }
            }
        }
        if worst.0 > i {
            keep[worst.0] = true;
            stack.push((i, worst.0));
            stack.push((worst.0, j));
        }
    }

    let kept: Vec<&(f64, Vec<f64>)> = samples
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(s, _)| s)
        .collect();
    let count = kept.len();
    let keys = kept
        .into_iter()
        .enumerate()
        .map(|(n, (t, v))| {
            let last = n + 1 == count;
            TrackKey {
                seconds: *t,
                value: v.clone(),
                hold: false,
                out: if last {
                    Vec::new()
                } else {
                    vec![LINEAR_OUT; dims]
                },
                inn: if last {
                    Vec::new()
                } else {
                    vec![LINEAR_IN; dims]
                },
            }
        })
        .collect();
    Track { keys }
}

/// How finely each kind of number is kept when it is sampled: what can't be
/// seen at the size it is drawn.
pub mod tolerance {
    /// Pixels: a hundredth of one.
    pub const PIXELS: f64 = 0.01;
    /// Degrees.
    pub const DEGREES: f64 = 0.01;
    /// Per cent.
    pub const PERCENT: f64 = 0.01;
    /// A colour channel from 0 to 1: half a step of eight bits.
    pub const CHANNEL: f64 = 0.5 / 255.0;
    /// Seconds (a Retime's source time): a tenth of a millisecond.
    pub const SECONDS: f64 = 1e-4;
}

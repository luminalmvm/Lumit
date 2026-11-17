//! The waveform of a node of a plan: the master after its fader for the
//! Sound mix row, or what is routed into a bus for its lane.
//!
//! Runs on whichever thread asks and waits for decodes and peak pyramids, so
//! never on the callback.

use lumit_audio::block::{DrySource, MixWork, SCRATCH_FRAMES};
use lumit_audio::peaks::{Band, PeakPyramid};
use lumit_audio::plan::{MixPlan, NONE, PEAK_SAMPLES_PER_BUCKET};
use lumit_media::audio::SoundSource;
use std::sync::Arc;

/// How many frames a bucket may cover and still be read from the samples.
/// Past it the picture is put together from the files' own peaks.
const READ_FRAMES: f64 = 1024.0;

/// `node`'s sound summarised over `[start_s, end_s)` in `buckets` buckets,
/// `[min, max, rms]` per bucket as [`MixPlan::peaks`] lays them out. `None` is
/// the master after its fader (racks dry, a plain clamp at the ceiling for
/// the limiter); `Some(bus)` is the bus's input: what reaches it through each
/// source's fader and send level, before its rack and fader.
///
/// A narrow window is mixed sample by sample; a wide one is put together from
/// each clip's file pyramid (asked of `pyramid_of`) scaled by its path's
/// gains at each bucket's middle.
#[must_use]
pub fn node_peaks(
    plan: &MixPlan,
    rate: u32,
    node: Option<u16>,
    start_s: f64,
    end_s: f64,
    buckets: usize,
    pyramid_of: &mut dyn FnMut(&SoundSource) -> Option<Arc<PeakPyramid>>,
) -> Vec<f32> {
    if buckets == 0 || rate == 0 || end_s <= start_s {
        return vec![0.0; buckets * 3];
    }
    let rate_f = f64::from(rate);
    if (end_s - start_s) * rate_f / buckets as f64 <= READ_FRAMES {
        return narrow(plan, rate, node, start_s, end_s, buckets);
    }
    match node {
        None => plan.peaks_wide(rate, start_s, end_s, buckets, pyramid_of),
        Some(bus) => wide(
            plan,
            rate,
            usize::from(bus),
            start_s,
            end_s,
            buckets,
            pyramid_of,
        ),
    }
}

/// [`node_peaks`] read off the mix itself.
fn narrow(
    plan: &MixPlan,
    rate: u32,
    node: Option<u16>,
    start_s: f64,
    end_s: f64,
    buckets: usize,
) -> Vec<f32> {
    let rate_f = f64::from(rate);
    let from = (start_s.max(0.0) * rate_f).floor() as usize;
    let to = ((end_s.max(0.0) * rate_f).ceil() as usize).min(plan.total_frames);
    let mut sums = vec![(0.0f32, 0.0f32, 0.0f64, 0u32); buckets];
    if to > from {
        plan.warm(from, to);
        let mut work = MixWork::new(rate);
        work.set_limiter(false);
        let step_s = (end_s - start_s) / buckets as f64;
        let mut chunk = vec![0.0f32; 16 * SCRATCH_FRAMES * 2];
        let mut at = from;
        while at < to {
            let n = (to - at).min(16 * SCRATCH_FRAMES);
            let out = &mut chunk[..n * 2];
            plan.mix_block(at, out, &mut work, &mut DrySource, None, node);
            for (k, f) in out.chunks_exact(2).enumerate() {
                let t = (at + k) as f64 / rate_f;
                let b = (((t - start_s) / step_s).floor().max(0.0) as usize).min(buckets - 1);
                let s = &mut sums[b];
                s.0 = s.0.min(f[0].min(f[1]));
                s.1 = s.1.max(f[0].max(f[1]));
                s.2 += f64::from(f[0] * f[0] + f[1] * f[1]) * 0.5;
                s.3 += 1;
            }
            at += n;
        }
    }
    sums.into_iter()
        .flat_map(|(lo, hi, sq, n)| {
            let rms = if n == 0 {
                0.0
            } else {
                (sq / f64::from(n)).sqrt() as f32
            };
            [lo, hi, rms]
        })
        .collect()
}

/// The gain from each node's output to bus `to`'s input at timeline frame
/// `frame`, through faders, outputs and send levels: `[l, r]` per node up to
/// `to`, the bus itself unity.
fn reach(plan: &MixPlan, to: usize, frame: usize) -> Vec<[f32; 2]> {
    let mut g = vec![[0.0f32; 2]; to + 1];
    if let Some(own) = g.get_mut(to) {
        *own = [1.0, 1.0];
    }
    // Edges run upward, so every node after this one is already worked out.
    for n in (0..to).rev() {
        let Some(node) = plan.nodes.get(n) else {
            continue;
        };
        let fader = node.fader.at(frame);
        let mut sum = [0.0f32; 2];
        let mut add = |via: usize, gain: [f32; 2]| {
            if let Some(r) = g.get(via).copied() {
                sum[0] += gain[0] * r[0];
                sum[1] += gain[1] * r[1];
            }
        };
        if node.out_on {
            if let Some(o) = node.out {
                add(usize::from(o), fader);
            }
        }
        for send in node.sends.iter().filter(|s| s.on) {
            let level = send.level.at(frame);
            let from = if send.pre { [1.0, 1.0] } else { fader };
            add(
                usize::from(send.to),
                [level[0] * from[0], level[1] * from[1]],
            );
        }
        g[n] = sum;
    }
    g
}

/// [`node_peaks`] for bus `to`, put together from the files' pyramids, as
/// [`MixPlan::peaks_wide`] puts the master's together.
fn wide(
    plan: &MixPlan,
    rate: u32,
    to: usize,
    start_s: f64,
    end_s: f64,
    buckets: usize,
    pyramid_of: &mut dyn FnMut(&SoundSource) -> Option<Arc<PeakPyramid>>,
) -> Vec<f32> {
    let rate_f = f64::from(rate);
    let step_s = (end_s - start_s) / buckets as f64;
    let paths: Vec<Vec<[f32; 2]>> = (0..buckets)
        .map(|b| {
            reach(
                plan,
                to,
                ((start_s + step_s * (b as f64 + 0.5)) * rate_f).max(0.0) as usize,
            )
        })
        .collect();
    let mut sums = vec![(0.0f32, 0.0f32, 0.0f64); buckets];
    for clip in plan.clips.iter().filter(|c| !c.silent && c.node != NONE) {
        let n = usize::from(clip.node);
        if n > to {
            continue;
        }
        let c0 = clip.start_frame as f64 / rate_f;
        let c1 = c0 + clip.len as f64 / rate_f;
        let (from_s, to_s) = (c0.max(start_s), c1.min(end_s));
        if to_s <= from_s {
            continue;
        }
        let pyramid = clip.source.sound().and_then(&mut *pyramid_of);
        let first = ((from_s - start_s) / step_s).floor().max(0.0) as usize;
        let last = (((to_s - start_s) / step_s).ceil().max(0.0) as usize).min(buckets);
        for b in first..last {
            let b0 = start_s + step_s * b as f64;
            let (a, z) = (b0.max(from_s), (b0 + step_s).min(to_s));
            if z <= a {
                continue;
            }
            let path = paths[b][n];
            if path == [0.0, 0.0] {
                continue;
            }
            let at = ((a - c0) * rate_f) as usize;
            let count = (((z - a) * rate_f) as usize).max(1);
            let own = clip
                .envelope
                .as_ref()
                .map_or(clip.gain, |e| e.gain_at(at + count / 2));
            let g = [own[0] * path[0], own[1] * path[1]];
            let (lo, hi, rms) = match &pyramid {
                Some(pyramid) => {
                    let s0 = (clip.src_start + at) as f64 / rate_f;
                    let w = pyramid.window(Band::Full, s0, s0 + (z - a));
                    let gm = 0.5 * (g[0] + g[1]);
                    (w.min * gm, w.max * gm, w.rms * gm)
                }
                None => {
                    let stride = (count / PEAK_SAMPLES_PER_BUCKET).max(1);
                    let (mut lo, mut hi, mut sq, mut k) = (0.0f32, 0.0f32, 0.0f64, 0u32);
                    for i in (at..at + count).step_by(stride) {
                        let Some((l, r)) = clip.source.frame(clip.src_start + i) else {
                            continue;
                        };
                        let (l, r) = (l * g[0], r * g[1]);
                        lo = lo.min(l.min(r));
                        hi = hi.max(l.max(r));
                        sq += f64::from(l * l + r * r) * 0.5;
                        k += 1;
                    }
                    let rms = if k == 0 {
                        0.0
                    } else {
                        (sq / f64::from(k)).sqrt() as f32
                    };
                    (lo, hi, rms)
                }
            };
            let sum = &mut sums[b];
            sum.0 += lo;
            sum.1 += hi;
            sum.2 += f64::from(rms * rms) * ((z - a) / step_s);
        }
    }
    sums.into_iter()
        .flat_map(|(lo, hi, sq)| [lo, hi, sq.sqrt() as f32])
        .collect()
}

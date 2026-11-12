//! The frames a Stabilise analysis reads, decoded on a thread of their own and
//! brought down to the size the tracker follows them at.
//!
//! In plain terms: decoding a frame and following specks through it take about
//! as long as each other, so they are done side by side. A decode thread opens
//! the clip, shrinks each frame to at most [`TRACK_EDGE`] on its long side and
//! hands it over through a short queue while the analysis thread follows the
//! frame before. The queue is bounded, so a slow tracker holds the decoder back
//! rather than letting frames pile up in memory.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use crate::track::LumaFrames;

/// The longest edge the tracker follows frames at. A speck a lens can resolve
/// survives the shrink, and the per-frame cost stops growing with the footage:
/// a 4K clip is followed at 1280 by 720.
pub const TRACK_EDGE: usize = 1280;

/// How many decoded frames may wait for the tracker.
const QUEUE: usize = 4;

/// What the decode thread says before its first frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opened {
    /// The clip's frames, its raster as decoded, and its rate.
    pub clip: usize,
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    /// Layer pixels per decoded pixel: more than one for a nested comp
    /// rendered smaller than it is.
    pub to_layer: f64,
    /// How many decoded pixels each tracked pixel averages, along each side.
    pub div: usize,
}

impl Opened {
    /// The tracked raster.
    #[must_use]
    pub fn tracked(&self) -> (usize, usize) {
        (self.width / self.div, self.height / self.div)
    }
}

/// One frame, ready to follow: its number and its brightness at the tracked
/// size.
pub struct Plane {
    pub n: usize,
    pub luma: Vec<f32>,
}

/// The decode thread's side of the hand-over: what was opened, then the frames
/// `first..=last` in order, stopping early at the first that will not decode.
pub struct Feed {
    pub opened: Opened,
    pub frames: Receiver<Plane>,
}

/// Start decoding on a thread of its own. `None` when the frames could not be
/// opened, or the clip has none.
pub fn start(
    open: Box<dyn FnOnce() -> Option<Box<dyn LumaFrames>> + Send>,
    range: impl FnOnce(usize, f64) -> (usize, usize) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> Option<Feed> {
    let (head_tx, head_rx) = sync_channel::<Option<(Opened, usize, usize)>>(1);
    let (tx, rx) = sync_channel::<Plane>(QUEUE);
    let spawned = std::thread::Builder::new()
        .name("lumit-stabilise-decode".into())
        .spawn(move || {
            let Some(mut frames) = open() else {
                let _ = head_tx.send(None);
                return;
            };
            let (clip, w, h, fps) = frames.info();
            let (w, h) = (w as usize, h as usize);
            if clip == 0 || w == 0 || h == 0 || !fps.is_finite() || fps <= 0.0 {
                let _ = head_tx.send(None);
                return;
            }
            let div = w.max(h).div_ceil(TRACK_EDGE).max(1);
            let scale = frames.analysis_scale();
            let to_layer = if scale > 0.0 { 1.0 / scale } else { 1.0 };
            let (first, last) = range(clip, fps);
            let opened = Opened {
                clip,
                width: w,
                height: h,
                fps,
                to_layer,
                div,
            };
            if head_tx.send(Some((opened, first, last))).is_err() {
                return;
            }
            for n in first..=last {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let Some(luma) = frames.luma(n) else {
                    return;
                };
                let luma = shrink(&luma, w, h, div);
                if tx.send(Plane { n, luma }).is_err() {
                    return;
                }
            }
        });
    if spawned.is_err() {
        return None;
    }
    let (opened, _, _) = head_rx.recv().ok().flatten()?;
    Some(Feed { opened, frames: rx })
}

/// `luma` (`w` by `h`) averaged down `div` times along each side. The box the
/// tracker's own pyramid uses, so a halved frame here is the level it would
/// have made.
#[must_use]
pub fn shrink(luma: &[f32], w: usize, h: usize, div: usize) -> Vec<f32> {
    if div <= 1 {
        return luma.to_vec();
    }
    let (sw, sh) = (w / div, h / div);
    let mut out = vec![0.0f32; sw * sh];
    let norm = 1.0 / (div * div) as f32;
    for y in 0..sh {
        for x in 0..sw {
            let mut sum = 0.0f32;
            for dy in 0..div {
                let row = (y * div + dy) * w + x * div;
                if let Some(px) = luma.get(row..row + div) {
                    sum += px.iter().sum::<f32>();
                }
            }
            out[y * sw + x] = sum * norm;
        }
    }
    out
}

/// A frame's brightness as an eight-by-eight grid of block means: enough to see
/// a cut, which changes all of them at once.
#[must_use]
pub fn signature(luma: &[f32], w: usize, h: usize) -> [f32; 64] {
    let mut out = [0.0f32; 64];
    if w < 8 || h < 8 {
        return out;
    }
    for (b, slot) in out.iter_mut().enumerate() {
        let (bx, by) = (b % 8, b / 8);
        let (x0, x1) = (bx * w / 8, (bx + 1) * w / 8);
        let (y0, y1) = (by * h / 8, (by + 1) * h / 8);
        let mut sum = 0.0f32;
        // Every fourth pixel either way: a cut moves the mean of a block by a
        // lot, and a quarter of the pixels see that as clearly as all of them.
        for y in (y0..y1).step_by(4) {
            for x in (x0..x1).step_by(4) {
                sum += luma.get(y * w + x).copied().unwrap_or(0.0);
            }
        }
        let count = y1.saturating_sub(y0).div_ceil(4) * x1.saturating_sub(x0).div_ceil(4);
        *slot = sum / count.max(1) as f32;
    }
    out
}

/// How different two signatures are: the mean absolute difference of their
/// blocks, in luma.
#[must_use]
pub fn change(a: &[f32; 64], b: &[f32; 64]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / 64.0
}

/// Which frame pairs jumped as a cut does: a change far above the shot's own
/// usual change between neighbouring frames, and large in itself.
#[must_use]
pub fn jumps(changes: &[f32]) -> Vec<bool> {
    let mut sorted: Vec<f32> = changes.to_vec();
    sorted.sort_by(f32::total_cmp);
    let usual = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    changes
        .iter()
        .map(|&c| c > 0.06 && c > 4.0 * usual)
        .collect()
}

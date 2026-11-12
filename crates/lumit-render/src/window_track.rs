//! Tracking a Correction's window across a shot, on the analysis thread.
//!
//! **In plain terms.** The bridge works out which frames are to be tracked,
//! in order, where the window starts, and how to draw each frame. This
//! draws those frames one at a time, hands each to the window follower in
//! `lumit-track`, and gives back the window's box on every frame it could
//! follow. It shares the one analysis slot, the progress reading and Cancel
//! with the other trackers, so two long decodes never run at once.

use std::sync::atomic::{AtomicBool, Ordering};

use lumit_track::window::{WindowBox, WindowFollower, WindowOutline, WindowSettings};
use lumit_track::FramePlane;
use uuid::Uuid;

use crate::track::{AnalysisError, LumaFrames, Progress, Requested};

/// One window track, as handed to the analysis thread.
pub struct WindowJob {
    /// What the run is filed under: the Correction's instance id, which Cancel
    /// and the progress reading name.
    pub effect: Uuid,
    /// Opens the frames to follow, on the analysis thread.
    pub open: Box<dyn FnOnce() -> Option<Box<dyn LumaFrames>> + Send>,
    /// The moment of each step on the frames' own clock, seconds, in
    /// tracking order.
    pub times: Vec<f64>,
    /// The picture the window's rows are in, pixels: the clip's or the layer's
    /// own size. The frames may be drawn at another.
    pub raster: (f64, f64),
    /// The window on the first step, in the rows' pixels.
    pub start: WindowBox,
    pub outline: WindowOutline,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEnd {
    /// Every step was followed.
    Finished,
    /// The window could not be followed onto the next step.
    Lost,
    /// Cancel was pressed.
    Cancelled,
}

/// What a run followed: one box per step from the first, in the rows' pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRun {
    pub boxes: Vec<WindowBox>,
    pub end: WindowEnd,
}

/// Start `job` on the analysis thread. `step` hears each step as it starts
/// (done, of how many); `done` hears the outcome before the slot is let go.
pub fn request_window(
    job: WindowJob,
    step: impl Fn(usize, usize) + Send + 'static,
    done: impl FnOnce(Result<WindowRun, AnalysisError>) + Send + 'static,
) -> Requested {
    let effect = job.effect;
    crate::track::start(effect, move |cancel| {
        let outcome = follow(job, cancel, &|k, n| {
            crate::track::report(effect, Progress::Tracking { done: k, total: n });
            step(k, n);
        });
        let progress = match &outcome {
            Ok(run) if run.end == WindowEnd::Cancelled => Progress::Cancelled,
            Ok(_) => Progress::Done,
            Err(e) => Progress::Failed(e.clone()),
        };
        done(outcome);
        crate::track::finish(effect, Some(progress));
    })
}

/// The frame loop: one frame and one push per step, checking Cancel before
/// each.
fn follow(
    job: WindowJob,
    cancel: &AtomicBool,
    report: &dyn Fn(usize, usize),
) -> Result<WindowRun, AnalysisError> {
    let mut frames = (job.open)().ok_or(AnalysisError::Unreadable)?;
    let (count, w, h, fps) = frames.info();
    let (rw, rh) = job.raster;
    let usable = |v: f64| v.is_finite() && v > 0.0;
    if count == 0 || w == 0 || h == 0 || !usable(rw) || !usable(rh) || !usable(fps) {
        return Err(AnalysisError::NoFrames);
    }
    // Frame pixels per row pixel, each way: less than one for a picture
    // drawn smaller than its own size.
    let k = [f64::from(w) / rw, f64::from(h) / rh];
    let to_frame = |b: WindowBox| WindowBox {
        centre: [b.centre[0] * k[0], b.centre[1] * k[1]],
        size: [b.size[0] * k[0], b.size[1] * k[1]],
        angle: b.angle,
    };
    let to_rows = |b: WindowBox| WindowBox {
        centre: [b.centre[0] / k[0], b.centre[1] / k[1]],
        size: [b.size[0] / k[0], b.size[1] / k[1]],
        angle: b.angle,
    };
    let (w, h) = (w as usize, h as usize);
    let mut follower = WindowFollower::new(
        to_frame(job.start),
        job.outline,
        (w, h),
        WindowSettings::default(),
    );
    let total = job.times.len();
    let last = count.saturating_sub(1);
    let mut boxes = Vec::with_capacity(total);
    let mut end = WindowEnd::Finished;
    for (i, t) in job.times.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            end = WindowEnd::Cancelled;
            break;
        }
        report(i, total);
        // The frame showing at that moment, as the planar track picks it.
        let n = (t * fps).round().max(0.0).min(last as f64) as usize;
        let Some(luma) = frames.luma(n) else {
            end = WindowEnd::Lost;
            break;
        };
        let plane = FramePlane::new(&luma, w, h).map_err(AnalysisError::Tracking)?;
        match follower.push(plane).map_err(AnalysisError::Tracking)? {
            Some(b) => boxes.push(to_rows(b)),
            None => {
                end = WindowEnd::Lost;
                break;
            }
        }
    }
    report(boxes.len(), total);
    Ok(WindowRun { boxes, end })
}

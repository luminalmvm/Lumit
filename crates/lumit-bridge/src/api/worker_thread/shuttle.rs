//! The J and L shuttle heard. A leg played backwards, or faster than the
//! comp's rate, runs no mix of its own, so its moving playhead is handed to
//! the jog, which plays toward it at the leg's speed with the pitch
//! following. Runs on the worker thread, once a turn.

use super::Playback;
use std::sync::atomic::{AtomicU64, Ordering};

/// The fastest leg the jog keeps up with. A faster one stays silent rather
/// than falling further behind its picture the longer it runs.
const FASTEST: u64 = 4;

/// The frame last handed to the jog, as bits, so a turn that has not moved
/// the playhead far sends nothing.
static SENT: AtomicU64 = AtomicU64::new(u64::MAX);

/// How far, in frames, the playhead moves before the jog is told again.
const STEP_FRAMES: f64 = 0.1;

/// Tell the jog where a silent leg's clock has got to.
pub(super) fn follow(playback: &Playback) {
    if !playback.pre_rolled || !playback.silent() || playback.speed > FASTEST {
        return;
    }
    // The leg's clock, as [`Playback::clock_frame`] reads it, between frames.
    let passed = playback.started.elapsed().as_secs_f64() * playback.fps * playback.speed as f64;
    let from = playback.from as f64;
    let frame = if playback.reverse {
        (from - passed).max(playback.first as f64)
    } else {
        (from + passed).min(playback.last as f64)
    };
    let sent = f64::from_bits(SENT.load(Ordering::Relaxed));
    if (frame - sent).abs() < STEP_FRAMES {
        return;
    }
    SENT.store(frame.to_bits(), Ordering::Relaxed);
    #[cfg(feature = "media")]
    {
        let Ok(project) = playback.comp.project() else {
            return;
        };
        let Ok(doc) = project.read().map(|p| p.store.snapshot()) else {
            return;
        };
        crate::audio::scrub::jog(playback.comp.id, frame, doc);
    }
}

//! The decode-ahead thread (docs/impl/playback-scheduler.md §5: decode(N+k)
//! runs alongside evaluate/present, not before them).
//!
//! # In plain terms
//!
//! During playback the worker knows exactly which source frames the next few
//! renders will need — the plan tells it. This thread decodes them EARLY, on
//! its own decoders, and hands the pixels back; the worker files them into
//! the renderer's decoded-frame cache, so when the render arrives its decode
//! is a lookup. The render thread and the decode thread then work at the same
//! time instead of taking turns, and a frame costs the LARGER of decode and
//! composite rather than their sum.
//!
//! Correctness is carried by the cache key, not by trust: a result is filed
//! under (item, source frame, decode width) — the same key the render's own
//! decode would use — so the worst a late or wasted prefetch can do is warm
//! the cache with pixels nobody asks for. That is also why a stop or seek
//! needs no cancellation here: a result that arrives late is still correct,
//! and filing it is a favour to the next visit, never a hazard.

use lumit_render::PrefetchWant;
#[cfg(feature = "media")]
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use uuid::Uuid;

pub(crate) struct Done {
    pub item: Uuid,
    pub frame: usize,
    pub target_width: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The worker's handle: send wants, drain finished decodes. Dropping it ends
/// the thread (its receiver disconnects).
pub(crate) struct Prefetcher {
    tx: Sender<PrefetchWant>,
    rx: Receiver<Done>,
}

impl Default for Prefetcher {
    fn default() -> Self {
        let (tx, jobs) = channel::<PrefetchWant>();
        let (done_tx, rx) = channel::<Done>();
        std::thread::spawn(move || run(jobs, done_tx));
        Self { tx, rx }
    }
}

impl Prefetcher {
    /// Queue one decode-ahead. Never blocks; a dead thread makes this a no-op
    /// (playback then simply decodes inline, exactly as before prefetch).
    pub(crate) fn request(&self, want: PrefetchWant) {
        let _ = self.tx.send(want);
    }

    /// Everything decoded since the last drain.
    pub(crate) fn drain(&self) -> Vec<Done> {
        let mut out = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            out.push(done);
        }
        out
    }
}

/// The thread: its own decoders (the renderer's are untouched — no lock ever
/// crosses the seam), decoding jobs in the order they arrive. Playback asks
/// for frames in playing order, so the decoders run sequentially — the cheap
/// direction. A job that fails to decode is skipped: the render will try it
/// inline and surface the error through the path that already knows how.
#[cfg(feature = "media")]
fn run(jobs: Receiver<PrefetchWant>, done: Sender<Done>) {
    let mut decoders: HashMap<Uuid, lumit_media::VideoDecoder> = HashMap::new();
    while let Ok(want) = jobs.recv() {
        let dec = match decoders.entry(want.item) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                // Through the shared sidecar-cache helper, like every other
                // decoder open: a decode-ahead thread that re-scanned the file
                // would spend the first seconds of playback doing work the
                // probe had already done.
                let Ok(index) = lumit_render::media_index::load_or_build_index(&want.path) else {
                    continue;
                };
                let Ok(dec) = lumit_media::VideoDecoder::open(&want.path, index) else {
                    continue;
                };
                e.insert(dec)
            }
        };
        let frame = want.frame.min(dec.frame_count().saturating_sub(1));
        let Ok(out) = dec.frame_rgba(frame, want.target_width) else {
            continue;
        };
        if done
            .send(Done {
                item: want.item,
                frame: want.frame,
                target_width: want.target_width,
                width: out.width,
                height: out.height,
                rgba: out.rgba,
            })
            .is_err()
        {
            return;
        }
    }
}

/// Without the decoder there is nothing to decode ahead (K-273). The thread
/// still exists and still drains its queue, so the worker's request/drain
/// calls need no feature gate of their own — it simply never produces a
/// result, and every frame decodes inline exactly as it did before prefetch
/// existed. `--no-default-features` is a build without FFmpeg, not a build
/// with a different scheduler.
#[cfg(not(feature = "media"))]
fn run(jobs: Receiver<PrefetchWant>, _done: Sender<Done>) {
    while jobs.recv().is_ok() {}
}

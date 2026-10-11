//! The decode-ahead threads (docs/impl/playback-scheduler.md §5: decode(N+k)
//! runs alongside evaluate/present, not before them).
//!
//! # In plain terms
//!
//! During playback the worker knows exactly which source frames the next few
//! renders will need, because the plan tells it. These threads decode them
//! EARLY, on their own decoders, and hand the pixels back. The worker files
//! them into the renderer's decoded-frame cache, so when the render arrives
//! its decode is a lookup. Rendering and decoding then happen at the same
//! time instead of taking turns, and a frame costs the LARGER of decode and
//! composite rather than their sum.
//!
//! **One thread for each file being read**, as docs/05 §2 has it. There was
//! one thread for all of them, and a cut showed why that is not enough. Three
//! picture rows at 1080p60 kept it busy most of every frame just decoding, and
//! opening the next clip's file takes about as long as six frames last, so
//! each new file stalled the rows that were already playing behind it. With a
//! thread apiece, a file being opened holds up nothing but itself.
//!
//! Correctness is carried by the cache key, not by trust: a result is filed
//! under (item, source frame, decode width), the same key the render's own
//! decode would use, so the worst a late or wasted decode can do is warm the
//! cache with pixels nobody asks for. That is also why a stop or seek needs no
//! cancellation here: a result that arrives late is still correct, and filing
//! it is a favour to the next visit, never a hazard.
//!
//! Owned and called by the thread that renders. Each stream's own thread only
//! decodes.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use uuid::Uuid;

use crate::decode::{OpenDecoders, MAX_OPEN_DECODERS};
use crate::headless::PrefetchWant;

/// How many frames' wants may wait on one stream. Playback posts a handful of
/// frames ahead, so this is room to spare. Past it a want is dropped and the
/// render decodes that frame itself.
const WAITING_PER_STREAM: usize = 16;

/// How many finished decodes may wait to be collected. Each is a whole
/// picture, so the bound is what stops a paused player from holding hundreds
/// of megabytes nobody will read. Past it a decode is dropped, as above.
const FINISHED_WAITING: usize = 32;

/// One finished decode, to be filed with
/// [`crate::headless::HeadlessRenderer::preload_decoded`].
pub struct Prefetched {
    pub item: Uuid,
    pub frame: usize,
    pub target_width: Option<u32>,
    /// The decode itself, handed on whole, so a float frame is filed as what
    /// it is rather than read as bytes.
    pub decoded: lumit_media::DecodedFrame,
}

/// One file being read ahead: the way to its thread, and when it is next
/// expected to be wanted.
struct Stream {
    tx: SyncSender<Vec<PrefetchWant>>,
    /// The frame count ([`Prefetcher::frame`]) this stream was last asked
    /// for, or will first be asked for when it was opened ahead of its clip.
    wanted: u64,
}

/// The renderer's handle on the decode-ahead threads: send wants, drain
/// finished decodes. Dropping it ends every thread.
///
/// **The streams are capped** at [`MAX_OPEN_DECODERS`], each being a thread
/// and an open decoder. When another is wanted past that, the one wanted
/// longest ago goes. One wanted for this frame or the last never goes, nor
/// one opened ahead whose clip has not started, so the count can pass the cap
/// for as long as that many files really are in use at once.
pub struct Prefetcher {
    streams: HashMap<Uuid, Stream>,
    /// Counts the frames asked for, which is the clock "lately" is read on.
    frame: u64,
    done_tx: SyncSender<Prefetched>,
    done_rx: Receiver<Prefetched>,
}

impl Default for Prefetcher {
    fn default() -> Self {
        let (done_tx, done_rx) = sync_channel(FINISHED_WAITING);
        Self {
            streams: HashMap::new(),
            frame: 0,
            done_tx,
            done_rx,
        }
    }
}

impl Prefetcher {
    /// Queue one coming frame's decodes. Never blocks. A want that cannot be
    /// queued is dropped, and playback decodes that frame inline, exactly as
    /// it did before there was any decode-ahead.
    pub fn request(&mut self, frame: Vec<PrefetchWant>) {
        self.frame += 1;
        self.send(frame, 0);
    }

    /// Queue decodes for clips that start `frames_ahead` frames from now
    /// ([`crate::headless::HeadlessRenderer::cut_wants`]). Their files are
    /// opened at once and kept until the clips have come and gone.
    pub fn request_ahead(&mut self, wants: Vec<PrefetchWant>, frames_ahead: u64) {
        self.send(wants, frames_ahead);
    }

    /// Everything decoded since the last drain.
    pub fn drain(&self) -> Vec<Prefetched> {
        let mut out = Vec::new();
        while let Ok(done) = self.done_rx.try_recv() {
            out.push(done);
        }
        out
    }

    /// How many files are open for reading ahead.
    #[must_use]
    pub fn open(&self) -> usize {
        self.streams.len()
    }

    /// Hand each want to its file's stream, starting one where there is none.
    fn send(&mut self, wants: Vec<PrefetchWant>, frames_ahead: u64) {
        let wanted = self.frame + frames_ahead;
        // Grouped by file, in the order the files were first named, so one
        // message is everything this frame wants of one decoder.
        let mut groups: Vec<(Uuid, Vec<PrefetchWant>)> = Vec::new();
        for want in wants {
            match groups.iter_mut().find(|(item, _)| *item == want.item) {
                Some((_, group)) => group.push(want),
                None => groups.push((want.item, vec![want])),
            }
        }
        for (item, group) in groups {
            if !self.streams.contains_key(&item) {
                self.make_room();
                let Some(tx) = spawn(self.done_tx.clone()) else {
                    continue;
                };
                self.streams.insert(item, Stream { tx, wanted });
            }
            if let Some(stream) = self.streams.get_mut(&item) {
                stream.wanted = stream.wanted.max(wanted);
                let _ = stream.tx.try_send(group);
            }
        }
    }

    /// Close the streams wanted longest ago until there is room under the
    /// cap, stopping at the first that is still in use. Dropping a stream's
    /// sender is what ends its thread.
    fn make_room(&mut self) {
        while self.streams.len() >= MAX_OPEN_DECODERS {
            // By id where two were last wanted together, so which one closes
            // never depends on the map's order.
            let oldest = self
                .streams
                .iter()
                .map(|(id, stream)| (stream.wanted, *id))
                .min();
            match oldest {
                Some((wanted, id)) if wanted + 1 < self.frame => self.streams.remove(&id),
                _ => break,
            };
        }
    }
}

/// Start one stream's thread. `None` when the system will not give one, and
/// the frames it would have decoded are decoded by the render instead.
fn spawn(done: SyncSender<Prefetched>) -> Option<SyncSender<Vec<PrefetchWant>>> {
    let (tx, jobs) = sync_channel::<Vec<PrefetchWant>>(WAITING_PER_STREAM);
    std::thread::Builder::new()
        .name("lumit-decode-ahead".into())
        .spawn(move || run(&jobs, &done))
        .ok()
        .map(|_| tx)
}

/// One stream's thread: its own decoder, decoding what it is sent in the
/// order it arrives. Playback asks for frames in playing order, so the
/// decoder runs sequentially, the cheap direction. A want that fails to
/// decode is skipped: the render will try it inline and surface the error
/// through the path that already knows how. Ends when its sender is dropped.
fn run(jobs: &Receiver<Vec<PrefetchWant>>, done: &SyncSender<Prefetched>) {
    // Never more than the one decoder: every want here is for one file.
    let mut decoders = OpenDecoders::default();
    // The widths already read by the file's own reader. Such a file is a
    // still, and every coming frame asks for it again.
    let mut read: HashSet<Option<u32>> = HashSet::new();
    // The file this stream is reading. An item whose file has changed, as
    // one relinked or one whose stand-in gave way to the original has, is
    // opened again rather than read on from the old file.
    let mut source: Option<std::path::PathBuf> = None;
    while let Ok(frame) = jobs.recv() {
        for want in frame {
            if source.as_ref() != Some(&want.source.path) {
                if source.is_some() {
                    decoders.close(want.item);
                    read.clear();
                }
                source = Some(want.source.path.clone());
            }
            // One layer of a layered file, or an Illustrator document, is
            // read by the file's own reader, as the render reads it. ffmpeg
            // would hand back the flattened picture of the one and can't open
            // the other.
            let decoded = if lumit_media::reads_own(&want.source) {
                if !read.insert(want.target_width) {
                    continue;
                }
                match lumit_media::read_own(&want.source, want.target_width) {
                    Some(Ok(out)) => out,
                    _ => continue,
                }
            } else {
                match decoders.decode_want(&want) {
                    Some(out) => out,
                    None => continue,
                }
            };
            // Full means nobody is collecting, and the picture is let go.
            let _ = done.try_send(Prefetched {
                item: want.item,
                frame: want.frame,
                target_width: want.target_width,
                decoded,
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A want for a file that is not there: the stream starts, fails to open
    /// it and waits for more, which is all these tests need of one.
    fn want(item: Uuid) -> PrefetchWant {
        PrefetchWant {
            item,
            source: lumit_media::MediaSource::file("not-on-disk.mp4"),
            frame: 0,
            target_width: None,
        }
    }

    /// Playing through more files than the cap closes the ones left behind,
    /// and never the one opened ahead for a clip that has not started: that
    /// is the next file playback needs, however long ago it was asked for.
    #[test]
    fn the_cap_closes_what_was_left_behind_and_keeps_what_is_coming() {
        let mut ahead = Prefetcher::default();
        let coming = Uuid::now_v7();
        ahead.request_ahead(vec![want(coming)], 3 * MAX_OPEN_DECODERS as u64);
        let first = Uuid::now_v7();
        ahead.request(vec![want(first)]);
        for _ in 0..2 * MAX_OPEN_DECODERS {
            ahead.request(vec![want(Uuid::now_v7())]);
        }
        assert_eq!(ahead.open(), MAX_OPEN_DECODERS, "the cap holds");
        assert!(!ahead.streams.contains_key(&first), "the oldest closed");
        assert!(
            ahead.streams.contains_key(&coming),
            "the next clip's file stayed"
        );

        // One frame that reads more files than the cap keeps them all.
        let mut wide = Prefetcher::default();
        let frame: Vec<PrefetchWant> = (0..MAX_OPEN_DECODERS + 4)
            .map(|_| want(Uuid::now_v7()))
            .collect();
        wide.request(frame);
        assert_eq!(wide.open(), MAX_OPEN_DECODERS + 4);
    }
}
